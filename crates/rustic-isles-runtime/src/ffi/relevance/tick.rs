//! `SP-3@R13`'s handler (7.1) and the one wake call (7.3).

use super::{BUDGETS, ProcessGame, with_state};
use crate::ffi::loading::game::{params_fit, process_event_original};
use crate::ffi::memory::frame_tick;
use crate::ffi::{detaching, engine, flags, log, reflect, repgraph};
use crate::listen::{client_connections, net_driver};
use rustic_isles_runtime_core::offsets::relevance::DORM_AWAKE;
use rustic_isles_runtime_core::reflect::find_function;
use rustic_isles_runtime_core::relevance::{self as core, WakeCall};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// `SetNetDormancy`'s parameter block: one `TEnumAsByte`, padded to 8.
const WAKE_PARAMS: usize = 8;

static PANICKED: AtomicBool = AtomicBool::new(false);
static SET_NET_DORMANCY: AtomicUsize = AtomicUsize::new(0);

/// `DISABLE_PLAYER_REP_BOOST.flag`, read once (8).
fn pawn_boost() -> bool {
    static OFF: OnceLock<bool> = OnceLock::new();
    !*OFF.get_or_init(|| flags::flag(core::flags::DISABLE_PLAYER_REP_BOOST))
}

/// The after handler, order 400.
pub fn tick(_engine: usize, _delta: f32) {
    if detaching() {
        return;
    }
    crate::lifecycle::write_status_once();
    if catch_unwind(AssertUnwindSafe(run)).is_err() && !PANICKED.swap(true, Ordering::SeqCst) {
        log::line("[Panic] SP-3@R13 relevance tick panicked; the frame goes on");
    }
}

fn run() {
    let Some(driver) = net_driver() else {
        return;
    };
    let connections = client_connections(driver);
    let boost = pawn_boost();
    with_state(|state| core::tick(&ProcessGame, state, &BUDGETS, &connections, &repgraph::graph, boost));
}

/// `SetNetDormancy` on the weapon's class chain, kept while it is live.
fn set_net_dormancy(weapon: usize) -> Option<usize> {
    let cached = SET_NET_DORMANCY.load(Ordering::Relaxed);
    if cached != 0 && engine::is_live_object(cached) {
        return Some(cached);
    }
    let function = find_function(&reflect::engine()?, reflect::class_of(weapon)?, "SetNetDormancy")?;
    SET_NET_DORMANCY.store(function, Ordering::Relaxed);
    Some(function)
}

/// 7.3: `SetNetDormancy(DORM_Awake)` past every wrapper, so no SP-1 or SP-2 handler sees it.
pub fn wake(weapon: usize) -> WakeCall {
    match set_net_dormancy(weapon) {
        Some(function) => call_wake(weapon, function),
        None => WakeCall::NotFound,
    }
}

/// The call itself, once the function is found (the tests pass their own).
pub fn call_wake(weapon: usize, function: usize) -> WakeCall {
    if params_fit(function, WAKE_PARAMS).is_err() {
        return WakeCall::Skipped;
    }
    let mut block = [u64::from(DORM_AWAKE)];
    if !process_event_original(weapon, function, &mut block) {
        return WakeCall::Skipped;
    }
    // Game code ran (R11 8.3)
    frame_tick(true);
    WakeCall::Made
}
