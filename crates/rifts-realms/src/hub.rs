// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Realm hub rules: identity and manifests, the hub state machine and its tick, who is in a hub and how a row moves,
//! ticket eligibility, capacity and the realm hosting settings.

use rifts_seam::{CapabilityScope, HubKind, Settings, UnixMillis};
use serde_json::Value;

use crate::host_policy::{EligibilityPolicy, host_ineligibility_reason};
use crate::hunt::{HostPresenceRecord, HuntSession, HuntState};
use crate::input::whole_number;
use crate::time::Later as _;

/// Both the hub session's party id and its active party key, so a realm has one live hub per kind.
pub fn hub_party_key(realm_id: &str, kind: HubKind) -> String {
    let kind = match kind {
        HubKind::Ramsgate => "ramsgate",
        HubKind::Training => "training",
    };
    format!("realmhub:{realm_id}:{kind}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RealmRequest {
    Hub(HubKind),
    Hunt,
}

pub fn classify_realm_request(game_mode: &str, hunt_id: &str) -> RealmRequest {
    if game_mode == "CITY" || hunt_id.contains("ReturnToRamsgate") {
        RealmRequest::Hub(HubKind::Ramsgate)
    } else if hunt_id.contains("TrainingDojo") {
        RealmRequest::Hub(HubKind::Training)
    } else {
        RealmRequest::Hunt
    }
}

/// The host included.
pub const DEFAULT_HUB_MAX_PLAYERS: i64 = 16;
pub const MAXIMUM_HUB_MAX_PLAYERS: i64 = 64;
pub const HUB_SESSION_LIFETIME_MS: i64 = 7 * 24 * 60 * 60 * 1000;
pub const HUB_OFFER_LIFETIME_MS: i64 = 120_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HubManifest {
    pub hub_kind: HubKind,
    pub hunt_definition_id: &'static str,
    pub game_mode: &'static str,
    pub map_path: &'static str,
    pub matchmaker_hunt_id: &'static str,
    pub behemoth_path: &'static str,
    pub expected_player_string: &'static str,
    pub guard_completion_mode: &'static str,
}

pub const fn hub_manifest(kind: HubKind) -> HubManifest {
    match kind {
        HubKind::Ramsgate => HubManifest {
            hub_kind: kind,
            hunt_definition_id: "ShatteredIsles_ReturnToRamsgate",
            game_mode: "CITY",
            map_path: "/Game/Maps/ramsgate/ramsgate_01_persistent",
            matchmaker_hunt_id: "NO_MM_HUNTID",
            behemoth_path: "NO_BEHEMOTH",
            expected_player_string: "NO_EXPECTED_PLAYERS",
            guard_completion_mode: "NO_COMPLETION",
        },
        HubKind::Training => HubManifest {
            hub_kind: kind,
            hunt_definition_id: "ShatteredIsles_TrainingDojo",
            game_mode: "SHARED",
            map_path: "/Game/Maps/islands/dojo/training_dojo_persistent",
            matchmaker_hunt_id: "NO_MM_HUNTID",
            behemoth_path: "NO_BEHEMOTH",
            expected_player_string: "NO_EXPECTED_PLAYERS",
            guard_completion_mode: "NO_COMPLETION",
        },
    }
}

/// No evidence and no completion: a hub has neither.
pub const HUB_CAPABILITY_SCOPES: [CapabilityScope; 5] = [
    CapabilityScope::HostReady,
    CapabilityScope::LeaseHeartbeat,
    CapabilityScope::PeerAdmitted,
    CapabilityScope::GameServerRuntime,
    CapabilityScope::Abort,
];

fn hub_kind_of(body: &Value) -> Result<HubKind, &'static str> {
    if !body.is_object() {
        return Err("The request body must be an object.");
    }
    match body.get("hubKind").and_then(Value::as_str) {
        Some("RAMSGATE") => Ok(HubKind::Ramsgate),
        Some("TRAINING") => Ok(HubKind::Training),
        _ => Err("hubKind must be RAMSGATE or TRAINING."),
    }
}

pub fn parse_hub_start_body(body: &Value) -> Result<(HubKind, i64), &'static str> {
    let kind = hub_kind_of(body)?;
    let max_players = match body.get("maxPlayers") {
        None => DEFAULT_HUB_MAX_PLAYERS,
        Some(value) => whole_number(value)
            .filter(|n| (1..=MAXIMUM_HUB_MAX_PLAYERS).contains(n))
            .ok_or("maxPlayers must be a whole number from 1 to 64.")?,
    };
    Ok((kind, max_players))
}

pub fn parse_hub_stop_body(body: &Value) -> Result<HubKind, &'static str> {
    hub_kind_of(body)
}

use HuntState as H;

pub const HUB_LIVE_STATES: [HuntState; 6] = [
    H::HostOffered,
    H::Starting,
    H::HostReady,
    H::Admitting,
    H::Running,
    H::Suspect,
];
pub const HUB_TERMINAL_STATES: [HuntState; 2] = [H::Completed, H::Aborted];
/// The host's runtime holds a live credential.
pub const HUB_RUNTIME_STATES: [HuntState; 5] = [H::Starting, H::HostReady, H::Admitting, H::Running, H::Suspect];
pub const HUB_LEASED_STATES: [HuntState; 3] = [H::HostReady, H::Admitting, H::Running];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HubEvent {
    HostBootstrap,
    HostReady,
    HostRouteReady,
    HostAdmitted,
    LeaseLapsed,
    LeaseBack,
    SuspectTimeout,
    Stop,
    Abort,
}

pub fn is_hub_live(state: HuntState) -> bool {
    HUB_LIVE_STATES.contains(&state)
}

/// `None` when the event does not apply in `state`.
pub fn next_hub_state(state: HuntState, event: HubEvent, pre_suspect: Option<HuntState>) -> Option<HuntState> {
    match event {
        HubEvent::HostBootstrap => (state == H::HostOffered).then_some(H::Starting),
        HubEvent::HostReady => matches!(state, H::Starting | H::HostReady).then_some(H::HostReady),
        HubEvent::HostRouteReady => (state == H::HostReady).then_some(H::Admitting),
        HubEvent::HostAdmitted => (state == H::Admitting).then_some(H::Running),
        HubEvent::LeaseLapsed => HUB_LEASED_STATES.contains(&state).then_some(H::Suspect),
        HubEvent::LeaseBack => pre_suspect.filter(|pre| state == H::Suspect && HUB_LEASED_STATES.contains(pre)),
        HubEvent::SuspectTimeout => (state == H::Suspect).then_some(H::Aborted),
        HubEvent::Stop => is_hub_live(state).then_some(H::Completed),
        HubEvent::Abort => is_hub_live(state).then_some(H::Aborted),
    }
}

pub fn is_hub_transition_allowed(from: HuntState, to: HuntState) -> bool {
    use HubEvent as E;
    [
        E::HostBootstrap,
        E::HostReady,
        E::HostRouteReady,
        E::HostAdmitted,
        E::LeaseLapsed,
        E::SuspectTimeout,
        E::Stop,
        E::Abort,
    ]
    .into_iter()
    .any(|event| next_hub_state(from, event, None) == Some(to))
        || (from == H::Suspect && HUB_LEASED_STATES.contains(&to))
}

pub fn hub_ran(state: HuntState, pre_suspect: Option<HuntState>) -> bool {
    state == H::Running || (state == H::Suspect && pre_suspect == Some(H::Running))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HubTick {
    None,
    Suspect { reason: &'static str },
    Recover,
    End { reason: &'static str },
}

fn lapsed(deadline: Option<UnixMillis>, now: UnixMillis) -> bool {
    deadline.is_some_and(|at| at <= now)
}

/// Every hub end from the tick is ABORTED: hubs have no clean end, no quorum and no recovery.
pub fn decide_hub_tick(hub: &HuntSession, now: UnixMillis, emergency_stop: bool, hosting_enabled: bool) -> HubTick {
    let state = hub.state;
    if !is_hub_live(state) {
        return HubTick::None;
    }
    if emergency_stop {
        return HubTick::End {
            reason: "OPERATOR_P2P_STOP",
        };
    }
    if !hosting_enabled {
        return HubTick::End {
            reason: "REALM_HOSTING_DISABLED",
        };
    }
    if hub.expires_at <= now {
        return HubTick::End {
            reason: "HUB_SESSION_EXPIRED",
        };
    }
    if state == H::HostOffered && lapsed(hub.offer_expires_at, now) {
        return HubTick::End {
            reason: "HUB_OFFER_EXPIRED",
        };
    }
    // A host that reported ready but never published its route is still on the startup clock.
    if matches!(state, H::Starting | H::HostReady | H::Admitting) && lapsed(hub.startup_deadline, now) {
        return HubTick::End {
            reason: "HUB_STARTUP_TIMEOUT",
        };
    }
    if HUB_LEASED_STATES.contains(&state) && lapsed(hub.lease_expires_at, now) {
        return HubTick::Suspect {
            reason: "HOST_LEASE_MISSED",
        };
    }
    if state == H::Suspect {
        let lease_back = hub.lease_expires_at.is_some_and(|at| at > now)
            && next_hub_state(state, HubEvent::LeaseBack, hub.pre_suspect_state).is_some();
        if lease_back {
            return HubTick::Recover;
        }
        if lapsed(hub.suspect_deadline, now) {
            return HubTick::End {
                reason: "HOST_LEASE_LOST",
            };
        }
    }
    HubTick::None
}

pub fn hub_end_counts_as_host_failure(reason: &str) -> bool {
    matches!(
        reason,
        "HOST_LEASE_LOST" | "HOST_PROCESS_UNHEALTHY" | "HUB_STARTUP_TIMEOUT"
    )
}

/// A member's row in one hub.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PresenceState {
    /// Routed in, no ticket yet; expires.
    Pending,
    /// Ticket issued; expires with it.
    Ticketed,
    /// Connected; the host's own row starts here.
    Admitted,
    Lost,
    /// Travelled to a hunt, or left.
    Left,
    /// Kicked, banned, or the realm was released.
    Removed,
    Expired,
}

impl PresenceState {
    pub const ALL: [PresenceState; 7] = [
        Self::Pending,
        Self::Ticketed,
        Self::Admitted,
        Self::Lost,
        Self::Left,
        Self::Removed,
        Self::Expired,
    ];
    pub const PRESENT: [PresenceState; 1] = [Self::Admitted];
    pub const LIVE: [PresenceState; 3] = [Self::Pending, Self::Ticketed, Self::Admitted];
    pub const EXPIRING: [PresenceState; 2] = [Self::Pending, Self::Ticketed];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::Ticketed => "TICKETED",
            Self::Admitted => "ADMITTED",
            Self::Lost => "LOST",
            Self::Left => "LEFT",
            Self::Removed => "REMOVED",
            Self::Expired => "EXPIRED",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.as_str() == text)
    }

    pub fn is_live_state(self) -> bool {
        Self::LIVE.contains(&self)
    }

    pub fn is_expiring_state(self) -> bool {
        Self::EXPIRING.contains(&self)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PresenceEvent {
    EntryStarted,
    TicketIssued,
    Admitted,
    Disconnected,
    Travelled,
    Expired,
    Removed,
    SessionEnded,
}

/// The presence table: `None` when the event does not apply in `from`.
pub fn next_presence_state(from: PresenceState, event: PresenceEvent) -> Option<PresenceState> {
    use PresenceState as S;
    let applies = match event {
        PresenceEvent::EntryStarted => true,
        PresenceEvent::TicketIssued | PresenceEvent::Expired => from.is_expiring_state(),
        PresenceEvent::Admitted => from.is_live_state(),
        PresenceEvent::Disconnected | PresenceEvent::Travelled => from == S::Admitted,
        PresenceEvent::Removed | PresenceEvent::SessionEnded => from.is_live_state(),
    };
    applies.then_some(match event {
        PresenceEvent::EntryStarted => S::Pending,
        PresenceEvent::TicketIssued => S::Ticketed,
        PresenceEvent::Admitted => S::Admitted,
        PresenceEvent::Disconnected | PresenceEvent::SessionEnded => S::Lost,
        PresenceEvent::Travelled => S::Left,
        PresenceEvent::Expired => S::Expired,
        PresenceEvent::Removed => S::Removed,
    })
}

/// A waiting entry lives this long.
pub const ENTRY_LIFETIME_MS: i64 = 90_000;
/// A row that stopped being live is deleted a week later.
pub const PRESENCE_PURGE_AFTER_MS: i64 = 7 * 24 * 60 * 60 * 1000;

pub fn entry_expires_at(now: UnixMillis) -> UnixMillis {
    now.saturating_add(ENTRY_LIFETIME_MS)
}

pub fn presence_purge_at(at: UnixMillis) -> UnixMillis {
    at.saturating_add(PRESENCE_PURGE_AFTER_MS)
}

/// Only a waiting row expires; a connected one is ended by its link.
pub fn is_presence_expired(state: PresenceState, expires_at: UnixMillis, now: UnixMillis) -> bool {
    state.is_expiring_state() && expires_at <= now
}

pub fn is_presence_live(state: PresenceState, expires_at: UnixMillis, now: UnixMillis) -> bool {
    state.is_live_state() && !is_presence_expired(state, expires_at, now)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryDecision {
    Create,
    /// A waiting row still valid: neither the candidate churns nor a ticket in flight goes stale.
    Reuse,
    Reenter,
}

pub fn decide_entry_request(existing: Option<(PresenceState, UnixMillis)>, now: UnixMillis) -> EntryDecision {
    match existing {
        None => EntryDecision::Create,
        Some((state, expires_at)) if state.is_expiring_state() && expires_at > now => EntryDecision::Reuse,
        Some(_) => EntryDecision::Reenter,
    }
}

/// What a hub ticket request is checked against.
#[derive(Debug, Clone, Copy)]
pub struct HubTicketCheck<'a> {
    pub hub: &'a HuntSession,
    pub generation: Option<i64>,
    pub account_id: &'a str,
    /// (state, expires at, peer id) of the account's row.
    pub presence: Option<(PresenceState, UnixMillis, &'a str)>,
    pub is_member: bool,
    pub is_banned: bool,
    /// (status, peer id) of the account's peer link.
    pub link: Option<(&'a str, &'a str)>,
    pub now: UnixMillis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HubTicketDenial {
    HubNotRunning,
    HubExpired,
    GenerationMismatch,
    IsHost,
    NotExpected,
    EntryExpired,
    Banned,
    NotMember,
    PeerLinkMissing,
    PeerIdMismatch,
}

pub fn hub_ticket_denial(check: &HubTicketCheck<'_>) -> Option<HubTicketDenial> {
    use HubTicketDenial as D;
    let hub = check.hub;
    if hub.state != H::Running {
        return Some(D::HubNotRunning);
    }
    if hub.expires_at <= check.now {
        return Some(D::HubExpired);
    }
    if check.generation.is_some_and(|g| g != hub.generation) {
        return Some(D::GenerationMismatch);
    }
    if hub.host_account_id == check.account_id {
        return Some(D::IsHost);
    }
    let Some((state, expires_at, entry_peer)) = check.presence.filter(|(state, _, _)| state.is_expiring_state()) else {
        return Some(D::NotExpected);
    };
    if is_presence_expired(state, expires_at, check.now) {
        return Some(D::EntryExpired);
    }
    if check.is_banned {
        return Some(D::Banned);
    }
    if !check.is_member {
        return Some(D::NotMember);
    }
    match check.link {
        Some(("VERIFIED", peer)) if peer == entry_peer => None,
        Some(("VERIFIED", _)) => Some(D::PeerIdMismatch),
        _ => Some(D::PeerLinkMissing),
    }
}

pub fn hub_ticket_denial_error(denial: HubTicketDenial) -> (&'static str, &'static str) {
    use HubTicketDenial as D;
    match denial {
        D::HubNotRunning | D::HubExpired => ("P2P_SESSION_NOT_READY", "The realm's hub is not accepting players."),
        D::GenerationMismatch => (
            "P2P_GENERATION_FENCED",
            "This request belongs to a stale hub generation.",
        ),
        D::IsHost | D::NotExpected | D::EntryExpired => {
            ("P2P_NOT_ROSTERED", "This account has no open entry into the hub.")
        }
        D::Banned => ("REALM_BANNED", "You cannot join this realm."),
        D::NotMember => ("REALM_NOT_MEMBER", "You are not a member of this realm."),
        D::PeerLinkMissing | D::PeerIdMismatch => (
            "P2P_STEAM_LINK_REQUIRED",
            "The current co-op transport no longer matches this hub entry.",
        ),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RealmCapacityInput {
    pub active_servers: i64,
    pub max_servers: i64,
    pub free_physical_memory_mib: f64,
    pub commit_headroom_mib: f64,
    pub minimum_free_memory_mib: f64,
    pub minimum_commit_headroom_mib: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RealmCapacity {
    Ok,
    /// Goes ahead; running out of commit, not free memory, is what ends a server.
    OkLowFreeMemory,
    CommitLow,
    ServerLimit,
}

pub fn decide_realm_capacity(input: &RealmCapacityInput) -> RealmCapacity {
    if input.active_servers >= input.max_servers {
        RealmCapacity::ServerLimit
    } else if input.commit_headroom_mib < input.minimum_commit_headroom_mib {
        RealmCapacity::CommitLow
    } else if input.free_physical_memory_mib < input.minimum_free_memory_mib {
        RealmCapacity::OkLowFreeMemory
    } else {
        RealmCapacity::Ok
    }
}

pub fn hub_is_full(present: i64, max_peers: i64, already_present: bool) -> bool {
    !already_present && present >= max_peers
}

pub(crate) const REALMS_HOSTING_SETTING: &str = "RUSTIC_RIFTS_REALMS_HOSTING";

pub fn realm_hosting_enabled(settings: &dyn Settings) -> bool {
    let value = settings.get(REALMS_HOSTING_SETTING).unwrap_or("").trim();
    ["on", "true", "1"].iter().any(|on| value.eq_ignore_ascii_case(on))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RealmHubPolicy {
    pub capability_ttl_ms: i64,
    pub minimum_free_memory_mib: f64,
    pub minimum_commit_headroom_mib: f64,
    pub default_max_servers: i64,
}

impl Default for RealmHubPolicy {
    fn default() -> Self {
        Self::from_settings(&std::collections::BTreeMap::new())
    }
}

/// 1 to 10 ASCII digits within the range, trimmed; anything else is the default.
fn digits_setting(settings: &dyn Settings, name: &str, default: i64, min: i64, max: i64) -> i64 {
    let Some(text) = settings.get(name).map(str::trim) else {
        return default;
    };
    if text.is_empty() || text.len() > 10 || !text.bytes().all(|b| b.is_ascii_digit()) {
        return default;
    }
    text.parse::<i64>()
        .ok()
        .filter(|n| (min..=max).contains(n))
        .unwrap_or(default)
}

impl RealmHubPolicy {
    pub fn from_settings(settings: &dyn Settings) -> Self {
        let hours = digits_setting(settings, "P2P_REALM_HUB_CAPABILITY_TTL_HOURS", 24, 1, 168);
        let free = digits_setting(settings, "P2P_REALM_MIN_FREE_MEMORY_MIB_PER_SERVER", 3072, 512, 262_144);
        Self {
            capability_ttl_ms: hours * 3_600_000,
            minimum_free_memory_mib: free as f64,
            minimum_commit_headroom_mib: 4096.0,
            default_max_servers: 6,
        }
    }
}

/// The host check with a realm's relaxations: the realm host plays on its own PC, so the network bounds go and the
/// capacity check stands in for the memory floors.
pub fn realm_host_ineligibility_reason(
    presence: &HostPresenceRecord,
    policy: &EligibilityPolicy,
    minimum_free_memory_mib: f64,
    minimum_commit_headroom_mib: f64,
    guard_required: bool,
    now: UnixMillis,
) -> Option<&'static str> {
    let relaxed = EligibilityPolicy {
        maximum_worst_peer_rtt_ms: f64::INFINITY,
        maximum_jitter_ms: f64::INFINITY,
        maximum_packet_loss_percent: 100.0,
        minimum_free_memory_mib,
        minimum_commit_headroom_mib,
        ..policy.clone()
    };
    host_ineligibility_reason(presence, None, 0, &relaxed, guard_required, now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use PresenceEvent as E;
    use PresenceState as S;

    #[test]
    fn presence_table() {
        let expected: &[(E, &[S], S)] = &[
            (E::EntryStarted, &S::ALL, S::Pending),
            (E::TicketIssued, &[S::Pending, S::Ticketed], S::Ticketed),
            (E::Admitted, &[S::Pending, S::Ticketed, S::Admitted], S::Admitted),
            (E::Disconnected, &[S::Admitted], S::Lost),
            (E::Travelled, &[S::Admitted], S::Left),
            (E::Expired, &[S::Pending, S::Ticketed], S::Expired),
            (E::Removed, &[S::Pending, S::Ticketed, S::Admitted], S::Removed),
            (E::SessionEnded, &[S::Pending, S::Ticketed, S::Admitted], S::Lost),
        ];
        for (event, from, to) in expected {
            for state in S::ALL {
                let want = from.contains(&state).then_some(*to);
                assert_eq!(next_presence_state(state, *event), want, "{state:?} {event:?}");
            }
        }
        for state in S::ALL {
            assert_eq!(S::parse(state.as_str()), Some(state));
        }
    }

    #[test]
    fn expiry_and_entry_decisions() {
        assert!(is_presence_expired(S::Pending, UnixMillis(10), UnixMillis(10)));
        assert!(!is_presence_expired(S::Pending, UnixMillis(11), UnixMillis(10)));
        assert!(!is_presence_expired(S::Admitted, UnixMillis(0), UnixMillis(10)));
        assert!(is_presence_live(S::Admitted, UnixMillis(0), UnixMillis(10)));
        assert!(!is_presence_live(S::Ticketed, UnixMillis(10), UnixMillis(10)));
        assert!(!is_presence_live(S::Lost, UnixMillis(99), UnixMillis(10)));
        assert_eq!(decide_entry_request(None, UnixMillis(5)), EntryDecision::Create);
        assert_eq!(
            decide_entry_request(Some((S::Pending, UnixMillis(6))), UnixMillis(5)),
            EntryDecision::Reuse
        );
        assert_eq!(
            decide_entry_request(Some((S::Ticketed, UnixMillis(5))), UnixMillis(5)),
            EntryDecision::Reenter
        );
        assert_eq!(
            decide_entry_request(Some((S::Admitted, UnixMillis(99))), UnixMillis(5)),
            EntryDecision::Reenter
        );
        assert_eq!(
            decide_entry_request(Some((S::Left, UnixMillis(99))), UnixMillis(5)),
            EntryDecision::Reenter
        );
        assert_eq!(entry_expires_at(UnixMillis(1)), UnixMillis(90_001));
        assert_eq!(presence_purge_at(UnixMillis(0)), UnixMillis(604_800_000));
    }
}
