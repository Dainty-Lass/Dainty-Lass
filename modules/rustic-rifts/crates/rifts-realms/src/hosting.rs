//! The session routes not ported yet (RF2's surface, still answering through [`crate::Stub`]): host bootstrap, the
//! host's ready, abort and end, status, hub start and stop, the hosting status, the emergency stop and the Director's
//! manifest. They need rifts-admission's tickets and isles' hunt data, operations policy and party (RU4).

use rifts_seam::{AccountId, BoxFuture, HubKind, Json, RuntimeAuth, Secret, SessionId};

use crate::RealmsError;
use crate::hunt::HuntState;

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
