//! Services as libraries the core loads at start (RD27): the gateway, content, chat and the allowlist leave the core's
//! binary and become libraries on the same C ABI a game module's library uses ([`module_api::plugin`]), with another
//! protocol on it. A service is a [`Service`]; [`export_service!`] makes it a library. The core runs one as a
//! supervised module of its controller and, when it takes modules' registrations, as a core plugin of its registry
//! (`controller::service_library`).
//!
//! The protocol is length-prefixed JSON frames, as DD5's, with the same limits; every message is one of
//! [`ToService`] or [`FromService`]. What a service needs from the core beyond them (the gateway's route table, chat's
//! drivers) is added here as a typed message, never as a pointer.

pub mod chat;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use module_api::frame::{MAX_FRAME, read_frame, write_frame};
use module_api::manifest::ModuleManifest;
use module_api::plugin::{ChannelReader, LibraryWriter};
use module_api::registrar::Registration;
use module_api::{BoxFuture, CoreCapability, ModuleError, ModuleId};
use serde::{Deserialize, Serialize};
use service_kit::context::{ComponentStatus, ServiceContext, StatusRegistry};
use service_kit::env::Env;
use tokio::io::BufReader;
use tokio::sync::{mpsc, watch};

pub mod gateway;

/// Raised when a message changes shape; the core refuses a library speaking any other.
pub const PROTOCOL_VERSION: u32 = 1;

/// What the core sends a service.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ToService {
    /// Runs the service once, on its env file as the host kit wrote it (`Config::from_env` reads it)
    Start {
        protocol: u32,
        env: Vec<(String, String)>,
    },
    /// Ends the run [`ToService::Start`] began, as the controller stops a module
    Stop {},
    /// A module's registration, as `CorePlugin::attach` takes it, with the folder its files are relative to
    Attach {
        id: u64,
        module: Box<ModuleManifest>,
        registration: Box<Registration>,
        dir: Option<PathBuf>,
    },
    Detach {
        module: ModuleId,
    },
    /// The gateway library's own (`gateway`)
    Gateway {
        message: gateway::ToGateway,
    },
    /// Chat's host calls answered, and its drivers' comings and goings ([`chat`])
    Chat(chat::ToChat),
}

/// What a service sends the core.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum FromService {
    /// First, before anything else: who it is and what it serves as a core plugin
    Hello {
        protocol: u32,
        name: String,
        serves: Vec<CoreCapability>,
    },
    /// Its component's status, each time it changes
    Status {
        status: Box<ComponentStatus>,
    },
    /// A log line of its own (a library's tracing has no subscriber of the core's)
    Log {
        level: LogLevel,
        target: String,
        message: String,
    },
    Attached {
        id: u64,
        result: Result<(), ModuleError>,
    },
    /// The run ended, on its own or after [`ToService::Stop {}`]
    Ended {
        result: Result<(), String>,
    },
    Gateway {
        message: gateway::FromGateway,
    },
    /// Chat's host calls ([`chat`])
    Chat(chat::FromChat),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

/// A service built as a library.
pub trait Service: Send + Sync + 'static {
    fn name(&self) -> &'static str;

    /// What it serves as a core plugin; nothing for a service that takes no module's registration.
    fn serves(&self) -> &'static [CoreCapability] {
        &[]
    }

    /// `CorePlugin::attach`, with the folder the module's files are relative to.
    fn attach(
        &self,
        _module: &ModuleManifest,
        _registration: &Registration,
        _dir: Option<&Path>,
    ) -> Result<(), ModuleError> {
        Ok(())
    }

    fn detach(&self, _module: &ModuleId) {}

    /// The gateway's messages from the core; `out` reaches the core for as long as the instance is open.
    fn gateway(&self, _message: gateway::ToGateway, _out: &gateway::Outbox) {}
    /// The instance's [`chat::ChatLink`], once, before [`ToService::Start`]; only chat keeps it.
    fn chat_link(&self, _link: chat::ChatLink) {}

    /// One run, until `ctx` says stop: what the compiled-in module's start function did.
    fn run(&self, env: Env, ctx: ServiceContext) -> BoxFuture<'static, Result<(), String>>;
}

/// Where the library's log lines go: the open instance's frames.
fn log_sink() -> &'static Mutex<Option<mpsc::UnboundedSender<FromService>>> {
    static SINK: OnceLock<Mutex<Option<mpsc::UnboundedSender<FromService>>>> = OnceLock::new();
    SINK.get_or_init(Mutex::default)
}

/// Sends the library's tracing events to the core as [`FromService::Log`].
struct Forward;

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Forward {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        let level = match *event.metadata().level() {
            tracing::Level::ERROR => LogLevel::Error,
            tracing::Level::WARN => LogLevel::Warn,
            tracing::Level::INFO => LogLevel::Info,
            tracing::Level::DEBUG => LogLevel::Debug,
            tracing::Level::TRACE => LogLevel::Trace,
        };
        let mut text = Text::default();
        event.record(&mut text);
        if let Some(tx) = log_sink().lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
            _ = tx.send(FromService::Log {
                level,
                target: event.metadata().target().to_owned(),
                message: text.0,
            });
        }
    }
}

/// An event's message, then its other fields as `name=value`.
#[derive(Default)]
struct Text(String);

impl tracing::field::Visit for Text {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0.insert_str(0, &format!("{value:?}"));
        } else {
            self.0.push_str(&format!(" {}={value:?}", field.name()));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.0.insert_str(0, value);
        } else {
            self.0.push_str(&format!(" {}={value}", field.name()));
        }
    }
}

fn install_logging() {
    static INSTALLED: OnceLock<()> = OnceLock::new();
    INSTALLED.get_or_init(|| {
        use tracing_subscriber::layer::SubscriberExt as _;
        _ = tracing::subscriber::set_global_default(tracing_subscriber::registry().with(Forward));
    });
}

/// The library side of one open instance: answers the core's messages until it closes the stream. Its tracing goes
/// to the core as log lines.
pub async fn serve<S: Service>(service: Arc<S>, reader: ChannelReader, writer: LibraryWriter) {
    serve_with(service, reader, writer, true).await;
}

/// The same service compiled into the core: the same protocol, over in-memory streams, on a runtime of its own as a
/// library's instance has (`module_api::plugin::open_with`), so it behaves exactly as its library does. Its tracing is
/// the core's own.
pub struct InProcess {
    pub reader: ChannelReader,
    pub writer: LibraryWriter,
    closer: InProcessCloser,
}

impl InProcess {
    /// Splits it into the core's streams and what closes it.
    pub fn split(self) -> (ChannelReader, LibraryWriter, InProcessCloser) {
        (self.reader, self.writer, self.closer)
    }
}

/// Ends the stream to the service, as closing its library does: it gets a moment to finish, then its runtime goes.
#[derive(Clone)]
pub struct InProcessCloser(Arc<Mutex<Option<Running>>>);

struct Running {
    to_service: Arc<Mutex<Option<mpsc::UnboundedSender<Vec<u8>>>>>,
    runtime: tokio::runtime::Runtime,
    serving: tokio::task::JoinHandle<()>,
}

/// How long a closed service has to finish, and its runtime to stop: `module_api::plugin::close`'s.
const CLOSE_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

impl InProcessCloser {
    pub async fn close(&self) {
        let taken = self.0.lock().unwrap_or_else(PoisonError::into_inner).take();
        let Some(Running {
            to_service,
            runtime,
            serving,
        }) = taken
        else {
            return;
        };
        // Off the async threads: a runtime can't be waited on or dropped from inside another's
        _ = tokio::task::spawn_blocking(move || {
            drop(to_service.lock().unwrap_or_else(PoisonError::into_inner).take());
            _ = runtime.block_on(async { tokio::time::timeout(CLOSE_GRACE, serving).await });
            runtime.shutdown_timeout(CLOSE_GRACE);
        })
        .await;
    }
}

/// Starts `service` in this process, on a runtime of its own, behind the protocol its library speaks.
pub fn in_process<S: Service>(service: Arc<S>) -> Result<InProcess, String> {
    // A library's instance has its own: the core's blocking calls (an attach) never wait on the service's threads
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("the compiled-in service's runtime: {e}"))?;
    let (to_core, from_service) = mpsc::unbounded_channel::<Vec<u8>>();
    let (to_service, from_core) = mpsc::unbounded_channel::<Vec<u8>>();
    let service_writer: LibraryWriter = module_api::plugin::CallWriter::new(Box::new(move |bytes: &[u8]| {
        _ = to_core.send(bytes.to_vec());
    }));
    let serving = runtime.spawn(serve_with(
        service,
        ChannelReader::new(from_core),
        service_writer,
        false,
    ));
    let to_service = Arc::new(Mutex::new(Some(to_service)));
    let writer: LibraryWriter = module_api::plugin::CallWriter::new(Box::new({
        let to_service = to_service.clone();
        move |bytes: &[u8]| {
            if let Some(tx) = to_service.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
                _ = tx.send(bytes.to_vec());
            }
        }
    }));
    Ok(InProcess {
        reader: ChannelReader::new(from_service),
        writer,
        closer: InProcessCloser(Arc::new(Mutex::new(Some(Running {
            to_service,
            runtime,
            serving,
        })))),
    })
}

async fn serve_with<S: Service>(service: Arc<S>, reader: ChannelReader, mut writer: LibraryWriter, forward: bool) {
    let (out, mut outgoing) = mpsc::unbounded_channel::<FromService>();
    if forward {
        install_logging();
        *log_sink().lock().unwrap_or_else(PoisonError::into_inner) = Some(out.clone());
    }
    let writing = tokio::spawn(async move {
        while let Some(message) = outgoing.recv().await {
            if write_frame(&mut writer, &message, MAX_FRAME).await.is_err() {
                break;
            }
        }
    });
    _ = out.send(FromService::Hello {
        protocol: PROTOCOL_VERSION,
        name: service.name().to_owned(),
        serves: service.serves().to_vec(),
    });
    let chat = chat::ChatLink::new(out.clone());
    service.chat_link(chat.clone());
    let mut reader = BufReader::new(reader);
    let mut run: Option<(watch::Sender<bool>, tokio::task::JoinHandle<()>)> = None;
    let mut dirs: BTreeMap<ModuleId, PathBuf> = BTreeMap::new();
    while let Ok(Some(message)) = read_frame::<ToService, _>(&mut reader, MAX_FRAME).await {
        match message {
            ToService::Start { protocol, env } => {
                if protocol != PROTOCOL_VERSION || run.as_ref().is_some_and(|(_, task)| !task.is_finished()) {
                    _ = out.send(FromService::Ended {
                        result: Err(format!("a start it can't take (protocol {protocol})")),
                    });
                    continue;
                }
                run = Some(start(&service, env, &out));
            }
            ToService::Stop {} => {
                if let Some((stop, _)) = &run {
                    _ = stop.send(true);
                }
            }
            ToService::Attach {
                id,
                module,
                registration,
                dir,
            } => {
                if let Some(dir) = &dir {
                    dirs.insert(module.id.clone(), dir.clone());
                }
                let result = service.attach(&module, &registration, dirs.get(&module.id).map(PathBuf::as_path));
                _ = out.send(FromService::Attached { id, result });
            }
            ToService::Detach { module } => {
                service.detach(&module);
                dirs.remove(&module);
            }
            ToService::Gateway { message } => service.gateway(message, &gateway::Outbox(out.clone())),
            ToService::Chat(message) => chat.deliver(message),
        }
    }
    // The core closed the stream: the run stops as it would on Stop
    if let Some((stop, task)) = run {
        _ = stop.send(true);
        _ = tokio::time::timeout(std::time::Duration::from_secs(5), task).await;
    }
    chat.close();
    if forward {
        // Only its own: a newer instance of the same library may have taken the sink since
        let mut sink = log_sink().lock().unwrap_or_else(PoisonError::into_inner);
        if sink.as_ref().is_some_and(|tx| tx.same_channel(&out)) {
            sink.take();
        }
    }
    drop(out);
    _ = writing.await;
}

/// One run: its context's status goes to the core as it changes, and its end as [`FromService::Ended`].
fn start<S: Service>(
    service: &Arc<S>,
    env: Vec<(String, String)>,
    out: &mpsc::UnboundedSender<FromService>,
) -> (watch::Sender<bool>, tokio::task::JoinHandle<()>) {
    let name = service.name();
    let registry = StatusRegistry::default();
    let (ctx, stop) = registry.service_context(name);
    let mut changes = registry.subscribe();
    let status_out = out.clone();
    tokio::spawn(async move {
        let mut last: Option<ComponentStatus> = None;
        loop {
            let now = changes.borrow_and_update().get(name).cloned();
            if let Some(now) = now
                && last.as_ref() != Some(&now)
            {
                last = Some(now.clone());
                if status_out.send(FromService::Status { status: Box::new(now) }).is_err() {
                    break;
                }
            }
            if changes.changed().await.is_err() {
                break;
            }
        }
    });
    let env = Env::from_pairs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    let running = service.run(env, ctx);
    let ended = out.clone();
    let task = tokio::spawn(async move {
        let result = running.await;
        _ = ended.send(FromService::Ended { result });
    });
    (stop, task)
}

/// Exports the service `$make` builds as a library. In a `cdylib` crate:
/// `service_plugin::export_service!(MyService::new());`
#[macro_export]
macro_rules! export_service {
    ($make:expr) => {
        #[allow(unsafe_code)]
        #[unsafe(no_mangle)]
        pub extern "C" fn dainty_service_v1() -> *const ::module_api::plugin::PluginV1 {
            unsafe extern "C" fn open(sink: ::module_api::plugin::HostSink) -> *mut ::std::ffi::c_void {
                ::module_api::plugin::open_with(
                    |reader, writer| {
                        let service = ::std::sync::Arc::new($make);
                        $crate::serve(service, reader, writer)
                    },
                    sink,
                )
            }
            static PLUGIN: ::module_api::plugin::PluginV1 = ::module_api::plugin::PluginV1 {
                abi_version: ::module_api::plugin::ABI_VERSION,
                open,
                send: ::module_api::plugin::send,
                close: ::module_api::plugin::close,
            };
            &PLUGIN
        }
    };
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn messages_are_tagged_json_and_refuse_unknown_fields() {
        let stop = serde_json::to_string(&ToService::Stop {}).unwrap();
        assert_eq!(stop, r#"{"type":"stop"}"#);
        let start: ToService = serde_json::from_str(r#"{"type":"start","protocol":1,"env":[["A","b"]]}"#).unwrap();
        assert_eq!(
            start,
            ToService::Start {
                protocol: 1,
                env: vec![("A".into(), "b".into())]
            }
        );
        assert!(serde_json::from_str::<ToService>(r#"{"type":"stop","extra":1}"#).is_err());
        assert!(serde_json::from_str::<ToService>(r#"{"type":"detach","module":"stub","extra":1}"#).is_err());
        assert!(serde_json::from_str::<FromService>(r#"{"type":"shell","cmd":"x"}"#).is_err());
        let ended = FromService::Ended {
            result: Err("no".into()),
        };
        let back: FromService = serde_json::from_str(&serde_json::to_string(&ended).unwrap()).unwrap();
        assert_eq!(back, ended);
    }
}
