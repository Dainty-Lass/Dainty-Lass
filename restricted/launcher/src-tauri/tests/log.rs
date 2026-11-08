//! dainty-launcher-kit's log (spec §2.7.4, row L29), as the shell's `platform/log.rs` tested it: DR's recorded table
//! (`test/cases/log.json`) and a registered key kept from the file.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use std::sync::Mutex;

use dainty_launcher_kit::platform::log as kit;
use serde::Deserialize;
use tracing_subscriber::layer::SubscriberExt as _;

/// The secrets and the folder are the process's.
static GLOBAL: Mutex<()> = Mutex::new(());

#[derive(Deserialize)]
struct Cases {
    secrets: Vec<String>,
    removed: String,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    line: String,
    redacted: String,
}

#[test]
fn the_shared_table() {
    let _global = GLOBAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let cases: Cases = serde_json::from_value(common::cases("log.json")).unwrap();
    for secret in &cases.secrets {
        kit::register_secret(secret);
    }
    let (last, before) = cases.cases.split_last().unwrap();
    for case in before {
        assert_eq!(kit::redact(&case.line), case.redacted, "{:?}", case.line);
    }
    kit::unregister_secret(&cases.removed);
    assert_eq!(kit::redact(&last.line), last.redacted);
}

#[test]
fn a_registered_key_never_reaches_the_file() {
    let _global = GLOBAL.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = tempfile::tempdir().unwrap();
    kit::set_folder(dir.path());
    let key = concat!("RRK_", "fedcba9876543210", "fedcba9876543210", "fedcba9876543210");
    kit::register_secret(key);
    kit::info(format!("launching with -AUTH_PASSWORD={key} and {}", &key[4..]));
    // The subscriber dainty-launcher-kit's `install` sets, here for this thread only
    let subscriber = tracing_subscriber::registry().with(kit::LogFilter).with(kit::LogLayer);
    tracing::subscriber::with_default(subscriber, || {
        tracing::warn!("library saw {}", &key[4..]);
        tracing::error!(target: "egs_api::api", "request to https://x/?sid=egs-api-secret failed");
    });
    let text = std::fs::read_to_string(dir.path().join(kit::LOG_FILE)).unwrap();
    assert!(!text.contains(&key[4..]), "{text}");
    assert!(!text.contains("egs-api-secret"), "{text}");
    assert!(text.contains(" INFO launching with -AUTH_PASSWORD=<hidden>"), "{text}");
    assert!(text.contains(" WARN "), "{text}");
    // Later lines go to a folder of the test process's own
    kit::set_folder(&std::env::temp_dir().join(format!("launcher-tests-{}", std::process::id())));
}
