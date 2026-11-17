// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! What every handler shares, one per module (core spec §5.1), and the home and presence it hands areas.

use std::any::{Any, TypeId};
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use dainty_module_api::network::{
    HomeError, HomeForward, HuntRewards, InstanceRecord, JoinDecision, PublishedTicket, SessionTicket, StandaloneHome,
    TicketState,
};
use dainty_module_api::{BoxFuture, GameVersion, GlobalId, HomeApi, ModuleContext, ModuleError, NodeId, NodeIdentity};
use rustic_isles_auth::{Auth, AuthConfig, AuthStore, Seams};

use crate::dedicated::{DedicatedServers, Unconfigured};
use crate::game_data::GameDataRoot;
use crate::handlers::Handlers;
use crate::observe::{NoObserver, Observer};
use crate::settings::Settings;
use crate::version::IslesVersion;

/// The module's sign-in area over whatever store the composer opened.
pub type IslesAuth = Auth<Arc<dyn AuthStore>>;

/// Typed area state the composer fills; a handler asks for a type and gets it or none.
#[derive(Default, Clone)]
pub struct Slots(HashMap<TypeId, Arc<dyn Any + Send + Sync>>);

impl Slots {
    /// Fills the slot for `T`, replacing what it held.
    pub fn insert<T: Any + Send + Sync>(&mut self, value: Arc<T>) {
        self.0.insert(TypeId::of::<T>(), value);
    }

    /// [`Slots::insert`], chained.
    pub fn with<T: Any + Send + Sync>(mut self, value: Arc<T>) -> Self {
        self.insert(value);
        self
    }

    /// The value in `T`'s slot, none when empty.
    pub fn get<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.0.get(&TypeId::of::<T>()).cloned()?.downcast().ok()
    }
}

/// The user ids the chat plugin reports online, per module (core spec §15 item 14).
#[derive(Debug, Default)]
pub struct Presence(Mutex<BTreeSet<String>>);

impl Presence {
    fn set(&self) -> std::sync::MutexGuard<'_, BTreeSet<String>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Marks the user online.
    pub fn set_online(&self, user_id: &str) {
        self.set().insert(user_id.to_owned());
    }

    /// Marks the user offline.
    pub fn set_offline(&self, user_id: &str) {
        self.set().remove(user_id);
    }

    /// Whether the user is online.
    pub fn is_online(&self, user_id: &str) -> bool {
        self.set().contains(user_id)
    }

    /// Every user online, sorted.
    pub fn online_user_ids(&self) -> Vec<String> {
        self.set().iter().cloned().collect()
    }

    /// Only forgets the user here; 1 if they were online, else 0.
    pub fn close_connections(&self, user_id: &str, _code: u16, _reason: &str) -> usize {
        usize::from(self.set().remove(user_id))
    }
}

/// Who an admin caller is, as the Reforged API's admin auth found them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminCaller {
    /// The admin key's id.
    pub key_id: String,
}

/// What the admin authenticator reads of a relayed `/admin/` request: the manager credential signs the method,
/// target and body, so the body is read before it is asked (reforged-admin spec §3.5).
#[derive(Debug, Clone, Copy)]
pub struct AdminRequestHead<'a> {
    /// The HTTP method.
    pub method: &'a str,
    /// The path and query as the handler sees them
    pub target: &'a str,
    /// The caller's address.
    pub peer: Option<std::net::IpAddr>,
    /// The request's headers.
    pub headers: &'a http::HeaderMap,
    /// The request's body.
    pub body: &'a [u8],
}

/// The Reforged API's admin auth, a slot in front of the `/admin/` rifts rows (auth spec §4.4); the Reforged unit
/// fills it.
pub trait AdminAuthenticator: Send + Sync {
    /// The admin caller, or the refusal to answer.
    fn authenticate<'a>(
        &'a self,
        request: AdminRequestHead<'a>,
    ) -> BoxFuture<'a, Result<AdminCaller, axum::response::Response>>;
}

/// The answer while the slot is empty: refused as launcher auth refuses a missing sign-in.
pub fn admin_refusal() -> axum::response::Response {
    let refusal = rustic_isles_auth::LauncherError::new("AUTH_UNAUTHORIZED", "Sign in required.");
    refusal.response(&crate::replay::next_uuid())
}

/// The inputs [`Shared::new`] builds from.
pub struct SharedParts {
    /// The game version.
    pub version: IslesVersion,
    /// The module's settings.
    pub settings: Settings,
    /// The node the module runs on.
    pub node: NodeIdentity,
    /// The module's game data folders.
    pub game_data: GameDataRoot,
    /// Whether a database is configured (`DAINTY_DB_FILE` or `DAINTY_DB_ENGINE`).
    pub database_wanted: bool,
    /// The rifts seam.
    pub rifts: rustic_rifts_seam::Rifts,
    /// Whether the admin API is on.
    pub admin_api: bool,
    /// The store auth signs in against; none without a database.
    pub auth_store: Option<Arc<dyn AuthStore>>,
    /// What auth calls outside itself; core fills its home.
    pub auth_seams: Seams,
    /// Whether the store is Postgres.
    pub engine_is_postgres: bool,
    /// The module's context, for the home and for areas that emit; none in a bare module
    pub context: Option<ModuleContext>,
    /// The areas' typed state.
    pub slots: Slots,
    /// Watches every request.
    pub observer: Arc<dyn Observer>,
    /// The admin auth in front of the `/admin/` rifts rows, none until the Reforged unit fills it.
    pub admin_authenticator: Option<Arc<dyn AdminAuthenticator>>,
    /// The backend's handlers
    pub handlers: Handlers,
}

impl SharedParts {
    /// No database, standalone node, empty settings, no game data folders, dedicated-only rifts, admin API off,
    /// an auth with no store, empty slots.
    pub fn bare(version: IslesVersion) -> Self {
        Self {
            version,
            settings: Settings::default(),
            node: NodeIdentity::standalone(),
            game_data: GameDataRoot::default(),
            database_wanted: false,
            rifts: rustic_rifts_seam::Rifts::dedicated_only(),
            admin_api: false,
            auth_store: None,
            auth_seams: Seams::default(),
            engine_is_postgres: false,
            context: None,
            slots: Slots::default(),
            observer: Arc::new(NoObserver),
            admin_authenticator: None,
            handlers: Handlers::new(),
        }
    }
}

/// What every handler shares, one per module (§5.1).
pub struct Shared {
    version: IslesVersion,
    settings: Arc<Settings>,
    node: NodeIdentity,
    game_data: GameDataRoot,
    database_wanted: bool,
    rifts: rustic_rifts_seam::Rifts,
    admin_api: bool,
    auth: Arc<IslesAuth>,
    home: Arc<dyn HomeApi>,
    presence: Presence,
    dedicated: RwLock<Arc<dyn DedicatedServers>>,
    slots: Slots,
    context: Option<ModuleContext>,
    observer: Arc<dyn Observer>,
    admin_authenticator: RwLock<Option<Arc<dyn AdminAuthenticator>>>,
    handlers: Handlers,
}

impl Shared {
    /// Builds the shared state and its auth from the parts; fails when the tables' game version does not parse.
    pub fn new(parts: SharedParts) -> Result<Arc<Self>, ModuleError> {
        let tables = parts.version.tables;
        let game_version: GameVersion = tables
            .game_version
            .parse()
            .map_err(|e| ModuleError::failed(format!("game version {}: {e}", tables.game_version)))?;
        let home: Arc<dyn HomeApi> = match &parts.context {
            Some(context) => Arc::new(ContextHome(context.clone())),
            None => Arc::new(StandaloneHome {
                identity: parts.node.clone(),
            }),
        };
        let settings = Arc::new(parts.settings);
        let config = AuthConfig {
            target_changelist: tables.target_changelist,
            game_version,
            node: parts.node.clone(),
            no_player: parts.version.hooks.no_player_policy().for_auth(),
            session_payload: tables.session_payload,
            engine_is_postgres: parts.engine_is_postgres,
            clock: Arc::new(crate::replay::now_ms),
            ids: Arc::new(crate::replay::next_uuid),
        };
        let seams = Seams {
            home: Arc::clone(&home),
            ..parts.auth_seams
        };
        let auth = Auth::new(config, settings.clone(), parts.auth_store, seams);
        Ok(Arc::new(Self {
            version: parts.version,
            settings,
            node: parts.node,
            game_data: parts.game_data,
            database_wanted: parts.database_wanted,
            rifts: parts.rifts,
            admin_api: parts.admin_api,
            auth,
            home,
            presence: Presence::default(),
            dedicated: RwLock::new(Arc::new(Unconfigured)),
            slots: parts.slots,
            context: parts.context,
            observer: parts.observer,
            admin_authenticator: RwLock::new(parts.admin_authenticator),
            handlers: parts.handlers,
        }))
    }

    /// A bare module's state (tests).
    pub fn bare(version: IslesVersion) -> Result<Arc<Self>, ModuleError> {
        Self::new(SharedParts::bare(version))
    }

    /// The game version's tables, hooks and route set.
    pub fn version(&self) -> &IslesVersion {
        &self.version
    }

    /// The module's settings.
    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    /// The node the module runs on.
    pub fn node(&self) -> &NodeIdentity {
        &self.node
    }

    /// The module's game data folders.
    pub fn game_data(&self) -> &GameDataRoot {
        &self.game_data
    }

    /// Whether a database is configured.
    pub fn database_wanted(&self) -> bool {
        self.database_wanted
    }

    /// The rifts seam.
    pub fn rifts(&self) -> &rustic_rifts_seam::Rifts {
        &self.rifts
    }

    /// Whether the admin API is on.
    pub fn admin_api(&self) -> bool {
        self.admin_api
    }

    /// The module's sign-in area.
    pub fn auth(&self) -> &Arc<IslesAuth> {
        &self.auth
    }

    /// Home calls for accounts, through the module context or standalone.
    pub fn home(&self) -> Arc<dyn HomeApi> {
        Arc::clone(&self.home)
    }

    /// Who the chat plugin reports online.
    pub fn presence(&self) -> &Presence {
        &self.presence
    }

    /// A staff kick, ban or timeout: the player's game tokens stop working (so their game calls, a new hunt and
    /// chat refuse them until the launcher signs in again), and chat is told to end their sessions. A game server
    /// they are on is not told (it has no channel for it). 1 if chat had them online, else 0.
    pub async fn kick_player(&self, user_id: &str, reason: &str) -> usize {
        self.auth.kick(user_id);
        let closed = self.presence.close_connections(user_id, 1008, reason);
        if let Some(context) = &self.context
            && let Ok(name) = dainty_module_api::EventName::new(dainty_module_api::chat::DISCONNECT_EVENT)
            && let Ok(payload) = serde_json::to_value(dainty_module_api::chat::DisconnectEvent {
                user_id: user_id.to_owned(),
                reason: reason.to_owned(),
            })
            && let Err(error) = context.emit(dainty_module_api::Event { name, payload }).await
        {
            tracing::warn!("kick: chat was not told to end {user_id}'s sessions: {error}");
        }
        closed
    }

    /// The Director slot; unconfigured until one fills it.
    pub fn dedicated_servers(&self) -> Arc<dyn DedicatedServers> {
        Arc::clone(&self.dedicated.read().unwrap_or_else(PoisonError::into_inner))
    }

    /// Fills the Director slot.
    pub fn set_dedicated_servers(&self, servers: Arc<dyn DedicatedServers>) {
        *self.dedicated.write().unwrap_or_else(PoisonError::into_inner) = servers;
    }

    /// The area state of type `T`, none when no area filled it.
    pub fn slot<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.slots.get::<T>()
    }

    /// The module context, none in a bare module.
    pub fn context(&self) -> Option<&ModuleContext> {
        self.context.as_ref()
    }

    /// Watches every request.
    pub fn observer(&self) -> &Arc<dyn Observer> {
        &self.observer
    }

    /// The admin auth, none until the Reforged unit fills it.
    pub fn admin_authenticator(&self) -> Option<Arc<dyn AdminAuthenticator>> {
        self.admin_authenticator
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Fills the admin auth slot.
    pub fn set_admin_authenticator(&self, authenticator: Arc<dyn AdminAuthenticator>) {
        *self.admin_authenticator.write().unwrap_or_else(PoisonError::into_inner) = Some(authenticator);
    }

    /// The backend's handlers (the composer's and the auth routes).
    pub fn handlers(&self) -> &Handlers {
        &self.handlers
    }
}

/// The module context's home calls; a failure of the call itself is `Unreachable`.
pub struct ContextHome(pub ModuleContext);

fn reach<T>(answer: Result<Result<T, HomeError>, ModuleError>) -> Result<T, HomeError> {
    answer.unwrap_or_else(|e| Err(HomeError::Unreachable(e.to_string())))
}

impl HomeApi for ContextHome {
    fn resolve_home<'a>(&'a self, account: &'a GlobalId) -> BoxFuture<'a, Result<NodeIdentity, HomeError>> {
        Box::pin(async move { reach(self.0.resolve_home(account).await) })
    }

    fn session_ticket<'a>(
        &'a self,
        account: &'a GlobalId,
        audience: &'a NodeId,
    ) -> BoxFuture<'a, Result<SessionTicket, HomeError>> {
        Box::pin(async move { reach(self.0.session_ticket(account, audience).await) })
    }

    fn forward<'a>(&'a self, call: &'a HomeForward) -> BoxFuture<'a, Result<serde_json::Value, HomeError>> {
        Box::pin(async move {
            let answer = self
                .0
                .forward_home(
                    &call.account,
                    &call.actor,
                    &call.operation,
                    call.body.clone(),
                    call.key.clone(),
                )
                .await;
            reach(answer)
        })
    }

    fn admit<'a>(
        &'a self,
        ticket: &'a SessionTicket,
        game: &'a GameVersion,
    ) -> BoxFuture<'a, Result<GlobalId, HomeError>> {
        Box::pin(async move { reach(self.0.admit_ticket(ticket, game).await) })
    }

    fn verify_session_ticket<'a>(
        &'a self,
        ticket: &'a SessionTicket,
        game: &'a GameVersion,
    ) -> BoxFuture<'a, Result<GlobalId, HomeError>> {
        Box::pin(async move { reach(self.0.verify_session_ticket(ticket, game).await) })
    }

    fn register_account_key_hash<'a>(
        &'a self,
        key_sha256: &'a str,
        account: &'a GlobalId,
    ) -> BoxFuture<'a, Result<(), HomeError>> {
        Box::pin(async move { reach(self.0.register_account_key_hash(key_sha256, account).await) })
    }

    fn register_directory_account<'a>(
        &'a self,
        account: &'a GlobalId,
        username: &'a str,
    ) -> BoxFuture<'a, Result<String, HomeError>> {
        Box::pin(async move { reach(self.0.register_directory_account(account, username).await) })
    }

    fn ring_submit_ticket<'a>(
        &'a self,
        account: &'a GlobalId,
        queue: &'a str,
        game: &'a GameVersion,
        party_size: u16,
        client_ip: &'a str,
    ) -> BoxFuture<'a, Result<PublishedTicket, HomeError>> {
        Box::pin(async move {
            reach(
                self.0
                    .ring_submit_ticket(account, queue, game, party_size, client_ip)
                    .await,
            )
        })
    }

    fn ring_try_form_matches<'a>(
        &'a self,
        queue: &'a str,
        game: &'a GameVersion,
    ) -> BoxFuture<'a, Result<Vec<InstanceRecord>, HomeError>> {
        Box::pin(async move { reach(self.0.ring_try_form_matches(queue, game).await) })
    }

    fn ring_ticket_state<'a>(&'a self, ticket: &'a GlobalId) -> BoxFuture<'a, Result<Option<TicketState>, HomeError>> {
        Box::pin(async move { reach(self.0.ring_ticket_state(ticket).await) })
    }

    fn ring_cancel_ticket<'a>(&'a self, ticket: &'a GlobalId) -> BoxFuture<'a, Result<bool, HomeError>> {
        Box::pin(async move { reach(self.0.ring_cancel_ticket(ticket).await) })
    }

    fn ring_lookup_instance<'a>(
        &'a self,
        instance: &'a GlobalId,
    ) -> BoxFuture<'a, Result<Option<InstanceRecord>, HomeError>> {
        Box::pin(async move { reach(self.0.ring_lookup_instance(instance).await) })
    }

    fn ring_register_instance<'a>(&'a self, record: &'a InstanceRecord) -> BoxFuture<'a, Result<(), HomeError>> {
        Box::pin(async move { reach(self.0.ring_register_instance(record).await) })
    }

    fn ring_resolve_join<'a>(
        &'a self,
        account: &'a GlobalId,
        client_ip: &'a str,
        instance: &'a GlobalId,
    ) -> BoxFuture<'a, Result<JoinDecision, HomeError>> {
        Box::pin(async move { reach(self.0.ring_resolve_join(account, client_ip, instance).await) })
    }

    fn ring_assign_ramsgate<'a>(
        &'a self,
        account: &'a GlobalId,
        client_ip: &'a str,
        game: &'a GameVersion,
    ) -> BoxFuture<'a, Result<(InstanceRecord, Option<SessionTicket>), HomeError>> {
        Box::pin(async move { reach(self.0.ring_assign_ramsgate(account, client_ip, game).await) })
    }

    fn ring_forward_hunt_rewards<'a>(&'a self, rewards: &'a HuntRewards) -> BoxFuture<'a, Result<(), HomeError>> {
        Box::pin(async move { reach(self.0.ring_forward_hunt_rewards(rewards).await) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presence_and_slots() {
        let presence = Presence::default();
        presence.set_online("u1");
        assert!(presence.is_online("u1"));
        assert_eq!(presence.online_user_ids(), ["u1"]);
        assert_eq!(presence.close_connections("u1", 1008, "kicked"), 1);
        assert_eq!(presence.close_connections("u1", 1008, "kicked"), 0);
        presence.set_online("u2");
        presence.set_offline("u2");
        assert!(!presence.is_online("u2"));

        let slots = Slots::default().with(Arc::new(7u32));
        assert_eq!(slots.get::<u32>().as_deref(), Some(&7));
        assert!(slots.get::<String>().is_none());
    }
}
