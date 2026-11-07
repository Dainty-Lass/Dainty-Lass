// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The module in dainty-lass's `module-api` (core spec §7): manifest, registration, start and stop, and the chat,
//! admin, event and command answers.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use dainty_module_api::chat::{ChatAnswer, ChatQuery, HEADLINE_EVENT, PRESENCE_EVENT, PresenceEvent};
use dainty_module_api::manifest::{PortDefault, Transport};
use dainty_module_api::network::{DirectorInvocation, HomeInvocation};
use dainty_module_api::registrar::ConfigSection;
use dainty_module_api::{
    API_VERSION, Actor, ActorContext, AdminAction, AdminCall, AdminRequest, AdminResponse, BoxFuture, Capabilities,
    CommandOutput, CommandRequest, ErrorKind, Event, EventName, GameModule, GameVersion, LogLevel, ModuleConfig,
    ModuleContext, ModuleError, ModuleId, ModuleKind, ModuleManifest, ModuleStatus, Phase, PortRange, Registrar,
};
use serde_json::{Map, Value};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::composer::{ComposeInputs, Composer, ModuleHooks, StartedListener, no_database};
use crate::dedicated::{Launch, LaunchRequest};
use crate::game_data::GameDataRoot;
use crate::handlers::auth_handlers;
use crate::ids::AccountId;
use crate::routes::Service;
use crate::settings::{OPTIONAL_SWITCHES, SWITCHES, Settings, config_text};
use crate::shared::{Shared, SharedParts};
use crate::version::IslesVersion;

/// Emitted once the services listen: the recorded addresses as JSON text, in order.
pub const LISTENING_EVENT: &str = "isles.listening";

/// Operator routes the gateway keeps off the internet.
pub const HOST_ONLY_PREFIXES: [&str; 5] = [
    "/reforged/api/CreateInvite",
    "/reforged/api/InviteCodes",
    "/reforged/api/InviteCode",
    "/reforged/api/SetAccountStatus",
    "/reforged/api/Kick",
];

const DRAIN: Duration = Duration::from_secs(2);
const LISTENER_STOP: Duration = Duration::from_secs(5);
const RAMSGATE_HUNT: &str = "ShatteredIsles_ReturnToRamsgate";

/// A service on one of the module's TCP ports.
pub trait PortService: Send + Sync {
    /// The port's name in the manifest
    fn port(&self) -> &'static str;
    /// The service's router over the module's shared state.
    fn app(&self, shared: Arc<Shared>) -> Result<axum::Router, ModuleError>;
    /// Called once the service listens.
    fn started(&self) {}
    /// The Director lets go of its game servers here.
    fn stopping(&self) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

/// The backend over the composer's handlers and the auth routes.
#[derive(Debug, Clone, Copy, Default)]
pub struct BackendService;

impl PortService for BackendService {
    fn port(&self) -> &'static str {
        "backend"
    }

    fn app(&self, shared: Arc<Shared>) -> Result<axum::Router, ModuleError> {
        let handlers = shared.handlers().clone();
        crate::pipeline::app(shared, Service::Backend, &handlers).map_err(|e| ModuleError::failed(e.to_string()))
    }
}

/// The ports a module takes unless the config assigns others.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DefaultPorts {
    /// The backend's port.
    pub backend: u16,
    /// The Director's port.
    pub director: u16,
    /// The region-ping listener's port.
    pub qos: u16,
    /// The game servers' ports.
    pub game: PortRange,
}

/// Everything a game version's module is built from.
pub struct ModuleSpec {
    /// The module's id, kept in `host.toml` and invites.
    pub id: ModuleId,
    /// The name hosts and the launcher show.
    pub display_name: String,
    /// Other names an invite may give the module.
    pub invite_aliases: Vec<String>,
    /// What the module offers (realms, network, chat, patch sets).
    pub capabilities: Capabilities,
    /// Built but not checked against recorded traffic (DD9); the UI says so.
    pub unverified: bool,
    /// The ports it takes by default.
    pub ports: DefaultPorts,
    /// The version's tables, hooks and route set.
    pub version: IslesVersion,
    /// In start order; the backend is usually first
    pub services: Vec<Arc<dyn PortService>>,
    /// Builds the areas at start.
    pub composer: Arc<dyn Composer>,
}

struct Running {
    shared: Arc<Shared>,
    hooks: Arc<dyn ModuleHooks>,
    tasks: Vec<JoinHandle<()>>,
    listeners: Vec<StartedListener>,
    servers: Vec<JoinHandle<std::io::Result<()>>>,
    stop: watch::Sender<bool>,
}

struct State {
    phase: Phase,
    detail: Option<String>,
    running: Option<Running>,
}

/// A game version's module as the controller runs it (`dainty_module_api::GameModule`).
pub struct IslesModule {
    spec: ModuleSpec,
    game_version: GameVersion,
    state: Mutex<State>,
}

fn invalid(message: impl Into<String>) -> ModuleError {
    ModuleError::new(ErrorKind::Invalid, message)
}

fn not_running() -> ModuleError {
    ModuleError::new(ErrorKind::Unavailable, "the module is not running")
}

/// The default windows the env file's own (possibly wider) range may stand in for.
fn is_default_window(range: PortRange) -> bool {
    [(8770, 8777), (8780, 8787)].contains(&(range.first, range.last))
}

fn sibling_files(path: &Path) -> Vec<PathBuf> {
    let named_backend = path.file_name().is_some_and(|name| name == "backend.env");
    match path.parent().filter(|p| !p.as_os_str().is_empty()) {
        Some(parent) if named_backend => ["admin.env", "director.env"]
            .iter()
            .map(|name| parent.join(name))
            .filter(|p| p.is_file())
            .collect(),
        _ => Vec::new(),
    }
}

/// `RUSTIC_ISLES_PORT_SERVICES`: the port services this server runs, all of them when unset or blank.
pub fn port_services(settings: &Settings) -> Option<Vec<String>> {
    let names: Vec<String> = settings
        .raw("RUSTIC_ISLES_PORT_SERVICES")?
        .split(',')
        .map(|name| name.trim().to_ascii_lowercase())
        .filter(|name| !name.is_empty())
        .collect();
    (!names.is_empty()).then_some(names)
}

impl IslesModule {
    /// Fails when the version's game version does not parse.
    pub fn new(spec: ModuleSpec) -> Result<Self, ModuleError> {
        let text = spec.version.tables.game_version;
        let game_version = text.parse().map_err(|e| invalid(format!("game version {text}: {e}")))?;
        Ok(Self {
            spec,
            game_version,
            state: Mutex::new(State {
                phase: Phase::Loaded,
                detail: None,
                running: None,
            }),
        })
    }

    /// The spec it was built from.
    pub fn spec(&self) -> &ModuleSpec {
        &self.spec
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The env file (with `backend.env`'s siblings), the switches, and the `game` port range forced (§6.4).
    pub fn settings(&self, config: &ModuleConfig) -> Result<Settings, ModuleError> {
        let env_file = config
            .get("backend", "env_file")
            .and_then(config_text)
            .map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty());
        let mut env = BTreeMap::new();
        if let Some(file) = env_file {
            let path = PathBuf::from(file);
            for source in sibling_files(&path).iter().chain(std::iter::once(&path)) {
                let read = Settings::read_env_file(source).map_err(|e| ModuleError::failed(e.to_string()))?;
                env.extend(read);
            }
        }
        let keep_env_range = |range: PortRange| {
            env.contains_key("PORT_RANGE_BEGIN") && env.contains_key("PORT_RANGE_END") && is_default_window(range)
        };
        let forced = config
            .ports
            .get("game")
            .copied()
            .filter(|range| !keep_env_range(*range));
        let mut settings = Settings::new(env, Settings::switches_from_config(config.sections.get("switches")));
        if let Some(range) = forced {
            settings = settings
                .forced("PORT_RANGE_BEGIN", &range.first.to_string())
                .forced("PORT_RANGE_END", &range.last.to_string());
        }
        Ok(settings)
    }

    fn port_for(&self, name: &str, config: &ModuleConfig) -> Option<u16> {
        if let Some(range) = config.ports.get(name) {
            return Some(range.first);
        }
        match name {
            "backend" => Some(self.spec.ports.backend),
            "director" => Some(self.spec.ports.director),
            _ => None,
        }
    }

    async fn start_inner(
        &self,
        ctx: &ModuleContext,
        config: &ModuleConfig,
    ) -> Result<(Running, Vec<(String, String)>), ModuleError> {
        let settings = self.settings(config)?;
        let admin_api = settings.admin_api().map_err(|e| ModuleError::failed(e.to_string()))?;
        let game_data = GameDataRoot::from_config(config);
        let database_wanted = settings.database_wanted();
        let composition = self
            .spec
            .composer
            .compose(ComposeInputs {
                settings: settings.clone(),
                node: ctx.node().clone(),
                module_id: ctx.module_id().clone(),
                context: ctx.clone(),
                game_data: game_data.clone(),
                database_wanted,
                ports: config.ports.clone(),
            })
            .await?;
        let mut handlers = auth_handlers().map_err(|e| ModuleError::failed(e.to_string()))?;
        handlers.extend(composition.handlers);
        let has_store = composition.auth_store.is_some();
        let shared = Shared::new(SharedParts {
            version: self.spec.version.clone(),
            settings,
            node: ctx.node().clone(),
            game_data,
            database_wanted,
            rifts: composition.rifts,
            admin_api,
            auth_store: composition.auth_store,
            auth_seams: composition.auth_seams,
            engine_is_postgres: composition.engine_is_postgres,
            context: Some(ctx.clone()),
            slots: composition.slots,
            observer: composition.observer,
            admin_authenticator: composition.admin_authenticator,
            handlers,
        })?;
        let (stop, _) = watch::channel(false);
        let mut running = Running {
            shared: Arc::clone(&shared),
            hooks: composition.hooks,
            tasks: Vec::new(),
            listeners: Vec::new(),
            servers: Vec::new(),
            stop,
        };
        let failed = |running: Running, error: ModuleError| {
            Self::abort(running);
            error
        };
        for startup in &composition.startups {
            match startup.start(&shared).await {
                Ok(started) => {
                    for line in started.logs {
                        let _ = ctx.log(LogLevel::Info, line).await;
                    }
                    running.tasks.extend(started.tasks);
                    running.listeners.extend(started.listeners);
                }
                Err(text) => return Err(failed(running, ModuleError::failed(text))),
            }
        }
        if database_wanted || has_store {
            match shared.auth().start().await {
                Ok(tasks) => running.tasks.extend(tasks),
                Err(e) => return Err(failed(running, ModuleError::failed(e.to_string()))),
            }
        }
        let mut addresses = Vec::new();
        let wanted = port_services(shared.settings());
        for service in &self.spec.services {
            let name = service.port();
            // The other half of a split server runs it, reached through the node-link tunnel (docs/split-roles.md)
            if wanted.as_ref().is_some_and(|w| !w.iter().any(|n| n == name)) {
                continue;
            }
            let Some(port) = self.port_for(name, config) else {
                return Err(failed(running, ModuleError::failed(format!("no {name} port"))));
            };
            let app = match service.app(Arc::clone(&shared)) {
                Ok(app) => app,
                Err(e) => return Err(failed(running, e)),
            };
            let listener = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
                Ok(listener) => listener,
                Err(e) => {
                    let error = ModuleError::failed(format!("{name} on 127.0.0.1:{port}: {e}"));
                    return Err(failed(running, error));
                }
            };
            let local = match listener.local_addr() {
                Ok(local) => local,
                Err(e) => return Err(failed(running, ModuleError::failed(format!("{name}: {e}")))),
            };
            addresses.push((name.to_owned(), local.to_string()));
            let mut stop = running.stop.subscribe();
            let signal = async move {
                let _ = stop.wait_for(|stopped| *stopped).await;
            };
            running.servers.push(tokio::spawn(dainty_service_kit::serve::serve(
                listener, app, signal, DRAIN,
            )));
        }
        for listener in &running.listeners {
            addresses.extend(listener.addresses.iter().cloned());
        }
        Ok((running, addresses))
    }

    /// Lets everything started so far go after a failed start.
    fn abort(running: Running) {
        let _ = running.stop.send(true);
        for task in running.tasks {
            task.abort();
        }
        for listener in running.listeners {
            (listener.stop)();
            listener.task.abort();
        }
    }

    fn running_shared(&self) -> Result<(Arc<Shared>, Arc<dyn ModuleHooks>), ModuleError> {
        let state = self.state();
        let running = state.running.as_ref().ok_or_else(not_running)?;
        Ok((Arc::clone(&running.shared), Arc::clone(&running.hooks)))
    }

    async fn admin_call(&self, mut call: AdminCall) -> Result<AdminResponse, ModuleError> {
        // Checked before anything else, so a past `until` is refused even for an unknown player (§15 item 15)
        if let AdminRequest::Ban { until: Some(until), .. } = &mut call.request {
            let instant = chrono::DateTime::parse_from_rfc3339(until.trim())
                .map_err(|_| invalid("a timeout ends at an RFC 3339 time"))?;
            let iso = crate::season::to_iso_string(instant.timestamp_millis());
            if iso <= crate::season::to_iso_string(crate::replay::now_ms()) {
                return Err(invalid("a timeout ends in the future"));
            }
            *until = iso;
        }
        let (shared, hooks) = self.running_shared().map_err(|_| no_database())?;
        if !shared.database_wanted() {
            return Err(no_database());
        }
        hooks.admin(&shared, call).await
    }

    async fn director(&self, invocation: DirectorInvocation) -> Result<CommandOutput, ModuleError> {
        let (shared, hooks) = self.running_shared()?;
        let servers = shared.dedicated_servers();
        if !servers.configured() {
            return Err(ModuleError::new(
                ErrorKind::Unavailable,
                "dedicated servers are not configured on this node",
            ));
        }
        let placed = |host: &str, port: i64| {
            let port = u16::try_from(port).unwrap_or(7777);
            let mut data = Map::new();
            data.insert("server_host".into(), Value::from(host));
            data.insert("server_port".into(), Value::from(port));
            (port, Value::Object(data))
        };
        let (instance, request, refused) = match invocation {
            DirectorInvocation::PlaceHunt {
                match_id,
                queue,
                players,
                ..
            } => {
                let users = hooks.place_hunt_players(&shared, &players).await;
                let request = LaunchRequest {
                    game_mode: Some(crate::version::GameMode::Island.as_str().to_owned()),
                    game_args: Some(String::new()),
                    hunt_id: queue,
                    expected_players: users,
                    request_id: None,
                };
                (match_id, request, "director refused hunt launch")
            }
            DirectorInvocation::PlaceHub { hub_id, .. } => {
                let live = servers.hub_status().await.and_then(|status| status.ramsgate);
                if let Some(hub) = live.filter(|hub| hub.ready) {
                    let (port, data) = placed(&hub.host, hub.port);
                    hooks.record_ring_instance(&shared, &hub_id, &hub.host, port, &[]).await;
                    return Ok(CommandOutput {
                        text: "ok".into(),
                        data: Some(data),
                    });
                }
                let request = LaunchRequest {
                    game_mode: Some(crate::version::GameMode::City.as_str().to_owned()),
                    game_args: None,
                    hunt_id: RAMSGATE_HUNT.to_owned(),
                    expected_players: Vec::new(),
                    request_id: None,
                };
                (hub_id, request, "director refused hub launch")
            }
        };
        let players = request.expected_players.clone();
        match servers.launch(request).await.map_err(ModuleError::failed)? {
            Launch::Online { host, port } => {
                let (port, data) = placed(&host, port);
                hooks
                    .record_ring_instance(&shared, &instance, &host, port, &players)
                    .await;
                Ok(CommandOutput {
                    text: "ok".into(),
                    data: Some(data),
                })
            }
            Launch::Refused { reason } => Err(ModuleError::failed(reason.unwrap_or_else(|| refused.to_owned()))),
        }
    }

    async fn home(&self, invocation: HomeInvocation) -> Result<CommandOutput, ModuleError> {
        let (shared, hooks) = self.running_shared()?;
        if !shared.database_wanted() {
            return Err(no_database());
        }
        let account = AccountId::try_from(invocation.account).map_err(|e| invalid(e.to_string()))?;
        let node = &shared.node().node;
        if account.home() != node {
            return Err(invalid(format!(
                "account {account} is homed on {}, not {node}",
                account.home()
            )));
        }
        let actor = ActorContext::new(Actor::Player {
            account: account.global().clone(),
        });
        let name = invocation.operation.as_str();
        match hooks
            .home_operation(&shared, name, &account, actor, invocation.body)
            .await?
        {
            Some(data) => Ok(CommandOutput {
                text: "ok".into(),
                data: Some(data),
            }),
            None => Err(ModuleError::new(
                ErrorKind::NotFound,
                format!("unknown home operation {name}"),
            )),
        }
    }
}

fn event(name: &str, payload: Value) -> Option<Event> {
    Some(Event {
        name: EventName::new(name).ok()?,
        payload,
    })
}

impl GameModule for IslesModule {
    fn manifest(&self) -> ModuleManifest {
        let spec = &self.spec;
        let tcp = |name: &str, port: u16| PortDefault {
            name: name.to_owned(),
            transport: Transport::Tcp,
            range: PortRange::single(port),
        };
        ModuleManifest {
            id: spec.id.clone(),
            kind: ModuleKind::Game,
            display_name: spec.display_name.clone(),
            game_version: self.game_version.clone(),
            invite_aliases: spec.invite_aliases.clone(),
            api: API_VERSION,
            ports: vec![
                tcp("backend", spec.ports.backend),
                tcp("director", spec.ports.director),
                tcp("qos", spec.ports.qos),
                PortDefault {
                    name: "game".to_owned(),
                    transport: Transport::Udp,
                    range: spec.ports.game,
                },
            ],
            capabilities: spec.capabilities.clone(),
            unverified: spec.unverified,
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
        let name = |text: &str| EventName::new(text).map_err(|e| invalid(e.to_string()));
        registrar.emits(name(LISTENING_EVENT)?);
        registrar.emits(name(HEADLINE_EVENT)?);
        registrar.subscribes(name(PRESENCE_EVENT)?);
        registrar.config_section(ConfigSection::new("backend").key(
            "env_file",
            Value::from(""),
            "The env file the backend and Director read, under Node's variable names",
        ));
        let tables = self.spec.version.tables;
        let mut switches = ConfigSection::new("switches");
        for switch in SWITCHES {
            switches = switches.key(switch.key, Value::from(tables.switch_default(switch.key)), switch.help);
        }
        for switch in OPTIONAL_SWITCHES {
            switches = switches.key(
                switch.key,
                Value::Null,
                format!("Optional host switch ({})", switch.env),
            );
        }
        registrar.config_section(switches);
        registrar.config_section(
            ConfigSection::new("game_data")
                .key(
                    "backend_dir",
                    Value::from(""),
                    "The backend's game data exports for this version",
                )
                .key(
                    "director_dir",
                    Value::from(""),
                    "The Director's hunt tables for this version",
                ),
        );
        Ok(())
    }

    fn start(&self, ctx: ModuleContext, config: ModuleConfig) -> BoxFuture<'_, Result<(), ModuleError>> {
        Box::pin(async move {
            {
                let mut state = self.state();
                if matches!(state.phase, Phase::Ready | Phase::Starting) {
                    return Err(invalid("already started"));
                }
                state.phase = Phase::Starting;
                state.detail = None;
            }
            match self.start_inner(&ctx, &config).await {
                Ok((running, addresses)) => {
                    let detail = addresses
                        .iter()
                        .map(|(name, address)| format!("{name} {address}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    {
                        let mut state = self.state();
                        state.phase = Phase::Ready;
                        state.detail = Some(detail.clone());
                        state.running = Some(running);
                    }
                    for service in &self.spec.services {
                        service.started();
                    }
                    let _ = ctx
                        .log(
                            LogLevel::Info,
                            format!("{} listening: {detail}", self.spec.display_name),
                        )
                        .await;
                    let mut listening = Map::new();
                    for (name, address) in addresses {
                        listening.insert(name, Value::String(address));
                    }
                    let text = dainty_service_kit::jsjson::stringify(&Value::Object(listening));
                    if let Some(event) = event(LISTENING_EVENT, Value::String(text)) {
                        let _ = ctx.emit(event).await;
                    }
                    Ok(())
                }
                Err(error) => {
                    let mut state = self.state();
                    state.phase = Phase::Failed;
                    state.detail = Some(error.message.clone());
                    Err(error)
                }
            }
        })
    }

    fn stop(&self, ctx: ModuleContext) -> BoxFuture<'_, Result<(), ModuleError>> {
        Box::pin(async move {
            for service in &self.spec.services {
                service.stopping().await;
            }
            let running = {
                let mut state = self.state();
                state.phase = Phase::Stopping;
                state.running.take()
            };
            if let Some(running) = running {
                let Running {
                    shared,
                    hooks,
                    tasks,
                    listeners,
                    servers,
                    stop,
                } = running;
                drop(shared);
                for task in tasks {
                    task.abort();
                }
                for listener in listeners {
                    (listener.stop)();
                    if tokio::time::timeout(LISTENER_STOP, listener.task).await.is_err() {
                        tracing::warn!("a listener did not stop within 5 s");
                    }
                }
                let _ = stop.send(true);
                for server in servers {
                    let _ = tokio::time::timeout(LISTENER_STOP, server).await;
                }
                if let Err(error) = hooks.stop().await {
                    tracing::warn!("checkpoint: {error}");
                }
            }
            {
                let mut state = self.state();
                state.phase = Phase::Stopped;
                state.detail = None;
            }
            let _ = ctx
                .log(LogLevel::Info, format!("{} stopped", self.spec.display_name))
                .await;
            Ok(())
        })
    }

    fn status(&self) -> ModuleStatus {
        let state = self.state();
        ModuleStatus {
            phase: state.phase,
            detail: state.detail.clone(),
        }
    }

    fn command(
        &self,
        _ctx: ModuleContext,
        request: CommandRequest,
    ) -> BoxFuture<'_, Result<CommandOutput, ModuleError>> {
        Box::pin(async move {
            self.running_shared()?;
            if let Some(invocation) = DirectorInvocation::from_command(&request) {
                return self.director(invocation?).await;
            }
            if let Some(invocation) = HomeInvocation::from_command(&request) {
                return self.home(invocation?).await;
            }
            Err(ModuleError::new(
                ErrorKind::Unsupported,
                format!("command {}", request.name),
            ))
        })
    }

    fn admin(&self, _ctx: ModuleContext, call: AdminCall) -> BoxFuture<'_, Result<AdminResponse, ModuleError>> {
        Box::pin(self.admin_call(call))
    }

    fn event(&self, _ctx: ModuleContext, event: Event) -> BoxFuture<'_, Result<(), ModuleError>> {
        Box::pin(async move {
            if event.name.as_str() != PRESENCE_EVENT {
                return Ok(());
            }
            let Ok(presence) = serde_json::from_value::<PresenceEvent>(event.payload) else {
                return Ok(());
            };
            let Ok((shared, hooks)) = self.running_shared() else {
                return Ok(());
            };
            if presence.online {
                shared.presence().set_online(&presence.user_id);
            } else {
                shared.presence().set_offline(&presence.user_id);
                hooks.player_offline(&shared, &presence.user_id).await;
            }
            Ok(())
        })
    }

    fn chat(&self, ctx: ModuleContext, query: ChatQuery) -> BoxFuture<'_, Result<ChatAnswer, ModuleError>> {
        Box::pin(async move {
            let (shared, hooks) = self.running_shared()?;
            hooks.chat(&shared, ctx, query).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_services_names_the_ones_this_half_runs() {
        assert_eq!(port_services(&Settings::default()), None);
        assert_eq!(
            port_services(&Settings::from_pairs([("RUSTIC_ISLES_PORT_SERVICES", " , ")])),
            None
        );
        assert_eq!(
            port_services(&Settings::from_pairs([("RUSTIC_ISLES_PORT_SERVICES", "Director, ")])),
            Some(vec!["director".to_owned()])
        );
    }

    #[test]
    fn default_windows() {
        assert!(is_default_window(PortRange {
            first: 8770,
            last: 8777
        }));
        assert!(!is_default_window(PortRange {
            first: 8778,
            last: 8785
        }));
    }
}
