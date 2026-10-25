//! In-process `ServerController` integration for `server-gui*` binaries (`docs/app-structure-plan.md`).
//!
//! Compiled only when the `server-gui` feature is enabled; `online-client*` binaries exclude this
//! module and all server crates completely.

use std::path::PathBuf;
use std::sync::Arc;

use protocol::edition::EditionSelection;
use server_controller::ServerController;
use service_kit::context::ServerSnapshot;
use service_kit::telemetry::{ChannelLogLayer, HostLogEntry, LogFilterReloadHandle};
use tokio::sync::{Mutex, mpsc, watch};

/// Holds the in-process `ServerController` and `LauncherGuiLayer` log channel for `server-gui*`.
pub struct EmbeddedServerState {
    compiled_editions: EditionSelection,
    controller: Arc<Mutex<Option<ServerController>>>,
    log_layer: ChannelLogLayer,
    filter_handle: Option<LogFilterReloadHandle>,
}

impl EmbeddedServerState {
    pub fn new(
        compiled_editions: EditionSelection,
        log_layer: ChannelLogLayer,
        filter_handle: Option<LogFilterReloadHandle>,
    ) -> Self {
        Self {
            compiled_editions,
            controller: Arc::new(Mutex::new(None)),
            log_layer,
            filter_handle,
        }
    }

    pub fn compiled_editions(&self) -> EditionSelection {
        self.compiled_editions
    }

    pub fn log_layer(&self) -> &ChannelLogLayer {
        &self.log_layer
    }

    pub fn filter_handle(&self) -> Option<&LogFilterReloadHandle> {
        self.filter_handle.as_ref()
    }

    /// Starts the server of the checkout at `root` (the folder holding `.workspace/`) in this process.
    pub async fn start(&self, root: PathBuf) -> Result<watch::Receiver<ServerSnapshot>, String> {
        let mut guard = self.controller.lock().await;
        if let Some(existing) = guard.as_ref() {
            return Ok(existing.subscribe_status());
        }
        let controller = host::serve::start_embedded(&host::paths::Paths::new(root)).await?;
        let rx = controller.subscribe_status();
        *guard = Some(controller);
        Ok(rx)
    }

    /// Stops the in-process `ServerController` in reverse dependency order.
    pub async fn stop(&self) {
        let mut guard = self.controller.lock().await;
        if let Some(mut controller) = guard.take() {
            controller.shutdown().await;
        }
    }
}

/// Installs the global `tracing` subscriber with `LauncherGuiLayer` (`ChannelLogLayer`) and returns
/// the layer, receiver, and reload handle.
pub fn init_gui_telemetry() -> (
    ChannelLogLayer,
    mpsc::UnboundedReceiver<HostLogEntry>,
    Option<LogFilterReloadHandle>,
) {
    let (layer, rx, handle) =
        service_kit::telemetry::init_gui_channel_subscriber(tracing::level_filters::LevelFilter::INFO);
    (layer, rx, Some(handle))
}
