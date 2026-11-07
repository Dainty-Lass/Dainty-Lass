// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! What a session thread shares with the API: its queues, its ready line and result, and the roster view the API
//! checks adds and removes against.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use crate::Ready;
use crate::text::truncate;

/// Diagnostic lines kept unread; the oldest is dropped past this.
pub const MAX_DIAGNOSTICS: usize = 256;
/// The longest diagnostic line kept, in bytes.
pub const DIAGNOSTIC_BYTES: usize = 1023;
/// Peer event lines kept unread; the oldest is dropped past this.
pub const MAX_EVENTS: usize = 256;
/// The longest peer event line kept, in bytes.
pub const EVENT_BYTES: usize = 255;
/// Signals queued each way; past this, new ones are dropped.
pub const MAX_SIGNALS: usize = 2048;
/// Roster commands queued for the session thread.
pub const MAX_ROSTER_COMMANDS: usize = 256;
/// The longest failure code in a `SESSION_FAILURE` line, in bytes.
pub const FAILURE_CODE_BYTES: usize = 96;
/// The host's roster limit until its configuration sets one.
pub const DEFAULT_ROSTER_LIMIT: usize = 64;

/// Result codes (spec §2.17).
pub mod result {
    /// The session stopped cleanly.
    pub const CLEAN: i32 = 0;
    /// An input file or value was invalid.
    pub const BAD_INPUT: i32 = 2;
    /// The session could not be set up, or its thread panicked.
    pub const SETUP: i32 = 3;
    /// The joiner was refused admission.
    pub const REFUSED_ADMISSION: i32 = 4;
    /// The host's lease failed.
    pub const LEASE_FAILED: i32 = 5;
    /// No session has this handle.
    pub const UNKNOWN_HANDLE: i32 = -1;
}

/// A signal for the launcher to relay to a peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signal {
    /// The peer's account.
    pub to: String,
    /// `DESCRIPTION`, `CANDIDATE` or `GATHERING_DONE`.
    pub kind: String,
    /// The signal's text.
    pub data: String,
}

/// A signal the launcher relayed from a peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingSignal {
    /// The peer's account.
    pub from: String,
    /// `DESCRIPTION`, `CANDIDATE` or `GATHERING_DONE`.
    pub kind: String,
    /// The signal's text.
    pub data: String,
}

/// A roster change queued for a running host session.
#[derive(Clone, PartialEq, Eq)]
pub enum RosterCommand {
    /// Admit this joiner.
    Add {
        /// The joiner's account.
        account: String,
        /// The joiner's peer id.
        peer_id: u64,
        /// The key sealing this joiner's frames.
        pair_key: [u8; 32],
    },
    /// Close this joiner's link.
    Remove {
        /// The joiner's account.
        account: String,
        /// Why, cut to 48 bytes, for the diagnostics.
        reason: String,
    },
}

impl std::fmt::Debug for RosterCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Add { account, peer_id, .. } => write!(f, "Add({account}, {peer_id})"),
            Self::Remove { account, reason } => write!(f, "Remove({account}, {reason})"),
        }
    }
}

#[derive(Debug)]
struct RosterView {
    limit: usize,
    peers: HashMap<String, u64>,
}

#[derive(Debug)]
struct State {
    ready: Option<String>,
    ended_before_ready: bool,
    result: i32,
    diagnostics: VecDeque<String>,
    events: VecDeque<String>,
    outbox: VecDeque<Signal>,
    outbox_drops: u64,
    inbox: VecDeque<IncomingSignal>,
    commands: VecDeque<RosterCommand>,
    roster: RosterView,
}

/// One session's shared state; the session thread writes through [`SessionIo`], the API reads.
#[derive(Debug)]
pub struct Shared {
    /// Whether this is a host session, which alone takes roster commands.
    pub is_host: bool,
    stop: AtomicBool,
    running: AtomicBool,
    state: Mutex<State>,
    changed: Condvar,
}

impl Shared {
    /// A running session's state, nothing reported yet.
    pub fn new(is_host: bool) -> Self {
        Self {
            is_host,
            stop: AtomicBool::new(false),
            running: AtomicBool::new(true),
            state: Mutex::new(State {
                ready: None,
                ended_before_ready: false,
                result: result::SETUP,
                diagnostics: VecDeque::new(),
                events: VecDeque::new(),
                outbox: VecDeque::new(),
                outbox_drops: 0,
                inbox: VecDeque::new(),
                commands: VecDeque::new(),
                roster: RosterView {
                    limit: DEFAULT_ROSTER_LIMIT,
                    peers: HashMap::new(),
                },
            }),
            changed: Condvar::new(),
        }
    }

    fn lock(&self) -> Option<MutexGuard<'_, State>> {
        self.state.lock().ok()
    }

    /// Asks the session thread to stop.
    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    /// Whether a stop was asked for.
    pub fn stop_requested(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    /// Whether the session thread is still running.
    pub fn running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Called once by the session thread on exit.
    pub fn finish(&self, code: i32) {
        if let Some(mut state) = self.lock() {
            state.result = code;
            if state.ready.is_none() {
                state.ended_before_ready = true;
            }
        }
        self.running.store(false, Ordering::SeqCst);
        self.changed.notify_all();
    }

    /// A ready line once reported is kept after the end.
    pub fn wait_ready(&self, timeout: Duration) -> Ready {
        let deadline = Instant::now() + timeout;
        let Some(mut state) = self.lock() else {
            return Ready::Ended;
        };
        loop {
            if let Some(line) = &state.ready {
                return Ready::Ready(line.clone());
            }
            if state.ended_before_ready {
                return Ready::Ended;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ready::TimedOut;
            }
            state = match self.changed.wait_timeout(state, left) {
                Ok((state, _)) => state,
                Err(_) => return Ready::Ended,
            };
        }
    }

    /// The result code, 0 while running.
    pub fn result(&self) -> i32 {
        if self.running() {
            return 0;
        }
        self.lock().map_or(result::SETUP, |s| s.result)
    }

    /// Whether diagnostics, events or signals wait to be read.
    pub fn has_unread(&self) -> bool {
        self.lock()
            .is_some_and(|s| !s.diagnostics.is_empty() || !s.outbox.is_empty() || !s.events.is_empty())
    }

    /// The next diagnostic line, if any.
    pub fn read_diagnostic(&self) -> Option<String> {
        self.lock()?.diagnostics.pop_front()
    }

    /// The next peer event line, if any.
    pub fn read_event(&self) -> Option<String> {
        self.lock()?.events.pop_front()
    }

    /// The next outgoing signal, if any.
    pub fn read_signal(&self) -> Option<Signal> {
        self.lock()?.outbox.pop_front()
    }

    /// Queues a relayed signal; `false` when the session ended or the inbox is full.
    pub fn submit_signal(&self, signal: IncomingSignal) -> bool {
        let Some(mut state) = self.lock() else {
            return false;
        };
        if !self.running() || state.inbox.len() >= MAX_SIGNALS {
            return false;
        }
        state.inbox.push_back(signal);
        true
    }

    /// Queues a roster add; refused for a joiner session, a duplicate account or peer id, a full roster or queue.
    pub fn add_peer(&self, account: &str, peer_id: u64, pair_key: [u8; 32]) -> bool {
        let Some(mut state) = self.lock() else {
            return false;
        };
        let roster = &state.roster;
        let refused = roster.peers.contains_key(account)
            || roster.peers.values().any(|id| *id == peer_id)
            || roster.peers.len() >= roster.limit
            || state.commands.len() >= MAX_ROSTER_COMMANDS;
        if refused || !self.running() || !self.is_host {
            return false;
        }
        state.roster.peers.insert(account.to_owned(), peer_id);
        state.commands.push_back(RosterCommand::Add {
            account: account.to_owned(),
            peer_id,
            pair_key,
        });
        true
    }

    /// Queues a roster removal; refused for a joiner session, an account not on the roster or a full queue.
    pub fn remove_peer(&self, account: &str, reason: &str) -> bool {
        let Some(mut state) = self.lock() else {
            return false;
        };
        if !self.running() || !self.is_host || state.commands.len() >= MAX_ROSTER_COMMANDS {
            return false;
        }
        if state.roster.peers.remove(account).is_none() {
            return false;
        }
        state.commands.push_back(RosterCommand::Remove {
            account: account.to_owned(),
            reason: truncate(reason, 48).to_owned(),
        });
        true
    }
}

/// The session thread's side of [`Shared`].
#[derive(Debug, Clone, Copy)]
pub struct SessionIo<'a>(pub &'a Shared);

impl SessionIo<'_> {
    /// Whether a stop was asked for.
    pub fn stop_requested(&self) -> bool {
        self.0.stop_requested()
    }

    /// Queues a diagnostic line, cut to 1023 bytes; the oldest unread line is dropped when full.
    pub fn diagnostic(&self, line: &str) {
        if let Some(mut state) = self.0.lock() {
            if state.diagnostics.len() >= MAX_DIAGNOSTICS {
                state.diagnostics.pop_front();
            }
            state.diagnostics.push_back(truncate(line, DIAGNOSTIC_BYTES).to_owned());
        }
    }

    /// Queues a `SESSION_FAILURE code=...` diagnostic.
    pub fn failure(&self, code: &str) {
        self.diagnostic(&format!("SESSION_FAILURE code={}", truncate(code, FAILURE_CODE_BYTES)));
    }

    /// Queues a peer event line, cut to 255 bytes; the oldest unread line is dropped when full.
    pub fn event(&self, line: &str) {
        if let Some(mut state) = self.0.lock() {
            if state.events.len() >= MAX_EVENTS {
                state.events.pop_front();
            }
            state.events.push_back(truncate(line, EVENT_BYTES).to_owned());
        }
    }

    /// Queues a signal for the launcher; dropped when the outbox is full, with an occasional diagnostic.
    pub fn signal(&self, signal: Signal) {
        let dropped = {
            let Some(mut state) = self.0.lock() else {
                return;
            };
            if state.outbox.len() < MAX_SIGNALS {
                state.outbox.push_back(signal);
                None
            } else {
                state.outbox_drops += 1;
                Some(state.outbox_drops)
            }
        };
        if let Some(count) = dropped
            && count.is_power_of_two()
        {
            self.diagnostic("SIGNAL_DROPPED reason=outbox_full");
        }
    }

    /// Takes every relayed signal.
    pub fn take_inbox(&self) -> Vec<IncomingSignal> {
        self.0
            .lock()
            .map(|mut s| s.inbox.drain(..).collect())
            .unwrap_or_default()
    }

    /// Takes every queued roster command.
    pub fn take_commands(&self) -> Vec<RosterCommand> {
        self.0
            .lock()
            .map(|mut s| s.commands.drain(..).collect())
            .unwrap_or_default()
    }

    /// Reports the ready line and wakes anyone waiting on it.
    pub fn set_ready(&self, line: String) {
        if let Some(mut state) = self.0.lock() {
            state.ready = Some(line);
        }
        self.0.changed.notify_all();
    }

    /// The limit and starting peers, once the host accepted its configuration.
    pub fn report_roster(&self, limit: usize, peers: &[(String, u64)]) {
        if let Some(mut state) = self.0.lock() {
            state.roster.limit = limit;
            for (account, peer_id) in peers {
                state.roster.peers.insert(account.clone(), *peer_id);
            }
        }
    }

    /// Drops an account from the API's roster view.
    pub fn forget_peer(&self, account: &str) {
        if let Some(mut state) = self.0.lock() {
            state.roster.peers.remove(account);
        }
    }
}
