//! `SP-3@R12` (11): the quarantine's stale sweep, then the graph drive. `UNetDriver::TickFlush` never calls the
//! replication driver on this build, so undriven no client receives its game state. No fault catch around the
//! call (11.4, R11's ruling): a fault ends the server through R3's crash path.

use super::identity::ProcessGame;
use super::{BUDGETS, with_state};
use crate::ffi::memory::frame_tick;
use crate::ffi::{detaching, log};
use crate::listen::net_driver;
use rustic_isles_runtime_core::net::Reentry;
use rustic_isles_runtime_core::repgraph::{drive_line, drive_target};
use std::cell::Cell;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};

/// The graph's `ServerReplicateActors(graph, delta)`: the actors replicated.
type ServerReplicateActorsFn = unsafe extern "C" fn(*mut c_void, f32) -> i32;

static PANICKED: AtomicBool = AtomicBool::new(false);

thread_local! {
    // A nested engine tick inside the graph must not drive again
    static DRIVING: Cell<bool> = const { Cell::new(false) };
}

/// The after handler, order 500.
pub fn tick(_engine: usize, delta: f32) {
    if detaching() {
        return;
    }
    if catch_unwind(AssertUnwindSafe(|| drive(delta))).is_err() && !PANICKED.swap(true, Ordering::SeqCst) {
        log::line("[Panic] SP-3@R12 graph drive panicked; the frame goes on");
    }
}

/// Steps 1 to 6: the sweep, then the checked target.
pub fn target() -> Option<(usize, usize)> {
    with_state(|state| {
        state.quarantine.sweep(&ProcessGame, &BUDGETS);
        drive_target(&ProcessGame, &mut state.checked, net_driver())
    })
}

fn drive(delta: f32) {
    let Some((graph, function)) = target() else {
        return;
    };
    DRIVING.with(|flag| {
        let Some(_driving) = Reentry::enter(flag) else {
            return;
        };
        // SAFETY: the graph's own virtual slot, checked to be a `.pdata` function entry of the game, called on the
        // game thread with the graph of R9's live driver
        let result = unsafe {
            let function = std::mem::transmute::<usize, ServerReplicateActorsFn>(function);
            function(graph as *mut c_void, delta)
        };
        frame_tick(true);
        drive_line(&ProcessGame, &BUDGETS, graph, function, result);
    });
}
