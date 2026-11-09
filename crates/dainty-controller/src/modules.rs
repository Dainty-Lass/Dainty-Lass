// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The MIT services as modules (admin, remote), and the game modules compiled in: each reads the env file the
//! host kit wrote for it (`hostconfig.rs`), through its own `Config::from_env`. dainty-restricted's host
//! composes the AGPL services until each is a library the core loads (RD27).

#[cfg(any(feature = "admin", feature = "discord"))]
use std::sync::Arc;

#[cfg(any(feature = "admin", feature = "discord"))]
use dainty_service_kit::env::Env;

#[cfg(any(feature = "admin", feature = "discord"))]
use crate::Module;

/// The modules built into this binary, for the message an empty build prints.
pub const BUILT_IN: &[&str] = &[
    #[cfg(feature = "admin")]
    "admin",
    #[cfg(feature = "discord")]
    "discord",
];

/// Every module the crate can build, for the same message.
pub const ALL: &[&str] = &["admin", "discord"];

/// The game modules compiled into this binary (DD3, DF5): the stub only here; dainty-restricted adds its own. A server's
/// `[modules]` section picks which of them load; the rest stay listed and unloaded.
#[cfg_attr(
    not(feature = "stub-module"),
    allow(unused_mut, reason = "only the stub module pushes")
)]
pub fn game_modules() -> Vec<crate::registry::ModuleFactory> {
    let mut modules: Vec<crate::registry::ModuleFactory> = Vec::new();
    #[cfg(feature = "stub-module")]
    modules.push(std::sync::Arc::new(|| {
        std::sync::Arc::new(dainty_module_api::stub::StubModule::new())
            as std::sync::Arc<dyn dainty_module_api::GameModule>
    }));
    modules
}

/// The admin API (`admin.env`), with the server's module registry when it has one (DF3), and host.toml's settings
/// the dashboard edits when the host kit hands them in.
#[cfg(feature = "admin")]
pub fn admin(
    env: &Env,
    registry: Option<crate::ModuleRegistry>,
    host: Option<Arc<dyn dainty_admin::host_settings::HostSettings>>,
) -> Result<Module, String> {
    let config = dainty_admin::config::Config::from_env(env)?;
    let (plugin, control, services) = match registry {
        Some(registry) => {
            let plugin = Arc::new(dainty_admin::modules::AdminPlugin::new(Arc::new(AdminRoute(
                registry.downgrade(),
            ))));
            registry.add_plugin(plugin.clone());
            let services = registry.node_services().clone();
            let control = dainty_admin::module_control::Modules(Some(Arc::new(AdminModules {
                registry,
                host,
                #[cfg(feature = "events")]
                events: crate::events::EventsReader::for_node(&config.database),
            })
                as Arc<dyn dainty_admin::module_control::ModuleControl>));
            (plugin, control, Some(services))
        }
        None => (
            Arc::new(dainty_admin::modules::AdminPlugin::none()),
            dainty_admin::module_control::Modules::default(),
            None,
        ),
    };
    Ok(Module::new(
        "admin",
        Arc::new(move |ctx| {
            let (config, plugin, control, services) =
                (config.clone(), plugin.clone(), control.clone(), services.clone());
            Box::pin(async move {
                let network: Arc<dyn dainty_admin::relay::AdminNetwork> = match &services {
                    Some(services) => Arc::new(NodeServicesNetwork(services.clone())),
                    None => Arc::new(dainty_admin::relay::NoNetwork),
                };
                let attach_services = services.clone();
                let on_ready = attach_services.map(|services| {
                    Box::new(move |admin: Arc<dainty_admin::schema::Admin>| {
                        services.attach(network_plugin::Service::Admin, admin.relay_service());
                    }) as Box<dyn FnOnce(Arc<dainty_admin::schema::Admin>) + Send>
                });
                let result =
                    dainty_admin::server::run_with_network(config, ctx, plugin, control, network, on_ready).await;
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
impl dainty_admin::relay::AdminNetwork for NodeServicesNetwork {
    fn local_node(&self) -> Option<dainty_module_api::network::NodeId> {
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
struct AdminModules {
    registry: crate::ModuleRegistry,
    host: Option<Arc<dyn dainty_admin::host_settings::HostSettings>>,
    /// The recorder's table, beside the node's database as the admin API's env file names it (A5)
    #[cfg(feature = "events")]
    events: crate::events::EventsReader,
}

#[cfg(feature = "admin")]
fn admin_module(info: crate::ModuleInfo) -> dainty_admin::module_control::Module {
    let text = |value: serde_json::Value| value.as_str().map(str::to_owned).unwrap_or_default();
    let manifest = info.manifest.as_ref();
    dainty_admin::module_control::Module {
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
            .map(|(name, range)| dainty_admin::module_control::ModulePort {
                name,
                first: range.first,
                last: range.last,
            })
            .collect(),
        commands: info.commands.into_iter().map(|c| c.name).collect(),
    }
}

#[cfg(feature = "admin")]
impl dainty_admin::module_control::ModuleControl for AdminModules {
    fn list(&self) -> Vec<dainty_admin::module_control::Module> {
        self.registry.list().into_iter().map(admin_module).collect()
    }

    fn get(&self, id: &str) -> Result<dainty_admin::module_control::Module, String> {
        let id = id
            .parse()
            .map_err(|e: dainty_module_api::ids::NameError| e.to_string())?;
        self.registry.info(&id).map(admin_module).map_err(|e| e.to_string())
    }

    fn load<'a>(
        &'a self,
        id: &'a str,
    ) -> dainty_admin::module_control::Pending<'a, dainty_admin::module_control::Module> {
        Box::pin(async move {
            let id = id
                .parse()
                .map_err(|e: dainty_module_api::ids::NameError| e.to_string())?;
            self.registry
                .load(&id)
                .await
                .map(admin_module)
                .map_err(|e| e.to_string())
        })
    }

    fn unload<'a>(
        &'a self,
        id: &'a str,
    ) -> dainty_admin::module_control::Pending<'a, dainty_admin::module_control::Module> {
        Box::pin(async move {
            let id = id
                .parse()
                .map_err(|e: dainty_module_api::ids::NameError| e.to_string())?;
            self.registry.unload(&id).await.map_err(|e| e.to_string())?;
            self.registry.info(&id).map(admin_module).map_err(|e| e.to_string())
        })
    }

    fn restart<'a>(
        &'a self,
        id: &'a str,
    ) -> dainty_admin::module_control::Pending<'a, dainty_admin::module_control::Module> {
        Box::pin(async move {
            let id = id
                .parse()
                .map_err(|e: dainty_module_api::ids::NameError| e.to_string())?;
            self.registry
                .restart(&id)
                .await
                .map(admin_module)
                .map_err(|e| e.to_string())
        })
    }

    fn host_settings(&self) -> Option<Arc<dyn dainty_admin::host_settings::HostSettings>> {
        self.host.clone()
    }

    fn game<'a>(
        &'a self,
        version: &'a dainty_module_api::GameVersion,
        call: dainty_module_api::GameCall,
    ) -> dainty_module_api::BoxFuture<'a, Result<dainty_module_api::GameAnswer, dainty_module_api::ModuleError>> {
        Box::pin(self.registry.game(version, call))
    }

    fn game_servers(&self) -> dainty_module_api::BoxFuture<'_, Vec<dainty_admin::game::GameServers>> {
        Box::pin(async move {
            self.registry
                .game_servers()
                .await
                .into_iter()
                .map(|g| dainty_admin::game::GameServers {
                    module: g.module,
                    version: g.version,
                    servers: g.servers,
                })
                .collect()
        })
    }

    #[cfg(feature = "events")]
    fn events(
        &self,
        after: i64,
        names: Vec<String>,
        limit: i64,
    ) -> dainty_module_api::BoxFuture<'_, Result<Vec<dainty_admin::events::KeptEvent>, dainty_module_api::ModuleError>>
    {
        Box::pin(async move {
            let kept = self.events.list(after, &names, limit).await.map_err(|e| {
                tracing::warn!(error = %e, "the events database did not answer");
                dainty_module_api::ModuleError::new(dainty_module_api::ErrorKind::Unavailable, e)
            })?;
            Ok(kept
                .into_iter()
                .map(|e| dainty_admin::events::KeptEvent {
                    seq: e.seq,
                    at_ms: e.at_ms,
                    name: e.name,
                    source: e.source,
                    payload: e.payload,
                })
                .collect())
        })
    }
}

/// The modules' calls, as the admin API makes them (DF4c): to the loaded module, through its driver.
#[cfg(feature = "admin")]
struct AdminRoute(crate::WeakRegistry);

#[cfg(feature = "admin")]
impl dainty_admin::modules::AdminCalls for AdminRoute {
    fn admin<'a>(
        &'a self,
        module: &'a dainty_module_api::ModuleId,
        call: dainty_module_api::AdminCall,
    ) -> dainty_module_api::BoxFuture<'a, Result<dainty_module_api::AdminResponse, dainty_module_api::ModuleError>>
    {
        Box::pin(async move {
            let Some(registry) = self.0.upgrade() else {
                return Err(dainty_module_api::ModuleError::new(
                    dainty_module_api::ErrorKind::Unavailable,
                    "the server is stopping",
                ));
            };
            registry.admin_for(module, call).await
        })
    }
}

/// The Discord bot (`discord.env`, module sharing A7): `/key` and `/status` through the registry's game calls to
/// the 1.14.7 module, each `/key`'s outcome on the bus for the recorder (D6). `invite` is this server's invite,
/// whose code the bot replaces; the snapshot's `invite_url` wins once one is set.
#[cfg(feature = "discord")]
pub fn discord(
    env: &Env,
    registry: &crate::ModuleRegistry,
    invite: Option<dainty_protocol::invite::Invite>,
) -> Result<Module, String> {
    let config = dainty_discord::Config::from_env(env)?;
    let access: Arc<dyn dainty_discord::GameAccess> = Arc::new(DiscordAccess {
        registry: registry.downgrade(),
        status: registry.status_registry().clone(),
        invite,
    });
    Ok(Module::new(
        "discord",
        Arc::new(move |ctx| {
            let (config, access) = (config.clone(), access.clone());
            Box::pin(dainty_discord::run(config, ctx, access))
        }),
    )
    .optional())
}

/// The registry as the bot reaches it; weak, so the bot never keeps a stopping server alive.
#[cfg(feature = "discord")]
struct DiscordAccess {
    registry: crate::WeakRegistry,
    status: dainty_service_kit::context::StatusRegistry,
    invite: Option<dainty_protocol::invite::Invite>,
}

#[cfg(feature = "discord")]
impl dainty_discord::GameAccess for DiscordAccess {
    fn game(
        &self,
        call: dainty_module_api::GameCall,
    ) -> dainty_module_api::BoxFuture<'_, Result<dainty_module_api::GameAnswer, dainty_module_api::ModuleError>> {
        Box::pin(async move {
            let Some(registry) = self.registry.upgrade() else {
                return Err(dainty_module_api::UnavailableReason::ModuleNotReady.error());
            };
            registry.game(&dainty_protocol::invite::dainty_version(), call).await
        })
    }

    fn emit(&self, event: dainty_module_api::Event) {
        if let Some(registry) = self.registry.upgrade() {
            registry.emit(event);
        }
    }

    fn invite(&self) -> Option<dainty_protocol::invite::Invite> {
        self.status
            .snapshot()
            .invite_url
            .and_then(|url| dainty_protocol::invite::parse_invite(&url).ok())
            .or_else(|| self.invite.clone())
    }
}

/// What [`remote_controller`] runs with.
#[cfg(feature = "remote")]
pub struct RemoteControllerOptions {
    /// The enrolled installs and the enrollment tokens they redeem.
    pub state: std::sync::Arc<std::sync::Mutex<dainty_remote::RemoteControllerState>>,
    /// Where the state is saved after each enrollment and reread before the next; `None` keeps it in memory.
    pub state_file: Option<std::path::PathBuf>,
    /// The cluster CA that signs each enrolled install's certificate.
    pub ca: std::sync::Arc<network_plugin::cluster::ClusterCa>,
    /// Where the module attaches `Service::Remote` and the enroll handler.
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
                let enrolled_nodes: Vec<dainty_module_api::network::NodeId> = {
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
                            let req = match dainty_remote::decode_enroll_request(&body) {
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
                                    && let Ok(fresh) = dainty_remote::RemoteControllerState::load(path)
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
                                    match dainty_remote::encode_enroll_answer(&ans) {
                                        Ok(bytes) => (200, bytes),
                                        Err(e) => {
                                            let err = serde_json::json!({ "error": e.to_string() });
                                            (500, err.to_string().into_bytes())
                                        }
                                    }
                                }
                                Err(e) => {
                                    let status = match e {
                                        dainty_remote::RemoteError::InvalidEnrollmentToken
                                        | dainty_remote::RemoteError::EnrollmentTokenUsed
                                        | dainty_remote::RemoteError::EnrollmentTokenExpired => 403,
                                        _ => 400,
                                    };
                                    let err = serde_json::json!({ "error": e.to_string() });
                                    (status, err.to_string().into_bytes())
                                }
                            }
                        })
                    });

                services.set_enroll_handler(Some(enroll_cb));
                let svc = std::sync::Arc::new(dainty_remote::RemoteControllerService::new(state, state_file));
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

/// What [`remote_managed`] runs with.
#[cfg(feature = "remote")]
pub struct RemoteManagedOptions {
    /// This install's enrollment: the controller it answers to and the sync state.
    pub agent: std::sync::Arc<std::sync::Mutex<dainty_remote::RemoteManagedAgent>>,
    /// Carries out what the controller pushes or sends back on a sync.
    pub handler: std::sync::Arc<dyn dainty_remote::RemoteHostHandler>,
    /// The node link the sync reaches the controller over.
    pub link: std::sync::Arc<dyn network_plugin::PeerLink>,
    /// Where the module attaches `Service::Remote`.
    pub services: crate::NodeServices,
    /// How long between syncs while the controller answers; at least one second.
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
                let svc = std::sync::Arc::new(dainty_remote::RemoteManagedService::new(agent.clone(), handler.clone()));
                services.attach(network_plugin::Service::Remote, svc);
                let pending_outcomes =
                    std::sync::Arc::new(std::sync::Mutex::new(Vec::<dainty_remote::CommandOutcome>::new()));
                let loop_agent = agent.clone();
                let loop_handler = handler.clone();
                let loop_link = link.clone();
                let loop_outcomes = pending_outcomes.clone();
                let sync_task = tokio::spawn(async move {
                    loop {
                        let res = dainty_remote::sync_once_with_controller(
                            loop_link.as_ref(),
                            &loop_agent,
                            loop_handler.as_ref(),
                            &loop_outcomes,
                        )
                        .await;
                        let wait_ms = match res {
                            Ok(_) => sync_interval.as_millis() as u64,
                            Err(dainty_remote::RemoteError::RevokedPeer(_)) => break,
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
                                dainty_remote::offline_backoff_delay_ms(failures)
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

/// One half of a split server's tunnel (`docs/split-roles.md`).
pub struct TunnelOptions {
    /// Where the tunnel's service is attached.
    pub services: crate::NodeServices,
    /// The other half
    pub peer: dainty_module_api::network::NodeId,
    /// The port the other half runs, listened on here on loopback
    pub listen: (network_plugin::tunnel::Port, u16),
    /// The port this half runs, which the other half reaches through the link
    pub expose: (network_plugin::tunnel::Port, u16),
}

/// The split server's tunnel (`Service::Tunnel`): this half's loopback listener for the other half's port, and the
/// node service that replays the other half's calls on this half's own.
pub fn tunnel(options: TunnelOptions) -> crate::Module {
    use network_plugin::tunnel::{TunnelClient, TunnelService, serve};
    use std::sync::Arc;

    let options = Arc::new(options);
    crate::Module::new(
        "tunnel",
        Arc::new(move |mut ctx| {
            let options = Arc::clone(&options);
            Box::pin(async move {
                let (port, number) = options.listen;
                let listener = tokio::net::TcpListener::bind(("127.0.0.1", number))
                    .await
                    .map_err(|e| format!("tunnel to the {} on 127.0.0.1:{number}: {e}", port.as_str()))?;
                let lookup = options.services.clone();
                let client = Arc::new(TunnelClient {
                    link: Arc::new(move || lookup.peer_link()),
                    peer: options.peer.clone(),
                    port,
                });
                options.services.attach(
                    network_plugin::Service::Tunnel,
                    Arc::new(TunnelService::new(options.peer.clone(), vec![options.expose])),
                );
                let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
                let server = tokio::spawn(serve(listener, client, async {
                    let _ = stop_rx.await;
                }));
                ctx.status.report_ready(
                    Some(number),
                    format!("the {} on {}, over the node link", port.as_str(), options.peer),
                );
                ctx.wait_stop().await;
                ctx.status.stopping();
                let _ = stop_tx.send(());
                let _ = server.await;
                options.services.detach(network_plugin::Service::Tunnel);
                ctx.status.stopped();
                Ok(())
            })
        }),
    )
    .ready_within(std::time::Duration::from_secs(10))
}

#[cfg(all(test, feature = "admin", feature = "events"))]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use std::collections::BTreeMap;

    use dainty_admin::module_control::ModuleControl;
    use dainty_module_api::{GameAnswer, GameCall, GameModule, GameVersion};
    use dainty_service_kit::context::{ComponentPhase, StatusRegistry};

    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn the_admin_api_reaches_the_game_modules_and_the_kept_events() {
        let dir = tempfile::tempdir().unwrap();
        let node = dainty_store::DatabaseConfig::Sqlite {
            file: dir.path().join("dainty.db"),
        };
        let status = StatusRegistry::new("private", Vec::new());
        let registry = crate::ModuleRegistry::new(crate::RegistryOptions::standalone(), status.clone());
        let recorder = crate::events::module(
            &registry,
            crate::events::EventRecorder::database_for(&node),
            crate::events::EventsOptions::default(),
        );
        let (ctx, stop) = status.service_context("events");
        let recording = tokio::spawn((recorder.start)(ctx));
        // Both opening one SQLite file at once can find it locked; a reader asks again on its next read
        for _ in 0..100 {
            if status.snapshot().get("events").map(|c| c.phase) == Some(ComponentPhase::Ready) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        registry
            .add_entry(crate::ModuleEntry {
                id: "stub".parse().unwrap(),
                source: crate::ModuleSource::CompiledIn(Arc::new(|| {
                    Arc::new(dainty_module_api::stub::StubModule::new()) as Arc<dyn GameModule>
                })),
                autoload: true,
                settings: BTreeMap::new(),
            })
            .unwrap();
        registry.load(&"stub".parse().unwrap()).await.unwrap();
        let admin = AdminModules {
            registry: registry.clone(),
            host: None,
            events: crate::events::EventsReader::for_node(&node),
        };

        let stub: GameVersion = "0.0.1".parse().unwrap();
        match admin.game(&stub, GameCall::Availability).await.unwrap() {
            GameAnswer::Availability(found) => assert!(found.up),
            other => panic!("{other:?}"),
        }
        let every = admin.game_servers().await;
        assert_eq!(every.len(), 1);
        assert_eq!(every[0].version, stub);
        assert_eq!(every[0].servers.as_ref().unwrap().len(), 1, "the stub's hub");

        registry.emit(dainty_module_api::Event {
            name: dainty_module_api::EventName::new("server.ready").unwrap(),
            payload: serde_json::json!({ "n": 1 }),
        });
        let mut kept = Vec::new();
        for _ in 0..50 {
            kept = admin.events(0, vec!["server.ready".into()], 10).await.unwrap();
            if !kept.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].payload, serde_json::json!({ "n": 1 }));
        assert_eq!(kept[0].source, None);
        assert!(
            admin
                .events(kept[0].seq, vec!["server.ready".into()], 10)
                .await
                .unwrap()
                .is_empty()
        );

        registry.unload_all().await;
        stop.send(true).unwrap();
        recording.await.unwrap().unwrap();
    }
}
