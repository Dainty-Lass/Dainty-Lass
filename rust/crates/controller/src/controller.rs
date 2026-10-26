//! [`ServerController`]: starts modules in order, stops them in reverse, and keeps every one's
//! phase in a [`StatusRegistry`] the REPL, the TUI and the GUI read.

use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use service_kit::context::{ComponentPhase, ServerSnapshot, ServiceContext, StatusRegistry};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::Instrument as _;

/// How long a module may take to report `Ready` unless it says otherwise.
pub const START_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a module may take to stop before it is aborted.
pub const STOP_GRACE: Duration = Duration::from_secs(10);

pub type ModuleFuture = Pin<Box<dyn Future<Output = Result<(), String>> + Send>>;
/// Runs the module until its context says stop; called again on every restart.
pub type ModuleStart = Arc<dyn Fn(ServiceContext) -> ModuleFuture + Send + Sync>;

/// One thing the controller runs: a Rust service in this process, or a program it supervises.
#[derive(Clone)]
pub struct Module {
    pub name: String,
    pub edition: Option<String>,
    pub ready_timeout: Duration,
    /// How long it may take to stop before it is aborted.
    pub stop_grace: Duration,
    /// The server does not run without it: a start that fails stops `start_all`.
    pub required: bool,
    pub start: ModuleStart,
}

impl Module {
    pub fn new(name: impl Into<String>, start: ModuleStart) -> Self {
        Self {
            name: name.into(),
            edition: None,
            ready_timeout: START_TIMEOUT,
            stop_grace: STOP_GRACE,
            required: true,
            start,
        }
    }

    pub fn optional(mut self) -> Self {
        self.required = false;
        self
    }

    pub fn ready_within(mut self, timeout: Duration) -> Self {
        self.ready_timeout = timeout;
        self
    }

    pub fn stop_within(mut self, grace: Duration) -> Self {
        self.stop_grace = grace;
        self
    }

    pub fn for_edition(mut self, edition: impl Into<String>) -> Self {
        self.edition = Some(edition.into());
        self
    }
}

struct Running {
    stop_tx: watch::Sender<bool>,
    handle: JoinHandle<Result<(), String>>,
}

pub struct ServerController {
    status_registry: StatusRegistry,
    modules: Vec<Module>,
    running: BTreeMap<String, Running>,
    startup_order: Vec<String>,
}

impl ServerController {
    pub fn new(mode: impl Into<String>, editions: Vec<String>) -> Self {
        Self {
            status_registry: StatusRegistry::new(mode, editions),
            modules: Vec::new(),
            running: BTreeMap::new(),
            startup_order: Vec::new(),
        }
    }

    /// Adds a module after the ones added before it; a second module of the same name replaces the first.
    pub fn add(&mut self, module: Module) -> &mut Self {
        self.modules.retain(|m| m.name != module.name);
        self.status_registry
            .register_component(&module.name, module.edition.as_deref());
        self.modules.push(module);
        self
    }

    pub fn module_names(&self) -> Vec<String> {
        self.modules.iter().map(|m| m.name.clone()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }

    pub fn status_registry(&self) -> &StatusRegistry {
        &self.status_registry
    }

    pub fn subscribe_status(&self) -> watch::Receiver<ServerSnapshot> {
        self.status_registry.subscribe()
    }

    pub fn snapshot(&self) -> ServerSnapshot {
        self.status_registry.snapshot()
    }

    /// What the snapshot shows as the server's invite and certificate.
    pub fn set_invite_info(&self, invite_url: Option<String>, cert_fingerprint: Option<String>) {
        self.status_registry.set_invite_info(invite_url, cert_fingerprint);
    }

    /// Starts every module in the order added. An optional module that fails is left `Failed`; a
    /// required one stops the start with its error, and what started stays up for `shutdown`.
    pub async fn start_all(&mut self) -> Result<(), String> {
        self.status_registry.set_overall_phase(ComponentPhase::Starting);
        let names = self.module_names();
        for name in names {
            if let Err(error) = self.start_component(&name).await {
                let required = self.modules.iter().any(|m| m.name == name && m.required);
                if required {
                    self.status_registry.set_overall_phase(ComponentPhase::Failed);
                    return Err(error);
                }
                tracing::warn!(target: "host", "{error}");
            }
        }
        self.status_registry.set_overall_phase(ComponentPhase::Ready);
        Ok(())
    }

    pub async fn start_component(&mut self, name: &str) -> Result<(), String> {
        if self.running.contains_key(name) {
            return Err(format!("component {name} is already running"));
        }
        let module = self
            .modules
            .iter()
            .find(|m| m.name == name)
            .cloned()
            .ok_or_else(|| format!("unknown component `{name}`"))?;
        let (ctx, stop_tx) = self.status_registry.service_context(name);
        let start = module.start.clone();
        let span_name = name.to_string();
        let handle = tokio::spawn(
            async move { start(ctx).await }.instrument(tracing::info_span!("component", service = %span_name)),
        );
        self.record_started(name, module.ready_timeout, stop_tx, handle).await
    }

    pub async fn stop_component(&mut self, name: &str) -> Result<(), String> {
        let Some(mut running) = self.running.remove(name) else {
            return Err(format!("component {name} is not running"));
        };
        let grace = self
            .modules
            .iter()
            .find(|m| m.name == name)
            .map_or(STOP_GRACE, |m| m.stop_grace);
        let _ = running.stop_tx.send(true);
        let status = self.status_registry.register_component(name, None);
        // A component that ignores its stop signal must not keep its port and threads
        if tokio::time::timeout(grace, &mut running.handle).await.is_err() {
            tracing::warn!(target: "host", "component {name} did not stop within {grace:?}; aborting it");
            running.handle.abort();
            let _ = (&mut running.handle).await;
            status.report_stopped(format!("aborted: did not stop within {grace:?}"));
        } else if !matches!(
            self.snapshot().get(name).map(|c| c.phase),
            Some(ComponentPhase::Stopped | ComponentPhase::Failed)
        ) {
            // Its task ended: what the views show must not stay `Ready`
            status.stopped();
        }
        self.startup_order.retain(|n| n != name);
        Ok(())
    }

    pub async fn restart_component(&mut self, name: &str) -> Result<(), String> {
        if self.running.contains_key(name) {
            self.stop_component(name).await?;
        }
        self.start_component(name).await
    }

    /// Stops what runs, the last started first.
    pub async fn shutdown(&mut self) {
        self.status_registry.set_overall_phase(ComponentPhase::Stopping);
        let reverse: Vec<String> = self.startup_order.iter().rev().cloned().collect();
        for name in reverse {
            let _ = self.stop_component(&name).await;
        }
        self.status_registry.set_overall_phase(ComponentPhase::Stopped);
    }

    async fn record_started(
        &mut self,
        name: &str,
        ready_timeout: Duration,
        stop_tx: watch::Sender<bool>,
        handle: JoinHandle<Result<(), String>>,
    ) -> Result<(), String> {
        let mut rx = self.status_registry.subscribe();
        let deadline = tokio::time::Instant::now() + ready_timeout;
        loop {
            let phase = rx.borrow_and_update().get(name).map(|c| c.phase);
            if matches!(phase, Some(ComponentPhase::Ready)) {
                break;
            }
            if matches!(phase, Some(ComponentPhase::Failed)) {
                let detail = rx.borrow().get(name).map(|c| c.detail.clone()).unwrap_or_default();
                return Err(format!("component {name} failed to start: {detail}"));
            }
            if handle.is_finished() {
                return match handle.await {
                    Ok(Err(e)) => Err(format!("component {name} exited during start: {e}")),
                    Ok(Ok(())) => Err(format!("component {name} exited unexpectedly during start")),
                    Err(e) => Err(format!("component {name} panicked: {e}")),
                };
            }
            if tokio::time::Instant::now() >= deadline {
                handle.abort();
                let error = format!("component {name} was not ready after {ready_timeout:?}");
                self.status_registry
                    .register_component(name, None)
                    .failed(error.clone());
                return Err(error);
            }
            let _ = tokio::time::timeout(Duration::from_millis(50), rx.changed()).await;
        }
        if !self.startup_order.iter().any(|n| n == name) {
            self.startup_order.push(name.to_string());
        }
        self.running.insert(name.to_string(), Running { stop_tx, handle });
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    /// A module that is ready at once and records when it stops.
    fn ready_module(name: &str, log: Arc<Mutex<Vec<String>>>) -> Module {
        let stop_name = name.to_string();
        Module::new(
            name,
            Arc::new(move |mut ctx: ServiceContext| {
                let log = log.clone();
                let stop_name = stop_name.clone();
                Box::pin(async move {
                    ctx.status().report_ready(None, "up");
                    ctx.wait_stop().await;
                    log.lock().unwrap().push(format!("stop {stop_name}"));
                    ctx.status().report_stopped("stopped");
                    Ok(())
                })
            }),
        )
    }

    fn failing_module(name: &str) -> Module {
        Module::new(
            name,
            Arc::new(|ctx: ServiceContext| {
                Box::pin(async move {
                    ctx.status().failed("no such port");
                    Err("no such port".into())
                })
            }),
        )
    }

    fn silent_module(name: &str) -> Module {
        Module::new(
            name,
            Arc::new(|mut ctx: ServiceContext| {
                Box::pin(async move {
                    ctx.wait_stop().await;
                    Ok(())
                })
            }),
        )
        .ready_within(Duration::from_millis(200))
    }

    #[tokio::test]
    async fn starts_in_order_and_stops_in_reverse() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut controller = ServerController::new("private", vec!["reforged".into()]);
        controller
            .add(ready_module("backend", log.clone()))
            .add(ready_module("content", log.clone()))
            .add(ready_module("gateway", log.clone()));
        controller.start_all().await.unwrap();
        let snap = controller.snapshot();
        assert_eq!(snap.overall_phase, ComponentPhase::Ready);
        for name in ["backend", "content", "gateway"] {
            assert_eq!(snap.get(name).map(|c| c.phase), Some(ComponentPhase::Ready), "{name}");
        }
        controller.restart_component("content").await.unwrap();
        assert_eq!(
            controller.snapshot().get("content").map(|c| c.phase),
            Some(ComponentPhase::Ready)
        );
        controller.shutdown().await;
        // The restarted module is the last started, so it is the first stopped
        assert_eq!(
            *log.lock().unwrap(),
            ["stop content", "stop content", "stop gateway", "stop backend"]
        );
        assert_eq!(controller.snapshot().overall_phase, ComponentPhase::Stopped);
    }

    #[tokio::test]
    async fn a_required_failure_stops_the_start_and_an_optional_one_does_not() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut controller = ServerController::new("private", Vec::new());
        controller
            .add(ready_module("first", log.clone()))
            .add(failing_module("admin").optional())
            .add(ready_module("last", log.clone()));
        controller.start_all().await.unwrap();
        assert_eq!(
            controller.snapshot().get("admin").map(|c| c.phase),
            Some(ComponentPhase::Failed)
        );
        assert_eq!(
            controller.snapshot().get("last").map(|c| c.phase),
            Some(ComponentPhase::Ready)
        );
        controller.shutdown().await;

        let mut controller = ServerController::new("private", Vec::new());
        controller
            .add(ready_module("first", log.clone()))
            .add(failing_module("gateway"))
            .add(ready_module("never", log.clone()));
        let error = controller.start_all().await.unwrap_err();
        assert!(error.contains("gateway failed to start"), "{error}");
        assert_eq!(controller.snapshot().overall_phase, ComponentPhase::Failed);
        assert_eq!(
            controller.snapshot().get("never").map(|c| c.phase),
            Some(ComponentPhase::Stopped),
            "nothing after a required failure starts"
        );
        controller.shutdown().await;
    }

    #[tokio::test]
    async fn a_module_that_never_reports_ready_is_a_failed_start() {
        let mut controller = ServerController::new("private", Vec::new());
        controller.add(silent_module("slow"));
        let error = controller.start_all().await.unwrap_err();
        assert!(error.contains("not ready after"), "{error}");
        assert!(controller.start_component("slow").await.is_err());
        assert!(controller.stop_component("slow").await.is_err(), "it never ran");
        assert!(controller.start_component("nobody").await.is_err());
        assert_eq!(
            controller.snapshot().get("slow").map(|c| c.phase),
            Some(ComponentPhase::Failed),
            "the views show it failed, not starting"
        );
    }

    /// Binds `addr` as a service does and reports what happened.
    fn binding_module(name: &str, addr: std::net::SocketAddr) -> Module {
        Module::new(
            name,
            Arc::new(move |mut ctx: ServiceContext| {
                Box::pin(async move {
                    match tokio::net::TcpListener::bind(addr).await {
                        Ok(listener) => {
                            let port = listener.local_addr().ok().map(|a| a.port());
                            ctx.status().report_ready(port, "listening");
                            ctx.wait_stop().await;
                            ctx.status().report_stopped("stopped");
                            Ok(())
                        }
                        Err(e) => {
                            ctx.status().failed(format!("cannot listen on {addr}: {e}"));
                            Err(e.to_string())
                        }
                    }
                })
            }),
        )
    }

    #[tokio::test]
    async fn a_port_already_taken_fails_the_start_with_the_reason() {
        let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = taken.local_addr().unwrap();
        let mut controller = ServerController::new("private", Vec::new());
        controller.add(binding_module("gateway", addr));
        let error = controller.start_all().await.unwrap_err();
        assert!(error.contains("cannot listen on"), "{error}");
        let snap = controller.snapshot();
        assert_eq!(snap.get("gateway").map(|c| c.phase), Some(ComponentPhase::Failed));
        assert!(snap.get("gateway").unwrap().detail.contains(&addr.to_string()));
        // Once the port is free, a start from the console takes it
        drop(taken);
        controller.start_component("gateway").await.unwrap();
        assert_eq!(controller.snapshot().get("gateway").unwrap().port, Some(addr.port()));
        controller.shutdown().await;
    }

    #[tokio::test]
    async fn a_module_that_ignores_stop_is_aborted_after_its_grace() {
        struct Dropped(Arc<Mutex<bool>>);
        impl Drop for Dropped {
            fn drop(&mut self) {
                *self.0.lock().unwrap() = true;
            }
        }
        let dropped = Arc::new(Mutex::new(false));
        let flag = dropped.clone();
        let stubborn = Module::new(
            "stubborn",
            Arc::new(move |ctx: ServiceContext| {
                let guard = Dropped(flag.clone());
                Box::pin(async move {
                    let _guard = guard;
                    ctx.status().report_ready(None, "up");
                    std::future::pending::<()>().await;
                    Ok(())
                })
            }),
        )
        .stop_within(Duration::from_millis(100));
        let mut controller = ServerController::new("private", Vec::new());
        controller.add(stubborn);
        controller.start_all().await.unwrap();
        let begun = tokio::time::Instant::now();
        controller.stop_component("stubborn").await.unwrap();
        assert!(begun.elapsed() < Duration::from_secs(5));
        assert!(*dropped.lock().unwrap(), "its task is gone, with whatever it held");
        let snap = controller.snapshot();
        let status = snap.get("stubborn").unwrap();
        assert_eq!(status.phase, ComponentPhase::Stopped);
        assert!(status.detail.contains("aborted"), "{}", status.detail);
        // It can be started again
        controller.start_component("stubborn").await.unwrap();
        controller.shutdown().await;
    }

    #[tokio::test]
    async fn a_restart_leaves_a_stopped_neighbour_stopped() {
        let log = Arc::new(Mutex::new(Vec::new()));
        let mut controller = ServerController::new("private", Vec::new());
        controller
            .add(ready_module("backend", log.clone()))
            .add(ready_module("content", log.clone()))
            .add(ready_module("gateway", log.clone()));
        controller.start_all().await.unwrap();
        controller.stop_component("content").await.unwrap();
        assert!(controller.stop_component("content").await.is_err(), "not running");
        controller.restart_component("gateway").await.unwrap();
        let phase = |c: &ServerController, name: &str| c.snapshot().get(name).map(|s| s.phase);
        assert_eq!(phase(&controller, "content"), Some(ComponentPhase::Stopped));
        assert_eq!(phase(&controller, "gateway"), Some(ComponentPhase::Ready));
        // A restart of a stopped module starts it
        controller.restart_component("content").await.unwrap();
        assert_eq!(phase(&controller, "content"), Some(ComponentPhase::Ready));
        // The last started stops first: content, then the restarted gateway
        controller.shutdown().await;
        assert_eq!(
            *log.lock().unwrap(),
            [
                "stop content",
                "stop gateway",
                "stop content",
                "stop gateway",
                "stop backend"
            ]
        );
    }
}
