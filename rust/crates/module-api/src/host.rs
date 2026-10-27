//! The host side of the frame protocol (DD5): [`ChildModule`] starts an external module's executable,
//! checked against the SHA-256 `host.toml` pins for it, and drives it as a [`ModuleDriver`]. The file is
//! opened once and the bytes hashed are the bytes started ([`open_pinned`]); the child gets only the
//! environment the controller gives it, and its whole process tree is killed on unload.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, Semaphore};
use tokio::task::JoinHandle;

use crate::driver::ModuleDriver;
use crate::frame::{
    FrameError, MAX_FRAME, MAX_HELLO_FRAME, ModuleAnswer, ModuleCall, PROTOCOL_VERSION, Refusal, ToHost, ToModule,
    Welcome, check_hello, read_frame,
};
use crate::ids::{ModuleId, Sha256Hex};
use crate::manifest::ModuleManifest;
use crate::module::{BoxFuture, ErrorKind, HostCall, HostLink, LogRecord, ModuleError};
use crate::network::NodeIdentity;
use crate::peer::{self, MAX_IN_FLIGHT, MAX_LOG_LINE, Pending, Resolved, Writer, log_safe};
use crate::tree::Tree;
use crate::version::API_VERSION;

/// An external module as `host.toml` names it (DF3): an executable, pinned by its SHA-256.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalModule {
    pub path: PathBuf,
    pub sha256: Sha256Hex,
    pub args: Vec<String>,
    /// The module `host.toml` lists under this entry; the child's hello must name it
    pub expected: Option<ModuleId>,
}

#[derive(Debug, Clone)]
pub struct ChildOptions {
    pub handshake_timeout: Duration,
    pub call_timeout: Duration,
    pub start_timeout: Duration,
    /// How long `stop` may take, and how long the child has to exit after it before it is killed
    pub stop_grace: Duration,
    pub max_frame: usize,
    /// The child's whole environment. `None` passes the host's on, every service's settings with it: the
    /// controller always gives one, as `start` gives each service only its own (DF3)
    pub env: Option<Vec<(String, String)>>,
    /// Where the child runs; the host's working folder when `None`
    pub current_dir: Option<PathBuf>,
}

impl Default for ChildOptions {
    fn default() -> Self {
        Self {
            handshake_timeout: Duration::from_secs(10),
            call_timeout: Duration::from_secs(30),
            start_timeout: Duration::from_secs(120),
            stop_grace: Duration::from_secs(10),
            max_frame: MAX_FRAME,
            env: None,
            current_dir: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("could not read {path} to check it: {error}")]
    Read { path: PathBuf, error: std::io::Error },
    #[error("{path} has SHA-256 {found}, host.toml pins {expected}")]
    HashMismatch {
        path: PathBuf,
        expected: Sha256Hex,
        found: Sha256Hex,
    },
    #[error("could not start {path}: {error}")]
    Spawn { path: PathBuf, error: std::io::Error },
    #[error("the module did not say hello within {0:?}")]
    HandshakeTimeout(Duration),
    #[error("the module's hello was refused: {0}")]
    Refused(#[from] Refusal),
    #[error("the module's handshake failed: {0}")]
    Handshake(String),
    #[error("the module's handshake failed: {0}")]
    Frame(#[from] FrameError),
}

/// Opens an executable to hash and then start from the same handle. On Windows nobody may write, rename
/// or delete it while the handle is open (only reads are shared). On Linux the child is started through
/// the handle (`/proc/self/fd`), so a file renamed over it in between is not the one that runs; a write
/// into the same file in between still is, which is why its folder must be the host's alone
/// (docs/security/module-host.md).
pub fn open_pinned(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        // FILE_SHARE_READ alone
        options.share_mode(0x1);
    }
    options.open(path)
}

fn hash_file(file: &mut std::fs::File) -> std::io::Result<Sha256Hex> {
    use std::io::Read as _;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 << 10];
    loop {
        match file.read(&mut buf)? {
            0 => break,
            n => hasher.update(&buf[..n]),
        }
    }
    Ok(Sha256Hex::from_bytes(hasher.finalize().into()))
}

/// The file opened with [`open_pinned`], and its SHA-256 read in full from that handle.
async fn open_and_hash(path: &Path) -> std::io::Result<(std::fs::File, Sha256Hex)> {
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || {
        let mut file = open_pinned(&path)?;
        let digest = hash_file(&mut file)?;
        Ok((file, digest))
    })
    .await
    .map_err(std::io::Error::other)?
}

/// The SHA-256 of a file, read in full.
pub async fn file_sha256(path: &Path) -> std::io::Result<Sha256Hex> {
    Ok(open_and_hash(path).await?.1)
}

/// What to start: the open handle itself where the OS can run one, the path elsewhere.
#[cfg(target_os = "linux")]
fn program(path: &Path, file: &std::fs::File) -> PathBuf {
    use std::os::fd::AsRawFd as _;
    let by_handle = PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()));
    if by_handle.exists() { by_handle } else { path.to_owned() }
}

#[cfg(not(target_os = "linux"))]
fn program(path: &Path, _file: &std::fs::File) -> PathBuf {
    path.to_owned()
}

/// A started child and the tree it leads.
struct Process {
    child: Child,
    tree: Tree,
}

impl Process {
    fn kill(&mut self) {
        self.tree.kill();
        _ = self.child.start_kill();
    }
}

/// A module running as a child process, or on any pair of streams ([`ChildModule::connect`]).
pub struct ChildModule {
    manifest: ModuleManifest,
    writer: Writer,
    pending: Arc<Pending<ModuleAnswer>>,
    options: ChildOptions,
    process: Arc<Mutex<Option<Process>>>,
    tasks: Vec<JoinHandle<()>>,
}

impl Drop for ChildModule {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

impl ChildModule {
    /// Checks the executable against its pinned SHA-256, starts it with piped stdio and runs the handshake.
    /// Nothing is started for a file that doesn't match.
    pub async fn spawn(
        module: &ExternalModule,
        node: NodeIdentity,
        link: Arc<dyn HostLink>,
        options: ChildOptions,
    ) -> Result<Self, LoadError> {
        let path = &module.path;
        let (file, found) = open_and_hash(path).await.map_err(|error| LoadError::Read {
            path: path.clone(),
            error,
        })?;
        if found != module.sha256 {
            return Err(LoadError::HashMismatch {
                path: path.clone(),
                expected: module.sha256,
                found,
            });
        }
        let mut command = Command::new(program(path, &file));
        command
            .args(&module.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(env) = &options.env {
            command.env_clear().envs(env.iter().map(|(k, v)| (k, v)));
        }
        if let Some(dir) = &options.current_dir {
            command.current_dir(dir);
        }
        // Its own group, so the whole tree can be killed (DD5); its own name as it sees it
        #[cfg(unix)]
        command.process_group(0).arg0(path);
        // CREATE_NO_WINDOW: no console flashes up under the server GUI
        #[cfg(windows)]
        command.creation_flags(0x0800_0000);
        let spawned = |error| LoadError::Spawn {
            path: path.clone(),
            error,
        };
        let mut child = command.spawn().map_err(spawned)?;
        let tree = match Tree::adopt(&child) {
            Ok(tree) => tree,
            Err(error) => {
                _ = child.start_kill();
                return Err(spawned(error));
            }
        };
        // Held until the child runs: the bytes checked are the bytes started
        drop(file);
        let (Some(stdin), Some(stdout), Some(stderr)) = (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            return Err(LoadError::Handshake(String::from("the child's stdio was not piped")));
        };
        let process = Arc::new(Mutex::new(Some(Process { child, tree })));
        let mut this = Self::handshake(stdout, stdin, node, link, options, module.expected.as_ref(), process).await?;
        let id = this.manifest.id.clone();
        this.tasks.push(tokio::spawn(forward_stderr(stderr, id)));
        Ok(this)
    }

    /// Runs the host side over any streams: `reader` is what the module writes, `writer` what it reads.
    pub async fn connect<R, W>(
        reader: R,
        writer: W,
        node: NodeIdentity,
        link: Arc<dyn HostLink>,
        options: ChildOptions,
        expected: Option<&ModuleId>,
    ) -> Result<Self, LoadError>
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + 'static,
    {
        Self::handshake(
            reader,
            writer,
            node,
            link,
            options,
            expected,
            Arc::new(Mutex::new(None)),
        )
        .await
    }

    async fn handshake<R, W>(
        reader: R,
        writer: W,
        node: NodeIdentity,
        link: Arc<dyn HostLink>,
        options: ChildOptions,
        expected: Option<&ModuleId>,
        process: Arc<Mutex<Option<Process>>>,
    ) -> Result<Self, LoadError>
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + 'static,
    {
        let mut reader = BufReader::new(reader);
        let writer = peer::writer(writer);
        let first = tokio::time::timeout(
            options.handshake_timeout,
            read_frame::<ToHost, _>(&mut reader, MAX_HELLO_FRAME),
        )
        .await
        .map_err(|_| LoadError::HandshakeTimeout(options.handshake_timeout))??;
        let hello = match first {
            Some(ToHost::Hello(hello)) => *hello,
            Some(_) => return Err(LoadError::Handshake(String::from("the first frame was not a hello"))),
            None => return Err(LoadError::Handshake(String::from("the module exited before its hello"))),
        };
        if let Err(refusal) = check_hello(&hello, expected) {
            // Best effort: the module is told why before its pipe closes
            _ = peer::send(
                &writer,
                &ToModule::Refused {
                    reason: refusal.to_string(),
                },
                options.max_frame,
            )
            .await;
            return Err(refusal.into());
        }
        let welcome = ToModule::Welcome(Welcome {
            protocol: PROTOCOL_VERSION,
            api: API_VERSION,
            node,
        });
        peer::send(&writer, &welcome, options.max_frame)
            .await
            .map_err(|e| LoadError::Handshake(e.message))?;
        let manifest = hello.manifest;
        let pending = Arc::new(Pending::new());
        let reading = tokio::spawn(read_loop(
            reader,
            writer.clone(),
            pending.clone(),
            link,
            manifest.id.clone(),
            options.max_frame,
            process.clone(),
        ));
        Ok(Self {
            manifest,
            writer,
            pending,
            options,
            process,
            tasks: vec![reading],
        })
    }

    /// Whether the pipe still works; once it broke, every call fails and the controller restarts it (DF3).
    pub fn is_running(&self) -> bool {
        self.failure().is_none()
    }

    /// Why the module is gone: its pipe broke, or its process exited. A process of its own can hold the
    /// pipe open after it died (a handle it passed on), so the exit is checked as well, and what the
    /// module left running is killed with it.
    pub fn failure(&self) -> Option<String> {
        if let Some(reason) = self.pending.closed() {
            return Some(reason);
        }
        let mut guard = self.process.try_lock().ok()?;
        let process = guard.as_mut()?;
        let status = process.child.try_wait().ok()??;
        process.tree.kill();
        let reason = format!("the module exited ({status})");
        self.pending.close(reason.clone());
        Some(reason)
    }

    fn timeout_for(&self, call: &ModuleCall) -> Duration {
        match call {
            ModuleCall::Start(_) => self.options.start_timeout,
            ModuleCall::Stop => self.options.stop_grace,
            _ => self.options.call_timeout,
        }
    }

    async fn run(&self, call: ModuleCall) -> Result<ModuleAnswer, ModuleError> {
        let timeout = self.timeout_for(&call);
        let (id, rx) = self.pending.open()?;
        if let Err(error) = peer::send(&self.writer, &ToModule::Call { id, call }, self.options.max_frame).await {
            self.pending.forget(id);
            return Err(error);
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(ModuleError::new(ErrorKind::Unavailable, "the module's pipe closed")),
            Err(_) => {
                self.pending.forget(id);
                Err(ModuleError::new(
                    ErrorKind::Timeout,
                    format!("no answer within {timeout:?}"),
                ))
            }
        }
    }
}

impl ModuleDriver for ChildModule {
    fn manifest(&self) -> &ModuleManifest {
        &self.manifest
    }

    fn call(&self, call: ModuleCall) -> BoxFuture<'_, Result<ModuleAnswer, ModuleError>> {
        Box::pin(self.run(call))
    }

    fn unload(&self) -> BoxFuture<'_, Result<(), ModuleError>> {
        Box::pin(async move {
            let stopped = match self.run(ModuleCall::Stop).await {
                Ok(ModuleAnswer::Stopped) => Ok(()),
                Ok(other) => Err(ModuleError::new(
                    ErrorKind::Protocol,
                    format!("the module answered {other:?}"),
                )),
                Err(error) => Err(error),
            };
            peer::close(&self.writer).await;
            if let Some(mut process) = self.process.lock().await.take() {
                _ = tokio::time::timeout(self.options.stop_grace, process.child.wait()).await;
                // What the module started goes with it, whether or not it exited in time
                process.kill();
                _ = process.child.wait().await;
            }
            self.pending.close("the module was unloaded");
            stopped
        })
    }
}

fn violation(pending: &Pending<ModuleAnswer>, what: &str) {
    pending.close(format!("the module broke the frame protocol: {what}"));
}

async fn read_loop<R: AsyncRead + Send + Unpin>(
    mut reader: BufReader<R>,
    writer: Writer,
    pending: Arc<Pending<ModuleAnswer>>,
    link: Arc<dyn HostLink>,
    module: ModuleId,
    max_frame: usize,
    process: Arc<Mutex<Option<Process>>>,
) {
    let slots = Arc::new(Semaphore::new(MAX_IN_FLIGHT));
    loop {
        match read_frame::<ToHost, _>(&mut reader, max_frame).await {
            Ok(Some(ToHost::Reply { id, result })) => match pending.resolve(id, result) {
                Resolved::Delivered | Resolved::Late => {}
                Resolved::Unknown => break violation(&pending, "a reply to a call never made"),
            },
            Ok(Some(ToHost::Call { id, call })) => {
                let Ok(slot) = slots.clone().acquire_owned().await else {
                    break;
                };
                let (link, writer, module) = (link.clone(), writer.clone(), module.clone());
                tokio::spawn(async move {
                    let call = match call {
                        HostCall::Log(LogRecord { level, message }) => HostCall::Log(LogRecord {
                            level,
                            message: log_safe(&message),
                        }),
                        call => call,
                    };
                    let result = link.call(&module, call).await;
                    _ = peer::send(&writer, &ToModule::Reply { id, result }, max_frame).await;
                    drop(slot);
                });
            }
            Ok(Some(ToHost::Hello(_))) => break violation(&pending, "a second hello"),
            Ok(None) => break pending.close("the module closed its output"),
            Err(error) => break violation(&pending, &error.to_string()),
        }
    }
    // A child whose pipe is broken can't be driven; the controller sees it failed and restarts it (DF3)
    if let Some(process) = process.lock().await.as_mut() {
        process.kill();
    }
}

/// The child's stderr into the host's tracing layers, one bounded line at a time.
async fn forward_stderr<R: AsyncRead + Unpin>(stderr: R, module: ModuleId) {
    let mut reader = BufReader::new(stderr);
    let mut line = Vec::new();
    loop {
        let buf = match reader.fill_buf().await {
            Ok([]) | Err(_) => break,
            Ok(buf) => buf,
        };
        let (taken, newline) = match buf.iter().position(|&b| b == b'\n') {
            Some(i) => (i + 1, true),
            None => (buf.len(), false),
        };
        let room = MAX_LOG_LINE.saturating_sub(line.len());
        line.extend_from_slice(&buf[..taken.min(room)]);
        reader.consume(taken);
        if newline || line.len() >= MAX_LOG_LINE {
            tracing::info!(target: "module", service = %module, "{}", log_safe(&String::from_utf8_lossy(&line)));
            line.clear();
        }
    }
    if !line.is_empty() {
        tracing::info!(target: "module", service = %module, "{}", log_safe(&String::from_utf8_lossy(&line)));
    }
}
