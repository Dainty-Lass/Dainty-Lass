// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The tests' game server (spec §13): no game and no SDK. It takes the real arguments, reports what it got, and
//! behaves by map; it never outlives its test.

use std::io::Write as _;
use std::net::UdpSocket;
use std::path::PathBuf;
use std::process::{Child, ExitCode, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Map, Value};

const OTHER_LAUNCH: &str = "00000000-0000-4000-8000-000000000000";

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

fn say(line: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

/// Exit 9 once the test's process is gone or the time is up.
fn watch(started: Instant) -> Option<ExitCode> {
    let max = env("FAKE_GS_MAX_SECS")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(900);
    if started.elapsed() >= Duration::from_secs(max) {
        return Some(ExitCode::from(9));
    }
    let parent = env("FAKE_GS_PARENT_PID").and_then(|v| v.parse::<u32>().ok());
    if parent.is_some_and(|pid| !rustic_isles_director::os::pid_alive(pid)) {
        return Some(ExitCode::from(9));
    }
    None
}

fn holder(started: Instant) -> ExitCode {
    let Ok(socket) = UdpSocket::bind("127.0.0.1:0") else {
        return ExitCode::from(4);
    };
    let port = socket.local_addr().map(|a| a.port()).unwrap_or(0);
    say(&format!("FAKE_GRANDCHILD_UDP={port}"));
    loop {
        if let Some(code) = watch(started) {
            return code;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// A copy of itself holding a UDP port, on the same stdout.
fn grandchild() -> Option<Child> {
    let exe = std::env::current_exe().ok()?;
    std::process::Command::new(exe)
        .env("FAKE_GS_HOLDER", "1")
        .stdin(Stdio::null())
        .spawn()
        .ok()
}

fn main() -> ExitCode {
    let started = Instant::now();
    if env("FAKE_GS_HOLDER").as_deref() == Some("1") {
        return holder(started);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(port), Some(map)) = (args.get(1), args.get(2)) else {
        eprintln!("usage: rustic-isles-fake-gameserver <key> <port> <map> [behemoth] [mmHunt] [players] [address] ...");
        return ExitCode::from(2);
    };
    let mut report = Map::new();
    report.insert("args".into(), Value::from(args.clone()));
    report.insert(
        "launchId".into(),
        Value::from(env("RUSTIC_ISLES_GAMESERVER_LAUNCH_ID").unwrap_or_default()),
    );
    report.insert(
        "trialsWeek".into(),
        env("RUSTIC_ISLES_TRIALS_WEEK").map_or(Value::Null, Value::from),
    );
    report.insert(
        "inherited".into(),
        env("REFORGED_SERVER_MAX_FPS").map_or(Value::Null, Value::from),
    );
    say(&format!("FAKE_ARGS {}", Value::Object(report)));
    if map == "/Game/Maps/crash" {
        eprintln!("LogWindows: fatal");
        return ExitCode::from(3);
    }
    let socket = match UdpSocket::bind(format!("127.0.0.1:{port}")) {
        Ok(socket) => socket,
        Err(e) => {
            say(&format!("Networking::Listen failed on {port}: {e}"));
            return ExitCode::from(4);
        }
    };
    let mut held = None;
    if map == "/Game/Maps/grandchild" {
        held = grandchild();
        std::thread::sleep(Duration::from_millis(500));
    }
    if map != "/Game/Maps/silent" {
        let launch = env("RUSTIC_ISLES_GAMESERVER_LAUNCH_ID").unwrap_or_default();
        say("LogNet: listening");
        say(&format!(
            "RUSTIC_ISLES_GAMESERVER_READY launchId={OTHER_LAUNCH} port={port}"
        ));
        say(&format!("RUSTIC_ISLES_GAMESERVER_READY launchId={launch} port={port}"));
    }
    if map == "/Game/Maps/chatty" {
        let line = "c".repeat(1023);
        let mut out = std::io::stdout().lock();
        for _ in 0..3072 {
            let _ = writeln!(out, "{line}");
        }
        let _ = writeln!(out, "CHATTY_DONE");
        let _ = out.flush();
    }
    if map == "/Game/Maps/flaky" {
        std::thread::sleep(Duration::from_millis(300));
        return ExitCode::from(5);
    }
    let stop = env("FAKE_GS_CONTROL").map(|dir| PathBuf::from(dir).join(format!("stop-{port}")));
    loop {
        if let Some(file) = stop.as_ref().filter(|f| f.exists()) {
            let _ = std::fs::remove_file(file);
            if let Some(mut child) = held.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
            drop(socket);
            return ExitCode::SUCCESS;
        }
        if let Some(code) = watch(started) {
            return code;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
