// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Rustic Rifts on the launcher: the co-op transport behind [`TransportAbi`], the two JSON files the host PC keeps,
//! and the realm sessions it hosts or joins. It opens no listener and makes no HTTP call; the launcher's
//! [`HostBackend`] does, and holds the account key.

pub mod files;
mod loader;
pub mod session;
pub mod transport;

use std::path::Path;

use rifts_seam::{AccountId, BoxFuture, Component, HubKind, PeerId, Secret, SessionId, Unimplemented};

pub use crate::files::{
    ADMISSION_PUBLIC_KEY_FILE, DISABLE_COOP_FLAG_FILE, FileHostState, HOST_SETTINGS_FILE, HUB_LIMITS_FILE, HostState,
    SESSION_FILES, TRANSPORT_DLL, validate_hub_limits,
};
#[cfg(feature = "in-process")]
pub use crate::loader::in_process::InProcessTransport;
pub use crate::loader::{NativeTransportLoader, sha256_file};
pub use crate::session::{
    CLEARED_VARIABLES, CapacityDecision, CommonBootstrapConfig, DIRECTOR_SERVER_SWITCHES, DefaultHostBackend, FilesFor,
    GAME_LEFT_IDLE, GameTrafficWatch, HOST_COMMIT_FLOOR_MIB, HOST_MEMORY_FLOOR_MIB, HostBackend, HostBootstrapSpec,
    HostedRole, HubLimits, IceFiles, IcePeerEntry, IceServerEntry, JoinBootstrapSpec, LAUNCH_VARIABLES,
    LaunchEnvironmentSpec, LocalRealmHosting, LocalSession, LocalState, MAX_HUB_PEERS, MAX_HUNT_PEERS,
    NoopServerSpawner, PeerDiff, PeerEvent, PeerListEntry, PeerLive, RealmHosting, RelayGrant, ServerSpawner,
    SessionLive, TransportConfig, WantedPeer, apply_peer_event, apply_peer_line, attempts_in_window,
    build_launch_environment, capacity_decision, cleanup_directory, create_session_directory, default_max_servers,
    host_server_arguments, hunt_join_leave_outcome, ice_files, is_local_admission, next_idle_since, parse_ice_ready,
    parse_peer_event, peer_diff, relay_refresh_due, relay_restart_due, restart_allowed, restart_backoff, safe_account,
    safe_config_value, safe_credential, safe_key, safe_peer_id, safe_url, sanitize_central_log_line,
    scheduled_restart_due, training_should_stop, trials_week, udp_stats_local_rx, wanted_peers, windows_command_line,
    wire_session_id, write_session_file_0600,
};
pub use crate::transport::{
    ABI_VERSION, Handle, HostOpen, JoinOpen, ReadyState, SessionResult, Signal, TransportAbi, TransportLoader,
};

/// The only error type of the crate. The launcher matches the variants, so names and payloads are API.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum HostError {
    #[error(transparent)]
    Unimplemented(#[from] Unimplemented),
    #[error("{0}")]
    Io(String),
    /// Kept for the launcher's error mapping; the in-process transport has no file to check (D1).
    #[error("transport SHA-256 mismatch")]
    HashMismatch { expected: [u8; 32], actual: [u8; 32] },
    #[error("transport is missing export {0}")]
    MissingExport(&'static str),
    #[error("transport speaks ABI version {0}; expected 3")]
    UnsupportedAbi(u32),
    #[error("{0}")]
    InvalidInput(String),
    #[error("{0}")]
    Transport(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Busy(String),
}

/// Refuses everything, as the seam's other stubs do.
#[derive(Debug, Clone, Copy, Default)]
pub struct Stub;

fn refuse<T>(operation: &'static str) -> Result<T, HostError> {
    Err(Unimplemented::new(Component::Host, operation).into())
}

impl TransportLoader for Stub {
    fn load(&self, _dll: &Path, _expected_sha256: [u8; 32]) -> Result<Box<dyn TransportAbi>, HostError> {
        refuse("transport.load")
    }
}

impl TransportAbi for Stub {
    fn abi_version(&self) -> Result<u32, HostError> {
        refuse("transport.abi_version")
    }
    fn initialize(&self) -> Result<(), HostError> {
        refuse("transport.initialize")
    }
    fn shutdown(&self) -> Result<(), HostError> {
        refuse("transport.shutdown")
    }
    fn peer_id(&self) -> Result<PeerId, HostError> {
        refuse("transport.peer_id")
    }
    fn networking_ready(&self) -> Result<bool, HostError> {
        refuse("transport.networking_ready")
    }
    fn host_open(&self, _open: &HostOpen) -> Result<Handle, HostError> {
        refuse("transport.host_open")
    }
    fn join_open(&self, _open: &JoinOpen) -> Result<Handle, HostError> {
        refuse("transport.join_open")
    }
    fn wait_ready(&self, _handle: Handle, _timeout_ms: u32) -> Result<ReadyState, HostError> {
        refuse("transport.wait_ready")
    }
    fn session_running(&self, _handle: Handle) -> Result<bool, HostError> {
        refuse("transport.session_running")
    }
    fn stop_session(&self, _handle: Handle) -> Result<(), HostError> {
        refuse("transport.stop_session")
    }
    fn last_session_result(&self, _handle: Handle) -> Result<SessionResult, HostError> {
        refuse("transport.last_session_result")
    }
    fn read_signal(&self, _handle: Handle) -> Result<Option<Signal>, HostError> {
        refuse("transport.read_signal")
    }
    fn submit_signal(&self, _handle: Handle, _from: &AccountId, _kind: &str, _data: &str) -> Result<(), HostError> {
        refuse("transport.submit_signal")
    }
    fn read_peer_event(&self, _handle: Handle) -> Result<Option<String>, HostError> {
        refuse("transport.read_peer_event")
    }
    fn read_diagnostic(&self, _handle: Handle) -> Result<Option<String>, HostError> {
        refuse("transport.read_diagnostic")
    }
    fn add_peer(&self, _handle: Handle, _account: &AccountId, _peer: PeerId, _key: &Secret) -> Result<(), HostError> {
        refuse("transport.add_peer")
    }
    fn remove_peer(&self, _handle: Handle, _account: &AccountId, _reason: &str) -> Result<(), HostError> {
        refuse("transport.remove_peer")
    }
    fn update_relay_credential(&self, _handle: Handle, _user: &str, _credential: &Secret) -> Result<(), HostError> {
        refuse("transport.update_relay_credential")
    }
}

fn refuse_later<T: Send + 'static>(operation: &'static str) -> BoxFuture<'static, Result<T, HostError>> {
    rifts_seam::stub(Component::Host, operation)
}

impl RealmHosting for Stub {
    fn coop_available(&self) -> BoxFuture<'_, Result<bool, HostError>> {
        refuse_later("coop_available")
    }
    fn host_hunt(&self, _account: AccountId, _session: SessionId) -> BoxFuture<'_, Result<LocalSession, HostError>> {
        refuse_later("host_hunt")
    }
    fn join_hunt(&self, _account: AccountId, _session: SessionId) -> BoxFuture<'_, Result<LocalSession, HostError>> {
        refuse_later("join_hunt")
    }
    fn start_hub(
        &self,
        _account: AccountId,
        _kind: HubKind,
        _limits: HubLimits,
    ) -> BoxFuture<'_, Result<LocalSession, HostError>> {
        refuse_later("start_hub")
    }
    fn stop_hub(&self, _account: AccountId, _kind: HubKind) -> BoxFuture<'_, Result<(), HostError>> {
        refuse_later("stop_hub")
    }
    fn stop(&self, _session: SessionId) -> BoxFuture<'_, Result<(), HostError>> {
        refuse_later("stop")
    }
    fn sessions(&self) -> BoxFuture<'_, Result<Vec<LocalSession>, HostError>> {
        refuse_later("sessions")
    }
}

impl HostState for Stub {
    fn hub_limits(&self) -> Result<Option<HubLimits>, HostError> {
        refuse("state.hub_limits")
    }
    fn save_hub_limits(&self, _limits: &HubLimits) -> Result<(), HostError> {
        refuse("state.save_hub_limits")
    }
    fn control_cursor(&self, _account: &AccountId) -> Result<Option<u64>, HostError> {
        refuse("state.control_cursor")
    }
    fn set_control_cursor(&self, _account: &AccountId, _sequence: u64) -> Result<(), HostError> {
        refuse("state.set_control_cursor")
    }
}
