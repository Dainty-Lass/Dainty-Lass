// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `content [serve] [--env-file <path>]`, the hidden `dainty-host content` subcommand.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use serde_json::json;
use service_kit::env::Env;

use crate::config::Config;
use crate::{log, server};

#[derive(Parser)]
#[command(name = "content", version, about = "DaintyLass content server")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// The env file setup wrote; the process environment wins over it
    #[arg(long, global = true)]
    env_file: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = "Serves the game runtime, news and art pack.")]
    Serve,
}

pub fn main_from<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) => {
            _ = error.print();
            return if error.use_stderr() {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            };
        }
    };
    let (Some(Command::Serve) | None) = cli.command;
    // As `node --env-file-if-exists=.env` (spec section 5, item 5: the host always passes --env-file)
    let env_file = cli
        .env_file
        .or_else(|| Path::new(".env").is_file().then(|| PathBuf::from(".env")));
    let config = match Env::load(env_file.as_deref())
        .map_err(|e| e.to_string())
        .and_then(|env| Config::from_env(&env))
    {
        Ok(config) => config,
        Err(error) => {
            log::error(&format!("Configuration: {error}"), json!({}));
            return ExitCode::FAILURE;
        }
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            log::error(&error.to_string(), json!({}));
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(server::run(config)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            log::error(&error, json!({}));
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use clap::CommandFactory as _;

    #[test]
    fn the_command_line() {
        Cli::command().debug_assert();
        assert!(Cli::try_parse_from(["content"]).is_ok());
        let cli = Cli::try_parse_from(["content", "serve", "--env-file", "x.env"]).unwrap();
        assert_eq!(cli.env_file.as_deref(), Some(Path::new("x.env")));
        assert!(Cli::try_parse_from(["content", "--env-file", "x.env", "serve"]).is_ok());
        assert!(Cli::try_parse_from(["content", "other"]).is_err());
        let help = Cli::command().render_long_help().to_string();
        assert!(
            help.contains("DaintyLass content server") && help.contains("Serves the game runtime, news and art pack.")
        );
    }

    #[test]
    fn a_bad_setting_fails_the_start() {
        let dir = tempfile::tempdir().unwrap();
        let env = dir.path().join("content.env");
        std::fs::write(&env, "PORT=0\n").unwrap();
        let code = main_from([
            "content".into(),
            "serve".into(),
            "--env-file".into(),
            env.into_os_string(),
        ]);
        assert_eq!(code, ExitCode::FAILURE);
    }
}
