// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The `/game_tuning/*` payloads (spec §3.4), held as the spec's one-line JSON and parsed once; `preserve_order`
//! keeps each text's key order, which is the wire order (weekly's differs from daily's on purpose).

use std::sync::LazyLock;

use rustic_isles::BountyItemGrant;
use serde_json::{Value, json};

use crate::envelope;

const SCHEDULE: &str = r#"{"ScheduledItems":[]}"#;
const HUNTPASS_XP: &str =
    r#"{"EventConfigs":[],"GlobalConfig":{"DifficultyBias":1,"GlobalMultiplier":1,"MaxXPAwarded":200}}"#;
const ISLAND_CONTENT: &str = r#"{"DisallowedIslandContentAssets":[]}"#;
/// `item_grant_data` is filled from the version's windows.
const BOUNTY: &str = r#"{"bounty_data":[{"bounty_id":"Bounty_Bronze_KillWithFriends","enabled":false},{"bounty_id":"Bounty_Silver_KillWithFriends","enabled":false},{"bounty_id":"Bounty_Gold_KillWithFriends","enabled":false}],"bounty_token_grant_hour":0,"bounty_token_id":"TOKEN_BOUNTY_DRAFT","bronze_count":9,"silver_count":3,"gold_count":1,"history_length":10,"item_grant_data":[],"max_slots":4,"new_season_reset_bounties":false,"num_draft_options":3,"num_spicy_options":1,"num_tokens_hp_start":4,"num_tokens_per_day":0,"premium_bounty_token_id":"TOKEN_BOUNTY_DRAFT_PREMIUM","token_rollover_warning_days":1000}"#;
/// An empty grant list on purpose: 1.14.7 measured negative with one (2026-10-04).
const DAILY: &str = r#"{"automatic_claim":true,"automatic_draft":true,"bounty_data":[{"bounty_id":"Challenge_Daily_Bronze_GetHuntPassXP","enabled":false}],"bounty_token_grant_hour":0,"bounty_token_id":"TOKEN_DAILY_CHALLENGE_DRAFT","bronze_count":1,"delete_claimed_bounties":false,"gold_count":0,"history_length":10,"item_grant_data":[],"max_slots":1,"new_season_reset_bounties":true,"num_draft_options":3,"num_spicy_options":1,"num_tokens_hp_start":1,"num_tokens_per_day":0,"premium_bounty_token_id":"TOKEN_DAILY_CHALLENGE_DRAFT_PREMIUM","silver_count":0,"token_rollover_warning_days":1000}"#;
const WEEKLY: &str = r#"{"automatic_claim":true,"automatic_draft":true,"bounty_data":[{"bounty_id":"26_11_7_Challenge_Season_BreakParts_Firesacs_Aether_Sally_Terra","enabled":false}],"item_grant_data":[],"bounty_token_id":"TOKEN_WEEKLY_CHALLENGE_DRAFT","bounty_token_grant_hour":0,"bronze_count":0,"silver_count":0,"gold_count":0,"history_length":8,"max_slots":4,"num_draft_options":3,"num_spicy_options":1,"num_tokens_hp_start":4,"num_tokens_per_day":0,"new_season_reset_bounties":true,"delete_claimed_bounties":false,"premium_bounty_token_id":"TOKEN_WEEKLY_CHALLENGE_DRAFT_PREMIUM","token_rollover_warning_days":1000}"#;

struct Parsed {
    schedule: Value,
    huntpass_xp: Value,
    island_content: Value,
    bounty: Value,
    daily: Value,
    weekly: Value,
}

// The texts are constants a unit test parses, so the null fallback never serves
static PARSED: LazyLock<Parsed> = LazyLock::new(|| {
    let parse = |text: &str| serde_json::from_str(text).unwrap_or(Value::Null);
    Parsed {
        schedule: parse(SCHEDULE),
        huntpass_xp: parse(HUNTPASS_XP),
        island_content: parse(ISLAND_CONTENT),
        bounty: parse(BOUNTY),
        daily: parse(DAILY),
        weekly: parse(WEEKLY),
    }
});

/// `/game_tuning/seasonal_event_schedule`: no scheduled items.
pub fn seasonal_event_schedule() -> Value {
    envelope(PARSED.schedule.clone())
}

/// `/game_tuning/huntpass_xp_config`: no event configs, the global XP rules.
pub fn huntpass_xp_config() -> Value {
    envelope(PARSED.huntpass_xp.clone())
}

/// `/game_tuning/island_content_config`: nothing disallowed.
pub fn island_content_config() -> Value {
    envelope(PARSED.island_content.clone())
}

/// One grant object per window, in table order (not enveloped). The windows must cover now, or the client's
/// bounty component finds no season and the hub disconnects (1.14.7, 2026-10-03).
pub fn item_grant_data(windows: &[BountyItemGrant]) -> Value {
    windows
        .iter()
        .map(|window| {
            json!({
                "on_claim_grant_item_amount": window.amount,
                "on_claim_grant_item_end_date": window.end,
                "on_claim_grant_item_event_id": "",
                "on_claim_grant_item_id": window.item_id,
                "on_claim_grant_item_start_date": window.start,
            })
        })
        .collect()
}

/// The bounty config with `item_grant_data` from `windows`, enveloped.
pub fn bounty_game_data(windows: &[BountyItemGrant]) -> Value {
    let mut payload = PARSED.bounty.clone();
    if let Some(map) = payload.as_object_mut() {
        // Replaced in place, so the key keeps its position
        map.insert("item_grant_data".to_owned(), item_grant_data(windows));
    }
    envelope(payload)
}

/// The daily bounty config, enveloped.
pub fn bounty_game_data_daily() -> Value {
    envelope(PARSED.daily.clone())
}

/// The weekly bounty config, enveloped.
pub fn bounty_game_data_weekly() -> Value {
    envelope(PARSED.weekly.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texts_parse_and_keep_their_order() {
        for text in [SCHEDULE, HUNTPASS_XP, ISLAND_CONTENT, BOUNTY, DAILY, WEEKLY] {
            let value: Value = serde_json::from_str(text).unwrap();
            assert_eq!(dainty_service_kit::jsjson::stringify(&value), text);
        }
        let payload = &bounty_game_data(&[])["payload"];
        assert_eq!(dainty_service_kit::jsjson::stringify(payload), BOUNTY);
    }
}
