//! The gateway and the allowlist as one service of the core (RD27): the one implementation `gateway-plugin` exports
//! as a library and the core runs compiled in, behind `service_plugin`'s protocol and its gateway messages either way.
//! Both are in one service so the gateway's player admits reach the allowlist helper in-process. The data plane stays
//! here: TLS, the policy, the proxying to backends and the node-link endpoint that admits peers. Only control traffic
//! goes to the core (docs/security/gateway.md, "The library's boundary").

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Duration;

use crate::allowlist::config::AllowlistConfig;
use crate::allowlist::guard::Guard;
use crate::allowlist::helper::Helper;
use crate::allowlist::modules::{AllowlistPlugin, PortView};
use crate::config::{Config, Endpoint};
use crate::modules::{ModuleRoutes, RegistryView};
use crate::nodelink::NodeLinks;
use module_api::manifest::ModuleManifest;
use module_api::network::NodeId;
use module_api::registrar::Registration;
use module_api::{BoxFuture, CoreCapability, CorePlugin, ModuleError, ModuleId, PortRange};
use network_plugin::{LinkError, NodeService, Service};
use service_kit::context::{ComponentPhase, ServiceContext, StatusRegistry};
use service_kit::env::Env;
use service_plugin::gateway::{AllowlistMode, Bytes, FromGateway, NodeFailure, Outbox, Parts, RegionRtt, ToGateway};
use tokio::sync::{oneshot, watch};

/// How long a peer's call, an enrollment or a region report waits for the core.
const CORE_TIMEOUT: Duration = Duration::from_secs(60);
/// How long the in-process allowlist has to be ready before the gateway opens (compose.rs's `ready_within`).
const ALLOWLIST_READY: Duration = Duration::from_secs(60);

type Slot = Arc<Mutex<Option<Arc<Helper>>>>;

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A module's ports by name, and whether it runs.
type Ports = (BTreeMap<String, PortRange>, bool);

/// Where the controller placed each module's ports, as the core sends them before each attach: the service can't
/// ask the registry back while it answers one.
#[derive(Default)]
struct Placed {
    modules: RwLock<BTreeMap<ModuleId, Ports>>,
    /// The stand-in's port, at `GATEWAY_BACKEND_URL` (compose.rs's `GatewayView::own`)
    own: RwLock<Option<(ModuleId, Endpoint)>>,
}

impl RegistryView for Placed {
    /// The port of a running module, on loopback, as `registry.port` answered.
    fn endpoint(&self, module: &ModuleManifest, port: &str) -> Option<Endpoint> {
        let running = self
            .modules
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&module.id)
            .filter(|(_, running)| *running)
            .and_then(|(ports, _)| ports.get(port).copied());
        if let Some(range) = running {
            return Some(Endpoint {
                host: "127.0.0.1".into(),
                port: range.first,
            });
        }
        let own = self.own.read().unwrap_or_else(PoisonError::into_inner);
        own.as_ref()
            .filter(|(id, _)| *id == module.id && port == "backend")
            .map(|(_, endpoint)| endpoint.clone())
    }
}

impl PortView for Placed {
    fn assigned(&self, module: &ModuleId, port: &str) -> Option<PortRange> {
        self.modules
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(module)
            .and_then(|(ports, _)| ports.get(port).copied())
    }
}

/// What the core said before Start.
struct Configured {
    registry: bool,
    ws_port: Option<u16>,
    /// The in-process allowlist's settings and the guard its plugin shares, or why they were refused
    in_process: Option<Result<(AllowlistConfig, Arc<Guard>), String>>,
}

enum Answer {
    Node(Result<Vec<u8>, NodeFailure>),
    Enrolled(u16, Vec<u8>),
    Region(bool),
}

#[derive(Default)]
struct State {
    out: Mutex<Option<Outbox>>,
    configured: Mutex<Option<Configured>>,
    allowlist: RwLock<Option<AllowlistPlugin>>,
    slot: Slot,
    links: Mutex<Option<Arc<NodeLinks>>>,
    enroll: Mutex<bool>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Answer>>>,
    parts: Mutex<Parts>,
    next: AtomicU64,
}

impl State {
    fn outbox(&self) -> Option<Outbox> {
        lock(&self.out).clone()
    }

    /// Sends what `make` builds under a new id, and waits for its answer.
    async fn ask(&self, body: Vec<u8>, make: impl FnOnce(u64, Bytes) -> FromGateway) -> Option<Answer> {
        let out = self.outbox()?;
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        lock(&self.pending).insert(id, tx);
        let answer = if out.send_body(id, body, |last| make(id, last)) {
            tokio::time::timeout(CORE_TIMEOUT, rx).await.ok().and_then(Result::ok)
        } else {
            None
        };
        lock(&self.pending).remove(&id);
        answer
    }

    fn answer(&self, id: u64, answer: Answer) {
        if let Some(tx) = lock(&self.pending).remove(&id) {
            _ = tx.send(answer);
        }
    }

    /// The core's enroll handler, on the node links once both are there.
    fn apply_enroll(self: &Arc<Self>) {
        let Some(links) = lock(&self.links).clone() else {
            return;
        };
        if !*lock(&self.enroll) {
            links.set_enroll_handler(None);
            return;
        }
        let state = Arc::downgrade(self);
        links.set_enroll_handler(Some(Arc::new(move |body: Vec<u8>| {
            let state = state.clone();
            Box::pin(async move {
                let Some(state) = state.upgrade() else {
                    return unavailable();
                };
                match state.ask(body, |id, body| FromGateway::Enroll { id, body }).await {
                    Some(Answer::Enrolled(status, body)) => (status, body),
                    _ => unavailable(),
                }
            })
        })));
    }
}

fn unavailable() -> (u16, Vec<u8>) {
    (503, br#"{"error":"unavailable"}"#.to_vec())
}

/// The service's node service: each call the endpoint admitted goes to the core's node services.
struct ToCore(std::sync::Weak<State>, NodeId);

impl NodeService for ToCore {
    fn serve<'a>(
        &'a self,
        from: &'a NodeId,
        service: Service,
        body: Vec<u8>,
    ) -> BoxFuture<'a, Result<Vec<u8>, LinkError>> {
        Box::pin(async move {
            let unreachable = |reason: &str| LinkError::Unreachable {
                node: self.1.clone(),
                reason: reason.to_owned(),
            };
            let state = self.0.upgrade().ok_or_else(|| unreachable("the gateway is stopping"))?;
            let make = |id, body| FromGateway::NodeCall {
                id,
                from: from.clone(),
                service: service.as_str().to_owned(),
                body,
            };
            match state.ask(body, make).await {
                Some(Answer::Node(Ok(answer))) => Ok(answer),
                Some(Answer::Node(Err(failure))) => Err(match failure {
                    NodeFailure::TooLarge => LinkError::TooLarge,
                    NodeFailure::NoService => LinkError::NoService {
                        node: self.1.clone(),
                        service,
                    },
                    NodeFailure::Refused { reason } => LinkError::Refused {
                        node: from.clone(),
                        reason,
                    },
                    NodeFailure::Failed { reason } => unreachable(&reason),
                }),
                _ => Err(unreachable("the core did not answer")),
            }
        })
    }
}

/// The gateway and the allowlist as a service (`service_plugin::Service`): what `gateway-plugin` exports as a library
/// ([`GatewayService::library`]) and the core runs compiled in ([`GatewayService::default`]), the same code either way.
pub struct GatewayService {
    placed: Arc<Placed>,
    routes: Arc<ModuleRoutes>,
    state: Arc<State>,
    /// A library's linelog statics aren't the core's: its lines go through its tracing, which reaches the core
    linelog_to_tracing: bool,
}

impl Default for GatewayService {
    fn default() -> Self {
        let placed = Arc::new(Placed::default());
        Self {
            routes: Arc::new(ModuleRoutes::new(placed.clone())),
            placed,
            state: Arc::default(),
            linelog_to_tracing: false,
        }
    }
}

impl GatewayService {
    /// The service as its library runs it.
    pub fn library() -> Self {
        Self {
            linelog_to_tracing: true,
            ..Self::default()
        }
    }

    fn configure(&self, registry: bool, allowlist: AllowlistMode, ws_port: Option<u16>) {
        let view: Arc<dyn PortView> = self.placed.clone();
        let (plugin, in_process) = match allowlist {
            AllowlistMode::Off => (None, None),
            AllowlistMode::Absent => (Some(AllowlistPlugin::absent(view)), None),
            AllowlistMode::External { base } => (Some(AllowlistPlugin::external(base, view)), None),
            AllowlistMode::InProcess { env } => {
                let env = Env::from_pairs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
                // The guard is the plugin's and the helper's, as compose.rs shares it
                let made = AllowlistConfig::from_env(&env)
                    .and_then(|config| Guard::new(&config.ports).map(|guard| (config, Arc::new(guard))));
                match made {
                    Ok((config, guard)) => (
                        Some(AllowlistPlugin::new(guard.clone(), view)),
                        Some(Ok((config, guard))),
                    ),
                    // Every UDP port is refused until a run with good settings
                    Err(error) => (Some(AllowlistPlugin::absent(view)), Some(Err(error))),
                }
            }
        };
        *self.state.allowlist.write().unwrap_or_else(PoisonError::into_inner) = plugin;
        *lock(&self.state.configured) = Some(Configured {
            registry,
            ws_port,
            in_process,
        });
    }
}

impl service_plugin::Service for GatewayService {
    fn name(&self) -> &'static str {
        "gateway"
    }

    fn serves(&self) -> &'static [CoreCapability] {
        &[CoreCapability::Routes, CoreCapability::UdpPorts]
    }

    fn attach(
        &self,
        module: &ModuleManifest,
        registration: &Registration,
        _dir: Option<&Path>,
    ) -> Result<(), ModuleError> {
        self.routes.attach(module, registration)?;
        let allowlist = self.state.allowlist.read().unwrap_or_else(PoisonError::into_inner);
        if let Some(plugin) = allowlist.as_ref()
            && let Err(error) = plugin.attach(module, registration)
        {
            self.routes.detach(&module.id);
            return Err(error);
        }
        Ok(())
    }

    fn detach(&self, module: &ModuleId) {
        self.routes.detach(module);
        if let Some(plugin) = self
            .state
            .allowlist
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            plugin.detach(module);
        }
        self.placed
            .modules
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(module);
    }

    fn gateway(&self, message: ToGateway, out: &Outbox) {
        let state = &self.state;
        match message {
            ToGateway::Configure {
                registry,
                allowlist,
                ws_port,
            } => {
                *lock(&state.out) = Some(out.clone());
                self.configure(registry, allowlist, ws_port);
            }
            ToGateway::Placed { module, ports, running } => {
                self.placed
                    .modules
                    .write()
                    .unwrap_or_else(PoisonError::into_inner)
                    .insert(module, (ports, running));
            }
            ToGateway::Stopped { module } => {
                if let Some((_, running)) = self
                    .placed
                    .modules
                    .write()
                    .unwrap_or_else(PoisonError::into_inner)
                    .get_mut(&module)
                {
                    *running = false;
                }
            }
            ToGateway::AdmitUdp { ip, port } => {
                let helper = lock(&state.slot).clone();
                if !helper.is_some_and(|helper| helper.admit_udp(&ip, port)) {
                    tracing::warn!(
                        target: "host",
                        udp_port = port,
                        "allowlist helper refused player UDP admission; game port stays closed"
                    );
                }
            }
            ToGateway::AddPeer { node } => {
                if let Some(links) = lock(&state.links).as_ref() {
                    links.endpoint().add_peer(node);
                }
            }
            ToGateway::RemovePeer { node } => {
                if let Some(links) = lock(&state.links).as_ref() {
                    links.endpoint().remove_peer(&node);
                }
            }
            ToGateway::Enroll { present } => {
                *lock(&state.enroll) = present;
                state.apply_enroll();
            }
            ToGateway::Part { id, data } => lock(&state.parts).push(id, data),
            ToGateway::NodeAnswer { id, result } => {
                let result = match result {
                    Ok(last) => lock(&state.parts).finish(id, last).ok_or(NodeFailure::TooLarge),
                    Err(failure) => Err(failure),
                };
                state.answer(id, Answer::Node(result));
            }
            ToGateway::Enrolled { id, status, body } => {
                let answer = match lock(&state.parts).finish(id, body) {
                    Some(body) => Answer::Enrolled(status, body),
                    None => Answer::Enrolled(502, br#"{"error":"too_large"}"#.to_vec()),
                };
                state.answer(id, answer);
            }
            ToGateway::RegionAnswer { id, accepted } => state.answer(id, Answer::Region(accepted)),
        }
    }

    fn run(&self, env: Env, ctx: ServiceContext) -> BoxFuture<'static, Result<(), String>> {
        let (placed, routes, state) = (self.placed.clone(), self.routes.clone(), self.state.clone());
        if self.linelog_to_tracing {
            log_through_tracing();
        }
        Box::pin(run(env, ctx, placed, routes, state))
    }
}

/// The library's linelog lines, through its tracing to the core's log.
fn log_through_tracing() {
    service_kit::linelog::set_sink(Some(Box::new(|line, level| match level {
        service_kit::linelog::Level::Error => tracing::error!(target: "linelog", "{line}"),
        service_kit::linelog::Level::Warn => tracing::warn!(target: "linelog", "{line}"),
        service_kit::linelog::Level::Info => tracing::info!(target: "linelog", "{line}"),
        service_kit::linelog::Level::Debug => tracing::debug!(target: "linelog", "{line}"),
    })));
}

async fn run(
    env: Env,
    ctx: ServiceContext,
    placed: Arc<Placed>,
    routes: Arc<ModuleRoutes>,
    state: Arc<State>,
) -> Result<(), String> {
    let (registry, ws_port, in_process) = {
        let mut configured = lock(&state.configured);
        let Some(configured) = configured.as_mut() else {
            return Err(String::from("the core did not configure the gateway before its start"));
        };
        (configured.registry, configured.ws_port, configured.in_process.take())
    };
    let mut config = Config::from_env(&env)?;
    if let Some(port) = ws_port
        && config.ws.port == config.backend.port
    {
        config.ws.port = port;
    }
    let capture = crate::server::load_capture(&env)?;

    // The allowlist first: in public mode the game ports close before any game server listens
    let mut outer_stop = ctx.stop_receiver();
    let allowlist = match in_process {
        Some(Err(error)) => {
            ctx.status.failed(error.clone());
            return Err(error);
        }
        Some(Ok((config, guard))) => Some(start_allowlist(config, guard, state.slot.clone(), &mut outer_stop).await?),
        None => None,
    };
    let sink: Option<crate::feed::InProcessSink> = allowlist.as_ref().map(|_| {
        let slot = state.slot.clone();
        Arc::new(move |ip: &str| lock(&slot).as_ref().is_some_and(|helper| helper.allow_ip(ip)))
            as crate::feed::InProcessSink
    });

    let node_links = registry.then(crate::server::NodeLinkSlot::default);
    let region_handler = registry.then(|| region_handler(&state));
    let routes = if registry {
        let (manifest, registration) = crate::modules::stand_in(&config.version)?;
        *placed.own.write().unwrap_or_else(PoisonError::into_inner) =
            Some((manifest.id.clone(), config.backend.clone()));
        routes
            .attach_stand_in(&manifest, &registration)
            .map_err(|e| e.to_string())?;
        Some(routes)
    } else {
        None
    };
    if let Some(slot) = node_links.clone() {
        tokio::spawn(announce_links(slot, state.clone(), ctx.stop_receiver()));
    }

    // The gateway stops on the core's Stop or when the allowlist ends under it
    let (stop, stopping) = watch::channel(false);
    let gateway_ctx = ServiceContext::new(ctx.name.clone(), ctx.status.clone(), stopping);
    let gateway =
        crate::server::run_with_network(config, capture, sink, gateway_ctx, routes, node_links, region_handler);
    tokio::pin!(gateway);
    let (result, allowlist) = match allowlist {
        None => {
            let result = tokio::select! {
                result = &mut gateway => result,
                () = stopped(&mut outer_stop) => { _ = stop.send(true); gateway.await }
            };
            (result, None)
        }
        Some(mut allowlist) => {
            let result = tokio::select! {
                result = &mut gateway => result,
                () = stopped(&mut outer_stop) => { _ = stop.send(true); gateway.await }
                ended = &mut allowlist.task => {
                    _ = stop.send(true);
                    _ = gateway.await;
                    let error = match ended {
                        Ok(Ok(())) => String::from("the allowlist stopped"),
                        Ok(Err(error)) => error,
                        Err(error) => error.to_string(),
                    };
                    ctx.status.failed(format!("allowlist: {error}"));
                    return Err(format!("allowlist: {error}"));
                }
            };
            (result, Some(allowlist))
        }
    };
    lock(&state.links).take();
    // Then the allowlist, which closes the game ports on its way out
    if let Some(allowlist) = allowlist {
        _ = allowlist.stop.send(true);
        match allowlist.task.await {
            Ok(Err(error)) => return Err(format!("allowlist: {error}")),
            Err(error) => return Err(format!("allowlist: {error}")),
            Ok(Ok(())) => {}
        }
    }
    lock(&state.slot).take();
    result
}

struct Allowlist {
    stop: watch::Sender<bool>,
    task: tokio::task::JoinHandle<Result<(), String>>,
}

/// The allowlist helper with a context of its own, once it is ready.
async fn start_allowlist(
    config: AllowlistConfig,
    guard: Arc<Guard>,
    slot: Slot,
    outer_stop: &mut watch::Receiver<bool>,
) -> Result<Allowlist, String> {
    let status = StatusRegistry::default();
    let (ctx, stop) = status.service_context("allowlist");
    let mut changes = status.subscribe();
    let mut task =
        tokio::spawn(async move { crate::allowlist::helper::run_with_guard(config, ctx, Some(slot), guard).await });
    let ready = async {
        loop {
            let phase = changes
                .borrow_and_update()
                .get("allowlist")
                .map(|c| (c.phase, c.last_error.clone()));
            match phase {
                Some((ComponentPhase::Ready, _)) => return Ok(()),
                Some((ComponentPhase::Failed, error)) => return Err(error.unwrap_or_default()),
                _ => {}
            }
            if changes.changed().await.is_err() {
                return Err(String::from("the allowlist ended before it was ready"));
            }
        }
    };
    let outcome = tokio::select! {
        ready = tokio::time::timeout(ALLOWLIST_READY, ready) => ready.unwrap_or_else(|_| Err(format!("not ready within {ALLOWLIST_READY:?}"))),
        ended = &mut task => Err(match ended {
            Ok(Ok(())) => String::from("the allowlist ended before it was ready"),
            Ok(Err(error)) => error,
            Err(error) => error.to_string(),
        }),
        () = stopped(outer_stop) => Err(String::from("stopped before the allowlist was ready")),
    };
    match outcome {
        Ok(()) => Ok(Allowlist { stop, task }),
        Err(error) => {
            _ = stop.send(true);
            if !task.is_finished() {
                _ = tokio::time::timeout(Duration::from_secs(90), &mut task).await;
            }
            Err(format!("allowlist: {error}"))
        }
    }
}

/// Once the gateway placed its node links: their calls go to the core, which is told who this node is.
async fn announce_links(slot: crate::server::NodeLinkSlot, state: Arc<State>, mut stop: watch::Receiver<bool>) {
    let links = loop {
        if let Some(links) = lock(&slot).clone() {
            break links;
        }
        if *stop.borrow() {
            return;
        }
        _ = tokio::time::timeout(Duration::from_millis(20), stop.changed()).await;
    };
    let identity = links.endpoint().identity().clone();
    links
        .endpoint()
        .attach(Arc::new(ToCore(Arc::downgrade(&state), identity.node.clone())));
    *lock(&state.links) = Some(links);
    state.apply_enroll();
    if let Some(out) = state.outbox() {
        out.send(FromGateway::NodeLinks { identity });
    }
}

/// `POST /network/v1/regions`, answered by the core's ring (compose.rs's `region_report_handler`).
fn region_handler(state: &Arc<State>) -> crate::network::RegionReportHandler {
    let state = Arc::downgrade(state);
    Arc::new(move |api_key, ticket, regions| {
        let state = state.clone();
        Box::pin(async move {
            let Some(state) = state.upgrade() else {
                return false;
            };
            let regions = regions
                .into_iter()
                .map(|r| RegionRtt {
                    region: r.region,
                    rtt_ms: r.rtt_ms,
                })
                .collect();
            let make = |id, _| FromGateway::RegionReport {
                id,
                api_key,
                ticket,
                regions,
            };
            matches!(state.ask(Vec::new(), make).await, Some(Answer::Region(true)))
        })
    })
}

async fn stopped(stop: &mut watch::Receiver<bool>) {
    while !*stop.borrow() {
        if stop.changed().await.is_err() {
            return std::future::pending().await;
        }
    }
}
