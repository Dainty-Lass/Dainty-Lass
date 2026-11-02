// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The session routes behind `Hosting`: bootstrap, host ready, route ready, abort and end, status, realm hubs and
//! their status, matchmaking, the adapters rifts-admission and rifts-relay call, the realm event sink, and the typed
//! `HuntLifecycle` facade with seam ids.

use std::future::Future;

use rifts_admission::{
    AdmissionError, AdmissionHunt, AdmissionHunts, JOIN_TICKET_LIFETIME_SECONDS, LinkStatus, PeerAdmissionRecord,
    PeerLinkRecord, SessionUuid, TicketAuthority, TicketBindings, TicketRecord, TicketStatus,
};
use rifts_relay::rules::{CoopTransport, HuntSnapshot, PresenceRow, RosterEntry, SessionKind};
use rifts_relay::{ControlMessage, CoopTransportOutcome, IceHunts, OperationsPolicy, P2PGate, RelayError};
use rifts_seam::{
    AccountId, BoxFuture, Candidate, CapabilityScope, HubKind, HuntRouteContext, HuntRouteOutcome, Json, PeerId,
    Refusal, RuntimeAuth, Secret, SessionId, StoreError, UnixMillis,
};
use serde_json::{Map, Value, json};

use crate::RealmsError;
use crate::hub::{
    DEFAULT_HUB_MAX_PLAYERS, HUB_CAPABILITY_SCOPES, HUB_SESSION_LIFETIME_MS, HubTicketCheck, PresenceState,
    RealmCapacity, RealmCapacityInput, RealmRequest, classify_realm_request, decide_realm_capacity, entry_expires_at,
    hub_manifest, hub_party_key, hub_ticket_denial, hub_ticket_denial_error, is_hub_live, is_presence_expired,
    parse_hub_start_body, parse_hub_stop_body, presence_purge_at, realm_host_ineligibility_reason,
};
use crate::hunt::{
    HostCapabilityRecord, HuntSession, HuntState, Participant, ReliabilityIncrements, TransitionActor, hub_kind_name,
};
use crate::ids::{hex, new_session_ulid, random_uuid, seam_session_id, uuid_text, wire_session_id};
use crate::leases::{Hosting, as_account, integer_in, refused, short_text};
use crate::lifecycle::{CONTROL_LIFE_MS, HOST_END_STATES, queue, transition};
use crate::ports::RealmEventSink;
use crate::realm::{HubState, RealmEvent, hub_room_id, js_trim};
use crate::repo::hub::{EntryAt, HubEntryBatch, HubEntryRequest};
use crate::repo::hunts::{HostReadyMark, HostStartup};
use crate::selection::derive_ice_peer_id;
use crate::tiers::{HuntRoute, RouteInput, decide_hunt_route};
use crate::time::{Later as _, iso};

use HuntState as H;

type Fut<'a, T> = BoxFuture<'a, Result<T, RealmsError>>;

pub const DEFAULT_GAME_BUILD: i64 = 647_472;
const REJOIN_WINDOW_MS: i64 = 180_000;
const MAX_RECONNECTS: i64 = 5;
const HUNT_LIFETIME_MS: i64 = 2 * 60 * 60 * 1000;
const HUNT_MAX_PEERS: i64 = 4;
const DEFAULT_SERVER_UDP_PORT: u16 = 8791;
const PLACEHOLDER_NONCE: &str = "0123456789abcdef0123456789abcdef";
const ROUTE_ACTOR: rifts_seam::Actor = rifts_seam::Actor::System("realms-route");
const DIRECTORY_ACTOR: rifts_seam::Actor = rifts_seam::Actor::System("realms-directory");

/// A hunt host's capability, in this order.
pub const HUNT_CAPABILITY_SCOPES: [CapabilityScope; 7] = [
    CapabilityScope::HostReady,
    CapabilityScope::LeaseHeartbeat,
    CapabilityScope::PeerAdmitted,
    CapabilityScope::EventsWrite,
    CapabilityScope::CompleteRequest,
    CapabilityScope::GameServerRuntime,
    CapabilityScope::Abort,
];

/// What the host's launcher starts a hosted server from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostBootstrap {
    pub session: SessionId,
    pub generation: u64,
    pub capability: Secret,
    pub host_ticket: Option<Secret>,
    pub backend_url: String,
    pub server_udp_port: u16,
    pub maximum_peers: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HuntStatus {
    pub state: HuntState,
    pub generation: u64,
    pub verdict: Option<Json>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    pub host_reachable: bool,
    pub local_bridge_alive: bool,
    pub route_kind: Option<String>,
    pub ping_ms: Option<u32>,
    pub jitter_ms: Option<u32>,
    pub packet_loss_percent: Option<u8>,
    pub last_packet_age_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// A hub's manifest, or the fixed placeholder every other hunt gets (kept as found).
pub fn resolve_manifest_for_hunt(hunt_definition_id: &str, expected_players: &[String]) -> Value {
    let hub = if hunt_definition_id.contains("ReturnToRamsgate") || hunt_definition_id.eq_ignore_ascii_case("CITY") {
        Some(HubKind::Ramsgate)
    } else if hunt_definition_id.contains("TrainingDojo") {
        Some(HubKind::Training)
    } else {
        None
    };
    if let Some(kind) = hub {
        let m = hub_manifest(kind);
        return json!({
            "huntDefinitionId": m.hunt_definition_id,
            "gameMode": m.game_mode,
            "mapPath": m.map_path,
            "behemothPath": m.behemoth_path,
            "matchmakerHuntId": m.matchmaker_hunt_id,
            "expectedPlayerString": m.expected_player_string,
            "guardCompletionMode": m.guard_completion_mode,
        });
    }
    let players = if expected_players.is_empty() {
        "1".to_owned()
    } else {
        expected_players.join(",")
    };
    json!({
        "huntDefinitionId": hunt_definition_id,
        "gameMode": "ISLAND",
        "mapPath": "/Game/Maps/islands/1705/dia_moss_triforce",
        "behemothPath": "Behemoth_Gnasher",
        "matchmakerHuntId": hunt_definition_id,
        "expectedPlayerString": players,
        "guardCompletionMode": "FIRST_DEFEAT",
    })
}

/// The stored peer id when it is a positive u64, else the account's derived one, else 1.
fn peer_number(stored: &str, account: &str) -> u64 {
    stored
        .parse::<u64>()
        .ok()
        .filter(|n| *n > 0)
        .or_else(|| derive_ice_peer_id(account).parse().ok())
        .unwrap_or(1)
}

fn host_peer_text(hunt: &HuntSession) -> String {
    if hunt.host_steam_id.is_empty() {
        derive_ice_peer_id(&hunt.host_account_id)
    } else {
        hunt.host_steam_id.clone()
    }
}

fn party_text(party_id: &str) -> String {
    party_id.replace('@', ":")
}

fn clamp_u16(value: i64, min: i64, max: i64) -> u16 {
    u16::try_from(value.clamp(min, max)).unwrap_or(1)
}

fn random_nonce() -> String {
    let bytes: [u8; 16] = rand::random();
    hex(&bytes)
}

fn relay_state(state: HuntState) -> rifts_relay::rules::HuntState {
    use rifts_relay::rules::HuntState as R;
    match state {
        H::Requested => R::Requested,
        H::HostSelecting => R::HostSelecting,
        H::HostOffered => R::HostOffered,
        H::Starting => R::Starting,
        H::HostReady => R::HostReady,
        H::Admitting => R::Admitting,
        H::Running => R::Running,
        H::Draining => R::Draining,
        H::Suspect => R::Suspect,
        H::Fenced => R::Fenced,
        H::Aborting => R::Aborting,
        H::Recovering => R::Recovering,
        H::Completed => R::Completed,
        H::Aborted => R::Aborted,
    }
}

fn relay_presence(state: PresenceState) -> rifts_relay::rules::PresenceState {
    use rifts_relay::rules::PresenceState as R;
    match state {
        PresenceState::Pending => R::Pending,
        PresenceState::Ticketed => R::Ticketed,
        PresenceState::Admitted => R::Admitted,
        PresenceState::Lost => R::Lost,
        PresenceState::Left => R::Left,
        PresenceState::Removed => R::Removed,
        PresenceState::Expired => R::Expired,
    }
}

fn snapshot(hunt: &HuntSession) -> HuntSnapshot {
    HuntSnapshot {
        wire_session: hunt.session_id.clone(),
        generation: u64::try_from(hunt.generation.max(1)).unwrap_or(1),
        state: relay_state(hunt.state),
        kind: if hunt.is_realm_hub() {
            SessionKind::RealmHub
        } else {
            SessionKind::Hunt
        },
        realm: hunt.realm_id.clone(),
        coop_transport: hunt
            .coop_transport
            .as_deref()
            .and_then(CoopTransport::parse)
            .unwrap_or(CoopTransport::Ice),
        host_account: AccountId(hunt.host_account_id.clone()),
        host_peer: PeerId(peer_number(&hunt.host_steam_id, &hunt.host_account_id)),
        expires_at: hunt.expires_at,
        roster: hunt
            .roster
            .iter()
            .map(|p| RosterEntry {
                account: AccountId(p.account_id.clone()),
                peer: PeerId(peer_number(&p.steam_id, &p.account_id)),
                returned_to_hub: p.returned_to_hub_at.is_some(),
            })
            .collect(),
    }
}

fn admission_hunt(hunt: &HuntSession) -> Option<AdmissionHunt> {
    let session = hunt.session_id.parse::<SessionUuid>().ok()?;
    let lobby = hunt.lobby_id.clone().or_else(|| {
        matches!(hunt.state, H::Starting | H::HostReady | H::Admitting | H::Running).then(|| "ICE".to_owned())
    });
    Some(AdmissionHunt {
        snapshot: snapshot(hunt),
        session,
        party_id: party_text(&hunt.party_id),
        party_revision: u64::try_from(hunt.party_revision.max(1)).unwrap_or(1),
        hunt_definition_id: hunt.hunt_definition_id.clone(),
        build: u32::try_from(hunt.build.max(1)).unwrap_or(647_472),
        bridge_protocol: clamp_u16(hunt.bridge_protocol, 1, 255),
        virtual_port: clamp_u16(hunt.virtual_port, 1, 65_535),
        lobby_id: lobby,
        bridge_instance_id: (!hunt.bridge_instance_id.is_empty()).then(|| hunt.bridge_instance_id.clone()),
    })
}

fn candidate_json(fields: &[(&str, Value)]) -> Candidate {
    let mut map = Map::new();
    for (key, value) in fields {
        map.insert((*key).to_owned(), value.clone());
    }
    Candidate(Json(Value::Object(map).to_string()))
}

/// The matchmaker's view of where an account's game connects.
fn hunt_candidate(hunt: &HuntSession, candidate_id: &str, ready: bool, port: i64) -> Candidate {
    candidate_json(&[
        ("HuntId", json!(hunt.hunt_definition_id)),
        ("CandidateId", json!(candidate_id)),
        ("GameSessionId", json!(hunt.session_id)),
        ("Host", json!(if ready { "127.0.0.1" } else { "" })),
        ("Port", json!(port)),
        ("Ready", json!(ready)),
        ("ReadyAt", json!(0)),
    ])
}

fn hub_candidate(hub: &HuntSession, candidate_id: &str, ready: bool, port: i64) -> Candidate {
    let game_session = hub.hub_room_id.clone().unwrap_or_else(|| hub.session_id.clone());
    candidate_json(&[
        ("HuntId", json!(hub.hunt_definition_id)),
        ("CandidateId", json!(candidate_id)),
        ("GameSessionId", json!(game_session)),
        ("Host", json!(if ready { "127.0.0.1" } else { "" })),
        ("Port", json!(port)),
        ("Ready", json!(ready)),
        ("ReadyAt", json!(0)),
        ("Source", json!("REALM_HUB")),
        ("SourceSessionId", json!(hub.session_id)),
    ])
}

fn participant_candidate(hunt: &HuntSession, p: &Participant) -> Candidate {
    let ready = p.local_bridge_port.is_some() && p.local_bridge_ready_at.is_some();
    hunt_candidate(hunt, &p.candidate_id, ready, p.local_bridge_port.unwrap_or(0))
}

fn capability_of_runtime(runtime: &RuntimeAuth) -> HostCapabilityRecord {
    HostCapabilityRecord {
        capability_id: runtime.capability_id.clone(),
        secret_digest: String::new(),
        session_id: runtime
            .wire_session
            .clone()
            .unwrap_or_else(|| wire_session_id(&runtime.session)),
        generation: i64::try_from(runtime.generation).unwrap_or(i64::MAX),
        host_account_id: runtime.host_account.0.clone(),
        host_steam_id: runtime.host_peer.map(|p| p.0.to_string()).unwrap_or_default(),
        bridge_instance_id: runtime.bridge_instance.clone().unwrap_or_default(),
        scopes: runtime.scopes.clone(),
        issued_at: UnixMillis(0),
        expires_at: UnixMillis(i64::MAX),
        revoked_at: None,
        last_sequence: 0,
    }
}

impl Hosting {
    fn set_candidate(&self, account: &str, candidate: Candidate) {
        if let Some(runtime) = &self.runtime {
            let _ = runtime.core.set_candidate(AccountId(account.to_owned()), candidate);
        }
    }

    /// For a host that also plays: a ticket of its own; any failure is no ticket.
    fn self_ticket(&self, hunt: &HuntSession, account: &str) -> Option<Secret> {
        let runtime = self.runtime.as_ref()?;
        let session = hunt.session_id.parse::<SessionUuid>().ok()?;
        let joiner = peer_number(hunt.participant(account).map_or("", |p| p.steam_id.as_str()), account);
        let host_peer = peer_number(&hunt.host_steam_id, &hunt.host_account_id);
        let bindings = TicketBindings {
            account: AccountId(account.to_owned()),
            joiner_peer: PeerId(joiner),
            host_peer: PeerId(host_peer),
            session,
            generation: u32::try_from(hunt.generation.max(1)).ok()?,
            party_id: party_text(&hunt.party_id),
            party_revision: u64::try_from(hunt.party_revision.max(1)).ok()?,
            hunt_definition_id: hunt.hunt_definition_id.clone(),
            build: u32::try_from(hunt.build.max(1)).unwrap_or(647_472),
            bridge_protocol: clamp_u16(hunt.bridge_protocol, 1, 255),
            virtual_port: clamp_u16(hunt.virtual_port, 1, 65_535),
        };
        let minted = TicketAuthority::mint(
            runtime.tickets.as_ref(),
            bindings,
            self.now(),
            Some(JOIN_TICKET_LIFETIME_SECONDS),
        )
        .ok()?;
        let seconds = |s: u64| i64::try_from(s).unwrap_or(i64::MAX).saturating_mul(1000);
        let claims = &minted.claims;
        let record = TicketRecord {
            jti_digest: minted.jti_digest.clone(),
            key_id: claims.key_id.0.clone(),
            session: hunt.session_id.clone(),
            generation: u64::from(claims.bindings.generation),
            account: AccountId(account.to_owned()),
            joiner_peer: claims.bindings.joiner_peer,
            host_peer: claims.bindings.host_peer,
            bridge_protocol: claims.bindings.bridge_protocol,
            virtual_port: claims.bindings.virtual_port,
            issued_at: UnixMillis(seconds(claims.issued_at)),
            expires_at: UnixMillis(seconds(claims.expires_at)),
            status: TicketStatus::Issued,
            consumed_at: None,
            consumer_bridge_instance_id: None,
            consumer_request_id: None,
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
        let roster: Vec<String> = hunt.roster.iter().map(|p| p.account_id.clone()).collect();
        let manifest = resolve_manifest_for_hunt(&hunt.hunt_definition_id, &roster);
        let peer = host_peer_text(hunt);
        let completion = hunt.guard_completion_mode.clone().unwrap_or_else(|| {
            manifest
                .get("guardCompletionMode")
                .and_then(Value::as_str)
                .unwrap_or("FIRST_DEFEAT")
                .to_owned()
        });
        let runtime = self.runtime.as_ref();
        let server_udp_port = hunt
            .server_udp_port
            .and_then(|p| u16::try_from(p).ok())
            .or_else(|| runtime.map(|r| r.host_server_udp_port))
            .unwrap_or(DEFAULT_SERVER_UDP_PORT);
        let maximum_peers = hunt.max_peers.map(|n| n.clamp(1, 64)).unwrap_or(if is_hub {
            DEFAULT_HUB_MAX_PLAYERS
        } else {
            HUNT_MAX_PEERS
        });
        let players = manifest.get("expectedPlayerString").cloned().unwrap_or(Value::Null);
        json!({
            "keyId": runtime.map_or_else(|| "local-key-1".to_owned(), |r| r.tickets.key_id.0.clone()),
            "sessionId": hunt.session_id,
            "partyId": party_text(&hunt.party_id),
            "partyRevision": hunt.party_revision.max(1),
            "huntDefinitionId": hunt.hunt_definition_id,
            "generation": hunt.generation.max(1),
            "build": hunt.build.max(1),
            "bridgeProtocol": hunt.bridge_protocol.clamp(1, 255),
            "virtualPort": hunt.virtual_port.clamp(1, 65_535),
            "hostPeerId": peer,
            "hostSteamId": peer,
            "bridgeInstanceId": hunt.bridge_instance_id,
            "metadataNonce": hunt.metadata_nonce.clone().unwrap_or_else(|| PLACEHOLDER_NONCE.to_owned()),
            "forceRelay": self.settings().policy.force_sdr,
            "guardCompletionMode": completion,
            "sessionRole": if is_hub { "HUB" } else { "HUNT" },
            "hubKind": hunt.hub_kind.map(hub_kind_name),
            "capability": capability.expose(),
            "hostTicket": host_ticket.map(Secret::expose),
            "backendUrl": runtime.map_or_else(|| "http://127.0.0.1:8080".to_owned(), |r| r.backend_url.clone()),
            "serverUdpPort": server_udp_port,
            "maximumPeers": maximum_peers,
            "hostLocalClient": hunt.host_participates.unwrap_or(true),
            "huntEvidence": !is_hub,
            "matchmakerHuntId": manifest.get("matchmakerHuntId").cloned().unwrap_or(Value::Null),
            "mapPath": manifest.get("mapPath").cloned().unwrap_or(Value::Null),
            "behemothPath": manifest.get("behemothPath").cloned().unwrap_or(Value::Null),
            "expectedPlayers": players.clone(),
            "expectedPlayerString": players,
            "runtimeProfile": "production",
            "maxPlayers": if is_hub { json!(maximum_peers) } else { Value::Null },
            "hubAfkSeconds": hub_afk_seconds,
        })
    }

    /// Node's `CreateHostBootstrap`.
    pub async fn create_host_bootstrap(&self, account: &str, raw_session_id: &str) -> Result<Value, RealmsError> {
        if !self.settings().policy.enabled {
            return Err(refused("P2P_DISABLED", "Player-hosted hunts are disabled."));
        }
        if self.emergency_stop().await? {
            return Err(refused(
                "P2P_EMERGENCY_STOP",
                "Player-hosted hunts are stopped by operations policy.",
            ));
        }
        let Some(runtime) = self.runtime.clone() else {
            return Err(refused("P2P_DISABLED", "Host bootstrap runtime is not configured."));
        };
        let hunt = self
            .find_by_path(raw_session_id)
            .await?
            .ok_or_else(|| refused("P2P_SESSION_NOT_FOUND", "Hunt session not found."))?;
        if hunt.host_account_id != account {
            return Err(refused(
                "P2P_NOT_ROSTERED",
                "Only the selected host may bootstrap this session.",
            ));
        }
        let now = self.now();
        let bridge = if hunt.bridge_instance_id.is_empty() || hunt.state == H::HostOffered {
            format!("bridge-{}", random_uuid())
        } else {
            hunt.bridge_instance_id.clone()
        };
        let nonce = hunt.metadata_nonce.clone().unwrap_or_else(random_nonce);
        let deadline = now.saturating_add(self.settings().eligibility.startup_lifetime_ms);
        let hunt = match hunt.state {
            H::HostOffered => {
                let startup = HostStartup {
                    session_id: hunt.session_id.clone(),
                    host_account_id: account.to_owned(),
                    bridge_instance_id: bridge,
                    metadata_nonce: nonce,
                    startup_deadline: deadline,
                    server_udp_port: hunt.server_udp_port.or(Some(i64::from(runtime.host_server_udp_port))),
                };
                let who = account.to_owned();
                self.hunts
                    .transaction(&as_account(account), move |tx| -> Result<HuntSession, RealmsError> {
                        let started = tx.begin_host_startup(&startup, now)?.ok_or_else(|| {
                            refused(
                                "P2P_GENERATION_FENCED",
                                "The session is no longer waiting for host startup.",
                            )
                        })?;
                        transition(
                            tx,
                            H::HostOffered,
                            &started,
                            "HOST_BOOTSTRAP_ISSUED",
                            TransitionActor::Host,
                            Some(&who),
                            now,
                        )?;
                        Ok(started)
                    })
                    .await?
            }
            // A repeated bootstrap.
            H::Starting | H::HostReady | H::Admitting | H::Running => hunt,
            _ => {
                return Err(refused(
                    "P2P_GENERATION_FENCED",
                    "This session generation cannot be bootstrapped.",
                ));
            }
        };
        let peer = host_peer_text(&hunt);
        self.ensure_verified_link(account, &peer, 1, "ICE_ACCOUNT_DERIVED");
        let (scopes, expires) = if hunt.is_realm_hub() {
            (
                HUB_CAPABILITY_SCOPES.to_vec(),
                now.saturating_add(self.settings().realm_hub.capability_ttl_ms),
            )
        } else {
            (HUNT_CAPABILITY_SCOPES.to_vec(), hunt.expires_at)
        };
        let (secret, _) = runtime
            .capabilities
            .mint(&hunt, &hunt.bridge_instance_id, scopes, now, expires)
            .await?;
        let ticket = if hunt.host_participates.unwrap_or(true) {
            self.self_ticket(&hunt, account)
        } else {
            None
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
            return Err(refused(
                "P2P_EMERGENCY_STOP",
                "Player-hosted hunt operations are temporarily stopped.",
            ));
        }
        let hunt = self
            .hunts
            .find_hunt(&capability.session_id)
            .await?
            .ok_or_else(|| refused("P2P_SESSION_NOT_FOUND", "Hunt session not found."))?;
        if hunt.generation != capability.generation {
            return Err(refused(
                "P2P_GENERATION_FENCED",
                "The host capability belongs to a stale generation.",
            ));
        }
        let now = self.now();
        let lease_life = self.settings().eligibility.lease_lifetime_ms;
        let answer = |h: &HuntSession, lease: UnixMillis| {
            json!({
                "ready": true,
                "sessionId": h.session_id,
                "generation": h.generation,
                "state": h.state.as_str(),
                "leaseExpiresAt": iso(lease),
            })
        };
        if matches!(hunt.state, H::HostReady | H::Admitting | H::Running) {
            let lease = hunt.lease_expires_at.unwrap_or(now.saturating_add(lease_life));
            return Ok(answer(&hunt, lease));
        }
        let lobby = short_text(body.get("lobbyId"), 64).unwrap_or("ICE").to_owned();
        let virtual_port = integer_in(body.get("virtualPort"), 1, 65_535).unwrap_or(hunt.virtual_port);
        let lease = now.saturating_add(lease_life);
        let mark = HostReadyMark {
            session_id: capability.session_id.clone(),
            generation: capability.generation,
            host_steam_id: capability.host_steam_id.clone(),
            bridge_instance_id: capability.bridge_instance_id.clone(),
            lobby_id: lobby,
            virtual_port,
            lease_expires_at: lease,
        };
        let host = capability.host_account_id.clone();
        let ready = self
            .hunts
            .transaction(&as_account(&host), move |tx| -> Result<HuntSession, RealmsError> {
                let previous = tx.find_hunt(&mark.session_id)?.map(|h| h.state);
                let ready = tx.mark_host_ready(&mark, now)?.ok_or_else(|| {
                    refused(
                        "P2P_GENERATION_FENCED",
                        "The hunt generation is not waiting for host readiness.",
                    )
                })?;
                if previous != Some(H::HostReady) {
                    transition(
                        tx,
                        previous.unwrap_or(H::Starting),
                        &ready,
                        "HOST_READY",
                        TransitionActor::Host,
                        Some(&host),
                        now,
                    )?;
                    let add = ReliabilityIncrements {
                        starts_succeeded: 1,
                        ..ReliabilityIncrements::default()
                    };
                    tx.increment_reliability(&host, add, now, None)?;
                    if !ready.is_realm_hub() {
                        for p in ready.roster.iter().filter(|p| p.account_id != host) {
                            let payload = json!({
                                "sessionId": ready.session_id,
                                "generation": ready.generation,
                                "role": "JOIN",
                            });
                            queue(
                                tx,
                                &p.account_id,
                                "JOIN_BOOTSTRAP",
                                &ready,
                                payload,
                                now,
                                CONTROL_LIFE_MS,
                            )?;
                        }
                    }
                }
                Ok(ready)
            })
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
        let port = integer_in(body.get("localBridgePort").or_else(|| body.get("localPort")), 1, 65_535)
            .ok_or_else(|| refused("P2P_TICKET_INVALID", "localBridgePort must be between 1 and 65535."))?;
        let hunt = self
            .find_by_path(raw_session_id)
            .await?
            .ok_or_else(|| refused("P2P_SESSION_NOT_FOUND", "Hunt session not found."))?;
        if let Some(generation) = body.get("generation").and_then(Value::as_i64)
            && generation != hunt.generation
        {
            return Err(refused(
                "P2P_GENERATION_FENCED",
                "The route-ready report belongs to a stale generation.",
            ));
        }
        let now = self.now();
        let answer = |h: &HuntSession| {
            json!({
                "ready": true,
                "sessionId": h.session_id,
                "generation": h.generation,
                "localBridgePort": port,
                "state": h.state.as_str(),
            })
        };
        if hunt.is_realm_hub() {
            let Some(realm) = hunt.realm_id.clone() else {
                return Err(refused("P2P_SESSION_NOT_FOUND", "Hub session has no realm."));
            };
            if hunt.host_account_id == account {
                let lease_life = self.settings().eligibility.lease_lifetime_ms;
                let (id, generation, version, from) =
                    (hunt.session_id.clone(), hunt.generation, hunt.state_version, hunt.state);
                let who = account.to_owned();
                let running = self
                    .hunts
                    .transaction(&as_account(account), move |tx| -> Result<HuntSession, RealmsError> {
                        let allowed = [H::Starting, H::HostReady, H::Admitting, H::Running];
                        let running = tx
                            .compare_and_set(&id, generation, &allowed, version, |h| {
                                h.state = H::Running;
                                h.host_local_bridge_port = Some(port);
                                h.host_local_bridge_ready_at = Some(now);
                                if h.lobby_id.is_none() {
                                    h.lobby_id = Some("ICE".to_owned());
                                }
                                if h.lease_expires_at.is_none() {
                                    h.lease_expires_at = Some(now.saturating_add(lease_life));
                                    h.last_heartbeat_at = Some(now);
                                    h.last_lease_sequence = Some(0);
                                }
                            })?
                            .ok_or_else(|| refused("P2P_GENERATION_FENCED", "Hub session is not active."))?;
                        if from != H::Running {
                            transition(
                                tx,
                                from,
                                &running,
                                "HUB_HOST_ROUTE_READY",
                                TransitionActor::Host,
                                Some(&who),
                                now,
                            )?;
                        }
                        Ok(running)
                    })
                    .await?;
                let peer = host_peer_text(&running);
                self.hub
                    .admit_host(
                        &as_account(account),
                        &running.session_id,
                        running.generation,
                        &realm,
                        account,
                        &peer,
                        port,
                        now,
                    )
                    .await?;
                self.invalidate_hub(&running.session_id);
                self.mirror_hub(&running, HubState::Running).await;
                let candidate_id = format!("cand-{}-{account}", running.session_id);
                self.set_candidate(account, hub_candidate(&running, &candidate_id, true, port));
                return Ok(answer(&running));
            }
            let row = self
                .hub
                .mark_route_ready(
                    &as_account(account),
                    &hunt.session_id,
                    hunt.generation,
                    account,
                    port,
                    now,
                    now,
                )
                .await?
                .ok_or_else(|| {
                    refused(
                        "P2P_NOT_ROSTERED",
                        "This account has no active entry into the realm hub.",
                    )
                })?;
            self.invalidate_hub(&hunt.session_id);
            self.set_candidate(account, hub_candidate(&hunt, &row.candidate_id, true, port));
            return Ok(answer(&hunt));
        }
        let mut hunt = hunt;
        // A host whose transport reports route-ready while still STARTING is ready first.
        if hunt.host_account_id == account && hunt.state == H::Starting {
            let synthetic = HostCapabilityRecord {
                capability_id: String::new(),
                secret_digest: String::new(),
                session_id: hunt.session_id.clone(),
                generation: hunt.generation,
                host_account_id: hunt.host_account_id.clone(),
                host_steam_id: hunt.host_steam_id.clone(),
                bridge_instance_id: hunt.bridge_instance_id.clone(),
                scopes: Vec::new(),
                issued_at: now,
                expires_at: hunt.expires_at,
                revoked_at: None,
                last_sequence: 0,
            };
            self.mark_host_ready(&synthetic, &Map::new()).await?;
            hunt = self
                .hunts
                .find_hunt(&hunt.session_id)
                .await?
                .ok_or_else(|| refused("P2P_SESSION_NOT_FOUND", "Hunt session not found."))?;
        }
        let participant = hunt
            .participant(account)
            .cloned()
            .ok_or_else(|| refused("P2P_NOT_ROSTERED", "This account is not in the hunt roster."))?;
        let bridge = short_text(body.get("bridgeInstanceId"), 96)
            .map(str::to_owned)
            .or_else(|| participant.local_bridge_instance_id.clone())
            .unwrap_or_else(|| format!("local-{}", random_uuid()));
        let updated = self
            .hunts
            .mark_participant_route_ready(
                &as_account(account),
                &hunt.session_id,
                hunt.generation,
                account,
                &participant.steam_id,
                &bridge,
                port,
                now,
            )
            .await?
            .ok_or_else(|| {
                refused(
                    "P2P_GENERATION_FENCED",
                    "The hunt generation does not accept route updates.",
                )
            })?;
        self.set_candidate(account, hunt_candidate(&updated, &participant.candidate_id, true, port));
        Ok(answer(&updated))
    }

    async fn end_by_host(
        &self,
        capability: &HostCapabilityRecord,
        body: &Map<String, Value>,
        default_reason: &str,
        terminal: HuntState,
        allowed: &[HuntState],
    ) -> Result<(HuntSession, Option<HuntSession>), RealmsError> {
        let reason = short_text(body.get("reason"), 128).unwrap_or(default_reason).to_owned();
        let hunt = self
            .hunts
            .find_hunt(&capability.session_id)
            .await?
            .ok_or_else(|| refused("P2P_SESSION_NOT_FOUND", "Hunt session not found."))?;
        if hunt.generation != capability.generation {
            return Err(refused("P2P_GENERATION_FENCED", "Stale hunt generation."));
        }
        let host = capability.host_account_id.clone();
        let ended = if hunt.is_realm_hub() {
            self.end_hub(
                &hunt.session_id,
                hunt.generation,
                &reason,
                terminal,
                TransitionActor::Host,
                Some(&host),
            )
            .await?
        } else {
            self.end_hunt(
                &as_account(&host),
                &hunt,
                &reason,
                allowed,
                TransitionActor::Host,
                Some(&host),
                true,
            )
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
        let allowed = [
            H::Starting,
            H::HostReady,
            H::Admitting,
            H::Running,
            H::Draining,
            H::Suspect,
        ];
        let (read, ended) = self
            .end_by_host(capability, body, "HOST_ABORTED", H::Aborted, &allowed)
            .await?;
        let state = ended.as_ref().map_or(read.state, |e| e.state);
        Ok(json!({
            "aborted": true,
            "sessionId": read.session_id,
            "generation": read.generation,
            "state": state.as_str(),
        }))
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
        let state = ended.as_ref().map_or(read.state, |e| e.state);
        Ok(json!({
            "ended": true,
            "sessionId": read.session_id,
            "generation": read.generation,
            "state": state.as_str(),
        }))
    }

    /// Node's `GetHuntStatus`: a caller must be part of the session; none is an admin or a capability.
    pub async fn get_hunt_status(&self, caller: Option<&str>, raw_session_id: &str) -> Result<Value, RealmsError> {
        let hunt = self
            .find_by_path(raw_session_id)
            .await?
            .ok_or_else(|| refused("P2P_SESSION_NOT_FOUND", "Hunt session not found."))?;
        if let Some(account) = caller {
            let part = hunt.host_account_id == account
                || hunt.participant(account).is_some()
                || (hunt.is_realm_hub() && self.hub.find_presence(&hunt.session_id, account).await?.is_some());
            if !part {
                return Err(refused("P2P_NOT_ROSTERED", "You are not part of this session."));
            }
        }
        Ok(json!({
            "sessionId": hunt.session_id,
            "generation": hunt.generation,
            "state": hunt.state.as_str(),
            "hostAccountId": hunt.host_account_id,
            "partyId": hunt.party_id,
            "huntDefinitionId": hunt.hunt_definition_id,
            "completionReason": hunt.completion_reason,
            "failureReason": hunt.failure_reason,
        }))
    }

    async fn live_hub(&self, realm_id: &str, kind: HubKind) -> Result<Option<HuntSession>, RealmsError> {
        Ok(self
            .hunts
            .find_active_hunt_for_party(&hub_party_key(realm_id, kind))
            .await?
            .filter(|h| h.is_realm_hub() && is_hub_live(h.state)))
    }

    /// Node's `CreateRealmHubSession`.
    pub async fn start_realm_hub(&self, account: &str, body: &Map<String, Value>) -> Result<Value, RealmsError> {
        let settings = self.settings();
        if !settings.realm_hosting || !settings.policy.enabled {
            return Err(refused(
                "REALM_HOSTING_DISABLED",
                "Realm hosting is not enabled on this server.",
            ));
        }
        if self.emergency_stop().await? {
            return Err(refused(
                "P2P_EMERGENCY_STOP",
                "Player-hosted operations are temporarily stopped.",
            ));
        }
        let Some(runtime) = self.runtime.clone() else {
            return Err(refused(
                "REALM_HOSTING_DISABLED",
                "Realm hosting runtime is not configured.",
            ));
        };
        let (kind, max_players) = parse_hub_start_body(&Value::Object(body.clone()))
            .map_err(|message| refused("REALM_VALIDATION_FAILED", message))?;
        let afk = integer_in(body.get("afkSeconds").or_else(|| body.get("hubAfkSeconds")), 0, 7200).unwrap_or(0);
        let realm = self
            .realms
            .find_by_host(account)
            .await?
            .ok_or_else(|| refused("REALM_NOT_FOUND", "You do not host an active realm."))?;
        if let Some(live) = self.live_hub(&realm.realm_id, kind).await? {
            self.end_hub(
                &live.session_id,
                live.generation,
                "SUPERSEDED_BY_HOST",
                H::Completed,
                TransitionActor::Host,
                Some(account),
            )
            .await?;
        }
        let active = self.hunts.count_active_realm_servers(&realm.realm_id).await?;
        let max = realm
            .hosting
            .as_ref()
            .map_or(settings.realm_hub.default_max_servers, |h| h.max_servers);
        if active >= max {
            return Err(refused(
                "REALM_SERVER_LIMIT",
                "Your realm has reached its active server limit.",
            ));
        }
        let now = self.now();
        let age = settings.eligibility.presence_maximum_age_ms;
        let presence = self
            .hunts
            .list_host_presences(&[account.to_owned()], UnixMillis(now.0 - age), now)
            .await?
            .into_iter()
            .next();
        // Without a fresh presence the start goes ahead unchecked.
        if let Some(presence) = presence {
            let guard = self.parts.operations.guard_refuses();
            if let Some(reason) =
                realm_host_ineligibility_reason(&presence, &settings.eligibility, 0.0, 0.0, guard, now)
            {
                return Err(refused(
                    "P2P_HOST_INELIGIBLE",
                    &format!("Host presence is ineligible ({reason})."),
                ));
            }
            let capacity = decide_realm_capacity(&RealmCapacityInput {
                active_servers: active,
                max_servers: max,
                free_physical_memory_mib: presence.free_physical_memory_mib,
                commit_headroom_mib: presence.commit_headroom_mib,
                minimum_free_memory_mib: settings.realm_hub.minimum_free_memory_mib,
                minimum_commit_headroom_mib: settings.realm_hub.minimum_commit_headroom_mib,
            });
            if matches!(capacity, RealmCapacity::ServerLimit | RealmCapacity::CommitLow) {
                return Err(refused(
                    "REALM_CAPACITY_EXCEEDED",
                    "Insufficient host capacity to start another realm server.",
                ));
            }
        }
        let manifest = hub_manifest(kind);
        let party = hub_party_key(&realm.realm_id, kind);
        let peer = derive_ice_peer_id(account);
        self.ensure_verified_link(account, &peer, 1, "ICE_ACCOUNT_DERIVED");
        let udp = i64::from(runtime.host_server_udp_port) + i64::from(kind == HubKind::Training);
        let hub = HuntSession {
            session_id: uuid_text(new_session_ulid(now)),
            generation: 1,
            state: H::Starting,
            state_version: 1,
            party_id: party.clone(),
            active_party_key: Some(party),
            party_revision: 1,
            hunt_definition_id: manifest.hunt_definition_id.to_owned(),
            game_mode: Some(manifest.game_mode.to_owned()),
            build: DEFAULT_GAME_BUILD,
            bridge_protocol: 1,
            virtual_port: 1,
            host_account_id: account.to_owned(),
            host_steam_id: peer,
            bridge_instance_id: format!("bridge-{}", random_uuid()),
            roster: Vec::new(),
            lobby_id: Some("ICE".to_owned()),
            coop_transport: Some("ICE".to_owned()),
            lease_expires_at: Some(now.saturating_add(settings.eligibility.lease_lifetime_ms)),
            created_at: now,
            expires_at: now.saturating_add(HUB_SESSION_LIFETIME_MS),
            metadata_nonce: Some(random_nonce()),
            hub_kind: Some(kind),
            realm_id: Some(realm.realm_id.clone()),
            host_participates: Some(true),
            host_local_bridge_port: None,
            server_udp_port: Some(udp),
            host_local_bridge_ready_at: None,
            hub_room_id: Some(hub_room_id(&realm.realm_id, kind)),
            max_peers: Some(max_players),
            guard_completion_mode: Some(manifest.guard_completion_mode.to_owned()),
            host_offer_id: None,
            offer_expires_at: None,
            startup_deadline: Some(now.saturating_add(settings.eligibility.startup_lifetime_ms)),
            last_heartbeat_at: Some(now),
            last_lease_sequence: Some(0),
            suspect_since: None,
            suspect_deadline: None,
            pre_suspect_state: None,
            failure_reason: None,
            tried_host_account_ids: vec![account.to_owned()],
            terminal_at: None,
            recovery_deadline: None,
            recovery_retry_at: None,
            completion_reason: None,
            drain_deadline: None,
            cancelled_by_account_id: None,
            trust_tier: Some(settings.policy.trust_tier),
            guard_encounter_state: None,
            extra: Map::new(),
        };
        self.hunts.create_hunt(&as_account(account), &hub).await?;
        self.mirror_hub(&hub, HubState::Starting).await;
        let expires = now.saturating_add(settings.realm_hub.capability_ttl_ms);
        let (secret, _) = runtime
            .capabilities
            .mint(
                &hub,
                &hub.bridge_instance_id,
                HUB_CAPABILITY_SCOPES.to_vec(),
                now,
                expires,
            )
            .await?;
        let ticket = self.self_ticket(&hub, account);
        Ok(self.bootstrap_json(&hub, &secret, ticket.as_ref(), Some(afk)))
    }

    /// Node's `StopRealmHubSession`.
    pub async fn stop_realm_hub(&self, account: &str, body: &Map<String, Value>) -> Result<Value, RealmsError> {
        let kind = parse_hub_stop_body(&Value::Object(body.clone()))
            .map_err(|message| refused("REALM_VALIDATION_FAILED", message))?;
        let realm = self
            .realms
            .find_by_host(account)
            .await?
            .ok_or_else(|| refused("REALM_NOT_FOUND", "You do not host an active realm."))?;
        let Some(live) = self.live_hub(&realm.realm_id, kind).await? else {
            return Ok(json!({ "stopped": false, "hubKind": hub_kind_name(kind), "sessionId": null }));
        };
        let ended = self
            .end_hub(
                &live.session_id,
                live.generation,
                "HOST_STOPPED_HUB",
                H::Completed,
                TransitionActor::Host,
                Some(account),
            )
            .await?;
        Ok(json!({ "stopped": ended.is_some(), "hubKind": hub_kind_name(kind), "sessionId": live.session_id }))
    }

    async fn hub_status(&self, realm_id: &str, kind: HubKind) -> Result<Value, RealmsError> {
        let Some(hub) = self.live_hub(realm_id, kind).await? else {
            return Ok(Value::Null);
        };
        let admitted = self
            .hub
            .count_by_state(&hub.session_id, hub.generation, &[PresenceState::Admitted])
            .await?;
        Ok(json!({
            "sessionId": hub.session_id,
            "generation": hub.generation,
            "state": hub.state.as_str(),
            "admittedPlayers": admitted,
            "maxPlayers": hub.max_peers.unwrap_or(DEFAULT_HUB_MAX_PLAYERS),
        }))
    }

    /// Node's `GetRealmHostingStatus`.
    pub async fn get_realm_hosting_status(&self, account: &str) -> Result<Value, RealmsError> {
        let enabled = self.settings().realm_hosting && self.settings().policy.enabled;
        let Some(realm) = self.realms.find_by_host(account).await? else {
            return Ok(json!({
                "enabled": enabled,
                "realmId": null,
                "realmName": null,
                "hosting": null,
                "ramsgate": null,
                "training": null,
                "activeHunts": [],
            }));
        };
        let hosting = realm.hosting.as_ref().map(|h| {
            json!({
                "maxServers": h.max_servers,
                "maxPlayers": h.max_players,
                "afkSeconds": h.afk_seconds,
                "hubSessionId": h.hub_session_id,
                "hubState": h.hub_state.as_str(),
                "trainingState": h.training_state.as_str(),
                "activeServers": h.active_servers,
                "updatedAt": iso(h.updated_at),
            })
        });
        let hunts: Vec<Value> = self
            .hunts
            .list_live_realm_hunts(&realm.realm_id, account)
            .await?
            .into_iter()
            .filter(|h| !h.is_realm_hub())
            .map(|h| {
                json!({
                    "sessionId": h.session_id,
                    "generation": h.generation,
                    "state": h.state.as_str(),
                    "huntDefinitionId": h.hunt_definition_id,
                    "partySize": h.roster.len(),
                })
            })
            .collect();
        Ok(json!({
            "enabled": enabled,
            "realmId": realm.realm_id,
            "realmName": realm.name,
            "hosting": hosting,
            "ramsgate": self.hub_status(&realm.realm_id, HubKind::Ramsgate).await?,
            "training": self.hub_status(&realm.realm_id, HubKind::Training).await?,
            "activeHunts": hunts,
        }))
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
        let session_id = uuid_text(new_session_ulid(now));
        let roster: Vec<Participant> = members
            .iter()
            .map(|account| {
                let peer = derive_ice_peer_id(account);
                self.ensure_verified_link(account, &peer, 1, "ICE_ACCOUNT_DERIVED");
                Participant::pending(account, &peer, &format!("cand-{session_id}-{account}"))
            })
            .collect();
        let host = members.first().cloned().unwrap_or_default();
        let hunt = HuntSession {
            session_id,
            generation: 1,
            state: H::HostSelecting,
            state_version: 1,
            party_id: party_id.to_owned(),
            active_party_key: Some(party_id.to_owned()),
            party_revision: party_revision.max(1),
            hunt_definition_id: hunt_definition_id.to_owned(),
            game_mode: Some(game_mode.to_owned()),
            build: DEFAULT_GAME_BUILD,
            bridge_protocol: 1,
            virtual_port: 1,
            host_steam_id: if host.is_empty() {
                String::new()
            } else {
                derive_ice_peer_id(&host)
            },
            host_account_id: host.clone(),
            bridge_instance_id: String::new(),
            roster,
            lobby_id: None,
            coop_transport: Some("ICE".to_owned()),
            lease_expires_at: None,
            created_at: now,
            expires_at: now.saturating_add(HUNT_LIFETIME_MS),
            metadata_nonce: Some(random_nonce()),
            hub_kind: None,
            realm_id: realm_id.map(str::to_owned),
            host_participates: Some(true),
            host_local_bridge_port: None,
            server_udp_port: self.runtime.as_ref().map(|r| i64::from(r.host_server_udp_port)),
            host_local_bridge_ready_at: None,
            hub_room_id: None,
            max_peers: Some(HUNT_MAX_PEERS),
            guard_completion_mode: Some("FIRST_DEFEAT".to_owned()),
            host_offer_id: None,
            offer_expires_at: None,
            startup_deadline: None,
            last_heartbeat_at: None,
            last_lease_sequence: None,
            suspect_since: None,
            suspect_deadline: None,
            pre_suspect_state: None,
            failure_reason: None,
            tried_host_account_ids: Vec::new(),
            terminal_at: None,
            recovery_deadline: None,
            recovery_retry_at: None,
            completion_reason: None,
            drain_deadline: None,
            cancelled_by_account_id: None,
            trust_tier: Some(self.settings().policy.trust_tier),
            guard_encounter_state: None,
            extra: Map::new(),
        };
        let actor = if host.is_empty() {
            ROUTE_ACTOR
        } else {
            as_account(&host)
        };
        self.hunts.create_hunt(&actor, &hunt).await?;
        self.begin_host_selection(&hunt.session_id).await?;
        Ok(self.hunts.find_hunt(&hunt.session_id).await?.unwrap_or(hunt))
    }

    pub(crate) async fn restore_matchmaking_candidate(
        &self,
        player: &AccountId,
    ) -> Result<Option<Candidate>, RealmsError> {
        let account = player.0.as_str();
        if let Some(hunt) = self.hunts.find_active_hunt_for_account(account).await?
            && let Some(p) = hunt.participant(account)
        {
            return Ok(Some(participant_candidate(&hunt, p)));
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
        Ok(Some(hub_candidate(
            &hub,
            &row.candidate_id,
            ready,
            row.local_bridge_port.unwrap_or(0),
        )))
    }

    pub(crate) async fn poll_candidate_status(
        &self,
        player: &AccountId,
        current: &Candidate,
    ) -> Result<Option<Candidate>, RealmsError> {
        let Ok(Value::Object(mut fields)) = serde_json::from_str::<Value>(&current.0.0) else {
            return Ok(None);
        };
        let text = |fields: &Map<String, Value>, key: &str| fields.get(key).and_then(Value::as_str).map(str::to_owned);
        let account = player.0.as_str();
        let set_ready = |fields: &mut Map<String, Value>, port: i64| {
            fields.insert("Ready".to_owned(), json!(true));
            fields.insert("Host".to_owned(), json!("127.0.0.1"));
            fields.insert("Port".to_owned(), json!(port));
        };
        let cancel = |fields: &mut Map<String, Value>, reason: String| {
            fields.insert("Ready".to_owned(), json!(false));
            fields.insert("Canceled".to_owned(), json!(true));
            fields.insert("StatusReason".to_owned(), json!(reason));
        };
        if text(&fields, "Source").as_deref() == Some("REALM_HUB") {
            let Some(id) = text(&fields, "SourceSessionId") else {
                return Ok(None);
            };
            let Some(hub) = self.hunts.find_hunt(&id).await? else {
                return Ok(None);
            };
            if matches!(hub.state, H::Completed | H::Aborted) {
                let reason = hub
                    .failure_reason
                    .or(hub.completion_reason)
                    .unwrap_or_else(|| "HUB_ENDED".to_owned());
                cancel(&mut fields, reason);
            } else if let Some(row) = self.hub.find_presence(&hub.session_id, account).await?
                && let (Some(port), Some(_)) = (row.local_bridge_port, row.local_bridge_ready_at)
            {
                set_ready(&mut fields, port);
            }
            return Ok(Some(Candidate(Json(Value::Object(fields).to_string()))));
        }
        let Some(id) = text(&fields, "GameSessionId") else {
            return Ok(None);
        };
        let Some(hunt) = self.hunts.find_hunt(&id).await? else {
            return Ok(None);
        };
        if hunt.state == H::Aborted {
            let reason = hunt.failure_reason.clone().unwrap_or_else(|| "HUNT_ABORTED".to_owned());
            cancel(&mut fields, reason);
        } else if let Some(p) = hunt.participant(account)
            && let (Some(port), Some(_)) = (p.local_bridge_port, p.local_bridge_ready_at)
        {
            set_ready(&mut fields, port);
        }
        Ok(Some(Candidate(Json(Value::Object(fields).to_string()))))
    }

    pub(crate) async fn route_matchmaking_hunt(
        &self,
        context: &HuntRouteContext,
        realm_id: Option<String>,
    ) -> Result<HuntRouteOutcome, RealmsError> {
        let Some(runtime) = self.runtime.clone() else {
            return Ok(HuntRouteOutcome::NotHandled);
        };
        let members: Vec<String> = if context.party_members.is_empty() {
            vec![context.player.0.clone()]
        } else {
            context.party_members.iter().map(|a| a.0.clone()).collect()
        };
        let cancel = |reason: &str| {
            let accounts: Vec<AccountId> = members.iter().cloned().map(AccountId).collect();
            let _ = runtime.core.cancel_travel(&context.hunt_id, &accounts, reason);
            Ok(HuntRouteOutcome::Handled { result: true })
        };
        let now = self.now();
        if let Some(realm) = &realm_id
            && let RealmRequest::Hub(kind) = classify_realm_request(&context.game_mode, &context.hunt_id)
        {
            let Some(hub) = self.live_hub(realm, kind).await? else {
                return cancel("REALM_HUB_NOT_RUNNING");
            };
            let entries: Vec<HubEntryRequest> = members
                .iter()
                .filter(|a| **a != hub.host_account_id)
                .map(|account| {
                    let peer = derive_ice_peer_id(account);
                    self.ensure_verified_link(account, &peer, 1, "ICE_ACCOUNT_DERIVED");
                    HubEntryRequest {
                        account_id: account.clone(),
                        peer_id: peer,
                        candidate_id: format!("cand-{}-{account}", hub.session_id),
                    }
                })
                .collect();
            let at = EntryAt {
                session_id: hub.session_id.clone(),
                generation: hub.generation,
                realm_id: realm.clone(),
                now,
                expires_at: entry_expires_at(now),
            };
            let max_peers = hub.max_peers.unwrap_or(DEFAULT_HUB_MAX_PLAYERS);
            match self
                .hub
                .request_entries(&as_account(&context.player.0), &at, max_peers, &entries)
                .await?
            {
                HubEntryBatch::Full { .. } => return cancel("REALM_HUB_FULL"),
                HubEntryBatch::Placed(rows) => {
                    for (account, row, _) in rows {
                        let ready = row.local_bridge_port.is_some_and(|p| p > 0) && row.local_bridge_ready_at.is_some();
                        let port = row.local_bridge_port.unwrap_or(0);
                        self.set_candidate(&account, hub_candidate(&hub, &row.candidate_id, ready, port));
                        let payload = json!({
                            "sessionId": hub.session_id,
                            "generation": hub.generation,
                            "hubKind": hub_kind_name(kind),
                            "realmId": realm,
                        });
                        let message = crate::repo::hunts::NewControlMessage {
                            account_id: account.clone(),
                            message_type: "REALM_HUB_JOIN".to_owned(),
                            session_id: Some(hub.session_id.clone()),
                            generation: Some(hub.generation),
                            payload,
                            created_at: now,
                            expires_at: now.saturating_add(CONTROL_LIFE_MS),
                        };
                        let _ = self
                            .hunts
                            .enqueue_control_message(&as_account(&account), &message)
                            .await;
                    }
                }
            }
            if members.contains(&hub.host_account_id) {
                let port = hub.host_local_bridge_port.unwrap_or(0);
                let ready = port > 0 && hub.host_local_bridge_ready_at.is_some();
                let candidate_id = format!("host:{}", hub.session_id);
                self.set_candidate(&hub.host_account_id, hub_candidate(&hub, &candidate_id, ready, port));
            }
            return Ok(HuntRouteOutcome::Handled { result: true });
        }
        let stop = self.emergency_stop().await?;
        let policy = &self.settings().policy;
        let host = if realm_id.is_some() {
            policy.enabled && !stop && !context.is_hub
        } else {
            let input = RouteInput {
                game_mode: context.game_mode.clone(),
                hunt_id: context.hunt_id.clone(),
                account_ids: members.clone(),
                party_size: i64::try_from(members.len()).unwrap_or(i64::MAX),
                is_private_party: true,
            };
            decide_hunt_route(&input, policy, true, stop).route == HuntRoute::PlayerHosted
        };
        if !host {
            return Ok(HuntRouteOutcome::NotHandled);
        }
        let key = context
            .party
            .as_ref()
            .map_or_else(|| format!("solo:{}", context.player.0), ToString::to_string);
        let hunt = match self.hunts.find_active_hunt_for_party(&key).await? {
            Some(live) => live,
            None => {
                let revision = context.party_revision.and_then(|r| i64::try_from(r).ok()).unwrap_or(1);
                self.create_routed_hunt(
                    &key,
                    revision,
                    &context.hunt_id,
                    &context.game_mode,
                    &members,
                    realm_id.as_deref(),
                )
                .await?
            }
        };
        for p in &hunt.roster {
            self.set_candidate(&p.account_id, participant_candidate(&hunt, p));
        }
        Ok(HuntRouteOutcome::Handled { result: true })
    }

    fn emergency_policy(&self, admin_key: &str, text: &str) -> Result<Value, RealmsError> {
        let invalid = |message: &str| refused("ADMIN_VALIDATION", message);
        let body: Value = serde_json::from_str(text).map_err(|_| invalid("Invalid emergency stop payload."))?;
        let stop = body
            .get("p2pEmergencyStop")
            .and_then(Value::as_bool)
            .ok_or_else(|| invalid("p2pEmergencyStop must be a boolean."))?;
        let expected = body.get("expectedVersion").and_then(Value::as_u64).unwrap_or(1);
        let reason = body.get("reason").and_then(Value::as_str).map(js_trim).unwrap_or("");
        if reason.len() < 8 {
            return Err(invalid("An audit reason of at least 8 characters is required."));
        }
        let coop = if self.parts.operations.ice_transport() {
            "ICE"
        } else {
            "STEAM_EXPERIMENTAL"
        };
        // Nothing is stored: the stop is read from the operations view (kept as found).
        Ok(json!({
            "policy": {
                "version": expected.saturating_add(1),
                "p2pEmergencyStop": stop,
                "coopTransport": coop,
                "updatedAt": iso(self.now()),
                "updatedBy": admin_key,
                "reason": reason,
            },
            "requestId": random_uuid(),
        }))
    }

    fn launch_manifest(text: &str) -> Result<Value, RealmsError> {
        let invalid = |message: &str| refused("P2P_TICKET_INVALID", message);
        let body: Value = serde_json::from_str(text).map_err(|_| invalid("Invalid launch manifest request."))?;
        let hunt = body
            .get("huntDefinitionId")
            .and_then(Value::as_str)
            .map(js_trim)
            .filter(|h| !h.is_empty())
            .ok_or_else(|| invalid("huntDefinitionId is required."))?;
        let players: Vec<String> = body
            .get("expectedPlayers")
            .and_then(Value::as_array)
            .map(|list| list.iter().filter_map(Value::as_str).map(str::to_owned).collect())
            .unwrap_or_default();
        Ok(resolve_manifest_for_hunt(hunt, &players))
    }
}

fn store_error(error: &RealmsError) -> StoreError {
    StoreError::Database(error.to_string())
}

fn to_admission(error: RealmsError) -> AdmissionError {
    match error {
        RealmsError::Refused(refusal) => AdmissionError::Refused(refusal),
        RealmsError::Unimplemented(u) => AdmissionError::Unimplemented(u),
        other => AdmissionError::Store(store_error(&other)),
    }
}

fn to_relay(error: RealmsError) -> RelayError {
    match error {
        RealmsError::Refused(refusal) => RelayError::Refused(refusal),
        RealmsError::Unimplemented(u) => RelayError::Unimplemented(u),
        other => RelayError::Store(store_error(&other)),
    }
}

/// Runs a store future from a synchronous port.
fn blocking<T: Send>(future: impl Future<Output = Result<T, RealmsError>> + Send) -> Result<T, RealmsError> {
    rifts_guard::block_on_store(async { Ok(future.await) })
        .map_err(|e| RealmsError::Database(e.to_string()))
        .and_then(|inner| inner)
}

impl Hosting {
    async fn gate_now(&self) -> Result<P2PGate, RealmsError> {
        Ok(P2PGate {
            enabled: self.settings().policy.enabled,
            emergency_stop: self.emergency_stop().await?,
        })
    }

    async fn check_hub_entry_now(
        &self,
        hub: &AdmissionHunt,
        account: &AccountId,
        link: Option<&PeerLinkRecord>,
        now: UnixMillis,
    ) -> Result<PeerId, RealmsError> {
        let session = self
            .hunts
            .find_hunt(&hub.snapshot.wire_session)
            .await?
            .ok_or_else(|| refused("P2P_SESSION_NOT_FOUND", "Hub session not found."))?;
        let not_ready = || refused("P2P_SESSION_NOT_READY", "The realm's hub is not accepting players.");
        let realm_id = session.realm_id.clone().ok_or_else(not_ready)?;
        let realm = self.realms.find_by_id(&realm_id).await?;
        let who = account.0.as_str();
        let banned = realm
            .as_ref()
            .is_some_and(|r| r.bans.iter().any(|b| b.account_id == who));
        let member = realm
            .as_ref()
            .is_some_and(|r| r.host_account_id == who || r.members.iter().any(|m| m.account_id == who));
        let derived = derive_ice_peer_id(who);
        let ice = session.coop_transport.as_deref().is_none_or(|t| t == "ICE");
        if link.is_none() && ice {
            self.ensure_verified_link(who, &derived, 1, "ICE_ACCOUNT_DERIVED");
        }
        let effective = link.map_or(derived, |l| l.peer.0.to_string());
        let mut row = self.hub.find_presence(&session.session_id, who).await?;
        let waiting = row.as_ref().is_some_and(|r| {
            r.generation == session.generation
                && r.state.is_expiring_state()
                && !is_presence_expired(r.state, r.expires_at, now)
        });
        if !waiting && member && !banned && session.host_account_id != who && is_hub_live(session.state) {
            let at = EntryAt {
                session_id: session.session_id.clone(),
                generation: session.generation,
                realm_id: realm_id.clone(),
                now,
                expires_at: entry_expires_at(now),
            };
            let request = HubEntryRequest {
                account_id: who.to_owned(),
                peer_id: effective.clone(),
                candidate_id: format!("cand-{}-{who}", session.session_id),
            };
            let max = session.max_peers.unwrap_or(DEFAULT_HUB_MAX_PLAYERS);
            match self.hub.request_entries(&as_account(who), &at, max, &[request]).await? {
                HubEntryBatch::Full { .. } => return Err(refused("REALM_HUB_FULL", "The realm's hub is full.")),
                HubEntryBatch::Placed(rows) => row = rows.into_iter().next().map(|(_, r, _)| r),
            }
        }
        // Read STARTING, HOST_READY or ADMITTING as RUNNING: a member may queue while the host comes up.
        let mut view = session.clone();
        if matches!(view.state, H::Starting | H::HostReady | H::Admitting) {
            view.state = H::Running;
        }
        let presence = row.as_ref().map(|r| (r.state, r.expires_at, r.peer_id.as_str()));
        let check = HubTicketCheck {
            hub: &view,
            generation: i64::try_from(hub.snapshot.generation).ok(),
            account_id: who,
            presence,
            is_member: member,
            is_banned: banned,
            link: Some(("VERIFIED", effective.as_str())),
            now,
        };
        if let Some(denial) = hub_ticket_denial(&check) {
            let (code, message) = hub_ticket_denial_error(denial);
            return Err(refused(code, message));
        }
        Ok(PeerId(effective.parse().unwrap_or(1)))
    }

    async fn mark_participant_admitted_now(
        &self,
        hunt: &AdmissionHunt,
        account: &AccountId,
        at: UnixMillis,
    ) -> Result<bool, RealmsError> {
        let (id, generation, who) = (
            hunt.snapshot.wire_session.clone(),
            i64::try_from(hunt.snapshot.generation).unwrap_or(i64::MAX),
            account.0.clone(),
        );
        self.hunts
            .transaction(&as_account(&who), move |tx| -> Result<bool, RealmsError> {
                let Some(marked) = tx.mark_participant_admitted(&id, generation, &who, at)? else {
                    return Ok(false);
                };
                if matches!(marked.state, H::HostReady | H::Admitting) {
                    let all = marked
                        .roster
                        .iter()
                        .all(|p| p.admission_state == crate::hunt::AdmissionState::Admitted);
                    let target = if all { H::Running } else { H::Admitting };
                    if target != marked.state {
                        let from = marked.state;
                        let moved = tx.compare_and_set(
                            &id,
                            generation,
                            &[H::HostReady, H::Admitting],
                            marked.state_version,
                            |h| {
                                h.state = target;
                            },
                        )?;
                        if let Some(moved) = moved {
                            let reason = if all {
                                "ALL_PARTICIPANTS_ADMITTED"
                            } else {
                                "FIRST_PARTICIPANT_ADMITTED"
                            };
                            transition(tx, from, &moved, reason, TransitionActor::Host, Some(&who), at)?;
                        }
                    }
                }
                Ok(true)
            })
            .await
    }

    async fn peer_disconnected_now(&self, admission: &PeerAdmissionRecord, at: UnixMillis) -> Result<(), RealmsError> {
        let Some(hunt) = self.hunts.find_hunt(&admission.session).await? else {
            return Ok(());
        };
        let generation = i64::try_from(admission.generation).unwrap_or(i64::MAX);
        let who = admission.account.0.as_str();
        if hunt.is_realm_hub() {
            self.hub
                .mark_lost(
                    &as_account(who),
                    &hunt.session_id,
                    generation,
                    who,
                    Some(&admission.admission_id),
                    at,
                    presence_purge_at(at),
                )
                .await?;
            self.invalidate_hub(&hunt.session_id);
        } else {
            self.hunts
                .mark_participant_disconnected(
                    &as_account(who),
                    &hunt.session_id,
                    generation,
                    who,
                    at,
                    at.saturating_add(REJOIN_WINDOW_MS),
                )
                .await?;
        }
        Ok(())
    }

    async fn hub_rows(&self, wire: &str, generation: u64, limit: usize) -> Result<Vec<PresenceRow>, RealmsError> {
        let generation = i64::try_from(generation).unwrap_or(i64::MAX);
        let limit = u32::try_from(limit.min(256)).unwrap_or(256);
        let rows = self
            .hub
            .list_by_state(wire, generation, &PresenceState::LIVE, limit)
            .await?;
        Ok(rows
            .into_iter()
            .map(|r| PresenceRow {
                peer: PeerId(peer_number(&r.peer_id, &r.account_id)),
                account: AccountId(r.account_id),
                state: relay_presence(r.state),
            })
            .collect())
    }

    async fn join_common_now(
        &self,
        hunt: &AdmissionHunt,
        key_id: &str,
        bridge_instance_id: &str,
    ) -> Result<Option<Json>, RealmsError> {
        let stored = self.hunts.find_hunt(&hunt.snapshot.wire_session).await?;
        let hub = hunt.snapshot.is_hub();
        let nonce = stored
            .as_ref()
            .and_then(|s| s.metadata_nonce.clone())
            .unwrap_or_else(|| PLACEHOLDER_NONCE.to_owned());
        let completion = stored
            .as_ref()
            .and_then(|s| s.guard_completion_mode.clone())
            .unwrap_or_else(|| if hub { "NO_COMPLETION" } else { "FIRST_DEFEAT" }.to_owned());
        let peer = hunt.snapshot.host_peer.0.to_string();
        let common = json!({
            "keyId": key_id,
            "sessionId": hunt.snapshot.wire_session,
            "partyId": hunt.party_id,
            "partyRevision": hunt.party_revision,
            "huntDefinitionId": hunt.hunt_definition_id,
            "generation": hunt.snapshot.generation,
            "build": hunt.build,
            "bridgeProtocol": hunt.bridge_protocol,
            "virtualPort": hunt.virtual_port,
            "hostPeerId": peer,
            "hostSteamId": peer,
            "bridgeInstanceId": bridge_instance_id,
            "metadataNonce": nonce,
            "forceRelay": self.settings().policy.force_sdr,
            "guardCompletionMode": completion,
            "sessionRole": if hub { "HUB" } else { "HUNT" },
        });
        Ok(Some(Json(common.to_string())))
    }
}

impl AdmissionHunts for Hosting {
    fn gate(&self) -> Result<P2PGate, AdmissionError> {
        blocking(self.gate_now()).map_err(to_admission)
    }

    fn find_hunt(&self, session: &SessionId) -> Result<Option<AdmissionHunt>, AdmissionError> {
        let wire = wire_session_id(session);
        let found = blocking(async { Ok(self.hunts.find_hunt(&wire).await?) }).map_err(to_admission)?;
        Ok(found.as_ref().and_then(admission_hunt))
    }

    fn reserve_participant(
        &self,
        hunt: &AdmissionHunt,
        account: &AccountId,
        now: UnixMillis,
    ) -> Result<Option<AdmissionHunt>, AdmissionError> {
        let generation = i64::try_from(hunt.snapshot.generation).unwrap_or(i64::MAX);
        let reserved = blocking(async {
            Ok(self
                .hunts
                .reserve_participant_ticket(
                    &as_account(&account.0),
                    &hunt.snapshot.wire_session,
                    generation,
                    &account.0,
                    now,
                    now.saturating_add(REJOIN_WINDOW_MS),
                    MAX_RECONNECTS,
                )
                .await?)
        })
        .map_err(to_admission)?;
        Ok(reserved.as_ref().and_then(admission_hunt))
    }

    fn check_hub_entry(
        &self,
        hub: &AdmissionHunt,
        account: &AccountId,
        link: Option<&PeerLinkRecord>,
        now: UnixMillis,
    ) -> Result<PeerId, AdmissionError> {
        let link = link.filter(|l| l.status == LinkStatus::Verified || l.status == LinkStatus::Revoked);
        blocking(self.check_hub_entry_now(hub, account, link, now)).map_err(to_admission)
    }

    fn mark_hub_ticketed(
        &self,
        hub: &AdmissionHunt,
        account: &AccountId,
        now: UnixMillis,
        ticket_expires_at: UnixMillis,
    ) -> Result<bool, AdmissionError> {
        let generation = i64::try_from(hub.snapshot.generation).unwrap_or(i64::MAX);
        blocking(async {
            Ok(self
                .hub
                .mark_ticketed(
                    &as_account(&account.0),
                    &hub.snapshot.wire_session,
                    generation,
                    &account.0,
                    now,
                    ticket_expires_at,
                )
                .await?
                .is_some())
        })
        .map_err(to_admission)
    }

    fn mark_participant_admitted(
        &self,
        hunt: &AdmissionHunt,
        account: &AccountId,
        admitted_at: UnixMillis,
    ) -> Result<bool, AdmissionError> {
        blocking(self.mark_participant_admitted_now(hunt, account, admitted_at)).map_err(to_admission)
    }

    fn admit_hub_peer(
        &self,
        hub: &AdmissionHunt,
        account: &AccountId,
        admission_id: &str,
        admitted_at: UnixMillis,
    ) -> Result<(), AdmissionError> {
        if hub.snapshot.host_account == *account {
            return Ok(());
        }
        let generation = i64::try_from(hub.snapshot.generation).unwrap_or(i64::MAX);
        blocking(async {
            self.hub
                .mark_admitted(
                    &as_account(&account.0),
                    &hub.snapshot.wire_session,
                    generation,
                    &account.0,
                    admission_id,
                    admitted_at,
                    self.now(),
                    Some("ice"),
                )
                .await?;
            Ok(())
        })
        .map_err(to_admission)?;
        self.invalidate_hub(&hub.snapshot.wire_session);
        Ok(())
    }

    fn peer_disconnected(&self, admission: &PeerAdmissionRecord, at: UnixMillis) -> Result<(), AdmissionError> {
        blocking(self.peer_disconnected_now(admission, at)).map_err(to_admission)
    }

    fn hub_presence(&self, hub: &AdmissionHunt, limit: usize) -> Result<Vec<PresenceRow>, AdmissionError> {
        blocking(self.hub_rows(&hub.snapshot.wire_session, hub.snapshot.generation, limit)).map_err(to_admission)
    }

    fn join_bootstrap_common(
        &self,
        hunt: &AdmissionHunt,
        key_id: &str,
        bridge_instance_id: &str,
    ) -> Result<Option<Json>, AdmissionError> {
        blocking(self.join_common_now(hunt, key_id, bridge_instance_id)).map_err(to_admission)
    }
}

impl IceHunts for Hosting {
    fn gate(&self) -> Result<P2PGate, RelayError> {
        blocking(self.gate_now()).map_err(to_relay)
    }

    fn find_hunt(&self, session: &SessionId) -> Result<Option<HuntSnapshot>, RelayError> {
        let wire = wire_session_id(session);
        let found = blocking(async { Ok(self.hunts.find_hunt(&wire).await?) }).map_err(to_relay)?;
        Ok(found.as_ref().map(snapshot))
    }

    fn hub_presence(&self, hunt: &HuntSnapshot, limit: usize) -> Result<Vec<PresenceRow>, RelayError> {
        blocking(self.hub_rows(&hunt.wire_session, hunt.generation, limit)).map_err(to_relay)
    }

    fn enqueue(&self, message: ControlMessage) -> Result<(), RelayError> {
        let payload: Value = serde_json::from_str(&message.payload.0).map_err(|e| {
            RelayError::Refused(Refusal {
                code: "P2P_SIGNAL_INVALID".to_owned(),
                message: e.to_string(),
            })
        })?;
        let record = crate::repo::hunts::NewControlMessage {
            account_id: message.account.0.clone(),
            message_type: message.message_type.to_owned(),
            session_id: Some(message.wire_session.clone()),
            generation: Some(i64::try_from(message.generation).unwrap_or(i64::MAX)),
            payload,
            created_at: message.created_at,
            expires_at: message.expires_at,
        };
        blocking(async {
            self.hunts
                .enqueue_control_message(&as_account(&message.account.0), &record)
                .await?;
            Ok(())
        })
        .map_err(to_relay)
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
        let now = iso(self.now());
        if expected_version > 1 {
            let current = if self.parts.operations.ice_transport() {
                "ICE"
            } else {
                "STEAM_EXPERIMENTAL"
            };
            let policy = json!({
                "version": 1,
                "p2pEmergencyStop": stop,
                "coopTransport": current,
                "updatedAt": now,
            });
            return Ok(CoopTransportOutcome::Conflict(Json(policy.to_string())));
        }
        let policy = json!({
            "version": expected_version + 1,
            "p2pEmergencyStop": stop,
            "coopTransport": transport.as_str(),
            "updatedAt": now,
            "updatedBy": admin_key_id,
            "reason": reason,
            "requestId": request_id,
        });
        Ok(CoopTransportOutcome::Updated(Json(policy.to_string())))
    }
}

impl RealmEventSink for Hosting {
    fn emit(&self, event: RealmEvent) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            let now = self.now();
            match event {
                RealmEvent::MemberRemoved {
                    realm_id,
                    account_id,
                    reason,
                } => {
                    let rows = self
                        .hub
                        .remove_account_from_realm(
                            &DIRECTORY_ACTOR,
                            &realm_id,
                            &account_id,
                            now,
                            presence_purge_at(now),
                        )
                        .await
                        .map_err(|e| e.to_string())?;
                    for row in &rows {
                        self.invalidate_hub(&row.session_id);
                    }
                    self.parts.log.info(&format!(
                        "[Realms] removed account {account_id} from realm {realm_id} hub presence ({})",
                        reason.as_str()
                    ));
                }
                RealmEvent::RealmReleased {
                    realm_id,
                    host_account_id,
                } => {
                    self.hub
                        .remove_realm(&as_account(&host_account_id), &realm_id, now, presence_purge_at(now))
                        .await
                        .map_err(|e| e.to_string())?;
                    self.end_realm_hubs(
                        &realm_id,
                        "REALM_RELEASED",
                        H::Completed,
                        TransitionActor::Host,
                        &host_account_id,
                    )
                    .await;
                }
                RealmEvent::HostOffline {
                    realm_id,
                    host_account_id,
                } => {
                    self.end_realm_hubs(
                        &realm_id,
                        "REALM_HOST_OFFLINE",
                        H::Aborted,
                        TransitionActor::System,
                        &host_account_id,
                    )
                    .await;
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
        reason: &str,
        terminal: HuntState,
        actor: TransitionActor,
        host: &str,
    ) {
        for kind in [HubKind::Ramsgate, HubKind::Training] {
            if let Ok(Some(hub)) = self.live_hub(realm_id, kind).await {
                let _ = self
                    .end_hub(&hub.session_id, hub.generation, reason, terminal, actor, Some(host))
                    .await;
            }
        }
    }

    fn typed_bootstrap(
        &self,
        json: &Value,
        session: SessionId,
        default_peers: u8,
    ) -> Result<HostBootstrap, RealmsError> {
        let capability = json
            .get("capability")
            .and_then(Value::as_str)
            .ok_or_else(|| RealmsError::Database("bootstrap without a capability".to_owned()))?;
        Ok(HostBootstrap {
            session,
            generation: json.get("generation").and_then(Value::as_u64).unwrap_or(1),
            capability: Secret::new(capability.to_owned()),
            host_ticket: json
                .get("hostTicket")
                .and_then(Value::as_str)
                .map(|t| Secret::new(t.to_owned())),
            backend_url: json
                .get("backendUrl")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            server_udp_port: json
                .get("serverUdpPort")
                .and_then(Value::as_u64)
                .and_then(|p| u16::try_from(p).ok())
                .unwrap_or(DEFAULT_SERVER_UDP_PORT),
            maximum_peers: json
                .get("maximumPeers")
                .and_then(Value::as_u64)
                .and_then(|p| u8::try_from(p).ok())
                .unwrap_or(default_peers),
        })
    }
}

fn internal(message: &str) -> RealmsError {
    refused("INTERNAL", message)
}

impl HuntLifecycle for Hosting {
    fn host_bootstrap(&self, account: AccountId, session: SessionId) -> Fut<'_, HostBootstrap> {
        Box::pin(async move {
            let json = self
                .create_host_bootstrap(&account.0, &wire_session_id(&session))
                .await?;
            self.typed_bootstrap(&json, session, 4)
        })
    }

    fn route_ready(&self, account: AccountId, session: SessionId, local_port: u16) -> Fut<'_, ()> {
        Box::pin(async move {
            let mut body = Map::new();
            body.insert("localBridgePort".to_owned(), json!(local_port));
            self.mark_local_route_ready(&account.0, &wire_session_id(&session), &body)
                .await?;
            Ok(())
        })
    }

    fn host_ready(&self, runtime: RuntimeAuth) -> Fut<'_, ()> {
        Box::pin(async move {
            self.mark_host_ready(&capability_of_runtime(&runtime), &Map::new())
                .await?;
            Ok(())
        })
    }

    fn abort(&self, runtime: RuntimeAuth, reason: String) -> Fut<'_, ()> {
        Box::pin(async move {
            let mut body = Map::new();
            body.insert("reason".to_owned(), json!(reason));
            self.abort_hunt(&capability_of_runtime(&runtime), &body).await?;
            Ok(())
        })
    }

    fn end(&self, runtime: RuntimeAuth, reason: String) -> Fut<'_, ()> {
        Box::pin(async move {
            let mut body = Map::new();
            body.insert("reason".to_owned(), json!(reason));
            self.finish_hunt(&capability_of_runtime(&runtime), &body).await?;
            Ok(())
        })
    }

    fn status(&self, account: AccountId, session: SessionId) -> Fut<'_, HuntStatus> {
        Box::pin(async move {
            let json = self
                .get_hunt_status(Some(&account.0), &wire_session_id(&session))
                .await?;
            Ok(HuntStatus {
                state: json
                    .get("state")
                    .and_then(Value::as_str)
                    .and_then(HuntState::parse)
                    .unwrap_or(H::Running),
                generation: json.get("generation").and_then(Value::as_u64).unwrap_or(1),
                verdict: None,
            })
        })
    }

    /// Always generation 1 (kept as found): a retried hunt refuses it.
    fn observe(&self, account: AccountId, session: SessionId, observation: Observation) -> Fut<'_, ()> {
        Box::pin(async move {
            let metrics = observation.route_kind.is_some();
            let body = json!({
                "generation": 1,
                "observedAt": iso(self.now()),
                "hostReachable": observation.host_reachable,
                "localBridgeAlive": observation.local_bridge_alive,
                "metricsAvailable": metrics,
                "routeKind": observation.route_kind.clone().unwrap_or_else(|| "UNKNOWN".to_owned()),
                "lastPacketAgeMs": observation.last_packet_age_ms,
                "pingMs": observation.ping_ms,
                "jitterMs": observation.jitter_ms,
                "packetLossPercent": observation.packet_loss_percent,
            });
            let Value::Object(body) = body else {
                return Ok(());
            };
            self.observe(&account.0, &wire_session_id(&session), &body).await?;
            Ok(())
        })
    }

    fn start_hub(&self, account: AccountId, hub: StartHub) -> Fut<'_, HostBootstrap> {
        Box::pin(async move {
            let mut body = Map::new();
            body.insert("hubKind".to_owned(), json!(hub_kind_name(hub.kind)));
            body.insert("maxPlayers".to_owned(), json!(hub.max_players));
            body.insert("afkSeconds".to_owned(), json!(hub.afk_seconds));
            let json = self.start_realm_hub(&account.0, &body).await?;
            let node_name = self.runtime.as_ref().map_or("local", |r| r.node.as_str());
            let node = module_api::NodeId::new(node_name).map_err(|_| internal("Invalid node id."))?;
            let wire = json.get("sessionId").and_then(Value::as_str).unwrap_or_default();
            let session = seam_session_id(wire, &node).ok_or_else(|| internal("Invalid hub session id."))?;
            self.typed_bootstrap(&json, session, hub.max_players)
        })
    }

    fn stop_hub(&self, account: AccountId, kind: HubKind) -> Fut<'_, ()> {
        Box::pin(async move {
            let mut body = Map::new();
            body.insert("hubKind".to_owned(), json!(hub_kind_name(kind)));
            self.stop_realm_hub(&account.0, &body).await?;
            Ok(())
        })
    }

    fn hosting_status(&self, account: AccountId) -> Fut<'_, Json> {
        Box::pin(async move { Ok(Json(self.get_realm_hosting_status(&account.0).await?.to_string())) })
    }

    fn set_emergency_stop(&self, admin_key: String, policy: Json) -> Fut<'_, Json> {
        Box::pin(async move { Ok(Json(self.emergency_policy(&admin_key, &policy.0)?.to_string())) })
    }

    fn resolve_launch_manifest(&self, request: Json) -> Fut<'_, Json> {
        Box::pin(async move { Ok(Json(Self::launch_manifest(&request.0)?.to_string())) })
    }
}
