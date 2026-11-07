// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! A scripted transport (the part of the old fake library the session flows see), a recording spawner and backends
//! that wrap the default one.

#![allow(dead_code, reason = "shared by several test binaries, each using only some")]

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};

use rustic_rifts_host::{
    DefaultHostBackend, Handle, HostBackend, HostBootstrapSpec, HostError, HostOpen, HostedRole, HubLimits,
    JoinBootstrapSpec, JoinOpen, PeerListEntry, ReadyState, ServerSpawner, SessionResult, Signal, TransportAbi,
    TransportConfig,
};
use rustic_rifts_seam::{AccountId, HubKind, PeerId, Secret, SessionId};

pub const HOST_LINE: &str = "HOST_READY localHost=127.0.0.1 localPort=8791 transport=ice routePolicy=AUTOMATIC";
pub const ROUTE_LINE: &str =
    "ROUTE_READY localHost=127.0.0.1 localPort=29100 transport=ice routePolicy=AUTOMATIC route=direct";

#[derive(Debug, Default)]
pub struct FakeTransport {
    next: AtomicI32,
    pub host_line: Mutex<String>,
    pub join_line: Mutex<String>,
    /// `wait_ready` calls on a join handle answered `TimedOut` before it is ready.
    pub join_timeouts: AtomicU32,
    pub fail_host_open: AtomicBool,
    pub host_ready_state: Mutex<Option<ReadyState>>,
    pub disabled: AtomicBool,
    pub running: Mutex<HashSet<i32>>,
    pub forgotten: Mutex<HashSet<i32>>,
    pub joins: Mutex<HashSet<i32>>,
    pub host_opens: Mutex<Vec<HostOpen>>,
    pub join_opens: Mutex<Vec<JoinOpen>>,
    pub wait_timeouts: Mutex<Vec<u32>>,
    pub outgoing: Mutex<VecDeque<Signal>>,
    pub submitted: Mutex<Vec<(String, String, String)>>,
    pub events: Mutex<VecDeque<String>>,
    pub added: Mutex<Vec<(String, u64, String)>>,
    pub removed: Mutex<Vec<(String, String)>>,
    pub results: Mutex<BTreeMap<i32, i32>>,
}

impl FakeTransport {
    pub fn new() -> Self {
        let fake = Self::default();
        *fake.host_line.lock().unwrap() = HOST_LINE.to_owned();
        *fake.join_line.lock().unwrap() = ROUTE_LINE.to_owned();
        fake
    }

    fn open(&self, join: bool) -> Handle {
        let handle = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        self.running.lock().unwrap().insert(handle);
        if join {
            self.joins.lock().unwrap().insert(handle);
        }
        Handle(handle)
    }

    /// The session ends on its own with `code`.
    pub fn end(&self, handle: Handle, code: i32) {
        self.running.lock().unwrap().remove(&handle.0);
        self.results.lock().unwrap().insert(handle.0, code);
    }

    /// The transport loses the handle entirely.
    pub fn forget(&self, handle: Handle) {
        self.end(handle, -1);
        self.forgotten.lock().unwrap().insert(handle.0);
    }

    pub fn running_count(&self) -> usize {
        self.running.lock().unwrap().len()
    }
}

impl TransportAbi for FakeTransport {
    fn abi_version(&self) -> Result<u32, HostError> {
        Ok(3)
    }
    fn initialize(&self) -> Result<(), HostError> {
        Ok(())
    }
    fn shutdown(&self) -> Result<(), HostError> {
        Ok(())
    }
    fn peer_id(&self) -> Result<PeerId, HostError> {
        Ok(PeerId(0xDEAD_BEEF_CAFE_1234))
    }
    fn networking_ready(&self) -> Result<bool, HostError> {
        Ok(!self.disabled.load(Ordering::SeqCst))
    }
    fn host_open(&self, open: &HostOpen) -> Result<Handle, HostError> {
        self.host_opens.lock().unwrap().push(open.clone());
        if self.fail_host_open.load(Ordering::SeqCst) {
            return Err(HostError::Transport("fake host_open refused".to_owned()));
        }
        Ok(self.open(false))
    }
    fn join_open(&self, open: &JoinOpen) -> Result<Handle, HostError> {
        self.join_opens.lock().unwrap().push(open.clone());
        Ok(self.open(true))
    }
    fn wait_ready(&self, handle: Handle, timeout_ms: u32) -> Result<ReadyState, HostError> {
        self.wait_timeouts.lock().unwrap().push(timeout_ms);
        if !self.running.lock().unwrap().contains(&handle.0) {
            return Ok(ReadyState::Ended);
        }
        if self.joins.lock().unwrap().contains(&handle.0) {
            let left = self.join_timeouts.load(Ordering::SeqCst);
            if left > 0 {
                self.join_timeouts.store(left - 1, Ordering::SeqCst);
                return Ok(ReadyState::TimedOut);
            }
            return Ok(ReadyState::Ready(self.join_line.lock().unwrap().clone()));
        }
        if let Some(state) = self.host_ready_state.lock().unwrap().clone() {
            return Ok(state);
        }
        Ok(ReadyState::Ready(self.host_line.lock().unwrap().clone()))
    }
    fn session_running(&self, handle: Handle) -> Result<bool, HostError> {
        Ok(self.running.lock().unwrap().contains(&handle.0))
    }
    fn stop_session(&self, handle: Handle) -> Result<(), HostError> {
        if self.forgotten.lock().unwrap().contains(&handle.0) {
            return Err(HostError::NotFound(format!("unknown session handle {}", handle.0)));
        }
        self.end(handle, 0);
        Ok(())
    }
    fn last_session_result(&self, handle: Handle) -> Result<SessionResult, HostError> {
        Ok(SessionResult::from_code(
            self.results.lock().unwrap().get(&handle.0).copied().unwrap_or(0),
        ))
    }
    fn read_signal(&self, _handle: Handle) -> Result<Option<Signal>, HostError> {
        Ok(self.outgoing.lock().unwrap().pop_front())
    }
    fn submit_signal(&self, _handle: Handle, from: &AccountId, kind: &str, data: &str) -> Result<(), HostError> {
        self.submitted
            .lock()
            .unwrap()
            .push((from.0.clone(), kind.to_owned(), data.to_owned()));
        Ok(())
    }
    fn read_peer_event(&self, _handle: Handle) -> Result<Option<String>, HostError> {
        Ok(self.events.lock().unwrap().pop_front())
    }
    fn read_diagnostic(&self, _handle: Handle) -> Result<Option<String>, HostError> {
        Ok(None)
    }
    fn add_peer(&self, _handle: Handle, account: &AccountId, peer: PeerId, key: &Secret) -> Result<(), HostError> {
        self.added
            .lock()
            .unwrap()
            .push((account.0.clone(), peer.0, key.expose().to_owned()));
        Ok(())
    }
    fn remove_peer(&self, _handle: Handle, account: &AccountId, reason: &str) -> Result<(), HostError> {
        self.removed
            .lock()
            .unwrap()
            .push((account.0.clone(), reason.to_owned()));
        Ok(())
    }
    fn update_relay_credential(&self, _handle: Handle, _user: &str, _credential: &Secret) -> Result<(), HostError> {
        Ok(())
    }
}

/// Program, arguments, environment.
pub type Spawned = (String, Vec<String>, BTreeMap<String, String>);

#[derive(Debug, Default)]
pub struct RecordingSpawner {
    pub spawned: Mutex<Vec<Spawned>>,
    pub stopped: Mutex<Vec<u32>>,
}

impl ServerSpawner for RecordingSpawner {
    fn spawn(
        &self,
        program: &Path,
        arguments: &[String],
        _working_dir: &Path,
        environment: &BTreeMap<String, String>,
    ) -> Result<u32, HostError> {
        self.spawned.lock().unwrap().push((
            program.to_string_lossy().into_owned(),
            arguments.to_vec(),
            environment.clone(),
        ));
        Ok(77)
    }
    fn stop(&self, pid: u32) {
        self.stopped.lock().unwrap().push(pid);
    }
}

/// The default backend with knobs: an away host, signalling, a scripted roster and signal inbox.
#[derive(Debug, Default)]
pub struct TestBackend {
    pub inner: DefaultHostBackend,
    pub away: bool,
    pub signalling: bool,
    pub peers: Mutex<Vec<PeerListEntry>>,
    pub inbox: Mutex<Vec<(AccountId, Signal)>>,
    pub sent: Mutex<Vec<Signal>>,
    pub routes: Mutex<Vec<u16>>,
}

impl HostBackend for TestBackend {
    fn host_bootstrap(
        &self,
        account: &AccountId,
        session: &SessionId,
    ) -> Result<(HostBootstrapSpec, TransportConfig), HostError> {
        let (mut spec, config) = self.inner.host_bootstrap(account, session)?;
        if self.away {
            spec.local_client = false;
        }
        Ok((spec, config))
    }
    fn join_bootstrap(
        &self,
        account: &AccountId,
        session: &SessionId,
    ) -> Result<(JoinBootstrapSpec, TransportConfig), HostError> {
        self.inner.join_bootstrap(account, session)
    }
    fn start_hub(
        &self,
        account: &AccountId,
        kind: HubKind,
        limits: &HubLimits,
    ) -> Result<(HostBootstrapSpec, TransportConfig), HostError> {
        self.inner.start_hub(account, kind, limits)
    }
    fn stop_hub(&self, account: &AccountId, kind: HubKind) -> Result<Option<SessionId>, HostError> {
        self.inner.stop_hub(account, kind)
    }
    fn route_ready(&self, account: &AccountId, session: &SessionId, gen_: u64, port: u16) -> Result<(), HostError> {
        self.routes.lock().unwrap().push(port);
        self.inner.route_ready(account, session, gen_, port)
    }
    fn supports_signalling(&self) -> bool {
        self.signalling
    }
    fn poll_peers(
        &self,
        _account: &AccountId,
        _session: &SessionId,
        _generation: u64,
        _role: HostedRole,
    ) -> Result<Vec<PeerListEntry>, HostError> {
        Ok(self.peers.lock().unwrap().clone())
    }
    fn send_signal(&self, _a: &AccountId, _s: &SessionId, _g: u64, signal: &Signal) -> Result<(), HostError> {
        self.sent.lock().unwrap().push(signal.clone());
        Ok(())
    }
    fn poll_signals(&self, _a: &AccountId, _s: &SessionId, _g: u64) -> Result<Vec<(AccountId, Signal)>, HostError> {
        Ok(std::mem::take(&mut *self.inbox.lock().unwrap()))
    }
}

pub fn session(text: &str) -> SessionId {
    text.parse().unwrap()
}

pub fn wait_until(limit: std::time::Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = std::time::Instant::now() + limit;
    while std::time::Instant::now() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    done()
}
