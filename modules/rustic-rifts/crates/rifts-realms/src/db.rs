//! Realms on DaintyLass's `store` (DF7): one database on SQLite or Postgres, every write through [`Database::write`]
//! with the actor the change log records (DD23, DH1).

use diesel::query_builder::{BoxedSqlQuery, SqlQuery};
use diesel_migrations::{EmbeddedMigrations, embed_migrations};
use module_api::NodeId;
use rifts_seam::Actor;
use store::{AnyConnection, Database, DatabaseConfig, DatabaseOptions, DatabaseSchema, MultiBackend, Tx};

pub const SQLITE_MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations/sqlite");
pub const POSTGRES_MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations/postgres");

/// Realms' schema on its own; `isles-1-14-7` folds the two sets into its database's (dated versions, so they don't
/// clash with its own). No realm table is player-facing in DD23's sense, so none is soft-deleted.
pub const SCHEMA: DatabaseSchema = DatabaseSchema {
    sqlite: || SQLITE_MIGRATIONS,
    postgres: || POSTGRES_MIGRATIONS,
    soft_delete: &[],
};

#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    #[error(transparent)]
    Store(#[from] store::Error),
    #[error("database: {0}")]
    Query(#[from] diesel::result::Error),
    /// A unique key is taken (Node's E11000).
    #[error("duplicate key: {0}")]
    Duplicate(&'static str),
    /// A row the code can't read back (an unknown state name, bad JSON): someone wrote around this crate.
    #[error("realms table: {0}")]
    Corrupt(String),
}

impl From<RepoError> for crate::RealmsError {
    fn from(error: RepoError) -> Self {
        Self::Database(error.to_string())
    }
}

impl From<store::Error> for crate::RealmsError {
    fn from(error: store::Error) -> Self {
        Self::Database(error.to_string())
    }
}

impl From<diesel::result::Error> for crate::RealmsError {
    fn from(error: diesel::result::Error) -> Self {
        Self::Database(format!("database: {error}"))
    }
}

/// The handle every realms repository works through. Cloning is cheap and shares the pools.
#[derive(Clone)]
pub struct RealmsDb {
    db: Database,
    node: NodeId,
}

impl RealmsDb {
    /// Opens the database with realms' schema alone (tests, a standalone server). Blocking: call it at start.
    pub fn open(config: &DatabaseConfig, options: &DatabaseOptions, node: NodeId) -> Result<Self, RepoError> {
        Ok(Self::new(Database::open(config, options, &SCHEMA)?, node))
    }

    /// Over a database whose schema already holds realms' migrations.
    pub fn new(db: Database, node: NodeId) -> Self {
        Self { db, node }
    }

    pub fn database(&self) -> &Database {
        &self.db
    }

    /// The node that owns the sessions this server creates (module-api's global ids, DD18).
    pub fn node(&self) -> &NodeId {
        &self.node
    }

    pub(crate) async fn read<T, E, F>(&self, f: F) -> Result<T, E>
    where
        F: FnOnce(&mut AnyConnection) -> Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: From<store::Error> + From<diesel::result::Error> + Send + 'static,
    {
        self.db.read(f).await
    }

    /// One write transaction as `actor`; an `Err` rolls it back.
    pub(crate) async fn write<T, E, F>(&self, actor: &Actor, f: F) -> Result<T, E>
    where
        F: FnOnce(&mut AnyConnection) -> Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: From<store::Error> + From<diesel::result::Error> + Send + 'static,
    {
        self.db.write(store_actor(actor), move |tx: &mut Tx<'_>| f(tx.conn())).await
    }
}

/// The change log's actor. A host runtime acts with its host's capability, so its writes are that player's to
/// revert (`revert --actor player:<host>`).
pub fn store_actor(actor: &Actor) -> store::Actor {
    match actor {
        Actor::Account(account) => store::Actor::Player(account.0.clone()),
        Actor::HostRuntime { host, .. } => store::Actor::Player(host.0.clone()),
        Actor::Admin { key_id } => store::Actor::Admin(key_id.clone()),
        Actor::System(task) => store::Actor::System((*task).to_owned()),
    }
}

pub(crate) type Query = BoxedSqlQuery<'static, MultiBackend, SqlQuery>;

/// A query written with `?` placeholders, numbered for both engines (`store::portable`).
pub(crate) fn sql(text: &str) -> Result<Query, RepoError> {
    let numbered = store::portable::numbered(text).map_err(RepoError::Corrupt)?;
    Ok(diesel::sql_query(numbered).into_boxed())
}

/// `?, ?, ?` for an `IN` list.
pub(crate) fn marks(count: usize) -> String {
    vec!["?"; count].join(", ")
}

pub(crate) fn corrupt(what: &str, value: &str) -> RepoError {
    RepoError::Corrupt(format!("{what} {value:?}"))
}

#[derive(diesel::QueryableByName)]
pub(crate) struct Count {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub n: i64,
}
