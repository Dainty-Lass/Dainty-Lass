//! The login screen's patch-notes version lines (spec-runtime-r17 4.2). The background and logo wait for an MIT
//! image (RT-9, 4.4).

use super::{Budget, collapse, read_once};
use crate::ffi::{engine, log, reflect};
use crate::handlers::PeCall;
use rustic_isles_runtime_core::branding::{flags, function_short_name};
use rustic_isles_runtime_core::offsets::r17;
use std::sync::OnceLock;

static DISABLED: OnceLock<bool> = OnceLock::new();
static LOGGED: Budget = Budget::new();

pub(super) fn after(call: &PeCall) {
    if read_once(&DISABLED, flags::DISABLE_LOGIN_BRANDING) {
        return;
    }
    let screen = call.object;
    if !engine::is_live_object(screen) || !reflect::is_a(screen, "LoginScreen_bps_C") {
        return;
    }
    let lines = [
        ("preview", r17::LOGIN_PATCH_PREVIEW, r17::PREVIEW_VERSION_TEXT),
        ("popup", r17::LOGIN_PATCH_POPUP, r17::POPUP_VERSION_TEXT),
    ];
    for (which, widget, text) in lines {
        let Some(line) = reflect::live_ptr(screen + widget).and_then(|widget| reflect::live_ptr(widget + text)) else {
            continue;
        };
        if collapse(line) && LOGGED.take(16) {
            let name = call.entry.full_name();
            let short = name.as_deref().map_or("null", function_short_name);
            log::line(&format!("[MitBranding] collapsed {which} version line after {short}"));
        }
    }
}
