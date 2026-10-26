//! Player-hosted hunts and realm hubs: their lifecycle, host capabilities, leases, host selection and the launcher's
//! control channel (`p2p/controllers/*`, `p2p/security/hostCapabilities.ts`, `p2p/realm/*`).

use rifts_seam::{AccountId, BoxFuture, CapabilityScope, HubKind, Json, RuntimeAuth, Secret, SessionId, UnixMillis};

use crate::RealmsError;
use crate::store::{ControlMessageRecord, HostCapabilityRecord, HostOfferRecord, HuntState};

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

/// `RequireHostCapability` and its minting.
pub trait HostCapabilities: Send + Sync {
    fn mint(
        &self,
        session: SessionId,
        generation: u64,
        host: AccountId,
        scopes: Vec<CapabilityScope>,
    ) -> BoxFuture<'_, Result<(Secret, HostCapabilityRecord), RealmsError>>;
    /// By SHA-256 digest, session, expiry and scope.
    fn check(
        &self,
        secret: Secret,
        session: SessionId,
        scope: CapabilityScope,
    ) -> BoxFuture<'_, Result<RuntimeAuth, RealmsError>>;
    fn revoke(&self, session: SessionId, generation: u64) -> BoxFuture<'_, Result<(), RealmsError>>;
}

pub trait Leases: Send + Sync {
    /// `RenewHostLease`, every ~2 s per host session; the new expiry (`P2P_HOST_LEASE_LIFETIME_MS`).
    fn renew(&self, runtime: RuntimeAuth, sequence: u64) -> BoxFuture<'_, Result<UnixMillis, RealmsError>>;
    /// The lifecycle monitor's pass: sessions whose lease or deadline ran out, moved on.
    fn sweep(&self, now: UnixMillis) -> BoxFuture<'_, Result<Vec<(SessionId, u64)>, RealmsError>>;
}

/// `SubmitHostPresence`: a PC offering to host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPresence {
    pub consent_to_host: bool,
    pub launcher_version: String,
    /// Memory, CPU and network metrics, as the launcher sends them.
    pub metrics: Json,
    /// `gameExe`, `internalServer`, `winmmProxy`, the transport and `admissionPublicKey` hashes.
    pub artifacts: Json,
    pub guard_session_id: Option<String>,
}

pub trait HostSelection: Send + Sync {
    fn submit_presence(&self, account: AccountId, presence: HostPresence) -> BoxFuture<'_, Result<(), RealmsError>>;
    /// `RespondToHostOffer`.
    fn respond_to_offer(
        &self,
        account: AccountId,
        offer_id: String,
        accept: bool,
        reason: Option<String>,
    ) -> BoxFuture<'_, Result<(), RealmsError>>;
    /// Ranks the fresh presences and offers the session to the best (`P2P_HOST_SELECTION_ENABLED`).
    fn select_host(
        &self,
        session: SessionId,
        generation: u64,
    ) -> BoxFuture<'_, Result<Option<HostOfferRecord>, RealmsError>>;
}

/// `p2pControl.ts`: the launcher's long poll.
pub trait ControlChannel: Send + Sync {
    /// `PollP2PControl`.
    fn poll(
        &self,
        account: AccountId,
        after_sequence: u64,
        wait_ms: u32,
    ) -> BoxFuture<'_, Result<Vec<ControlMessageRecord>, RealmsError>>;
    /// `AcknowledgeP2PControl`.
    fn ack(&self, account: AccountId, message_id: String) -> BoxFuture<'_, Result<(), RealmsError>>;
    /// Queues one message; its sequence.
    fn send(
        &self,
        account: AccountId,
        kind: String,
        session: Option<SessionId>,
        payload: Json,
    ) -> BoxFuture<'_, Result<u64, RealmsError>>;
}
