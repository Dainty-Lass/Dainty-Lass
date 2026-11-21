// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! What setup checks of the backend's database: SQLite by default, Postgres when configured in
//! `[database]`, or MongoDB under the `paradox` rollback feature.

use crate::hostconfig::{HostConfig, Platform};

/// The backend's database, as setup prepares it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Database {
    /// SQLite in `.workspace/config/dainty-lass.db` (default on the Rust backend).
    Sqlite,
    /// Postgres (`[database] engine = "postgres"` in `host.toml`).
    Postgres,
    /// MongoDB with a replica set (`host.toml [mongo]`), for the `paradox` rollback build.
    Mongo,
}

/// The database setup prepares for this config.
pub fn database(config: &HostConfig) -> Database {
    if cfg!(feature = "paradox") {
        return Database::Mongo;
    }
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

/// What setup says about the database at its end: nothing when it is ready to start.
pub fn database_notes(config: &HostConfig, platform: Platform) -> Vec<String> {
    match database(config) {
        Database::Sqlite | Database::Postgres => Vec::new(),
        Database::Mongo => {
            if !config.mongo.manage || config.mongo.mongod.is_some() {
                return Vec::new();
            }
            let linux = platform == Platform::Linux;
            vec![format!(
                "\n! MongoDB was not found. Install MongoDB Community Server ({}), then run setup again, or pass --mongod <path to {}>.",
                if linux {
                    "docs/hosting-linux.md"
                } else {
                    "https://www.mongodb.com/try/download/community"
                },
                if linux { "mongod" } else { "mongod.exe" }
            )]
        }
    }
}
