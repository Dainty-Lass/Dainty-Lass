//! The Hunt Pass window rewrite (spec-rustic-isles-gamedata §3.7, §11): arbitrary JSON as `payload.paths`, any
//! served id and recent-season text, any now in the JS date range. Never panics; non-pass elements are unchanged;
//! the path count grows by at most the recent-season count; every opened or added pass has the expected window.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rustic_isles::{Settings, StarterManifest, VersionTables, season};
use rustic_isles_gamedata::progression_config::{
    FUTURE_END, FUTURE_START, recent_seasons, served_huntpass_id, widen_huntpass_windows,
};
use serde_json::{Value, json};

static TABLES: VersionTables = VersionTables {
    target_changelist: 1,
    game_version: "1.14.7",
    active_huntpass_id: "season1",
    recent_huntpass_seasons: &["season1", "season2"],
    patchnotes_release_version: "",
    bounty_item_grant_data: &[],
    store_sections: &[],
    seasonal_coin: None,
    masked_journey_nodes: &[],
    starter_manifest: StarterManifest {
        bootstrap_version: "1",
        stacked_item_ids: &[],
        quick_emoji_wheel: &[],
    },
    switch_defaults: &[],
    session_payload: rustic_isles_auth::session_payload_1_14_7,
};

// Date's range: ±8.64e15 ms
const RANGE: i64 = 8_640_000_000_000_000;

fn is_pass(path: &Value) -> bool {
    path.get("progression_id").and_then(Value::as_str).is_some_and(|id| {
        let lower = id.to_ascii_lowercase();
        ["eventpass_", "d24_", "season"].iter().any(|p| lower.starts_with(p))
    })
}

fuzz_target!(|data: &[u8]| {
    let Some((head, rest)) = data.split_first_chunk::<8>() else {
        return;
    };
    let now = i64::from_le_bytes(*head).rem_euclid(2 * RANGE) - RANGE;
    let text = String::from_utf8_lossy(rest);
    let mut parts = text.splitn(3, '\u{0}');
    let paths_text = parts.next().unwrap_or("");
    let id = parts.next();
    let seasons = parts.next();
    let Ok(paths) = service_kit::jsjson::parse(paths_text) else {
        return;
    };
    let mut pairs = Vec::new();
    if let Some(id) = id {
        pairs.push(("GAMESERVER_HUNTPASS_ID", id));
    }
    if let Some(seasons) = seasons {
        pairs.push(("GAMESERVER_RECENT_SEASONS", seasons));
    }
    let settings = Settings::from_pairs(pairs);
    let config = json!({ "payload": { "paths": paths } });
    let out = widen_huntpass_windows(config.clone(), &TABLES, &settings, now);
    let (Some(before), Some(after)) = (
        config["payload"]["paths"].as_array(),
        out["payload"]["paths"].as_array(),
    ) else {
        assert_eq!(out, config);
        return;
    };
    let added = after.len() - before.len();
    assert!(added <= recent_seasons(&TABLES, &settings).len());
    let served = served_huntpass_id(&TABLES, &settings);
    let (start, end) = season::active_season_window(now);
    for (old, new) in before.iter().zip(after) {
        if !is_pass(old) {
            assert_eq!(old, new);
        } else if old["progression_id"].as_str() == Some(served.as_str()) {
            assert_eq!((&new["start_date"], &new["end_date"]), (&json!(start), &json!(end)));
        }
    }
    for new in &after[before.len()..] {
        if new.is_object() {
            assert_eq!(
                (&new["start_date"], &new["end_date"]),
                (&json!(FUTURE_START), &json!(FUTURE_END))
            );
        }
    }
});
