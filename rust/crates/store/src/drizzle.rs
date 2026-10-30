// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Drizzle Kit migration inspection, hashing, and replay.

use std::path::Path;

use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Nullable, Text};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::{Connection, Error};

#[derive(Debug, Clone)]
pub struct Migration {
    pub tag: String,
    pub created_at: i64,
    pub sql: String,
}

#[derive(Deserialize)]
struct Journal {
    entries: Vec<JournalEntry>,
}

#[derive(Deserialize)]
struct JournalEntry {
    tag: String,
    when: i64,
}

pub fn read_folder(dir: &Path) -> Result<Vec<Migration>, Error> {
    let journal_path = dir.join("meta").join("_journal.json");
    let raw = std::fs::read_to_string(&journal_path)
        .map_err(|e| Error::Migration(format!("{}: {e}", journal_path.display())))?;
    let journal: Journal =
        serde_json::from_str(&raw).map_err(|e| Error::Migration(format!("{}: {e}", journal_path.display())))?;

    let mut migrations = Vec::with_capacity(journal.entries.len());
    for entry in journal.entries {
        let sql_path = dir.join(format!("{}.sql", entry.tag));
        let sql =
            std::fs::read_to_string(&sql_path).map_err(|e| Error::Migration(format!("{}: {e}", sql_path.display())))?;
        migrations.push(Migration {
            tag: entry.tag,
            created_at: entry.when,
            sql,
        });
    }
    Ok(migrations)
}

pub fn hash(sql: &str) -> String {
    let digest = Sha256::digest(sql.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn hash_matches(recorded: &str, sql: &str) -> bool {
    let lf = sql.replace("\r\n", "\n");
    if recorded == hash(&lf) {
        return true;
    }
    let crlf = lf.replace('\n', "\r\n");
    recorded == hash(&crlf)
}

#[derive(QueryableByName)]
struct LastAppliedRow {
    #[diesel(sql_type = Nullable<BigInt>)]
    created_at: Option<i64>,
    #[diesel(sql_type = Text)]
    hash: String,
}

pub fn last_applied(conn: &mut Connection) -> Result<Option<(Option<i64>, String)>, Error> {
    let rows = diesel::sql_query(
        "SELECT CAST(created_at AS INTEGER) AS created_at, hash FROM __drizzle_migrations ORDER BY created_at DESC LIMIT 1",
    )
    .load::<LastAppliedRow>(conn)?;
    Ok(rows.into_iter().next().map(|r| (r.created_at, r.hash)))
}

pub fn replay(conn: &mut Connection, migrations: &[Migration]) -> Result<(), Error> {
    conn.batch_execute(
        "CREATE TABLE IF NOT EXISTS \"__drizzle_migrations\" (
            id SERIAL PRIMARY KEY,
            hash text NOT NULL,
            created_at numeric
        );",
    )?;
    let last = last_applied(conn)?.and_then(|(at, _)| at);
    conn.immediate_transaction(|conn| {
        for m in migrations {
            if last.is_some_and(|l| m.created_at <= l) {
                continue;
            }
            for stmt in m.sql.split("--> statement-breakpoint") {
                let trimmed = stmt.trim();
                if !trimmed.is_empty() {
                    conn.batch_execute(trimmed)?;
                }
            }
            let h = hash(&m.sql);
            diesel::sql_query("INSERT INTO \"__drizzle_migrations\" (hash, created_at) VALUES ($1, $2)")
                .bind::<Text, _>(h)
                .bind::<BigInt, _>(m.created_at)
                .execute(conn)?;
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use diesel::connection::Connection as _;

    use super::*;

    #[test]
    fn sha256_hash_and_line_ending_matching() {
        let lf = "CREATE TABLE t (id INTEGER PRIMARY KEY);\n";
        let crlf = "CREATE TABLE t (id INTEGER PRIMARY KEY);\r\n";
        let h_lf = hash(lf);
        let h_crlf = hash(crlf);
        assert_eq!(h_lf.len(), 64);
        assert_ne!(h_lf, h_crlf);
        assert!(hash_matches(&h_lf, lf));
        assert!(hash_matches(&h_lf, crlf));
        assert!(hash_matches(&h_crlf, lf));
        assert!(hash_matches(&h_crlf, crlf));
        assert!(!hash_matches("0000", lf));
    }

    #[test]
    fn read_folder_and_replay_migrations() {
        let dir = tempfile::tempdir().unwrap();
        let meta = dir.path().join("meta");
        std::fs::create_dir_all(&meta).unwrap();
        std::fs::write(
            meta.join("_journal.json"),
            r#"{"entries":[{"tag":"0000_init","when":1000},{"tag":"0001_more","when":2000}]}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("0000_init.sql"),
            "CREATE TABLE a (id INTEGER PRIMARY KEY);\n--> statement-breakpoint\nCREATE TABLE b (id INTEGER PRIMARY KEY);\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("0001_more.sql"), "INSERT INTO a (id) VALUES (1);\n").unwrap();

        let migrations = read_folder(dir.path()).unwrap();
        assert_eq!(migrations.len(), 2);
        assert_eq!(migrations[0].tag, "0000_init");
        assert_eq!(migrations[0].created_at, 1000);

        let mut conn = Connection::establish(":memory:").unwrap();
        replay(&mut conn, &migrations[..1]).unwrap();
        let (at, h) = last_applied(&mut conn).unwrap().unwrap();
        assert_eq!(at, Some(1000));
        assert!(hash_matches(&h, &migrations[0].sql));

        replay(&mut conn, &migrations).unwrap();
        let (at2, h2) = last_applied(&mut conn).unwrap().unwrap();
        assert_eq!(at2, Some(2000));
        assert!(hash_matches(&h2, &migrations[1].sql));
    }
}
