//! The Rust services this binary was built with, as modules: each reads the env file the host kit
//! wrote for it (`hostconfig.rs`), through its own `Config::from_env`, so a service in this process
//! sees exactly what it saw as a process of its own.

#[cfg(any(feature = "gateway", feature = "allowlist", feature = "content", feature = "admin"))]
use std::sync::Arc;

#[cfg(any(feature = "gateway", feature = "allowlist", feature = "content", feature = "admin"))]
use service_kit::env::Env;

#[cfg(any(feature = "gateway", feature = "allowlist", feature = "content", feature = "admin"))]
use crate::Module;

/// The modules built into this binary, for the message an empty build prints.
pub const BUILT_IN: &[&str] = &[
    #[cfg(feature = "gateway")]
    "gateway",
    #[cfg(feature = "allowlist")]
    "allowlist",
    #[cfg(feature = "content")]
    "content",
    #[cfg(feature = "admin")]
    "admin",
];

/// Every module the crate can build, for the same message.
pub const ALL: &[&str] = &["gateway", "allowlist", "content", "admin"];

/// The game modules and plugins compiled into this binary (DD3), each by the id its manifest gives.
/// None yet: DF5's bins add one Cargo feature per game module here.
pub fn game_modules() -> Vec<crate::registry::ModuleFactory> {
    Vec::new()
}

/// Where the allowlist module leaves its helper for the gateway module's in-process feed.
#[cfg(feature = "allowlist")]
pub type AllowlistSlot = Arc<std::sync::Mutex<Option<Arc<gateway::allowlist::helper::Helper>>>>;

#[cfg(feature = "allowlist")]
pub fn allowlist_slot() -> AllowlistSlot {
    Arc::new(std::sync::Mutex::new(None))
}

/// The allowlist helper (`allowlist.env`). Started before the Director in public mode, so the game
/// ports are closed before any game server listens (what `--close-ports` did for a process).
#[cfg(feature = "allowlist")]
pub fn allowlist(env: &Env, slot: AllowlistSlot) -> Result<Module, String> {
    use gateway::allowlist::config::AllowlistConfig;
    let config = AllowlistConfig::from_env(env)?;
    Ok(Module::new(
        "allowlist",
        Arc::new(move |ctx| {
            let (config, slot) = (config.clone(), slot.clone());
            Box::pin(async move { gateway::allowlist::helper::run_with_context(config, ctx, Some(slot)).await })
        }),
    )
    .ready_within(std::time::Duration::from_secs(60)))
}

/// The gateway (`gateway.env`). With `slot`, logged-in players' addresses go to the allowlist
/// module directly; without it, over loopback HTTP as `GATEWAY_ALLOWLIST` says (an external helper).
#[cfg(feature = "gateway")]
pub fn gateway(
    env: &Env,
    slot: Option<AllowlistSlot>,
    registry: Option<crate::ModuleRegistry>,
) -> Result<Module, String> {
    use gateway::config::Config;
    let config = Config::from_env(env)?;
    let routes = registry
        .map(|registry| gateway_routes(&config, &registry))
        .transpose()?;
    let env = env.clone();
    Ok(Module::new(
        "gateway",
        Arc::new(move |ctx| {
            let (config, env, slot, routes) = (config.clone(), env.clone(), slot.clone(), routes.clone());
            Box::pin(async move {
                let capture = gateway::server::load_capture(&env)?;
                let sink: Option<gateway::feed::InProcessSink> = slot.map(|slot| {
                    Arc::new(move |ip: &str| {
                        let guard = slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                        guard.as_ref().is_some_and(|helper| helper.allow_ip(ip))
                    }) as gateway::feed::InProcessSink
                });
                gateway::server::run_with_routes(config, capture, sink, ctx, routes).await
            })
        }),
    )
    .ready_within(std::time::Duration::from_secs(60)))
}

#[cfg(all(feature = "gateway", not(feature = "allowlist")))]
pub type AllowlistSlot = Arc<std::sync::Mutex<Option<Arc<gateway::allowlist::helper::Helper>>>>;

/// The content server (`content.env`).
#[cfg(feature = "content")]
pub fn content(env: &Env, registry: Option<crate::ModuleRegistry>) -> Result<Module, String> {
    let config = content::config::Config::from_env(env)?;
    let modules = Arc::new(match &registry {
        Some(registry) => content::modules::ContentModules::new(Arc::new(ContentView(registry.downgrade()))),
        None => content::modules::ContentModules::default(),
    });
    if let Some(registry) = &registry {
        registry.add_plugin(modules.clone());
    }
    content_module(config, modules)
}

#[cfg(feature = "content")]
fn content_module(
    config: content::config::Config,
    modules: Arc<content::modules::ContentModules>,
) -> Result<Module, String> {
    Ok(Module::new(
        "content",
        Arc::new(move |ctx| {
            let (config, modules) = (config.clone(), modules.clone());
            Box::pin(async move { content::server::run_with_modules(config, ctx, modules).await })
        }),
    )
    .optional()
    .ready_within(std::time::Duration::from_secs(60)))
}

/// The admin API (`admin.env`), with the server's module registry when it has one (DF3).
#[cfg(feature = "admin")]
pub fn admin(env: &Env, registry: Option<crate::ModuleRegistry>) -> Result<Module, String> {
    let config = admin::config::Config::from_env(env)?;
    let (plugin, control) = match registry {
        Some(registry) => {
            let plugin = Arc::new(admin::modules::AdminPlugin::new(Arc::new(AdminRoute(
                registry.downgrade(),
            ))));
            registry.add_plugin(plugin.clone());
            let control = admin::module_control::Modules(Some(
                Arc::new(AdminModules(registry)) as Arc<dyn admin::module_control::ModuleControl>
            ));
            (plugin, control)
        }
        None => (
            Arc::new(admin::modules::AdminPlugin::none()),
            admin::module_control::Modules::default(),
        ),
    };
    Ok(Module::new(
        "admin",
        Arc::new(move |ctx| {
            let (config, plugin, control) = (config.clone(), plugin.clone(), control.clone());
            Box::pin(async move { admin::server::run_with_control(config, ctx, plugin, control).await })
        }),
    )
    .optional()
    .ready_within(std::time::Duration::from_secs(60)))
}

/// The registry as the admin API reads and drives it.
#[cfg(feature = "admin")]
struct AdminModules(crate::ModuleRegistry);

#[cfg(feature = "admin")]
fn admin_module(info: crate::ModuleInfo) -> admin::module_control::Module {
    let text = |value: serde_json::Value| value.as_str().map(str::to_owned).unwrap_or_default();
    let manifest = info.manifest.as_ref();
    admin::module_control::Module {
        id: info.id.to_string(),
        state: text(serde_json::to_value(info.state).unwrap_or_default()),
        source: info.source.to_owned(),
        autoload: info.autoload,
        display_name: manifest.map(|m| m.display_name.clone()),
        game_version: manifest.map(|m| m.game_version.to_string()),
        kind: manifest.map(|m| text(serde_json::to_value(m.kind).unwrap_or_default())),
        unverified: manifest.is_some_and(|m| m.unverified),
        detail: info.detail,
        ports: info
            .ports
            .into_iter()
            .map(|(name, range)| admin::module_control::ModulePort {
                name,
                first: range.first,
                last: range.last,
            })
            .collect(),
        commands: info.commands.into_iter().map(|c| c.name).collect(),
    }
}

#[cfg(feature = "admin")]
impl admin::module_control::ModuleControl for AdminModules {
    fn list(&self) -> Vec<admin::module_control::Module> {
        self.0.list().into_iter().map(admin_module).collect()
    }

    fn get(&self, id: &str) -> Result<admin::module_control::Module, String> {
        let id = id.parse().map_err(|e: module_api::ids::NameError| e.to_string())?;
        self.0.info(&id).map(admin_module).map_err(|e| e.to_string())
    }

    fn load<'a>(&'a self, id: &'a str) -> admin::module_control::Pending<'a, admin::module_control::Module> {
        Box::pin(async move {
            let id = id.parse().map_err(|e: module_api::ids::NameError| e.to_string())?;
            self.0.load(&id).await.map(admin_module).map_err(|e| e.to_string())
        })
    }

    fn unload<'a>(&'a self, id: &'a str) -> admin::module_control::Pending<'a, admin::module_control::Module> {
        Box::pin(async move {
            let id = id.parse().map_err(|e: module_api::ids::NameError| e.to_string())?;
            self.0.unload(&id).await.map_err(|e| e.to_string())?;
            self.0.info(&id).map(admin_module).map_err(|e| e.to_string())
        })
    }

    fn restart<'a>(&'a self, id: &'a str) -> admin::module_control::Pending<'a, admin::module_control::Module> {
        Box::pin(async move {
            let id = id.parse().map_err(|e: module_api::ids::NameError| e.to_string())?;
            self.0.restart(&id).await.map(admin_module).map_err(|e| e.to_string())
        })
    }
}

/// The modules' calls, as the admin API makes them (DF4c): to the loaded module, through its driver.
#[cfg(feature = "admin")]
struct AdminRoute(crate::WeakRegistry);

#[cfg(feature = "admin")]
impl admin::modules::AdminCalls for AdminRoute {
    fn admin<'a>(
        &'a self,
        module: &'a module_api::ModuleId,
        call: module_api::AdminCall,
    ) -> module_api::BoxFuture<'a, Result<module_api::AdminResponse, module_api::ModuleError>> {
        Box::pin(async move {
            let Some(registry) = self.0.upgrade() else {
                return Err(module_api::ModuleError::new(
                    module_api::ErrorKind::Unavailable,
                    "the server is stopping",
                ));
            };
            registry.admin_for(module, call).await
        })
    }
}

/// Where a module's files are, for content (DF4b): the folder its executable is in.
#[cfg(feature = "content")]
struct ContentView(crate::WeakRegistry);

#[cfg(feature = "content")]
impl content::modules::RegistryView for ContentView {
    fn module_dir(&self, module: &module_api::ModuleId) -> Option<std::path::PathBuf> {
        self.0.upgrade()?.module_dir(module)
    }
}

/// Where a module listens, for the gateway (DF4a): the port the controller gave it, on loopback. The
/// server's own backend, which no module serves yet, stays at the address `GATEWAY_BACKEND_URL` gives.
#[cfg(feature = "gateway")]
struct GatewayView {
    registry: crate::WeakRegistry,
    own: (module_api::ModuleId, &'static str, gateway::config::Endpoint),
}

#[cfg(feature = "gateway")]
impl gateway::modules::RegistryView for GatewayView {
    fn endpoint(&self, module: &module_api::ModuleManifest, port: &str) -> Option<gateway::config::Endpoint> {
        if module.id == self.own.0 {
            return (port == self.own.1).then(|| self.own.2.clone());
        }
        let range = self.registry.upgrade()?.port(&module.id, port)?;
        Some(gateway::config::Endpoint {
            host: "127.0.0.1".into(),
            port: range.first,
        })
    }
}

/// The gateway's module routes: the stand-in for the game this gateway fronts (the Node backend, until a
/// module replaces it), and every module the registry loads from now on.
#[cfg(feature = "gateway")]
fn gateway_routes(
    config: &gateway::config::Config,
    registry: &crate::ModuleRegistry,
) -> Result<Arc<gateway::modules::ModuleRoutes>, String> {
    let (manifest, registration) = gateway::modules::stand_in(&config.version)?;
    let port = if config.version == protocol::invite::revived_version() {
        "metagame"
    } else {
        "backend"
    };
    let view = GatewayView {
        registry: registry.downgrade(),
        own: (manifest.id.clone(), port, config.backend.clone()),
    };
    let routes = Arc::new(gateway::modules::ModuleRoutes::new(Arc::new(view)));
    module_api::CorePlugin::attach(routes.as_ref(), &manifest, &registration).map_err(|e| e.to_string())?;
    registry.add_plugin(routes.clone());
    Ok(routes)
}
