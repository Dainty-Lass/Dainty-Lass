// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The guarded actions for the server GUI (DD23 layer 4, DH6): what `dainty-host admin guarded` does, in the
//! GUI process, for a workspace. The decision is the host: it may approve or cancel what any admin key asked
//! (an owner no key can be), and the server carries an approved action out within 30 seconds.

use std::path::Path;

use serde_json::Value;

/// Why a call failed: a stable code (`not_found`, `not_waiting`, `bad_id`, `not_configured`, `failed`) and
/// the words, which are English and for the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// The stable code.
    pub code: &'static str,
    /// What happened, in English, for the log.
    pub message: String,
}

#[cfg(feature = "admin")]
mod run {
    use super::{Failure, Path, Value};
    use crate::paths::Paths;
    use dainty_admin::guard::Guards;
    use dainty_admin::guarded_cli::{self, record_json};

    impl From<guarded_cli::Failure> for Failure {
        fn from(f: guarded_cli::Failure) -> Self {
            Self {
                code: f.code,
                message: f.message,
            }
        }
    }

    /// Opens the store setup wrote `admin.env` for. Blocking: it migrates the schema.
    async fn open(workspace: &Path) -> Result<Guards, Failure> {
        let file = Paths::new(workspace).config.join("admin.env");
        if !file.is_file() {
            return Err(Failure {
                code: "not_configured",
                message: "no admin.env: run setup first".to_owned(),
            });
        }
        tokio::task::spawn_blocking(move || guarded_cli::open(Some(&file)))
            .await
            .map_err(|e| Failure {
                code: "failed",
                message: e.to_string(),
            })?
            .map_err(|message| Failure {
                code: "failed",
                message,
            })
    }

    /// The waiting actions, oldest first.
    pub async fn list(workspace: &Path) -> Result<Vec<Value>, Failure> {
        let guards = open(workspace).await?;
        Ok(guarded_cli::list(&guards, false)
            .await?
            .iter()
            .map(record_json)
            .collect())
    }

    /// Approves the waiting action `id`; returns its record as JSON.
    pub async fn approve(workspace: &Path, id: &str) -> Result<Value, Failure> {
        let guards = open(workspace).await?;
        Ok(record_json(&guarded_cli::approve(&guards, id).await?))
    }

    /// Cancels the waiting action `id`; returns its record as JSON.
    pub async fn cancel(workspace: &Path, id: &str) -> Result<Value, Failure> {
        let guards = open(workspace).await?;
        Ok(record_json(&guarded_cli::cancel(&guards, id).await?))
    }
}

#[cfg(feature = "admin")]
pub use run::{approve, cancel, list};

#[cfg(not(feature = "admin"))]
mod absent {
    use super::{Failure, Path, Value};

    fn built_without() -> Failure {
        Failure {
            code: "not_configured",
            message: "this build has no admin API".to_owned(),
        }
    }

    /// Fails: this build has no admin API.
    pub async fn list(_workspace: &Path) -> Result<Vec<Value>, Failure> {
        Err(built_without())
    }

    /// Fails: this build has no admin API.
    pub async fn approve(_workspace: &Path, _id: &str) -> Result<Value, Failure> {
        Err(built_without())
    }

    /// Fails: this build has no admin API.
    pub async fn cancel(_workspace: &Path, _id: &str) -> Result<Value, Failure> {
        Err(built_without())
    }
}

#[cfg(not(feature = "admin"))]
pub use absent::{approve, cancel, list};
