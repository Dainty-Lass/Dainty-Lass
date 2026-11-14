//! RG-2's detour (6): the corrections first, then the original, always (a panic only skips the corrections).

use super::{BUDGETS, ProcessGame, panicked};
use crate::ffi::broker::NextCell;
use crate::ffi::detaching;
use rustic_isles_runtime_core::relevance as core;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::AtomicBool;

type Ptr = *mut c_void;

/// `RouteAddNetworkActorToNodes(graph, info, global_info)`.
pub type RouteFn = unsafe extern "C" fn(Ptr, Ptr, Ptr);

/// The function the detour calls next.
pub static NEXT: NextCell = NextCell::new();
/// The detour the broker installs.
pub const DETOUR: RouteFn = route_detour;
static PANICKED: AtomicBool = AtomicBool::new(false);

/// RG-2 with its `next` (tests pass their own).
pub fn route_with(next: Option<RouteFn>, graph: Ptr, info: Ptr, global_info: Ptr) {
    if !detaching() {
        crate::lifecycle::write_status_once();
        let corrected = catch_unwind(AssertUnwindSafe(|| {
            core::route(&ProcessGame, &BUDGETS, info as usize, global_info as usize)
        }));
        if corrected.is_err() {
            panicked("RG-2", &PANICKED);
        }
    }
    if let Some(next) = next {
        // SAFETY: the original arguments to the original function
        unsafe { next(graph, info, global_info) }
    }
}

unsafe extern "C" fn route_detour(graph: Ptr, info: Ptr, global_info: Ptr) {
    // SAFETY: the broker wrote a function of this signature into the cell (0 before it is hooked)
    let next = (NEXT.load() != 0).then(|| unsafe { std::mem::transmute::<usize, RouteFn>(NEXT.load()) });
    route_with(next, graph, info, global_info);
}
