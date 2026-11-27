// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! What the page hands the module panel (`docs/security/module-panels.md`): a command's form becomes bounded plain
//! arguments, settings merge only into what the module registered, and palette colours carry nothing a CSS
//! variable could escape with.
#![no_main]

use std::collections::BTreeMap;

use dainty_launcher_kit::bridge::modules::{
    ARG_MAX, Settings, VALUE_MAX, command_args, is_colour, merge_settings, plain,
};
use dainty_launcher_kit::bridge::spec::{MODULE_JSON_MAX, fits};
use dainty_module_api::registrar::{CommandSpec, ConfigSection};
use libfuzzer_sys::fuzz_target;
use serde_json::{Value, json};

fuzz_target!(|text: &str| {
    let spec = CommandSpec::new("give", "Gives an item")
        .arg("account", true, "Who")
        .arg("item", false, "What");
    if let Ok(args) = command_args(&spec, text) {
        assert!(fits(text, MODULE_JSON_MAX));
        assert!(!args.is_empty() && args.len() <= spec.args.len());
        for arg in &args {
            assert!(!arg.is_empty() && arg.chars().count() <= ARG_MAX && !arg.chars().any(char::is_control));
        }
    }

    let schema = vec![
        ConfigSection::new("game")
            .key("greeting", json!("hello"), "Text")
            .key("slots", json!(8), "A count")
            .key("pvp", json!(false), "A switch"),
    ];
    let saved: Settings = BTreeMap::new();
    if let Ok(merged) = merge_settings(&schema, &saved, text) {
        assert!(fits(text, MODULE_JSON_MAX));
        for (section, keys) in &merged {
            assert_eq!(section, "game");
            for (key, value) in keys {
                match (key.as_str(), value) {
                    ("greeting", Value::String(s)) => assert!(s.chars().count() <= VALUE_MAX && !s.contains('\0')),
                    ("slots", Value::Number(n)) => assert!(n.is_i64() || n.is_u64()),
                    ("pvp", Value::Bool(_)) => {}
                    other => panic!("merged a setting the module did not register: {other:?}"),
                }
            }
        }
    }

    if is_colour(text) {
        assert!(!text.contains(['(', ')']) || text.matches('(').count() == 1);
        assert!(!text.contains([';', '{', '}', '"', '\'', '\\', ':']));
        assert!(!text.contains("url(") && !text.contains("var("));
    }
    let line = plain(text, 80);
    assert!(line.chars().count() <= 80 && !line.chars().any(char::is_control));
});
