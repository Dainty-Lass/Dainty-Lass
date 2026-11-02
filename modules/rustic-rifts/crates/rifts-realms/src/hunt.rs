//! Player-hosted sessions' records (`p2p/persistence/contracts/P2PHuntRepository.ts`): hunts and realm hubs, their
//! roster and transitions, host capabilities, host selection and the launcher's control channel. Admission tickets,
//! peer admissions and transport links are rifts-admission's (RF4); the `guard*` fields are rifts-guard's (RF6).

use rifts_seam::{CapabilityScope, HubKind, UnixMillis};
use serde::{Deserialize, Serialize};

/// `P2PHuntState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HuntState {
    Requested,
    HostSelecting,
    HostOffered,
    Starting,
    HostReady,
    Admitting,
    Running,
    /// A superseded Tier 0 hunt whose server still serves the players in it.
    Draining,
    Suspect,
    Fenced,
    Aborting,
    Recovering,
    Completed,
    Aborted,
}

impl HuntState {
    pub const ALL: [Self; 14] = [
        Self::Requested,
        Self::HostSelecting,
        Self::HostOffered,
        Self::Starting,
        Self::HostReady,
        Self::Admitting,
        Self::Running,
        Self::Draining,
        Self::Suspect,
        Self::Fenced,
        Self::Aborting,
        Self::Recovering,
        Self::Completed,
        Self::Aborted,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Requested => "REQUESTED",
            Self::HostSelecting => "HOST_SELECTING",
            Self::HostOffered => "HOST_OFFERED",
            Self::Starting => "STARTING",
            Self::HostReady => "HOST_READY",
            Self::Admitting => "ADMITTING",
            Self::Running => "RUNNING",
            Self::Draining => "DRAINING",
            Self::Suspect => "SUSPECT",
            Self::Fenced => "FENCED",
            Self::Aborting => "ABORTING",
            Self::Recovering => "RECOVERING",
            Self::Completed => "COMPLETED",
            Self::Aborted => "ABORTED",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.as_str() == text)
    }
}

/// `admissionState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AdmissionState {
    Pending,
    Issued,
    Admitted,
    Lost,
}

impl AdmissionState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Issued => "ISSUED",
            Self::Admitted => "ADMITTED",
            Self::Lost => "LOST",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "PENDING" => Some(Self::Pending),
            "ISSUED" => Some(Self::Issued),
            "ADMITTED" => Some(Self::Admitted),
            "LOST" => Some(Self::Lost),
            _ => None,
        }
    }
}

/// One roster entry (`P2PHuntParticipantRecord`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Participant {
    pub account_id: String,
    /// The transport peer id: the account-derived id with ICE.
    pub steam_id: String,
    pub candidate_id: String,
    pub reconnect_epoch: i64,
    pub admission_state: AdmissionState,
    pub local_bridge_instance_id: Option<String>,
    pub local_bridge_port: Option<i64>,
    pub local_bridge_ready_at: Option<UnixMillis>,
    pub admitted_at: Option<UnixMillis>,
    pub disconnected_at: Option<UnixMillis>,
    pub rejoin_deadline: Option<UnixMillis>,
    /// This admitted participant asked for a hub again: it has left the hunt.
    pub returned_to_hub_at: Option<UnixMillis>,
}

impl Participant {
    pub fn pending(account_id: &str, steam_id: &str, candidate_id: &str) -> Self {
        Self {
            account_id: account_id.to_owned(),
            steam_id: steam_id.to_owned(),
            candidate_id: candidate_id.to_owned(),
            reconnect_epoch: 0,
            admission_state: AdmissionState::Pending,
            local_bridge_instance_id: None,
            local_bridge_port: None,
            local_bridge_ready_at: None,
            admitted_at: None,
            disconnected_at: None,
            rejoin_deadline: None,
            returned_to_hub_at: None,
        }
    }
}

/// `UnixMillis` as JSON numbers inside a hunt's `detail`.
pub(crate) mod ms {
    use rifts_seam::UnixMillis;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &UnixMillis, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_i64(value.0)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<UnixMillis, D::Error> {
        i64::deserialize(d).map(UnixMillis)
    }

    pub mod opt {
        use super::*;

        pub fn serialize<S: Serializer>(value: &Option<UnixMillis>, s: S) -> Result<S::Ok, S::Error> {
            match value {
                Some(at) => s.serialize_some(&at.0),
                None => s.serialize_none(),
            }
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<UnixMillis>, D::Error> {
            Ok(Option::<i64>::deserialize(d)?.map(UnixMillis))
        }
    }
}

fn is_none<T>(value: &Option<T>) -> bool {
    value.is_none()
}

/// `P2PHuntSessionRecord`, without the `guard*` fields (rifts-guard's `rifts_hunt_guard`). What no query reads stays in
/// the row's `detail` JSON; `extra` keeps fields this port doesn't name, so a write never drops one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HuntSession {
    /// Node's 36-character UUID; its bits are the session's ULID (`ids`).
    pub session_id: String,
    pub generation: i64,
    pub state: HuntState,
    pub state_version: i64,
    pub party_id: String,
    /// Unique while the session is the party's (or the realm hub kind's) live one.
    #[serde(default, skip_serializing_if = "is_none")]
    pub active_party_key: Option<String>,
    pub party_revision: i64,
    pub hunt_definition_id: String,
    #[serde(default, skip_serializing_if = "is_none")]
    pub game_mode: Option<String>,
    pub build: i64,
    pub bridge_protocol: i64,
    pub virtual_port: i64,
    pub host_account_id: String,
    pub host_steam_id: String,
    pub bridge_instance_id: String,
    #[serde(skip)]
    pub roster: Vec<Participant>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub lobby_id: Option<String>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub coop_transport: Option<String>,
    #[serde(default, with = "ms::opt", skip_serializing_if = "is_none")]
    pub lease_expires_at: Option<UnixMillis>,
    #[serde(with = "ms")]
    pub created_at: UnixMillis,
    #[serde(with = "ms")]
    pub expires_at: UnixMillis,
    #[serde(default, skip_serializing_if = "is_none")]
    pub metadata_nonce: Option<String>,
    /// A realm's Ramsgate or Training server: long-lived, an empty roster, members in `rifts_hub_presence`.
    #[serde(default, with = "hub_kind", skip_serializing_if = "is_none")]
    pub hub_kind: Option<HubKind>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub realm_id: Option<String>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub host_participates: Option<bool>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub host_local_bridge_port: Option<i64>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub server_udp_port: Option<i64>,
    #[serde(default, with = "ms::opt", skip_serializing_if = "is_none")]
    pub host_local_bridge_ready_at: Option<UnixMillis>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub hub_room_id: Option<String>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub max_peers: Option<i64>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub guard_completion_mode: Option<String>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub host_offer_id: Option<String>,
    #[serde(default, with = "ms::opt", skip_serializing_if = "is_none")]
    pub offer_expires_at: Option<UnixMillis>,
    #[serde(default, with = "ms::opt", skip_serializing_if = "is_none")]
    pub startup_deadline: Option<UnixMillis>,
    #[serde(default, with = "ms::opt", skip_serializing_if = "is_none")]
    pub last_heartbeat_at: Option<UnixMillis>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub last_lease_sequence: Option<i64>,
    #[serde(default, with = "ms::opt", skip_serializing_if = "is_none")]
    pub suspect_since: Option<UnixMillis>,
    #[serde(default, with = "ms::opt", skip_serializing_if = "is_none")]
    pub suspect_deadline: Option<UnixMillis>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub pre_suspect_state: Option<HuntState>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub failure_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tried_host_account_ids: Vec<String>,
    #[serde(default, with = "ms::opt", skip_serializing_if = "is_none")]
    pub terminal_at: Option<UnixMillis>,
    /// RECOVERING: when the recovery window closes (isles' recovery writes it).
    #[serde(default, with = "ms::opt", skip_serializing_if = "is_none")]
    pub recovery_deadline: Option<UnixMillis>,
    /// ABORTING: when the recovery is tried again.
    #[serde(default, with = "ms::opt", skip_serializing_if = "is_none")]
    pub recovery_retry_at: Option<UnixMillis>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub completion_reason: Option<String>,
    #[serde(default, with = "ms::opt", skip_serializing_if = "is_none")]
    pub drain_deadline: Option<UnixMillis>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub cancelled_by_account_id: Option<String>,
    /// Frozen at creation; Tier 0 never applies persistent progression.
    #[serde(default, skip_serializing_if = "is_none")]
    pub trust_tier: Option<i64>,
    /// Read from rifts-guard's chain when the lifecycle needs it (`COMPLETION_REQUESTED`...).
    #[serde(default, skip_serializing_if = "is_none")]
    pub guard_encounter_state: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

mod hub_kind {
    use rifts_seam::HubKind;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &Option<HubKind>, s: S) -> Result<S::Ok, S::Error> {
        match value {
            Some(kind) => s.serialize_some(crate::realm::hub_kind_name(*kind)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<HubKind>, D::Error> {
        match Option::<String>::deserialize(d)? {
            None => Ok(None),
            Some(text) => crate::realm::parse_hub_kind(&text)
                .map(Some)
                .ok_or_else(|| serde::de::Error::custom(format!("hub kind {text:?}"))),
        }
    }
}

impl HuntSession {
    /// `sessionKind === "REALM_HUB"`.
    pub fn is_realm_hub(&self) -> bool {
        self.hub_kind.is_some()
    }

    pub fn participant(&self, account_id: &str) -> Option<&Participant> {
        self.roster.iter().find(|entry| entry.account_id == account_id)
    }
}

/// `p2pHuntTransitions.actorType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionActor {
    System,
    Host,
    Participant,
    Admin,
}

impl TransitionActor {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "SYSTEM",
            Self::Host => "HOST",
            Self::Participant => "PARTICIPANT",
            Self::Admin => "ADMIN",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HuntTransition {
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

pub const fn scope_name(scope: CapabilityScope) -> &'static str {
    match scope {
        CapabilityScope::HostReady => "HOST_READY",
        CapabilityScope::LeaseHeartbeat => "LEASE_HEARTBEAT",
        CapabilityScope::PeerAdmitted => "PEER_ADMITTED",
        CapabilityScope::GameServerRuntime => "GAME_SERVER_RUNTIME",
        CapabilityScope::EventsWrite => "EVENTS_WRITE",
        CapabilityScope::CompleteRequest => "COMPLETE_REQUEST",
        CapabilityScope::Abort => "ABORT",
    }
}

pub fn parse_scope(text: &str) -> Option<CapabilityScope> {
    [
        CapabilityScope::HostReady,
        CapabilityScope::LeaseHeartbeat,
        CapabilityScope::PeerAdmitted,
        CapabilityScope::GameServerRuntime,
        CapabilityScope::EventsWrite,
        CapabilityScope::CompleteRequest,
        CapabilityScope::Abort,
    ]
    .into_iter()
    .find(|scope| scope_name(*scope) == text)
}

/// `p2pHostCapabilities`; only the secret's SHA-256 is kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostCapabilityRecord {
    pub capability_id: String,
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

/// `P2PArtifactHashes`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactHashes {
    pub game_exe: String,
    pub internal_server: String,
    pub winmm_proxy: String,
    pub steam_bridge: String,
    pub steam_runtime: String,
    pub admission_public_key: String,
}

/// A verified Launcher Guard session of one role.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardRole {
    pub role: String,
    pub guard_session_id: String,
    pub manifest_sequence: i64,
    pub sequence: i64,
    pub risk_score: f64,
}

/// `P2PHostPresenceRecord`: a PC offering to host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostPresenceRecord {
    pub account_id: String,
    pub revision: i64,
    pub consent_to_host: bool,
    #[serde(with = "ms")]
    pub observed_at: UnixMillis,
    #[serde(with = "ms")]
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
    #[serde(default, skip_serializing_if = "is_none")]
    pub steam_id: Option<String>,
    pub backend_rtt_ms: f64,
    pub worst_peer_rtt_ms: f64,
    pub jitter_ms: f64,
    pub packet_loss_percent: f64,
    pub artifacts: ArtifactHashes,
    pub artifacts_verified: bool,
    pub launcher_version: String,
    #[serde(default, skip_serializing_if = "is_none")]
    pub guard_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub guard_risk_score: Option<f64>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub guard_verified: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guard_roles: Vec<GuardRole>,
    #[serde(default, skip_serializing_if = "is_none")]
    pub realm_host_sessions: Option<i64>,
}

/// `P2PHostScoreBreakdown`: seven risks, each 0 to 100.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfferStatus {
    Offered,
    Accepted,
    Declined,
    Expired,
    Cancelled,
}

impl OfferStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Offered => "OFFERED",
            Self::Accepted => "ACCEPTED",
            Self::Declined => "DECLINED",
            Self::Expired => "EXPIRED",
            Self::Cancelled => "CANCELLED",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "OFFERED" => Some(Self::Offered),
            "ACCEPTED" => Some(Self::Accepted),
            "DECLINED" => Some(Self::Declined),
            "EXPIRED" => Some(Self::Expired),
            "CANCELLED" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
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

/// `p2pHostReliability`: the counters host selection scores stability by.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// A host with no history yet.
    pub fn new(account_id: &str, updated_at: UnixMillis) -> Self {
        Self {
            account_id: account_id.to_owned(),
            offers_accepted: 0,
            offers_declined: 0,
            offers_expired: 0,
            starts_succeeded: 0,
            starts_failed: 0,
            clean_completions: 0,
            unexpected_failures: 0,
            cooldown_until: None,
            updated_at,
        }
    }
}

/// What `incrementHostReliability` adds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReliabilityIncrements {
    pub offers_accepted: i64,
    pub offers_declined: i64,
    pub offers_expired: i64,
    pub starts_succeeded: i64,
    pub starts_failed: i64,
    pub clean_completions: i64,
    pub unexpected_failures: i64,
}

#[derive(Debug, Clone, PartialEq)]
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

/// `p2pControlMessages`: one of Node's 12 `messageType`s (`ICE_SIGNAL`, `HOST_OFFERED`...) for one launcher.
#[derive(Debug, Clone, PartialEq)]
pub struct ControlMessageRecord {
    pub message_id: String,
    pub account_id: String,
    pub sequence: i64,
    pub message_type: String,
    pub session_id: Option<String>,
    pub generation: Option<i64>,
    pub payload: serde_json::Value,
    pub created_at: UnixMillis,
    pub expires_at: UnixMillis,
    pub acknowledged_at: Option<UnixMillis>,
}
