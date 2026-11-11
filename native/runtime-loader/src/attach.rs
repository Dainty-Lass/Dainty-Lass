//! `DllMain`: loads the runtime and the ini's DLLs while the game starts.

use std::ffi::c_void;
use std::path::Path;

use crate::{ini, runtime, sys};

const DLL_PROCESS_ATTACH: u32 = 1;
const TRUE: i32 = 1;

/// # Safety
///
/// Called by the Windows loader only.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllMain(module: sys::Module, reason: u32, _reserved: *mut c_void) -> i32 {
    if reason == DLL_PROCESS_ATTACH {
        // Loads under the loader lock: the game expects the runtime in place before its own code runs.
        // A panic is swallowed so the game still starts (release builds abort instead of unwinding).
        let _ = std::panic::catch_unwind(|| attach(module));
    }
    TRUE
}

fn attach(module: sys::Module) {
    let Some(own) = sys::module_path(module) else {
        return;
    };
    let Some(folder) = own.parent() else {
        return;
    };
    if let Some(runtime) = runtime::choose(folder) {
        let _ = sys::load(&runtime);
    }
    load_extras(folder);
}

fn load_extras(folder: &Path) {
    let Ok(bytes) = std::fs::read(folder.join(ini::FILE)) else {
        return;
    };
    for path in ini::paths(folder, &ini::values(&bytes)) {
        if path.exists() {
            let _ = sys::load(&path);
        }
    }
}
