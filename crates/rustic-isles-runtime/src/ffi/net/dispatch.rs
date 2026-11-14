//! NETD-1, the base `UNetDriver::TickDispatch` (6): the engine walks `ClientConnections` there unchecked.

use super::{budgeted, hook, panicked, sanitize};
use crate::ffi::broker::NextCell;
use crate::ffi::exit::Install;
use crate::ffi::{detaching, log};
use rustic_isles_runtime_core::net::{DISPATCH_SKIPPED, DISPATCH_SKIPPED_KIND, LineBudget, budgets, dispatch_first};
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};

type TickDispatchFn = unsafe extern "C" fn(*mut c_void, f32);

static NEXT: NextCell = NextCell::new();
static FIRST: AtomicBool = AtomicBool::new(true);
static PANICKED: AtomicBool = AtomicBool::new(false);
static SKIPPED: LineBudget = LineBudget::new(budgets::DISPATCH_SKIPPED);

unsafe extern "C" fn detour(driver: *mut c_void, delta: f32) {
    let next = NEXT.load();
    if next == 0 {
        return;
    }
    // SAFETY: the broker wrote a function of TickDispatch's signature into this cell
    let next: TickDispatchFn = unsafe { std::mem::transmute::<usize, TickDispatchFn>(next) };
    let call_next = || {
        // SAFETY: the original arguments
        unsafe { next(driver, delta) }
    };
    if detaching() {
        call_next();
        return;
    }
    crate::lifecycle::write_status_once();
    let safe = catch_unwind(AssertUnwindSafe(|| {
        if FIRST.swap(false, Ordering::SeqCst) {
            log::line(&dispatch_first(driver as usize));
        }
        sanitize(driver as usize, "dispatch-inner")
    }));
    match safe {
        Ok(true) => call_next(),
        // Absorbed: the next frame runs it again on the repaired lists
        Ok(false) => budgeted(&SKIPPED, DISPATCH_SKIPPED_KIND, || DISPATCH_SKIPPED.to_owned()),
        Err(_) => {
            panicked("NETD-1", &PANICKED);
            call_next();
        }
    }
}

/// Installs NETD-1.
pub fn install(base: usize, hooks_off: Option<&'static str>) -> Install {
    hook("NETD-1", base, detour as *const () as usize, &NEXT, hooks_off)
}
