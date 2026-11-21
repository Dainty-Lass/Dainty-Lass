// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! A stand-in for `dainty-host` in dainty-launcher-kit's tests (RD27 spec §2.10). A first argument `fail` makes `start`
//! fail and `deaf` makes it ignore the stop line; `invite` answers one JSON line; anything else fails.

use std::io::{BufRead, Write};
use std::process::ExitCode;
use std::time::Duration;

/// The link `invite` answers; tests look for it where it must not be.
const INVITE_LINK: &str = "dauntless://fake-host-kit-invite-secret";

/// How `start` misbehaves, picked by an optional leading argument.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    Fail,
    Deaf,
}

fn main() -> ExitCode {
    let all: Vec<String> = std::env::args().skip(1).collect();
    let (mode, command) = match all.split_first() {
        Some((first, rest)) if first == "fail" => (Mode::Fail, rest),
        Some((first, rest)) if first == "deaf" => (Mode::Deaf, rest),
        _ => (Mode::Normal, all.as_slice()),
    };
    if let [verb, rest @ ..] = command {
        if verb == "start" {
            return start(mode);
        }
        if verb == "invite" {
            return invite(rest);
        }
    }
    println!("Error: the fake host kit has no `{}`", command.join(" "));
    ExitCode::FAILURE
}

/// The host kit's control contract: the ready line when stdin controls it, then a `stop` line (or EOF) ends it.
fn start(mode: Mode) -> ExitCode {
    println!("Starting the services...");
    if mode == Mode::Fail {
        println!("Error: the backend did not start");
        return ExitCode::FAILURE;
    }
    let controlled = std::env::var("DAINTY_CONTROL").is_ok_and(|value| value == "stdin");
    println!("Every service is up.");
    if controlled {
        println!("DAINTY_SERVER_READY");
    }
    let _ = std::io::stdout().flush();
    if mode == Mode::Deaf {
        loop {
            std::thread::sleep(Duration::from_secs(1));
        }
    }
    for line in std::io::stdin().lock().lines() {
        match line {
            Ok(line) if line.trim().eq_ignore_ascii_case("stop") => break,
            Ok(_) => {}
            Err(_) => break,
        }
    }
    println!("Stopping...");
    println!("Stopped.");
    ExitCode::SUCCESS
}

fn invite(rest: &[String]) -> ExitCode {
    let for_name = rest
        .windows(2)
        .find(|pair| pair[0] == "--for")
        .map(|pair| pair[1].clone());
    println!("Creating an invite");
    println!("{}", serde_json::json!({ "link": INVITE_LINK, "for": for_name }));
    ExitCode::SUCCESS
}
