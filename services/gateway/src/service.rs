// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The gateway and the allowlist as one [`service_plugin::Service`] (RD27): what the gateway library and
//! restricted's compiled-in path both run. Only control traffic crosses the seam (`service_plugin::gateway`):
//! where modules' ports are, admits, peers, and the calls only the core can answer.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use module_api::manifest::ModuleManifest;
use module_api::network::NodeId;
use module_api::registrar::Registration;
use module_api::{BoxFuture, CoreCapability, ModuleError, ModuleId, PortRange};
use network_plugin::link::{LinkError, NodeService, Service as LinkService};
use serde_json::json;
use service_kit::context::{ServiceContext, StatusReporter};
use service_kit::env::Env;
use service_kit::linelog;
use service_plugin::gateway::{AllowlistMode, Bytes, FromGateway, NodeFailure, Outbox, Parts, ToGateway};
use tokio::sync::{oneshot, watch};

use crate::allowlist::config::AllowlistConfig;
use crate::allowlist::guard::PortGuard;
use crate::allowlist::helper::{HelperSink, HelperSlot, run_with_guard};
use crate::allowlist::modules::{UdpMode, UdpPorts};
use crate::config::Config;
use crate::modules::ModuleRoutes;
use crate::network::{RegionHandler, RegionReport};
use crate::nodelink::{EnrollHandler, NodeLinks};
use crate::server::{NodeLinkSlot, load_capture, run_with_network};

/// How long a call waits for the core.
const CORE_ANSWER: Duration = Duration::from_secs(60);
/// How long the in-process helper has to be ready, so the game ports close before any game server listens.
const HELPER_READY: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
struct Configured {
    registry: bool,
    allowlist: AllowlistMode,
    ws_port: Option<u16>,
}

enum Answer {
    Node(Result<Vec<u8>, NodeFailure>),
    Enrolled(u16, Vec<u8>),
    Region(bool),
}

#[derive(Default)]
struct Calls {
    next: AtomicU64,
    open: Mutex<HashMap<u64, oneshot::Sender<Option<Answer>>>>,
    parts: Mutex<Parts>,
}

impl Calls {
    fn open(&self) -> (u64, oneshot::Receiver<Option<Answer>>) {
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = oneshot::channel();
        self.open.lock().unwrap_or_else(PoisonError::into_inner).insert(id, tx);
        (id, rx)
    }

    fn close(&self, id: u64, answer: Option<Answer>) {
        if let Some(tx) = self.open.lock().unwrap_or_else(PoisonError::into_inner).remove(&id) {
            _ = tx.send(answer);
        }
    }

    fn forget(&self, id: u64) {
        self.open.lock().unwrap_or_else(PoisonError::into_inner).remove(&id);
    }

    fn finish(&self, id: u64, last: Bytes) -> Option<Vec<u8>> {
        self.parts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .finish(id, last)
    }
}

struct Inner {
    configured: Mutex<Option<Configured>>,
    outbox: Mutex<Option<Outbox>>,
    routes: Arc<ModuleRoutes>,
    udp: UdpPorts,
    placed: Mutex<BTreeMap<ModuleId, BTreeMap<String, PortRange>>>,
    helper: HelperSlot,
    links: NodeLinkSlot,
    /// Peers added and removed before the links were up, replayed once they are
    peers: Mutex<Vec<(NodeId, bool)>>,
    enroll: AtomicBool,
    calls: Calls,
}

/// The gateway library's service.
#[derive(Clone)]
pub struct GatewayService(Arc<Inner>);

impl Default for GatewayService {
    fn default() -> Self {
        Self::new()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl GatewayService {
    pub fn new() -> Self {
        Self(Arc::new(Inner {
            configured: Mutex::default(),
            outbox: Mutex::default(),
            routes: Arc::new(ModuleRoutes::new()),
            udp: UdpPorts::new(UdpMode::Off),
            placed: Mutex::default(),
            helper: HelperSlot::default(),
            links: NodeLinkSlot::default(),
            peers: Mutex::default(),
            enroll: AtomicBool::new(false),
            calls: Calls::default(),
        }))
    }

    pub fn routes(&self) -> &Arc<ModuleRoutes> {
        &self.0.routes
    }

    fn outbox(&self) -> Option<Outbox> {
        lock(&self.0.outbox).clone()
    }

    /// Sends a call to the core and waits for its answer.
    async fn call(&self, message: impl FnOnce(u64) -> Option<bool>) -> Option<Answer> {
        let (id, rx) = self.0.calls.open();
        if message(id) != Some(true) {
            self.0.calls.forget(id);
            return None;
        }
        match tokio::time::timeout(CORE_ANSWER, rx).await {
            Ok(Ok(answer)) => answer,
            _ => {
                self.0.calls.forget(id);
                None
            }
        }
    }

    fn enroll_handler(&self) -> EnrollHandler {
        let service = self.clone();
        Arc::new(move |body: Vec<u8>| {
            let service = service.clone();
            Box::pin(async move {
                let outbox = service.outbox();
                let answer = service
                    .call(|id| outbox.map(|o| o.send_body(id, body, |body| FromGateway::Enroll { id, body })))
                    .await;
                match answer {
                    Some(Answer::Enrolled(status, body)) => (status, body),
                    _ => (503, br#"{"error":"unavailable"}"#.to_vec()),
                }
            }) as BoxFuture<'static, (u16, Vec<u8>)>
        })
    }

    fn region_handler(&self) -> RegionHandler {
        let service = self.clone();
        Arc::new(move |report: RegionReport| {
            let service = service.clone();
            Box::pin(async move {
                let outbox = service.outbox();
                let answer = service
                    .call(|id| {
                        outbox.map(|o| {
                            o.send(FromGateway::RegionReport {
                                id,
                                api_key: report.api_key,
                                ticket: report.ticket,
                                regions: report.regions,
                            })
                        })
                    })
                    .await;
                matches!(answer, Some(Answer::Region(true)))
            }) as BoxFuture<'static, bool>
        })
    }

    /// Once the links are up: tell the core, answer peers through it, replay peers and the enroll handler.
    fn links_up(&self, links: &Arc<NodeLinks>) {
        links.endpoint().attach(Arc::new(CoreNodes(self.clone())));
        for (node, add) in lock(&self.0.peers).drain(..) {
            if add {
                links.endpoint().add_peer(node);
            } else {
                links.endpoint().remove_peer(&node);
            }
        }
        if self.0.enroll.load(Ordering::Relaxed) {
            links.set_enroll(Some(self.enroll_handler()));
        }
        if let Some(outbox) = self.outbox() {
            outbox.send(FromGateway::NodeLinks {
                identity: links.identity().clone(),
            });
        }
    }

    fn links(&self) -> Option<Arc<NodeLinks>> {
        lock(&self.0.links).clone()
    }

    fn udp_ranges(&self, module: &ModuleId, registration: &Registration) -> Vec<PortRange> {
        let placed = lock(&self.0.placed);
        registration
            .udp_ports
            .iter()
            .filter_map(|name| placed.get(module).and_then(|ports| ports.get(name)).copied())
            .collect()
    }

    async fn run_inner(self, env: Env, ctx: ServiceContext) -> Result<(), String> {
        let Some(configured) = lock(&self.0.configured).clone() else {
            return Err("the core did not configure the gateway before its start".to_owned());
        };
        let mut config = Config::from_env(&env).map_err(|e| format!("Configuration: {e}"))?;
        if let Some(port) = configured.ws_port
            && config.ws.port == config.backend.port
        {
            config.ws.port = port;
        }
        let capture = load_capture(&env)?;
        let (helper_stop, helper_stopped) = watch::channel(false);
        let mut helper_task = None;
        let mut sink = None;
        match &configured.allowlist {
            AllowlistMode::Off => self.0.udp.set_mode(UdpMode::Off),
            AllowlistMode::External { base } => self.0.udp.set_mode(UdpMode::External(*base)),
            AllowlistMode::Absent => self.0.udp.set_mode(UdpMode::Absent),
            AllowlistMode::InProcess { env } => {
                let pairs = Env::from_pairs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
                let settings = AllowlistConfig::from_env(&pairs).and_then(|c| Ok((PortGuard::new(&c.ports)?, c)));
                let (guard, settings) = match settings {
                    Ok((guard, settings)) => (Arc::new(guard), settings),
                    Err(e) => {
                        self.0.udp.set_mode(UdpMode::Absent);
                        return Err(format!("allowlist: Configuration: {e}"));
                    }
                };
                self.0.udp.set_mode(UdpMode::InProcess(guard.clone()));
                let helper_ctx =
                    ServiceContext::new("allowlist", StatusReporter::noop("allowlist"), helper_stopped.clone());
                let slot = self.0.helper.clone();
                let mut task = tokio::spawn(run_with_guard(settings, helper_ctx, Some(slot.clone()), guard));
                let ready = async {
                    loop {
                        if let Some(helper) = lock(&slot).clone() {
                            return Ok(helper);
                        }
                        if task.is_finished() {
                            return Err(match (&mut task).await {
                                Ok(Err(e)) => e,
                                _ => "the allowlist stopped".to_owned(),
                            });
                        }
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                };
                let helper = match tokio::time::timeout(HELPER_READY, ready).await {
                    Ok(Ok(helper)) => helper,
                    Ok(Err(e)) => return Err(format!("allowlist: {e}")),
                    Err(_) => {
                        helper_stop.send_replace(true);
                        return Err("allowlist: not ready within 60 s".to_owned());
                    }
                };
                sink = Some(Arc::new(HelperSink(Arc::downgrade(&helper))) as Arc<dyn crate::feed::InProcessSink>);
                helper_task = Some(task);
            }
        }
        self.0.routes.attach_stand_in(&config.version, &config.backend);
        let (region, slot) = if configured.registry {
            (Some(self.region_handler()), Some(self.0.links.clone()))
        } else {
            (None, None)
        };
        let (gateway_stop, gateway_stopped) = watch::channel(false);
        let gateway_ctx = ServiceContext::new(ctx.name.clone(), ctx.status.clone(), gateway_stopped);
        let gateway = tokio::spawn(run_with_network(
            config,
            capture,
            sink,
            gateway_ctx,
            Some(self.0.routes.clone()),
            slot.clone(),
            region,
        ));
        tokio::pin!(gateway);
        let mut ctx = ctx;
        let mut links_seen = slot.is_none();
        let mut ended_helper: Option<Result<(), String>> = None;
        let gateway_result = loop {
            let helper_done = async {
                match helper_task.as_mut() {
                    Some(task) => task.await.unwrap_or_else(|_| Err("the allowlist stopped".to_owned())),
                    None => std::future::pending().await,
                }
            };
            tokio::select! {
                result = &mut gateway => break result.unwrap_or_else(|e| Err(e.to_string())),
                () = ctx.wait_stop() => {
                    gateway_stop.send_replace(true);
                    break (&mut gateway).await.unwrap_or_else(|e| Err(e.to_string()));
                }
                ended = helper_done => {
                    helper_task = None;
                    ended_helper = Some(ended);
                    gateway_stop.send_replace(true);
                    break (&mut gateway).await.unwrap_or_else(|e| Err(e.to_string()));
                }
                () = tokio::time::sleep(Duration::from_millis(50)), if !links_seen => {
                    if let Some(links) = self.links() {
                        links_seen = true;
                        self.links_up(&links);
                    }
                }
            }
        };
        // The gateway first, then the helper (which closes the game ports)
        helper_stop.send_replace(true);
        if let Some(task) = helper_task {
            _ = task.await;
        }
        if let Some(links) = self.links() {
            links.endpoint().detach();
        }
        lock(&self.0.links).take();
        match ended_helper {
            Some(Err(e)) => Err(format!("allowlist: {e}")),
            Some(Ok(())) => Err("allowlist: the allowlist stopped".to_owned()),
            None => gateway_result,
        }
    }

    fn on_message(&self, message: ToGateway, out: &Outbox) {
        *lock(&self.0.outbox) = Some(out.clone());
        match message {
            ToGateway::Configure {
                registry,
                allowlist,
                ws_port,
            } => {
                *lock(&self.0.configured) = Some(Configured {
                    registry,
                    allowlist,
                    ws_port,
                });
            }
            ToGateway::Placed { module, ports, running } => {
                self.0.routes.placed(&module, ports.clone(), running);
                lock(&self.0.placed).insert(module, ports);
            }
            ToGateway::Stopped { module } => self.0.routes.stopped(&module),
            ToGateway::AdmitUdp { ip, port } => {
                let helper = lock(&self.0.helper).clone();
                let admitted = helper.is_some_and(|h| h.admit_udp(&ip, port));
                if !admitted {
                    linelog::warn(
                        "a player's game port stays closed: the allowlist does not guard it or refused the address",
                        json!({ "ip": ip, "port": port }),
                    );
                }
            }
            ToGateway::AddPeer { node } => match self.links() {
                Some(links) => links.endpoint().add_peer(node),
                None => lock(&self.0.peers).push((node, true)),
            },
            ToGateway::RemovePeer { node } => match self.links() {
                Some(links) => links.endpoint().remove_peer(&node),
                None => lock(&self.0.peers).push((node, false)),
            },
            ToGateway::Enroll { present } => {
                self.0.enroll.store(present, Ordering::Relaxed);
                if let Some(links) = self.links() {
                    links.set_enroll(present.then(|| self.enroll_handler()));
                }
            }
            ToGateway::Part { id, data } => lock(&self.0.calls.parts).push(id, data),
            ToGateway::NodeAnswer { id, result } => {
                let result = match result {
                    Ok(last) => self.0.calls.finish(id, last).ok_or(NodeFailure::TooLarge),
                    Err(failure) => Err(failure),
                };
                self.0.calls.close(id, Some(Answer::Node(result)));
            }
            ToGateway::Enrolled { id, status, body } => {
                let answer = match self.0.calls.finish(id, body) {
                    Some(body) => Answer::Enrolled(status, body),
                    None => Answer::Enrolled(502, br#"{"error":"too_large"}"#.to_vec()),
                };
                self.0.calls.close(id, Some(answer));
            }
            ToGateway::RegionAnswer { id, accepted } => self.0.calls.close(id, Some(Answer::Region(accepted))),
        }
    }
}

/// A peer's admitted call, answered by the core.
struct CoreNodes(GatewayService);

impl NodeService for CoreNodes {
    fn serve<'a>(
        &'a self,
        from: &'a NodeId,
        service: LinkService,
        body: Vec<u8>,
    ) -> BoxFuture<'a, Result<Vec<u8>, LinkError>> {
        Box::pin(async move {
            let outbox = self.0.outbox();
            let answer = self
                .0
                .call(|id| {
                    outbox.map(|o| {
                        o.send_body(id, body, |body| FromGateway::NodeCall {
                            id,
                            from: from.clone(),
                            service: service.as_str().to_owned(),
                            body,
                        })
                    })
                })
                .await;
            match answer {
                Some(Answer::Node(Ok(body))) => Ok(body),
                Some(Answer::Node(Err(failure))) => Err(match failure {
                    NodeFailure::TooLarge => LinkError::TooLarge,
                    NodeFailure::NoService => LinkError::NoService {
                        node: from.clone(),
                        service,
                    },
                    NodeFailure::Refused { reason } => LinkError::Refused {
                        node: from.clone(),
                        reason,
                    },
                    NodeFailure::Failed { reason } => LinkError::Unreachable {
                        node: from.clone(),
                        reason,
                    },
                }),
                _ => Err(LinkError::Unreachable {
                    node: from.clone(),
                    reason: "the core did not answer".to_owned(),
                }),
            }
        })
    }
}

impl service_plugin::Service for GatewayService {
    fn name(&self) -> &'static str {
        "gateway"
    }

    fn serves(&self) -> &'static [CoreCapability] {
        &[CoreCapability::Routes, CoreCapability::UdpPorts]
    }

    /// The module's routes, then its UDP ranges; when the second refuses, the first is detached too.
    fn attach(
        &self,
        module: &ModuleManifest,
        registration: &Registration,
        _dir: Option<&Path>,
    ) -> Result<(), ModuleError> {
        self.0.routes.attach(module, registration)?;
        let ranges = self.udp_ranges(&module.id, registration);
        if let Err(e) = self.0.udp.attach(&module.id, &ranges) {
            self.0.routes.detach(&module.id);
            return Err(e);
        }
        Ok(())
    }

    fn detach(&self, module: &ModuleId) {
        self.0.routes.detach(module);
        self.0.udp.detach(module);
        lock(&self.0.placed).remove(module);
    }

    fn gateway(&self, message: ToGateway, out: &Outbox) {
        self.on_message(message, out);
    }

    fn run(&self, env: Env, ctx: ServiceContext) -> BoxFuture<'static, Result<(), String>> {
        Box::pin(self.clone().run_inner(env, ctx))
    }
}
