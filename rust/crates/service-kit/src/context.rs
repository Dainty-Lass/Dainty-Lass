//! In-process service lifecycle context (`ServiceContext`), component phases (`ComponentPhase`),
//! and the shared `StatusRegistry` observed by the console REPL, TUI, and GUI Status Page.

use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

/// Lifecycle phase of an individual server component or the overall server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ComponentPhase {
    Stopped,
    Starting,
    Ready,
    Stopping,
    Failed,
    Disabled,
}

impl ComponentPhase {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Starting => "starting",
            Self::Ready => "ready",
            Self::Stopping => "stopping",
            Self::Failed => "failed",
            Self::Disabled => "disabled",
        }
    }
}

/// Live status of one orchestrated component (`gateway`, `allowlist`, `content`, `admin`, `backend`, `director`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentStatus {
    pub name: String,
    pub edition: Option<String>,
    pub phase: ComponentPhase,
    pub listen_addr: Option<String>,
    pub port: Option<u16>,
    pub pid: Option<u32>,
    pub restarts: u32,
    pub started_at_ms: Option<i64>,
    pub uptime_secs: u64,
    pub last_error: Option<String>,
    pub detail: String,
}

impl ComponentStatus {
    pub fn new(name: impl Into<String>, edition: Option<String>) -> Self {
        Self {
            name: name.into(),
            edition,
            phase: ComponentPhase::Stopped,
            listen_addr: None,
            port: None,
            pid: None,
            restarts: 0,
            started_at_ms: None,
            uptime_secs: 0,
            last_error: None,
            detail: String::new(),
        }
    }
}

/// Snapshot of the entire orchestrated server state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerSnapshot {
    pub overall_phase: ComponentPhase,
    pub mode: String,
    pub editions: Vec<String>,
    pub components: Vec<ComponentStatus>,
    pub invite_url: Option<String>,
    pub cert_fingerprint: Option<String>,
    pub online_players: u32,
    pub active_hunts: u32,
    pub max_hunts: u32,
}

impl ServerSnapshot {
    pub fn get(&self, name: &str) -> Option<&ComponentStatus> {
        self.components.iter().find(|c| c.name == name)
    }
}

impl Default for ServerSnapshot {
    fn default() -> Self {
        Self {
            overall_phase: ComponentPhase::Stopped,
            mode: "private".into(),
            editions: Vec::new(),
            components: Vec::new(),
            invite_url: None,
            cert_fingerprint: None,
            online_players: 0,
            active_hunts: 0,
            max_hunts: 4,
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Thread-safe registry tracking every component's live status and broadcasting `ServerSnapshot` updates.
#[derive(Clone)]
pub struct StatusRegistry {
    state: Arc<Mutex<ServerSnapshot>>,
    tx: Arc<watch::Sender<ServerSnapshot>>,
}

impl Default for StatusRegistry {
    fn default() -> Self {
        Self::new("private", Vec::new())
    }
}

impl StatusRegistry {
    pub fn new(mode: impl Into<String>, editions: Vec<String>) -> Self {
        let initial = ServerSnapshot {
            overall_phase: ComponentPhase::Stopped,
            mode: mode.into(),
            editions,
            components: Vec::new(),
            invite_url: None,
            cert_fingerprint: None,
            online_players: 0,
            active_hunts: 0,
            max_hunts: 4,
        };
        let (tx, _) = watch::channel(initial.clone());
        Self {
            state: Arc::new(Mutex::new(initial)),
            tx: Arc::new(tx),
        }
    }

    pub fn service_context(&self, name: &str) -> (ServiceContext, watch::Sender<bool>) {
        let reporter = self.register_component(name, None);
        reporter.starting();
        let (stop_tx, stop_rx) = watch::channel(false);
        (ServiceContext::new(name, reporter, stop_rx), stop_tx)
    }

    pub fn snapshot(&self) -> ServerSnapshot {
        lock(&self.state).clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<ServerSnapshot> {
        self.tx.subscribe()
    }

    pub fn set_overall_phase(&self, phase: ComponentPhase) {
        let snap = {
            let mut st = lock(&self.state);
            st.overall_phase = phase;
            st.clone()
        };
        let _ = self.tx.send_replace(snap);
    }

    pub fn set_invite_info(&self, invite_url: Option<String>, cert_fingerprint: Option<String>) {
        let snap = {
            let mut st = lock(&self.state);
            st.invite_url = invite_url;
            st.cert_fingerprint = cert_fingerprint;
            st.clone()
        };
        let _ = self.tx.send_replace(snap);
    }

    pub fn register_component(&self, name: &str, edition: Option<&str>) -> StatusReporter {
        {
            let mut st = lock(&self.state);
            if !st.components.iter().any(|c| c.name == name) {
                st.components
                    .push(ComponentStatus::new(name, edition.map(str::to_owned)));
            }
            let _ = self.tx.send_replace(st.clone());
        }
        StatusReporter {
            name: name.to_owned(),
            registry: self.clone(),
        }
    }

    fn update_component(&self, name: &str, f: impl FnOnce(&mut ComponentStatus)) {
        let snap = {
            let mut st = lock(&self.state);
            if let Some(comp) = st.components.iter_mut().find(|c| c.name == name) {
                f(comp);
            } else {
                let mut comp = ComponentStatus::new(name, None);
                f(&mut comp);
                st.components.push(comp);
            }
            st.clone()
        };
        let _ = self.tx.send_replace(snap);
    }
}

/// Handle used by a single component to report its phase and bind address to the `StatusRegistry`.
#[derive(Clone)]
pub struct StatusReporter {
    name: String,
    registry: StatusRegistry,
}

impl StatusReporter {
    pub fn noop(name: impl Into<String>) -> Self {
        let name = name.into();
        let registry = StatusRegistry::default();
        registry.register_component(&name, None)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn starting(&self) {
        self.registry.update_component(&self.name, |c| {
            c.phase = ComponentPhase::Starting;
            c.last_error = None;
        });
    }

    pub fn report_starting(&self, detail: impl Into<String>) {
        let detail = detail.into();
        self.registry.update_component(&self.name, |c| {
            c.phase = ComponentPhase::Starting;
            c.detail = detail;
            c.last_error = None;
        });
    }

    pub fn ready(&self, listen_addr: Option<String>, detail: Option<String>) {
        let now = now_ms();
        let port = listen_addr
            .as_deref()
            .and_then(|s| s.rsplit_once(':'))
            .and_then(|(_, p)| p.parse::<u16>().ok());
        self.registry.update_component(&self.name, |c| {
            c.phase = ComponentPhase::Ready;
            c.listen_addr = listen_addr;
            if port.is_some() {
                c.port = port;
            }
            c.started_at_ms = Some(now);
            c.last_error = None;
            c.detail = detail.unwrap_or_default();
        });
    }

    pub fn report_ready(&self, port: Option<u16>, detail: impl Into<String>) {
        let now = now_ms();
        let detail = detail.into();
        self.registry.update_component(&self.name, |c| {
            c.phase = ComponentPhase::Ready;
            c.port = port;
            c.listen_addr = port.map(|p| format!("127.0.0.1:{p}"));
            c.started_at_ms = Some(now);
            c.last_error = None;
            c.detail = detail;
        });
    }

    pub fn stopping(&self) {
        self.registry.update_component(&self.name, |c| {
            c.phase = ComponentPhase::Stopping;
        });
    }

    pub fn report_stopping(&self, detail: impl Into<String>) {
        let detail = detail.into();
        self.registry.update_component(&self.name, |c| {
            c.phase = ComponentPhase::Stopping;
            c.detail = detail;
        });
    }

    pub fn stopped(&self) {
        self.registry.update_component(&self.name, |c| {
            c.phase = ComponentPhase::Stopped;
            c.pid = None;
        });
    }

    pub fn report_stopped(&self, detail: impl Into<String>) {
        let detail = detail.into();
        self.registry.update_component(&self.name, |c| {
            c.phase = ComponentPhase::Stopped;
            c.pid = None;
            c.detail = detail;
        });
    }

    pub fn failed(&self, error: impl Into<String>) {
        let err = error.into();
        self.registry.update_component(&self.name, |c| {
            c.phase = ComponentPhase::Failed;
            c.detail.clone_from(&err);
            c.last_error = Some(err);
        });
    }

    pub fn disabled(&self, reason: impl Into<String>) {
        self.registry.update_component(&self.name, |c| {
            c.phase = ComponentPhase::Disabled;
            c.detail = reason.into();
        });
    }

    pub fn set_pid(&self, pid: Option<u32>) {
        self.registry.update_component(&self.name, |c| {
            c.pid = pid;
        });
    }

    pub fn inc_restarts(&self) {
        self.registry.update_component(&self.name, |c| {
            c.restarts += 1;
        });
    }
}

/// Execution context passed to each in-process service library (`gateway`, `content`, `admin`, `backend`, `director`).
#[derive(Clone)]
pub struct ServiceContext {
    pub name: String,
    pub status: StatusReporter,
    pub stop: watch::Receiver<bool>,
}

impl ServiceContext {
    pub fn new(name: impl Into<String>, status: StatusReporter, stop: watch::Receiver<bool>) -> Self {
        Self {
            name: name.into(),
            status,
            stop,
        }
    }

    /// Creates a standalone `ServiceContext` that stops when `stop_requested()` resolves.
    pub fn standalone(name: impl Into<String>) -> Self {
        let name = name.into();
        let (tx, rx) = watch::channel(false);
        tokio::spawn(async move {
            crate::serve::stop_requested().await;
            let _ = tx.send(true);
        });
        Self {
            status: StatusReporter::noop(name.clone()),
            name,
            stop: rx,
        }
    }

    /// Waits until a stop has been requested on this context.
    pub async fn wait_stop(&mut self) {
        let _ = self.stop.wait_for(|stopped| *stopped).await;
    }

    pub fn status(&self) -> &StatusReporter {
        &self.status
    }

    pub fn stop_receiver(&self) -> watch::Receiver<bool> {
        self.stop.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_tracks_component_phases_and_notifies_subscribers() {
        let registry = StatusRegistry::new("public", vec!["reforged".into(), "refined".into()]);
        let rx = registry.subscribe();
        let gw = registry.register_component("gateway", None);
        gw.starting();
        assert_eq!(rx.borrow().components[0].phase, ComponentPhase::Starting);
        gw.ready(Some("0.0.0.0:443".into()), Some("TLS".into()));
        assert_eq!(rx.borrow().components[0].phase, ComponentPhase::Ready);
        assert_eq!(rx.borrow().components[0].listen_addr.as_deref(), Some("0.0.0.0:443"));
        gw.stopping();
        gw.stopped();
        assert_eq!(rx.borrow().components[0].phase, ComponentPhase::Stopped);
    }
}
