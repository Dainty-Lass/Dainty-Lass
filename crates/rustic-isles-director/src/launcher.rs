// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! How a game server is started (spec §5.7): the seam tests replace, and the operating system's launcher.

use std::io;
use std::pin::Pin;
use std::sync::Arc;

use futures_util::future::BoxFuture;
use tokio::io::AsyncRead;

/// What to start. No `Debug`: the gameserver key is an argument (§15).
pub struct Command {
    /// The executable.
    pub program: String,
    /// Its arguments.
    pub args: Vec<String>,
    /// The whole environment: the child gets this and nothing else
    pub env: Vec<(String, String)>,
    /// Hide its window on Windows.
    pub windows_hide: bool,
    /// Loaded, in order, into the process started suspended before it runs (Windows); empty starts it as is
    pub inject: Vec<dainty_inject::InjectDll>,
}

/// How a process exited.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Exit {
    /// The exit code, none when a signal ended it.
    pub code: Option<i32>,
    /// `SIGTERM` and the like (Unix)
    pub signal: Option<String>,
}

/// A value as a JavaScript template writes it: `null` when there is none (the exit texts of §5.2 and §5.5).
pub struct OrNull<'a, T>(pub &'a Option<T>);

impl<T: std::fmt::Display> std::fmt::Display for OrNull<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            Some(value) => value.fmt(f),
            None => f.write_str("null"),
        }
    }
}

/// A game server's process tree.
pub trait TreeControl: Send + Sync {
    /// Asks the tree to stop (SIGTERM to the group; the job ended on Windows).
    fn terminate(&self) -> io::Result<()>;
    /// Kills the whole tree at once.
    fn kill(&self);
}

/// One of the child's output streams.
pub type Pipe = Pin<Box<dyn AsyncRead + Send>>;

/// A started child.
pub struct Started {
    /// The process id, when the OS gave one.
    pub pid: Option<u32>,
    /// Its standard output.
    pub stdout: Pipe,
    /// Its standard error.
    pub stderr: Pipe,
    /// Resolves when it exits.
    pub wait: BoxFuture<'static, Exit>,
    /// Stops its process tree.
    pub tree: Arc<dyn TreeControl>,
}

/// Starts game servers: the operating system's, or a test's.
pub trait Launcher: Send + Sync {
    /// Starts the command; an error when the OS refuses.
    fn start(&self, command: &Command) -> io::Result<Started>;
}

/// The operating system's: a job object on Windows, a process group on Unix (§5.7).
#[derive(Debug, Clone, Copy, Default)]
pub struct OsLauncher;

impl Launcher for OsLauncher {
    fn start(&self, command: &Command) -> io::Result<Started> {
        let texts = std::iter::once(command.program.as_str())
            .chain(command.args.iter().map(String::as_str))
            .chain(command.env.iter().flat_map(|(k, v)| [k.as_str(), v.as_str()]));
        if crate::cmdline::has_nul(texts) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "an argument contains a NUL",
            ));
        }
        // Under Wine the wrapper's helper injects, inside the prefix
        if !cfg!(windows) && !command.inject.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "only Windows injects into a game server directly",
            ));
        }
        crate::os::spawn(command)
    }
}
