// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! [`ModuleRegistry`]: the game modules and plugins this server runs (DF3), keyed by game version (DD4),
//! compiled in or started as their own process (DD5). Only what `settings.toml` lists can be loaded
//! ([`ModuleEntry`]); a second game module for a version already served is refused whichever came first,
//! the host kit's own stacks included ([`RegistryOptions::taken_versions`]).
//!
//! A load: the driver (in process, or the child checked against its SHA-256), the manifest's game
//! version checked, the registration checked against the manifest, ports placed ([`PortBook`]), the
//! `[modules.<id>.<section>]` settings laid over the registered defaults, the core plugins attached, and
//! `start`. The registry is also the modules' [`HostLink`]: their events go on the bus to the modules that
//! subscribed, their log lines into tracing (target `module`, under the module's name), their home calls
//! to the node's [`HomeApi`]. A child that dies shows as failed and stays so until it is loaded again, unless
//! `settings.toml` asks for it to be restarted ([`ModuleRegistry::set_restart`], [`RestartPolicy`]).

use std::collections::{BTreeMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError, RwLock, Weak};
use std::time::{Duration, Instant};

use dainty_module_api::manifest::{FitError, Transport, plugin_fits};
use dainty_module_api::network::dispatch_home;
use dainty_module_api::registrar::{AdminAction, CommandSpec, ConfigSection, Registration};
use dainty_module_api::{
    AdminCall, AdminResponse, BoxFuture, ChildModule, ChildOptions, CommandOutput, CommandRequest, CorePlugin,
    ErrorKind, Event, EventName, ExternalModule, GameAnswer, GameCall, GameCallKind, GameModule, GameServer,
    GameVersion, HomeApi, HostAnswer, HostCall, HostLink, InProcessModule, LogLevel, ModuleConfig, ModuleDriver,
    ModuleDriverExt, ModuleError, ModuleId, ModuleKind, ModuleManifest, NodeIdentity, PortRange, UnavailableReason,
};
use dainty_service_kit::context::{StatusRegistry, StatusReporter};
use serde::Serialize;
use tokio::sync::broadcast;

use crate::node_services::NodeServices;
use crate::ports::PortBook;

/// Builds a compiled-in module afresh for each load.
pub type ModuleFactory = Arc<dyn Fn() -> Arc<dyn GameModule> + Send + Sync>;

/// Where a listed module comes from.
#[derive(Clone)]
pub enum ModuleSource {
    /// Built into this binary; the factory makes a fresh instance for each load.
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

/// A module `settings.toml` lists: the only kind the registry loads.
#[derive(Debug, Clone)]
pub struct ModuleEntry {
    /// The module's id, also its component name.
    pub id: ModuleId,
    /// Where it is loaded from.
    pub source: ModuleSource,
    /// Loaded when the server starts; otherwise loaded from the console or the admin API
    pub autoload: bool,
    /// `[modules.<id>.<section>]`: values laid over the defaults the module registered
    pub settings: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
}

/// How the registry runs its modules.
pub struct RegistryOptions {
    /// This node's identity, given to every module.
    pub node: NodeIdentity,
    /// The home API modules reach through `HostLink` until the network swaps it ([`ModuleRegistry::set_home`])
    pub home: Arc<dyn HomeApi>,
    /// For every external module; `env` and `current_dir` are filled per module when `None`
    pub child: ChildOptions,
    /// The server's own ports, which no module is given
    pub reserved_ports: Vec<(Transport, PortRange)>,
    /// The host's first choice for a manifest's port of that name, over the manifest's default (settings.toml
    /// `[ports]`)
    pub port_defaults: std::collections::BTreeMap<String, u16>,
    /// Game versions the host kit serves outside the registry, and what serves them
    pub taken_versions: Vec<(GameVersion, String)>,
    /// How often children are checked for a broken pipe
    pub watch_every: Duration,
    /// How a dead external module with `restart = true` is brought back
    pub restart: RestartPolicy,
}

/// The event the host emits before each restart of a dead external module (MS2).
pub const MODULE_RESTARTED_EVENT: &str = "module.restarted";

/// How a dead external module with `restart = true` is started again (the module-sharing plan's A2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestartPolicy {
    /// The wait before the first restart.
    pub first_delay: Duration,
    /// The longest wait; each failure doubles it up to this.
    pub max_delay: Duration,
    /// Failures inside it count toward `max_failures`; up this long, the wait goes back to `first_delay`
    pub window: Duration,
    /// This many failures inside `window` leave the module failed.
    pub max_failures: usize,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            first_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(60),
            window: Duration::from_secs(600),
            max_failures: 5,
        }
    }
}

/// One loaded game module's game servers, or why it couldn't say.
#[derive(Debug, Clone, PartialEq)]
pub struct GameServers {
    /// The game module.
    pub module: ModuleId,
    /// The game version it serves.
    pub version: GameVersion,
    /// Its answer to [`GameCall::Servers`].
    pub servers: Result<Vec<GameServer>, ModuleError>,
}

impl RegistryOptions {
    /// A server outside a network, no ports or versions taken, children given only the OS's own
    /// variables ([`system_environment`]).
    pub fn standalone() -> Self {
        let identity = NodeIdentity::standalone();
        Self {
            home: Arc::new(dainty_module_api::network::StandaloneHome {
                identity: identity.clone(),
            }),
            node: identity,
            child: ChildOptions {
                env: Some(system_environment(std::env::vars())),
                ..ChildOptions::default()
            },
            reserved_ports: Vec::new(),
            port_defaults: std::collections::BTreeMap::new(),
            taken_versions: Vec::new(),
            watch_every: Duration::from_millis(250),
            restart: RestartPolicy::default(),
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

/// Why the registry refused or failed a load, unload or call.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    /// The id is not in `settings.toml`'s `[modules]`.
    #[error("no module {0} is listed in settings.toml ([modules])")]
    NotListed(ModuleId),
    /// `settings.toml` lists the id more than once.
    #[error("module {0} is listed twice")]
    ListedTwice(ModuleId),
    /// A service already has the id as its name.
    #[error("{0} is already a service's name")]
    NameTaken(ModuleId),
    /// A load asked of a module that is already loaded.
    #[error("module {0} is already loaded")]
    AlreadyLoaded(ModuleId),
    /// An unload or call asked of a module that is not loaded.
    #[error("module {0} is not loaded")]
    NotLoaded(ModuleId),
    /// A game module for a version another module or the host kit already serves.
    #[error("{id} was refused: game version {version} is already served by {by}")]
    Duplicate {
        /// The module refused.
        id: ModuleId,
        /// The game version both serve.
        version: GameVersion,
        /// What already serves it.
        by: String,
    },
    /// A plugin for a game version no loaded module serves.
    #[error("{id} extends game version {version}, and no module for it is loaded")]
    NoGameFor {
        /// The plugin refused.
        id: ModuleId,
        /// The game version it extends.
        version: GameVersion,
    },
    /// The module's manifest does not match its listing or what the server allows.
    #[error("{id} was refused: {reason}")]
    Refused {
        /// The module refused.
        id: ModuleId,
        /// Why it was refused.
        reason: String,
    },
    /// The module failed to start, or an answer from it failed.
    #[error("{id}: {reason}")]
    Failed {
        /// The module that failed.
        id: ModuleId,
        /// The module's or the driver's error.
        reason: String,
    },
}

/// Where a listed module is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModuleState {
    /// Listed and not running: never loaded, unloaded, or stopped.
    NotLoaded,
    /// Loading, until it reports ready.
    Starting,
    /// Running and ready.
    Ready,
    /// It broke after it started (its process died, its pipe broke) or failed to start
    Failed,
}

/// One listed module, as the console, the TUI, the server GUI and the admin API show it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModuleInfo {
    /// Its id, also its component name.
    pub id: ModuleId,
    /// `compiled_in` or `external`
    pub source: &'static str,
    /// Whether it loads when the server starts.
    pub autoload: bool,
    /// Where it is now.
    pub state: ModuleState,
    /// Why it failed or stopped, when it did.
    pub detail: Option<String>,
    /// Known once it was loaded, or at once for a compiled-in one
    pub manifest: Option<ModuleManifest>,
    /// The port ranges it was given, by the manifest's port name (DD4).
    pub ports: BTreeMap<String, PortRange>,
    /// The commands it registered for the console and the admin API (DD3).
    pub commands: Vec<CommandSpec>,
    /// The `[modules.<id>.<section>]` keys it registered, with their defaults: what the server GUI's
    /// settings editor draws (DU3). Empty until it loads
    pub config: Vec<ConfigSection>,
}

enum Driver {
    InProcess(InProcessModule),
    Child(ChildModule),
    Library(dainty_plugin_host::DylibModule),
}

impl ModuleDriver for Driver {
    fn manifest(&self) -> &ModuleManifest {
        self.get().manifest()
    }

    fn call(
        &self,
        call: dainty_module_api::frame::ModuleCall,
    ) -> BoxFuture<'_, Result<dainty_module_api::frame::ModuleAnswer, ModuleError>> {
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
    /// `restart = true` in its `[[modules.external]]`
    restart: bool,
    restarts: Restarts,
}

#[derive(Default)]
struct Restarts {
    failures: VecDeque<Instant>,
    delay: Option<Duration>,
    up_since: Option<Instant>,
    /// Bumped by every load or unload from outside, so a pending restart sees it was overtaken
    generation: u64,
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
    /// The registry, while the server still holds it.
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
        for (name, first) in &options.port_defaults {
            state.ports.prefer(name, *first);
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

    /// A handle that doesn't keep the registry alive.
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

    /// The status the modules show in, beside the services.
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
                restart: false,
                restarts: Restarts::default(),
            },
        );
        Ok(())
    }

    /// Whether a listed external module is started again when it dies ([`RestartPolicy`]); off unless set.
    pub fn set_restart(&self, id: &ModuleId, restart: bool) -> Result<(), RegistryError> {
        let mut state = lock(&self.inner.state);
        let slot = state
            .slots
            .get_mut(id)
            .ok_or_else(|| RegistryError::NotListed(id.clone()))?;
        if restart && matches!(slot.entry.source, ModuleSource::CompiledIn(_)) {
            return Err(RegistryError::Refused {
                id: id.clone(),
                reason: "only an external module is restarted when it dies".into(),
            });
        }
        slot.restart = restart;
        Ok(())
    }

    /// Whether `settings.toml` lists the module `id` names; false for a string that is no module id.
    pub fn is_listed(&self, id: &str) -> bool {
        id.parse::<ModuleId>()
            .is_ok_and(|id| lock(&self.inner.state).slots.contains_key(&id))
    }

    /// Every listed module, loaded or not.
    pub fn list(&self) -> Vec<ModuleInfo> {
        lock(&self.inner.state).slots.values().map(Slot::info).collect()
    }

    /// One listed module; [`RegistryError::NotListed`] when `settings.toml` doesn't list it.
    pub fn info(&self, id: &ModuleId) -> Result<ModuleInfo, RegistryError> {
        lock(&self.inner.state)
            .slots
            .get(id)
            .map(Slot::info)
            .ok_or_else(|| RegistryError::NotListed(id.clone()))
    }

    /// Replaces the `[modules.<id>.<section>]` values the next load lays over the defaults (the server GUI
    /// saved them to settings.toml, DU3). A module that is running keeps what it started with.
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
        self.load_as(id, None).await
    }

    /// `restart` is the generation an automatic restart was scheduled at; a load from outside starts its
    /// count of failures afresh
    async fn load_as(&self, id: &ModuleId, restart: Option<u64>) -> Result<ModuleInfo, RegistryError> {
        let _op = self.inner.ops.lock().await;
        self.inner.watch(&self.inner);
        let entry = {
            let mut state = lock(&self.inner.state);
            let slot = state
                .slots
                .get_mut(id)
                .ok_or_else(|| RegistryError::NotListed(id.clone()))?;
            match restart {
                None => slot.restarts.reset(),
                Some(generation) if generation != slot.restarts.generation || slot.state != ModuleState::Failed => {
                    return Err(RegistryError::AlreadyLoaded(id.clone()));
                }
                Some(_) => {}
            }
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
                if restart.is_some() {
                    self.inner.failed(id, error.to_string());
                }
                return Err(error);
            }
        }
        self.info(id)
    }

    /// Stops a module for good: its plugins first, then it, its process tree killed after the grace (DD5).
    pub async fn unload(&self, id: &ModuleId) -> Result<(), RegistryError> {
        let _op = self.inner.ops.lock().await;
        {
            let mut state = lock(&self.inner.state);
            let slot = state
                .slots
                .get_mut(id)
                .ok_or_else(|| RegistryError::NotListed(id.clone()))?;
            slot.restarts.reset();
            if slot.state == ModuleState::NotLoaded {
                return Err(RegistryError::NotLoaded(id.clone()));
            }
        }
        self.inner.unload_with_plugins(id).await;
        Ok(())
    }

    /// Unloads the module if it is loaded and loads it again.
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
    pub async fn module_status(&self, id: &ModuleId) -> Result<dainty_module_api::ModuleStatus, RegistryError> {
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

    /// A game call for the game module of `version`, if it registered that call (MS2). One loaded and not
    /// ready answers [`UnavailableReason::ModuleNotReady`], so a reader translates it like the module's own
    pub async fn game(&self, version: &GameVersion, call: GameCall) -> Result<GameAnswer, ModuleError> {
        let id = lock(&self.inner.state)
            .games
            .get(version)
            .cloned()
            .ok_or_else(|| ModuleError::new(ErrorKind::NotFound, format!("no module serves {version}")))?;
        let loaded = self.ready(&id).map_err(|_| UnavailableReason::ModuleNotReady.error())?;
        let kind = call.kind();
        if !loaded.registration.game_calls.contains(&kind) {
            return Err(ModuleError::unsupported(format!("{kind:?} on {id}")));
        }
        loaded.driver.get().game(call).await
    }

    /// The game servers of every ready game module that answers [`GameCall::Servers`], by module.
    pub async fn game_servers(&self) -> Vec<GameServers> {
        let modules: Vec<(ModuleId, Arc<Loaded>)> = {
            let state = lock(&self.inner.state);
            state
                .games
                .values()
                .filter_map(|id| state.slots.get(id))
                .filter(|slot| slot.state == ModuleState::Ready)
                .filter_map(|slot| Some((slot.entry.id.clone(), slot.loaded.clone()?)))
                .filter(|(_, loaded)| loaded.registration.game_calls.contains(&GameCallKind::Servers))
                .collect()
        };
        let mut out = Vec::with_capacity(modules.len());
        for (module, loaded) in modules {
            let servers = match loaded.driver.get().game(GameCall::Servers).await {
                Ok(GameAnswer::Servers(servers)) => Ok(servers),
                Ok(other) => Err(ModuleError::new(
                    ErrorKind::Protocol,
                    format!("{module} answered Servers with {other:?}"),
                )),
                Err(error) => Err(error),
            };
            out.push(GameServers {
                module,
                version: loaded.manifest.game_version.clone(),
                servers,
            });
        }
        out
    }
}

impl Restarts {
    fn reset(&mut self) {
        self.failures.clear();
        self.delay = None;
        self.generation += 1;
    }
}

impl Inner {
    /// A restartable module died or a restart of it failed: schedules the next try, or gives up after
    /// `max_failures` inside `window` with the last reason in its detail.
    fn failed(self: &Arc<Self>, id: &ModuleId, why: String) {
        let policy = self.options.restart;
        let (delay, generation, attempt) = {
            let mut state = lock(&self.state);
            let Some(slot) = state.slots.get_mut(id).filter(|slot| slot.restart) else {
                return;
            };
            let now = Instant::now();
            let restarts = &mut slot.restarts;
            if restarts
                .up_since
                .take()
                .is_some_and(|since| now.duration_since(since) >= policy.window)
            {
                restarts.delay = None;
            }
            restarts.failures.retain(|at| now.duration_since(*at) < policy.window);
            restarts.failures.push_back(now);
            let attempt = restarts.failures.len();
            if attempt >= policy.max_failures {
                let detail = format!(
                    "{why} (not restarted: {attempt} failures in {} s)",
                    policy.window.as_secs()
                );
                tracing::warn!(target: "host", "module {id}: {detail}");
                slot.detail = Some(detail.clone());
                slot.status.failed(detail);
                return;
            }
            let delay = restarts
                .delay
                .map_or(policy.first_delay, |last| (last * 2).min(policy.max_delay));
            restarts.delay = Some(delay);
            (delay, restarts.generation, attempt)
        };
        let weak = Arc::downgrade(self);
        let id = id.clone();
        dainty_service_kit::task::spawn_named(&format!("{id} restart"), async move {
            tokio::time::sleep(delay).await;
            let Some(inner) = weak.upgrade() else {
                return;
            };
            let current = lock(&inner.state)
                .slots
                .get(&id)
                .is_some_and(|slot| slot.state == ModuleState::Failed && slot.restarts.generation == generation);
            if !current {
                return;
            }
            tracing::info!(target: "host", "module {id}: restarting (attempt {attempt}) after: {why}");
            if let Ok(name) = EventName::new(MODULE_RESTARTED_EVENT) {
                let payload = serde_json::json!({ "module": id.as_str(), "attempt": attempt, "reason": why });
                inner.publish(None, Event { name, payload });
            }
            // A failed try comes back through `failed` from `load_as`
            _ = ModuleRegistry { inner }.load_as(&id, Some(generation)).await;
        });
    }

    fn set_state(&self, id: &ModuleId, state: ModuleState, detail: Option<String>) {
        let mut guard = lock(&self.state);
        let Some(slot) = guard.slots.get_mut(id) else {
            return;
        };
        slot.state = state;
        slot.detail.clone_from(&detail);
        if state == ModuleState::Ready {
            slot.restarts.up_since = Some(Instant::now());
        }
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
        dainty_service_kit::task::spawn_named("module watch", async move {
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
                    inner.set_state(&id, ModuleState::Failed, Some(why.clone()));
                    inner.failed(&id, why);
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
                if dainty_plugin_host::is_library(&external.path) {
                    Driver::Library(
                        dainty_plugin_host::DylibModule::load(&external, self.options.node.clone(), link, options)
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
                    "settings.toml has [modules.{id}.{section}], a section it does not read"
                )));
            };
            for (key, value) in values {
                if !registered.keys.iter().any(|k| k.name == *key) {
                    return Err(refused(format!(
                        "settings.toml sets {key} in [modules.{id}.{section}], which it does not read"
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
            dainty_service_kit::task::spawn_named(&format!("{id} event"), async move {
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
