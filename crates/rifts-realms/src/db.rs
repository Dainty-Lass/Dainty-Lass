// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The realms tables on `store` (DD15): one database with Guard's, every write in the change log under its actor.

use diesel_migrations::{EmbeddedMigrations, embed_migrations};
use module_api::NodeId;
use rifts_seam::{AccountId, Actor};
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
    use store::Actor as Logged;
    match actor {
        Actor::Account(AccountId(player))
        | Actor::HostRuntime {
            host: AccountId(player),
            ..
        } => Logged::Player(player.clone()),
        Actor::Admin { key_id } => Logged::Admin(key_id.clone()),
        Actor::System(task) => Logged::System(String::from(*task)),
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
        Database::open(config, options, &rifts_guard::SCHEMA).map(drop)?;
        let db = Database::open(config, options, &SCHEMA)?;
        Ok(Self::new(db, node))
    }

    pub fn new(db: Database, node: NodeId) -> RealmsDb {
        RealmsDb { node, db }
    }

    pub fn database(&self) -> &Database {
        &self.db
    }

    /// The node that owns the sessions this server creates (DD18).
    pub fn node(&self) -> &NodeId {
        &self.node
    }

    pub fn guard_store(&self) -> rifts_guard::SqlGuardStore {
        let shared = self.db.clone();
        rifts_guard::SqlGuardStore::new(shared)
    }

    pub(crate) async fn read<T: Send + 'static>(
        &self,
        query: impl FnOnce(&mut AnyConnection) -> Result<T, RepoError> + Send + 'static,
    ) -> Result<T, RepoError> {
        self.db.read(query).await
    }

    /// One transaction, logged under `actor`.
    pub(crate) async fn write<T: Send + 'static>(
        &self,
        actor: &Actor,
        change: impl FnOnce(&mut AnyConnection) -> Result<T, RepoError> + Send + 'static,
    ) -> Result<T, RepoError> {
        let logged = store_actor(actor);
        self.db.write(logged, move |tx| change(tx.conn())).await
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr as _;

    use rifts_seam::SessionId;

    use super::*;

    #[test]
    fn actors_map_onto_the_change_log() {
        let account = |id: &str| AccountId(id.to_owned());
        let runtime = Actor::HostRuntime {
            session: SessionId::from_str("01ARZ3NDEKTSV4RRFFQ69G5FAV@local").unwrap(),
            host: account("host"),
        };
        let cases = [
            (Actor::Account(account("a")), store::Actor::Player("a".into())),
            (runtime, store::Actor::Player("host".into())),
            (Actor::Admin { key_id: "k".into() }, store::Actor::Admin("k".into())),
            (
                Actor::System("realms-purge"),
                store::Actor::System("realms-purge".into()),
            ),
        ];
        assert!(cases.iter().all(|(actor, logged)| store_actor(actor) == *logged));
    }

    #[test]
    fn error_texts() {
        let cases = [
            (RepoError::Duplicate("session_id"), "duplicate key: session_id"),
            (
                RepoError::Corrupt("hub state \"BOGUS\"".into()),
                "realms table: hub state \"BOGUS\"",
            ),
            (
                RepoError::from(diesel::result::Error::NotFound),
                "database: Record not found",
            ),
        ];
        for (error, text) in cases {
            assert_eq!(error.to_string(), text);
        }
    }
}
