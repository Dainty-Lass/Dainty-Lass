// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Synthetic game data for tests (bounties spec §12.3): small files written into a test's own folder. Nothing here
//! comes from the game.

use std::path::Path;

use serde_json::{Value, json};

/// Writes `value` as `dir/name`.
pub fn write_json(dir: &Path, name: &str, value: &Value) -> std::io::Result<()> {
    std::fs::write(dir.join(name), service_kit::jsjson::stringify(value))
}

/// A storage policy naming one instanced weapon, one stackable material and one owned cosmetic.
pub fn storage_policy() -> Value {
    json!({"schema": 1, "entries": {
        "WP_TEST_SWORD": {"storage": "instanced", "maxQuantity": 1, "tags": []},
        "MAT_TEST": {"storage": "stacked", "maxQuantity": 999, "tags": ["Material"]},
        "COSMETIC_TEST": {"storage": "stacked", "maxQuantity": 1, "tags": ["Transmog"]},
    }})
}

/// Curves for the player level, every weapon's ExperienceTrack (ranks 1 to 3, 1000 XP each, prestige 1000 a level
/// paying `CURRENCY_PJM_PRESTIGE_FILLED` 1), the mastery tracks and a Hunt Pass.
pub fn progression_config(pass: &str) -> Value {
    let ranks = |xp: i64| json!([{"rank_id": 1, "xp_required": xp}, {"rank_id": 2, "xp_required": xp}, {"rank_id": 3, "xp_required": xp}]);
    let mut paths = vec![json!({"progression_id": "ExperienceTrack_PlayerLevel", "requirements": ranks(1000)})];
    for weapon in crate::progression::tracks::WEAPONS {
        paths.push(json!({
            "progression_id": format!("ExperienceTrack_Weapon_{weapon}"),
            "requirements": ranks(1000),
            "prestige": {"xp_per_level": 1000, "free_rewards": {"stacked_items": [{"catalog_id": "CURRENCY_PJM_PRESTIGE_FILLED", "quantity": 1}]}},
        }));
        paths.push(json!({"progression_id": format!("PrestigeTrack_Weapon_{weapon}"), "requirements": ranks(1)}));
        paths.push(json!({"progression_id": format!("MasteryTrack_Weapon_{weapon}"), "requirements": ranks(1)}));
    }
    paths.push(json!({"progression_id": "MasteryTrack_PlayerLevel", "requirements": ranks(1)}));
    paths.push(json!({"progression_id": "MasteryTrack_Behemoth", "requirements": ranks(1)}));
    paths.push(json!({"progression_id": pass, "requirements": ranks(100)}));
    json!({"payload": {"paths": paths}})
}
