//! `ParadoxTransport.dll`'s ABI (`src/paradox_transport_api.hpp`, RF1 section 4) as a trait. `ffi` implements it over
//! the DLL and is the one module allowed `unsafe_code`.

use std::path::{Path, PathBuf};

use rifts_seam::{AccountId, PeerId, Secret};

use crate::HostError;

/// The version this surface speaks. Version 1's start functions always fail (an ICE session needs its secrets
/// file) and version 2's are wrappers over a default handle, so neither is in the trait.
pub const ABI_VERSION: u32 = 3;

/// Every export of ABI 3, in the header's order; the loader checks them all before the first call.
pub const EXPORTS: [&str; 29] = [
    "MysticTransport_AbiVersion",
    "MysticTransport_Initialize",
    "MysticTransport_Shutdown",
    "MysticTransport_PeerId",
    "MysticTransport_NetworkingReady",
    "MysticTransport_StartHost",
    "MysticTransport_StartJoin",
    "MysticTransport_StartHostV2",
    "MysticTransport_StartJoinV2",
    "MysticTransport_WaitReady",
    "MysticTransport_SessionRunning",
    "MysticTransport_LastSessionResult",
    "MysticTransport_StopSession",
    "MysticTransport_ReadDiagnostic",
    "MysticTransport_ReadSignal",
    "MysticTransport_SubmitSignal",
    "MysticTransport_ReadPeerEvent",
    "MysticTransport_HostOpen",
    "MysticTransport_JoinOpen",
    "MysticTransport_WaitReadyH",
    "MysticTransport_SessionRunningH",
    "MysticTransport_StopSessionH",
    "MysticTransport_LastSessionResultH",
    "MysticTransport_ReadSignalH",
    "MysticTransport_SubmitSignalH",
    "MysticTransport_ReadPeerEventH",
    "MysticTransport_ReadDiagnosticH",
    "MysticTransport_AddPeerH",
    "MysticTransport_RemovePeerH",
];

/// Looked up but not required: no transport exports it yet (RF1, decision 6).
pub const OPTIONAL_EXPORTS: [&str; 1] = ["MysticTransport_UpdateRelayCredentialH"];

/// A session handle, above 0; up to 8 per process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Handle(pub i32);

/// `HostOpen`'s files, all in the session folder but the public key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostOpen {
    /// The game folder's `p2p-admission-public.xy`.
    pub public_key: PathBuf,
    pub config: PathBuf,
    pub capability: PathBuf,
    /// For a host that also plays.
    pub host_ticket: Option<PathBuf>,
    pub secrets: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinOpen {
    pub public_key: PathBuf,
    pub config: PathBuf,
    pub ticket: PathBuf,
    pub secrets: PathBuf,
}

/// `WaitReadyH`: 1, 0 or -1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadyState {
    Ready(String),
    TimedOut,
    Ended,
}

/// `LastSessionResultH`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionResult {
    Clean,
    BadInput,
    Setup,
    RefusedAdmission,
    LeaseFailed,
    UnknownHandle,
    Other(i32),
}

impl SessionResult {
    pub const fn from_code(code: i32) -> Self {
        match code {
            0 => Self::Clean,
            2 => Self::BadInput,
            3 => Self::Setup,
            4 => Self::RefusedAdmission,
            5 => Self::LeaseFailed,
            -1 => Self::UnknownHandle,
            other => Self::Other(other),
        }
    }
}

/// `ReadSignalH`: one ICE signal for the backend to relay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signal {
    pub to_account: AccountId,
    pub kind: String,
    pub data: String,
}

/// The ABI, one method per export the launcher calls. Sync, as the C calls are; callers keep them off async tasks.
pub trait TransportAbi: Send + Sync {
    fn abi_version(&self) -> Result<u32, HostError>;
    fn initialize(&self) -> Result<(), HostError>;
    fn shutdown(&self) -> Result<(), HostError>;
    fn peer_id(&self) -> Result<PeerId, HostError>;
    fn networking_ready(&self) -> Result<bool, HostError>;
    fn host_open(&self, open: &HostOpen) -> Result<Handle, HostError>;
    fn join_open(&self, open: &JoinOpen) -> Result<Handle, HostError>;
    fn wait_ready(&self, handle: Handle, timeout_ms: u32) -> Result<ReadyState, HostError>;
    fn session_running(&self, handle: Handle) -> Result<bool, HostError>;
    fn stop_session(&self, handle: Handle) -> Result<(), HostError>;
    fn last_session_result(&self, handle: Handle) -> Result<SessionResult, HostError>;
    fn read_signal(&self, handle: Handle) -> Result<Option<Signal>, HostError>;
    /// Held 20 s by the transport for an account not yet on the roster.
    fn submit_signal(&self, handle: Handle, from: &AccountId, kind: &str, data: &str) -> Result<(), HostError>;
    /// `PEER_CONNECTED`, `PEER_ADMITTED`, `PEER_LOST`, `PEER_REMOVED`, `PEER_ADD_REJECTED` lines, as written.
    fn read_peer_event(&self, handle: Handle) -> Result<Option<String>, HostError>;
    fn read_diagnostic(&self, handle: Handle) -> Result<Option<String>, HostError>;
    fn add_peer(&self, handle: Handle, account: &AccountId, peer: PeerId, pair_key: &Secret) -> Result<(), HostError>;
    fn remove_peer(&self, handle: Handle, account: &AccountId, reason: &str) -> Result<(), HostError>;
    /// The optional export; a transport without it keeps the credential it started with.
    fn update_relay_credential(&self, handle: Handle, username: &str, credential: &Secret) -> Result<(), HostError>;
}

/// Loads the DLL from the game folder, after checking it against the signed Guard manifest's hash.
pub trait TransportLoader: Send + Sync {
    fn load(&self, dll: &Path, expected_sha256: [u8; 32]) -> Result<Box<dyn TransportAbi>, HostError>;
}
