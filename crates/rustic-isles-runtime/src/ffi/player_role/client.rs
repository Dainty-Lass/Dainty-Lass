//! `SP-1@R14` (11, 12): the client's handlers on `ProcessEvent`.

use super::{ProcessGame, panicked, with_state};
use crate::ffi::{detaching, dispatch};
use crate::handlers::{PeCall, Verdict};
use rustic_isles_runtime_core::player_role::{self as core, State};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::AtomicBool;

static PANICKED: AtomicBool = AtomicBool::new(false);

/// Runs `body` past the detaching check, inside `catch_unwind` (4.1).
fn guarded(body: impl FnOnce(&State)) {
    if detaching() {
        return;
    }
    if catch_unwind(AssertUnwindSafe(|| with_state(body))).is_err() {
        panicked("SP-1@R14", &PANICKED);
    }
}

/// Before `OnAirshipStateUpdate` (12.1).
pub fn airship_capture(call: &PeCall) -> Verdict {
    guarded(|state| core::hud_capture(&ProcessGame, state, call.object, call.params));
    Verdict::Continue
}

/// Before every call: the pump (11.2), on outermost calls only.
pub fn pump(_call: &PeCall) -> Verdict {
    let depth = dispatch::depth();
    if depth == 1 {
        guarded(|state| core::client_pump(&ProcessGame, state, depth));
    }
    Verdict::Continue
}

/// After `ClientRestart`, past R7's capture (11.1).
pub fn client_restart(_call: &PeCall) {
    guarded(|state| core::client_restart(&ProcessGame, state));
}

/// After `OnInputModeChanged` (11.3, 11.6).
pub fn input_mode_changed(call: &PeCall) {
    guarded(|state| core::input_mode_changed(&ProcessGame, state, call.object, call.params));
}

/// After the HUD names (12.2).
pub fn hud(call: &PeCall) {
    let trigger = match call
        .entry
        .full_name()
        .as_deref()
        .and_then(|name| name.rsplit('.').next())
    {
        Some("OnAirshipStateUpdate") => "state-update",
        Some("OnHUDVisibilityChanged") => "hud-visibility-changed",
        _ => "hud-show-path",
    };
    guarded(|state| core::reconcile(&ProcessGame, state, trigger));
}
