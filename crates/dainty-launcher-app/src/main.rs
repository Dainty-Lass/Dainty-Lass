// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! `dainty-launcher` (RD27 spec §2.1.1); with `server-gui`, the manager.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::process::ExitCode;

fn main() -> ExitCode {
    // This executable is also the reader of the game's console (§2.7.16), and that must start nothing else
    if let Some(code) = dainty_launcher_kit::platform::game_console::helper_main() {
        return code;
    }
    match dainty_launcher_app::run(dainty_launcher_app::Options::default()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Dainty Lass could not start: {error}");
            ExitCode::from(1)
        }
    }
}
