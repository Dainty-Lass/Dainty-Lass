// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The launcher's end of the hosted server's player host (spec-runtime-r10 12.2 row 2): the ready pipe's server end,
//! the fence on the control pipe, and the spawner that starts the server between them. The wire is the runtime
//! core's own (`rustic_isles_runtime_core::player_host`), so the two ends cannot drift.

use std::{
    collections::{BTreeMap, HashMap},
    fmt,
    path::Path,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use rustic_isles_runtime_core::config::auth::{self, AuthMode};
use rustic_isles_runtime_core::config::launch::parse_launch;
use rustic_isles_runtime_core::env::Raw;
use rustic_isles_runtime_core::player_host::{self as wire, Verdict};
pub use rustic_isles_runtime_core::player_host::{
    FENCE_EXIT_CODE, FENCE_TIMEOUT_EXIT_CODE, INVALID_EXIT_CODE, MAX_MESSAGE, THREAD_EXIT_CODE,
};

use super::backend::ServerSpawner;
use crate::HostError;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use self::windows::{ReadyListener, fence as send_fence};

#[cfg(not(windows))]
mod unsupported;
#[cfg(not(windows))]
use self::unsupported::{ReadyListener, fence as send_fence};

/// How long a started server has to report ready: a cold boot with `-nullrhi` takes about a minute.
pub const READY_TIMEOUT: Duration = Duration::from_secs(180);
/// How long the fence's acknowledgement may take (12.2: "within a few seconds").
pub const FENCE_ACK_TIMEOUT: Duration = Duration::from_secs(5);
/// After the acknowledgement: the next frame ends the server, or the runtime's own 5 s fallback (`0xE105`) does.
pub const EXIT_AFTER_FENCE: Duration = Duration::from_secs(8);

/// What the launcher checks one hosted server's pipes against: the values R10 keeps from the same launch.
#[derive(Clone, PartialEq, Eq)]
pub struct HostedPipes {
    session: String,
    generation: String,
    launch_id: String,
    ready_pipe: String,
    control_pipe: String,
    port: u16,
}

impl fmt::Debug for HostedPipes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The session, launch id and pipe names stay out of logs on this end too (spec-runtime-r10 14.1)
        f.debug_struct("HostedPipes")
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

impl HostedPipes {
    /// The runtime's own verdict (R1's launch, R10's bootstrap) over this launch; refused unless it would host.
    pub fn from_launch(
        program: &Path,
        arguments: &[String],
        environment: &BTreeMap<String, String>,
    ) -> Result<Self, HostError> {
        let refused = |why: &str| HostError::InvalidInput(format!("the hosted server's launch is refused: {why}"));
        let value = |name: &str| environment.get(name).map(String::as_str);
        let argv: Vec<Vec<u16>> = std::iter::once(program.to_string_lossy().into_owned())
            .chain(arguments.iter().cloned())
            .map(|entry| entry.encode_utf16().collect())
            .collect();
        let mode = auth::auth_mode(&Raw::from_text(value(auth::VARIABLE), auth::READ_CAP));
        if mode != AuthMode::PlayerHost {
            return Err(refused("not a player host"));
        }
        let launch = parse_launch(&argv, &[], mode).map_err(|_| refused("port"))?;
        let read = |name: &str, cap: usize| Raw::from_text(value(name), cap);
        let exists = |name: &str| environment.contains_key(name);
        match wire::bootstrap(read, exists, &launch, &argv) {
            Verdict::Valid(kept) if kept.launcher_pid == std::process::id() => Ok(Self {
                session: kept.session,
                generation: kept.generation,
                launch_id: kept.launch_id,
                ready_pipe: kept.ready_pipe,
                control_pipe: kept.control_pipe,
                port: launch.port,
            }),
            Verdict::Valid(_) => Err(refused(wire::LAUNCHER_PID)),
            // Labels and problem texts only: never a value (14.1)
            Verdict::Invalid {
                failed_check,
                role_problem,
            } => Err(refused(failed_check.or(role_problem).unwrap_or("hosted role"))),
        }
    }

    /// The port the server listens on, which its `READY` must name.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The one `RIS1 READY` line this launch may send.
    pub fn ready_message(&self) -> String {
        wire::ready_message(&self.session, &self.generation, &self.launch_id, self.port)
    }

    /// One message from the ready pipe, untrusted: only the exact `READY` for this launch passes.
    pub fn check_ready(&self, message: &[u8]) -> Result<(), ReadyRefusal> {
        if message.len() > MAX_MESSAGE {
            Err(ReadyRefusal::TooLong)
        } else if message.last() != Some(&b'\n') || message[..message.len() - 1].contains(&b'\n') {
            Err(ReadyRefusal::NotOneLine)
        } else if message == self.ready_message().as_bytes() {
            Ok(())
        } else {
            Err(ReadyRefusal::Mismatch)
        }
    }

    /// The `RIS1 FENCE` the launcher writes on the control pipe.
    pub fn fence_request(&self) -> String {
        wire::fence_request(&self.session, &self.generation)
    }

    /// One message from the control pipe, untrusted: the exact `FENCE_ACK` for this session and generation.
    pub fn is_fence_ack(&self, message: &[u8]) -> bool {
        message.len() <= MAX_MESSAGE && message == wire::fence_ack(&self.session, &self.generation).as_bytes()
    }

    fn ready_pipe(&self) -> &str {
        &self.ready_pipe
    }

    fn control_pipe(&self) -> &str {
        &self.control_pipe
    }
}

/// Why a ready pipe message was refused. The texts never echo it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadyRefusal {
    /// Over 511 bytes.
    TooLong,
    /// Not one line ending in LF.
    NotOneLine,
    /// Not this launch's `READY`: another session, generation, launch, port or bind, or another tag.
    Mismatch,
}

impl fmt::Display for ReadyRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TooLong => "longer than 511 bytes",
            Self::NotOneLine => "not one line ending in LF",
            Self::Mismatch => "not this launch's READY",
        })
    }
}

/// Why a pipe end gave up. The texts name no pipe, session or pid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PipeFailure {
    /// Nothing came in time.
    TimedOut,
    /// The server exited first.
    Exited,
    /// The other end is not the process this launcher started.
    WrongPeer,
    /// A message over 511 bytes.
    TooLong,
    /// A short write.
    ShortWrite,
    /// The OS refused a call, with its error code.
    Os(u32),
    /// This platform has no player host pipes.
    Unsupported,
}

impl fmt::Display for PipeFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TimedOut => f.write_str("timed out"),
            Self::Exited => f.write_str("the server exited"),
            Self::WrongPeer => f.write_str("the pipe's other end is not the hosted server"),
            Self::TooLong => f.write_str("a message over 511 bytes"),
            Self::ShortWrite => f.write_str("a short write"),
            Self::Os(code) => write!(f, "error {code}"),
            Self::Unsupported => f.write_str("realm hosting starts its server on Windows only"),
        }
    }
}

/// A started hosted server, as the spawner needs it.
pub trait HostedProcess: Send + Sync {
    /// Its pid.
    fn pid(&self) -> u32;
    /// Whether it has exited.
    fn has_exited(&self) -> bool;
    /// Waits up to `timeout` for it to exit; whether it did.
    fn wait_exit(&self, timeout: Duration) -> bool;
    /// Ends it now.
    fn kill(&self);
}

/// Starts the hosted server's process; the launcher's starts it suspended and injects the runtime (RD9).
pub trait ProcessStarter: Send + Sync {
    /// Starts `program` with exactly `environment` and returns it running.
    fn start(
        &self,
        program: &Path,
        arguments: &[String],
        working_dir: &Path,
        environment: &BTreeMap<String, String>,
    ) -> Result<Box<dyn HostedProcess>, HostError>;
}

struct Server {
    process: Arc<dyn HostedProcess>,
    pipes: HostedPipes,
    ready: Option<ReadyListener>,
}

/// The real [`ServerSpawner`]: the ready pipe before the server, the server through a [`ProcessStarter`], its
/// `READY` checked, and stopped by the fence or, failing that, by ending its process.
pub struct PlayerHostSpawner {
    starter: Arc<dyn ProcessStarter>,
    servers: Mutex<HashMap<u32, Server>>,
    ready_timeout: Duration,
}

impl fmt::Debug for PlayerHostSpawner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlayerHostSpawner")
            .field("ready_timeout", &self.ready_timeout)
            .finish_non_exhaustive()
    }
}

fn held<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(PoisonError::into_inner)
}

impl PlayerHostSpawner {
    /// Servers started through `starter`.
    pub fn new(starter: Arc<dyn ProcessStarter>) -> Self {
        Self {
            starter,
            servers: Mutex::new(HashMap::new()),
            ready_timeout: READY_TIMEOUT,
        }
    }

    /// How long [`ServerSpawner::wait_ready`] waits.
    #[must_use]
    pub fn with_ready_timeout(mut self, timeout: Duration) -> Self {
        self.ready_timeout = timeout;
        self
    }

    /// Fences a running server: `true` only for this generation's exact acknowledgement.
    fn fence(server: &Server, pid: u32) -> bool {
        let request = server.pipes.fence_request();
        send_fence(server.pipes.control_pipe(), pid, request.as_bytes(), FENCE_ACK_TIMEOUT)
            .is_ok_and(|ack| server.pipes.is_fence_ack(&ack))
    }
}

impl ServerSpawner for PlayerHostSpawner {
    fn spawn(
        &self,
        program: &Path,
        arguments: &[String],
        working_dir: &Path,
        environment: &BTreeMap<String, String>,
    ) -> Result<u32, HostError> {
        let pipes = HostedPipes::from_launch(program, arguments, environment)?;
        // Before the server starts: the runtime waits only 5 s for this pipe (spec-runtime-r10 8.2)
        let ready = ReadyListener::create(pipes.ready_pipe())
            .map_err(|failed| HostError::Io(format!("the ready pipe could not be made: {failed}")))?;
        let process: Arc<dyn HostedProcess> =
            Arc::from(self.starter.start(program, arguments, working_dir, environment)?);
        let pid = process.pid();
        let server = Server {
            process,
            pipes,
            ready: Some(ready),
        };
        held(&self.servers).insert(pid, server);
        Ok(pid)
    }

    fn wait_ready(&self, pid: u32) -> Result<(), HostError> {
        let (process, pipes, ready) = {
            let mut servers = held(&self.servers);
            let server = servers
                .get_mut(&pid)
                .ok_or_else(|| HostError::NotFound("no hosted server under that pid".to_owned()))?;
            (Arc::clone(&server.process), server.pipes.clone(), server.ready.take())
        };
        let ready =
            ready.ok_or_else(|| HostError::InvalidInput("the hosted server's readiness was read".to_owned()))?;
        let alive = || !process.has_exited();
        let message = ready
            .accept(pid, self.ready_timeout, &alive)
            .map_err(|failed| HostError::Transport(format!("the hosted server did not report ready: {failed}")))?;
        pipes
            .check_ready(&message)
            .map_err(|refused| HostError::Transport(format!("the hosted server's readiness was refused: {refused}")))
    }

    fn stop(&self, pid: u32) {
        let Some(server) = held(&self.servers).remove(&pid) else {
            return;
        };
        if server.process.has_exited() {
            return;
        }
        // 12.2: a server that cannot be fenced, or does not end once fenced, is ended here
        if !(Self::fence(&server, pid) && server.process.wait_exit(EXIT_AFTER_FENCE)) {
            server.process.kill();
        }
    }
}

#[cfg(test)]
mod tests;
