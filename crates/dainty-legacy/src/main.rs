// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! With no arguments, the `legacy` plugin out of process (L-2; stdout is the frame pipe, so nothing prints
//! there); with `import <kind> ...`, the command line.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Result<Vec<String>, _> = std::env::args_os().skip(1).map(|a| a.into_string()).collect();
    match args {
        Ok(args) if args.is_empty() => dainty_module_api::child::main(dainty_legacy::plugin::LegacyPlugin::new()),
        Ok(args) => dainty_legacy::cli::main_from(&args),
        Err(_) => {
            eprintln!(
                "dainty-legacy: an argument is not valid Unicode\n{}",
                dainty_legacy::USAGE
            );
            ExitCode::from(2)
        }
    }
}
