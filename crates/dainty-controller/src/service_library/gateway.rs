// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The core's side of the gateway service (RD27, `dainty_service_plugin::gateway`), loaded as a library or compiled in: what
//! the host handed the gateway and the allowlist in-process, as messages. Before each attach the service is told
//! where the module's ports were placed (it can't ask the registry back while it answers the attach); its node links
//! are bound to this registry's [`NodeServices`](crate::NodeServices) through [`GatewayLinks`]; and it asks the core
//! for each admitted peer call, each enrollment and each region report. Only control traffic crosses: the service
//! keeps TLS, the policy and the proxying to backends (docs/security/gateway.md in dainty-restricted).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError, Weak};

use dainty_module_api::manifest::ModuleManifest;
use dainty_module_api::network::{GlobalId, NodeId, NodeIdentity};
use dainty_module_api::{ModuleDriver, ModuleId};
use dainty_service_kit::env::Env;
use dainty_service_plugin::gateway::{Bytes, FromGateway, NodeFailure, Parts, RegionRtt, ToGateway, split};
use dainty_service_plugin::{FromService, ToService};
use network_plugin::{LinkError, NodeEndpoint, NodeService, Service};

pub use dainty_service_plugin::gateway::AllowlistMode;

use super::{Extension, RunLink, ServiceLibrary, service_compiled_in_with, service_library_with};
use crate::node_services::{GatewayLinks, RemoteEnrollCallback};
use crate::{LoadHook, Module, ModuleRegistry, WeakRegistry};

/// How the core composes the gateway, sent before each start.
#[derive(Debug, Clone)]
pub struct GatewayOptions {
    /// Whether and where the allowlist runs, which decides how modules' UDP ports are guarded.
    pub allowlist: AllowlistMode,
    /// The chat listener's port, when the core runs chat on the backend's
    pub ws_port: Option<u16>,
}

/// The gateway's core side: the ring's allowlist sink, for the network module.
#[derive(Clone)]
pub struct GatewayHandle(Arc<Core>);

/// The module that runs the gateway library on `gateway.env`, registered with `registry` as the core plugin for
/// routes and UDP ranges, and the handle the network module takes the allowlist from.
pub fn gateway_library(
    library: ServiceLibrary,
    env: Env,
    registry: Option<&ModuleRegistry>,
    options: GatewayOptions,
) -> (Module, GatewayHandle) {
    let core = core(registry, options);
    let module = service_library_with(library, env, registry, Some(Arc::new(Ext(core.clone()))));
    (module, GatewayHandle(core))
}

/// [`gateway_library`] for the same service compiled in: `make` builds what the library exports, and it runs over the
/// same protocol with the same core side (RD27).
pub fn gateway_compiled_in<S: dainty_service_plugin::Service>(
    make: impl Fn() -> S + Send + Sync + 'static,
    env: Env,
    registry: Option<&ModuleRegistry>,
    options: GatewayOptions,
) -> (Module, GatewayHandle) {
    let core = core(registry, options);
    let module = service_compiled_in_with(make, env, registry, Some(Arc::new(Ext(core.clone()))));
    (module, GatewayHandle(core))
}

fn core(registry: Option<&ModuleRegistry>, options: GatewayOptions) -> Arc<Core> {
    let core = Arc::new(Core {
        registry: registry.map(ModuleRegistry::downgrade),
        options,
        run: Mutex::default(),
        parts: Mutex::default(),
        links: Mutex::default(),
    });
    if let Some(registry) = registry {
        registry.add_hook(Arc::new(Hook(Arc::downgrade(&core))));
    }
    core
}

impl GatewayHandle {
    /// Players the ring admits go to the gateway's allowlist (DN8, DN9).
    pub fn allowlist_sink(&self) -> Arc<dyn network_plugin::ring::AllowlistSink> {
        Arc::new(Sink(Arc::downgrade(&self.0)))
    }
}

struct Core {
    registry: Option<WeakRegistry>,
    options: GatewayOptions,
    /// The open instance, and a runtime of the core's for the answers that wait
    run: Mutex<Option<(RunLink, tokio::runtime::Handle)>>,
    parts: Mutex<Parts>,
    links: Mutex<Option<Arc<Links>>>,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Core {
    fn send(&self, message: ToGateway) -> bool {
        let run = lock(&self.run).as_ref().map(|(run, _)| run.clone());
        run.map(|run| run.send(&ToService::Gateway { message })).is_some()
    }

    fn send_body(&self, id: u64, body: Vec<u8>, end: impl FnOnce(Bytes) -> ToGateway) {
        let (parts, last) = split(body);
        for data in parts {
            self.send(ToGateway::Part { id, data });
        }
        self.send(end(last));
    }

    fn registry(&self) -> Option<ModuleRegistry> {
        self.registry.as_ref()?.upgrade()
    }

    /// `module`'s ports as the controller placed them.
    fn placed(&self, module: &ModuleManifest, running: bool) -> Option<ToService> {
        let registry = self.registry()?;
        let ports: BTreeMap<String, dainty_module_api::PortRange> = module
            .ports
            .iter()
            .filter_map(|port| Some((port.name.clone(), registry.assigned(&module.id, &port.name)?)))
            .collect();
        Some(ToService::Gateway {
            message: ToGateway::Placed {
                module: module.id.clone(),
                ports,
                running,
            },
        })
    }

    /// Runs `answer` on the core's runtime, off the reader thread.
    fn spawn(self: &Arc<Self>, answer: impl Future<Output = ()> + Send + 'static) {
        let handle = lock(&self.run).as_ref().map(|(_, handle)| handle.clone());
        if let Some(handle) = handle {
            handle.spawn(answer);
        }
    }

    fn services(&self) -> Option<crate::NodeServices> {
        self.registry().map(|registry| registry.node_services().clone())
    }

    fn take(&self, id: u64, last: Bytes) -> Option<Vec<u8>> {
        lock(&self.parts).finish(id, last)
    }
}

/// The core's [`Extension`] for the gateway.
struct Ext(Arc<Core>);

impl Extension for Ext {
    fn opened(&self, run: RunLink) -> Vec<ToService> {
        self.0.opened(run)
    }

    fn closed(&self) {
        self.0.closed();
    }

    fn before_attach(&self, module: &ModuleManifest) -> Vec<ToService> {
        self.0.before_attach(module)
    }

    fn message(&self, message: FromService) -> Result<(), String> {
        let FromService::Gateway { message } = message else {
            return Err(String::from("a message this service may not send"));
        };
        self.0.message(message);
        Ok(())
    }
}

impl Core {
    fn opened(&self, run: RunLink) -> Vec<ToService> {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return Vec::new();
        };
        *lock(&self.run) = Some((run, handle));
        let allowlist = self.options.allowlist.clone();
        vec![ToService::Gateway {
            message: ToGateway::Configure {
                registry: self.registry.is_some(),
                allowlist,
                ws_port: self.options.ws_port,
            },
        }]
    }

    fn closed(&self) {
        lock(&self.run).take();
        *lock(&self.parts) = Parts::default();
        if lock(&self.links).take().is_some()
            && let Some(services) = self.services()
        {
            services.set_gateway(None);
        }
    }

    fn before_attach(&self, module: &ModuleManifest) -> Vec<ToService> {
        let running = self
            .registry()
            .is_some_and(|r| module.ports.iter().any(|p| r.port(&module.id, &p.name).is_some()));
        self.placed(module, running).into_iter().collect()
    }

    fn message(self: &Arc<Self>, message: FromGateway) {
        match message {
            FromGateway::NodeLinks { identity } => self.bind(identity),
            FromGateway::Part { id, data } => lock(&self.parts).push(id, data),
            FromGateway::NodeCall {
                id,
                from,
                service,
                body,
            } => {
                let body = self.take(id, body);
                let core = self.clone();
                self.spawn(async move {
                    let result = match (body, Service::from_name(&service), core.services()) {
                        (None, _, _) => Err(NodeFailure::TooLarge),
                        (_, None, _) | (_, _, None) => Err(NodeFailure::NoService),
                        (Some(body), Some(service), Some(services)) => {
                            services.serve(&from, service, body).await.map_err(|e| failure(&e))
                        }
                    };
                    match result {
                        Ok(answer) => core.send_body(id, answer, |last| ToGateway::NodeAnswer { id, result: Ok(last) }),
                        Err(failure) => {
                            _ = core.send(ToGateway::NodeAnswer {
                                id,
                                result: Err(failure),
                            })
                        }
                    }
                });
            }
            FromGateway::Enroll { id, body } => {
                let body = self.take(id, body);
                let core = self.clone();
                self.spawn(async move {
                    let handler = core.services().and_then(|s| s.enroll_handler());
                    let (status, answer) = match (body, handler) {
                        (Some(body), Some(handler)) => handler(body).await,
                        (None, _) => (413, br#"{"error":"too_large"}"#.to_vec()),
                        (_, None) => (404, br#"{"error":"not_found"}"#.to_vec()),
                    };
                    core.send_body(id, answer, |body| ToGateway::Enrolled { id, status, body });
                });
            }
            FromGateway::RegionReport {
                id,
                api_key,
                ticket,
                regions,
            } => {
                let core = self.clone();
                self.spawn(async move {
                    let accepted = match core.services() {
                        Some(services) => region_report(&services, api_key, ticket, regions).await,
                        None => true,
                    };
                    core.send(ToGateway::RegionAnswer { id, accepted });
                });
            }
        }
    }
}

impl Core {
    /// The service's node links are up: this registry's node services answer them.
    fn bind(self: &Arc<Self>, identity: NodeIdentity) {
        let Some(services) = self.services() else {
            return;
        };
        let links = Arc::new(Links {
            core: Arc::downgrade(self),
            endpoint: NodeEndpoint::new(identity, std::iter::empty()),
        });
        *lock(&self.links) = Some(links.clone());
        services.set_gateway(Some(links));
    }
}

fn failure(error: &LinkError) -> NodeFailure {
    match error {
        LinkError::TooLarge => NodeFailure::TooLarge,
        LinkError::NoService { .. } => NodeFailure::NoService,
        LinkError::Refused { reason, .. } => NodeFailure::Refused { reason: reason.clone() },
        other => NodeFailure::Failed {
            reason: other.to_string(),
        },
    }
}

/// The service's node links as [`NodeServices`](crate::NodeServices) binds them. The endpoint that admits peers is
/// the service's; this one only holds the identity it reported. Calls are answered by the registry's node services
/// whatever is attached here, so `attach` sends nothing.
struct Links {
    core: Weak<Core>,
    endpoint: NodeEndpoint,
}

impl GatewayLinks for Links {
    fn endpoint(&self) -> &NodeEndpoint {
        &self.endpoint
    }

    fn set_enroll_handler(&self, handler: Option<RemoteEnrollCallback>) {
        if let Some(core) = self.core.upgrade() {
            core.send(ToGateway::Enroll {
                present: handler.is_some(),
            });
        }
    }

    fn attach(&self, _services: Arc<dyn NodeService>) {}

    fn add_peer(&self, node: NodeId) {
        if let Some(core) = self.core.upgrade() {
            core.send(ToGateway::AddPeer { node });
        }
    }

    fn remove_peer(&self, node: &NodeId) {
        if let Some(core) = self.core.upgrade() {
            core.send(ToGateway::RemovePeer { node: node.clone() });
        }
    }
}

/// A module that started is reachable at its ports from now on; one that went is not.
struct Hook(Weak<Core>);

impl LoadHook for Hook {
    fn loaded(&self, manifest: &ModuleManifest, _driver: Arc<dyn ModuleDriver>) {
        if let Some(core) = self.0.upgrade()
            && let Some(ToService::Gateway { message }) = core.placed(manifest, true)
        {
            core.send(message);
        }
    }

    fn unloaded(&self, module: &ModuleId) {
        if let Some(core) = self.0.upgrade() {
            core.send(ToGateway::Stopped { module: module.clone() });
        }
    }
}

struct Sink(Weak<Core>);

impl network_plugin::ring::AllowlistSink for Sink {
    fn admit_player(&self, instance: &GlobalId, _account: &GlobalId, client_ip: &str, udp_port: u16) {
        let sent = self.0.upgrade().is_some_and(|core| {
            core.send(ToGateway::AdmitUdp {
                ip: client_ip.to_owned(),
                port: udp_port,
            })
        });
        if !sent {
            tracing::warn!(
                target: "host",
                instance = %instance,
                udp_port,
                "no gateway runs to admit the player's UDP; game port stays closed"
            );
        }
    }
}

/// `POST /network/v1/regions` into the ring when the network module runs it, or accepted shape-checked when it
/// doesn't (DN7, DN8): compose.rs's `region_report_handler`, on this side of the service.
#[cfg(feature = "network")]
async fn region_report(
    services: &crate::NodeServices,
    api_key: Option<String>,
    ticket: Option<String>,
    regions: Vec<RegionRtt>,
) -> bool {
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
    let parsed = match ticket.as_deref() {
        Some(token) => match network_plugin::home::ticket::session_ticket_from_token(token) {
            Ok(pair) => Some(pair),
            Err(_) => return false,
        },
        None => None,
    };
    let ticket = parsed.as_ref().map(|(t, g)| (t, g));
    ring.record_region_pings_authenticated(api_key.as_deref(), ticket, pings)
        .await
        .is_ok()
}

#[cfg(not(feature = "network"))]
async fn region_report(
    _services: &crate::NodeServices,
    _api_key: Option<String>,
    _ticket: Option<String>,
    _regions: Vec<RegionRtt>,
) -> bool {
    true
}
