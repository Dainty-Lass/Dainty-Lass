// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! What off-server shipping and restore (DD23, DH4) need from a database: a snapshot of it while it is in
//! use, and a restore that loads a snapshot into either engine and replays the change log after it.
//!
//! A SQLite snapshot is the online backup of the file ([`crate::backup`]); a Postgres one is a logical copy,
//! JSON Lines read in one `REPEATABLE READ` snapshot: a header, then every row of every table, the log last.
//! A restore reads either kind as logical rows into a fresh database built by this build's migrations, so a
//! snapshot from the bucket brings rows and nothing else (no triggers or views of its own). The log goes in
//! as it was, ids included; its `migrate` entries run the migration where they fall. Nothing is logged
//! while it runs: `Database::open` at the end applies newer migrations and puts the triggers back.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use diesel::connection::{Connection as _, SimpleConnection};
use diesel::pg::Pg;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Text};
use diesel::sqlite::Sqlite;
use diesel_migrations::MigrationHarness;
use serde::{Deserialize, Serialize};

use crate::connection::AnyConnection;
use crate::database::{applied_versions, now_ms, run_blocking, versions};
use crate::history::{self, Break, CHANGE_LOG, Chain, ChangeEntry, Column, Head};
use crate::portable::{quote_ident, quote_literal};
use crate::transfer::{PAGE, Row, fix_sequences, insert_row, is_blob, ordered, select_rows};
use crate::{Database, DatabaseConfig, DatabaseOptions, DatabaseSchema, Engine, Error};

/// The first line of a logical snapshot names this format.
pub const LOGICAL_FORMAT: &str = "dainty-lass-snapshot";
const LOGICAL_VERSION: u32 = 1;
/// How long the online backup of a SQLite database may take (the Node kit's 15 minutes).
const BACKUP_DEADLINE: Duration = Duration::from_secs(15 * 60);
const SQLITE_MAGIC: &[u8] = b"SQLite format 3\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotKind {
    /// A SQLite database file.
    Sqlite,
    /// JSON Lines: a [`LogicalHeader`], then `{"table":..,"row":{..}}` per row.
    Logical,
}

impl SnapshotKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::Logical => "logical",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "sqlite" => Some(Self::Sqlite),
            "logical" => Some(Self::Logical),
            _ => None,
        }
    }
}

/// A whole database at one moment, uncompressed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub kind: SnapshotKind,
    /// The newest change-log entry it holds (0: none). A restore replays the entries after it.
    pub last_id: i64,
    /// Unix milliseconds once the copy's view was fixed: every entry it holds is older.
    pub taken_at: i64,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogicalHeader {
    pub format: String,
    pub version: u32,
    /// The engine it was read from.
    pub engine: String,
    /// The migrations applied, oldest first.
    pub migrations: Vec<String>,
    pub last_id: i64,
    pub taken_at: i64,
}

#[derive(Deserialize)]
struct LogicalLine {
    table: String,
    row: serde_json::Value,
}

impl Database {
    /// A snapshot of the database while it is in use: SQLite's online backup, or a logical copy of Postgres.
    pub async fn snapshot(&self) -> Result<Snapshot, Error> {
        match self.config().clone() {
            DatabaseConfig::Sqlite { file } => {
                let busy_timeout = self.busy_timeout();
                let clock = self.clock();
                run_blocking(move || sqlite_snapshot(&file, busy_timeout, clock)).await
            }
            DatabaseConfig::Postgres(_) => {
                let clock = self.clock();
                self.read(move |conn| {
                    conn.batch_execute("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")?;
                    let mut data = Vec::new();
                    let header = write_logical(conn, clock, &mut data)?;
                    Ok::<_, Error>(Snapshot {
                        kind: SnapshotKind::Logical,
                        last_id: header.last_id,
                        taken_at: header.taken_at,
                        data,
                    })
                })
                .await
            }
        }
    }
}

fn sqlite_snapshot(file: &Path, busy_timeout: Duration, clock: fn() -> i64) -> Result<Snapshot, Error> {
    let mut name = file.as_os_str().to_owned();
    name.push(format!(".snapshot-{}-{}", std::process::id(), now_ms()));
    let copy = PathBuf::from(name);
    crate::backup::backup_file(file, &copy, busy_timeout, BACKUP_DEADLINE)?;
    // Taken once the backup's read has ended, so it is later than every entry the copy holds.
    let taken_at = clock();
    let result = (|| {
        let data = std::fs::read(&copy).map_err(|e| Error::Backup(e.to_string()))?;
        let conn = rusqlite::Connection::open_with_flags(&copy, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| Error::Backup(e.to_string()))?;
        let last_id: i64 = conn
            .query_row(
                &format!("SELECT coalesce(max(id), 0) FROM {}", quote_ident(CHANGE_LOG)),
                [],
                |row| row.get(0),
            )
            .map_err(|e| Error::Backup(e.to_string()))?;
        Ok(Snapshot {
            kind: SnapshotKind::Sqlite,
            last_id,
            taken_at,
            data,
        })
    })();
    let _ = std::fs::remove_file(&copy);
    result
}

/// Every table's rows, parents first, then the log, as JSON Lines after a header. `conn` is in the
/// transaction whose view this copies.
fn write_logical(conn: &mut AnyConnection, clock: fn() -> i64, out: &mut Vec<u8>) -> Result<LogicalHeader, Error> {
    let migrations = applied_versions(conn)?;
    // After the first read, which fixed the snapshot's view.
    let taken_at = clock();
    let last_id = history::last_change(conn)?.map_or(0, |(id, _)| id);
    let header = LogicalHeader {
        format: LOGICAL_FORMAT.into(),
        version: LOGICAL_VERSION,
        engine: conn.engine().as_str().into(),
        migrations,
        last_id,
        taken_at,
    };
    let json_error = |e: serde_json::Error| Error::Backup(e.to_string());
    serde_json::to_writer(&mut *out, &header).map_err(json_error)?;
    out.push(b'\n');
    let tables = history::user_tables(conn)?;
    let tables = ordered(conn, tables)?;
    for table in tables.iter().map(String::as_str).chain([CHANGE_LOG]) {
        let key = history::primary_key(conn, table)?;
        let columns = history::columns(conn, table)?;
        let select = select_rows(conn, table, &columns, &key);
        let name = serde_json::to_string(table).map_err(json_error)?;
        let mut read = 0i64;
        loop {
            let rows = diesel::sql_query(&select)
                .bind::<BigInt, _>(PAGE)
                .bind::<BigInt, _>(read)
                .load::<Row>(conn)?;
            for row in &rows {
                out.extend_from_slice(format!("{{\"table\":{name},\"row\":{}}}\n", row.row).as_bytes());
            }
            read += i64::try_from(rows.len()).unwrap_or(i64::MAX);
            if i64::try_from(rows.len()).unwrap_or(i64::MAX) < PAGE {
                break;
            }
        }
    }
    Ok(header)
}

// ------------------------------------------------------------------ migrations, one at a time

fn source_versions(engine: Engine, schema: &DatabaseSchema) -> Result<Vec<String>, Error> {
    let mut out = match engine {
        Engine::Sqlite => versions::<Sqlite>(&(schema.sqlite)())?,
        Engine::Postgres => versions::<Pg>(&(schema.postgres)())?,
    };
    out.sort();
    Ok(out)
}

fn applied(conn: &mut AnyConnection) -> Result<Vec<String>, Error> {
    let found = match conn {
        AnyConnection::Sqlite(c) => MigrationHarness::<Sqlite>::applied_migrations(c),
        AnyConnection::Postgres(c) => MigrationHarness::<Pg>::applied_migrations(c),
    }
    .map_err(|e| Error::Migration(e.to_string()))?;
    let mut out: Vec<String> = found.iter().map(ToString::to_string).collect();
    out.sort();
    Ok(out)
}

fn run_next(conn: &mut AnyConnection, schema: &DatabaseSchema) -> Result<(), Error> {
    match conn {
        AnyConnection::Sqlite(c) => c.run_next_migration((schema.sqlite)()).map(|_| ()),
        AnyConnection::Postgres(c) => c.run_next_migration((schema.postgres)()).map(|_| ()),
    }
    .map_err(|e| Error::Migration(e.to_string()))
}

/// Runs this build's migrations, oldest first, until `version` is applied.
fn migrate_through(conn: &mut AnyConnection, schema: &DatabaseSchema, version: &str) -> Result<(), Error> {
    if !source_versions(conn.engine(), schema)?.iter().any(|v| v == version) {
        return Err(Error::Restore(format!(
            "the backup needs migration {version}, which this build doesn't have: restore with a newer build"
        )));
    }
    while !applied(conn)?.iter().any(|v| v == version) {
        run_next(conn, schema)?;
    }
    Ok(())
}

/// Brings a fresh database to exactly the migrations a snapshot was at.
fn migrate_to(conn: &mut AnyConnection, schema: &DatabaseSchema, wanted: &[String]) -> Result<(), Error> {
    let mut wanted = wanted.to_vec();
    wanted.sort();
    if let Some(last) = wanted.last() {
        migrate_through(conn, schema, last)?;
    }
    let now = applied(conn)?;
    if now != wanted {
        return Err(Error::Restore(format!(
            "the snapshot is at the migrations {wanted:?}, which this build's ({now:?}) don't reproduce"
        )));
    }
    Ok(())
}

// ------------------------------------------------------------------ loading rows

/// Per table: its columns and key, and the statements that write it.
struct Table {
    columns: Vec<Column>,
    names: HashSet<String>,
    insert: String,
    update: String,
    delete: String,
}

fn json_path(column: &str) -> String {
    quote_literal(&format!("$.{}", quote_ident(column)))
}

/// SQLite: a column's value out of the JSON parameter `$n`.
fn sqlite_value(column: &Column, param: usize) -> String {
    let path = json_path(&column.name);
    if is_blob(column) {
        format!("unhex(substr(json_extract(${param}, {path}), 3))")
    } else {
        format!("json_extract(${param}, {path})")
    }
}

impl Table {
    fn read(conn: &mut AnyConnection, table: &str) -> Result<Self, Error> {
        let columns = history::columns(conn, table)?;
        let key = history::primary_key(conn, table)?;
        if columns.is_empty() || key.is_empty() {
            return Err(Error::Restore(format!("no table {table} with a primary key here")));
        }
        let t = quote_ident(table);
        let key_columns: Vec<&Column> = columns.iter().filter(|c| key.contains(&c.name)).collect();
        let (update, delete) = match conn.engine() {
            Engine::Sqlite => {
                let set: Vec<String> = columns
                    .iter()
                    .map(|c| format!("{} = {}", quote_ident(&c.name), sqlite_value(c, 1)))
                    .collect();
                let matches = |param| {
                    key_columns
                        .iter()
                        .map(|c| format!("{} = {}", quote_ident(&c.name), sqlite_value(c, param)))
                        .collect::<Vec<_>>()
                        .join(" AND ")
                };
                (
                    format!("UPDATE {t} SET {} WHERE {}", set.join(", "), matches(2)),
                    format!("DELETE FROM {t} WHERE {}", matches(1)),
                )
            }
            Engine::Postgres => {
                let set: Vec<String> = columns
                    .iter()
                    .map(|c| format!("{0} = \"__after\".{0}", quote_ident(&c.name)))
                    .collect();
                let matches = key_columns
                    .iter()
                    .map(|c| format!("{t}.{0} = \"__key\".{0}", quote_ident(&c.name)))
                    .collect::<Vec<_>>()
                    .join(" AND ");
                (
                    format!(
                        "UPDATE {t} SET {} FROM jsonb_populate_record(NULL::{t}, $1::jsonb) AS \"__after\",
                         jsonb_populate_record(NULL::{t}, $2::jsonb) AS \"__key\" WHERE {matches}",
                        set.join(", ")
                    ),
                    format!(
                        "DELETE FROM {t} USING jsonb_populate_record(NULL::{t}, $1::jsonb) AS \"__key\" WHERE {matches}"
                    ),
                )
            }
        };
        Ok(Self {
            insert: insert_row(conn, table, &columns),
            names: columns.iter().map(|c| c.name.clone()).collect(),
            columns,
            update,
            delete,
        })
    }

    /// A row from a snapshot or the log must name exactly this table's columns.
    fn check(&self, table: &str, row: &serde_json::Value) -> Result<(), Error> {
        let keys: HashSet<String> = row
            .as_object()
            .ok_or_else(|| Error::Restore(format!("a row of {table} is not a JSON object")))?
            .keys()
            .cloned()
            .collect();
        if keys != self.names {
            let mut have: Vec<_> = keys.into_iter().collect();
            have.sort();
            let mut want: Vec<_> = self.columns.iter().map(|c| c.name.clone()).collect();
            want.sort();
            return Err(Error::Restore(format!(
                "a row of {table} has the columns {have:?}, the table here {want:?}"
            )));
        }
        Ok(())
    }
}

struct Loader {
    tables: HashMap<String, Table>,
}

impl Loader {
    fn table(&mut self, conn: &mut AnyConnection, name: &str) -> Result<&Table, Error> {
        if !self.tables.contains_key(name) {
            let known = name == CHANGE_LOG || history::user_tables(conn)?.iter().any(|t| t == name);
            if !known {
                return Err(Error::Restore(format!("there is no table {name} at this point")));
            }
            let table = Table::read(conn, name)?;
            self.tables.insert(name.to_owned(), table);
        }
        self.tables
            .get(name)
            .ok_or_else(|| Error::Restore(format!("there is no table {name}")))
    }

    fn insert(&mut self, conn: &mut AnyConnection, name: &str, row: &serde_json::Value) -> Result<(), Error> {
        let table = self.table(conn, name)?;
        table.check(name, row)?;
        let sql = table.insert.clone();
        let n = diesel::sql_query(sql).bind::<Text, _>(row.to_string()).execute(conn)?;
        if n != 1 {
            return Err(Error::Restore(format!("a row of {name} was not inserted")));
        }
        Ok(())
    }
}

/// Reads a SQLite snapshot's rows as a logical one. The file came from a bucket: nothing in its schema runs.
fn sqlite_to_logical(data: &[u8], scratch: &Path) -> Result<Vec<u8>, Error> {
    if !data.starts_with(SQLITE_MAGIC) {
        return Err(Error::Restore("the snapshot is not a SQLite database".into()));
    }
    let file = scratch.join(format!("restore-{}-{}.db", std::process::id(), now_ms()));
    std::fs::write(&file, data).map_err(|e| Error::Restore(format!("{}: {e}", file.display())))?;
    let result = (|| {
        let mut conn = AnyConnection::Sqlite(SqliteConnection::establish(&file.to_string_lossy())?);
        conn.batch_execute("PRAGMA trusted_schema = OFF; PRAGMA cell_size_check = ON; PRAGMA query_only = ON;")?;
        let mut out = Vec::new();
        conn.transaction(|conn| write_logical(conn, now_ms, &mut out))?;
        Ok(out)
    })();
    let _ = std::fs::remove_file(&file);
    result
}

/// Loads a logical snapshot and checks its log against the hash chain from genesis (DH5), so a snapshot that
/// was edited names the entry. Returns the snapshot's head.
fn load_logical(
    conn: &mut AnyConnection,
    schema: &DatabaseSchema,
    data: &[u8],
    trusted: Option<&Head>,
) -> Result<(LogicalHeader, Option<Head>), Error> {
    let mut lines = data.split(|b| *b == b'\n').filter(|l| !l.is_empty());
    let header: LogicalHeader = lines
        .next()
        .and_then(|l| serde_json::from_slice(l).ok())
        .ok_or_else(|| Error::Restore("the snapshot has no header".into()))?;
    if header.format != LOGICAL_FORMAT || header.version != LOGICAL_VERSION {
        return Err(Error::Restore(format!(
            "the snapshot is {} version {}, not {LOGICAL_FORMAT} version {LOGICAL_VERSION}",
            header.format, header.version
        )));
    }
    history::ensure_tables(conn)?;
    migrate_to(conn, schema, &header.migrations)?;
    let mut loader = Loader { tables: HashMap::new() };
    let mut last_log_id = 0;
    for line in lines {
        let line: LogicalLine =
            serde_json::from_slice(line).map_err(|e| Error::Restore(format!("a snapshot line: {e}")))?;
        if line.table == CHANGE_LOG {
            last_log_id = line.row.get("id").and_then(serde_json::Value::as_i64).unwrap_or(0);
        }
        loader.insert(conn, &line.table, &line.row)?;
    }
    if history::last_change(conn)?.map_or(0, |(id, _)| id) != header.last_id || last_log_id > header.last_id {
        return Err(Error::Restore(format!(
            "the snapshot says it holds the log through {}, but holds {}",
            header.last_id,
            history::last_change(conn)?.map_or(0, |(id, _)| id)
        )));
    }
    // A trusted head inside the snapshot's range is checked by the walk; one beyond it, while replaying.
    let trusted = trusted.filter(|t| t.id <= header.last_id);
    let walked = history::verify(conn, trusted)?;
    if let Some(broken) = walked.broken {
        return Err(Error::Restore(format!("the snapshot's log is broken: {broken}")));
    }
    Ok((header, walked.head))
}

// ------------------------------------------------------------------ replay

fn entry_row(entry: &ChangeEntry) -> serde_json::Value {
    serde_json::json!({
        "id": entry.id,
        "at": entry.at,
        "actor": entry.actor,
        "table_name": entry.table_name,
        "row_key": entry.row_key,
        "op": entry.op,
        "before": entry.before,
        "after": entry.after,
        "prev_hash": entry.prev_hash,
        "hash": entry.hash,
    })
}

fn parse_row(entry: &ChangeEntry, text: Option<&str>, what: &str) -> Result<serde_json::Value, Error> {
    let text = text.ok_or_else(|| Error::Restore(format!("log entry {} has no {what} row", entry.id)))?;
    serde_json::from_str(text).map_err(|e| Error::Restore(format!("log entry {}'s {what} row: {e}", entry.id)))
}

/// Applies one entry's change and adds the entry to the log. Returns whether a delete found its row gone
/// already (a parent's cascade took it first).
fn apply(
    conn: &mut AnyConnection,
    schema: &DatabaseSchema,
    loader: &mut Loader,
    entry: &ChangeEntry,
) -> Result<bool, Error> {
    let mut missing = false;
    let table = entry.table_name.as_str();
    match entry.op.as_str() {
        "migrate" => {
            let row = parse_row(entry, entry.after.as_deref(), "after")?;
            let version = row
                .get("version")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| Error::Restore(format!("log entry {} names no migration", entry.id)))?;
            migrate_through(conn, schema, version)?;
            // A migration changes columns: read every table afresh.
            loader.tables.clear();
        }
        "insert" => {
            let after = parse_row(entry, entry.after.as_deref(), "after")?;
            loader.insert(conn, table, &after)?;
        }
        "update" | "soft_delete" | "restore" => {
            let after = parse_row(entry, entry.after.as_deref(), "after")?;
            let key = parse_row(entry, Some(&entry.row_key), "key")?;
            let t = loader.table(conn, table)?;
            t.check(table, &after)?;
            let sql = t.update.clone();
            let n = diesel::sql_query(sql)
                .bind::<Text, _>(after.to_string())
                .bind::<Text, _>(key.to_string())
                .execute(conn)?;
            if n != 1 {
                return Err(Error::Restore(format!(
                    "log entry {} updates a row of {table} that isn't there",
                    entry.id
                )));
            }
        }
        "delete" => {
            let key = parse_row(entry, Some(&entry.row_key), "key")?;
            let sql = loader.table(conn, table)?.delete.clone();
            missing = diesel::sql_query(sql).bind::<Text, _>(key.to_string()).execute(conn)? == 0;
        }
        other => {
            return Err(Error::Restore(format!(
                "log entry {} has the operation {other:?}",
                entry.id
            )));
        }
    }
    loader.insert(conn, CHANGE_LOG, &entry_row(entry))?;
    Ok(missing)
}

/// What a restore did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RestoreReport {
    /// The snapshot's newest entry.
    pub snapshot_last_id: i64,
    pub snapshot_taken_at: i64,
    /// Entries replayed after it.
    pub replayed: usize,
    /// The newest entry in the restored database, and its time.
    pub last_id: i64,
    pub last_at: Option<i64>,
    /// Deletes whose row a cascade had taken already.
    pub cascaded_deletes: usize,
}

/// What a restore is asked for.
#[derive(Debug, Clone)]
pub struct RestoreRequest {
    /// The moment to restore to, unix milliseconds, inclusive.
    pub until: i64,
    /// A folder for a SQLite snapshot's file while it is read.
    pub scratch: PathBuf,
    /// A head the owner holds off the server (DH5): the restore fails if the log it reads has another hash
    /// at that entry, or lacks it.
    pub trusted_head: Option<Head>,
}

/// Restores `snapshot` into `target`, which must not exist (SQLite) or must hold no tables (Postgres), and
/// replays `entries` after the snapshot's last one up to `request.until`. Entries must come oldest first;
/// those the snapshot holds are skipped. Every entry replayed or looked at must chain onto the snapshot's
/// head (DH5), or the restore fails naming the entry. Blocking. On failure the target is left as it was.
pub fn restore(
    target: &DatabaseConfig,
    schema: &DatabaseSchema,
    options: &DatabaseOptions,
    snapshot: &Snapshot,
    entries: &[ChangeEntry],
    request: &RestoreRequest,
) -> Result<RestoreReport, Error> {
    let (until, scratch) = (request.until, request.scratch.as_path());
    if snapshot.taken_at > until {
        return Err(Error::Restore("the snapshot is newer than the moment asked for".into()));
    }
    let logical = match snapshot.kind {
        SnapshotKind::Logical => std::borrow::Cow::Borrowed(snapshot.data.as_slice()),
        SnapshotKind::Sqlite => std::borrow::Cow::Owned(sqlite_to_logical(&snapshot.data, scratch)?),
    };
    let (mut conn, partial) = open_target(target)?;
    let result = conn.transaction(|conn| {
        let (header, snapshot_head) = load_logical(conn, schema, &logical, request.trusted_head.as_ref())?;
        if header.last_id != snapshot.last_id {
            return Err(Error::Restore(format!(
                "the snapshot is named for the log through {}, but holds it through {}",
                snapshot.last_id, header.last_id
            )));
        }
        let mut report = RestoreReport {
            snapshot_last_id: header.last_id,
            snapshot_taken_at: snapshot.taken_at,
            last_id: header.last_id,
            last_at: history::last_change(conn)?.map(|(_, at)| at),
            ..RestoreReport::default()
        };
        let mut loader = Loader { tables: HashMap::new() };
        let mut chain = snapshot_head.as_ref().map_or_else(Chain::genesis, Chain::after);
        let mut trusted_seen = request.trusted_head.as_ref().is_none_or(|t| t.id <= header.last_id);
        for entry in entries {
            if entry.id <= header.last_id {
                continue;
            }
            if let Some(t) = &request.trusted_head
                && !trusted_seen
                && entry.id > t.id
            {
                return Err(Error::Restore(Break::HeadMissing { id: t.id }.to_string()));
            }
            chain.check(entry).map_err(|b| Error::Restore(b.to_string()))?;
            if let Some(t) = &request.trusted_head
                && entry.id == t.id
            {
                if entry.hash.as_deref() != Some(t.hash.as_str()) {
                    return Err(Error::Restore(Break::HeadMismatch { id: t.id }.to_string()));
                }
                trusted_seen = true;
            }
            // Every entry of one write has its time, so this never splits a write.
            if entry.at > until {
                break;
            }
            if apply(conn, schema, &mut loader, entry)? {
                report.cascaded_deletes += 1;
            }
            report.replayed += 1;
            report.last_id = entry.id;
            report.last_at = Some(entry.at);
        }
        if conn.engine() == Engine::Postgres {
            for table in history::user_tables(conn)?
                .iter()
                .map(String::as_str)
                .chain([CHANGE_LOG])
            {
                let columns = history::columns(conn, table)?;
                fix_sequences(conn, table, &columns)?;
            }
        }
        Ok(report)
    });
    drop(conn);
    let report = match (result, &partial) {
        (Ok(report), Some((partial, file))) => {
            std::fs::rename(partial, file).map_err(|e| Error::Restore(format!("{}: {e}", file.display())))?;
            report
        }
        (Ok(report), None) => report,
        (Err(e), Some((partial, _))) => {
            let _ = std::fs::remove_file(partial);
            return Err(e);
        }
        (Err(e), None) => return Err(e),
    };
    // Newer migrations (logged as such) and the triggers, as for any database this build opens.
    Database::open(target, options, schema)?;
    Ok(report)
}

/// A connection to the restore's target; for SQLite, to a file beside it that becomes it once complete.
#[allow(clippy::type_complexity)]
fn open_target(target: &DatabaseConfig) -> Result<(AnyConnection, Option<(PathBuf, PathBuf)>), Error> {
    match target {
        DatabaseConfig::Sqlite { file } => {
            let mut sidecar = file.as_os_str().to_owned();
            sidecar.push("-wal");
            if file.exists() || PathBuf::from(sidecar).exists() {
                return Err(Error::Restore(format!(
                    "{} exists already: restore into a new file",
                    file.display()
                )));
            }
            let mut name = file.as_os_str().to_owned();
            name.push(".restoring");
            let partial = PathBuf::from(name);
            let _ = std::fs::remove_file(&partial);
            let mut conn = AnyConnection::Sqlite(SqliteConnection::establish(&partial.to_string_lossy())?);
            conn.batch_execute("PRAGMA foreign_keys = ON;")?;
            Ok((conn, Some((partial, file.clone()))))
        }
        DatabaseConfig::Postgres(pg) => {
            pg.check_address().map_err(Error::Config)?;
            let mut conn = AnyConnection::Postgres(PgConnection::establish(&pg.conninfo())?);
            conn.batch_execute("SET client_min_messages = warning;")?;
            let tables = history::user_tables(&mut conn)?;
            let log = diesel::sql_query(
                "SELECT table_name::text AS row FROM information_schema.tables
                 WHERE table_schema = current_schema() AND table_name IN ('__store_change_log', '__diesel_schema_migrations')",
            )
            .load::<Row>(&mut conn)?;
            if !tables.is_empty() || !log.is_empty() {
                return Err(Error::Restore(
                    "the Postgres database has tables already: restore into an empty one".into(),
                ));
            }
            Ok((conn, None))
        }
    }
}
