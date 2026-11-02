//! The isles database: `store`'s [`Database`] on SQLite or Postgres (DD15) with its change log (DD23), one schema for
//! the module.
//!
//! Every write names its actor, and a write to an account's data first asks the home API (DN2, DD17) where the
//! account lives: only its home writes it. A server outside a network is its own accounts' home, so nothing changes
//! for it; in a network, DN5 forwards the write that [`RepoError::NotHome`] refuses here.

use std::sync::Arc;

use diesel_migrations::{EmbeddedMigrations, embed_migrations};
use module_api::network::{
    HomeError, HomeForward, HuntRewards, InstanceRecord, JoinDecision, PublishedTicket, SessionTicket, TicketState,
};
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
    soft_delete: &[
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
    ],
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

    /// The same store with other repositories, for a test that stands in for the ones not ported yet.
    #[must_use]
    pub fn with_repos(mut self, repos: Repositories) -> Self {
        self.repos = repos;
        self
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

    /// The home API attached to this store.
    pub fn home(&self) -> &Arc<dyn HomeApi> {
        &self.home
    }

    pub async fn read<T, F>(&self, f: F) -> RepoResult<T>
    where
        F: FnOnce(&mut AnyConnection, &Repositories) -> RepoResult<T> + Send + 'static,
        T: Send + 'static,
    {
        let repos = self.repos.clone();
        let scope = crate::game_data::season_window::current_replay_scope();
        self.db.read(move |conn| scope.enter(|| f(conn, &repos))).await
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
            Err(HomeError::UnknownAccount) if account.global().owner() == &self.node => {}
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
        let scope = crate::game_data::season_window::current_replay_scope();
        self.db
            .write(store_actor(actor), move |tx| scope.enter(|| f(tx, &repos)))
            .await
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

    fn forward<'a>(&'a self, call: &'a HomeForward) -> BoxFuture<'a, Result<serde_json::Value, HomeError>> {
        Box::pin(async move {
            self.0
                .forward_home(
                    &call.account,
                    &call.actor,
                    &call.operation,
                    call.body.clone(),
                    call.key.clone(),
                )
                .await
                .unwrap_or_else(|error| Err(HomeError::Unreachable(error.to_string())))
        })
    }

    fn verify_session_ticket<'a>(
        &'a self,
        ticket: &'a SessionTicket,
        game: &'a module_api::GameVersion,
    ) -> BoxFuture<'a, Result<GlobalId, HomeError>> {
        Box::pin(async move {
            self.0
                .verify_session_ticket(ticket, game)
                .await
                .unwrap_or_else(|error| Err(HomeError::Unreachable(error.to_string())))
        })
    }

    fn register_account_key_hash<'a>(
        &'a self,
        key_sha256: &'a str,
        account: &'a GlobalId,
    ) -> BoxFuture<'a, Result<(), HomeError>> {
        Box::pin(async move {
            self.0
                .register_account_key_hash(key_sha256, account)
                .await
                .unwrap_or_else(|error| Err(HomeError::Unreachable(error.to_string())))
        })
    }

    fn register_directory_account<'a>(
        &'a self,
        account: &'a GlobalId,
        username: &'a str,
    ) -> BoxFuture<'a, Result<String, HomeError>> {
        Box::pin(async move {
            self.0
                .register_directory_account(account, username)
                .await
                .unwrap_or_else(|error| Err(HomeError::Unreachable(error.to_string())))
        })
    }

    fn ring_submit_ticket<'a>(
        &'a self,
        account: &'a GlobalId,
        queue: &'a str,
        game: &'a module_api::GameVersion,
        party_size: u16,
        client_ip: &'a str,
    ) -> BoxFuture<'a, Result<PublishedTicket, HomeError>> {
        Box::pin(async move {
            self.0
                .ring_submit_ticket(account, queue, game, party_size, client_ip)
                .await
                .unwrap_or_else(|error| Err(HomeError::Unreachable(error.to_string())))
        })
    }

    fn ring_try_form_matches<'a>(
        &'a self,
        queue: &'a str,
        game: &'a module_api::GameVersion,
    ) -> BoxFuture<'a, Result<Vec<InstanceRecord>, HomeError>> {
        Box::pin(async move {
            self.0
                .ring_try_form_matches(queue, game)
                .await
                .unwrap_or_else(|error| Err(HomeError::Unreachable(error.to_string())))
        })
    }

    fn ring_ticket_state<'a>(
        &'a self,
        ticket_id: &'a GlobalId,
    ) -> BoxFuture<'a, Result<Option<TicketState>, HomeError>> {
        Box::pin(async move {
            self.0
                .ring_ticket_state(ticket_id)
                .await
                .unwrap_or_else(|error| Err(HomeError::Unreachable(error.to_string())))
        })
    }

    fn ring_cancel_ticket<'a>(&'a self, ticket_id: &'a GlobalId) -> BoxFuture<'a, Result<bool, HomeError>> {
        Box::pin(async move {
            self.0
                .ring_cancel_ticket(ticket_id)
                .await
                .unwrap_or_else(|error| Err(HomeError::Unreachable(error.to_string())))
        })
    }

    fn ring_lookup_instance<'a>(
        &'a self,
        instance: &'a GlobalId,
    ) -> BoxFuture<'a, Result<Option<InstanceRecord>, HomeError>> {
        Box::pin(async move {
            self.0
                .ring_lookup_instance(instance)
                .await
                .unwrap_or_else(|error| Err(HomeError::Unreachable(error.to_string())))
        })
    }

    fn ring_register_instance<'a>(&'a self, record: &'a InstanceRecord) -> BoxFuture<'a, Result<(), HomeError>> {
        Box::pin(async move {
            self.0
                .ring_register_instance(record)
                .await
                .unwrap_or_else(|error| Err(HomeError::Unreachable(error.to_string())))
        })
    }

    fn ring_resolve_join<'a>(
        &'a self,
        account: &'a GlobalId,
        client_ip: &'a str,
        instance: &'a GlobalId,
    ) -> BoxFuture<'a, Result<JoinDecision, HomeError>> {
        Box::pin(async move {
            self.0
                .ring_resolve_join(account, client_ip, instance)
                .await
                .unwrap_or_else(|error| Err(HomeError::Unreachable(error.to_string())))
        })
    }

    fn ring_assign_ramsgate<'a>(
        &'a self,
        account: &'a GlobalId,
        client_ip: &'a str,
        game: &'a module_api::GameVersion,
    ) -> BoxFuture<'a, Result<(InstanceRecord, Option<SessionTicket>), HomeError>> {
        Box::pin(async move {
            self.0
                .ring_assign_ramsgate(account, client_ip, game)
                .await
                .unwrap_or_else(|error| Err(HomeError::Unreachable(error.to_string())))
        })
    }

    fn ring_forward_hunt_rewards<'a>(&'a self, rewards: &'a HuntRewards) -> BoxFuture<'a, Result<(), HomeError>> {
        Box::pin(async move {
            self.0
                .ring_forward_hunt_rewards(rewards)
                .await
                .unwrap_or_else(|error| Err(HomeError::Unreachable(error.to_string())))
        })
    }
}
