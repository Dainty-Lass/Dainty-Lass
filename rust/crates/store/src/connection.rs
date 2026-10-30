// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The connection every repository on two engines queries through (DD15). Its own module because the
//! derive puts `MultiBackend` and friends next to it.

use diesel::{PgConnection, SqliteConnection};

/// SQLite or Postgres, chosen by [`crate::Engine`], never by guessing from a URL: Diesel's
/// `establish` on this enum tries each variant in turn and hides libpq's error (DF7 spike).
#[derive(diesel::MultiConnection)]
pub enum AnyConnection {
    Sqlite(SqliteConnection),
    Postgres(PgConnection),
}

impl AnyConnection {
    pub fn engine(&self) -> crate::Engine {
        match self {
            Self::Sqlite(_) => crate::Engine::Sqlite,
            Self::Postgres(_) => crate::Engine::Postgres,
        }
    }
}
