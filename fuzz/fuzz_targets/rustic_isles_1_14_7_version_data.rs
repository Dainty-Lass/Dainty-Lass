//! What the 1.14.7 module reads (spec-rustic-isles-1-14-7 §13): arbitrary bytes as `version_data.json` through
//! core's JSON reader and `VersionData::from_json`, which either refuses with a `version_data.json:` text or gives a
//! value whose invariants hold; the same bytes as the design merged with the shipped facts (port plan item 9) and as
//! the host's export (V-4); and arbitrary text through the hooks, which echo it in their fixed shapes.
#![no_main]

use std::collections::HashSet;

use libfuzzer_sys::fuzz_target;
use rustic_isles::{GameMode, ObjectiveRow, VersionHooks};
use rustic_isles_1_14_7::{FACTS_DOCUMENT, Hooks, VersionData, export};
use serde_json::Value;

fn check_document(text: &str) {
    let Ok(value) = dainty_service_kit::jsjson::parse(text) else {
        return;
    };
    check_data(VersionData::from_json(&value), false);
    // The bytes as the design beside the shipped facts
    if let Ok(facts) = serde_json::from_str::<Value>(FACTS_DOCUMENT) {
        check_data(VersionData::from_parts(Some(&facts), &value), true);
        let mut facts = facts;
        let _ = export::default_weapon_parts(&mut facts, text);
        let _ = export::missing_ids(&value, &export::catalog_ids(text));
    }
}

fn check_data(result: Result<VersionData, String>, split: bool) {
    match result {
        Err(reason) => assert!(
            reason.starts_with("version_data.json: ") || (split && reason.starts_with("version_facts.json: ")),
            "{reason}"
        ),
        Ok(data) => {
            let kit = data.starter_kit();
            let instanced: HashSet<&str> = kit.instanced.iter().map(String::as_str).collect();
            assert_eq!(instanced.len(), kit.instanced.len());
            let referenced = kit
                .equipment
                .iter()
                .map(|(_, id)| id)
                .chain(&kit.default_weapon_parts)
                .chain([&kit.flask, &kit.banner, &kit.glider]);
            for id in referenced {
                assert!(instanced.contains(id.as_str()), "{id}");
            }
            assert_eq!(kit.default_quick_chats.len(), 9);
            assert!(kit.wallet.iter().all(|(_, amount)| amount.is_finite()));
            assert!(data.character_data().trigger_status.is_finite());
        }
    }
}

fn check_hooks(text: &str) {
    let hooks = Hooks;
    for hub in [false, true] {
        let destination = hooks.candidate_destination(text, hub);
        match destination.hunt_id {
            None => assert_eq!(destination.game_mode, GameMode::City),
            Some(id) => assert_eq!(id, text),
        }
    }
    let pass = hooks.huntpass_payload(text);
    assert_eq!(pass["huntpass_id"], Value::from(text));
    assert_eq!(pass["season"], Value::from(text));
    let row = ObjectiveRow {
        objective_id: text.to_owned(),
        value: text.len() as f64,
        completed_count: 0.0,
        created: text.to_owned(),
        last_modified: text.to_owned(),
    };
    let written = hooks.objective_row(text, &row);
    let keys: Vec<&str> = written.as_object().map(|o| o.keys().map(String::as_str).collect()).unwrap_or_default();
    assert_eq!(
        keys,
        ["phx_account_id", "objective_id", "progress", "completed_count", "created_date", "last_modified_date"]
    );
    assert_eq!(written["objective_id"], Value::from(text));
    let _ = dainty_service_kit::jsjson::stringify(&written);
}

fuzz_target!(|bytes: &[u8]| {
    let text = String::from_utf8_lossy(bytes);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    check_document(text);
    check_hooks(text);
});
