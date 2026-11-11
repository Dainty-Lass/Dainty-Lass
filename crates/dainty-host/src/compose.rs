// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The AGPL services as modules of the MIT controller, until each is a library the core loads (RD27): moved
//! from `dainty_controller::modules` with the repository split. Each reads the env file the host kit wrote for it
//! (`hostconfig.rs`), through its own `Config::from_env`, so a service in this process sees exactly what it
//! saw as a process of its own.

pub use dainty_controller::modules::*;

#[cfg(feature = "gateway")]
use std::sync::Arc;

#[cfg(any(feature = "gateway", feature = "allowlist", feature = "content", feature = "chat"))]
use dainty_service_kit::env::Env;

#[cfg(any(feature = "gateway", feature = "allowlist", feature = "content", feature = "chat"))]
use dainty_controller::Module;

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
#[cfg_attr(
    not(feature = "isles-1-14-7"),
    allow(unused_mut, reason = "only the 1.14.7 module is pushed onto the list")
)]
/// The 1.14.7 module reads the installed `version_data.json` when it starts, as the library does.
pub fn game_modules() -> Vec<dainty_controller::ModuleFactory> {
    #[cfg(feature = "isles-1-14-7")]
    {
        with_isles(None)
    }
    #[cfg(not(feature = "isles-1-14-7"))]
    {
        dainty_controller::modules::game_modules()
    }
}

/// The 1.14.7 module handed the version data `serve` read (`version_data`).
#[cfg(feature = "isles-1-14-7")]
pub fn game_modules_with(data: rustic_isles_1_14_7::VersionData) -> Vec<dainty_controller::ModuleFactory> {
    with_isles(Some(data))
}

#[cfg(feature = "isles-1-14-7")]
fn with_isles(data: Option<rustic_isles_1_14_7::VersionData>) -> Vec<dainty_controller::ModuleFactory> {
    let mut modules = dainty_controller::modules::game_modules();
    modules.push(std::sync::Arc::new(move || {
        std::sync::Arc::new(isles_1_14_7(data.clone())) as std::sync::Arc<dyn dainty_module_api::GameModule>
    }));
    modules
}

/// The id is a constant that parses, so the one construction failure cannot happen.
#[cfg(feature = "isles-1-14-7")]
fn isles_1_14_7(data: Option<rustic_isles_1_14_7::VersionData>) -> rustic_isles::IslesModule {
    let spec = match data {
        Some(data) => rustic_isles_1_14_7::spec_with(data),
        None => rustic_isles_1_14_7::spec(),
    };
    match spec.and_then(rustic_isles::IslesModule::new) {
        Ok(module) => module,
        Err(error) => panic!("rustic-isles-1-14-7: {error}"),
    }
}

/// The in-process allowlist (`allowlist.env`), for [`gateway`]: it runs inside the gateway service, as it does in
/// the gateway library, so the gateway's player admits reach its helper directly and its table closes the game ports
/// before the gateway listens. With the registry, it also guards the UDP ranges of the game modules that load (DF4d).
/// Bad settings are refused here, as the allowlist module refused them.
#[cfg(feature = "allowlist")]
pub fn allowlist(env: &Env) -> Result<dainty_controller::service_library::gateway::AllowlistMode, String> {
    let config = dainty_gateway::allowlist::config::AllowlistConfig::from_env(env)?;
    dainty_gateway::allowlist::guard::Guard::new(&config.ports)?;
    Ok(allowlist_in_library(env))
}

/// The gateway and its allowlist (`gateway.env`), compiled in: `gateway::service::GatewayService`, the same service
/// `gateway-plugin` exports, run over the same protocol (RD27). `allowlist` is [`allowlist`]'s in-process helper, or
/// how a public server whose allowlist this process doesn't run treats UDP ranges (`External`, `Absent`, DF4d). The
/// handle gives the network module its allowlist sink ([`NetworkOptions::allowlist`]).
#[cfg(feature = "gateway")]
pub fn gateway(
    env: &Env,
    allowlist: dainty_controller::service_library::gateway::AllowlistMode,
    registry: Option<dainty_controller::ModuleRegistry>,
) -> Result<(Module, dainty_controller::service_library::gateway::GatewayHandle), String> {
    let options = gateway_options(env, allowlist, registry.is_some())?;
    let (module, handle) = dainty_controller::service_library::gateway::gateway_compiled_in(
        dainty_gateway::service::GatewayService::default,
        env.clone(),
        registry.as_ref(),
        options,
    );
    // The allowlist is ready before the gateway opens, within the service's start
    Ok((module.ready_within(std::time::Duration::from_secs(120)), handle))
}

/// The gateway and the allowlist as the library `library` pins (RD27), on `gateway.env`: what [`gateway`] runs
/// compiled in, registered with `registry` as the core plugin for routes and UDP ranges.
#[cfg(feature = "gateway")]
pub fn gateway_library(
    library: dainty_controller::ServiceLibrary,
    env: &Env,
    allowlist: dainty_controller::service_library::gateway::AllowlistMode,
    registry: Option<dainty_controller::ModuleRegistry>,
) -> Result<(Module, dainty_controller::service_library::gateway::GatewayHandle), String> {
    let options = gateway_options(env, allowlist, registry.is_some())?;
    let (module, handle) =
        dainty_controller::service_library::gateway::gateway_library(library, env.clone(), registry.as_ref(), options);
    Ok((module.ready_within(std::time::Duration::from_secs(120)), handle))
}

/// What the core tells the gateway before each start. Bad settings are refused here rather than when it starts.
#[cfg(feature = "gateway")]
fn gateway_options(
    env: &Env,
    allowlist: dainty_controller::service_library::gateway::AllowlistMode,
    registry: bool,
) -> Result<dainty_controller::service_library::gateway::GatewayOptions, String> {
    #[cfg_attr(not(feature = "chat"), allow(unused_variables))]
    let config = dainty_gateway::config::Config::from_env(env)?;
    #[cfg_attr(not(feature = "chat"), allow(unused_mut))]
    let mut ws_port = None;
    // Chat listens on its own port when the gateway would send /ws to the backend's
    #[cfg(feature = "chat")]
    if registry && config.ws.port == config.backend.port {
        ws_port = Some(dainty_chat::env::settings(env)?.port);
    }
    #[cfg(not(feature = "chat"))]
    let _ = registry;
    Ok(dainty_controller::service_library::gateway::GatewayOptions { allowlist, ws_port })
}

/// The in-process allowlist for [`gateway_library`], on `allowlist.env`.
#[cfg(feature = "gateway")]
pub fn allowlist_in_library(env: &Env) -> dainty_controller::service_library::gateway::AllowlistMode {
    dainty_controller::service_library::gateway::AllowlistMode::InProcess {
        env: env.vars().map(|(k, v)| (k.to_owned(), v.to_owned())).collect(),
    }
}

/// The ring's admits when no gateway runs in this process: refused, and the game port stays closed.
#[cfg(feature = "gateway")]
struct NoGateway;

#[cfg(feature = "gateway")]
impl network_plugin::ring::AllowlistSink for NoGateway {
    fn admit_player(
        &self,
        instance: &dainty_module_api::network::GlobalId,
        _account: &dainty_module_api::network::GlobalId,
        _client_ip: &str,
        udp_port: u16,
    ) {
        tracing::warn!(
            target: "host",
            instance = %instance,
            udp_port,
            "no gateway runs to admit the player's UDP; game port stays closed"
        );
    }
}

/// Configuration for the `"network"` module (DD2, DN3, DN5, DH7).
#[cfg(feature = "gateway")]
pub struct NetworkOptions {
    /// This node.
    pub node: dainty_module_api::NodeIdentity,
    /// The ring's leader.
    pub leader: dainty_module_api::network::NodeId,
    /// The other nodes, as `[[network.nodes]]` lists them.
    pub peers: Vec<network_plugin::cluster::Peer>,
    /// This node's certificate chain, key and the cluster CA.
    pub credentials: network_plugin::cluster::NodeCredentials,
    /// This node's private signing key.
    pub signing_key: network_plugin::home::SigningKey,
    /// The node database.
    pub database: dainty_store::DatabaseConfig,
    /// The game version the ring serves.
    pub game: dainty_module_api::GameVersion,
    /// The registry the module joins.
    pub registry: dainty_controller::ModuleRegistry,
    /// The gateway this process runs ([`gateway`] or [`gateway_library`]): the ring's players are admitted through
    /// its allowlist (DN8, DN9)
    pub allowlist: Option<dainty_controller::service_library::gateway::GatewayHandle>,
    /// How often the directory replicates to the peers.
    pub replication_interval: std::time::Duration,
    /// Folder where this node stores log segments and snapshots shipped from peers (`Service::LogShip`, DH7).
    pub peer_log_dir: Option<std::path::PathBuf>,
    /// Peer node to ship this node's change log and snapshots to (`[history] ship_to_peer`, DH7), and its state file.
    pub ship_to_peer: Option<(dainty_module_api::network::NodeId, std::path::PathBuf)>,
    /// Policy when a player joins an instance hosted on a different node (`[network] wrong_node_policy`, DN8).
    pub wrong_node_policy: Option<network_plugin::ring::WrongNodePolicy>,
    /// `[network] failover = true`: the election this node runs, and the file its term and vote are kept in
    /// (`docs/distributed-mode.md`). `leader` is unused then
    pub election: Option<(network_plugin::ElectionConfig, std::path::PathBuf)>,
}

#[cfg(feature = "gateway")]
struct HomeRouterHook {
    router: Arc<network_plugin::home::HomeRouter>,
    director: Arc<network_plugin::ring::DriverDirectorRouter>,
}

#[cfg(feature = "gateway")]
impl dainty_controller::LoadHook for HomeRouterHook {
    fn loaded(&self, manifest: &dainty_module_api::ModuleManifest, driver: Arc<dyn dainty_module_api::ModuleDriver>) {
        if manifest.kind == dainty_module_api::ModuleKind::Game {
            self.router.register(
                manifest.id.clone(),
                Arc::new(network_plugin::home::DriverBackend::new(driver.clone())),
            );
            self.director.register(driver);
        }
    }

    fn unloaded(&self, module: &dainty_module_api::ModuleId) {
        self.router.unregister(module);
        self.director.unregister(module);
    }
}

/// The node's network runtime (`[network]` in `host.toml` when peers are configured): opens the
/// directory table in the node database, connects [`network_plugin::TlsPeerLink`] to peers, starts
/// directory replication and peer health checks, wires [`network_plugin::home::NetworkedHome`] into
/// the [`dainty_controller::ModuleRegistry`], and attaches [`network_plugin::Service::Directory`] and
/// [`network_plugin::Service::Home`] to [`dainty_controller::NodeServices`].
#[cfg(feature = "gateway")]
pub fn network(options: NetworkOptions) -> Result<Module, String> {
    let sink: Arc<dyn network_plugin::ring::AllowlistSink> = match &options.allowlist {
        Some(gateway) => gateway.allowlist_sink(),
        None => Arc::new(NoGateway),
    };
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
    let leader = (options.leader, options.election);
    let peers = options.peers;
    let database = options.database;
    let game = options.game;
    let registry = options.registry;
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
                sink,
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
                sink.clone(),
                link.clone(),
                tickets.clone(),
                router.clone(),
                director.clone(),
                peer_log_dir.clone(),
                ship_to_peer.clone(),
                wrong_node_policy,
            );
            Box::pin(async move {
                if let dainty_store::DatabaseConfig::Sqlite { file } = &database
                    && let Some(parent) = file.parent()
                {
                    let _ = std::fs::create_dir_all(parent);
                }
                let db = dainty_store::Database::open(
                    &database,
                    &dainty_store::DatabaseOptions::default(),
                    &network_plugin::directory::SCHEMA,
                )
                .map_err(|e| format!("network directory database: {e}"))?;
                tickets
                    .attach_database(db.clone())
                    .await
                    .map_err(|e| format!("network ticket revocations: {e}"))?;
                let _health_task = link.spawn_health_checks();
                let (leader, election) = leader;
                let dir_config = match &election {
                    Some((config, _)) => {
                        network_plugin::directory::DirectoryConfig::failover(node.node.clone(), config.peers.clone())
                    }
                    None => network_plugin::directory::DirectoryConfig::in_network(node.node.clone(), leader),
                };
                let directory = Arc::new(
                    network_plugin::directory::Directory::new(db.clone(), dir_config, link.clone())
                        .await
                        .map_err(|e| format!("network directory: {e}"))?,
                );
                let replication_task = directory.spawn_replication(replication_interval);
                let election = match election {
                    Some((config, state)) => {
                        let election = network_plugin::Election::new(
                            config,
                            link.clone(),
                            directory.clone(),
                            Arc::new(network_plugin::election::FileTermStore::new(state)),
                        )
                        .map_err(|e| format!("network election: {e}"))?;
                        let tasks = [directory.follow(election.subscribe()), election.spawn()];
                        Some((election, tasks))
                    }
                    None => None,
                };
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
                    .or_else(
                        || match std::env::var("DAINTY_WRONG_NODE_POLICY").ok().as_deref().map(str::trim) {
                            Some("redirect") => Some(network_plugin::ring::WrongNodePolicy::Redirect),
                            Some("refuse") => Some(network_plugin::ring::WrongNodePolicy::Refuse),
                            _ => None,
                        },
                    )
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
                    sink,
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
                if let Some((election, _)) = &election {
                    services.attach(network_plugin::Service::Election, election.clone());
                }
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
                    let mut settings = dainty_history_ship::ShipSettings::new(
                        network_plugin::logship::PeerBucket::prefix_for(&node.node),
                        state_file,
                    );
                    settings.interval = interval;
                    settings.segment_bytes = 512 * 1024;
                    // Plaintext until a key kept off the node is decided: one derived from the node's own signing
                    // key dies with the node, and the peer's copies with it (DH7)
                    let bucket = network_plugin::logship::PeerBucket::new(target_peer.clone(), link.clone());
                    let mut shipper = dainty_history_ship::Shipper::new(db, bucket, settings)
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
                services.detach(network_plugin::Service::Election);
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
                for task in election.iter().flat_map(|(_, tasks)| tasks) {
                    task.abort();
                }
                ctx.status.stopped();
                Ok(())
            })
        }),
    )
    .ready_within(std::time::Duration::from_secs(60)))
}

/// The content server (`content.env`), compiled in: `content::service::ContentService`, the same service
/// `content-plugin` exports, run over the same protocol (RD27).
#[cfg(feature = "content")]
pub fn content(env: &Env, registry: Option<dainty_controller::ModuleRegistry>) -> Result<Module, String> {
    // Refused here, as before, rather than when the module starts
    dainty_content::config::Config::from_env(env)?;
    Ok(dainty_controller::service_compiled_in(
        dainty_content::service::ContentService::default,
        env.clone(),
        registry.as_ref(),
    )
    .optional()
    .ready_within(std::time::Duration::from_secs(60)))
}

/// The content server as the library `library` pins (RD27), on `content.env`: what [`content`] runs compiled in,
/// registered with `registry` as the same core plugin.
pub fn content_library(
    library: dainty_controller::ServiceLibrary,
    env: &dainty_service_kit::env::Env,
    registry: Option<dainty_controller::ModuleRegistry>,
) -> dainty_controller::Module {
    dainty_controller::service_library(library, env.clone(), registry.as_ref())
        .optional()
        .ready_within(std::time::Duration::from_secs(60))
}

/// Network peer configuration for [`chat`] when running in a multi-node cluster (DN10b).
#[derive(Clone)]
pub struct ChatNetworkOptions {
    /// The ring's leader.
    pub leader: dainty_module_api::network::NodeId,
    /// The other nodes and their regions.
    pub peers: Vec<(dainty_module_api::network::NodeId, dainty_module_api::network::Region)>,
}

/// Configuration for the `"chat"` module (DD21, DN10a, DN10b), in-process or as a library.
pub struct ChatOptions {
    /// The chat service's env file.
    pub env: dainty_service_kit::env::Env,
    /// This node.
    pub node: dainty_module_api::NodeIdentity,
    /// The node database, beside which `chat.db` lives.
    pub database: dainty_store::DatabaseConfig,
    /// The registry the module joins.
    pub registry: dainty_controller::ModuleRegistry,
    /// The cluster's peers; `None` on a single node.
    pub network: Option<ChatNetworkOptions>,
}

/// The chat core plugin (`chat.db` beside the node database, listening on loopback `CHAT_PORT` / `41006` behind the
/// gateway, and attached to `Service::Chat` when in a network; DD21, DN10a, DN10b), compiled in:
/// `chat::service::ChatService`, the same service `chat-plugin` exports, run over the same protocol (RD27).
#[cfg(feature = "chat")]
pub fn chat(options: ChatOptions) -> Result<Module, String> {
    // Refused here, as before, rather than when the module starts
    dainty_chat::env::settings(&options.env)?;
    dainty_chat::env::dashboard(&options.env)?;
    let registry = options.registry.clone();
    let (env, setup) = chat_env_and_setup(options);
    Ok(dainty_controller::service_library::chat_compiled_in(
        dainty_chat::service::ChatService::default,
        env,
        &registry,
        setup,
    ))
}

/// Chat as a library the core loads (RD27, `crates/dainty-chat-plugin`): what [`chat`] runs compiled in.
pub fn chat_library(library: dainty_controller::ServiceLibrary, options: ChatOptions) -> dainty_controller::Module {
    let registry = options.registry.clone();
    let (env, setup) = chat_env_and_setup(options);
    dainty_controller::service_library::chat_library(library, env, &registry, setup)
}

/// Chat's env and setup, either way it runs: the node database goes in its env (`DAINTY_DB_*`, as setup writes
/// them), the node and network in its setup.
fn chat_env_and_setup(options: ChatOptions) -> (dainty_service_kit::env::Env, dainty_service_plugin::chat::ChatSetup) {
    let mut env: Vec<(String, String)> = options
        .env
        .vars()
        .filter(|(key, _)| !key.starts_with("DAINTY_DB_"))
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
    env.extend(database_env(&options.database));
    let env = dainty_service_kit::env::Env::from_pairs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    let setup = dainty_service_plugin::chat::ChatSetup {
        node: options.node,
        network: options.network.map(|n| dainty_service_plugin::chat::ChatNetwork {
            leader: n.leader,
            peers: n.peers,
        }),
    };
    (env, setup)
}

/// What store's `DatabaseConfig::from_lookup` reads back as `database`.
fn database_env(database: &dainty_store::DatabaseConfig) -> Vec<(String, String)> {
    match database {
        dainty_store::DatabaseConfig::Sqlite { file } => vec![
            ("DAINTY_DB_ENGINE".into(), "sqlite".into()),
            ("DAINTY_DB_FILE".into(), file.to_string_lossy().into_owned()),
        ],
        dainty_store::DatabaseConfig::Postgres(pg) => {
            let mut out = vec![
                ("DAINTY_DB_ENGINE".into(), "postgres".into()),
                ("DAINTY_DB_HOST".into(), pg.host.clone()),
                ("DAINTY_DB_PORT".into(), pg.port.to_string()),
                ("DAINTY_DB_USER".into(), pg.user.clone()),
                ("DAINTY_DB_NAME".into(), pg.database.clone()),
                ("DAINTY_DB_PASSWORD".into(), pg.password.clone()),
            ];
            if pg.allow_remote {
                out.push(("DAINTY_DB_ALLOW_REMOTE".into(), "1".into()));
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
        let sqlite = dainty_store::DatabaseConfig::Sqlite {
            file: std::path::PathBuf::from("/srv/dainty/dainty.db"),
        };
        let postgres = dainty_store::DatabaseConfig::Postgres(dainty_store::PostgresConfig {
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
            let back = dainty_store::DatabaseConfig::from_lookup(lookup, std::path::PathBuf::new()).unwrap();
            assert_eq!(back, database);
        }
    }
}
