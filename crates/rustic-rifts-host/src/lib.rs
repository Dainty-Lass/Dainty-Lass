// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Rustic Rifts on the launcher: the co-op transport behind [`TransportAbi`], the two JSON files the host PC keeps,
//! and the realm sessions it hosts or joins. It opens no listener and makes no HTTP call; the launcher's
//! [`HostBackend`] does, and holds the account key.

pub mod files;
mod loader;
pub mod session;
pub mod transport;

use std::fmt;

use rustic_rifts_seam::{BoxFuture, Component, Unimplemented};

// The host PC's files.
pub use crate::files::{
    ADMISSION_PUBLIC_KEY_FILE, DISABLE_COOP_FLAG_FILE, FileHostState, HOST_SETTINGS_FILE, HUB_LIMITS_FILE, HostState,
    SESSION_FILES, validate_hub_limits,
};
// Loading the transport.
#[cfg(feature = "in-process")]
pub use crate::loader::in_process::InProcessTransport;
pub use crate::loader::{NativeTransportLoader, sha256_file};
// Ports, flows and the records they pass.
pub use crate::session::{
    CapacityDecision, CommonBootstrapConfig, DefaultHostBackend, FilesFor, GameTrafficWatch, HostBackend,
    HostBootstrapSpec, HostedRole, HubLimits, IceFiles, IcePeerEntry, IceServerEntry, JoinBootstrapSpec,
    LaunchEnvironmentSpec, LocalRealmHosting, LocalSession, LocalState, NoopServerSpawner, PeerDiff, PeerEvent,
    PeerListEntry, PeerLive, RealmHosting, RelayGrant, ServerSpawner, SessionLive, TransportConfig, WantedPeer,
};
// The hosted server's player host: the real spawner and the launcher's pipe ends (spec-runtime-r10 12.2).
pub use crate::session::{
    EXIT_AFTER_FENCE, FENCE_ACK_TIMEOUT, FENCE_EXIT_CODE, FENCE_TIMEOUT_EXIT_CODE, HostedPipes, HostedProcess,
    INVALID_EXIT_CODE, PipeFailure, PlayerHostSpawner, ProcessStarter, READY_TIMEOUT, ReadyRefusal, THREAD_EXIT_CODE,
};
// Limits and names the launcher shows or passes on.
pub use crate::session::{
    CLEARED_VARIABLES, DIRECTOR_SERVER_SWITCHES, GAME_LEFT_IDLE, HOST_COMMIT_FLOOR_MIB, HOST_MEMORY_FLOOR_MIB,
    LAUNCH_VARIABLES, MAX_HUB_PEERS, MAX_HUNT_PEERS,
};
// The pure rules and the session folder.
pub use crate::session::{
    apply_peer_event, apply_peer_line, attempts_in_window, build_launch_environment, capacity_decision,
    cleanup_directory, create_session_directory, default_max_servers, host_server_arguments, hunt_join_leave_outcome,
    ice_files, is_local_admission, next_idle_since, parse_ice_ready, parse_peer_event, peer_diff, relay_refresh_due,
    relay_restart_due, restart_allowed, restart_backoff, safe_account, safe_config_value, safe_credential, safe_key,
    safe_peer_id, safe_url, sanitize_central_log_line, scheduled_restart_due, training_should_stop, trials_week,
    udp_stats_local_rx, wanted_peers, windows_command_line, wire_session_id, write_session_file_0600,
};
pub use crate::transport::{
    ABI_VERSION, Handle, HostOpen, JoinOpen, ReadyState, SessionResult, Signal, TransportAbi, TransportLoader,
};

/// The only error type of the crate. The launcher matches the variants, so names and payloads are API.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum HostError {
    /// A route or call that is not implemented yet.
    Unimplemented(Unimplemented),
    /// A file or folder could not be read or written.
    Io(String),
    /// The transport lacks this call.
    MissingExport(&'static str),
    /// The transport speaks another ABI version.
    UnsupportedAbi(u32),
    /// A value was refused before anything was done; the text says which.
    InvalidInput(String),
    /// The transport or a backend call failed.
    Transport(String),
    /// No such session, hub or handle.
    NotFound(String),
    /// The session is already running; the text names it.
    Busy(String),
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unimplemented(inner) => fmt::Display::fmt(inner, f),
            Self::MissingExport(name) => write!(f, "transport is missing export {name}"),
            Self::UnsupportedAbi(found) => write!(f, "transport speaks ABI version {found}; expected 3"),
            Self::Io(text)
            | Self::InvalidInput(text)
            | Self::Transport(text)
            | Self::NotFound(text)
            | Self::Busy(text) => f.write_str(text),
        }
    }
}

impl std::error::Error for HostError {
    // Transparent for the stub refusal, as before: its own source, if any.
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Unimplemented(inner) => std::error::Error::source(inner),
            _ => None,
        }
    }
}

impl From<Unimplemented> for HostError {
    fn from(inner: Unimplemented) -> Self {
        Self::Unimplemented(inner)
    }
}

/// Refuses everything, as the seam's other stubs do.
#[derive(Clone, Copy, Debug, Default)]
pub struct Stub;

fn refusal(operation: &'static str) -> HostError {
    Unimplemented::new(Component::Host, operation).into()
}

// Each stub method refuses with `<prefix><method name>`, at once or as a ready future.
macro_rules! refusing {
    (now $prefix:literal: $($name:ident($($arg:ty),*) -> $out:ty;)*) => {$(
        fn $name(&self, $(_: $arg),*) -> Result<$out, HostError> {
            Err(refusal(concat!($prefix, stringify!($name))))
        }
    )*};
    (later: $($name:ident($($arg:ty),*) -> $out:ty;)*) => {$(
        fn $name(&self, $(_: $arg),*) -> BoxFuture<'_, Result<$out, HostError>> {
            rustic_rifts_seam::stub(Component::Host, stringify!($name))
        }
    )*};
}

impl TransportLoader for Stub {
    refusing! { now "transport.":
        load() -> Box<dyn TransportAbi>;
    }
}

impl TransportAbi for Stub {
    refusing! { now "transport.":
        abi_version() -> u32;
        initialize() -> ();
        shutdown() -> ();
        peer_id() -> rustic_rifts_seam::PeerId;
        networking_ready() -> bool;
        host_open(&HostOpen) -> Handle;
        join_open(&JoinOpen) -> Handle;
        wait_ready(Handle, u32) -> ReadyState;
        session_running(Handle) -> bool;
        stop_session(Handle) -> ();
        last_session_result(Handle) -> SessionResult;
        read_signal(Handle) -> Option<Signal>;
        submit_signal(Handle, &rustic_rifts_seam::AccountId, &str, &str) -> ();
        read_peer_event(Handle) -> Option<String>;
        read_diagnostic(Handle) -> Option<String>;
        add_peer(Handle, &rustic_rifts_seam::AccountId, rustic_rifts_seam::PeerId, &rustic_rifts_seam::Secret) -> ();
        remove_peer(Handle, &rustic_rifts_seam::AccountId, &str) -> ();
        update_relay_credential(Handle, &str, &rustic_rifts_seam::Secret) -> ();
    }
}

impl RealmHosting for Stub {
    refusing! { later:
        coop_available() -> bool;
        host_hunt(rustic_rifts_seam::AccountId, rustic_rifts_seam::SessionId) -> LocalSession;
        join_hunt(rustic_rifts_seam::AccountId, rustic_rifts_seam::SessionId) -> LocalSession;
        start_hub(rustic_rifts_seam::AccountId, rustic_rifts_seam::HubKind, HubLimits) -> LocalSession;
        stop_hub(rustic_rifts_seam::AccountId, rustic_rifts_seam::HubKind) -> ();
        stop(rustic_rifts_seam::SessionId) -> ();
        sessions() -> Vec<LocalSession>;
    }
}

impl HostState for Stub {
    refusing! { now "state.":
        hub_limits() -> Option<HubLimits>;
        save_hub_limits(&HubLimits) -> ();
        control_cursor(&rustic_rifts_seam::AccountId) -> Option<u64>;
        set_control_cursor(&rustic_rifts_seam::AccountId, u64) -> ();
    }
}
