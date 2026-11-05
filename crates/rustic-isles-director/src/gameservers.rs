// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The game servers (spec §5, §7): the registry, starting one, exits and clean-up, the hubs, the Training
//! Grounds on demand, the watchdog, start-up and shutdown.

use std::io::Write as _;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;

use futures_util::FutureExt;
use serde_json::{Map, Value};
use tokio::time::Instant;

use crate::config::Config;
use crate::hub::{self, HubSupervisor, Level, monotonic_ms};
use crate::hunt::{HuntResolver, MissingHuntRegistry, ResolverOptions, Tables};
use crate::launcher::{Command, Launcher};
use crate::memory::MemoryAdmission;
use crate::occupancy::{IdleStopPolicy, RuntimeLogOccupancy};
use crate::port_pool::PortPool;
use crate::process::{self, Labels, Proc};

pub const RAMSGATE_MAP: &str = "/Game/Maps/ramsgate/ramsgate_01_persistent";
pub const TRAINING_DOJO_MAP: &str = "/Game/Maps/islands/dojo/training_dojo_persistent";
pub const TRAINING_DOJO_HUNT_ID: &str = "ShatteredIsles_TrainingDojo";

const WATCHDOG: Duration = Duration::from_secs(60);
const HUB_QUEUE_CAP: Duration = Duration::from_secs(60);
const IDLE_ABANDON_MS: u64 = 120_000;
const ON_DEMAND_WAIT: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerKind {
    Ramsgate,
    Training,
    Hunt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerStatus {
    Starting,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartErrorKind {
    NoCapacity,
    Memory,
    Startup,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartError {
    pub kind: StartErrorKind,
    pub message: String,
}

impl StartError {
    fn other(message: impl Into<String>) -> Self {
        Self {
            kind: StartErrorKind::Other,
            message: message.into(),
        }
    }

    fn startup(message: impl Into<String>) -> Self {
        Self {
            kind: StartErrorKind::Startup,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// An expected roster entry: the id as sent, and the player hunt id it came for.
pub type RosterEntry = (Value, String);

struct ServerState {
    status: ServerStatus,
    ready_at_ms: Option<i64>,
    ready_mono: Option<u64>,
    roster: Option<Vec<RosterEntry>>,
}

pub struct GameServer {
    pub launch_id: String,
    pub port: u16,
    pub map: String,
    pub behemoth: Option<String>,
    pub matchmaker_hunt_id: Option<String>,
    pub kind: ServerKind,
    pub label: String,
    pub log_path: PathBuf,
    pub started_at_ms: i64,
    proc: Arc<Proc>,
    state: Mutex<ServerState>,
}

impl PartialEq for GameServer {
    fn eq(&self, other: &Self) -> bool {
        self.launch_id == other.launch_id
    }
}

impl std::fmt::Debug for GameServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GameServer")
            .field("label", &self.label)
            .field("port", &self.port)
            .field("pid", &self.proc.pid)
            .finish_non_exhaustive()
    }
}

impl GameServer {
    fn state(&self) -> MutexGuard<'_, ServerState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn pid(&self) -> Option<u32> {
        self.proc.pid
    }

    pub fn alive(&self) -> bool {
        self.proc.alive()
    }

    pub fn status(&self) -> ServerStatus {
        self.state().status
    }

    pub fn roster(&self) -> Option<Vec<RosterEntry>> {
        self.state().roster.clone()
    }

    fn ready_and_alive(&self) -> bool {
        self.status() == ServerStatus::Ready && self.alive()
    }

    pub fn is_hunt(&self) -> bool {
        self.kind == ServerKind::Hunt
    }

    /// The process tree (tests stop a server alone with it).
    pub fn terminate(&self) -> std::io::Result<()> {
        self.proc.tree.terminate()
    }

    fn pid_text(&self) -> String {
        self.proc.pid.map_or_else(|| "undefined".to_owned(), |p| p.to_string())
    }
}

/// What to start.
pub(crate) struct ServerSpec {
    pub kind: ServerKind,
    pub map: String,
    pub behemoth: Option<String>,
    pub matchmaker_hunt_id: Option<String>,
    pub roster: Option<Vec<RosterEntry>>,
}

impl ServerSpec {
    fn hub(kind: ServerKind) -> Self {
        let map = if kind == ServerKind::Ramsgate {
            RAMSGATE_MAP
        } else {
            TRAINING_DOJO_MAP
        };
        Self {
            kind,
            map: map.to_owned(),
            behemoth: None,
            matchmaker_hunt_id: None,
            roster: None,
        }
    }
}

/// `host` (none when `MY_IP` is unset) and port.
pub type Details = (Option<String>, u16);

struct Inner {
    servers: Vec<Arc<GameServer>>,
    pool: PortPool,
    procs: Vec<Arc<Proc>>,
    queue_tail: Instant,
    /// Hunts past the capacity check and not yet listed
    launching: usize,
}

struct IdleStop {
    server: Arc<GameServer>,
    sent_at: u64,
}

struct Training {
    wanted: bool,
    requested_once: bool,
    last_request: Option<u64>,
    idle_stop: Option<IdleStop>,
    policy: IdleStopPolicy,
    occupancy: Option<RuntimeLogOccupancy>,
}

pub type Supervisor = HubSupervisor<Arc<GameServer>>;

pub struct Director {
    config: Config,
    resolver: HuntResolver,
    memory: MemoryAdmission,
    launcher: Arc<dyn Launcher>,
    missing: Arc<MissingHuntRegistry>,
    inner: Mutex<Inner>,
    training_state: Mutex<Training>,
    stopping: AtomicBool,
    hubs_ready: AtomicBool,
    hub_gate: tokio::sync::Mutex<()>,
    ramsgate: Arc<Supervisor>,
    training: Arc<Supervisor>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn js_number_text(n: f64) -> String {
    if n.is_nan() {
        "NaN".to_owned()
    } else {
        service_kit::jsjson::number(n)
    }
}

/// The Behemoth path's last `/` segment cut at its first `.` (`fulgar_bp`), else `hunt`.
pub fn label_for(kind: ServerKind, behemoth: Option<&str>) -> String {
    match kind {
        ServerKind::Ramsgate => "ramsgate".to_owned(),
        ServerKind::Training => "training_dojo".to_owned(),
        ServerKind::Hunt => behemoth
            .map(|path| path.rsplit('/').next().unwrap_or(path))
            .map(|last| last.split('.').next().unwrap_or(last))
            .filter(|label| !label.is_empty())
            .unwrap_or("hunt")
            .to_owned(),
    }
}

/// Argument 6: `{id}:{playerHuntId}` joined with `,`, or `NO_EXPECTED_PLAYERS` without a roster.
pub fn roster_argument(roster: Option<&[RosterEntry]>) -> String {
    match roster {
        None => "NO_EXPECTED_PLAYERS".to_owned(),
        Some(roster) => roster
            .iter()
            .map(|(id, hunt)| format!("{}:{hunt}", service_kit::js::string(Some(id))))
            .collect::<Vec<_>>()
            .join(","),
    }
}

impl Director {
    pub fn new(config: Config, tables: Tables, memory: MemoryAdmission, launcher: Arc<dyn Launcher>) -> Arc<Director> {
        let missing = Arc::new(MissingHuntRegistry::default());
        let mut options = ResolverOptions::new(tables);
        options.trials_rotation_minutes = config.trials_rotation_minutes;
        let registry = Arc::clone(&missing);
        options.on_missing_hunt = Some(Arc::new(move |id: &str| registry.record(id, now_ms())));
        let idle_ms = config.training_idle_minutes * 60_000;
        let occupancy = config.runtime_log_dir.as_ref().map(|dir| {
            RuntimeLogOccupancy::new(dir.join(format!(
                "rustic_isles_dll_port{}.log",
                config.port_range.last.wrapping_sub(1)
            )))
        });
        Arc::new_cyclic(|me: &Weak<Director>| {
            let supervisor = |kind: ServerKind| {
                let weak = me.clone();
                let start: hub::StartFn<Arc<GameServer>> = Arc::new(move |reason: String| {
                    let weak = weak.clone();
                    Box::pin(async move {
                        let director = weak.upgrade().ok_or_else(|| "the Director is gone".to_owned())?;
                        director.hub_start(kind, &reason).await.map_err(|e| e.message)
                    })
                });
                let label = label_for(kind, None);
                let mut options = hub::Options::new(&label, start, Arc::new(|s: &Arc<GameServer>| s.alive()));
                if let Some(backoff) = &config.hub_backoff_ms {
                    options.backoff_ms.clone_from(backoff);
                }
                HubSupervisor::new(options)
            };
            Director {
                resolver: HuntResolver::new(options),
                memory,
                launcher,
                missing,
                inner: Mutex::new(Inner {
                    servers: Vec::new(),
                    pool: PortPool::new(),
                    procs: Vec::new(),
                    queue_tail: Instant::now(),
                    launching: 0,
                }),
                training_state: Mutex::new(Training {
                    wanted: false,
                    requested_once: false,
                    last_request: None,
                    idle_stop: None,
                    policy: IdleStopPolicy::new(idle_ms),
                    occupancy,
                }),
                stopping: AtomicBool::new(false),
                hubs_ready: AtomicBool::new(false),
                hub_gate: tokio::sync::Mutex::new(()),
                ramsgate: supervisor(ServerKind::Ramsgate),
                training: supervisor(ServerKind::Training),
                config,
            }
        })
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn resolver(&self) -> &HuntResolver {
        &self.resolver
    }

    pub fn ramsgate(&self) -> &Arc<Supervisor> {
        &self.ramsgate
    }

    pub fn training(&self) -> &Arc<Supervisor> {
        &self.training
    }

    pub fn memory(&self) -> &MemoryAdmission {
        &self.memory
    }

    pub fn is_stopping(&self) -> bool {
        self.stopping.load(Ordering::SeqCst)
    }

    pub fn persistent_hubs_ready(&self) -> bool {
        self.hubs_ready.load(Ordering::SeqCst)
    }

    pub fn set_persistent_hubs_ready(&self, ready: bool) {
        self.hubs_ready.store(ready, Ordering::SeqCst);
    }

    /// Every listed server, in start order.
    pub fn gameservers(&self) -> Vec<Arc<GameServer>> {
        lock(&self.inner).servers.clone()
    }

    pub fn missing_hunt_requests(&self) -> Value {
        self.missing.to_json()
    }

    /// Free and held hunt ports.
    pub fn port_counts(&self) -> (usize, usize) {
        let inner = lock(&self.inner);
        (inner.pool.free_count(), inner.pool.held_count())
    }

    fn supervisor(&self, kind: ServerKind) -> Option<&Arc<Supervisor>> {
        match kind {
            ServerKind::Ramsgate => Some(&self.ramsgate),
            ServerKind::Training => Some(&self.training),
            ServerKind::Hunt => None,
        }
    }

    /// Logs the table audits (§9.7).
    pub fn validate_hunt_tables(&self) {
        for (level, line) in self.resolver.audit_lines() {
            match level {
                Level::Info => tracing::info!("{line}"),
                Level::Warn => tracing::warn!("{line}"),
                Level::Error => tracing::error!("{line}"),
            }
        }
    }

    fn remove(&self, server: &GameServer) -> bool {
        let mut inner = lock(&self.inner);
        let before = inner.servers.len();
        inner.servers.retain(|s| **s != *server);
        before != inner.servers.len()
    }

    fn release_lease(&self, server: &GameServer) {
        lock(&self.inner).pool.release(server.port, &server.launch_id);
    }

    async fn wait_hunt_slot(&self) {
        let slot = {
            let mut inner = lock(&self.inner);
            let slot = inner.queue_tail.max(Instant::now());
            inner.queue_tail = slot + self.config.stagger;
            slot
        };
        tokio::time::sleep_until(slot).await;
    }

    /// Starts one game server and waits for its readiness marker (§5.2).
    pub(crate) async fn start_server(self: &Arc<Self>, spec: ServerSpec) -> Result<Arc<GameServer>, StartError> {
        if self.is_stopping() {
            return Err(StartError::other("the Director is stopping"));
        }
        let hunt = spec.kind == ServerKind::Hunt;
        if hunt {
            self.wait_hunt_slot().await;
        }
        let launch_id = rustic_isles::replay::random_uuid();
        let range = self.config.port_range;
        let mut reservation = None;
        let port = match spec.kind {
            ServerKind::Ramsgate => range.last,
            ServerKind::Training => range.last.wrapping_sub(1),
            ServerKind::Hunt => {
                {
                    let mut inner = lock(&self.inner);
                    let count = inner.servers.iter().filter(|s| s.is_hunt()).count() + inner.launching;
                    // Compared as numbers, so NaN never refuses (§16 item 7)
                    if count as f64 >= self.config.max_hunts {
                        return Err(StartError {
                            kind: StartErrorKind::NoCapacity,
                            message: format!(
                                "Hunt server capacity reached ({count}/{})",
                                js_number_text(self.config.max_hunts)
                            ),
                        });
                    }
                    inner.launching += 1;
                }
                let undo_launching = || lock(&self.inner).launching -= 1;
                match self.memory.reserve() {
                    Ok(id) => reservation = id,
                    Err(refused) => {
                        undo_launching();
                        let count = self.gameservers().iter().filter(|s| s.is_hunt()).count();
                        tracing::warn!("[MemoryGuard] refusing a new hunt ({count} running): {refused}");
                        return Err(StartError {
                            kind: StartErrorKind::Memory,
                            message: refused.reason,
                        });
                    }
                }
                let taken = lock(&self.inner).pool.take(&launch_id);
                match taken {
                    Some(port) => port,
                    None => {
                        undo_launching();
                        if let Some(id) = reservation {
                            self.memory.give_back(id);
                        }
                        return Err(StartError::other("No free ports left!"));
                    }
                }
            }
        };
        let undo = |message: String| {
            if hunt {
                let mut inner = lock(&self.inner);
                inner.launching -= 1;
                inner.pool.release(port, &launch_id);
            }
            if let Some(id) = reservation {
                self.memory.give_back(id);
            }
            StartError::other(message)
        };
        let label = label_for(spec.kind, spec.behemoth.as_deref());
        let folder = &self.config.log_dir;
        if let Err(e) = std::fs::create_dir_all(folder) {
            return Err(undo(format!("{}: {e}", folder.display())));
        }
        let log_path = folder.join(format!("port{port}_{label}.log"));
        let mut file = match std::fs::OpenOptions::new().create(true).append(true).open(&log_path) {
            Ok(file) => file,
            Err(e) => return Err(undo(format!("{}: {e}", log_path.display()))),
        };
        let players = roster_argument(spec.roster.as_deref());
        let header = format!(
            "=== Gameserver launch {} (local time) ===\nport={port} map={} behemoth={} mmHunt={} players={players}\n\n",
            chrono::Local::now().format("%-m/%-d/%Y, %-I:%M:%S %p"),
            spec.map,
            spec.behemoth.as_deref().unwrap_or("NO_BEHEMOTH"),
            spec.matchmaker_hunt_id.as_deref().unwrap_or("NO_MM_HUNTID"),
        );
        let _ = file.write_all(header.as_bytes());
        let (Some(program), Some(key)) = (self.config.binary_path.clone(), self.config.api_key()) else {
            return Err(undo(
                "GAMESERVER_BINARY_PATH and METAGAME_API_KEY must be set to start a game server".to_owned(),
            ));
        };
        let mut args = vec![
            key.to_owned(),
            port.to_string(),
            spec.map.clone(),
            spec.behemoth.clone().unwrap_or_else(|| "NO_BEHEMOTH".to_owned()),
            spec.matchmaker_hunt_id
                .clone()
                .unwrap_or_else(|| "NO_MM_HUNTID".to_owned()),
            players,
            format!("{}:{port}", self.config.my_ip_text()),
        ];
        args.extend(self.config.launch_args.iter().cloned());
        let command = Command {
            program: program.clone(),
            args,
            env: self.config.child_env(&launch_id, spec.matchmaker_hunt_id.as_deref()),
            windows_hide: self.config.windows_hide,
        };
        if spec.kind == ServerKind::Training {
            // Just before the process starts: what the log holds now is an earlier process's (§7.4)
            if let Some(occupancy) = lock(&self.training_state).occupancy.as_mut() {
                occupancy.begin();
            }
        }
        let labels = Labels {
            port,
            label: label.clone(),
            launch_id: launch_id.clone(),
        };
        let spawned = match process::launch(self.launcher.as_ref(), &command, file, labels) {
            Ok(spawned) => spawned,
            Err(e) => return Err(undo(format!("spawn {program} {e}"))),
        };
        drop(command);
        let proc = spawned.proc;
        let pid_text = proc.pid.map_or_else(|| "unknown".to_owned(), |p| p.to_string());
        tracing::info!(
            "{label} spawned on port {port} (pid={pid_text}) -> {}",
            log_path.display()
        );
        let server = Arc::new(GameServer {
            launch_id: launch_id.clone(),
            port,
            map: spec.map,
            behemoth: spec.behemoth,
            matchmaker_hunt_id: spec.matchmaker_hunt_id,
            kind: spec.kind,
            label: label.clone(),
            log_path,
            started_at_ms: now_ms(),
            proc: Arc::clone(&proc),
            state: Mutex::new(ServerState {
                status: ServerStatus::Starting,
                ready_at_ms: None,
                ready_mono: None,
                roster: spec.roster,
            }),
        });
        {
            let mut inner = lock(&self.inner);
            if hunt {
                inner.launching -= 1;
                if let Some(pid) = proc.pid {
                    inner.pool.attach_pid(port, &launch_id, pid);
                }
            }
            inner.servers.push(Arc::clone(&server));
            inner.procs.push(Arc::clone(&proc));
        }
        let weak = Arc::downgrade(self);
        let watched = Arc::clone(&server);
        tokio::spawn(async move {
            let exit = watched.proc.exited().await;
            if let Some(director) = weak.upgrade() {
                director.on_process_exit(&watched, &exit);
            }
        });

        let mut ready = spawned.ready;
        let started = Instant::now();
        let waited = tokio::time::timeout(self.config.ready_timeout, async {
            tokio::select! {
                biased;
                signal = &mut ready => match signal {
                    Ok(()) => None,
                    Err(_) => Some(proc.exited().await),
                },
                exit = proc.exited() => Some(exit),
            }
        })
        .await;
        let failure = match waited {
            Ok(None) => {
                {
                    let mut state = server.state();
                    state.status = ServerStatus::Ready;
                    state.ready_at_ms = Some(now_ms());
                    state.ready_mono = Some(monotonic_ms());
                }
                tracing::info!(
                    "{label} ready on {}:{port} after {}ms",
                    self.config.my_ip_text(),
                    started.elapsed().as_millis()
                );
                return Ok(server);
            }
            Ok(Some(exit)) => format!(
                "{label} port {port} exited before ready (code={}, signal={})",
                exit.code_text(),
                exit.signal_text()
            ),
            Err(_) => format!(
                "{label} port {port} did not report ready within {}ms",
                js_number_text(self.config.ready_timeout_setting)
            ),
        };
        server.state().status = ServerStatus::Failed;
        self.remove(&server);
        if let Some(id) = reservation {
            self.memory.give_back(id);
        }
        if server.alive() {
            // Its port comes back with its exit
            let director = Arc::clone(self);
            let proc = Arc::clone(&proc);
            tokio::spawn(async move {
                let _ = director.stop_proc(&proc).await;
            });
        } else if hunt {
            self.release_lease(&server);
        }
        Err(StartError::startup(failure))
    }

    /// Asks the tree to stop, and kills it if the leader outlives the grace (§5.7).
    async fn stop_proc(&self, proc: &Proc) -> Result<(), String> {
        if !proc.alive() {
            return Err("the process is gone".to_owned());
        }
        proc.tree.terminate().map_err(|e| e.to_string())?;
        if tokio::time::timeout(self.config.stop_grace, proc.exited())
            .await
            .is_err()
            && proc.alive()
        {
            let pid = proc.pid.map_or_else(|| "undefined".to_owned(), |p| p.to_string());
            tracing::warn!("a game server (pid={pid}) ignored its stop; killing its process tree");
            proc.tree.kill();
        }
        Ok(())
    }

    fn on_process_exit(self: &Arc<Self>, server: &Arc<GameServer>, exit: &crate::launcher::Exit) {
        if server.is_hunt() {
            self.remove(server);
            self.release_lease(server);
        } else if self.config.hub_prompt_restart {
            let reason = format!("exit code={} signal={}", exit.code_text(), exit.signal_text());
            self.cleanup(server, &reason);
        }
    }

    /// Idempotent: the exit, the getters and the watchdog may all report the same server (§5.11).
    fn cleanup(self: &Arc<Self>, server: &Arc<GameServer>, reason: &str) {
        let listed = self.remove(server);
        if self.is_stopping() {
            return;
        }
        if server.kind == ServerKind::Training {
            let mut training = lock(&self.training_state);
            if training.idle_stop.as_ref().is_some_and(|stop| stop.server == *server) {
                training.idle_stop = None;
                training.policy.reset();
                if self.training.release(server, "idle stop") {
                    training.wanted = false;
                    tracing::info!(
                        "[TrainingOnDemand] the Training Grounds stopped (no player for {} min); they start again on \
                         the next request",
                        self.config.training_idle_minutes
                    );
                }
                return;
            }
        }
        let Some(supervisor) = self.supervisor(server.kind) else {
            self.release_lease(server);
            return;
        };
        if supervisor.current().as_ref() == Some(server) {
            if server.kind == ServerKind::Ramsgate {
                tracing::warn!("RAMSGATE HAS FALLEN ({reason})! Restarting!");
            } else {
                tracing::warn!("Training Dojo down ({reason})! Restarting!");
            }
            let supervisor = Arc::clone(supervisor);
            let server = Arc::clone(server);
            let reason = reason.to_owned();
            tokio::spawn(async move {
                supervisor.on_exit(&server, &reason).await;
            });
        } else if listed {
            tracing::info!(
                "[HubSupervisor] {} pid={} exited during startup ({reason}); its start attempt handles the retry",
                server.label,
                server.pid_text()
            );
        }
    }

    /// Both hubs' cold starts go through one gate: a concurrent Ramsgate and Training start crashed Ramsgate's
    /// async loading thread (§7.2).
    async fn hub_start(self: &Arc<Self>, kind: ServerKind, reason: &str) -> Result<Arc<GameServer>, StartError> {
        let _gate = self.hub_gate.lock().await;
        let label = label_for(kind, None);
        if reason != "boot" {
            let tail = lock(&self.inner).queue_tail;
            let began = Instant::now();
            let capped = tail.saturating_duration_since(began) > HUB_QUEUE_CAP;
            tokio::time::sleep_until(tail.min(began + HUB_QUEUE_CAP)).await;
            let waited = began.elapsed().as_millis();
            if capped {
                tracing::warn!(
                    "[HubSupervisor] {label} hunt launch queue still busy after {waited} ms - cold-starting anyway"
                );
            } else if waited >= 100 {
                tracing::info!(
                    "[HubSupervisor] {label} waited {waited} ms for the hunt launch queue before cold start"
                );
            }
        }
        if kind == ServerKind::Training {
            lock(&self.training_state).policy.reset();
        }
        self.start_server(ServerSpec::hub(kind)).await
    }

    fn live_hub(&self, kind: ServerKind) -> Option<Arc<GameServer>> {
        self.supervisor(kind)?.current().filter(|s| s.ready_and_alive())
    }

    fn details(&self, server: &GameServer) -> Details {
        (self.config.my_ip.clone(), server.port)
    }

    /// A hub's connection details (§5.9).
    pub(crate) fn hub_details(self: &Arc<Self>, kind: ServerKind) -> Result<Details, StartError> {
        if kind == ServerKind::Ramsgate && !self.config.ramsgate_hub {
            return Err(StartError::other(
                "the dedicated Ramsgate hub is switched off (RAMSGATE_HUB=off)",
            ));
        }
        if let Some(server) = self.live_hub(kind) {
            return Ok(self.details(&server));
        }
        if self.config.hub_prompt_restart
            && let Some(supervisor) = self.supervisor(kind)
        {
            match supervisor.current() {
                Some(current) if !current.alive() => self.cleanup(&current, "getter: process gone"),
                Some(_) => {}
                None => supervisor.ensure_in_background("request"),
            }
        }
        Err(StartError::startup(if kind == ServerKind::Ramsgate {
            "Ramsgate is not ready"
        } else {
            "Training Dojo is not ready"
        }))
    }

    /// The Training Grounds for a SHARED request (§7.3).
    pub(crate) async fn training_for_request(self: &Arc<Self>) -> Result<Details, StartError> {
        if !self.config.training_hub {
            lock(&self.training_state).last_request = Some(monotonic_ms());
            let began = Instant::now();
            while lock(&self.training_state).idle_stop.is_some() && began.elapsed() < ON_DEMAND_WAIT {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            {
                let mut training = lock(&self.training_state);
                training.wanted = true;
                if !training.requested_once {
                    training.requested_once = true;
                    tracing::info!(
                        "[TrainingHub] on-demand Training hub requested for the first time (TRAINING_HUB=off) - \
                         starting it"
                    );
                }
            }
            if self.live_hub(ServerKind::Training).is_none() {
                self.training.ensure("request").await;
            }
        }
        self.hub_details(ServerKind::Training)
    }

    /// Public hunt reuse (§5.10): keyed on the player hunt id, not the matchmaker row, which is picked at random and
    /// would split a party.
    pub(crate) fn reuse_public_hunt(&self, hunt_id: &str, expected: &Value) -> Option<Details> {
        if !self.config.public_reuse || service_kit::js::trim(hunt_id).is_empty() {
            return None;
        }
        let max = self.config.public_max_players;
        let servers = self.gameservers();
        let server = servers.iter().find(|s| {
            let state = s.state();
            s.is_hunt()
                && state.status == ServerStatus::Ready
                && state
                    .roster
                    .as_ref()
                    .is_some_and(|r| r.first().is_some_and(|(_, h)| h == hunt_id) && (r.len() as f64) < max)
        })?;
        let joiners: Vec<Value> = match expected {
            Value::Array(items) => items.clone(),
            Value::String(text) => text
                .encode_utf16()
                .map(|unit| Value::String(String::from_utf16_lossy(&[unit])))
                .collect(),
            _ => Vec::new(),
        };
        let count = {
            let mut state = server.state();
            let roster = state.roster.get_or_insert_with(Vec::new);
            for joiner in joiners {
                if !roster.iter().any(|(id, _)| *id == joiner) {
                    roster.push((joiner, hunt_id.to_owned()));
                }
            }
            roster.len()
        };
        tracing::info!(
            "[PublicHuntReuse] joining existing server port={} pid={} huntId={hunt_id} roster={count}/{}",
            server.port,
            server.pid_text(),
            js_number_text(max)
        );
        Some(self.details(server))
    }

    /// A hunt (§5.8).
    pub(crate) async fn start_hunt(
        self: &Arc<Self>,
        map: String,
        behemoth: Option<String>,
        matchmaker_hunt_id: Option<String>,
        roster: Option<Vec<RosterEntry>>,
    ) -> Result<Details, StartError> {
        let spec = ServerSpec {
            kind: ServerKind::Hunt,
            map,
            behemoth,
            matchmaker_hunt_id,
            roster,
        };
        let server = self.start_server(spec).await?;
        Ok(self.details(&server))
    }

    fn hub_json(&self, kind: ServerKind) -> Value {
        let Some(server) = self.live_hub(kind) else {
            return Value::Null;
        };
        let mut hub = Map::new();
        hub.insert("ready".into(), Value::Bool(true));
        if let Some(ip) = &self.config.my_ip {
            hub.insert("host".into(), Value::String(ip.clone()));
        }
        hub.insert("port".into(), Value::from(server.port));
        if let Some(pid) = server.pid() {
            hub.insert("processId".into(), Value::from(pid));
        }
        Value::Object(hub)
    }

    /// `{"ramsgate":..,"trainingDojo":..}`: a cached ready is no proof, so the process is checked now.
    pub fn persistent_hub_status(&self) -> Value {
        let mut out = Map::new();
        out.insert("ramsgate".into(), self.hub_json(ServerKind::Ramsgate));
        out.insert("trainingDojo".into(), self.hub_json(ServerKind::Training));
        Value::Object(out)
    }

    pub(crate) fn live_hub_details(&self, kind: ServerKind) -> Option<Details> {
        self.live_hub(kind).map(|s| self.details(&s))
    }

    /// Every listed server that is ready and alive, without pids, addresses or launch ids.
    pub fn instances_for_status(&self) -> Value {
        let servers = self.gameservers();
        let list = servers
            .iter()
            .filter(|s| s.ready_and_alive())
            .map(|s| {
                let state = s.state();
                let kind = match s.kind {
                    ServerKind::Ramsgate => "city",
                    ServerKind::Training => "dojo",
                    ServerKind::Hunt => "hunt",
                };
                let roster = state.roster.clone().unwrap_or_default();
                let max = if s.is_hunt() {
                    let max = self.config.public_max_players;
                    if max.is_finite() { Value::from(max) } else { Value::Null }
                } else {
                    Value::from(0)
                };
                let mut out = Map::new();
                out.insert("id".into(), Value::String(format!("{kind}-{}", s.port)));
                out.insert("kind".into(), Value::from(kind));
                out.insert("map".into(), Value::String(s.map.clone()));
                out.insert("behemoth".into(), s.behemoth.clone().map_or(Value::Null, Value::String));
                out.insert("players".into(), Value::from(roster.len()));
                out.insert("maxPlayers".into(), max);
                out.insert(
                    "playerIds".into(),
                    Value::Array(roster.into_iter().map(|(id, _)| id).collect()),
                );
                let started = state.ready_at_ms.unwrap_or(s.started_at_ms);
                out.insert(
                    "startedAt".into(),
                    Value::String(rustic_isles::season::to_iso_string(started)),
                );
                Value::Object(out)
            })
            .collect();
        Value::Array(list)
    }

    /// The boot, in the background once the listener is up (§5.11).
    pub async fn startup(self: &Arc<Self>) {
        self.validate_hunt_tables();
        let range = self.config.port_range;
        if range.last >= range.first.saturating_add(2) {
            let mut inner = lock(&self.inner);
            for port in range.first..=range.last - 2 {
                inner.pool.add(port);
            }
        }
        let mut failures = Vec::new();
        if self.config.ramsgate_hub {
            if self.ramsgate.ensure("boot").await.is_none() {
                failures.push("ramsgate");
            }
        } else {
            tracing::info!(
                "[RamsgateHub] RAMSGATE_HUB=off - no dedicated Ramsgate hub (realms run their own on the host's PC)"
            );
        }
        if self.config.training_hub {
            lock(&self.training_state).wanted = true;
            if self.training.ensure("boot").await.is_none() {
                failures.push("training_dojo");
            }
        } else {
            tracing::info!(
                "[TrainingHub] TRAINING_HUB=off - the dedicated Training hub starts on its first request (fallback \
                 for hosted Training)"
            );
        }
        if !failures.is_empty() {
            tracing::warn!(
                "Persistent hub startup reported failures ({}) - continuing with whichever hubs are up; retrying \
                 with backoff until up",
                failures.join("; ")
            );
        }
        self.set_persistent_hubs_ready(true);
    }

    /// One watchdog pass; a pass that panics is logged and the next still runs.
    pub async fn run_watchdog(self: &Arc<Self>) {
        let pass = AssertUnwindSafe(self.watchdog_pass()).catch_unwind().await;
        if pass.is_err() {
            tracing::error!("Gameserver watchdog pass failed");
        }
    }

    async fn watchdog_pass(self: &Arc<Self>) {
        if !self.config.can_launch() {
            return;
        }
        tracing::info!("Running Gameserver Watchdog!");
        for server in self.gameservers() {
            if !server.alive() {
                tracing::warn!("Cleaning up dead gameserver on port {}", server.port);
                self.cleanup(&server, "watchdog: process gone");
            }
        }
        {
            let mut inner = lock(&self.inner);
            let listed: Vec<String> = inner.servers.iter().map(|s| s.launch_id.clone()).collect();
            let procs = inner.procs.clone();
            let alive = |pid: u32| procs.iter().any(|p| p.pid == Some(pid) && p.alive());
            inner.pool.reclaim(|owner| listed.iter().any(|l| l == owner), alive);
        }
        if self.config.ramsgate_hub {
            self.ramsgate.tick().await;
        }
        if lock(&self.training_state).wanted {
            self.training.tick().await;
        }
        self.idle_stop_check();
        if let Some(summary) = self.missing.summary() {
            tracing::warn!("{summary}");
        }
        let mut inner = lock(&self.inner);
        let listed: Vec<Arc<Proc>> = inner.servers.iter().map(|s| Arc::clone(&s.proc)).collect();
        let leased: Vec<u32> = inner.pool.leases().iter().filter_map(|l| l.pid).collect();
        inner.procs.retain(|p| {
            p.alive() || listed.iter().any(|l| Arc::ptr_eq(l, p)) || p.pid.is_some_and(|pid| leased.contains(&pid))
        });
    }

    fn idle_stop_active(&self) -> bool {
        !self.config.training_hub && self.config.training_idle_minutes > 0 && self.config.runtime_log_dir.is_some()
    }

    fn idle_stop_check(self: &Arc<Self>) {
        if !self.idle_stop_active() {
            return;
        }
        let now = monotonic_ms();
        let mut training = lock(&self.training_state);
        if let Some(stop) = &training.idle_stop {
            if stop.server.alive() && now.saturating_sub(stop.sent_at) >= IDLE_ABANDON_MS {
                tracing::warn!(
                    "[TrainingOnDemand] the Training Grounds (pid={}) did not stop within 2 min; leaving them running",
                    stop.server.pid_text()
                );
                training.idle_stop = None;
                training.policy.reset();
            }
            return;
        }
        let current = self.training.current().filter(|s| s.ready_and_alive());
        let ready_at = current.as_ref().and_then(|s| s.state().ready_mono);
        let (Some(server), Some(ready_at), true) = (current, ready_at, training.wanted) else {
            training.policy.reset();
            return;
        };
        let count = training.occupancy.as_mut().and_then(RuntimeLogOccupancy::poll);
        let last_request = training.last_request;
        if !training.policy.should_stop(now, count, ready_at, last_request) {
            return;
        }
        tracing::info!(
            "[TrainingOnDemand] no player in the Training Grounds for {} min - stopping them (pid={})",
            self.config.training_idle_minutes,
            server.pid_text()
        );
        let asked = if server.alive() {
            server.proc.tree.terminate().map_err(|e| e.to_string())
        } else {
            Err("the process is gone".to_owned())
        };
        match asked {
            Ok(()) => {
                training.idle_stop = Some(IdleStop {
                    server: Arc::clone(&server),
                    sent_at: now,
                });
                let director = Arc::clone(self);
                tokio::spawn(async move {
                    if tokio::time::timeout(director.config.stop_grace, server.proc.exited())
                        .await
                        .is_err()
                    {
                        tracing::warn!(
                            "a game server (pid={}) ignored its stop; killing its process tree",
                            server.pid_text()
                        );
                        server.proc.tree.kill();
                    }
                });
            }
            Err(error) => tracing::warn!(error, "[TrainingOnDemand] could not stop the Training Grounds"),
        }
    }

    /// The service's stop: no restart, no new start, every tree asked to stop and killed after the grace.
    pub async fn shutdown(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.ramsgate.dispose();
        self.training.dispose();
        let servers = self.gameservers();
        for server in &servers {
            let _ = server.proc.tree.terminate();
        }
        let deadline = Instant::now() + self.config.stop_grace + Duration::from_secs(1);
        while servers.iter().any(|s| s.alive()) && Instant::now() < deadline {
            tokio::time::sleep(POLL).await;
        }
        for server in &servers {
            server.proc.tree.kill();
        }
        let killed = Instant::now() + Duration::from_secs(2);
        while servers.iter().any(|s| s.alive()) && Instant::now() < killed {
            tokio::time::sleep(POLL).await;
        }
        let mut inner = lock(&self.inner);
        inner.servers.clear();
        let leases: Vec<(u16, String)> = inner.pool.leases().iter().map(|l| (l.port, l.owner.clone())).collect();
        for (port, owner) in leases {
            inner.pool.release(port, &owner);
        }
    }

    /// A test that ends in a hurry: every tree killed at once.
    pub fn kill_all(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.ramsgate.dispose();
        self.training.dispose();
        let procs = lock(&self.inner).procs.clone();
        for proc in procs {
            proc.tree.kill();
        }
        let servers = self.gameservers();
        for server in servers {
            server.proc.tree.kill();
        }
    }

    pub(crate) fn watchdog_period() -> Duration {
        WATCHDOG
    }
}

impl Drop for Director {
    fn drop(&mut self) {
        // Dropping the trees ends them as well; this only makes it prompt
        let inner = self.inner.get_mut().unwrap_or_else(PoisonError::into_inner);
        for proc in &inner.procs {
            if proc.alive() {
                proc.tree.kill();
            }
        }
    }
}
