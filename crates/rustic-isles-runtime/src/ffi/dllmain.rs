//! `DllMain` (R0 3.1). It runs under the loader lock before the engine reads its command line: Init allocates only
//! through the process heap, takes only SRW locks, starts no thread and writes nothing to the log.

use std::ffi::c_void;
use std::panic::catch_unwind;
use windows_sys::Win32::Foundation::HMODULE;
use windows_sys::Win32::System::LibraryLoader::{
    DisableThreadLibraryCalls, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN,
    GetModuleHandleExW,
};

const DLL_PROCESS_DETACH: u32 = 0;
const DLL_PROCESS_ATTACH: u32 = 1;

/// The loader's entry point (called by the static CRT's `_DllMainCRTStartup`).
///
/// # Safety
/// Called by the Windows loader only.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllMain(module: HMODULE, reason: u32, _reserved: *mut c_void) -> i32 {
    match reason {
        DLL_PROCESS_ATTACH => {
            // SAFETY: this module's handle and an address inside it; pinned so a chain never points into an
            // unloaded image
            unsafe {
                DisableThreadLibraryCalls(module);
                let mut pinned: HMODULE = std::ptr::null_mut();
                GetModuleHandleExW(
                    GET_MODULE_HANDLE_EX_FLAG_PIN | GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
                    DllMain as *const () as *const u16,
                    &mut pinned,
                );
            }
            // Init never fails the attach: a broken step leaves its part off
            let _ = catch_unwind(crate::lifecycle::init);
        }
        DLL_PROCESS_DETACH => super::set_detaching(),
        _ => {}
    }
    1
}
