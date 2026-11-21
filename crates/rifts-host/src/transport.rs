// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The co-op transport as the launcher sees it: ABI 3's calls as a trait, whatever implements them.

use std::path::{Path, PathBuf};

use rifts_seam::{AccountId, PeerId, Secret};

use super::HostError;

pub const ABI_VERSION: u32 = 3;

/// A transport session; valid handles are > 0, at most 8 running per transport.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Handle(pub i32);

/// The files a hosting session opens; `host_ticket` is absent when the host runs without one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostOpen {
    pub public_key: PathBuf,
    pub config: PathBuf,
    pub capability: PathBuf,
    pub host_ticket: Option<PathBuf>,
    pub secrets: PathBuf,
}

/// The files a joining session opens.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JoinOpen {
    pub public_key: PathBuf,
    pub config: PathBuf,
    pub ticket: PathBuf,
    pub secrets: PathBuf,
}

/// `Ready` carries the transport's ready line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReadyState {
    Ready(String),
    TimedOut,
    Ended,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionResult {
    Clean,
    BadInput,
    Setup,
    RefusedAdmission,
    LeaseFailed,
    UnknownHandle,
    Other(i32),
}

// ABI 3's result codes; 1 is unused and any code not here is `Other`.
const RESULT_CODES: [(i32, SessionResult); 6] = [
    (0, SessionResult::Clean),
    (2, SessionResult::BadInput),
    (3, SessionResult::Setup),
    (4, SessionResult::RefusedAdmission),
    (5, SessionResult::LeaseFailed),
    (-1, SessionResult::UnknownHandle),
];

impl SessionResult {
    pub const fn from_code(code: i32) -> Self {
        let mut at = 0;
        while at < RESULT_CODES.len() {
            if RESULT_CODES[at].0 == code {
                return RESULT_CODES[at].1;
            }
            at += 1;
        }
        Self::Other(code)
    }
}

/// An outgoing signalling message the transport wants delivered.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Signal {
    pub to_account: AccountId,
    pub kind: String,
    pub data: String,
}

/// Synchronous: callers keep it off async executors.
///
/// The transport holds a signal from an account not yet on its roster for 20 s. Peer event lines are
/// `PEER_CONNECTED`, `PEER_ADMITTED`, `PEER_LOST`, `PEER_REMOVED` and `PEER_ADD_REJECTED`. A transport that cannot take
/// a new relay credential keeps the one it started with.
pub trait TransportAbi: Send + Sync {
    // The transport as a whole.
    fn abi_version(&self) -> Result<u32, HostError>;
    fn initialize(&self) -> Result<(), HostError>;
    fn networking_ready(&self) -> Result<bool, HostError>;
    fn peer_id(&self) -> Result<PeerId, HostError>;
    fn shutdown(&self) -> Result<(), HostError>;

    // One session's life.
    fn host_open(&self, open: &HostOpen) -> Result<Handle, HostError>;
    fn join_open(&self, open: &JoinOpen) -> Result<Handle, HostError>;
    fn wait_ready(&self, handle: Handle, timeout_ms: u32) -> Result<ReadyState, HostError>;
    fn session_running(&self, handle: Handle) -> Result<bool, HostError>;
    fn last_session_result(&self, handle: Handle) -> Result<SessionResult, HostError>;
    fn stop_session(&self, handle: Handle) -> Result<(), HostError>;

    // What a session reports.
    fn read_peer_event(&self, handle: Handle) -> Result<Option<String>, HostError>;
    fn read_diagnostic(&self, handle: Handle) -> Result<Option<String>, HostError>;

    // Signalling, roster and relay.
    fn read_signal(&self, handle: Handle) -> Result<Option<Signal>, HostError>;
    fn submit_signal(&self, handle: Handle, from: &AccountId, kind: &str, data: &str) -> Result<(), HostError>;
    fn add_peer(&self, handle: Handle, account: &AccountId, peer: PeerId, pair_key: &Secret) -> Result<(), HostError>;
    fn remove_peer(&self, handle: Handle, account: &AccountId, reason: &str) -> Result<(), HostError>;
    fn update_relay_credential(&self, handle: Handle, username: &str, credential: &Secret) -> Result<(), HostError>;
}

/// `expected_sha256` is the Guard manifest's digest for `dll`; the in-process loader has no file to check (D1).
pub trait TransportLoader: Send + Sync {
    fn load(&self, dll: &Path, expected_sha256: [u8; 32]) -> Result<Box<dyn TransportAbi>, HostError>;
}
