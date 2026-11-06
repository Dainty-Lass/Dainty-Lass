// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Runs `cases/log.json`, the MIT redaction table written from spec §2.7.4 (L5).

use launcher_kit::platform::log::Redactor;
use serde::Deserialize;

#[derive(Deserialize)]
struct Table {
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    #[serde(default)]
    register: Vec<String>,
    #[serde(default)]
    unregister: Vec<String>,
    #[serde(default)]
    registered: Vec<String>,
    #[serde(default)]
    not_registered: Vec<String>,
    input: Option<String>,
    expected: Option<String>,
}

#[test]
fn log_redaction_table() {
    let table: Table = serde_json::from_str(include_str!("cases/log.json")).unwrap();
    assert!(table.cases.len() >= 30);
    for case in table.cases {
        let mut redactor = Redactor::new();
        for value in &case.register {
            redactor.register(value);
        }
        for value in &case.unregister {
            redactor.unregister(value);
        }
        for value in &case.registered {
            assert!(
                redactor.is_registered(value),
                "{}: {value:?} should be remembered",
                case.name
            );
        }
        for value in &case.not_registered {
            assert!(
                !redactor.is_registered(value),
                "{}: {value:?} should not be remembered",
                case.name
            );
        }
        if let (Some(input), Some(expected)) = (&case.input, &case.expected) {
            assert_eq!(&redactor.redact(input), expected, "{}", case.name);
        }
    }
}
