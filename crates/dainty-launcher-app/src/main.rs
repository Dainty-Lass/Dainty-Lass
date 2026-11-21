// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `dainty-launcher` (RD27 spec §2.1.1); with `server-gui`, the manager.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::process::ExitCode;

use dainty_launcher_app::Options;

/// The player's launcher.
#[cfg(not(feature = "server-gui"))]
fn options() -> Options {
    Options {
        icon: Some(tauri::include_image!("icons/32x32.png")),
    }
}

/// The manager: its services log to the page, so the tracing goes in before anything starts (spec-tauri-shell §8).
#[cfg(feature = "server-gui")]
fn options() -> Options {
    let logs = dainty_manager_server::init_telemetry();
    Options {
        embedded_server: Some(std::sync::Arc::new(dainty_manager_server::EmbeddedServerState::new(
            logs,
        ))),
        icon: Some(tauri::include_image!("icons/manager/32x32.png")),
    }
}

fn main() -> ExitCode {
    // This executable is also the reader of the game's console (§2.7.16), and that must start nothing else
    if let Some(code) = dainty_launcher_kit::platform::game_console::helper_main() {
        return code;
    }
    match dainty_launcher_app::run(options()) {
        // A game's exit code is 32 bits on Windows, more than ExitCode carries
        Ok(code) => std::process::exit(code),
        Err(error) => {
            eprintln!("Dainty Lass could not start: {error}");
            ExitCode::from(1)
        }
    }
}
