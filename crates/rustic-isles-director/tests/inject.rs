// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The runtime injected into every game server (RD9 as of 2026-11-13): what each start is asked to load, and that
//! a server whose runtime is refused is never left running.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{DIRECTOR_1_14_7, port_band, settings, write_tables};
use rustic_isles_director::runtime::{ENV_INJECT_DLL, ENV_INJECT_SHA256};
use rustic_isles_director::{Command, Config, Director, Launcher, MemoryAdmission, Started, hunt};
use serde_json::json;

const BOUND: Duration = Duration::from_secs(180);

async fn bounded(test: impl Future<Output = ()>) {
    tokio::time::timeout(BOUND, test)
        .await
        .expect("the test took longer than 180 s");
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// A published runtime folder holding `dll` as `Rustic-Isles.dll`, listed with `sha256`.
fn publish(dir: &Path, dll: &[u8], sha256: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("Rustic-Isles.dll"), dll).unwrap();
    let manifest = json!({
        "version": "20261113-00000000",
        "build": {"changelist": 647472},
        "files": [{"name": "Rustic-Isles.dll", "size": dll.len(), "sha256": sha256}],
    });
    std::fs::write(dir.join("runtime.json"), manifest.to_string()).unwrap();
}

/// What each start asked for: the program, the DLLs to inject and the wrapper's two variables.
type Asked = (String, Vec<(String, String)>, Vec<(String, String)>);

/// Records the start and refuses it, as an OS that could not start the program.
#[derive(Default)]
struct Recording(Mutex<Vec<Asked>>);

impl Launcher for Recording {
    fn start(&self, command: &Command) -> io::Result<Started> {
        let dlls = command
            .inject
            .iter()
            .map(|d| (d.path.to_string_lossy().into_owned(), d.sha256.clone()))
            .collect();
        let wrapper = command
            .env
            .iter()
            .filter(|(k, _)| k == ENV_INJECT_DLL || k == ENV_INJECT_SHA256)
            .cloned()
            .collect();
        self.0.lock().unwrap().push((command.program.clone(), dlls, wrapper));
        Err(io::Error::other("recorded"))
    }
}

async fn director(dir: &Path, extra: &[(&str, &str)], launcher: Arc<dyn Launcher>) -> Arc<Director> {
    let range = port_band();
    let mut pairs = vec![("RAMSGATE_HUB", "off"), ("TRAINING_HUB", "off")];
    pairs.extend_from_slice(extra);
    let settings = settings(dir, range, &pairs);
    let tables = dir.join("tables");
    write_tables(&tables);
    let config = Config::from_settings(&settings, &DIRECTOR_1_14_7, range, std::env::vars()).unwrap();
    let memory = MemoryAdmission::new(config.memory);
    let director = Director::new(config, hunt::load_tables(&tables).unwrap(), memory, launcher);
    director.startup().await;
    director
}

fn hunt_body() -> serde_json::Value {
    json!({"GameMode": "ISLAND", "GameArgs": "/Game/Maps/test/steady"})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn each_start_names_the_published_runtime() {
    bounded(async {
        let tmp = tempfile::tempdir().unwrap();
        let runtime = tmp.path().join("runtime");
        let sha = sha256_hex(b"runtime");
        publish(&runtime, b"runtime", &sha);
        let runtime_text = runtime.to_string_lossy().into_owned();
        let dll = runtime.join("Rustic-Isles.dll").to_string_lossy().into_owned();

        for program in [
            "C:\\Games\\Win64\\Dauntless-Win64-Shipping.exe",
            "/srv/.workspace/bin/dainty-gameserver",
        ] {
            let launcher = Arc::new(Recording::default());
            let extra = [
                ("GAMESERVER_BINARY_PATH", program),
                ("GAMESERVER_RUNTIME_DIR", runtime_text.as_str()),
            ];
            let d = director(tmp.path(), &extra, launcher.clone()).await;
            assert!(d.handle_matchmaking(Some(&hunt_body())).await.is_err());
            let asked = launcher.0.lock().unwrap().clone();
            assert_eq!(asked.len(), 1, "{program}");
            let (_, dlls, wrapper) = &asked[0];
            if cfg!(windows) && program.ends_with(".exe") {
                assert_eq!(dlls, &[(dll.clone(), sha.clone())]);
                assert!(wrapper.is_empty());
            } else {
                assert!(dlls.is_empty());
                let expected = [
                    (ENV_INJECT_DLL.to_owned(), dll.clone()),
                    (ENV_INJECT_SHA256.to_owned(), sha.clone()),
                ];
                assert_eq!(wrapper.as_slice(), expected.as_slice());
            }
        }
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_published_runtime_starts_nothing() {
    bounded(async {
        let tmp = tempfile::tempdir().unwrap();
        let runtime = tmp.path().join("runtime");
        std::fs::create_dir_all(&runtime).unwrap();
        let runtime_text = runtime.to_string_lossy().into_owned();
        let launcher = Arc::new(Recording::default());
        let d = director(
            tmp.path(),
            &[("GAMESERVER_RUNTIME_DIR", runtime_text.as_str())],
            launcher.clone(),
        )
        .await;
        let refused = d.handle_matchmaking(Some(&hunt_body())).await.unwrap_err();
        assert!(refused.contains("runtime.json"), "{refused}");
        assert!(launcher.0.lock().unwrap().is_empty());
        assert!(d.gameservers().is_empty());
    })
    .await;
}

/// The real thing on Windows: a system DLL stands in for the runtime and is loaded into the fake server, which then
/// reaches ready; with the wrong hash the server is ended before it ever runs.
#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_runtime_is_loaded_before_the_server_runs() {
    use rustic_isles_director::OsLauncher;

    bounded(async {
        let system = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_owned());
        let stand_in = std::fs::read(Path::new(&system).join("System32").join("dbghelp.dll")).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let runtime = tmp.path().join("runtime");
        let runtime_text = runtime.to_string_lossy().into_owned();
        let extra = [("GAMESERVER_RUNTIME_DIR", runtime_text.as_str())];

        publish(&runtime, &stand_in, &"0".repeat(64));
        let d = director(tmp.path(), &extra, Arc::new(OsLauncher)).await;
        let refused = d.handle_matchmaking(Some(&hunt_body())).await.unwrap_err();
        assert!(refused.contains("not the file that was checked"), "{refused}");
        assert!(d.gameservers().is_empty());

        publish(&runtime, &stand_in, &sha256_hex(&stand_in));
        let d = director(tmp.path(), &extra, Arc::new(OsLauncher)).await;
        let started = d.handle_matchmaking(Some(&hunt_body())).await;
        assert!(started.is_ok(), "{started:?}");
        d.shutdown().await;
    })
    .await;
}
