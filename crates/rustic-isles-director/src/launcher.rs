// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! How a game server is started (spec §5.7): the seam tests replace, and the operating system's launcher.

use std::io;
use std::pin::Pin;
use std::sync::Arc;

use futures_util::future::BoxFuture;
use tokio::io::AsyncRead;

/// What to start. No `Debug`: the gameserver key is an argument (§15).
pub struct Command {
    pub program: String,
    pub args: Vec<String>,
    /// The whole environment: the child gets this and nothing else
    pub env: Vec<(String, String)>,
    pub windows_hide: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Exit {
    pub code: Option<i32>,
    /// `SIGTERM` and the like (Unix)
    pub signal: Option<String>,
}

impl Exit {
    pub fn code_text(&self) -> String {
        self.code.map_or_else(|| "null".to_owned(), |c| c.to_string())
    }

    pub fn signal_text(&self) -> String {
        self.signal.clone().unwrap_or_else(|| "null".to_owned())
    }
}

/// A game server's process tree.
pub trait TreeControl: Send + Sync {
    /// Asks the tree to stop (SIGTERM to the group; the job ended on Windows).
    fn terminate(&self) -> io::Result<()>;
    fn kill(&self);
}

pub type Pipe = Pin<Box<dyn AsyncRead + Send>>;

pub struct Started {
    pub pid: Option<u32>,
    pub stdout: Pipe,
    pub stderr: Pipe,
    pub wait: BoxFuture<'static, Exit>,
    pub tree: Arc<dyn TreeControl>,
}

pub trait Launcher: Send + Sync {
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
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "an argument contains a NUL"));
        }
        crate::os::spawn(command)
    }
}
