// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The co-op transport as the launcher sees it: ABI 3's calls as a trait, whatever implements them.

use std::path::{Path, PathBuf};

use rifts_seam::{AccountId, PeerId, Secret};

use crate::HostError;

pub const ABI_VERSION: u32 = 3;

/// A transport session; valid handles are > 0, at most 8 running per transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Handle(pub i32);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostOpen {
    pub public_key: PathBuf,
    pub config: PathBuf,
    pub capability: PathBuf,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadyState {
    Ready(String),
    TimedOut,
    Ended,
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
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
    fn submit_signal(&self, handle: Handle, from: &AccountId, kind: &str, data: &str) -> Result<(), HostError>;
    fn read_peer_event(&self, handle: Handle) -> Result<Option<String>, HostError>;
    fn read_diagnostic(&self, handle: Handle) -> Result<Option<String>, HostError>;
    fn add_peer(&self, handle: Handle, account: &AccountId, peer: PeerId, pair_key: &Secret) -> Result<(), HostError>;
    fn remove_peer(&self, handle: Handle, account: &AccountId, reason: &str) -> Result<(), HostError>;
    fn update_relay_credential(&self, handle: Handle, username: &str, credential: &Secret) -> Result<(), HostError>;
}

/// `expected_sha256` is the Guard manifest's digest for `dll`; the in-process loader has no file to check (D1).
pub trait TransportLoader: Send + Sync {
    fn load(&self, dll: &Path, expected_sha256: [u8; 32]) -> Result<Box<dyn TransportAbi>, HostError>;
}
