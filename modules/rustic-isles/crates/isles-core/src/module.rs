//! The game module both version crates are (DD3-DD5, DD8): a [`ModuleSpec`] of manifest, hooks, tables, rifts and
//! services, run compiled in or as the version crate's executable through `module_api::child::main`.
//!
//! It registers `/` with the gateway on its `backend` port (the gateway picks the module by the invite's version,
//! DF4a), the Reforged API's host-only routes as host-only, its game servers' UDP range, and three config sections:
//! `backend` (the env file), `switches` (the shared switches, defaulting to the version's, RU3c seam 6) and
//! `game_data` (the export folders, seam 4). Every listener binds 127.0.0.1 (AGENTS.md).

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use isles_chat::{ChatRules, ChatSource};
use module_api::chat::{ChatAnswer, ChatQuery};
use module_api::manifest::{PortDefault, Transport};
use module_api::module::PlayerInfo;
use module_api::registrar::ConfigSection;
use module_api::{
    API_VERSION, AdminAction, AdminCall, AdminRequest, AdminResponse, BoxFuture, Capabilities, ErrorKind, Event,
    EventName, GameModule, LogLevel, ModuleConfig, ModuleContext, ModuleError, ModuleKind, ModuleManifest,
    ModuleStatus, Phase, PortRange, Registrar,
};
use serde_json::{Value, json};
use store::{DatabaseConfig, DatabaseOptions};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::areas::matchmaking::qos;
use crate::areas::matchmaking::state::MatchmakingState;
use crate::db::{ContextHome, IslesStore};
use crate::game_data::GameDataRoot;
use crate::http::{Rifts, Shared};
use crate::routes::Service;
use crate::settings::{SWITCHES, Settings};
use crate::version::IslesVersion;

/// The Reforged API routes `HostOnly` guards (`reforgedApi.ts`: the host's admin key from a loopback peer). The
/// gateway refuses them from outside whatever the module does (AGENTS.md).
pub const HOST_ONLY_PREFIXES: [&str; 5] = [
    "/reforged/api/CreateInvite",
    "/reforged/api/InviteCodes",
    "/reforged/api/InviteCode",
    "/reforged/api/SetAccountStatus",
    "/reforged/api/Kick",
];

/// Sent once the listeners are up: `{"backend": "127.0.0.1:61000", "director": ...}`.
pub const LISTENING_EVENT: &str = "isles.listening";

/// A listener of the module, on a TCP port its manifest names.
pub trait PortService: Send + Sync {
    fn port(&self) -> &'static str;

    fn app(&self, shared: Arc<Shared>) -> Result<axum::Router, ModuleError>;

    /// Called once the listener is up (the Director boots its hubs here).
    fn started(&self) {}

    /// Called before the listeners stop; resolves when the service has let go of what it started (the Director's
    /// game servers).
    fn stopping(&self) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

/// The backend: every area's handlers over the route table.
#[derive(Debug, Clone, Copy, Default)]
pub struct BackendService;

impl PortService for BackendService {
    fn port(&self) -> &'static str {
        "backend"
    }

    fn app(&self, shared: Arc<Shared>) -> Result<axum::Router, ModuleError> {
        crate::http::app(shared, Service::Backend, &crate::areas::backend())
            .map_err(|e| ModuleError::failed(e.to_string()))
    }
}

/// The ports a version listens on when `host.toml` says nothing; the controller moves a clash (DD4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefaultPorts {
    pub backend: u16,
    pub director: u16,
    /// The game servers' UDP range, which the allowlist opens to admitted players
    pub game: PortRange,
}

pub struct ModuleSpec {
    pub id: &'static str,
    pub display_name: &'static str,
    pub invite_aliases: &'static [&'static str],
    pub capabilities: Capabilities,
    /// Built, not checked against recorded traffic (DD9)
    pub unverified: bool,
    pub ports: DefaultPorts,
    pub version: IslesVersion,
    pub chat: Arc<dyn ChatRules>,
    /// The version's rustic-rifts, from the module's settings and `IslesCore` (RF2)
    pub rifts: fn(&Settings, Arc<dyn rifts_seam::IslesCore>) -> Rifts,
    pub services: Vec<Arc<dyn PortService>>,
}

#[derive(Default)]
struct ChatPresenceTracker {
    online: Mutex<std::collections::HashSet<String>>,
}

impl crate::areas::matchmaking::state::ChatPresence for ChatPresenceTracker {
    fn is_online(&self, user_id: &str) -> bool {
        self.online
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(user_id)
    }
}

impl crate::areas::reforged_api::presence::ChatSessions for ChatPresenceTracker {
    fn online_user_ids(&self) -> Vec<String> {
        self.online
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .cloned()
            .collect()
    }

    fn close_connections(&self, user_id: &str, _code: u16, _reason: &str) -> usize {
        usize::from(
            self.online
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(user_id),
        )
    }
}

#[derive(Default)]
struct Running {
    phase: Option<Phase>,
    detail: Option<String>,
    servers: Vec<(oneshot::Sender<()>, JoinHandle<()>)>,
    store: Option<IslesStore>,
    shared: Option<Arc<Shared>>,
    presence: Arc<ChatPresenceTracker>,
    /// What the chat rules read, once started
    chat: Option<Arc<dyn ChatSource>>,
    /// Background tasks of the areas (the auth area's registration sweep), aborted at stop
    background: Vec<JoinHandle<()>>,
}

pub struct IslesModule {
    spec: ModuleSpec,
    running: Mutex<Running>,
}

fn lock(mutex: &Mutex<Running>) -> MutexGuard<'_, Running> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn failed(error: impl std::fmt::Display) -> ModuleError {
    ModuleError::failed(error.to_string())
}

/// A config value as text: strings as they are, other values as JSON.
fn text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

impl IslesModule {
    pub fn new(spec: ModuleSpec) -> Self {
        Self {
            spec,
            running: Mutex::new(Running::default()),
        }
    }

    pub fn spec(&self) -> &ModuleSpec {
        &self.spec
    }

    /// The settings a start reads: the `backend` section's env file, then the `switches` section.
    pub fn settings(config: &ModuleConfig) -> Result<Settings, ModuleError> {
        let env = match config
            .get("backend", "env_file")
            .and_then(text)
            .filter(|p| !p.trim().is_empty())
        {
            Some(path) => Settings::read_env_file(Path::new(path.trim())).map_err(failed)?,
            None => BTreeMap::new(),
        };
        let settings = Settings::new(env, Settings::switches_from_config(config.sections.get("switches")));
        // The Director's PORT_RANGE_BEGIN and _END are the range the controller gave the module, which the
        // allowlist opens to admitted players, not whatever the env file says
        Ok(match config.ports.get("game") {
            Some(range) => settings
                .forced("PORT_RANGE_BEGIN", range.first)
                .forced("PORT_RANGE_END", range.last),
            None => settings,
        })
    }

    fn open_store(settings: &Settings, ctx: &ModuleContext) -> Result<Option<IslesStore>, ModuleError> {
        // Without a database named, every route answers as a stub does (RU4)
        let wanted = settings.get("REFORGED_DB_FILE").is_some() || settings.get("REFORGED_DB_ENGINE").is_some();
        if !wanted {
            return Ok(None);
        }
        let lookup = |name: &str| settings.get(name).map(str::to_owned);
        let file = settings.get("REFORGED_DB_FILE").unwrap_or_default().into();
        let config = DatabaseConfig::from_lookup(lookup, file).map_err(failed)?;
        let home = Arc::new(ContextHome(ctx.clone()));
        IslesStore::open(&config, &DatabaseOptions::default(), home, ctx.node())
            .map(Some)
            .map_err(failed)
    }

    async fn start_services(&self, ctx: &ModuleContext, config: &ModuleConfig) -> Result<(), ModuleError> {
        let settings = Self::settings(config)?;
        let admin_api = settings.admin_api().map_err(failed)?;
        let game_data = GameDataRoot::from_config(
            config.get("game_data", "backend_dir").and_then(text).as_deref(),
            config.get("game_data", "director_dir").and_then(text).as_deref(),
        );
        let store = {
            let (settings, ctx) = (settings.clone(), ctx.clone());
            tokio::task::spawn_blocking(move || Self::open_store(&settings, &ctx))
                .await
                .map_err(failed)??
        };
        let matchmaking = Arc::new(MatchmakingState::from_settings(&settings));
        let presence_tracker = Arc::new(ChatPresenceTracker::default());
        matchmaking.set_chat_presence(presence_tracker.clone());
        crate::areas::reforged_api::presence::set_chat_sessions(Some(presence_tracker.clone()));
        if let Some(store) = &store {
            matchmaking.set_hunt_activity(Arc::new(crate::areas::reforged_api::activity::StoreHuntActivity {
                store: store.clone(),
                settings: settings.clone(),
            }));
        }
        let fallback_core: Arc<dyn rifts_seam::IslesCore> = Arc::new(crate::areas::auth::wiring::AuthReforgedCore {
            store: store.clone(),
            settings: settings.clone(),
        });
        let rifts_core: Arc<dyn rifts_seam::IslesCore> =
            Arc::new(crate::areas::matchmaking::rifts_core::MatchmakingCore {
                store: store.clone(),
                node: ctx.node().node.clone(),
                state: matchmaking.clone(),
                fallback: fallback_core,
            });
        let shared = Arc::new(Shared {
            version: self.spec.version.clone(),
            rifts: (self.spec.rifts)(&settings, rifts_core),
            matchmaking,
            settings,
            node: ctx.node().clone(),
            game_data,
            store: store.clone(),
            admin_api,
            auth: crate::areas::auth::AuthRuntime::default(),
        });
        let chat: Arc<dyn ChatSource> = Arc::new(crate::areas::chat::rules::source(
            ctx.node(),
            store.clone(),
            &shared.settings,
        ));
        let background = crate::areas::auth::start(&shared).await.map_err(failed)?;
        let mut servers = Vec::new();
        let mut addresses = serde_json::Map::new();
        for service in &self.spec.services {
            let port = config
                .ports
                .get(service.port())
                .map(|range| range.first)
                .or_else(|| self.default_port(service.port()))
                .ok_or_else(|| failed(format!("no {} port", service.port())))?;
            let app = service.app(shared.clone())?;
            let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
                .await
                .map_err(|e| failed(format!("{} on 127.0.0.1:{port}: {e}", service.port())))?;
            let address: SocketAddr = listener.local_addr().map_err(failed)?;
            addresses.insert(service.port().to_owned(), json!(address.to_string()));
            let (stop, stopped) = oneshot::channel::<()>();
            let name = service.port();
            let task = tokio::spawn(async move {
                let stop = async {
                    _ = stopped.await;
                };
                if let Err(error) = service_kit::serve::serve(listener, app, stop, Duration::from_secs(2)).await {
                    tracing::error!("{name}: {error}");
                }
            });
            servers.push((stop, task));
        }
        // The QoS pong answers on loopback; the gateway publishes it on the public QOS_PORT (areas/matchmaking/qos.rs)
        if let Some(pong) = qos::start(&shared.settings, &self.spec.version.tables).await {
            addresses.insert("qos".to_owned(), json!(pong.address.to_string()));
            addresses.insert("qos_public_port".to_owned(), json!(pong.public_port.to_string()));
            servers.push((pong.stop, pong.task));
        }
        // The party outbox's headlines go to the chat plugin as events (areas/matchmaking/outbox.rs)
        if let Some(store) = &store
            && let Some(dispatcher) = crate::areas::matchmaking::outbox::start(store.clone(), ctx.clone())
        {
            servers.push(dispatcher);
        }
        let detail = addresses
            .iter()
            .map(|(name, address)| format!("{name} {}", address.as_str().unwrap_or_default()))
            .collect::<Vec<_>>()
            .join(", ");
        {
            let mut running = lock(&self.running);
            running.servers = servers;
            running.background = background;
            running.store = store;
            running.shared = Some(shared.clone());
            running.presence = presence_tracker;
            running.chat = Some(chat);
            running.phase = Some(Phase::Ready);
            running.detail = Some(detail.clone());
        }
        for service in &self.spec.services {
            service.started();
        }
        ctx.log(
            LogLevel::Info,
            format!("{} listening: {detail}", self.spec.display_name),
        )
        .await?;
        let name = EventName::new(LISTENING_EVENT).map_err(failed)?;
        ctx.emit(Event {
            name,
            payload: Value::Object(addresses),
        })
        .await
    }

    fn default_port(&self, name: &str) -> Option<u16> {
        match name {
            "backend" => Some(self.spec.ports.backend),
            "director" => Some(self.spec.ports.director),
            _ => None,
        }
    }
}

impl GameModule for IslesModule {
    fn manifest(&self) -> ModuleManifest {
        let spec = &self.spec;
        let id_and_version = (spec.id.parse(), spec.version.tables.game_version.parse());
        let (Ok(id), Ok(game_version)) = id_and_version else {
            unreachable!("a version crate's constants parse, as its manifest test checks")
        };
        let tcp = |name: &str, port: u16| PortDefault {
            name: name.to_owned(),
            transport: Transport::Tcp,
            range: PortRange::single(port),
        };
        ModuleManifest {
            id,
            kind: ModuleKind::Game,
            display_name: spec.display_name.to_owned(),
            game_version,
            invite_aliases: spec.invite_aliases.iter().map(|a| (*a).to_owned()).collect(),
            api: API_VERSION,
            ports: vec![
                tcp("backend", spec.ports.backend),
                tcp("director", spec.ports.director),
                PortDefault {
                    name: "game".to_owned(),
                    transport: Transport::Udp,
                    range: spec.ports.game,
                },
            ],
            capabilities: spec.capabilities.clone(),
            unverified: spec.unverified,
            // DE4 fills the client profile and its Epic build
            client: None,
        }
    }

    fn register(&self, registrar: &mut Registrar) -> Result<(), ModuleError> {
        registrar.route("/", "backend");
        for prefix in HOST_ONLY_PREFIXES {
            registrar.host_only_route(prefix, "backend");
        }
        registrar.udp_ports("game");
        for action in [
            AdminAction::Kick,
            AdminAction::Ban,
            AdminAction::Unban,
            AdminAction::LiftTimeout,
            AdminAction::Lookup,
        ] {
            registrar.admin_action(action);
        }
        registrar.emits(EventName::new(LISTENING_EVENT).map_err(failed)?);
        registrar.emits(EventName::new(isles_chat::headline::HEADLINE_EVENT).map_err(failed)?);
        registrar.subscribes(EventName::new(module_api::chat::PRESENCE_EVENT).map_err(failed)?);
        registrar.config_section(ConfigSection::new("backend").key(
            "env_file",
            json!(""),
            "The env file the backend and Director read, under Node's variable names",
        ));
        let mut switches = ConfigSection::new("switches");
        for switch in SWITCHES {
            let default = self.spec.version.tables.switch_default(switch).unwrap_or_default();
            switches = switches.key(switch.key, json!(default), switch.help);
        }
        registrar.config_section(switches);
        registrar.config_section(
            ConfigSection::new("game_data")
                .key(
                    "backend_dir",
                    json!(""),
                    "The backend's game data exports for this version",
                )
                .key("director_dir", json!(""), "The Director's hunt tables for this version"),
        );
        Ok(())
    }

    fn start(&self, ctx: ModuleContext, config: ModuleConfig) -> BoxFuture<'_, Result<(), ModuleError>> {
        Box::pin(async move {
            {
                let mut running = lock(&self.running);
                if matches!(running.phase, Some(Phase::Ready | Phase::Starting)) {
                    return Err(ModuleError::new(ErrorKind::Invalid, "already started"));
                }
                running.phase = Some(Phase::Starting);
            }
            let started = self.start_services(&ctx, &config).await;
            if let Err(error) = &started {
                let mut running = lock(&self.running);
                running.phase = Some(Phase::Failed);
                running.detail = Some(error.message.clone());
            }
            started
        })
    }

    fn stop(&self, ctx: ModuleContext) -> BoxFuture<'_, Result<(), ModuleError>> {
        Box::pin(async move {
            for service in &self.spec.services {
                service.stopping().await;
            }
            let (servers, store) = {
                let mut running = lock(&self.running);
                running.phase = Some(Phase::Stopping);
                running.chat = None;
                running.shared = None;
                for task in std::mem::take(&mut running.background) {
                    task.abort();
                }
                (std::mem::take(&mut running.servers), running.store.take())
            };
            for (stop, task) in servers {
                _ = stop.send(());
                if tokio::time::timeout(Duration::from_secs(5), task).await.is_err() {
                    tracing::warn!("a listener did not stop within 5 s");
                }
            }
            if let Some(store) = store
                && let Err(error) = store.database().checkpoint().await
            {
                tracing::warn!("checkpoint: {error}");
            }
            {
                let mut running = lock(&self.running);
                running.phase = Some(Phase::Stopped);
                running.detail = None;
            }
            ctx.log(LogLevel::Info, format!("{} stopped", self.spec.display_name))
                .await
        })
    }

    fn status(&self) -> ModuleStatus {
        let running = lock(&self.running);
        ModuleStatus {
            phase: running.phase.unwrap_or(Phase::Loaded),
            detail: running.detail.clone(),
        }
    }

    fn chat(&self, _ctx: ModuleContext, query: ChatQuery) -> BoxFuture<'_, Result<ChatAnswer, ModuleError>> {
        Box::pin(async move {
            let source = lock(&self.running).chat.clone();
            // A module that is not running refuses, so the plugin never opens a room the rules would have closed
            let source = source.ok_or_else(|| ModuleError::new(ErrorKind::Unavailable, "the module is not running"))?;
            self.spec.chat.answer(&*source, &query).await
        })
    }

    fn admin(&self, _ctx: ModuleContext, call: AdminCall) -> BoxFuture<'_, Result<AdminResponse, ModuleError>> {
        Box::pin(async move {
            let store = lock(&self.running).store.clone();
            let store = store.ok_or_else(|| ModuleError::new(ErrorKind::Unavailable, "the module has no database"))?;
            handle_admin(&store, call).await
        })
    }

    fn event(&self, _ctx: ModuleContext, event: Event) -> BoxFuture<'_, Result<(), ModuleError>> {
        Box::pin(async move {
            if event.name.as_str() == module_api::chat::PRESENCE_EVENT {
                let Ok(presence) = serde_json::from_value::<module_api::chat::PresenceEvent>(event.payload) else {
                    return Ok(());
                };
                let (tracker, shared) = {
                    let running = lock(&self.running);
                    (running.presence.clone(), running.shared.clone())
                };
                if presence.online {
                    tracker
                        .online
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .insert(presence.user_id);
                } else {
                    tracker
                        .online
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .remove(&presence.user_id);
                    if let Some(shared) = shared {
                        let _ = crate::areas::matchmaking::party::Parties::of(&shared, None)
                            .leave(&presence.user_id)
                            .await;
                    }
                }
            }
            Ok(())
        })
    }
}

async fn resolve_player(
    store: &IslesStore,
    player: &str,
) -> Result<Option<(crate::ids::AccountId, String, String)>, ModuleError> {
    use crate::ids::{AccountId, UserId};
    use module_api::GlobalId;

    let raw = player.trim().to_owned();
    if raw.is_empty() {
        return Ok(None);
    }
    store
        .read(move |conn, repos| {
            if let Ok(global) = raw.parse::<GlobalId>()
                && let Ok(id) = AccountId::try_from(global)
            {
                if let Some(acc) = repos.accounts.find(conn, &id)? {
                    return Ok(Some((acc.id, acc.user_id.0, acc.name)));
                }
                if let Some(la) = repos.launcher_accounts.find_by_account(conn, &id)? {
                    return Ok(Some((la.account.clone(), la.account.to_string(), la.display_name)));
                }
            }
            if let Some(acc) = repos.accounts.find_by_user_id(conn, &UserId(raw.clone()))? {
                return Ok(Some((acc.id, acc.user_id.0, acc.name)));
            }
            let normalized = raw.to_lowercase();
            if let Some(la) = repos
                .launcher_accounts
                .find_by_display_name_normalized(conn, &normalized)?
            {
                let user_id = repos
                    .accounts
                    .find(conn, &la.account)?
                    .map_or_else(|| la.account.to_string(), |a| a.user_id.0);
                return Ok(Some((la.account, user_id, la.display_name)));
            }
            Ok(None)
        })
        .await
        .map_err(failed)
}

async fn handle_admin(store: &IslesStore, call: AdminCall) -> Result<AdminResponse, ModuleError> {
    use crate::areas::reforged_api::launcher_api::forget_status;
    use crate::areas::reforged_api::{presence, util};
    use crate::ids::AccountId;
    use crate::repo::auth::{AccessChange, OperationalStatus};

    let actor = call.actor;
    let revoke_and_close = |store: IslesStore,
                            actor: module_api::ActorContext,
                            target: AccountId,
                            user_id: String,
                            reason: &'static str| async move {
        store
            .account_write(&target, &actor, {
                let target = target.clone();
                move |tx, repos| {
                    let now = util::now_iso();
                    repos.refresh_sessions.revoke_all_for_account(tx, &target, &now)?;
                    repos.game_exchange_codes.revoke_unused_for_account(tx, &target, &now)
                }
            })
            .await
            .map_err(failed)?;
        if let Some(chat) = presence::chat_sessions() {
            chat.close_connections(&user_id, 1008, reason);
        }
        forget_status();
        Ok::<(), ModuleError>(())
    };

    let set_status = |store: IslesStore,
                      actor: module_api::ActorContext,
                      target: AccountId,
                      status: OperationalStatus,
                      reason: Option<String>| async move {
        let change = AccessChange {
            approval_status: None,
            status: Some(status),
            approval_updated_at: util::now_iso(),
            approval_updated_by: "host".to_owned(),
            approval_reason: reason.map(|r| r.chars().take(500).collect()),
        };
        store
            .account_write(&target, &actor, {
                let target = target.clone();
                move |tx, repos| repos.launcher_accounts.set_access_state(tx, &target, &change)
            })
            .await
            .map_err(failed)?;
        forget_status();
        Ok::<(), ModuleError>(())
    };

    match call.request {
        AdminRequest::Kick { player, .. } => {
            let Some((target, user_id, _)) = resolve_player(store, &player).await? else {
                return Ok(AdminResponse::NotFound);
            };
            revoke_and_close(store.clone(), actor, target, user_id, "kicked by the host").await?;
            Ok(AdminResponse::Done)
        }
        AdminRequest::Ban { player, reason, until } => {
            let Some((target, user_id, _)) = resolve_player(store, &player).await? else {
                return Ok(AdminResponse::NotFound);
            };
            let status = if until.is_some() {
                OperationalStatus::Disabled
            } else {
                OperationalStatus::Banned
            };
            set_status(store.clone(), actor.clone(), target.clone(), status, reason).await?;
            revoke_and_close(store.clone(), actor, target, user_id, "banned by the host").await?;
            Ok(AdminResponse::Done)
        }
        AdminRequest::Unban { player, reason } | AdminRequest::LiftTimeout { player, reason } => {
            let Some((target, _, _)) = resolve_player(store, &player).await? else {
                return Ok(AdminResponse::NotFound);
            };
            set_status(store.clone(), actor, target, OperationalStatus::Active, reason).await?;
            Ok(AdminResponse::Done)
        }
        AdminRequest::Lookup { player } => {
            let Some((target, user_id, name)) = resolve_player(store, &player).await? else {
                return Ok(AdminResponse::NotFound);
            };
            let online = presence::active_players(crate::areas::reforged_api::activity::ACTIVITY_SESSION_GAP_MS)
                .iter()
                .any(|u| u == &user_id);
            Ok(AdminResponse::Player(PlayerInfo {
                account: Some(target.global().clone()),
                name,
                online,
            }))
        }
    }
}
