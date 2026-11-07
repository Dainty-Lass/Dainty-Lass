// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `allowlist [--env-file <file>] [--close-ports]`.

use std::process::ExitCode;

use dainty_service_kit::context::ServiceContext;
use dainty_service_kit::env::Env;
use dainty_service_kit::linelog;
use serde_json::json;

use super::config::AllowlistConfig;

fn fatal(message: &str) -> ExitCode {
    linelog::error(message, json!({}));
    ExitCode::from(1)
}

/// The helper's command line; `args` are the arguments after the program's (or subcommand's) name.
pub fn main_from(args: Vec<String>) -> ExitCode {
    let (file, flags) = match crate::cli::env_file(&args, &["--close-ports"]) {
        Ok(found) => found,
        Err(e) => return fatal(&format!("Configuration: {e}")),
    };
    let env = match Env::load(file.as_deref()) {
        Ok(env) => env,
        Err(e) => return fatal(&format!("Configuration: {e}")),
    };
    let config = match AllowlistConfig::from_env(&env) {
        Ok(config) => config,
        Err(e) => return fatal(&format!("Configuration: {e}")),
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(e) => return fatal(&format!("Could not start the runtime: {e}")),
    };
    let close_ports = flags.iter().any(|f| f == "--close-ports");
    runtime.block_on(async {
        let result = if close_ports {
            super::helper::close_ports_once(&config).await
        } else {
            super::helper::run_with_context(config, ServiceContext::standalone("allowlist")).await
        };
        match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fatal(&e),
        }
    })
}
