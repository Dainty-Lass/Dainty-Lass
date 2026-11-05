//! `routes/tuning.ts`: the `/game_tuning/*` answers, written field for field as Node writes them.

use serde_json::{Value, json};

use super::gauntlet::envelope;
use crate::tables::ItemGrantWindow;

pub fn seasonal_event_schedule() -> Value {
    envelope(json!({ "ScheduledItems": [] }))
}

pub fn huntpass_xp_config() -> Value {
    envelope(json!({
        "EventConfigs": [],
        "GlobalConfig": { "DifficultyBias": 1, "GlobalMultiplier": 1, "MaxXPAwarded": 200 }
    }))
}

pub fn island_content_config() -> Value {
    envelope(json!({ "DisallowedIslandContentAssets": [] }))
}

/// `BountyGrantWindows()`, from the version's `BOUNTY_ITEM_GRANT_DATA`: windows that cover now, or the bounty
/// component finds no season and the hub disconnects (1.14.7 2026-10-03).
pub fn item_grant_data(windows: &[ItemGrantWindow]) -> Value {
    Value::Array(
        windows
            .iter()
            .map(|window| {
                json!({
                    "on_claim_grant_item_amount": window.amount,
                    "on_claim_grant_item_end_date": window.end,
                    "on_claim_grant_item_event_id": "",
                    "on_claim_grant_item_id": window.item_id,
                    "on_claim_grant_item_start_date": window.start
                })
            })
            .collect(),
    )
}

pub fn bounty_game_data(windows: &[ItemGrantWindow]) -> Value {
    envelope(json!({
        "bounty_data": [
            { "bounty_id": "Bounty_Bronze_KillWithFriends", "enabled": false },
            { "bounty_id": "Bounty_Silver_KillWithFriends", "enabled": false },
            { "bounty_id": "Bounty_Gold_KillWithFriends", "enabled": false }
        ],
        "bounty_token_grant_hour": 0,
        "bounty_token_id": "TOKEN_BOUNTY_DRAFT",
        "bronze_count": 9,
        "silver_count": 3,
        "gold_count": 1,
        "history_length": 10,
        "item_grant_data": item_grant_data(windows),
        "max_slots": 4,
        "new_season_reset_bounties": false,
        "num_draft_options": 3,
        "num_spicy_options": 1,
        "num_tokens_hp_start": 4,
        "num_tokens_per_day": 0,
        "premium_bounty_token_id": "TOKEN_BOUNTY_DRAFT_PREMIUM",
        "token_rollover_warning_days": 1000
    }))
}

/// Serves `item_grant_data: []` on purpose (1.14.7 MEASURED NEGATIVE 2026-10-04).
pub fn bounty_game_data_daily() -> Value {
    envelope(json!({
        "automatic_claim": true,
        "automatic_draft": true,
        "bounty_data": [
            { "bounty_id": "Challenge_Daily_Bronze_GetHuntPassXP", "enabled": false }
        ],
        "bounty_token_grant_hour": 0,
        "bounty_token_id": "TOKEN_DAILY_CHALLENGE_DRAFT",
        "bronze_count": 1,
        "delete_claimed_bounties": false,
        "gold_count": 0,
        "history_length": 10,
        "item_grant_data": [],
        "max_slots": 1,
        "new_season_reset_bounties": true,
        "num_draft_options": 3,
        "num_spicy_options": 1,
        "num_tokens_hp_start": 1,
        "num_tokens_per_day": 0,
        "premium_bounty_token_id": "TOKEN_DAILY_CHALLENGE_DRAFT_PREMIUM",
        "silver_count": 0,
        "token_rollover_warning_days": 1000
    }))
}

pub fn bounty_game_data_weekly() -> Value {
    envelope(json!({
        "automatic_claim": true,
        "automatic_draft": true,
        "bounty_data": [
            { "bounty_id": "26_11_7_Challenge_Season_BreakParts_Firesacs_Aether_Sally_Terra", "enabled": false }
        ],
        "item_grant_data": [],
        "bounty_token_id": "TOKEN_WEEKLY_CHALLENGE_DRAFT",
        "bounty_token_grant_hour": 0,
        "bronze_count": 0,
        "silver_count": 0,
        "gold_count": 0,
        "history_length": 8,
        "max_slots": 4,
        "num_draft_options": 3,
        "num_spicy_options": 1,
        "num_tokens_hp_start": 4,
        "num_tokens_per_day": 0,
        "new_season_reset_bounties": true,
        "delete_claimed_bounties": false,
        "premium_bounty_token_id": "TOKEN_WEEKLY_CHALLENGE_DRAFT_PREMIUM",
        "token_rollover_warning_days": 1000
    }))
}
