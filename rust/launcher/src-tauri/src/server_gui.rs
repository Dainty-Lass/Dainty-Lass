//! The server GUI (`server-gui` feature, RD26): the host kit's server running inside the launcher, driven
//! by the Host page's server panel (`platform::hostserver::EmbeddedServer`) in place of a child
//! `reforged-host start`. Its services' log events come over [`ChannelLogLayer`]; only the server's own
//! (events inside a service's span, and the host kit's) reach the page.

use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

use server_controller::ServerController;
use service_kit::context::{ComponentPhase, ServerSnapshot};
use service_kit::telemetry::{ChannelLogLayer, HostLogEntry};
use tokio::sync::{Mutex, mpsc, watch};

use crate::bridge::types::{HostComponentPhase, HostServerComponent};
use crate::platform::hostserver::{EmbeddedServer, LineSink};

type Boxed<T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send>>;
type Slot<T> = Arc<StdMutex<Option<T>>>;

fn lock<T>(mutex: &StdMutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub struct EmbeddedServerState {
    controller: Arc<Mutex<Option<ServerController>>>,
    /// The services' state, read without waiting on the controller (it is busy while starting)
    snapshot: Slot<watch::Receiver<ServerSnapshot>>,
    sink: Slot<LineSink>,
    logs: StdMutex<Option<mpsc::UnboundedReceiver<HostLogEntry>>>,
}

impl EmbeddedServerState {
    pub fn new(logs: mpsc::UnboundedReceiver<HostLogEntry>) -> Self {
        Self {
            controller: Arc::new(Mutex::new(None)),
            snapshot: Arc::new(StdMutex::new(None)),
            sink: Arc::new(StdMutex::new(None)),
            logs: StdMutex::new(Some(logs)),
        }
    }

    /// The log events into the page's lines, once: the server's services and the host kit, not the
    /// launcher's own events.
    fn drain_logs(&self) {
        let Some(mut logs) = lock(&self.logs).take() else {
            return;
        };
        let (sink, snapshot) = (self.sink.clone(), self.snapshot.clone());
        tauri::async_runtime::spawn(async move {
            while let Some(entry) = logs.recv().await {
                // A game module logs under its id, which is a component of the server's too (DF3)
                let module = || {
                    lock(&snapshot)
                        .as_ref()
                        .is_some_and(|snapshot| snapshot.borrow().get(&entry.service).is_some())
                };
                let ours = entry.service == "host"
                    || crate::bridge::spec::HOST_SERVER_COMPONENTS.contains(&entry.service.as_str())
                    || module();
                let sink = lock(&sink).clone();
                if let (true, Some(sink)) = (ours, sink) {
                    sink(format!("{:<14} | {}", entry.service, entry.message));
                }
            }
        });
    }
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

impl EmbeddedServer for EmbeddedServerState {
    fn start(&self, workspace: PathBuf, lines: LineSink) -> Boxed<Result<(), String>> {
        *lock(&self.sink) = Some(lines.clone());
        self.drain_logs();
        let (controller, snapshot) = (self.controller.clone(), self.snapshot.clone());
        Box::pin(async move {
            let mut guard = controller.lock().await;
            if guard.is_some() {
                return Ok(());
            }
            let printer: host::services::Printer =
                Arc::new(move |name: &str, line: &str| lines(format!("{name:<14} | {line}")));
            // Setup's checks and files, and the 1.4.4 stack's migrations, block: off the async threads
            let mut server = tauri::async_runtime::spawn_blocking(move || {
                host::serve::prepare_embedded(&host::paths::Paths::new(workspace), printer)
            })
            .await
            .map_err(|e| e.to_string())??;
            *lock(&snapshot) = Some(server.subscribe_status());
            if let Err(why) = server.start_all().await {
                server.shutdown().await;
                *lock(&snapshot) = None;
                return Err(why);
            }
            *guard = Some(server);
            Ok(())
        })
    }

    fn stop(&self) -> Boxed<()> {
        let (controller, snapshot) = (self.controller.clone(), self.snapshot.clone());
        Box::pin(async move {
            if let Some(mut server) = controller.lock().await.take() {
                server.shutdown().await;
            }
            *lock(&snapshot) = None;
        })
    }

    fn restart(&self, name: String) -> Boxed<Result<(), String>> {
        let controller = self.controller.clone();
        Box::pin(async move {
            match controller.lock().await.as_mut() {
                Some(server) => server.restart_component(&name).await,
                None => Err("the server is not running".into()),
            }
        })
    }

    fn components(&self) -> Vec<HostServerComponent> {
        let Some(snapshot) = lock(&self.snapshot).clone() else {
            return Vec::new();
        };
        let components = snapshot.borrow().components.clone();
        components
            .into_iter()
            .map(|c| HostServerComponent {
                name: c.name,
                phase: phase(c.phase),
                port: c.port,
                detail: c.detail,
            })
            .collect()
    }
}

/// The launcher's own log layer and the server's channel, as one subscriber (the launcher's
/// `platform::log::install` then finds one set and only adds its panic hook).
pub fn init_telemetry() -> mpsc::UnboundedReceiver<HostLogEntry> {
    use tracing_subscriber::Layer as _;
    use tracing_subscriber::layer::SubscriberExt as _;
    use tracing_subscriber::util::SubscriberInitExt as _;
    let (tx, rx) = mpsc::unbounded_channel();
    let channel = ChannelLogLayer::new(tx).with_filter(tracing::level_filters::LevelFilter::INFO);
    let installed = tracing_subscriber::registry()
        .with(crate::platform::log::Layer)
        .with(channel)
        .try_init();
    if installed.is_ok() {
        service_kit::telemetry::set_tracing_bridge_active(true);
    }
    rx
}
