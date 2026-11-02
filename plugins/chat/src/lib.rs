//! The chat core plugin (DD3, DD21, DN10a): the games' XMPP over WebSocket, on one node.
//!
//! Ported from Dauntless Revived's metagame chat at `e1c5fec` (`services/metagame/src/chat/`, DR M32-M34),
//! which keeps its structure: one task owns all of chat, connections hand it their frames. The 1.14.7 client's
//! subset (its paths, room names, SASL PLAIN with the game token, the `name:id:resource` nickname) was read
//! from Mystic Paradox's realtime gateway (`ParadoxBackend/src/realtime`), as behaviour: none of its code is
//! here.
//!
//! It never listens off loopback: the gateway forwards the WebSocket upgrade to [`Settings::port`] on
//! 127.0.0.1 with its secret and the player's address (docs/security/chat.md). Each game module gives its
//! JID and room rules through `GameModule::chat` ([`hooks`]); its history, waiting direct messages and mutes
//! live in a database of chat's own ([`history`]). Federation between nodes is DN10b's ([`federation`]).

pub mod connection;
pub mod env;
pub mod federation;
pub mod history;
pub mod hooks;
pub mod moderation;
pub mod muc;
pub mod naming;
pub mod nick;
pub mod redact;
pub mod server;
pub mod service;
pub mod settings;
pub mod xmpp;

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, PoisonError};

use module_api::{
    CoreCapability, CorePlugin, GlobalId, ModuleError, ModuleId, ModuleManifest, NodeIdentity, Registration,
};
use service_kit::serve::ConnectionInfo;
use tokio::sync::{mpsc, oneshot};

pub use federation::{
    ChatFederation, ChatFederationConfig, DirectWire, Envelope, Federation, LocalOnly, MuteWire, RoomWire,
};
pub use history::{DirectMessage, History, Mute, RoomMessage};
pub use hooks::{ChatHost, Drivers, Login, LoginRefusal, Player};
pub use moderation::{DashboardSettings, Moderation};
pub use server::Options;
pub use settings::{NickCheck, Settings};

use server::{Backends, Chat, ChatModules, Event};

#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error("chat settings: {0}")]
    Settings(String),
    #[error("chat could not listen on {address} ({source})")]
    Listen {
        address: SocketAddr,
        source: std::io::Error,
    },
}

/// The plugin as the controller holds it: which modules answer chat, and the running server once started.
#[derive(Default)]
pub struct ChatPlugin {
    modules: ChatModules,
}

impl ChatPlugin {
    pub fn new() -> Self {
        Self::default()
    }

    /// The game modules attached with the chat capability.
    pub fn modules(&self) -> BTreeSet<ModuleId> {
        self.modules.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Starts chat's listener and task on this node.
    pub async fn start(
        &self,
        settings: &Settings,
        node: NodeIdentity,
        host: Arc<dyn ChatHost>,
        federation: Arc<dyn Federation>,
        history: History,
        options: Options,
    ) -> Result<Running, StartError> {
        let problems = settings.problems();
        if !problems.is_empty() {
            return Err(StartError::Settings(problems.join("; ")));
        }
        let address = SocketAddr::new(settings.host, settings.port);
        let listener = tokio::net::TcpListener::bind(address)
            .await
            .map_err(|source| StartError::Listen { address, source })?;
        let port = listener.local_addr().map_or(settings.port, |a| a.port());
        tracing::info!(
            "chat: listening on {}:{port} (nick check {})",
            settings.host,
            settings.nick_check.as_str()
        );
        let backends = Backends {
            host: host.clone(),
            federation: federation.clone(),
            history: history.clone(),
            node,
            modules: self.modules.clone(),
        };
        let (events, receiver) = mpsc::unbounded_channel();
        federation.attach(events.clone(), history.clone(), host.clone(), options.clock.clone());
        let chat = Chat::new(backends, settings, options);
        let largest_backlog = chat.largest_backlog.clone();
        let task = tokio::spawn(server::run(chat, receiver));
        let router = axum::Router::new()
            .route("/__chat/mute", axum::routing::post(connection::mute_account))
            .route("/__chat/unmute", axum::routing::post(connection::unmute_account))
            .route("/__chat/history/direct", axum::routing::get(connection::direct_history))
            .route(
                "/__chat/history/waiting",
                axum::routing::get(connection::waiting_history),
            )
            .route("/__chat/tick", axum::routing::post(connection::tick_chat))
            .route("/__chat/client/login", axum::routing::post(connection::client_login))
            .route("/__chat/client/send", axum::routing::post(connection::client_send))
            .route("/__chat/client/drain", axum::routing::get(connection::client_drain))
            .route("/__chat/client/close", axum::routing::post(connection::client_close))
            .fallback(connection::upgrade)
            .with_state(connection::Listener {
                port,
                events: events.clone(),
                gateway_secret: settings.gateway_secret.as_deref().map(Arc::from),
                history: history.clone(),
                federation: federation.clone(),
                host,
                clients: Arc::default(),
            });
        let (stop_listening, stopped) = oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            let serving = axum::serve(
                service_kit::serve::NoDelay(listener),
                router.into_make_service_with_connect_info::<ConnectionInfo>(),
            )
            .with_graceful_shutdown(async move {
                let _ = stopped.await;
            });
            if let Err(error) = serving.await {
                tracing::error!("chat: listener error ({})", error.kind());
            }
        });
        Ok(Running {
            events,
            task,
            server,
            stop_listening,
            port,
            history,
            federation,
            largest_backlog,
        })
    }
}

impl CorePlugin for ChatPlugin {
    fn name(&self) -> &'static str {
        "chat"
    }

    fn serves(&self) -> &'static [CoreCapability] {
        &[CoreCapability::Chat]
    }

    fn attach(&self, module: &ModuleManifest, _registration: &Registration) -> Result<(), ModuleError> {
        if module.capabilities.chat {
            self.modules
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(module.id.clone());
        }
        Ok(())
    }

    fn detach(&self, module: &ModuleId) {
        self.modules
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(module);
    }
}

/// Chat while it runs.
pub struct Running {
    events: mpsc::UnboundedSender<Event>,
    task: tokio::task::JoinHandle<()>,
    server: tokio::task::JoinHandle<()>,
    stop_listening: oneshot::Sender<()>,
    pub port: u16,
    history: History,
    federation: Arc<dyn Federation>,
    /// The most unsent output held for one session so far
    pub largest_backlog: Arc<AtomicUsize>,
}

impl Running {
    pub fn history(&self) -> &History {
        &self.history
    }

    /// Mutes an account until `until` (unix milliseconds; none is until lifted): stored, then its live
    /// sessions follow at once, and propagated to the account's home and peers (DD21, DN10b).
    pub async fn mute(
        &self,
        account: &GlobalId,
        until: Option<i64>,
        reason: Option<String>,
        actor: store::Actor,
    ) -> Result<(), store::Error> {
        let mute = Mute {
            account: account.to_string(),
            until,
            reason,
            by: actor.as_text(),
            at: server::now_ms(),
        };
        self.history.set_mute(mute.clone(), actor.clone()).await?;
        let _ = self.events.send(Event::Muted {
            account: account.clone(),
            mute: Some(mute.clone()),
        });
        self.federation.set_mute(account, Some(mute), actor).await;
        Ok(())
    }

    /// Lifts a mute. Returns whether there was one.
    pub async fn unmute(&self, account: &GlobalId, actor: store::Actor) -> Result<bool, store::Error> {
        let had = self.history.clear_mute(&account.to_string(), actor.clone()).await?;
        let _ = self.events.send(Event::Muted {
            account: account.clone(),
            mute: None,
        });
        self.federation.set_mute(account, None, actor).await;
        Ok(had)
    }

    /// One tick now, for a chat started with `auto_tick` off.
    pub async fn tick(&self) {
        let (done, finished) = oneshot::channel();
        if self.events.send(Event::Tick(done)).is_ok() {
            let _ = finished.await;
        }
    }

    /// Shutdown: `<close/>` to every session, at most 1 s for them to go, then the listener stops.
    pub async fn stop(self) {
        let (done, finished) = oneshot::channel();
        if self.events.send(Event::Shutdown(done)).is_ok() {
            let _ = finished.await;
        }
        let _ = self.task.await;
        let _ = self.stop_listening.send(());
        let _ = self.server.await;
    }
}
