// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The one error type of the host kit. `main` prints it as `\nError: <message>\n` on stderr
//! and exits with code 1.

use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    /// A command or helper the skeleton (RH1) registered and a later row fills.
    #[error("{what} is not written yet in reforged-host (MIGRATION_PLAN.md {row})")]
    NotImplemented { what: &'static str, row: &'static str },
    /// A problem told to the host.
    #[error("{0}")]
    Message(String),
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl HostError {
    pub fn message(text: impl Into<String>) -> Self {
        Self::Message(text.into())
    }

    pub fn io(path: impl AsRef<Path>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.as_ref().to_path_buf(),
            source,
        }
    }
}

impl From<std::io::Error> for HostError {
    fn from(source: std::io::Error) -> Self {
        Self::Message(source.to_string())
    }
}

impl PartialEq for HostError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::NotImplemented { what: w1, row: r1 }, Self::NotImplemented { what: w2, row: r2 }) => {
                w1 == w2 && r1 == r2
            }
            (Self::Message(a), Self::Message(b)) => a == b,
            (Self::Io { path: p1, source: s1 }, Self::Io { path: p2, source: s2 }) => {
                p1 == p2 && s1.kind() == s2.kind() && s1.to_string() == s2.to_string()
            }
            _ => false,
        }
    }
}

impl Eq for HostError {}

/// What a stub returns: `what` names the command or helper, `row` the MIGRATION_PLAN.md row that fills it.
pub fn not_implemented<T>(what: &'static str, row: &'static str) -> Result<T, HostError> {
    Err(HostError::NotImplemented { what, row })
}
