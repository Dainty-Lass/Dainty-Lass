//! Modules as libraries the core loads at start (RD27): a library exports one C entry point, [`ENTRY`], returning a
//! [`PluginV1`] vtable whose calls carry the frame protocol (DD5) as bytes. Each side copies what it is handed before
//! the call returns, so no buffer crosses an allocator, and the library runs the module on a runtime of its own, so no
//! Rust type crosses either: core and library need not share a compiler. A library is one line,
//! `dainty_module_api::export_plugin!(MyModule::new());`, in a crate with `crate-type = ["cdylib"]`.
//!
//! The unsafe code of the library side is here, where every module shares it; the core's is `plugin-host`'s.
#![allow(
    unsafe_code,
    reason = "the library side of the plugin ABI: raw pointers across the C boundary"
)]

use std::ffi::c_void;
use std::future::Future;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::mpsc;

use crate::child;
use crate::module::GameModule;

/// Raised when [`PluginV1`] changes shape; the core refuses a library with any other.
pub const ABI_VERSION: u32 = 1;

/// The symbol a game module's library exports, NUL-terminated as the loader wants it.
pub const ENTRY: &[u8] = b"dainty_plugin_v1\0";

/// The symbol a service's library exports (crate `service-plugin`): the same vtable, another protocol on it.
pub const SERVICE_ENTRY: &[u8] = b"dainty_service_v1\0";

/// How long a closing library waits for its module to finish before its runtime is shut down.
const CLOSE_GRACE: Duration = Duration::from_secs(2);

/// The core's side of one open module, handed to [`PluginV1::open`].
#[repr(C)]
#[derive(Clone, Copy)]
pub struct HostSink {
    /// The core's state for this module, passed back to `send` and `closed`.
    pub ctx: *mut c_void,
    /// Bytes from the library: the core copies them before it returns
    pub send: unsafe extern "C" fn(ctx: *mut c_void, ptr: *const u8, len: usize),
    /// The library's side ended. At most once, and never after [`PluginV1::close`] returned
    pub closed: unsafe extern "C" fn(ctx: *mut c_void),
}

/// What [`ENTRY`] returns, valid as long as the library stays loaded.
#[repr(C)]
pub struct PluginV1 {
    /// [`ABI_VERSION`] of the library; the core refuses one it does not speak.
    pub abi_version: u32,
    /// Starts the module, which speaks first (its hello); null when it could not start. `sink` stays valid until
    /// `close` returns.
    pub open: unsafe extern "C" fn(sink: HostSink) -> *mut c_void,
    /// Bytes from the core: the library copies them before it returns
    pub send: unsafe extern "C" fn(handle: *mut c_void, ptr: *const u8, len: usize),
    /// Ends the module and frees `handle`, waiting for it a moment. The core calls it off its async threads,
    /// exactly once per handle.
    pub close: unsafe extern "C" fn(handle: *mut c_void),
}

/// What the other side hands over, read as one stream; it ends when the sender goes.
pub struct ChannelReader {
    rx: mpsc::UnboundedReceiver<Vec<u8>>,
    chunk: Vec<u8>,
    at: usize,
}

impl ChannelReader {
    /// Reads what arrives on `rx`.
    pub fn new(rx: mpsc::UnboundedReceiver<Vec<u8>>) -> Self {
        Self {
            rx,
            chunk: Vec::new(),
            at: 0,
        }
    }
}

impl AsyncRead for ChannelReader {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, out: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let this = &mut *self;
        while this.at >= this.chunk.len() {
            match this.rx.poll_recv(cx) {
                Poll::Ready(Some(chunk)) => {
                    this.chunk = chunk;
                    this.at = 0;
                }
                Poll::Ready(None) => return Poll::Ready(Ok(())),
                Poll::Pending => return Poll::Pending,
            }
        }
        let n = (this.chunk.len() - this.at).min(out.remaining());
        out.put_slice(&this.chunk[this.at..this.at + n]);
        this.at += n;
        Poll::Ready(Ok(()))
    }
}

/// Writes by handing each buffer to `send`, which copies it before returning.
pub struct CallWriter<F>(F);

impl<F: Fn(&[u8]) + Send + Unpin> CallWriter<F> {
    /// Writes through `send`.
    pub fn new(send: F) -> Self {
        Self(send)
    }

    /// Hands `bytes` over at once, from code that isn't async; they are copied before this returns.
    pub fn send(&self, bytes: &[u8]) {
        (self.0)(bytes);
    }
}

impl<F: Fn(&[u8]) + Send + Unpin> AsyncWrite for CallWriter<F> {
    fn poll_write(self: Pin<&mut Self>, _: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        (self.0)(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

/// The sink, moved onto the library's runtime threads.
#[derive(Clone, Copy)]
struct Sink(HostSink);

// SAFETY: the core keeps `ctx` and its callbacks valid, and callable from any thread, until `close` returns
unsafe impl Send for Sink {}
unsafe impl Sync for Sink {}

impl Sink {
    fn send(self, bytes: &[u8]) {
        // SAFETY: valid until `close` returns (above), and the core copies the bytes before returning
        unsafe { (self.0.send)(self.0.ctx, bytes.as_ptr(), bytes.len()) }
    }

    fn closed(self) {
        // SAFETY: as in `send`; called once, by the one task that serves the module
        unsafe { (self.0.closed)(self.0.ctx) }
    }
}

struct Instance {
    to_module: Mutex<Option<mpsc::UnboundedSender<Vec<u8>>>>,
    runtime: tokio::runtime::Runtime,
    serving: tokio::task::JoinHandle<()>,
}

/// What a library writes to the core with.
pub type LibraryWriter = CallWriter<Box<dyn Fn(&[u8]) + Send + Sync>>;

/// [`PluginV1::open`] for the module `make` builds; [`export_plugin!`](crate::export_plugin) calls it.
pub fn open<M: GameModule>(make: impl FnOnce() -> M, sink: HostSink) -> *mut c_void {
    open_with(
        move |reader, writer| {
            let module = Arc::new(make());
            async move {
                if let Err(error) = child::serve(module, reader, writer).await {
                    // The library's own tracing has no subscriber: stderr is the core's
                    eprintln!("module library: {error}");
                }
            }
        },
        sink,
    )
}

/// [`PluginV1::open`] for any protocol: `serve` runs on the library's own runtime over the core's bytes until the
/// core closes them.
pub fn open_with<F, Fut>(serve: F, sink: HostSink) -> *mut c_void
where
    F: FnOnce(ChannelReader, LibraryWriter) -> Fut,
    Fut: Future<Output = ()> + Send + 'static,
{
    let opened = catch_unwind(AssertUnwindSafe(|| {
        let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().ok()?;
        let (to_module, from_host) = mpsc::unbounded_channel();
        let sink = Sink(sink);
        let writer: LibraryWriter = CallWriter::new(Box::new(move |bytes: &[u8]| sink.send(bytes)));
        let served = {
            let _entered = runtime.enter();
            serve(ChannelReader::new(from_host), writer)
        };
        let serving = runtime.spawn(async move {
            served.await;
            sink.closed();
        });
        Some(Box::new(Instance {
            to_module: Mutex::new(Some(to_module)),
            runtime,
            serving,
        }))
    }));
    match opened {
        Ok(Some(instance)) => Box::into_raw(instance).cast(),
        _ => std::ptr::null_mut(),
    }
}

/// [`PluginV1::send`].
///
/// # Safety
/// `handle` is one [`open`] returned and not yet closed; `ptr` points at `len` readable bytes.
pub unsafe extern "C" fn send(handle: *mut c_void, ptr: *const u8, len: usize) {
    if handle.is_null() || (ptr.is_null() && len > 0) {
        return;
    }
    _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: the caller's contract above
        let instance = unsafe { &*handle.cast::<Instance>() };
        let bytes = if len == 0 {
            Vec::new()
        } else {
            // SAFETY: the caller's contract above
            unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec()
        };
        let guard = instance.to_module.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(tx) = guard.as_ref() {
            _ = tx.send(bytes);
        }
    }));
}

/// [`PluginV1::close`].
///
/// # Safety
/// `handle` is one [`open`] returned, closed only this once.
pub unsafe extern "C" fn close(handle: *mut c_void) {
    if handle.is_null() {
        return;
    }
    // SAFETY: the caller's contract above: the Box `open` made, taken back once
    let instance = unsafe { Box::from_raw(handle.cast::<Instance>()) };
    _ = catch_unwind(AssertUnwindSafe(move || {
        // Its own thread: a runtime can't be waited on or dropped from inside another's
        let shutdown = std::thread::spawn(move || {
            let Instance {
                to_module,
                runtime,
                serving,
            } = *instance;
            // The module's stream ends, so it finishes as a child does when its stdin closes
            drop(to_module.lock().unwrap_or_else(PoisonError::into_inner).take());
            _ = runtime.block_on(async { tokio::time::timeout(CLOSE_GRACE, serving).await });
            runtime.shutdown_timeout(CLOSE_GRACE);
        });
        _ = shutdown.join();
    }));
}

/// Exports the [`ENTRY`] symbol for the module `$make` builds. In a `cdylib` crate:
/// `dainty_module_api::export_plugin!(MyModule::new());`
#[macro_export]
macro_rules! export_plugin {
    ($make:expr) => {
        #[allow(unsafe_code, reason = "the exported C entry point; no_mangle is unsafe")]
        /// The core's entry point into this library: the module's plugin table.
        #[unsafe(no_mangle)]
        pub extern "C" fn dainty_plugin_v1() -> *const $crate::plugin::PluginV1 {
            unsafe extern "C" fn open(sink: $crate::plugin::HostSink) -> *mut ::std::ffi::c_void {
                $crate::plugin::open(|| $make, sink)
            }
            static PLUGIN: $crate::plugin::PluginV1 = $crate::plugin::PluginV1 {
                abi_version: $crate::plugin::ABI_VERSION,
                open,
                send: $crate::plugin::send,
                close: $crate::plugin::close,
            };
            &PLUGIN
        }
    };
}
