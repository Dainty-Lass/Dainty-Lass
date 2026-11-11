// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `dainty-legacy import <kind> ...` (spec §3): the report on stdout and 0, `dainty-legacy: <error>` on stderr
//! and 1, the usage on stderr and 2 for an unknown kind or command. The node is `local` unless `--node` says.

use std::process::ExitCode;

use crate::args::{self, Kind};
use crate::error::USAGE;

/// Runs the command line on `args`, the arguments after the program's name.
pub fn main_from(args: &[String]) -> ExitCode {
    let kind = match args {
        [command, kind, ..] if command == "import" => Kind::parse(kind),
        _ => None,
    };
    let Some(kind) = kind else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let fail = |error: crate::ImportError| {
        eprintln!("dainty-legacy: {error}");
        ExitCode::FAILURE
    };
    let parsed = match args::parse(kind, &args[2..]) {
        Ok(parsed) => parsed,
        Err(error) => return fail(error),
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("dainty-legacy: could not start the async runtime: {error}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(crate::run(parsed, "local")) {
        Ok(report) => {
            println!("{}", report.text());
            ExitCode::SUCCESS
        }
        Err(error) => fail(error),
    }
}
