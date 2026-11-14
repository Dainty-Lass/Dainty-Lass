//! NETD-6 `UNetConnection::Close` (12), both roles: the only native view of who closes a connection. The
//! connection's `+0x1318` and the two virtual calls `Close` makes are never read (an AGPL read of `+0x1318` crashed).

use super::{hook, log, panicked};
use crate::ffi::broker::NextCell;
use crate::ffi::exit::{Install, stack_text};
use crate::ffi::memory::{read, readable};
use crate::ffi::{detaching, reflect};
use rustic_isles_runtime_core::net::{
    CLOSE_CAPTURE, CLOSE_CHECKED, CLOSE_KEEP, CLOSE_LOGGED, close_enter, close_exit, close_invalid,
};
use rustic_isles_runtime_core::offsets::net::{CONNECTION_LAST_RECEIVE, CONNECTION_STATE};
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

type CloseFn = unsafe extern "C" fn(*mut c_void);

static NEXT: NextCell = NextCell::new();
static SEQ: AtomicU32 = AtomicU32::new(0);
static SERVER: AtomicBool = AtomicBool::new(false);
static PANICKED: AtomicBool = AtomicBool::new(false);

/// Init records the role for the lines.
pub fn set_role(server: bool) {
    SERVER.store(server, Ordering::SeqCst);
}

unsafe extern "C" fn detour(connection: *mut c_void) {
    let next = NEXT.load();
    if next == 0 {
        return;
    }
    // SAFETY: the broker wrote a function of Close's signature into this cell
    let next: CloseFn = unsafe { std::mem::transmute::<usize, CloseFn>(next) };
    let call_next = || {
        // SAFETY: the original argument
        unsafe { next(connection) }
    };
    if detaching() {
        call_next();
        return;
    }
    crate::lifecycle::write_status_once();
    // Every call counts, valid or not (R11 departure 6)
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    if seq >= CLOSE_LOGGED {
        call_next();
        return;
    }
    let server = SERVER.load(Ordering::Relaxed);
    let connection = connection as usize;
    let entered = catch_unwind(AssertUnwindSafe(|| {
        if connection == 0 || !readable(connection, CLOSE_CHECKED) {
            log::line(&close_invalid(server, seq, connection));
            return false;
        }
        let state = reflect::u8_at(connection + CONNECTION_STATE).unwrap_or(0);
        let last_receive = read::<f64>(connection + CONNECTION_LAST_RECEIVE).unwrap_or(0.0);
        let stack = stack_text(CLOSE_CAPTURE, CLOSE_KEEP);
        log::line(&close_enter(server, seq, connection, state, last_receive, &stack));
        true
    }));
    if entered.is_err() {
        panicked("NETD-6", &PANICKED);
    }
    call_next();
    if entered.unwrap_or(false) {
        log::line(&close_exit(server, seq, connection));
    }
}

/// Installs NETD-6.
pub fn install(base: usize, hooks_off: Option<&'static str>) -> Install {
    hook("NETD-6", base, detour as *const () as usize, &NEXT, hooks_off)
}
