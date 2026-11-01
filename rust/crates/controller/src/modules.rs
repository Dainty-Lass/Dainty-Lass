// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The Rust services this binary was built with, as modules: each reads the env file the host kit
//! wrote for it (`hostconfig.rs`), through its own `Config::from_env`, so a service in this process
//! sees exactly what it saw as a process of its own.

#[cfg(any(
    feature = "gateway",
    feature = "allowlist",
    feature = "content",
    feature = "admin",
    feature = "chat"
))]
use std::sync::Arc;

#[cfg(any(
    feature = "gateway",
    feature = "allowlist",
    feature = "content",
    feature = "admin",
    feature = "chat"
))]
use service_kit::env::Env;

#[cfg(any(
    feature = "gateway",
    feature = "allowlist",
    feature = "content",
    feature = "admin",
    feature = "chat"
))]
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

/// The game modules compiled into this binary (DD3, DF5): one Cargo feature each, none by default. A server's
/// `[modules]` section picks which of them load; the rest stay listed and unloaded.
#[cfg_attr(
    not(any(feature = "stub-module", feature = "isles-1-14-7", feature = "isles-1-12-0")),
    allow(unused_mut)
)]
pub fn game_modules() -> Vec<crate::registry::ModuleFactory> {
    let mut modules: Vec<crate::registry::ModuleFactory> = Vec::new();
    #[cfg(feature = "stub-module")]
    modules.push(std::sync::Arc::new(|| {
        std::sync::Arc::new(module_api::stub::StubModule::new()) as std::sync::Arc<dyn module_api::GameModule>
    }));
    #[cfg(feature = "isles-1-14-7")]
    modules.push(std::sync::Arc::new(|| {
        std::sync::Arc::new(isles_1_14_7::module()) as std::sync::Arc<dyn module_api::GameModule>
    }));
    #[cfg(feature = "isles-1-12-0")]
    modules.push(std::sync::Arc::new(|| {
        std::sync::Arc::new(isles_1_12_0::module()) as std::sync::Arc<dyn module_api::GameModule>
    }));
    modules
}

/// The game-module features this build was asked for whose crate is not linked yet, so `game_modules` has
/// nothing for them. The host kit says so at start rather than let a release lack a module unnoticed.
pub const UNLINKED: &[&str] = &[
    #[cfg(feature = "revived-1-4-4")]
    "revived-1-4-4",
];

/// Where the allowlist module leaves its helper for the gateway module's in-process feed.
#[cfg(feature = "allowlist")]
pub type AllowlistSlot = Arc<std::sync::Mutex<Option<Arc<gateway::allowlist::helper::Helper>>>>;

#[cfg(feature = "allowlist")]
pub fn allowlist_slot() -> AllowlistSlot {
    Arc::new(std::sync::Mutex::new(None))
}

/// The allowlist helper (`allowlist.env`). Started before the Director in public mode, so the game
/// ports are closed before any game server listens (what `--close-ports` did for a process). With the
/// registry, it also guards the UDP ranges of the game modules that load (DF4d).
#[cfg(feature = "allowlist")]
pub fn allowlist(env: &Env, slot: AllowlistSlot, registry: Option<crate::ModuleRegistry>) -> Result<Module, String> {
    use gateway::allowlist::config::AllowlistConfig;
    use gateway::allowlist::guard::Guard;
    let config = AllowlistConfig::from_env(env)?;
    // Made here, not by the helper, so a restarted allowlist module keeps the ranges of the modules loaded
    let guard = Arc::new(Guard::new(&config.ports)?);
    if let Some(registry) = registry {
        let view = Arc::new(RegistryPorts(registry.downgrade()));
        registry.add_plugin(Arc::new(gateway::allowlist::modules::AllowlistPlugin::new(
            guard.clone(),
            view,
        )));
    }
    Ok(Module::new(
        "allowlist",
        Arc::new(move |ctx| {
            let (config, slot, guard) = (config.clone(), slot.clone(), guard.clone());
            Box::pin(async move { gateway::allowlist::helper::run_with_guard(config, ctx, Some(slot), guard).await })
        }),
    )
    .ready_within(std::time::Duration::from_secs(60)))
}

/// A public server whose allowlist this process does not run: a module that registers a UDP port is refused,
/// because nothing here can guard it (DF4d).
#[cfg(feature = "allowlist")]
pub fn allowlist_absent(registry: &crate::ModuleRegistry) {
    let view = Arc::new(RegistryPorts(registry.downgrade()));
    registry.add_plugin(Arc::new(gateway::allowlist::modules::AllowlistPlugin::absent(view)));
}

/// The ranges the controller placed, for the allowlist plugin.
#[cfg(feature = "allowlist")]
struct RegistryPorts(crate::WeakRegistry);

#[cfg(feature = "allowlist")]
impl gateway::allowlist::modules::PortView for RegistryPorts {
    fn assigned(&self, module: &module_api::ModuleId, port: &str) -> Option<module_api::PortRange> {
        self.0.upgrade()?.assigned(module, port)
    }
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
    #[allow(unused_mut)]
    let mut config = Config::from_env(env)?;
    #[cfg(feature = "chat")]
    if registry.is_some() && config.ws.port == config.backend.port {
        config.ws.port = chat::settings::DEFAULT_PORT;
    }
    let node_links = registry.as_ref().map(|r| r.node_services().gateway_slot());
    #[cfg(feature = "network")]
    let region_handler = registry.as_ref().map(|r| r.node_services().region_report_handler());
    #[cfg(not(feature = "network"))]
    let region_handler: Option<gateway::network::RegionReportHandler> = None;
    let bind_on_ready = registry
        .as_ref()
        .map(|r| (r.node_services().clone(), r.status_registry().clone()));
    let routes = registry
        .map(|registry| gateway_routes(&config, &registry))
        .transpose()?;
    let env = env.clone();
    Ok(Module::new(
        "gateway",
        Arc::new(move |ctx| {
            let (config, env, slot, routes, node_links, region_handler, bind_on_ready) = (
                config.clone(),
                env.clone(),
                slot.clone(),
                routes.clone(),
                node_links.clone(),
                region_handler.clone(),
                bind_on_ready.clone(),
            );
            Box::pin(async move {
                let capture = gateway::server::load_capture(&env)?;
                let sink: Option<gateway::feed::InProcessSink> = slot.map(|slot| {
                    Arc::new(move |ip: &str| {
                        let guard = slot.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                        guard.as_ref().is_some_and(|helper| helper.allow_ip(ip))
                    }) as gateway::feed::InProcessSink
                });
                if let Some((services, status)) = bind_on_ready {
                    let mut rx = status.subscribe();
                    tokio::spawn(async move {
                        while rx.changed().await.is_ok() {
                            let phase = rx.borrow().get("gateway").map(|c| c.phase);
                            if let Some(phase) = phase {
                                match phase {
                                    service_kit::context::ComponentPhase::Ready => {
                                        services.bind_gateway();
                                        break;
                                    }
                                    service_kit::context::ComponentPhase::Failed
                                    | service_kit::context::ComponentPhase::Stopped => break,
                                    _ => {}
                                }
                            }
                        }
                    });
                }
                gateway::server::run_with_network(config, capture, sink, ctx, routes, node_links, region_handler).await
            })
        }),
    )
    .ready_within(std::time::Duration::from_secs(60)))
}

/// In-process [`network_plugin::ring::AllowlistSink`] backed by the allowlist helper module (`AllowlistSlot`, DF4d, DN8, DN9).
#[cfg(feature = "network")]
pub struct InProcessAllowlistSink {
    slot: Option<AllowlistSlot>,
}

#[cfg(feature = "network")]
impl InProcessAllowlistSink {
    pub fn new(slot: Option<AllowlistSlot>) -> Self {
        Self { slot }
    }
}

#[cfg(feature = "network")]
impl network_plugin::ring::AllowlistSink for InProcessAllowlistSink {
    fn admit_player(
        &self,
        instance: &module_api::network::GlobalId,
        _account: &module_api::network::GlobalId,
        client_ip: &str,
        udp_port: u16,
    ) {
        let admitted = self
            .slot
            .as_ref()
            .and_then(|slot| {
                slot.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .as_ref()
                    .cloned()
            })
            .is_some_and(|helper| helper.admit_udp(client_ip, udp_port));
        if !admitted {
            tracing::warn!(
                target: "host",
                instance = %instance,
                udp_port,
                "allowlist helper refused player UDP admission; game port stays closed"
            );
        }
    }
}

/// Configuration for the `"network"` module (DD2, DN3, DN5, DH7).
#[cfg(feature = "network")]
pub struct NetworkOptions {
    pub node: module_api::NodeIdentity,
    pub leader: module_api::network::NodeId,
    pub peers: Vec<network_plugin::cluster::Peer>,
    pub credentials: network_plugin::cluster::NodeCredentials,
    pub signing_key: network_plugin::home::SigningKey,
    pub database: store::DatabaseConfig,
    pub game: module_api::GameVersion,
    pub registry: crate::ModuleRegistry,
    pub allowlist: Option<AllowlistSlot>,
    pub replication_interval: std::time::Duration,
    /// Folder where this node stores log segments and snapshots shipped from peers (`Service::LogShip`, DH7).
    pub peer_log_dir: Option<std::path::PathBuf>,
    /// Peer node to ship this node's change log and snapshots to (`[history] ship_to_peer`, DH7), and its state file.
    pub ship_to_peer: Option<(module_api::network::NodeId, std::path::PathBuf)>,
    /// Policy when a player joins an instance hosted on a different node (`[network] wrong_node_policy`, DN8).
    pub wrong_node_policy: Option<network_plugin::ring::WrongNodePolicy>,
}

#[cfg(feature = "network")]
struct HomeRouterHook {
    router: Arc<network_plugin::home::HomeRouter>,
    director: Arc<network_plugin::ring::DriverDirectorRouter>,
}

#[cfg(feature = "network")]
impl crate::registry::LoadHook for HomeRouterHook {
    fn loaded(&self, manifest: &module_api::ModuleManifest, driver: Arc<dyn module_api::ModuleDriver>) {
        if manifest.kind == module_api::ModuleKind::Game {
            self.router.register(
                manifest.id.clone(),
                Arc::new(network_plugin::home::DriverBackend::new(driver.clone())),
            );
            self.director.register(driver);
        }
    }

    fn unloaded(&self, module: &module_api::ModuleId) {
        self.router.unregister(module);
        self.director.unregister(module);
    }
}

/// The node's network runtime (`[network]` in `host.toml` when peers are configured): opens the
/// directory table in the node database, connects [`network_plugin::TlsPeerLink`] to peers, starts
/// directory replication and peer health checks, wires [`network_plugin::home::NetworkedHome`] into
/// the [`crate::ModuleRegistry`], and attaches [`network_plugin::Service::Directory`] and
/// [`network_plugin::Service::Home`] to [`crate::NodeServices`].
#[cfg(feature = "network")]
pub fn network(options: NetworkOptions) -> Result<Module, String> {
    let link = Arc::new(
        network_plugin::TlsPeerLink::new(network_plugin::TlsLinkConfig::new(
            options.credentials,
            options.peers.clone(),
        ))
        .map_err(|e| format!("network peer link: {e}"))?,
    );
    let log_ship_key = network_plugin::logship::LogShipKey::from_signing_key(&options.signing_key);
    let tickets = Arc::new(network_plugin::home::Tickets::new(
        &options.node,
        options.signing_key,
        Arc::new(network_plugin::Clock::default()),
        network_plugin::home::TicketConfig::default(),
    ));
    let router = Arc::new(network_plugin::home::HomeRouter::default());
    let director = Arc::new(network_plugin::ring::DriverDirectorRouter::default());
    options.registry.add_hook(Arc::new(HomeRouterHook {
        router: router.clone(),
        director: director.clone(),
    }));
    let node = options.node;
    let leader = options.leader;
    let peers = options.peers;
    let database = options.database;
    let game = options.game;
    let registry = options.registry;
    let allowlist = options.allowlist;
    let replication_interval = options.replication_interval;
    let peer_log_dir = options.peer_log_dir;
    let ship_to_peer = options.ship_to_peer;
    let wrong_node_policy = options.wrong_node_policy;
    Ok(Module::new(
        "network",
        Arc::new(move |mut ctx| {
            let (
                node,
                leader,
                peers,
                database,
                game,
                registry,
                allowlist,
                link,
                tickets,
                log_ship_key,
                router,
                director,
                peer_log_dir,
                ship_to_peer,
                wrong_node_policy,
            ) = (
                node.clone(),
                leader.clone(),
                peers.clone(),
                database.clone(),
                game.clone(),
                registry.clone(),
                allowlist.clone(),
                link.clone(),
                tickets.clone(),
                log_ship_key.clone(),
                router.clone(),
                director.clone(),
                peer_log_dir.clone(),
                ship_to_peer.clone(),
                wrong_node_policy,
            );
            Box::pin(async move {
                if let store::DatabaseConfig::Sqlite { file } = &database
                    && let Some(parent) = file.parent()
                {
                    let _ = std::fs::create_dir_all(parent);
                }
                let db = store::Database::open(
                    &database,
                    &store::DatabaseOptions::default(),
                    &network_plugin::directory::SCHEMA,
                )
                .map_err(|e| format!("network directory database: {e}"))?;
                tickets
                    .attach_database(db.clone())
                    .await
                    .map_err(|e| format!("network ticket revocations: {e}"))?;
                let _health_task = link.spawn_health_checks();
                let dir_config = network_plugin::directory::DirectoryConfig::in_network(node.node.clone(), leader);
                let directory = Arc::new(
                    network_plugin::directory::Directory::new(db.clone(), dir_config, link.clone())
                        .await
                        .map_err(|e| format!("network directory: {e}"))?,
                );
                let replication_task = directory.spawn_replication(replication_interval);
                let home_service = Arc::new(network_plugin::home::HomeService::new(
                    node.clone(),
                    tickets.clone(),
                    router.clone(),
                    directory.clone(),
                    peers.iter().map(|p| p.id.clone()),
                    network_plugin::home::HomeConfig::default(),
                ));
                let networked_home = Arc::new(
                    network_plugin::home::NetworkedHome::new(
                        node.clone(),
                        peers.iter().map(|p| (p.id.clone(), p.region.clone())),
                        link.clone(),
                        directory.clone(),
                        tickets.clone(),
                        network_plugin::home::NetworkedConfig::new(game),
                    )
                    .with_local_router(router)
                    .with_directory(directory.clone()),
                );
                let mut ring_config = network_plugin::ring::RingConfig::new(node.clone());
                ring_config.wrong_node_policy = wrong_node_policy
                    .or_else(|| {
                        match std::env::var("REFORGED_WRONG_NODE_POLICY")
                            .ok()
                            .as_deref()
                            .map(str::trim)
                        {
                            Some("redirect") => Some(network_plugin::ring::WrongNodePolicy::Redirect),
                            Some("refuse") => Some(network_plugin::ring::WrongNodePolicy::Refuse),
                            _ => None,
                        }
                    })
                    .unwrap_or_default();
                ring_config.peers = peers
                    .iter()
                    .map(|p| network_plugin::ring::PeerGateway {
                        node: p.id.clone(),
                        region: p.region.clone(),
                        host: p.host.clone(),
                        port: p.port,
                        fingerprint: p.fingerprint.clone(),
                    })
                    .collect();
                let ring_node = network_plugin::ring::RingNode::new(
                    ring_config,
                    Arc::new(network_plugin::Clock::default()),
                    link.clone(),
                    networked_home.clone(),
                    director,
                    Arc::new(InProcessAllowlistSink::new(allowlist)),
                );
                networked_home.set_ring(&ring_node);
                let ring_ticker = ring_node.spawn_ticker(
                    replication_interval
                        .min(std::time::Duration::from_secs(1))
                        .max(std::time::Duration::from_millis(50)),
                );
                let services = registry.node_services();
                services.set_peer_link(Some(link.clone()));
                services.set_peer_roster(Some(link.clone()));
                services.set_directory(Some(directory.clone()));
                services.set_tickets(Some(tickets));
                services.set_ring(Some(ring_node.clone()));
                services.attach(network_plugin::Service::Directory, directory);
                services.attach(network_plugin::Service::Home, home_service);
                services.attach(network_plugin::Service::Ring, ring_node);
                if let Some(dir) = peer_log_dir {
                    let store = Arc::new(network_plugin::logship::PeerLogStore::new(
                        dir,
                        peers.iter().map(|p| p.id.clone()),
                        Arc::new(network_plugin::Clock::default()),
                    ));
                    services.attach(network_plugin::Service::LogShip, store);
                }
                let ship_task = if let Some((target_peer, state_file)) = ship_to_peer {
                    if let Some(parent) = state_file.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let interval = replication_interval
                        .min(std::time::Duration::from_secs(10))
                        .max(std::time::Duration::from_millis(50));
                    let mut settings = history_ship::ShipSettings::new(
                        network_plugin::logship::PeerBucket::prefix_for(&node.node),
                        state_file,
                    );
                    settings.interval = interval;
                    settings.segment_bytes = 512 * 1024;
                    let bucket =
                        network_plugin::logship::PeerBucket::with_key(target_peer.clone(), link.clone(), log_ship_key);
                    let mut shipper = history_ship::Shipper::new(db, bucket, settings)
                        .map_err(|e| format!("network peer log shipper: {e}"))?;
                    Some(tokio::spawn(async move {
                        let mut every = tokio::time::interval(interval);
                        every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                        loop {
                            every.tick().await;
                            if let Err(e) = shipper.tick().await {
                                tracing::warn!(peer = %target_peer, error = %e, "peer log shipping tick failed");
                            }
                        }
                    }))
                } else {
                    None
                };
                registry.set_home(networked_home);
                ctx.status
                    .ready(None, Some(format!("{} ({} peers)", node.node, peers.len())));
                ctx.wait_stop().await;
                ctx.status.stopping();
                services.detach(network_plugin::Service::LogShip);
                services.detach(network_plugin::Service::Ring);
                services.detach(network_plugin::Service::Directory);
                services.detach(network_plugin::Service::Home);
                services.set_ring(None);
                services.set_tickets(None);
                services.set_directory(None);
                services.set_peer_roster(None);
                services.set_peer_link(None);
                registry.reset_home();
                ring_ticker.abort();
                if let Some(task) = ship_task {
                    task.abort();
                }
                if let Some(task) = replication_task {
                    task.abort();
                }
                ctx.status.stopped();
                Ok(())
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
    let (plugin, control, services) = match registry {
        Some(registry) => {
            let plugin = Arc::new(admin::modules::AdminPlugin::new(Arc::new(AdminRoute(
                registry.downgrade(),
            ))));
            registry.add_plugin(plugin.clone());
            let services = registry.node_services().clone();
            let control = admin::module_control::Modules(Some(
                Arc::new(AdminModules(registry)) as Arc<dyn admin::module_control::ModuleControl>
            ));
            (plugin, control, Some(services))
        }
        None => (
            Arc::new(admin::modules::AdminPlugin::none()),
            admin::module_control::Modules::default(),
            None,
        ),
    };
    Ok(Module::new(
        "admin",
        Arc::new(move |ctx| {
            let (config, plugin, control, services) =
                (config.clone(), plugin.clone(), control.clone(), services.clone());
            Box::pin(async move {
                let network: Arc<dyn admin::relay::AdminNetwork> = match &services {
                    Some(services) => Arc::new(NodeServicesNetwork(services.clone())),
                    None => Arc::new(admin::relay::NoNetwork),
                };
                let attach_services = services.clone();
                let on_ready = attach_services.map(|services| {
                    Box::new(move |admin: Arc<admin::schema::Admin>| {
                        services.attach(network_plugin::Service::Admin, admin.relay_service());
                    }) as Box<dyn FnOnce(Arc<admin::schema::Admin>) + Send>
                });
                let result = admin::server::run_with_network(config, ctx, plugin, control, network, on_ready).await;
                if let Some(services) = services {
                    services.detach(network_plugin::Service::Admin);
                }
                result
            })
        }),
    )
    .optional()
    .ready_within(std::time::Duration::from_secs(60)))
}

#[cfg(feature = "admin")]
struct NodeServicesNetwork(crate::NodeServices);

#[cfg(feature = "admin")]
impl admin::relay::AdminNetwork for NodeServicesNetwork {
    fn local_node(&self) -> Option<module_api::network::NodeId> {
        Some(self.0.node().clone())
    }

    fn peer_link(&self) -> Option<Arc<dyn network_plugin::PeerLink>> {
        self.0.peer_link()
    }

    fn peer_roster(&self) -> Option<Arc<dyn network_plugin::PeerRoster>> {
        self.0.peer_roster()
    }

    fn directory(&self) -> Option<Arc<network_plugin::directory::Directory>> {
        #[cfg(feature = "network")]
        {
            self.0.directory()
        }
        #[cfg(not(feature = "network"))]
        {
            None
        }
    }

    fn tickets(&self) -> Option<Arc<network_plugin::home::Tickets>> {
        #[cfg(feature = "network")]
        {
            self.0.tickets()
        }
        #[cfg(not(feature = "network"))]
        {
            None
        }
    }
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
        if let Some(range) = self.registry.upgrade().and_then(|r| r.port(&module.id, port)) {
            return Some(gateway::config::Endpoint {
                host: "127.0.0.1".into(),
                port: range.first,
            });
        }
        if module.id == self.own.0 {
            return (port == self.own.1).then(|| self.own.2.clone());
        }
        None
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
    routes
        .attach_stand_in(&manifest, &registration)
        .map_err(|e| e.to_string())?;
    registry.add_plugin(routes.clone());
    Ok(routes)
}

#[cfg(feature = "remote")]
pub struct RemoteControllerOptions {
    pub state: std::sync::Arc<std::sync::Mutex<remote::RemoteControllerState>>,
    pub state_file: Option<std::path::PathBuf>,
    pub ca: std::sync::Arc<network_plugin::cluster::ClusterCa>,
    pub services: crate::NodeServices,
}

/// Controller-side remote management module (`[remote] role = "controller"`, DD27, RM2, RM3):
/// attaches `Service::Remote` and `POST /__node/enroll` to [`crate::NodeServices`] and syncs
/// enrolled peers to the gateway's `NodeEndpoint`.
#[cfg(feature = "remote")]
pub fn remote_controller(options: RemoteControllerOptions) -> Result<crate::Module, String> {
    let state = options.state;
    let state_file = options.state_file;
    let ca = options.ca;
    let services = options.services;
    Ok(crate::Module::new(
        "remote",
        std::sync::Arc::new(move |mut ctx| {
            let (state, state_file, ca, services) = (state.clone(), state_file.clone(), ca.clone(), services.clone());
            Box::pin(async move {
                let enrolled_nodes: Vec<module_api::network::NodeId> = {
                    let guard = state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    guard
                        .installs
                        .values()
                        .filter(|rec| !rec.revoked)
                        .map(|rec| rec.node.clone())
                        .collect()
                };
                for node in enrolled_nodes {
                    services.add_dynamic_peer(node);
                }

                let enroll_state = state.clone();
                let enroll_file = state_file.clone();
                let enroll_ca = ca.clone();
                let enroll_services = services.clone();
                let enroll_cb: crate::node_services::RemoteEnrollCallback =
                    std::sync::Arc::new(move |body: Vec<u8>| {
                        let enroll_state = enroll_state.clone();
                        let enroll_file = enroll_file.clone();
                        let enroll_ca = enroll_ca.clone();
                        let enroll_services = enroll_services.clone();
                        Box::pin(async move {
                            let req = match remote::decode_enroll_request(&body) {
                                Ok(r) => r,
                                Err(e) => {
                                    let err = serde_json::json!({ "error": e.to_string() });
                                    return (400, err.to_string().into_bytes());
                                }
                            };
                            let now_ms = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_millis() as i64)
                                .unwrap_or(0);
                            let answer_res = {
                                let mut g = enroll_state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                                if let Some(path) = &enroll_file
                                    && path.is_file()
                                    && let Ok(fresh) = remote::RemoteControllerState::load(path)
                                {
                                    *g = fresh;
                                }
                                let res = g.redeem_enrollment(&enroll_ca, &req, now_ms);
                                if res.is_ok()
                                    && let Some(path) = &enroll_file
                                {
                                    let _ = g.save(path);
                                }
                                res
                            };
                            match answer_res {
                                Ok(ans) => {
                                    enroll_services.add_dynamic_peer(req.node);
                                    match remote::encode_enroll_answer(&ans) {
                                        Ok(bytes) => (200, bytes),
                                        Err(e) => {
                                            let err = serde_json::json!({ "error": e.to_string() });
                                            (500, err.to_string().into_bytes())
                                        }
                                    }
                                }
                                Err(e) => {
                                    let status = match e {
                                        remote::RemoteError::InvalidEnrollmentToken
                                        | remote::RemoteError::EnrollmentTokenUsed
                                        | remote::RemoteError::EnrollmentTokenExpired => 403,
                                        _ => 400,
                                    };
                                    let err = serde_json::json!({ "error": e.to_string() });
                                    (status, err.to_string().into_bytes())
                                }
                            }
                        })
                    });

                services.set_enroll_handler(Some(enroll_cb));
                let svc = std::sync::Arc::new(remote::RemoteControllerService::new(state, state_file));
                services.attach(network_plugin::Service::Remote, svc);
                ctx.status.ready(None, Some("controller".to_string()));
                ctx.wait_stop().await;
                ctx.status.stopping();
                services.detach(network_plugin::Service::Remote);
                services.set_enroll_handler(None);
                ctx.status.stopped();
                Ok(())
            })
        }),
    )
    .optional()
    .ready_within(std::time::Duration::from_secs(30)))
}

#[cfg(feature = "remote")]
pub struct RemoteManagedOptions {
    pub agent: std::sync::Arc<std::sync::Mutex<remote::RemoteManagedAgent>>,
    pub handler: std::sync::Arc<dyn remote::RemoteHostHandler>,
    pub link: std::sync::Arc<dyn network_plugin::PeerLink>,
    pub services: crate::NodeServices,
    pub sync_interval: std::time::Duration,
}

/// Managed-install remote management module (`[remote] role = "managed"`, DD27, RM2, RM3):
/// attaches `Service::Remote` to [`crate::NodeServices`] for direct controller pushes and runs the
/// outbound heartbeat sync loop with exponential backoff when the controller is unreachable.
#[cfg(feature = "remote")]
pub fn remote_managed(options: RemoteManagedOptions) -> Result<crate::Module, String> {
    let agent = options.agent;
    let handler = options.handler;
    let link = options.link;
    let services = options.services;
    let sync_interval = options.sync_interval.max(std::time::Duration::from_secs(1));
    Ok(crate::Module::new(
        "remote",
        std::sync::Arc::new(move |mut ctx| {
            let (agent, handler, link, services) = (agent.clone(), handler.clone(), link.clone(), services.clone());
            Box::pin(async move {
                let controller_node = {
                    agent
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .controller_node
                        .clone()
                };
                services.add_dynamic_peer(controller_node.clone());
                let svc = std::sync::Arc::new(remote::RemoteManagedService::new(agent.clone(), handler.clone()));
                services.attach(network_plugin::Service::Remote, svc);
                let pending_outcomes = std::sync::Arc::new(std::sync::Mutex::new(Vec::<remote::CommandOutcome>::new()));
                let loop_agent = agent.clone();
                let loop_handler = handler.clone();
                let loop_link = link.clone();
                let loop_outcomes = pending_outcomes.clone();
                let sync_task = tokio::spawn(async move {
                    loop {
                        let res = remote::sync_once_with_controller(
                            loop_link.as_ref(),
                            &loop_agent,
                            loop_handler.as_ref(),
                            &loop_outcomes,
                        )
                        .await;
                        let wait_ms = match res {
                            Ok(_) => sync_interval.as_millis() as u64,
                            Err(remote::RemoteError::RevokedPeer(_)) => break,
                            Err(e) => {
                                let failures = loop_agent
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                                    .consecutive_unreachable;
                                if failures == 1 {
                                    tracing::warn!(
                                        error = %e,
                                        "controller unreachable; continuing on cached configuration"
                                    );
                                }
                                remote::offline_backoff_delay_ms(failures)
                            }
                        };
                        tokio::time::sleep(std::time::Duration::from_millis(wait_ms)).await;
                    }
                });
                ctx.status.ready(None, Some(format!("managed by {controller_node}")));
                ctx.wait_stop().await;
                ctx.status.stopping();
                sync_task.abort();
                services.detach(network_plugin::Service::Remote);
                ctx.status.stopped();
                Ok(())
            })
        }),
    )
    .optional()
    .ready_within(std::time::Duration::from_secs(30)))
}

/// Network peer configuration for [`chat`] when running in a multi-node cluster (DN10b).
#[cfg(feature = "chat")]
#[derive(Clone)]
pub struct ChatNetworkOptions {
    pub leader: module_api::network::NodeId,
    pub peers: Vec<(module_api::network::NodeId, module_api::network::Region)>,
}

/// Configuration for the `"chat"` module (DD21, DN10a, DN10b).
#[cfg(feature = "chat")]
pub struct ChatOptions {
    pub env: Env,
    pub node: module_api::NodeIdentity,
    pub database: store::DatabaseConfig,
    pub registry: crate::ModuleRegistry,
    pub network: Option<ChatNetworkOptions>,
}

#[cfg(feature = "chat")]
struct ChatDriversHook {
    drivers: Arc<chat::Drivers>,
}

#[cfg(feature = "chat")]
impl crate::registry::LoadHook for ChatDriversHook {
    fn loaded(&self, manifest: &module_api::ModuleManifest, driver: Arc<dyn module_api::ModuleDriver>) {
        if manifest.capabilities.chat {
            self.drivers.insert(driver);
        }
    }

    fn unloaded(&self, module: &module_api::ModuleId) {
        self.drivers.remove(module);
    }
}

#[cfg(feature = "chat")]
struct ControllerChatHost {
    node: module_api::NodeIdentity,
    drivers: Arc<chat::Drivers>,
    plugin: Arc<chat::ChatPlugin>,
    registry: crate::WeakRegistry,
    federation: Arc<dyn chat::Federation>,
}

#[cfg(feature = "chat")]
impl ControllerChatHost {
    fn active_modules(&self) -> Vec<module_api::ModuleId> {
        let attached: Vec<module_api::ModuleId> = self.plugin.modules().into_iter().collect();
        if !attached.is_empty() {
            attached
        } else {
            self.drivers.module_ids()
        }
    }
}

#[cfg(feature = "chat")]
impl chat::ChatHost for ControllerChatHost {
    fn login<'a>(
        &'a self,
        login: chat::Login<'a>,
    ) -> module_api::BoxFuture<'a, Result<chat::Player, chat::LoginRefusal>> {
        let authcid = login.authcid.to_owned();
        let token = login.token.to_owned();
        Box::pin(async move {
            let modules = self.active_modules();
            if modules.is_empty() {
                return Err(chat::LoginRefusal::Unavailable);
            }
            let mut saw_unavailable = false;
            for module in &modules {
                let query = module_api::chat::ChatQuery::Login {
                    authcid: authcid.clone(),
                    token: token.clone(),
                };
                match self.drivers.ask(module, query).await {
                    Ok(module_api::chat::ChatAnswer::Login { account, name }) => {
                        return Ok(chat::Player {
                            account,
                            module: module.clone(),
                            name,
                        });
                    }
                    Ok(_) => {}
                    Err(err) if err.kind == module_api::ErrorKind::Unavailable => {
                        saw_unavailable = true;
                    }
                    Err(_) => {}
                }
            }
            if let Some(account) = self.federation.resolve(&authcid).await
                && account.owner() != &self.node.node
                && let Some(reg) = self.registry.upgrade()
                && let Ok(operation) = "chat.login".parse::<module_api::network::OperationName>()
            {
                let home = reg.home();
                for module in &modules {
                    let forward = module_api::network::HomeForward {
                        module: module.clone(),
                        account: account.clone(),
                        actor: module_api::ActorContext::new(module_api::Actor::System),
                        operation: operation.clone(),
                        body: serde_json::json!({
                            "authcid": authcid,
                            "token": token,
                        }),
                        key: None,
                    };
                    if let Ok(val) = home.forward(&forward).await
                        && let Some(name) = val.get("name").and_then(serde_json::Value::as_str)
                    {
                        return Ok(chat::Player {
                            account,
                            module: module.clone(),
                            name: name.to_owned(),
                        });
                    }
                }
            }
            if saw_unavailable {
                Err(chat::LoginRefusal::Unavailable)
            } else {
                Err(chat::LoginRefusal::BadToken)
            }
        })
    }

    fn ask<'a>(
        &'a self,
        module: &'a module_api::ModuleId,
        query: module_api::chat::ChatQuery,
    ) -> module_api::BoxFuture<'a, Result<module_api::chat::ChatAnswer, module_api::ModuleError>> {
        Box::pin(async move { self.drivers.ask(module, query).await })
    }

    fn blocked<'a>(
        &'a self,
        sender: &'a module_api::GlobalId,
        recipient: &'a module_api::GlobalId,
    ) -> module_api::BoxFuture<'a, bool> {
        Box::pin(async move {
            for module in self.active_modules() {
                let query = module_api::chat::ChatQuery::MayWhisper {
                    from: sender.clone(),
                    to: recipient.clone(),
                };
                if let Ok(module_api::chat::ChatAnswer::MayWhisper(allowed)) = self.drivers.ask(&module, query).await {
                    return !allowed;
                }
            }
            false
        })
    }

    fn account_of<'a>(&'a self, local: &'a str) -> module_api::BoxFuture<'a, Option<module_api::GlobalId>> {
        let local = local.to_owned();
        Box::pin(async move {
            for module in self.active_modules() {
                let query = module_api::chat::ChatQuery::AccountOf { local: local.clone() };
                if let Ok(module_api::chat::ChatAnswer::AccountOf(Some(account))) =
                    self.drivers.ask(&module, query).await
                {
                    return Some(account);
                }
            }
            if let Some(reg) = self.registry.upgrade()
                && let Some(dir) = reg.node_services().directory()
                && let Ok(Some(record)) = dir.find(&local).await
            {
                return Some(record.account);
            }
            None
        })
    }

    fn presence<'a>(
        &'a self,
        account: &'a module_api::GlobalId,
        local: &'a str,
        online: bool,
    ) -> module_api::BoxFuture<'a, ()> {
        let account = account.clone();
        let local = local.to_owned();
        Box::pin(async move {
            if let Some(reg) = self.registry.upgrade()
                && let Ok(name) = module_api::chat::PRESENCE_EVENT.parse()
                && let Ok(payload) = serde_json::to_value(module_api::chat::PresenceEvent {
                    account,
                    user_id: local,
                    online,
                })
            {
                reg.emit(module_api::Event { name, payload });
            }
        })
    }
}

/// The chat core plugin (`chat.db` beside the node database, listening on loopback `CHAT_PORT` / `61006`
/// behind the gateway, and attached to `Service::Chat` when in a network; DD21, DN10a, DN10b).
#[cfg(feature = "chat")]
pub fn chat(options: ChatOptions) -> Result<Module, String> {
    let mut settings = chat::Settings::default();
    if let Some(secret) = options
        .env
        .get("GATEWAY_SECRET")
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        settings.gateway_secret = Some(secret.to_owned());
    }
    if let Some(port) = options.env.get("CHAT_PORT").and_then(|s| s.trim().parse::<u16>().ok()) {
        settings.port = port;
    }
    if options
        .env
        .get("CHAT_NICK_CHECK")
        .is_some_and(|s| s.trim().eq_ignore_ascii_case("log"))
    {
        settings.nick_check = chat::NickCheck::Log;
    }
    if options
        .env
        .get("CHAT_TRACE")
        .is_some_and(|s| matches!(s.trim(), "1" | "true" | "TRUE"))
    {
        settings.trace = true;
    }
    let problems = settings.problems();
    if !problems.is_empty() {
        return Err(format!("chat settings: {}", problems.join("; ")));
    }
    let plugin = Arc::new(chat::ChatPlugin::new());
    let drivers = Arc::new(chat::Drivers::default());
    options.registry.add_plugin(plugin.clone());
    options.registry.add_hook(Arc::new(ChatDriversHook {
        drivers: drivers.clone(),
    }));
    let node = options.node;
    let database = options.database;
    let registry = options.registry;
    let network = options.network;
    Ok(Module::new(
        "chat",
        Arc::new(move |mut ctx| {
            let (settings, plugin, drivers, node, database, registry, network) = (
                settings.clone(),
                plugin.clone(),
                drivers.clone(),
                node.clone(),
                database.clone(),
                registry.clone(),
                network.clone(),
            );
            Box::pin(async move {
                if let store::DatabaseConfig::Sqlite { file } = &database
                    && let Some(parent) = file.parent()
                {
                    let _ = std::fs::create_dir_all(parent);
                }
                let chat_db = chat::History::beside(&database);
                let history = chat::History::open(&chat_db).map_err(|e| format!("chat history database: {e}"))?;
                let federation: Arc<dyn chat::Federation> = if let Some(net) = network {
                    let link = {
                        let mut tries = 0u32;
                        loop {
                            if let Some(link) = registry.node_services().peer_link() {
                                break link;
                            }
                            tries += 1;
                            if tries > 200 {
                                return Err("chat federation: network peer link did not become ready".into());
                            }
                            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                        }
                    };
                    let fed_config = chat::ChatFederationConfig::new(node.clone(), net.peers).with_leader(net.leader);
                    let fed = Arc::new(chat::ChatFederation::new(fed_config, link));
                    registry
                        .node_services()
                        .attach(network_plugin::Service::Chat, fed.clone());
                    fed
                } else {
                    Arc::new(chat::LocalOnly)
                };
                let host: Arc<dyn chat::ChatHost> = Arc::new(ControllerChatHost {
                    node: node.clone(),
                    drivers,
                    plugin: plugin.clone(),
                    registry: registry.downgrade(),
                    federation: federation.clone(),
                });
                let running = plugin
                    .start(&settings, node, host, federation, history, chat::Options::default())
                    .await
                    .map_err(|e| e.to_string())?;
                ctx.status.ready(Some(format!("127.0.0.1:{}", running.port)), None);
                ctx.wait_stop().await;
                ctx.status.stopping();
                registry.node_services().detach(network_plugin::Service::Chat);
                running.stop().await;
                ctx.status.stopped();
                Ok(())
            })
        }),
    )
    .optional()
    .ready_within(std::time::Duration::from_secs(30)))
}
