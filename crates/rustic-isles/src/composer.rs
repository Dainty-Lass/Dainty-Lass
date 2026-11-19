// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The composer seam (core spec §11.4): what restricted implements with today's areas until each is ported, and
//! what a bare test module composes with nothing.

use std::collections::BTreeMap;
use std::sync::Arc;

use dainty_module_api::chat::{ChatAnswer, ChatQuery};
use dainty_module_api::{
    ActorContext, AdminCall, AdminResponse, BoxFuture, ErrorKind, GlobalId, ModuleContext, ModuleError, ModuleId,
    NodeIdentity, PortRange,
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

/// The error every database-backed call answers in a module without a database.
pub fn no_database() -> ModuleError {
    ModuleError::new(ErrorKind::Unavailable, "the module has no database")
}

/// What a [`Composer`] is given at start.
pub struct ComposeInputs {
    /// The module's settings.
    pub settings: Settings,
    /// The node the module runs on.
    pub node: NodeIdentity,
    /// The module's id.
    pub module_id: ModuleId,
    /// The module context, for events, home calls and logs.
    pub context: ModuleContext,
    /// The module's game data folders.
    pub game_data: GameDataRoot,
    /// Open the database (off the async threads) only when this is set
    pub database_wanted: bool,
    /// The config's assigned ports by manifest name (the QoS pong binds the `qos` one)
    pub ports: BTreeMap<String, PortRange>,
}

/// A listener an area started (the QoS pong), recorded after the services.
pub struct StartedListener {
    /// `(name, address)` in the order `isles.listening` lists them
    pub addresses: Vec<(String, String)>,
    /// Stops the listener.
    pub stop: Box<dyn FnOnce() + Send>,
    /// The listener's task, awaited at stop.
    pub task: JoinHandle<()>,
}

/// What a [`Startup`] began: tasks and listeners stopped with the module, and lines to log.
#[derive(Default)]
pub struct Started {
    /// Background tasks aborted at stop.
    pub tasks: Vec<JoinHandle<()>>,
    /// Listeners stopped at stop.
    pub listeners: Vec<StartedListener>,
    /// Logged at info through the module context
    pub logs: Vec<String>,
}

/// Work an area runs once the module's services are up.
pub trait Startup: Send + Sync {
    /// Starts the work; an error fails the module start.
    fn start<'a>(&'a self, shared: &'a Arc<Shared>) -> BoxFuture<'a, Result<Started, String>>;
}

/// The module calls the areas make (core spec §7.5-§7.8). Each default answers as a module with no database, or
/// does nothing.
pub trait ModuleHooks: Send + Sync {
    /// A chat plugin query (§7.5).
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

    /// The chat plugin reported a player online or offline: their playtime opens or closes.
    fn presence_changed<'a>(
        &'a self,
        _shared: &'a Arc<Shared>,
        _account: &'a GlobalId,
        _online: bool,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async {})
    }

    /// Writes an invite (MS3): `expires_at` is ISO text, none never; answers its code.
    fn create_invite<'a>(
        &'a self,
        _shared: &'a Arc<Shared>,
        _invite: NewInvite,
    ) -> BoxFuture<'a, Result<String, ModuleError>> {
        Box::pin(async { Err(no_database()) })
    }

    /// Deletes an invite; false when there was none.
    fn revoke_invite<'a>(
        &'a self,
        _shared: &'a Arc<Shared>,
        _code: String,
    ) -> BoxFuture<'a, Result<bool, ModuleError>> {
        Box::pin(async { Err(no_database()) })
    }

    /// The account holding an account key, by the same check as a keyed launcher call (N-1); none when none here
    /// does or the holder may not use it.
    fn account_for_key<'a>(
        &'a self,
        _shared: &'a Arc<Shared>,
        _key: &'a str,
    ) -> BoxFuture<'a, Result<Option<GlobalId>, ModuleError>> {
        Box::pin(async { Err(no_database()) })
    }

    /// The store's checkpoint at stop; a failure is logged as a warning.
    fn stop(&self) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }
}

/// A [`GameCall::CreateInvite`](dainty_module_api::game::GameCall) checked by the module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewInvite {
    /// From 1 to 1000, as CreateInvite takes.
    pub uses: i64,
    /// Kept as staff wrote it; the hook cleans it as CreateInvite does
    pub note: Option<String>,
    /// ISO text; none never expires.
    pub expires_at: Option<String>,
}

/// Every default.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoHooks;

impl ModuleHooks for NoHooks {}

/// What the composer hands core: the areas' state, handlers and hooks.
pub struct Composition {
    /// The areas' typed state.
    pub slots: Slots,
    /// The store auth signs in against; none without a database.
    pub auth_store: Option<Arc<dyn AuthStore>>,
    /// What auth calls outside itself.
    pub auth_seams: Seams,
    /// Whether the store is Postgres, which changes auth's consent default.
    pub engine_is_postgres: bool,
    /// The rifts seam; dedicated-only when rustic-rifts is not loaded.
    pub rifts: rustic_rifts_seam::Rifts,
    /// The backend's; this crate adds the auth routes under them
    pub handlers: Handlers,
    /// Watches every request (the Reforged API's log and metrics).
    pub observer: Arc<dyn Observer>,
    /// In front of the `/admin/` rifts rows
    pub admin_authenticator: Option<Arc<dyn AdminAuthenticator>>,
    /// The areas' answers to the module's calls.
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
            rifts: rustic_rifts_seam::Rifts::dedicated_only(),
            handlers: Handlers::new(),
            observer: Arc::new(NoObserver),
            admin_authenticator: None,
            hooks: Arc::new(NoHooks),
            startups: Vec::new(),
        }
    }
}

/// Builds a module's areas from its inputs; restricted's composer until the areas are ported (§11.4).
pub trait Composer: Send + Sync + 'static {
    /// The areas' composition, or the error that fails the module start.
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
