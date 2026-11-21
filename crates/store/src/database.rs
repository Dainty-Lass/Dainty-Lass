// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! A database on either engine (DD15), with the change log on every write (DD23, DH1).
//!
//! Works as [`crate::Store`] does: a request's work is one closure on tokio's blocking pool. A read runs
//! in a read-only transaction. A write runs through [`Database::write`], the only write path: it names
//! the [`Actor`], the closure gets a [`Tx`], and the triggers log every row it changes in the same
//! transaction. Writes are serial on both engines (SQLite's one writer, an advisory lock on Postgres).
//!
//! [`Database::open`] builds the connection for the engine its [`DatabaseConfig`] names, brings the
//! schema up to date with that engine's migrations and (re)puts the change-log triggers on every table.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use diesel::connection::{Connection as _, SimpleConnection};
use diesel::migration::MigrationSource;
use diesel::pg::Pg;
use diesel::prelude::*;
use diesel::r2d2::{CustomizeConnection, ManageConnection, Pool, PooledConnection, R2D2Connection};
use diesel::sql_types::{BigInt, Text};
use diesel::sqlite::Sqlite;
use diesel_migrations::{EmbeddedMigrations, MigrationHarness};

use crate::connection::{AnyConnection, MultiBackend, PgConnection};
use crate::history::{self, Actor, ChangeEntry, DELETED_AT, Head, Verification};
use crate::portable::quote_ident;
use crate::revert::AccountColumn;
use crate::{DatabaseConfig, Engine, Error};

pub(crate) async fn run_blocking<T, E, F>(f: F) -> Result<T, E>
where
    F: FnOnce() -> Result<T, E> + Send + 'static,
    T: Send + 'static,
    E: From<Error> + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|_| E::from(Error::Panicked))?
}

/// A service's schema on both engines.
pub struct DatabaseSchema {
    /// `embed_migrations!` of the SQLite set and of the Postgres set: functions, as `EmbeddedMigrations`
    /// can't be copied. Both sets have the same versions, so `db move` can tell the two are at one schema.
    pub sqlite: fn() -> EmbeddedMigrations,
    pub postgres: fn() -> EmbeddedMigrations,
    /// Player-facing tables (accounts, characters, inventory, bans): deleted by setting `deleted_at`, and
    /// removed for good only by [`Database::purge_soft_deleted`] once the retention period has passed.
    pub soft_delete: &'static [&'static str],
}

#[derive(Debug, Clone)]
pub struct DatabaseOptions {
    /// Connections for read closures.
    pub read_connections: u32,
    /// Postgres only: connections for write closures (SQLite has one writer). Writes are still serial;
    /// a spare connection saves a reconnect.
    pub write_connections: u32,
    /// How long a statement waits for a lock: SQLite's `busy_timeout`, Postgres's `lock_timeout`.
    pub busy_timeout: Duration,
    /// Unix milliseconds for the log's `at`: the system clock, or a test's own (DH4's restore drills).
    pub clock: fn() -> i64,
    /// Postgres only: a schema of its own (such as `"chat"`), created at [`Database::open`] and set as each
    /// connection's `search_path`, with its own write and open advisory locks so its writes don't queue behind
    /// the game's. `None` uses the connection's default schema (`public`).
    pub postgres_schema: Option<String>,
}

impl Default for DatabaseOptions {
    fn default() -> Self {
        Self {
            read_connections: 4,
            write_connections: 2,
            busy_timeout: Duration::from_secs(5),
            clock: now_ms,
            postgres_schema: None,
        }
    }
}

fn check_schema_name(schema: &str) -> Result<(), Error> {
    let ok = !schema.is_empty()
        && schema.len() <= 63
        && schema
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        && schema.as_bytes()[0].is_ascii_lowercase();
    if ok {
        Ok(())
    } else {
        Err(Error::Config(format!("bad Postgres schema name {schema:?}")))
    }
}

/// Builds the variant the config names (DF7 spike: `AnyConnection::establish` hides auth errors).
#[derive(Debug)]
pub(crate) struct Manager {
    config: DatabaseConfig,
}

impl ManageConnection for Manager {
    type Connection = AnyConnection;
    type Error = diesel::r2d2::Error;

    fn connect(&self) -> Result<AnyConnection, Self::Error> {
        match &self.config {
            DatabaseConfig::Sqlite { file } => {
                SqliteConnection::establish(&file.to_string_lossy()).map(AnyConnection::Sqlite)
            }
            DatabaseConfig::Postgres(pg) => PgConnection::establish(&pg.conninfo()).map(AnyConnection::Postgres),
        }
        .map_err(diesel::r2d2::Error::ConnectionError)
    }

    fn is_valid(&self, conn: &mut AnyConnection) -> Result<(), Self::Error> {
        conn.ping().map_err(diesel::r2d2::Error::QueryError)
    }

    fn has_broken(&self, conn: &mut AnyConnection) -> bool {
        std::thread::panicking() || conn.is_broken()
    }
}

#[derive(Debug)]
struct Session {
    busy_timeout: Duration,
    read_only: bool,
    postgres_schema: Option<String>,
}

impl CustomizeConnection<AnyConnection, diesel::r2d2::Error> for Session {
    fn on_acquire(&self, conn: &mut AnyConnection) -> Result<(), diesel::r2d2::Error> {
        let ms = self.busy_timeout.as_millis();
        let sql = match conn {
            // busy_timeout first, so the statements after it already wait for locks.
            AnyConnection::Sqlite(_) => {
                format!(
                    "PRAGMA busy_timeout = {ms}; PRAGMA foreign_keys = ON;{}",
                    if self.read_only { " PRAGMA query_only = ON;" } else { "" }
                )
            }
            // NOTICEs (IF NOT EXISTS) would go to stderr.
            AnyConnection::Postgres(_) => format!(
                "SET client_min_messages = warning; SET lock_timeout = {ms};{}{}",
                if let Some(schema) = &self.postgres_schema {
                    format!(" SET search_path TO {};", quote_ident(schema))
                } else {
                    String::new()
                },
                if self.read_only {
                    " SET default_transaction_read_only = on;"
                } else {
                    ""
                }
            ),
        };
        conn.batch_execute(&sql).map_err(diesel::r2d2::Error::QueryError)
    }
}

/// The connection pools of one database. Cloning is cheap and shares the pools.
#[derive(Clone)]
pub struct Database {
    write: Pool<Manager>,
    read: Pool<Manager>,
    engine: Engine,
    soft_delete: Arc<[&'static str]>,
    accounts: Arc<[AccountColumn]>,
    config: Arc<DatabaseConfig>,
    busy_timeout: Duration,
    clock: fn() -> i64,
    write_lock: i64,
}

/// One write: what a write closure gets. Every row it changes through [`Tx::conn`] is logged under
/// [`Tx::actor`]; there is no other way to a connection that can write.
pub struct Tx<'a> {
    conn: &'a mut AnyConnection,
    actor: &'a Actor,
    at: i64,
    soft_delete: &'a [&'static str],
}

/// A value of a primary key column, for [`Tx::soft_delete`] and [`Tx::restore`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyValue {
    Int(i64),
    Text(String),
}

impl Tx<'_> {
    /// The connection, inside this write's transaction.
    pub fn conn(&mut self) -> &mut AnyConnection {
        self.conn
    }

    pub fn engine(&self) -> Engine {
        self.conn.engine()
    }

    pub fn actor(&self) -> &Actor {
        self.actor
    }

    pub(crate) fn soft_delete_tables(&self) -> &[&'static str] {
        self.soft_delete
    }

    /// When this write started, in unix milliseconds: every log entry of it has this time.
    pub fn at(&self) -> i64 {
        self.at
    }

    /// Marks the live row with this key deleted. Returns whether there was one.
    pub fn soft_delete(&mut self, table: &str, key: &[(&str, KeyValue)]) -> Result<bool, Error> {
        self.set_deleted(table, key, Some(self.at))
    }

    /// Brings a soft-deleted row back. Returns whether there was one.
    pub fn restore(&mut self, table: &str, key: &[(&str, KeyValue)]) -> Result<bool, Error> {
        self.set_deleted(table, key, None)
    }

    fn set_deleted(&mut self, table: &str, key: &[(&str, KeyValue)], at: Option<i64>) -> Result<bool, Error> {
        if !self.soft_delete.contains(&table) {
            return Err(Error::Schema(format!("{table} is not a soft-delete table")));
        }
        if key.is_empty() {
            return Err(Error::Schema("a soft delete needs the row's key".into()));
        }
        let deleted = quote_ident(DELETED_AT);
        let mut conditions: Vec<String> = key
            .iter()
            .enumerate()
            .map(|(i, (column, _))| format!("{} = ${}", quote_ident(column), i + 2))
            .collect();
        conditions.push(format!("{deleted} IS {}NULL", if at.is_some() { "" } else { "NOT " }));
        let sql = format!(
            "UPDATE {} SET {deleted} = $1 WHERE {}",
            quote_ident(table),
            conditions.join(" AND ")
        );
        let mut query = diesel::sql_query(sql)
            .into_boxed::<MultiBackend>()
            .bind::<diesel::sql_types::Nullable<BigInt>, _>(at);
        for (_, value) in key {
            query = match value {
                KeyValue::Int(n) => query.bind::<BigInt, _>(*n),
                KeyValue::Text(t) => query.bind::<Text, _>(t.clone()),
            };
        }
        Ok(query.execute(self.conn)? > 0)
    }
}

pub(crate) fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

fn checkout<E: From<Error>>(pool: &Pool<Manager>) -> Result<PooledConnection<Manager>, E> {
    pool.get().map_err(|e| E::from(Error::Pool(e)))
}

pub(crate) fn versions<DB: diesel::backend::Backend>(set: &EmbeddedMigrations) -> Result<Vec<String>, Error>
where
    EmbeddedMigrations: MigrationSource<DB>,
{
    Ok(MigrationSource::<DB>::migrations(set)
        .map_err(|e| Error::Migration(e.to_string()))?
        .iter()
        .map(|m| m.name().version().to_string())
        .collect())
}

impl Database {
    /// Opens (or creates) the database, brings its schema up to date and puts the change-log triggers on.
    /// Blocking: call it at start, before serving. A Postgres address off this machine is refused unless
    /// the config allows it (plaintext, DF7 spike).
    pub fn open(config: &DatabaseConfig, options: &DatabaseOptions, schema: &DatabaseSchema) -> Result<Self, Error> {
        if let DatabaseConfig::Postgres(pg) = config {
            pg.check_address().map_err(Error::Config)?;
        }
        if let Some(schema_name) = &options.postgres_schema {
            check_schema_name(schema_name)?;
        }
        let sqlite = versions::<Sqlite>(&(schema.sqlite)())?;
        let postgres = versions::<Pg>(&(schema.postgres)())?;
        if sqlite != postgres {
            return Err(Error::Migration(format!(
                "the SQLite migrations ({}) and the Postgres ones ({}) differ",
                sqlite.join(", "),
                postgres.join(", ")
            )));
        }
        let engine = config.engine();
        // Prepared on a connection of its own: through the pool, a refused login would surface only as a
        // checkout timeout, the server's reason lost in r2d2's log.
        {
            let mut conn = Manager { config: config.clone() }.connect().map_err(|e| match e {
                diesel::r2d2::Error::ConnectionError(e) => Error::Connection(e),
                diesel::r2d2::Error::QueryError(e) => Error::Query(e),
            })?;
            if engine == Engine::Postgres
                && let Some(schema_name) = &options.postgres_schema
            {
                conn.batch_execute(&format!("CREATE SCHEMA IF NOT EXISTS {}", quote_ident(schema_name)))
                    .map_err(Error::Query)?;
            }
            Session {
                busy_timeout: options.busy_timeout,
                read_only: false,
                postgres_schema: options.postgres_schema.clone(),
            }
            .on_acquire(&mut conn)
            .map_err(|e| Error::Migration(e.to_string()))?;
            if engine == Engine::Sqlite {
                set_wal(&mut conn)?;
            }
            prepare(&mut conn, schema, options.clock, options.postgres_schema.as_deref())?;
        }
        let pool = |size, read_only| {
            Pool::builder()
                .max_size(size)
                .min_idle(Some(0))
                .test_on_check_out(false)
                .connection_customizer(Box::new(Session {
                    busy_timeout: options.busy_timeout,
                    read_only,
                    postgres_schema: options.postgres_schema.clone(),
                }))
                .build(Manager { config: config.clone() })
        };
        let writers = match engine {
            Engine::Sqlite => 1,
            Engine::Postgres => options.write_connections.max(1),
        };
        let write = pool(writers, false)?;
        let read = pool(options.read_connections.max(1), true)?;
        Ok(Self {
            write,
            read,
            engine,
            soft_delete: schema.soft_delete.into(),
            accounts: Arc::new([]),
            config: Arc::new(config.clone()),
            busy_timeout: options.busy_timeout,
            clock: options.clock,
            write_lock: history::write_lock_for(options.postgres_schema.as_deref()),
        })
    }

    /// Opens a database another service's migrations made, as it is (DH3b): `history` and `revert` need the
    /// log and the tables, not the owner's code. Nothing is migrated, and the file must exist on SQLite.
    /// The triggers are put back on every table, a table with a `deleted_at` column soft-deleted as its owner
    /// has it (DD23), and `accounts` are given to [`Database::with_account_columns`] for the tables and columns
    /// that are there.
    pub fn attach(
        config: &DatabaseConfig,
        options: &DatabaseOptions,
        accounts: &[AccountColumn],
    ) -> Result<Self, Error> {
        if let DatabaseConfig::Sqlite { file } = config
            && !file.is_file()
        {
            return Err(Error::Config(format!("{}: there is no database here", file.display())));
        }
        if let DatabaseConfig::Postgres(pg) = config {
            pg.check_address().map_err(Error::Config)?;
        }
        if let Some(schema_name) = &options.postgres_schema {
            check_schema_name(schema_name)?;
        }
        let (soft_delete, accounts) = {
            let mut conn = Manager { config: config.clone() }.connect().map_err(|e| match e {
                diesel::r2d2::Error::ConnectionError(e) => Error::Connection(e),
                diesel::r2d2::Error::QueryError(e) => Error::Query(e),
            })?;
            if config.engine() == Engine::Postgres
                && let Some(schema_name) = &options.postgres_schema
            {
                conn.batch_execute(&format!("CREATE SCHEMA IF NOT EXISTS {}", quote_ident(schema_name)))
                    .map_err(Error::Query)?;
            }
            Session {
                busy_timeout: options.busy_timeout,
                read_only: false,
                postgres_schema: options.postgres_schema.clone(),
            }
            .on_acquire(&mut conn)
            .map_err(|e| Error::Migration(e.to_string()))?;
            let mut soft_delete: Vec<&'static str> = Vec::new();
            let mut present = Vec::new();
            for table in history::user_tables(&mut conn)? {
                let columns = history::columns(&mut conn, &table)?;
                if columns.iter().any(|c| c.name == DELETED_AT) {
                    // The trigger set takes `&'static str`: a table name leaks once, per attach
                    soft_delete.push(Box::leak(table.clone().into_boxed_str()));
                }
                present.extend(
                    accounts
                        .iter()
                        .filter(|a| a.table == table && columns.iter().any(|c| c.name == a.column)),
                );
            }
            (soft_delete, present.into_iter().copied().collect::<Vec<_>>())
        };
        let schema = DatabaseSchema {
            sqlite: || EmbeddedMigrations::new(&[]),
            postgres: || EmbeddedMigrations::new(&[]),
            soft_delete: Box::leak(soft_delete.into_boxed_slice()),
        };
        Ok(Self::open(config, options, &schema)?.with_account_columns(&accounts))
    }

    /// Names the column of each table that holds a row's account, for history and reverts by account
    /// (DH3). Without it, a filter naming an account is refused.
    #[must_use]
    pub fn with_account_columns(mut self, columns: &[AccountColumn]) -> Self {
        self.accounts = columns.into();
        self
    }

    pub fn account_columns(&self) -> Arc<[AccountColumn]> {
        self.accounts.clone()
    }

    pub(crate) fn soft_delete_tables(&self) -> Arc<[&'static str]> {
        self.soft_delete.clone()
    }

    pub fn engine(&self) -> Engine {
        self.engine
    }

    /// Runs `f` in a read-only transaction: all its queries see one snapshot on SQLite; Postgres reads
    /// committed data per statement unless `f` asks for more.
    pub async fn read<T, E, F>(&self, f: F) -> Result<T, E>
    where
        F: FnOnce(&mut AnyConnection) -> Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: From<Error> + From<diesel::result::Error> + Send + 'static,
    {
        let pool = self.read.clone();
        run_blocking(move || {
            let mut conn = checkout::<E>(&pool)?;
            (*conn).transaction(f)
        })
        .await
    }

    /// Runs `f` as `actor`'s write: one transaction, every changed row logged in it. Returning `Err`
    /// rolls it back, log entries included.
    pub async fn write<T, E, F>(&self, actor: Actor, f: F) -> Result<T, E>
    where
        F: FnOnce(&mut Tx<'_>) -> Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: From<Error> + From<diesel::result::Error> + Send + 'static,
    {
        let pool = self.write.clone();
        let soft_delete = self.soft_delete.clone();
        let clock = self.clock;
        let write_lock = self.write_lock;
        run_blocking(move || {
            let mut conn = checkout::<E>(&pool)?;
            (*conn).transaction(|conn| {
                let begun = history::begin_with_lock(conn, &actor, clock, write_lock)?;
                let out = f(&mut Tx {
                    conn,
                    actor: &actor,
                    at: begun.at,
                    soft_delete: &soft_delete,
                })?;
                history::end(conn, begun)?;
                Ok(out)
            })
        })
        .await
    }

    /// Change-log entries after `after_id`, oldest first.
    pub async fn changes(&self, after_id: i64, limit: i64) -> Result<Vec<ChangeEntry>, Error> {
        self.read(move |conn| history::changes(conn, after_id, limit)).await
    }

    /// The log's last sealed entry: what DH4 ships with the log (DH5).
    pub async fn head(&self) -> Result<Option<Head>, Error> {
        self.read(history::head).await
    }

    /// `history verify`: walks the log from its start and names the first entry edited, removed or
    /// reordered, checking `trusted` (a shipped head) on the way ([`history::verify`]).
    pub async fn verify(&self, trusted: Option<Head>) -> Result<Verification, Error> {
        self.read(move |conn| history::verify(conn, trusted.as_ref())).await
    }

    /// The newest change-log entry's id and time, if there is one.
    pub async fn last_change(&self) -> Result<Option<(i64, i64)>, Error> {
        self.read(history::last_change).await
    }

    /// The time the change log records, from [`DatabaseOptions::clock`].
    pub fn now(&self) -> i64 {
        (self.clock)()
    }

    /// Removes for good the rows soft-deleted longer ago than `retention`, as `system:retention`. Each
    /// removal is logged with the row as it was. Returns how many went, per table.
    pub async fn purge_soft_deleted(&self, retention: Duration) -> Result<Vec<(String, usize)>, Error> {
        let tables = self.soft_delete.clone();
        let retention = i64::try_from(retention.as_millis()).unwrap_or(i64::MAX);
        self.write(Actor::System("retention".into()), move |tx| {
            let cutoff = tx.at().saturating_sub(retention);
            let mut out = Vec::new();
            for table in tables.iter() {
                let deleted = quote_ident(DELETED_AT);
                let removed = diesel::sql_query(format!(
                    "DELETE FROM {} WHERE {deleted} IS NOT NULL AND {deleted} < $1",
                    quote_ident(table)
                ))
                .bind::<BigInt, _>(cutoff)
                .execute(tx.conn())?;
                out.push(((*table).to_owned(), removed));
            }
            Ok(out)
        })
        .await
    }

    /// Moves the SQLite WAL into the database file; on a clean shutdown (D17). Nothing on Postgres.
    pub async fn checkpoint(&self) -> Result<(), Error> {
        if self.engine != Engine::Sqlite {
            return Ok(());
        }
        let pool = self.write.clone();
        run_blocking(move || {
            let mut conn = checkout::<Error>(&pool)?;
            conn.batch_execute("PRAGMA wal_checkpoint(TRUNCATE);")?;
            Ok(())
        })
        .await
    }

    /// For `db move`: a write connection with no change-log context, and a read one.
    pub(crate) fn pools(&self) -> (&Pool<Manager>, &Pool<Manager>) {
        (&self.write, &self.read)
    }

    pub(crate) fn config(&self) -> &DatabaseConfig {
        &self.config
    }

    pub(crate) fn busy_timeout(&self) -> Duration {
        self.busy_timeout
    }

    pub(crate) fn clock(&self) -> fn() -> i64 {
        self.clock
    }
}

#[derive(QueryableByName)]
struct JournalMode {
    #[diesel(sql_type = Text)]
    journal_mode: String,
}

fn set_wal(conn: &mut AnyConnection) -> Result<(), Error> {
    let mode = diesel::sql_query("PRAGMA journal_mode = WAL").get_result::<JournalMode>(conn)?;
    if !mode.journal_mode.eq_ignore_ascii_case("wal") {
        return Err(Error::Migration(format!(
            "the database stayed in journal mode {}",
            mode.journal_mode
        )));
    }
    Ok(())
}

/// In one transaction: the log's tables, the triggers off, the pending migrations (logged as such), the
/// log sealed into the hash chain, the triggers back on every table. The first write takes SQLite's lock;
/// Postgres takes advisory ones first, the write lock among them, so the log's table is never locked by an
/// open waiting on a write that needs it.
fn prepare(
    conn: &mut AnyConnection,
    schema: &DatabaseSchema,
    clock: fn() -> i64,
    postgres_schema: Option<&str>,
) -> Result<(), Error> {
    conn.transaction(|conn| {
        match conn.engine() {
            Engine::Sqlite => {
                history::ensure_tables(conn)?;
                conn.batch_execute(&format!("DELETE FROM {}", quote_ident(history::CONTEXT)))?;
            }
            Engine::Postgres => {
                for lock in [
                    history::open_lock_for(postgres_schema),
                    history::write_lock_for(postgres_schema),
                ] {
                    diesel::sql_query("SELECT pg_advisory_xact_lock($1)")
                        .bind::<BigInt, _>(lock)
                        .execute(conn)?;
                }
                history::ensure_tables(conn)?;
            }
        }
        history::drop_triggers(conn)?;
        let applied: Vec<String> = match conn {
            AnyConnection::Sqlite(c) => c.run_pending_migrations((schema.sqlite)()),
            AnyConnection::Postgres(c) => c.run_pending_migrations((schema.postgres)()),
        }
        .map_err(|e| Error::Migration(e.to_string()))?
        .iter()
        .map(ToString::to_string)
        .collect();
        history::log_migrations(conn, &applied, clock())?;
        let sealed = history::head(conn)?;
        history::seal(conn, sealed)?;
        history::install_triggers(conn, schema.soft_delete)?;
        Ok(())
    })
}

/// The newest migration applied, the schema version both ends of a `db move` must share.
pub(crate) fn applied_versions(conn: &mut AnyConnection) -> Result<Vec<String>, Error> {
    #[derive(QueryableByName)]
    struct Version {
        #[diesel(sql_type = Text)]
        version: String,
    }
    Ok(diesel::sql_query(format!(
        "SELECT version FROM {} ORDER BY version",
        quote_ident(history::MIGRATIONS)
    ))
    .load::<Version>(conn)?
    .into_iter()
    .map(|v| v.version)
    .collect())
}
