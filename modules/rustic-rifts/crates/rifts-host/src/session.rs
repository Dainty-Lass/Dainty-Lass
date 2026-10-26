//! The launcher's host and join surface (`ParadoxLauncher/src-tauri/src/p2p/`, RF1 section 3), behind DaintyLass's
//! realms feature (DD12). The launcher calls it from its bridge commands (DF8); the account key never reaches it.

use rifts_seam::{AccountId, BoxFuture, HubKind, SessionId};

use crate::HostError;
use crate::transport::SessionResult;

/// What the launcher gives the hosted game server, checked by the runtime's `ReadLaunch`. A change here is a change
/// to rustic-isles' runtime too (RF1, decision 5).
pub const LAUNCH_VARIABLES: [&str; 17] = [
    "MYSTICPARADOX_AUTH_MODE",
    "MYSTICPARADOX_BIND_ADDRESS",
    "MYSTICPARADOX_HOST_RUNTIME_CAPABILITY",
    "MYSTICPARADOX_HOSTED_ROLE",
    "MYSTICPARADOX_SESSION_ID",
    "MYSTICPARADOX_SESSION_GENERATION",
    "MYSTICPARADOX_GAMESERVER_LAUNCH_ID",
    "MYSTICPARADOX_READY_PIPE",
    "MYSTICPARADOX_CONTROL_PIPE",
    "MYSTICPARADOX_GUARD_PIPE",
    "MYSTICPARADOX_HUNT_GUARD_EVENTS",
    "MYSTICPARADOX_HUNT_GUARD_COMPLETION_MODE",
    "MYSTICPARADOX_RUNTIME_PROFILE",
    "MYSTICPARADOX_LAUNCHER_PID",
    "MYSTICPARADOX_MAX_PLAYERS",
    "MYSTICPARADOX_HUB_AFK_SECONDS",
    "MYSTICPARADOX_TRIALS_WEEK",
];

/// Removed from the environment the hosted server inherits.
pub const CLEARED_VARIABLES: [&str; 2] = [
    "MYSTICPARADOX_ALLOW_DAMAGE_DEBUG",
    "MYSTICPARADOX_DEBUG_DAMAGE_MULTIPLIER",
];

/// `MYSTICPARADOX_HOSTED_ROLE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostedRole {
    Hunt,
    Hub(HubKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalState {
    Starting,
    Ready,
    Running,
    Ended(SessionResult),
}

/// One session this PC hosts or joined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalSession {
    pub session: SessionId,
    pub generation: u64,
    pub role: HostedRole,
    pub hosting: bool,
    pub state: LocalState,
}

/// `realm-host-limits.json`, per hub server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HubLimits {
    /// 1 to 64.
    pub max_players: u8,
    pub max_servers: u8,
    /// 0 to 7200.
    pub afk_seconds: u32,
}

pub trait RealmHosting: Send + Sync {
    /// The co-op switch (`p2p/mod.rs`): off with `DISABLE_COOP_TRANSPORT.flag` or an unusable transport.
    fn coop_available(&self) -> BoxFuture<'_, Result<bool, HostError>>;
    /// Host bootstrap, session folder, transport `HostOpen`, then the game server with [`LAUNCH_VARIABLES`].
    fn host_hunt(&self, account: AccountId, session: SessionId) -> BoxFuture<'_, Result<LocalSession, HostError>>;
    /// Join bootstrap, transport config, `JoinOpen`; the game connects to the local port.
    fn join_hunt(&self, account: AccountId, session: SessionId) -> BoxFuture<'_, Result<LocalSession, HostError>>;
    fn start_hub(
        &self,
        account: AccountId,
        kind: HubKind,
        limits: HubLimits,
    ) -> BoxFuture<'_, Result<LocalSession, HostError>>;
    fn stop_hub(&self, account: AccountId, kind: HubKind) -> BoxFuture<'_, Result<(), HostError>>;
    /// Ends one session and removes its folder (`session.rs:cleanup_directory`).
    fn stop(&self, session: SessionId) -> BoxFuture<'_, Result<(), HostError>>;
    /// The hosting panel's view.
    fn sessions(&self) -> BoxFuture<'_, Result<Vec<LocalSession>, HostError>>;
}
