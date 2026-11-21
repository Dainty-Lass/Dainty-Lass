// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The co-op transport as the launcher sees it: ABI 3's calls as a trait, whatever implements them.

use std::path::PathBuf;

use rustic_rifts_seam::{AccountId, PeerId, Secret};

use super::HostError;

/// The transport ABI the launcher speaks.
pub const ABI_VERSION: u32 = 3;

/// A transport session; valid handles are > 0, at most 8 running per transport.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Handle(pub i32);

/// The files a hosting session opens; `host_ticket` is absent when the host runs without one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HostOpen {
    /// The ticket signing public key.
    pub public_key: PathBuf,
    /// `bridge.conf`.
    pub config: PathBuf,
    /// The host capability file.
    pub capability: PathBuf,
    /// The host's own ticket, if any.
    pub host_ticket: Option<PathBuf>,
    /// `transport.secret`.
    pub secrets: PathBuf,
}

/// The files a joining session opens.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JoinOpen {
    /// The ticket signing public key.
    pub public_key: PathBuf,
    /// `bridge.conf`.
    pub config: PathBuf,
    /// The joiner's admission ticket.
    pub ticket: PathBuf,
    /// `transport.secret`.
    pub secrets: PathBuf,
}

/// `Ready` carries the transport's ready line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReadyState {
    /// The session is ready; the ready line it reported.
    Ready(String),
    /// Not ready within the timeout; the session still runs.
    TimedOut,
    /// The session ended before it was ready, or the handle is unknown.
    Ended,
}

/// How a transport session ended (ABI 3's result codes).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionResult {
    /// Stopped cleanly (0).
    Clean,
    /// An input file or value was invalid (2).
    BadInput,
    /// The session could not be set up (3).
    Setup,
    /// The joiner was refused admission (4).
    RefusedAdmission,
    /// The host's lease failed (5).
    LeaseFailed,
    /// No session has this handle (-1).
    UnknownHandle,
    /// Any other code.
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
    /// The result for an ABI 3 code.
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
    /// The peer's account.
    pub to_account: AccountId,
    /// `DESCRIPTION`, `CANDIDATE` or `GATHERING_DONE`.
    pub kind: String,
    /// The signal's text.
    pub data: String,
}

/// Synchronous: callers keep it off async executors.
///
/// The transport holds a signal from an account not yet on its roster for 20 s. Peer event lines are
/// `PEER_CONNECTED`, `PEER_ADMITTED`, `PEER_LOST`, `PEER_REMOVED` and `PEER_ADD_REJECTED`. A transport that cannot take
/// a new relay credential keeps the one it started with.
pub trait TransportAbi: Send + Sync {
    // The transport as a whole.
    /// The ABI version the transport speaks.
    fn abi_version(&self) -> Result<u32, HostError>;
    /// Draws the transport's peer id once; later calls keep it.
    fn initialize(&self) -> Result<(), HostError>;
    /// Whether the peer id was drawn.
    fn networking_ready(&self) -> Result<bool, HostError>;
    /// The transport's peer id.
    fn peer_id(&self) -> Result<PeerId, HostError>;
    /// Stops every session and waits for each.
    fn shutdown(&self) -> Result<(), HostError>;

    // One session's life.
    /// Opens a host session.
    fn host_open(&self, open: &HostOpen) -> Result<Handle, HostError>;
    /// Opens a joiner session.
    fn join_open(&self, open: &JoinOpen) -> Result<Handle, HostError>;
    /// Waits up to `timeout_ms` for the session's ready line.
    fn wait_ready(&self, handle: Handle, timeout_ms: u32) -> Result<ReadyState, HostError>;
    /// Whether the session is still running.
    fn session_running(&self, handle: Handle) -> Result<bool, HostError>;
    /// How the session ended, `Clean` while running.
    fn last_session_result(&self, handle: Handle) -> Result<SessionResult, HostError>;
    /// Stops the session and waits through its shutdown.
    fn stop_session(&self, handle: Handle) -> Result<(), HostError>;

    // What a session reports.
    /// The next peer event line, if any.
    fn read_peer_event(&self, handle: Handle) -> Result<Option<String>, HostError>;
    /// The next diagnostic line, if any.
    fn read_diagnostic(&self, handle: Handle) -> Result<Option<String>, HostError>;

    // Signalling, roster and relay.
    /// The next signal to relay to a peer, if any.
    fn read_signal(&self, handle: Handle) -> Result<Option<Signal>, HostError>;
    /// Hands the session a signal relayed from `from`.
    fn submit_signal(&self, handle: Handle, from: &AccountId, kind: &str, data: &str) -> Result<(), HostError>;
    /// Queues a joiner for a running host session.
    fn add_peer(&self, handle: Handle, account: &AccountId, peer: PeerId, pair_key: &Secret) -> Result<(), HostError>;
    /// Queues a joiner's removal from a running host session.
    fn remove_peer(&self, handle: Handle, account: &AccountId, reason: &str) -> Result<(), HostError>;
    /// Hands a running session a fresh TURN credential.
    fn update_relay_credential(&self, handle: Handle, username: &str, credential: &Secret) -> Result<(), HostError>;
}

/// Hands out a transport. It is linked into the launcher, so there is no file to find or pin (D1).
pub trait TransportLoader: Send + Sync {
    /// A fresh transport.
    fn load(&self) -> Result<Box<dyn TransportAbi>, HostError>;
}
