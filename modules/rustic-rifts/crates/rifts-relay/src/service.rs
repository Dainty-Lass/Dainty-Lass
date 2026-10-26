//! The relay routes' operations: `p2p/controllers/p2pIce.ts` (`transport-config`, `signal`) and
//! `p2pAdminOperations.ts:UpdateCoopTransport`, over what realms keeps ([`IceHunts`], RF5's).

use std::sync::Arc;

use rifts_seam::{AccountId, BoxFuture, Json, Refusal, Reply, SessionId, UnixMillis};
use serde_json::Value;

use crate::credentials::{HourlyCounter, RelayGrants, relay_ttl_seconds};
use crate::rules::{
    CoopTransport, HuntSnapshot, MAXIMUM_HUB_PEERS, PresenceRow, filter_relay_only, ice_peers_for, parse_ice_signal,
    peer_id_of, resolve_role, signal_route_allowed,
};
use crate::wire::{JsonObject, is_safe_integer, js_number_value, js_trim, json_array, random_uuid};
use crate::{IceService, RelayError};

const SIGNALS_PER_HOUR: u32 = 300;
/// A hub's host sends to every member that joins.
const HUB_HOST_SIGNALS_PER_HOUR: u32 = 3_000;
/// A realm's members and hunts ask more often than one party's hunts (an entry, a travel, a return).
const REALM_CONFIGS_PER_HOUR: u32 = 120;
const SIGNAL_LIFETIME_MS: i64 = 120_000;

/// `ReadP2PPolicy().enabled` and the operations policy's emergency stop, both realms'.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P2PGate {
    pub enabled: bool,
    pub emergency_stop: bool,
}

/// One message for the launcher's control channel (`enqueueControlMessage`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlMessage {
    pub account: AccountId,
    /// `ICE_SIGNAL`.
    pub message_type: &'static str,
    pub wire_session: String,
    pub generation: u64,
    pub payload: Json,
    pub created_at: UnixMillis,
    pub expires_at: UnixMillis,
}

/// What the relay reads and writes in realms (RF5): hunts, hub presence, the control queue. Sync, like the store
/// repositories; the store-backed implementation is RF5's.
pub trait IceHunts: Send + Sync {
    fn gate(&self) -> Result<P2PGate, RelayError>;
    fn find_hunt(&self, session: &SessionId) -> Result<Option<HuntSnapshot>, RelayError>;
    /// A hub's live presence rows (`PENDING`, `TICKETED`, `ADMITTED`), at most `limit`.
    fn hub_presence(&self, hunt: &HuntSnapshot, limit: usize) -> Result<Vec<PresenceRow>, RelayError>;
    fn enqueue(&self, message: ControlMessage) -> Result<(), RelayError>;
}

/// The admin route's write, through realms' operations policy (with its audit row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoopTransportOutcome {
    /// The policy after the change, as realms writes it.
    Updated(Json),
    /// The policy as it is, when `expectedVersion` no longer matches.
    Conflict(Json),
}

pub trait OperationsPolicy: Send + Sync {
    fn set_coop_transport(
        &self,
        admin_key_id: &str,
        transport: CoopTransport,
        expected_version: u64,
        reason: &str,
        request_id: &str,
    ) -> Result<CoopTransportOutcome, RelayError>;
}

pub fn refusal(code: &str, message: &str) -> RelayError {
    RelayError::Refused(Refusal {
        code: code.to_owned(),
        message: message.to_owned(),
    })
}

/// The real [`IceService`].
pub struct Ice {
    pub hunts: Arc<dyn IceHunts>,
    pub grants: RelayGrants,
    pub policy: Arc<dyn OperationsPolicy>,
    pub clock: Arc<dyn Fn() -> UnixMillis + Send + Sync>,
    configs: HourlyCounter,
    signals: HourlyCounter,
}

impl Ice {
    pub fn new(
        hunts: Arc<dyn IceHunts>,
        grants: RelayGrants,
        policy: Arc<dyn OperationsPolicy>,
        clock: Arc<dyn Fn() -> UnixMillis + Send + Sync>,
    ) -> Self {
        Self {
            hunts,
            grants,
            policy,
            clock,
            configs: HourlyCounter::default(),
            signals: HourlyCounter::default(),
        }
    }

    /// `RequireP2PRunning`.
    fn require_running(&self) -> Result<(), RelayError> {
        let gate = self.hunts.gate()?;
        if !gate.enabled {
            return Err(refusal("P2P_DISABLED", "Player-hosted hunts are disabled."));
        }
        if gate.emergency_stop {
            return Err(refusal(
                "P2P_EMERGENCY_STOP",
                "Player-hosted hunts are currently stopped by operations policy.",
            ));
        }
        Ok(())
    }

    /// `LoadActiveIceHunt`: the hunt's own frozen transport decides, so a dashboard switch never changes a running hunt.
    fn active_ice_hunt(&self, session: &SessionId, generation: Option<i64>) -> Result<HuntSnapshot, RelayError> {
        let generation = generation.ok_or_else(|| refusal("P2P_TICKET_INVALID", "The hunt generation is missing."))?;
        let hunt = self
            .hunts
            .find_hunt(session)?
            .ok_or_else(|| refusal("P2P_SESSION_NOT_FOUND", "Hunt session not found."))?;
        if hunt.coop_transport != CoopTransport::Ice {
            return Err(refusal(
                "P2P_TRANSPORT_MISMATCH",
                "This hunt does not use the ICE co-op transport.",
            ));
        }
        if i64::try_from(hunt.generation).ok() != Some(generation) {
            return Err(refusal(
                "P2P_GENERATION_FENCED",
                "This request belongs to a stale hunt generation.",
            ));
        }
        if !hunt.state.is_ice_active() {
            return Err(refusal("P2P_SESSION_NOT_READY", "The hunt is not running."));
        }
        Ok(hunt)
    }

    fn presence_of(&self, hunt: &HuntSnapshot) -> Result<Option<Vec<PresenceRow>>, RelayError> {
        if !hunt.is_hub() {
            return Ok(None);
        }
        self.hunts.hub_presence(hunt, MAXIMUM_HUB_PEERS + 1).map(Some)
    }

    async fn create_transport_config(
        &self,
        account: AccountId,
        session: SessionId,
        generation: Option<i64>,
    ) -> Result<Json, RelayError> {
        self.require_running()?;
        let config = Arc::clone(&self.grants.config);
        let Some(secret) = config.transport_key_secret.clone() else {
            return Err(refusal(
                "P2P_ICE_UNAVAILABLE",
                "The co-op transport is not configured on this server.",
            ));
        };
        let hunt = self.active_ice_hunt(&session, generation)?;
        let presence = self.presence_of(&hunt)?;
        let role = resolve_role(&hunt, &account, presence.as_deref())
            .ok_or_else(|| refusal("P2P_NOT_ROSTERED", "This account is not part of the hunt."))?;
        let now = (self.clock)();
        let per_hour = if hunt.is_realm_session() {
            config.configs_per_hour.max(REALM_CONFIGS_PER_HOUR)
        } else {
            config.configs_per_hour
        };
        if !self.configs.allow(&account.0, per_hour, now) {
            return Err(refusal(
                "P2P_RELAY_RATE_LIMITED",
                "Too many co-op transport requests; try again later.",
            ));
        }
        // RelayTtlCapSeconds: a hub keeps its credential for the hub's life (no UpdateRelayCredentialH, RF1 decision 6)
        let cap = if hunt.is_hub() {
            config.hub_credential_ttl_seconds
        } else {
            config.credential_ttl_seconds
        };
        let ttl = relay_ttl_seconds(hunt.expires_at, now, cap);
        let grant = self.grants.grant(&account, ttl, now).await;
        let peers = ice_peers_for(&hunt, &account, role, &secret, presence.as_deref());
        let peer_text = |account: &AccountId| {
            peer_id_of(&hunt, account, presence.as_deref()).map_or_else(String::new, |peer| peer.0.to_string())
        };
        let peers_json = json_array(peers.iter().map(|peer| {
            JsonObject::new()
                .string("accountId", &peer.account.0)
                .string("role", peer.role.as_str())
                .string("transportKey", peer.transport_key.expose())
                .string("peerId", &peer_text(&peer.account))
                .finish()
        }));
        let body = JsonObject::new()
            .string("sessionId", &hunt.wire_session)
            .number("generation", hunt.generation)
            .string("role", role.as_str())
            .string("localAccountId", &account.0)
            .string("localPeerId", &peer_text(&account))
            .string("hostAccountId", &hunt.host_account.0)
            .string("hostPeerId", &hunt.host_peer.0.to_string())
            .boolean("forceRelay", config.force_relay)
            .raw("iceServers", &grant.ice_servers_json())
            .raw("relay", &grant.relay_json())
            .raw("peers", &peers_json)
            .finish();
        Ok(Json(body))
    }

    async fn submit(&self, account: AccountId, session: SessionId, body: Value) -> Result<Json, RelayError> {
        self.require_running()?;
        let signal = parse_ice_signal(&body)
            .ok_or_else(|| refusal("P2P_SIGNAL_INVALID", "The ICE signal is malformed or too large."))?;
        let hunt = self.active_ice_hunt(&session, Some(signal.generation))?;
        let presence = self.presence_of(&hunt)?;
        if !signal_route_allowed(&hunt, &account, &signal.to, presence.as_deref()) {
            return Err(refusal(
                "P2P_NOT_ROSTERED",
                "Signals go only between the host and a joiner of this hunt.",
            ));
        }
        let now = (self.clock)();
        let limit = if hunt.is_hub() && account == hunt.host_account {
            HUB_HOST_SIGNALS_PER_HOUR
        } else {
            SIGNALS_PER_HOUR
        };
        let key = format!("{}:{}:{}", hunt.wire_session, hunt.generation, account.0);
        if !self.signals.allow(&key, limit, now) {
            return Err(refusal("P2P_SIGNAL_LIMIT", "Too many ICE signals for this hunt."));
        }
        let data = if self.grants.config.force_relay {
            filter_relay_only(signal.kind, &signal.data)
        } else {
            Some(signal.data.clone())
        };
        let Some(data) = data else {
            return Ok(Json(
                JsonObject::new()
                    .boolean("delivered", false)
                    .string("reason", "FILTERED_RELAY_ONLY")
                    .finish(),
            ));
        };
        let signal_id = random_uuid().map_err(|_| RelayError::Random)?;
        let from_role = resolve_role(&hunt, &account, presence.as_deref());
        let payload = JsonObject::new()
            .string("sessionId", &hunt.wire_session)
            .number("generation", hunt.generation)
            .string("signalId", &signal_id)
            .string("fromAccountId", &account.0)
            .optional_string("fromRole", from_role.map(|role| role.as_str()))
            .string("kind", signal.kind.as_str())
            .string("data", &data)
            .finish();
        self.hunts.enqueue(ControlMessage {
            account: signal.to,
            message_type: "ICE_SIGNAL",
            wire_session: hunt.wire_session.clone(),
            generation: hunt.generation,
            payload: Json(payload),
            created_at: now,
            expires_at: UnixMillis(hunt.expires_at.0.min(now.0 + SIGNAL_LIFETIME_MS)),
        })?;
        Ok(Json(
            JsonObject::new()
                .boolean("delivered", true)
                .string("signalId", &signal_id)
                .finish(),
        ))
    }

    fn update_coop_transport(&self, admin_key_id: &str, body: &Value) -> Result<Reply, RelayError> {
        let transport = body
            .get("coopTransport")
            .and_then(Value::as_str)
            .and_then(CoopTransport::parse);
        let expected = body
            .get("expectedVersion")
            .map(js_number_value)
            .filter(|version| is_safe_integer(*version) && *version >= 0.0);
        // `.trim().slice(0, 500)` counts UTF-16 units
        let reason_units: Vec<u16> = body
            .get("reason")
            .and_then(Value::as_str)
            .map(|reason| js_trim(reason).encode_utf16().take(500).collect())
            .unwrap_or_default();
        let reason = String::from_utf16_lossy(&reason_units);
        let valid = reason_units.len() >= 8;
        let (Some(transport), Some(expected), true) = (transport, expected, valid) else {
            return Ok(Reply {
                status: 400,
                body: Json(
                    r#"{"error":{"code":"ADMIN_VALIDATION","message":"coopTransport (ICE or STEAM_EXPERIMENTAL), expectedVersion, and an audit reason of at least 8 characters are required."}}"#
                        .to_owned(),
                ),
            });
        };
        let request_id = random_uuid().map_err(|_| RelayError::Random)?;
        // A safe, non-negative integer by the check above
        let expected = expected as u64;
        match self
            .policy
            .set_coop_transport(admin_key_id, transport, expected, &reason, &request_id)?
        {
            CoopTransportOutcome::Updated(policy) => Ok(Reply {
                status: 200,
                body: Json(
                    JsonObject::new()
                        .raw("policy", &policy.0)
                        .string("requestId", &request_id)
                        .finish(),
                ),
            }),
            CoopTransportOutcome::Conflict(before) => Ok(Reply {
                status: 409,
                body: Json(
                    JsonObject::new()
                        .raw(
                            "error",
                            r#"{"code":"ADMIN_POLICY_CONFLICT","message":"The operations policy changed. Refresh it before trying again."}"#,
                        )
                        .raw("policy", &before.0)
                        .finish(),
                ),
            }),
        }
    }
}

impl IceService for Ice {
    fn transport_config(
        &self,
        account: AccountId,
        session: SessionId,
        generation: Option<i64>,
    ) -> BoxFuture<'_, Result<Json, RelayError>> {
        Box::pin(self.create_transport_config(account, session, generation))
    }

    fn submit_signal(
        &self,
        account: AccountId,
        session: SessionId,
        body: Value,
    ) -> BoxFuture<'_, Result<Json, RelayError>> {
        Box::pin(self.submit(account, session, body))
    }

    fn set_coop_transport(&self, admin_key_id: String, body: Value) -> BoxFuture<'_, Result<Reply, RelayError>> {
        let reply = self.update_coop_transport(&admin_key_id, &body);
        Box::pin(async move { reply })
    }
}

#[cfg(test)]
mod tests;
