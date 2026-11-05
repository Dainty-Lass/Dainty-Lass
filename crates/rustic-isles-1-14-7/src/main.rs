// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The 1.14.7 module as a child executable (spec §9.3): stdout is the frame pipe, so nothing prints there.

use std::process::ExitCode;

fn main() -> ExitCode {
    match rustic_isles_1_14_7::module() {
        Ok(module) => module_api::child::main(module),
        Err(error) => {
            eprintln!("rustic-isles-1-14-7: {error}");
            ExitCode::FAILURE
        }
    }
}
