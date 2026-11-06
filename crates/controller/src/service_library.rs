// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! A service built as a library (RD27, crate `service-plugin`), run as a module of this controller and, when it takes
//! modules' registrations, as a core plugin of the registry. Each start of the module opens the library anew; the
//! plugin outlives the runs and replays the registrations it holds into each one. The library is pinned by its
//! SHA-256 and loaded by `plugin-host` (docs/security/plugin-host.md).

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use module_api::frame::{MAX_FRAME, encode, read_frame};
use module_api::manifest::ModuleManifest;
use module_api::plugin::LibraryWriter as CoreWriter;
use module_api::plugin::{ChannelReader, SERVICE_ENTRY};
use module_api::registrar::Registration;
use module_api::{BoxFuture, CoreCapability, CorePlugin, ErrorKind, ModuleError, ModuleId, Sha256Hex};
use plugin_host::LibraryStream;
use service_kit::context::{ComponentPhase, ComponentStatus, ServiceContext, StatusReporter};
use service_kit::env::Env;
use service_plugin::{FromService, LogLevel, PROTOCOL_VERSION, Service, ToService};
use tokio::io::BufReader;
use tokio::sync::oneshot;

use crate::{Module, ModuleRegistry, WeakRegistry};

mod chat;
pub mod gateway;
pub use chat::{chat_compiled_in, chat_library};

/// How long the library has to say hello, to answer an attach, and to end after Stop.
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
const ATTACH_TIMEOUT: Duration = Duration::from_secs(5);
const STOP_TIMEOUT: Duration = Duration::from_secs(10);

/// A service library as `host.toml` pins it.
#[derive(Debug, Clone)]
pub struct ServiceLibrary {
    /// The service's name, also its component name.
    pub name: &'static str,
    /// The library file.
    pub path: PathBuf,
    /// The SHA-256 `host.toml` pins it to; a file that doesn't match is refused.
    pub sha256: Sha256Hex,
    /// What it serves as a core plugin, checked against its hello; empty for a service that takes no registration
    pub serves: &'static [CoreCapability],
}

/// The module that runs the library on `env`, and registers it with `registry` as a core plugin when it serves one.
pub fn service_library(library: ServiceLibrary, env: Env, registry: Option<&ModuleRegistry>) -> Module {
    service_library_with(library, env, registry, None)
}

/// [`service_library`], with `extension` for the service's own messages.
pub fn service_library_with(
    library: ServiceLibrary,
    env: Env,
    registry: Option<&ModuleRegistry>,
    extension: Option<Arc<dyn Extension>>,
) -> Module {
    let ServiceLibrary {
        name,
        path,
        sha256,
        serves,
    } = library;
    service_module(name, serves, Source::Library { path, sha256 }, env, registry, extension)
}

/// The same service compiled into this binary: the [`Service`] its library exports, run over the same protocol on
/// in-memory streams, so a service behaves the same loaded from a library or built in (RD27).
pub fn service_compiled_in<S: Service>(
    make: impl Fn() -> S + Send + Sync + 'static,
    env: Env,
    registry: Option<&ModuleRegistry>,
) -> Module {
    service_compiled_in_with(make, env, registry, None)
}

/// [`service_compiled_in`], with `extension` for the service's own messages.
pub fn service_compiled_in_with<S: Service>(
    make: impl Fn() -> S + Send + Sync + 'static,
    env: Env,
    registry: Option<&ModuleRegistry>,
    extension: Option<Arc<dyn Extension>>,
) -> Module {
    let probe = make();
    let (name, serves) = (probe.name(), probe.serves());
    let open = Arc::new(move || service_plugin::in_process(Arc::new(make())));
    service_module(name, serves, Source::CompiledIn(open), env, registry, extension)
}

/// What a service's own messages need of the core (`gateway`, `chat`): told of each run, it says what goes before
/// its Start and before each Attach, and takes the service's messages the base protocol doesn't.
pub trait Extension: Send + Sync {
    /// A run opened: what to send before Start.
    fn opened(&self, run: RunLink) -> Vec<ToService>;
    /// After [`Extension::opened`], before Start: what has to wait on the core first. An error ends the run.
    fn prepare<'a>(&'a self, _run: &'a RunLink) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async { Ok(()) })
    }
    /// Called once the run has ended.
    fn closed(&self);
    /// What to send before attaching `_module`.
    fn before_attach(&self, _module: &ModuleManifest) -> Vec<ToService> {
        Vec::new()
    }
    /// Called on the run's reader thread: hand anything slow to a runtime of the core's. An error breaks the
    /// protocol and ends the run.
    fn message(&self, message: FromService) -> Result<(), String>;
}

/// A way to the open instance, for an [`Extension`].
#[derive(Clone)]
pub struct RunLink(Arc<Run>);

impl RunLink {
    /// Sends `message` to the open instance; one that cannot be encoded is logged and dropped.
    pub fn send(&self, message: &ToService) {
        self.0.send(message);
    }
}

fn service_module(
    name: &'static str,
    serves: &'static [CoreCapability],
    source: Source,
    env: Env,
    registry: Option<&ModuleRegistry>,
    extension: Option<Arc<dyn Extension>>,
) -> Module {
    let shared = Arc::new(Shared {
        name,
        serves,
        source,
        registry: registry.map(ModuleRegistry::downgrade),
        state: Mutex::default(),
        extension,
    });
    if let Some(registry) = registry
        && !serves.is_empty()
    {
        registry.add_plugin(Arc::new(Plugin(shared.clone())));
    }
    Module::new(
        name,
        Arc::new(move |ctx| {
            let (shared, env) = (shared.clone(), env.clone());
            Box::pin(async move { run_once(shared, env, ctx).await })
        }),
    )
}

/// Where a service's code is; everything after opening it is the same for both.
enum Source {
    Library { path: PathBuf, sha256: Sha256Hex },
    CompiledIn(Arc<dyn Fn() -> Result<service_plugin::InProcess, String> + Send + Sync>),
}

impl Source {
    fn describe(&self) -> String {
        match self {
            Self::Library { path, .. } => path.display().to_string(),
            Self::CompiledIn(_) => String::from("the compiled-in service"),
        }
    }
}

/// Ends an open instance, whichever source it came from.
enum Closer {
    Library(plugin_host::Closer),
    CompiledIn(service_plugin::InProcessCloser),
}

impl Closer {
    async fn close(&self) {
        match self {
            Self::Library(closer) => closer.close().await,
            Self::CompiledIn(closer) => closer.close().await,
        }
    }
}

struct Shared {
    name: &'static str,
    serves: &'static [CoreCapability],
    source: Source,
    registry: Option<WeakRegistry>,
    state: Mutex<State>,
    extension: Option<Arc<dyn Extension>>,
}

#[derive(Default)]
struct State {
    /// What the registry attached, replayed into every run
    attached: BTreeMap<ModuleId, (ModuleManifest, Registration)>,
    run: Option<Arc<Run>>,
}

impl Shared {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn module_dir(&self, module: &ModuleId) -> Option<PathBuf> {
        self.registry.as_ref()?.upgrade()?.module_dir(module)
    }
}

/// One open instance of the library.
struct Run {
    writer: CoreWriter,
    pending: Mutex<HashMap<u64, std::sync::mpsc::Sender<Result<(), ModuleError>>>>,
    next: AtomicU64,
    extension: Option<Arc<dyn Extension>>,
}

impl Run {
    fn send(&self, message: &ToService) {
        match encode(message, MAX_FRAME) {
            Ok(bytes) => self.writer.send(&bytes),
            Err(error) => tracing::warn!(target: "host", "service library: a message not sent: {error}"),
        }
    }

    /// Attaches and waits for the answer. Blocking, as `CorePlugin::attach` is; the answer is read on the run's own
    /// thread (`read_loop`), so this never waits on the runtime it is called from.
    fn attach(
        &self,
        module: &ModuleManifest,
        registration: &Registration,
        dir: Option<PathBuf>,
    ) -> Result<(), ModuleError> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = std::sync::mpsc::channel();
        self.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, tx);
        for message in self.extension.iter().flat_map(|e| e.before_attach(module)) {
            self.send(&message);
        }
        self.send(&ToService::Attach {
            id,
            module: Box::new(module.clone()),
            registration: Box::new(registration.clone()),
            dir,
        });
        let answer = rx.recv_timeout(ATTACH_TIMEOUT);
        self.pending.lock().unwrap_or_else(PoisonError::into_inner).remove(&id);
        answer.unwrap_or_else(|_| Err(ModuleError::new(ErrorKind::Unavailable, "the service did not answer")))
    }
}

struct Plugin(Arc<Shared>);

impl CorePlugin for Plugin {
    fn name(&self) -> &'static str {
        self.0.name
    }

    fn serves(&self) -> &'static [CoreCapability] {
        self.0.serves
    }

    fn attach(&self, module: &ModuleManifest, registration: &Registration) -> Result<(), ModuleError> {
        let run = {
            let mut state = self.0.state();
            state
                .attached
                .insert(module.id.clone(), (module.clone(), registration.clone()));
            state.run.clone()
        };
        let Some(run) = run else {
            // Not running: the next run takes it with the rest
            return Ok(());
        };
        let result = run.attach(module, registration, self.0.module_dir(&module.id));
        if result.is_err() {
            self.0.state().attached.remove(&module.id);
        }
        result
    }

    fn detach(&self, module: &ModuleId) {
        let run = {
            let mut state = self.0.state();
            state.attached.remove(module);
            state.run.clone()
        };
        if let Some(run) = run {
            run.send(&ToService::Detach { module: module.clone() });
        }
    }
}

async fn run_once(shared: Arc<Shared>, env: Env, ctx: ServiceContext) -> Result<(), String> {
    let (reader, writer, closer) = match &shared.source {
        Source::Library { path, sha256 } => {
            let LibraryStream { reader, writer, closer } = plugin_host::open_library(path, *sha256, SERVICE_ENTRY)
                .await
                .map_err(|e| e.to_string())?;
            (reader, writer, Closer::Library(closer))
        }
        Source::CompiledIn(open) => {
            let (reader, writer, closer) = open()?.split();
            (reader, writer, Closer::CompiledIn(closer))
        }
    };
    let run = Arc::new(Run {
        writer,
        pending: Mutex::default(),
        next: AtomicU64::new(1),
        extension: shared.extension.clone(),
    });
    let (hello_tx, hello_rx) = oneshot::channel();
    let (ended_tx, mut ended_rx) = oneshot::channel();
    let reading = {
        let (run, reporter, name) = (run.clone(), ctx.status().clone(), shared.name);
        std::thread::spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread().build() else {
                return;
            };
            runtime.block_on(read_loop(reader, run, reporter, name, hello_tx, ended_tx));
        })
    };
    let result = drive(&shared, &run, env, ctx, hello_rx, &mut ended_rx).await;
    shared.state().run = None;
    if let Some(extension) = &shared.extension {
        extension.closed();
    }
    closer.close().await;
    _ = tokio::task::spawn_blocking(move || reading.join()).await;
    result
}

async fn drive(
    shared: &Arc<Shared>,
    run: &Arc<Run>,
    env: Env,
    ctx: ServiceContext,
    hello_rx: oneshot::Receiver<(u32, String, Vec<CoreCapability>)>,
    ended_rx: &mut oneshot::Receiver<Result<(), String>>,
) -> Result<(), String> {
    let (protocol, name, serves) = tokio::time::timeout(HELLO_TIMEOUT, hello_rx)
        .await
        .map_err(|_| {
            format!(
                "{} did not say hello within {HELLO_TIMEOUT:?}",
                shared.source.describe()
            )
        })?
        .map_err(|_| format!("{} ended before its hello", shared.source.describe()))?;
    if protocol != PROTOCOL_VERSION {
        return Err(format!(
            "it speaks service protocol {protocol}, this core {PROTOCOL_VERSION}"
        ));
    }
    if name != shared.name {
        return Err(format!("it is the service {name}, host.toml names {}", shared.name));
    }
    let mut serves = serves;
    serves.sort();
    let mut expected = shared.serves.to_vec();
    expected.sort();
    if serves != expected {
        return Err(format!("it serves {serves:?}, this core expects {expected:?}"));
    }
    if let Some(extension) = &shared.extension {
        let link = RunLink(run.clone());
        for message in extension.opened(link.clone()) {
            run.send(&message);
        }
        extension.prepare(&link).await?;
    }
    run.send(&ToService::Start {
        protocol: PROTOCOL_VERSION,
        env: env.vars().map(|(k, v)| (k.to_owned(), v.to_owned())).collect(),
    });
    let replay: Vec<(ModuleManifest, Registration)> = {
        let mut state = shared.state();
        state.run = Some(run.clone());
        state.attached.values().cloned().collect()
    };
    for (module, registration) in replay {
        let dir = shared.module_dir(&module.id);
        let run = run.clone();
        let id = module.id.clone();
        let result = tokio::task::spawn_blocking(move || run.attach(&module, &registration, dir))
            .await
            .unwrap_or_else(|e| Err(ModuleError::new(ErrorKind::Unavailable, e.to_string())));
        if let Err(error) = result {
            tracing::warn!(target: "host", "{}: module {id} not taken again: {error}", shared.name);
        }
    }
    let mut stop = ctx.stop_receiver();
    tokio::select! {
        ended = &mut *ended_rx => ended.unwrap_or_else(|_| Err(String::from("the library's stream ended"))),
        () = stopped(&mut stop) => {
            run.send(&ToService::Stop {});
            match tokio::time::timeout(STOP_TIMEOUT, ended_rx).await {
                Ok(Ok(result)) => result,
                Ok(Err(_)) => Ok(()),
                Err(_) => Err(format!("it did not stop within {STOP_TIMEOUT:?}")),
            }
        }
    }
}

async fn stopped(stop: &mut tokio::sync::watch::Receiver<bool>) {
    while !*stop.borrow() {
        if stop.changed().await.is_err() {
            return;
        }
    }
}

/// Reads the library's messages on a thread of its own, until its stream ends.
async fn read_loop(
    reader: ChannelReader,
    run: Arc<Run>,
    reporter: StatusReporter,
    name: &'static str,
    hello: oneshot::Sender<(u32, String, Vec<CoreCapability>)>,
    ended: oneshot::Sender<Result<(), String>>,
) {
    let mut reader = BufReader::new(reader);
    let mut hello = Some(hello);
    let mut ended = Some(ended);
    let end = loop {
        let message = match read_frame::<FromService, _>(&mut reader, MAX_FRAME).await {
            Ok(Some(message)) => message,
            Ok(None) => break Err(String::from("the library's stream ended")),
            Err(error) => break Err(format!("the library broke the protocol: {error}")),
        };
        match message {
            FromService::Hello { protocol, name, serves } => match hello.take() {
                Some(tx) => _ = tx.send((protocol, name, serves)),
                None => break Err(String::from("a second hello")),
            },
            _ if hello.is_some() => break Err(String::from("the first message was not a hello")),
            FromService::Status { status } => apply(&reporter, &status),
            FromService::Log { level, target, message } => log(name, level, &target, &message),
            FromService::Attached { id, result } => {
                if let Some(tx) = run.pending.lock().unwrap_or_else(PoisonError::into_inner).remove(&id) {
                    _ = tx.send(result);
                }
            }
            FromService::Ended { result } => {
                if let Some(tx) = ended.take() {
                    _ = tx.send(result);
                }
            }
            message @ (FromService::Gateway { .. } | FromService::Chat(_)) => match &run.extension {
                Some(extension) => {
                    if let Err(error) = extension.message(message) {
                        break Err(error);
                    }
                }
                None => break Err(String::from("a message this service may not send")),
            },
        }
    };
    if let Some(tx) = ended.take() {
        _ = tx.send(end);
    }
    for (_, tx) in run.pending.lock().unwrap_or_else(PoisonError::into_inner).drain() {
        _ = tx.send(Err(ModuleError::new(ErrorKind::Unavailable, "the service ended")));
    }
}

/// The library's component status, onto the core's.
fn apply(reporter: &StatusReporter, status: &ComponentStatus) {
    let detail = (!status.detail.is_empty()).then(|| status.detail.clone());
    match status.phase {
        ComponentPhase::Starting => match detail {
            Some(detail) => reporter.report_starting(detail),
            None => reporter.starting(),
        },
        ComponentPhase::Ready => reporter.ready(status.listen_addr.clone(), detail),
        ComponentPhase::Stopping => reporter.stopping(),
        ComponentPhase::Stopped => reporter.stopped(),
        ComponentPhase::Failed => reporter.failed(status.last_error.clone().unwrap_or_default()),
        ComponentPhase::Disabled => reporter.disabled(detail.unwrap_or_default()),
    }
}

/// A library's log line, into the core's log, under its service's name.
fn log(service: &str, level: LogLevel, target: &str, message: &str) {
    match level {
        LogLevel::Error => tracing::error!(target: "service", service, source = target, "{message}"),
        LogLevel::Warn => tracing::warn!(target: "service", service, source = target, "{message}"),
        LogLevel::Info => tracing::info!(target: "service", service, source = target, "{message}"),
        LogLevel::Debug => tracing::debug!(target: "service", service, source = target, "{message}"),
        LogLevel::Trace => tracing::trace!(target: "service", service, source = target, "{message}"),
    }
}
