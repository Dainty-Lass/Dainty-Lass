//! Loads modules built as libraries ([`module_api::plugin`], RD27) and drives them as any other module: the bytes a
//! library writes go through [`ChildModule`]'s frame protocol code, so they are checked as a child process's are
//! (docs/security/plugin-host.md). The only crate in dainty-lass allowed unsafe code; every block of it is here.
#![allow(unsafe_code)]

use std::collections::HashMap;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use module_api::frame::{ModuleAnswer, ModuleCall};
use module_api::host::{hash_file, open_pinned, program};
use module_api::manifest::ModuleManifest;
use module_api::plugin::{ABI_VERSION, CallWriter, ChannelReader, ENTRY, HostSink, PluginV1};
use module_api::{
    BoxFuture, ChildModule, ChildOptions, ExternalModule, HostLink, LoadError, ModuleDriver, ModuleError, NodeIdentity,
    Sha256Hex,
};
use tokio::sync::mpsc;

/// Whether `host.toml`'s module path names a library rather than an executable.
pub fn is_library(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ["dll", "so", "dylib"].contains(&ext.to_ascii_lowercase().as_str()))
}

/// A loaded library and its vtable.
struct Library {
    // Never dropped (`loaded`): kept so the handle is owned by something
    _lib: libloading::Library,
    vtable: &'static PluginV1,
}

type Loaded = Mutex<HashMap<(PathBuf, Sha256Hex), Arc<Library>>>;

/// Every library loaded, by path and digest. None is unloaded until the core exits: a runtime thread of a closed
/// module can outlive its grace, and code that is unmapped under it crashes the process.
fn loaded() -> &'static Loaded {
    static LOADED: OnceLock<Loaded> = OnceLock::new();
    LOADED.get_or_init(Mutex::default)
}

fn read_error(path: &Path) -> impl Fn(std::io::Error) -> LoadError + '_ {
    move |error| LoadError::Read {
        path: path.to_owned(),
        error,
    }
}

fn refused(path: &Path, why: impl Into<String>) -> LoadError {
    LoadError::Spawn {
        path: path.to_owned(),
        error: std::io::Error::other(why.into()),
    }
}

/// Checks the file against its pinned SHA-256 and loads it from the same handle; nothing in a file that doesn't match
/// runs, not even its initialisers.
fn load(path: &Path, expected: Sha256Hex) -> Result<Arc<Library>, LoadError> {
    let mut file = open_pinned(path).map_err(read_error(path))?;
    let found = hash_file(&mut file).map_err(read_error(path))?;
    if found != expected {
        return Err(LoadError::HashMismatch {
            path: path.to_owned(),
            expected,
            found,
        });
    }
    let key = (path.to_owned(), found);
    let mut libraries = loaded().lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(library) = libraries.get(&key) {
        return Ok(library.clone());
    }
    // SAFETY: loading runs the library's initialisers. It is the file just hashed, through the handle still held, so
    // on Windows nobody can have written to it since (module_api::host::open_pinned; docs/security/plugin-host.md)
    let lib = unsafe { libloading::Library::new(program(path, &file)) }.map_err(|e| refused(path, e.to_string()))?;
    drop(file);
    let entry = {
        // SAFETY: ENTRY is the symbol export_plugin! defines, with this type
        let symbol = unsafe { lib.get::<extern "C" fn() -> *const PluginV1>(ENTRY) }
            .map_err(|e| refused(path, format!("not a module library: {e}")))?;
        *symbol
    };
    // A panic can't cross an extern "C" function: it aborts there, in the library, as the ABI says
    let vtable: *const PluginV1 = entry();
    // SAFETY: the entry point returns a static of the library, or null; the library is never unloaded (`loaded`)
    let Some(vtable) = (unsafe { vtable.as_ref() }) else {
        return Err(refused(path, "its entry point returned nothing"));
    };
    if vtable.abi_version != ABI_VERSION {
        return Err(refused(
            path,
            format!("it speaks plugin ABI {}, this core {ABI_VERSION}", vtable.abi_version),
        ));
    }
    let library = Arc::new(Library { _lib: lib, vtable });
    libraries.insert(key, library.clone());
    Ok(library)
}

/// The core's end of one open module: what the library writes, as a stream.
struct Sink {
    to_core: Mutex<Option<mpsc::UnboundedSender<Vec<u8>>>>,
}

unsafe extern "C" fn sink_send(ctx: *mut c_void, ptr: *const u8, len: usize) {
    if ctx.is_null() || (ptr.is_null() && len > 0) {
        return;
    }
    _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: ctx is the Sink `Open` owns, freed only after the library's close returned
        let sink = unsafe { &*ctx.cast::<Sink>() };
        let bytes = if len == 0 {
            Vec::new()
        } else {
            // SAFETY: the ABI's contract (module_api::plugin::HostSink): `len` readable bytes, copied before returning
            unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec()
        };
        if let Some(tx) = sink.to_core.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
            _ = tx.send(bytes);
        }
    }));
}

unsafe extern "C" fn sink_closed(ctx: *mut c_void) {
    if ctx.is_null() {
        return;
    }
    _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: as in sink_send
        let sink = unsafe { &*ctx.cast::<Sink>() };
        drop(sink.to_core.lock().unwrap_or_else(PoisonError::into_inner).take());
    }));
}

/// One open module: its handle in the library, and the sink the library writes to.
struct Open {
    library: Arc<Library>,
    handle: *mut c_void,
    sink: *mut Sink,
}

// SAFETY: the handle is the library's to use from any thread (module_api::plugin::PluginV1), and the sink is only
// touched through its Mutex
unsafe impl Send for Open {}

impl Open {
    fn send(&self, bytes: &[u8]) {
        // SAFETY: the handle is open until `drop`, and the library copies the bytes before returning
        unsafe { (self.library.vtable.send)(self.handle, bytes.as_ptr(), bytes.len()) }
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        // SAFETY: closed once, here; the library uses the sink no more once close returns, so it is freed after
        unsafe {
            (self.library.vtable.close)(self.handle);
            drop(Box::from_raw(self.sink));
        }
    }
}

/// A module running from a library in this process, behind the frame protocol.
pub struct DylibModule {
    inner: ChildModule,
    open: Arc<Mutex<Option<Open>>>,
}

impl DylibModule {
    /// Checks the library against its pinned SHA-256, loads it, opens its module and runs the handshake.
    pub async fn load(
        module: &ExternalModule,
        node: NodeIdentity,
        link: Arc<dyn HostLink>,
        options: ChildOptions,
    ) -> Result<Self, LoadError> {
        let (path, sha256) = (module.path.clone(), module.sha256);
        let library = tokio::task::spawn_blocking(move || load(&path, sha256))
            .await
            .map_err(|e| refused(&module.path, e.to_string()))??;
        let (to_core, from_library) = mpsc::unbounded_channel();
        let sink = Box::into_raw(Box::new(Sink {
            to_core: Mutex::new(Some(to_core)),
        }));
        let host_sink = HostSink {
            ctx: sink.cast(),
            send: sink_send,
            closed: sink_closed,
        };
        // SAFETY: the vtable stays valid (the library is never unloaded); the sink lives until `Open` drops
        let handle = unsafe { (library.vtable.open)(host_sink) };
        if handle.is_null() {
            // SAFETY: the library refused it and holds no pointer to it
            drop(unsafe { Box::from_raw(sink) });
            return Err(refused(&module.path, "its module did not start"));
        }
        let open = Arc::new(Mutex::new(Some(Open { library, handle, sink })));
        let writer = CallWriter::new({
            let open = open.clone();
            move |bytes: &[u8]| {
                if let Some(open) = open.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
                    open.send(bytes);
                }
            }
        });
        let reader = ChannelReader::new(from_library);
        match ChildModule::connect(reader, writer, node, link, options, module.expected.as_ref()).await {
            Ok(inner) => Ok(Self { inner, open }),
            Err(error) => {
                close(open).await;
                Err(error)
            }
        }
    }

    /// Why the module is gone, if it is: its library's side ended or broke the protocol.
    pub fn failure(&self) -> Option<String> {
        self.inner.failure()
    }
}

/// Closes the module off the async threads: the library waits for its runtime there.
async fn close(open: Arc<Mutex<Option<Open>>>) {
    let taken = open.lock().unwrap_or_else(PoisonError::into_inner).take();
    if let Some(taken) = taken {
        _ = tokio::task::spawn_blocking(move || drop(taken)).await;
    }
}

impl ModuleDriver for DylibModule {
    fn manifest(&self) -> &ModuleManifest {
        self.inner.manifest()
    }

    fn call(&self, call: ModuleCall) -> BoxFuture<'_, Result<ModuleAnswer, ModuleError>> {
        self.inner.call(call)
    }

    fn unload(&self) -> BoxFuture<'_, Result<(), ModuleError>> {
        Box::pin(async move {
            let stopped = self.inner.unload().await;
            close(self.open.clone()).await;
            stopped
        })
    }
}
