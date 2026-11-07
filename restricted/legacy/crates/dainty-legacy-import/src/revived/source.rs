//! A 1.4.4 metagame database, brought to the final 1.4.4 schema on a private copy: the host's file is never
//! written. Ported from DR's `offline_save::import` and `db::adopt_sqlite_if_needed` (legacy-revived `fac6efd`).
//!
//! The copy's migration history must be a nonempty prefix of the known one, the frozen Drizzle migrations of the
//! Node metagame or the Diesel ones of the Rust metagame, and its schema exactly what that prefix makes. Then the
//! rest is applied: the remaining Drizzle SQL, the Diesel baseline's adoption and any newer Diesel migration (DR
//! D11). The result must have the final schema and pass `integrity_check`. Until it has been compared, the copy's
//! schema is untrusted: `trusted_schema` is off, so none of its functions or views run.

use std::path::{Path, PathBuf};
use std::time::Duration;

use dainty_store::portable::quote_ident;
use dainty_store::{Connection, drizzle, schema_snapshot};
use diesel::Connection as _;
use diesel::connection::SimpleConnection;
use diesel::migration::MigrationSource;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Nullable, Text};
use diesel::sqlite::Sqlite;
use diesel_migrations::MigrationHarness;
use tempfile::TempDir;

use super::schema::{MIGRATIONS, SCHEMA, drizzle_migrations};
use crate::error::{ImportError, Result};

/// The most a server copy may take (DR `dbtools::BACKUP_DEADLINE`)
const BACKUP_DEADLINE: Duration = Duration::from_secs(15 * 60);
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

/// Where a database comes from, which decides what it may carry besides the metagame's tables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// A server's live or stopped database, copied with SQLite's online backup
    Server,
    /// An offline save's database: a stopped file, which must be in rollback-journal mode as the launcher left it
    OfflineSave,
}

/// A database at the final 1.4.4 schema, in a folder of its own that goes with it.
pub struct Prepared {
    pub conn: Connection,
    path: PathBuf,
    _dir: TempDir,
}

impl Prepared {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn invalid(message: &str) -> ImportError {
    ImportError::invalid(format!("not a Dauntless Revived 1.4.4 database: {message}"))
}

fn memory() -> Result<Connection> {
    Connection::establish(":memory:").map_err(|e| ImportError::Io(e.to_string()))
}

#[derive(QueryableByName)]
struct Integrity {
    #[diesel(sql_type = Text)]
    integrity_check: String,
}

#[derive(QueryableByName)]
struct JournalMode {
    #[diesel(sql_type = Text)]
    journal_mode: String,
}

#[derive(QueryableByName)]
struct Sql {
    #[diesel(sql_type = Nullable<Text>)]
    sql: Option<String>,
}

#[derive(QueryableByName)]
struct Named {
    #[diesel(sql_type = Text)]
    kind: String,
    #[diesel(sql_type = Text)]
    name: String,
}

#[derive(QueryableByName)]
struct History {
    #[diesel(sql_type = Text)]
    hash: String,
    #[diesel(sql_type = Nullable<BigInt>)]
    created_at: Option<i64>,
}

#[derive(QueryableByName)]
struct Version {
    #[diesel(sql_type = Text)]
    version: String,
}

fn integrity_ok(conn: &mut Connection) -> Result<bool> {
    let rows = diesel::sql_query("PRAGMA integrity_check").load::<Integrity>(conn)?;
    Ok(rows.len() == 1 && rows[0].integrity_check == "ok")
}

fn journal_mode(conn: &mut Connection) -> Result<String> {
    let mode = diesel::sql_query("PRAGMA journal_mode").get_result::<JournalMode>(conn)?;
    Ok(mode.journal_mode.to_ascii_lowercase())
}

/// A table's `CREATE` statement with its white space made single spaces, or None.
fn table_sql(conn: &mut Connection, name: &str) -> Result<Option<String>> {
    let row = diesel::sql_query("SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = ?")
        .bind::<Text, _>(name)
        .get_result::<Sql>(conn)
        .optional()?;
    Ok(row
        .and_then(|row| row.sql)
        .map(|sql| sql.split_whitespace().collect::<Vec<_>>().join(" ")))
}

/// The bookkeeping tables as Drizzle and Diesel make them.
fn bookkeeping() -> Result<(String, String)> {
    let mut conn = memory()?;
    drizzle::replay(&mut conn, &[])?;
    conn.applied_migrations().map_err(|_| invalid("Diesel's bookkeeping"))?;
    let drizzle = table_sql(&mut conn, "__drizzle_migrations")?.ok_or_else(|| invalid("Drizzle's bookkeeping"))?;
    let diesel = table_sql(&mut conn, "__diesel_schema_migrations")?.ok_or_else(|| invalid("Diesel's bookkeeping"))?;
    Ok((drizzle, diesel))
}

/// Each bookkeeping table the copy has must be the one its tool makes, before its rows are read.
fn bookkeeping_ok(conn: &mut Connection) -> Result<()> {
    let (drizzle, diesel) = bookkeeping()?;
    for (name, expected) in [
        ("__drizzle_migrations", drizzle),
        ("__diesel_schema_migrations", diesel),
    ] {
        if table_sql(conn, name)?.is_some_and(|actual| actual != expected) {
            return Err(invalid("a migration table is not the one its tool makes"));
        }
    }
    Ok(())
}

/// The schema of a database at the final 1.4.4 migration.
pub fn final_schema() -> Result<String> {
    let mut conn = memory()?;
    dainty_store::migrate(&mut conn, &SCHEMA)?;
    Ok(schema_snapshot(&mut conn)?)
}

/// Removes Dainty Lass's change log and its triggers (`__store_*`, DD23) from the copy: the Rust metagame opened
/// its database through `dainty_store::Database`, which put them there, and they are no part of the metagame's schema.
fn set_aside_change_log(conn: &mut Connection) -> Result<()> {
    let objects = diesel::sql_query(
        "SELECT type AS kind, name FROM sqlite_schema WHERE name LIKE '!_!_store!_%' ESCAPE '!' \
         AND type IN ('trigger', 'table') ORDER BY type DESC",
    )
    .load::<Named>(conn)?;
    for object in objects {
        let kind = if object.kind == "trigger" { "TRIGGER" } else { "TABLE" };
        conn.batch_execute(&format!("DROP {kind} IF EXISTS {};", quote_ident(&object.name)))?;
    }
    Ok(())
}

/// A Drizzle history that is a nonempty prefix of the frozen one, with the schema that prefix makes.
fn check_drizzle_prefix(db: &mut Connection) -> Result<()> {
    let frozen = drizzle_migrations();
    let history = diesel::sql_query("SELECT hash, created_at FROM __drizzle_migrations ORDER BY created_at LIMIT ?")
        .bind::<BigInt, _>(frozen.len() as i64 + 1)
        .load::<History>(db)?;
    if history.is_empty() || history.len() > frozen.len() {
        return Err(invalid("its Drizzle history is not one the Node metagame made"));
    }
    for (row, migration) in history.iter().zip(&frozen) {
        if row.created_at != Some(migration.created_at) || !drizzle::hash_matches(&row.hash, &migration.sql) {
            return Err(invalid("its Drizzle history is not one the Node metagame made"));
        }
    }
    let mut canonical = memory()?;
    drizzle::replay(&mut canonical, &frozen[..history.len()])?;
    if schema_snapshot(db)? != schema_snapshot(&mut canonical)? {
        return Err(invalid("its schema is not the one its Drizzle history makes"));
    }
    Ok(())
}

/// A Diesel history that is a nonempty prefix of the metagame's migrations, with the schema that prefix makes.
fn check_diesel_prefix(db: &mut Connection) -> Result<()> {
    let known = MigrationSource::<Sqlite>::migrations(&MIGRATIONS).map_err(|_| invalid("Diesel's migrations"))?;
    let applied =
        diesel::sql_query("SELECT version FROM __diesel_schema_migrations ORDER BY version").load::<Version>(db)?;
    if applied.is_empty() || applied.len() > known.len() {
        return Err(invalid("its Diesel history is not one the 1.4.4 metagame made"));
    }
    for (row, migration) in applied.iter().zip(&known) {
        if row.version != migration.name().version().to_string() {
            return Err(invalid("its Diesel history is not one the 1.4.4 metagame made"));
        }
    }
    let mut canonical = memory()?;
    // Creates Diesel's bookkeeping table, which run_migration records into
    canonical
        .applied_migrations()
        .map_err(|_| invalid("Diesel's bookkeeping"))?;
    for migration in &known[..applied.len()] {
        canonical
            .run_migration(migration.as_ref())
            .map_err(|_| invalid("Diesel's migrations"))?;
    }
    if schema_snapshot(db)? != schema_snapshot(&mut canonical)? {
        return Err(invalid("its schema is not the one its Diesel history makes"));
    }
    Ok(())
}

/// Brings a copy at `path` forward in place and checks the result.
fn bring_forward(conn: &mut Connection, origin: Origin) -> Result<()> {
    conn.batch_execute("PRAGMA trusted_schema = OFF;")?;
    set_aside_change_log(conn)?;
    bookkeeping_ok(conn)?;
    if !integrity_ok(conn)? {
        return Err(invalid("it fails SQLite's integrity check"));
    }
    if origin == Origin::OfflineSave && journal_mode(conn)? != "delete" {
        return Err(invalid(
            "an offline save is a stopped database in rollback-journal mode",
        ));
    }
    if table_sql(conn, "__diesel_schema_migrations")?.is_some() {
        check_diesel_prefix(conn)?;
    } else if table_sql(conn, "__drizzle_migrations")?.is_some() {
        check_drizzle_prefix(conn)?;
        // The rest of the frozen Drizzle history, then the baseline's adoption
        drizzle::replay(conn, &drizzle_migrations())?;
    } else {
        return Err(invalid("it has neither Drizzle's nor Diesel's migration history"));
    }
    dainty_store::migrate(conn, &SCHEMA)?;
    if schema_snapshot(conn)? != final_schema()?
        || !integrity_ok(conn)?
        || table_sql(conn, "__diesel_schema_migrations")?.is_none()
    {
        return Err(invalid("it does not reach the final 1.4.4 schema"));
    }
    Ok(())
}

/// Copies `source` into a private folder and brings the copy to the final 1.4.4 schema.
pub fn prepare(source: &Path, origin: Origin) -> Result<Prepared> {
    if !source.is_file() {
        return Err(ImportError::io(source, "there is no file here"));
    }
    let dir = tempfile::Builder::new()
        .prefix("dainty-legacy-")
        .tempdir()
        .map_err(|e| ImportError::Io(format!("a private folder for the copy: {e}")))?;
    let path = dir.path().join("revived.db");
    match origin {
        Origin::Server => {
            dainty_store::backup::backup_file(source, &path, BUSY_TIMEOUT, BACKUP_DEADLINE)?;
        }
        Origin::OfflineSave => {
            // Journal files beside it mean the launcher did not stop cleanly: refuse rather than miss their pages
            for suffix in ["-wal", "-shm", "-journal"] {
                let mut sidecar = source.as_os_str().to_owned();
                sidecar.push(suffix);
                if Path::new(&sidecar).exists() {
                    return Err(invalid("it has journal files beside it, so it was not stopped cleanly"));
                }
            }
            std::fs::copy(source, &path).map_err(|e| ImportError::io(source, e))?;
        }
    }
    let mut conn = Connection::establish(&path.to_string_lossy()).map_err(|e| ImportError::io(&path, e))?;
    if origin == Origin::Server {
        // The backup copies the source's journal mode; a private copy needs no WAL
        conn.batch_execute("PRAGMA journal_mode = DELETE;")?;
    }
    bring_forward(&mut conn, origin)?;
    Ok(Prepared { conn, path, _dir: dir })
}
