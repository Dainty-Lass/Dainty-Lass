// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! [`ModuleRegistry`]: the game modules and plugins this server runs (DF3), keyed by game version (DD4),
//! compiled in or started as their own process (DD5). Only what `host.toml` lists can be loaded
//! ([`ModuleEntry`]); a second game module for a version already served is refused whichever came first,
//! the host kit's own stacks included ([`RegistryOptions::taken_versions`]).
//!
//! A load: the driver (in process, or the child checked against its SHA-256), the manifest's game
//! version checked, the registration checked against the manifest, ports placed ([`PortBook`]), the
//! `[modules.<id>.<section>]` settings laid over the registered defaults, the core plugins attached, and
//! `start`. The registry is also the modules' [`HostLink`]: their events go on the bus to the modules that
//! subscribed, their log lines into tracing (target `module`, under the module's name), their home calls
//! to the node's [`HomeApi`]. A child that dies shows as failed and stays so until it is loaded again.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError, RwLock, Weak};
use std::time::Duration;

use module_api::manifest::{FitError, Transport, plugin_fits};
use module_api::network::dispatch_home;
use module_api::registrar::{AdminAction, CommandSpec, ConfigSection, Registration};
use module_api::{
    AdminCall, AdminResponse, BoxFuture, ChildModule, ChildOptions, CommandOutput, CommandRequest, CorePlugin,
    ErrorKind, Event, ExternalModule, GameModule, GameVersion, HomeApi, HostAnswer, HostCall, HostLink,
    InProcessModule, LogLevel, ModuleConfig, ModuleDriver, ModuleDriverExt, ModuleError, ModuleId, ModuleKind,
    ModuleManifest, NodeIdentity, PortRange,
};
use serde::Serialize;
use service_kit::context::{StatusRegistry, StatusReporter};
use tokio::sync::broadcast;

use crate::node_services::NodeServices;
use crate::ports::PortBook;

/// Builds a compiled-in module afresh for each load.
pub type ModuleFactory = Arc<dyn Fn() -> Arc<dyn GameModule> + Send + Sync>;

/// Where a listed module comes from.
#[derive(Clone)]
pub enum ModuleSource {
    CompiledIn(ModuleFactory),
    /// An executable, or a library (`.dll`, `.so`) the core loads (RD27), pinned by its SHA-256 (DD5)
    External(ExternalModule),
}

impl std::fmt::Debug for ModuleSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CompiledIn(_) => f.write_str("CompiledIn"),
            Self::External(external) => f.debug_tuple("External").field(&external.path).finish(),
        }
    }
}

/// A module `host.toml` lists: the only kind the registry loads.
#[derive(Debug, Clone)]
pub struct ModuleEntry {
    pub id: ModuleId,
    pub source: ModuleSource,
    /// Loaded when the server starts; otherwise loaded from the console or the admin API
    pub autoload: bool,
    /// `[modules.<id>.<section>]`: values laid over the defaults the module registered
    pub settings: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
}

/// How the registry runs its modules.
pub struct RegistryOptions {
    pub node: NodeIdentity,
    /// The home API modules reach through `HostLink` until the network swaps it ([`ModuleRegistry::set_home`])
    pub home: Arc<dyn HomeApi>,
    /// For every external module; `env` and `current_dir` are filled per module when `None`
    pub child: ChildOptions,
    /// The server's own ports, which no module is given
    pub reserved_ports: Vec<(Transport, PortRange)>,
    /// Game versions the host kit serves outside the registry, and what serves them
    pub taken_versions: Vec<(GameVersion, String)>,
    /// How often children are checked for a broken pipe
    pub watch_every: Duration,
}

impl RegistryOptions {
    /// A server outside a network, no ports or versions taken, children given only the OS's own
    /// variables ([`system_environment`]).
    pub fn standalone() -> Self {
        let identity = NodeIdentity::standalone();
        Self {
            home: Arc::new(module_api::network::StandaloneHome {
                identity: identity.clone(),
            }),
            node: identity,
            child: ChildOptions {
                env: Some(system_environment(std::env::vars())),
                ..ChildOptions::default()
            },
            reserved_ports: Vec::new(),
            taken_versions: Vec::new(),
            watch_every: Duration::from_millis(250),
        }
    }
}

/// The variables a child needs from the OS to run at all, and nothing a service reads: an allowlist, so
/// a server setting left in the shell never reaches a module (as `start` does for its services).
pub fn system_environment(vars: impl IntoIterator<Item = (String, String)>) -> Vec<(String, String)> {
    const KEEP: [&str; 24] = [
        "PATH",
        "PATHEXT",
        "SYSTEMROOT",
        "SYSTEMDRIVE",
        "WINDIR",
        "COMSPEC",
        "TEMP",
        "TMP",
        "TMPDIR",
        "HOME",
        "USERPROFILE",
        "APPDATA",
        "LOCALAPPDATA",
        "PROGRAMDATA",
        "USERNAME",
        "USER",
        "LOGNAME",
        "LANG",
        "LC_ALL",
        "TZ",
        "XDG_RUNTIME_DIR",
        "NUMBER_OF_PROCESSORS",
        "PROCESSOR_ARCHITECTURE",
        "RUST_BACKTRACE",
    ];
    vars.into_iter()
        .filter(|(key, _)| KEEP.contains(&key.to_ascii_uppercase().as_str()))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    #[error("no module {0} is listed in host.toml ([modules])")]
    NotListed(ModuleId),
    #[error("module {0} is listed twice")]
    ListedTwice(ModuleId),
    #[error("{0} is already a service's name")]
    NameTaken(ModuleId),
    #[error("module {0} is already loaded")]
    AlreadyLoaded(ModuleId),
    #[error("module {0} is not loaded")]
    NotLoaded(ModuleId),
    #[error("{id} was refused: game version {version} is already served by {by}")]
    Duplicate {
        id: ModuleId,
        version: GameVersion,
        by: String,
    },
    #[error("{id} extends game version {version}, and no module for it is loaded")]
    NoGameFor { id: ModuleId, version: GameVersion },
    #[error("{id} was refused: {reason}")]
    Refused { id: ModuleId, reason: String },
    #[error("{id}: {reason}")]
    Failed { id: ModuleId, reason: String },
}

/// Where a listed module is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleState {
    NotLoaded,
    Starting,
    Ready,
    /// It broke after it started (its process died, its pipe broke) or failed to start
    Failed,
}

/// One listed module, as the console, the TUI, the server GUI and the admin API show it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModuleInfo {
    pub id: ModuleId,
    /// `compiled_in` or `external`
    pub source: &'static str,
    pub autoload: bool,
    pub state: ModuleState,
    pub detail: Option<String>,
    /// Known once it was loaded, or at once for a compiled-in one
    pub manifest: Option<ModuleManifest>,
    pub ports: BTreeMap<String, PortRange>,
    pub commands: Vec<CommandSpec>,
    /// The `[modules.<id>.<section>]` keys it registered, with their defaults: what the server GUI's
    /// settings editor draws (DU3). Empty until it loads
    pub config: Vec<ConfigSection>,
}

enum Driver {
    InProcess(InProcessModule),
    Child(ChildModule),
    Library(plugin_host::DylibModule),
}

impl ModuleDriver for Driver {
    fn manifest(&self) -> &ModuleManifest {
        self.get().manifest()
    }

    fn call(
        &self,
        call: module_api::frame::ModuleCall,
    ) -> BoxFuture<'_, Result<module_api::frame::ModuleAnswer, ModuleError>> {
        self.get().call(call)
    }

    fn unload(&self) -> BoxFuture<'_, Result<(), ModuleError>> {
        self.get().unload()
    }
}

impl Driver {
    fn get(&self) -> &dyn ModuleDriver {
        match self {
            Self::InProcess(module) => module,
            Self::Child(module) => module,
            Self::Library(module) => module,
        }
    }

    /// Why a child's pipe broke, if it did.
    fn failure(&self) -> Option<String> {
        match self {
            Self::InProcess(_) => None,
            Self::Child(child) => child.failure(),
            Self::Library(library) => library.failure(),
        }
    }
}

struct Loaded {
    driver: Arc<Driver>,
    manifest: ModuleManifest,
    registration: Registration,
    ports: BTreeMap<String, PortRange>,
    plugins: Vec<Arc<dyn CorePlugin>>,
}

struct Slot {
    entry: ModuleEntry,
    status: StatusReporter,
    state: ModuleState,
    detail: Option<String>,
    /// A compiled-in module's manifest is known before it loads
    known: Option<ModuleManifest>,
    loaded: Option<Arc<Loaded>>,
}

impl Slot {
    fn info(&self) -> ModuleInfo {
        let loaded = self.loaded.as_ref();
        ModuleInfo {
            id: self.entry.id.clone(),
            source: match self.entry.source {
                ModuleSource::CompiledIn(_) => "compiled_in",
                ModuleSource::External(_) => "external",
            },
            autoload: self.entry.autoload,
            state: self.state,
            detail: self.detail.clone(),
            manifest: loaded.map(|l| l.manifest.clone()).or_else(|| self.known.clone()),
            ports: loaded.map(|l| l.ports.clone()).unwrap_or_default(),
            commands: loaded.map(|l| l.registration.commands.clone()).unwrap_or_default(),
            config: loaded.map(|l| l.registration.config.clone()).unwrap_or_default(),
        }
    }
}

#[derive(Default)]
struct State {
    slots: BTreeMap<ModuleId, Slot>,
    /// The game module serving each version (DD4)
    games: BTreeMap<GameVersion, ModuleId>,
    ports: PortBook,
    watching: bool,
}

/// Told when a module is up and when it is gone, with the driver that reaches it: for the parts of the host that
/// answer for a module without being asked to attach to its registration (the network's home backends, DN5).
pub trait LoadHook: Send + Sync {
    /// The module started. Not called for a plugin that failed to start.
    fn loaded(&self, manifest: &ModuleManifest, driver: Arc<dyn ModuleDriver>);

    /// The module is gone: unloaded, or it never finished starting.
    fn unloaded(&self, module: &ModuleId);
}

struct Inner {
    options: RegistryOptions,
    status: StatusRegistry,
    /// What `HostCall::Home` reaches: `options.home` until [`ModuleRegistry::set_home`] swaps it
    home: RwLock<Arc<dyn HomeApi>>,
    node_services: NodeServices,
    plugins: Mutex<Vec<Arc<dyn CorePlugin>>>,
    hooks: Mutex<Vec<Arc<dyn LoadHook>>>,
    state: Mutex<State>,
    /// One load, unload or restart at a time, so the version check and the insert can't interleave
    ops: tokio::sync::Mutex<()>,
    events: broadcast::Sender<(Option<ModuleId>, Event)>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The registry, shared by the controller, its console and the admin API.
#[derive(Clone)]
pub struct ModuleRegistry {
    inner: Arc<Inner>,
}

/// A registry that doesn't keep the server alive: what the core plugins hold, since the registry holds
/// them.
#[derive(Clone)]
pub struct WeakRegistry {
    inner: Weak<Inner>,
}

impl WeakRegistry {
    pub fn upgrade(&self) -> Option<ModuleRegistry> {
        self.inner.upgrade().map(|inner| ModuleRegistry { inner })
    }
}

impl ModuleRegistry {
    /// Its modules show in `status` beside the services, each as a component named by its id.
    pub fn new(options: RegistryOptions, status: StatusRegistry) -> Self {
        let mut state = State::default();
        for (transport, range) in &options.reserved_ports {
            state.ports.reserve(*transport, *range);
        }
        let (events, _) = broadcast::channel(256);
        Self {
            inner: Arc::new(Inner {
                home: RwLock::new(options.home.clone()),
                node_services: NodeServices::new(options.node.node.clone()),
                options,
                status,
                plugins: Mutex::new(Vec::new()),
                hooks: Mutex::new(Vec::new()),
                state: Mutex::new(state),
                ops: tokio::sync::Mutex::new(()),
                events,
            }),
        }
    }

    pub fn downgrade(&self) -> WeakRegistry {
        WeakRegistry {
            inner: Arc::downgrade(&self.inner),
        }
    }

    /// The range a running module was given for its port `name` (DD4: where the controller moved it).
    pub fn port(&self, id: &ModuleId, name: &str) -> Option<PortRange> {
        let state = lock(&self.inner.state);
        let slot = state.slots.get(id).filter(|slot| slot.state == ModuleState::Ready)?;
        slot.loaded.as_ref()?.ports.get(name).copied()
    }

    /// The range assigned to a module's port `name` from the moment the controller places it, before the module
    /// starts; the core plugins read it as they attach (DF4d). [`port`](Self::port) answers once it runs.
    pub fn assigned(&self, id: &ModuleId, name: &str) -> Option<PortRange> {
        lock(&self.inner.state).ports.held(id, name)
    }

    /// The folder an external module's files are relative to (its executable's); none for a compiled-in one.
    pub fn module_dir(&self, id: &ModuleId) -> Option<std::path::PathBuf> {
        let state = lock(&self.inner.state);
        match &state.slots.get(id)?.entry.source {
            ModuleSource::External(external) => external
                .path
                .parent()
                .filter(|dir| !dir.as_os_str().is_empty())
                .map(Path::to_owned),
            ModuleSource::CompiledIn(_) => None,
        }
    }

    /// An admin action for the loaded module `id`, if it registered that action (DF4c).
    pub async fn admin_for(&self, id: &ModuleId, call: AdminCall) -> Result<AdminResponse, ModuleError> {
        let loaded = self
            .ready(id)
            .map_err(|e| ModuleError::new(ErrorKind::Unavailable, e.to_string()))?;
        let action: AdminAction = call.request.action();
        if !loaded.registration.admin_actions.contains(&action) {
            return Err(ModuleError::unsupported(format!("{action:?} on {id}")));
        }
        loaded.driver.get().admin(call).await
    }

    /// A compiled-in core plugin, handed each module's registration as it loads (DD3).
    pub fn add_plugin(&self, plugin: Arc<dyn CorePlugin>) {
        lock(&self.inner.plugins).push(plugin);
    }

    /// Told of every module that starts and every one that goes (see [`LoadHook`]).
    pub fn add_hook(&self, hook: Arc<dyn LoadHook>) {
        let already_loaded: Vec<(ModuleManifest, Arc<Driver>)> = lock(&self.inner.state)
            .slots
            .values()
            .filter(|slot| slot.state == ModuleState::Ready)
            .filter_map(|slot| {
                let loaded = slot.loaded.as_ref()?;
                Some((loaded.manifest.clone(), loaded.driver.clone()))
            })
            .collect();
        lock(&self.inner.hooks).push(hook.clone());
        for (manifest, driver) in already_loaded {
            hook.loaded(&manifest, driver);
        }
    }

    pub fn status_registry(&self) -> &StatusRegistry {
        &self.inner.status
    }

    /// What this node answers over its node links, by service. A core plugin that serves other nodes attaches
    /// its service here (`node_services().attach(Service::X, ..)`); a server outside a network never serves it.
    pub fn node_services(&self) -> &NodeServices {
        &self.inner.node_services
    }

    /// The home API modules reach through `HostCall::Home`: `options.home` unless a network swapped it.
    pub fn home(&self) -> Arc<dyn HomeApi> {
        self.inner.home.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Modules' home calls go to `home` from now on (the network's, DN5).
    pub fn set_home(&self, home: Arc<dyn HomeApi>) {
        *self.inner.home.write().unwrap_or_else(PoisonError::into_inner) = home;
    }

    /// Back to `options.home`, the home of a server outside a network.
    pub fn reset_home(&self) {
        self.set_home(self.inner.options.home.clone());
    }

    /// Lists a module. A name a service already has is refused, so the console names one thing.
    pub fn add_entry(&self, entry: ModuleEntry) -> Result<(), RegistryError> {
        let mut state = lock(&self.inner.state);
        if state.slots.contains_key(&entry.id) {
            return Err(RegistryError::ListedTwice(entry.id));
        }
        if self.inner.status.snapshot().get(entry.id.as_str()).is_some() {
            return Err(RegistryError::NameTaken(entry.id));
        }
        let known = match &entry.source {
            ModuleSource::CompiledIn(factory) => Some(factory().manifest()),
            ModuleSource::External(_) => None,
        };
        let version = known.as_ref().map(|m| m.game_version.to_string());
        let status = self
            .inner
            .status
            .register_component(entry.id.as_str(), version.as_deref());
        state.slots.insert(
            entry.id.clone(),
            Slot {
                entry,
                status,
                state: ModuleState::NotLoaded,
                detail: None,
                known,
                loaded: None,
            },
        );
        Ok(())
    }

    pub fn is_listed(&self, id: &str) -> bool {
        id.parse::<ModuleId>()
            .is_ok_and(|id| lock(&self.inner.state).slots.contains_key(&id))
    }

    pub fn list(&self) -> Vec<ModuleInfo> {
        lock(&self.inner.state).slots.values().map(Slot::info).collect()
    }

    pub fn info(&self, id: &ModuleId) -> Result<ModuleInfo, RegistryError> {
        lock(&self.inner.state)
            .slots
            .get(id)
            .map(Slot::info)
            .ok_or_else(|| RegistryError::NotListed(id.clone()))
    }

    /// Replaces the `[modules.<id>.<section>]` values the next load lays over the defaults (the server GUI
    /// saved them to host.toml, DU3). A module that is running keeps what it started with.
    pub fn update_settings(
        &self,
        id: &ModuleId,
        settings: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
    ) -> Result<(), RegistryError> {
        lock(&self.inner.state)
            .slots
            .get_mut(id)
            .map(|slot| slot.entry.settings = settings)
            .ok_or_else(|| RegistryError::NotListed(id.clone()))
    }

    /// The game module an invite's `version` names, by version or alias (DF4a).
    pub fn for_invite(&self, version: &str) -> Option<ModuleInfo> {
        let state = lock(&self.inner.state);
        state
            .games
            .values()
            .filter_map(|id| state.slots.get(id))
            .find(|slot| slot.loaded.as_ref().is_some_and(|l| l.manifest.answers_invite(version)))
            .map(Slot::info)
    }

    /// What happened on the bus: the sending module (None for the host) and the event.
    pub fn subscribe_events(&self) -> broadcast::Receiver<(Option<ModuleId>, Event)> {
        self.inner.events.subscribe()
    }

    /// An event from the host itself (`server.ready`), to every module that subscribed.
    pub fn emit(&self, event: Event) {
        self.inner.publish(None, event);
    }

    /// Loads every module listed to load at start; what failed stays listed as failed, with its reason.
    pub async fn load_autoload(&self) -> Vec<RegistryError> {
        let ids: Vec<ModuleId> = lock(&self.inner.state)
            .slots
            .values()
            .filter(|slot| slot.entry.autoload)
            .map(|slot| slot.entry.id.clone())
            .collect();
        let mut failed = Vec::new();
        for id in ids {
            if let Err(error) = self.load(&id).await {
                failed.push(error);
            }
        }
        failed
    }

    /// Loads a listed module. One that failed is unloaded first, so this is also its restart.
    pub async fn load(&self, id: &ModuleId) -> Result<ModuleInfo, RegistryError> {
        let _op = self.inner.ops.lock().await;
        self.inner.watch(&self.inner);
        let entry = {
            let state = lock(&self.inner.state);
            let slot = state
                .slots
                .get(id)
                .ok_or_else(|| RegistryError::NotListed(id.clone()))?;
            match slot.state {
                ModuleState::NotLoaded | ModuleState::Failed => slot.entry.clone(),
                ModuleState::Starting | ModuleState::Ready => return Err(RegistryError::AlreadyLoaded(id.clone())),
            }
        };
        self.inner.take_down(id, "restarting").await;
        self.inner.set_state(id, ModuleState::Starting, None);
        match self.inner.bring_up(entry).await {
            Ok(()) => {
                self.inner.set_state(id, ModuleState::Ready, None);
                tracing::info!(target: "host", "module {id} loaded");
            }
            Err(error) => {
                self.inner.set_state(id, ModuleState::Failed, Some(error.to_string()));
                tracing::warn!(target: "host", "{error}");
                return Err(error);
            }
        }
        self.info(id)
    }

    /// Stops a module for good: its plugins first, then it, its process tree killed after the grace (DD5).
    pub async fn unload(&self, id: &ModuleId) -> Result<(), RegistryError> {
        let _op = self.inner.ops.lock().await;
        {
            let state = lock(&self.inner.state);
            let slot = state
                .slots
                .get(id)
                .ok_or_else(|| RegistryError::NotListed(id.clone()))?;
            if slot.state == ModuleState::NotLoaded {
                return Err(RegistryError::NotLoaded(id.clone()));
            }
        }
        self.inner.unload_with_plugins(id).await;
        Ok(())
    }

    pub async fn restart(&self, id: &ModuleId) -> Result<ModuleInfo, RegistryError> {
        match self.unload(id).await {
            Ok(()) | Err(RegistryError::NotLoaded(_)) => {}
            Err(error) => return Err(error),
        }
        self.load(id).await
    }

    /// Unloads everything, plugins before the game modules they extend.
    pub async fn unload_all(&self) {
        let _op = self.inner.ops.lock().await;
        let ids: Vec<(ModuleId, bool)> = lock(&self.inner.state)
            .slots
            .values()
            .filter(|slot| slot.state != ModuleState::NotLoaded)
            .map(|slot| {
                let plugin = slot
                    .loaded
                    .as_ref()
                    .is_some_and(|l| l.manifest.kind == ModuleKind::Plugin);
                (slot.entry.id.clone(), plugin)
            })
            .collect();
        for plugins in [true, false] {
            for (id, _) in ids.iter().filter(|(_, plugin)| *plugin == plugins) {
                self.inner.take_down(id, "unloaded").await;
                self.inner
                    .set_state(id, ModuleState::NotLoaded, Some("unloaded".into()));
            }
        }
    }

    fn ready(&self, id: &ModuleId) -> Result<Arc<Loaded>, RegistryError> {
        let state = lock(&self.inner.state);
        let slot = state
            .slots
            .get(id)
            .ok_or_else(|| RegistryError::NotListed(id.clone()))?;
        match (&slot.loaded, slot.state) {
            (Some(loaded), ModuleState::Ready) => Ok(loaded.clone()),
            _ => Err(RegistryError::NotLoaded(id.clone())),
        }
    }

    /// The module's live status, asked of the module itself.
    pub async fn module_status(&self, id: &ModuleId) -> Result<module_api::ModuleStatus, RegistryError> {
        let loaded = self.ready(id)?;
        loaded
            .driver
            .get()
            .status()
            .await
            .map_err(|error| RegistryError::Failed {
                id: id.clone(),
                reason: error.to_string(),
            })
    }

    /// One of the commands the module registered (DD3), with the actor that asked (DH1).
    pub async fn command(&self, id: &ModuleId, request: CommandRequest) -> Result<CommandOutput, ModuleError> {
        let loaded = self
            .ready(id)
            .map_err(|e| ModuleError::new(ErrorKind::Unavailable, e.to_string()))?;
        if !loaded.registration.commands.iter().any(|c| c.name == request.name) {
            return Err(ModuleError::new(
                ErrorKind::NotFound,
                format!("{id} has no command {}", request.name),
            ));
        }
        loaded.driver.get().command(request).await
    }

    /// An admin action for the game module of `version`, if it registered that action (DF4c).
    pub async fn admin(&self, version: &GameVersion, call: AdminCall) -> Result<AdminResponse, ModuleError> {
        let id = lock(&self.inner.state)
            .games
            .get(version)
            .cloned()
            .ok_or_else(|| ModuleError::new(ErrorKind::NotFound, format!("no module serves {version}")))?;
        let loaded = self
            .ready(&id)
            .map_err(|e| ModuleError::new(ErrorKind::Unavailable, e.to_string()))?;
        let action: AdminAction = call.request.action();
        if !loaded.registration.admin_actions.contains(&action) {
            return Err(ModuleError::unsupported(format!("{action:?} on {id}")));
        }
        loaded.driver.get().admin(call).await
    }
}

impl Inner {
    fn set_state(&self, id: &ModuleId, state: ModuleState, detail: Option<String>) {
        let mut guard = lock(&self.state);
        let Some(slot) = guard.slots.get_mut(id) else {
            return;
        };
        slot.state = state;
        slot.detail.clone_from(&detail);
        let port = slot
            .loaded
            .as_ref()
            .and_then(|l| l.ports.values().next().map(|range| range.first));
        let detail = detail.unwrap_or_default();
        match state {
            ModuleState::NotLoaded => slot.status.report_stopped(detail),
            ModuleState::Starting => slot.status.report_starting("loading"),
            ModuleState::Ready => slot.status.report_ready(port, "loaded"),
            ModuleState::Failed => slot.status.failed(detail),
        }
    }

    /// The child watcher, started with the first load (the registry may be made before a runtime runs).
    fn watch(&self, this: &Arc<Inner>) {
        {
            let mut state = lock(&self.state);
            if state.watching {
                return;
            }
            state.watching = true;
        }
        let weak: Weak<Inner> = Arc::downgrade(this);
        let every = self.options.watch_every;
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(every);
            loop {
                tick.tick().await;
                let Some(inner) = weak.upgrade() else {
                    break;
                };
                let broken: Vec<(ModuleId, String)> = lock(&inner.state)
                    .slots
                    .values()
                    .filter(|slot| slot.state == ModuleState::Ready)
                    .filter_map(|slot| {
                        let failure = slot.loaded.as_ref()?.driver.failure()?;
                        Some((slot.entry.id.clone(), failure))
                    })
                    .collect();
                for (id, why) in broken {
                    tracing::warn!(target: "host", "module {id} failed: {why}");
                    inner.set_state(&id, ModuleState::Failed, Some(why));
                }
            }
        });
    }

    async fn bring_up(self: &Arc<Self>, entry: ModuleEntry) -> Result<(), RegistryError> {
        let id = entry.id.clone();
        let failed = |reason: String| RegistryError::Failed { id: id.clone(), reason };
        let link: Arc<dyn HostLink> = Arc::new(Link {
            inner: Arc::downgrade(self),
        });
        let driver = match &entry.source {
            ModuleSource::CompiledIn(factory) => Driver::InProcess(
                InProcessModule::new(factory(), self.options.node.clone(), link).map_err(|e| failed(e.to_string()))?,
            ),
            ModuleSource::External(external) => {
                let mut external = external.clone();
                external.expected = Some(id.clone());
                let mut options = self.options.child.clone();
                if options.env.is_none() {
                    options.env = Some(system_environment(std::env::vars()));
                }
                if options.current_dir.is_none() {
                    options.current_dir = external
                        .path
                        .parent()
                        .filter(|p| !p.as_os_str().is_empty())
                        .map(Path::to_owned);
                }
                if plugin_host::is_library(&external.path) {
                    Driver::Library(
                        plugin_host::DylibModule::load(&external, self.options.node.clone(), link, options)
                            .await
                            .map_err(|e| failed(e.to_string()))?,
                    )
                } else {
                    Driver::Child(
                        ChildModule::spawn(&external, self.options.node.clone(), link, options)
                            .await
                            .map_err(|e| failed(e.to_string()))?,
                    )
                }
            }
        };
        let driver = Arc::new(driver);
        match self.check_and_start(&entry, driver.clone()).await {
            Ok(()) => Ok(()),
            Err(error) => {
                // Whatever was started goes, the child's tree with it
                if let Err(stop) = driver.get().unload().await {
                    tracing::debug!(target: "host", "module {id}: {stop}");
                }
                Err(error)
            }
        }
    }

    async fn check_and_start(self: &Arc<Self>, entry: &ModuleEntry, driver: Arc<Driver>) -> Result<(), RegistryError> {
        let id = &entry.id;
        let manifest = driver.get().manifest().clone();
        let refused = |reason: String| RegistryError::Refused { id: id.clone(), reason };
        if manifest.id != *id {
            return Err(refused(format!("its manifest names it {}", manifest.id)));
        }
        let version = manifest.game_version.clone();
        {
            let state = lock(&self.state);
            match manifest.kind {
                ModuleKind::Game => {
                    let by = self
                        .options
                        .taken_versions
                        .iter()
                        .find(|(taken, _)| *taken == version)
                        .map(|(_, by)| by.clone())
                        .or_else(|| state.games.get(&version).map(ToString::to_string));
                    if let Some(by) = by {
                        return Err(RegistryError::Duplicate {
                            id: id.clone(),
                            version,
                            by,
                        });
                    }
                }
                ModuleKind::Plugin => {
                    let game = state
                        .games
                        .get(&version)
                        .and_then(|game| state.slots.get(game))
                        .and_then(|slot| slot.loaded.clone())
                        .ok_or_else(|| RegistryError::NoGameFor {
                            id: id.clone(),
                            version: version.clone(),
                        })?;
                    plugin_fits(&game.manifest, &manifest).map_err(|e: FitError| refused(e.to_string()))?;
                }
            }
        }
        let registration = driver.get().register().await.map_err(|e| refused(e.to_string()))?;
        let mut config = ModuleConfig::defaults(&manifest, &registration);
        for (section, values) in &entry.settings {
            let Some(registered) = registration.config.iter().find(|s| s.name == *section) else {
                return Err(refused(format!(
                    "host.toml has [modules.{id}.{section}], a section it does not read"
                )));
            };
            for (key, value) in values {
                if !registered.keys.iter().any(|k| k.name == *key) {
                    return Err(refused(format!(
                        "host.toml sets {key} in [modules.{id}.{section}], which it does not read"
                    )));
                }
                config
                    .sections
                    .entry(section.clone())
                    .or_default()
                    .insert(key.clone(), value.clone());
            }
        }
        config.ports = lock(&self.state)
            .ports
            .assign(id, &manifest.ports)
            .map_err(|e| refused(e.to_string()))?;
        let needs = registration.needs();
        let candidates: Vec<Arc<dyn CorePlugin>> = lock(&self.plugins).clone();
        let mut attached: Vec<Arc<dyn CorePlugin>> = Vec::new();
        for plugin in candidates {
            if !plugin.serves().iter().any(|cap| needs.contains(cap)) {
                continue;
            }
            if let Err(error) = plugin.attach(&manifest, &registration) {
                for done in &attached {
                    done.detach(id);
                }
                lock(&self.state).ports.release(id);
                return Err(refused(format!("{}: {error}", plugin.name())));
            }
            attached.push(plugin);
        }
        let loaded = Arc::new(Loaded {
            driver: driver.clone(),
            manifest: manifest.clone(),
            registration,
            ports: config.ports.clone(),
            plugins: attached,
        });
        {
            let mut state = lock(&self.state);
            if let Some(slot) = state.slots.get_mut(id) {
                slot.loaded = Some(loaded.clone());
            }
            if manifest.kind == ModuleKind::Game {
                state.games.insert(version.clone(), id.clone());
            }
        }
        // Listed before it starts: its first log line and event already go through the link
        if let Err(error) = driver.get().start(config).await {
            self.forget(id, &loaded);
            return Err(RegistryError::Failed {
                id: id.clone(),
                reason: error.to_string(),
            });
        }
        let hooks: Vec<Arc<dyn LoadHook>> = lock(&self.hooks).clone();
        for hook in hooks {
            hook.loaded(&manifest, driver.clone());
        }
        Ok(())
    }

    /// Drops what the registry held for a module: its version, its ports, its core plugins' parts.
    fn forget(&self, id: &ModuleId, loaded: &Loaded) {
        for plugin in &loaded.plugins {
            plugin.detach(id);
        }
        let hooks: Vec<Arc<dyn LoadHook>> = lock(&self.hooks).clone();
        for hook in hooks {
            hook.unloaded(id);
        }
        let mut state = lock(&self.state);
        if state.games.get(&loaded.manifest.game_version) == Some(id) {
            state.games.remove(&loaded.manifest.game_version);
        }
        state.ports.release(id);
        if let Some(slot) = state.slots.get_mut(id) {
            slot.loaded = None;
        }
    }

    /// Unloads `id` if anything of it is loaded (a failed child's tree is still killed).
    async fn take_down(&self, id: &ModuleId, why: &str) {
        let loaded = lock(&self.state).slots.get(id).and_then(|slot| slot.loaded.clone());
        let Some(loaded) = loaded else {
            return;
        };
        if let Err(error) = loaded.driver.get().unload().await {
            tracing::debug!(target: "host", "module {id} {why}: {error}");
        }
        self.forget(id, &loaded);
    }

    async fn unload_with_plugins(&self, id: &ModuleId) {
        let plugins: Vec<ModuleId> = {
            let state = lock(&self.state);
            let game = state.slots.get(id).and_then(|slot| slot.loaded.clone());
            match game {
                Some(game) if game.manifest.kind == ModuleKind::Game => state
                    .slots
                    .values()
                    .filter(|slot| {
                        slot.loaded.as_ref().is_some_and(|l| {
                            l.manifest.kind == ModuleKind::Plugin
                                && l.manifest.game_version == game.manifest.game_version
                        })
                    })
                    .map(|slot| slot.entry.id.clone())
                    .collect(),
                _ => Vec::new(),
            }
        };
        for plugin in plugins.iter().chain(std::iter::once(id)) {
            self.take_down(plugin, "unloaded").await;
            self.set_state(plugin, ModuleState::NotLoaded, Some("unloaded".into()));
        }
    }

    /// The bus (DF3): an event to every loaded module that subscribed to it, the sender excepted, and to
    /// the host's own listeners.
    fn publish(&self, from: Option<ModuleId>, event: Event) {
        let targets: Vec<(ModuleId, Arc<Driver>)> = lock(&self.state)
            .slots
            .values()
            .filter(|slot| slot.state == ModuleState::Ready && Some(&slot.entry.id) != from.as_ref())
            .filter_map(|slot| {
                let loaded = slot.loaded.as_ref()?;
                loaded
                    .registration
                    .subscribes
                    .contains(&event.name)
                    .then(|| (slot.entry.id.clone(), loaded.driver.clone()))
            })
            .collect();
        for (id, driver) in targets {
            let event = event.clone();
            tokio::spawn(async move {
                if let Err(error) = driver.get().event(event).await {
                    tracing::warn!(target: "host", "module {id} did not take an event: {error}");
                }
            });
        }
        _ = self.events.send((from, event));
    }
}

/// The host as every module sees it (`HostLink`).
struct Link {
    inner: Weak<Inner>,
}

impl HostLink for Link {
    fn call<'a>(&'a self, module: &'a ModuleId, call: HostCall) -> BoxFuture<'a, Result<HostAnswer, ModuleError>> {
        Box::pin(async move {
            let inner = self
                .inner
                .upgrade()
                .ok_or_else(|| ModuleError::new(ErrorKind::Unavailable, "the server is stopping"))?;
            match call {
                HostCall::Emit(event) => {
                    // Only what it registered: a module can't speak for another's events
                    let emits = lock(&inner.state)
                        .slots
                        .get(module)
                        .and_then(|slot| slot.loaded.as_ref())
                        .is_some_and(|l| l.registration.emits.contains(&event.name));
                    if !emits {
                        return Err(ModuleError::new(
                            ErrorKind::Invalid,
                            format!("{module} did not register the event {}", event.name),
                        ));
                    }
                    inner.publish(Some(module.clone()), event);
                    Ok(HostAnswer::Done)
                }
                HostCall::Log(record) => {
                    let message = record.message;
                    match record.level {
                        LogLevel::Error => tracing::error!(target: "module", service = %module, "{message}"),
                        LogLevel::Warn => tracing::warn!(target: "module", service = %module, "{message}"),
                        LogLevel::Info => tracing::info!(target: "module", service = %module, "{message}"),
                        LogLevel::Debug => tracing::debug!(target: "module", service = %module, "{message}"),
                    }
                    Ok(HostAnswer::Done)
                }
                HostCall::Home(call) => {
                    let home = inner.home.read().unwrap_or_else(PoisonError::into_inner).clone();
                    Ok(HostAnswer::Home(dispatch_home(home.as_ref(), call).await))
                }
            }
        })
    }
}
