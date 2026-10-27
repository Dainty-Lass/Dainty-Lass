//! The isles database: `store`'s [`Database`] on SQLite or Postgres (DD15) with its change log (DD23), one schema for
//! both modules (RU3c: a 1.12 account simply has no rows in the 1.14.7-only tables).
//!
//! Every write names its actor, and a write to an account's data first asks the home API (DN2, DD17) where the
//! account lives: only its home writes it. A server outside a network is its own accounts' home, so nothing changes
//! for it; in a network, DN5 forwards the write that [`RepoError::NotHome`] refuses here.

use std::sync::Arc;

use diesel_migrations::{EmbeddedMigrations, embed_migrations};
use module_api::network::{HomeError, SessionTicket};
use module_api::{Actor, ActorContext, BoxFuture, GlobalId, HomeApi, ModuleContext, NodeId, NodeIdentity};
use store::{AnyConnection, Database, DatabaseConfig, DatabaseOptions, DatabaseSchema, Tx};

use crate::ids::AccountId;
use crate::repo::{RepoError, RepoResult, Repositories};

const SQLITE: EmbeddedMigrations = embed_migrations!("migrations/sqlite");
const POSTGRES: EmbeddedMigrations = embed_migrations!("migrations/postgres");

/// Both dialects' migrations; a new table is a migration in each, with the same version (DF7).
pub const SCHEMA: DatabaseSchema = DatabaseSchema {
    sqlite: || SQLITE,
    postgres: || POSTGRES,
    // Player-facing tables are soft-deleted (DD23); each area adds its own as it lands them
    soft_delete: &["accounts"],
};

/// The change log's actor for a module's actor (`store` keeps its own, smaller type).
pub fn store_actor(actor: &ActorContext) -> store::Actor {
    match &actor.actor {
        Actor::System => store::Actor::System("isles".into()),
        Actor::Console => store::Actor::Host("console".into()),
        Actor::Admin { key } => store::Actor::Admin(key.clone()),
        Actor::Player { account } => store::Actor::Player(account.to_string()),
        Actor::Module { module } => store::Actor::System(module.to_string()),
    }
}

#[derive(Clone)]
pub struct IslesStore {
    db: Database,
    home: Arc<dyn HomeApi>,
    node: NodeId,
    repos: Repositories,
}

impl IslesStore {
    /// Opens the database and brings it to the current schema. Blocking: call it at start, before serving.
    pub fn open(
        config: &DatabaseConfig,
        options: &DatabaseOptions,
        home: Arc<dyn HomeApi>,
        node: &NodeIdentity,
    ) -> RepoResult<Self> {
        let db = Database::open(config, options, &SCHEMA)?;
        Ok(Self {
            db,
            home,
            node: node.node.clone(),
            repos: Repositories::on_store(),
        })
    }

    pub fn database(&self) -> &Database {
        &self.db
    }

    pub fn repos(&self) -> &Repositories {
        &self.repos
    }

    /// The node this store's accounts are homed on.
    pub fn node(&self) -> &NodeId {
        &self.node
    }

    pub async fn read<T, F>(&self, f: F) -> RepoResult<T>
    where
        F: FnOnce(&mut AnyConnection, &Repositories) -> RepoResult<T> + Send + 'static,
        T: Send + 'static,
    {
        let repos = self.repos.clone();
        self.db.read(move |conn| f(conn, &repos)).await
    }

    /// A write to one account's data, as `actor`: refused unless this node is the account's home.
    pub async fn account_write<T, F>(&self, account: &AccountId, actor: &ActorContext, f: F) -> RepoResult<T>
    where
        F: FnOnce(&mut Tx<'_>, &Repositories) -> RepoResult<T> + Send + 'static,
        T: Send + 'static,
    {
        match self.home.resolve_home(account.global()).await {
            Ok(home) if home.node == self.node => {}
            Ok(home) => {
                return Err(RepoError::NotHome {
                    account: account.clone(),
                    home: home.node.to_string(),
                });
            }
            Err(error) => return Err(RepoError::Home(error.to_string())),
        }
        self.write(actor, f).await
    }

    /// A write that belongs to no one account (notices, API keys, a new account's first row), as `actor`.
    pub async fn write<T, F>(&self, actor: &ActorContext, f: F) -> RepoResult<T>
    where
        F: FnOnce(&mut Tx<'_>, &Repositories) -> RepoResult<T> + Send + 'static,
        T: Send + 'static,
    {
        let repos = self.repos.clone();
        self.db.write(store_actor(actor), move |tx| f(tx, &repos)).await
    }
}

/// The home API as a module reaches it: through its context, over the pipe when out of process (DN2).
#[derive(Debug, Clone)]
pub struct ContextHome(pub ModuleContext);

impl HomeApi for ContextHome {
    fn resolve_home<'a>(&'a self, account: &'a GlobalId) -> BoxFuture<'a, Result<NodeIdentity, HomeError>> {
        Box::pin(async move {
            self.0
                .resolve_home(account)
                .await
                .unwrap_or_else(|error| Err(HomeError::Unreachable(error.to_string())))
        })
    }

    fn session_ticket<'a>(
        &'a self,
        account: &'a GlobalId,
        audience: &'a NodeId,
    ) -> BoxFuture<'a, Result<SessionTicket, HomeError>> {
        Box::pin(async move {
            self.0
                .session_ticket(account, audience)
                .await
                .unwrap_or_else(|error| Err(HomeError::Unreachable(error.to_string())))
        })
    }
}
