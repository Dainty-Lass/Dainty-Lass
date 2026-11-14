//! PR-1's detour (5.2): the game's application first, then the validity note; and R14's own two ways in.

use super::{APPLY, ProcessGame, panicked, with_state};
use crate::config;
use crate::ffi::broker::{self, NextCell};
use crate::ffi::detaching;
use rustic_isles_runtime_core::player_role as core;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::AtomicBool;

type Ptr = *mut c_void;

/// `ApplyPlayerRole(loadout)`.
pub type ApplyFn = unsafe extern "C" fn(Ptr);

/// The function the detour calls next.
pub static NEXT: NextCell = NextCell::new();
/// The detour the broker installs.
pub const DETOUR: ApplyFn = apply_detour;
static PANICKED: AtomicBool = AtomicBool::new(false);

/// PR-1 with its `next` (tests pass their own).
pub fn apply_with(next: Option<ApplyFn>, loadout: Ptr) {
    if let Some(next) = next {
        // SAFETY: the original argument to the original function
        unsafe { next(loadout) }
    }
    if detaching() {
        return;
    }
    crate::lifecycle::write_status_once();
    let noted = catch_unwind(AssertUnwindSafe(|| {
        with_state(|state| core::on_apply(&ProcessGame, state, loadout as usize, config::is_server()))
    }));
    if noted.is_err() {
        panicked("PR-1", &PANICKED);
    }
}

unsafe extern "C" fn apply_detour(loadout: Ptr) {
    // SAFETY: the broker wrote a function of this signature into the cell (0 before it is hooked)
    let next = (NEXT.load() != 0).then(|| unsafe { std::mem::transmute::<usize, ApplyFn>(NEXT.load()) });
    apply_with(next, loadout);
}

/// The application on `loadout`: through the hooked entry (`wrapper`, so 5.2 notes it) or past every wrapper.
pub fn call(loadout: usize, wrapper: bool) {
    let entry = APPLY.address();
    if entry == 0 || loadout == 0 || detaching() || !crate::ffi::engine::is_live_object(loadout) {
        return;
    }
    let function = if wrapper { entry } else { broker::original(entry) };
    // SAFETY: PR-1 of this build (its first bytes checked) or its trampoline, `(loadout)`; the callers
    // and this call checked the loadout live
    unsafe { std::mem::transmute::<usize, ApplyFn>(function)(loadout as Ptr) }
}
