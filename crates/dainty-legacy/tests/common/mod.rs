// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Fixtures built from the spec (§13), never from real 1.4.4 SQL: a 1.4.4 database with the columns of §6
//! for a chosen prefix and a history written from revived-1.4.4.toml, and a 1.14.7 target rustic-isles'
//! own migrations make.

#![allow(dead_code, reason = "each test file uses some of the fixtures")]
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::path::{Path, PathBuf};

use dainty_legacy::{ImportArgs, ImportError, Kind, Report, Target};
use dainty_store::{Database, DatabaseConfig, DatabaseOptions};
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Nullable, Text};
use diesel_migrations::{EmbeddedMigrations, MigrationHarness};
use sha2::{Digest, Sha256};

pub const KEY: &str = "UUK_0123456789abcdef0123456789abcdef0123456789abcdef";

pub fn open(path: &Path) -> SqliteConnection {
    SqliteConnection::establish(&path.to_string_lossy()).unwrap()
}

pub fn exec(conn: &mut SqliteConnection, sql: &str) {
    conn.batch_execute(sql).unwrap_or_else(|e| panic!("{sql}: {e}"));
}

/// A 1.4.4 database at `prefix` (1 to 31), with Drizzle's history or, with `diesel`, the Rust metagame's.
pub fn revived(path: &Path, prefix: usize, diesel: bool) -> SqliteConnection {
    let data = dainty_legacy::data::revived().unwrap();
    let mut conn = open(path);
    let has = |table: &str| data.has_table(prefix, table);
    let col = |table: &str, column: &str| data.has_column(prefix, table, column);
    exec(
        &mut conn,
        &format!(
            "CREATE TABLE users (\"userId\" TEXT PRIMARY KEY, \"name\" TEXT NOT NULL, \"notes\" INTEGER NOT NULL \
             DEFAULT 0{})",
            if col("users", "isAdmin") {
                ", \"isAdmin\" INTEGER NOT NULL DEFAULT 0"
            } else {
                ""
            }
        ),
    );
    if has("userapikeys") {
        exec(
            &mut conn,
            "CREATE TABLE userapikeys (\"userId\" TEXT NOT NULL, \"keyHash\" TEXT NOT NULL)",
        );
    }
    if has("invitecodes") {
        exec(
            &mut conn,
            "CREATE TABLE invitecodes (\"invitecode\" TEXT PRIMARY KEY, \"usesRemaining\" INTEGER NOT NULL, \
             \"infiniteUses\" INTEGER NOT NULL)",
        );
    }
    if has("discordlinks") {
        exec(
            &mut conn,
            "CREATE TABLE discordlinks (\"discordId\" TEXT PRIMARY KEY, \"userId\" TEXT NOT NULL, \
             \"linkedAt\" TEXT NOT NULL)",
        );
    }
    if has("account_bans") {
        exec(
            &mut conn,
            &format!(
                "CREATE TABLE account_bans (id INTEGER PRIMARY KEY, \"account_id\" TEXT NOT NULL, \"reason\" TEXT, \
                 \"active\" INTEGER NOT NULL{})",
                if col("account_bans", "expires_at") {
                    ", \"expires_at\" INTEGER"
                } else {
                    ""
                }
            ),
        );
    }
    if diesel {
        conn.run_pending_migrations(EmbeddedMigrations::new(&[])).unwrap();
        exec(
            &mut conn,
            "INSERT INTO __diesel_schema_migrations (version) VALUES ('0001')",
        );
    } else {
        drizzle_history(&mut conn, prefix);
    }
    conn
}

/// `__drizzle_migrations` as the store's replay makes it, with the first `rows` entries.
pub fn drizzle_history(conn: &mut SqliteConnection, rows: usize) {
    let data = dainty_legacy::data::revived().unwrap();
    dainty_store::drizzle::replay(conn, &[]).unwrap();
    for entry in data.drizzle.iter().take(rows) {
        diesel::sql_query("INSERT INTO \"__drizzle_migrations\" (hash, created_at) VALUES (?, ?)")
            .bind::<Text, _>(&entry.sha256_lf)
            .bind::<BigInt, _>(entry.when)
            .execute(conn)
            .unwrap();
    }
}

pub fn user(conn: &mut SqliteConnection, user_id: &str, name: &str) {
    diesel::sql_query("INSERT INTO users (\"userId\", \"name\") VALUES (?, ?)")
        .bind::<Text, _>(user_id)
        .bind::<Text, _>(name)
        .execute(conn)
        .unwrap();
}

pub fn api_key(conn: &mut SqliteConnection, user_id: &str, key: &str) {
    diesel::sql_query("INSERT INTO userapikeys (\"userId\", \"keyHash\") VALUES (?, ?)")
        .bind::<Text, _>(user_id)
        .bind::<Text, _>(hex::encode(Sha256::digest(key.as_bytes())))
        .execute(conn)
        .unwrap();
}

/// A 1.14.7 database made by rustic-isles' migrations, as the backend leaves it.
pub fn target(dir: &Path) -> PathBuf {
    let file = dir.join("isles.db");
    let config = DatabaseConfig::Sqlite { file: file.clone() };
    drop(Database::open(&config, &DatabaseOptions::default(), &rustic_isles_store::SCHEMA).unwrap());
    file
}

#[derive(QueryableByName, Debug, PartialEq, Eq)]
pub struct Launcher {
    #[diesel(sql_type = Text)]
    pub display_name: String,
    #[diesel(sql_type = Text)]
    pub status: String,
    #[diesel(sql_type = Nullable<Text>)]
    pub approval_status: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub approval_updated_by: Option<String>,
    #[diesel(sql_type = Text)]
    pub roles: String,
    #[diesel(sql_type = Nullable<Text>)]
    pub timeout_until: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    pub timeout_reason: Option<String>,
}

pub fn launcher(target: &Path, account_id: &str) -> Option<Launcher> {
    diesel::sql_query(
        "SELECT display_name, status, approval_status, approval_updated_by, roles, timeout_until, timeout_reason \
         FROM launcher_accounts WHERE account_id = ?",
    )
    .bind::<Text, _>(account_id)
    .get_result(&mut open(target))
    .optional()
    .unwrap()
}

#[derive(QueryableByName)]
struct Count {
    #[diesel(sql_type = BigInt)]
    n: i64,
}

pub fn count(path: &Path, sql: &str) -> i64 {
    diesel::sql_query(sql).get_result::<Count>(&mut open(path)).unwrap().n
}

pub fn args(kind: Kind, source: &Path, target: &Path) -> ImportArgs {
    ImportArgs {
        kind,
        source: source.to_path_buf(),
        target: Some(Target::To(target.to_path_buf())),
        node: None,
        dry_run: false,
        fresh_start: false,
        db_tool: None,
    }
}

pub async fn import(args: ImportArgs) -> Result<Report, ImportError> {
    dainty_legacy::run(args, "local").await
}

pub fn n(report: &Report, what: &str) -> u64 {
    report.imported.get(what).copied().unwrap_or(0)
}

pub fn same(report: &Report, what: &str) -> u64 {
    report.unchanged.get(what).copied().unwrap_or(0)
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
