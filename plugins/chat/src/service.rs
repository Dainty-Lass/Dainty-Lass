//! Chat as a service of the core (RD27): the one implementation `chat-plugin` exports as a library and the core runs
//! compiled in, over `service_plugin::chat`'s messages either way. Chat keeps its listener, sessions, history and
//! federation here; what it asks the controller for (the game modules' answers, the home API, the directory, the
//! event bus, the node link) goes to the core through the [`ChatLink`].

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use module_api::chat::{ChatAnswer, ChatQuery, PresenceEvent};
use module_api::network::NodeId;
use module_api::{BoxFuture, CoreCapability, CorePlugin, ErrorKind, GlobalId, ModuleError, ModuleId};
use module_api::{ModuleManifest, Registration};
use network::{LinkError, NodeService, PeerLink, Service};
use service_kit::context::ServiceContext;
use service_kit::env::Env;
use service_plugin::chat::{ChatLink, PeerError, PeerServer};

/// Chat as a service (`service_plugin::Service`): what `chat-plugin` exports as a library and the core runs compiled
/// in, the same code either way. Its plugin outlives its runs, as the controller held it in-process.
#[derive(Default)]
pub struct ChatService {
    plugin: Arc<crate::ChatPlugin>,
    link: Mutex<Option<ChatLink>>,
}

impl service_plugin::Service for ChatService {
    fn name(&self) -> &'static str {
        "chat"
    }

    fn serves(&self) -> &'static [CoreCapability] {
        &[CoreCapability::Chat]
    }

    fn attach(
        &self,
        module: &ModuleManifest,
        registration: &Registration,
        _dir: Option<&std::path::Path>,
    ) -> Result<(), ModuleError> {
        self.plugin.attach(module, registration)
    }

    fn detach(&self, module: &ModuleId) {
        self.plugin.detach(module);
    }

    fn chat_link(&self, link: ChatLink) {
        *self.link.lock().unwrap_or_else(PoisonError::into_inner) = Some(link);
    }

    fn run(&self, env: Env, ctx: ServiceContext) -> BoxFuture<'static, Result<(), String>> {
        let plugin = self.plugin.clone();
        let link = self.link.lock().unwrap_or_else(PoisonError::into_inner).clone();
        Box::pin(run(plugin, link, env, ctx))
    }
}

async fn run(
    plugin: Arc<crate::ChatPlugin>,
    link: Option<ChatLink>,
    env: Env,
    mut ctx: ServiceContext,
) -> Result<(), String> {
    let settings = crate::env::settings(&env)?;
    let dashboard = crate::env::dashboard(&env)?;
    let database = database(&env)?;
    let link = link.ok_or("chat: no link to the core")?;
    let setup = link.setup().ok_or("chat: the core sent no setup")?;
    if let store::DatabaseConfig::Sqlite { file } = &database
        && let Some(parent) = file.parent()
    {
        _ = std::fs::create_dir_all(parent);
    }
    let history =
        crate::History::open(&crate::History::beside(&database)).map_err(|e| format!("chat history database: {e}"))?;
    let federation: Arc<dyn crate::Federation> = match setup.network {
        Some(network) => {
            let config =
                crate::ChatFederationConfig::new(setup.node.clone(), network.peers).with_leader(network.leader);
            let federation = Arc::new(crate::ChatFederation::new(config, Arc::new(Peers(link.clone()))));
            link.serve_peers(Some(Arc::new(Served(federation.clone()))));
            federation
        }
        None => Arc::new(crate::LocalOnly),
    };
    let host: Arc<dyn crate::ChatHost> = Arc::new(LinkChatHost {
        node: setup.node.node.clone(),
        link: link.clone(),
        plugin: plugin.clone(),
        federation: federation.clone(),
    });
    let options = crate::Options {
        moderation: dashboard.map_or_else(crate::Moderation::off, crate::Moderation::start),
        ..crate::Options::default()
    };
    let running = plugin
        .start(&settings, setup.node, host, federation, history, options)
        .await
        .map_err(|e| e.to_string())?;
    ctx.status().ready(Some(format!("127.0.0.1:{}", running.port)), None);
    ctx.wait_stop().await;
    ctx.status().stopping();
    link.serve_peers(None);
    running.stop().await;
    ctx.status().stopped();
    Ok(())
}

/// The node database from the env, as the host kit writes it (`REFORGED_DB_*`, store's `from_lookup`). The core
/// always names it: nothing falls back to a file in the working directory.
fn database(env: &Env) -> Result<store::DatabaseConfig, String> {
    if env.get("REFORGED_DB_ENGINE").is_none() {
        return Err(String::from("chat: REFORGED_DB_ENGINE is not set"));
    }
    let database = store::DatabaseConfig::from_lookup(|key| env.get(key).map(str::to_owned), PathBuf::new())?;
    if matches!(&database, store::DatabaseConfig::Sqlite { file } if file.as_os_str().is_empty()) {
        return Err(String::from("chat: REFORGED_DB_FILE is not set"));
    }
    Ok(database)
}

/// What chat asks of its host, with the controller behind the link.
struct LinkChatHost {
    node: NodeId,
    link: ChatLink,
    plugin: Arc<crate::ChatPlugin>,
    federation: Arc<dyn crate::Federation>,
}

impl LinkChatHost {
    fn active_modules(&self) -> Vec<ModuleId> {
        let attached: Vec<ModuleId> = self.plugin.modules().into_iter().collect();
        if attached.is_empty() {
            self.link.loaded()
        } else {
            attached
        }
    }
}

impl crate::ChatHost for LinkChatHost {
    fn login<'a>(&'a self, login: crate::Login<'a>) -> BoxFuture<'a, Result<crate::Player, crate::LoginRefusal>> {
        Box::pin(async move {
            let modules = self.active_modules();
            if modules.is_empty() {
                return Err(crate::LoginRefusal::Unavailable);
            }
            let mut saw_unavailable = false;
            for module in &modules {
                let query = ChatQuery::Login {
                    authcid: login.authcid.to_owned(),
                    token: login.token.to_owned(),
                };
                match self.link.ask(module, query).await {
                    Ok(ChatAnswer::Login { account, name }) => {
                        return Ok(crate::Player {
                            account,
                            module: module.clone(),
                            name,
                        });
                    }
                    Ok(_) => {}
                    Err(error) if error.kind == ErrorKind::Unavailable => saw_unavailable = true,
                    Err(_) => {}
                }
            }
            if let Some(account) = self.federation.resolve(login.authcid).await
                && account.owner() != &self.node
            {
                for module in &modules {
                    if let Some(name) = self.link.home_login(module, &account, login.authcid, login.token).await {
                        return Ok(crate::Player {
                            account,
                            module: module.clone(),
                            name,
                        });
                    }
                }
            }
            if saw_unavailable {
                Err(crate::LoginRefusal::Unavailable)
            } else {
                Err(crate::LoginRefusal::BadToken)
            }
        })
    }

    fn ask<'a>(&'a self, module: &'a ModuleId, query: ChatQuery) -> BoxFuture<'a, Result<ChatAnswer, ModuleError>> {
        Box::pin(self.link.ask(module, query))
    }

    fn blocked<'a>(&'a self, sender: &'a GlobalId, recipient: &'a GlobalId) -> BoxFuture<'a, bool> {
        Box::pin(async move {
            for module in self.active_modules() {
                let query = ChatQuery::MayWhisper {
                    from: sender.clone(),
                    to: recipient.clone(),
                };
                if let Ok(ChatAnswer::MayWhisper(allowed)) = self.link.ask(&module, query).await {
                    return !allowed;
                }
            }
            false
        })
    }

    fn account_of<'a>(&'a self, local: &'a str) -> BoxFuture<'a, Option<GlobalId>> {
        Box::pin(async move {
            for module in self.active_modules() {
                let query = ChatQuery::AccountOf {
                    local: local.to_owned(),
                };
                if let Ok(ChatAnswer::AccountOf(Some(account))) = self.link.ask(&module, query).await {
                    return Some(account);
                }
            }
            self.link.find_account(local).await
        })
    }

    fn presence<'a>(&'a self, account: &'a GlobalId, local: &'a str, online: bool) -> BoxFuture<'a, ()> {
        self.link.presence(PresenceEvent {
            account: account.clone(),
            user_id: local.to_owned(),
            online,
        });
        Box::pin(async {})
    }
}

/// The node link, through the core.
struct Peers(ChatLink);

impl PeerLink for Peers {
    fn call<'a>(
        &'a self,
        to: &'a NodeId,
        service: Service,
        body: Vec<u8>,
    ) -> BoxFuture<'a, Result<Vec<u8>, LinkError>> {
        Box::pin(async move {
            if service != Service::Chat {
                return Err(LinkError::NoService {
                    node: to.clone(),
                    service,
                });
            }
            let body = serde_json::from_slice(&body).map_err(|e| LinkError::Protocol {
                node: to.clone(),
                reason: e.to_string(),
            })?;
            let answer = self.0.peer_call(to, body).await.map_err(|e| to_link_error(e, to))?;
            serde_json::to_vec(&answer).map_err(|e| LinkError::Protocol {
                node: to.clone(),
                reason: e.to_string(),
            })
        })
    }
}

/// `Service::Chat` as the core hands peers' calls over.
struct Served(Arc<crate::ChatFederation>);

impl PeerServer for Served {
    fn serve<'a>(
        &'a self,
        from: &'a NodeId,
        body: serde_json::Value,
    ) -> BoxFuture<'a, Result<serde_json::Value, PeerError>> {
        Box::pin(async move {
            let body = serde_json::to_vec(&body).map_err(|e| PeerError::Protocol { reason: e.to_string() })?;
            let answer = self.0.serve(from, Service::Chat, body).await.map_err(to_peer_error)?;
            serde_json::from_slice(&answer).map_err(|e| PeerError::Protocol { reason: e.to_string() })
        })
    }
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
