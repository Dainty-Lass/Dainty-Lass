//! The manual net tick (8): the hub's driver is R9's own and not in the engine's world context, so the engine tick
//! never dispatches or flushes it. After the engine tick: the IP driver's `TickDispatch`, then `TickFlush`. No
//! fault catch around either (rulings R11): a fault ends the server through R3's crash path.

use super::{Callee, log, sanitize};
use crate::ffi::detaching;
use crate::ffi::memory::frame_tick;
use crate::listen::{listening, net_driver};
use rustic_isles_runtime_core::net::{Reentry, TickCall, manual_tick_mismatch, manual_tick_on};
use rustic_isles_runtime_core::offsets::net::{IP_TICK_DISPATCH, IP_TICK_DISPATCH_BYTES, TICK_FLUSH, TICK_FLUSH_BYTES};
use std::cell::Cell;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};

type DriverTickFn = unsafe extern "C" fn(*mut c_void, f32);

static DISPATCH: Callee = Callee::new(IP_TICK_DISPATCH, IP_TICK_DISPATCH_BYTES);
static FLUSH: Callee = Callee::new(TICK_FLUSH, TICK_FLUSH_BYTES);
static BOTH_LOGGED: AtomicBool = AtomicBool::new(false);
static PANICKED: AtomicBool = AtomicBool::new(false);

thread_local! {
    // A nested engine tick inside the drive must not drive again (the AGPL measured a stack overflow)
    static DRIVING: Cell<bool> = const { Cell::new(false) };
}

/// The after handler, order 800.
pub fn tick(_engine: usize, delta: f32) {
    if detaching() {
        return;
    }
    if catch_unwind(AssertUnwindSafe(|| drive(delta))).is_err() && !PANICKED.swap(true, Ordering::SeqCst) {
        log::line("[Panic] SP-3@R11 manual net tick panicked; the frame goes on");
    }
}

fn drive(delta: f32) {
    if !listening() {
        return;
    }
    let Some(driver) = net_driver() else {
        return;
    };
    DRIVING.with(|flag| {
        let Some(_driving) = Reentry::enter(flag) else {
            return;
        };
        let dispatched = call(&DISPATCH, TickCall::Dispatch, driver, delta, "before-dispatch");
        let flushed = call(&FLUSH, TickCall::Flush, driver, delta, "before-flush");
        if dispatched && flushed && !BOTH_LOGGED.swap(true, Ordering::SeqCst) {
            log::line(&manual_tick_on(driver));
        }
    });
}

/// One call when the sanitiser and the target's check allow it; then a new frame epoch, since game code ran.
fn call(callee: &Callee, which: TickCall, driver: usize, delta: f32, site: &'static str) -> bool {
    if !sanitize(driver, site) {
        return false;
    }
    let Some(function) = callee.address(|| log::line(&manual_tick_mismatch(which, callee.rva))) else {
        return false;
    };
    // SAFETY: the engine function at its checked entry, on the game thread, with R9's live driver (the AGPL
    // measured that passing the engine faults at once)
    unsafe {
        let function = std::mem::transmute::<usize, DriverTickFn>(function);
        function(driver as *mut c_void, delta);
    }
    frame_tick(true);
    true
}
