// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

use super::*;

const DEFAULT_FILE: &str = r#"{
  "version": 1,
  "server": null,
  "savedServers": [],
  "gameRoot": null,
  "graphics": -1,
  "windowed": false,
  "streamerMode": false,
  "language": "en",
  "gameLanguage": "auto",
  "runtimeProfile": "production",
  "hostTools": false,
  "usernames": {},
  "backupOffered": {},
  "trustedRuntimes": {},
  "localRuntimeDir": null,
  "hostWorkspace": null,
  "dumperDll": null
}"#;

#[test]
fn defaults_are_written_byte_for_byte() {
    assert_eq!(
        String::from_utf8(to_bytes(&defaults(Language::En))).unwrap(),
        DEFAULT_FILE
    );
}

#[test]
fn a_server_and_flags_are_written_byte_for_byte() {
    let raw: Value = serde_json::from_str(
        r#"{"closeToTray":true,"graphics":2.0,"server":{"name":"A","host":"127.0.0.1","port":80.0,"code":"abcd"},
            "trustedRuntimes":{"0123456789abcdef01234567":["0000000000000000000000000000000000000000000000000000000000000000"]}}"#,
    )
    .unwrap();
    let expected = r#"{
  "version": 1,
  "server": {
    "game": "reforged",
    "mode": "private",
    "host": "127.0.0.1",
    "port": 80,
    "name": "A",
    "share": null,
    "fp": null,
    "code": "abcd"
  },
  "savedServers": [
    {
      "game": "reforged",
      "mode": "private",
      "host": "127.0.0.1",
      "port": 80,
      "name": "A",
      "share": null,
      "fp": null,
      "code": "abcd"
    }
  ],
  "gameRoot": null,
  "graphics": 2,
  "windowed": false,
  "streamerMode": false,
  "language": "en",
  "gameLanguage": "auto",
  "runtimeProfile": "production",
  "hostTools": false,
  "usernames": {},
  "backupOffered": {},
  "trustedRuntimes": {
    "0123456789abcdef01234567": [
      "0000000000000000000000000000000000000000000000000000000000000000"
    ]
  },
  "localRuntimeDir": null,
  "hostWorkspace": null,
  "dumperDll": null,
  "closeToTray": true
}"#;
    let got = sanitize(&raw, Language::En, Os::Windows);
    assert_eq!(String::from_utf8(to_bytes(&got)).unwrap(), expected);
}

#[test]
fn non_ascii_is_written_raw() {
    let raw = json!({"savedServers": [{"name": "Ilha São João", "host": "localhost", "port": 1}]});
    let text = String::from_utf8(to_bytes(&sanitize(&raw, Language::En, Os::Linux))).unwrap();
    assert!(text.contains(r#""name": "Ilha São João""#), "{text}");
}

#[test]
fn integral_matches_javascript() {
    let parse = |text: &str| integral(&serde_json::from_str(text).unwrap());
    assert_eq!(parse("2"), Some(2));
    assert_eq!(parse("2.0"), Some(2));
    assert_eq!(parse("-0"), Some(0));
    assert_eq!(parse("1e2"), Some(100));
    assert_eq!(parse("1.5"), None);
    assert_eq!(parse("\"2\""), None);
    assert_eq!(parse("1e300"), None);
}

#[test]
fn missing_or_broken_file_loads_defaults_without_writing() {
    let dir = tempfile::tempdir().unwrap();
    let store = SettingsStore::load(dir.path(), Language::PtBr, Os::Windows);
    assert_eq!(store.map(), &defaults(Language::PtBr));
    assert!(!store.path().exists());

    fs::write(dir.path().join(SETTINGS_FILE), b"{not json").unwrap();
    let store = SettingsStore::load(dir.path(), Language::En, Os::Windows);
    assert_eq!(store.map(), &defaults(Language::En));
    assert_eq!(fs::read(store.path()).unwrap(), b"{not json");
}

#[test]
fn load_sanitises_and_update_writes_through_tmp() {
    let dir = tempfile::tempdir().unwrap();
    let user_data = dir.path().join("User Data");
    fs::create_dir_all(&user_data).unwrap();
    fs::write(user_data.join(SETTINGS_FILE), br#"{"windowed":true,"junk":1}"#).unwrap();
    let mut store = SettingsStore::load(&user_data, Language::PtBr, Os::Windows);
    assert_eq!(store.map()["windowed"], json!(true));
    assert_eq!(store.map()["language"], json!("pt-BR"));
    assert!(!store.map().contains_key("junk"));

    store
        .update(|map| {
            map.insert("graphics".into(), json!(4));
            map.insert("minimizeToTray".into(), json!(true));
        })
        .unwrap();
    assert!(!user_data.join(SETTINGS_TMP_FILE).exists());
    let written = fs::read(user_data.join(SETTINGS_FILE)).unwrap();
    assert_eq!(written, to_bytes(store.map()));
    assert!(!written.ends_with(b"\n"));

    let reloaded = SettingsStore::load(&user_data, Language::En, Os::Windows);
    assert_eq!(reloaded.map(), store.map());
    assert_eq!(reloaded.map()["graphics"], json!(4));
    assert_eq!(reloaded.map()["language"], json!("pt-BR"));
}

#[test]
fn an_update_without_a_language_defaults_to_english() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = SettingsStore::load(dir.path(), Language::PtBr, Os::Linux);
    store.update(|map| drop(map.remove("language"))).unwrap();
    assert_eq!(store.map()["language"], json!("en"));
    store
        .update(|map| drop(map.insert("graphics".into(), json!(9))))
        .unwrap();
    assert_eq!(store.map()["graphics"], json!(-1), "an edit is sanitised too");
}

#[test]
fn a_flag_turned_off_leaves_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = SettingsStore::load(dir.path(), Language::En, Os::Windows);
    store
        .update(|map| drop(map.insert("showGameConsole".into(), json!(true))))
        .unwrap();
    store
        .update(|map| drop(map.insert("showGameConsole".into(), json!(false))))
        .unwrap();
    assert_eq!(
        String::from_utf8(fs::read(store.path()).unwrap()).unwrap(),
        DEFAULT_FILE
    );
}

#[test]
fn folder_path_rules() {
    assert!(is_folder_path("C:\\x", 200, Os::Windows));
    assert!(is_folder_path("z:/xy", 200, Os::Windows));
    assert!(!is_folder_path("C:\\", 200, Os::Windows));
    assert!(!is_folder_path("CC:\\x", 200, Os::Windows));
    assert!(!is_folder_path("é:\\x", 200, Os::Windows));
    assert!(is_folder_path("/opt", 200, Os::Linux));
    assert!(!is_folder_path("~/opt", 200, Os::Linux));
    assert!(!is_folder_path("/op\0t", 200, Os::Linux));
}
