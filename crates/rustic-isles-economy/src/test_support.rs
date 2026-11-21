// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Synthetic game data for tests (bounties spec §12.3): small files written into a test's own folder. Nothing here
//! comes from the game.

use std::path::Path;

use serde_json::{Value, json};

/// Writes `value` as `dir/name`.
pub fn write_json(dir: &Path, name: &str, value: &Value) -> std::io::Result<()> {
    std::fs::write(dir.join(name), dainty_service_kit::jsjson::stringify(value))
}

/// A storage policy naming one instanced weapon, one stackable material and one owned cosmetic.
pub fn storage_policy() -> Value {
    json!({"schema": 1, "entries": {
        "WP_TEST_SWORD": {"storage": "instanced", "maxQuantity": 1, "tags": []},
        "MAT_TEST": {"storage": "stacked", "maxQuantity": 999, "tags": ["Material"]},
        "COSMETIC_TEST": {"storage": "stacked", "maxQuantity": 1, "tags": ["Transmog"]},
        "MAT_FREE": {"storage": "stacked", "maxQuantity": 0, "tags": []},
    }})
}

/// A Lady Luck store (one one-time SKU, one repeatable), a Reward Cache SKU, Platinum SKUs (a dye in two sections, a
/// Hunt Pass rank skip) and a fountain bundle, as `(file, contents)`.
pub fn store_files() -> Vec<(&'static str, Value)> {
    let steel = |price: i64| json!([{"currencyId": "id_currency_marks_steel", "price": price}]);
    vec![
        (
            "ladyluck_store.json",
            json!([
                {"id": "ll_one", "displayName": "One", "prices": steel(10), "maxAllowed": 1, "tags": ["ladyluckstore"],
                 "items": [{"catalogId": "MAT_TEST", "quantity": 2, "instanced": false}]},
                {"id": "ll_repeat", "displayName": "Again", "prices": steel(5), "maxAllowed": null, "tags": ["ladyluckstore"],
                 "items": [{"catalogId": "WP_TEST_SWORD", "quantity": 1, "instanced": true}]},
            ]),
        ),
        (
            "reward_cache_store.json",
            json!([{"id": "rc_1", "prices": [{"currencyId": "id_currency_seasonal_coin", "price": 3}], "tags": ["season_store"],
                "items": [{"catalogId": "MAT_TEST", "quantity": 1, "instanced": false}]}]),
        ),
        (
            "platinum_store.json",
            json!([
                {"id": "plat_dye", "prices": [{"currencyId": "id_currency_platinum", "price": 100}], "tags": ["webstore", "dyes"],
                 "items": [{"catalogId": "DYE_TEST", "quantity": 1, "instanced": false}]},
                {"id": "plat_skip", "prices": [{"currencyId": "id_currency_platinum", "price": 200}], "tags": ["hp_level_skip_rank"],
                 "items": [], "skuProgression": {"progressionId": "selected_huntpass", "ranks": 1}},
            ]),
        ),
        (
            "fountain_daily_bundle.json",
            json!({"skus": [{"id": "fountain_1", "tags": ["fountain_daily_free_bundle"],
                "prices": [{"currencyId": "id_currency_platinum", "price": 0}],
                "items": [{"catalogId": "MAT_FREE", "quantity": 5, "instanced": false}]}]}),
        ),
    ]
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
