//! Unified server controller crate (`docs/app-structure-plan.md`).

pub mod config;
pub mod controller;
pub mod provision;
pub mod yaml;

use std::path::PathBuf;
use std::process::ExitCode;

pub use config::{ConfigFormat, EditionConfig, EditionsMap, ServerConfig};
pub use controller::ServerController;
use protocol::edition::EditionSelection;
pub use provision::{
    ProvisionedWorkspace, ensure_database_and_preset, ensure_secrets, ensure_tls_certificate, provision_workspace,
};

/// Headless minimal-server CLI entry point shared by `server-minimal`, `server-minimal-refined`,
/// and `server-minimal-reforged`.
pub fn run_minimal_cli(compiled_editions: EditionSelection) -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut config_path: Option<PathBuf> = None;
    let mut workspace_dir = PathBuf::from(".workspace");
    let mut log_filter = "info".to_string();
    let mut idx = 0;
    while idx < args.len() {
        match args[idx].as_str() {
            "--config" | "-c" => {
                idx += 1;
                if let Some(p) = args.get(idx) {
                    config_path = Some(PathBuf::from(p));
                } else {
                    eprintln!("--config requires a file path (.toml, .yaml, .yml, or .json)");
                    return ExitCode::from(2);
                }
            }
            "--workspace" | "-w" => {
                idx += 1;
                if let Some(p) = args.get(idx) {
                    workspace_dir = PathBuf::from(p);
                } else {
                    eprintln!("--workspace requires a directory path");
                    return ExitCode::from(2);
                }
            }
            "--log-level" => {
                idx += 1;
                if let Some(lvl) = args.get(idx) {
                    log_filter.clone_from(lvl);
                }
            }
            "--help" | "-h" => {
                println!(
                    "Usage: [OPTIONS]\n  -c, --config <FILE>      Path to .toml, .yaml, .yml, or .json config\n  -w, --workspace <DIR>    Workspace directory (default: .workspace)\n      --log-level <FILTER> Tracing filter (default: info)"
                );
                return ExitCode::SUCCESS;
            }
            other => {
                eprintln!("Unknown argument: {other}");
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
    let level =
        service_kit::telemetry::parse_level_filter(&log_filter).unwrap_or(tracing::level_filters::LevelFilter::INFO);
    let _ = service_kit::telemetry::init_table_stdout_subscriber(level);

    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("failed to build tokio runtime: {e}");
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
            "all server components ready; waiting for shutdown signal"
        );
        service_kit::serve::stop_requested().await;
        tracing::info!(target: "host", "shutdown signal received; stopping components in reverse order");
        controller.shutdown().await;
        ExitCode::SUCCESS
    })
}
