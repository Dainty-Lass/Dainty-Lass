//! The `content` binary: `content serve [--env-file <file>]` runs the server (`npm start`); `serve` is
//! the default. Without `--env-file`, a `.env` in the working folder is read if there is one, as
//! `node --env-file-if-exists=.env` did. A legacy build also has DR's `content verify`.
//!
//! The command line, run by `reforged-host <service>` (RD26: one binary runs every service).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use service_kit::env::Env;

#[derive(Parser)]
#[command(name = "content", version, about = "Dauntless Reforged Revival content server")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Serves the game runtime, news and art pack.
    Serve {
        #[arg(long)]
        env_file: Option<PathBuf>,
    },
    /// Hashes a legacy server's 1.4.4 game folder against the manifest.
    #[cfg(feature = "legacy")]
    Verify {
        #[arg(long)]
        env_file: Option<PathBuf>,
        /// Sizes only.
        #[arg(long)]
        quick: bool,
        #[arg(long)]
        game_dir: Option<PathBuf>,
        #[arg(long)]
        manifest: Option<PathBuf>,
    },
}

fn env_file(given: Option<PathBuf>) -> Option<PathBuf> {
    given.or_else(|| Some(PathBuf::from(".env")).filter(|p| p.is_file()))
}

pub fn main_from<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let cli = Cli::parse_from(args);
    match cli.command.unwrap_or(Command::Serve { env_file: None }) {
        Command::Serve { env_file: file } => serve(env_file(file)),
        #[cfg(feature = "legacy")]
        Command::Verify {
            env_file: file,
            quick,
            game_dir,
            manifest,
        } => {
            let env = match Env::load(env_file(file).as_deref()) {
                Ok(env) => env,
                Err(error) => {
                    eprintln!("{error}");
                    return ExitCode::from(2);
                }
            };
            ExitCode::from(crate::legacy::verify::run(&env, game_dir, manifest, quick) as u8)
        }
    }
}

fn serve(env_file: Option<PathBuf>) -> ExitCode {
    let env = match Env::load(env_file.as_deref()) {
        Ok(env) => env,
        Err(error) => {
            crate::log::error(&format!("Configuration: {error}"), serde_json::json!({}));
            return ExitCode::FAILURE;
        }
    };
    let config = match crate::config::Config::from_env(&env) {
        Ok(config) => config,
        Err(error) => {
            crate::log::error(&format!("Configuration: {error}"), serde_json::json!({}));
            return ExitCode::FAILURE;
        }
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            crate::log::error(&error.to_string(), serde_json::json!({}));
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(crate::server::run(config)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            crate::log::error(&error, serde_json::json!({}));
            ExitCode::FAILURE
        }
    }
}
