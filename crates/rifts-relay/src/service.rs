// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The launcher's ICE operations (transport config, signal relay) and the admin co-op transport switch.

use std::sync::Arc;

use rifts_seam::{AccountId, BoxFuture, Json, PeerId, Refusal, Reply, SessionId, UnixMillis};
use serde_json::Value;

use crate::credentials::{HourlyCounter, RelayGrants, relay_ttl_seconds};
use crate::rules::{
    CoopTransport, HuntSnapshot, MAXIMUM_HUB_PEERS, PresenceRow, filter_relay_only, ice_peers_for, parse_ice_signal,
    peer_id_of, resolve_role, signal_route_allowed,
};
use crate::wire::{JsonObject, is_safe_integer, js_number_value, js_trim, json_array, random_uuid};
use crate::{IceService, RelayError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct P2PGate {
    pub enabled: bool,
    pub emergency_stop: bool,
}

/// A message queued for one account's launcher.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlMessage {
    pub account: AccountId,
    pub message_type: &'static str,
    pub wire_session: String,
    pub generation: u64,
    pub payload: Json,
    pub created_at: UnixMillis,
    pub expires_at: UnixMillis,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoopTransportOutcome {
    Updated(Json),
    /// The policy's version moved on; carries the current policy.
    Conflict(Json),
}

pub fn refusal(code: &str, message: &str) -> RelayError {
    RelayError::Refused(Refusal {
        code: code.to_owned(),
        message: message.to_owned(),
    })
}

/// What the relay needs from the realms side.
pub trait IceHunts: Send + Sync {
    fn gate(&self) -> Result<P2PGate, RelayError>;
    fn find_hunt(&self, session: &SessionId) -> Result<Option<HuntSnapshot>, RelayError>;
    fn hub_presence(&self, hunt: &HuntSnapshot, limit: usize) -> Result<Vec<PresenceRow>, RelayError>;
    fn enqueue(&self, message: ControlMessage) -> Result<(), RelayError>;
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

const CONFIGS_PER_HOUR_IN_REALMS: u32 = 120;
const SIGNALS_PER_HOUR: u32 = 300;
// A hub host signals every member, up to 64 of them.
const HUB_HOST_SIGNALS_PER_HOUR: u32 = 3000;
const SIGNAL_LIFETIME_MS: i64 = 120_000;
const REASON_MAX_UNITS: usize = 500;
const REASON_MIN_UNITS: usize = 8;

const ADMIN_VALIDATION: &str = r#"{"error":{"code":"ADMIN_VALIDATION","message":"coopTransport (ICE or STEAM_EXPERIMENTAL), expectedVersion, and an audit reason of at least 8 characters are required."}}"#;
const ADMIN_CONFLICT: &str =
    r#"{"code":"ADMIN_POLICY_CONFLICT","message":"The operations policy changed. Refresh it before trying again."}"#;

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

    fn running_gate(&self) -> Result<(), RelayError> {
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
        if i128::from(hunt.generation) != i128::from(generation) {
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

    fn presence(&self, hunt: &HuntSnapshot) -> Result<Option<Vec<PresenceRow>>, RelayError> {
        if hunt.is_hub() {
            self.hunts.hub_presence(hunt, MAXIMUM_HUB_PEERS + 1).map(Some)
        } else {
            Ok(None)
        }
    }

    async fn config_for(
        &self,
        account: AccountId,
        session: SessionId,
        generation: Option<i64>,
    ) -> Result<Json, RelayError> {
        self.running_gate()?;
        let config = Arc::clone(&self.grants.config);
        let secret = config.transport_key_secret.clone().ok_or_else(|| {
            refusal(
                "P2P_ICE_UNAVAILABLE",
                "The co-op transport is not configured on this server.",
            )
        })?;
        let hunt = self.active_ice_hunt(&session, generation)?;
        let presence = self.presence(&hunt)?;
        let presence = presence.as_deref();
        let role = resolve_role(&hunt, &account, presence)
            .ok_or_else(|| refusal("P2P_NOT_ROSTERED", "This account is not part of the hunt."))?;
        let now = (self.clock)();
        let limit = if hunt.is_realm_session() {
            config.configs_per_hour.max(CONFIGS_PER_HOUR_IN_REALMS)
        } else {
            config.configs_per_hour
        };
        if !self.configs.allow(&account.0, limit, now) {
            return Err(refusal(
                "P2P_RELAY_RATE_LIMITED",
                "Too many co-op transport requests; try again later.",
            ));
        }
        let cap = if hunt.is_hub() {
            config.hub_credential_ttl_seconds
        } else {
            config.credential_ttl_seconds
        };
        let ttl = relay_ttl_seconds(hunt.expires_at, now, cap);
        let grant = self.grants.grant(&account, ttl, now).await;
        let peers = ice_peers_for(&hunt, &account, role, &secret, presence);

        let peer_text = |peer: Option<PeerId>| peer.map(|peer| peer.0.to_string()).unwrap_or_default();
        let peers_json = json_array(peers.iter().map(|peer| {
            JsonObject::new()
                .string("accountId", &peer.account.0)
                .string("role", peer.role.as_str())
                .string("transportKey", peer.transport_key.expose())
                .string("peerId", &peer_text(peer_id_of(&hunt, &peer.account, presence)))
                .finish()
        }));
        let body = JsonObject::new()
            .string("sessionId", &hunt.wire_session)
            .number("generation", hunt.generation)
            .string("role", role.as_str())
            .string("localAccountId", &account.0)
            .string("localPeerId", &peer_text(peer_id_of(&hunt, &account, presence)))
            .string("hostAccountId", &hunt.host_account.0)
            .string("hostPeerId", &hunt.host_peer.0.to_string())
            .boolean("forceRelay", config.force_relay)
            .raw("iceServers", &grant.ice_servers_json())
            .raw("relay", &grant.relay_json())
            .raw("peers", &peers_json)
            .finish();
        Ok(Json(body))
    }

    fn signal(&self, account: &AccountId, session: &SessionId, body: &Value) -> Result<Json, RelayError> {
        self.running_gate()?;
        let signal = parse_ice_signal(body)
            .ok_or_else(|| refusal("P2P_SIGNAL_INVALID", "The ICE signal is malformed or too large."))?;
        let hunt = self.active_ice_hunt(session, Some(signal.generation))?;
        let presence = self.presence(&hunt)?;
        let presence = presence.as_deref();
        if !signal_route_allowed(&hunt, account, &signal.to, presence) {
            return Err(refusal(
                "P2P_NOT_ROSTERED",
                "Signals go only between the host and a joiner of this hunt.",
            ));
        }
        let now = (self.clock)();
        let limit = if hunt.is_hub() && account == &hunt.host_account {
            HUB_HOST_SIGNALS_PER_HOUR
        } else {
            SIGNALS_PER_HOUR
        };
        let key = format!("{}:{}:{}", hunt.wire_session, hunt.generation, account.0);
        if !self.signals.allow(&key, limit, now) {
            return Err(refusal("P2P_SIGNAL_LIMIT", "Too many ICE signals for this hunt."));
        }
        let data = if self.grants.config.force_relay {
            match filter_relay_only(signal.kind, &signal.data) {
                Some(data) => data,
                // Still counted against the limit above.
                None => {
                    let reply = JsonObject::new()
                        .boolean("delivered", false)
                        .string("reason", "FILTERED_RELAY_ONLY")
                        .finish();
                    return Ok(Json(reply));
                }
            }
        } else {
            signal.data
        };
        let signal_id = random_uuid().map_err(|_| RelayError::Random)?;
        let payload = JsonObject::new()
            .string("sessionId", &hunt.wire_session)
            .number("generation", hunt.generation)
            .string("signalId", &signal_id)
            .string("fromAccountId", &account.0)
            .optional_string(
                "fromRole",
                resolve_role(&hunt, account, presence).map(|role| role.as_str()),
            )
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

    fn coop_transport(&self, admin_key_id: &str, body: &Value) -> Result<Reply, RelayError> {
        let Some((transport, expected, reason)) = validate_coop_transport(body) else {
            return Ok(Reply {
                status: 400,
                body: Json(ADMIN_VALIDATION.to_owned()),
            });
        };
        let request_id = random_uuid().map_err(|_| RelayError::Random)?;
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
            CoopTransportOutcome::Conflict(policy) => Ok(Reply {
                status: 409,
                body: Json(
                    JsonObject::new()
                        .raw("error", ADMIN_CONFLICT)
                        .raw("policy", &policy.0)
                        .finish(),
                ),
            }),
        }
    }
}

/// (transport, expected version, audit reason), or `None` for the 400 answer.
fn validate_coop_transport(body: &Value) -> Option<(CoopTransport, u64, String)> {
    let transport = CoopTransport::parse(body.get("coopTransport")?.as_str()?)?;
    let expected = js_number_value(body.get("expectedVersion")?);
    if !is_safe_integer(expected) || expected < 0.0 {
        return None;
    }
    // A non-negative safe integer, so the cast is exact.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let expected = expected as u64;
    let reason = js_trim(body.get("reason").and_then(Value::as_str).unwrap_or(""));
    let reason = first_utf16_units(reason, REASON_MAX_UNITS);
    (reason.encode_utf16().count() >= REASON_MIN_UNITS).then(|| (transport, expected, reason.to_owned()))
}

/// The longest prefix of at most `limit` UTF-16 units that does not split a character.
fn first_utf16_units(text: &str, limit: usize) -> &str {
    let mut units = 0;
    for (at, c) in text.char_indices() {
        units += c.len_utf16();
        if units > limit {
            return &text[..at];
        }
    }
    text
}

impl IceService for Ice {
    fn transport_config(
        &self,
        account: AccountId,
        session: SessionId,
        generation: Option<i64>,
    ) -> BoxFuture<'_, Result<Json, RelayError>> {
        Box::pin(self.config_for(account, session, generation))
    }

    fn submit_signal(
        &self,
        account: AccountId,
        session: SessionId,
        body: Value,
    ) -> BoxFuture<'_, Result<Json, RelayError>> {
        Box::pin(std::future::ready(self.signal(&account, &session, &body)))
    }

    fn set_coop_transport(&self, admin_key_id: String, body: Value) -> BoxFuture<'_, Result<Reply, RelayError>> {
        Box::pin(std::future::ready(self.coop_transport(&admin_key_id, &body)))
    }
}

#[cfg(test)]
mod tests;
