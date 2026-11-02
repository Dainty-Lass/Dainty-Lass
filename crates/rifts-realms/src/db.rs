// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The realms tables on `store` (DD15): one database with Guard's, every write in the change log under its actor.

use diesel_migrations::{EmbeddedMigrations, embed_migrations};
use module_api::NodeId;
use rifts_seam::Actor;
use store::{AnyConnection, Database, DatabaseConfig, DatabaseOptions, DatabaseSchema};

pub const SQLITE_MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations/sqlite");
pub const POSTGRES_MIGRATIONS: EmbeddedMigrations = embed_migrations!("migrations/postgres");

/// No realms table is player-facing in DD23's sense, so nothing is soft-deleted.
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
    /// A row that does not read back: an unknown enum text or bad JSON.
    #[error("realms table: {0}")]
    Corrupt(String),
}

/// A host runtime acts with its host's capability, so its writes are that player's to revert.
pub fn store_actor(actor: &Actor) -> store::Actor {
    match actor {
        Actor::Account(account) => store::Actor::Player(account.0.clone()),
        Actor::HostRuntime { host, .. } => store::Actor::Player(host.0.clone()),
        Actor::Admin { key_id } => store::Actor::Admin(key_id.clone()),
        Actor::System(task) => store::Actor::System((*task).to_owned()),
    }
}

/// `?` placeholders for both engines.
pub(crate) fn sql(text: &str) -> Result<String, RepoError> {
    store::portable::numbered(text).map_err(RepoError::Corrupt)
}

/// The realms handle: cloning shares the pools.
#[derive(Clone)]
pub struct RealmsDb {
    db: Database,
    node: NodeId,
}

impl RealmsDb {
    /// Blocking. Guard's set is applied first on its own handle, so both sets end up in the one database.
    pub fn open(config: &DatabaseConfig, options: &DatabaseOptions, node: NodeId) -> Result<RealmsDb, RepoError> {
        drop(Database::open(config, options, &rifts_guard::SCHEMA)?);
        let db = Database::open(config, options, &SCHEMA)?;
        Ok(Self { db, node })
    }

    pub fn new(db: Database, node: NodeId) -> RealmsDb {
        Self { db, node }
    }

    pub fn database(&self) -> &Database {
        &self.db
    }

    /// The node that owns the sessions this server creates (DD18).
    pub fn node(&self) -> &NodeId {
        &self.node
    }

    pub fn guard_store(&self) -> rifts_guard::SqlGuardStore {
        rifts_guard::SqlGuardStore::new(self.db.clone())
    }

    pub(crate) async fn read<T, F>(&self, f: F) -> Result<T, RepoError>
    where
        F: FnOnce(&mut AnyConnection) -> Result<T, RepoError> + Send + 'static,
        T: Send + 'static,
    {
        self.db.read(f).await
    }

    pub(crate) async fn write<T, F>(&self, actor: &Actor, f: F) -> Result<T, RepoError>
    where
        F: FnOnce(&mut AnyConnection) -> Result<T, RepoError> + Send + 'static,
        T: Send + 'static,
    {
        self.db.write(store_actor(actor), |tx| f(tx.conn())).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rifts_seam::{AccountId, SessionId};

    #[test]
    fn actors_map_onto_the_change_log() {
        let session: SessionId = "01ARZ3NDEKTSV4RRFFQ69G5FAV@local".parse().unwrap();
        let cases = [
            (Actor::Account(AccountId("a".into())), store::Actor::Player("a".into())),
            (
                Actor::HostRuntime {
                    session,
                    host: AccountId("host".into()),
                },
                store::Actor::Player("host".into()),
            ),
            (Actor::Admin { key_id: "k".into() }, store::Actor::Admin("k".into())),
            (
                Actor::System("realms-purge"),
                store::Actor::System("realms-purge".into()),
            ),
        ];
        for (actor, expected) in cases {
            assert_eq!(store_actor(&actor), expected);
        }
    }

    #[test]
    fn error_texts() {
        assert_eq!(
            RepoError::Duplicate("session_id").to_string(),
            "duplicate key: session_id"
        );
        assert_eq!(
            RepoError::Corrupt("hub state \"BOGUS\"".into()).to_string(),
            "realms table: hub state \"BOGUS\""
        );
        assert_eq!(
            RepoError::from(diesel::result::Error::NotFound).to_string(),
            "database: Record not found"
        );
    }
}
