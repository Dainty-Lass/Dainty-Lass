//! The press-start popup's welcome text (spec-runtime-r17 4.6). The panel class also backs the login screen's
//! queue dialogs, which keep their text: only a panel owned by a press-start screen is touched.

use super::{Budget, read_once, text};
use crate::ffi::{engine, log, reflect};
use crate::handlers::PeCall;
use rustic_isles_runtime_core::branding::{flags, function_short_name, keys};
use rustic_isles_runtime_core::offsets::{layout, r17};
use std::sync::OnceLock;

static DISABLED: OnceLock<bool> = OnceLock::new();
static LOGGED: Budget = Budget::new();

const SCREEN: &str = "PressStartScreen_bps_C";

fn panel(full_name: &str, object: usize) -> Option<usize> {
    if !engine::is_live_object(object) {
        return None;
    }
    if full_name.contains("PressStartScreen_bps_C.") {
        return reflect::is_a(object, SCREEN)
            .then(|| reflect::live_ptr(object + r17::PRESS_START_PANEL))
            .flatten();
    }
    if full_name.contains("PressStartPanel_bpw_C.") {
        // The panel's outer is its widget tree, whose outer is the owning screen
        let tree = reflect::ptr(object + layout::UOBJECT_OUTER)?;
        let owner = reflect::ptr(tree + layout::UOBJECT_OUTER)?;
        return reflect::is_a(owner, SCREEN).then_some(object);
    }
    None
}

pub(super) fn after(call: &PeCall) {
    if read_once(&DISABLED, flags::DISABLE_PRESS_START_BRANDING) {
        return;
    }
    let Some(full_name) = call.entry.full_name() else {
        return;
    };
    let Some(welcome) =
        panel(&full_name, call.object).and_then(|panel| reflect::live_ptr(panel + r17::PANEL_WELCOME_TEXT))
    else {
        return;
    };
    if reflect::set_text(welcome, text(keys::WELCOME)) && LOGGED.take(6) {
        log::line(&format!(
            "[MitPressStart] welcome text set after {}",
            function_short_name(&full_name)
        ));
    }
}
