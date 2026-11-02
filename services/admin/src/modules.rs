//! The admin API on the module registry (DF4c): kick, ban and lookup go to the module of the player's
//! game. The controller (DF3) attaches each module's registration to [`AdminPlugin`], a core plugin, and
//! carries the calls through [`AdminCalls`], the one thing it implements for the admin API.
//!
//! A player's game is found by asking every game that answers lookups (`locate` in `schema.rs`): a
//! registration names no accounts, and each game keeps its own until the network's directory (DD17).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use module_api::registrar::{AdminAction, Registration};
use module_api::{
    Actor, ActorContext, AdminCall, AdminRequest, AdminResponse, BoxFuture, CoreCapability, CorePlugin, ErrorKind,
    GameVersion, ModuleError, ModuleId, ModuleKind, ModuleManifest,
};

/// The id `REFORGED_ADMIN_API_KEY` acts under: it is the owner's key (DD23). Every other key acts under
/// its own id (`keys.rs`, DH2).
pub const OWNER_KEY: &str = "owner";

/// The game the Node backend serves. Its admin routes answer for it until a module for this version is
/// loaded (DD6), so today's 1.14.7 path is unchanged.
pub const NODE_GAME: &str = "1.14.7";

/// How long one module call may take, as long as one backend call (`upstream.rs`).
pub const CALL_TIMEOUT: Duration = Duration::from_secs(5);

/// A loaded game module that answers admin actions, as the admin API sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminModule {
    pub id: ModuleId,
    pub game_version: GameVersion,
    /// "Rustic Isles 1.14.7", "Dauntless Revived" (DD4)
    pub display_name: String,
    pub actions: BTreeSet<AdminAction>,
}

impl AdminModule {
    pub fn answers(&self, action: AdminAction) -> bool {
        self.actions.contains(&action)
    }
}

/// The controller's side (DF3): one admin call to one loaded module, through its driver
/// (`ModuleDriverExt::admin`), compiled in or out of process.
pub trait AdminCalls: Send + Sync {
    fn admin<'a>(&'a self, module: &'a ModuleId, call: AdminCall) -> BoxFuture<'a, Result<AdminResponse, ModuleError>>;
}

/// No modules loaded: `admin serve` on its own, and every server before DF3's controller runs modules.
struct NoCalls;

impl AdminCalls for NoCalls {
    fn admin<'a>(
        &'a self,
        module: &'a ModuleId,
        _call: AdminCall,
    ) -> BoxFuture<'a, Result<AdminResponse, ModuleError>> {
        Box::pin(async move {
            Err(ModuleError::new(
                ErrorKind::Unavailable,
                format!("{module} is not loaded"),
            ))
        })
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The admin API as a core plugin: which game modules answer which admin actions, kept as the controller
/// attaches and detaches them, and the calls to them.
pub struct AdminPlugin {
    attached: Mutex<BTreeMap<ModuleId, AdminModule>>,
    calls: Arc<dyn AdminCalls>,
    requests: AtomicU64,
}

impl AdminPlugin {
    pub fn new(calls: Arc<dyn AdminCalls>) -> Self {
        Self {
            attached: Mutex::new(BTreeMap::new()),
            calls,
            requests: AtomicU64::new(0),
        }
    }

    /// A plugin with nothing attached and nothing to call.
    pub fn none() -> Self {
        Self::new(Arc::new(NoCalls))
    }

    /// The attached modules, by id.
    pub fn modules(&self) -> Vec<AdminModule> {
        lock(&self.attached).values().cloned().collect()
    }

    /// Whether a loaded module serves `version`, which takes it over from the Node backend.
    pub fn has_game(&self, version: &str) -> bool {
        lock(&self.attached)
            .values()
            .any(|m| m.game_version.to_string() == version)
    }

    /// The actor every call carries: the key that asked (its id, never the key), and a request id the admin
    /// API logs beside the operator id, so a change in a module's log leads back to the request that made it
    /// and the person who asked (DH1, DH2).
    pub fn actor(&self, key: &str, now_ms: i64) -> ActorContext {
        let n = self.requests.fetch_add(1, Ordering::Relaxed);
        ActorContext::new(Actor::Admin { key: key.to_owned() }).with_request(format!("admin-{now_ms}-{n}"))
    }

    /// One call, refused before it is sent when the module didn't register the action, and given up
    /// after [`CALL_TIMEOUT`].
    pub async fn call(
        &self,
        module: &AdminModule,
        actor: ActorContext,
        request: AdminRequest,
    ) -> Result<AdminResponse, ModuleError> {
        let action = request.action();
        if !module.answers(action) {
            return Err(ModuleError::unsupported(format!("{action:?}")));
        }
        let call = AdminCall { actor, request };
        match tokio::time::timeout(CALL_TIMEOUT, self.calls.admin(&module.id, call)).await {
            Ok(answer) => answer,
            Err(_) => Err(ModuleError::new(ErrorKind::Timeout, "no answer in time")),
        }
    }
}

impl CorePlugin for AdminPlugin {
    fn name(&self) -> &'static str {
        "admin"
    }

    fn serves(&self) -> &'static [CoreCapability] {
        &[CoreCapability::AdminActions]
    }

    fn attach(&self, module: &ModuleManifest, registration: &Registration) -> Result<(), ModuleError> {
        if registration.admin_actions.is_empty() {
            return Ok(());
        }
        let refuse = |why: String| Err(ModuleError::new(ErrorKind::Invalid, format!("{}: {why}", module.id)));
        // A plugin has no players of its own (DD25)
        if module.kind != ModuleKind::Game {
            return refuse(String::from("only a game module registers admin actions"));
        }
        // Without it the admin API can't tell which players are this game's
        if !registration.admin_actions.contains(&AdminAction::Lookup) {
            return refuse(String::from("a game with admin actions must answer lookup"));
        }
        let mut attached = lock(&self.attached);
        if let Some(other) = attached
            .values()
            .find(|m| m.id == module.id || m.game_version == module.game_version)
        {
            return refuse(format!(
                "{} already answers admin actions for {}",
                other.id, other.game_version
            ));
        }
        attached.insert(
            module.id.clone(),
            AdminModule {
                id: module.id.clone(),
                game_version: module.game_version.clone(),
                display_name: module.display_name.clone(),
                actions: registration.admin_actions.iter().copied().collect(),
            },
        );
        Ok(())
    }

    fn detach(&self, module: &ModuleId) {
        lock(&self.attached).remove(module);
    }
}
