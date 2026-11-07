// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Stand-in for the tray (RD27 spec §2.2.1, Category A, moved in later): hiding only minimises.
//! The tray's Quit calls `CloseGuard::set_quitting` and then closes the window.

use tauri::{AppHandle, Manager, WebviewWindow};

use crate::window::MAIN;

/// Hides the window to the tray.
pub fn hide(window: &WebviewWindow) {
    let _ = window.minimize();
}

/// Shows, unminimises and focuses the window (the tray's Open, a second launcher).
pub fn restore(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
