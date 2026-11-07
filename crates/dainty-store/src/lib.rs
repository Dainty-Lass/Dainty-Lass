// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Storage for the services: SQLite or Postgres through Diesel, schema migrations and Drizzle adoption, the
//! hash-chained change log with revert, and backup, snapshot and restore.

#![cfg_attr(
    test,
    allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")
)]

pub mod adopt;
pub mod backup;
mod connection;
mod database;
mod declared;
pub mod drizzle;
mod engine;
pub mod history;
pub mod portable;
pub mod readonly;
pub mod recovery;
pub mod revert;
mod snapshot;
pub mod transfer;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use diesel::connection::{Connection as _, SimpleConnection};
use diesel::prelude::*;
use diesel::r2d2::{ConnectionManager, CustomizeConnection, Pool};
use diesel::sql_types::Text;

pub use adopt::{DrizzleBaseline, Schema};
pub use connection::{AnyConnection, MultiBackend, PgConnection};
pub use database::{Database, DatabaseOptions, DatabaseSchema, KeyValue, Tx};
pub use declared::{declare_accounts, declared_accounts};
pub use engine::{DatabaseConfig, Engine, PostgresConfig, is_local_address};
pub use history::{Actor, ChangeEntry, Head};
pub use readonly::{open_read_only, schema_version};
pub use revert::{AccountColumn, Cursor, Filter, RevertOutcome, RevertPlan};
pub use snapshot::schema_snapshot;

/// The SQLite connection a [`Store`] hands its closures.
pub type Connection = diesel::SqliteConnection;

/// A SQLite database in WAL mode: one writer connection and a pool of query-only readers; cheap to clone.
#[derive(Clone)]
pub struct Store {
    writer_pool: Pool<ConnectionManager<Connection>>,
    reader_pool: Pool<ConnectionManager<Connection>>,
    file_path: Arc<PathBuf>,
    lock_wait: Duration,
}

impl Store {
    /// Opens `path`, creating it if needed, and migrates it to `schema` before any pool connection opens.
    pub fn open(path: &Path, options: &Options, schema: &Schema) -> Result<Self, Error> {
        let dsn = path.to_string_lossy().into_owned();
        {
            let mut setup_conn = Connection::establish(&dsn)?;
            configure_connection(&mut setup_conn, options.busy_timeout, false)?;
            activate_wal_mode(&mut setup_conn)?;
            migrate(&mut setup_conn, schema)?;
        }

        let writer_pool = Pool::builder()
            .max_size(1)
            .min_idle(Some(0))
            .test_on_check_out(false)
            .connection_customizer(Box::new(ConnectionInit {
                timeout: options.busy_timeout,
                query_only: false,
            }))
            .build(ConnectionManager::<Connection>::new(dsn.clone()))?;

        let reader_pool = Pool::builder()
            .max_size(options.read_connections.max(1))
            .min_idle(Some(0))
            .test_on_check_out(false)
            .connection_customizer(Box::new(ConnectionInit {
                timeout: options.busy_timeout,
                query_only: true,
            }))
            .build(ConnectionManager::<Connection>::new(dsn))?;

        Ok(Self {
            writer_pool,
            reader_pool,
            file_path: Arc::new(path.to_path_buf()),
            lock_wait: options.busy_timeout,
        })
    }

    /// Runs `f` in a transaction on a query-only reader connection, off the async threads.
    pub async fn read<T, E, F>(&self, f: F) -> Result<T, E>
    where
        F: FnOnce(&mut Connection) -> Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: From<Error> + From<diesel::result::Error> + Send + 'static,
    {
        let pool = self.reader_pool.clone();
        database::run_blocking(move || {
            let mut conn = pool.get().map_err(|e| E::from(Error::Pool(e)))?;
            (*conn).transaction(f)
        })
        .await
    }

    /// Runs `f` in an immediate transaction on the one writer connection, off the async threads.
    pub async fn write<T, E, F>(&self, f: F) -> Result<T, E>
    where
        F: FnOnce(&mut Connection) -> Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: From<Error> + From<diesel::result::Error> + Send + 'static,
    {
        let pool = self.writer_pool.clone();
        database::run_blocking(move || {
            let mut conn = pool.get().map_err(|e| E::from(Error::Pool(e)))?;
            conn.immediate_transaction(f)
        })
        .await
    }

    /// Truncates the write-ahead log into the database file.
    pub async fn checkpoint(&self) -> Result<(), Error> {
        let pool = self.writer_pool.clone();
        database::run_blocking(move || {
            let mut conn = pool.get().map_err(Error::Pool)?;
            conn.batch_execute("PRAGMA wal_checkpoint(TRUNCATE);")?;
            Ok(())
        })
        .await
    }

    pub(crate) fn path(&self) -> &Path {
        &self.file_path
    }

    pub(crate) fn busy_timeout(&self) -> Duration {
        self.lock_wait
    }
}

/// Brings `conn` up to date with `schema` without changing the connection's journal mode.
pub fn migrate(conn: &mut Connection, schema: &Schema) -> Result<(), Error> {
    adopt::apply_schema(conn, schema)
}

/// How a [`Store`] opens its connections.
#[derive(Debug, Clone)]
pub struct Options {
    /// Reader connections in the pool (at least one); 4 by default.
    pub read_connections: u32,
    /// How long a connection waits on a locked database; 5 s by default.
    pub busy_timeout: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            read_connections: 4,
            busy_timeout: Duration::from_secs(5),
        }
    }
}

/// Every way a store operation fails.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A query failed.
    #[error("database: {0}")]
    Query(#[from] diesel::result::Error),
    /// A connection could not be opened.
    #[error("database connection: {0}")]
    Connection(#[from] diesel::ConnectionError),
    /// No pooled connection was available.
    #[error("database pool: {0}")]
    Pool(#[from] diesel::r2d2::PoolError),
    /// A migration failed to apply.
    #[error("migration: {0}")]
    Migration(String),
    /// The database holds a schema the migrations cannot take over.
    #[error("the database can't be adopted: {0}")]
    Adoption(String),
    /// A backup could not be made.
    #[error("backup: {0}")]
    Backup(String),
    /// The database settings are invalid.
    #[error("database settings: {0}")]
    Config(String),
    /// The schema is not what the store expects.
    #[error("schema: {0}")]
    Schema(String),
    /// An [`Actor`] whose text form does not read back as itself.
    #[error("not an actor the change log can record: {0:?}")]
    Actor(String),
    /// `db move` could not copy a database.
    #[error("db move: {0}")]
    Move(String),
    /// A revert or history request was refused.
    #[error("revert: {0}")]
    Revert(String),
    /// The history changed between a revert's dry run and its apply.
    #[error("the history changed since the dry run: run it again")]
    PlanChanged,
    /// A restore could not be done.
    #[error("restore: {0}")]
    Restore(String),
    /// The blocking task running a closure panicked.
    #[error("the database task panicked")]
    Panicked,
}

#[derive(Debug)]
struct ConnectionInit {
    timeout: Duration,
    query_only: bool,
}

impl CustomizeConnection<Connection, diesel::r2d2::Error> for ConnectionInit {
    fn on_acquire(&self, conn: &mut Connection) -> Result<(), diesel::r2d2::Error> {
        configure_connection(conn, self.timeout, self.query_only).map_err(diesel::r2d2::Error::QueryError)
    }
}

fn configure_connection(
    conn: &mut Connection,
    timeout: Duration,
    query_only: bool,
) -> Result<(), diesel::result::Error> {
    let mut pragmas = format!(
        "PRAGMA busy_timeout = {}; PRAGMA foreign_keys = ON;",
        timeout.as_millis()
    );
    if query_only {
        pragmas.push_str(" PRAGMA query_only = ON;");
    }
    conn.batch_execute(&pragmas)
}

fn activate_wal_mode(conn: &mut Connection) -> Result<(), Error> {
    #[derive(QueryableByName)]
    struct ModeCell {
        #[diesel(sql_type = Text)]
        journal_mode: String,
    }
    let cell = diesel::sql_query("PRAGMA journal_mode = WAL").get_result::<ModeCell>(conn)?;
    if !cell.journal_mode.eq_ignore_ascii_case("wal") {
        return Err(Error::Migration(format!(
            "the database stayed in journal mode {}",
            cell.journal_mode
        )));
    }
    Ok(())
}
