//! The server's ways in: RG-3's detour (9), `SP-2@R14` (10) and `SP-3@R14` (6).

use super::{ProcessGame, panicked, with_state};
use crate::ffi::broker::NextCell;
use crate::ffi::detaching;
use crate::handlers::{PeCall, Verdict};
use rustic_isles_runtime_core::player_role as core;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::AtomicBool;

type Ptr = *mut c_void;

/// `RouteRemoveNetworkActor(graph, info)`.
pub type RemoveFn = unsafe extern "C" fn(Ptr, Ptr);

/// The function RG-3's detour calls next.
pub static REMOVE_NEXT: NextCell = NextCell::new();
/// RG-3's detour.
pub const REMOVE_DETOUR: RemoveFn = remove_detour;
static RG3_PANICKED: AtomicBool = AtomicBool::new(false);
static SP2_PANICKED: AtomicBool = AtomicBool::new(false);
static SP3_PANICKED: AtomicBool = AtomicBool::new(false);

/// RG-3 with its `next` (tests pass their own): the purge, then the original, always.
pub fn remove_with(next: Option<RemoveFn>, graph: Ptr, info: Ptr) {
    if !detaching() {
        crate::lifecycle::write_status_once();
        let purged = catch_unwind(AssertUnwindSafe(|| {
            with_state(|state| core::route_remove(&ProcessGame, state, info as usize))
        }));
        if purged.is_err() {
            panicked("RG-3", &RG3_PANICKED);
        }
    }
    if let Some(next) = next {
        // SAFETY: the original arguments to the original function
        unsafe { next(graph, info) }
    }
}

unsafe extern "C" fn remove_detour(graph: Ptr, info: Ptr) {
    // SAFETY: the broker wrote a function of this signature into the cell (0 before it is hooked)
    let next = (REMOVE_NEXT.load() != 0).then(|| unsafe { std::mem::transmute::<usize, RemoveFn>(REMOVE_NEXT.load()) });
    remove_with(next, graph, info);
}

/// SP-2 before the two ability RPCs: the server activates first, and the RPC still runs.
pub fn ability(call: &PeCall) -> Verdict {
    if detaching() {
        return Verdict::Continue;
    }
    let long = call
        .entry
        .full_name()
        .is_some_and(|name| name.ends_with("WithEventData"));
    let routed = catch_unwind(AssertUnwindSafe(|| {
        with_state(|state| core::ability(&ProcessGame, state, call.object, call.params, long))
    }));
    if routed.is_err() {
        panicked("SP-2@R14", &SP2_PANICKED);
    }
    Verdict::Continue
}

/// SP-3 after the engine tick, order 600.
pub fn tick(_engine: usize, _delta: f32) {
    if detaching() {
        return;
    }
    if catch_unwind(AssertUnwindSafe(|| {
        with_state(|state| core::server_pump(&ProcessGame, state))
    }))
    .is_err()
    {
        panicked("SP-3@R14", &SP3_PANICKED);
    }
}
