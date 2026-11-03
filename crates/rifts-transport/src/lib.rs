// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Rustic Rifts' co-op transport, linked into the launcher (umbrella spec D1). [`Transport`] carries the old
//! transport ABI 3 as Rust calls: the same handles, ready and event lines, signals, roster commands, limits and
//! result codes. ABI 1 (always refused) and ABI 2 (one default handle over ABI 3) are not carried; a caller keeps
//! its own handle instead.
//!
//! One thread per session owns its ICE links and UDP sockets; the backend calls run on lanes of their own, on the
//! transport's tokio runtime.

pub mod backend;
pub mod bridge;
pub mod files;
pub mod filter;
pub mod fragment;
pub mod guard;
pub mod host;
pub mod join;
pub mod link;
pub mod pipe;
pub mod router;
pub mod run;
pub mod seal;
pub mod session;
pub mod text;
pub mod ticket;
pub mod url;

use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use rand::TryRng as _;

pub use crate::host::Timing;
use crate::run::{HostPaths, JoinPaths};
use crate::seal::decode_pair_key;
use crate::session::{IncomingSignal, SessionIo, Shared};
pub use crate::session::{Signal, result};
use crate::text::is_account;

pub const ABI_VERSION: u32 = 3;
pub const MAX_RUNNING: usize = 8;
const MAX_ENDED_KEPT: usize = 8;
const MAX_SIGNAL_DATA: usize = 4096;

/// A session handle: > 0, counted from 1, never reused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Handle(pub i32);

/// `WaitReadyH`'s three answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ready {
    Ready(String),
    TimedOut,
    /// The session ended before it was ready, or the handle is unknown.
    Ended,
}

struct Entry {
    shared: Arc<Shared>,
    thread: Option<thread::JoinHandle<()>>,
}

#[derive(Default)]
struct Registry {
    next: i32,
    sessions: BTreeMap<i32, Entry>,
}

struct Inner {
    peer_id: AtomicU64,
    registry: Mutex<Registry>,
    runtime: Option<tokio::runtime::Runtime>,
    timing: Timing,
}

impl Inner {
    fn shutdown(&self) {
        let threads: Vec<thread::JoinHandle<()>> = match self.registry.lock() {
            Ok(mut registry) => registry
                .sessions
                .values_mut()
                .filter_map(|entry| {
                    entry.shared.request_stop();
                    entry.thread.take()
                })
                .collect(),
            Err(_) => Vec::new(),
        };
        for thread in threads {
            let _ = thread.join();
        }
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// The co-op transport. Every method may be called from any thread, concurrently.
#[derive(Clone)]
pub struct Transport {
    inner: Arc<Inner>,
}

impl Default for Transport {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for Transport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Transport")
            .field("peer_id", &self.peer_id())
            .finish_non_exhaustive()
    }
}

fn path_given(path: &Path) -> bool {
    !path.as_os_str().is_empty()
}

impl Transport {
    pub fn new() -> Self {
        Self::with_timing(Timing::default())
    }

    /// Shorter admission and held-signal timeouts, for tests.
    pub fn with_timing(timing: Timing) -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("rifts-transport")
            .enable_all()
            .build()
            .ok();
        Self {
            inner: Arc::new(Inner {
                peer_id: AtomicU64::new(0),
                registry: Mutex::default(),
                runtime,
                timing,
            }),
        }
    }

    /// Whether both values are the same transport instance.
    pub fn same_as(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    pub fn abi_version(&self) -> u32 {
        ABI_VERSION
    }

    /// Draws the transport's peer id once; later calls keep it.
    pub fn initialize(&self) -> Result<(), String> {
        if self.inner.peer_id.load(Ordering::SeqCst) != 0 {
            return Ok(());
        }
        loop {
            let mut bytes = [0_u8; 8];
            rand::rngs::SysRng
                .try_fill_bytes(&mut bytes)
                .map_err(|_| "The co-op transport could not create its identity.".to_owned())?;
            let id = u64::from_be_bytes(bytes);
            if id != 0 {
                let _ = self
                    .inner
                    .peer_id
                    .compare_exchange(0, id, Ordering::SeqCst, Ordering::SeqCst);
                return Ok(());
            }
        }
    }

    /// Stops every session and waits for each; their lines stay readable.
    pub fn shutdown(&self) {
        self.inner.shutdown();
    }

    pub fn peer_id(&self) -> u64 {
        self.inner.peer_id.load(Ordering::SeqCst)
    }

    pub fn networking_ready(&self) -> bool {
        self.peer_id() != 0
    }

    fn open(&self, is_host: bool, run: impl FnOnce(SessionIo<'_>) -> i32 + Send + 'static) -> Result<Handle, String> {
        let Ok(mut registry) = self.inner.registry.lock() else {
            return Err("The co-op session thread could not start.".to_owned());
        };
        for entry in registry.sessions.values_mut() {
            if !entry.shared.running()
                && let Some(thread) = entry.thread.take()
            {
                let _ = thread.join();
            }
        }
        registry
            .sessions
            .retain(|_, entry| entry.shared.running() || entry.shared.has_unread());
        let ended: Vec<i32> = registry
            .sessions
            .iter()
            .filter(|(_, e)| !e.shared.running())
            .map(|(h, _)| *h)
            .collect();
        for handle in ended.iter().rev().skip(MAX_ENDED_KEPT) {
            registry.sessions.remove(handle);
        }
        let running = registry.sessions.values().filter(|e| e.shared.running()).count();
        if running >= MAX_RUNNING {
            return Err("At most 8 co-op sessions can run at once.".to_owned());
        }
        let shared = Arc::new(Shared::new(is_host));
        let thread_shared = shared.clone();
        let thread = thread::Builder::new()
            .name("rifts-session".to_owned())
            .spawn(move || {
                let io = SessionIo(&thread_shared);
                let code = catch_unwind(AssertUnwindSafe(|| run(io))).unwrap_or_else(|_| {
                    io.failure("exception");
                    result::SETUP
                });
                thread_shared.finish(code);
            })
            .map_err(|_| "The co-op session thread could not start.".to_owned())?;
        registry.next += 1;
        let handle = registry.next;
        registry.sessions.insert(
            handle,
            Entry {
                shared,
                thread: Some(thread),
            },
        );
        Ok(Handle(handle))
    }

    /// Opens a host session; `host_ticket` may be absent. Files are read on the session thread, so a bad file shows
    /// as an end before readiness.
    pub fn host_open(
        &self,
        public_key: &Path,
        config: &Path,
        capability: &Path,
        host_ticket: Option<&Path>,
        secrets: &Path,
    ) -> Result<Handle, String> {
        if ![public_key, config, capability, secrets].into_iter().all(path_given) {
            return Err("A co-op host input path is missing.".to_owned());
        }
        let paths = HostPaths {
            public_key: public_key.to_owned(),
            config: config.to_owned(),
            capability: capability.to_owned(),
            host_ticket: host_ticket.map(PathBuf::from),
            secrets: secrets.to_owned(),
        };
        let runtime = self.inner.runtime.as_ref().map(|r| r.handle().clone());
        let timing = self.inner.timing;
        self.open(true, move |io| run::host(io, &paths, runtime.as_ref(), timing))
    }

    pub fn join_open(&self, public_key: &Path, config: &Path, ticket: &Path, secrets: &Path) -> Result<Handle, String> {
        if ![public_key, config, ticket, secrets].into_iter().all(path_given) {
            return Err("A co-op join input path is missing.".to_owned());
        }
        let paths = JoinPaths {
            public_key: public_key.to_owned(),
            config: config.to_owned(),
            ticket: ticket.to_owned(),
            secrets: secrets.to_owned(),
        };
        let timing = self.inner.timing;
        self.open(false, move |io| run::join(io, &paths, timing))
    }

    fn shared(&self, handle: Handle) -> Option<Arc<Shared>> {
        let registry = self.inner.registry.lock().ok()?;
        registry.sessions.get(&handle.0).map(|e| e.shared.clone())
    }

    /// A ready line once reported is kept, even after the session ended.
    pub fn wait_ready(&self, handle: Handle, timeout_ms: u32) -> Ready {
        let Some(shared) = self.shared(handle) else {
            return Ready::Ended;
        };
        shared.wait_ready(Duration::from_millis(u64::from(timeout_ms)))
    }

    pub fn session_running(&self, handle: Handle) -> bool {
        self.shared(handle).is_some_and(|s| s.running())
    }

    /// Stops the session and waits through its shutdown; `false` for an unknown handle.
    pub fn stop_session(&self, handle: Handle) -> bool {
        let thread = {
            let Ok(mut registry) = self.inner.registry.lock() else {
                return false;
            };
            let Some(entry) = registry.sessions.get_mut(&handle.0) else {
                return false;
            };
            entry.shared.request_stop();
            entry.thread.take()
        };
        if let Some(thread) = thread {
            let _ = thread.join();
        }
        true
    }

    /// The result code (0 while running), -1 for an unknown handle.
    pub fn last_session_result(&self, handle: Handle) -> i32 {
        self.shared(handle).map_or(result::UNKNOWN_HANDLE, |s| s.result())
    }

    pub fn read_signal(&self, handle: Handle) -> Option<Signal> {
        self.shared(handle)?.read_signal()
    }

    /// Refused for a bad account, kind or data, an unknown or ended session, or a full queue.
    pub fn submit_signal(&self, handle: Handle, from_account: &str, kind: &str, data: &str) -> bool {
        let kind_ok = matches!(kind, filter::DESCRIPTION | filter::CANDIDATE | filter::GATHERING_DONE);
        let data_ok = data.len() <= MAX_SIGNAL_DATA
            && data
                .bytes()
                .all(|b| (0x20..=0x7e).contains(&b) || b == b'\r' || b == b'\n');
        if !is_account(from_account) || !kind_ok || !data_ok {
            return false;
        }
        self.shared(handle).is_some_and(|s| {
            s.submit_signal(IncomingSignal {
                from: from_account.to_owned(),
                kind: kind.to_owned(),
                data: data.to_owned(),
            })
        })
    }

    pub fn read_peer_event(&self, handle: Handle) -> Option<String> {
        self.shared(handle)?.read_event()
    }

    pub fn read_diagnostic(&self, handle: Handle) -> Option<String> {
        self.shared(handle)?.read_diagnostic()
    }

    /// Queues a roster add on a running host. The host's own peer id is refused later by the session
    /// (`PEER_ADD_REJECTED … reason=invalid_peer`).
    pub fn add_peer(&self, handle: Handle, account: &str, peer_id: u64, pair_key_b64url: &str) -> bool {
        if peer_id == 0 || !is_account(account) {
            return false;
        }
        let Some(mut key) = decode_pair_key(pair_key_b64url) else {
            return false;
        };
        let added = self.shared(handle).is_some_and(|s| s.add_peer(account, peer_id, key));
        key = [0; 32];
        let _ = key;
        added
    }

    /// Queues a roster remove; the reason is cut to 48 bytes.
    pub fn remove_peer(&self, handle: Handle, account: &str, reason: Option<&str>) -> bool {
        if !is_account(account) {
            return false;
        }
        self.shared(handle)
            .is_some_and(|s| s.remove_peer(account, reason.unwrap_or_default()))
    }
}
