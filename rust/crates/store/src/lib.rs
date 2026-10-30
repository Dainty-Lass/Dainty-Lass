// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! SQLite connection pooling, Drizzle/Diesel migration adoption, online backup, and change-log persistence.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use diesel::connection::{Connection as _, SimpleConnection};
use diesel::prelude::*;
use diesel::r2d2::{ConnectionManager, CustomizeConnection, Pool, PooledConnection};
use diesel::sql_types::Text;

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

pub use adopt::{DrizzleBaseline, Schema};
pub use connection::{AnyConnection, MultiBackend};
pub use database::{Database, DatabaseOptions, DatabaseSchema, KeyValue, Tx};
pub use declared::{declare_accounts, declared_accounts};
pub use engine::{DatabaseConfig, Engine, PostgresConfig, is_local_address};
pub use history::{Actor, ChangeEntry, Head};
pub use readonly::{open_read_only, schema_version};
pub use revert::{AccountColumn, Cursor, Filter, RevertOutcome, RevertPlan};
pub use snapshot::schema_snapshot;

pub type Connection = diesel::SqliteConnection;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database: {0}")]
    Query(#[from] diesel::result::Error),
    #[error("database connection: {0}")]
    Connection(#[from] diesel::ConnectionError),
    #[error("database pool: {0}")]
    Pool(#[from] diesel::r2d2::PoolError),
    #[error("migration: {0}")]
    Migration(String),
    #[error("the database can't be adopted: {0}")]
    Adoption(String),
    #[error("backup: {0}")]
    Backup(String),
    #[error("database settings: {0}")]
    Config(String),
    #[error("schema: {0}")]
    Schema(String),
    #[error("not an actor the change log can record: {0:?}")]
    Actor(String),
    #[error("db move: {0}")]
    Move(String),
    #[error("revert: {0}")]
    Revert(String),
    #[error("the history changed since the dry run: run it again")]
    PlanChanged,
    #[error("restore: {0}")]
    Restore(String),
    #[error("the database task panicked")]
    Panicked,
}

#[derive(Debug, Clone)]
pub struct Options {
    pub read_connections: u32,
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

#[derive(Debug)]
struct SqliteCustomizer {
    busy_timeout: Duration,
    read_only: bool,
}

impl CustomizeConnection<Connection, diesel::r2d2::Error> for SqliteCustomizer {
    fn on_acquire(&self, conn: &mut Connection) -> Result<(), diesel::r2d2::Error> {
        let ms = self.busy_timeout.as_millis();
        let sql = format!(
            "PRAGMA busy_timeout = {ms}; PRAGMA foreign_keys = ON;{}",
            if self.read_only { " PRAGMA query_only = ON;" } else { "" }
        );
        conn.batch_execute(&sql).map_err(diesel::r2d2::Error::QueryError)
    }
}

type SqlitePool = Pool<ConnectionManager<Connection>>;

#[derive(Clone)]
pub struct Store {
    pub(crate) path: Arc<PathBuf>,
    pub(crate) busy_timeout: Duration,
    write: SqlitePool,
    read: SqlitePool,
}

#[derive(QueryableByName)]
struct JournalModeRow {
    #[diesel(sql_type = Text)]
    journal_mode: String,
}

fn checkout_conn<E: From<Error>>(pool: &SqlitePool) -> Result<PooledConnection<ConnectionManager<Connection>>, E> {
    pool.get().map_err(|e| E::from(Error::Pool(e)))
}

impl Store {
    pub fn open(path: &Path, options: &Options, schema: &Schema) -> Result<Self, Error> {
        let target = path.to_string_lossy().into_owned();
        let build_pool = |max_size: u32, read_only: bool| {
            Pool::builder()
                .max_size(max_size)
                .min_idle(Some(0))
                .test_on_check_out(false)
                .connection_customizer(Box::new(SqliteCustomizer {
                    busy_timeout: options.busy_timeout,
                    read_only,
                }))
                .build(ConnectionManager::<Connection>::new(target.clone()))
        };
        let write = build_pool(1, false)?;
        let read = build_pool(options.read_connections.max(1), true)?;
        {
            let mut conn = checkout_conn::<Error>(&write)?;
            let row = diesel::sql_query("PRAGMA journal_mode = WAL").get_result::<JournalModeRow>(&mut conn)?;
            if !row.journal_mode.eq_ignore_ascii_case("wal") {
                return Err(Error::Migration(format!(
                    "expected WAL journal mode, got {}",
                    row.journal_mode
                )));
            }
            adopt::prepare(&mut conn, schema)?;
        }
        Ok(Self {
            path: Arc::new(path.to_path_buf()),
            busy_timeout: options.busy_timeout,
            write,
            read,
        })
    }

    pub async fn read<T, E, F>(&self, f: F) -> Result<T, E>
    where
        F: FnOnce(&mut Connection) -> Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: From<Error> + From<diesel::result::Error> + Send + 'static,
    {
        let pool = self.read.clone();
        run_blocking(move || {
            let mut conn = checkout_conn::<E>(&pool)?;
            (*conn).transaction(f)
        })
        .await
    }

    pub async fn write<T, E, F>(&self, f: F) -> Result<T, E>
    where
        F: FnOnce(&mut Connection) -> Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: From<Error> + From<diesel::result::Error> + Send + 'static,
    {
        let pool = self.write.clone();
        run_blocking(move || {
            let mut conn = checkout_conn::<E>(&pool)?;
            (*conn).immediate_transaction(f)
        })
        .await
    }

    pub async fn checkpoint(&self) -> Result<(), Error> {
        let pool = self.write.clone();
        run_blocking(move || {
            let mut conn = checkout_conn::<Error>(&pool)?;
            conn.batch_execute("PRAGMA wal_checkpoint(TRUNCATE);")?;
            Ok(())
        })
        .await
    }
}

pub fn migrate(conn: &mut Connection, schema: &Schema) -> Result<(), Error> {
    adopt::prepare(conn, schema)
}

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
