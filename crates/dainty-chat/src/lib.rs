// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Chat (DN10): XMPP over WebSocket for the game's client, as a core plugin. One task owns every session and room
//! ([`server`], [`muc`]); the listener hands it frames ([`connection`]); history, federation and moderation are its
//! seams. Game modules with the chat capability give it JIDs and room rules through `dainty_module_api::chat`.

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
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use dainty_module_api::{
    CoreCapability, CorePlugin, GlobalId, ModuleError, ModuleId, ModuleManifest, NodeIdentity, Registration,
};
use tokio::sync::{mpsc, oneshot};
use tracing::info;

pub use federation::{
    ChatFederation, ChatFederationConfig, DirectWire, Envelope, Federation, LocalOnly, MuteWire, RoomWire,
};
pub use history::{DirectMessage, History, Mute, RoomMessage};
pub use hooks::{ChatHost, Drivers, Login, LoginRefusal, Player};
pub use moderation::{DashboardSettings, Moderation};
pub use server::Options;
pub use settings::{NickCheck, Settings};

use crate::connection::Shared;
use crate::server::{Chat, Event, Parts};

/// How long the listener waits for its requests once told to stop
const DRAIN: Duration = Duration::from_secs(1);

/// Why [`ChatPlugin::start`] failed.
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    /// The settings were refused; the problems joined with `; `.
    #[error("chat settings: {0}")]
    Settings(String),
    /// The listener could not bind.
    #[error("chat could not listen on {address} ({source})")]
    Listen {
        /// The address it tried.
        address: SocketAddr,
        /// The bind error.
        source: std::io::Error,
    },
}

/// Chat as a core plugin: the modules that answer chat, shared with every chat it starts.
#[derive(Default)]
pub struct ChatPlugin {
    modules: Arc<Mutex<BTreeSet<ModuleId>>>,
}

impl ChatPlugin {
    /// A plugin that answers for no module yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The modules attached that answer chat.
    pub fn modules(&self) -> BTreeSet<ModuleId> {
        self.modules.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Checks the settings, binds the listener and starts the chat task. Errors before anything is spawned.
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
        let port = listener
            .local_addr()
            .map_err(|source| StartError::Listen { address, source })?
            .port();
        info!(
            "chat: listening on {}:{port} nick_check={}",
            settings.host,
            settings.nick_check.as_str()
        );
        let (events, receiver) = mpsc::unbounded_channel();
        federation.attach(events.clone(), history.clone(), host.clone(), options.clock.clone());
        let largest_backlog = Arc::new(AtomicUsize::new(0));
        let chat = Chat::new(Parts {
            settings: settings.clone(),
            node,
            host: host.clone(),
            federation: federation.clone(),
            history: history.clone(),
            options,
            modules: self.modules.clone(),
            largest_backlog: largest_backlog.clone(),
        });
        let task = tokio::spawn(chat.run(receiver));
        let shared = Arc::new(Shared::new(
            events,
            history,
            host,
            federation,
            settings.gateway_secret.clone(),
            port,
        ));
        let (stop, stopped) = oneshot::channel::<()>();
        let app = dainty_service_kit::task::monitored("chat", connection::router(shared.clone()));
        let server = tokio::spawn(async move {
            let stopped = async {
                _ = stopped.await;
            };
            if let Err(e) = dainty_service_kit::serve::serve(listener, app, stopped, DRAIN).await {
                tracing::error!("chat: the listener failed: {e}");
            }
        });
        Ok(Running {
            port,
            largest_backlog,
            shared,
            task,
            server,
            stop,
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

    fn attach(&self, module: &ModuleManifest, _: &Registration) -> Result<(), ModuleError> {
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

/// A started chat.
pub struct Running {
    /// The port it listens on: the one actually bound when the settings asked for 0.
    pub port: u16,
    /// The most unsent output any connection has held, bytes
    pub largest_backlog: Arc<AtomicUsize>,
    shared: Arc<Shared>,
    task: tokio::task::JoinHandle<()>,
    server: tokio::task::JoinHandle<()>,
    stop: oneshot::Sender<()>,
}

impl Running {
    /// Chat's database.
    pub fn history(&self) -> &History {
        &self.shared.history
    }

    /// Mutes `account` until `until` (unix milliseconds, none for until lifted): stored, applied to live sessions and
    /// pushed to the other nodes.
    pub async fn mute(
        &self,
        account: &GlobalId,
        until: Option<i64>,
        reason: Option<String>,
        actor: dainty_store::Actor,
    ) -> Result<(), dainty_store::Error> {
        self.shared.mute(account, until, reason, actor).await
    }

    /// Lifts the mute on `account` everywhere; returns whether there was one.
    pub async fn unmute(&self, account: &GlobalId, actor: dainty_store::Actor) -> Result<bool, dainty_store::Error> {
        self.shared.unmute(account, actor).await
    }

    /// Ends every session bound to the JID local part `uid` (a staff kick).
    pub fn disconnect(&self, uid: &str, reason: &str) {
        _ = self.shared.events.send(Event::Disconnect {
            uid: uid.to_owned(),
            reason: reason.to_owned(),
        });
    }

    /// A handle that ends a JID local part's sessions while chat runs (for the core's [`Running::disconnect`]).
    pub fn disconnector(&self) -> impl Fn(&str, &str) + Send + Sync + 'static {
        let events = self.shared.events.clone();
        move |uid: &str, reason: &str| {
            _ = events.send(Event::Disconnect {
                uid: uid.to_owned(),
                reason: reason.to_owned(),
            });
        }
    }

    /// Runs the periodic work now and waits for it.
    pub async fn tick(&self) {
        self.shared.tick().await;
    }

    /// Ends every session, stops the task and then the listener.
    pub async fn stop(self) {
        _ = self.shared.events.send(Event::Shutdown);
        _ = self.task.await;
        _ = self.stop.send(());
        _ = self.server.await;
    }
}
