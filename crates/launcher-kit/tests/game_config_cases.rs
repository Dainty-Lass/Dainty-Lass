// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Runs `cases/game-config.json`, the MIT config-file table written from spec §2.7.6 (L5).

#![allow(clippy::unwrap_used, reason = "test helpers outside #[test] functions")]

use launcher_kit::bridge::types::GraphicsPreset;
use launcher_kit::platform::game_config::{
    ConfigError, ConfigRequest, decode, edit_engine_ini, join_lines, remove_phoenix, repair_user_settings, split_lines,
    write_game_config,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct Table {
    engine: Vec<EngineCase>,
    user_settings: Vec<UserCase>,
    game: Vec<GameCase>,
}

#[derive(Deserialize)]
struct EngineCase {
    name: String,
    input: Option<String>,
    host: Option<String>,
    port: Option<u16>,
    graphics: Option<i64>,
    expected: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize)]
struct UserCase {
    name: String,
    input: String,
    #[serde(default)]
    safe_window: bool,
    graphics: Option<i64>,
    expected: String,
    repaired: bool,
}

#[derive(Deserialize)]
struct GameCase {
    name: String,
    input: String,
    expected: Option<String>,
    keys_removed: usize,
}

fn table() -> Table {
    serde_json::from_str(include_str!("cases/game-config.json")).unwrap()
}

fn preset(value: Option<i64>) -> GraphicsPreset {
    value.map_or(GraphicsPreset::UNSET, |v| GraphicsPreset::new(v).unwrap())
}

#[test]
fn engine_ini_table() {
    let cases = table().engine;
    assert!(cases.len() >= 15);
    for case in cases {
        let lines = split_lines(&decode(case.input.as_deref().unwrap_or_default().as_bytes()));
        let host = case.host.as_deref().unwrap_or("127.0.0.1");
        let result = edit_engine_ini(&lines, host, case.port.unwrap_or(61099), preset(case.graphics));
        match (&case.expected, case.error.as_deref(), result) {
            (Some(expected), None, Ok(out)) => assert_eq!(&join_lines(&out), expected, "{}", case.name),
            (None, Some("invalid_host"), Err(ConfigError::InvalidHost))
            | (None, Some("invalid_port"), Err(ConfigError::InvalidPort)) => {}
            (_, _, other) => panic!("{}: {other:?}", case.name),
        }
    }
}

#[test]
fn game_user_settings_table() {
    let cases = table().user_settings;
    assert!(cases.len() >= 15);
    for case in cases {
        let lines = split_lines(&case.input);
        let edit = repair_user_settings(&lines, case.safe_window, preset(case.graphics));
        assert_eq!(join_lines(&edit.lines), case.expected, "{}", case.name);
        assert_eq!(edit.repaired, case.repaired, "{}", case.name);
    }
}

#[test]
fn game_ini_table() {
    let cases = table().game;
    assert!(cases.len() >= 6);
    for case in cases {
        let result = remove_phoenix(&split_lines(&case.input));
        match (&case.expected, result) {
            (None, None) => assert_eq!(case.keys_removed, 0, "{}", case.name),
            (Some(expected), Some((kept, keys))) => {
                assert_eq!(&join_lines(&kept), expected, "{}", case.name);
                assert_eq!(keys, case.keys_removed, "{}", case.name);
            }
            (_, other) => panic!("{}: {other:?}", case.name),
        }
    }
}

// The same cases through the files: what is read from disk is what the table edits, and the bytes written are
// the table's text (all of it ASCII, so written as it is).
#[test]
fn engine_ini_table_through_the_file() {
    for case in table().engine {
        let (Some(expected), None) = (&case.expected, &case.error) else {
            continue;
        };
        let dir = tempfile::tempdir().unwrap();
        if let Some(input) = &case.input {
            std::fs::write(dir.path().join("Engine.ini"), input).unwrap();
        }
        let request = ConfigRequest {
            host: case.host.as_deref().unwrap_or("127.0.0.1"),
            port: case.port.unwrap_or(61099),
            graphics: preset(case.graphics),
            safe_window: false,
            clean_phoenix: false,
        };
        let outcome = write_game_config(dir.path(), &request).unwrap();
        assert_eq!(
            std::fs::read(outcome.engine_ini).unwrap(),
            expected.as_bytes(),
            "{}",
            case.name
        );
    }
}
