// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `dainty-launcher play|export|servers` without the window: the kit's controller over [`HeadlessPlatform`],
//! run to the game's exit (`dainty_launcher_kit::controller::cli`).

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use dainty_launcher_kit::controller::cli::{CliRequest, HELD_FLAG, HeadlessPlatform, REFUSED_EXIT, STARTED};
use dainty_launcher_kit::controller::{Controller, ControllerConfig, Seams};
use dainty_launcher_kit::platform::invite_link::Holder;
use dainty_launcher_kit::platform::log;

/// Runs `request` with this config, holding the single-launcher channel while it does.
pub fn run(request: &CliRequest, config: ControllerConfig, holder: Option<Holder>) -> i32 {
    tauri::async_runtime::block_on(async move {
        if let Some(holder) = holder {
            tauri::async_runtime::spawn(refuse_second_launchers(holder));
        }
        let controller = Controller::new(config, std::sync::Arc::new(HeadlessPlatform), Seams::default());
        let mut out = std::io::stdout();
        let code = controller.run_cli(request, &mut out).await;
        controller.shutdown().await;
        code
    })
}

/// A launcher opened while a command line runs the game would be a second controller over the same settings,
/// key and relay port: it is turned away and told why in the log.
async fn refuse_second_launchers(mut holder: Holder) {
    loop {
        if holder.receive().await.is_ok() {
            log::warn("a launcher was opened while a command-line run holds the game; it was not opened");
        }
    }
}

/// A launch without `--wait`: the same command line in a detached child that holds the relay until the game
/// exits, its output passed through until the game is up.
pub fn detach(user_args: &[String]) -> i32 {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(error) => {
            println!("error: could not find the launcher's own executable: {error}");
            return REFUSED_EXIT;
        }
    };
    let mut command = Command::new(exe);
    command
        .args(user_args)
        .arg(HELD_FLAG)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // No console, as the window's launcher has none: the game opens its own (§2.7.16)
        const DETACHED_PROCESS: u32 = 0x8;
        command.creation_flags(DETACHED_PROCESS);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            println!("error: could not start the launcher in the background: {error}");
            return REFUSED_EXIT;
        }
    };
    let mut stdout = std::io::stdout();
    if let Some(lines) = child.stdout.take() {
        for line in BufReader::new(lines).lines() {
            let Ok(line) = line else { break };
            let _ = writeln!(stdout, "{line}");
            if line.starts_with(STARTED) {
                let _ = writeln!(
                    stdout,
                    "the launcher keeps the relay running in the background until the game exits"
                );
                return 0;
            }
        }
    }
    match child.wait() {
        Ok(status) => status.code().unwrap_or(REFUSED_EXIT),
        Err(_) => REFUSED_EXIT,
    }
}
