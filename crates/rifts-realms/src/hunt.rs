// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! A player-hosted hunt or realm hub session and the records around it: roster, transitions, host capabilities,
//! host presences, offers, reliability, observations and control messages.

use rifts_seam::{CapabilityScope, HubKind, UnixMillis};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::time::{millis, opt_millis};
use crate::wire::wire_text;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HuntState {
    Requested,
    HostSelecting,
    HostOffered,
    Starting,
    HostReady,
    Admitting,
    Running,
    /// A superseded Tier 0 hunt whose server still serves its players.
    Draining,
    Suspect,
    Fenced,
    Aborting,
    Recovering,
    Completed,
    Aborted,
}

wire_text!(impl HuntState { as_str, parse; ALL }
    Requested => "REQUESTED",
    HostSelecting => "HOST_SELECTING",
    HostOffered => "HOST_OFFERED",
    Starting => "STARTING",
    HostReady => "HOST_READY",
    Admitting => "ADMITTING",
    Running => "RUNNING",
    Draining => "DRAINING",
    Suspect => "SUSPECT",
    Fenced => "FENCED",
    Aborting => "ABORTING",
    Recovering => "RECOVERING",
    Completed => "COMPLETED",
    Aborted => "ABORTED",
);

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AdmissionState {
    Pending,
    Issued,
    Admitted,
    Lost,
}

wire_text!(impl AdmissionState { as_str, parse }
    Pending => "PENDING",
    Issued => "ISSUED",
    Admitted => "ADMITTED",
    Lost => "LOST",
);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Participant {
    pub account_id: String,
    /// The transport peer id.
    pub steam_id: String,
    pub candidate_id: String,
    pub reconnect_epoch: i64,
    pub admission_state: AdmissionState,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub local_bridge_instance_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub local_bridge_port: Option<i64>,
    #[serde(with = "opt_millis", skip_serializing_if = "Option::is_none", default)]
    pub local_bridge_ready_at: Option<UnixMillis>,
    #[serde(with = "opt_millis", skip_serializing_if = "Option::is_none", default)]
    pub admitted_at: Option<UnixMillis>,
    #[serde(with = "opt_millis", skip_serializing_if = "Option::is_none", default)]
    pub disconnected_at: Option<UnixMillis>,
    #[serde(with = "opt_millis", skip_serializing_if = "Option::is_none", default)]
    pub rejoin_deadline: Option<UnixMillis>,
    #[serde(with = "opt_millis", skip_serializing_if = "Option::is_none", default)]
    pub returned_to_hub_at: Option<UnixMillis>,
}

impl Participant {
    pub fn pending(account_id: &str, steam_id: &str, candidate_id: &str) -> Self {
        Self {
            admission_state: AdmissionState::Pending,
            reconnect_epoch: 0,
            account_id: account_id.into(),
            candidate_id: candidate_id.into(),
            steam_id: steam_id.into(),
            returned_to_hub_at: None,
            rejoin_deadline: None,
            disconnected_at: None,
            admitted_at: None,
            local_bridge_ready_at: None,
            local_bridge_port: None,
            local_bridge_instance_id: None,
        }
    }
}

wire_text!(fn HubKind { hub_kind_name, parse_hub_kind }
    Ramsgate => "RAMSGATE",
    Training => "TRAINING",
);

/// `hubKind` as its text; an unknown text fails the whole record.
mod hub_kind_text {
    use rifts_seam::HubKind;
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serializer};

    use super::{hub_kind_name, parse_hub_kind};

    // serde's `with` hands the field by reference.
    #[allow(clippy::ref_option)]
    pub fn serialize<S: Serializer>(kind: &Option<HubKind>, out: S) -> Result<S::Ok, S::Error> {
        let Some(kind) = kind else {
            return out.serialize_none();
        };
        out.serialize_str(hub_kind_name(*kind))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(input: D) -> Result<Option<HubKind>, D::Error> {
        let Some(text) = Option::<String>::deserialize(input)? else {
            return Ok(None);
        };
        let kind = parse_hub_kind(&text).ok_or_else(|| D::Error::custom(format!("unknown hubKind {text:?}")))?;
        Ok(Some(kind))
    }
}

/// A session as its `detail` JSON holds it (camelCase, absent optionals omitted); the roster is stored in rows of
/// its own. Field order is the JSON's.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HuntSession {
    pub session_id: String,
    pub generation: i64,
    pub state: HuntState,
    pub state_version: i64,
    pub party_id: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub active_party_key: Option<String>,
    pub party_revision: i64,
    pub hunt_definition_id: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub game_mode: Option<String>,
    pub build: i64,
    pub bridge_protocol: i64,
    pub virtual_port: i64,
    pub host_account_id: String,
    pub host_steam_id: String,
    pub bridge_instance_id: String,
    #[serde(skip)]
    pub roster: Vec<Participant>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub lobby_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub coop_transport: Option<String>,
    #[serde(with = "opt_millis", skip_serializing_if = "Option::is_none", default)]
    pub lease_expires_at: Option<UnixMillis>,
    #[serde(with = "millis")]
    pub created_at: UnixMillis,
    #[serde(with = "millis")]
    pub expires_at: UnixMillis,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub metadata_nonce: Option<String>,
    #[serde(with = "hub_kind_text", skip_serializing_if = "Option::is_none", default)]
    pub hub_kind: Option<HubKind>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub realm_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub host_participates: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub host_local_bridge_port: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub server_udp_port: Option<i64>,
    #[serde(with = "opt_millis", skip_serializing_if = "Option::is_none", default)]
    pub host_local_bridge_ready_at: Option<UnixMillis>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub hub_room_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub max_peers: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub guard_completion_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub host_offer_id: Option<String>,
    #[serde(with = "opt_millis", skip_serializing_if = "Option::is_none", default)]
    pub offer_expires_at: Option<UnixMillis>,
    #[serde(with = "opt_millis", skip_serializing_if = "Option::is_none", default)]
    pub startup_deadline: Option<UnixMillis>,
    #[serde(with = "opt_millis", skip_serializing_if = "Option::is_none", default)]
    pub last_heartbeat_at: Option<UnixMillis>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub last_lease_sequence: Option<i64>,
    #[serde(with = "opt_millis", skip_serializing_if = "Option::is_none", default)]
    pub suspect_since: Option<UnixMillis>,
    #[serde(with = "opt_millis", skip_serializing_if = "Option::is_none", default)]
    pub suspect_deadline: Option<UnixMillis>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub pre_suspect_state: Option<HuntState>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub failure_reason: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tried_host_account_ids: Vec<String>,
    #[serde(with = "opt_millis", skip_serializing_if = "Option::is_none", default)]
    pub terminal_at: Option<UnixMillis>,
    #[serde(with = "opt_millis", skip_serializing_if = "Option::is_none", default)]
    pub recovery_deadline: Option<UnixMillis>,
    #[serde(with = "opt_millis", skip_serializing_if = "Option::is_none", default)]
    pub recovery_retry_at: Option<UnixMillis>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub completion_reason: Option<String>,
    #[serde(with = "opt_millis", skip_serializing_if = "Option::is_none", default)]
    pub drain_deadline: Option<UnixMillis>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cancelled_by_account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub trust_tier: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub guard_encounter_state: Option<String>,
    /// Keys this port does not name, kept so a write never drops one.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl HuntSession {
    pub fn is_realm_hub(&self) -> bool {
        self.hub_kind.is_some()
    }

    pub fn participant(&self, account_id: &str) -> Option<&Participant> {
        self.roster.iter().find(|p| p.account_id == account_id)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TransitionActor {
    System,
    Host,
    Participant,
    Admin,
}

wire_text!(impl TransitionActor { as_str }
    System => "SYSTEM",
    Host => "HOST",
    Participant => "PARTICIPANT",
    Admin => "ADMIN",
);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HuntTransition {
    /// Empty: the repository gives it one.
    pub transition_id: String,
    pub session_id: String,
    pub generation: i64,
    pub from_state: HuntState,
    pub to_state: HuntState,
    pub state_version: i64,
    pub reason: String,
    pub actor_type: String,
    pub actor_account_id: Option<String>,
    pub at: UnixMillis,
}

wire_text!(fn CapabilityScope { scope_name, parse_scope }
    HostReady => "HOST_READY",
    LeaseHeartbeat => "LEASE_HEARTBEAT",
    PeerAdmitted => "PEER_ADMITTED",
    GameServerRuntime => "GAME_SERVER_RUNTIME",
    EventsWrite => "EVENTS_WRITE",
    CompleteRequest => "COMPLETE_REQUEST",
    Abort => "ABORT",
);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostCapabilityRecord {
    pub capability_id: String,
    /// Lower-case hex SHA-256 of the secret; the secret itself is never stored.
    pub secret_digest: String,
    pub session_id: String,
    pub generation: i64,
    pub host_account_id: String,
    pub host_steam_id: String,
    pub bridge_instance_id: String,
    pub scopes: Vec<CapabilityScope>,
    pub issued_at: UnixMillis,
    pub expires_at: UnixMillis,
    pub revoked_at: Option<UnixMillis>,
    pub last_sequence: i64,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactHashes {
    pub game_exe: String,
    pub internal_server: String,
    pub winmm_proxy: String,
    pub steam_bridge: String,
    pub steam_runtime: String,
    pub admission_public_key: String,
}

/// A Launcher Guard session as a host presence stores it.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardRole {
    /// `client` or `host`.
    pub role: String,
    pub guard_session_id: String,
    pub manifest_sequence: i64,
    pub sequence: i64,
    pub risk_score: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostPresenceRecord {
    pub account_id: String,
    pub revision: i64,
    pub consent_to_host: bool,
    #[serde(with = "millis")]
    pub observed_at: UnixMillis,
    #[serde(with = "millis")]
    pub expires_at: UnixMillis,
    #[serde(rename = "freePhysicalMemoryMiB")]
    pub free_physical_memory_mib: f64,
    #[serde(rename = "commitHeadroomMiB")]
    pub commit_headroom_mib: f64,
    pub logical_cpu_count: f64,
    pub cpu_load_percent: f64,
    pub power_saver: bool,
    pub runtime_supported: bool,
    pub steam_runtime_ready: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub steam_id: Option<String>,
    pub backend_rtt_ms: f64,
    pub worst_peer_rtt_ms: f64,
    pub jitter_ms: f64,
    pub packet_loss_percent: f64,
    pub artifacts: ArtifactHashes,
    pub artifacts_verified: bool,
    pub launcher_version: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub guard_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub guard_risk_score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub guard_verified: Option<bool>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub guard_roles: Vec<GuardRole>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub realm_host_sessions: Option<i64>,
}

/// Each part 0-100; lower is better.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoreBreakdown {
    pub latency_risk: f64,
    pub network_risk: f64,
    pub memory_risk: f64,
    pub cpu_risk: f64,
    pub stability_risk: f64,
    pub host_load_risk: f64,
    pub integrity_risk: f64,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum OfferStatus {
    Offered,
    Accepted,
    Declined,
    Expired,
    Cancelled,
}

wire_text!(impl OfferStatus { as_str, parse }
    Offered => "OFFERED",
    Accepted => "ACCEPTED",
    Declined => "DECLINED",
    Expired => "EXPIRED",
    Cancelled => "CANCELLED",
);

#[derive(Clone, Debug, PartialEq)]
pub struct HostOfferRecord {
    pub offer_id: String,
    pub session_id: String,
    pub generation: i64,
    pub account_id: String,
    pub steam_id: String,
    pub rank: i64,
    pub score: f64,
    pub score_breakdown: ScoreBreakdown,
    pub status: OfferStatus,
    pub offered_at: UnixMillis,
    pub expires_at: UnixMillis,
    pub responded_at: Option<UnixMillis>,
    pub response_reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostReliabilityRecord {
    pub account_id: String,
    pub offers_accepted: i64,
    pub offers_declined: i64,
    pub offers_expired: i64,
    pub starts_succeeded: i64,
    pub starts_failed: i64,
    pub clean_completions: i64,
    pub unexpected_failures: i64,
    pub cooldown_until: Option<UnixMillis>,
    pub updated_at: UnixMillis,
}

impl HostReliabilityRecord {
    pub fn new(account_id: &str, at: UnixMillis) -> Self {
        let ReliabilityIncrements {
            offers_accepted,
            offers_declined,
            offers_expired,
            starts_succeeded,
            starts_failed,
            clean_completions,
            unexpected_failures,
        } = ReliabilityIncrements::default();
        Self {
            account_id: account_id.into(),
            updated_at: at,
            cooldown_until: None,
            offers_accepted,
            offers_declined,
            offers_expired,
            starts_succeeded,
            starts_failed,
            clean_completions,
            unexpected_failures,
        }
    }

    /// Adds each counter; the cooldown and time are the caller's.
    pub(crate) fn add(&mut self, more: ReliabilityIncrements) {
        let counters = [
            (&mut self.offers_accepted, more.offers_accepted),
            (&mut self.offers_declined, more.offers_declined),
            (&mut self.offers_expired, more.offers_expired),
            (&mut self.starts_succeeded, more.starts_succeeded),
            (&mut self.starts_failed, more.starts_failed),
            (&mut self.clean_completions, more.clean_completions),
            (&mut self.unexpected_failures, more.unexpected_failures),
        ];
        for (total, extra) in counters {
            *total += extra;
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReliabilityIncrements {
    pub offers_accepted: i64,
    pub offers_declined: i64,
    pub offers_expired: i64,
    pub starts_succeeded: i64,
    pub starts_failed: i64,
    pub clean_completions: i64,
    pub unexpected_failures: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ObservationRecord {
    pub observation_id: String,
    pub session_id: String,
    pub generation: i64,
    pub account_id: String,
    pub steam_id: String,
    pub host_reachable: bool,
    pub local_bridge_alive: bool,
    pub metrics_available: bool,
    pub last_packet_age_ms: Option<i64>,
    /// `DIRECT`, `RELAY`, `CONNECTING` or `UNKNOWN`.
    pub route_kind: String,
    pub ping_ms: Option<i64>,
    pub jitter_ms: Option<i64>,
    pub packet_loss_percent: Option<f64>,
    pub observed_at: UnixMillis,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ControlMessageRecord {
    pub message_id: String,
    pub account_id: String,
    pub sequence: i64,
    pub message_type: String,
    pub session_id: Option<String>,
    pub generation: Option<i64>,
    pub payload: Value,
    pub created_at: UnixMillis,
    pub expires_at: UnixMillis,
    pub acknowledged_at: Option<UnixMillis>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// Deserialized from the detail JSON, so only what the test sets is spelled.
    fn session() -> HuntSession {
        let mut hunt: HuntSession = serde_json::from_value(json!({
            "sessionId": "s", "generation": 1, "state": "RUNNING", "stateVersion": 1, "partyId": "party-1",
            "partyRevision": 0, "huntDefinitionId": "hunt", "build": 647_472, "bridgeProtocol": 1, "virtualPort": 1,
            "hostAccountId": "host", "hostSteamId": "1", "bridgeInstanceId": "bridge", "createdAt": 0,
            "expiresAt": 10, "hubKind": "TRAINING", "preSuspectState": "HOST_READY",
        }))
        .unwrap();
        hunt.roster.push(Participant::pending("host", "1", "c"));
        hunt
    }

    #[test]
    fn detail_json_is_camel_case_in_field_order_without_the_roster() {
        let mut hunt = session();
        assert_eq!(hunt.hub_kind, Some(HubKind::Training));
        hunt.extra.insert("frozenLoadouts".into(), Value::Array(Vec::new()));
        let text = serde_json::to_string(&hunt).unwrap();
        let expected = concat!(
            r#"{"sessionId":"s","generation":1,"state":"RUNNING","stateVersion":1,"partyId":"party-1","#,
            r#""partyRevision":0,"huntDefinitionId":"hunt","build":647472,"bridgeProtocol":1,"virtualPort":1,"#,
            r#""hostAccountId":"host","hostSteamId":"1","bridgeInstanceId":"bridge","createdAt":0,"expiresAt":10,"#,
            r#""hubKind":"TRAINING","preSuspectState":"HOST_READY","frozenLoadouts":[]}"#,
        );
        assert_eq!(text, expected);
        let back: HuntSession = serde_json::from_str(&text).unwrap();
        assert!(back.roster.is_empty());
        hunt.roster.clear();
        assert_eq!(back, hunt);
        assert!(serde_json::from_str::<HuntSession>(&text.replace("TRAINING", "DOJO")).is_err());
    }

    #[test]
    fn texts_round_trip() {
        HuntState::ALL.into_iter().for_each(|state| {
            let text = state.as_str();
            assert_eq!(HuntState::parse(text), Some(state));
            assert_eq!(serde_json::to_value(state).unwrap(), Value::from(text));
        });
        let offers = ["OFFERED", "ACCEPTED", "DECLINED", "EXPIRED", "CANCELLED"];
        assert!(
            offers
                .iter()
                .all(|text| OfferStatus::parse(text).map(OfferStatus::as_str) == Some(*text))
        );
        let scopes = ["HOST_READY", "ABORT", "EVENTS_WRITE"];
        assert!(
            scopes
                .iter()
                .all(|text| parse_scope(text).map(scope_name) == Some(*text))
        );
        assert!(parse_scope("host_ready").is_none());
        assert_eq!(AdmissionState::parse("ISSUED"), Some(AdmissionState::Issued));
    }

    #[test]
    fn host_presence_spells_the_memory_keys() {
        let presence: HostPresenceRecord = serde_json::from_value(json!({
            "accountId": "a", "revision": 1, "consentToHost": true, "observedAt": 1, "expiresAt": 2,
            "freePhysicalMemoryMiB": 1.5, "commitHeadroomMiB": 2.0, "logicalCpuCount": 8.0, "cpuLoadPercent": 0.0,
            "powerSaver": false, "runtimeSupported": true, "steamRuntimeReady": true, "backendRttMs": 0.0,
            "worstPeerRttMs": 0.0, "jitterMs": 0.0, "packetLossPercent": 0.0, "artifacts": ArtifactHashes::default(),
            "artifactsVerified": false, "launcherVersion": "1",
        }))
        .unwrap();
        assert_eq!(presence.free_physical_memory_mib, 1.5);
        assert!(presence.guard_roles.is_empty() && presence.steam_id.is_none());
        let text = serde_json::to_string(&presence).unwrap();
        assert!(
            text.contains(r#""freePhysicalMemoryMiB":1.5,"commitHeadroomMiB":2.0"#),
            "{text}"
        );
        assert!(!text.contains("guardRoles"));
        assert_eq!(serde_json::from_str::<HostPresenceRecord>(&text).unwrap(), presence);
    }
}
