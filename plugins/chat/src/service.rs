// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Chat as a `service_plugin::Service` (RD27): the one implementation both the compiled-in build
//! (`service_plugin::in_process`) and the chat library (`export_service!`) run. Its host is the core, over the
//! instance's [`ChatLink`].

use std::sync::{Arc, Mutex, PoisonError};

use module_api::chat::{ChatAnswer, ChatQuery, PresenceEvent};
use module_api::manifest::ModuleManifest;
use module_api::network::NodeId;
use module_api::registrar::Registration;
use module_api::{BoxFuture, CoreCapability, CorePlugin, ErrorKind, GlobalId, ModuleError, ModuleId, NodeIdentity};
use network::{LinkError, NodeService, PeerLink, Service as LinkService};
use service_kit::context::ServiceContext;
use service_kit::env::Env;
use service_plugin::chat::{ChatLink, ChatSetup, PeerError, PeerServer};

use crate::ChatPlugin;
use crate::federation::{ChatFederation, ChatFederationConfig, Federation, LocalOnly};
use crate::history::History;
use crate::hooks::{ChatHost, Login, LoginRefusal, Player};
use crate::moderation::Moderation;
use crate::server::Options;

/// The chat service.
#[derive(Default)]
pub struct ChatService {
    plugin: Arc<ChatPlugin>,
    link: Mutex<Option<ChatLink>>,
}

impl ChatService {
    pub fn new() -> Self {
        Self::default()
    }
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

    fn run(&self, env: Env, mut ctx: ServiceContext) -> BoxFuture<'static, Result<(), String>> {
        let link = self.link.lock().unwrap_or_else(PoisonError::into_inner).clone();
        let plugin = self.plugin.clone();
        Box::pin(async move {
            let link = link.ok_or("chat: no link to the core")?;
            let setup = link.setup().ok_or("chat: the core sent no setup")?;
            let settings = crate::env::settings(&env)?;
            let moderation = crate::env::dashboard(&env)?.map_or_else(Moderation::off, Moderation::start);
            let database = crate::env::database(&env)?;
            let history = History::open(database)
                .await
                .map_err(|e| format!("chat history database: {e}"))?;
            let federation = federation(&setup, &link);
            let host: Arc<dyn ChatHost> = Arc::new(LibraryChatHost {
                link: link.clone(),
                plugin: plugin.clone(),
                federation: federation.clone(),
                node: setup.node.clone(),
            });
            let options = Options {
                moderation,
                ..Options::default()
            };
            let running = plugin
                .start(&settings, setup.node.clone(), host, federation, history, options)
                .await
                .map_err(|e| e.to_string())?;
            ctx.status().ready(Some(format!("127.0.0.1:{}", running.port)), None);
            ctx.wait_stop().await;
            link.serve_peers(None);
            running.stop().await;
            Ok(())
        })
    }
}

/// `LocalOnly` outside a network; in one, the node link reached through the core.
fn federation(setup: &ChatSetup, link: &ChatLink) -> Arc<dyn Federation> {
    let Some(network) = &setup.network else {
        return Arc::new(LocalOnly);
    };
    let config =
        ChatFederationConfig::new(setup.node.clone()).with_leader(network.leader.clone(), network.peers.clone());
    let federation = ChatFederation::new(config, Arc::new(CoreLink(link.clone())));
    link.serve_peers(Some(Arc::new(ServePeers(federation.clone()))));
    federation
}

/// The node link, as the core's `PeerCall`.
struct CoreLink(ChatLink);

impl PeerLink for CoreLink {
    fn call<'a>(
        &'a self,
        to: &'a NodeId,
        service: LinkService,
        body: Vec<u8>,
    ) -> BoxFuture<'a, Result<Vec<u8>, LinkError>> {
        Box::pin(async move {
            if service != LinkService::Chat {
                return Err(LinkError::NoService {
                    node: to.clone(),
                    service,
                });
            }
            let body: serde_json::Value = serde_json::from_slice(&body).map_err(|e| LinkError::Protocol {
                node: to.clone(),
                reason: e.to_string(),
            })?;
            let answer = self.0.peer_call(to, body).await.map_err(|e| link_error(to, e))?;
            serde_json::to_vec(&answer).map_err(|e| LinkError::Protocol {
                node: to.clone(),
                reason: e.to_string(),
            })
        })
    }
}

fn link_error(node: &NodeId, error: PeerError) -> LinkError {
    let node = node.clone();
    match error {
        PeerError::NoPeers {} => LinkError::NoPeers,
        PeerError::UnknownPeer {} => LinkError::UnknownPeer(node),
        PeerError::Unreachable { reason } => LinkError::Unreachable { node, reason },
        PeerError::TooLarge {} => LinkError::TooLarge,
        PeerError::NoService {} => LinkError::NoService {
            node,
            service: LinkService::Chat,
        },
        PeerError::ClockSkew { skew_ms } => LinkError::ClockSkew { node, skew_ms },
        PeerError::Refused { reason } => LinkError::Refused { node, reason },
        PeerError::Untrusted { reason } => LinkError::Untrusted { node, reason },
        PeerError::Protocol { reason } => LinkError::Protocol { node, reason },
    }
}

fn peer_error(error: LinkError) -> PeerError {
    match error {
        LinkError::NoPeers => PeerError::NoPeers {},
        LinkError::UnknownPeer(_) => PeerError::UnknownPeer {},
        LinkError::Unreachable { reason, .. } => PeerError::Unreachable { reason },
        LinkError::TooLarge => PeerError::TooLarge {},
        LinkError::NoService { .. } => PeerError::NoService {},
        LinkError::ClockSkew { skew_ms, .. } => PeerError::ClockSkew { skew_ms },
        LinkError::Refused { reason, .. } => PeerError::Refused { reason },
        LinkError::Untrusted { reason, .. } => PeerError::Untrusted { reason },
        other => PeerError::Protocol {
            reason: other.to_string(),
        },
    }
}

/// Peers' calls the core relays, answered by the federation's `NodeService`.
struct ServePeers(Arc<ChatFederation>);

impl PeerServer for ServePeers {
    fn serve<'a>(
        &'a self,
        from: &'a NodeId,
        body: serde_json::Value,
    ) -> BoxFuture<'a, Result<serde_json::Value, PeerError>> {
        Box::pin(async move {
            let bytes = serde_json::to_vec(&body).map_err(|e| PeerError::Protocol { reason: e.to_string() })?;
            let answer = self.0.serve(from, LinkService::Chat, bytes).await.map_err(peer_error)?;
            serde_json::from_slice(&answer).map_err(|e| PeerError::Protocol { reason: e.to_string() })
        })
    }
}

/// `ChatHost` over the core: the loaded chat modules answer, the account's home logs in a player homed elsewhere.
struct LibraryChatHost {
    link: ChatLink,
    plugin: Arc<ChatPlugin>,
    federation: Arc<dyn Federation>,
    node: NodeIdentity,
}

impl LibraryChatHost {
    /// The attached chat modules, or the core's loaded list when none are attached.
    fn modules(&self) -> Vec<ModuleId> {
        let attached = self.plugin.modules();
        if attached.is_empty() {
            self.link.loaded()
        } else {
            attached.into_iter().collect()
        }
    }
}

impl ChatHost for LibraryChatHost {
    fn login(&self, login: Login) -> BoxFuture<'_, Result<Player, LoginRefusal>> {
        Box::pin(async move {
            let modules = self.modules();
            let mut unavailable = false;
            for module in &modules {
                let query = ChatQuery::Login {
                    authcid: login.authcid.clone(),
                    token: login.token.clone(),
                };
                match self.link.ask(module, query).await {
                    Ok(ChatAnswer::Login { account, name }) => {
                        return Ok(Player {
                            account,
                            module: module.clone(),
                            name,
                        });
                    }
                    Err(e) if e.kind == ErrorKind::Unavailable || e.kind == ErrorKind::Timeout => unavailable = true,
                    _ => {}
                }
            }
            if let Some(account) = self.federation.resolve(&login.authcid).await
                && account.owner() != &self.node.node
                && let Some(module) = modules.first()
                && let Some(name) = self
                    .link
                    .home_login(module, &account, &login.authcid, &login.token)
                    .await
            {
                return Ok(Player {
                    account,
                    module: module.clone(),
                    name,
                });
            }
            Err(if unavailable {
                LoginRefusal::Unavailable
            } else {
                LoginRefusal::BadToken
            })
        })
    }

    fn ask<'a>(&'a self, module: &'a ModuleId, query: ChatQuery) -> BoxFuture<'a, Result<ChatAnswer, ModuleError>> {
        Box::pin(self.link.ask(module, query))
    }

    fn blocked<'a>(&'a self, sender: &'a GlobalId, recipient: &'a GlobalId) -> BoxFuture<'a, bool> {
        Box::pin(async move {
            for module in self.modules() {
                let query = ChatQuery::MayWhisper {
                    from: sender.clone(),
                    to: recipient.clone(),
                };
                if let Ok(ChatAnswer::MayWhisper(may)) = self.link.ask(&module, query).await {
                    return !may;
                }
            }
            false
        })
    }

    fn account_of<'a>(&'a self, local: &'a str) -> BoxFuture<'a, Option<GlobalId>> {
        Box::pin(async move {
            for module in self.modules() {
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
