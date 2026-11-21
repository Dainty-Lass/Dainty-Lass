// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The four ways an import fails (spec §9), and how SQLite's text sorts into them (§6).

use std::fmt::Display;
use std::path::Path;

use dainty_module_api::{ErrorKind, ModuleError};

/// The usage text, appended to every usage error after a newline (spec §3).
pub const USAGE: &str = "usage:
  import revived      <database> (--to <1.14.7 database> | --env-file <backend env file>) [--node <id>] [--dry-run]
  import offline-save <.drsave or database> (--to <file> | --env-file <file>) [--node <id>] [--dry-run]
  import reforged     <checkout> [--to <file> | --env-file <file>] [--fresh-start] [--db-tool <program>] [--dry-run]
every path is absolute";

/// Why an import stopped. No message holds a value read from the data other than a report's ids and names.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImportError {
    /// The source (or a file named with it) is not one the importer takes.
    #[error("{0}")]
    Invalid(String),
    /// The 1.14.7 database refused or failed.
    #[error("target database: {0}")]
    Target(String),
    /// Reading or writing a file or running a program failed; the text names the path where there is one.
    #[error("{0}")]
    Io(String),
    /// The arguments are wrong.
    #[error("{0}\n{USAGE}")]
    Usage(String),
}

impl ImportError {
    /// `<path>: <error>`.
    pub(crate) fn io(path: &Path, error: impl Display) -> Self {
        Self::Io(format!("{}: {error}", path.display()))
    }

    /// A refusal of sections 5 and 6.
    pub(crate) fn not_1_4_4(reason: impl Display) -> Self {
        Self::Invalid(format!("not a Dauntless Revived 1.4.4 database: {reason}"))
    }

    /// A SQLite error while copying, checking or reading a 1.4.4 database (§6): the machine's fault is I/O,
    /// anything else the file's.
    pub(crate) fn sqlite(path: &Path, text: impl Display) -> Self {
        let text = text.to_string();
        if ["disk I/O error", "database or disk is full", "unable to open"]
            .iter()
            .any(|t| text.contains(t))
        {
            Self::io(path, text)
        } else {
            Self::Invalid(format!("not a readable 1.4.4 database ({text})"))
        }
    }

    /// The module error kind the console sees: invalid and usage are the caller's, the rest the server's.
    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::Invalid(_) | Self::Usage(_) => ErrorKind::Invalid,
            Self::Target(_) | Self::Io(_) => ErrorKind::Failed,
        }
    }
}

impl From<ImportError> for ModuleError {
    fn from(error: ImportError) -> Self {
        ModuleError::new(error.kind(), error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_and_texts() {
        let path = Path::new("/srv/a.db");
        assert_eq!(
            ImportError::sqlite(path, "disk I/O error").to_string(),
            format!("{}: disk I/O error", path.display())
        );
        let invalid = ImportError::sqlite(path, "file is not a database");
        assert_eq!(
            invalid.to_string(),
            "not a readable 1.4.4 database (file is not a database)"
        );
        assert_eq!(invalid.kind(), ErrorKind::Invalid);
        assert_eq!(ImportError::Target("x".into()).to_string(), "target database: x");
        assert_eq!(ImportError::Target("x".into()).kind(), ErrorKind::Failed);
        let usage = ImportError::Usage("one source only".into()).to_string();
        assert!(usage.starts_with("one source only\nusage:\n"), "{usage}");
        assert!(usage.ends_with("every path is absolute"));
    }
}
