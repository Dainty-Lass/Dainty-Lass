//! The vendored MinHook's C API (`minhook/include/MinHook.h`). Only the broker calls it (R0 7.2).

use std::ffi::c_void;
use std::sync::OnceLock;

/// `MH_OK`.
pub const MH_OK: i32 = 0;
/// `MH_ERROR_ALREADY_INITIALIZED`.
pub const MH_ERROR_ALREADY_INITIALIZED: i32 = 1;
/// `MH_UNKNOWN`: also what an export answers when it caught a panic.
pub const MH_UNKNOWN: i32 = -1;

unsafe extern "system" {
    fn MH_Initialize() -> i32;
    fn MH_CreateHook(target: *mut c_void, detour: *mut c_void, original: *mut *mut c_void) -> i32;
    fn MH_RemoveHook(target: *mut c_void) -> i32;
    fn MH_EnableHook(target: *mut c_void) -> i32;
}

/// MinHook's status name, for the install lines.
pub fn status_name(status: i32) -> &'static str {
    match status {
        -1 => "MH_UNKNOWN",
        0 => "MH_OK",
        1 => "MH_ERROR_ALREADY_INITIALIZED",
        2 => "MH_ERROR_NOT_INITIALIZED",
        3 => "MH_ERROR_ALREADY_CREATED",
        4 => "MH_ERROR_NOT_CREATED",
        5 => "MH_ERROR_ENABLED",
        6 => "MH_ERROR_DISABLED",
        7 => "MH_ERROR_NOT_EXECUTABLE",
        8 => "MH_ERROR_UNSUPPORTED_FUNCTION",
        9 => "MH_ERROR_MEMORY_ALLOC",
        10 => "MH_ERROR_MEMORY_PROTECT",
        11 => "MH_ERROR_MODULE_NOT_FOUND",
        12 => "MH_ERROR_FUNCTION_NOT_FOUND",
        _ => "MH_STATUS_UNKNOWN_VALUE",
    }
}

/// `MH_Initialize`, once per process; an earlier initialisation counts as success.
pub fn initialize() -> i32 {
    static STATUS: OnceLock<i32> = OnceLock::new();
    *STATUS.get_or_init(|| {
        // SAFETY: no arguments; MinHook allocates its own private heap
        match unsafe { MH_Initialize() } {
            MH_ERROR_ALREADY_INITIALIZED => MH_OK,
            status => status,
        }
    })
}

/// `MH_CreateHook(target, detour, &trampoline)`: the trampoline on success.
pub fn create(target: usize, detour: usize) -> Result<usize, i32> {
    let mut trampoline: *mut c_void = std::ptr::null_mut();
    // SAFETY: MinHook checks the target is executable and decodes its prologue before patching anything
    let status = unsafe { MH_CreateHook(target as *mut c_void, detour as *mut c_void, &mut trampoline) };
    if status == MH_OK {
        Ok(trampoline as usize)
    } else {
        Err(status)
    }
}

/// `MH_EnableHook(target)`.
pub fn enable(target: usize) -> i32 {
    // SAFETY: only for a target `create` hooked
    unsafe { MH_EnableHook(target as *mut c_void) }
}

/// `MH_RemoveHook(target)`, to roll back a hook that could not be enabled.
pub fn remove(target: usize) -> i32 {
    // SAFETY: only for a target `create` hooked
    unsafe { MH_RemoveHook(target as *mut c_void) }
}
