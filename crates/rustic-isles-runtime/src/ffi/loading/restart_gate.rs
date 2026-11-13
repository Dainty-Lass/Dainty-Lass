//! `SP-2@R7`, the restart data gate (7): a hunt player's restart waits for LD-2, at most 90 s.

use super::super::{flags, log};
use super::game::{self, read_ptr, readable};
use crate::config;
use crate::handlers::PeCall;
use rustic_isles_runtime_core::config::auth::AuthMode;
use rustic_isles_runtime_core::loading::{Hold, RestartGate, is_hub_class, lines};
use rustic_isles_runtime_core::offsets::r7;
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};

static GATE: Mutex<RestartGate> = Mutex::new(RestartGate::new());

fn gate() -> MutexGuard<'static, RestartGate> {
    GATE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// LD-2: the controller's data loaded.
pub fn mark_loaded(controller: usize) {
    gate().mark_loaded(controller);
}

/// Whether the controller's data loaded, as the gate knows it (server only; R14's HUD refresh may read it).
pub fn loaded(controller: usize) -> bool {
    gate().loaded(controller)
}

/// 7.1's process-wide part: a DeployServer gameserver without `DISABLE_RESTART_DATA_GATE.flag` (read once).
fn enabled() -> bool {
    static DISABLED: OnceLock<bool> = OnceLock::new();
    config::is_server()
        && config::auth_mode() == AuthMode::GlobalGameserver
        && !*DISABLED.get_or_init(|| flags::flag("DISABLE_RESTART_DATA_GATE"))
}

/// 7.1's per-call part and `should_hold`: a remote player's restart on a hunt's game mode.
fn held(game_mode: usize, controller: usize) -> bool {
    let Some(class) = game::class_name(game_mode) else {
        return false;
    };
    if is_hub_class(&class) {
        return false;
    }
    if !readable(controller, r7::PC_SPAN_GATE) || !game::is_live(controller) {
        return false;
    }
    if read_ptr(controller + r7::PC_NET_CONNECTION).unwrap_or(0) == 0 {
        return false;
    }
    let hold = gate().should_hold(controller, game::now());
    match hold {
        Hold::First => log::line(&lines::restart_held(controller)),
        Hold::GaveUp => log::line(&lines::restart_released(controller)),
        Hold::Held | Hold::Allow => {}
    }
    hold.holds()
}

/// `restart_gate::hold`, for R20's `PlayerCanRestart` force (7.4): true = answer false without the original.
/// `game_mode` is the object `PlayerCanRestart` runs on, which 7.1's hub rule reads.
pub fn hold(game_mode: usize, controller: usize) -> bool {
    enabled() && held(game_mode, controller)
}

/// The gate on one `PlayerCanRestart` params block: a held player's `ReturnValue` becomes 0.
pub fn apply(game_mode: usize, params: usize, enabled: bool) {
    if !enabled || !readable(params, r7::CAN_RESTART_PARAMS) {
        return;
    }
    let controller = read_ptr(params).unwrap_or(0);
    if held(game_mode, controller) {
        game::write::<u8>(params + r7::CAN_RESTART_RETURN, 0);
    }
}

/// SP-2 after `PlayerCanRestart` (7.3): after R20's AGPL block forced true, so its fix-ups still run.
pub fn after(call: &PeCall) {
    apply(call.object, call.params, enabled());
}
