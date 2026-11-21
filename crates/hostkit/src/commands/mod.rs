// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

pub mod backup;
pub mod db;
pub mod doctor;
pub mod gamedata;
pub mod history;
pub mod install_game;
pub mod invite;
pub mod legacy;
pub mod multipliers;
pub mod network;
pub mod preset;
pub mod publish_runtime;
pub mod remote;
pub mod setup;
pub mod start;
pub mod update;

use std::process::ExitCode;

use crate::Outcome;
use crate::cli::Command;
use crate::paths::Paths;

pub fn dispatch(command: &Command, paths: &Paths) -> Outcome {
    match command {
        Command::Setup(args) => Ok(setup::run(paths, args)),
        Command::Start => start::run(paths),
        Command::Serve(args) => crate::serve::run(
            paths,
            &crate::serve::ServeOptions {
                config_file: args.config.clone(),
                interactive: true,
                tui: args.tui,
                log_filter: args.log_level.clone(),
            },
        ),
        Command::Invite(args) => invite::run(paths, args),
        Command::Doctor(args) => Ok(doctor::run(paths, args)),
        Command::Gamedata(args) => gamedata::run(paths, args),
        Command::PublishRuntime(args) => publish_runtime::run(paths, args),
        Command::InstallGame(args) => install_game::run(paths, args),
        Command::Preset(args) => preset::run(paths, args),
        Command::Multipliers(args) => multipliers::run(paths, args),
        Command::Update(args) => Ok(update::run(paths, args)),
        Command::Backup => backup::run(paths),
        Command::Db(args) => db::run(paths, args),
        Command::Network(args) => network::run(args, Some(paths)),
        Command::Remote(args) => remote::run(paths, args),
        Command::History(args) => history::run_history(paths, args),
        Command::Revert(args) => history::run_revert(paths, args),
        Command::Legacy(args) => legacy::run(paths, args),
        Command::Gameserver(args) => {
            let code = crate::wine::run_game_server(&args.args)?;
            Ok(ExitCode::from(code.clamp(0, 255) as u8))
        }
        Command::Gateway { args }
        | Command::Allowlist { args }
        | Command::Content { args }
        | Command::Admin { args }
        | Command::Backend { args }
        | Command::Director { args }
        | Command::RemoteAgent { args } => {
            let name = command.name();
            let full_args = crate::with_admin_env(name, args);
            Ok(crate::service_main(name, &full_args))
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::cli::DoctorArgs;
    use tempfile::TempDir;

    #[test]
    fn dispatch_routes_doctor_command() {
        let tmp = TempDir::new().unwrap();
        let paths = Paths::new(tmp.path());
        let cmd = Command::Doctor(DoctorArgs { no_hash: true });
        let res = dispatch(&cmd, &paths).unwrap();
        assert_eq!(res, ExitCode::FAILURE);
    }
}
