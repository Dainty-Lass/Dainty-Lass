// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Hosting and joining realm sessions: bootstrap from the backend, the session folder, the hosted server, the
//! transport session and its readiness, then a pump thread that carries signals and the roster.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use rustic_rifts_seam::{AccountId, BoxFuture, HubKind, PeerId, Secret, SessionId};

use super::backend::{DefaultHostBackend, HostBackend, NoopServerSpawner, ServerSpawner};
use super::folder::{cleanup_directory, create_session_directory, random_hex, write_session_file_0600};
use super::rules::{
    build_launch_environment, host_server_arguments, ice_files, parse_ice_ready, peer_diff, safe_config_value,
    trials_week, wanted_peers,
};
use super::{
    FilesFor, HostBootstrapSpec, HostedRole, HubLimits, IceFiles, JoinBootstrapSpec, LaunchEnvironmentSpec,
    LocalSession, LocalState, TransportConfig,
};
use crate::HostError;
use crate::files::{ADMISSION_PUBLIC_KEY_FILE, DISABLE_COOP_FLAG_FILE, SESSION_FILES};
use crate::transport::{ABI_VERSION, Handle, HostOpen, JoinOpen, ReadyState, SessionResult, TransportAbi};

/// Hosting and joining realm sessions from the launcher.
pub trait RealmHosting: Send + Sync {
    /// Whether co-op can run now: no opt-out flag, the right transport ABI and networking ready.
    fn coop_available(&self) -> BoxFuture<'_, Result<bool, HostError>>;
    /// Hosts a hunt: bootstraps it, writes its folder, opens the transport and starts the server.
    fn host_hunt(&self, account: AccountId, session: SessionId) -> BoxFuture<'_, Result<LocalSession, HostError>>;
    /// Joins a hunt hosted by another player.
    fn join_hunt(&self, account: AccountId, session: SessionId) -> BoxFuture<'_, Result<LocalSession, HostError>>;
    /// Starts a hub of this kind under these limits.
    fn start_hub(
        &self,
        account: AccountId,
        kind: HubKind,
        limits: HubLimits,
    ) -> BoxFuture<'_, Result<LocalSession, HostError>>;
    /// Stops this account's hub of this kind.
    fn stop_hub(&self, account: AccountId, kind: HubKind) -> BoxFuture<'_, Result<(), HostError>>;
    /// Stops a local session and its server.
    fn stop(&self, session: SessionId) -> BoxFuture<'_, Result<(), HostError>>;
    /// Every local session, states refreshed from the transport.
    fn sessions(&self) -> BoxFuture<'_, Result<Vec<LocalSession>, HostError>>;
}

const DEFAULT_WAIT_READY_MS: u32 = 5000;
const PUMP_EVERY: Duration = Duration::from_millis(50);
const JOIN_WAIT_SLICE_MS: u128 = 50;
/// What a joiner reports when the transport's ready text is not a `ROUTE_READY` line.
const JOIN_FALLBACK_PORT: u16 = 29001;
/// A second place the opt-out flag is looked for, under the app's data folder.
const FLAG_SUBFOLDER: &str = "RusticRifts";
const SERVER_EXECUTABLE: &str = "Dauntless.exe";

struct Pump {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

struct Recorded {
    local: LocalSession,
    account: AccountId,
    handle: Handle,
    directory: PathBuf,
    pid: Option<u32>,
    environment: BTreeMap<String, String>,
    local_port: u16,
    pump: Option<Pump>,
}

/// Removes the session folder on every failure after it was made; [`Folder::keep`] ends that.
struct Folder(Option<PathBuf>);

impl Folder {
    fn path(&self) -> &Path {
        self.0.as_deref().unwrap_or(Path::new(""))
    }

    fn keep(mut self) -> PathBuf {
        self.0.take().unwrap_or_default()
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        if let Some(path) = &self.0 {
            cleanup_directory(path);
        }
    }
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The launcher's own environment; a variable that is not UTF-8 is left out rather than panicking.
fn inherited_environment() -> Vec<(String, String)> {
    std::env::vars_os()
        .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
        .collect()
}

/// Hosts and joins on this PC, synchronously inside each call; the futures it returns are already complete.
pub struct LocalRealmHosting {
    app_local_data: PathBuf,
    game_dir: PathBuf,
    transport: Arc<dyn TransportAbi>,
    backend: Arc<dyn HostBackend>,
    spawner: Arc<dyn ServerSpawner>,
    wait_ready_timeout_ms: u32,
    sessions: Mutex<BTreeMap<SessionId, Recorded>>,
}

impl std::fmt::Debug for LocalRealmHosting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalRealmHosting")
            .field("app_local_data", &self.app_local_data)
            .field("game_dir", &self.game_dir)
            .field("wait_ready_timeout_ms", &self.wait_ready_timeout_ms)
            .finish_non_exhaustive()
    }
}

struct Roster {
    role: HostedRole,
    host_account: String,
    linked: HashSet<String>,
}

struct PumpWork {
    transport: Arc<dyn TransportAbi>,
    backend: Arc<dyn HostBackend>,
    handle: Handle,
    account: AccountId,
    session: SessionId,
    generation: u64,
    roster: Option<Roster>,
}

/// One pass: outgoing signals to the backend, incoming ones to the transport, then the event and diagnostic queues
/// drained so they never fill.
fn signal_pass(
    transport: &dyn TransportAbi,
    backend: &dyn HostBackend,
    handle: Handle,
    account: &AccountId,
    session: &SessionId,
    generation: u64,
) {
    while let Ok(Some(signal)) = transport.read_signal(handle) {
        let _ = backend.send_signal(account, session, generation, &signal);
    }
    if let Ok(incoming) = backend.poll_signals(account, session, generation) {
        for (from, signal) in incoming {
            let _ = transport.submit_signal(handle, &from, &signal.kind, &signal.data);
        }
    }
    while let Ok(Some(_)) = transport.read_peer_event(handle) {}
    while let Ok(Some(_)) = transport.read_diagnostic(handle) {}
}

impl PumpWork {
    fn roster_pass(&mut self) {
        let Some(roster) = self.roster.as_mut() else {
            return;
        };
        let Ok(entries) = self
            .backend
            .poll_peers(&self.account, &self.session, self.generation, roster.role)
        else {
            return;
        };
        let (wanted, _refused) = wanted_peers(&entries, Some(&roster.host_account));
        let diff = peer_diff(&roster.linked, &wanted);
        for peer in diff.add {
            let added = self.transport.add_peer(
                self.handle,
                &AccountId(peer.account_id.clone()),
                PeerId(peer.peer_id),
                &Secret::new(peer.transport_key),
            );
            if added.is_ok() {
                roster.linked.insert(peer.account_id);
            }
        }
        for account in diff.remove {
            let _ = self
                .transport
                .remove_peer(self.handle, &AccountId(account.clone()), "roster");
            roster.linked.remove(&account);
        }
    }

    fn run(mut self, stop: &AtomicBool) {
        while !stop.load(Ordering::SeqCst) {
            if !matches!(self.transport.session_running(self.handle), Ok(true)) {
                return;
            }
            self.roster_pass();
            signal_pass(
                self.transport.as_ref(),
                self.backend.as_ref(),
                self.handle,
                &self.account,
                &self.session,
                self.generation,
            );
            thread::sleep(PUMP_EVERY);
        }
    }
}

fn start_pump(work: PumpWork) -> Result<Pump, HostError> {
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = stop.clone();
    let thread = thread::Builder::new()
        .name("rifts-host-pump".to_owned())
        .spawn(move || work.run(&thread_stop))
        .map_err(|e| HostError::Io(format!("could not start the session pump: {e}")))?;
    Ok(Pump {
        stop,
        thread: Some(thread),
    })
}

fn bool_text(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

impl LocalRealmHosting {
    /// Hosting over this transport with the local backend, which spawns no server.
    pub fn new(
        app_local_data: impl Into<PathBuf>,
        game_dir: impl Into<PathBuf>,
        transport: Arc<dyn TransportAbi>,
    ) -> Self {
        Self::with_backend_and_spawner(
            app_local_data,
            game_dir,
            transport,
            Arc::new(DefaultHostBackend::new()),
            Arc::new(NoopServerSpawner::new()),
        )
    }

    /// Hosting over this transport, backend and server spawner.
    pub fn with_backend_and_spawner(
        app_local_data: impl Into<PathBuf>,
        game_dir: impl Into<PathBuf>,
        transport: Arc<dyn TransportAbi>,
        backend: Arc<dyn HostBackend>,
        spawner: Arc<dyn ServerSpawner>,
    ) -> Self {
        Self {
            app_local_data: app_local_data.into(),
            game_dir: game_dir.into(),
            transport,
            backend,
            spawner,
            wait_ready_timeout_ms: DEFAULT_WAIT_READY_MS,
            sessions: Mutex::new(BTreeMap::new()),
        }
    }

    /// How long opening a session waits for the transport's ready line, in milliseconds.
    #[must_use]
    pub fn with_wait_ready_timeout_ms(mut self, timeout_ms: u32) -> Self {
        self.wait_ready_timeout_ms = timeout_ms;
        self
    }

    /// The session's folder, while the session is recorded.
    pub fn session_directory(&self, session: &SessionId) -> Option<PathBuf> {
        locked(&self.sessions).get(session).map(|r| r.directory.clone())
    }

    /// Empty for a joined session, which starts no server.
    pub fn session_launch_environment(&self, session: &SessionId) -> Option<BTreeMap<String, String>> {
        locked(&self.sessions).get(session).map(|r| r.environment.clone())
    }

    /// The loopback port the local game client connects to, 0 when none does.
    pub fn session_local_port(&self, session: &SessionId) -> Option<u16> {
        locked(&self.sessions).get(session).map(|r| r.local_port)
    }

    fn coop_now(&self) -> bool {
        let flagged = [
            self.app_local_data.clone(),
            self.app_local_data.join(FLAG_SUBFOLDER),
            self.game_dir.clone(),
        ]
        .iter()
        .any(|dir| dir.join(DISABLE_COOP_FLAG_FILE).exists());
        if flagged || !matches!(self.transport.abi_version(), Ok(ABI_VERSION)) {
            return false;
        }
        self.transport.networking_ready().unwrap_or(false)
    }

    fn admit_start(&self, session: &SessionId) -> Result<(), HostError> {
        if !self.coop_now() {
            return Err(HostError::Transport(
                "co-op transport is disabled or not ready".to_owned(),
            ));
        }
        if locked(&self.sessions).contains_key(session) {
            return Err(HostError::Busy(format!("session {session} is already running")));
        }
        Ok(())
    }

    fn public_key(&self) -> PathBuf {
        self.game_dir.join(ADMISSION_PUBLIC_KEY_FILE)
    }

    fn stop_transport(&self, handle: Handle) {
        let _ = self.transport.stop_session(handle);
    }

    fn host_conf(spec: &HostBootstrapSpec, ice: &IceFiles, guard_token: &str, pid: u32) -> Result<String, HostError> {
        let common = spec.common.to_config_lines()?;
        let backend_url = safe_config_value(&spec.backend_url, 512)?;
        let mut text = format!(
            "{common}backend_url={backend_url}\nserver_udp_port={}\nmaximum_peers={}\nforce_relay={}\n\
             guard_pipe_name={guard_token}\nhunt_evidence={}\nexpected_server_pid={pid}\n",
            spec.server_udp_port,
            spec.maximum_peers,
            bool_text(spec.common.force_relay),
            bool_text(spec.hunt_evidence),
        );
        match spec.role {
            HostedRole::Hub(_) => text.push_str("session_role=HUB\nhost_local_client=1\n"),
            HostedRole::Hunt if !spec.local_client => text.push_str("host_local_client=0\n"),
            HostedRole::Hunt => {}
        }
        text.push_str(&ice.config);
        Ok(text)
    }

    fn write_files(dir: &Path, contents: [&[u8]; 4]) -> Result<[PathBuf; 4], HostError> {
        let paths = SESSION_FILES.map(|name| dir.join(name));
        for (path, bytes) in paths.iter().zip(contents) {
            write_session_file_0600(path, bytes)?;
        }
        Ok(paths)
    }

    /// Writes the four files and opens the host session.
    fn open_host(
        &self,
        dir: &Path,
        spec: &HostBootstrapSpec,
        ice: &IceFiles,
        guard_token: &str,
        pid: u32,
    ) -> Result<Handle, HostError> {
        let conf = Self::host_conf(spec, ice, guard_token, pid)?;
        let ticket = spec.host_ticket.as_ref().map_or("", Secret::expose);
        let [config, secrets, capability, ticket_path] = Self::write_files(
            dir,
            [
                conf.as_bytes(),
                ice.secrets.expose().as_bytes(),
                spec.capability.expose().as_bytes(),
                ticket.as_bytes(),
            ],
        )?;
        self.transport.host_open(&HostOpen {
            public_key: self.public_key(),
            config,
            capability,
            host_ticket: spec.local_client.then_some(ticket_path),
            secrets,
        })
    }

    fn host_port(&self, handle: Handle, spec: &HostBootstrapSpec) -> Result<u16, HostError> {
        match self.transport.wait_ready(handle, self.wait_ready_timeout_ms)? {
            ReadyState::Ready(line) if line.starts_with("HOST_READY ") => {
                parse_ice_ready(&line, "HOST_READY", spec.common.force_relay, spec.local_client)
            }
            ReadyState::Ready(_) => Ok(if spec.local_client { spec.server_udp_port } else { 0 }),
            ReadyState::TimedOut => Err(HostError::Transport(
                "transport timed out waiting for HOST_READY".to_owned(),
            )),
            ReadyState::Ended => Err(HostError::Transport(
                "transport session ended before HOST_READY".to_owned(),
            )),
        }
    }

    fn start_host(
        &self,
        account: AccountId,
        spec: HostBootstrapSpec,
        config: TransportConfig,
    ) -> Result<LocalSession, HostError> {
        let session = spec.common.session_id.clone();
        let generation = spec.common.generation;
        self.admit_start(&session)?;
        let files_for = match spec.role {
            HostedRole::Hub(_) => FilesFor::HubHost,
            HostedRole::Hunt if spec.local_client => FilesFor::HuntHost,
            HostedRole::Hunt => FilesFor::AwayHuntHost,
        };
        let ice = ice_files(&config, files_for, &spec.common.host_peer_id)?;
        let folder = Folder(Some(create_session_directory(
            &self.app_local_data,
            &session,
            generation,
        )?));

        let x = random_hex::<16>()?;
        let guard_token = format!("RusticHuntGuard.events.{x}");
        let hub = matches!(spec.role, HostedRole::Hub(_));
        let environment = build_launch_environment(
            &LaunchEnvironmentSpec {
                capability: spec.capability.clone(),
                role: spec.role,
                session_id: session.clone(),
                generation,
                launch_id: format!("host-{x}"),
                ready_pipe: format!(r"\\.\pipe\RusticInternalServer.ready.{x}"),
                control_pipe: format!(r"\\.\pipe\RusticInternalServer.control.{x}"),
                guard_pipe: format!(r"\\.\pipe\{guard_token}"),
                hunt_evidence: spec.hunt_evidence,
                guard_completion_mode: spec.common.guard_completion_mode.clone(),
                runtime_profile: spec.runtime_profile.clone(),
                launcher_pid: std::process::id(),
                max_players: spec.max_players,
                hub_afk_seconds: spec.hub_afk_seconds,
                trials_week: trials_week(&spec.matchmaker_hunt_id).map(str::to_owned),
                backend_url: spec.backend_url.clone(),
            },
            inherited_environment(),
        );
        let arguments = host_server_arguments(
            spec.server_udp_port,
            &spec.map_path,
            &spec.behemoth_path,
            &spec.matchmaker_hunt_id,
            &spec.expected_players,
        );
        let pid = self.spawner.spawn(
            &self.game_dir.join(SERVER_EXECUTABLE),
            &arguments,
            &self.game_dir,
            &environment,
        )?;
        // The server listens before the transport opens and the backend hears the route (spec-runtime-r10 12.2)
        if let Err(error) = self.spawner.wait_ready(pid) {
            self.spawner.stop(pid);
            return Err(error);
        }

        let started = self
            .open_host(folder.path(), &spec, &ice, &guard_token, pid)
            .and_then(|handle| {
                let ready = self.host_port(handle, &spec).and_then(|port| {
                    if spec.local_client {
                        self.backend.route_ready(&account, &session, generation, port)?;
                    }
                    let pump = if self.backend.supports_signalling() {
                        let linked = if hub {
                            HashSet::new()
                        } else {
                            config.peers.iter().map(|p| p.account_id.clone()).collect()
                        };
                        Some(start_pump(PumpWork {
                            transport: self.transport.clone(),
                            backend: self.backend.clone(),
                            handle,
                            account: account.clone(),
                            session: session.clone(),
                            generation,
                            roster: Some(Roster {
                                role: spec.role,
                                host_account: config.local_account_id.clone(),
                                linked,
                            }),
                        })?)
                    } else {
                        None
                    };
                    Ok((port, pump))
                });
                if ready.is_err() {
                    self.stop_transport(handle);
                }
                ready.map(|(port, pump)| (handle, port, pump))
            });
        let (handle, local_port, pump) = match started {
            Ok(started) => started,
            Err(error) => {
                self.spawner.stop(pid);
                return Err(error);
            }
        };

        let local = LocalSession {
            session: session.clone(),
            generation,
            role: spec.role,
            hosting: true,
            state: LocalState::Running,
        };
        let recorded = Recorded {
            local: local.clone(),
            account,
            handle,
            directory: folder.keep(),
            pid: Some(pid),
            environment,
            local_port,
            pump,
        };
        locked(&self.sessions).insert(session, recorded);
        Ok(local)
    }

    fn join_ready(&self, handle: Handle, account: &AccountId, session: &SessionId, generation: u64) -> ReadyResult {
        let timeout = self.wait_ready_timeout_ms;
        if !self.backend.supports_signalling() {
            return self.transport.wait_ready(handle, timeout);
        }
        let deadline = Instant::now() + Duration::from_millis(u64::from(timeout));
        loop {
            signal_pass(
                self.transport.as_ref(),
                self.backend.as_ref(),
                handle,
                account,
                session,
                generation,
            );
            let left = deadline.saturating_duration_since(Instant::now()).as_millis();
            let slice = u32::try_from(left.clamp(1, JOIN_WAIT_SLICE_MS)).unwrap_or(1);
            match self.transport.wait_ready(handle, slice) {
                Ok(ReadyState::TimedOut) if Instant::now() < deadline => {}
                other => return other,
            }
        }
    }

    fn join_port(&self, handle: Handle, spec: &JoinBootstrapSpec, account: &AccountId) -> Result<u16, HostError> {
        let common = &spec.common;
        match self.join_ready(handle, account, &common.session_id, common.generation)? {
            ReadyState::Ready(line) if line.starts_with("ROUTE_READY ") => {
                parse_ice_ready(&line, "ROUTE_READY", common.force_relay, true)
            }
            ReadyState::Ready(_) => Ok(JOIN_FALLBACK_PORT),
            ReadyState::TimedOut => Err(HostError::Transport(
                "transport timed out waiting for ROUTE_READY".to_owned(),
            )),
            ReadyState::Ended => Err(HostError::Transport(
                "transport session ended before ROUTE_READY".to_owned(),
            )),
        }
    }

    fn start_join(
        &self,
        account: AccountId,
        spec: JoinBootstrapSpec,
        config: TransportConfig,
    ) -> Result<LocalSession, HostError> {
        let session = spec.common.session_id.clone();
        let generation = spec.common.generation;
        self.admit_start(&session)?;
        let ice = ice_files(&config, FilesFor::Joiner, &spec.common.host_peer_id)?;
        let folder = Folder(Some(create_session_directory(
            &self.app_local_data,
            &session,
            generation,
        )?));
        let conf = format!(
            "{}local_udp_port=0\nforce_relay={}\n{}",
            spec.common.to_config_lines()?,
            bool_text(spec.common.force_relay),
            ice.config
        );
        let [config_path, secrets, _capability, ticket] = Self::write_files(
            folder.path(),
            [
                conf.as_bytes(),
                ice.secrets.expose().as_bytes(),
                b"",
                spec.ticket.expose().as_bytes(),
            ],
        )?;
        let handle = self.transport.join_open(&JoinOpen {
            public_key: self.public_key(),
            config: config_path,
            ticket,
            secrets,
        })?;
        let ready = self.join_port(handle, &spec, &account).and_then(|port| {
            self.backend.route_ready(&account, &session, generation, port)?;
            let pump = if self.backend.supports_signalling() {
                Some(start_pump(PumpWork {
                    transport: self.transport.clone(),
                    backend: self.backend.clone(),
                    handle,
                    account: account.clone(),
                    session: session.clone(),
                    generation,
                    roster: None,
                })?)
            } else {
                None
            };
            Ok((port, pump))
        });
        let (local_port, pump) = match ready {
            Ok(ready) => ready,
            Err(error) => {
                self.stop_transport(handle);
                return Err(error);
            }
        };
        let local = LocalSession {
            session: session.clone(),
            generation,
            role: spec.role,
            hosting: false,
            state: LocalState::Running,
        };
        let recorded = Recorded {
            local: local.clone(),
            account,
            handle,
            directory: folder.keep(),
            pid: None,
            environment: BTreeMap::new(),
            local_port,
            pump,
        };
        locked(&self.sessions).insert(session, recorded);
        Ok(local)
    }

    /// The transport's answer to the stop is returned after everything else is torn down.
    fn finish(&self, mut recorded: Recorded) -> Result<(), HostError> {
        if let Some(pump) = &recorded.pump {
            pump.stop.store(true, Ordering::SeqCst);
        }
        let stopped = self.transport.stop_session(recorded.handle);
        if let Some(thread) = recorded.pump.as_mut().and_then(|p| p.thread.take()) {
            let _ = thread.join();
        }
        if let Some(pid) = recorded.pid {
            self.spawner.stop(pid);
        }
        cleanup_directory(&recorded.directory);
        stopped
    }

    fn stop_now(&self, session: &SessionId) -> Result<(), HostError> {
        let recorded = locked(&self.sessions)
            .remove(session)
            .ok_or_else(|| HostError::NotFound(format!("session {session} is not running")))?;
        self.finish(recorded)
    }

    fn stop_hub_now(&self, account: &AccountId, kind: HubKind) -> Result<(), HostError> {
        let named = self.backend.stop_hub(account, kind)?;
        let recorded = {
            let mut sessions = locked(&self.sessions);
            let chosen = named.filter(|s| sessions.contains_key(s)).or_else(|| {
                sessions
                    .iter()
                    .find(|(_, r)| &r.account == account && r.local.role == HostedRole::Hub(kind))
                    .map(|(s, _)| s.clone())
            });
            let Some(chosen) = chosen else {
                return Err(HostError::NotFound("no running hub session for that kind".to_owned()));
            };
            sessions
                .remove(&chosen)
                .ok_or_else(|| HostError::NotFound("hub session already stopped".to_owned()))?
        };
        self.finish(recorded)
    }

    fn sessions_now(&self) -> Vec<LocalSession> {
        let mut sessions = locked(&self.sessions);
        for recorded in sessions.values_mut() {
            let live = matches!(
                recorded.local.state,
                LocalState::Starting | LocalState::Ready | LocalState::Running
            );
            if live && !matches!(self.transport.session_running(recorded.handle), Ok(true)) {
                let result = self
                    .transport
                    .last_session_result(recorded.handle)
                    .unwrap_or(SessionResult::Clean);
                recorded.local.state = LocalState::Ended(result);
                cleanup_directory(&recorded.directory);
            }
        }
        sessions.values().map(|r| r.local.clone()).collect()
    }
}

type ReadyResult = Result<ReadyState, HostError>;

fn ready<T: Send + 'static>(value: T) -> BoxFuture<'static, T> {
    Box::pin(std::future::ready(value))
}

impl RealmHosting for LocalRealmHosting {
    fn coop_available(&self) -> BoxFuture<'_, Result<bool, HostError>> {
        ready(Ok(self.coop_now()))
    }

    fn host_hunt(&self, account: AccountId, session: SessionId) -> BoxFuture<'_, Result<LocalSession, HostError>> {
        ready(
            self.backend
                .host_bootstrap(&account, &session)
                .and_then(|(spec, config)| self.start_host(account, spec, config)),
        )
    }

    fn join_hunt(&self, account: AccountId, session: SessionId) -> BoxFuture<'_, Result<LocalSession, HostError>> {
        ready(
            self.backend
                .join_bootstrap(&account, &session)
                .and_then(|(spec, config)| self.start_join(account, spec, config)),
        )
    }

    fn start_hub(
        &self,
        account: AccountId,
        kind: HubKind,
        limits: HubLimits,
    ) -> BoxFuture<'_, Result<LocalSession, HostError>> {
        ready(
            self.backend
                .start_hub(&account, kind, &limits)
                .and_then(|(spec, config)| self.start_host(account, spec, config)),
        )
    }

    fn stop_hub(&self, account: AccountId, kind: HubKind) -> BoxFuture<'_, Result<(), HostError>> {
        ready(self.stop_hub_now(&account, kind))
    }

    fn stop(&self, session: SessionId) -> BoxFuture<'_, Result<(), HostError>> {
        ready(self.stop_now(&session))
    }

    fn sessions(&self) -> BoxFuture<'_, Result<Vec<LocalSession>, HostError>> {
        ready(Ok(self.sessions_now()))
    }
}

impl Drop for LocalRealmHosting {
    fn drop(&mut self) {
        let all = std::mem::take(&mut *locked(&self.sessions));
        for recorded in all.into_values() {
            let _ = self.finish(recorded);
        }
    }
}
