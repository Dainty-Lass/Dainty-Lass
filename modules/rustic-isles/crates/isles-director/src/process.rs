//! One game server process: started through a [`Launcher`] with its output appended to its own log
//! file, its readiness marker watched on stdout, its exit reported once, and its whole process tree
//! ended with it.
//!
//! The game thread deadlocked mid-hunt once (2026-07-19 GAME-THREAD FREEZE FIX in the Node
//! Director): the readiness reader paused stdout, the pipe filled, and the server's next log write
//! blocked forever holding the CRT's stdio lock. So both pipes are read to the end, always: the
//! readiness check only looks at what the log copy reads, and never stops it.
//!
//! The log file outlives the process (2026-10-23, DS5): a game server that dies at once (a missing
//! executable, Wine refusing it) closes its pipes before its exit is seen, and the exit line written
//! after them was a 'write after end' that took the Node Director down. Here the file belongs to one
//! writer task that ends when the last sender is dropped, and a line sent after it is ignored.

use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot, watch};

/// How long a stopped process tree gets before what is left of it is killed.
pub const STOP_GRACE: Duration = Duration::from_secs(10);

/// How a process ended, as Node's exit event reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Exit {
    pub code: Option<i32>,
    pub signal: Option<String>,
}

impl Exit {
    pub fn code_text(&self) -> String {
        self.code.map_or_else(|| "null".to_owned(), |c| c.to_string())
    }

    pub fn signal_text(&self) -> String {
        self.signal.clone().unwrap_or_else(|| "null".to_owned())
    }

    pub fn unknown() -> Self {
        Self {
            code: None,
            signal: None,
        }
    }
}

/// A game server's process tree: the process and everything it started (Wine, Xvfb, the game).
pub trait TreeControl: Send + Sync {
    /// Asks the tree to stop: SIGTERM to the process group; a job object has no polite stop, so it ends it.
    fn terminate(&self);
    /// Ends whatever is left: SIGKILL to the group, `TerminateJobObject`.
    fn kill(&self);
}

/// What a [`Launcher`] runs. The arguments carry the gameserver key and the environment more secrets, so
/// there is no `Debug`.
#[derive(Clone)]
pub struct Command {
    pub program: String,
    pub args: Vec<String>,
    /// The whole environment: nothing else is inherited
    pub env: Vec<(String, String)>,
    pub windows_hide: bool,
}

pub type Pipe = Box<dyn AsyncRead + Unpin + Send>;

/// A started process, whatever the platform or the fake.
pub struct Started {
    pub pid: Option<u32>,
    pub stdout: Pipe,
    pub stderr: Pipe,
    /// Resolves once, when the leader has exited
    pub wait: Pin<Box<dyn Future<Output = Exit> + Send>>,
    pub tree: Arc<dyn TreeControl>,
}

/// How game servers are started. The Director uses [`crate::OsLauncher`]; tests put a fake behind it.
pub trait Launcher: Send + Sync {
    fn start(&self, command: &Command) -> io::Result<Started>;
}

/// A spawned process: whether it still runs, and the way to stop it.
pub struct Proc {
    pub pid: Option<u32>,
    exited: AtomicBool,
    tree: Arc<dyn TreeControl>,
    grace: Duration,
}

impl std::fmt::Debug for Proc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Proc")
            .field("pid", &self.pid)
            .field("exited", &self.exited)
            .finish()
    }
}

impl Proc {
    pub fn alive(&self) -> bool {
        !self.exited.load(Ordering::SeqCst)
    }

    /// Stops the tree (SIGTERM, which on Linux the game-server launcher passes to Wine, the game and
    /// Xvfb; the job object on Windows) and, if the leader is still there after the grace, kills it.
    pub fn terminate(self: &Arc<Self>) -> Result<(), String> {
        if !self.alive() {
            return Err("the process is gone".to_owned());
        }
        self.tree.terminate();
        let proc = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(proc.grace).await;
            if proc.alive() {
                tracing::warn!(
                    "a game server (pid={:?}) ignored its stop; killing its process tree",
                    proc.pid
                );
                proc.tree.kill();
            }
        });
        Ok(())
    }

    /// Ends the tree now.
    pub fn kill(&self) {
        self.tree.kill();
    }
}

/// What [`spawn`] needs beyond the command.
pub struct Spec {
    pub command: Command,
    pub log: std::fs::File,
    pub launch_id: String,
    pub port: u16,
    pub grace: Duration,
}

pub struct Spawned {
    pub proc: Arc<Proc>,
    /// Fires on the first matching readiness marker.
    pub ready: oneshot::Receiver<()>,
    /// `Some` once the process has exited.
    pub exit: watch::Receiver<Option<Exit>>,
    /// Lines for the log file after the process output.
    pub log: mpsc::UnboundedSender<Vec<u8>>,
}

/// `MYSTICPARADOX_GAMESERVER_READY launchId=<id> port=<port>`, which the runtime prints only after
/// Listen succeeded: `Some(true)` for this launch, `Some(false)` for another one's.
pub fn readiness(line: &str, launch_id: &str, port: u16) -> Option<bool> {
    let rest = service_kit::js::trim(line).strip_prefix("MYSTICPARADOX_GAMESERVER_READY launchId=")?;
    let (id, rest) = rest.split_once(" port=")?;
    if id.is_empty() || !id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f' | b'-')) {
        return None;
    }
    if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some(id == launch_id && rest.parse::<u64>().ok() == Some(u64::from(port)))
}

/// What runs once the process has exited, with a sender for lines after its output in the log.
pub type OnExit = Box<dyn FnOnce(Exit, mpsc::UnboundedSender<Vec<u8>>) + Send>;

pub fn spawn(launcher: &dyn Launcher, spec: Spec, on_exit: OnExit) -> Result<Spawned, String> {
    let started = launcher
        .start(&spec.command)
        .map_err(|e| format!("spawn {} {e}", spec.command.program))?;
    let proc = Arc::new(Proc {
        pid: started.pid,
        exited: AtomicBool::new(false),
        tree: started.tree.clone(),
        grace: spec.grace,
    });
    let (log_tx, log_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    tokio::spawn(write_log(tokio::fs::File::from_std(spec.log), log_rx));
    let (ready_tx, ready_rx) = oneshot::channel();
    let scan = Scan {
        launch_id: spec.launch_id.clone(),
        port: spec.port,
        ready: Some(ready_tx),
        buffered: Vec::new(),
    };
    // Each reader holds its own sender, so the log ends only when both pipes and the exit are done.
    tokio::spawn(drain(started.stdout, log_tx.clone(), Some(scan)));
    tokio::spawn(drain(started.stderr, log_tx.clone(), None));
    let (exit_tx, exit_rx) = watch::channel(None);
    let watched = proc.clone();
    let exit_log = log_tx.clone();
    let wait = started.wait;
    tokio::spawn(async move {
        let exit = wait.await;
        watched.exited.store(true, Ordering::SeqCst);
        // The leader is gone: so is whatever it left running (a stray Xvfb would hold the port).
        watched.tree.kill();
        let _ = exit_tx.send(Some(exit.clone()));
        on_exit(exit, exit_log);
    });
    Ok(Spawned {
        proc,
        ready: ready_rx,
        exit: exit_rx,
        log: log_tx,
    })
}

async fn write_log(mut file: tokio::fs::File, mut lines: mpsc::UnboundedReceiver<Vec<u8>>) {
    let mut failed = false;
    while let Some(bytes) = lines.recv().await {
        // Keep receiving after a failure: the senders must never block or fail because the log did.
        if !failed && file.write_all(&bytes).await.is_err() {
            failed = true;
        }
    }
    let _ = file.flush().await;
}

struct Scan {
    launch_id: String,
    port: u16,
    ready: Option<oneshot::Sender<()>>,
    buffered: Vec<u8>,
}

impl Scan {
    fn feed(&mut self, chunk: &[u8]) {
        if self.ready.is_none() {
            return;
        }
        self.buffered.extend_from_slice(chunk);
        while let Some(newline) = self.buffered.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buffered.drain(..=newline).collect();
            let line = String::from_utf8_lossy(&line[..line.len() - 1]).into_owned();
            match readiness(&line, &self.launch_id, self.port) {
                Some(true) => {
                    if let Some(ready) = self.ready.take() {
                        let _ = ready.send(());
                    }
                    self.buffered.clear();
                    return;
                }
                Some(false) => tracing::warn!(
                    "Ignoring mismatched gameserver readiness marker on port {}: {line}",
                    self.port
                ),
                None => {}
            }
        }
        // A server that never writes a newline must not grow this without bound.
        if self.buffered.len() > 65536 {
            let keep = self.buffered.len() - 4096;
            self.buffered.drain(..keep);
        }
    }
}

async fn drain(mut pipe: Pipe, log: mpsc::UnboundedSender<Vec<u8>>, mut scan: Option<Scan>) {
    let mut buffer = vec![0u8; 16 * 1024];
    loop {
        match pipe.read(&mut buffer).await {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                let _ = log.send(buffer[..n].to_vec());
                if let Some(scan) = &mut scan {
                    scan.feed(&buffer[..n]);
                }
            }
        }
    }
}

/// Where a game server's log goes: `<dir>/port<port>_<label>.log`.
pub fn log_path(dir: &Path, port: u16, label: &str) -> PathBuf {
    dir.join(format!("port{port}_{label}.log"))
}
