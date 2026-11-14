// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! A player-hosted hunt or realm hub session and the records around it: roster, transitions, host capabilities,
//! host presences, offers, reliability, observations and control messages.

use rustic_rifts_seam::{CapabilityScope, HubKind, UnixMillis};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::time::{millis, opt_millis};
use crate::wire::wire_text;

/// Where a player-hosted session is in its lifecycle; stored as its SCREAMING_SNAKE_CASE name.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HuntState {
    /// Asked for; no host chosen yet.
    Requested,
    /// Choosing a host.
    HostSelecting,
    /// A host was offered the session and has not accepted yet.
    HostOffered,
    /// The host accepted and its runtime is starting.
    Starting,
    /// The host's runtime is ready for players.
    HostReady,
    /// Players are being admitted.
    Admitting,
    /// Under way with players in it.
    Running,
    /// A superseded Tier 0 hunt whose server still serves its players.
    Draining,
    /// The host's lease lapsed; waiting for it to come back.
    Suspect,
    /// The host was fenced out and may change nothing.
    Fenced,
    /// Being aborted.
    Aborting,
    /// Recovering after losing its host.
    Recovering,
    /// Ended normally.
    Completed,
    /// Ended without completing.
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

/// Where a roster member's admission stands.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AdmissionState {
    /// Seated; no ticket issued yet.
    Pending,
    /// An admission ticket was issued.
    Issued,
    /// The host admitted the member.
    Admitted,
    /// The member's connection was lost.
    Lost,
}

wire_text!(impl AdmissionState { as_str, parse }
    Pending => "PENDING",
    Issued => "ISSUED",
    Admitted => "ADMITTED",
    Lost => "LOST",
);

/// One seat in a session's roster.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Participant {
    /// The member's account.
    pub account_id: String,
    /// The transport peer id.
    pub steam_id: String,
    /// The member's matchmaking candidate id.
    pub candidate_id: String,
    /// Bumped each time the member reconnects.
    pub reconnect_epoch: i64,
    /// Where the member's admission stands.
    pub admission_state: AdmissionState,
    /// The member's local bridge instance, once it reported.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub local_bridge_instance_id: Option<String>,
    /// The member's local bridge port, once it reported.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub local_bridge_port: Option<i64>,
    /// When the member's local bridge reported ready.
    #[serde(with = "opt_millis", skip_serializing_if = "Option::is_none", default)]
    pub local_bridge_ready_at: Option<UnixMillis>,
    /// When the host admitted the member.
    #[serde(with = "opt_millis", skip_serializing_if = "Option::is_none", default)]
    pub admitted_at: Option<UnixMillis>,
    /// When the member disconnected.
    #[serde(with = "opt_millis", skip_serializing_if = "Option::is_none", default)]
    pub disconnected_at: Option<UnixMillis>,
    /// When a disconnected member's seat is given up.
    #[serde(with = "opt_millis", skip_serializing_if = "Option::is_none", default)]
    pub rejoin_deadline: Option<UnixMillis>,
    /// When the member went back to the hub, leaving the hunt.
    #[serde(with = "opt_millis", skip_serializing_if = "Option::is_none", default)]
    pub returned_to_hub_at: Option<UnixMillis>,
}

impl Participant {
    /// A member who is seated and not yet issued a ticket.
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
    use rustic_rifts_seam::HubKind;
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serializer};

    use super::{hub_kind_name, parse_hub_kind};

    #[allow(clippy::ref_option, reason = "serde's `with` hands the field by reference")]
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
    /// The session id.
    pub session_id: String,
    /// Bumped each time the session gets a new host.
    pub generation: i64,
    /// Where the session is in its lifecycle.
    pub state: HuntState,
    /// Bumped on every state change; writes compare it.
    pub state_version: i64,
    /// The party the session was made for.
    pub party_id: String,
    /// Set while the session is the party's active one; unique among sessions.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub active_party_key: Option<String>,
    /// The party's revision when the session was made.
    pub party_revision: i64,
    /// The hunt (map) id.
    pub hunt_definition_id: String,
    /// The game mode asked for.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub game_mode: Option<String>,
    /// The game build the session runs.
    pub build: i64,
    /// The bridge protocol version, 1 to 255.
    pub bridge_protocol: i64,
    /// The game's virtual port, 1 to 65535.
    pub virtual_port: i64,
    /// The player hosting the session.
    pub host_account_id: String,
    /// The host's transport peer id.
    pub host_steam_id: String,
    /// The host's bridge instance.
    pub bridge_instance_id: String,
    /// The roster, kept in rows of its own and never in the detail JSON.
    #[serde(skip)]
    pub roster: Vec<Participant>,
    /// The transport lobby; `ICE` once started.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub lobby_id: Option<String>,
    /// The co-op transport frozen when the session was made, such as `ICE`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub coop_transport: Option<String>,
    /// When the host's lease lapses unless renewed.
    #[serde(with = "opt_millis", skip_serializing_if = "Option::is_none", default)]
    pub lease_expires_at: Option<UnixMillis>,
    /// When the session was made.
    #[serde(with = "millis")]
    pub created_at: UnixMillis,
    /// When the session expires.
    #[serde(with = "millis")]
    pub expires_at: UnixMillis,
    /// The random nonce the host's runtime metadata carries.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub metadata_nonce: Option<String>,
    /// The hub's kind; set only for a realm hub.
    #[serde(with = "hub_kind_text", skip_serializing_if = "Option::is_none", default)]
    pub hub_kind: Option<HubKind>,
    /// The realm the session belongs to, if any.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub realm_id: Option<String>,
    /// Whether the host also plays in the session; true when unset.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub host_participates: Option<bool>,
    /// The host's local bridge port, once it reported.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub host_local_bridge_port: Option<i64>,
    /// The game server's UDP port, when set for this session.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub server_udp_port: Option<i64>,
    /// When the host's local bridge reported ready.
    #[serde(with = "opt_millis", skip_serializing_if = "Option::is_none", default)]
    pub host_local_bridge_ready_at: Option<UnixMillis>,
    /// The hub's game room id.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub hub_room_id: Option<String>,
    /// The most players the session takes, 1 to 64; the default when unset.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub max_peers: Option<i64>,
    /// How Guard decides the hunt is complete, such as `FIRST_DEFEAT`; the manifest's when unset.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub guard_completion_mode: Option<String>,
    /// The host offer currently out.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub host_offer_id: Option<String>,
    /// When that offer expires.
    #[serde(with = "opt_millis", skip_serializing_if = "Option::is_none", default)]
    pub offer_expires_at: Option<UnixMillis>,
    /// When the host must be ready by.
    #[serde(with = "opt_millis", skip_serializing_if = "Option::is_none", default)]
    pub startup_deadline: Option<UnixMillis>,
    /// When the host last sent a lease heartbeat.
    #[serde(with = "opt_millis", skip_serializing_if = "Option::is_none", default)]
    pub last_heartbeat_at: Option<UnixMillis>,
    /// The sequence number of the last lease heartbeat.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub last_lease_sequence: Option<i64>,
    /// When the session became suspect.
    #[serde(with = "opt_millis", skip_serializing_if = "Option::is_none", default)]
    pub suspect_since: Option<UnixMillis>,
    /// When a suspect session is aborted unless its host comes back.
    #[serde(with = "opt_millis", skip_serializing_if = "Option::is_none", default)]
    pub suspect_deadline: Option<UnixMillis>,
    /// The state the session was in before it became suspect.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub pre_suspect_state: Option<HuntState>,
    /// Why the session failed, as a code.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub failure_reason: Option<String>,
    /// The hosts already tried, so none is offered the session twice.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub tried_host_account_ids: Vec<String>,
    /// When the session reached a terminal state.
    #[serde(with = "opt_millis", skip_serializing_if = "Option::is_none", default)]
    pub terminal_at: Option<UnixMillis>,
    /// When a recovering session gives up.
    #[serde(with = "opt_millis", skip_serializing_if = "Option::is_none", default)]
    pub recovery_deadline: Option<UnixMillis>,
    /// When a recovering session next tries for a host.
    #[serde(with = "opt_millis", skip_serializing_if = "Option::is_none", default)]
    pub recovery_retry_at: Option<UnixMillis>,
    /// Why the session completed, as a code.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub completion_reason: Option<String>,
    /// When a draining session closes.
    #[serde(with = "opt_millis", skip_serializing_if = "Option::is_none", default)]
    pub drain_deadline: Option<UnixMillis>,
    /// The account that cancelled the session, if one did.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cancelled_by_account_id: Option<String>,
    /// The trust tier the session runs at.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub trust_tier: Option<u8>,
    /// Guard's encounter state for the hunt.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub guard_encounter_state: Option<String>,
    /// Keys this port does not name, kept so a write never drops one.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

impl HuntSession {
    /// Whether the session is a realm hub rather than a hunt.
    pub fn is_realm_hub(&self) -> bool {
        self.hub_kind.is_some()
    }

    /// The roster member with this account, if seated.
    pub fn participant(&self, account_id: &str) -> Option<&Participant> {
        self.roster.iter().find(|p| p.account_id == account_id)
    }
}

/// Who made a state transition.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TransitionActor {
    /// Rifts itself.
    System,
    /// The session's host.
    Host,
    /// A roster member.
    Participant,
    /// An operator.
    Admin,
}

wire_text!(impl TransitionActor { as_str }
    System => "SYSTEM",
    Host => "HOST",
    Participant => "PARTICIPANT",
    Admin => "ADMIN",
);

/// One recorded state change of a session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HuntTransition {
    /// Empty: the repository gives it one.
    pub transition_id: String,
    /// The session.
    pub session_id: String,
    /// The session's generation at the change.
    pub generation: i64,
    /// The state before.
    pub from_state: HuntState,
    /// The state after.
    pub to_state: HuntState,
    /// The session's state version after the change.
    pub state_version: i64,
    /// Why, as a code.
    pub reason: String,
    /// Who made the change, as a [`TransitionActor`] name.
    pub actor_type: String,
    /// The account that made it, if one did.
    pub actor_account_id: Option<String>,
    /// When it happened.
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

/// A capability issued to a host's runtime; only its digest is stored.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostCapabilityRecord {
    /// The capability's id, safe to log.
    pub capability_id: String,
    /// Lower-case hex SHA-256 of the secret; the secret itself is never stored.
    pub secret_digest: String,
    /// The session it is for.
    pub session_id: String,
    /// The session's generation it is for.
    pub generation: i64,
    /// The host it was issued to.
    pub host_account_id: String,
    /// The host's transport peer id.
    pub host_steam_id: String,
    /// The bridge instance it was issued to.
    pub bridge_instance_id: String,
    /// What it lets the runtime call.
    pub scopes: Vec<CapabilityScope>,
    /// When it was issued.
    pub issued_at: UnixMillis,
    /// When it expires.
    pub expires_at: UnixMillis,
    /// When it was revoked, if it was.
    pub revoked_at: Option<UnixMillis>,
    /// The highest request sequence seen from it, so a replay is refused.
    pub last_sequence: i64,
}

/// SHA-256 hashes of the files a host runs, as its launcher reports them.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactHashes {
    /// The game executable.
    pub game_exe: String,
    /// The internal server.
    pub internal_server: String,
    /// The winmm proxy.
    pub winmm_proxy: String,
    /// The Steam bridge.
    pub steam_bridge: String,
    /// The Steam runtime.
    pub steam_runtime: String,
    /// The admission public key.
    pub admission_public_key: String,
    /// The MIT runtime (RT-5): the zero hash from a launcher that reports none, and not written then. Since R21
    /// `internal_server` already pins it (the runtime manifest's hash), so this is redundant (spec-runtime-r21 H-8).
    #[serde(default = "zero_sha256", skip_serializing_if = "is_zero_sha256")]
    pub mit_runtime: String,
}

/// The hash a host reports for a file it does not run, and the one expected for an optional role nobody pinned.
pub const ZERO_SHA256: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn zero_sha256() -> String {
    ZERO_SHA256.to_owned()
}

fn is_zero_sha256(hash: &str) -> bool {
    hash == ZERO_SHA256
}

/// A Launcher Guard session as a host presence stores it.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GuardRole {
    /// `client` or `host`.
    pub role: String,
    /// The Guard session.
    pub guard_session_id: String,
    /// The Guard manifest's sequence the session runs under.
    pub manifest_sequence: i64,
    /// The session's latest report sequence.
    pub sequence: i64,
    /// Guard's risk score for the session.
    pub risk_score: f64,
}

/// A launcher's report that its player can host, renewed while it runs.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostPresenceRecord {
    /// The reporting account.
    pub account_id: String,
    /// Bumped on every report.
    pub revision: i64,
    /// Whether the player agreed to host.
    pub consent_to_host: bool,
    /// When the report was made.
    #[serde(with = "millis")]
    pub observed_at: UnixMillis,
    /// When the report stops counting.
    #[serde(with = "millis")]
    pub expires_at: UnixMillis,
    /// Free physical memory, in MiB.
    #[serde(rename = "freePhysicalMemoryMiB")]
    pub free_physical_memory_mib: f64,
    /// Commit headroom, in MiB.
    #[serde(rename = "commitHeadroomMiB")]
    pub commit_headroom_mib: f64,
    /// Logical CPUs.
    pub logical_cpu_count: f64,
    /// CPU load, 0 to 100.
    pub cpu_load_percent: f64,
    /// Whether the machine is in power-saver mode.
    pub power_saver: bool,
    /// Whether the machine can run the host runtime.
    pub runtime_supported: bool,
    /// Whether the Steam runtime is ready.
    pub steam_runtime_ready: bool,
    /// The player's transport peer id, when known.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub steam_id: Option<String>,
    /// Round trip to the backend, in milliseconds.
    pub backend_rtt_ms: f64,
    /// The worst round trip to a peer, in milliseconds.
    pub worst_peer_rtt_ms: f64,
    /// Network jitter, in milliseconds.
    pub jitter_ms: f64,
    /// Packet loss, 0 to 100.
    pub packet_loss_percent: f64,
    /// The hashes of the files the host would run.
    pub artifacts: ArtifactHashes,
    /// Whether those hashes matched the published ones.
    pub artifacts_verified: bool,
    /// The launcher's version.
    pub launcher_version: String,
    /// The player's Launcher Guard session, if any.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub guard_session_id: Option<String>,
    /// Guard's risk score for that session.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub guard_risk_score: Option<f64>,
    /// Whether Guard verified that session.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub guard_verified: Option<bool>,
    /// The Guard sessions the player runs.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub guard_roles: Vec<GuardRole>,
    /// How many realm hub sessions the player hosts now.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub realm_host_sessions: Option<i64>,
}

/// Each part 0-100; lower is better.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoreBreakdown {
    /// Round trip to the backend and peers.
    pub latency_risk: f64,
    /// Jitter and packet loss.
    pub network_risk: f64,
    /// Memory headroom.
    pub memory_risk: f64,
    /// CPU count and load.
    pub cpu_risk: f64,
    /// The host's reliability history.
    pub stability_risk: f64,
    /// How much the host already hosts.
    pub host_load_risk: f64,
    /// Guard and artifact checks.
    pub integrity_risk: f64,
}

/// Where a host offer stands.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum OfferStatus {
    /// Out, waiting for an answer.
    Offered,
    /// The host accepted.
    Accepted,
    /// The host declined.
    Declined,
    /// No answer came in time.
    Expired,
    /// Withdrawn before an answer.
    Cancelled,
}

wire_text!(impl OfferStatus { as_str, parse }
    Offered => "OFFERED",
    Accepted => "ACCEPTED",
    Declined => "DECLINED",
    Expired => "EXPIRED",
    Cancelled => "CANCELLED",
);

/// An offer to host a session, one per candidate host.
#[derive(Clone, Debug, PartialEq)]
pub struct HostOfferRecord {
    /// The offer's id.
    pub offer_id: String,
    /// The session offered.
    pub session_id: String,
    /// The session's generation it is for.
    pub generation: i64,
    /// The candidate host.
    pub account_id: String,
    /// The candidate's transport peer id.
    pub steam_id: String,
    /// The candidate's place in the ranking, 1 first.
    pub rank: i64,
    /// The candidate's score, 0 to 100; lower is better.
    pub score: f64,
    /// The parts of that score.
    pub score_breakdown: ScoreBreakdown,
    /// Where the offer stands.
    pub status: OfferStatus,
    /// When it was made.
    pub offered_at: UnixMillis,
    /// When it expires unanswered.
    pub expires_at: UnixMillis,
    /// When the host answered, if it did.
    pub responded_at: Option<UnixMillis>,
    /// Why the host declined, if it said.
    pub response_reason: Option<String>,
}

/// One host's track record, which ranks it for later offers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostReliabilityRecord {
    /// The host.
    pub account_id: String,
    /// Offers accepted.
    pub offers_accepted: i64,
    /// Offers declined.
    pub offers_declined: i64,
    /// Offers left to expire.
    pub offers_expired: i64,
    /// Starts that reached ready.
    pub starts_succeeded: i64,
    /// Starts that failed.
    pub starts_failed: i64,
    /// Sessions that ended cleanly.
    pub clean_completions: i64,
    /// Sessions that failed unexpectedly.
    pub unexpected_failures: i64,
    /// Until when the host gets no offers, if it is cooling down.
    pub cooldown_until: Option<UnixMillis>,
    /// When the record last changed.
    pub updated_at: UnixMillis,
}

impl HostReliabilityRecord {
    /// A record with every counter at zero.
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

/// What to add to a [`HostReliabilityRecord`]'s counters.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ReliabilityIncrements {
    /// Offers accepted.
    pub offers_accepted: i64,
    /// Offers declined.
    pub offers_declined: i64,
    /// Offers left to expire.
    pub offers_expired: i64,
    /// Starts that reached ready.
    pub starts_succeeded: i64,
    /// Starts that failed.
    pub starts_failed: i64,
    /// Sessions that ended cleanly.
    pub clean_completions: i64,
    /// Sessions that failed unexpectedly.
    pub unexpected_failures: i64,
}

/// A participant's report of its connection to the host.
#[derive(Clone, Debug, PartialEq)]
pub struct ObservationRecord {
    /// The report's id.
    pub observation_id: String,
    /// The session.
    pub session_id: String,
    /// The session's generation.
    pub generation: i64,
    /// The reporting participant.
    pub account_id: String,
    /// The participant's transport peer id.
    pub steam_id: String,
    /// Whether the host was reachable.
    pub host_reachable: bool,
    /// Whether the participant's local bridge was alive.
    pub local_bridge_alive: bool,
    /// Whether the connection metrics below were available.
    pub metrics_available: bool,
    /// How long since the last packet from the host, in milliseconds.
    pub last_packet_age_ms: Option<i64>,
    /// `DIRECT`, `RELAY`, `CONNECTING` or `UNKNOWN`.
    pub route_kind: String,
    /// Round trip to the host, in milliseconds.
    pub ping_ms: Option<i64>,
    /// Jitter, in milliseconds.
    pub jitter_ms: Option<i64>,
    /// Packet loss, 0 to 100.
    pub packet_loss_percent: Option<f64>,
    /// When it was observed.
    pub observed_at: UnixMillis,
}

/// A message queued for one account's launcher, delivered in sequence order.
#[derive(Clone, Debug, PartialEq)]
pub struct ControlMessageRecord {
    /// The message's id.
    pub message_id: String,
    /// The account it is for.
    pub account_id: String,
    /// Its place in the account's queue.
    pub sequence: i64,
    /// The message type, such as `ICE_SIGNAL`.
    pub message_type: String,
    /// The session it is about, if any.
    pub session_id: Option<String>,
    /// That session's generation.
    pub generation: Option<i64>,
    /// The message body.
    pub payload: Value,
    /// When it was queued.
    pub created_at: UnixMillis,
    /// When it is dropped undelivered.
    pub expires_at: UnixMillis,
    /// When the launcher acknowledged it.
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
