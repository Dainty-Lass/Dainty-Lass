// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The composer seam (core spec §11.4): what restricted implements with today's areas until each is ported, and
//! what a bare test module composes with nothing.

use std::sync::Arc;

use module_api::chat::{ChatAnswer, ChatQuery};
use module_api::{
    ActorContext, AdminCall, AdminResponse, BoxFuture, ErrorKind, GlobalId, ModuleContext, ModuleError, ModuleId,
    NodeIdentity,
};
use rustic_isles_auth::{AuthStore, Seams};
use serde_json::Value;
use tokio::task::JoinHandle;

use crate::game_data::GameDataRoot;
use crate::handlers::Handlers;
use crate::ids::AccountId;
use crate::observe::{NoObserver, Observer};
use crate::settings::Settings;
use crate::shared::{AdminAuthenticator, Shared, Slots};

pub fn no_database() -> ModuleError {
    ModuleError::new(ErrorKind::Unavailable, "the module has no database")
}

pub struct ComposeInputs {
    pub settings: Settings,
    pub node: NodeIdentity,
    pub module_id: ModuleId,
    pub context: ModuleContext,
    pub game_data: GameDataRoot,
    /// Open the database (off the async threads) only when this is set
    pub database_wanted: bool,
}

/// A listener an area started (the QoS pong), recorded after the services.
pub struct StartedListener {
    /// `(name, address)` in the order `isles.listening` lists them
    pub addresses: Vec<(String, String)>,
    pub stop: Box<dyn FnOnce() + Send>,
    pub task: JoinHandle<()>,
}

#[derive(Default)]
pub struct Started {
    pub tasks: Vec<JoinHandle<()>>,
    pub listeners: Vec<StartedListener>,
    /// Logged at info through the module context
    pub logs: Vec<String>,
}

pub trait Startup: Send + Sync {
    fn start<'a>(&'a self, shared: &'a Arc<Shared>) -> BoxFuture<'a, Result<Started, String>>;
}

/// The module calls the areas make (core spec §7.5-§7.8). Each default answers as a module with no database, or
/// does nothing.
pub trait ModuleHooks: Send + Sync {
    fn chat<'a>(
        &'a self,
        _shared: &'a Arc<Shared>,
        _ctx: ModuleContext,
        _query: ChatQuery,
    ) -> BoxFuture<'a, Result<ChatAnswer, ModuleError>> {
        Box::pin(async { Err(no_database()) })
    }

    /// An admin action whose `Ban` `until` is already checked and written as an ISO text.
    fn admin<'a>(
        &'a self,
        _shared: &'a Arc<Shared>,
        _call: AdminCall,
    ) -> BoxFuture<'a, Result<AdminResponse, ModuleError>> {
        Box::pin(async { Err(no_database()) })
    }

    /// A home operation (DN5) for an account homed here; `Ok(None)` is an operation the module doesn't have.
    fn home_operation<'a>(
        &'a self,
        _shared: &'a Arc<Shared>,
        _operation: &'a str,
        _account: &'a AccountId,
        _actor: ActorContext,
        _body: Value,
    ) -> BoxFuture<'a, Result<Option<Value>, ModuleError>> {
        Box::pin(async { Err(no_database()) })
    }

    /// The players of a placed hunt as the Director expects them: an account homed here and in the store is its
    /// userId, any other stays its global id text.
    fn place_hunt_players<'a>(
        &'a self,
        _shared: &'a Arc<Shared>,
        players: &'a [GlobalId],
    ) -> BoxFuture<'a, Vec<String>> {
        Box::pin(async move { players.iter().map(ToString::to_string).collect() })
    }

    /// Matchmaking's ring map: an instance placed for these players.
    fn record_ring_instance<'a>(
        &'a self,
        _shared: &'a Arc<Shared>,
        _instance: &'a GlobalId,
        _host: &'a str,
        _port: u16,
        _players: &'a [String],
    ) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }

    /// The chat plugin reported a player offline (today: they leave their party; errors are ignored).
    fn player_offline<'a>(&'a self, _shared: &'a Arc<Shared>, _user_id: &'a str) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }

    /// The store's checkpoint at stop; a failure is logged as a warning.
    fn stop(&self) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }
}

/// Every default.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoHooks;

impl ModuleHooks for NoHooks {}

pub struct Composition {
    pub slots: Slots,
    pub auth_store: Option<Arc<dyn AuthStore>>,
    pub auth_seams: Seams,
    pub engine_is_postgres: bool,
    pub rifts: rifts_seam::Rifts,
    /// The backend's; this crate adds the auth routes under them
    pub handlers: Handlers,
    pub observer: Arc<dyn Observer>,
    /// In front of the `/admin/` rifts rows
    pub admin_authenticator: Option<Arc<dyn AdminAuthenticator>>,
    pub hooks: Arc<dyn ModuleHooks>,
    /// Run in order at start
    pub startups: Vec<Arc<dyn Startup>>,
}

impl Composition {
    /// No database and no areas: every non-auth backend route answers 501.
    pub fn bare() -> Self {
        Self {
            slots: Slots::default(),
            auth_store: None,
            auth_seams: Seams::default(),
            engine_is_postgres: false,
            rifts: rifts_seam::Rifts::dedicated_only(),
            handlers: Handlers::new(),
            observer: Arc::new(NoObserver),
            admin_authenticator: None,
            hooks: Arc::new(NoHooks),
            startups: Vec::new(),
        }
    }
}

pub trait Composer: Send + Sync + 'static {
    fn compose(&self, inputs: ComposeInputs) -> BoxFuture<'_, Result<Composition, ModuleError>>;
}

/// Composes [`Composition::bare`].
#[derive(Debug, Clone, Copy, Default)]
pub struct BareComposer;

impl Composer for BareComposer {
    fn compose(&self, _: ComposeInputs) -> BoxFuture<'_, Result<Composition, ModuleError>> {
        Box::pin(async { Ok(Composition::bare()) })
    }
}
