// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The launcher's ICE operations (transport config, signal relay) and the admin co-op transport switch.

use std::{future::ready, sync::Arc};

use rustic_rifts_seam::{AccountId, Json, PeerId, Reply, SessionId, UnixMillis};
use serde_json::Value;

use crate::{
    Answer, IceService, RelayError,
    credentials::{HourlyCounter, RelayGrants, relay_ttl_seconds},
    rules::{self, CoopTransport, HuntSnapshot, MAXIMUM_HUB_PEERS, PresenceRow},
    wire::{self, JsonObject},
};

/// Whether player hosting runs.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct P2PGate {
    /// Whether player hosting is switched on.
    pub enabled: bool,
    /// Whether the operators' emergency stop is on, which stops it regardless.
    pub emergency_stop: bool,
}

/// A message queued for one account's launcher.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlMessage {
    /// The account whose launcher gets it.
    pub account: AccountId,
    /// The message type, such as `ICE_SIGNAL`.
    pub message_type: &'static str,
    /// The session it is about, as the wire spells it.
    pub wire_session: String,
    /// The session's generation.
    pub generation: u64,
    /// The message body.
    pub payload: Json,
    /// When it was queued.
    pub created_at: UnixMillis,
    /// When it is dropped undelivered.
    pub expires_at: UnixMillis,
}

/// How setting the co-op transport ended.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CoopTransportOutcome {
    /// Changed; carries the new policy.
    Updated(Json),
    /// The policy's version moved on; carries the current policy.
    Conflict(Json),
}

/// A relay refusal with this code and text.
pub fn refusal(code: &str, message: &str) -> RelayError {
    rustic_rifts_seam::Refusal {
        code: code.into(),
        message: message.into(),
    }
    .into()
}

type Outcome<T> = Result<T, RelayError>;

/// What the relay needs from the realms side.
pub trait IceHunts: Send + Sync {
    /// Whether player hosting runs.
    fn gate(&self) -> Outcome<P2PGate>;
    /// The hunt or hub session, if it exists.
    fn find_hunt(&self, id: &SessionId) -> Outcome<Option<HuntSnapshot>>;
    /// The hub's presence rows, at most `most` of them.
    fn hub_presence(&self, of: &HuntSnapshot, most: usize) -> Outcome<Vec<PresenceRow>>;
    /// Queues a message for an account's launcher.
    fn enqueue(&self, queued: ControlMessage) -> Outcome<()>;
}

/// Where the operations policy's co-op transport is changed.
pub trait OperationsPolicy: Send + Sync {
    /// Sets the transport if the policy is still at `expected_version`, recording the admin, reason and request id.
    fn set_coop_transport(
        &self,
        admin: &str,
        to: CoopTransport,
        expected_version: u64,
        reason: &str,
        request: &str,
    ) -> Outcome<CoopTransportOutcome>;
}

/// A refusal the relay answers with: its code, then its text.
#[derive(Clone, Copy)]
struct No(&'static str, &'static str);

impl From<No> for RelayError {
    fn from(No(code, text): No) -> Self {
        refusal(code, text)
    }
}

/// `Err(no)` unless `holds`.
fn ensure(holds: bool, no: No) -> Outcome<()> {
    if holds { Ok(()) } else { Err(no.into()) }
}

const OFF: No = No("P2P_DISABLED", "Player-hosted hunts are disabled.");
const STOPPED: No = No(
    "P2P_EMERGENCY_STOP",
    "Player-hosted hunts are currently stopped by operations policy.",
);
const NO_GENERATION: No = No("P2P_TICKET_INVALID", "The hunt generation is missing.");
const NO_HUNT: No = No("P2P_SESSION_NOT_FOUND", "Hunt session not found.");
const NOT_ICE: No = No(
    "P2P_TRANSPORT_MISMATCH",
    "This hunt does not use the ICE co-op transport.",
);
const STALE: No = No(
    "P2P_GENERATION_FENCED",
    "This request belongs to a stale hunt generation.",
);
const IDLE: No = No("P2P_SESSION_NOT_READY", "The hunt is not running.");
const NO_TRANSPORT: No = No(
    "P2P_ICE_UNAVAILABLE",
    "The co-op transport is not configured on this server.",
);
const OUTSIDER: No = No("P2P_NOT_ROSTERED", "This account is not part of the hunt.");
const TOO_MANY_CONFIGS: No = No(
    "P2P_RELAY_RATE_LIMITED",
    "Too many co-op transport requests; try again later.",
);
const BAD_SIGNAL: No = No("P2P_SIGNAL_INVALID", "The ICE signal is malformed or too large.");
const WRONG_ROUTE: No = No(
    "P2P_NOT_ROSTERED",
    "Signals go only between the host and a joiner of this hunt.",
);
const TOO_MANY_SIGNALS: No = No("P2P_SIGNAL_LIMIT", "Too many ICE signals for this hunt.");

/// Hourly limits; a realm's members reconnect more often, and a hub host signals up to 64 members.
const REALM_CONFIGS_FLOOR: u32 = 120;
const SIGNALS_PER_HOUR: u32 = 300;
const HUB_HOST_SIGNALS_PER_HOUR: u32 = 3000;
const SIGNAL_LIFETIME_MS: i64 = 120_000;
/// The audit reason, in UTF-16 units: at least this many after trimming, cut to the longer one.
const REASON_UNITS: (usize, usize) = (8, 500);

const ADMIN_VALIDATION: &str = r#"{"error":{"code":"ADMIN_VALIDATION","message":"coopTransport (ICE or STEAM_EXPERIMENTAL), expectedVersion, and an audit reason of at least 8 characters are required."}}"#;
const ADMIN_CONFLICT: &str =
    r#"{"code":"ADMIN_POLICY_CONFLICT","message":"The operations policy changed. Refresh it before trying again."}"#;

/// The relay's ICE operations.
pub struct Ice {
    /// The hunts and hubs the relay serves.
    pub hunts: Arc<dyn IceHunts>,
    /// Where relay credentials come from.
    pub grants: RelayGrants,
    /// The operations policy the admin switch changes.
    pub policy: Arc<dyn OperationsPolicy>,
    /// The time now.
    pub clock: Arc<dyn Fn() -> UnixMillis + Send + Sync>,
    config_calls: HourlyCounter,
    signal_calls: HourlyCounter,
}

/// A hunt as one request sees it: the snapshot and, for a hub, its presence rows.
struct Seen {
    hunt: HuntSnapshot,
    presence: Option<Vec<PresenceRow>>,
}

impl Seen {
    fn rows(&self) -> Option<&[PresenceRow]> {
        self.presence.as_deref()
    }

    fn peer_text(&self, account: &AccountId) -> String {
        rules::peer_id_of(&self.hunt, account, self.rows()).map_or_else(String::new, |PeerId(id)| id.to_string())
    }
}

impl Ice {
    /// The operations over these parts, with empty hourly limits.
    pub fn new(
        hunts: Arc<dyn IceHunts>,
        grants: RelayGrants,
        policy: Arc<dyn OperationsPolicy>,
        clock: Arc<dyn Fn() -> UnixMillis + Send + Sync>,
    ) -> Self {
        let (config_calls, signal_calls) = Default::default();
        Self {
            clock,
            policy,
            grants,
            hunts,
            config_calls,
            signal_calls,
        }
    }

    fn now(&self) -> UnixMillis {
        (self.clock)()
    }

    fn hosting_running(&self) -> Outcome<()> {
        let P2PGate {
            enabled,
            emergency_stop,
        } = self.hunts.gate()?;
        ensure(enabled, OFF)?;
        ensure(!emergency_stop, STOPPED)
    }

    /// The hunt behind `id` at `generation`, while its transports may connect, with its presence when a hub.
    fn live_ice_hunt(&self, id: &SessionId, generation: Option<i64>) -> Outcome<Seen> {
        let asked = generation.ok_or(NO_GENERATION)?;
        let hunt = self.hunts.find_hunt(id)?.ok_or(NO_HUNT)?;
        ensure(hunt.coop_transport == CoopTransport::Ice, NOT_ICE)?;
        ensure(i64::try_from(hunt.generation) == Ok(asked), STALE)?;
        ensure(hunt.state.is_ice_active(), IDLE)?;
        let presence = match hunt.is_hub() {
            true => Some(self.hunts.hub_presence(&hunt, MAXIMUM_HUB_PEERS + 1)?),
            false => None,
        };
        Ok(Seen { hunt, presence })
    }

    async fn config_for(&self, account: AccountId, session: SessionId, generation: Option<i64>) -> Outcome<Json> {
        self.hosting_running()?;
        let config = Arc::clone(&self.grants.config);
        let secret = config.transport_key_secret.clone().ok_or(NO_TRANSPORT)?;
        let seen = self.live_ice_hunt(&session, generation)?;
        let hunt = &seen.hunt;
        let role = rules::resolve_role(hunt, &account, seen.rows()).ok_or(OUTSIDER)?;
        let at = self.now();
        let hourly = match hunt.is_realm_session() {
            true => config.configs_per_hour.max(REALM_CONFIGS_FLOOR),
            false => config.configs_per_hour,
        };
        ensure(self.config_calls.allow(&account.0, hourly, at), TOO_MANY_CONFIGS)?;
        let cap = match hunt.is_hub() {
            true => config.hub_credential_ttl_seconds,
            false => config.credential_ttl_seconds,
        };
        let grant = self
            .grants
            .grant(&account, relay_ttl_seconds(hunt.expires_at, at, cap), at)
            .await;
        let peers = rules::ice_peers_for(hunt, &account, role, &secret, seen.rows());
        let peers = wire::json_array(peers.iter().map(|peer| {
            JsonObject::new()
                .string("accountId", &peer.account.0)
                .string("role", peer.role.as_str())
                .string("transportKey", peer.transport_key.expose())
                .string("peerId", &seen.peer_text(&peer.account))
                .finish()
        }));
        let body = JsonObject::new()
            .string("sessionId", &hunt.wire_session)
            .number("generation", hunt.generation)
            .string("role", role.as_str())
            .string("localAccountId", &account.0)
            .string("localPeerId", &seen.peer_text(&account))
            .string("hostAccountId", &hunt.host_account.0)
            .string("hostPeerId", &hunt.host_peer.0.to_string())
            .boolean("forceRelay", config.force_relay)
            .raw("iceServers", &grant.ice_servers_json())
            .raw("relay", &grant.relay_json())
            .raw("peers", &peers);
        Ok(Json(body.finish()))
    }

    fn signal(&self, from: &AccountId, session: &SessionId, body: &Value) -> Outcome<Json> {
        self.hosting_running()?;
        let signal = rules::parse_ice_signal(body).ok_or(BAD_SIGNAL)?;
        let seen = self.live_ice_hunt(session, Some(signal.generation))?;
        let hunt = &seen.hunt;
        ensure(
            rules::signal_route_allowed(hunt, from, &signal.to, seen.rows()),
            WRONG_ROUTE,
        )?;
        let at = self.now();
        let hub_host = hunt.is_hub() && *from == hunt.host_account;
        let hourly = if hub_host {
            HUB_HOST_SIGNALS_PER_HOUR
        } else {
            SIGNALS_PER_HOUR
        };
        let budget_key = [hunt.wire_session.as_str(), &hunt.generation.to_string(), &from.0].join(":");
        ensure(self.signal_calls.allow(&budget_key, hourly, at), TOO_MANY_SIGNALS)?;
        let forwarded = match self.grants.config.force_relay {
            false => Some(signal.data.clone()),
            true => rules::filter_relay_only(signal.kind, &signal.data),
        };
        // A filtered signal still counted against the limit above.
        let Some(data) = forwarded else {
            let reply = JsonObject::new()
                .boolean("delivered", false)
                .string("reason", "FILTERED_RELAY_ONLY");
            return Ok(Json(reply.finish()));
        };
        let signal_id = wire::random_uuid().map_err(|_| RelayError::Random)?;
        let sender_role = rules::resolve_role(hunt, from, seen.rows()).map(|role| role.as_str());
        let payload = JsonObject::new()
            .string("sessionId", &hunt.wire_session)
            .number("generation", hunt.generation)
            .string("signalId", &signal_id)
            .string("fromAccountId", &from.0)
            .optional_string("fromRole", sender_role)
            .string("kind", signal.kind.as_str())
            .string("data", &data);
        self.hunts.enqueue(ControlMessage {
            message_type: "ICE_SIGNAL",
            payload: Json(payload.finish()),
            account: signal.to,
            wire_session: hunt.wire_session.clone(),
            generation: hunt.generation,
            created_at: at,
            expires_at: UnixMillis((at.0 + SIGNAL_LIFETIME_MS).min(hunt.expires_at.0)),
        })?;
        let delivered = JsonObject::new()
            .boolean("delivered", true)
            .string("signalId", &signal_id);
        Ok(Json(delivered.finish()))
    }

    fn coop_transport(&self, admin: &str, body: &Value) -> Outcome<Reply> {
        let Some(change) = CoopTransportChange::read(body) else {
            return Ok(reply(400, ADMIN_VALIDATION.to_owned()));
        };
        let request_id = wire::random_uuid().map_err(|_| RelayError::Random)?;
        let outcome = self.policy.set_coop_transport(
            admin,
            change.transport,
            change.expected_version,
            &change.reason,
            &request_id,
        )?;
        Ok(match outcome {
            CoopTransportOutcome::Updated(Json(policy)) => reply(
                200,
                JsonObject::new()
                    .raw("policy", &policy)
                    .string("requestId", &request_id)
                    .finish(),
            ),
            CoopTransportOutcome::Conflict(Json(policy)) => reply(
                409,
                JsonObject::new()
                    .raw("error", ADMIN_CONFLICT)
                    .raw("policy", &policy)
                    .finish(),
            ),
        })
    }
}

fn reply(status: u16, body: String) -> Reply {
    Reply {
        status,
        body: Json(body),
    }
}

/// The admin switch's body, once it passed every check.
struct CoopTransportChange {
    transport: CoopTransport,
    expected_version: u64,
    reason: String,
}

impl CoopTransportChange {
    /// `None` is the 400 answer.
    fn read(body: &Value) -> Option<Self> {
        let transport = body
            .get("coopTransport")
            .and_then(Value::as_str)
            .and_then(CoopTransport::parse)?;
        let version = wire::js_number_value(body.get("expectedVersion")?);
        if version < 0.0 || !wire::is_safe_integer(version) {
            return None;
        }
        let trimmed = wire::js_trim(body.get("reason").and_then(Value::as_str).unwrap_or_default());
        let (shortest, longest) = REASON_UNITS;
        let reason = first_utf16_units(trimmed, longest);
        (reason.encode_utf16().count() >= shortest).then(|| Self {
            transport,
            // A non-negative safe integer, so the cast is exact.
            expected_version: version as u64,
            reason: reason.to_owned(),
        })
    }
}

/// The longest prefix of at most `limit` UTF-16 units that does not split a character.
fn first_utf16_units(text: &str, limit: usize) -> &str {
    let mut used = 0;
    let end = text
        .char_indices()
        .find(|&(_, c)| {
            used += c.len_utf16();
            used > limit
        })
        .map_or(text.len(), |(at, _)| at);
    &text[..end]
}

impl IceService for Ice {
    fn transport_config(&self, who: AccountId, hunt: SessionId, generation: Option<i64>) -> Answer<'_, Json> {
        Box::pin(self.config_for(who, hunt, generation))
    }

    fn submit_signal(&self, who: AccountId, hunt: SessionId, body: Value) -> Answer<'_, Json> {
        Box::pin(ready(self.signal(&who, &hunt, &body)))
    }

    fn set_coop_transport(&self, admin: String, body: Value) -> Answer<'_, Reply> {
        Box::pin(ready(self.coop_transport(&admin, &body)))
    }
}

#[cfg(test)]
mod tests;
