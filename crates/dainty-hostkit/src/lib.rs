// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The host kit (`dainty-host`): CLI, configuration (`host.toml` and `.env` files), setup,
//! game data generation, process supervision, diagnostics, and interactive server console/TUI.

pub mod captures;
pub mod cli;
pub mod commands;
pub mod doctor;
pub mod envfile;
pub mod epicinstall;
pub mod error;
pub mod gamedata;
pub mod gamepack;
pub mod guarded;
pub mod hooks;
pub mod hostconfig;
pub mod hostfiles;
pub mod hosttoml;
pub mod invite;
pub mod linux;
pub mod mongo;
pub mod paths;
pub mod preset;
pub mod progression;
pub mod publish;
pub mod repl;
pub mod run;
pub mod selfupdate;
pub mod serve;
pub mod services;
pub mod tls;
pub mod tui;
pub mod wine;

use std::process::ExitCode;

use clap::Parser as _;
pub use error::HostError;

/// What a command returns: its exit code, or the error `main` prints.
pub type Outcome = Result<ExitCode, HostError>;

/// Default front services considered built-in unless overridden by [`hooks::HostHooks::built_in_services`].
pub const BUILT_IN_SERVICES: &[&str] = &[
    "gateway",
    "allowlist",
    "content",
    #[cfg(feature = "admin")]
    "admin",
];

/// Runs the parsed command line: finds the checkout, then the command. A service run alone needs
/// no checkout (a container holds the binary and the workspace only).
pub fn run(cli: &cli::Cli) -> Outcome {
    run_in(cli, None)
}

/// [`run()`] in the server folder `root` (the one that holds `.workspace/`), or the one found from here.
pub fn run_in(cli: &cli::Cli, root: Option<&std::path::Path>) -> Outcome {
    use cli::Command;
    let found = || root.map_or_else(paths::locate_repo_root, |r| Ok(r.to_path_buf()));
    if let Command::Gateway { args }
    | Command::Allowlist { args }
    | Command::Content { args }
    | Command::Admin { args }
    | Command::Backend { args }
    | Command::Director { args } = &cli.command
    {
        return Ok(service_main(
            cli.command.name(),
            &with_admin_env(cli.command.name(), args),
        ));
    }
    // The CA and node certificates are made on the owner's machine, in or out of a server's folder
    if let Command::Network(args) = &cli.command {
        let paths = found().ok().map(paths::Paths::new);
        return commands::network::run(args, paths.as_ref());
    }
    let root = found()?;
    commands::dispatch(&cli.command, &paths::Paths::new(root))
}

/// `admin key ...` and `admin guarded ...` without `--env-file` read this checkout's `admin.env` (DH2, DH6); a
/// container with no checkout names the file itself.
pub fn with_admin_env(name: &str, args: &[String]) -> Vec<String> {
    let mut args = args.to_vec();
    let named = args.iter().any(|a| a == "--env-file" || a.starts_with("--env-file="));
    if name == "admin"
        && args.first().is_some_and(|a| a == "key" || a == "guarded")
        && !named
        && let Ok(root) = paths::locate_repo_root()
    {
        let file = paths::Paths::new(root).config.join("admin.env");
        if file.is_file() {
            args.extend(["--env-file".to_owned(), file.display().to_string()]);
        }
    }
    args
}

/// One service with the command line its own binary had: `name` is its `argv[0]`.
pub fn service_main(name: &str, args: &[String]) -> ExitCode {
    if let Some(code) = hooks::get().run_service(name, args) {
        return code;
    }
    let argv = std::iter::once(name.to_owned()).chain(args.iter().cloned());
    match name {
        #[cfg(feature = "admin")]
        "admin" => dainty_admin::cli::main_from(argv),
        _ => {
            drop(argv);
            eprintln!(
                "\nError: this binary was built without the {name} service (built in: {})",
                hooks::get().built_in_services().join(", ")
            );
            ExitCode::FAILURE
        }
    }
}

/// `dainty-host`: the command line (help when no command is given).
pub fn host_main() -> ExitCode {
    let cli = cli::Cli::parse();
    match run(&cli) {
        Ok(code) => code,
        Err(error) => {
            eprint!("\nError: {error}\n");
            ExitCode::FAILURE
        }
    }
}

/// Entry point of the `server-console*` binaries: `dainty-host`'s commands, with `serve` when no
/// command is given (a bare console serves).
pub fn console_main() -> ExitCode {
    let mut args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    if args.len() == 1 || args.get(1).is_some_and(|a| a.to_string_lossy().starts_with('-')) {
        args.insert(1, "serve".into());
    }
    let cli = cli::Cli::parse_from(args);
    match run(&cli) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("\nError: {error}");
            ExitCode::FAILURE
        }
    }
}
