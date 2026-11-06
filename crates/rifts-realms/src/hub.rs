// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Realm hub rules: identity and manifests, the hub state machine and its tick, who is in a hub and how a row moves,
//! ticket eligibility, capacity and the realm hosting settings.

use rifts_seam::{CapabilityScope as Scope, HubKind, Settings, UnixMillis};
use serde_json::Value;

use crate::host_policy::{EligibilityPolicy, host_ineligibility_reason};
use crate::hunt::{HostPresenceRecord, HuntSession, HuntState as H};
use crate::input::whole_number;
use crate::time::Later as _;

/// The host included.
pub const DEFAULT_HUB_MAX_PLAYERS: i64 = 16;
/// The most players a hub may be started for, the host included.
pub const MAXIMUM_HUB_MAX_PLAYERS: i64 = 64;
/// How long a hub session lasts, in milliseconds.
pub const HUB_SESSION_LIFETIME_MS: i64 = WEEK_MS;
/// How long a host has to accept a hub offer, in milliseconds.
pub const HUB_OFFER_LIFETIME_MS: i64 = 120_000;
/// A waiting entry lives this long.
pub const ENTRY_LIFETIME_MS: i64 = 90_000;
/// A row that stopped being live is deleted a week later.
pub const PRESENCE_PURGE_AFTER_MS: i64 = WEEK_MS;

const WEEK_MS: i64 = 604_800_000;

/// Both the hub session's party id and its active party key, so a realm has one live hub per kind.
pub fn hub_party_key(realm_id: &str, kind: HubKind) -> String {
    let suffix = if kind == HubKind::Training {
        "training"
    } else {
        "ramsgate"
    };
    ["realmhub", realm_id, suffix].join(":")
}

/// What a realm player's matchmaking request is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RealmRequest {
    /// One of the realm's hubs.
    Hub(HubKind),
    /// A hunt.
    Hunt,
}

/// Classifies a request: Ramsgate for the `CITY` mode or a return to Ramsgate, training for the dojo, a hunt otherwise.
pub fn classify_realm_request(game_mode: &str, hunt_id: &str) -> RealmRequest {
    let to_ramsgate = game_mode == "CITY" || hunt_id.contains("ReturnToRamsgate");
    match (to_ramsgate, hunt_id.contains("TrainingDojo")) {
        (true, _) => RealmRequest::Hub(HubKind::Ramsgate),
        (false, true) => RealmRequest::Hub(HubKind::Training),
        (false, false) => RealmRequest::Hunt,
    }
}

/// What a host's runtime loads for a hub.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HubManifest {
    /// Which hub.
    pub hub_kind: HubKind,
    /// The hunt definition id the game knows the hub by.
    pub hunt_definition_id: &'static str,
    /// The game mode.
    pub game_mode: &'static str,
    /// The map to load.
    pub map_path: &'static str,
    /// The matchmaker hunt id; empty for a hub.
    pub matchmaker_hunt_id: &'static str,
    /// The behemoth to load; empty for a hub.
    pub behemoth_path: &'static str,
    /// The expected roster; empty for a hub.
    pub expected_player_string: &'static str,
    /// Guard's completion mode; empty for a hub.
    pub guard_completion_mode: &'static str,
}

/// A hub has no matchmaking, behemoth, expected roster or completion; only the map differs.
const fn hub_level(kind: HubKind, definition: &'static str, mode: &'static str, map: &'static str) -> HubManifest {
    HubManifest {
        hub_kind: kind,
        hunt_definition_id: definition,
        game_mode: mode,
        map_path: map,
        matchmaker_hunt_id: "NO_MM_HUNTID",
        behemoth_path: "NO_BEHEMOTH",
        expected_player_string: "NO_EXPECTED_PLAYERS",
        guard_completion_mode: "NO_COMPLETION",
    }
}

/// The manifest for a hub of this kind.
pub const fn hub_manifest(kind: HubKind) -> HubManifest {
    match kind {
        HubKind::Ramsgate => hub_level(
            kind,
            "ShatteredIsles_ReturnToRamsgate",
            "CITY",
            "/Game/Maps/ramsgate/ramsgate_01_persistent",
        ),
        HubKind::Training => hub_level(
            kind,
            "ShatteredIsles_TrainingDojo",
            "SHARED",
            "/Game/Maps/islands/dojo/training_dojo_persistent",
        ),
    }
}

/// No evidence and no completion: a hub has neither.
pub const HUB_CAPABILITY_SCOPES: [Scope; 5] = [
    Scope::HostReady,
    Scope::LeaseHeartbeat,
    Scope::PeerAdmitted,
    Scope::GameServerRuntime,
    Scope::Abort,
];

fn hub_kind_of(body: &Value) -> Result<HubKind, &'static str> {
    let Some(fields) = body.as_object() else {
        return Err("The request body must be an object.");
    };
    let kind = fields.get("hubKind").and_then(Value::as_str).unwrap_or_default();
    [("RAMSGATE", HubKind::Ramsgate), ("TRAINING", HubKind::Training)]
        .into_iter()
        .find_map(|(name, parsed)| (name == kind).then_some(parsed))
        .ok_or("hubKind must be RAMSGATE or TRAINING.")
}

/// The hub kind and player limit from a hub start body; the limit defaults to [`DEFAULT_HUB_MAX_PLAYERS`]. `Err` is the 400 text.
pub fn parse_hub_start_body(body: &Value) -> Result<(HubKind, i64), &'static str> {
    let kind = hub_kind_of(body)?;
    let Some(requested) = body.get("maxPlayers") else {
        return Ok((kind, DEFAULT_HUB_MAX_PLAYERS));
    };
    match whole_number(requested) {
        Some(n) if (1..=MAXIMUM_HUB_MAX_PLAYERS).contains(&n) => Ok((kind, n)),
        _ => Err("maxPlayers must be a whole number from 1 to 64."),
    }
}

/// The hub kind from a hub stop body. `Err` is the 400 text.
pub fn parse_hub_stop_body(body: &Value) -> Result<HubKind, &'static str> {
    hub_kind_of(body)
}

/// States in which a hub counts as live.
pub const HUB_LIVE_STATES: [H; 6] = [
    H::HostOffered,
    H::Starting,
    H::HostReady,
    H::Admitting,
    H::Running,
    H::Suspect,
];
/// States a hub ends in.
pub const HUB_TERMINAL_STATES: [H; 2] = [H::Completed, H::Aborted];
/// The host's runtime holds a live credential.
pub const HUB_RUNTIME_STATES: [H; 5] = [H::Starting, H::HostReady, H::Admitting, H::Running, H::Suspect];
/// States in which the host holds a lease it must renew.
pub const HUB_LEASED_STATES: [H; 3] = [H::HostReady, H::Admitting, H::Running];

/// The hub events, declared once with the list `is_hub_transition_allowed` walks.
macro_rules! hub_events {
    ($( $event:ident ),* $(,)?) => {
        /// Something that happens to a hub session; [`next_hub_state`] says where it moves.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[allow(missing_docs, reason = "the event list takes no attributes; the names say what happened")]
        pub enum HubEvent { $( $event, )* }

        const EVERY_HUB_EVENT: &[HubEvent] = &[$( HubEvent::$event, )*];
    };
}

hub_events! { HostBootstrap, HostReady, HostRouteReady, HostAdmitted, LeaseLapsed, LeaseBack, SuspectTimeout, Stop, Abort }

/// Whether the hub is in one of [`HUB_LIVE_STATES`].
pub fn is_hub_live(state: H) -> bool {
    HUB_LIVE_STATES.contains(&state)
}

fn is_leased(state: &H) -> bool {
    HUB_LEASED_STATES.contains(state)
}

/// `None` when the event does not apply in `state`.
pub fn next_hub_state(state: H, event: HubEvent, pre_suspect: Option<H>) -> Option<H> {
    let (applies, to) = match event {
        HubEvent::HostBootstrap => (state == H::HostOffered, H::Starting),
        HubEvent::HostReady => (state == H::Starting || state == H::HostReady, H::HostReady),
        HubEvent::HostRouteReady => (state == H::HostReady, H::Admitting),
        HubEvent::HostAdmitted => (state == H::Admitting, H::Running),
        HubEvent::LeaseLapsed => (is_leased(&state), H::Suspect),
        // Back to whatever leased state it was suspected from.
        HubEvent::LeaseBack => return pre_suspect.filter(|before| state == H::Suspect && is_leased(before)),
        HubEvent::SuspectTimeout => (state == H::Suspect, H::Aborted),
        HubEvent::Stop => (is_hub_live(state), H::Completed),
        HubEvent::Abort => (is_hub_live(state), H::Aborted),
    };
    applies.then_some(to)
}

/// Whether some hub event, or a lease coming back, moves `from` to `to`.
pub fn is_hub_transition_allowed(from: H, to: H) -> bool {
    // LeaseBack needs the state before suspicion: it is covered here and is a no-op in the walk.
    let recovers = from == H::Suspect && is_leased(&to);
    recovers
        || EVERY_HUB_EVENT
            .iter()
            .any(|&event| next_hub_state(from, event, None) == Some(to))
}

/// Whether the hub was running, before suspicion if it is suspect.
pub fn hub_ran(state: H, pre_suspect: Option<H>) -> bool {
    match state {
        H::Running => true,
        H::Suspect => pre_suspect == Some(H::Running),
        _ => false,
    }
}

/// What a periodic check does to a hub.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HubTick {
    /// Nothing.
    None,
    /// The host's lease lapsed; the hub becomes suspect.
    Suspect {
        /// Why, as a code.
        reason: &'static str,
    },
    /// The lease came back; the hub returns to its state before suspicion.
    Recover,
    /// The hub ends, aborted.
    End {
        /// Why, as a code.
        reason: &'static str,
    },
}

/// Every hub end from the tick is ABORTED: hubs have no clean end, no quorum and no recovery.
pub fn decide_hub_tick(hub: &HuntSession, now: UnixMillis, emergency_stop: bool, hosting_enabled: bool) -> HubTick {
    let state = hub.state;
    if !is_hub_live(state) {
        return HubTick::None;
    }
    let passed = |deadline: Option<UnixMillis>| matches!(deadline, Some(at) if at <= now);
    // A host that reported ready but never published its route is still on the startup clock.
    let starting = matches!(state, H::Starting | H::HostReady | H::Admitting);
    let ends = [
        (emergency_stop, "OPERATOR_P2P_STOP"),
        (!hosting_enabled, "REALM_HOSTING_DISABLED"),
        (hub.expires_at <= now, "HUB_SESSION_EXPIRED"),
        (
            state == H::HostOffered && passed(hub.offer_expires_at),
            "HUB_OFFER_EXPIRED",
        ),
        (starting && passed(hub.startup_deadline), "HUB_STARTUP_TIMEOUT"),
    ];
    if let Some(&(_, reason)) = ends.iter().find(|(hit, _)| *hit) {
        return HubTick::End { reason };
    }
    if is_leased(&state) && passed(hub.lease_expires_at) {
        return HubTick::Suspect {
            reason: "HOST_LEASE_MISSED",
        };
    }
    if state != H::Suspect {
        return HubTick::None;
    }
    let lease_current = matches!(hub.lease_expires_at, Some(at) if at > now);
    if lease_current && next_hub_state(state, HubEvent::LeaseBack, hub.pre_suspect_state).is_some() {
        HubTick::Recover
    } else if passed(hub.suspect_deadline) {
        HubTick::End {
            reason: "HOST_LEASE_LOST",
        }
    } else {
        HubTick::None
    }
}

/// Whether a hub ending for `reason` counts against the host's reliability.
pub fn hub_end_counts_as_host_failure(reason: &str) -> bool {
    ["HOST_LEASE_LOST", "HOST_PROCESS_UNHEALTHY", "HUB_STARTUP_TIMEOUT"].contains(&reason)
}

/// One list gives the presence states, their order in `ALL` and their stored text.
macro_rules! presence_states {
    ($( $(#[$doc:meta])* $variant:ident = $text:literal ),* $(,)?) => {
        /// A member's row in one hub.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum PresenceState { $( $(#[$doc])* $variant, )* }

        impl PresenceState {
            /// Every state, in declaration order.
            pub const ALL: [PresenceState; 7] = [$( Self::$variant, )*];

            /// The state's stored text.
            pub const fn as_str(self) -> &'static str {
                match self { $( Self::$variant => $text, )* }
            }
        }
    };
}

presence_states! {
    /// Routed in, no ticket yet; expires.
    Pending = "PENDING",
    /// Ticket issued; expires with it.
    Ticketed = "TICKETED",
    /// Connected; the host's own row starts here.
    Admitted = "ADMITTED",
    /// Disconnected, or the hub session ended.
    Lost = "LOST",
    /// Travelled to a hunt, or left.
    Left = "LEFT",
    /// Kicked, banned, or the realm was released.
    Removed = "REMOVED",
    /// A waiting row whose time ran out.
    Expired = "EXPIRED",
}

impl PresenceState {
    /// States in which the member is connected.
    pub const PRESENT: [PresenceState; 1] = [Self::Admitted];
    /// States in which the row counts as live.
    pub const LIVE: [PresenceState; 3] = [Self::Pending, Self::Ticketed, Self::Admitted];
    /// States in which the row waits and can expire.
    pub const EXPIRING: [PresenceState; 2] = [Self::Pending, Self::Ticketed];

    /// The state for its stored text.
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|candidate| candidate.as_str() == text)
    }

    /// Whether the state is one of [`PresenceState::LIVE`].
    pub fn is_live_state(self) -> bool {
        Self::LIVE.contains(&self)
    }

    /// Whether the state is one of [`PresenceState::EXPIRING`].
    pub fn is_expiring_state(self) -> bool {
        Self::EXPIRING.contains(&self)
    }
}

/// The presence events and their table: an event applies to the rows in its source set and moves them to its target.
macro_rules! presence_events {
    ($( $event:ident: $sources:ident -> $target:ident ),* $(,)?) => {
        /// Something that happens to a member's hub row; [`next_presence_state`] says where it moves.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[allow(missing_docs, reason = "the event table takes no attributes; the table gives each one's move")]
        pub enum PresenceEvent { $( $event, )* }

        /// The presence table: `None` when the event does not apply in `from`.
        pub fn next_presence_state(from: PresenceState, event: PresenceEvent) -> Option<PresenceState> {
            let (sources, target): (&[PresenceState], PresenceState) = match event {
                $( PresenceEvent::$event => (&PresenceState::$sources, PresenceState::$target), )*
            };
            sources.contains(&from).then_some(target)
        }
    };
}

presence_events! {
    EntryStarted: ALL -> Pending,
    TicketIssued: EXPIRING -> Ticketed,
    Admitted: LIVE -> Admitted,
    Disconnected: PRESENT -> Lost,
    Travelled: PRESENT -> Left,
    Expired: EXPIRING -> Expired,
    Removed: LIVE -> Removed,
    SessionEnded: LIVE -> Lost,
}

/// When a waiting entry made now expires.
pub fn entry_expires_at(now: UnixMillis) -> UnixMillis {
    now.saturating_add(ENTRY_LIFETIME_MS)
}

/// When a row that stopped being live at `at` is purged.
pub fn presence_purge_at(at: UnixMillis) -> UnixMillis {
    at.saturating_add(PRESENCE_PURGE_AFTER_MS)
}

/// Only a waiting row expires; a connected one is ended by its link.
pub fn is_presence_expired(state: PresenceState, expires_at: UnixMillis, now: UnixMillis) -> bool {
    expires_at <= now && state.is_expiring_state()
}

/// Whether the row is live and has not expired.
pub fn is_presence_live(state: PresenceState, expires_at: UnixMillis, now: UnixMillis) -> bool {
    !is_presence_expired(state, expires_at, now) && state.is_live_state()
}

/// What to do with a hub entry request, given the account's current row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryDecision {
    /// No row: make one.
    Create,
    /// A waiting row still valid: neither the candidate churns nor a ticket in flight goes stale.
    Reuse,
    /// A row that is no longer waiting: enter again.
    Reenter,
}

/// What to do with an entry request, given the account's row state and expiry.
pub fn decide_entry_request(existing: Option<(PresenceState, UnixMillis)>, now: UnixMillis) -> EntryDecision {
    let Some((state, expires_at)) = existing else {
        return EntryDecision::Create;
    };
    let waiting = state.is_expiring_state() && expires_at > now;
    if waiting {
        EntryDecision::Reuse
    } else {
        EntryDecision::Reenter
    }
}

/// What a hub ticket request is checked against.
#[derive(Clone, Copy, Debug)]
pub struct HubTicketCheck<'a> {
    /// The hub.
    pub hub: &'a HuntSession,
    /// The generation the request is for, if it named one.
    pub generation: Option<i64>,
    /// The account asking.
    pub account_id: &'a str,
    /// (state, expires at, peer id) of the account's row.
    pub presence: Option<(PresenceState, UnixMillis, &'a str)>,
    /// Whether the account is a member of the hub's realm.
    pub is_member: bool,
    /// Whether the account is banned from it.
    pub is_banned: bool,
    /// (status, peer id) of the account's peer link.
    pub link: Option<(&'a str, &'a str)>,
    /// The time now.
    pub now: UnixMillis,
}

/// Each denial beside the (code, text) it answers with.
macro_rules! ticket_denials {
    ($( $denial:ident => $error:ident ),* $(,)?) => {
        /// Why an account may not have a hub ticket.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        #[allow(missing_docs, reason = "the denial table takes no attributes; the table gives each one's error")]
        pub enum HubTicketDenial { $( $denial, )* }

        /// The code and text a ticket denial answers with.
        pub fn hub_ticket_denial_error(denial: HubTicketDenial) -> (&'static str, &'static str) {
            match denial { $( HubTicketDenial::$denial => $error, )* }
        }
    };
}

type ErrorPair = (&'static str, &'static str);
const NOT_READY: ErrorPair = ("P2P_SESSION_NOT_READY", "The realm's hub is not accepting players.");
const FENCED: ErrorPair = (
    "P2P_GENERATION_FENCED",
    "This request belongs to a stale hub generation.",
);
const NOT_ROSTERED: ErrorPair = ("P2P_NOT_ROSTERED", "This account has no open entry into the hub.");
const BANNED: ErrorPair = ("REALM_BANNED", "You cannot join this realm.");
const NOT_MEMBER: ErrorPair = ("REALM_NOT_MEMBER", "You are not a member of this realm.");
const LINK_REQUIRED: ErrorPair = (
    "P2P_STEAM_LINK_REQUIRED",
    "The current co-op transport no longer matches this hub entry.",
);

ticket_denials! {
    HubNotRunning => NOT_READY, HubExpired => NOT_READY, GenerationMismatch => FENCED, IsHost => NOT_ROSTERED,
    NotExpected => NOT_ROSTERED, EntryExpired => NOT_ROSTERED, Banned => BANNED, NotMember => NOT_MEMBER,
    PeerLinkMissing => LINK_REQUIRED, PeerIdMismatch => LINK_REQUIRED,
}

/// Why the account may not have a hub ticket, or `None` when it may.
pub fn hub_ticket_denial(check: &HubTicketCheck<'_>) -> Option<HubTicketDenial> {
    use HubTicketDenial as D;
    let HubTicketCheck { hub, now, .. } = *check;
    let hub_checks = [
        (hub.state != H::Running, D::HubNotRunning),
        (hub.expires_at <= now, D::HubExpired),
        (
            check.generation.is_some_and(|asked| asked != hub.generation),
            D::GenerationMismatch,
        ),
        (hub.host_account_id == check.account_id, D::IsHost),
    ];
    if let Some(&(_, denial)) = hub_checks.iter().find(|(denied, _)| *denied) {
        return Some(denial);
    }
    // Only a waiting entry can take a ticket.
    let Some((state, expires_at, entry_peer)) = check.presence.filter(|row| row.0.is_expiring_state()) else {
        return Some(D::NotExpected);
    };
    let entry_checks = [
        (is_presence_expired(state, expires_at, now), D::EntryExpired),
        (check.is_banned, D::Banned),
        (!check.is_member, D::NotMember),
    ];
    if let Some(&(_, denial)) = entry_checks.iter().find(|(denied, _)| *denied) {
        return Some(denial);
    }
    let Some(("VERIFIED", link_peer)) = check.link else {
        return Some(D::PeerLinkMissing);
    };
    (link_peer != entry_peer).then_some(D::PeerIdMismatch)
}

/// What a realm host's capacity for another server is checked against.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RealmCapacityInput {
    /// Servers running now.
    pub active_servers: i64,
    /// The realm's server limit.
    pub max_servers: i64,
    /// The host's free physical memory, in MiB.
    pub free_physical_memory_mib: f64,
    /// The host's commit headroom, in MiB.
    pub commit_headroom_mib: f64,
    /// The free memory a server should have, in MiB.
    pub minimum_free_memory_mib: f64,
    /// The commit headroom a server needs, in MiB.
    pub minimum_commit_headroom_mib: f64,
}

/// Whether a realm host can start another server.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RealmCapacity {
    /// It can.
    Ok,
    /// Goes ahead; running out of commit, not free memory, is what ends a server.
    OkLowFreeMemory,
    /// Too little commit headroom.
    CommitLow,
    /// The realm runs its maximum servers already.
    ServerLimit,
}

/// Whether the host can start another server: the server limit, then commit headroom, then free memory.
pub fn decide_realm_capacity(input: &RealmCapacityInput) -> RealmCapacity {
    let checks = [
        (input.active_servers >= input.max_servers, RealmCapacity::ServerLimit),
        (
            input.commit_headroom_mib < input.minimum_commit_headroom_mib,
            RealmCapacity::CommitLow,
        ),
        (
            input.free_physical_memory_mib < input.minimum_free_memory_mib,
            RealmCapacity::OkLowFreeMemory,
        ),
    ];
    checks
        .into_iter()
        .find_map(|(hit, verdict)| hit.then_some(verdict))
        .unwrap_or(RealmCapacity::Ok)
}

/// Whether a hub with `present` players is full for an account not yet present.
pub fn hub_is_full(present: i64, max_peers: i64, already_present: bool) -> bool {
    present >= max_peers && !already_present
}

pub(crate) const REALMS_HOSTING_SETTING: &str = "RUSTIC_RIFTS_REALMS_HOSTING";

/// Whether `RUSTIC_RIFTS_REALMS_HOSTING` turns realm hosting on; off unless `on`, `true` or `1`.
pub fn realm_hosting_enabled(settings: &dyn Settings) -> bool {
    let Some(raw) = settings.get(REALMS_HOSTING_SETTING) else {
        return false;
    };
    let value = raw.trim().to_ascii_lowercase();
    matches!(value.as_str(), "on" | "true" | "1")
}

/// The realm hub settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RealmHubPolicy {
    /// How long a hub host's capability lasts, in milliseconds (`P2P_REALM_HUB_CAPABILITY_TTL_HOURS`).
    pub capability_ttl_ms: i64,
    /// The free memory each server should have, in MiB (`P2P_REALM_MIN_FREE_MEMORY_MIB_PER_SERVER`).
    pub minimum_free_memory_mib: f64,
    /// The commit headroom each server needs, in MiB.
    pub minimum_commit_headroom_mib: f64,
    /// A realm's server limit when its host set none.
    pub default_max_servers: i64,
}

impl Default for RealmHubPolicy {
    fn default() -> Self {
        Self::from_settings(&std::collections::BTreeMap::<String, String>::new())
    }
}

/// 1 to 10 ASCII digits within the range, trimmed; anything else is the default.
fn digits_setting(settings: &dyn Settings, name: &str, default: i64, range: std::ops::RangeInclusive<i64>) -> i64 {
    let text = settings.get(name).unwrap_or_default().trim();
    let digits = (1..=10).contains(&text.len()) && text.chars().all(|c| c.is_ascii_digit());
    match text.parse::<i64>() {
        Ok(n) if digits && range.contains(&n) => n,
        _ => default,
    }
}

impl RealmHubPolicy {
    /// Reads the settings; a missing or out-of-range value takes its default.
    pub fn from_settings(settings: &dyn Settings) -> Self {
        let ttl_hours = digits_setting(settings, "P2P_REALM_HUB_CAPABILITY_TTL_HOURS", 24, 1..=168);
        let free_mib = digits_setting(
            settings,
            "P2P_REALM_MIN_FREE_MEMORY_MIB_PER_SERVER",
            3072,
            512..=262_144,
        );
        Self {
            default_max_servers: 6,
            minimum_commit_headroom_mib: 4096.0,
            minimum_free_memory_mib: free_mib as f64,
            capability_ttl_ms: ttl_hours * 60 * 60 * 1000,
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
    let mut relaxed = policy.clone();
    relaxed.minimum_free_memory_mib = minimum_free_memory_mib;
    relaxed.minimum_commit_headroom_mib = minimum_commit_headroom_mib;
    relaxed.maximum_packet_loss_percent = 100.0;
    relaxed.maximum_worst_peer_rtt_ms = f64::INFINITY;
    relaxed.maximum_jitter_ms = f64::INFINITY;
    host_ineligibility_reason(presence, None, 0, &relaxed, guard_required, now)
}

#[cfg(test)]
mod tests {
    use super::{
        EntryDecision, PresenceEvent, PresenceState, decide_entry_request, entry_expires_at, is_presence_expired,
        is_presence_live, next_presence_state, presence_purge_at,
    };
    use rifts_seam::UnixMillis as At;

    #[test]
    fn presence_table() {
        use PresenceEvent::*;
        use PresenceState::{Admitted as In, Lost as Gone, Pending as Wait, Ticketed as Tick};
        let table: [(PresenceEvent, &[PresenceState], PresenceState); 8] = [
            (EntryStarted, &PresenceState::ALL, Wait),
            (TicketIssued, &[Wait, Tick], Tick),
            (Admitted, &[Wait, Tick, In], In),
            (Disconnected, &[In], Gone),
            (Travelled, &[In], PresenceState::Left),
            (Expired, &[Wait, Tick], PresenceState::Expired),
            (Removed, &[Wait, Tick, In], PresenceState::Removed),
            (SessionEnded, &[Wait, Tick, In], Gone),
        ];
        for (event, sources, target) in table {
            for from in PresenceState::ALL {
                let expected = if sources.contains(&from) { Some(target) } else { None };
                assert_eq!(next_presence_state(from, event), expected, "{from:?} on {event:?}");
            }
        }
        assert!(
            PresenceState::ALL
                .iter()
                .all(|&s| PresenceState::parse(s.as_str()) == Some(s))
        );
    }

    #[test]
    fn expiry_and_entry_decisions() {
        use PresenceState::*;
        let now = At(10);
        assert!(is_presence_expired(Pending, At(10), now));
        assert!(!is_presence_expired(Pending, At(11), now));
        assert!(!is_presence_expired(Admitted, At(0), now));
        assert!(is_presence_live(Admitted, At(0), now));
        assert!(!is_presence_live(Ticketed, At(10), now));
        assert!(!is_presence_live(Lost, At(99), now));

        let cases = [
            (None, EntryDecision::Create),
            (Some((Pending, At(6))), EntryDecision::Reuse),
            (Some((Ticketed, At(5))), EntryDecision::Reenter),
            (Some((Admitted, At(99))), EntryDecision::Reenter),
            (Some((Left, At(99))), EntryDecision::Reenter),
        ];
        for (existing, decision) in cases {
            assert_eq!(decide_entry_request(existing, At(5)), decision, "{existing:?}");
        }
        assert_eq!(
            (entry_expires_at(At(1)), presence_purge_at(At(0))),
            (At(90_001), At(604_800_000))
        );
    }
}
