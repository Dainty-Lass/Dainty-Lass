//! The `admin` binary: `admin serve --env-file <file>` runs the admin API; `serve` is the default.
//! `admin schema` prints the GraphQL schema (`schema.graphql`). `admin key issue|list|revoke` makes, lists
//! and ends the per-person keys (DH2).
//!
//! The command line, run by `reforged-host <service>` (RD26: one binary runs every service).

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use service_kit::env::Env;

#[derive(Parser)]
#[command(name = "admin", version, about = "DaintyLass admin API")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Serves the admin API on loopback.
    Serve {
        #[arg(long)]
        env_file: Option<PathBuf>,
    },
    /// Prints the GraphQL schema.
    Schema,
    /// Admin keys: one per person, each with a role (viewer, moderator, admin or owner).
    Key {
        #[command(subcommand)]
        command: KeyCommand,
    },
}

#[derive(Subcommand)]
pub enum KeyCommand {
    /// Makes a key and prints it once. It is not kept: only its hash is.
    Issue {
        /// Who the key is for (one active key per name)
        #[arg(long)]
        name: String,
        /// viewer, moderator, admin or owner
        #[arg(long)]
        role: String,
        #[arg(long)]
        env_file: Option<PathBuf>,
    },
    /// Lists every key with its role and who issued it. Never a key.
    List {
        #[arg(long)]
        env_file: Option<PathBuf>,
    },
    /// Ends a key by its id (from `key list`): its next request is refused.
    Revoke {
        id: String,
        #[arg(long)]
        env_file: Option<PathBuf>,
    },
}

fn fail(error: &str) -> ExitCode {
    crate::log::error(&format!("Configuration: {error}"), serde_json::json!({}));
    ExitCode::FAILURE
}

pub fn main_from<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let file = match Cli::parse_from(args)
        .command
        .unwrap_or(Command::Serve { env_file: None })
    {
        Command::Schema => {
            print!("{}", crate::schema::sdl());
            return ExitCode::SUCCESS;
        }
        Command::Key { command } => return crate::key_cli::run(command),
        Command::Serve { env_file } => env_file,
    };
    let env = match Env::load(file.as_deref()) {
        Ok(env) => env,
        Err(error) => return fail(&error.to_string()),
    };
    let config = match crate::config::Config::from_env(&env) {
        Ok(config) => config,
        Err(error) => return fail(&error),
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => return fail(&error.to_string()),
    };
    match runtime.block_on(crate::server::run(config)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => fail(&error),
    }
}
