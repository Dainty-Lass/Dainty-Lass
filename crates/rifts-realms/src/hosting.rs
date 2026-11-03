// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The session routes behind `Hosting`: bootstrap, host ready, route ready, abort and end, status, realm hubs and
//! their status, matchmaking, the adapters rifts-admission and rifts-relay call, the realm event sink, and the typed
//! `HuntLifecycle` facade with seam ids.

use std::future::Future;

use rifts_admission::{AdmissionError, AdmissionHunt, AdmissionHunts, JOIN_TICKET_LIFETIME_SECONDS, LinkStatus};
use rifts_admission::{PeerAdmissionRecord, PeerLinkRecord, SessionUuid, TicketAuthority, TicketBindings};
use rifts_admission::{TicketRecord, TicketStatus};
use rifts_relay::rules::{CoopTransport, HuntSnapshot, PresenceRow, RosterEntry, SessionKind};
use rifts_relay::{ControlMessage, CoopTransportOutcome, IceHunts, OperationsPolicy, P2PGate, RelayError};
use rifts_seam::{AccountId, BoxFuture, Candidate, CapabilityScope as Scope, HubKind, HuntRouteContext};
use rifts_seam::{HuntRouteOutcome, Json, PeerId, Refusal, RuntimeAuth, Secret, SessionId, StoreError, UnixMillis};
use serde_json::{Map, Value, json};

use crate::RealmsError;
use crate::hub::{DEFAULT_HUB_MAX_PLAYERS, HUB_CAPABILITY_SCOPES, HUB_SESSION_LIFETIME_MS, HubTicketCheck};
use crate::hub::{PresenceState, RealmCapacity, RealmCapacityInput, RealmRequest, classify_realm_request};
use crate::hub::{decide_realm_capacity, entry_expires_at, hub_manifest, hub_party_key, hub_ticket_denial};
use crate::hub::{hub_ticket_denial_error, is_hub_live, is_presence_expired, parse_hub_start_body};
use crate::hub::{parse_hub_stop_body, presence_purge_at, realm_host_ineligibility_reason};
use crate::hunt::hub_kind_name;
use crate::hunt::{AdmissionState, HostCapabilityRecord, HuntSession, HuntState, Participant, TransitionActor};
use crate::ids::{hex, new_session_ulid, random_uuid, seam_session_id, uuid_text, wire_session_id};
use crate::leases::{Hosting, as_account, integer_in, refused, short_text};
use crate::lifecycle::{CONTROL_LIFE_MS, HOST_END_STATES, one, queue, transition};
use crate::ports::RealmEventSink;
use crate::realm::{HubState, RealmEvent, hub_room_id, js_trim};
use crate::repo::hub::{EntryAt, HubEntryBatch, HubEntryRequest};
use crate::repo::hunts::{HostReadyMark, HostStartup, HuntTx, NewControlMessage};
use crate::selection::derive_ice_peer_id;
use crate::tiers::{HuntRoute, RouteInput, decide_hunt_route};
use crate::time::{Later as _, iso};

use HuntState as H;

type Fut<'a, T> = BoxFuture<'a, Result<T, RealmsError>>;

pub const DEFAULT_GAME_BUILD: i64 = 647_472;
/// A disconnected participant may come back this long.
const REJOIN_WINDOW_MS: i64 = 180_000;
const MAX_RECONNECTS: i64 = 5;
const HUNT_LIFETIME_MS: i64 = 7_200_000;
const HUNT_MAX_PEERS: i64 = 4;
const DEFAULT_SERVER_UDP_PORT: u16 = 8791;
/// What a session without its own nonce reports (kept as found).
const PLACEHOLDER_NONCE: &str = "0123456789abcdef0123456789abcdef";
const ROUTE_ACTOR: rifts_seam::Actor = rifts_seam::Actor::System("realms-route");
const DIRECTORY_ACTOR: rifts_seam::Actor = rifts_seam::Actor::System("realms-directory");
/// Links the server derives itself are recorded under this method.
const DERIVED: &str = "ICE_ACCOUNT_DERIVED";

/// A hunt host's capability, in this order.
pub const HUNT_CAPABILITY_SCOPES: [Scope; 7] = [
    Scope::HostReady,
    Scope::LeaseHeartbeat,
    Scope::PeerAdmitted,
    Scope::EventsWrite,
    Scope::CompleteRequest,
    Scope::GameServerRuntime,
    Scope::Abort,
];

/// A refusal's (code, text).
type Says = (&'static str, &'static str);
const NO_SESSION: Says = ("P2P_SESSION_NOT_FOUND", "Hunt session not found.");
const FENCED: &str = "P2P_GENERATION_FENCED";
const NOT_AWAITING_READY: Says = (FENCED, "The hunt generation is not waiting for host readiness.");
const HUNTS_STOPPED: Says = (
    "P2P_EMERGENCY_STOP",
    "Player-hosted hunts are stopped by operations policy.",
);
const NOT_THE_HOST: Says = ("P2P_NOT_ROSTERED", "Only the selected host may bootstrap this session.");
const OPERATIONS_STOPPED: Says = (
    "P2P_EMERGENCY_STOP",
    "Player-hosted hunt operations are temporarily stopped.",
);
const BAD_LOCAL_PORT: Says = ("P2P_TICKET_INVALID", "localBridgePort must be between 1 and 65535.");
const NO_HUB_ENTRY: Says = (
    "P2P_NOT_ROSTERED",
    "This account has no active entry into the realm hub.",
);
const HOSTING_OFF: Says = ("REALM_HOSTING_DISABLED", "Realm hosting is not enabled on this server.");
const HOSTED_STOPPED: Says = (
    "P2P_EMERGENCY_STOP",
    "Player-hosted operations are temporarily stopped.",
);
const NO_HUB_RUNTIME: Says = ("REALM_HOSTING_DISABLED", "Realm hosting runtime is not configured.");
const SERVER_LIMIT: Says = ("REALM_SERVER_LIMIT", "Your realm has reached its active server limit.");
const HUB_NOT_READY: Says = ("P2P_SESSION_NOT_READY", "The realm's hub is not accepting players.");

fn no(says: Says) -> RealmsError {
    refused(says.0, says.1)
}

/// What the host's launcher starts a hosted server from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostBootstrap {
    pub session: SessionId,
    pub generation: u64,
    pub capability: Secret,
    pub host_ticket: Option<Secret>,
    pub backend_url: String,
    pub server_udp_port: u16,
    pub maximum_peers: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HuntStatus {
    pub state: HuntState,
    pub generation: u64,
    pub verdict: Option<Json>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Observation {
    pub host_reachable: bool,
    pub local_bridge_alive: bool,
    pub route_kind: Option<String>,
    pub ping_ms: Option<u32>,
    pub jitter_ms: Option<u32>,
    pub packet_loss_percent: Option<u8>,
    pub last_packet_age_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StartHub {
    pub kind: HubKind,
    pub max_players: u8,
    pub afk_seconds: u32,
}

/// The session routes as typed calls with seam ids.
pub trait HuntLifecycle: Send + Sync {
    fn host_bootstrap(&self, account: AccountId, session: SessionId) -> Fut<'_, HostBootstrap>;
    fn route_ready(&self, account: AccountId, session: SessionId, local_port: u16) -> Fut<'_, ()>;
    fn host_ready(&self, runtime: RuntimeAuth) -> Fut<'_, ()>;
    fn abort(&self, runtime: RuntimeAuth, reason: String) -> Fut<'_, ()>;
    fn end(&self, runtime: RuntimeAuth, reason: String) -> Fut<'_, ()>;
    fn status(&self, account: AccountId, session: SessionId) -> Fut<'_, HuntStatus>;
    fn observe(&self, account: AccountId, session: SessionId, observation: Observation) -> Fut<'_, ()>;
    fn start_hub(&self, account: AccountId, hub: StartHub) -> Fut<'_, HostBootstrap>;
    fn stop_hub(&self, account: AccountId, kind: HubKind) -> Fut<'_, ()>;
    fn hosting_status(&self, account: AccountId) -> Fut<'_, Json>;
    fn set_emergency_stop(&self, admin_key: String, policy: Json) -> Fut<'_, Json>;
    fn resolve_launch_manifest(&self, request: Json) -> Fut<'_, Json>;
}

/// A JSON object with its keys in the order given.
fn object<const N: usize>(pairs: [(&str, Value); N]) -> Value {
    Value::Object(pairs.into_iter().map(|(key, value)| (key.to_owned(), value)).collect())
}

/// A hub's manifest, or the fixed placeholder every other hunt gets (kept as found).
pub fn resolve_manifest_for_hunt(hunt_definition_id: &str, expected_players: &[String]) -> Value {
    let to_ramsgate =
        hunt_definition_id.contains("ReturnToRamsgate") || hunt_definition_id.eq_ignore_ascii_case("CITY");
    let hub = match (to_ramsgate, hunt_definition_id.contains("TrainingDojo")) {
        (true, _) => Some(HubKind::Ramsgate),
        (false, true) => Some(HubKind::Training),
        (false, false) => None,
    };
    let (id, mode, map, behemoth, matchmaker, players, completion) = match hub.map(hub_manifest) {
        Some(m) => (
            m.hunt_definition_id.to_owned(),
            m.game_mode,
            m.map_path,
            m.behemoth_path,
            m.matchmaker_hunt_id.to_owned(),
            m.expected_player_string.to_owned(),
            m.guard_completion_mode,
        ),
        None => {
            let players = if expected_players.is_empty() {
                String::from("1")
            } else {
                expected_players.join(",")
            };
            let id = hunt_definition_id.to_owned();
            (
                id.clone(),
                "ISLAND",
                "/Game/Maps/islands/1705/dia_moss_triforce",
                "Behemoth_Gnasher",
                id,
                players,
                "FIRST_DEFEAT",
            )
        }
    };
    object([
        ("huntDefinitionId", id.into()),
        ("gameMode", mode.into()),
        ("mapPath", map.into()),
        ("behemothPath", behemoth.into()),
        ("matchmakerHuntId", matchmaker.into()),
        ("expectedPlayerString", players.into()),
        ("guardCompletionMode", completion.into()),
    ])
}

/// The stored peer id when it is a positive u64, else the account's derived one, else 1.
fn peer_number(stored: &str, account: &str) -> u64 {
    match stored.parse::<u64>() {
        Ok(peer) if peer > 0 => peer,
        _ => derive_ice_peer_id(account).parse().unwrap_or(1),
    }
}

fn host_peer_text(hunt: &HuntSession) -> String {
    match hunt.host_steam_id.as_str() {
        "" => derive_ice_peer_id(&hunt.host_account_id),
        stored => stored.to_owned(),
    }
}

/// The party id as tickets carry it.
fn party_text(party_id: &str) -> String {
    party_id.replace('@', ":")
}

fn bounded_u16(value: i64, min: i64, max: i64) -> u16 {
    u16::try_from(value.clamp(min, max)).unwrap_or(1)
}

/// At least 1, as the ticket fields want it.
fn positive_u64(value: i64) -> u64 {
    u64::try_from(value.max(1)).unwrap_or(1)
}

fn build_number(hunt: &HuntSession) -> u32 {
    u32::try_from(hunt.build.max(1)).unwrap_or(647_472)
}

fn random_nonce() -> String {
    hex(&rand::random::<[u8; 16]>())
}

/// Maps between two enums whose variants carry the same names.
macro_rules! same_variant {
    ($value:expr, $from:ident => $to:ident { $($variant:ident),* $(,)? }) => {
        match $value { $( $from::$variant => $to::$variant, )* }
    };
}

fn relay_state(state: HuntState) -> rifts_relay::rules::HuntState {
    use rifts_relay::rules::HuntState as R;
    same_variant!(state, H => R {
        Requested, HostSelecting, HostOffered, Starting, HostReady, Admitting, Running, Draining, Suspect, Fenced,
        Aborting, Recovering, Completed, Aborted,
    })
}

fn relay_presence(state: PresenceState) -> rifts_relay::rules::PresenceState {
    use rifts_relay::rules::PresenceState as R;
    same_variant!(state, PresenceState => R { Pending, Ticketed, Admitted, Lost, Left, Removed, Expired })
}

fn snapshot(hunt: &HuntSession) -> HuntSnapshot {
    let peer_of = |stored: &str, account: &str| PeerId(peer_number(stored, account));
    let roster = hunt.roster.iter().map(|p| RosterEntry {
        returned_to_hub: p.returned_to_hub_at.is_some(),
        peer: peer_of(&p.steam_id, &p.account_id),
        account: AccountId(p.account_id.clone()),
    });
    let transport = hunt.coop_transport.as_deref().and_then(CoopTransport::parse);
    HuntSnapshot {
        kind: if hunt.is_realm_hub() {
            SessionKind::RealmHub
        } else {
            SessionKind::Hunt
        },
        coop_transport: transport.unwrap_or(CoopTransport::Ice),
        host_peer: peer_of(&hunt.host_steam_id, &hunt.host_account_id),
        host_account: AccountId(hunt.host_account_id.clone()),
        generation: positive_u64(hunt.generation),
        state: relay_state(hunt.state),
        wire_session: hunt.session_id.clone(),
        realm: hunt.realm_id.clone(),
        expires_at: hunt.expires_at,
        roster: roster.collect(),
    }
}

fn admission_hunt(hunt: &HuntSession) -> Option<AdmissionHunt> {
    let session = hunt.session_id.parse::<SessionUuid>().ok()?;
    // A started ICE session has no lobby of its own.
    let started = matches!(hunt.state, H::Starting | H::HostReady | H::Admitting | H::Running);
    let lobby_id = hunt.lobby_id.clone().or_else(|| started.then(|| String::from("ICE")));
    let bridge = hunt.bridge_instance_id.as_str();
    Some(AdmissionHunt {
        bridge_instance_id: if bridge.is_empty() {
            None
        } else {
            Some(bridge.to_owned())
        },
        virtual_port: bounded_u16(hunt.virtual_port, 1, 65_535),
        bridge_protocol: bounded_u16(hunt.bridge_protocol, 1, 255),
        build: build_number(hunt),
        hunt_definition_id: hunt.hunt_definition_id.clone(),
        party_revision: positive_u64(hunt.party_revision),
        party_id: party_text(&hunt.party_id),
        snapshot: snapshot(hunt),
        lobby_id,
        session,
    })
}

/// Where an account's game connects, as the matchmaker reads it; a hub's names its room and adds its source.
fn candidate_of(source: &HuntSession, candidate_id: &str, ready: bool, port: i64, hub: bool) -> Candidate {
    let host = if ready { "127.0.0.1" } else { "" };
    let mut fields = Map::new();
    fields.insert("HuntId".into(), json!(source.hunt_definition_id));
    fields.insert("CandidateId".into(), json!(candidate_id));
    let game_session = match (hub, source.hub_room_id.as_ref()) {
        (true, Some(room)) => room,
        _ => &source.session_id,
    };
    fields.insert("GameSessionId".into(), json!(game_session));
    fields.insert("Host".into(), json!(host));
    fields.insert("Port".into(), json!(port));
    fields.insert("Ready".into(), json!(ready));
    fields.insert("ReadyAt".into(), json!(0));
    if hub {
        fields.insert("Source".into(), json!("REALM_HUB"));
        fields.insert("SourceSessionId".into(), json!(source.session_id));
    }
    Candidate(Json(Value::Object(fields).to_string()))
}

fn hunt_candidate(hunt: &HuntSession, candidate_id: &str, ready: bool, port: i64) -> Candidate {
    candidate_of(hunt, candidate_id, ready, port, false)
}

fn hub_candidate(hub: &HuntSession, candidate_id: &str, ready: bool, port: i64) -> Candidate {
    candidate_of(hub, candidate_id, ready, port, true)
}

fn participant_candidate(hunt: &HuntSession, p: &Participant) -> Candidate {
    let port = p.local_bridge_port;
    hunt_candidate(
        hunt,
        &p.candidate_id,
        port.is_some() && p.local_bridge_ready_at.is_some(),
        port.unwrap_or(0),
    )
}

/// A capability record standing in for a credential the route already checked.
fn stand_in_capability(hunt_or_runtime: StandIn<'_>) -> HostCapabilityRecord {
    let mut record = HostCapabilityRecord {
        capability_id: String::new(),
        secret_digest: String::new(),
        session_id: String::new(),
        generation: 0,
        host_account_id: String::new(),
        host_steam_id: String::new(),
        bridge_instance_id: String::new(),
        scopes: Vec::new(),
        issued_at: UnixMillis(0),
        expires_at: UnixMillis(i64::MAX),
        revoked_at: None,
        last_sequence: 0,
    };
    match hunt_or_runtime {
        StandIn::Runtime(runtime) => {
            record.capability_id.clone_from(&runtime.capability_id);
            record.session_id = runtime
                .wire_session
                .clone()
                .unwrap_or_else(|| wire_session_id(&runtime.session));
            record.generation = i64::try_from(runtime.generation).unwrap_or(i64::MAX);
            record.host_account_id.clone_from(&runtime.host_account.0);
            record.host_steam_id = runtime.host_peer.map(|peer| peer.0.to_string()).unwrap_or_default();
            record.bridge_instance_id = runtime.bridge_instance.clone().unwrap_or_default();
            record.scopes.clone_from(&runtime.scopes);
        }
        StandIn::Hunt(hunt, now) => {
            record.session_id.clone_from(&hunt.session_id);
            record.generation = hunt.generation;
            record.host_account_id.clone_from(&hunt.host_account_id);
            record.host_steam_id.clone_from(&hunt.host_steam_id);
            record.bridge_instance_id.clone_from(&hunt.bridge_instance_id);
            (record.issued_at, record.expires_at) = (now, hunt.expires_at);
        }
    }
    record
}

enum StandIn<'a> {
    Runtime(&'a RuntimeAuth),
    Hunt(&'a HuntSession, UnixMillis),
}

/// A session with nothing set beyond its id, creation and lifetime; the creators fill in the rest.
fn new_session(session_id: String, created_at: UnixMillis, expires_at: UnixMillis) -> HuntSession {
    HuntSession {
        active_party_key: None,
        bridge_instance_id: String::new(),
        bridge_protocol: 1,
        build: DEFAULT_GAME_BUILD,
        cancelled_by_account_id: None,
        completion_reason: None,
        coop_transport: Some(String::from("ICE")),
        created_at,
        drain_deadline: None,
        expires_at,
        extra: Map::new(),
        failure_reason: None,
        game_mode: None,
        generation: 1,
        guard_completion_mode: None,
        guard_encounter_state: None,
        host_account_id: String::new(),
        host_local_bridge_port: None,
        host_local_bridge_ready_at: None,
        host_offer_id: None,
        host_participates: Some(true),
        host_steam_id: String::new(),
        hub_kind: None,
        hub_room_id: None,
        hunt_definition_id: String::new(),
        last_heartbeat_at: None,
        last_lease_sequence: None,
        lease_expires_at: None,
        lobby_id: None,
        max_peers: None,
        metadata_nonce: Some(random_nonce()),
        offer_expires_at: None,
        party_id: String::new(),
        party_revision: 1,
        pre_suspect_state: None,
        realm_id: None,
        recovery_deadline: None,
        recovery_retry_at: None,
        roster: Vec::new(),
        server_udp_port: None,
        session_id,
        startup_deadline: None,
        state: H::Requested,
        state_version: 1,
        suspect_deadline: None,
        suspect_since: None,
        terminal_at: None,
        tried_host_account_ids: Vec::new(),
        trust_tier: None,
        virtual_port: 1,
    }
}

/// HOST_OFFERED to STARTING with the bootstrap's bridge and nonce.
fn start_host(tx: &mut HuntTx<'_>, startup: &HostStartup, now: UnixMillis) -> Result<HuntSession, RealmsError> {
    let Some(started) = tx.begin_host_startup(startup, now)? else {
        return Err(refused(FENCED, "The session is no longer waiting for host startup."));
    };
    let host = Some(startup.host_account_id.as_str());
    transition(
        tx,
        H::HostOffered,
        &started,
        "HOST_BOOTSTRAP_ISSUED",
        TransitionActor::Host,
        host,
        now,
    )?;
    Ok(started)
}

/// The host is ready: the first report is recorded, credited, and sends every other player its join bootstrap.
fn record_host_ready(
    tx: &mut HuntTx<'_>,
    mark: &HostReadyMark,
    host: &str,
    now: UnixMillis,
) -> Result<HuntSession, RealmsError> {
    let before = tx.find_hunt(&mark.session_id)?.map(|h| h.state);
    let Some(ready) = tx.mark_host_ready(mark, now)? else {
        return Err(no(NOT_AWAITING_READY));
    };
    if before == Some(H::HostReady) {
        return Ok(ready);
    }
    let from = before.unwrap_or(H::Starting);
    transition(tx, from, &ready, "HOST_READY", TransitionActor::Host, Some(host), now)?;
    tx.increment_reliability(host, one(|r| &mut r.starts_succeeded), now, None)?;
    if !ready.is_realm_hub() {
        let join = json!({ "sessionId": ready.session_id, "generation": ready.generation, "role": "JOIN" });
        for p in ready.roster.iter().filter(|p| p.account_id != host) {
            queue(
                tx,
                &p.account_id,
                "JOIN_BOOTSTRAP",
                &ready,
                join.clone(),
                now,
                CONTROL_LIFE_MS,
            )?;
        }
    }
    Ok(ready)
}

/// What the hub host's own route-ready report moves.
struct HubHostRoute {
    id: String,
    generation: i64,
    version: i64,
    from: HuntState,
    host: String,
    port: i64,
    lease_ms: i64,
    now: UnixMillis,
}

/// A hub is RUNNING once its host's local route is up; a hub without a lease starts one.
fn hub_host_route_ready(tx: &mut HuntTx<'_>, route: HubHostRoute) -> Result<HuntSession, RealmsError> {
    let HubHostRoute {
        id,
        generation,
        version,
        from,
        host,
        port,
        lease_ms,
        now,
    } = route;
    let live = [H::Starting, H::HostReady, H::Admitting, H::Running];
    let run = |h: &mut HuntSession| {
        (h.host_local_bridge_port, h.host_local_bridge_ready_at) = (Some(port), Some(now));
        h.lobby_id.get_or_insert_with(|| String::from("ICE"));
        if h.lease_expires_at.is_none() {
            (h.last_heartbeat_at, h.last_lease_sequence) = (Some(now), Some(0));
            h.lease_expires_at = Some(now.saturating_add(lease_ms));
        }
        h.state = H::Running;
    };
    let Some(running) = tx.compare_and_set(&id, generation, &live, version, run)? else {
        return Err(refused(FENCED, "Hub session is not active."));
    };
    if from != H::Running {
        transition(
            tx,
            from,
            &running,
            "HUB_HOST_ROUTE_READY",
            TransitionActor::Host,
            Some(&host),
            now,
        )?;
    }
    Ok(running)
}

/// HOST_READY or ADMITTING moves on as participants are admitted: to ADMITTING with the first, RUNNING with all.
fn admit_participant(
    tx: &mut HuntTx<'_>,
    id: &str,
    generation: i64,
    who: &str,
    at: UnixMillis,
) -> Result<bool, RealmsError> {
    let Some(marked) = tx.mark_participant_admitted(id, generation, who, at)? else {
        return Ok(false);
    };
    let from = marked.state;
    if from != H::HostReady && from != H::Admitting {
        return Ok(true);
    }
    let everyone = marked
        .roster
        .iter()
        .all(|p| p.admission_state == AdmissionState::Admitted);
    let (target, reason) = if everyone {
        (H::Running, "ALL_PARTICIPANTS_ADMITTED")
    } else {
        (H::Admitting, "FIRST_PARTICIPANT_ADMITTED")
    };
    if target == from {
        return Ok(true);
    }
    let moved = tx.compare_and_set(
        id,
        generation,
        &[H::HostReady, H::Admitting],
        marked.state_version,
        |h| {
            h.state = target;
        },
    )?;
    if let Some(moved) = moved {
        transition(tx, from, &moved, reason, TransitionActor::Host, Some(who), at)?;
    }
    Ok(true)
}

impl Hosting {
    fn set_candidate(&self, account: &str, offer: Candidate) {
        let Some(runtime) = self.runtime.as_ref() else {
            return;
        };
        let _ = runtime.core.set_candidate(AccountId(account.into()), offer);
    }

    /// For a host that also plays: a ticket of its own; any failure is no ticket.
    fn self_ticket(&self, hunt: &HuntSession, account: &str) -> Option<Secret> {
        let runtime = self.runtime.as_ref()?;
        let own_peer = hunt.participant(account).map_or("", |p| p.steam_id.as_str());
        let bindings = TicketBindings {
            session: hunt.session_id.parse::<SessionUuid>().ok()?,
            generation: u32::try_from(hunt.generation.max(1)).ok()?,
            party_revision: u64::try_from(hunt.party_revision.max(1)).ok()?,
            joiner_peer: PeerId(peer_number(own_peer, account)),
            host_peer: PeerId(peer_number(&hunt.host_steam_id, &hunt.host_account_id)),
            account: AccountId(account.into()),
            party_id: party_text(&hunt.party_id),
            hunt_definition_id: hunt.hunt_definition_id.clone(),
            build: build_number(hunt),
            bridge_protocol: bounded_u16(hunt.bridge_protocol, 1, 255),
            virtual_port: bounded_u16(hunt.virtual_port, 1, 65_535),
        };
        let lifetime = Some(JOIN_TICKET_LIFETIME_SECONDS);
        let minted = TicketAuthority::mint(runtime.tickets.as_ref(), bindings, self.now(), lifetime).ok()?;
        let millis = |seconds: u64| UnixMillis(i64::try_from(seconds).unwrap_or(i64::MAX).saturating_mul(1000));
        let claims = &minted.claims;
        let bound = &claims.bindings;
        let record = TicketRecord {
            issued_at: millis(claims.issued_at),
            expires_at: millis(claims.expires_at),
            key_id: claims.key_id.0.clone(),
            jti_digest: minted.jti_digest.clone(),
            generation: u64::from(bound.generation),
            joiner_peer: bound.joiner_peer,
            host_peer: bound.host_peer,
            bridge_protocol: bound.bridge_protocol,
            virtual_port: bound.virtual_port,
            session: hunt.session_id.clone(),
            account: AccountId(account.into()),
            consumer_request_id: None,
            consumer_bridge_instance_id: None,
            consumed_at: None,
            status: TicketStatus::Issued,
        };
        let _ = runtime.admission_store.insert_ticket(&as_account(account), &record);
        Some(minted.text)
    }

    fn bootstrap_json(
        &self,
        hunt: &HuntSession,
        capability: &Secret,
        host_ticket: Option<&Secret>,
        hub_afk_seconds: Option<i64>,
    ) -> Value {
        let is_hub = hunt.is_realm_hub();
        let runtime = self.runtime.as_ref();
        let accounts: Vec<String> = hunt.roster.iter().map(|p| p.account_id.clone()).collect();
        let manifest = resolve_manifest_for_hunt(&hunt.hunt_definition_id, &accounts);
        let listed = |key: &str| manifest.get(key).cloned().unwrap_or(Value::Null);
        let completion = match hunt.guard_completion_mode.as_deref() {
            Some(mode) => mode.to_owned(),
            None => manifest["guardCompletionMode"]
                .as_str()
                .unwrap_or("FIRST_DEFEAT")
                .to_owned(),
        };
        let configured_port = hunt.server_udp_port.and_then(|port| u16::try_from(port).ok());
        let udp = configured_port
            .or(runtime.map(|r| r.host_server_udp_port))
            .unwrap_or(DEFAULT_SERVER_UDP_PORT);
        let default_peers = if is_hub {
            DEFAULT_HUB_MAX_PLAYERS
        } else {
            HUNT_MAX_PEERS
        };
        let peers = hunt.max_peers.map_or(default_peers, |n| n.clamp(1, 64));
        let key_id = runtime.map_or_else(|| String::from("local-key-1"), |r| r.tickets.key_id.0.clone());
        let backend = runtime.map_or_else(|| String::from("http://127.0.0.1:8080"), |r| r.backend_url.clone());
        let nonce = hunt.metadata_nonce.as_deref().unwrap_or(PLACEHOLDER_NONCE);
        let peer = host_peer_text(hunt);
        let expected = listed("expectedPlayerString");
        object([
            ("keyId", key_id.into()),
            ("sessionId", json!(hunt.session_id)),
            ("partyId", party_text(&hunt.party_id).into()),
            ("partyRevision", hunt.party_revision.max(1).into()),
            ("huntDefinitionId", json!(hunt.hunt_definition_id)),
            ("generation", hunt.generation.max(1).into()),
            ("build", hunt.build.max(1).into()),
            ("bridgeProtocol", hunt.bridge_protocol.clamp(1, 255).into()),
            ("virtualPort", hunt.virtual_port.clamp(1, 65_535).into()),
            ("hostPeerId", json!(peer)),
            ("hostSteamId", peer.into()),
            ("bridgeInstanceId", json!(hunt.bridge_instance_id)),
            ("metadataNonce", nonce.into()),
            ("forceRelay", self.settings().policy.force_sdr.into()),
            ("guardCompletionMode", completion.into()),
            ("sessionRole", (if is_hub { "HUB" } else { "HUNT" }).into()),
            ("hubKind", json!(hunt.hub_kind.map(hub_kind_name))),
            ("capability", capability.expose().into()),
            ("hostTicket", json!(host_ticket.map(Secret::expose))),
            ("backendUrl", backend.into()),
            ("serverUdpPort", udp.into()),
            ("maximumPeers", peers.into()),
            ("hostLocalClient", hunt.host_participates.unwrap_or(true).into()),
            ("huntEvidence", (!is_hub).into()),
            ("matchmakerHuntId", listed("matchmakerHuntId")),
            ("mapPath", listed("mapPath")),
            ("behemothPath", listed("behemothPath")),
            ("expectedPlayers", expected.clone()),
            ("expectedPlayerString", expected),
            ("runtimeProfile", "production".into()),
            ("maxPlayers", if is_hub { peers.into() } else { Value::Null }),
            ("hubAfkSeconds", json!(hub_afk_seconds)),
        ])
    }

    /// Node's `CreateHostBootstrap`.
    pub async fn create_host_bootstrap(&self, account: &str, raw_session_id: &str) -> Result<Value, RealmsError> {
        if !self.settings().policy.enabled {
            return Err(refused("P2P_DISABLED", "Player-hosted hunts are disabled."));
        }
        if self.emergency_stop().await? {
            return Err(no(HUNTS_STOPPED));
        }
        let Some(runtime) = self.runtime.clone() else {
            return Err(refused("P2P_DISABLED", "Host bootstrap runtime is not configured."));
        };
        let Some(found) = self.find_by_path(raw_session_id).await? else {
            return Err(no(NO_SESSION));
        };
        if found.host_account_id != account {
            return Err(no(NOT_THE_HOST));
        }
        let now = self.now();
        let hunt = match found.state {
            H::HostOffered => {
                // An offer always gets a fresh bridge.
                let startup = HostStartup {
                    bridge_instance_id: format!("bridge-{}", random_uuid()),
                    metadata_nonce: found.metadata_nonce.clone().unwrap_or_else(random_nonce),
                    startup_deadline: now.saturating_add(self.settings().eligibility.startup_lifetime_ms),
                    server_udp_port: found.server_udp_port.or(Some(i64::from(runtime.host_server_udp_port))),
                    host_account_id: account.into(),
                    session_id: found.session_id.clone(),
                };
                self.hunts
                    .transaction(&as_account(account), move |tx| start_host(tx, &startup, now))
                    .await?
            }
            // A repeated bootstrap.
            H::Starting | H::HostReady | H::Admitting | H::Running => found,
            _ => return Err(refused(FENCED, "This session generation cannot be bootstrapped.")),
        };
        self.ensure_verified_link(account, &host_peer_text(&hunt), 1, DERIVED);
        let (scopes, expires) = match hunt.is_realm_hub() {
            true => (
                HUB_CAPABILITY_SCOPES.to_vec(),
                now.saturating_add(self.settings().realm_hub.capability_ttl_ms),
            ),
            false => (HUNT_CAPABILITY_SCOPES.to_vec(), hunt.expires_at),
        };
        let (secret, _) = runtime
            .capabilities
            .mint(&hunt, &hunt.bridge_instance_id, scopes, now, expires)
            .await?;
        let ticket = if hunt.host_participates == Some(false) {
            None
        } else {
            self.self_ticket(&hunt, account)
        };
        Ok(self.bootstrap_json(&hunt, &secret, ticket.as_ref(), None))
    }

    /// Node's `MarkHostReady`.
    pub async fn mark_host_ready(
        &self,
        capability: &HostCapabilityRecord,
        body: &Map<String, Value>,
    ) -> Result<Value, RealmsError> {
        if self.emergency_stop().await? {
            return Err(no(OPERATIONS_STOPPED));
        }
        let Some(hunt) = self.hunts.find_hunt(&capability.session_id).await? else {
            return Err(no(NO_SESSION));
        };
        if hunt.generation != capability.generation {
            return Err(refused(FENCED, "The host capability belongs to a stale generation."));
        }
        let now = self.now();
        let lease = now.saturating_add(self.settings().eligibility.lease_lifetime_ms);
        let answer = |h: &HuntSession, until: UnixMillis| {
            object([
                ("ready", true.into()),
                ("sessionId", json!(h.session_id)),
                ("generation", h.generation.into()),
                ("state", h.state.as_str().into()),
                ("leaseExpiresAt", iso(until).into()),
            ])
        };
        // Already ready: the report is repeated, nothing changes.
        if [H::HostReady, H::Admitting, H::Running].contains(&hunt.state) {
            return Ok(answer(&hunt, hunt.lease_expires_at.unwrap_or(lease)));
        }
        let mark = HostReadyMark {
            lobby_id: short_text(body.get("lobbyId"), 64).unwrap_or("ICE").into(),
            virtual_port: integer_in(body.get("virtualPort"), 1, 65_535).unwrap_or(hunt.virtual_port),
            lease_expires_at: lease,
            session_id: capability.session_id.clone(),
            generation: capability.generation,
            host_steam_id: capability.host_steam_id.clone(),
            bridge_instance_id: capability.bridge_instance_id.clone(),
        };
        let host = capability.host_account_id.clone();
        let actor = as_account(&host);
        let ready = self
            .hunts
            .transaction(&actor, move |tx| record_host_ready(tx, &mark, &host, now))
            .await?;
        if ready.is_realm_hub() {
            self.mirror_hub(&ready, HubState::Starting).await;
        }
        Ok(answer(&ready, ready.lease_expires_at.unwrap_or(lease)))
    }

    /// Node's `MarkLocalRouteReady`.
    pub async fn mark_local_route_ready(
        &self,
        account: &str,
        raw_session_id: &str,
        body: &Map<String, Value>,
    ) -> Result<Value, RealmsError> {
        let reported = body.get("localBridgePort").or_else(|| body.get("localPort"));
        let Some(port) = integer_in(reported, 1, 65_535) else {
            return Err(no(BAD_LOCAL_PORT));
        };
        let Some(hunt) = self.find_by_path(raw_session_id).await? else {
            return Err(no(NO_SESSION));
        };
        let stale = body
            .get("generation")
            .and_then(Value::as_i64)
            .is_some_and(|g| g != hunt.generation);
        if stale {
            return Err(refused(FENCED, "The route-ready report belongs to a stale generation."));
        }
        let now = self.now();
        let answer = |h: &HuntSession| {
            object([
                ("ready", true.into()),
                ("sessionId", json!(h.session_id)),
                ("generation", h.generation.into()),
                ("localBridgePort", port.into()),
                ("state", h.state.as_str().into()),
            ])
        };
        if hunt.is_realm_hub() {
            let running = self.hub_route_ready(account, &hunt, port, now).await?;
            return Ok(answer(running.as_ref().unwrap_or(&hunt)));
        }
        // A host whose transport reports route-ready while still STARTING is ready first.
        let hunt = if hunt.state == H::Starting && hunt.host_account_id == account {
            self.mark_host_ready(&stand_in_capability(StandIn::Hunt(&hunt, now)), &Map::new())
                .await?;
            self.hunts
                .find_hunt(&hunt.session_id)
                .await?
                .ok_or_else(|| no(NO_SESSION))?
        } else {
            hunt
        };
        let Some(player) = hunt.participant(account).cloned() else {
            return Err(refused("P2P_NOT_ROSTERED", "This account is not in the hunt roster."));
        };
        let bridge = match short_text(body.get("bridgeInstanceId"), 96) {
            Some(reported) => reported.to_owned(),
            None => player
                .local_bridge_instance_id
                .clone()
                .unwrap_or_else(|| format!("local-{}", random_uuid())),
        };
        let (id, generation) = (hunt.session_id.as_str(), hunt.generation);
        let marked = self
            .hunts
            .mark_participant_route_ready(
                &as_account(account),
                id,
                generation,
                account,
                &player.steam_id,
                &bridge,
                port,
                now,
            )
            .await?;
        let Some(updated) = marked else {
            return Err(refused(FENCED, "The hunt generation does not accept route updates."));
        };
        self.set_candidate(account, hunt_candidate(&updated, &player.candidate_id, true, port));
        Ok(answer(&updated))
    }

    /// A hub's route-ready: its host's runs the hub, a guest's readies its entry. `Some` is the host's running hub.
    async fn hub_route_ready(
        &self,
        account: &str,
        hub: &HuntSession,
        port: i64,
        now: UnixMillis,
    ) -> Result<Option<HuntSession>, RealmsError> {
        let Some(realm) = hub.realm_id.clone() else {
            return Err(refused("P2P_SESSION_NOT_FOUND", "Hub session has no realm."));
        };
        let actor = as_account(account);
        if hub.host_account_id != account {
            let row = self
                .hub
                .mark_route_ready(&actor, &hub.session_id, hub.generation, account, port, now, now)
                .await?;
            let Some(row) = row else {
                return Err(no(NO_HUB_ENTRY));
            };
            self.invalidate_hub(&hub.session_id);
            self.set_candidate(account, hub_candidate(hub, &row.candidate_id, true, port));
            return Ok(None);
        }
        let route = HubHostRoute {
            id: hub.session_id.clone(),
            generation: hub.generation,
            version: hub.state_version,
            from: hub.state,
            host: account.to_owned(),
            lease_ms: self.settings().eligibility.lease_lifetime_ms,
            port,
            now,
        };
        let running = self
            .hunts
            .transaction(&actor, move |tx| hub_host_route_ready(tx, route))
            .await?;
        let (id, generation, peer) = (&running.session_id, running.generation, host_peer_text(&running));
        self.hub
            .admit_host(&actor, id, generation, &realm, account, &peer, port, now)
            .await?;
        self.invalidate_hub(id);
        self.mirror_hub(&running, HubState::Running).await;
        self.set_candidate(
            account,
            hub_candidate(&running, &format!("cand-{id}-{account}"), true, port),
        );
        Ok(Some(running))
    }

    /// The host's own end of its session; a hub ends through the hub path. Returns the session read and its end.
    async fn end_by_host(
        &self,
        capability: &HostCapabilityRecord,
        body: &Map<String, Value>,
        default_reason: &str,
        terminal: HuntState,
        allowed: &[HuntState],
    ) -> Result<(HuntSession, Option<HuntSession>), RealmsError> {
        let reason = short_text(body.get("reason"), 128).unwrap_or(default_reason);
        let hunt = self
            .hunts
            .find_hunt(&capability.session_id)
            .await?
            .ok_or_else(|| no(NO_SESSION))?;
        if hunt.generation != capability.generation {
            return Err(refused(FENCED, "Stale hunt generation."));
        }
        let host = capability.host_account_id.as_str();
        let (by, id) = (TransitionActor::Host, hunt.session_id.as_str());
        let ended = if hunt.is_realm_hub() {
            self.end_hub(id, hunt.generation, reason, terminal, by, Some(host))
                .await?
        } else {
            self.end_hunt(&as_account(host), &hunt, reason, allowed, by, Some(host), true)
                .await?
        };
        Ok((hunt, ended))
    }

    /// Node's `AbortHunt`.
    pub async fn abort_hunt(
        &self,
        capability: &HostCapabilityRecord,
        body: &Map<String, Value>,
    ) -> Result<Value, RealmsError> {
        let abortable = [
            H::Starting,
            H::HostReady,
            H::Admitting,
            H::Running,
            H::Draining,
            H::Suspect,
        ];
        let (read, ended) = self
            .end_by_host(capability, body, "HOST_ABORTED", H::Aborted, &abortable)
            .await?;
        Ok(ended_answer("aborted", &read, ended.as_ref()))
    }

    /// Node's `EndHunt`.
    pub async fn finish_hunt(
        &self,
        capability: &HostCapabilityRecord,
        body: &Map<String, Value>,
    ) -> Result<Value, RealmsError> {
        let (read, ended) = self
            .end_by_host(capability, body, "HOST_COMPLETED", H::Completed, &HOST_END_STATES)
            .await?;
        Ok(ended_answer("ended", &read, ended.as_ref()))
    }

    /// Node's `GetHuntStatus`: a caller must be part of the session; none is an admin or a capability.
    pub async fn get_hunt_status(&self, caller: Option<&str>, raw_session_id: &str) -> Result<Value, RealmsError> {
        let Some(hunt) = self.find_by_path(raw_session_id).await? else {
            return Err(no(NO_SESSION));
        };
        if let Some(account) = caller {
            let rostered = hunt.host_account_id == account || hunt.participant(account).is_some();
            let in_hub =
                !rostered && hunt.is_realm_hub() && self.hub.find_presence(&hunt.session_id, account).await?.is_some();
            if !(rostered || in_hub) {
                return Err(refused("P2P_NOT_ROSTERED", "You are not part of this session."));
            }
        }
        Ok(object([
            ("sessionId", json!(hunt.session_id)),
            ("generation", hunt.generation.into()),
            ("state", hunt.state.as_str().into()),
            ("hostAccountId", json!(hunt.host_account_id)),
            ("partyId", json!(hunt.party_id)),
            ("huntDefinitionId", json!(hunt.hunt_definition_id)),
            ("completionReason", json!(hunt.completion_reason)),
            ("failureReason", json!(hunt.failure_reason)),
        ]))
    }

    /// The host ends its own hub cleanly.
    async fn supersede_hub(
        &self,
        live: &HuntSession,
        why: &str,
        host: &str,
    ) -> Result<Option<HuntSession>, RealmsError> {
        self.end_hub(
            &live.session_id,
            live.generation,
            why,
            H::Completed,
            TransitionActor::Host,
            Some(host),
        )
        .await
    }

    async fn live_hub(&self, realm_id: &str, kind: HubKind) -> Result<Option<HuntSession>, RealmsError> {
        let found = self
            .hunts
            .find_active_hunt_for_party(&hub_party_key(realm_id, kind))
            .await?;
        Ok(found.filter(|hub| hub.is_realm_hub() && is_hub_live(hub.state)))
    }

    /// Node's `CreateRealmHubSession`.
    pub async fn start_realm_hub(&self, account: &str, body: &Map<String, Value>) -> Result<Value, RealmsError> {
        let settings = self.settings();
        if !(settings.realm_hosting && settings.policy.enabled) {
            return Err(no(HOSTING_OFF));
        }
        if self.emergency_stop().await? {
            return Err(no(HOSTED_STOPPED));
        }
        let Some(runtime) = self.runtime.clone() else {
            return Err(no(NO_HUB_RUNTIME));
        };
        let (kind, max_players) = parse_hub_start_body(&Value::Object(body.clone()))
            .map_err(|message| refused("REALM_VALIDATION_FAILED", message))?;
        let afk = integer_in(body.get("afkSeconds").or_else(|| body.get("hubAfkSeconds")), 0, 7200).unwrap_or(0);
        let Some(realm) = self.realms.find_by_host(account).await? else {
            return Err(refused("REALM_NOT_FOUND", "You do not host an active realm."));
        };
        // A new start replaces the hub of that kind.
        if let Some(live) = self.live_hub(&realm.realm_id, kind).await? {
            self.supersede_hub(&live, "SUPERSEDED_BY_HOST", account).await?;
        }
        let active = self.hunts.count_active_realm_servers(&realm.realm_id).await?;
        let max = realm
            .hosting
            .as_ref()
            .map_or(settings.realm_hub.default_max_servers, |h| h.max_servers);
        if active >= max {
            return Err(no(SERVER_LIMIT));
        }
        let now = self.now();
        let oldest = UnixMillis(now.0 - settings.eligibility.presence_maximum_age_ms);
        let mut fresh = self
            .hunts
            .list_host_presences(&[account.to_owned()], oldest, now)
            .await?;
        // Without a fresh presence the start goes ahead unchecked.
        if !fresh.is_empty() {
            let presence = fresh.swap_remove(0);
            let guard = self.parts.operations.guard_refuses();
            if let Some(reason) =
                realm_host_ineligibility_reason(&presence, &settings.eligibility, 0.0, 0.0, guard, now)
            {
                let message = format!("Host presence is ineligible ({reason}).");
                return Err(RealmsError::Refused(Refusal {
                    code: "P2P_HOST_INELIGIBLE".into(),
                    message,
                }));
            }
            let hub = &settings.realm_hub;
            let input = RealmCapacityInput {
                active_servers: active,
                max_servers: max,
                free_physical_memory_mib: presence.free_physical_memory_mib,
                commit_headroom_mib: presence.commit_headroom_mib,
                minimum_free_memory_mib: hub.minimum_free_memory_mib,
                minimum_commit_headroom_mib: hub.minimum_commit_headroom_mib,
            };
            if let RealmCapacity::ServerLimit | RealmCapacity::CommitLow = decide_realm_capacity(&input) {
                let text = "Insufficient host capacity to start another realm server.";
                return Err(refused("REALM_CAPACITY_EXCEEDED", text));
            }
        }
        let peer = derive_ice_peer_id(account);
        self.ensure_verified_link(account, &peer, 1, DERIVED);
        let hub = self.new_hub(
            account,
            &realm.realm_id,
            kind,
            max_players,
            peer,
            runtime.host_server_udp_port,
            now,
        );
        self.hunts.create_hunt(&as_account(account), &hub).await?;
        self.mirror_hub(&hub, HubState::Starting).await;
        let until = now.saturating_add(settings.realm_hub.capability_ttl_ms);
        let scopes = HUB_CAPABILITY_SCOPES.to_vec();
        let (secret, _) = runtime
            .capabilities
            .mint(&hub, &hub.bridge_instance_id, scopes, now, until)
            .await?;
        let ticket = self.self_ticket(&hub, account);
        Ok(self.bootstrap_json(&hub, &secret, ticket.as_ref(), Some(afk)))
    }

    /// A realm hub, STARTING on its host with a lease and the startup clock running.
    #[allow(clippy::too_many_arguments)]
    fn new_hub(
        &self,
        host: &str,
        realm_id: &str,
        kind: HubKind,
        max_players: i64,
        peer: String,
        base_udp: u16,
        now: UnixMillis,
    ) -> HuntSession {
        let settings = self.settings();
        let manifest = hub_manifest(kind);
        let party = hub_party_key(realm_id, kind);
        let mut hub = new_session(
            uuid_text(new_session_ulid(now)),
            now,
            now.saturating_add(HUB_SESSION_LIFETIME_MS),
        );
        hub.state = H::Starting;
        hub.active_party_key = Some(party.clone());
        hub.party_id = party;
        hub.hunt_definition_id = manifest.hunt_definition_id.into();
        hub.game_mode = Some(manifest.game_mode.into());
        hub.guard_completion_mode = Some(manifest.guard_completion_mode.into());
        hub.host_account_id = host.into();
        hub.host_steam_id = peer;
        hub.tried_host_account_ids = vec![host.into()];
        hub.bridge_instance_id = format!("bridge-{}", random_uuid());
        hub.lobby_id = Some(String::from("ICE"));
        hub.lease_expires_at = Some(now.saturating_add(settings.eligibility.lease_lifetime_ms));
        hub.startup_deadline = Some(now.saturating_add(settings.eligibility.startup_lifetime_ms));
        (hub.last_heartbeat_at, hub.last_lease_sequence) = (Some(now), Some(0));
        hub.hub_kind = Some(kind);
        hub.realm_id = Some(realm_id.into());
        hub.hub_room_id = Some(hub_room_id(realm_id, kind));
        // The training hub listens one port above Ramsgate's.
        hub.server_udp_port = Some(i64::from(base_udp) + i64::from(kind == HubKind::Training));
        hub.max_peers = Some(max_players);
        hub.trust_tier = Some(settings.policy.trust_tier);
        hub
    }

    /// Node's `StopRealmHubSession`.
    pub async fn stop_realm_hub(&self, account: &str, body: &Map<String, Value>) -> Result<Value, RealmsError> {
        let kind = parse_hub_stop_body(&Value::Object(body.clone()))
            .map_err(|message| refused("REALM_VALIDATION_FAILED", message))?;
        let Some(realm) = self.realms.find_by_host(account).await? else {
            return Err(refused("REALM_NOT_FOUND", "You do not host an active realm."));
        };
        let answer = |stopped: bool, session: Value| {
            object([
                ("stopped", stopped.into()),
                ("hubKind", hub_kind_name(kind).into()),
                ("sessionId", session),
            ])
        };
        let Some(live) = self.live_hub(&realm.realm_id, kind).await? else {
            return Ok(answer(false, Value::Null));
        };
        let stopped = self.supersede_hub(&live, "HOST_STOPPED_HUB", account).await?.is_some();
        Ok(answer(stopped, live.session_id.into()))
    }

    async fn hub_status(&self, realm_id: &str, kind: HubKind) -> Result<Value, RealmsError> {
        let Some(hub) = self.live_hub(realm_id, kind).await? else {
            return Ok(Value::Null);
        };
        let admitted = [PresenceState::Admitted];
        let count = self
            .hub
            .count_by_state(&hub.session_id, hub.generation, &admitted)
            .await?;
        Ok(object([
            ("sessionId", json!(hub.session_id)),
            ("generation", hub.generation.into()),
            ("state", hub.state.as_str().into()),
            ("admittedPlayers", json!(count)),
            ("maxPlayers", hub.max_peers.unwrap_or(DEFAULT_HUB_MAX_PLAYERS).into()),
        ]))
    }

    /// Node's `GetRealmHostingStatus`.
    pub async fn get_realm_hosting_status(&self, account: &str) -> Result<Value, RealmsError> {
        let enabled = self.settings().policy.enabled && self.settings().realm_hosting;
        let Some(realm) = self.realms.find_by_host(account).await? else {
            return Ok(object([
                ("enabled", enabled.into()),
                ("realmId", Value::Null),
                ("realmName", Value::Null),
                ("hosting", Value::Null),
                ("ramsgate", Value::Null),
                ("training", Value::Null),
                ("activeHunts", Value::Array(Vec::new())),
            ]));
        };
        let hosting = match realm.hosting.as_ref() {
            None => Value::Null,
            Some(h) => object([
                ("maxServers", h.max_servers.into()),
                ("maxPlayers", h.max_players.into()),
                ("afkSeconds", json!(h.afk_seconds)),
                ("hubSessionId", json!(h.hub_session_id)),
                ("hubState", h.hub_state.as_str().into()),
                ("trainingState", h.training_state.as_str().into()),
                ("activeServers", h.active_servers.into()),
                ("updatedAt", iso(h.updated_at).into()),
            ]),
        };
        let live = self.hunts.list_live_realm_hunts(&realm.realm_id, account).await?;
        let hunts: Vec<Value> = live
            .iter()
            .filter(|h| !h.is_realm_hub())
            .map(|h| {
                object([
                    ("sessionId", json!(h.session_id)),
                    ("generation", h.generation.into()),
                    ("state", h.state.as_str().into()),
                    ("huntDefinitionId", json!(h.hunt_definition_id)),
                    ("partySize", h.roster.len().into()),
                ])
            })
            .collect();
        let ramsgate = self.hub_status(&realm.realm_id, HubKind::Ramsgate).await?;
        let training = self.hub_status(&realm.realm_id, HubKind::Training).await?;
        Ok(object([
            ("enabled", enabled.into()),
            ("realmId", json!(realm.realm_id)),
            ("realmName", json!(realm.name)),
            ("hosting", hosting),
            ("ramsgate", ramsgate),
            ("training", training),
            ("activeHunts", Value::Array(hunts)),
        ]))
    }

    /// A routed hunt in selection, its first member as the presumed host.
    pub async fn create_routed_hunt(
        &self,
        party_id: &str,
        party_revision: i64,
        hunt_definition_id: &str,
        game_mode: &str,
        members: &[String],
        realm_id: Option<&str>,
    ) -> Result<HuntSession, RealmsError> {
        let now = self.now();
        let mut hunt = new_session(
            uuid_text(new_session_ulid(now)),
            now,
            now.saturating_add(HUNT_LIFETIME_MS),
        );
        let session_id = hunt.session_id.clone();
        for account in members {
            let peer = derive_ice_peer_id(account);
            self.ensure_verified_link(account, &peer, 1, DERIVED);
            hunt.roster.push(Participant::pending(
                account,
                &peer,
                &format!("cand-{session_id}-{account}"),
            ));
        }
        let host = members.first().map_or("", String::as_str);
        hunt.state = H::HostSelecting;
        (hunt.party_id, hunt.active_party_key) = (party_id.into(), Some(party_id.into()));
        hunt.party_revision = party_revision.max(1);
        hunt.hunt_definition_id = hunt_definition_id.into();
        hunt.game_mode = Some(game_mode.into());
        hunt.host_account_id = host.into();
        hunt.host_steam_id = if host.is_empty() {
            String::new()
        } else {
            derive_ice_peer_id(host)
        };
        hunt.realm_id = realm_id.map(String::from);
        hunt.server_udp_port = self.runtime.as_ref().map(|r| i64::from(r.host_server_udp_port));
        hunt.max_peers = Some(HUNT_MAX_PEERS);
        hunt.guard_completion_mode = Some(String::from("FIRST_DEFEAT"));
        hunt.trust_tier = Some(self.settings().policy.trust_tier);
        let actor = if host.is_empty() { ROUTE_ACTOR } else { as_account(host) };
        self.hunts.create_hunt(&actor, &hunt).await?;
        self.begin_host_selection(&session_id).await?;
        Ok(self.hunts.find_hunt(&session_id).await?.unwrap_or(hunt))
    }

    pub(crate) async fn restore_matchmaking_candidate(
        &self,
        player: &AccountId,
    ) -> Result<Option<Candidate>, RealmsError> {
        let account = player.0.as_str();
        let active = self.hunts.find_active_hunt_for_account(account).await?;
        if let Some(hunt) = active.as_ref()
            && let Some(me) = hunt.participant(account)
        {
            return Ok(Some(participant_candidate(hunt, me)));
        }
        let Some(row) = self.hub.find_for_account(account, None).await? else {
            return Ok(None);
        };
        let Some(hub) = self
            .hunts
            .find_hunt(&row.session_id)
            .await?
            .filter(|h| is_hub_live(h.state))
        else {
            return Ok(None);
        };
        let ready = row.local_bridge_port.is_some_and(|p| p > 0) && row.local_bridge_ready_at.is_some();
        let port = row.local_bridge_port.unwrap_or(0);
        Ok(Some(hub_candidate(&hub, &row.candidate_id, ready, port)))
    }

    pub(crate) async fn poll_candidate_status(
        &self,
        player: &AccountId,
        current: &Candidate,
    ) -> Result<Option<Candidate>, RealmsError> {
        let Ok(Value::Object(mut fields)) = serde_json::from_str::<Value>(&current.0.0) else {
            return Ok(None);
        };
        let field = |name: &str| fields.get(name).and_then(Value::as_str).map(String::from);
        let (from_hub, game_session, hub_session) = (field("Source"), field("GameSessionId"), field("SourceSessionId"));
        let account = player.0.as_str();
        // (ready on this port) or (cancelled with this reason); `None` leaves the candidate as it is.
        let verdict: Option<Result<i64, String>> = if from_hub.as_deref() == Some("REALM_HUB") {
            let Some(id) = hub_session else {
                return Ok(None);
            };
            let Some(hub) = self.hunts.find_hunt(&id).await? else {
                return Ok(None);
            };
            if [H::Completed, H::Aborted].contains(&hub.state) {
                Some(Err(hub
                    .failure_reason
                    .or(hub.completion_reason)
                    .unwrap_or_else(|| "HUB_ENDED".into())))
            } else {
                let row = self.hub.find_presence(&hub.session_id, account).await?;
                row.and_then(|r| r.local_bridge_ready_at.and(r.local_bridge_port))
                    .map(Ok)
            }
        } else {
            let Some(id) = game_session else {
                return Ok(None);
            };
            let Some(hunt) = self.hunts.find_hunt(&id).await? else {
                return Ok(None);
            };
            if hunt.state == H::Aborted {
                Some(Err(hunt
                    .failure_reason
                    .clone()
                    .unwrap_or_else(|| "HUNT_ABORTED".into())))
            } else {
                hunt.participant(account)
                    .and_then(|p| p.local_bridge_ready_at.and(p.local_bridge_port))
                    .map(Ok)
            }
        };
        let updates: Vec<(&str, Value)> = match verdict {
            None => Vec::new(),
            Some(Ok(port)) => vec![
                ("Ready", true.into()),
                ("Host", "127.0.0.1".into()),
                ("Port", port.into()),
            ],
            Some(Err(why)) => vec![
                ("Ready", false.into()),
                ("Canceled", true.into()),
                ("StatusReason", why.into()),
            ],
        };
        for (key, value) in updates {
            fields.insert(key.into(), value);
        }
        Ok(Some(Candidate(Json(Value::Object(fields).to_string()))))
    }

    /// The party's entries into a running hub, each told to join; the hub's host gets its own candidate.
    async fn route_into_hub(
        &self,
        context: &HuntRouteContext,
        members: &[String],
        realm: &str,
        kind: HubKind,
        now: UnixMillis,
    ) -> Result<Option<&'static str>, RealmsError> {
        let Some(hub) = self.live_hub(realm, kind).await? else {
            return Ok(Some("REALM_HUB_NOT_RUNNING"));
        };
        let id = hub.session_id.as_str();
        let mut entries = Vec::new();
        for account in members.iter().filter(|account| **account != hub.host_account_id) {
            let peer_id = derive_ice_peer_id(account);
            self.ensure_verified_link(account, &peer_id, 1, DERIVED);
            entries.push(HubEntryRequest {
                account_id: account.clone(),
                candidate_id: format!("cand-{id}-{account}"),
                peer_id,
            });
        }
        let at = EntryAt {
            session_id: id.to_owned(),
            generation: hub.generation,
            realm_id: realm.to_owned(),
            expires_at: entry_expires_at(now),
            now,
        };
        let cap = hub.max_peers.unwrap_or(DEFAULT_HUB_MAX_PLAYERS);
        let placed = match self
            .hub
            .request_entries(&as_account(&context.player.0), &at, cap, &entries)
            .await?
        {
            HubEntryBatch::Full { .. } => return Ok(Some("REALM_HUB_FULL")),
            HubEntryBatch::Placed(rows) => rows,
        };
        let join = object([
            ("sessionId", json!(id)),
            ("generation", hub.generation.into()),
            ("hubKind", hub_kind_name(kind).into()),
            ("realmId", json!(realm)),
        ]);
        for (account, row, _) in placed {
            let ready = row.local_bridge_port.is_some_and(|p| p > 0) && row.local_bridge_ready_at.is_some();
            let port = row.local_bridge_port.unwrap_or(0);
            self.set_candidate(&account, hub_candidate(&hub, &row.candidate_id, ready, port));
            let message = NewControlMessage {
                session_id: Some(id.to_owned()),
                generation: Some(hub.generation),
                message_type: String::from("REALM_HUB_JOIN"),
                created_at: now,
                expires_at: now.saturating_add(CONTROL_LIFE_MS),
                payload: join.clone(),
                account_id: account.clone(),
            };
            // A lost join message only delays the player; the candidate already points at the hub.
            let _ = self
                .hunts
                .enqueue_control_message(&as_account(&account), &message)
                .await;
        }
        if members.contains(&hub.host_account_id) {
            let port = hub.host_local_bridge_port.unwrap_or(0);
            let ready = port > 0 && hub.host_local_bridge_ready_at.is_some();
            self.set_candidate(
                &hub.host_account_id,
                hub_candidate(&hub, &format!("host:{id}"), ready, port),
            );
        }
        Ok(None)
    }

    pub(crate) async fn route_matchmaking_hunt(
        &self,
        context: &HuntRouteContext,
        realm_id: Option<String>,
    ) -> Result<HuntRouteOutcome, RealmsError> {
        let Some(runtime) = self.runtime.clone() else {
            return Ok(HuntRouteOutcome::NotHandled);
        };
        let members: Vec<String> = match context.party_members.as_slice() {
            [] => vec![context.player.0.clone()],
            party => party.iter().map(|member| member.0.clone()).collect(),
        };
        let handled = HuntRouteOutcome::Handled { result: true };
        let now = self.now();
        if let Some(realm) = realm_id.as_deref()
            && let RealmRequest::Hub(kind) = classify_realm_request(&context.game_mode, &context.hunt_id)
        {
            if let Some(reason) = self.route_into_hub(context, &members, realm, kind, now).await? {
                let accounts: Vec<AccountId> = members.iter().map(|m| AccountId(m.clone())).collect();
                let _ = runtime.core.cancel_travel(&context.hunt_id, &accounts, reason);
            }
            return Ok(handled);
        }
        let stop = self.emergency_stop().await?;
        let policy = &self.settings().policy;
        // A realm's hunt is player-hosted whenever hosting runs; any other passes the policy gates.
        let player_hosted = match realm_id {
            Some(_) => policy.enabled && !stop && !context.is_hub,
            None => {
                let input = RouteInput {
                    party_size: i64::try_from(members.len()).unwrap_or(i64::MAX),
                    account_ids: members.clone(),
                    game_mode: context.game_mode.clone(),
                    hunt_id: context.hunt_id.clone(),
                    is_private_party: true,
                };
                decide_hunt_route(&input, policy, true, stop).route == HuntRoute::PlayerHosted
            }
        };
        if !player_hosted {
            return Ok(HuntRouteOutcome::NotHandled);
        }
        let key = match context.party.as_ref() {
            Some(party) => party.to_string(),
            None => format!("solo:{}", context.player.0),
        };
        let hunt = match self.hunts.find_active_hunt_for_party(&key).await? {
            Some(live) => live,
            None => {
                let revision = context.party_revision.and_then(|r| i64::try_from(r).ok()).unwrap_or(1);
                let (hunt_id, mode) = (&context.hunt_id, &context.game_mode);
                self.create_routed_hunt(&key, revision, hunt_id, mode, &members, realm_id.as_deref())
                    .await?
            }
        };
        for p in &hunt.roster {
            self.set_candidate(&p.account_id, participant_candidate(&hunt, p));
        }
        Ok(handled)
    }

    fn coop_transport_name(&self) -> &'static str {
        if self.parts.operations.ice_transport() {
            "ICE"
        } else {
            "STEAM_EXPERIMENTAL"
        }
    }

    fn emergency_policy(&self, admin_key: &str, text: &str) -> Result<Value, RealmsError> {
        let invalid = |message: &str| refused("ADMIN_VALIDATION", message);
        let Ok(body) = serde_json::from_str::<Value>(text) else {
            return Err(invalid("Invalid emergency stop payload."));
        };
        let Some(stop) = body["p2pEmergencyStop"].as_bool() else {
            return Err(invalid("p2pEmergencyStop must be a boolean."));
        };
        let expected = body["expectedVersion"].as_u64().unwrap_or(1);
        let reason = body["reason"].as_str().map_or("", js_trim);
        if reason.len() < 8 {
            return Err(invalid("An audit reason of at least 8 characters is required."));
        }
        // Nothing is stored: the stop is read from the operations view (kept as found).
        let policy = object([
            ("version", expected.saturating_add(1).into()),
            ("p2pEmergencyStop", stop.into()),
            ("coopTransport", self.coop_transport_name().into()),
            ("updatedAt", iso(self.now()).into()),
            ("updatedBy", admin_key.into()),
            ("reason", reason.into()),
        ]);
        Ok(object([("policy", policy), ("requestId", random_uuid().into())]))
    }

    fn launch_manifest(text: &str) -> Result<Value, RealmsError> {
        let invalid = |message: &str| refused("P2P_TICKET_INVALID", message);
        let Ok(body) = serde_json::from_str::<Value>(text) else {
            return Err(invalid("Invalid launch manifest request."));
        };
        let Some(hunt) = body["huntDefinitionId"]
            .as_str()
            .map(js_trim)
            .filter(|id| !id.is_empty())
        else {
            return Err(invalid("huntDefinitionId is required."));
        };
        let listed = body
            .get("expectedPlayers")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let players: Vec<String> = listed.iter().filter_map(Value::as_str).map(String::from).collect();
        Ok(resolve_manifest_for_hunt(hunt, &players))
    }
}

/// `{ <done>: true, sessionId, generation, state }` for the host's abort or end.
fn ended_answer(done: &str, read: &HuntSession, ended: Option<&HuntSession>) -> Value {
    let state = ended.map_or(read.state, |after| after.state);
    object([
        (done, true.into()),
        ("sessionId", json!(read.session_id)),
        ("generation", read.generation.into()),
        ("state", state.as_str().into()),
    ])
}

fn store_error(error: &RealmsError) -> StoreError {
    StoreError::Database(format!("{error}"))
}

fn to_admission(error: RealmsError) -> AdmissionError {
    match error {
        RealmsError::Unimplemented(missing) => AdmissionError::Unimplemented(missing),
        RealmsError::Refused(refusal) => AdmissionError::Refused(refusal),
        database => AdmissionError::Store(store_error(&database)),
    }
}

fn to_relay(error: RealmsError) -> RelayError {
    match error {
        RealmsError::Unimplemented(missing) => RelayError::Unimplemented(missing),
        RealmsError::Refused(refusal) => RelayError::Refused(refusal),
        database => RelayError::Store(store_error(&database)),
    }
}

/// Runs a store future from a synchronous port.
fn blocking<T: Send>(future: impl Future<Output = Result<T, RealmsError>> + Send) -> Result<T, RealmsError> {
    let outer = rifts_guard::block_on_store(async { Ok(future.await) });
    outer.map_err(|error| RealmsError::Database(error.to_string()))?
}

/// The generation as the store keeps it.
fn stored_generation(generation: u64) -> i64 {
    i64::try_from(generation).unwrap_or(i64::MAX)
}

impl Hosting {
    async fn gate_now(&self) -> Result<P2PGate, RealmsError> {
        let emergency_stop = self.emergency_stop().await?;
        Ok(P2PGate {
            enabled: self.settings().policy.enabled,
            emergency_stop,
        })
    }

    async fn check_hub_entry_now(
        &self,
        hub: &AdmissionHunt,
        account: &AccountId,
        link: Option<&PeerLinkRecord>,
        now: UnixMillis,
    ) -> Result<PeerId, RealmsError> {
        let Some(session) = self.hunts.find_hunt(&hub.snapshot.wire_session).await? else {
            return Err(refused("P2P_SESSION_NOT_FOUND", "Hub session not found."));
        };
        let Some(realm_id) = session.realm_id.clone() else {
            return Err(no(HUB_NOT_READY));
        };
        let realm = self.realms.find_by_id(&realm_id).await?;
        let who = account.0.as_str();
        let (banned, member) = match realm.as_ref() {
            None => (false, false),
            Some(r) => (
                r.bans.iter().any(|ban| ban.account_id == who),
                r.host_account_id == who || r.members.iter().any(|m| m.account_id == who),
            ),
        };
        let derived = derive_ice_peer_id(who);
        // Under ICE the server knows the peer id; it records the link the first time.
        if link.is_none() && session.coop_transport.as_deref().is_none_or(|t| t == "ICE") {
            self.ensure_verified_link(who, &derived, 1, DERIVED);
        }
        let effective = match link {
            Some(known) => known.peer.0.to_string(),
            None => derived,
        };
        let mut row = self.hub.find_presence(&session.session_id, who).await?;
        let waiting = row.as_ref().is_some_and(|r| {
            r.generation == session.generation
                && r.state.is_expiring_state()
                && !is_presence_expired(r.state, r.expires_at, now)
        });
        let may_enter = member && !banned && session.host_account_id != who && is_hub_live(session.state);
        if may_enter && !waiting {
            let at = EntryAt {
                session_id: session.session_id.clone(),
                generation: session.generation,
                realm_id,
                expires_at: entry_expires_at(now),
                now,
            };
            let request = HubEntryRequest {
                candidate_id: format!("cand-{}-{who}", session.session_id),
                peer_id: effective.clone(),
                account_id: who.into(),
            };
            let cap = session.max_peers.unwrap_or(DEFAULT_HUB_MAX_PLAYERS);
            row = match self.hub.request_entries(&as_account(who), &at, cap, &[request]).await? {
                HubEntryBatch::Placed(rows) => rows.into_iter().next().map(|(_, placed, _)| placed),
                HubEntryBatch::Full { .. } => return Err(refused("REALM_HUB_FULL", "The realm's hub is full.")),
            };
        }
        // Read STARTING, HOST_READY or ADMITTING as RUNNING: a member may queue while the host comes up.
        let mut view = session;
        if [H::Starting, H::HostReady, H::Admitting].contains(&view.state) {
            view.state = H::Running;
        }
        let check = HubTicketCheck {
            presence: row.as_ref().map(|r| (r.state, r.expires_at, r.peer_id.as_str())),
            generation: i64::try_from(hub.snapshot.generation).ok(),
            link: Some(("VERIFIED", effective.as_str())),
            is_member: member,
            is_banned: banned,
            account_id: who,
            hub: &view,
            now,
        };
        match hub_ticket_denial(&check).map(hub_ticket_denial_error) {
            Some((code, message)) => Err(refused(code, message)),
            None => Ok(PeerId(effective.parse().unwrap_or(1))),
        }
    }

    async fn mark_participant_admitted_now(
        &self,
        hunt: &AdmissionHunt,
        account: &AccountId,
        at: UnixMillis,
    ) -> Result<bool, RealmsError> {
        let id = hunt.snapshot.wire_session.clone();
        let generation = stored_generation(hunt.snapshot.generation);
        let who = account.0.clone();
        let actor = as_account(&who);
        self.hunts
            .transaction(&actor, move |tx| admit_participant(tx, &id, generation, &who, at))
            .await
    }

    async fn peer_disconnected_now(&self, admission: &PeerAdmissionRecord, at: UnixMillis) -> Result<(), RealmsError> {
        let Some(hunt) = self.hunts.find_hunt(&admission.session).await? else {
            return Ok(());
        };
        let generation = stored_generation(admission.generation);
        let who = admission.account.0.as_str();
        let (actor, id) = (as_account(who), hunt.session_id.as_str());
        if !hunt.is_realm_hub() {
            let rejoin_until = at.saturating_add(REJOIN_WINDOW_MS);
            self.hunts
                .mark_participant_disconnected(&actor, id, generation, who, at, rejoin_until)
                .await?;
            return Ok(());
        }
        let admission_id = Some(admission.admission_id.as_str());
        self.hub
            .mark_lost(&actor, id, generation, who, admission_id, at, presence_purge_at(at))
            .await?;
        self.invalidate_hub(id);
        Ok(())
    }

    async fn hub_rows(&self, wire: &str, generation: u64, limit: usize) -> Result<Vec<PresenceRow>, RealmsError> {
        // At most 256 rows a read.
        let limit = u32::try_from(limit.min(256)).unwrap_or(256);
        let rows = self
            .hub
            .list_by_state(wire, stored_generation(generation), &PresenceState::LIVE, limit)
            .await?;
        let row = |r: crate::repo::hub::HubPresenceRecord| PresenceRow {
            peer: PeerId(peer_number(&r.peer_id, &r.account_id)),
            state: relay_presence(r.state),
            account: AccountId(r.account_id),
        };
        Ok(rows.into_iter().map(row).collect())
    }

    async fn join_common_now(
        &self,
        hunt: &AdmissionHunt,
        key_id: &str,
        bridge_instance_id: &str,
    ) -> Result<Option<Json>, RealmsError> {
        let stored = self.hunts.find_hunt(&hunt.snapshot.wire_session).await?;
        let is_hub = hunt.snapshot.is_hub();
        let nonce = stored
            .as_ref()
            .and_then(|s| s.metadata_nonce.clone())
            .unwrap_or_else(|| PLACEHOLDER_NONCE.into());
        let completion = match stored.and_then(|s| s.guard_completion_mode) {
            Some(mode) => mode,
            None => String::from(if is_hub { "NO_COMPLETION" } else { "FIRST_DEFEAT" }),
        };
        let peer = hunt.snapshot.host_peer.0.to_string();
        let common = object([
            ("keyId", key_id.into()),
            ("sessionId", json!(hunt.snapshot.wire_session)),
            ("partyId", json!(hunt.party_id)),
            ("partyRevision", hunt.party_revision.into()),
            ("huntDefinitionId", json!(hunt.hunt_definition_id)),
            ("generation", hunt.snapshot.generation.into()),
            ("build", hunt.build.into()),
            ("bridgeProtocol", hunt.bridge_protocol.into()),
            ("virtualPort", hunt.virtual_port.into()),
            ("hostPeerId", json!(peer)),
            ("hostSteamId", peer.into()),
            ("bridgeInstanceId", bridge_instance_id.into()),
            ("metadataNonce", nonce.into()),
            ("forceRelay", self.settings().policy.force_sdr.into()),
            ("guardCompletionMode", completion.into()),
            ("sessionRole", (if is_hub { "HUB" } else { "HUNT" }).into()),
        ]);
        Ok(Some(Json(common.to_string())))
    }
}

impl AdmissionHunts for Hosting {
    fn join_bootstrap_common(
        &self,
        hunt: &AdmissionHunt,
        key_id: &str,
        bridge_instance_id: &str,
    ) -> Result<Option<Json>, AdmissionError> {
        blocking(self.join_common_now(hunt, key_id, bridge_instance_id)).map_err(to_admission)
    }

    fn hub_presence(&self, hub: &AdmissionHunt, limit: usize) -> Result<Vec<PresenceRow>, AdmissionError> {
        let snapshot = &hub.snapshot;
        blocking(self.hub_rows(&snapshot.wire_session, snapshot.generation, limit)).map_err(to_admission)
    }

    fn peer_disconnected(&self, admission: &PeerAdmissionRecord, at: UnixMillis) -> Result<(), AdmissionError> {
        blocking(self.peer_disconnected_now(admission, at)).map_err(to_admission)
    }

    fn admit_hub_peer(
        &self,
        hub: &AdmissionHunt,
        account: &AccountId,
        admission_id: &str,
        admitted_at: UnixMillis,
    ) -> Result<(), AdmissionError> {
        // The host's own row was admitted with its route.
        if *account == hub.snapshot.host_account {
            return Ok(());
        }
        let (wire, generation) = (&hub.snapshot.wire_session, stored_generation(hub.snapshot.generation));
        let actor = as_account(&account.0);
        let now = self.now();
        let mark = self.hub.mark_admitted(
            &actor,
            wire,
            generation,
            &account.0,
            admission_id,
            admitted_at,
            now,
            Some("ice"),
        );
        blocking(async { Ok(mark.await.map(drop)?) }).map_err(to_admission)?;
        self.invalidate_hub(wire);
        Ok(())
    }

    fn mark_participant_admitted(
        &self,
        hunt: &AdmissionHunt,
        account: &AccountId,
        admitted_at: UnixMillis,
    ) -> Result<bool, AdmissionError> {
        blocking(self.mark_participant_admitted_now(hunt, account, admitted_at)).map_err(to_admission)
    }

    fn mark_hub_ticketed(
        &self,
        hub: &AdmissionHunt,
        account: &AccountId,
        now: UnixMillis,
        ticket_expires_at: UnixMillis,
    ) -> Result<bool, AdmissionError> {
        let (wire, generation) = (&hub.snapshot.wire_session, stored_generation(hub.snapshot.generation));
        let actor = as_account(&account.0);
        let mark = self
            .hub
            .mark_ticketed(&actor, wire, generation, &account.0, now, ticket_expires_at);
        blocking(async { Ok(mark.await?.is_some()) }).map_err(to_admission)
    }

    fn check_hub_entry(
        &self,
        hub: &AdmissionHunt,
        account: &AccountId,
        link: Option<&PeerLinkRecord>,
        now: UnixMillis,
    ) -> Result<PeerId, AdmissionError> {
        // Only a settled link counts; a pending one is as good as none.
        let settled = link.filter(|known| matches!(known.status, LinkStatus::Verified | LinkStatus::Revoked));
        blocking(self.check_hub_entry_now(hub, account, settled, now)).map_err(to_admission)
    }

    fn reserve_participant(
        &self,
        hunt: &AdmissionHunt,
        account: &AccountId,
        now: UnixMillis,
    ) -> Result<Option<AdmissionHunt>, AdmissionError> {
        let (wire, generation) = (&hunt.snapshot.wire_session, stored_generation(hunt.snapshot.generation));
        let (actor, rejoin_until) = (as_account(&account.0), now.saturating_add(REJOIN_WINDOW_MS));
        let reserve = self.hunts.reserve_participant_ticket(
            &actor,
            wire,
            generation,
            &account.0,
            now,
            rejoin_until,
            MAX_RECONNECTS,
        );
        let reserved = blocking(async { Ok(reserve.await?) }).map_err(to_admission)?;
        Ok(reserved.as_ref().and_then(admission_hunt))
    }

    fn find_hunt(&self, session: &SessionId) -> Result<Option<AdmissionHunt>, AdmissionError> {
        let wire = wire_session_id(session);
        let found = blocking(async { Ok(self.hunts.find_hunt(&wire).await?) }).map_err(to_admission)?;
        Ok(found.as_ref().and_then(admission_hunt))
    }

    fn gate(&self) -> Result<P2PGate, AdmissionError> {
        blocking(self.gate_now()).map_err(to_admission)
    }
}

impl IceHunts for Hosting {
    fn enqueue(&self, message: ControlMessage) -> Result<(), RelayError> {
        let payload = serde_json::from_str::<Value>(&message.payload.0).map_err(|error| {
            RelayError::Refused(Refusal {
                code: "P2P_SIGNAL_INVALID".into(),
                message: error.to_string(),
            })
        })?;
        let record = NewControlMessage {
            session_id: Some(message.wire_session.clone()),
            generation: Some(stored_generation(message.generation)),
            account_id: message.account.0.clone(),
            message_type: message.message_type.into(),
            created_at: message.created_at,
            expires_at: message.expires_at,
            payload,
        };
        let actor = as_account(&message.account.0);
        let write = self.hunts.enqueue_control_message(&actor, &record);
        blocking(async { Ok(write.await.map(drop)?) }).map_err(to_relay)
    }

    fn hub_presence(&self, hunt: &HuntSnapshot, limit: usize) -> Result<Vec<PresenceRow>, RelayError> {
        blocking(self.hub_rows(&hunt.wire_session, hunt.generation, limit)).map_err(to_relay)
    }

    fn find_hunt(&self, session: &SessionId) -> Result<Option<HuntSnapshot>, RelayError> {
        let wire = wire_session_id(session);
        let found = blocking(async { Ok(self.hunts.find_hunt(&wire).await?) }).map_err(to_relay)?;
        Ok(found.as_ref().map(snapshot))
    }

    fn gate(&self) -> Result<P2PGate, RelayError> {
        blocking(self.gate_now()).map_err(to_relay)
    }
}

impl OperationsPolicy for Hosting {
    /// Nothing is stored (kept as found): a version past 1 is always a conflict.
    fn set_coop_transport(
        &self,
        admin_key_id: &str,
        transport: CoopTransport,
        expected_version: u64,
        reason: &str,
        request_id: &str,
    ) -> Result<CoopTransportOutcome, RelayError> {
        let stop = blocking(self.emergency_stop()).map_err(to_relay)?;
        let updated_at = iso(self.now());
        if expected_version > 1 {
            let current = object([
                ("version", 1.into()),
                ("p2pEmergencyStop", stop.into()),
                ("coopTransport", self.coop_transport_name().into()),
                ("updatedAt", updated_at.into()),
            ]);
            return Ok(CoopTransportOutcome::Conflict(Json(current.to_string())));
        }
        let policy = object([
            ("version", (expected_version + 1).into()),
            ("p2pEmergencyStop", stop.into()),
            ("coopTransport", transport.as_str().into()),
            ("updatedAt", updated_at.into()),
            ("updatedBy", admin_key_id.into()),
            ("reason", reason.into()),
            ("requestId", request_id.into()),
        ]);
        Ok(CoopTransportOutcome::Updated(Json(policy.to_string())))
    }
}

impl RealmEventSink for Hosting {
    fn emit(&self, event: RealmEvent) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            let now = self.now();
            let purge = presence_purge_at(now);
            match event {
                RealmEvent::MemberRemoved {
                    realm_id,
                    account_id,
                    reason,
                } => {
                    let removal =
                        self.hub
                            .remove_account_from_realm(&DIRECTORY_ACTOR, &realm_id, &account_id, now, purge);
                    let rows = removal.await.map_err(|error| error.to_string())?;
                    rows.iter().for_each(|row| self.invalidate_hub(&row.session_id));
                    let why = reason.as_str();
                    self.parts.log.info(&format!(
                        "[Realms] removed account {account_id} from realm {realm_id} hub presence ({why})"
                    ));
                }
                RealmEvent::RealmReleased {
                    realm_id,
                    host_account_id: host,
                } => {
                    let actor = as_account(&host);
                    let removal = self.hub.remove_realm(&actor, &realm_id, now, purge);
                    removal.await.map_err(|error| error.to_string())?;
                    let ended = ("REALM_RELEASED", H::Completed, TransitionActor::Host);
                    self.end_realm_hubs(&realm_id, ended, &host).await;
                }
                RealmEvent::HostOffline {
                    realm_id,
                    host_account_id: host,
                } => {
                    let ended = ("REALM_HOST_OFFLINE", H::Aborted, TransitionActor::System);
                    self.end_realm_hubs(&realm_id, ended, &host).await;
                }
            }
            Ok(())
        })
    }
}

impl Hosting {
    /// Hub-end errors are ignored.
    async fn end_realm_hubs(
        &self,
        realm_id: &str,
        (reason, terminal, by): (&str, HuntState, TransitionActor),
        host: &str,
    ) {
        for kind in [HubKind::Ramsgate, HubKind::Training] {
            let Ok(Some(hub)) = self.live_hub(realm_id, kind).await else {
                continue;
            };
            let _ = self
                .end_hub(&hub.session_id, hub.generation, reason, terminal, by, Some(host))
                .await;
        }
    }

    fn typed_bootstrap(
        &self,
        json: &Value,
        session: SessionId,
        default_peers: u8,
    ) -> Result<HostBootstrap, RealmsError> {
        let text = |key: &str| json.get(key).and_then(Value::as_str);
        let number = |key: &str| json.get(key).and_then(Value::as_u64);
        let Some(capability) = text("capability") else {
            return Err(RealmsError::Database(String::from("bootstrap without a capability")));
        };
        Ok(HostBootstrap {
            maximum_peers: number("maximumPeers")
                .and_then(|n| u8::try_from(n).ok())
                .unwrap_or(default_peers),
            server_udp_port: number("serverUdpPort")
                .and_then(|n| u16::try_from(n).ok())
                .unwrap_or(DEFAULT_SERVER_UDP_PORT),
            backend_url: text("backendUrl").unwrap_or_default().into(),
            host_ticket: text("hostTicket").map(|ticket| Secret::new(ticket.into())),
            capability: Secret::new(capability.into()),
            generation: number("generation").unwrap_or(1),
            session,
        })
    }
}

fn internal(message: &str) -> RealmsError {
    refused("INTERNAL", message)
}

/// A route body with the fields given.
fn body_of<const N: usize>(pairs: [(&str, Value); N]) -> Map<String, Value> {
    pairs.into_iter().map(|(key, value)| (key.to_owned(), value)).collect()
}

impl HuntLifecycle for Hosting {
    fn host_bootstrap(&self, account: AccountId, session: SessionId) -> Fut<'_, HostBootstrap> {
        Box::pin(async move {
            let wire = wire_session_id(&session);
            let made = self.create_host_bootstrap(&account.0, &wire).await?;
            self.typed_bootstrap(&made, session, 4)
        })
    }

    fn route_ready(&self, account: AccountId, session: SessionId, local_port: u16) -> Fut<'_, ()> {
        Box::pin(async move {
            let body = body_of([("localBridgePort", local_port.into())]);
            self.mark_local_route_ready(&account.0, &wire_session_id(&session), &body)
                .await
                .map(drop)
        })
    }

    fn host_ready(&self, runtime: RuntimeAuth) -> Fut<'_, ()> {
        Box::pin(async move {
            let capability = stand_in_capability(StandIn::Runtime(&runtime));
            self.mark_host_ready(&capability, &Map::new()).await.map(drop)
        })
    }

    fn abort(&self, runtime: RuntimeAuth, reason: String) -> Fut<'_, ()> {
        Box::pin(async move {
            let capability = stand_in_capability(StandIn::Runtime(&runtime));
            self.abort_hunt(&capability, &body_of([("reason", reason.into())]))
                .await
                .map(drop)
        })
    }

    fn end(&self, runtime: RuntimeAuth, reason: String) -> Fut<'_, ()> {
        Box::pin(async move {
            let capability = stand_in_capability(StandIn::Runtime(&runtime));
            self.finish_hunt(&capability, &body_of([("reason", reason.into())]))
                .await
                .map(drop)
        })
    }

    fn status(&self, account: AccountId, session: SessionId) -> Fut<'_, HuntStatus> {
        Box::pin(async move {
            let read = self
                .get_hunt_status(Some(&account.0), &wire_session_id(&session))
                .await?;
            let state = read["state"].as_str().and_then(HuntState::parse).unwrap_or(H::Running);
            Ok(HuntStatus {
                state,
                generation: read["generation"].as_u64().unwrap_or(1),
                verdict: None,
            })
        })
    }

    /// Always generation 1 (kept as found): a retried hunt refuses it.
    fn observe(&self, account: AccountId, session: SessionId, observation: Observation) -> Fut<'_, ()> {
        Box::pin(async move {
            let Observation {
                host_reachable,
                local_bridge_alive,
                route_kind,
                ..
            } = &observation;
            let body = body_of([
                ("generation", 1.into()),
                ("observedAt", iso(self.now()).into()),
                ("hostReachable", (*host_reachable).into()),
                ("localBridgeAlive", (*local_bridge_alive).into()),
                ("metricsAvailable", route_kind.is_some().into()),
                ("routeKind", route_kind.as_deref().unwrap_or("UNKNOWN").into()),
                ("lastPacketAgeMs", json!(observation.last_packet_age_ms)),
                ("pingMs", json!(observation.ping_ms)),
                ("jitterMs", json!(observation.jitter_ms)),
                ("packetLossPercent", json!(observation.packet_loss_percent)),
            ]);
            self.observe(&account.0, &wire_session_id(&session), &body)
                .await
                .map(drop)
        })
    }

    fn start_hub(&self, account: AccountId, hub: StartHub) -> Fut<'_, HostBootstrap> {
        Box::pin(async move {
            let body = body_of([
                ("hubKind", hub_kind_name(hub.kind).into()),
                ("maxPlayers", hub.max_players.into()),
                ("afkSeconds", hub.afk_seconds.into()),
            ]);
            let started = self.start_realm_hub(&account.0, &body).await?;
            let node_name = self.runtime.as_ref().map_or("local", |r| r.node.as_str());
            let node = module_api::NodeId::new(node_name).map_err(|_| internal("Invalid node id."))?;
            let wire = started["sessionId"].as_str().unwrap_or_default();
            let Some(session) = seam_session_id(wire, &node) else {
                return Err(internal("Invalid hub session id."));
            };
            self.typed_bootstrap(&started, session, hub.max_players)
        })
    }

    fn stop_hub(&self, account: AccountId, kind: HubKind) -> Fut<'_, ()> {
        Box::pin(async move {
            let body = body_of([("hubKind", hub_kind_name(kind).into())]);
            self.stop_realm_hub(&account.0, &body).await.map(drop)
        })
    }

    fn hosting_status(&self, account: AccountId) -> Fut<'_, Json> {
        Box::pin(async move {
            let status = self.get_realm_hosting_status(&account.0).await?;
            Ok(Json(status.to_string()))
        })
    }

    fn set_emergency_stop(&self, admin_key: String, policy: Json) -> Fut<'_, Json> {
        Box::pin(async move {
            let answer = self.emergency_policy(&admin_key, &policy.0)?;
            Ok(Json(answer.to_string()))
        })
    }

    fn resolve_launch_manifest(&self, request: Json) -> Fut<'_, Json> {
        Box::pin(async move {
            let manifest = Self::launch_manifest(&request.0)?;
            Ok(Json(manifest.to_string()))
        })
    }
}
