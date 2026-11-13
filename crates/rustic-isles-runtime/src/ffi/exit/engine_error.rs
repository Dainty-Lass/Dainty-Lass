//! EXIT-13 (R3 6): `FWindowsErrorOutputDevice::HandleError` on servers. Logs the engine's error history as one line
//! and always calls the original: an earlier AGPL version that returned early left the engine running corrupted.

use super::super::broker::{self, NextCell};
use super::super::memory::{read_unaligned, verify_entry};
use super::super::minhook;
use super::{emit, enter, panicked};
use crate::lifecycle::write_status_once;
use rustic_isles_runtime_core::exit::{engine_error_line, error_text, read_error_history};
use rustic_isles_runtime_core::offsets::{FoundBy, data, target};
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: NextCell = NextCell::new();
static HISTORY: AtomicUsize = AtomicUsize::new(0);

type HandleErrorFn = unsafe extern "system" fn(*mut c_void);

unsafe extern "system" fn handle_error(this: *mut c_void) {
    let next = NEXT.load();
    if next == 0 {
        return;
    }
    // SAFETY: the broker wrote HandleError's trampoline (or the next wrapper) into the cell
    let next = unsafe { std::mem::transmute::<usize, HandleErrorFn>(next) };
    if let Some(inside) = enter() {
        write_status_once();
        if catch_unwind(AssertUnwindSafe(log_history)).is_err() {
            panicked(13, "hook panicked; the call goes on");
        }
        drop(inside);
    }
    // SAFETY: the original argument
    unsafe { next(this) }
}

fn log_history() {
    let history = HISTORY.load(Ordering::Relaxed);
    let (units, truncated) = read_error_history(|index| {
        history
            .checked_add(index * 2)
            .filter(|_| history != 0)
            .and_then(read_unaligned::<u16>)
    });
    emit(&engine_error_line(&error_text(&units, truncated)));
}

/// Hooks `HandleError` when its entry and prologue match (R3 6.1).
pub(super) fn install(base: usize, hooks_off: Option<&'static str>) -> Result<(), &'static str> {
    if let Some(status) = hooks_off {
        return Err(status);
    }
    let line = target("EXIT-13").ok_or("no target line")?;
    let FoundBy::RvaAndBytes(prologue) = line.found_by else {
        return Err("no target line");
    };
    if !verify_entry(line.rva, prologue) {
        return Err("prologue mismatch");
    }
    let address = base.checked_add(line.rva as usize).ok_or("prologue mismatch")?;
    HISTORY.store(base.saturating_add(data::ERROR_HISTORY as usize), Ordering::SeqCst);
    broker::hook(address, handle_error as *const () as usize, 0, &NEXT).map_err(minhook::status_name)
}
