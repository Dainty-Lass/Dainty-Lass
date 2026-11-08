// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The composition (spec §5): one call per start fills core's seams with every area, in the spec's order.

use std::sync::{Arc, OnceLock, PoisonError, RwLock, Weak};

use dainty_module_api::chat::{ChatAnswer, ChatQuery};
use dainty_module_api::network::{HomeInvocation, OperationName};
use dainty_module_api::{
    ActorContext, AdminCall, AdminResponse, BoxFuture, ErrorKind, GlobalId, ModuleContext, ModuleError,
};
use rustic_isles::{
    AccountId, ComposeInputs, Composer, Composition, ContextHome, Handlers, ModuleHooks, Shared, Slots, Started,
    Startup,
};
use rustic_isles_auth::{GameAccount, GameLogin, Seams};
use rustic_isles_characters::Characters;
use rustic_isles_chat::{ChatRules, HubSessions, IslesChat, StoreChat};
use rustic_isles_economy::EconomyCaches;
use rustic_isles_economy::bounties::{BountiesSeams, ForeignAccounts};
use rustic_isles_economy::inventory::{InventoryActivitySlot, InventoryPorts, RingInstances};
use rustic_isles_economy::progression::{ProgressionPorts, SessionSupplies, StoreCharacters};
use rustic_isles_launcher_api::activity::ActivityHooks;
use rustic_isles_launcher_api::admin::{EconomyFunding, LauncherApiAdmin, admin_startups};
use rustic_isles_launcher_api::{LauncherApiSeams, LauncherApiState, RequestObserver, public_startups};
use rustic_isles_matchmaking::{HttpDirector, MatchmakingCore, MatchmakingState, ring_instances_slot};
use rustic_isles_store::IslesStore;
use serde_json::Value;

use crate::accounts::AccountsCore;
use crate::data::{VersionData, empty_inputs};
use crate::realms::RealmsAdapter;

/// The home operations core §7.8 routes to this module.
const HOME_OPERATIONS: [&str; 11] = [
    "progression.grant",
    "progression.read",
    "inventory.write",
    "inventory.read",
    "bounty.write",
    "bounty.read",
    "character.load",
    "account.write",
    "chat.login",
    "chat.player",
    "chat.party",
];

/// The module's `Shared`, which core builds after `compose` returns; the first start-up fills it (§5.5).
#[derive(Default)]
pub struct SharedCell(RwLock<Option<Weak<Shared>>>);

impl SharedCell {
    /// An empty cell; the first start-up fills it.
    pub fn new() -> Self {
        Self::default()
    }

    /// Points the cell at the module's `Shared`, holding it weakly.
    pub fn fill(&self, shared: &Arc<Shared>) {
        *self.0.write().unwrap_or_else(PoisonError::into_inner) = Some(Arc::downgrade(shared));
    }

    /// The module's `Shared`, or `None` before start-up or once it is dropped.
    pub fn get(&self) -> Option<Arc<Shared>> {
        self.0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()?
            .upgrade()
    }
}

impl std::fmt::Debug for SharedCell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedCell")
            .field("filled", &self.get().is_some())
            .finish()
    }
}

#[derive(Debug, Clone)]
enum Source {
    File,
    Given(Arc<VersionData>),
}

/// Core's composer for 1.14.7; it keeps no state between starts.
#[derive(Debug, Clone)]
pub struct VersionComposer {
    source: Source,
}

impl VersionComposer {
    /// Reads `version_data.json` from the backend game data folder when a database is wanted.
    pub fn reading_file() -> Self {
        Self { source: Source::File }
    }

    /// Uses the given version data instead of reading `version_data.json`.
    pub fn with_data(data: VersionData) -> Self {
        Self {
            source: Source::Given(Arc::new(data)),
        }
    }

    async fn version_data(&self, inputs: &ComposeInputs) -> Result<VersionData, ModuleError> {
        match &self.source {
            Source::Given(data) => Ok(VersionData::clone(data)),
            Source::File => {
                let root = inputs.game_data.clone();
                tokio::task::spawn_blocking(move || VersionData::load(&root))
                    .await
                    .map_err(|e| ModuleError::failed(e.to_string()))?
                    .map_err(ModuleError::failed)
            }
        }
    }
}

async fn open_store(inputs: &ComposeInputs) -> Result<Option<IslesStore>, ModuleError> {
    if !inputs.database_wanted {
        return Ok(None);
    }
    let settings = inputs.settings.clone();
    let node = inputs.node.clone();
    let home = Arc::new(ContextHome(inputs.context.clone()));
    tokio::task::spawn_blocking(move || {
        IslesStore::open_configured(|name| settings.get(name).map(str::to_owned), home, &node)
    })
    .await
    .map_err(|e| ModuleError::failed(e.to_string()))?
    .map_err(|e| ModuleError::failed(e.to_string()))
}

impl Composer for VersionComposer {
    fn compose(&self, inputs: ComposeInputs) -> BoxFuture<'_, Result<Composition, ModuleError>> {
        Box::pin(async move {
            // 1-2: the database, then the version data only with one
            let store = open_store(&inputs).await?;
            let engine_is_postgres = store
                .as_ref()
                .is_some_and(|s| s.database().engine() == dainty_store::Engine::Postgres);
            let (character_data, starter_kit) = match &store {
                Some(_) => {
                    let data = self.version_data(&inputs).await?;
                    (data.character_data(), data.starter_kit())
                }
                None => empty_inputs(),
            };

            // 3: per-module state, none of it process-wide
            let characters = Arc::new(Characters::new(character_data, starter_kit));
            let matchmaking = Arc::new(MatchmakingState::new());
            let hubs = Arc::new(HubSessions::new());
            let reforged = Arc::new(LauncherApiState::new());
            let cell = Arc::new(SharedCell::new());
            let activity = Arc::new(ActivityHooks::default());
            let seams = LauncherApiSeams::new();

            // 4-5: the rifts cores, then rifts off the async threads
            let accounts: Arc<dyn rustic_rifts_seam::IslesCore> = Arc::new(AccountsCore::new(
                store.clone(),
                inputs.settings.clone(),
                Arc::clone(&cell),
            ));
            let core: Arc<dyn rustic_rifts_seam::IslesCore> = Arc::new(MatchmakingCore::new(
                store.clone(),
                inputs.node.node.clone(),
                Arc::clone(&matchmaking),
                accounts,
            ));
            let settings = inputs.settings.clone();
            let rifts = tokio::task::spawn_blocking(move || crate::rifts_with_core(&settings, core))
                .await
                .map_err(|e| ModuleError::failed(e.to_string()))?;

            // 6: every area's rows
            let ring: Arc<dyn RingInstances> = Arc::clone(&matchmaking) as Arc<dyn RingInstances>;
            let inventory_ports = InventoryPorts {
                ring: Arc::clone(&ring),
                ..InventoryPorts::default()
            };
            let progression_ports = ProgressionPorts {
                characters: Arc::new(StoreCharacters),
                activity: Arc::clone(&activity) as Arc<dyn rustic_isles_economy::progression::Activity>,
                ring,
                inventory: inventory_ports.clone(),
            };
            let handlers = handlers(inventory_ports, progression_ports.clone());

            // 7: slots
            reforged.set_matchmaking_presence(Arc::clone(&matchmaking) as _);
            reforged.set_matchmaking_candidates(Arc::clone(&matchmaking) as _);
            if store.is_some() {
                matchmaking.set_hunt_activity(Arc::clone(&activity) as _);
            }
            let discord: Arc<dyn rustic_isles_auth::DiscordApi> = Arc::new(rustic_isles_auth::HttpsDiscord::new());
            let mut slots = Slots::default()
                .with(characters)
                .with(Arc::clone(&matchmaking))
                .with(Arc::clone(&hubs))
                .with(Arc::clone(&reforged))
                .with(Arc::new(ring_instances_slot(&matchmaking)))
                .with(Arc::new(BountiesSeams {
                    foreign: Arc::new(Foreign),
                    activity: Arc::clone(&activity) as _,
                }))
                .with(Arc::new(InventoryActivitySlot(Arc::clone(&activity) as _)))
                .with(Arc::new(EconomyCaches::default()))
                .with(Arc::new(seams.clone()))
                .with(Arc::clone(&activity))
                .with(Arc::new(LauncherApiAdmin::new(
                    Arc::clone(&discord),
                    Arc::new(EconomyFunding),
                )));
            if let Some(store) = &store {
                slots.insert(Arc::new(store.clone()));
            }

            // 8: auth
            let auth_store = store
                .clone()
                .map(|s| Arc::new(rustic_isles_store::IslesAuthStore::new(s)) as Arc<dyn rustic_isles_auth::AuthStore>);
            let auth_seams = Seams {
                activity: seams.activity(),
                operations: seams.operations(),
                notices: seams.notices(),
                erasure: seams.erasure(),
                realms: Arc::new(RealmsAdapter::new(rifts.clone())),
                game_login: Arc::new(LateSupplies {
                    cell: Arc::clone(&cell),
                    ports: progression_ports,
                }),
                home: Arc::new(ContextHome(inputs.context.clone())),
                discord,
                ..Seams::default()
            };

            // 11: hooks and start-ups (the admin authenticator is installed by the admin start-up, which needs
            // the module's Shared)
            let runtime = Arc::new(Runtime {
                cell,
                chat: OnceLock::new(),
                store: store.clone(),
                hubs,
                matchmaking,
                module_id: inputs.module_id.clone(),
            });
            let qos_port = inputs
                .ports
                .get("qos")
                .map_or(crate::DEFAULT_PORTS.qos, |range| range.first);
            let mut startups: Vec<Arc<dyn Startup>> = vec![Arc::new(Bind(Arc::clone(&runtime)))];
            startups.extend(admin_startups());
            startups.extend(public_startups());
            startups.extend(rustic_isles_matchmaking::startups_on(qos_port));
            Ok(Composition {
                slots,
                auth_store,
                auth_seams,
                engine_is_postgres,
                rifts,
                handlers,
                // 9: the Reforged request log and metrics
                observer: Arc::new(RequestObserver::new(reforged)),
                admin_authenticator: None,
                hooks: runtime,
                startups,
            })
        })
    }
}

/// Every area's rows on the backend (§5.3); core adds auth's 49.
pub(crate) fn handlers(inventory: InventoryPorts, progression: ProgressionPorts) -> Handlers {
    let mut handlers = Handlers::new();
    rustic_isles_gamedata::register(&mut handlers);
    rustic_isles_characters::register(&mut handlers);
    rustic_isles_chat::register(&mut handlers);
    rustic_isles_economy::inventory::register(&mut handlers, inventory);
    rustic_isles_economy::progression::register(&mut handlers, progression);
    rustic_isles_economy::bounties::register(&mut handlers);
    rustic_isles_matchmaking::register(&mut handlers);
    rustic_isles_launcher_api::register_public(&mut handlers);
    rustic_isles_launcher_api::register_admin(&mut handlers);
    handlers
}

/// Bounties' foreign accounts (§5.6.4): the account a text names, when its owner node is not this one.
struct Foreign;

impl ForeignAccounts for Foreign {
    fn foreign_account<'a>(&'a self, shared: &'a Shared, user_id: &'a str) -> BoxFuture<'a, Option<GlobalId>> {
        Box::pin(async move {
            let named = match user_id
                .parse::<GlobalId>()
                .ok()
                .and_then(|g| AccountId::try_from(g).ok())
            {
                Some(account) => Some(account.global().clone()),
                None => {
                    let store = shared.slot::<IslesStore>()?;
                    let user = rustic_isles::UserId::new(user_id);
                    let found = store
                        .read(move |conn, repos| repos.accounts.find_by_user_id(conn, &user))
                        .await
                        .ok()??;
                    Some(found.id.global().clone())
                }
            }?;
            (named.owner() != &shared.node().node).then_some(named)
        })
    }
}

/// Progression's session supplies, built over the module's `Shared` once it exists.
struct LateSupplies {
    cell: Arc<SharedCell>,
    ports: ProgressionPorts,
}

impl GameLogin for LateSupplies {
    fn session_supplies<'a>(&'a self, account: &'a GameAccount) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if let Some(shared) = self.cell.get() {
                SessionSupplies::new(shared, self.ports.clone())
                    .session_supplies(account)
                    .await;
            }
        })
    }
}

/// What the hooks and the bind start-up share.
struct Runtime {
    cell: Arc<SharedCell>,
    chat: OnceLock<Arc<StoreChat>>,
    store: Option<IslesStore>,
    hubs: Arc<HubSessions>,
    matchmaking: Arc<MatchmakingState>,
    module_id: dainty_module_api::ModuleId,
}

fn not_running() -> ModuleError {
    ModuleError::new(ErrorKind::Unavailable, "the module is not running")
}

/// Start-up 1 (§5.8): the late cell, the Director over HTTP, the chat source and the multipliers line.
struct Bind(Arc<Runtime>);

impl Startup for Bind {
    fn start<'a>(&'a self, shared: &'a Arc<Shared>) -> BoxFuture<'a, Result<Started, String>> {
        Box::pin(async move {
            let runtime = &self.0;
            runtime.cell.fill(shared);
            runtime.matchmaking.bind_shared(shared);
            // The Director service swaps in its in-process one when it can launch (director §12.3)
            shared.set_dedicated_servers(HttpDirector::from_settings(shared.settings()));
            let chat = StoreChat::for_module(shared, runtime.module_id.clone(), runtime.store.clone());
            let _ = runtime.chat.set(Arc::new(chat));
            let multipliers = rustic_isles_economy::multipliers::read_test_multipliers(shared.settings());
            if multipliers.is_on() {
                tracing::warn!("{}", multipliers.startup_line());
            } else {
                tracing::info!("{}", multipliers.startup_line());
            }
            Ok(Started::default())
        })
    }
}

impl ModuleHooks for Runtime {
    fn chat<'a>(
        &'a self,
        _shared: &'a Arc<Shared>,
        _ctx: ModuleContext,
        query: ChatQuery,
    ) -> BoxFuture<'a, Result<ChatAnswer, ModuleError>> {
        Box::pin(async move {
            let source = self.chat.get().ok_or_else(not_running)?;
            IslesChat.answer(&**source, &self.hubs, &query).await
        })
    }

    fn admin<'a>(
        &'a self,
        shared: &'a Arc<Shared>,
        call: AdminCall,
    ) -> BoxFuture<'a, Result<AdminResponse, ModuleError>> {
        Box::pin(async move { LauncherApiAdmin::of(shared).admin_action(shared, call).await })
    }

    fn home_operation<'a>(
        &'a self,
        shared: &'a Arc<Shared>,
        operation: &'a str,
        account: &'a AccountId,
        actor: ActorContext,
        body: Value,
    ) -> BoxFuture<'a, Result<Option<Value>, ModuleError>> {
        Box::pin(async move {
            if !HOME_OPERATIONS.contains(&operation) {
                return Ok(None);
            }
            if operation.starts_with("chat.") {
                // The chat source exists only once the module is built, so chat answers here, not in the admin half
                let chat = self.chat.get().ok_or_else(not_running)?;
                let name = OperationName::new(operation).map_err(|e| ModuleError::failed(e.to_string()))?;
                let invocation = HomeInvocation {
                    from: shared.node().node.clone(),
                    account: account.global().clone(),
                    actor,
                    operation: name,
                    body,
                };
                return rustic_isles_chat::home_operation(operation, chat, &invocation)
                    .await
                    .transpose();
            }
            LauncherApiAdmin::of(shared)
                .home_operation(shared, operation, account, actor, body)
                .await
        })
    }

    fn place_hunt_players<'a>(
        &'a self,
        shared: &'a Arc<Shared>,
        players: &'a [GlobalId],
    ) -> BoxFuture<'a, Vec<String>> {
        rustic_isles_matchmaking::place_hunt_players(shared, players)
    }

    fn record_ring_instance<'a>(
        &'a self,
        shared: &'a Arc<Shared>,
        instance: &'a GlobalId,
        _host: &'a str,
        _port: u16,
        players: &'a [String],
    ) -> BoxFuture<'a, ()> {
        rustic_isles_matchmaking::record_ring_instance(shared, instance, players);
        Box::pin(async {})
    }

    fn player_offline<'a>(&'a self, shared: &'a Arc<Shared>, user_id: &'a str) -> BoxFuture<'a, ()> {
        rustic_isles_matchmaking::player_offline(shared, user_id)
    }

    fn stop(&self) -> BoxFuture<'_, Result<(), String>> {
        Box::pin(async move {
            match &self.store {
                Some(store) => store.database().checkpoint().await.map_err(|e| e.to_string()),
                None => Ok(()),
            }
        })
    }
}
