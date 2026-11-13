//! The seam exports, ABI version 1 (R0 7.3). The AGPL runtime (patch 0041) resolves all five by name and ends its
//! process with 0xE10A if one is missing or the version is not 1. Every export catches panics and answers its
//! failure value; none blocks for long. Lines go to the deferred list: these run under the AGPL's loader lock.

use super::{broker, log, minhook};
use rustic_isles_runtime_core::registry::{self, MAX_ID_BYTES};
use std::ffi::{c_char, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};

/// Up to `limit` bytes of a C string, or `None` when no NUL comes within them.
///
/// # Safety
/// `text` is null or readable up to its NUL or `limit` bytes.
unsafe fn c_bytes(text: *const c_char, limit: usize) -> Option<Vec<u8>> {
    if text.is_null() {
        return None;
    }
    let mut bytes = Vec::new();
    for index in 0..limit {
        // SAFETY: the caller's promise; the scan stops at the NUL
        let byte = unsafe { *text.add(index) } as u8;
        if byte == 0 {
            return Some(bytes);
        }
        bytes.push(byte);
    }
    None
}

/// The seam ABI version: 1.
#[unsafe(no_mangle)]
pub extern "C" fn rir_seam_version() -> u32 {
    rustic_isles_runtime_core::SEAM_VERSION
}

/// 1 when this build owns `id`, else 0 (also for null, unknown or over 47 bytes). No lock: callable under the
/// loader lock and from any thread.
///
/// # Safety
/// `id` is null or a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rir_owns(id: *const c_char) -> i32 {
    // SAFETY: the caller's promise about `id`
    catch_unwind(|| unsafe { c_bytes(id, MAX_ID_BYTES + 1) }.is_some_and(|id| registry::owns(&id))).map_or(0, i32::from)
}

/// Adds the AGPL wrapper on `target`'s chain (hooking it if needed) and writes the trampoline to `*next`.
/// MinHook's status numbers: 0, 3 when the AGPL band already has a wrapper there, or the hook's own error.
///
/// # Safety
/// `next` is null or writable; `id` is null or a NUL-terminated string; `target` and `wrapper` are functions.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rir_chain_add_inner(
    target: *mut c_void,
    wrapper: *mut c_void,
    id: *const c_char,
    next: *mut *mut c_void,
) -> i32 {
    let result = catch_unwind(AssertUnwindSafe(|| {
        if next.is_null() || wrapper.is_null() {
            return minhook::MH_UNKNOWN;
        }
        let status = match broker::add_inner(target as usize, wrapper as usize) {
            Ok(trampoline) => {
                // SAFETY: the caller's promise about `next`
                unsafe { *next = trampoline as *mut c_void };
                minhook::MH_OK
            }
            Err(status) => status,
        };
        // SAFETY: the caller's promise about `id`
        let id = unsafe { c_bytes(id, 64) }
            .map(|id| String::from_utf8_lossy(&id).into_owned())
            .unwrap_or_default();
        log::defer(format!(
            "[Broker] add inner {id} {target:p}: {}",
            minhook::status_name(status)
        ));
        status
    }));
    result.unwrap_or(minhook::MH_UNKNOWN)
}

/// Removes that AGPL wrapper: 0, or 4 when it is not there.
#[unsafe(no_mangle)]
pub extern "C" fn rir_chain_remove_inner(target: *mut c_void, wrapper: *mut c_void) -> i32 {
    let result = catch_unwind(|| {
        let status = match broker::remove_inner(target as usize, wrapper as usize) {
            Ok(()) => minhook::MH_OK,
            Err(status) => status,
        };
        log::defer(format!(
            "[Broker] remove inner {target:p}: {}",
            minhook::status_name(status)
        ));
        status
    });
    result.unwrap_or(minhook::MH_UNKNOWN)
}

/// The trampoline when the broker hooked `target`, else `target`: the game's function past every wrapper.
#[unsafe(no_mangle)]
pub extern "C" fn rir_original(target: *mut c_void) -> *mut c_void {
    catch_unwind(|| broker::original(target as usize) as *mut c_void).unwrap_or(target)
}
