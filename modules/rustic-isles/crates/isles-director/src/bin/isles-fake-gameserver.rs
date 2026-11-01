//! A stand-in for the game server, for the Director's tests (RU6): no game and no SDK. It takes the real
//! launch arguments, listens on its UDP port, prints the runtime's readiness marker and runs until it
//! is stopped (a signal, or the end of its process tree) or `stop-<port>` appears in `FAKE_GS_CONTROL`.
//!
//! What it does depends on its map (the second positional argument after the port), so a test picks it in
//! the travel arguments:
//! - `/Game/Maps/crash`: exits with code 3 at once
//! - `/Game/Maps/silent`: listens but never reports ready
//! - `/Game/Maps/grandchild`: starts a child process of its own that holds a second UDP port (printed as
//!   `FAKE_GRANDCHILD_UDP=<port>`), to see the whole tree ended
//! - `/Game/Maps/chatty`: writes 3 MB after it is ready, to see the pipes keep draining
//! - `/Game/Maps/flaky`: reports ready, then exits with code 5 after 300 ms
//! - anything else: ready and running
//!
//! It never outlives its test: it exits when the process named by `FAKE_GS_PARENT_PID` is gone (a killed or
//! panicked test run), and in any case after `FAKE_GS_MAX_SECS` (15 minutes).
#![allow(clippy::unwrap_used, clippy::expect_used, unsafe_code)] // a test helper; unsafe for the parent watch

use std::io::Write;
use std::net::UdpSocket;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};
use std::time::Duration;

const HOLDER: &str = "FAKE_GS_HOLDER";

/// Whether the process `pid` still runs.
#[cfg(windows)]
fn alive(pid: u32) -> bool {
    use windows::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};
    // SAFETY: a fresh handle, waited on without blocking and closed.
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) else {
            return false;
        };
        let running = WaitForSingleObject(handle, 0) == WAIT_TIMEOUT;
        let _ = CloseHandle(handle);
        running
    }
}

#[cfg(unix)]
fn alive(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // SAFETY: signal 0 only checks that the process exists.
    unsafe { libc::kill(pid, 0) == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM) }
}

fn watch_parent() {
    let parent = std::env::var("FAKE_GS_PARENT_PID")
        .ok()
        .and_then(|p| p.parse::<u32>().ok());
    let limit = std::env::var("FAKE_GS_MAX_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(900);
    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        while started.elapsed() < Duration::from_secs(limit) && parent.is_none_or(alive) {
            std::thread::sleep(Duration::from_millis(250));
        }
        std::process::exit(9);
    });
}

fn main() -> ExitCode {
    watch_parent();
    if std::env::var_os(HOLDER).is_some() {
        let socket = UdpSocket::bind("127.0.0.1:0").unwrap();
        println!("FAKE_GRANDCHILD_UDP={}", socket.local_addr().unwrap().port());
        std::io::stdout().flush().unwrap();
        loop {
            std::thread::sleep(Duration::from_secs(1));
        }
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(port), Some(map)) = (args.get(1), args.get(2)) else {
        eprintln!("usage: <key> <port> <map> <behemoth> <mm hunt> <players> <address> [switches]");
        return ExitCode::from(2);
    };
    let launch = std::env::var("RUSTIC_ISLES_GAMESERVER_LAUNCH_ID").unwrap_or_default();
    println!(
        "FAKE_ARGS {}",
        serde_json::json!({
            "args": args,
            "launchId": launch,
            "trialsWeek": std::env::var("RUSTIC_ISLES_TRIALS_WEEK").ok(),
            "inherited": std::env::var("REFORGED_SERVER_MAX_FPS").ok(),
        })
    );
    if map == "/Game/Maps/crash" {
        eprintln!("LogWindows: fatal");
        return ExitCode::from(3);
    }
    let _socket = match UdpSocket::bind(format!("127.0.0.1:{port}")) {
        Ok(socket) => socket,
        Err(error) => {
            eprintln!("Networking::Listen failed on {port}: {error}");
            return ExitCode::from(4);
        }
    };
    let mut grandchild = None;
    if map == "/Game/Maps/grandchild" {
        grandchild = Some(
            Command::new(std::env::current_exe().unwrap())
                .env(HOLDER, "1")
                .stdin(Stdio::null())
                .spawn()
                .unwrap(),
        );
        // Its port is printed through the shared pipe; give it a moment before the marker
        std::thread::sleep(Duration::from_millis(500));
    }
    if map != "/Game/Maps/silent" {
        println!("LogNet: listening");
        // Another launch's marker is ignored
        println!("RUSTIC_ISLES_GAMESERVER_READY launchId=00000000-0000-4000-8000-000000000000 port={port}");
        println!("RUSTIC_ISLES_GAMESERVER_READY launchId={launch} port={port}");
        std::io::stdout().flush().unwrap();
    }
    if map == "/Game/Maps/chatty" {
        let line = format!("{:-<1023}\n", "LogNet: chatter ");
        let mut stdout = std::io::stdout().lock();
        for _ in 0..3072 {
            stdout.write_all(line.as_bytes()).unwrap();
        }
        stdout.flush().unwrap();
        drop(stdout);
        println!("CHATTY_DONE");
        std::io::stdout().flush().unwrap();
    }
    let stop = std::env::var_os("FAKE_GS_CONTROL").map(|dir| PathBuf::from(dir).join(format!("stop-{port}")));
    let flaky = map == "/Game/Maps/flaky";
    let mut waited = 0;
    loop {
        if let Some(stop) = &stop
            && stop.exists()
        {
            let _ = std::fs::remove_file(stop);
            drop(grandchild.take());
            return ExitCode::SUCCESS;
        }
        if flaky && waited >= 300 {
            return ExitCode::from(5);
        }
        std::thread::sleep(Duration::from_millis(50));
        waited += 50;
    }
}
