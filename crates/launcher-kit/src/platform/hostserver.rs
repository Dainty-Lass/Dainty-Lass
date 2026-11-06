// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Host server lifecycle management, embedded server trait, and CLI command runner.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::future::Future;
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::process::{ChildStdin, Command};
use tokio::sync::Mutex as AsyncMutex;

pub use crate::bridge::types::{HostComponentPhase, HostInvite, HostServerComponent, HostServerState, HostServerView};
use crate::platform::hostkit::parse::{clean_line, cut, failure_line};
use crate::platform::hostkit::{WorkspacePaths, find_host_binary};

/// Environment variable telling `dainty-host start` to accept control commands on stdin.
pub const CONTROL_ENV: &str = "DAINTY_CONTROL";

/// Output line emitted by `dainty-host start` when all server listeners are ready.
pub const SERVER_READY_LINE: &str = "DAINTY_SERVER_READY";

/// Default local TCP port for the backend service.
pub const BACKEND_PORT: u16 = 61000;

/// Maximum number of log lines retained in the view ring buffer.
pub const MAX_LINES: usize = 300;

/// Alias for the maximum log line count in the view ring buffer.
pub const MAX_LOG_LINES: usize = MAX_LINES;

/// Graceful stop wait duration in milliseconds before force-killing a child tree.
pub const STOP_GRACE_MS: u64 = 5000;

/// Maximum startup wait window in milliseconds.
pub const STARTUP_DEADLINE_MS: u64 = 30_000;

/// Startup readiness poll interval in milliseconds.
pub const STARTUP_POLL_MS: u64 = 250;

/// Graceful stop timeout before terminating the child process tree.
pub const STOP_TIMEOUT: Duration = Duration::from_secs(45);
const KILL_GRACE: Duration = Duration::from_secs(5);
const PROBE_TIMEOUT: Duration = Duration::from_millis(700);
const CHANGE_EVERY: Duration = Duration::from_millis(250);

/// Alias for [`HostServerState`].
pub type ServerState = HostServerState;

/// Alias for [`HostServerComponent`].
pub type ServerModule = HostServerComponent;

/// Alias for [`HostServerView`].
pub type ServerView = HostServerView;

/// Result returned when starting a host server instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartServerOutput {
    /// The server panel after starting.
    pub view: HostServerView,
    /// The started process's id; none for the embedded server.
    pub pid: Option<u32>,
}

/// Asynchronous port/liveness probe function.
pub type Probe = Arc<dyn Fn() -> Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync>;

/// Callback notified whenever [`HostServerView`] changes.
pub type Listener = Arc<dyn Fn(HostServerView) + Send + Sync>;

/// Callback receiving formatted log lines from an embedded server.
pub type LineSink = Arc<dyn Fn(String) + Send + Sync>;

/// Trait implemented by an in-process embedded server controller.
pub trait EmbeddedServer: Send + Sync {
    /// Starts every service from `workspace`'s configuration, sending each console line to `lines`.
    fn start(&self, workspace: PathBuf, lines: LineSink) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>>;
    /// Stops every service; returns once they are down.
    fn stop(&self) -> Pin<Box<dyn Future<Output = ()> + Send>>;
    /// Restarts the service called `name`; fails on an unknown name.
    fn restart(&self, name: String) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>>;
    /// Each service's state.
    fn components(&self) -> Vec<HostServerComponent>;
    /// The module panel, when the server has a module registry.
    fn modules(&self) -> Option<Arc<dyn crate::bridge::modules::ModulePanel>> {
        None
    }
}

/// Resolved command invocation for `dainty-host`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostTool {
    /// The program to run.
    pub program: PathBuf,
    /// Arguments placed before the subcommand.
    pub prefix: Vec<OsString>,
}

impl HostTool {
    /// Locates the `dainty-host` executable inside `workspace`.
    pub fn find(workspace: &Path) -> Option<Self> {
        find_host_binary(workspace).map(|program| Self {
            program,
            prefix: Vec::new(),
        })
    }
}

/// Error returned when spawning or invoking the host tool fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostKitError {
    /// No `dainty-host` was found in the workspace.
    Missing,
    /// The process could not be started or waited on.
    Spawn(String),
    /// Another server operation is in progress.
    Busy,
    /// The server is already running.
    AlreadyRunning,
}

impl HostKitError {
    /// Returns a human-readable detail string for error reporting.
    pub fn detail(&self) -> String {
        match self {
            Self::Missing => "hostkit_missing".to_string(),
            Self::Spawn(err) => format!("hostkit_start:{}", cut(err, 200)),
            Self::Busy => "server_busy".to_string(),
            Self::AlreadyRunning => "already_running".to_string(),
        }
    }
}

/// Captured result of a one-shot `dainty-host` CLI subcommand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    /// The process's exit code, -1 when it had none.
    pub code: i32,
    /// Every cleaned output line.
    pub lines: Vec<String>,
    /// The line explaining a failure, when the exit code was not 0.
    pub failure: Option<String>,
}

struct ChildControl {
    pid: u32,
    stdin: Arc<AsyncMutex<Option<ChildStdin>>>,
    done: tokio::sync::watch::Receiver<bool>,
}

struct Inner {
    state: HostServerState,
    lines: VecDeque<String>,
    invite: Option<HostInvite>,
    message: Option<String>,
    listener: Option<Listener>,
    embedded: Option<Arc<dyn EmbeddedServer>>,
    embedded_running: bool,
    stop_requested: bool,
    child: Option<ChildControl>,
    last_emit: Option<Instant>,
}

impl Inner {
    fn push_line(&mut self, cleaned: String) {
        if cleaned.is_empty() {
            return;
        }
        if self.lines.len() >= MAX_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(cleaned);
    }

    fn view(&self) -> HostServerView {
        let components = match (&self.embedded, self.embedded_running) {
            (Some(server), true) => server.components(),
            _ => Vec::new(),
        };
        HostServerView {
            state: self.state,
            message: self.message.clone(),
            lines: self.lines.iter().cloned().collect(),
            invite: self.invite.clone(),
            embedded: self.embedded.is_some(),
            components,
        }
    }
}

/// Manages the lifecycle and log stream of the local host server.
#[derive(Clone)]
pub struct HostServer {
    probe: Probe,
    stop_timeout: Duration,
    inner: Arc<Mutex<Inner>>,
}

impl Default for HostServer {
    fn default() -> Self {
        let probe: Probe = Arc::new(|| Box::pin(port_answers(BACKEND_PORT)));
        Self::new(probe, STOP_TIMEOUT)
    }
}

impl HostServer {
    /// Creates a new [`HostServer`] with a custom liveness probe and graceful stop timeout.
    pub fn new(probe: Probe, stop_timeout: Duration) -> Self {
        Self {
            probe,
            stop_timeout,
            inner: Arc::new(Mutex::new(Inner {
                state: HostServerState::Stopped,
                lines: VecDeque::with_capacity(MAX_LINES),
                invite: None,
                message: None,
                listener: None,
                embedded: None,
                embedded_running: false,
                stop_requested: false,
                child: None,
                last_emit: None,
            })),
        }
    }

    /// Registers a listener to receive [`HostServerView`] updates.
    pub fn set_listener(&self, listener: Listener) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.listener = Some(listener);
        }
    }

    /// Configures an in-process [`EmbeddedServer`] implementation.
    pub fn set_embedded(&self, server: Arc<dyn EmbeddedServer>) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.embedded = Some(server);
        }
    }

    /// Returns `true` if an in-process [`EmbeddedServer`] is attached.
    pub fn has_embedded(&self) -> bool {
        self.inner
            .lock()
            .ok()
            .and_then(|g| g.embedded.as_ref().map(|_| true))
            .unwrap_or(false)
    }

    /// Returns the embedded server's [`crate::bridge::modules::ModulePanel`], if available.
    pub fn modules(&self) -> Option<Arc<dyn crate::bridge::modules::ModulePanel>> {
        let embedded = self.inner.lock().ok()?.embedded.clone()?;
        embedded.modules()
    }

    /// Returns a snapshot of the current [`HostServerView`].
    pub fn view(&self) -> HostServerView {
        self.inner.lock().map(|g| g.view()).unwrap_or(HostServerView {
            state: HostServerState::Stopped,
            message: None,
            lines: Vec::new(),
            invite: None,
            embedded: false,
            components: Vec::new(),
        })
    }

    /// Returns the current [`HostServerState`].
    pub fn state(&self) -> HostServerState {
        self.inner.lock().map(|g| g.state).unwrap_or(HostServerState::Stopped)
    }

    /// Returns `true` when the launcher started a server that has not exited.
    pub fn running(&self) -> bool {
        self.inner
            .lock()
            .map(|g| g.child.is_some() || g.embedded_running)
            .unwrap_or(false)
    }

    /// Updates the active [`HostInvite`] and notifies the listener immediately.
    pub fn set_invite(&self, invite: Option<HostInvite>) {
        let notify = {
            let Ok(mut guard) = self.inner.lock() else {
                return;
            };
            guard.invite = invite;
            guard.last_emit = Some(Instant::now());
            guard.listener.clone().map(|l| (l, guard.view()))
        };
        if let Some((listener, view)) = notify {
            listener(view);
        }
    }

    /// Appends a note line to the server log buffer and notifies the listener.
    pub fn note(&self, line: &str) {
        self.record_line(line, false);
    }

    fn emit_immediate(&self) {
        let notify = {
            let Ok(mut guard) = self.inner.lock() else {
                return;
            };
            guard.last_emit = Some(Instant::now());
            guard.listener.clone().map(|l| (l, guard.view()))
        };
        if let Some((listener, view)) = notify {
            listener(view);
        }
    }

    fn record_line(&self, raw: &str, throttle: bool) {
        let cleaned = clean_line(raw);
        if cleaned.is_empty() {
            return;
        }
        let notify = {
            let Ok(mut guard) = self.inner.lock() else {
                return;
            };
            guard.push_line(cleaned);
            let now = Instant::now();
            let should_emit = !throttle
                || guard
                    .last_emit
                    .map(|prev| now.duration_since(prev) >= CHANGE_EVERY)
                    .unwrap_or(true);
            if should_emit {
                guard.last_emit = Some(now);
                guard.listener.clone().map(|l| (l, guard.view()))
            } else {
                None
            }
        };
        if let Some((listener, view)) = notify {
            listener(view);
        }
    }

    /// Runs a one-shot `dainty-host` subcommand, streaming non-redacted lines to the view and log file.
    pub async fn command(
        &self,
        tool: &HostTool,
        workspace: &Path,
        args: &[String],
        log_name: &str,
    ) -> Result<CommandOutput, HostKitError> {
        let is_invite = log_name == "invite" || args.first().map(|s| s.as_str()) == Some("invite");
        let log_path = WorkspacePaths::new(workspace).launcher_log(log_name);
        if let Some(parent) = log_path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        let mut cmd = Command::new(&tool.program);
        cmd.args(&tool.prefix)
            .args(args)
            .current_dir(workspace)
            .env("NO_COLOR", "1")
            .env("FORCE_COLOR", "0")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW

        let mut child = cmd.spawn().map_err(|e| HostKitError::Spawn(e.to_string()))?;
        self.note(&format!("> {}", args.first().map_or("", String::as_str)));

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();

        if let Some(out) = stdout {
            let tx_out = tx.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(out).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let _ = tx_out.send(line);
                }
            });
        }

        if let Some(err) = stderr {
            let tx_err = tx.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(err).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let _ = tx_err.send(line);
                }
            });
        }
        drop(tx);

        let mut log_file = OpenOptions::new().create(true).append(true).open(&log_path).ok();
        if let Some(f) = log_file.as_mut() {
            let name = tool
                .program
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            let _ = writeln!(
                f,
                "\n> {name} {}  (in {}, {})",
                args.join(" "),
                workspace.display(),
                chrono::Utc::now().to_rfc3339()
            );
        }

        let mut collected = Vec::new();
        while let Some(raw) = rx.recv().await {
            let cleaned = clean_line(&raw);
            if cleaned.is_empty() {
                continue;
            }
            let is_secret_invite_json = is_invite && cleaned.trim_start().starts_with('{');
            if !is_secret_invite_json {
                if let Some(f) = log_file.as_mut() {
                    let _ = writeln!(f, "{cleaned}");
                }
                self.record_line(&cleaned, true);
            }
            collected.push(cleaned);
        }

        let status = child.wait().await.map_err(|e| HostKitError::Spawn(e.to_string()))?;
        let code = status.code().unwrap_or(-1);
        if let Some(f) = log_file.as_mut() {
            let _ = writeln!(f, "(exit code {code})");
        }
        let failure = if code == 0 { None } else { failure_line(&collected) };

        self.emit_immediate();
        Ok(CommandOutput {
            code,
            lines: collected,
            failure,
        })
    }

    /// Probes whether an external server is answering on `BACKEND_PORT` when stopped.
    pub async fn refresh(&self) {
        {
            let Ok(guard) = self.inner.lock() else {
                return;
            };
            if guard.child.is_some()
                || guard.embedded_running
                || matches!(guard.state, HostServerState::Starting | HostServerState::Stopping)
            {
                return;
            }
        }
        let answers = (self.probe)().await;
        let changed = {
            let Ok(mut guard) = self.inner.lock() else {
                return;
            };
            if answers && guard.state != HostServerState::External {
                guard.state = HostServerState::External;
                guard.message = None;
                true
            } else if !answers && guard.state == HostServerState::External {
                guard.state = HostServerState::Stopped;
                guard.message = None;
                true
            } else {
                false
            }
        };
        if changed {
            self.emit_immediate();
        }
    }

    /// Starts the host server (either embedded or as a `dainty-host start` child process).
    pub async fn start(&self, tool: &HostTool, workspace: &Path) -> Result<(), HostKitError> {
        {
            let Ok(mut guard) = self.inner.lock() else {
                return Err(HostKitError::Spawn("lock poisoned".to_string()));
            };
            if guard.child.is_some()
                || guard.embedded_running
                || matches!(guard.state, HostServerState::Starting | HostServerState::Stopping)
            {
                return Ok(());
            }
            guard.state = HostServerState::Starting;
            guard.message = None;
            guard.stop_requested = false;
        }
        if (self.probe)().await {
            if let Ok(mut guard) = self.inner.lock() {
                guard.state = HostServerState::External;
                guard.message = None;
            }
            self.emit_immediate();
            return Ok(());
        }
        self.emit_immediate();

        let embedded = self.inner.lock().ok().and_then(|g| g.embedded.clone());
        if let Some(server) = embedded {
            self.note("> start (inside the launcher)");
            let this = self.clone();
            let sink: LineSink = Arc::new(move |line: String| {
                this.record_line(&line, true);
            });
            let this = self.clone();
            let ws_buf = workspace.to_path_buf();
            tokio::spawn(async move {
                match server.start(ws_buf, sink).await {
                    Ok(()) => {
                        if let Ok(mut guard) = this.inner.lock() {
                            guard.embedded_running = true;
                            guard.state = HostServerState::Running;
                            guard.message = None;
                        }
                        this.emit_immediate();
                        let mut last = Vec::new();
                        loop {
                            let still_running = this.inner.lock().map(|g| g.embedded_running).unwrap_or(false);
                            if !still_running {
                                break;
                            }
                            let now = server.components();
                            if now != last {
                                last = now;
                                this.emit_immediate();
                            }
                            tokio::time::sleep(CHANGE_EVERY).await;
                        }
                    }
                    Err(why) => {
                        this.note(&why);
                        if let Ok(mut guard) = this.inner.lock() {
                            guard.embedded_running = false;
                            guard.state = HostServerState::Failed;
                            guard.message = Some(why);
                        }
                        this.emit_immediate();
                    }
                }
            });
            return Ok(());
        }

        let log_path = WorkspacePaths::new(workspace).launcher_log("server");
        if let Some(parent) = log_path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        let mut cmd = Command::new(&tool.program);
        cmd.args(&tool.prefix)
            .arg("start")
            .current_dir(workspace)
            .env("NO_COLOR", "1")
            .env("FORCE_COLOR", "0")
            .env(CONTROL_ENV, "stdin")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        #[cfg(unix)]
        cmd.process_group(0);

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let err = HostKitError::Spawn(e.to_string());
                if let Ok(mut guard) = self.inner.lock() {
                    guard.state = HostServerState::Failed;
                    guard.message = Some(err.detail());
                }
                self.emit_immediate();
                return Err(err);
            }
        };
        self.note("> start");
        let pid = child.id().unwrap_or(0);
        let stdin = Arc::new(AsyncMutex::new(child.stdin.take()));
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let (done_tx, done_rx) = tokio::sync::watch::channel(false);

        if let Ok(mut guard) = self.inner.lock() {
            guard.child = Some(ChildControl {
                pid,
                stdin,
                done: done_rx,
            });
        }

        let (line_tx, mut line_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        if let Some(out) = stdout {
            let tx = line_tx.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(out).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    let _ = tx.send(line);
                }
            });
        }
        if let Some(err) = stderr {
            let tx = line_tx.clone();
            tokio::spawn(async move {
                let mut reader = BufReader::new(err).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    let _ = tx.send(line);
                }
            });
        }
        drop(line_tx);

        let prog_name = tool
            .program
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let ws_display = workspace.display().to_string();
        let server_ref = self.clone();
        tokio::spawn(async move {
            let mut log_file = OpenOptions::new().create(true).append(true).open(&log_path).ok();
            if let Some(f) = log_file.as_mut() {
                let _ = writeln!(
                    f,
                    "\n> {prog_name} start  (in {ws_display}, {})",
                    chrono::Utc::now().to_rfc3339()
                );
            }
            let mut recent = Vec::new();

            while let Some(raw) = line_rx.recv().await {
                let cleaned = clean_line(&raw);
                if cleaned.is_empty() {
                    continue;
                }
                if cleaned.trim() == SERVER_READY_LINE {
                    if let Ok(mut guard) = server_ref.inner.lock()
                        && guard.state == HostServerState::Starting
                    {
                        guard.state = HostServerState::Running;
                        guard.message = None;
                    }
                    server_ref.emit_immediate();
                    continue;
                }
                if let Some(f) = log_file.as_mut() {
                    let _ = writeln!(f, "{cleaned}");
                }
                recent.push(cleaned.clone());
                if recent.len() > MAX_LINES {
                    recent.remove(0);
                }
                server_ref.record_line(&cleaned, true);
            }

            let status = child.wait().await.ok();
            let code = status.and_then(|s| s.code());
            if let Some(f) = log_file.as_mut() {
                let _ = writeln!(
                    f,
                    "(exit code {})",
                    code.map_or_else(|| "none".to_string(), |c| c.to_string())
                );
            }
            {
                if let Ok(mut guard) = server_ref.inner.lock() {
                    guard.child = None;
                    if guard.stop_requested {
                        guard.state = HostServerState::Stopped;
                        guard.message = None;
                    } else {
                        guard.state = HostServerState::Failed;
                        guard.message = failure_line(&recent).or_else(|| Some(format!("exit {}", code.unwrap_or(-1))));
                    }
                }
            }
            let _ = done_tx.send(true);
            server_ref.emit_immediate();
        });

        Ok(())
    }

    /// Restarts an individual service by name on the embedded server.
    pub async fn restart(&self, name: &str) -> Result<(), String> {
        let server = {
            let Ok(guard) = self.inner.lock() else {
                return Err("the server is not running inside this launcher".to_string());
            };
            guard.embedded.clone().filter(|_| guard.embedded_running)
        };
        let Some(server) = server else {
            return Err("the server is not running inside this launcher".to_string());
        };
        self.note(&format!("> restart {name}"));
        let res = server.restart(name.to_string()).await;
        if let Err(why) = &res {
            self.note(why);
        }
        self.emit_immediate();
        res
    }

    /// Stops the running host server cleanly, falling back to `kill_tree` on timeout.
    pub async fn stop(&self) {
        let embedded = {
            let Ok(mut guard) = self.inner.lock() else {
                return;
            };
            if let Some(server) = guard.embedded.clone().filter(|_| guard.embedded_running) {
                guard.stop_requested = true;
                guard.state = HostServerState::Stopping;
                guard.message = None;
                Some(server)
            } else {
                None
            }
        };
        if let Some(server) = embedded {
            self.emit_immediate();
            server.stop().await;
            if let Ok(mut guard) = self.inner.lock() {
                guard.embedded_running = false;
                guard.state = HostServerState::Stopped;
                guard.message = None;
            }
            self.emit_immediate();
            return;
        }

        let control = {
            let Ok(mut guard) = self.inner.lock() else {
                return;
            };
            let Some(ctrl) = guard.child.as_ref() else {
                return;
            };
            let next = ChildControl {
                pid: ctrl.pid,
                stdin: ctrl.stdin.clone(),
                done: ctrl.done.clone(),
            };
            guard.stop_requested = true;
            guard.state = HostServerState::Stopping;
            guard.message = None;
            next
        };
        self.emit_immediate();

        if let Some(mut stdin) = control.stdin.lock().await.take() {
            let _ = stdin.write_all(b"stop\n").await;
            let _ = stdin.flush().await;
            drop(stdin);
        }

        let mut done_rx = control.done.clone();
        let stop_timeout = self.stop_timeout;
        let exited_in_time = tokio::time::timeout(stop_timeout, async move {
            while !*done_rx.borrow_and_update() {
                if done_rx.changed().await.is_err() {
                    break;
                }
            }
        })
        .await
        .is_ok();

        if !exited_in_time && control.pid != 0 {
            self.note("The server did not stop in time: ending its processes.");
            kill_tree(control.pid).await;
            let mut grace_rx = control.done;
            let _ = tokio::time::timeout(KILL_GRACE, async move {
                while !*grace_rx.borrow_and_update() {
                    if grace_rx.changed().await.is_err() {
                        break;
                    }
                }
            })
            .await;
        }
    }
}

/// Checks whether a TCP listener on `127.0.0.1:<port>` accepts connections within `PROBE_TIMEOUT`.
pub async fn port_answers(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    tokio::time::timeout(PROBE_TIMEOUT, TcpStream::connect(addr))
        .await
        .map(|r| r.is_ok())
        .unwrap_or(false)
}

/// Terminates a process and its descendants.
pub async fn kill_tree(pid: u32) {
    if pid == 0 {
        return;
    }
    #[cfg(windows)]
    {
        let mut cmd = Command::new("taskkill.exe");
        cmd.args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x0800_0000);
        let _ = cmd.status().await;
    }
    #[cfg(not(windows))]
    {
        let group = format!("-{pid}");
        let mut cmd = Command::new("kill");
        cmd.args(["-KILL", "--", &group])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let _ = cmd.status().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tempfile::tempdir;

    struct FakeEmbedded {
        started: AtomicBool,
        fail_start: bool,
    }

    impl EmbeddedServer for FakeEmbedded {
        fn start(
            &self,
            _workspace: PathBuf,
            lines: LineSink,
        ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> {
            let fail = self.fail_start;
            self.started.store(!fail, Ordering::SeqCst);
            Box::pin(async move {
                lines("embedded starting".to_string());
                if fail {
                    Err("embedded boot error".to_string())
                } else {
                    Ok(())
                }
            })
        }

        fn stop(&self) -> Pin<Box<dyn Future<Output = ()> + Send>> {
            self.started.store(false, Ordering::SeqCst);
            Box::pin(async {})
        }

        fn restart(&self, name: String) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>> {
            Box::pin(async move {
                if name == "bad" {
                    Err("bad component".to_string())
                } else {
                    Ok(())
                }
            })
        }

        fn components(&self) -> Vec<HostServerComponent> {
            vec![HostServerComponent {
                name: "gateway".to_string(),
                phase: if self.started.load(Ordering::SeqCst) {
                    HostComponentPhase::Ready
                } else {
                    HostComponentPhase::Stopped
                },
                port: Some(10443),
                detail: String::new(),
            }]
        }
    }

    #[tokio::test]
    async fn embedded_server_lifecycle_and_components() {
        let probe: Probe = Arc::new(|| Box::pin(async { false }));
        let server = HostServer::new(probe, Duration::from_secs(1));
        let embedded = Arc::new(FakeEmbedded {
            started: AtomicBool::new(false),
            fail_start: false,
        });
        server.set_embedded(embedded);
        assert!(server.has_embedded());

        let tmp = tempdir().unwrap();
        let dummy_tool = HostTool {
            program: tmp.path().join("unused"),
            prefix: Vec::new(),
        };

        server.start(&dummy_tool, tmp.path()).await.unwrap();
        for _ in 0..50 {
            if server.state() == HostServerState::Running {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(server.state(), HostServerState::Running);
        assert!(server.running());
        assert!(server.view().embedded);
        assert_eq!(server.view().components.len(), 1);
        assert_eq!(server.view().components[0].phase, HostComponentPhase::Ready);

        server.restart("gateway").await.unwrap();
        assert!(server.restart("bad").await.is_err());

        server.stop().await;
        assert_eq!(server.state(), HostServerState::Stopped);
        assert!(!server.running());
    }

    #[tokio::test]
    async fn external_probe_marks_server_external() {
        let probe: Probe = Arc::new(|| Box::pin(async { true }));
        let server = HostServer::new(probe, Duration::from_secs(1));
        server.refresh().await;
        assert_eq!(server.state(), HostServerState::External);
    }

    #[tokio::test]
    async fn invite_command_redacts_json_lines_from_view_and_log() {
        let tmp = tempdir().unwrap();
        let probe: Probe = Arc::new(|| Box::pin(async { false }));
        let server = HostServer::new(probe, Duration::from_secs(1));

        #[cfg(windows)]
        let (program, prefix) = (
            PathBuf::from(r"C:\Windows\System32\cmd.exe"),
            vec![
                OsString::from("/C"),
                OsString::from(
                    "echo Generating invite & echo {\"invite\":\"https://dauntless.invalid/#secret\",\"for\":\"Alice\"}",
                ),
            ],
        );
        #[cfg(not(windows))]
        let (program, prefix) = (
            PathBuf::from("/bin/sh"),
            vec![
                OsString::from("-c"),
                OsString::from(
                    "echo Generating invite; echo '{\"invite\":\"https://dauntless.invalid/#secret\",\"for\":\"Alice\"}'",
                ),
            ],
        );

        let tool = HostTool { program, prefix };
        let out = server
            .command(&tool, tmp.path(), &[String::from("invite")], "invite")
            .await
            .unwrap();

        assert_eq!(out.code, 0);
        assert!(
            out.lines
                .iter()
                .any(|l| l.contains("https://dauntless.invalid/#secret"))
        );
        assert!(
            !server
                .view()
                .lines
                .iter()
                .any(|l| l.contains("https://dauntless.invalid/#secret"))
        );

        let log_path = WorkspacePaths::new(tmp.path()).launcher_log("invite");
        let log_text = fs::read_to_string(log_path).unwrap_or_default();
        assert!(!log_text.contains("https://dauntless.invalid/#secret"));
    }
}
