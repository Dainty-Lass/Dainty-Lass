//! Session routes and trait adapters for [`crate::leases::Hosting`]:
//! - Host bootstrap (`CreateHostBootstrap`), local route ready (`MarkLocalRouteReady`), host ready (`MarkHostReady`),
//!   abort (`AbortHunt`), end (`EndHunt`), and status (`GetHuntStatus`)
//! - Realm hub lifecycle (`CreateRealmHubSession`, `StopRealmHubSession`, `GetRealmHostingStatus`)
//! - Operations policy emergency stop (`UpdateP2PEmergencyStop`) and launch manifest (`ResolvePlayerHostedLaunchManifest`)
//! - [`rifts_admission::AdmissionHunts`], [`rifts_relay::IceHunts`], [`rifts_relay::OperationsPolicy`], and
//!   [`crate::ports::RealmEventSink`] implementations over `store`.

use std::collections::BTreeMap;
use std::future::Future;
use std::str::FromStr;

use rifts_admission::{
    AdmissionError, AdmissionHunt, AdmissionHunts, LinkStatus, PeerAdmissionRecord, PeerLinkRecord, SessionUuid,
    TicketAuthority, TicketBindings, TicketRecord, TicketStatus,
};
use rifts_guard::block_on_store;
use rifts_relay::rules::{
    CoopTransport, HuntSnapshot, HuntState as RelayHuntState, PresenceRow, PresenceState as RelayPresenceState,
    RosterEntry, SessionKind,
};
use rifts_relay::{ControlMessage, CoopTransportOutcome, IceHunts, OperationsPolicy, P2PGate, RelayError};
use rifts_seam::{
    AccountId, Actor, BoxFuture, CapabilityScope, HubKind, Json, PeerId, Refusal, RuntimeAuth, Secret, SessionId,
    UnixMillis,
};
use serde_json::{Value, json};

use crate::RealmsError;
use crate::hub::{
    HUB_CAPABILITY_SCOPES, HUB_SESSION_LIFETIME_MS, HubTicketCheck, PresenceState, decide_realm_capacity,
    entry_expires_at, hub_manifest, hub_party_key, hub_ticket_denial, hub_ticket_denial_error, is_hub_live,
    parse_hub_start_body, parse_hub_stop_body, presence_purge_at, realm_host_ineligibility_reason,
};
use crate::hunt::{
    AdmissionState, HostCapabilityRecord, HuntSession, HuntState, HuntTransition, Participant, ReliabilityIncrements,
    TransitionActor,
};
use crate::ids::{new_session_ulid, random_uuid, seam_session_id, uuid_text, wire_session_id};
use crate::leases::{Hosting, integer_in, refused};
use crate::lifecycle::{HOST_END_STATES, string_in};
use crate::ports::RealmEventSink;
use crate::realm::{HubState, RealmEvent, hub_kind_name, hub_room_id};
use crate::repo::hub::{EntryAt, HubEntryBatch, HubEntryRequest};
use crate::repo::hunts::{HostReadyMark, HostStartup, NewControlMessage};
use crate::selection::derive_ice_peer_id;
use crate::time::iso;

const DEFAULT_GAME_BUILD: i64 = 647_472;
const DEFAULT_BRIDGE_PROTOCOL: i64 = 1;
const DEFAULT_VIRTUAL_PORT: i64 = 1;
const REJOIN_WINDOW_MS: i64 = 180_000;
const MAX_RECONNECTS: i64 = 5;
const CONTROL_MESSAGE_TTL_MS: i64 = 10 * 60_000;

/// `CreateHostBootstrap` and `CreateRealmHubSession`: what the host's launcher writes into the session folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostBootstrap {
    pub session: SessionId,
    pub generation: u64,
    /// `capability.secret`, 43 characters.
    pub capability: Secret,
    /// `ticket.secret` for a host that also plays (`host_local_client`); minted by rifts-admission.
    pub host_ticket: Option<Secret>,
    /// `P2P_BACKEND_URL`.
    pub backend_url: String,
    /// From `P2P_HOST_SERVER_UDP_PORT`'s 32 ports, on 127.0.0.1.
    pub server_udp_port: u16,
    pub maximum_peers: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartHub {
    pub kind: HubKind,
    /// 1 to 64.
    pub max_players: u8,
    /// 0 to 7200.
    pub afk_seconds: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HuntStatus {
    pub state: HuntState,
    pub generation: u64,
    /// Guard's verdict (`hunts.result`), from rifts-guard.
    pub verdict: Option<Json>,
}

/// `RecordParticipantObservation`.
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

pub trait HuntLifecycle: Send + Sync {
    /// `CreateHostBootstrap`: the host capability and host ticket.
    fn host_bootstrap(
        &self,
        account: AccountId,
        session: SessionId,
    ) -> BoxFuture<'_, Result<HostBootstrap, RealmsError>>;
    /// `MarkLocalRouteReady`.
    fn route_ready(
        &self,
        account: AccountId,
        session: SessionId,
        local_port: u16,
    ) -> BoxFuture<'_, Result<(), RealmsError>>;
    /// `MarkHostReady`: STARTING to HOST_READY.
    fn host_ready(&self, runtime: RuntimeAuth) -> BoxFuture<'_, Result<(), RealmsError>>;
    /// `AbortHunt`.
    fn abort(&self, runtime: RuntimeAuth, reason: String) -> BoxFuture<'_, Result<(), RealmsError>>;
    /// `EndHunt`.
    fn end(&self, runtime: RuntimeAuth, reason: String) -> BoxFuture<'_, Result<(), RealmsError>>;
    /// `GetHuntStatus`.
    fn status(&self, account: AccountId, session: SessionId) -> BoxFuture<'_, Result<HuntStatus, RealmsError>>;
    /// `RecordParticipantObservation`.
    fn observe(
        &self,
        account: AccountId,
        session: SessionId,
        observation: Observation,
    ) -> BoxFuture<'_, Result<(), RealmsError>>;
    /// `CreateRealmHubSession` (`MYSTICPARADOX_REALMS_HOSTING`).
    fn start_hub(&self, account: AccountId, hub: StartHub) -> BoxFuture<'_, Result<HostBootstrap, RealmsError>>;
    /// `StopRealmHubSession`.
    fn stop_hub(&self, account: AccountId, kind: HubKind) -> BoxFuture<'_, Result<(), RealmsError>>;
    /// `GetRealmHostingStatus`.
    fn hosting_status(&self, account: AccountId) -> BoxFuture<'_, Result<Json, RealmsError>>;
    /// `UpdateP2PEmergencyStop`.
    fn set_emergency_stop(&self, admin_key: String, policy: Json) -> BoxFuture<'_, Result<Json, RealmsError>>;
    /// The Director's `ResolvePlayerHostedLaunchManifest` (`MYSTICPARADOX_HUNT_MANIFEST_SOURCE=director`).
    fn resolve_launch_manifest(&self, request: Json) -> BoxFuture<'_, Result<Json, RealmsError>>;
}

const HUNT_CAPABILITY_SCOPES: &[CapabilityScope] = &[
    CapabilityScope::HostReady,
    CapabilityScope::LeaseHeartbeat,
    CapabilityScope::PeerAdmitted,
    CapabilityScope::EventsWrite,
    CapabilityScope::CompleteRequest,
    CapabilityScope::GameServerRuntime,
    CapabilityScope::Abort,
];

fn map_relay_state(state: HuntState) -> RelayHuntState {
    match state {
        HuntState::Requested => RelayHuntState::Requested,
        HuntState::HostSelecting => RelayHuntState::HostSelecting,
        HuntState::HostOffered => RelayHuntState::HostOffered,
        HuntState::Starting => RelayHuntState::Starting,
        HuntState::HostReady => RelayHuntState::HostReady,
        HuntState::Admitting => RelayHuntState::Admitting,
        HuntState::Running => RelayHuntState::Running,
        HuntState::Draining => RelayHuntState::Draining,
        HuntState::Suspect => RelayHuntState::Suspect,
        HuntState::Fenced => RelayHuntState::Fenced,
        HuntState::Aborting => RelayHuntState::Aborting,
        HuntState::Recovering => RelayHuntState::Recovering,
        HuntState::Completed => RelayHuntState::Completed,
        HuntState::Aborted => RelayHuntState::Aborted,
    }
}

fn map_relay_presence_state(state: PresenceState) -> RelayPresenceState {
    match state {
        PresenceState::Pending => RelayPresenceState::Pending,
        PresenceState::Ticketed => RelayPresenceState::Ticketed,
        PresenceState::Admitted => RelayPresenceState::Admitted,
        PresenceState::Lost => RelayPresenceState::Lost,
        PresenceState::Left => RelayPresenceState::Left,
        PresenceState::Removed => RelayPresenceState::Removed,
        PresenceState::Expired => RelayPresenceState::Expired,
    }
}

fn parse_peer_id(steam_id: &str, fallback_account: &str) -> PeerId {
    steam_id
        .parse::<u64>()
        .ok()
        .filter(|&n| n > 0)
        .or_else(|| derive_ice_peer_id(fallback_account).parse::<u64>().ok())
        .map_or(PeerId(1), PeerId)
}

fn to_hunt_snapshot(hunt: &HuntSession) -> HuntSnapshot {
    let coop_transport = hunt
        .coop_transport
        .as_deref()
        .and_then(CoopTransport::parse)
        .unwrap_or(CoopTransport::Ice);
    let kind = if hunt.is_realm_hub() {
        SessionKind::RealmHub
    } else {
        SessionKind::Hunt
    };
    let host_peer = parse_peer_id(&hunt.host_steam_id, &hunt.host_account_id);
    let roster = hunt
        .roster
        .iter()
        .map(|p| RosterEntry {
            account: AccountId(p.account_id.clone()),
            peer: parse_peer_id(&p.steam_id, &p.account_id),
            returned_to_hub: p.returned_to_hub_at.is_some(),
        })
        .collect();
    HuntSnapshot {
        wire_session: hunt.session_id.clone(),
        generation: u64::try_from(hunt.generation.max(1)).unwrap_or(1),
        state: map_relay_state(hunt.state),
        kind,
        realm: hunt.realm_id.clone(),
        coop_transport,
        host_account: AccountId(hunt.host_account_id.clone()),
        host_peer,
        expires_at: hunt.expires_at,
        roster,
    }
}

fn to_admission_hunt(hunt: &HuntSession) -> Option<AdmissionHunt> {
    let session = SessionUuid::from_str(&hunt.session_id).ok()?;
    let snapshot = to_hunt_snapshot(hunt);
    let lobby_id = hunt.lobby_id.clone().or_else(|| {
        matches!(
            hunt.state,
            HuntState::Starting | HuntState::HostReady | HuntState::Admitting | HuntState::Running
        )
        .then(|| "ICE".to_owned())
    });
    let bridge_instance_id = (!hunt.bridge_instance_id.is_empty()).then(|| hunt.bridge_instance_id.clone());
    Some(AdmissionHunt {
        snapshot,
        session,
        party_id: hunt.party_id.replace('@', ":"),
        party_revision: u64::try_from(hunt.party_revision.max(1)).unwrap_or(1),
        hunt_definition_id: hunt.hunt_definition_id.clone(),
        build: u32::try_from(hunt.build.max(1)).unwrap_or(647_472),
        bridge_protocol: u16::try_from(hunt.bridge_protocol.clamp(1, 255)).unwrap_or(1),
        virtual_port: u16::try_from(hunt.virtual_port.clamp(1, 65535)).unwrap_or(1),
        lobby_id,
        bridge_instance_id,
    })
}

fn admission_db(err: impl std::fmt::Display) -> AdmissionError {
    AdmissionError::Store(rifts_seam::StoreError::Database(err.to_string()))
}

fn admission_refused(code: &str, message: &str) -> AdmissionError {
    AdmissionError::Refused(Refusal {
        code: code.to_owned(),
        message: message.to_owned(),
    })
}

fn relay_db(err: impl std::fmt::Display) -> RelayError {
    RelayError::Store(rifts_seam::StoreError::Database(err.to_string()))
}

fn block_on_repo<F, T>(fut: F) -> Result<T, rifts_seam::StoreError>
where
    F: Future<Output = Result<T, crate::db::RepoError>> + Send,
    T: Send,
{
    block_on_store(async move { fut.await.map_err(|e| rifts_seam::StoreError::Database(e.to_string())) })
}

fn block_on_realms<F, T>(fut: F) -> Result<T, rifts_seam::StoreError>
where
    F: Future<Output = Result<T, RealmsError>> + Send,
    T: Send,
{
    block_on_store(async move { fut.await.map_err(|e| rifts_seam::StoreError::Database(e.to_string())) })
}

pub fn resolve_manifest_for_hunt(hunt_definition_id: &str, expected_players: &[String]) -> Value {
    if hunt_definition_id.contains("ReturnToRamsgate") || hunt_definition_id.eq_ignore_ascii_case("CITY") {
        let m = hub_manifest(HubKind::Ramsgate);
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
    if hunt_definition_id.contains("TrainingDojo") {
        let m = hub_manifest(HubKind::Training);
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
    let expected = if expected_players.is_empty() {
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
        "expectedPlayerString": expected,
        "guardCompletionMode": "FIRST_DEFEAT",
    })
}

impl Hosting {
    pub(crate) fn normalize_session_id(&self, raw: &str) -> String {
        let stripped = raw.split_once(':').map_or(raw, |(_, rest)| rest);
        if let Ok(seam) = stripped.parse::<SessionId>() {
            return wire_session_id(&seam);
        }
        stripped.to_owned()
    }

    async fn find_session(&self, raw_session_id: &str) -> Result<Option<HuntSession>, RealmsError> {
        let wire = self.normalize_session_id(raw_session_id);
        let mut found = self.hunts.find_hunt(&wire).await?;
        if found.is_none() && wire != raw_session_id {
            found = self.hunts.find_hunt(raw_session_id).await?;
        }
        Ok(found)
    }

    fn ensure_verified_link(&self, account: &str, peer_str: &str, now: UnixMillis) {
        if let (Some(runtime), Ok(peer_num)) = (&self.runtime, peer_str.parse::<u64>()) {
            let link = PeerLinkRecord {
                account: AccountId(account.to_owned()),
                peer: PeerId(peer_num),
                status: LinkStatus::Verified,
                revision: 1,
                verified_at: now,
                verification_method: "ICE_ACCOUNT_DERIVED".to_owned(),
            };
            let _ = runtime
                .admission_store
                .put_peer_link(&Actor::Account(AccountId(account.to_owned())), &link);
        }
    }

    fn mint_self_ticket(&self, hunt: &HuntSession, account: &str, peer_str: &str, now: UnixMillis) -> Option<Secret> {
        let runtime = self.runtime.as_ref()?;
        let session = SessionUuid::from_str(&hunt.session_id).ok()?;
        let joiner_peer = parse_peer_id(peer_str, account);
        let host_peer = parse_peer_id(&hunt.host_steam_id, &hunt.host_account_id);
        let bindings = TicketBindings {
            account: AccountId(account.to_owned()),
            joiner_peer,
            host_peer,
            session,
            generation: u32::try_from(hunt.generation.max(1)).ok()?,
            party_id: hunt.party_id.replace('@', ":"),
            party_revision: u64::try_from(hunt.party_revision.max(1)).unwrap_or(1),
            hunt_definition_id: hunt.hunt_definition_id.clone(),
            build: u32::try_from(hunt.build.max(1)).unwrap_or(647_472),
            bridge_protocol: u16::try_from(hunt.bridge_protocol.clamp(1, 255)).unwrap_or(1),
            virtual_port: u16::try_from(hunt.virtual_port.clamp(1, 65535)).unwrap_or(1),
        };
        let minted = TicketAuthority::mint(
            &*runtime.tickets,
            bindings,
            now,
            Some(rifts_admission::JOIN_TICKET_LIFETIME_SECONDS),
        )
        .ok()?;
        let claims = &minted.claims;
        let record = TicketRecord {
            jti_digest: minted.jti_digest.clone(),
            key_id: claims.key_id.0.clone(),
            session: hunt.session_id.clone(),
            generation: u64::from(claims.bindings.generation),
            account: claims.bindings.account.clone(),
            joiner_peer: claims.bindings.joiner_peer,
            host_peer: claims.bindings.host_peer,
            bridge_protocol: claims.bindings.bridge_protocol,
            virtual_port: claims.bindings.virtual_port,
            issued_at: UnixMillis(i64::try_from(claims.issued_at).unwrap_or_default() * 1000),
            expires_at: UnixMillis(i64::try_from(claims.expires_at).unwrap_or_default() * 1000),
            status: TicketStatus::Issued,
            consumed_at: None,
            consumer_bridge_instance_id: None,
            consumer_request_id: None,
        };
        let _ = runtime
            .admission_store
            .insert_ticket(&Actor::Account(AccountId(account.to_owned())), &record);
        Some(minted.text)
    }

    fn build_host_bootstrap_json(
        &self,
        hunt: &HuntSession,
        capability: &Secret,
        host_ticket: Option<&Secret>,
        afk_seconds: Option<u32>,
    ) -> Value {
        let runtime = self.runtime.as_ref();
        let key_id = runtime.map_or_else(|| "local-key-1".to_owned(), |rt| rt.tickets.key_id().0.clone());
        let backend_url = runtime.map_or_else(|| "http://127.0.0.1:8080".to_owned(), |rt| rt.backend_url.clone());
        let default_port = runtime.map_or(8791u16, |rt| rt.host_server_udp_port);
        let server_udp_port = hunt
            .server_udp_port
            .and_then(|p| u16::try_from(p).ok())
            .unwrap_or(default_port);
        let maximum_peers = hunt
            .max_peers
            .and_then(|p| u8::try_from(p.clamp(1, 64)).ok())
            .unwrap_or(if hunt.is_realm_hub() { 16 } else { 4 });
        let host_local_client = hunt.host_participates.unwrap_or(true);
        let hunt_evidence = !hunt.is_realm_hub();
        let roster_players: Vec<String> = hunt.roster.iter().map(|p| p.account_id.clone()).collect();
        let manifest = resolve_manifest_for_hunt(&hunt.hunt_definition_id, &roster_players);
        let map_path = manifest["mapPath"]
            .as_str()
            .unwrap_or("/Game/Maps/ramsgate/ramsgate_01_persistent");
        let behemoth_path = manifest["behemothPath"].as_str().unwrap_or("NO_BEHEMOTH");
        let matchmaker_hunt_id = manifest["matchmakerHuntId"]
            .as_str()
            .unwrap_or(&hunt.hunt_definition_id);
        let expected_players = manifest["expectedPlayerString"].as_str().unwrap_or("1");
        let guard_completion_mode = hunt
            .guard_completion_mode
            .as_deref()
            .or_else(|| manifest["guardCompletionMode"].as_str())
            .unwrap_or(if hunt.is_realm_hub() {
                "NO_COMPLETION"
            } else {
                "FIRST_DEFEAT"
            });
        let metadata_nonce = hunt
            .metadata_nonce
            .clone()
            .unwrap_or_else(|| "0123456789abcdef0123456789abcdef".to_owned());
        let host_peer = if hunt.host_steam_id.is_empty() {
            derive_ice_peer_id(&hunt.host_account_id)
        } else {
            hunt.host_steam_id.clone()
        };
        json!({
            "keyId": key_id,
            "sessionId": hunt.session_id,
            "partyId": hunt.party_id.replace('@', ":"),
            "partyRevision": hunt.party_revision.max(1),
            "huntDefinitionId": hunt.hunt_definition_id,
            "generation": hunt.generation.max(1),
            "build": hunt.build.max(1),
            "bridgeProtocol": hunt.bridge_protocol.clamp(1, 255),
            "virtualPort": hunt.virtual_port.clamp(1, 65535),
            "hostPeerId": host_peer,
            "hostSteamId": host_peer,
            "bridgeInstanceId": hunt.bridge_instance_id,
            "metadataNonce": metadata_nonce,
            "forceRelay": self.settings.policy.force_sdr,
            "guardCompletionMode": guard_completion_mode,
            "sessionRole": if hunt.is_realm_hub() { "HUB" } else { "HUNT" },
            "hubKind": hunt.hub_kind.map(hub_kind_name),
            "capability": capability.expose(),
            "hostTicket": host_ticket.map(Secret::expose),
            "backendUrl": backend_url,
            "serverUdpPort": server_udp_port,
            "maximumPeers": maximum_peers,
            "hostLocalClient": host_local_client,
            "huntEvidence": hunt_evidence,
            "matchmakerHuntId": matchmaker_hunt_id,
            "mapPath": map_path,
            "behemothPath": behemoth_path,
            "expectedPlayers": expected_players,
            "expectedPlayerString": expected_players,
            "runtimeProfile": "production",
            "maxPlayers": hunt.is_realm_hub().then_some(maximum_peers),
            "hubAfkSeconds": afk_seconds,
        })
    }

    /// `POST /launcher/v1/p2p/hunts/:sessionId/host-bootstrap` (`CreateHostBootstrap`).
    pub async fn create_host_bootstrap(&self, account: &str, raw_session_id: &str) -> Result<Value, RealmsError> {
        if !self.settings.policy.enabled {
            return Err(refused("P2P_DISABLED", "Player-hosted hunts are disabled."));
        }
        if self.emergency_stop().await? {
            return Err(refused(
                "P2P_EMERGENCY_STOP",
                "Player-hosted hunts are stopped by operations policy.",
            ));
        }
        let Some(runtime) = &self.runtime else {
            return Err(refused("P2P_DISABLED", "Host bootstrap runtime is not configured."));
        };
        let hunt = self
            .find_session(raw_session_id)
            .await?
            .ok_or_else(|| refused("P2P_SESSION_NOT_FOUND", "Hunt session not found."))?;
        if hunt.host_account_id != account {
            return Err(refused(
                "P2P_NOT_ROSTERED",
                "Only the selected host may bootstrap this session.",
            ));
        }
        let now = self.now();
        let default_port = i64::from(runtime.host_server_udp_port);
        let bridge_instance_id = if hunt.bridge_instance_id.is_empty() || hunt.state == HuntState::HostOffered {
            format!("bridge-{}", random_uuid())
        } else {
            hunt.bridge_instance_id.clone()
        };
        let metadata_nonce = hunt
            .metadata_nonce
            .clone()
            .unwrap_or_else(|| crate::ids::hex(&rand::random::<[u8; 16]>()));
        let startup_deadline = UnixMillis(now.0 + self.settings.eligibility.startup_lifetime_ms);
        let actor = Actor::Account(AccountId(account.to_owned()));
        let updated = if hunt.state == HuntState::HostOffered {
            let startup = HostStartup {
                session_id: hunt.session_id.clone(),
                host_account_id: account.to_owned(),
                bridge_instance_id: bridge_instance_id.clone(),
                metadata_nonce: metadata_nonce.clone(),
                startup_deadline,
                server_udp_port: Some(hunt.server_udp_port.unwrap_or(default_port)),
            };
            let started = self
                .hunts
                .transaction(&actor, move |tx| {
                    let Some(next) = tx.begin_host_startup(&startup, now)? else {
                        return Ok::<_, RealmsError>(None);
                    };
                    tx.append_transition(&HuntTransition {
                        transition_id: String::new(),
                        session_id: next.session_id.clone(),
                        generation: next.generation,
                        from_state: HuntState::HostOffered,
                        to_state: HuntState::Starting,
                        state_version: next.state_version,
                        reason: "HOST_BOOTSTRAP_ISSUED".to_owned(),
                        actor_type: TransitionActor::Host.as_str().to_owned(),
                        actor_account_id: Some(startup.host_account_id),
                        at: now,
                    })?;
                    Ok(Some(next))
                })
                .await?;
            started.ok_or_else(|| {
                refused(
                    "P2P_GENERATION_FENCED",
                    "The session is no longer waiting for host startup.",
                )
            })?
        } else if matches!(
            hunt.state,
            HuntState::Starting | HuntState::HostReady | HuntState::Admitting | HuntState::Running
        ) {
            hunt
        } else {
            return Err(refused(
                "P2P_GENERATION_FENCED",
                "This session generation cannot be bootstrapped.",
            ));
        };

        let host_peer = if updated.host_steam_id.is_empty() {
            derive_ice_peer_id(account)
        } else {
            updated.host_steam_id.clone()
        };
        self.ensure_verified_link(account, &host_peer, now);

        let scopes = if updated.is_realm_hub() {
            HUB_CAPABILITY_SCOPES.to_vec()
        } else {
            HUNT_CAPABILITY_SCOPES.to_vec()
        };
        let cap_expires = if updated.is_realm_hub() {
            UnixMillis(now.0 + self.settings.realm_hub.capability_ttl_ms)
        } else {
            updated.expires_at
        };
        let (capability, _) = runtime
            .capabilities
            .mint(&updated, &updated.bridge_instance_id, scopes, now, cap_expires)
            .await?;
        let host_ticket = if updated.host_participates.unwrap_or(true) {
            self.mint_self_ticket(&updated, account, &host_peer, now)
        } else {
            None
        };
        Ok(self.build_host_bootstrap_json(&updated, &capability, host_ticket.as_ref(), None))
    }

    /// `POST /mystic/v1/hunts/:sessionId/host-ready` (`MarkHostReady`).
    pub async fn mark_host_ready(&self, capability: &HostCapabilityRecord, body: &Value) -> Result<Value, RealmsError> {
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
        let lease_expires_at = UnixMillis(now.0 + self.settings.eligibility.lease_lifetime_ms);
        if matches!(
            hunt.state,
            HuntState::HostReady | HuntState::Admitting | HuntState::Running
        ) {
            return Ok(json!({
                "ready": true,
                "sessionId": hunt.session_id,
                "generation": hunt.generation,
                "state": hunt.state.as_str(),
                "leaseExpiresAt": iso(hunt.lease_expires_at.unwrap_or(lease_expires_at)),
            }));
        }
        let lobby_id = string_in(body.get("lobbyId"), 64).unwrap_or_else(|| "ICE".to_owned());
        let virtual_port = integer_in(body.get("virtualPort"), 1.0, 65_535.0).unwrap_or(hunt.virtual_port);
        let mark = HostReadyMark {
            session_id: capability.session_id.clone(),
            generation: capability.generation,
            host_steam_id: capability.host_steam_id.clone(),
            bridge_instance_id: capability.bridge_instance_id.clone(),
            lobby_id,
            virtual_port,
            lease_expires_at,
        };
        let host_account = capability.host_account_id.clone();
        let actor = Actor::Account(AccountId(host_account.clone()));
        let prev_state = hunt.state;
        let updated = self
            .hunts
            .transaction(&actor, move |tx| {
                let Some(next) = tx.mark_host_ready(&mark, now)? else {
                    return Err(refused(
                        "P2P_GENERATION_FENCED",
                        "The hunt generation is not waiting for host readiness.",
                    ));
                };
                if prev_state != HuntState::HostReady {
                    tx.append_transition(&HuntTransition {
                        transition_id: String::new(),
                        session_id: next.session_id.clone(),
                        generation: next.generation,
                        from_state: prev_state,
                        to_state: HuntState::HostReady,
                        state_version: next.state_version,
                        reason: "HOST_READY".to_owned(),
                        actor_type: TransitionActor::Host.as_str().to_owned(),
                        actor_account_id: Some(host_account.clone()),
                        at: now,
                    })?;
                    tx.increment_reliability(
                        &host_account,
                        ReliabilityIncrements {
                            starts_succeeded: 1,
                            ..Default::default()
                        },
                        now,
                        None,
                    )?;
                    if !next.is_realm_hub() {
                        let expires_at = UnixMillis(now.0 + CONTROL_MESSAGE_TTL_MS);
                        for p in &next.roster {
                            if p.account_id != host_account {
                                tx.enqueue_control(&NewControlMessage {
                                    account_id: p.account_id.clone(),
                                    message_type: "JOIN_BOOTSTRAP".to_owned(),
                                    session_id: Some(next.session_id.clone()),
                                    generation: Some(next.generation),
                                    payload: json!({
                                        "sessionId": next.session_id,
                                        "generation": next.generation,
                                        "role": "JOIN",
                                    }),
                                    created_at: now,
                                    expires_at,
                                })?;
                            }
                        }
                    }
                }
                Ok(next)
            })
            .await?;

        if updated.is_realm_hub() {
            self.mirror_hub(&updated, HubState::Starting).await;
        }
        Ok(json!({
            "ready": true,
            "sessionId": updated.session_id,
            "generation": updated.generation,
            "state": updated.state.as_str(),
            "leaseExpiresAt": iso(lease_expires_at),
        }))
    }

    /// `POST /launcher/v1/p2p/hunts/:sessionId/route-ready` (`MarkLocalRouteReady`).
    pub async fn mark_local_route_ready(
        &self,
        account: &str,
        raw_session_id: &str,
        body: &Value,
    ) -> Result<Value, RealmsError> {
        let local_port = integer_in(
            body.get("localBridgePort").or_else(|| body.get("localPort")),
            1.0,
            65_535.0,
        )
        .ok_or_else(|| refused("P2P_TICKET_INVALID", "localBridgePort must be between 1 and 65535."))?;
        let mut hunt = self
            .find_session(raw_session_id)
            .await?
            .ok_or_else(|| refused("P2P_SESSION_NOT_FOUND", "Hunt session not found."))?;
        if let Some(expected_gen) = body.get("generation").and_then(Value::as_i64)
            && expected_gen != hunt.generation
        {
            return Err(refused(
                "P2P_GENERATION_FENCED",
                "The route-ready report belongs to a stale generation.",
            ));
        }
        let now = self.now();
        let actor = Actor::Account(AccountId(account.to_owned()));

        if hunt.is_realm_hub() {
            let Some(realm_id) = hunt.realm_id.clone() else {
                return Err(refused("P2P_SESSION_NOT_FOUND", "Hub session has no realm."));
            };
            if account == hunt.host_account_id {
                let lease_expires_at = UnixMillis(now.0 + self.settings.eligibility.lease_lifetime_ms);
                let id = hunt.session_id.clone();
                let gen_i64 = hunt.generation;
                let host_acc = account.to_owned();
                let updated = self
                    .hunts
                    .transaction(&actor, move |tx| {
                        let Some(cur) = tx.find_hunt(&id)? else {
                            return Ok::<_, RealmsError>(None);
                        };
                        let from = cur.state;
                        let next = tx.compare_and_set(
                            &id,
                            gen_i64,
                            &[
                                HuntState::Starting,
                                HuntState::HostReady,
                                HuntState::Admitting,
                                HuntState::Running,
                            ],
                            cur.state_version,
                            move |h| {
                                h.state = HuntState::Running;
                                h.host_local_bridge_port = Some(local_port);
                                h.host_local_bridge_ready_at = Some(now);
                                if h.lobby_id.is_none() {
                                    h.lobby_id = Some("ICE".to_owned());
                                }
                                if h.lease_expires_at.is_none() {
                                    h.lease_expires_at = Some(lease_expires_at);
                                    h.last_heartbeat_at = Some(now);
                                    h.last_lease_sequence = Some(0);
                                }
                            },
                        )?;
                        if let Some(ref next_session) = next
                            && from != HuntState::Running
                        {
                            tx.append_transition(&HuntTransition {
                                transition_id: String::new(),
                                session_id: id,
                                generation: gen_i64,
                                from_state: from,
                                to_state: HuntState::Running,
                                state_version: next_session.state_version,
                                reason: "HUB_HOST_ROUTE_READY".to_owned(),
                                actor_type: TransitionActor::Host.as_str().to_owned(),
                                actor_account_id: Some(host_acc),
                                at: now,
                            })?;
                        }
                        Ok(next)
                    })
                    .await?
                    .ok_or_else(|| refused("P2P_GENERATION_FENCED", "Hub session is not active."))?;
                let peer_id = if updated.host_steam_id.is_empty() {
                    derive_ice_peer_id(account)
                } else {
                    updated.host_steam_id.clone()
                };
                self.hub
                    .admit_host(
                        &actor,
                        &updated.session_id,
                        updated.generation,
                        &realm_id,
                        account,
                        &peer_id,
                        local_port,
                        now,
                    )
                    .await?;
                if let Some(rt) = &self.runtime {
                    rt.capabilities.hub_presence().invalidate(&updated.session_id);
                }
                self.mirror_hub(&updated, HubState::Running).await;
                self.update_matchmaking_candidate_port(account, &updated, local_port);
                return Ok(json!({
                    "ready": true,
                    "sessionId": updated.session_id,
                    "generation": updated.generation,
                    "localBridgePort": local_port,
                    "state": updated.state.as_str(),
                }));
            }

            let row = self
                .hub
                .mark_route_ready(&actor, &hunt.session_id, hunt.generation, account, local_port, now, now)
                .await?;
            if row.is_none() {
                return Err(refused(
                    "P2P_NOT_ROSTERED",
                    "This account has no active entry into the realm hub.",
                ));
            }
            if let Some(rt) = &self.runtime {
                rt.capabilities.hub_presence().invalidate(&hunt.session_id);
            }
            self.update_matchmaking_candidate_port(account, &hunt, local_port);
            return Ok(json!({
                "ready": true,
                "sessionId": hunt.session_id,
                "generation": hunt.generation,
                "localBridgePort": local_port,
                "state": hunt.state.as_str(),
            }));
        }

        // Regular hunt: if the host reports route-ready while still in STARTING (e.g. MockTransport), advance to HOST_READY first.
        if account == hunt.host_account_id && hunt.state == HuntState::Starting {
            let cap = HostCapabilityRecord {
                capability_id: String::new(),
                secret_digest: String::new(),
                session_id: hunt.session_id.clone(),
                generation: hunt.generation,
                host_account_id: hunt.host_account_id.clone(),
                host_steam_id: hunt.host_steam_id.clone(),
                bridge_instance_id: hunt.bridge_instance_id.clone(),
                scopes: HUNT_CAPABILITY_SCOPES.to_vec(),
                issued_at: now,
                expires_at: hunt.expires_at,
                revoked_at: None,
                last_sequence: 0,
            };
            let _ = self.mark_host_ready(&cap, &json!({})).await?;
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
        let bridge_instance = string_in(body.get("bridgeInstanceId"), 96)
            .or(participant.local_bridge_instance_id)
            .unwrap_or_else(|| format!("local-{}", random_uuid()));
        let updated = self
            .hunts
            .mark_participant_route_ready(
                &actor,
                &hunt.session_id,
                hunt.generation,
                account,
                &participant.steam_id,
                &bridge_instance,
                local_port,
                now,
            )
            .await?
            .ok_or_else(|| {
                refused(
                    "P2P_GENERATION_FENCED",
                    "The hunt generation does not accept route updates.",
                )
            })?;
        self.update_matchmaking_candidate_port(account, &updated, local_port);
        Ok(json!({
            "ready": true,
            "sessionId": updated.session_id,
            "generation": updated.generation,
            "localBridgePort": local_port,
            "state": updated.state.as_str(),
        }))
    }

    fn update_matchmaking_candidate_port(&self, account: &str, hunt: &HuntSession, local_port: i64) {
        if let Some(rt) = &self.runtime {
            let candidate_id = hunt.participant(account).map_or_else(
                || format!("cand-{}-{account}", hunt.session_id),
                |p| p.candidate_id.clone(),
            );
            let game_session_id = hunt.hub_room_id.clone().unwrap_or_else(|| hunt.session_id.clone());
            let mut obj = json!({
                "HuntId": hunt.hunt_definition_id,
                "CandidateId": candidate_id,
                "GameSessionId": game_session_id,
                "Host": "127.0.0.1",
                "Port": local_port,
                "Ready": true,
                "ReadyAt": 0,
            });
            if hunt.is_realm_hub()
                && let Some(map) = obj.as_object_mut()
            {
                map.insert("Source".to_owned(), Value::String("REALM_HUB".to_owned()));
                map.insert("SourceSessionId".to_owned(), Value::String(hunt.session_id.clone()));
            }
            let candidate = rifts_seam::Candidate(Json(obj.to_string()));
            let _ = rt.core.set_candidate(AccountId(account.to_owned()), candidate);
        }
    }

    /// `POST /mystic/v1/hunts/:sessionId/abort` (`AbortHunt`).
    pub async fn abort_hunt(&self, capability: &HostCapabilityRecord, body: &Value) -> Result<Value, RealmsError> {
        let reason = string_in(body.get("reason"), 128).unwrap_or_else(|| "HOST_ABORTED".to_owned());
        let hunt = self
            .hunts
            .find_hunt(&capability.session_id)
            .await?
            .ok_or_else(|| refused("P2P_SESSION_NOT_FOUND", "Hunt session not found."))?;
        if hunt.generation != capability.generation {
            return Err(refused("P2P_GENERATION_FENCED", "Stale hunt generation."));
        }
        let actor = Actor::Account(AccountId(capability.host_account_id.clone()));
        let ended = if hunt.is_realm_hub() {
            self.end_hub(
                &hunt.session_id,
                hunt.generation,
                &reason,
                HuntState::Aborted,
                TransitionActor::Host,
                Some(&capability.host_account_id),
            )
            .await?
        } else {
            self.end_hunt(
                &actor,
                &hunt,
                &reason,
                &[
                    HuntState::Starting,
                    HuntState::HostReady,
                    HuntState::Admitting,
                    HuntState::Running,
                    HuntState::Draining,
                    HuntState::Suspect,
                ],
                TransitionActor::Host,
                Some(&capability.host_account_id),
                true,
            )
            .await?
        };
        let state = ended.as_ref().map_or(hunt.state.as_str(), |h| h.state.as_str());
        Ok(json!({
            "aborted": true,
            "sessionId": hunt.session_id,
            "generation": hunt.generation,
            "state": state,
        }))
    }

    /// `POST /mystic/v1/hunts/:sessionId/end` (`EndHunt`).
    pub async fn finish_hunt(&self, capability: &HostCapabilityRecord, body: &Value) -> Result<Value, RealmsError> {
        let reason = string_in(body.get("reason"), 128).unwrap_or_else(|| "HOST_COMPLETED".to_owned());
        let hunt = self
            .hunts
            .find_hunt(&capability.session_id)
            .await?
            .ok_or_else(|| refused("P2P_SESSION_NOT_FOUND", "Hunt session not found."))?;
        if hunt.generation != capability.generation {
            return Err(refused("P2P_GENERATION_FENCED", "Stale hunt generation."));
        }
        let actor = Actor::Account(AccountId(capability.host_account_id.clone()));
        let ended = if hunt.is_realm_hub() {
            self.end_hub(
                &hunt.session_id,
                hunt.generation,
                &reason,
                HuntState::Completed,
                TransitionActor::Host,
                Some(&capability.host_account_id),
            )
            .await?
        } else {
            self.end_hunt(
                &actor,
                &hunt,
                &reason,
                HOST_END_STATES,
                TransitionActor::Host,
                Some(&capability.host_account_id),
                true,
            )
            .await?
        };
        let state = ended.as_ref().map_or(hunt.state.as_str(), |h| h.state.as_str());
        Ok(json!({
            "ended": true,
            "sessionId": hunt.session_id,
            "generation": hunt.generation,
            "state": state,
        }))
    }

    /// `GET /mystic/v1/hunts/:sessionId/status` (`GetHuntStatus`).
    pub async fn get_hunt_status(
        &self,
        caller_account: Option<&str>,
        raw_session_id: &str,
    ) -> Result<Value, RealmsError> {
        let hunt = self
            .find_session(raw_session_id)
            .await?
            .ok_or_else(|| refused("P2P_SESSION_NOT_FOUND", "Hunt session not found."))?;
        if let Some(account) = caller_account {
            let allowed = hunt.host_account_id == account
                || hunt.participant(account).is_some()
                || (hunt.is_realm_hub() && self.hub.find_presence(&hunt.session_id, account).await?.is_some());
            if !allowed {
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

    /// `POST /launcher/v1/realm-hosting/hubs/start` (`CreateRealmHubSession`).
    pub async fn start_realm_hub(&self, account: &str, body: &Value) -> Result<Value, RealmsError> {
        if !self.settings.realm_hosting || !self.settings.policy.enabled {
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
        let Some(runtime) = &self.runtime else {
            return Err(refused(
                "REALM_HOSTING_DISABLED",
                "Realm hosting runtime is not configured.",
            ));
        };
        let (kind, max_players) = parse_hub_start_body(body).map_err(|msg| refused("REALM_VALIDATION_FAILED", msg))?;
        let afk_seconds = body
            .get("afkSeconds")
            .or_else(|| body.get("hubAfkSeconds"))
            .and_then(|v| integer_in(Some(v), 0.0, 7200.0))
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or(0);

        let realm = self
            .realms
            .find_by_host(account)
            .await?
            .ok_or_else(|| refused("REALM_NOT_FOUND", "You do not host an active realm."))?;

        let party_key = hub_party_key(&realm.realm_id, kind);
        if let Some(existing) = self.hunts.find_active_hunt_for_party(&party_key).await? {
            let _ = self
                .end_hub(
                    &existing.session_id,
                    existing.generation,
                    "SUPERSEDED_BY_HOST",
                    HuntState::Completed,
                    TransitionActor::Host,
                    Some(account),
                )
                .await?;
        }

        let now = self.now();
        let active_servers = self.hunts.count_active_realm_servers(&realm.realm_id).await?;
        let max_servers = realm
            .hosting
            .as_ref()
            .map_or(self.settings.realm_hub.default_max_servers, |h| h.max_servers);
        if active_servers >= max_servers {
            return Err(refused(
                "REALM_SERVER_LIMIT",
                "Your realm has reached its active server limit.",
            ));
        }

        let presences = self
            .hunts
            .list_host_presences(
                &[account.to_owned()],
                UnixMillis(now.0 - self.settings.eligibility.presence_maximum_age_ms),
                now,
            )
            .await?;
        if let Some(presence) = presences.first() {
            if let Some(reason) = realm_host_ineligibility_reason(
                presence,
                &self.settings.eligibility,
                0.0,
                0.0,
                self.operations.guard_refuses(),
                now,
            ) {
                return Err(refused(
                    "P2P_HOST_INELIGIBLE",
                    &format!("Host presence is ineligible ({reason})."),
                ));
            }
            let cap = decide_realm_capacity(&crate::hub::RealmCapacityInput {
                active_servers,
                max_servers,
                free_physical_memory_mib: presence.free_physical_memory_mib,
                commit_headroom_mib: presence.commit_headroom_mib,
                minimum_free_memory_mib: self.settings.realm_hub.minimum_free_memory_mib,
                minimum_commit_headroom_mib: self.settings.realm_hub.minimum_commit_headroom_mib,
            });
            if matches!(
                cap,
                crate::hub::RealmCapacity::ServerLimit | crate::hub::RealmCapacity::CommitLow
            ) {
                return Err(refused(
                    "REALM_CAPACITY_EXCEEDED",
                    "Insufficient host capacity to start another realm server.",
                ));
            }
        }

        let session_id = uuid_text(new_session_ulid(now));
        let manifest = hub_manifest(kind);
        let host_peer = derive_ice_peer_id(account);
        self.ensure_verified_link(account, &host_peer, now);

        let bridge_instance_id = format!("bridge-{}", random_uuid());
        let metadata_nonce = crate::ids::hex(&rand::random::<[u8; 16]>());
        let port_offset = i64::from(kind == HubKind::Training);
        let server_udp_port = i64::from(runtime.host_server_udp_port) + port_offset;
        let startup_deadline = UnixMillis(now.0 + self.settings.eligibility.startup_lifetime_ms);
        let expires_at = UnixMillis(now.0 + HUB_SESSION_LIFETIME_MS);

        let session = HuntSession {
            session_id: session_id.clone(),
            generation: 1,
            state: HuntState::Starting,
            state_version: 1,
            party_id: party_key.clone(),
            active_party_key: Some(party_key),
            party_revision: 1,
            hunt_definition_id: manifest.hunt_definition_id.to_owned(),
            game_mode: Some(manifest.game_mode.to_owned()),
            build: DEFAULT_GAME_BUILD,
            bridge_protocol: DEFAULT_BRIDGE_PROTOCOL,
            virtual_port: DEFAULT_VIRTUAL_PORT,
            host_account_id: account.to_owned(),
            host_steam_id: host_peer.clone(),
            bridge_instance_id: bridge_instance_id.clone(),
            roster: Vec::new(),
            lobby_id: Some("ICE".to_owned()),
            coop_transport: Some("ICE".to_owned()),
            lease_expires_at: Some(UnixMillis(now.0 + self.settings.eligibility.lease_lifetime_ms)),
            created_at: now,
            expires_at,
            metadata_nonce: Some(metadata_nonce),
            hub_kind: Some(kind),
            realm_id: Some(realm.realm_id.clone()),
            host_participates: Some(true),
            host_local_bridge_port: None,
            server_udp_port: Some(server_udp_port),
            host_local_bridge_ready_at: None,
            hub_room_id: Some(hub_room_id(&realm.realm_id, kind)),
            max_peers: Some(max_players),
            guard_completion_mode: Some(manifest.guard_completion_mode.to_owned()),
            host_offer_id: None,
            offer_expires_at: None,
            startup_deadline: Some(startup_deadline),
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
            trust_tier: Some(i64::from(self.settings.policy.trust_tier)),
            guard_encounter_state: None,
            extra: serde_json::Map::new(),
        };

        let actor = Actor::Account(AccountId(account.to_owned()));
        self.hunts.create_hunt(&actor, &session).await?;
        self.mirror_hub(&session, HubState::Starting).await;

        let cap_expires = UnixMillis(now.0 + self.settings.realm_hub.capability_ttl_ms);
        let (capability, _) = runtime
            .capabilities
            .mint(
                &session,
                &bridge_instance_id,
                HUB_CAPABILITY_SCOPES.to_vec(),
                now,
                cap_expires,
            )
            .await?;
        let host_ticket = self.mint_self_ticket(&session, account, &host_peer, now);
        Ok(self.build_host_bootstrap_json(&session, &capability, host_ticket.as_ref(), Some(afk_seconds)))
    }

    /// `POST /launcher/v1/realm-hosting/hubs/stop` (`StopRealmHubSession`).
    pub async fn stop_realm_hub(&self, account: &str, body: &Value) -> Result<Value, RealmsError> {
        let kind = parse_hub_stop_body(body).map_err(|msg| refused("REALM_VALIDATION_FAILED", msg))?;
        let realm = self
            .realms
            .find_by_host(account)
            .await?
            .ok_or_else(|| refused("REALM_NOT_FOUND", "You do not host an active realm."))?;
        let party_key = hub_party_key(&realm.realm_id, kind);
        let Some(hub) = self.hunts.find_active_hunt_for_party(&party_key).await? else {
            return Ok(json!({
                "stopped": false,
                "hubKind": hub_kind_name(kind),
                "sessionId": null,
            }));
        };
        let ended = self
            .end_hub(
                &hub.session_id,
                hub.generation,
                "HOST_STOPPED_HUB",
                HuntState::Completed,
                TransitionActor::Host,
                Some(account),
            )
            .await?;
        Ok(json!({
            "stopped": ended.is_some(),
            "hubKind": hub_kind_name(kind),
            "sessionId": hub.session_id,
        }))
    }

    /// `GET /launcher/v1/realm-hosting/status` (`GetRealmHostingStatus`).
    pub async fn get_realm_hosting_status(&self, account: &str) -> Result<Value, RealmsError> {
        let enabled = self.settings.realm_hosting && self.settings.policy.enabled;
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
        let mut hub_summaries = BTreeMap::new();
        for (key, kind) in [("ramsgate", HubKind::Ramsgate), ("training", HubKind::Training)] {
            let party_key = hub_party_key(&realm.realm_id, kind);
            if let Some(hub) = self.hunts.find_active_hunt_for_party(&party_key).await? {
                let admitted = self
                    .hub
                    .count_by_state(&hub.session_id, hub.generation, &[PresenceState::Admitted])
                    .await?;
                hub_summaries.insert(
                    key,
                    json!({
                        "sessionId": hub.session_id,
                        "generation": hub.generation,
                        "state": hub.state.as_str(),
                        "admittedPlayers": admitted,
                        "maxPlayers": hub.max_peers.unwrap_or(16),
                    }),
                );
            } else {
                hub_summaries.insert(key, Value::Null);
            }
        }
        let hunts = self.hunts.list_live_realm_hunts(&realm.realm_id, account).await?;
        let active_hunts: Vec<Value> = hunts
            .into_iter()
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
        let hosting_json = realm.hosting.as_ref().map(|h| {
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
        Ok(json!({
            "enabled": enabled,
            "realmId": realm.realm_id,
            "realmName": realm.name,
            "hosting": hosting_json,
            "ramsgate": hub_summaries.remove("ramsgate").unwrap_or(Value::Null),
            "training": hub_summaries.remove("training").unwrap_or(Value::Null),
            "activeHunts": active_hunts,
        }))
    }

    /// Creates a new player-hosted hunt session for a party or solo player and starts host selection.
    pub async fn create_routed_hunt(
        &self,
        party_id: &str,
        party_revision: i64,
        hunt_definition_id: &str,
        game_mode: Option<&str>,
        members: &[String],
        realm_id: Option<String>,
    ) -> Result<HuntSession, RealmsError> {
        let now = self.now();
        let session_id = uuid_text(new_session_ulid(now));
        let roster: Vec<Participant> = members
            .iter()
            .map(|acc| {
                let peer = derive_ice_peer_id(acc);
                self.ensure_verified_link(acc, &peer, now);
                Participant::pending(acc, &peer, &format!("cand-{session_id}-{acc}"))
            })
            .collect();
        let session = HuntSession {
            session_id: session_id.clone(),
            generation: 1,
            state: HuntState::HostSelecting,
            state_version: 1,
            party_id: party_id.to_owned(),
            active_party_key: Some(party_id.to_owned()),
            party_revision: party_revision.max(1),
            hunt_definition_id: hunt_definition_id.to_owned(),
            game_mode: game_mode.map(str::to_owned),
            build: DEFAULT_GAME_BUILD,
            bridge_protocol: DEFAULT_BRIDGE_PROTOCOL,
            virtual_port: DEFAULT_VIRTUAL_PORT,
            host_account_id: members.first().cloned().unwrap_or_default(),
            host_steam_id: members.first().map_or_else(String::new, |a| derive_ice_peer_id(a)),
            bridge_instance_id: String::new(),
            roster,
            lobby_id: None,
            coop_transport: Some("ICE".to_owned()),
            lease_expires_at: None,
            created_at: now,
            expires_at: UnixMillis(now.0 + 2 * 3_600_000),
            metadata_nonce: Some(crate::ids::hex(&rand::random::<[u8; 16]>())),
            hub_kind: None,
            realm_id,
            host_participates: Some(true),
            host_local_bridge_port: None,
            server_udp_port: self.runtime.as_ref().map(|rt| i64::from(rt.host_server_udp_port)),
            host_local_bridge_ready_at: None,
            hub_room_id: None,
            max_peers: Some(4),
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
            trust_tier: Some(i64::from(self.settings.policy.trust_tier)),
            guard_encounter_state: None,
            extra: serde_json::Map::new(),
        };
        let actor = members
            .first()
            .map_or(Actor::System("realms-route"), |a| Actor::Account(AccountId(a.clone())));
        self.hunts.create_hunt(&actor, &session).await?;
        self.begin_host_selection(&session_id).await?;
        Ok(self.hunts.find_hunt(&session_id).await?.unwrap_or(session))
    }

    pub(crate) async fn restore_matchmaking_candidate(
        &self,
        player: &str,
    ) -> Result<Option<rifts_seam::Candidate>, RealmsError> {
        if let Some(hunt) = self.hunts.find_active_hunt_for_account(player).await?
            && let Some(participant) = hunt.participant(player)
        {
            let ready = participant.local_bridge_ready_at.is_some() && participant.local_bridge_port.is_some();
            let obj = json!({
                "HuntId": hunt.hunt_definition_id,
                "CandidateId": participant.candidate_id,
                "GameSessionId": hunt.session_id,
                "Host": if ready { "127.0.0.1" } else { "" },
                "Port": participant.local_bridge_port.unwrap_or(0),
                "Ready": ready,
                "ReadyAt": 0,
            });
            return Ok(Some(rifts_seam::Candidate(Json(obj.to_string()))));
        }
        if let Some(row) = self.hub.find_for_account(player, None).await?
            && let Some(hub) = self.hunts.find_hunt(&row.session_id).await?
            && is_hub_live(hub.state)
        {
            let ready = row.local_bridge_ready_at.is_some() && row.local_bridge_port.is_some();
            let game_session_id = hub.hub_room_id.unwrap_or_else(|| hub.session_id.clone());
            let obj = json!({
                "HuntId": hub.hunt_definition_id,
                "CandidateId": row.candidate_id,
                "GameSessionId": game_session_id,
                "Host": if ready { "127.0.0.1" } else { "" },
                "Port": row.local_bridge_port.unwrap_or(0),
                "Ready": ready,
                "ReadyAt": 0,
                "Source": "REALM_HUB",
                "SourceSessionId": hub.session_id,
            });
            return Ok(Some(rifts_seam::Candidate(Json(obj.to_string()))));
        }
        Ok(None)
    }

    pub(crate) async fn poll_candidate_status(
        &self,
        player: &str,
        current: &rifts_seam::Candidate,
    ) -> Result<Option<rifts_seam::Candidate>, RealmsError> {
        let Ok(mut val) = serde_json::from_str::<Value>(&current.0.0) else {
            return Ok(None);
        };
        if val.get("Source").and_then(Value::as_str) == Some("REALM_HUB") {
            let Some(session_id) = val.get("SourceSessionId").and_then(Value::as_str).map(str::to_owned) else {
                return Ok(None);
            };
            let Some(hub) = self.hunts.find_hunt(&session_id).await? else {
                return Ok(None);
            };
            if matches!(hub.state, HuntState::Completed | HuntState::Aborted) {
                if let Some(map) = val.as_object_mut() {
                    map.insert("Ready".to_owned(), Value::Bool(false));
                    map.insert("Canceled".to_owned(), Value::Bool(true));
                    map.insert(
                        "StatusReason".to_owned(),
                        Value::String(
                            hub.failure_reason
                                .or(hub.completion_reason)
                                .unwrap_or_else(|| "HUB_ENDED".to_owned()),
                        ),
                    );
                }
                return Ok(Some(rifts_seam::Candidate(Json(val.to_string()))));
            }
            if let Some(row) = self.hub.find_presence(&session_id, player).await?
                && let Some(port) = row.local_bridge_port
                && row.local_bridge_ready_at.is_some()
            {
                if let Some(map) = val.as_object_mut() {
                    map.insert("Ready".to_owned(), Value::Bool(true));
                    map.insert("Host".to_owned(), Value::String("127.0.0.1".to_owned()));
                    map.insert("Port".to_owned(), Value::from(port));
                }
                return Ok(Some(rifts_seam::Candidate(Json(val.to_string()))));
            }
            return Ok(Some(rifts_seam::Candidate(Json(val.to_string()))));
        }

        let Some(game_session_id) = val.get("GameSessionId").and_then(Value::as_str).map(str::to_owned) else {
            return Ok(None);
        };
        let Some(hunt) = self.hunts.find_hunt(&game_session_id).await? else {
            return Ok(None);
        };
        if hunt.state == HuntState::Aborted {
            if let Some(map) = val.as_object_mut() {
                map.insert("Ready".to_owned(), Value::Bool(false));
                map.insert("Canceled".to_owned(), Value::Bool(true));
                map.insert(
                    "StatusReason".to_owned(),
                    Value::String(hunt.failure_reason.unwrap_or_else(|| "HUNT_ABORTED".to_owned())),
                );
            }
            return Ok(Some(rifts_seam::Candidate(Json(val.to_string()))));
        }
        if let Some(participant) = hunt.participant(player)
            && let Some(port) = participant.local_bridge_port
            && participant.local_bridge_ready_at.is_some()
            && let Some(map) = val.as_object_mut()
        {
            map.insert("Ready".to_owned(), Value::Bool(true));
            map.insert("Host".to_owned(), Value::String("127.0.0.1".to_owned()));
            map.insert("Port".to_owned(), Value::from(port));
        }
        Ok(Some(rifts_seam::Candidate(Json(val.to_string()))))
    }

    pub(crate) async fn route_matchmaking_hunt(
        &self,
        context: &rifts_seam::HuntRouteContext,
        realm_id: Option<String>,
    ) -> Result<rifts_seam::HuntRouteOutcome, RealmsError> {
        use crate::hub::{RealmRequest, classify_realm_request};
        let Some(rt) = &self.runtime else {
            return Ok(rifts_seam::HuntRouteOutcome::NotHandled);
        };
        let now = self.now();
        let req_class = classify_realm_request(Some(&context.game_mode), Some(&context.hunt_id));
        let members: Vec<String> = if context.party_members.is_empty() {
            vec![context.player.0.clone()]
        } else {
            context.party_members.iter().map(|a| a.0.clone()).collect()
        };

        if let Some(ref realm_id_str) = realm_id
            && let RealmRequest::Hub(kind) = req_class
        {
            let party_key = hub_party_key(realm_id_str, kind);
            let Some(hub) = self.hunts.find_active_hunt_for_party(&party_key).await? else {
                let accounts: Vec<AccountId> = members.iter().cloned().map(AccountId).collect();
                let _ = rt
                    .core
                    .cancel_travel(&context.hunt_id, &accounts, "REALM_HUB_NOT_RUNNING");
                return Ok(rifts_seam::HuntRouteOutcome::Handled { result: true });
            };
            if !is_hub_live(hub.state) {
                let accounts: Vec<AccountId> = members.iter().cloned().map(AccountId).collect();
                let _ = rt
                    .core
                    .cancel_travel(&context.hunt_id, &accounts, "REALM_HUB_NOT_RUNNING");
                return Ok(rifts_seam::HuntRouteOutcome::Handled { result: true });
            }
            let game_session_id = hub
                .hub_room_id
                .clone()
                .unwrap_or_else(|| hub_room_id(realm_id_str, kind));
            let expires_at = UnixMillis(now.0 + CONTROL_MESSAGE_TTL_MS);

            // For non-host members, create/reuse hub presence entries and enqueue REALM_HUB_JOIN.
            let mut entry_reqs = Vec::new();
            for acc in &members {
                if acc != &hub.host_account_id {
                    let peer = derive_ice_peer_id(acc);
                    self.ensure_verified_link(acc, &peer, now);
                    entry_reqs.push(HubEntryRequest {
                        account_id: acc.clone(),
                        peer_id: peer,
                        candidate_id: format!("cand-{}-{acc}", hub.session_id),
                    });
                }
            }
            if !entry_reqs.is_empty() {
                let at = EntryAt {
                    session_id: hub.session_id.clone(),
                    generation: hub.generation,
                    realm_id: realm_id_str.clone(),
                    now,
                    expires_at: entry_expires_at(now),
                };
                match self
                    .hub
                    .request_entries(
                        &Actor::Account(context.player.clone()),
                        &at,
                        hub.max_peers.unwrap_or(16),
                        &entry_reqs,
                    )
                    .await?
                {
                    HubEntryBatch::Placed(placed) => {
                        for (acc, row, _) in placed {
                            let ready = row.local_bridge_ready_at.is_some() && row.local_bridge_port.is_some();
                            let cand = json!({
                                "HuntId": hub.hunt_definition_id,
                                "CandidateId": row.candidate_id,
                                "GameSessionId": game_session_id,
                                "Host": if ready { "127.0.0.1" } else { "" },
                                "Port": row.local_bridge_port.unwrap_or(0),
                                "Ready": ready,
                                "ReadyAt": 0,
                                "Source": "REALM_HUB",
                                "SourceSessionId": hub.session_id,
                            });
                            let _ = rt
                                .core
                                .set_candidate(AccountId(acc.clone()), rifts_seam::Candidate(Json(cand.to_string())));
                            let _ = self
                                .hunts
                                .enqueue_control_message(
                                    &Actor::Account(AccountId(acc.clone())),
                                    &NewControlMessage {
                                        account_id: acc,
                                        message_type: "REALM_HUB_JOIN".to_owned(),
                                        session_id: Some(hub.session_id.clone()),
                                        generation: Some(hub.generation),
                                        payload: json!({
                                            "sessionId": hub.session_id,
                                            "generation": hub.generation,
                                            "hubKind": hub_kind_name(kind),
                                            "realmId": realm_id_str,
                                        }),
                                        created_at: now,
                                        expires_at,
                                    },
                                )
                                .await;
                        }
                    }
                    HubEntryBatch::Full { .. } => {
                        let accounts: Vec<AccountId> = members.iter().cloned().map(AccountId).collect();
                        let _ = rt.core.cancel_travel(&context.hunt_id, &accounts, "REALM_HUB_FULL");
                        return Ok(rifts_seam::HuntRouteOutcome::Handled { result: true });
                    }
                }
            }

            // If the host is among the travelling players, set their candidate too.
            if members.iter().any(|m| m == &hub.host_account_id) {
                let port = hub.host_local_bridge_port.unwrap_or(0);
                let ready = hub.host_local_bridge_ready_at.is_some() && port > 0;
                let cand = json!({
                    "HuntId": hub.hunt_definition_id,
                    "CandidateId": format!("host:{}", hub.session_id),
                    "GameSessionId": game_session_id,
                    "Host": if ready { "127.0.0.1" } else { "" },
                    "Port": port,
                    "Ready": ready,
                    "ReadyAt": 0,
                    "Source": "REALM_HUB",
                    "SourceSessionId": hub.session_id,
                });
                let _ = rt.core.set_candidate(
                    AccountId(hub.host_account_id.clone()),
                    rifts_seam::Candidate(Json(cand.to_string())),
                );
            }
            return Ok(rifts_seam::HuntRouteOutcome::Handled { result: true });
        }

        // Non-hub hunt: check if it should be player-hosted (either inside a realm or via P2P policy).
        let stop = self.emergency_stop().await?;
        let should_host = if realm_id.is_some() {
            self.settings.policy.enabled && !stop && !context.is_hub
        } else {
            let input = crate::tiers::RouteInput {
                game_mode: context.game_mode.clone(),
                hunt_id: context.hunt_id.clone(),
                account_ids: members.clone(),
                party_size: i64::try_from(members.len()).unwrap_or(1),
                is_private_party: true,
            };
            let decision = crate::tiers::decide_hunt_route(&input, &self.settings.policy, true, stop);
            decision.route == crate::tiers::HuntRoute::PlayerHosted
        };
        if !should_host {
            return Ok(rifts_seam::HuntRouteOutcome::NotHandled);
        }

        let party_key = context.party.as_ref().map_or_else(
            || format!("solo:{}", context.player.0),
            std::string::ToString::to_string,
        );
        let revision = context.party_revision.and_then(|r| i64::try_from(r).ok()).unwrap_or(1);
        let session = if let Some(existing) = self.hunts.find_active_hunt_for_party(&party_key).await? {
            existing
        } else {
            self.create_routed_hunt(
                &party_key,
                revision,
                &context.hunt_id,
                Some(&context.game_mode),
                &members,
                realm_id,
            )
            .await?
        };
        for p in &session.roster {
            let ready = p.local_bridge_ready_at.is_some() && p.local_bridge_port.is_some();
            let cand = json!({
                "HuntId": session.hunt_definition_id,
                "CandidateId": p.candidate_id,
                "GameSessionId": session.session_id,
                "Host": if ready { "127.0.0.1" } else { "" },
                "Port": p.local_bridge_port.unwrap_or(0),
                "Ready": ready,
                "ReadyAt": 0,
            });
            let _ = rt.core.set_candidate(
                AccountId(p.account_id.clone()),
                rifts_seam::Candidate(Json(cand.to_string())),
            );
        }
        Ok(rifts_seam::HuntRouteOutcome::Handled { result: true })
    }
}

impl AdmissionHunts for Hosting {
    fn gate(&self) -> Result<P2PGate, AdmissionError> {
        let stop = block_on_realms(self.emergency_stop()).map_err(admission_db)?;
        Ok(P2PGate {
            enabled: self.settings.policy.enabled,
            emergency_stop: stop,
        })
    }

    fn find_hunt(&self, session: &SessionId) -> Result<Option<AdmissionHunt>, AdmissionError> {
        let wire = wire_session_id(session);
        let found = block_on_repo(self.hunts.find_hunt(&wire)).map_err(admission_db)?;
        Ok(found.as_ref().and_then(to_admission_hunt))
    }

    fn reserve_participant(
        &self,
        hunt: &AdmissionHunt,
        account: &AccountId,
        now: UnixMillis,
    ) -> Result<Option<AdmissionHunt>, AdmissionError> {
        let wire = hunt.snapshot.wire_session.clone();
        let gen_i64 = i64::try_from(hunt.snapshot.generation).unwrap_or(1);
        let acc = account.0.clone();
        let rejoin_deadline = UnixMillis(now.0 + REJOIN_WINDOW_MS);
        let actor = Actor::Account(account.clone());
        let updated = block_on_repo(self.hunts.reserve_participant_ticket(
            &actor,
            &wire,
            gen_i64,
            &acc,
            now,
            rejoin_deadline,
            MAX_RECONNECTS,
        ))
        .map_err(admission_db)?;
        Ok(updated.as_ref().and_then(to_admission_hunt))
    }

    fn check_hub_entry(
        &self,
        hub: &AdmissionHunt,
        account: &AccountId,
        link: Option<&PeerLinkRecord>,
        now: UnixMillis,
    ) -> Result<PeerId, AdmissionError> {
        let wire = hub.snapshot.wire_session.clone();
        let acc = account.0.clone();
        let hunt = block_on_repo(self.hunts.find_hunt(&wire))
            .map_err(admission_db)?
            .ok_or_else(|| admission_refused("P2P_SESSION_NOT_FOUND", "Hub session not found."))?;
        let Some(realm_id) = hunt.realm_id.clone() else {
            return Err(admission_refused(
                "P2P_SESSION_NOT_READY",
                "The realm's hub is not accepting players.",
            ));
        };
        let realm = block_on_repo(self.realms.find_by_id(&realm_id)).map_err(admission_db)?;
        let is_banned = realm
            .as_ref()
            .is_some_and(|r| r.bans.iter().any(|b| b.account_id == acc));
        let is_member = realm
            .as_ref()
            .is_some_and(|r| r.host_account_id == acc || r.members.iter().any(|m| m.account_id == acc));

        let derived_peer = derive_ice_peer_id(&acc);
        if link.is_none() && hub.snapshot.coop_transport == CoopTransport::Ice {
            self.ensure_verified_link(&acc, &derived_peer, now);
        }
        let effective_peer_str = link.map_or_else(|| derived_peer.clone(), |l| l.peer.0.to_string());
        let link_pair = Some(("VERIFIED", effective_peer_str.as_str()));

        let mut presence = block_on_repo(self.hub.find_presence(&wire, &acc)).map_err(admission_db)?;
        let needs_entry = presence.as_ref().is_none_or(|r| {
            !matches!(r.state, PresenceState::Pending | PresenceState::Ticketed) || r.expires_at <= now
        });
        if needs_entry && is_member && !is_banned && acc != hunt.host_account_id && is_hub_live(hunt.state) {
            let at = EntryAt {
                session_id: wire.clone(),
                generation: hunt.generation,
                realm_id: realm_id.clone(),
                now,
                expires_at: entry_expires_at(now),
            };
            let req = HubEntryRequest {
                account_id: acc.clone(),
                peer_id: effective_peer_str.clone(),
                candidate_id: format!("cand-{wire}-{acc}"),
            };
            let max_peers = hunt.max_peers.unwrap_or(16);
            match block_on_repo(
                self.hub
                    .request_entries(&Actor::Account(account.clone()), &at, max_peers, &[req]),
            )
            .map_err(admission_db)?
            {
                HubEntryBatch::Placed(mut rows) => {
                    presence = rows.pop().map(|(_, row, _)| row);
                }
                HubEntryBatch::Full { .. } => {
                    return Err(admission_refused("REALM_HUB_FULL", "The realm's hub is full."));
                }
            }
        }

        let mut check_hunt = hunt.clone();
        if matches!(
            check_hunt.state,
            HuntState::Starting | HuntState::HostReady | HuntState::Admitting
        ) {
            check_hunt.state = HuntState::Running;
        }
        let presence_tuple = presence.as_ref().map(|r| (r.state, r.expires_at, r.peer_id.as_str()));
        let check = HubTicketCheck {
            hub: &check_hunt,
            generation: Some(i64::try_from(hub.snapshot.generation).unwrap_or(1)),
            account_id: &acc,
            presence: presence_tuple,
            is_member,
            is_banned,
            link: link_pair,
            now,
        };
        if let Some(denial) = hub_ticket_denial(&check) {
            let (code, msg) = hub_ticket_denial_error(denial);
            return Err(admission_refused(code, msg));
        }
        Ok(parse_peer_id(&effective_peer_str, &acc))
    }

    fn mark_hub_ticketed(
        &self,
        hub: &AdmissionHunt,
        account: &AccountId,
        now: UnixMillis,
        ticket_expires_at: UnixMillis,
    ) -> Result<bool, AdmissionError> {
        let wire = hub.snapshot.wire_session.clone();
        let gen_i64 = i64::try_from(hub.snapshot.generation).unwrap_or(1);
        let row = block_on_repo(self.hub.mark_ticketed(
            &Actor::Account(account.clone()),
            &wire,
            gen_i64,
            &account.0,
            now,
            ticket_expires_at,
        ))
        .map_err(admission_db)?;
        Ok(row.is_some())
    }

    fn mark_participant_admitted(
        &self,
        hunt: &AdmissionHunt,
        account: &AccountId,
        admitted_at: UnixMillis,
    ) -> Result<bool, AdmissionError> {
        let wire = hunt.snapshot.wire_session.clone();
        let gen_i64 = i64::try_from(hunt.snapshot.generation).unwrap_or(1);
        let acc = account.0.clone();
        let actor = Actor::Account(account.clone());
        let updated = block_on_repo(self.hunts.transaction(&actor, move |tx| {
            let Some(after_part) = tx.mark_participant_admitted(&wire, gen_i64, &acc, admitted_at)? else {
                return Ok::<_, crate::db::RepoError>(None);
            };
            let all_admitted = !after_part.roster.is_empty()
                && after_part
                    .roster
                    .iter()
                    .all(|p| p.admission_state == AdmissionState::Admitted);
            let target = if all_admitted {
                HuntState::Running
            } else {
                HuntState::Admitting
            };
            if matches!(after_part.state, HuntState::HostReady | HuntState::Admitting) && after_part.state != target {
                let from = after_part.state;
                if let Some(next) = tx.compare_and_set(
                    &wire,
                    gen_i64,
                    &[HuntState::HostReady, HuntState::Admitting],
                    after_part.state_version,
                    move |h| {
                        h.state = target;
                    },
                )? {
                    tx.append_transition(&HuntTransition {
                        transition_id: String::new(),
                        session_id: wire,
                        generation: gen_i64,
                        from_state: from,
                        to_state: target,
                        state_version: next.state_version,
                        reason: if target == HuntState::Running {
                            "ALL_PARTICIPANTS_ADMITTED".to_owned()
                        } else {
                            "FIRST_PARTICIPANT_ADMITTED".to_owned()
                        },
                        actor_type: TransitionActor::Host.as_str().to_owned(),
                        actor_account_id: Some(acc),
                        at: admitted_at,
                    })?;
                    return Ok(Some(next));
                }
            }
            Ok(Some(after_part))
        }))
        .map_err(admission_db)?;
        Ok(updated.is_some())
    }

    fn admit_hub_peer(
        &self,
        hub: &AdmissionHunt,
        account: &AccountId,
        admission_id: &str,
        admitted_at: UnixMillis,
    ) -> Result<(), AdmissionError> {
        let wire = hub.snapshot.wire_session.clone();
        let gen_i64 = i64::try_from(hub.snapshot.generation).unwrap_or(1);
        if account == &hub.snapshot.host_account {
            return Ok(());
        }
        block_on_repo(self.hub.mark_admitted(
            &Actor::Account(account.clone()),
            &wire,
            gen_i64,
            &account.0,
            admission_id,
            admitted_at,
            admitted_at,
            Some("ice"),
        ))
        .map_err(admission_db)?;
        if let Some(rt) = &self.runtime {
            rt.capabilities.hub_presence().invalidate(&wire);
        }
        Ok(())
    }

    fn peer_disconnected(&self, admission: &PeerAdmissionRecord, at: UnixMillis) -> Result<(), AdmissionError> {
        let wire = admission.session.clone();
        let gen_i64 = i64::try_from(admission.generation).unwrap_or(1);
        let acc = admission.account.0.clone();
        let actor = Actor::Account(admission.account.clone());
        let Some(hunt) = block_on_repo(self.hunts.find_hunt(&wire)).map_err(admission_db)? else {
            return Ok(());
        };
        if hunt.is_realm_hub() {
            block_on_repo(self.hub.mark_lost(
                &actor,
                &wire,
                gen_i64,
                &acc,
                Some(&admission.admission_id),
                at,
                presence_purge_at(at),
            ))
            .map_err(admission_db)?;
            if let Some(rt) = &self.runtime {
                rt.capabilities.hub_presence().invalidate(&wire);
            }
        } else {
            let rejoin_deadline = UnixMillis(at.0 + REJOIN_WINDOW_MS);
            block_on_repo(
                self.hunts
                    .mark_participant_disconnected(&actor, &wire, gen_i64, &acc, at, rejoin_deadline),
            )
            .map_err(admission_db)?;
        }
        Ok(())
    }

    fn hub_presence(&self, hub: &AdmissionHunt, limit: usize) -> Result<Vec<PresenceRow>, AdmissionError> {
        let wire = hub.snapshot.wire_session.clone();
        let gen_i64 = i64::try_from(hub.snapshot.generation).unwrap_or(1);
        let rows = block_on_repo(self.hub.list_by_state(
            &wire,
            gen_i64,
            crate::hub::PRESENCE_LIVE_STATES,
            u32::try_from(limit.min(256)).unwrap_or(65),
        ))
        .map_err(admission_db)?;
        Ok(rows
            .into_iter()
            .map(|r| PresenceRow {
                peer: parse_peer_id(&r.peer_id, &r.account_id),
                account: AccountId(r.account_id),
                state: map_relay_presence_state(r.state),
            })
            .collect())
    }

    fn join_bootstrap_common(
        &self,
        hunt: &AdmissionHunt,
        key_id: &str,
        bridge_instance_id: &str,
    ) -> Result<Option<Json>, AdmissionError> {
        let wire = &hunt.snapshot.wire_session;
        let full = block_on_repo(self.hunts.find_hunt(wire)).map_err(admission_db)?;
        let metadata_nonce = full
            .as_ref()
            .and_then(|h| h.metadata_nonce.clone())
            .unwrap_or_else(|| "0123456789abcdef0123456789abcdef".to_owned());
        let guard_completion_mode = full
            .as_ref()
            .and_then(|h| h.guard_completion_mode.clone())
            .unwrap_or_else(|| {
                if hunt.snapshot.is_hub() {
                    "NO_COMPLETION".to_owned()
                } else {
                    "FIRST_DEFEAT".to_owned()
                }
            });
        let host_peer = hunt.snapshot.host_peer.0.to_string();
        let obj = json!({
            "keyId": key_id,
            "sessionId": wire,
            "partyId": hunt.party_id,
            "partyRevision": hunt.party_revision,
            "huntDefinitionId": hunt.hunt_definition_id,
            "generation": hunt.snapshot.generation,
            "build": hunt.build,
            "bridgeProtocol": hunt.bridge_protocol,
            "virtualPort": hunt.virtual_port,
            "hostPeerId": host_peer,
            "hostSteamId": host_peer,
            "bridgeInstanceId": bridge_instance_id,
            "metadataNonce": metadata_nonce,
            "forceRelay": self.settings.policy.force_sdr,
            "guardCompletionMode": guard_completion_mode,
            "sessionRole": if hunt.snapshot.is_hub() { "HUB" } else { "HUNT" },
        });
        Ok(Some(Json(obj.to_string())))
    }
}

impl IceHunts for Hosting {
    fn gate(&self) -> Result<P2PGate, RelayError> {
        let stop = block_on_realms(self.emergency_stop()).map_err(relay_db)?;
        Ok(P2PGate {
            enabled: self.settings.policy.enabled,
            emergency_stop: stop,
        })
    }

    fn find_hunt(&self, session: &SessionId) -> Result<Option<HuntSnapshot>, RelayError> {
        let wire = wire_session_id(session);
        let found = block_on_repo(self.hunts.find_hunt(&wire)).map_err(relay_db)?;
        Ok(found.as_ref().map(to_hunt_snapshot))
    }

    fn hub_presence(&self, hunt: &HuntSnapshot, limit: usize) -> Result<Vec<PresenceRow>, RelayError> {
        let gen_i64 = i64::try_from(hunt.generation).unwrap_or(1);
        let rows = block_on_repo(self.hub.list_by_state(
            &hunt.wire_session,
            gen_i64,
            crate::hub::PRESENCE_LIVE_STATES,
            u32::try_from(limit.min(256)).unwrap_or(65),
        ))
        .map_err(relay_db)?;
        Ok(rows
            .into_iter()
            .map(|r| PresenceRow {
                peer: parse_peer_id(&r.peer_id, &r.account_id),
                account: AccountId(r.account_id),
                state: map_relay_presence_state(r.state),
            })
            .collect())
    }

    fn enqueue(&self, message: ControlMessage) -> Result<(), RelayError> {
        let payload: Value = serde_json::from_str(&message.payload.0).map_err(|e| {
            RelayError::Refused(Refusal {
                code: "P2P_SIGNAL_INVALID".to_owned(),
                message: e.to_string(),
            })
        })?;
        let actor = Actor::Account(message.account.clone());
        let record = NewControlMessage {
            account_id: message.account.0,
            message_type: message.message_type.to_owned(),
            session_id: Some(message.wire_session),
            generation: i64::try_from(message.generation).ok(),
            payload,
            created_at: message.created_at,
            expires_at: message.expires_at,
        };
        block_on_repo(self.hunts.enqueue_control_message(&actor, &record)).map_err(relay_db)?;
        Ok(())
    }
}

impl OperationsPolicy for Hosting {
    fn set_coop_transport(
        &self,
        admin_key_id: &str,
        transport: CoopTransport,
        expected_version: u64,
        reason: &str,
        request_id: &str,
    ) -> Result<CoopTransportOutcome, RelayError> {
        let stop = block_on_realms(self.emergency_stop()).unwrap_or(false);
        let now_iso = iso(self.now());
        if expected_version > 1 {
            let current = json!({
                "version": 1,
                "p2pEmergencyStop": stop,
                "coopTransport": if self.operations.ice_transport() { "ICE" } else { "STEAM_EXPERIMENTAL" },
                "updatedAt": now_iso,
            });
            return Ok(CoopTransportOutcome::Conflict(Json(current.to_string())));
        }
        let updated = json!({
            "version": expected_version + 1,
            "p2pEmergencyStop": stop,
            "coopTransport": transport.as_str(),
            "updatedAt": now_iso,
            "updatedBy": admin_key_id,
            "reason": reason,
            "requestId": request_id,
        });
        Ok(CoopTransportOutcome::Updated(Json(updated.to_string())))
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
                    let actor = Actor::System("realms-directory");
                    let _ = self
                        .hub
                        .remove_account_from_realm(&actor, &realm_id, &account_id, now, purge)
                        .await
                        .map_err(|e| e.to_string())?;
                    self.log.info(&format!(
                        "[Realms] removed account {account_id} from realm {realm_id} hub presence ({reason:?})"
                    ));
                }
                RealmEvent::RealmReleased {
                    realm_id,
                    host_account_id,
                } => {
                    let actor = Actor::Account(AccountId(host_account_id.clone()));
                    let _ = self
                        .hub
                        .remove_realm(&actor, &realm_id, now, purge)
                        .await
                        .map_err(|e| e.to_string())?;
                    for kind in [HubKind::Ramsgate, HubKind::Training] {
                        let key = hub_party_key(&realm_id, kind);
                        if let Ok(Some(hub)) = self.hunts.find_active_hunt_for_party(&key).await {
                            let _ = self
                                .end_hub(
                                    &hub.session_id,
                                    hub.generation,
                                    "REALM_RELEASED",
                                    HuntState::Completed,
                                    TransitionActor::Host,
                                    Some(&host_account_id),
                                )
                                .await;
                        }
                    }
                }
                RealmEvent::HostOffline {
                    realm_id,
                    host_account_id,
                } => {
                    for kind in [HubKind::Ramsgate, HubKind::Training] {
                        let key = hub_party_key(&realm_id, kind);
                        if let Ok(Some(hub)) = self.hunts.find_active_hunt_for_party(&key).await {
                            let _ = self
                                .end_hub(
                                    &hub.session_id,
                                    hub.generation,
                                    "REALM_HOST_OFFLINE",
                                    HuntState::Aborted,
                                    TransitionActor::System,
                                    Some(&host_account_id),
                                )
                                .await;
                        }
                    }
                }
            }
            Ok(())
        })
    }
}

impl HuntLifecycle for Hosting {
    fn host_bootstrap(
        &self,
        account: AccountId,
        session: SessionId,
    ) -> BoxFuture<'_, Result<HostBootstrap, RealmsError>> {
        Box::pin(async move {
            let wire = wire_session_id(&session);
            let value = self.create_host_bootstrap(&account.0, &wire).await?;
            let generation = value["generation"].as_u64().unwrap_or(1);
            let capability = Secret::new(value["capability"].as_str().unwrap_or_default().to_owned());
            let host_ticket = value["hostTicket"].as_str().map(|s| Secret::new(s.to_owned()));
            let backend_url = value["backendUrl"].as_str().unwrap_or_default().to_owned();
            let server_udp_port = value["serverUdpPort"]
                .as_u64()
                .and_then(|p| u16::try_from(p).ok())
                .unwrap_or(8791);
            let maximum_peers = value["maximumPeers"]
                .as_u64()
                .and_then(|p| u8::try_from(p).ok())
                .unwrap_or(4);
            Ok(HostBootstrap {
                session,
                generation,
                capability,
                host_ticket,
                backend_url,
                server_udp_port,
                maximum_peers,
            })
        })
    }

    fn route_ready(
        &self,
        account: AccountId,
        session: SessionId,
        local_port: u16,
    ) -> BoxFuture<'_, Result<(), RealmsError>> {
        Box::pin(async move {
            let wire = wire_session_id(&session);
            let _ = self
                .mark_local_route_ready(&account.0, &wire, &json!({ "localBridgePort": local_port }))
                .await?;
            Ok(())
        })
    }

    fn host_ready(&self, runtime: RuntimeAuth) -> BoxFuture<'_, Result<(), RealmsError>> {
        Box::pin(async move {
            let wire = runtime
                .wire_session
                .clone()
                .unwrap_or_else(|| wire_session_id(&runtime.session));
            let cap = HostCapabilityRecord {
                capability_id: runtime.capability_id,
                secret_digest: String::new(),
                session_id: wire,
                generation: i64::try_from(runtime.generation).unwrap_or(1),
                host_account_id: runtime.host_account.0,
                host_steam_id: runtime.host_peer.map_or_else(String::new, |p| p.0.to_string()),
                bridge_instance_id: runtime.bridge_instance.unwrap_or_default(),
                scopes: runtime.scopes,
                issued_at: self.now(),
                expires_at: UnixMillis(self.now().0 + 3_600_000),
                revoked_at: None,
                last_sequence: 0,
            };
            let _ = self.mark_host_ready(&cap, &json!({})).await?;
            Ok(())
        })
    }

    fn abort(&self, runtime: RuntimeAuth, reason: String) -> BoxFuture<'_, Result<(), RealmsError>> {
        Box::pin(async move {
            let wire = runtime
                .wire_session
                .clone()
                .unwrap_or_else(|| wire_session_id(&runtime.session));
            let cap = HostCapabilityRecord {
                capability_id: runtime.capability_id,
                secret_digest: String::new(),
                session_id: wire,
                generation: i64::try_from(runtime.generation).unwrap_or(1),
                host_account_id: runtime.host_account.0,
                host_steam_id: runtime.host_peer.map_or_else(String::new, |p| p.0.to_string()),
                bridge_instance_id: runtime.bridge_instance.unwrap_or_default(),
                scopes: runtime.scopes,
                issued_at: self.now(),
                expires_at: UnixMillis(self.now().0 + 3_600_000),
                revoked_at: None,
                last_sequence: 0,
            };
            let _ = self.abort_hunt(&cap, &json!({ "reason": reason })).await?;
            Ok(())
        })
    }

    fn end(&self, runtime: RuntimeAuth, reason: String) -> BoxFuture<'_, Result<(), RealmsError>> {
        Box::pin(async move {
            let wire = runtime
                .wire_session
                .clone()
                .unwrap_or_else(|| wire_session_id(&runtime.session));
            let cap = HostCapabilityRecord {
                capability_id: runtime.capability_id,
                secret_digest: String::new(),
                session_id: wire,
                generation: i64::try_from(runtime.generation).unwrap_or(1),
                host_account_id: runtime.host_account.0,
                host_steam_id: runtime.host_peer.map_or_else(String::new, |p| p.0.to_string()),
                bridge_instance_id: runtime.bridge_instance.unwrap_or_default(),
                scopes: runtime.scopes,
                issued_at: self.now(),
                expires_at: UnixMillis(self.now().0 + 3_600_000),
                revoked_at: None,
                last_sequence: 0,
            };
            let _ = self.finish_hunt(&cap, &json!({ "reason": reason })).await?;
            Ok(())
        })
    }

    fn status(&self, account: AccountId, session: SessionId) -> BoxFuture<'_, Result<HuntStatus, RealmsError>> {
        Box::pin(async move {
            let wire = wire_session_id(&session);
            let value = self.get_hunt_status(Some(&account.0), &wire).await?;
            let state = value["state"]
                .as_str()
                .and_then(HuntState::parse)
                .unwrap_or(HuntState::Running);
            let generation = value["generation"].as_u64().unwrap_or(1);
            Ok(HuntStatus {
                state,
                generation,
                verdict: None,
            })
        })
    }

    fn observe(
        &self,
        account: AccountId,
        session: SessionId,
        observation: Observation,
    ) -> BoxFuture<'_, Result<(), RealmsError>> {
        Box::pin(async move {
            let wire = wire_session_id(&session);
            let metrics = observation.route_kind.is_some();
            let body = json!({
                "generation": 1,
                "observedAt": iso(self.now()),
                "hostReachable": observation.host_reachable,
                "localBridgeAlive": observation.local_bridge_alive,
                "metricsAvailable": metrics,
                "routeKind": observation.route_kind.unwrap_or_else(|| "UNKNOWN".to_owned()),
                "pingMs": observation.ping_ms,
                "jitterMs": observation.jitter_ms,
                "packetLossPercent": observation.packet_loss_percent,
                "lastPacketAgeMs": observation.last_packet_age_ms,
            });
            let _ = Hosting::observe(self, &account.0, &wire, &body).await?;
            Ok(())
        })
    }

    fn start_hub(&self, account: AccountId, hub: StartHub) -> BoxFuture<'_, Result<HostBootstrap, RealmsError>> {
        Box::pin(async move {
            let body = json!({
                "hubKind": hub_kind_name(hub.kind),
                "maxPlayers": hub.max_players,
                "afkSeconds": hub.afk_seconds,
            });
            let value = self.start_realm_hub(&account.0, &body).await?;
            let wire = value["sessionId"].as_str().unwrap_or_default();
            let node = match &self.runtime {
                Some(rt) => rt.node.clone(),
                None => module_api::NodeId::new("local").map_err(|_| refused("INTERNAL", "Invalid node id."))?,
            };
            let session = seam_session_id(wire, &node).ok_or_else(|| refused("INTERNAL", "Invalid hub session id."))?;
            let generation = value["generation"].as_u64().unwrap_or(1);
            let capability = Secret::new(value["capability"].as_str().unwrap_or_default().to_owned());
            let host_ticket = value["hostTicket"].as_str().map(|s| Secret::new(s.to_owned()));
            let backend_url = value["backendUrl"].as_str().unwrap_or_default().to_owned();
            let server_udp_port = value["serverUdpPort"]
                .as_u64()
                .and_then(|p| u16::try_from(p).ok())
                .unwrap_or(8791);
            let maximum_peers = value["maximumPeers"]
                .as_u64()
                .and_then(|p| u8::try_from(p).ok())
                .unwrap_or(hub.max_players);
            Ok(HostBootstrap {
                session,
                generation,
                capability,
                host_ticket,
                backend_url,
                server_udp_port,
                maximum_peers,
            })
        })
    }

    fn stop_hub(&self, account: AccountId, kind: HubKind) -> BoxFuture<'_, Result<(), RealmsError>> {
        Box::pin(async move {
            let _ = self
                .stop_realm_hub(&account.0, &json!({ "hubKind": hub_kind_name(kind) }))
                .await?;
            Ok(())
        })
    }

    fn hosting_status(&self, account: AccountId) -> BoxFuture<'_, Result<Json, RealmsError>> {
        Box::pin(async move {
            let value = self.get_realm_hosting_status(&account.0).await?;
            Ok(Json(value.to_string()))
        })
    }

    fn set_emergency_stop(&self, admin_key: String, policy: Json) -> BoxFuture<'_, Result<Json, RealmsError>> {
        Box::pin(async move {
            let parsed: Value = serde_json::from_str(&policy.0)
                .map_err(|_| refused("ADMIN_VALIDATION", "Invalid emergency stop payload."))?;
            let stop = parsed
                .get("p2pEmergencyStop")
                .and_then(Value::as_bool)
                .ok_or_else(|| refused("ADMIN_VALIDATION", "p2pEmergencyStop must be a boolean."))?;
            let expected = parsed.get("expectedVersion").and_then(Value::as_u64).unwrap_or(1);
            let reason = parsed
                .get("reason")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|r| r.len() >= 8)
                .ok_or_else(|| {
                    refused(
                        "ADMIN_VALIDATION",
                        "An audit reason of at least 8 characters is required.",
                    )
                })?;
            let reply = json!({
                "policy": {
                    "version": expected + 1,
                    "p2pEmergencyStop": stop,
                    "coopTransport": if self.operations.ice_transport() { "ICE" } else { "STEAM_EXPERIMENTAL" },
                    "updatedAt": iso(self.now()),
                    "updatedBy": admin_key,
                    "reason": reason,
                },
                "requestId": random_uuid(),
            });
            Ok(Json(reply.to_string()))
        })
    }

    fn resolve_launch_manifest(&self, request: Json) -> BoxFuture<'_, Result<Json, RealmsError>> {
        Box::pin(async move {
            let parsed: Value = serde_json::from_str(&request.0)
                .map_err(|_| refused("P2P_TICKET_INVALID", "Invalid launch manifest request."))?;
            let hunt_def = parsed
                .get("huntDefinitionId")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| refused("P2P_TICKET_INVALID", "huntDefinitionId is required."))?;
            let players: Vec<String> = parsed
                .get("expectedPlayers")
                .and_then(Value::as_array)
                .map(|arr| arr.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                .unwrap_or_default();
            Ok(Json(resolve_manifest_for_hunt(hunt_def, &players).to_string()))
        })
    }
}
