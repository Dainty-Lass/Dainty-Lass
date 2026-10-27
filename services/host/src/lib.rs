//! The host kit, `reforged-host` (MIGRATION_PLAN.md RD3, section 4.4): the Node kit in
//! `legacy/ReforgedHost` as one Rust binary, with the same commands and options and the same files
//! (the `.env` files; `host.json` became `host.toml`, read from a Node kit's host.json until the
//! first write), so an existing host moves over with `update`.
//!
//! RH1 is the skeleton: the command line ([`cli`]), the layout ([`paths`]), the env files
//! ([`envfile`]) and the settings with the service env files built from it ([`hostconfig`]) are
//! real. Each command is a module in [`commands`]; the other modules follow the Node kit's files and
//! name the row that fills them. Shared helpers one row writes and another calls are declared here
//! with their signatures, so the wave 2 rows (RH2-RH5) each fill only their own files.

pub mod cli;
pub mod commands;
pub mod envfile;
pub mod error;
pub mod hostconfig;
pub mod hosttoml;
pub mod paths;

// RH2
pub mod run;
pub mod services;
// RH3
pub mod doctor;
pub mod invite;
pub mod mongo;
// RH4
pub mod captures;
pub mod gamedata;
pub mod gamepack;
pub mod preset;
pub mod progression;
pub mod publish;
// Dauntless Revived's 1.4.4 servers (RD5 reversed); without the feature, refusals with the same signatures
#[cfg(feature = "legacy")]
pub mod legacy;
#[cfg(not(feature = "legacy"))]
#[path = "legacy/off.rs"]
pub mod legacy;
// RH5
pub mod linux;
pub mod selfupdate;
pub mod wine;
// The server as one process (RD26): the controller's modules, the console and the dashboard
pub mod repl;
pub mod serve;
pub mod tui;

use std::process::ExitCode;

use clap::Parser as _;
pub use error::HostError;

/// What a command returns: its exit code, or the error `main` prints.
pub type Outcome = Result<ExitCode, HostError>;

/// Runs the parsed command line: finds the checkout, then the command. A service run alone needs
/// no checkout (a container holds the binary and the workspace only).
pub fn run(cli: &cli::Cli) -> Outcome {
    use cli::Command;
    if let Command::Gateway { args }
    | Command::Allowlist { args }
    | Command::Content { args }
    | Command::Admin { args } = &cli.command
    {
        return Ok(service_main(
            cli.command.name(),
            &with_admin_env(cli.command.name(), args),
        ));
    }
    let root = paths::locate_repo_root()?;
    commands::dispatch(&cli.command, &paths::Paths::new(root))
}

/// `admin key issue|list|revoke` without `--env-file` reads this checkout's admin.env (DH2); a container with
/// no checkout names the file itself.
fn with_admin_env(name: &str, args: &[String]) -> Vec<String> {
    let mut args = args.to_vec();
    let named = args.iter().any(|a| a == "--env-file" || a.starts_with("--env-file="));
    if name == "admin"
        && args.first().is_some_and(|a| a == "key")
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

/// Which services this binary runs on its own (`reforged-host <service>`).
pub const BUILT_IN_SERVICES: &[&str] = &[
    #[cfg(feature = "gateway")]
    "gateway",
    #[cfg(feature = "allowlist")]
    "allowlist",
    #[cfg(feature = "content")]
    "content",
    #[cfg(feature = "admin")]
    "admin",
];

/// One service with the command line its own binary had: `name` is its argv[0].
pub fn service_main(name: &str, args: &[String]) -> ExitCode {
    let argv = std::iter::once(name.to_owned()).chain(args.iter().cloned());
    match name {
        #[cfg(feature = "gateway")]
        "gateway" => gateway::cli::main_from(argv),
        #[cfg(feature = "allowlist")]
        "allowlist" => gateway::allowlist::cli::main_from(argv),
        #[cfg(feature = "content")]
        "content" => content::cli::main_from(argv),
        #[cfg(feature = "admin")]
        "admin" => admin::cli::main_from(argv),
        _ => {
            drop(argv);
            eprintln!(
                "\nError: this binary was built without the {name} service (built in: {})",
                BUILT_IN_SERVICES.join(", ")
            );
            ExitCode::FAILURE
        }
    }
}

/// `reforged-host`: the command line as the Node kit had it (help when no command is given).
pub fn host_main() -> ExitCode {
    let cli = cli::Cli::parse();
    match run(&cli) {
        Ok(code) => code,
        Err(error) => {
            // As the Node kit's fail().
            eprint!("\nError: {error}\n");
            ExitCode::FAILURE
        }
    }
}

/// Entry point of the `server-console*` binaries: `reforged-host`'s commands, with `serve` when no
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
