// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Rustic Isles' database layer (RD27, written clean-room from `docs/port/spec-rustic-isles-store.md`): the
//! handle on `store` (both engines, the change log, soft deletes, the actor mapping, the home check), every
//! migration, the repositories the areas call, and the adapter `rustic-isles-auth` is given as its store.

#![cfg_attr(
    test,
    allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")
)]

mod adapter;
#[doc(hidden)]
pub mod fuzzing;
pub mod repo;
mod sql;
#[cfg(feature = "test-support")]
pub mod test_support;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use diesel_migrations::{EmbeddedMigrations, embed_migrations};
use module_api::network::HomeError;
use module_api::{Actor as ApiActor, ActorContext, HomeApi, NodeId, NodeIdentity};
use store::{Database, DatabaseConfig, DatabaseOptions, DatabaseSchema};

pub use adapter::IslesAuthStore;
pub use repo::{Repositories, Stub};
pub use rustic_isles::{AccountId, UserId};
pub use store::{AnyConnection, Tx};

/// A JSON value as the repositories read and write it.
pub type Json = serde_json::Value;

const SQLITE: EmbeddedMigrations = embed_migrations!("migrations/sqlite");
const POSTGRES: EmbeddedMigrations = embed_migrations!("migrations/postgres");

/// The soft-delete tables (DD23), in the schema's order.
pub const SOFT_DELETE: &[&str] = &[
    "accounts",
    "launcher_accounts",
    "characters",
    "loadouts",
    "inventories",
    "progression_tracks",
    "progression_objectives",
    "escalation_progress",
    "player_journey",
    "breadcrumbs",
    "encountered_content",
    "bounty_states",
    "cooldown_states",
    "entitlements",
    "wallets",
    "wallet_balances",
];

/// The isles schema: both engines' migrations and the soft-delete tables.
pub const SCHEMA: DatabaseSchema = DatabaseSchema {
    sqlite: || SQLITE,
    postgres: || POSTGRES,
    soft_delete: SOFT_DELETE,
};

/// Why a repository call failed.
#[derive(Debug, thiserror::Error)]
pub enum RepoError {
    /// The operation has no implementation in this repository set ([`Stub`]).
    #[error("{repository}: {operation} is not implemented yet")]
    Unimplemented {
        /// The repository's name.
        repository: &'static str,
        /// The operation called.
        operation: &'static str,
    },
    /// The account lives on another node, so this one may not write its rows (DD17).
    #[error("{account} is homed on {home}, not here")]
    NotHome {
        /// The account written for.
        account: AccountId,
        /// The node it is homed on.
        home: String,
    },
    /// The home lookup failed.
    #[error("the account's home could not be resolved: {0}")]
    Home(String),
    /// A `store` call failed.
    #[error(transparent)]
    Store(#[from] store::Error),
    /// A query failed.
    #[error("database: {0}")]
    Query(#[from] diesel::result::Error),
    /// A stored row could not be read; the text says which.
    #[error("stored data: {0}")]
    Data(String),
}

impl RepoError {
    /// The error a [`Stub`] operation answers.
    pub fn unimplemented(repository: &'static str, operation: &'static str) -> Self {
        Self::Unimplemented { repository, operation }
    }

    pub(crate) fn data(text: impl Into<String>) -> Self {
        Self::Data(text.into())
    }
}

/// A repository call's result.
pub type RepoResult<T> = Result<T, RepoError>;

/// The change log's name for an actor (spec §2.3).
pub fn store_actor(actor: &ActorContext) -> store::Actor {
    match &actor.actor {
        ApiActor::System => store::Actor::System("isles".into()),
        ApiActor::Console => store::Actor::Host("console".into()),
        ApiActor::Admin { key } => store::Actor::Admin(key.clone()),
        ApiActor::Player { account } => store::Actor::Player(account.to_string()),
        ApiActor::Module { module } => store::Actor::System(module.to_string()),
    }
}

/// The Rustic Isles database: `store`'s handle, this node, its home API and the repositories in use.
#[derive(Clone)]
pub struct IslesStore {
    database: Database,
    repos: Repositories,
    node: NodeId,
    home: Arc<dyn HomeApi>,
}

impl IslesStore {
    /// Opens the database at the latest migration with the change-log triggers on. Blocking: call it at start.
    pub fn open(
        config: &DatabaseConfig,
        options: &DatabaseOptions,
        home: Arc<dyn HomeApi>,
        node: &NodeIdentity,
    ) -> RepoResult<Self> {
        let database = Database::open(config, options, &SCHEMA)?;
        Ok(Self {
            database,
            repos: Repositories::on_store(),
            node: node.node.clone(),
            home,
        })
    }

    /// The module's settings decide: no database settings at all is no database (core §5.1).
    pub fn open_configured(
        lookup: impl Fn(&str) -> Option<String>,
        home: Arc<dyn HomeApi>,
        node: &NodeIdentity,
    ) -> RepoResult<Option<Self>> {
        let file = lookup("REFORGED_DB_FILE");
        if file.is_none() && lookup("REFORGED_DB_ENGINE").is_none() {
            return Ok(None);
        }
        let config = DatabaseConfig::from_lookup(&lookup, PathBuf::from(file.unwrap_or_default()))
            .map_err(|e| RepoError::Store(store::Error::Config(e)))?;
        if let DatabaseConfig::Sqlite { file } = &config
            && let Some(parent) = file.parent().filter(|p| !p.as_os_str().is_empty())
        {
            create_folder(parent)?;
        }
        Self::open(&config, &DatabaseOptions::default(), home, node).map(Some)
    }

    /// The same store with these repositories (tests and fakes).
    #[must_use]
    pub fn with_repos(mut self, repos: Repositories) -> Self {
        self.repos = repos;
        self
    }

    /// The underlying `store` handle.
    pub fn database(&self) -> &Database {
        &self.database
    }

    /// The repositories the closures are given.
    pub fn repos(&self) -> &Repositories {
        &self.repos
    }

    /// This node's id.
    pub fn node(&self) -> &NodeId {
        &self.node
    }

    /// The home API that says where an account lives.
    pub fn home(&self) -> &Arc<dyn HomeApi> {
        &self.home
    }

    /// `f` in a read-only transaction, inside the caller's replay scope (RU8).
    pub async fn read<T, F>(&self, f: F) -> RepoResult<T>
    where
        F: FnOnce(&mut AnyConnection, &Repositories) -> RepoResult<T> + Send + 'static,
        T: Send + 'static,
    {
        let scope = rustic_isles::replay::current_replay_scope();
        let repos = self.repos.clone();
        self.database
            .read(move |conn| match scope {
                Some(scope) => scope.enter(|| f(conn, &repos)),
                None => f(conn, &repos),
            })
            .await
    }

    /// `f` as one transaction under `actor`, committed on `Ok`; writes are serial on both engines.
    pub async fn write<T, F>(&self, actor: &ActorContext, f: F) -> RepoResult<T>
    where
        F: FnOnce(&mut Tx<'_>, &Repositories) -> RepoResult<T> + Send + 'static,
        T: Send + 'static,
    {
        let scope = rustic_isles::replay::current_replay_scope();
        let repos = self.repos.clone();
        self.database
            .write(store_actor(actor), move |tx| match scope {
                Some(scope) => scope.enter(|| f(tx, &repos)),
                None => f(tx, &repos),
            })
            .await
    }

    /// A write for `account`, refused unless the account is homed here (DD17, DN2). The actor is not checked
    /// against the account: an admin may write a player's rows.
    pub async fn account_write<T, F>(&self, account: &AccountId, actor: &ActorContext, f: F) -> RepoResult<T>
    where
        F: FnOnce(&mut Tx<'_>, &Repositories) -> RepoResult<T> + Send + 'static,
        T: Send + 'static,
    {
        self.check_home(account).await?;
        self.write(actor, f).await
    }

    async fn check_home(&self, account: &AccountId) -> RepoResult<()> {
        match self.home.resolve_home(account.global()).await {
            Ok(identity) if identity.node == self.node => Ok(()),
            Ok(identity) => Err(RepoError::NotHome {
                account: account.clone(),
                home: identity.node.to_string(),
            }),
            Err(HomeError::UnknownAccount) if account.home() == &self.node => Ok(()),
            Err(e) => Err(RepoError::Home(e.to_string())),
        }
    }
}

fn create_folder(folder: &Path) -> RepoResult<()> {
    std::fs::create_dir_all(folder)
        .map_err(|e| RepoError::Store(store::Error::Config(format!("{}: {e}", folder.display()))))
}
