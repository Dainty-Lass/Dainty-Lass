// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Utilities for reading, hashing, inspecting, and replaying Drizzle Kit SQLite migrations.

use std::fmt::Write as _;
use std::path::Path;

use diesel::connection::{Connection as _, SimpleConnection};
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Nullable, Text};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{Connection, Error};

/// A single Drizzle Kit migration loaded from disk or constructed for replay.
#[derive(Debug, Clone)]
pub struct Migration {
    pub tag: String,
    pub created_at: i64,
    pub sql: String,
}

/// Reads Drizzle Kit migrations from `dir/meta/_journal.json` and `dir/<tag>.sql`.
pub fn read_folder(dir: &Path) -> Result<Vec<Migration>, Error> {
    let journal_path = dir.join("meta").join("_journal.json");
    let raw = std::fs::read_to_string(&journal_path)
        .map_err(|e| Error::Migration(format!("{}: {e}", journal_path.display())))?;
    let root: Value =
        serde_json::from_str(&raw).map_err(|e| Error::Migration(format!("{}: {e}", journal_path.display())))?;
    let entries = root
        .as_object()
        .and_then(|obj| obj.get("entries"))
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Migration(format!("{}: missing 'entries' array", journal_path.display())))?;

    let mut migrations = Vec::with_capacity(entries.len());
    for item in entries {
        let obj = item
            .as_object()
            .ok_or_else(|| Error::Migration(format!("{}: journal entry is not an object", journal_path.display())))?;
        let tag = obj
            .get("tag")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Migration(format!("{}: entry missing string 'tag'", journal_path.display())))?
            .to_owned();
        let created_at = obj
            .get("when")
            .and_then(Value::as_i64)
            .ok_or_else(|| Error::Migration(format!("{}: entry missing integer 'when'", journal_path.display())))?;
        let sql_path = dir.join(format!("{tag}.sql"));
        let sql =
            std::fs::read_to_string(&sql_path).map_err(|e| Error::Migration(format!("{}: {e}", sql_path.display())))?;
        migrations.push(Migration { tag, created_at, sql });
    }
    Ok(migrations)
}

/// Returns the 64-character lowercase hexadecimal SHA-256 digest of `sql`.
pub fn hash(sql: &str) -> String {
    let digest = Sha256::digest(sql.as_bytes());
    let mut hex = String::with_capacity(64);
    for byte in digest {
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// Checks whether `recorded` matches the SHA-256 hex digest of `sql` with LF or CRLF line endings.
pub fn hash_matches(recorded: &str, sql: &str) -> bool {
    let lf = sql.replace("\r\n", "\n");
    if hash(&lf) == recorded {
        return true;
    }
    let crlf = lf.replace('\n', "\r\n");
    hash(&crlf) == recorded
}

#[derive(QueryableByName)]
struct AppliedRow {
    #[diesel(sql_type = Nullable<BigInt>)]
    created_at: Option<i64>,
    #[diesel(sql_type = Text)]
    hash: String,
}

/// Returns the `(created_at, hash)` of the most recent entry in `__drizzle_migrations`, if any.
pub fn last_applied(conn: &mut Connection) -> Result<Option<(Option<i64>, String)>, Error> {
    let row = diesel::sql_query(
        "SELECT CAST(created_at AS BIGINT) AS created_at, hash
         FROM \"__drizzle_migrations\"
         ORDER BY created_at DESC, rowid DESC
         LIMIT 1",
    )
    .get_result::<AppliedRow>(conn)
    .optional()?;
    Ok(row.map(|r| (r.created_at, r.hash)))
}

/// Replays unapplied Drizzle `migrations` against `conn` and records them in `__drizzle_migrations`.
pub fn replay(conn: &mut Connection, migrations: &[Migration]) -> Result<(), Error> {
    conn.batch_execute(
        "CREATE TABLE IF NOT EXISTS \"__drizzle_migrations\" (
            id SERIAL PRIMARY KEY,
            hash text NOT NULL,
            created_at numeric
        );",
    )?;
    let highest = last_applied(conn)?.and_then(|(ts, _)| ts);

    conn.transaction::<_, Error, _>(|conn| {
        for migration in migrations {
            if highest.is_some_and(|ts| migration.created_at <= ts) {
                continue;
            }
            for part in migration.sql.split("--> statement-breakpoint") {
                let statement = part.trim();
                if !statement.is_empty() {
                    conn.batch_execute(statement)?;
                }
            }
            diesel::sql_query("INSERT INTO \"__drizzle_migrations\" (hash, created_at) VALUES ($1, $2)")
                .bind::<Text, _>(hash(&migration.sql))
                .bind::<BigInt, _>(migration.created_at)
                .execute(conn)?;
        }
        Ok(())
    })
}
