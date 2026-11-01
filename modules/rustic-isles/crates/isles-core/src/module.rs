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
use module_api::module::{PlayerInfo, PlayerRestriction, RestrictionKind};
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
use crate::settings::{OPTIONAL_SWITCHES, SWITCHES, Settings};
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

    /// The settings a start reads: the `backend` section's env file (merging sibling `admin.env`
    /// and `director.env` when given `backend.env` in a host kit config folder), then the `switches`
    /// section.
    pub fn settings(config: &ModuleConfig) -> Result<Settings, ModuleError> {
        let env = match config
            .get("backend", "env_file")
            .and_then(text)
            .filter(|p| !p.trim().is_empty())
        {
            Some(path) => {
                let path = Path::new(path.trim());
                let mut merged = BTreeMap::new();
                if path.file_name().is_some_and(|name| name == "backend.env")
                    && let Some(parent) = path.parent()
                {
                    for sibling in ["admin.env", "director.env"] {
                        let file = parent.join(sibling);
                        if file.is_file() {
                            merged.extend(Settings::read_env_file(&file).map_err(failed)?);
                        }
                    }
                }
                merged.extend(Settings::read_env_file(path).map_err(failed)?);
                merged
            }
            None => BTreeMap::new(),
        };
        let has_port_range = env.contains_key("PORT_RANGE_BEGIN") && env.contains_key("PORT_RANGE_END");
        let settings = Settings::new(env, Settings::switches_from_config(config.sections.get("switches")));
        // The Director's PORT_RANGE_BEGIN and _END come from the controller's assigned range when no
        // range was set in the env file or when the controller moved the range from the default 8-port
        // window; otherwise the wider range setup wrote for max_hunt_servers > 6 in director.env is kept.
        Ok(match config.ports.get("game") {
            Some(range) => {
                let default_window = matches!((range.first, range.last), (8770, 8777) | (8780, 8787));
                if !has_port_range || !default_window {
                    settings
                        .forced("PORT_RANGE_BEGIN", range.first)
                        .forced("PORT_RANGE_END", range.last)
                } else {
                    settings
                }
            }
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
        if let DatabaseConfig::Sqlite { file } = &config
            && let Some(parent) = file.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(failed)?;
        }
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
        let rifts = {
            let (settings, rifts_fn) = (settings.clone(), self.spec.rifts);
            tokio::task::spawn_blocking(move || rifts_fn(&settings, rifts_core))
                .await
                .map_err(failed)?
        };
        let shared = Arc::new(Shared {
            version: self.spec.version.clone(),
            rifts,
            matchmaking,
            settings,
            node: ctx.node().clone(),
            game_data,
            store: store.clone(),
            admin_api,
            auth: crate::areas::auth::AuthRuntime::default(),
        });
        let chat: Arc<dyn ChatSource> = Arc::new(
            crate::areas::chat::rules::source(ctx.node(), store.clone(), &shared.settings)
                .with_module_id(ctx.module_id().clone()),
        );
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
        for (key, env) in OPTIONAL_SWITCHES {
            switches = switches.key(key, Value::Null, format!("Optional host switch ({env})"));
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

    fn command(
        &self,
        _ctx: ModuleContext,
        request: module_api::CommandRequest,
    ) -> BoxFuture<'_, Result<module_api::CommandOutput, ModuleError>> {
        Box::pin(async move {
            let shared = lock(&self.running).shared.clone();
            let shared = shared.ok_or_else(|| ModuleError::new(ErrorKind::Unavailable, "the module is not running"))?;
            if let Some(director_res) = module_api::network::DirectorInvocation::from_command(&request) {
                let director_inv = director_res?;
                let data = handle_director_invocation(&shared, director_inv).await?;
                return Ok(module_api::CommandOutput {
                    text: "ok".to_owned(),
                    data: Some(data),
                });
            }
            if let Some(home_res) = module_api::network::HomeInvocation::from_command(&request) {
                let home_inv = home_res?;
                let data = handle_home_invocation(&shared, home_inv).await?;
                return Ok(module_api::CommandOutput {
                    text: "ok".to_owned(),
                    data: Some(data),
                });
            }
            Err(ModuleError::unsupported(format!("command {}", request.name)))
        })
    }
}

async fn handle_director_invocation(
    shared: &Arc<Shared>,
    invocation: module_api::network::DirectorInvocation,
) -> Result<Value, ModuleError> {
    use crate::areas::matchmaking::controller::RAMSGATE_HUNT;
    use crate::areas::matchmaking::deploy::{Launch, LaunchRequest};
    use module_api::network::{DirectorInvocation, PlacedInstance};

    let deploy = shared.matchmaking.dedicated_servers();
    if !deploy.configured() {
        return Err(ModuleError::new(
            ErrorKind::Unavailable,
            "dedicated servers are not configured on this node",
        ));
    }
    match invocation {
        DirectorInvocation::PlaceHunt {
            match_id,
            queue,
            game: _,
            players,
        } => {
            let mut expected_players = Vec::with_capacity(players.len());
            for p in &players {
                if let Some(store) = &shared.store
                    && p.owner() == &shared.node.node
                    && let Ok(acc_id) = crate::ids::AccountId::try_from(p.clone())
                    && let Ok(Some(rec)) = store.read(move |conn, repos| repos.accounts.find(conn, &acc_id)).await
                {
                    expected_players.push(rec.user_id.0);
                } else {
                    expected_players.push(p.to_string());
                }
            }
            let launch = deploy
                .launch(LaunchRequest {
                    game_mode: Some("ISLAND".to_owned()),
                    game_args: Some(String::new()),
                    hunt_id: queue,
                    expected_players,
                    request_id: None,
                })
                .await
                .map_err(failed)?;
            match launch {
                Launch::Online { host, port } => {
                    shared.matchmaking.record_ring_instance(match_id, &players);
                    let placed = PlacedInstance {
                        server_host: host,
                        server_port: u16::try_from(port).unwrap_or(7777),
                    };
                    serde_json::to_value(placed).map_err(failed)
                }
                Launch::Refused { reason } => Err(ModuleError::failed(
                    reason.unwrap_or_else(|| "director refused hunt launch".to_owned()),
                )),
            }
        }
        DirectorInvocation::PlaceHub {
            hub_id,
            region: _,
            shard: _,
            game: _,
        } => {
            if let Some(live) = deploy.hub_status().await
                && let Some(hub) = live.ramsgate
                && hub.ready
            {
                shared.matchmaking.record_ring_instance(hub_id, &[]);
                let placed = PlacedInstance {
                    server_host: hub.host,
                    server_port: u16::try_from(hub.port).unwrap_or(7777),
                };
                return serde_json::to_value(placed).map_err(failed);
            }
            let launch = deploy
                .launch(LaunchRequest {
                    game_mode: Some("CITY".to_owned()),
                    game_args: None,
                    hunt_id: RAMSGATE_HUNT.to_owned(),
                    expected_players: Vec::new(),
                    request_id: None,
                })
                .await
                .map_err(failed)?;
            match launch {
                Launch::Online { host, port } => {
                    shared.matchmaking.record_ring_instance(hub_id, &[]);
                    let placed = PlacedInstance {
                        server_host: host,
                        server_port: u16::try_from(port).unwrap_or(7777),
                    };
                    serde_json::to_value(placed).map_err(failed)
                }
                Launch::Refused { reason } => Err(ModuleError::failed(
                    reason.unwrap_or_else(|| "director refused hub launch".to_owned()),
                )),
            }
        }
    }
}

async fn handle_home_invocation(
    shared: &Arc<Shared>,
    invocation: module_api::network::HomeInvocation,
) -> Result<Value, ModuleError> {
    use crate::areas::reforged_api::util::now_iso;
    use crate::ids::AccountId;
    use crate::repo::bounties::{BountyGroup, BountyStateRecord, DraftData, StoredBounty};
    use crate::repo::characters::CharacterRecord;
    use crate::repo::inventory::InventoryRecord;
    use module_api::{Actor, ActorContext};

    let Some(store) = &shared.store else {
        return Err(ModuleError::new(ErrorKind::Unavailable, "the module has no database"));
    };
    if invocation.account.owner() != &shared.node.node {
        return Err(ModuleError::new(
            ErrorKind::Invalid,
            format!(
                "account {} is homed on {}, not {}",
                invocation.account,
                invocation.account.owner(),
                shared.node.node
            ),
        ));
    }
    let account_id = AccountId::try_from(invocation.account.clone())
        .map_err(|e| ModuleError::new(ErrorKind::Invalid, e.to_string()))?;
    let actor = ActorContext::new(Actor::Player {
        account: invocation.account.clone(),
    });
    match invocation.operation.as_str() {
        "progression.grant" => {
            let xp = invocation.body.get("xp").and_then(Value::as_i64).unwrap_or(0).max(0);
            let track = invocation
                .body
                .get("track")
                .and_then(Value::as_str)
                .unwrap_or("ExperienceTrack_PlayerLevel")
                .to_owned();
            let target = account_id.clone();
            let tr = track.clone();
            let updated = store
                .account_write(&account_id, &actor, move |tx, repos| {
                    repos.progression_tracks.increment(tx, &target, &tr, xp)
                })
                .await
                .map_err(failed)?;
            Ok(json!({
                "ok": true,
                "account": invocation.account.to_string(),
                "progression_id": updated.progression_id,
                "progress": updated.progress,
            }))
        }
        "progression.read" => {
            let target = account_id.clone();
            let tracks = store
                .read(move |conn, repos| repos.progression_tracks.get_all_for_account(conn, &target))
                .await
                .map_err(failed)?;
            let items: Vec<Value> = tracks
                .into_iter()
                .map(|t| {
                    json!({
                        "progression_id": t.progression_id,
                        "progress": t.progress,
                        "update_version": t.update_version,
                    })
                })
                .collect();
            Ok(json!({ "tracks": items }))
        }
        "inventory.write" => {
            let loot: Vec<String> = invocation
                .body
                .get("loot")
                .and_then(Value::as_array)
                .map(|arr| arr.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                .unwrap_or_default();
            let target = account_id.clone();
            let applied = store
                .account_write(&account_id, &actor, move |tx, repos| {
                    let chars = repos.characters.find_many_by_account(tx.conn(), &target)?;
                    let char_id = match chars.first() {
                        Some(c) => c.character_id.clone(),
                        None => {
                            let cid = format!("char-{}", target);
                            let now = now_iso();
                            repos.characters.create(
                                tx,
                                &CharacterRecord {
                                    character_id: cid.clone(),
                                    account: target.clone(),
                                    created_date: now.clone(),
                                    last_modified_date: now,
                                    name: "Slayer".to_owned(),
                                    update_version: 1,
                                    data: "{}".to_owned(),
                                },
                            )?;
                            cid
                        }
                    };
                    let existing = repos.inventories.find_by_character_id(tx.conn(), &char_id)?;
                    let (instanced_json, mut stacked, rev, is_new) = match existing {
                        Some(inv) => {
                            let parsed: Vec<Value> = serde_json::from_str(&inv.stacked_items).unwrap_or_default();
                            (inv.instanced_items, parsed, inv.revision.unwrap_or(0), false)
                        }
                        None => ("[]".to_owned(), Vec::new(), 0, true),
                    };
                    for item in &loot {
                        let (catalog_id, qty) = match item.split_once(':') {
                            Some((c, q)) => (c, q.parse::<i64>().unwrap_or(1).max(1)),
                            None => (item.as_str(), 1),
                        };
                        if let Some(existing_entry) = stacked
                            .iter_mut()
                            .find(|v| v.get("catalogId").and_then(Value::as_str) == Some(catalog_id))
                        {
                            let prev = existing_entry.get("quantity").and_then(Value::as_i64).unwrap_or(0);
                            if let Some(obj) = existing_entry.as_object_mut() {
                                obj.insert("quantity".into(), json!(prev + qty));
                            }
                        } else {
                            stacked.push(json!({
                                "catalogId": catalog_id,
                                "quantity": qty,
                            }));
                        }
                    }
                    let stacked_json = serde_json::to_string(&stacked).unwrap_or_else(|_| "[]".to_owned());
                    if is_new {
                        repos.inventories.create(
                            tx,
                            &InventoryRecord {
                                character_id: char_id,
                                account: Some(target),
                                instanced_items: instanced_json,
                                stacked_items: stacked_json,
                                revision: Some(1),
                                bootstrap_version: None,
                            },
                        )?;
                    } else {
                        repos.inventories.update_both_if_revision_matches(
                            tx,
                            &char_id,
                            &instanced_json,
                            &stacked_json,
                            rev,
                        )?;
                    }
                    Ok(loot.len())
                })
                .await
                .map_err(failed)?;
            Ok(json!({
                "ok": true,
                "account": invocation.account.to_string(),
                "loot_applied": applied,
            }))
        }
        "inventory.read" => {
            let target = account_id.clone();
            let inv = store
                .read(move |conn, repos| {
                    let chars = repos.characters.find_many_by_account(conn, &target)?;
                    if let Some(first) = chars.first() {
                        repos.inventories.find_by_character_id(conn, &first.character_id)
                    } else {
                        Ok(None)
                    }
                })
                .await
                .map_err(failed)?;
            match inv {
                Some(inv) => {
                    let stacked: Value = serde_json::from_str(&inv.stacked_items).unwrap_or_else(|_| json!([]));
                    let instanced: Value = serde_json::from_str(&inv.instanced_items).unwrap_or_else(|_| json!([]));
                    Ok(json!({
                        "character_id": inv.character_id,
                        "stacked_items": stacked,
                        "instanced_items": instanced,
                    }))
                }
                None => Ok(json!({ "stacked_items": [], "instanced_items": [] })),
            }
        }
        "bounty.write" => {
            let bounty_progress = invocation
                .body
                .get("bounty_progress")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let target = account_id.clone();
            let updated_ver = store
                .account_write(&account_id, &actor, move |tx, repos| {
                    let prev = repos.bounty_states.get(tx.conn(), &target)?;
                    let now = now_iso();
                    let (mut bounties, draft_data, expected_ver, created_at) = match prev {
                        Some(r) => (r.bounties, r.draft_data, Some(r.update_version), r.created_at),
                        None => (Vec::new(), DraftData::default(), None, now.clone()),
                    };
                    bounties.push(StoredBounty {
                        group: BountyGroup::Base,
                        bounty: bounty_progress,
                        claimed_at: None,
                        reward_credit: None,
                    });
                    let next_ver = expected_ver.unwrap_or(0) + 1;
                    let record = BountyStateRecord {
                        account: target,
                        bounties,
                        draft_data,
                        daily_reward_credited_on: None,
                        daily_premium_draft_granted_on: None,
                        update_version: next_ver,
                        created_at,
                        updated_at: now,
                    };
                    repos.bounty_states.save_if_version(tx, &record, expected_ver)?;
                    Ok(next_ver)
                })
                .await
                .map_err(failed)?;
            Ok(json!({
                "ok": true,
                "account": invocation.account.to_string(),
                "update_version": updated_ver,
            }))
        }
        "bounty.read" => {
            let target = account_id.clone();
            let state = store
                .read(move |conn, repos| repos.bounty_states.get(conn, &target))
                .await
                .map_err(failed)?;
            let count = state.as_ref().map_or(0, |s| s.bounties.len());
            let entries: Vec<Value> = state
                .map(|s| s.bounties.into_iter().map(|b| b.bounty).collect())
                .unwrap_or_default();
            Ok(json!({
                "count": count,
                "bounties": entries,
            }))
        }
        "character.load" | "account.write" => Ok(json!({
            "ok": true,
            "account": invocation.account.to_string(),
            "operation": invocation.operation.as_str(),
        })),
        "chat.login" => {
            let authcid = invocation
                .body
                .get("authcid")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let token = invocation.body.get("token").and_then(Value::as_str).unwrap_or_default();
            let chat_source = crate::areas::chat::rules::source(&shared.node, Some(store.clone()), &shared.settings);
            let verifier = crate::areas::auth::tokens::GameTokens(shared.settings.clone());
            let player = isles_chat::login::authenticate(authcid, token, &verifier, &chat_source)
                .await
                .map_err(|refusal| ModuleError::new(ErrorKind::Invalid, refusal.as_str()))?;
            if player.account != invocation.account {
                return Err(ModuleError::new(
                    ErrorKind::Invalid,
                    "token account does not match target account",
                ));
            }
            Ok(json!({
                "account": player.account.to_string(),
                "user_id": player.user_id,
                "name": player.name,
            }))
        }
        "chat.player" => {
            let target = account_id.clone();
            let rec = store
                .read(move |conn, repos| repos.accounts.find(conn, &target))
                .await
                .map_err(failed)?;
            match rec {
                Some(rec) => Ok(json!({
                    "account": rec.id.global().to_string(),
                    "user_id": rec.user_id.0,
                    "name": rec.name,
                })),
                None => Ok(json!({ "player": Value::Null })),
            }
        }
        "chat.party" => {
            let target = account_id.clone();
            let party = store
                .read(move |conn, repos| repos.parties.find_by_member(conn, &target))
                .await
                .map_err(failed)?;
            Ok(json!({
                "party_id": party.map(|p| p.party_id),
            }))
        }
        other => Err(ModuleError::new(
            ErrorKind::NotFound,
            format!("unknown home operation {other}"),
        )),
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
    use crate::repo::auth::{AccessChange, AccountTimeout, OperationalStatus};

    let actor = call.actor;
    // A ban signs the player out of the launcher too; a kick or a timeout only ends their play
    let revoke_and_close = |store: IslesStore,
                            actor: module_api::ActorContext,
                            target: AccountId,
                            user_id: String,
                            sign_out: bool,
                            reason: &'static str| async move {
        store
            .account_write(&target, &actor, {
                let target = target.clone();
                move |tx, repos| {
                    let now = util::now_iso();
                    if sign_out {
                        repos.refresh_sessions.revoke_all_for_account(tx, &target, &now)?;
                    }
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

    let set_timeout = |store: IslesStore,
                       actor: module_api::ActorContext,
                       target: AccountId,
                       timeout: Option<AccountTimeout>| async move {
        store
            .account_write(&target, &actor, {
                let target = target.clone();
                move |tx, repos| repos.launcher_accounts.set_timeout(tx, &target, timeout.as_ref())
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
            revoke_and_close(store.clone(), actor, target, user_id, false, "kicked by the host").await?;
            Ok(AdminResponse::Done)
        }
        AdminRequest::Ban {
            player,
            reason,
            until: Some(until),
        } => {
            let now = util::now_iso();
            let until = timeout_end(&until, &now)?;
            let Some((target, user_id, _)) = resolve_player(store, &player).await? else {
                return Ok(AdminResponse::NotFound);
            };
            let timeout = AccountTimeout {
                until,
                since: now,
                reason: reason.map(|r| r.chars().take(500).collect()),
            };
            set_timeout(store.clone(), actor.clone(), target.clone(), Some(timeout)).await?;
            revoke_and_close(store.clone(), actor, target, user_id, false, "timed out by the host").await?;
            Ok(AdminResponse::Done)
        }
        AdminRequest::Ban {
            player,
            reason,
            until: None,
        } => {
            let Some((target, user_id, _)) = resolve_player(store, &player).await? else {
                return Ok(AdminResponse::NotFound);
            };
            set_status(
                store.clone(),
                actor.clone(),
                target.clone(),
                OperationalStatus::Banned,
                reason,
            )
            .await?;
            // A ban outlasts any timeout, which would otherwise show beside it
            set_timeout(store.clone(), actor.clone(), target.clone(), None).await?;
            revoke_and_close(store.clone(), actor, target, user_id, true, "banned by the host").await?;
            Ok(AdminResponse::Done)
        }
        AdminRequest::Unban { player, reason } => {
            let Some((target, _, _)) = resolve_player(store, &player).await? else {
                return Ok(AdminResponse::NotFound);
            };
            set_status(store.clone(), actor, target, OperationalStatus::Active, reason).await?;
            Ok(AdminResponse::Done)
        }
        AdminRequest::LiftTimeout { player, .. } => {
            let Some((target, _, _)) = resolve_player(store, &player).await? else {
                return Ok(AdminResponse::NotFound);
            };
            set_timeout(store.clone(), actor, target, None).await?;
            Ok(AdminResponse::Done)
        }
        AdminRequest::Lookup { player } => {
            let Some((target, user_id, name)) = resolve_player(store, &player).await? else {
                return Ok(AdminResponse::NotFound);
            };
            let online = presence::active_players(crate::areas::reforged_api::activity::ACTIVITY_SESSION_GAP_MS)
                .iter()
                .any(|u| u == &user_id);
            let launcher = store
                .read({
                    let target = target.clone();
                    move |conn, repos| repos.launcher_accounts.find_by_account(conn, &target)
                })
                .await
                .map_err(failed)?;
            Ok(AdminResponse::Player(PlayerInfo {
                account: Some(target.global().clone()),
                name,
                online,
                restriction: launcher.and_then(|account| restriction_of(&account, &util::now_iso())),
            }))
        }
    }
}

/// A timeout's end as the launcher accounts keep it (`iso_from_ms`'s text), refused unless it is RFC 3339 and later
/// than `now`.
fn timeout_end(until: &str, now: &str) -> Result<String, ModuleError> {
    let end = chrono::DateTime::parse_from_rfc3339(until)
        .map(|t| crate::areas::auth::util::iso_from_ms(t.timestamp_millis()))
        .map_err(|_| ModuleError::new(ErrorKind::Invalid, "a timeout ends at an RFC 3339 time"))?;
    if end.as_str() <= now {
        return Err(ModuleError::new(ErrorKind::Invalid, "a timeout ends in the future"));
    }
    Ok(end)
}

/// What keeps the account out of play at `now`, for the admin API: a ban or disabling first, then a timeout.
fn restriction_of(account: &crate::repo::auth::LauncherAccountRecord, now: &str) -> Option<PlayerRestriction> {
    use crate::repo::auth::OperationalStatus;
    let kind = match account.status {
        OperationalStatus::Banned => Some(RestrictionKind::Ban),
        OperationalStatus::Disabled => Some(RestrictionKind::Disabled),
        OperationalStatus::Active => None,
    };
    if let Some(kind) = kind {
        return Some(PlayerRestriction {
            kind,
            reason: account.approval_reason.clone().filter(|r| !r.is_empty()),
            since: account.approval_updated_at.clone(),
            until: None,
        });
    }
    account.timeout_at(now).map(|timeout| PlayerRestriction {
        kind: RestrictionKind::Timeout,
        reason: timeout.reason.clone(),
        since: Some(timeout.since.clone()),
        until: Some(timeout.until.clone()),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::BTreeMap;

    use module_api::{ModuleConfig, PortRange};
    use serde_json::json;

    use super::IslesModule;

    #[test]
    fn settings_merge_sibling_admin_and_director_env_when_given_backend_env() {
        let dir = tempfile::tempdir().unwrap();
        let backend_env = dir.path().join("backend.env");
        let director_env = dir.path().join("director.env");
        let admin_env = dir.path().join("admin.env");
        std::fs::write(
            &admin_env,
            "REFORGED_DB_ENGINE=sqlite\nREFORGED_DB_FILE=/srv/dainty-lass.db\nPORT=61003\n",
        )
        .unwrap();
        std::fs::write(
            &director_env,
            "PORT=61001\nMY_IP=100.101.102.103\nPORT_RANGE_BEGIN=8766\nPORT_RANGE_END=8777\nGAMESERVER_BINARY_PATH=/srv/game\nMETAGAME_API_KEY=gs-secret\n",
        )
        .unwrap();
        std::fs::write(
            &backend_env,
            "PORT=61000\nDEPLOYSERVER_URL=127.0.0.1:61001\nREGISTRATION_MODE=INVITECODE\n",
        )
        .unwrap();

        let mut sections = BTreeMap::new();
        let mut backend_section = BTreeMap::new();
        backend_section.insert("env_file".into(), json!(backend_env.display().to_string()));
        sections.insert("backend".into(), backend_section);
        let mut ports = BTreeMap::new();
        ports.insert(
            "game".into(),
            PortRange {
                first: 8770,
                last: 8777,
            },
        );
        let config = ModuleConfig { sections, ports };

        let settings = IslesModule::settings(&config).unwrap();
        assert_eq!(settings.get("PORT"), Some("61000"));
        assert_eq!(settings.get("REFORGED_DB_ENGINE"), Some("sqlite"));
        assert_eq!(settings.get("REFORGED_DB_FILE"), Some("/srv/dainty-lass.db"));
        assert_eq!(settings.get("MY_IP"), Some("100.101.102.103"));
        assert_eq!(settings.get("GAMESERVER_BINARY_PATH"), Some("/srv/game"));
        assert_eq!(settings.get("METAGAME_API_KEY"), Some("gs-secret"));
        // Default 8770..=8777 window keeps the wider 8766..=8777 range from director.env.
        assert_eq!(settings.get("PORT_RANGE_BEGIN"), Some("8766"));
        assert_eq!(settings.get("PORT_RANGE_END"), Some("8777"));

        // When the controller moves the game port range to avoid a clash, the moved range wins.
        let mut moved_ports = BTreeMap::new();
        moved_ports.insert(
            "game".into(),
            PortRange {
                first: 8778,
                last: 8785,
            },
        );
        let moved = IslesModule::settings(&ModuleConfig {
            sections: config.sections,
            ports: moved_ports,
        })
        .unwrap();
        assert_eq!(moved.get("PORT_RANGE_BEGIN"), Some("8778"));
        assert_eq!(moved.get("PORT_RANGE_END"), Some("8785"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_timeout_keeps_a_player_out_of_play_until_it_ends_or_is_lifted() {
        use module_api::actor::{Actor, ActorContext};
        use module_api::module::RestrictionKind;
        use module_api::{AdminCall, AdminRequest, AdminResponse};

        use crate::areas::auth::policy::{assert_account_admitted, is_account_eligible};
        use crate::areas::auth::test_support::TestServer;

        let server = TestServer::new(&[]);
        let account = server
            .add_launcher_account("Slayer", "slayer@example.org", "hunter2-long")
            .await;
        let store = server.store();
        let call = |request| AdminCall {
            actor: ActorContext::new(Actor::Console),
            request,
        };
        let launcher = || async {
            let id = account.account.clone();
            store
                .read(move |conn, repos| repos.launcher_accounts.find_by_account(conn, &id))
                .await
                .unwrap()
                .unwrap()
        };
        let lookup = || async {
            match super::handle_admin(
                store,
                call(AdminRequest::Lookup {
                    player: "Slayer".into(),
                }),
            )
            .await
            .unwrap()
            {
                AdminResponse::Player(info) => info.restriction,
                other => panic!("{other:?}"),
            }
        };

        let past = super::handle_admin(
            store,
            call(AdminRequest::Ban {
                player: "Slayer".into(),
                reason: Some("spam".into()),
                until: Some("2020-01-01T00:00:00Z".into()),
            }),
        )
        .await;
        assert!(past.is_err(), "a timeout ends in the future");
        assert_eq!(lookup().await, None);

        super::handle_admin(
            store,
            call(AdminRequest::Ban {
                player: "Slayer".into(),
                reason: Some("spam".into()),
                until: Some("2099-01-15T08:30:00+00:00".into()),
            }),
        )
        .await
        .unwrap();
        let restriction = lookup().await.unwrap();
        assert_eq!(restriction.kind, RestrictionKind::Timeout);
        assert_eq!(restriction.until.as_deref(), Some("2099-01-15T08:30:00.000Z"));
        assert_eq!(restriction.reason.as_deref(), Some("spam"));
        let record = launcher().await;
        assert!(
            assert_account_admitted(&record).is_ok(),
            "the launcher still signs them in"
        );
        assert!(!is_account_eligible(&record), "but they can't play");

        super::handle_admin(
            store,
            call(AdminRequest::LiftTimeout {
                player: "Slayer".into(),
                reason: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!(lookup().await, None);
        assert!(is_account_eligible(&launcher().await));

        // A ban replaces a timeout, and lifting a timeout never lifts a ban
        for request in [
            AdminRequest::Ban {
                player: "Slayer".into(),
                reason: Some("spam".into()),
                until: Some("2099-01-15T08:30:00Z".into()),
            },
            AdminRequest::Ban {
                player: "Slayer".into(),
                reason: Some("cheating".into()),
                until: None,
            },
            AdminRequest::LiftTimeout {
                player: "Slayer".into(),
                reason: None,
            },
        ] {
            super::handle_admin(store, call(request)).await.unwrap();
        }
        let restriction = lookup().await.unwrap();
        assert_eq!(restriction.kind, RestrictionKind::Ban);
        assert_eq!(restriction.reason.as_deref(), Some("cheating"));
        assert_eq!(launcher().await.timeout, None);
    }
}
