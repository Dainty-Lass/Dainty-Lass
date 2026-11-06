//! The module side of the frame protocol (DD5): wraps a [`GameModule`] as an executable. A module crate's
//! binary is one line, `fn main() -> ExitCode { module_api::child::main(MyModule::new()) }`.
//!
//! Stdout is the pipe. Anything else written there breaks the protocol and the host kills the child, so a
//! module logs through `ModuleContext::log` or to stderr, which the host passes into its logs.

use std::process::ExitCode;
use std::sync::Arc;

use tokio::io::{AsyncRead, AsyncWrite, BufReader};
use tokio::sync::Semaphore;

use crate::driver::dispatch;
use crate::frame::Hello;
use crate::frame::{FrameError, MAX_FRAME, MAX_HELLO_FRAME, PROTOCOL_VERSION, ToHost, ToModule, read_frame};
use crate::ids::ModuleId;
use crate::module::{BoxFuture, ErrorKind, GameModule, HostAnswer, HostCall, HostLink, ModuleContext, ModuleError};
use crate::peer::{self, MAX_IN_FLIGHT, Pending, Resolved, Writer};

/// Why serving the host stopped with an error.
#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    /// The host refused this module's hello; holds the host's reason.
    #[error("the host refused this module: {0}")]
    Refused(String),
    /// The host sent something the protocol does not allow at that point.
    #[error("the host broke the frame protocol: {0}")]
    Protocol(String),
    /// Reading or writing a frame failed.
    #[error("{0}")]
    Frame(#[from] FrameError),
}

/// Runs `module` on stdin and stdout until the host closes stdin.
pub fn main<M: GameModule>(module: M) -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("module: could not start the async runtime: {error}");
            return ExitCode::FAILURE;
        }
    };
    let result = runtime.block_on(serve(Arc::new(module), tokio::io::stdin(), tokio::io::stdout()));
    // The module's own tasks get a moment, not the host's whole grace period
    runtime.shutdown_timeout(std::time::Duration::from_secs(2));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("module: {error}");
            ExitCode::FAILURE
        }
    }
}

/// The module's calls to the host, over the pipe.
struct ChildLink {
    writer: Writer,
    pending: Arc<Pending<HostAnswer>>,
}

impl HostLink for ChildLink {
    fn call<'a>(&'a self, _module: &'a ModuleId, call: HostCall) -> BoxFuture<'a, Result<HostAnswer, ModuleError>> {
        Box::pin(async move {
            let (id, rx) = self.pending.open()?;
            if let Err(error) = peer::send(&self.writer, &ToHost::Call { id, call }, MAX_FRAME).await {
                self.pending.forget(id);
                return Err(error);
            }
            rx.await
                .unwrap_or_else(|_| Err(ModuleError::new(ErrorKind::Unavailable, "the host's pipe closed")))
        })
    }
}

/// Runs `module` over any streams: `reader` is what the host writes, `writer` what it reads.
pub async fn serve<M, R, W>(module: Arc<M>, reader: R, writer: W) -> Result<(), ServeError>
where
    M: GameModule,
    R: AsyncRead + Send + Unpin,
    W: AsyncWrite + Send + 'static,
{
    let mut reader = BufReader::new(reader);
    let writer = peer::writer(writer);
    let manifest = module.manifest();
    peer::send(
        &writer,
        &ToHost::Hello(Box::new(Hello::new(manifest.clone()))),
        MAX_HELLO_FRAME,
    )
    .await
    .map_err(|error| ServeError::Protocol(error.message))?;
    let welcome = match read_frame::<ToModule, _>(&mut reader, MAX_HELLO_FRAME).await? {
        Some(ToModule::Welcome(welcome)) => welcome,
        Some(ToModule::Refused { reason }) => return Err(ServeError::Refused(reason)),
        Some(_) => return Err(ServeError::Protocol(String::from("the first frame was not a welcome"))),
        None => {
            return Err(ServeError::Protocol(String::from(
                "the host closed the pipe before its welcome",
            )));
        }
    };
    if welcome.protocol != PROTOCOL_VERSION {
        return Err(ServeError::Protocol(format!(
            "the host speaks frame protocol {}",
            welcome.protocol
        )));
    }
    let pending = Arc::new(Pending::new());
    let link = Arc::new(ChildLink {
        writer: writer.clone(),
        pending: pending.clone(),
    });
    let ctx = ModuleContext::new(manifest.id, welcome.node, link);
    let slots = Arc::new(Semaphore::new(MAX_IN_FLIGHT));
    let result = loop {
        match read_frame::<ToModule, _>(&mut reader, MAX_FRAME).await {
            Ok(Some(ToModule::Call { id, call })) => {
                let Ok(slot) = slots.clone().acquire_owned().await else {
                    break Ok(());
                };
                let (module, ctx, writer) = (module.clone(), ctx.clone(), writer.clone());
                tokio::spawn(async move {
                    let result = dispatch(module.as_ref(), ctx, call).await;
                    _ = peer::send(&writer, &ToHost::Reply { id, result }, MAX_FRAME).await;
                    drop(slot);
                });
            }
            Ok(Some(ToModule::Reply { id, result })) => match pending.resolve(id, result) {
                Resolved::Delivered | Resolved::Late => {}
                Resolved::Unknown => break Err(ServeError::Protocol(String::from("a reply to a call never made"))),
            },
            Ok(Some(_)) => break Err(ServeError::Protocol(String::from("a second handshake"))),
            Ok(None) => break Ok(()),
            Err(error) => break Err(error.into()),
        }
    };
    pending.close("the host closed the pipe");
    result
}
