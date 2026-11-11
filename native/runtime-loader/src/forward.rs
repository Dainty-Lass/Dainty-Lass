//! Lazy, lock-free resolution of the system DLLs' functions.

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::sys;

const UNRESOLVED: usize = 0;
// No module handle or function address is ever 1, so it can mark "looked for, not there".
const MISSING: usize = 1;

/// A system DLL, loaded by full path on first use.
pub struct Library {
    file: &'static str,
    handle: AtomicUsize,
}

pub static WINMM: Library = Library::new("winmm.dll");
pub static VERSION: Library = Library::new("version.dll");

impl Library {
    const fn new(file: &'static str) -> Self {
        Self {
            file,
            handle: AtomicUsize::new(UNRESOLVED),
        }
    }

    fn handle(&self) -> Option<sys::Module> {
        let handle = match self.handle.load(Ordering::Acquire) {
            UNRESOLVED => {
                // Never by bare name: that finds this DLL again.
                let found = sys::system_directory().and_then(|dir| sys::load(&dir.join(self.file)));
                let handle = found.map_or(MISSING, |module| module as usize);
                self.handle.store(handle, Ordering::Release);
                handle
            }
            handle => handle,
        };
        (handle != MISSING).then_some(handle as sys::Module)
    }
}

/// One export's cached system function.
pub struct Slot(AtomicUsize);

impl Slot {
    pub const fn new() -> Self {
        Self(AtomicUsize::new(UNRESOLVED))
    }

    /// The function's address, or `None` when the DLL or the function is missing. `name` is NUL-terminated ASCII.
    pub fn get(&self, library: &Library, name: &'static str) -> Option<usize> {
        let address = match self.0.load(Ordering::Acquire) {
            UNRESOLVED => {
                let address = library
                    .handle()
                    // SAFETY: `module` is a loaded DLL that is never freed and `name` ends in NUL.
                    .map(|module| unsafe { sys::GetProcAddress(module, name.as_ptr()) })
                    .filter(|&address| address != 0)
                    .unwrap_or(MISSING);
                self.0.store(address, Ordering::Release);
                address
            }
            address => address,
        };
        (address != MISSING).then_some(address)
    }
}
