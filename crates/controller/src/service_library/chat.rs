// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The core's side of the chat service ([`service_plugin::chat`]), loaded as a library or compiled in: what
//! `ControllerChatHost` and the chat drivers hook did in-process. The game modules' drivers, the home API, the
//! directory, the event bus and the node link stay here; chat asks for them by message and the core answers, at most
//! [`MAX_PENDING`] at once.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use module_api::chat::{PRESENCE_EVENT, PresenceEvent};
use module_api::frame::{MAX_FRAME, encode};
use module_api::network::{HomeForward, NodeId, OperationName};
use module_api::{
    Actor, ActorContext, BoxFuture, CoreCapability, Event, GlobalId, ModuleDriver, ModuleDriverExt, ModuleError,
    ModuleId, ModuleManifest,
};
use network_plugin::{LinkError, NodeService, Service};
use service_kit::env::Env;
use service_plugin::chat::{ChatSetup, FromChat, MAX_PENDING, PeerError, REPLY_TIMEOUT, ToChat};
use tokio::sync::{Semaphore, oneshot};

use service_plugin::{FromService, ToService};

use super::{Extension, RunLink, ServiceLibrary, service_compiled_in_with, service_library_with};
use crate::{LoadHook, Module, ModuleRegistry, WeakRegistry};

/// How long chat waits for the node link before it federates, as it did in-process.
const PEER_LINK_WAIT: std::time::Duration = std::time::Duration::from_secs(10);

/// The chat library: a supervised module, chat's core plugin (`CoreCapability::Chat`), and the hook that tells it
/// which game modules answer chat. `setup` is what `ChatOptions` gave in-process; the database comes in `env`.
pub fn chat_library(mut library: ServiceLibrary, env: Env, registry: &ModuleRegistry, setup: ChatSetup) -> Module {
    library.serves = &[CoreCapability::Chat];
    let extension = extension(registry, setup);
    supervised(service_library_with(library, env, Some(registry), Some(extension)))
}

/// [`chat_library`] for the same service compiled in: `make` builds what the library exports, and it runs over the
/// same protocol with the same core side (RD27).
pub fn chat_compiled_in<S: service_plugin::Service>(
    make: impl Fn() -> S + Send + Sync + 'static,
    env: Env,
    registry: &ModuleRegistry,
    setup: ChatSetup,
) -> Module {
    let extension = extension(registry, setup);
    supervised(service_compiled_in_with(make, env, Some(registry), Some(extension)))
}

/// As compose.rs composed chat in-process: a server runs without it, and its database may be slow to open.
fn supervised(module: Module) -> Module {
    module.optional().ready_within(std::time::Duration::from_secs(30))
}

fn extension(registry: &ModuleRegistry, setup: ChatSetup) -> Arc<dyn Extension> {
    let core = Arc::new(ChatCore {
        setup,
        registry: registry.downgrade(),
        drivers: Mutex::default(),
        run: Mutex::default(),
        runtime: Mutex::default(),
        serving: Mutex::default(),
        next: AtomicU64::new(1),
        in_flight: Arc::new(Semaphore::new(MAX_PENDING)),
        attached: AtomicBool::new(false),
    });
    registry.add_hook(Arc::new(Hook(core.clone())));
    Arc::new(Ext(core))
}

struct ChatCore {
    setup: ChatSetup,
    registry: WeakRegistry,
    /// The loaded game modules with the chat capability; held while their comings and goings are sent, so the
    /// replay into a new run and a hook never cross
    drivers: Mutex<BTreeMap<ModuleId, Arc<dyn ModuleDriver>>>,
    run: Mutex<Option<RunLink>>,
    /// The controller's runtime, where chat's requests are answered (the read loop has a thread of its own)
    runtime: Mutex<Option<tokio::runtime::Handle>>,
    /// Peers' calls chat has yet to answer
    serving: Mutex<HashMap<u64, oneshot::Sender<Result<serde_json::Value, PeerError>>>>,
    next: AtomicU64,
    in_flight: Arc<Semaphore>,
    attached: AtomicBool,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The core's [`Extension`] for chat.
struct Ext(Arc<ChatCore>);

impl Extension for Ext {
    fn opened(&self, _run: RunLink) -> Vec<ToService> {
        Vec::new()
    }

    fn prepare<'a>(&'a self, run: &'a RunLink) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(self.0.before_start(run))
    }

    fn closed(&self) {
        self.0.ended();
    }

    fn message(&self, message: FromService) -> Result<(), String> {
        match message {
            FromService::Chat(message) => self.0.message(message),
            _ => Err(String::from("a message this service may not send")),
        }
    }
}

impl ChatCore {
    /// Before `Start`: the setup and the loaded modules, then the node link's `Service::Chat` in a network.
    async fn before_start(self: &Arc<Self>, run: &RunLink) -> Result<(), String> {
        *lock(&self.runtime) = Some(tokio::runtime::Handle::current());
        let registry = self.registry.upgrade().ok_or("the registry is gone")?;
        if self.setup.network.is_some() {
            let waited = tokio::time::timeout(PEER_LINK_WAIT, async {
                while registry.node_services().peer_link().is_none() {
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
            });
            waited
                .await
                .map_err(|_| String::from("chat federation: network peer link did not become ready"))?;
        }
        {
            let drivers = lock(&self.drivers);
            run.send(&ToService::Chat(ToChat::Setup {
                setup: Box::new(self.setup.clone()),
            }));
            for module in drivers.keys() {
                run.send(&ToService::Chat(ToChat::Loaded { module: module.clone() }));
            }
            *lock(&self.run) = Some(run.clone());
        }
        if self.setup.network.is_some() {
            registry
                .node_services()
                .attach(Service::Chat, Arc::new(PeerProxy(self.clone())));
            self.attached.store(true, Ordering::Relaxed);
        }
        Ok(())
    }

    /// The run ended: peers' calls in flight fail, and the node link stops routing to it.
    fn ended(&self) {
        lock(&self.run).take();
        lock(&self.serving).clear();
        if self.attached.swap(false, Ordering::Relaxed)
            && let Some(registry) = self.registry.upgrade()
        {
            registry.node_services().detach(Service::Chat);
        }
    }

    /// A message from the service, on its read loop's thread: answered on the controller's runtime.
    fn message(self: &Arc<Self>, message: FromChat) -> Result<(), String> {
        if !message.in_bounds() {
            return Err(String::from("a chat message over its bounds"));
        }
        match message {
            FromChat::PeerServed { id, result } => {
                if let Some(tx) = lock(&self.serving).remove(&id) {
                    _ = tx.send(result);
                }
            }
            FromChat::Presence { event } => self.presence(event),
            request => {
                let (Some(runtime), Some(run)) = (lock(&self.runtime).clone(), lock(&self.run).clone()) else {
                    return Err(String::from("a chat request before its start"));
                };
                let Ok(permit) = self.in_flight.clone().try_acquire_owned() else {
                    if let Some(answer) = refused(request) {
                        reply(&run, answer);
                    }
                    return Ok(());
                };
                let core = self.clone();
                runtime.spawn(async move {
                    if let Some(answer) = core.answer(request).await {
                        reply(&run, answer);
                    }
                    drop(permit);
                });
            }
        }
        Ok(())
    }

    async fn answer(&self, request: FromChat) -> Option<ToChat> {
        Some(match request {
            FromChat::Ask { id, module, query } => {
                let driver = lock(&self.drivers).get(&module).cloned();
                let result = match driver {
                    Some(driver) => driver.chat(query).await,
                    None => Err(ModuleError::unsupported("chat")),
                };
                ToChat::Answer { id, result }
            }
            FromChat::HomeLogin {
                id,
                module,
                account,
                authcid,
                token,
            } => ToChat::HomeLoggedIn {
                id,
                name: self.home_login(module, account, authcid, token).await,
            },
            FromChat::FindAccount { id, local } => ToChat::Account {
                id,
                account: self.find_account(&local).await,
            },
            FromChat::PeerCall { id, to, body } => ToChat::PeerAnswer {
                id,
                result: self.peer_call(&to, &body).await,
            },
            // Handled where they arrive
            FromChat::Presence { .. } | FromChat::PeerServed { .. } => return None,
        })
    }

    /// `chat.login` at the account's home, as `ControllerChatHost::login` ran it.
    async fn home_login(&self, module: ModuleId, account: GlobalId, authcid: String, token: String) -> Option<String> {
        let registry = self.registry.upgrade()?;
        let forward = HomeForward {
            module,
            account,
            actor: ActorContext::new(Actor::System),
            operation: "chat.login".parse::<OperationName>().ok()?,
            body: serde_json::json!({ "authcid": authcid, "token": token }),
            key: None,
        };
        let answer = registry.home().forward(&forward).await.ok()?;
        answer
            .get("name")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
    }

    #[cfg(feature = "network")]
    async fn find_account(&self, local: &str) -> Option<GlobalId> {
        let directory = self.registry.upgrade()?.node_services().directory()?;
        Some(directory.find(local).await.ok()??.account)
    }

    #[cfg(not(feature = "network"))]
    async fn find_account(&self, _local: &str) -> Option<GlobalId> {
        None
    }

    fn presence(&self, event: PresenceEvent) {
        if let Some(registry) = self.registry.upgrade()
            && let Ok(name) = PRESENCE_EVENT.parse()
            && let Ok(payload) = serde_json::to_value(event)
        {
            registry.emit(Event { name, payload });
        }
    }

    async fn peer_call(&self, to: &NodeId, body: &serde_json::Value) -> Result<serde_json::Value, PeerError> {
        let link = self
            .registry
            .upgrade()
            .and_then(|registry| registry.node_services().peer_link())
            .ok_or(PeerError::NoPeers {})?;
        let body = serde_json::to_vec(body).map_err(|e| PeerError::Protocol { reason: e.to_string() })?;
        let answer = link.call(to, Service::Chat, body).await.map_err(to_peer_error)?;
        from_bytes(&answer)
    }
}

/// A request refused because too many are in flight.
fn refused(request: FromChat) -> Option<ToChat> {
    let busy = || ModuleError::new(module_api::ErrorKind::Unavailable, "too many chat requests in flight");
    Some(match request {
        FromChat::Ask { id, .. } => ToChat::Answer {
            id,
            result: Err(busy()),
        },
        FromChat::HomeLogin { id, .. } => ToChat::HomeLoggedIn { id, name: None },
        FromChat::FindAccount { id, .. } => ToChat::Account { id, account: None },
        FromChat::PeerCall { id, .. } => ToChat::PeerAnswer {
            id,
            result: Err(PeerError::Unreachable {
                reason: String::from("too many chat requests in flight"),
            }),
        },
        FromChat::Presence { .. } | FromChat::PeerServed { .. } => return None,
    })
}

/// An answer to the service; one too large for a frame goes as `TooLarge` (a peer's answer is the one that can be).
fn reply(run: &RunLink, answer: ToChat) {
    let message = ToService::Chat(answer);
    if encode(&message, MAX_FRAME).is_ok() {
        run.send(&message);
    } else if let ToService::Chat(ToChat::PeerAnswer { id, .. }) = message {
        run.send(&ToService::Chat(ToChat::PeerAnswer {
            id,
            result: Err(PeerError::TooLarge {}),
        }));
    } else {
        tracing::warn!(target: "host", "chat library: an answer over the frame limit not sent");
    }
}

fn from_bytes(bytes: &[u8]) -> Result<serde_json::Value, PeerError> {
    serde_json::from_slice(bytes).map_err(|e| PeerError::Protocol { reason: e.to_string() })
}

fn to_peer_error(error: LinkError) -> PeerError {
    match error {
        LinkError::NoPeers => PeerError::NoPeers {},
        LinkError::UnknownPeer(_) => PeerError::UnknownPeer {},
        LinkError::Unreachable { reason, .. } => PeerError::Unreachable { reason },
        LinkError::TooLarge => PeerError::TooLarge {},
        LinkError::NoService { .. } => PeerError::NoService {},
        LinkError::ClockSkew { skew_ms, .. } => PeerError::ClockSkew { skew_ms },
        LinkError::Refused { reason, .. } => PeerError::Refused { reason },
        LinkError::Untrusted { reason, .. } => PeerError::Untrusted { reason },
        LinkError::Protocol { reason, .. } => PeerError::Protocol { reason },
    }
    .bounded()
}

fn to_link_error(error: PeerError, node: &NodeId) -> LinkError {
    let node = node.clone();
    match error {
        PeerError::NoPeers {} => LinkError::NoPeers,
        PeerError::UnknownPeer {} => LinkError::UnknownPeer(node),
        PeerError::Unreachable { reason } => LinkError::Unreachable { node, reason },
        PeerError::TooLarge {} => LinkError::TooLarge,
        PeerError::NoService {} => LinkError::NoService {
            node,
            service: Service::Chat,
        },
        PeerError::ClockSkew { skew_ms } => LinkError::ClockSkew { node, skew_ms },
        PeerError::Refused { reason } => LinkError::Refused { node, reason },
        PeerError::Untrusted { reason } => LinkError::Untrusted { node, reason },
        PeerError::Protocol { reason } => LinkError::Protocol { node, reason },
    }
}

/// `Service::Chat` on the node link, answered by the service.
struct PeerProxy(Arc<ChatCore>);

impl NodeService for PeerProxy {
    fn serve<'a>(
        &'a self,
        from: &'a NodeId,
        service: Service,
        body: Vec<u8>,
    ) -> BoxFuture<'a, Result<Vec<u8>, LinkError>> {
        Box::pin(async move {
            let node = self.0.setup.node.node.clone();
            let no_service = || LinkError::NoService {
                node: node.clone(),
                service,
            };
            if service != Service::Chat {
                return Err(no_service());
            }
            let body = from_bytes(&body).map_err(|e| to_link_error(e, &node))?;
            let Some(run) = lock(&self.0.run).clone() else {
                return Err(no_service());
            };
            let id = self.0.next.fetch_add(1, Ordering::Relaxed);
            let (tx, rx) = oneshot::channel();
            {
                let mut serving = lock(&self.0.serving);
                if serving.len() >= MAX_PENDING {
                    return Err(LinkError::Unreachable {
                        node,
                        reason: String::from("chat is busy"),
                    });
                }
                serving.insert(id, tx);
            }
            let message = ToService::Chat(ToChat::PeerServe {
                id,
                from: from.clone(),
                body,
            });
            if encode(&message, MAX_FRAME).is_err() {
                lock(&self.0.serving).remove(&id);
                return Err(LinkError::TooLarge);
            }
            run.send(&message);
            let answer = tokio::time::timeout(REPLY_TIMEOUT, rx).await;
            lock(&self.0.serving).remove(&id);
            match answer {
                Ok(Ok(Ok(body))) => serde_json::to_vec(&body).map_err(|e| LinkError::Protocol {
                    node,
                    reason: e.to_string(),
                }),
                Ok(Ok(Err(error))) => Err(to_link_error(error, &node)),
                Ok(Err(_)) | Err(_) => Err(no_service()),
            }
        })
    }
}

/// The game modules that answer chat, as `ChatDriversHook` held them.
struct Hook(Arc<ChatCore>);

impl LoadHook for Hook {
    fn loaded(&self, manifest: &ModuleManifest, driver: Arc<dyn ModuleDriver>) {
        if !manifest.capabilities.chat {
            return;
        }
        let mut drivers = lock(&self.0.drivers);
        drivers.insert(manifest.id.clone(), driver);
        if let Some(run) = lock(&self.0.run).as_ref() {
            run.send(&ToService::Chat(ToChat::Loaded {
                module: manifest.id.clone(),
            }));
        }
    }

    fn unloaded(&self, module: &ModuleId) {
        let mut drivers = lock(&self.0.drivers);
        if drivers.remove(module).is_some()
            && let Some(run) = lock(&self.0.run).as_ref()
        {
            run.send(&ToService::Chat(ToChat::Unloaded { module: module.clone() }));
        }
    }
}
