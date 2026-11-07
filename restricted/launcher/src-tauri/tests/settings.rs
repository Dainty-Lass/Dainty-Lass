//! launcher-kit's settings file (spec §2.7.2, row L28), as the shell's `platform/settings.rs` tested it: DR's
//! recorded table (`test/cases/settings.json`) byte for byte, and the shell's own cases.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use launcher_kit::controller::stored::Stored;
use launcher_kit::core::i18n::Language;
use launcher_kit::platform::settings::{self as kit, Os, SettingsStore};
use serde::Deserialize;
use serde_json::{Map, Value, json};

/// `game` of a Dauntless Revived server (Dauntless 1.4.4), which 2.0.0 no longer plays (RD5)
const REVIVED: &str = "revived";

fn is_revived(server: &Map<String, Value>) -> bool {
    server.get("game").and_then(Value::as_str) == Some(REVIVED)
}

fn settings_text(settings: &Map<String, Value>) -> String {
    String::from_utf8(kit::to_bytes(settings)).unwrap()
}

#[derive(Deserialize)]
struct Case {
    platform: String,
    #[serde(default)]
    input: Value,
    language: String,
    text: String,
}

/// `test/cases/settings.json`, made by Reforged's `sanitizeSettings` (the Node test `platform-tables.test.ts`
/// checks it there) less the 1.4.4 parts this launcher drops.
#[test]
fn the_shared_table_byte_for_byte() {
    let cases: Vec<Case> = serde_json::from_value(common::cases("settings.json")).unwrap();
    assert!(cases.len() > 100);
    let mut run = 0;
    for case in cases {
        let os = if case.platform == "win32" {
            Os::Windows
        } else {
            Os::Linux
        };
        let language = Language::from_code(&case.language).unwrap();
        let settings = kit::sanitize(&case.input, language, os);
        assert_eq!(settings_text(&settings), case.text, "{} {}", case.platform, case.input);
        run += 1;
    }
    assert!(run > 100);
}

#[test]
fn the_tray_settings_are_kept_only_when_on() {
    let on = kit::sanitize(
        &json!({ "minimizeToTray": true, "closeToTray": "yes" }),
        Language::En,
        Os::Windows,
    );
    assert_eq!(on.get("minimizeToTray"), Some(&Value::Bool(true)));
    assert_eq!(on.get("closeToTray"), None);
    let stored = Stored(on.clone());
    assert!(stored.minimize_to_tray() && !stored.close_to_tray());
    let off = kit::sanitize(&json!({ "minimizeToTray": false }), Language::En, Os::Windows);
    assert_eq!(off.get("minimizeToTray"), None);
}

const FP: &str = "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1";

#[test]
fn a_profile_with_a_1_4_4_server_loads_and_keeps_it() {
    let revived = json!({ "game": "revived", "mode": "public", "host": "old.example.org", "port": 443, "name": "Old", "share": null, "fp": FP, "code": null });
    let raw = json!({
        "version": 1,
        "server": revived,
        "savedServers": [revived, { "mode": "private", "host": "100.64.0.1", "port": 61000, "name": "Friends" }],
        "revivedRoot": "D:\\Dauntless144",
        "revivedVerified": "D:\\Dauntless144",
    });
    let settings = kit::sanitize(&raw, Language::En, Os::Windows);
    assert_eq!(
        settings["server"],
        Value::Null,
        "never the current server in the reborn edition"
    );
    let saved = settings["savedServers"].as_array().unwrap();
    assert_eq!(saved.len(), 2);
    assert!(is_revived(saved[0].as_object().unwrap()));
    assert_eq!(saved[1]["game"], "reforged", "no game: a Reforged server");
    assert!(settings.get("revivedRoot").is_none() && settings.get("revivedVerified").is_none());
    assert!(settings.get("gameEdition").is_none());
}

#[test]
fn usernames_follow_the_registration_rule() {
    let slot = "0123456789abcdef01234567";
    let names = |name: Value| {
        kit::sanitize(&json!({ "usernames": { slot: name } }), Language::En, Os::Linux)["usernames"]
            .get(slot)
            .cloned()
    };
    assert_eq!(names(json!("Slayer_01")), Some(json!("Slayer_01")));
    for bad in [
        json!("Sam Smith"),
        json!("ab"),
        json!("a".repeat(17)),
        json!("Bjørn"),
        json!(7),
    ] {
        assert_eq!(names(bad.clone()), None, "{bad}");
    }
}

#[test]
fn a_saved_file_reads_back_the_same() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = SettingsStore::load(dir.path(), Language::En, Os::Windows);
    store
        .update(|settings| {
            settings.insert("gameRoot".into(), Value::from("D:\\Games\\Dauntless"));
            settings.insert("runtimeProfile".into(), Value::from("development"));
            // Dropped from DR: ignored when stored
            settings.insert("huntRegion".into(), Value::from("ger"));
        })
        .unwrap();
    let text = std::fs::read_to_string(dir.path().join("settings.json")).unwrap();
    let other = Language::ALL
        .into_iter()
        .find(|language| *language != Language::En)
        .unwrap();
    let again = SettingsStore::load(dir.path(), other, Os::Windows);
    assert_eq!(settings_text(again.map()), text);
    assert_eq!(again.map()["gameRoot"], "D:\\Games\\Dauntless");
    assert_eq!(again.map()["runtimeProfile"], "development");
    assert!(again.map().get("huntRegion").is_none());
    assert_eq!(again.map()["language"], "en", "a saved language wins over the system's");
}
