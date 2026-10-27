//! A realm hub's pure rules (`p2p/realm/realmHubRules.ts`): its state machine and lease clock, the presence
//! transitions both engines apply, who may get an admission ticket, and the capacity decisions. Callers pass `now`.

use rifts_seam::{HubKind, UnixMillis};
use serde_json::Value;

use crate::hunt::{HuntSession, HuntState};

/// `HubPartyKey`: the hub's `activePartyKey` and `partyId`, so a realm has one live hub per kind.
pub fn hub_party_key(realm_id: &str, kind: HubKind) -> String {
    format!("realmhub:{realm_id}:{}", crate::realm::hub_kind_name(kind).to_lowercase())
}

/// `RealmRequestClass`: what a realm member's matchmaking request asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealmRequest {
    Hub(HubKind),
    Hunt,
}

/// `CITY` or a `ReturnToRamsgate` hunt id is Ramsgate, a `TrainingDojo` hunt id is Training, anything else a hunt.
pub fn classify_realm_request(game_mode: Option<&str>, hunt_id: Option<&str>) -> RealmRequest {
    if game_mode == Some("CITY") {
        return RealmRequest::Hub(HubKind::Ramsgate);
    }
    match hunt_id {
        Some(id) if id.contains("ReturnToRamsgate") => RealmRequest::Hub(HubKind::Ramsgate),
        Some(id) if id.contains("TrainingDojo") => RealmRequest::Hub(HubKind::Training),
        _ => RealmRequest::Hunt,
    }
}

/// Players a hub takes when the host does not say (the host included).
pub const DEFAULT_HUB_MAX_PLAYERS: i64 = 16;
/// The runtime's `HubMaxPlayers` ceiling.
pub const MAXIMUM_HUB_MAX_PLAYERS: i64 = 64;
/// A hub session record lives at most this long; the host's capability ends sooner.
pub const HUB_SESSION_LIFETIME_MS: i64 = 7 * 24 * 60 * 60 * 1000;
/// How long a created hub waits for its host's launcher to bootstrap it.
pub const HUB_OFFER_LIFETIME_MS: i64 = 2 * 60 * 1000;

/// `RealmHubManifest` (`p2p/realm/hubManifests.ts`): the Director's persistent hub layout, as constants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HubManifest {
    pub hub_kind: HubKind,
    /// As the client knows it; also the admission ticket's `huntDefinitionId`.
    pub hunt_definition_id: &'static str,
    pub game_mode: &'static str,
    pub map_path: &'static str,
    pub matchmaker_hunt_id: &'static str,
    pub behemoth_path: &'static str,
    pub expected_player_string: &'static str,
    /// A hub produces no hunt evidence and never completes.
    pub guard_completion_mode: &'static str,
}

pub const fn hub_manifest(kind: HubKind) -> HubManifest {
    match kind {
        HubKind::Ramsgate => HubManifest {
            hub_kind: HubKind::Ramsgate,
            hunt_definition_id: "ShatteredIsles_ReturnToRamsgate",
            game_mode: "CITY",
            map_path: "/Game/Maps/ramsgate/ramsgate_01_persistent",
            matchmaker_hunt_id: "NO_MM_HUNTID",
            behemoth_path: "NO_BEHEMOTH",
            expected_player_string: "NO_EXPECTED_PLAYERS",
            guard_completion_mode: "NO_COMPLETION",
        },
        HubKind::Training => HubManifest {
            hub_kind: HubKind::Training,
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

/// A hub host's capability: no EVENTS_WRITE and no COMPLETE_REQUEST, since a hub has neither evidence nor completion.
pub const HUB_CAPABILITY_SCOPES: &[rifts_seam::CapabilityScope] = &[
    rifts_seam::CapabilityScope::HostReady,
    rifts_seam::CapabilityScope::LeaseHeartbeat,
    rifts_seam::CapabilityScope::PeerAdmitted,
    rifts_seam::CapabilityScope::GameServerRuntime,
    rifts_seam::CapabilityScope::Abort,
];

/// The body of `hubs/start`: `{ hubKind, maxPlayers? }` (1 to 64, default 16).
pub fn parse_hub_start_body(body: &Value) -> Result<(HubKind, i64), &'static str> {
    let Value::Object(input) = body else {
        return Err("The request body must be an object.");
    };
    let kind = input
        .get("hubKind")
        .and_then(Value::as_str)
        .and_then(crate::realm::parse_hub_kind)
        .ok_or("hubKind must be RAMSGATE or TRAINING.")?;
    let max_players = match input.get("maxPlayers") {
        None => DEFAULT_HUB_MAX_PLAYERS,
        Some(value) => crate::input::whole_number(value)
            .filter(|n| (1..=MAXIMUM_HUB_MAX_PLAYERS).contains(n))
            .ok_or("maxPlayers must be a whole number from 1 to 64.")?,
    };
    Ok((kind, max_players))
}

/// The body of `hubs/stop`: `{ hubKind }`.
pub fn parse_hub_stop_body(body: &Value) -> Result<HubKind, &'static str> {
    let Value::Object(input) = body else {
        return Err("The request body must be an object.");
    };
    input
        .get("hubKind")
        .and_then(Value::as_str)
        .and_then(crate::realm::parse_hub_kind)
        .ok_or("hubKind must be RAMSGATE or TRAINING.")
}

// ------------------------------------------------------------------ hub state machine

use HuntState::{Admitting, HostOffered, HostReady, Running, Starting, Suspect};

pub const HUB_LIVE_STATES: &[HuntState] = &[HostOffered, Starting, HostReady, Admitting, Running, Suspect];
pub const HUB_TERMINAL_STATES: &[HuntState] = &[HuntState::Completed, HuntState::Aborted];
/// The host's runtime holds a live credential from STARTING on.
pub const HUB_RUNTIME_STATES: &[HuntState] = &[Starting, HostReady, Admitting, Running, Suspect];
/// The host keeps renewing the lease in these.
pub const HUB_LEASED_STATES: &[HuntState] = &[HostReady, Admitting, Running];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

const ALL_HUB_EVENTS: [HubEvent; 9] = [
    HubEvent::HostBootstrap,
    HubEvent::HostReady,
    HubEvent::HostRouteReady,
    HubEvent::HostAdmitted,
    HubEvent::LeaseLapsed,
    HubEvent::LeaseBack,
    HubEvent::SuspectTimeout,
    HubEvent::Stop,
    HubEvent::Abort,
];

/// `HUB_TRANSITIONS`: the states an event applies in, and where it goes (`None`: back to the pre-suspect state).
fn hub_rule(event: HubEvent) -> (&'static [HuntState], Option<HuntState>) {
    match event {
        HubEvent::HostBootstrap => (&[HostOffered], Some(Starting)),
        HubEvent::HostReady => (&[Starting, HostReady], Some(HostReady)),
        HubEvent::HostRouteReady => (&[HostReady], Some(Admitting)),
        HubEvent::HostAdmitted => (&[Admitting], Some(Running)),
        HubEvent::LeaseLapsed => (HUB_LEASED_STATES, Some(Suspect)),
        HubEvent::LeaseBack => (&[Suspect], None),
        HubEvent::SuspectTimeout => (&[Suspect], Some(HuntState::Aborted)),
        HubEvent::Stop => (HUB_LIVE_STATES, Some(HuntState::Completed)),
        HubEvent::Abort => (HUB_LIVE_STATES, Some(HuntState::Aborted)),
    }
}

pub fn next_hub_state(state: HuntState, event: HubEvent, pre_suspect: Option<HuntState>) -> Option<HuntState> {
    let (from, to) = hub_rule(event);
    if !from.contains(&state) {
        return None;
    }
    match to {
        Some(to) => Some(to),
        None => pre_suspect.filter(|state| HUB_LEASED_STATES.contains(state)),
    }
}

pub fn is_hub_transition_allowed(from: HuntState, to: HuntState) -> bool {
    ALL_HUB_EVENTS.into_iter().any(|event| {
        let (sources, target) = hub_rule(event);
        sources.contains(&from)
            && match target {
                Some(target) => target == to,
                None => from == Suspect && HUB_LEASED_STATES.contains(&to),
            }
    })
}

pub fn is_hub_live(state: HuntState) -> bool {
    HUB_LIVE_STATES.contains(&state)
}

/// Exactly RUNNING, or SUSPECT after RUNNING.
pub fn hub_ran(state: HuntState, pre_suspect: Option<HuntState>) -> bool {
    state == Running || (state == Suspect && pre_suspect == Some(Running))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HubTick {
    None,
    Suspect { reason: &'static str },
    Recover,
    End { reason: &'static str },
}

fn lapsed(at: Option<UnixMillis>, now: UnixMillis) -> bool {
    at.is_some_and(|at| at <= now)
}

/// `DecideHubTick`: what the lifecycle monitor does with one live hub. Hunt-only logic never runs for a hub (no
/// clean-end rule, no observation quorum, no recovery); every end is ABORTED.
pub fn decide_hub_tick(hub: &HuntSession, now: UnixMillis, emergency_stop: bool, hosting_enabled: bool) -> HubTick {
    if !is_hub_live(hub.state) {
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
    let startup_timeout = HubTick::End {
        reason: "HUB_STARTUP_TIMEOUT",
    };
    let missed = HubTick::Suspect {
        reason: "HOST_LEASE_MISSED",
    };
    match hub.state {
        HostOffered if lapsed(hub.offer_expires_at, now) => HubTick::End {
            reason: "HUB_OFFER_EXPIRED",
        },
        Starting if lapsed(hub.startup_deadline, now) => startup_timeout,
        // A host that reported ready but never published its own route is still on the startup clock
        HostReady | Admitting if lapsed(hub.startup_deadline, now) => startup_timeout,
        HostReady | Admitting | Running if lapsed(hub.lease_expires_at, now) => missed,
        Suspect => {
            if hub.lease_expires_at.is_some()
                && !lapsed(hub.lease_expires_at, now)
                && next_hub_state(Suspect, HubEvent::LeaseBack, hub.pre_suspect_state).is_some()
            {
                HubTick::Recover
            } else if lapsed(hub.suspect_deadline, now) {
                HubTick::End {
                    reason: "HOST_LEASE_LOST",
                }
            } else {
                HubTick::None
            }
        }
        _ => HubTick::None,
    }
}

/// A lost lease, a dead process or a start that never finished counts against the host.
pub fn hub_end_counts_as_host_failure(reason: &str) -> bool {
    matches!(reason, "HOST_LEASE_LOST" | "HOST_PROCESS_UNHEALTHY" | "HUB_STARTUP_TIMEOUT")
}

// ------------------------------------------------------------------ presence

/// `RealmHubPresenceState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresenceState {
    /// Routed to the hub, no ticket yet (expires).
    Pending,
    /// Admission ticket issued (expires with the ticket).
    Ticketed,
    /// Connected; the host's own row is created this way.
    Admitted,
    Lost,
    /// Went to a hunt (TRAVEL) or left on its own.
    Left,
    /// Kicked, banned, or the realm was released.
    Removed,
    Expired,
}

impl PresenceState {
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
        PRESENCE_ALL_STATES.iter().copied().find(|state| state.as_str() == text)
    }
}

pub const PRESENCE_PRESENT_STATES: &[PresenceState] = &[PresenceState::Admitted];
pub const PRESENCE_LIVE_STATES: &[PresenceState] =
    &[PresenceState::Pending, PresenceState::Ticketed, PresenceState::Admitted];
pub const PRESENCE_EXPIRING_STATES: &[PresenceState] = &[PresenceState::Pending, PresenceState::Ticketed];
pub const PRESENCE_ALL_STATES: &[PresenceState] = &[
    PresenceState::Pending,
    PresenceState::Ticketed,
    PresenceState::Admitted,
    PresenceState::Lost,
    PresenceState::Left,
    PresenceState::Removed,
    PresenceState::Expired,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// `PRESENCE_TRANSITIONS`.
pub fn next_presence_state(from: PresenceState, event: PresenceEvent) -> Option<PresenceState> {
    use PresenceState as S;
    let (sources, to): (&[S], S) = match event {
        PresenceEvent::EntryStarted => (PRESENCE_ALL_STATES, S::Pending),
        PresenceEvent::TicketIssued => (&[S::Pending, S::Ticketed], S::Ticketed),
        PresenceEvent::Admitted => (&[S::Pending, S::Ticketed, S::Admitted], S::Admitted),
        PresenceEvent::Disconnected => (&[S::Admitted], S::Lost),
        PresenceEvent::Travelled => (&[S::Admitted], S::Left),
        PresenceEvent::Expired => (PRESENCE_EXPIRING_STATES, S::Expired),
        PresenceEvent::Removed => (PRESENCE_LIVE_STATES, S::Removed),
        PresenceEvent::SessionEnded => (PRESENCE_LIVE_STATES, S::Lost),
    };
    sources.contains(&from).then_some(to)
}

/// How long a PENDING row waits for the member.
pub const ENTRY_LIFETIME_MS: i64 = 90_000;
/// How long a row stays once it stopped being live.
pub const PRESENCE_PURGE_AFTER_MS: i64 = 7 * 24 * 60 * 60 * 1000;

pub fn entry_expires_at(now: UnixMillis) -> UnixMillis {
    UnixMillis(now.0 + ENTRY_LIFETIME_MS)
}

pub fn presence_purge_at(at: UnixMillis) -> UnixMillis {
    UnixMillis(at.0 + PRESENCE_PURGE_AFTER_MS)
}

/// A waiting row (PENDING or TICKETED) whose time ran out; other states do not expire.
pub fn is_presence_expired(state: PresenceState, expires_at: UnixMillis, now: UnixMillis) -> bool {
    PRESENCE_EXPIRING_STATES.contains(&state) && expires_at <= now
}

/// Connected, or waiting and not expired.
pub fn is_presence_live(state: PresenceState, expires_at: UnixMillis, now: UnixMillis) -> bool {
    PRESENCE_LIVE_STATES.contains(&state) && !is_presence_expired(state, expires_at, now)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryDecision {
    Create,
    /// A waiting row: neither the candidate churns nor a ticket in flight goes stale.
    Reuse,
    Reenter,
}

pub fn decide_entry_request(existing: Option<(PresenceState, UnixMillis)>, now: UnixMillis) -> EntryDecision {
    match existing {
        None => EntryDecision::Create,
        Some((state, expires_at)) if PRESENCE_EXPIRING_STATES.contains(&state) => {
            if is_presence_expired(state, expires_at, now) {
                EntryDecision::Reenter
            } else {
                EntryDecision::Reuse
            }
        }
        Some(_) => EntryDecision::Reenter,
    }
}

// ------------------------------------------------------------------ admission tickets

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

pub struct HubTicketCheck<'a> {
    pub hub: &'a HuntSession,
    /// The generation the request names, when the route has one.
    pub generation: Option<i64>,
    pub account_id: &'a str,
    /// The account's presence row: state, expiry, peer id.
    pub presence: Option<(PresenceState, UnixMillis, &'a str)>,
    pub is_member: bool,
    pub is_banned: bool,
    /// The account's transport link (rifts-admission's): status and peer id.
    pub link: Option<(&'a str, &'a str)>,
    pub now: UnixMillis,
}

/// The hub must be RUNNING; the account needs a live waiting entry, membership without a ban, and a VERIFIED link
/// whose peer id is the one the entry froze.
pub fn hub_ticket_denial(check: &HubTicketCheck<'_>) -> Option<HubTicketDenial> {
    if check.hub.state != Running {
        return Some(HubTicketDenial::HubNotRunning);
    }
    if check.hub.expires_at <= check.now {
        return Some(HubTicketDenial::HubExpired);
    }
    if check.generation.is_some_and(|generation| generation != check.hub.generation) {
        return Some(HubTicketDenial::GenerationMismatch);
    }
    if check.account_id == check.hub.host_account_id {
        return Some(HubTicketDenial::IsHost);
    }
    let Some((state, expires_at, peer_id)) = check.presence.filter(|(s, _, _)| PRESENCE_EXPIRING_STATES.contains(s))
    else {
        return Some(HubTicketDenial::NotExpected);
    };
    if is_presence_expired(state, expires_at, check.now) {
        return Some(HubTicketDenial::EntryExpired);
    }
    if check.is_banned {
        return Some(HubTicketDenial::Banned);
    }
    if !check.is_member {
        return Some(HubTicketDenial::NotMember);
    }
    match check.link {
        Some(("VERIFIED", link_peer)) => {
            (link_peer != peer_id).then_some(HubTicketDenial::PeerIdMismatch)
        }
        _ => Some(HubTicketDenial::PeerLinkMissing),
    }
}

/// The launcher error code and message of a ticket denial.
pub fn hub_ticket_denial_error(reason: HubTicketDenial) -> (&'static str, &'static str) {
    match reason {
        HubTicketDenial::HubNotRunning | HubTicketDenial::HubExpired => {
            ("P2P_SESSION_NOT_READY", "The realm's hub is not accepting players.")
        }
        HubTicketDenial::GenerationMismatch => ("P2P_GENERATION_FENCED", "This request belongs to a stale hub generation."),
        HubTicketDenial::IsHost | HubTicketDenial::NotExpected | HubTicketDenial::EntryExpired => {
            ("P2P_NOT_ROSTERED", "This account has no open entry into the hub.")
        }
        HubTicketDenial::Banned => ("REALM_BANNED", "You cannot join this realm."),
        HubTicketDenial::NotMember => ("REALM_NOT_MEMBER", "You are not a member of this realm."),
        HubTicketDenial::PeerLinkMissing | HubTicketDenial::PeerIdMismatch => (
            "P2P_STEAM_LINK_REQUIRED",
            "The current co-op transport no longer matches this hub entry.",
        ),
    }
}

// ------------------------------------------------------------------ capacity

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RealmCapacityInput {
    pub active_servers: i64,
    pub max_servers: i64,
    pub free_physical_memory_mib: f64,
    pub commit_headroom_mib: f64,
    pub minimum_free_memory_mib: f64,
    pub minimum_commit_headroom_mib: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RealmCapacity {
    Ok,
    /// Goes ahead; the caller logs it. Running out of commit is what ends a server, not free memory.
    OkLowFreeMemory,
    ServerLimit,
    CommitLow,
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

/// A member already present is never refused.
pub fn hub_is_full(present_count: i64, max_peers: i64, already_present: bool) -> bool {
    !already_present && present_count >= max_peers
}

// ------------------------------------------------------------------ realm hosting settings (`p2p/realm/realmHubPolicy.ts`)

/// `MYSTICPARADOX_REALMS_HOSTING=on|true|1`.
pub fn realm_hosting_enabled(settings: &dyn rifts_seam::Settings) -> bool {
    let value = settings.get("MYSTICPARADOX_REALMS_HOSTING").unwrap_or_default().trim();
    ["on", "true", "1"].iter().any(|on| value.eq_ignore_ascii_case(on))
}

fn bounded_integer(value: Option<&str>, fallback: i64, min: i64, max: i64) -> i64 {
    let Some(text) = value.map(str::trim) else {
        return fallback;
    };
    if !(1..=10).contains(&text.len()) || !text.bytes().all(|b| b.is_ascii_digit()) {
        return fallback;
    }
    text.parse().ok().filter(|n| (min..=max).contains(n)).unwrap_or(fallback)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RealmHubPolicy {
    /// How long a hub host's capability lives.
    pub capability_ttl_ms: i64,
    pub minimum_free_memory_mib: f64,
    pub minimum_commit_headroom_mib: f64,
    /// The server limit of a realm whose host never reported one.
    pub default_max_servers: i64,
}

impl RealmHubPolicy {
    pub fn from_settings(settings: &dyn rifts_seam::Settings) -> Self {
        Self {
            capability_ttl_ms: bounded_integer(settings.get("P2P_REALM_HUB_CAPABILITY_TTL_HOURS"), 24, 1, 168) * 3_600_000,
            #[allow(clippy::cast_precision_loss)]
            minimum_free_memory_mib: bounded_integer(
                settings.get("P2P_REALM_MIN_FREE_MEMORY_MIB_PER_SERVER"),
                3072,
                512,
                262_144,
            ) as f64,
            minimum_commit_headroom_mib: 4096.0,
            default_max_servers: 6,
        }
    }
}

/// A realm's host is checked as an official host with the realm's relaxations: no RTT, jitter, loss or cooldown tests,
/// no conflict with its own other servers, and the given memory floors.
pub fn realm_host_ineligibility_reason(
    presence: &crate::hunt::HostPresenceRecord,
    host_policy: &crate::host_policy::EligibilityPolicy,
    minimum_free_memory_mib: f64,
    minimum_commit_headroom_mib: f64,
    guard_required: bool,
    now: UnixMillis,
) -> Option<&'static str> {
    let relaxed = crate::host_policy::EligibilityPolicy {
        maximum_worst_peer_rtt_ms: f64::INFINITY,
        maximum_jitter_ms: f64::INFINITY,
        maximum_packet_loss_percent: 100.0,
        minimum_free_memory_mib,
        minimum_commit_headroom_mib,
        ..host_policy.clone()
    };
    crate::host_policy::host_ineligibility_reason(presence, None, 0, &relaxed, guard_required, now)
}
