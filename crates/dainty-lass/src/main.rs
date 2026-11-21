// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! `dainty-lass`: the MIT core (RD27). The admin API and remote management are built in; every other service
//! is a library `[services.<name>]` pins, and every game module one `[[modules.external]]` pins. Bare, or with
//! options only, it serves (as `dainty-server` does); otherwise it takes the host kit's commands.
//! `--root <dir>` first names the server folder, which is found from here otherwise.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser as _;

struct CoreHooks;

impl dainty_hostkit::hooks::HostHooks for CoreHooks {
    fn built_in_services(&self) -> &'static [&'static str] {
        &["admin"]
    }
}

static HOOKS: CoreHooks = CoreHooks;

fn main() -> ExitCode {
    dainty_hostkit::hooks::set_hooks(&HOOKS);
    let mut args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    let mut root = None;
    if args.get(1).is_some_and(|a| a == "--root") {
        let Some(dir) = args.get(2).cloned() else {
            eprintln!("--root needs a folder");
            return ExitCode::from(2);
        };
        root = Some(PathBuf::from(dir));
        args.drain(1..3);
    }
    if args.len() == 1 || args.get(1).is_some_and(|a| a.to_string_lossy().starts_with('-')) {
        args.insert(1, "serve".into());
    }
    let cli = match dainty_hostkit::cli::Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) => error.exit(),
    };
    match dainty_hostkit::run_in(&cli, root.as_deref()) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("\nError: {error}");
            ExitCode::FAILURE
        }
    }
}
