// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! A stand-in for `dainty-host` in launcher-kit's tests (RD27 spec §2.10). A first argument `fail` makes `start`
//! fail and `deaf` makes it ignore the stop line; `invite` answers one JSON line; anything else fails.

use std::io::{BufRead, Write};
use std::process::ExitCode;
use std::time::Duration;

/// The link `invite` answers; tests look for it where it must not be.
const INVITE_LINK: &str = "dauntless://fake-host-kit-invite-secret";

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mode = match args.first().map(String::as_str) {
        Some("fail" | "deaf") => Some(args.remove(0)),
        _ => None,
    };
    match args.first().map(String::as_str) {
        Some("start") => start(mode.as_deref()),
        Some("invite") => invite(&args[1..]),
        _ => {
            println!("Error: the fake host kit has no `{}`", args.join(" "));
            ExitCode::FAILURE
        }
    }
}

/// The host kit's control contract: the ready line when stdin controls it, then a `stop` line (or EOF) ends it.
fn start(mode: Option<&str>) -> ExitCode {
    println!("Starting the services...");
    if mode == Some("fail") {
        println!("Error: the backend did not start");
        return ExitCode::FAILURE;
    }
    let controlled = std::env::var("REFORGED_CONTROL").is_ok_and(|value| value == "stdin");
    println!("Every service is up.");
    if controlled {
        println!("REFORGED_SERVER_READY");
    }
    let _ = std::io::stdout().flush();
    if mode == Some("deaf") {
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
