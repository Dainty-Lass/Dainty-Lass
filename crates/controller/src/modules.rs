// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The MIT services as modules (admin, remote), and the game modules compiled in: each reads the env file the
//! host kit wrote for it (`hostconfig.rs`), through its own `Config::from_env`. dainty-restricted's host
//! composes the AGPL services until each is a library the core loads (RD27).

#[cfg(feature = "admin")]
use std::sync::Arc;

#[cfg(feature = "admin")]
use service_kit::env::Env;

#[cfg(feature = "admin")]
use crate::Module;

/// The modules built into this binary, for the message an empty build prints.
pub const BUILT_IN: &[&str] = &[
    #[cfg(feature = "admin")]
    "admin",
];

/// Every module the crate can build, for the same message.
pub const ALL: &[&str] = &["admin"];

/// The game modules compiled into this binary (DD3, DF5): the stub only here; dainty-restricted adds its own. A server's
/// `[modules]` section picks which of them load; the rest stay listed and unloaded.
#[cfg_attr(not(feature = "stub-module"), allow(unused_mut))]
pub fn game_modules() -> Vec<crate::registry::ModuleFactory> {
    let mut modules: Vec<crate::registry::ModuleFactory> = Vec::new();
    #[cfg(feature = "stub-module")]
    modules.push(std::sync::Arc::new(|| {
        std::sync::Arc::new(module_api::stub::StubModule::new()) as std::sync::Arc<dyn module_api::GameModule>
    }));
    modules
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

/// One half of a split server's tunnel (`docs/split-roles.md`).
pub struct TunnelOptions {
    pub services: crate::NodeServices,
    /// The other half
    pub peer: module_api::network::NodeId,
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
