// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The manager's embedded server (`docs/port/spec-tauri-shell.md` §4–8, RD26): the services, the host kit, the
//! module panels and the history run in the manager's own process. The app hands [`EmbeddedServerState`] to the
//! launcher kit's controller; the kit checks every page argument before any of this sees it.

pub mod engine;
pub mod history;
pub mod modules;

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use dainty_controller::ServerController;
use dainty_launcher_kit::bridge::modules::ModulePanel;
use dainty_launcher_kit::bridge::spec::HOST_SERVER_COMPONENTS;
use dainty_launcher_kit::platform::hostserver::{EmbeddedServer, HostComponentPhase, HostServerComponent, LineSink};
use dainty_launcher_kit::platform::server_engine::ServerEngine;
use dainty_service_kit::context::{ComponentPhase, ServerSnapshot};
use dainty_service_kit::telemetry::{ChannelLogLayer, HostLogEntry};
use tokio::sync::{mpsc, watch};

pub use engine::Engine;
pub use modules::ServerModules;

type Boxed<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// A std mutex's guard, poisoned or not: every value kept here stays whole between statements.
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Installs the manager's tracing before the app starts (§8): the launcher's own log, and every event of the
/// in-process services into the returned channel at `INFO`, which [`EmbeddedServerState`] forwards to the page.
/// When a subscriber is already set the channel stays silent.
pub fn init_telemetry() -> mpsc::UnboundedReceiver<HostLogEntry> {
    use tracing_subscriber::Layer as _;
    use tracing_subscriber::layer::SubscriberExt as _;
    use tracing_subscriber::util::SubscriberInitExt as _;

    let (tx, rx) = mpsc::unbounded_channel();
    let services = ChannelLogLayer::new(tx).with_filter(tracing_subscriber::filter::LevelFilter::INFO);
    let installed = tracing_subscriber::registry()
        .with(dainty_launcher_kit::platform::log::LogFilter)
        .with(dainty_launcher_kit::platform::log::LogLayer)
        .with(services)
        .try_init()
        .is_ok();
    if installed {
        dainty_service_kit::telemetry::set_tracing_bridge_active(true);
    }
    rx
}

/// Whether a log entry is the server's own and goes to the page (§4.1); a game module logs under its id (DF3).
pub fn is_server_line(service: &str, snapshot: Option<&ServerSnapshot>) -> bool {
    service == "host"
        || HOST_SERVER_COMPONENTS.contains(&service)
        || snapshot.is_some_and(|s| s.components.iter().any(|c| c.name == service))
}

fn phase(phase: ComponentPhase) -> HostComponentPhase {
    match phase {
        ComponentPhase::Stopped => HostComponentPhase::Stopped,
        ComponentPhase::Starting => HostComponentPhase::Starting,
        ComponentPhase::Ready => HostComponentPhase::Ready,
        ComponentPhase::Stopping => HostComponentPhase::Stopping,
        ComponentPhase::Failed => HostComponentPhase::Failed,
        ComponentPhase::Disabled => HostComponentPhase::Disabled,
    }
}

struct Inner {
    /// Async: a start holds it for as long as the services take.
    controller: tokio::sync::Mutex<Option<ServerController>>,
    /// Read without the controller's lock, which a start holds.
    status: Mutex<Option<watch::Receiver<ServerSnapshot>>>,
    sink: Mutex<Option<LineSink>>,
    logs: Mutex<Option<mpsc::UnboundedReceiver<HostLogEntry>>>,
    modules: ServerModules,
    engine: Arc<Engine>,
}

/// The manager's in-process server, made once at start-up.
#[derive(Clone)]
pub struct EmbeddedServerState {
    inner: Arc<Inner>,
}

impl EmbeddedServerState {
    /// A stopped server whose services log into `logs` ([`init_telemetry`]'s receiver).
    pub fn new(logs: mpsc::UnboundedReceiver<HostLogEntry>) -> Self {
        Self {
            inner: Arc::new(Inner {
                controller: tokio::sync::Mutex::new(None),
                status: Mutex::new(None),
                sink: Mutex::new(None),
                logs: Mutex::new(Some(logs)),
                modules: ServerModules::new(),
                engine: Arc::new(Engine),
            }),
        }
    }

    /// The module panel over this server's registry.
    pub fn server_modules(&self) -> ServerModules {
        self.inner.modules.clone()
    }

    fn clear(&self) {
        *lock(&self.inner.status) = None;
        self.inner.modules.detach_registry();
    }
}

/// Forwards the services' log to whichever page sink is current; the launcher's own events never reach it.
fn forward_logs(inner: Arc<Inner>, mut logs: mpsc::UnboundedReceiver<HostLogEntry>) {
    tokio::spawn(async move {
        while let Some(entry) = logs.recv().await {
            let snapshot = lock(&inner.status).as_ref().map(|rx| rx.borrow().clone());
            if !is_server_line(&entry.service, snapshot.as_ref()) {
                continue;
            }
            let sink = lock(&inner.sink).clone();
            if let Some(sink) = sink {
                sink(format!("{:<14} | {}", entry.service, entry.message));
            }
        }
    });
}

impl EmbeddedServer for EmbeddedServerState {
    fn start(&self, workspace: PathBuf, lines: LineSink) -> Boxed<Result<(), String>> {
        let this = self.clone();
        Box::pin(async move {
            let inner = &this.inner;
            *lock(&inner.sink) = Some(lines.clone());
            let logs = lock(&inner.logs).take();
            if let Some(logs) = logs {
                forward_logs(inner.clone(), logs);
            }
            let mut slot = inner.controller.lock().await;
            if slot.is_some() {
                return Ok(());
            }
            inner.modules.set_workspace(Some(workspace.clone()));
            // Setup's checks, its files and the migrations block
            let paths = dainty_host::paths::Paths::new(workspace);
            let printer: dainty_host::services::Printer =
                Arc::new(move |name: &str, line: &str| lines(format!("{name:<14} | {line}")));
            let mut server = tokio::task::spawn_blocking(move || dainty_host::serve::prepare_embedded(&paths, printer))
                .await
                .map_err(|error| error.to_string())??;
            *lock(&inner.status) = Some(server.subscribe_status());
            inner.modules.attach_registry(server.registry().cloned());
            if let Err(why) = server.start_all().await {
                server.shutdown().await;
                this.clear();
                return Err(why);
            }
            *slot = Some(server);
            Ok(())
        })
    }

    fn stop(&self) -> Boxed<()> {
        let this = self.clone();
        Box::pin(async move {
            let server = this.inner.controller.lock().await.take();
            if let Some(mut server) = server {
                server.shutdown().await;
            }
            this.clear();
        })
    }

    fn restart(&self, name: String) -> Boxed<Result<(), String>> {
        let this = self.clone();
        Box::pin(async move {
            match this.inner.controller.lock().await.as_mut() {
                Some(server) => server.restart_component(&name).await,
                None => Err("the server is not running".to_owned()),
            }
        })
    }

    fn components(&self) -> Vec<HostServerComponent> {
        let Some(snapshot) = lock(&self.inner.status).as_ref().map(|rx| rx.borrow().clone()) else {
            return Vec::new();
        };
        snapshot
            .components
            .into_iter()
            .map(|c| HostServerComponent {
                name: c.name,
                phase: phase(c.phase),
                port: c.port,
                detail: c.detail,
            })
            .collect()
    }

    fn modules(&self) -> Option<Arc<dyn ModulePanel>> {
        Some(Arc::new(self.inner.modules.clone()))
    }

    fn engine(&self) -> Option<Arc<dyn ServerEngine>> {
        Some(self.inner.engine.clone())
    }
}

#[cfg(test)]
mod tests {
    use dainty_service_kit::context::ComponentStatus;

    use super::*;

    #[test]
    fn only_the_servers_own_lines_reach_the_page() {
        let mut snapshot = ServerSnapshot {
            components: vec![ComponentStatus::new("stub", None)],
            ..ServerSnapshot::default()
        };
        assert!(is_server_line("host", None));
        assert!(is_server_line("gateway", None));
        assert!(is_server_line("backend", None));
        assert!(is_server_line("stub", Some(&snapshot)));
        assert!(!is_server_line("stub", None));
        assert!(!is_server_line("dainty-launcher-kit", Some(&snapshot)));
        snapshot.components.clear();
        assert!(!is_server_line("stub", Some(&snapshot)));
    }
}
