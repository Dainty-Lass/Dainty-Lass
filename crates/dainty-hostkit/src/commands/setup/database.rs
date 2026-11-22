// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The backend's database: SQLite by default, Postgres when configured in `[database]`.

use crate::hostconfig::HostConfig;

/// The backend's database, as setup prepares it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Database {
    /// SQLite in `.workspace/config/dainty-lass.db` (default on the Rust backend).
    Sqlite,
    /// Postgres (`[database] engine = "postgres"` in `settings.toml`).
    Postgres,
}

/// The database setup prepares for this config.
pub fn database(config: &HostConfig) -> Database {
    if config
        .database
        .as_ref()
        .is_some_and(crate::hostconfig::DatabaseSettings::postgres)
    {
        Database::Postgres
    } else {
        Database::Sqlite
    }
}
