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
pub mod repl;
pub mod selfupdate;
pub mod tui;
pub mod wine;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser as _;
use controller::{ServerConfig, ServerController};
pub use error::HostError;
use protocol::edition::EditionSelection;

/// What a command returns: its exit code, or the error `main` prints.
pub type Outcome = Result<ExitCode, HostError>;

/// Runs the parsed command line: finds the checkout, then the command.
pub fn run(cli: &cli::Cli) -> Outcome {
    let root = paths::locate_repo_root()?;
    commands::dispatch(&cli.command, &paths::Paths::new(root))
}

/// Entry point of the `server-console*` binaries. Every `reforged-host` command, `start` included,
/// goes to [`run`] unchanged (RD23: `start` is the Node kit's process contract). `serve`, or no
/// command, runs the in-process controller with the REPL, or the `ratatui` dashboard with `--tui`.
pub fn console_main(compiled_editions: EditionSelection) -> ExitCode {
    let raw_args: Vec<String> = std::env::args().collect();
    if let Some(first) = raw_args.get(1).map(String::as_str)
        && matches!(
            first,
            "setup"
                | "start"
                | "invite"
                | "doctor"
                | "gamedata"
                | "publish-runtime"
                | "preset"
                | "multipliers"
                | "update"
                | "backup"
                | "gameserver"
        )
    {
        let cli = cli::Cli::parse();
        return match run(&cli) {
            Ok(code) => code,
            Err(error) => {
                eprintln!("\nError: {error}");
                ExitCode::FAILURE
            }
        };
    }

    let mut config_path: Option<PathBuf> = None;
    let mut workspace_dir = PathBuf::from(".workspace");
    let mut use_tui = false;
    let mut log_filter = "info".to_string();
    let mut idx = 1;
    while idx < raw_args.len() {
        match raw_args[idx].as_str() {
            "serve" => {}
            "--tui" => use_tui = true,
            "--config" | "-c" => {
                idx += 1;
                if let Some(p) = raw_args.get(idx) {
                    config_path = Some(PathBuf::from(p));
                } else {
                    eprintln!("--config requires a file path");
                    return ExitCode::from(2);
                }
            }
            "--workspace" | "-w" => {
                idx += 1;
                if let Some(p) = raw_args.get(idx) {
                    workspace_dir = PathBuf::from(p);
                } else {
                    eprintln!("--workspace requires a directory path");
                    return ExitCode::from(2);
                }
            }
            "--log-level" => {
                idx += 1;
                if let Some(l) = raw_args.get(idx) {
                    log_filter.clone_from(l);
                }
            }
            "--help" | "-h" => {
                println!(
                    "Usage: [serve] [OPTIONS]\n\nOptions:\n  -c, --config <FILE>      Path to .toml, .yaml, .yml, or .json config\n  -w, --workspace <DIR>    Workspace directory (default: .workspace)\n      --tui                Launch interactive ratatui split-view dashboard\n      --log-level <FILTER> Tracing filter (default: info)\n\nreforged-host commands, unchanged:\n  setup, start, invite, doctor, gamedata, publish-runtime, preset, multipliers, update, backup, gameserver"
                );
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("Unknown option or command: {other}");
                return ExitCode::from(2);
            }
        }
        idx += 1;
    }

    let config = match ServerConfig::load_or_default(config_path.as_deref(), compiled_editions, workspace_dir) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };

    let log_file = config.workspace_dir.join("logs").join("server.log");
    let level =
        service_kit::telemetry::parse_level_filter(&log_filter).unwrap_or(tracing::level_filters::LevelFilter::INFO);
    let (tui_ring, reload_handle) = if use_tui {
        let (ring, handle) = service_kit::telemetry::init_tui_subscriber(level, 2000, Some(log_file));
        (Some(ring), Some(handle))
    } else {
        let handle = service_kit::telemetry::init_table_stdout_subscriber(level);
        (None, Some(handle))
    };

    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("failed to create tokio runtime: {e}");
            return ExitCode::FAILURE;
        }
    };

    rt.block_on(async move {
        let mut controller = match ServerController::new(config, compiled_editions) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(target: "host", "failed to initialize controller: {e}");
                return ExitCode::FAILURE;
            }
        };
        if let Err(e) = controller.start_all().await {
            tracing::error!(target: "host", "failed to start server components: {e}");
            controller.shutdown().await;
            return ExitCode::FAILURE;
        }
        tracing::info!(
            target: "host",
            invite = %controller.invite_url(),
            "server console ready (type `help` for REPL commands)"
        );
        if let Some(ring) = tui_ring {
            if let Err(e) = tui::run_tui(&mut controller, ring, reload_handle).await {
                tracing::error!(target: "host", "TUI error: {e}");
            }
        } else {
            repl::run_repl(&mut controller, reload_handle).await;
        }
        controller.shutdown().await;
        ExitCode::SUCCESS
    })
}
