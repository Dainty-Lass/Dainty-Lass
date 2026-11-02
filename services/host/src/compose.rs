// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The AGPL services as modules of the MIT controller, until each is a library the core loads (RD27): moved
//! from `controller::modules` with the repository split. Each reads the env file the host kit wrote for it
//! (`hostconfig.rs`), through its own `Config::from_env`, so a service in this process sees exactly what it
//! saw as a process of its own.

pub use controller::modules::*;

#[cfg(any(feature = "gateway", feature = "allowlist", feature = "content", feature = "chat"))]
use std::sync::Arc;

#[cfg(any(feature = "gateway", feature = "allowlist", feature = "content", feature = "chat"))]
use service_kit::env::Env;

#[cfg(any(feature = "gateway", feature = "allowlist", feature = "content", feature = "chat"))]
use controller::Module;

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

/// The controller's game modules, and the ones this binary links (DD3, DF5).
#[cfg_attr(not(feature = "isles-1-14-7"), allow(unused_mut))]
pub fn game_modules() -> Vec<controller::ModuleFactory> {
    let mut modules = controller::modules::game_modules();
    #[cfg(feature = "isles-1-14-7")]
    modules.push(std::sync::Arc::new(|| {
        std::sync::Arc::new(isles_1_14_7::module()) as std::sync::Arc<dyn module_api::GameModule>
    }));
    modules
}

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
pub fn allowlist(
    env: &Env,
    slot: AllowlistSlot,
    registry: Option<controller::ModuleRegistry>,
) -> Result<Module, String> {
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
pub fn allowlist_absent(registry: &controller::ModuleRegistry) {
    let view = Arc::new(RegistryPorts(registry.downgrade()));
    registry.add_plugin(Arc::new(gateway::allowlist::modules::AllowlistPlugin::absent(view)));
}

/// A public server whose allowlist runs in an external helper process (`reforged-allowlist.service`)
/// guarding `base` (`game_ports(config)`): a module whose assigned UDP ports lie inside `base` is
/// guarded by that helper; any module asking for UDP ports outside `base` is refused (DF4d).
#[cfg(feature = "allowlist")]
pub fn allowlist_external(registry: &controller::ModuleRegistry, base: module_api::PortRange) {
    let view = Arc::new(RegistryPorts(registry.downgrade()));
    registry.add_plugin(Arc::new(gateway::allowlist::modules::AllowlistPlugin::external(
        base, view,
    )));
}

/// The ranges the controller placed, for the allowlist plugin.
#[cfg(feature = "allowlist")]
struct RegistryPorts(controller::WeakRegistry);

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
    registry: Option<controller::ModuleRegistry>,
) -> Result<Module, String> {
    use gateway::config::Config;
    #[allow(unused_mut)]
    let mut config = Config::from_env(env)?;
    #[cfg(feature = "chat")]
    if registry.is_some() && config.ws.port == config.backend.port {
        config.ws.port = chat::settings::DEFAULT_PORT;
    }
    let node_links = registry.as_ref().map(|_| gateway::server::NodeLinkSlot::default());
    let region_handler = registry.as_ref().map(|r| region_report_handler(r.node_services()));
    let bind_on_ready = registry
        .as_ref()
        .zip(node_links.clone())
        .map(|(r, links)| (r.node_services().clone(), r.status_registry().clone(), links));
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
                if let Some((services, status, links)) = bind_on_ready {
                    let mut rx = status.subscribe();
                    tokio::spawn(async move {
                        while rx.changed().await.is_ok() {
                            let phase = rx.borrow().get("gateway").map(|c| c.phase);
                            if let Some(phase) = phase {
                                match phase {
                                    service_kit::context::ComponentPhase::Ready => {
                                        let placed =
                                            links.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
                                        if let Some(placed) = placed {
                                            services.set_gateway(Some(Arc::new(GatewayNodeLinks(placed))));
                                        }
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

/// The gateway's node links, handed to [`controller::NodeServices`] as the controller's own trait (RD27).
#[cfg(feature = "gateway")]
struct GatewayNodeLinks(Arc<gateway::nodelink::NodeLinks>);

#[cfg(feature = "gateway")]
impl controller::node_services::GatewayLinks for GatewayNodeLinks {
    fn endpoint(&self) -> &network_plugin::NodeEndpoint {
        self.0.endpoint()
    }

    fn set_enroll_handler(&self, handler: Option<controller::node_services::RemoteEnrollCallback>) {
        self.0
            .set_enroll_handler(handler.map(|cb| Arc::new(move |body| cb(body)) as gateway::network::EnrollHandler));
    }
}

/// Builds a [`gateway::network::RegionReportHandler`] that forwards `POST /network/v1/regions` reports
/// to [`network_plugin::ring::RingNode::record_region_pings_authenticated`] when the network plugin is
/// running, or accepts standalone shape-checked reports when no network ring is active (DN7, DN8).
#[cfg(feature = "gateway")]
fn region_report_handler(services: &controller::NodeServices) -> gateway::network::RegionReportHandler {
    let services = services.clone();
    Arc::new(move |api_key, ticket_token, regions| {
        let services = services.clone();
        Box::pin(async move {
            let Some(ring) = services.ring() else {
                return true;
            };
            let mut pings = Vec::with_capacity(regions.len());
            for r in regions {
                let Ok(region) = r.region.parse() else {
                    return false;
                };
                let rtt_ms = r.rtt_ms.and_then(|ms| {
                    if ms.is_finite() && (0.0..=f64::from(u32::MAX)).contains(&ms) {
                        Some(ms.round() as u32)
                    } else {
                        None
                    }
                });
                pings.push(network_plugin::ring::RegionPing { region, rtt_ms });
            }
            let parsed_ticket = match ticket_token.as_deref() {
                Some(token) => match network_plugin::home::ticket::session_ticket_from_token(token) {
                    Ok(pair) => Some(pair),
                    Err(_) => return false,
                },
                None => None,
            };
            let ticket_ref = parsed_ticket.as_ref().map(|(t, g)| (t, g));
            ring.record_region_pings_authenticated(api_key.as_deref(), ticket_ref, pings)
                .await
                .is_ok()
        })
    })
}

/// In-process [`network_plugin::ring::AllowlistSink`] backed by the allowlist helper module (`AllowlistSlot`, DF4d, DN8, DN9).
#[cfg(feature = "gateway")]
pub struct InProcessAllowlistSink {
    slot: Option<AllowlistSlot>,
}

#[cfg(feature = "gateway")]
impl InProcessAllowlistSink {
    pub fn new(slot: Option<AllowlistSlot>) -> Self {
        Self { slot }
    }
}

#[cfg(feature = "gateway")]
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
#[cfg(feature = "gateway")]
pub struct NetworkOptions {
    pub node: module_api::NodeIdentity,
    pub leader: module_api::network::NodeId,
    pub peers: Vec<network_plugin::cluster::Peer>,
    pub credentials: network_plugin::cluster::NodeCredentials,
    pub signing_key: network_plugin::home::SigningKey,
    pub database: store::DatabaseConfig,
    pub game: module_api::GameVersion,
    pub registry: controller::ModuleRegistry,
    pub allowlist: Option<AllowlistSlot>,
    pub replication_interval: std::time::Duration,
    /// Folder where this node stores log segments and snapshots shipped from peers (`Service::LogShip`, DH7).
    pub peer_log_dir: Option<std::path::PathBuf>,
    /// Peer node to ship this node's change log and snapshots to (`[history] ship_to_peer`, DH7), and its state file.
    pub ship_to_peer: Option<(module_api::network::NodeId, std::path::PathBuf)>,
    /// Policy when a player joins an instance hosted on a different node (`[network] wrong_node_policy`, DN8).
    pub wrong_node_policy: Option<network_plugin::ring::WrongNodePolicy>,
}

#[cfg(feature = "gateway")]
struct HomeRouterHook {
    router: Arc<network_plugin::home::HomeRouter>,
    director: Arc<network_plugin::ring::DriverDirectorRouter>,
}

#[cfg(feature = "gateway")]
impl controller::LoadHook for HomeRouterHook {
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
/// the [`controller::ModuleRegistry`], and attaches [`network_plugin::Service::Directory`] and
/// [`network_plugin::Service::Home`] to [`controller::NodeServices`].
#[cfg(feature = "gateway")]
pub fn network(options: NetworkOptions) -> Result<Module, String> {
    let link = Arc::new(
        network_plugin::TlsPeerLink::new(network_plugin::TlsLinkConfig::new(
            options.credentials,
            options.peers.clone(),
        ))
        .map_err(|e| format!("network peer link: {e}"))?,
    );
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
                    // Plaintext until a key kept off the node is decided: one derived from the node's own signing
                    // key dies with the node, and the peer's copies with it (DH7)
                    let bucket = network_plugin::logship::PeerBucket::new(target_peer.clone(), link.clone());
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
pub fn content(env: &Env, registry: Option<controller::ModuleRegistry>) -> Result<Module, String> {
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

/// Where a module's files are, for content (DF4b): the folder its executable is in.
#[cfg(feature = "content")]
struct ContentView(controller::WeakRegistry);

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
    registry: controller::WeakRegistry,
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
    registry: &controller::ModuleRegistry,
) -> Result<Arc<gateway::modules::ModuleRoutes>, String> {
    let (manifest, registration) = gateway::modules::stand_in(&config.version)?;
    let view = GatewayView {
        registry: registry.downgrade(),
        own: (manifest.id.clone(), "backend", config.backend.clone()),
    };
    let routes = Arc::new(gateway::modules::ModuleRoutes::new(Arc::new(view)));
    routes
        .attach_stand_in(&manifest, &registration)
        .map_err(|e| e.to_string())?;
    registry.add_plugin(routes.clone());
    Ok(routes)
}

/// Network peer configuration for [`chat`] when running in a multi-node cluster (DN10b).
#[derive(Clone)]
pub struct ChatNetworkOptions {
    pub leader: module_api::network::NodeId,
    pub peers: Vec<(module_api::network::NodeId, module_api::network::Region)>,
}

/// Configuration for the `"chat"` module (DD21, DN10a, DN10b), in-process or as a library.
pub struct ChatOptions {
    pub env: service_kit::env::Env,
    pub node: module_api::NodeIdentity,
    pub database: store::DatabaseConfig,
    pub registry: controller::ModuleRegistry,
    pub network: Option<ChatNetworkOptions>,
}

#[cfg(feature = "chat")]
struct ChatDriversHook {
    drivers: Arc<chat::Drivers>,
}

#[cfg(feature = "chat")]
impl controller::LoadHook for ChatDriversHook {
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
    registry: controller::WeakRegistry,
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
    let settings = chat::env::settings(&options.env)?;
    let dashboard = chat::env::dashboard(&options.env)?;
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
            let (dashboard, settings, plugin, drivers, node, database, registry, network) = (
                dashboard.clone(),
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
                let options = chat::Options {
                    moderation: dashboard.map_or_else(chat::Moderation::off, chat::Moderation::start),
                    ..chat::Options::default()
                };
                let running = plugin
                    .start(&settings, node, host, federation, history, options)
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

/// Chat as a library the core loads (RD27, `plugins/chat-plugin`): what [`chat`] composes, with the node and network
/// as the library's setup and the node database added to its env (`REFORGED_DB_*`, as setup writes them).
pub fn chat_library(library: controller::ServiceLibrary, options: ChatOptions) -> controller::Module {
    let mut env: Vec<(String, String)> = options
        .env
        .vars()
        .filter(|(key, _)| !key.starts_with("REFORGED_DB_"))
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
    env.extend(database_env(&options.database));
    let env = service_kit::env::Env::from_pairs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    let setup = service_plugin::chat::ChatSetup {
        node: options.node,
        network: options.network.map(|n| service_plugin::chat::ChatNetwork {
            leader: n.leader,
            peers: n.peers,
        }),
    };
    controller::service_library::chat_library(library, env, &options.registry, setup)
}

/// What store's `DatabaseConfig::from_lookup` reads back as `database`.
fn database_env(database: &store::DatabaseConfig) -> Vec<(String, String)> {
    match database {
        store::DatabaseConfig::Sqlite { file } => vec![
            ("REFORGED_DB_ENGINE".into(), "sqlite".into()),
            ("REFORGED_DB_FILE".into(), file.to_string_lossy().into_owned()),
        ],
        store::DatabaseConfig::Postgres(pg) => {
            let mut out = vec![
                ("REFORGED_DB_ENGINE".into(), "postgres".into()),
                ("REFORGED_DB_HOST".into(), pg.host.clone()),
                ("REFORGED_DB_PORT".into(), pg.port.to_string()),
                ("REFORGED_DB_USER".into(), pg.user.clone()),
                ("REFORGED_DB_NAME".into(), pg.database.clone()),
                ("REFORGED_DB_PASSWORD".into(), pg.password.clone()),
            ];
            if pg.allow_remote {
                out.push(("REFORGED_DB_ALLOW_REMOTE".into(), "1".into()));
            }
            out
        }
    }
}

#[cfg(test)]
mod chat_library_tests {
    use super::*;

    #[test]
    fn the_database_env_reads_back_as_the_database() {
        let sqlite = store::DatabaseConfig::Sqlite {
            file: std::path::PathBuf::from("/srv/dainty/dainty.db"),
        };
        let postgres = store::DatabaseConfig::Postgres(store::PostgresConfig {
            host: "127.0.0.1".into(),
            port: 5433,
            user: "u".into(),
            database: "d".into(),
            password: "p".into(),
            allow_remote: true,
        });
        for database in [sqlite, postgres] {
            let env = database_env(&database);
            let lookup = |key: &str| env.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
            let back = store::DatabaseConfig::from_lookup(lookup, std::path::PathBuf::new()).unwrap();
            assert_eq!(back, database);
        }
    }
}
