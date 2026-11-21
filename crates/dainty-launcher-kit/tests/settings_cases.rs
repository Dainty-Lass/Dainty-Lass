// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Runs `cases/settings.json`, the MIT settings table written from spec §2.7.2 (L5).

use dainty_launcher_kit::core::i18n::Language;
use dainty_launcher_kit::platform::settings::{Os, defaults, sanitize, to_bytes};
use serde::Deserialize;
use serde_json::{Map, Value};

#[derive(Deserialize)]
struct Table {
    base: Map<String, Value>,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    os: String,
    language: String,
    input: Value,
    expected: Map<String, Value>,
}

#[test]
fn settings_table() {
    let table: Table = serde_json::from_str(include_str!("cases/settings.json")).unwrap();
    assert!(table.cases.len() >= 100);
    assert_eq!(
        table.base,
        defaults(Language::En),
        "the table's base is the English defaults"
    );
    for case in table.cases {
        let os = match case.os.as_str() {
            "windows" => Os::Windows,
            "linux" => Os::Linux,
            other => panic!("{}: unknown os {other}", case.name),
        };
        let language = Language::from_code(&case.language).unwrap();
        let mut expected = table.base.clone();
        for (key, value) in case.expected {
            expected.insert(key, value);
        }
        let got = sanitize(&case.input, language, os);
        // Bytes, not values: the key order is the contract.
        assert_eq!(
            String::from_utf8(to_bytes(&got)).unwrap(),
            String::from_utf8(to_bytes(&expected)).unwrap(),
            "{}",
            case.name
        );
        let again = sanitize(&Value::Object(got.clone()), language, os);
        assert_eq!(
            to_bytes(&again),
            to_bytes(&got),
            "{}: sanitising is idempotent",
            case.name
        );
    }
}
