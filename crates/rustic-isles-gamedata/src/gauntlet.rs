// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The Gauntlet host's stubs (spec §3.5), not enveloped.

use rustic_isles::season;
use serde_json::{Value, json};

/// The client asks progression and rewards for this season next.
pub const GAUNTLET_SEASON_ID: &str = "ESC_SEASON_1";

/// The season config with the Hunt Pass window holding `now_ms`.
pub fn config_payload(now_ms: i64) -> Value {
    let (start, end) = season::active_season_window(now_ms);
    json!({ "gauntlet_id": GAUNTLET_SEASON_ID, "start_at": start, "end_at": end })
}

/// The level-access request is an integer (0: no access); any other season gets both levels at 0.
pub fn progression_payload(first_segment: &str) -> Value {
    if first_segment.to_lowercase() == "level_access" {
        json!(0)
    } else {
        json!({ "personal_progression": 0, "guild_progression": 0 })
    }
}

pub fn personal_rewards_payload() -> Value {
    json!({})
}

pub fn guild_rewards_payload() -> Value {
    json!(0)
}

/// An array, not an object.
pub fn leaderboard_payload() -> Value {
    json!([])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payloads() {
        assert!(leaderboard_payload().is_array());
        assert_eq!(progression_payload("LEVEL_ACCESS"), json!(0));
        assert_eq!(
            progression_payload(""),
            json!({"personal_progression": 0, "guild_progression": 0})
        );
        assert_eq!(guild_rewards_payload(), json!(0));
        let now = service_kit::js::date_parse("2026-10-16T08:30:00Z").unwrap();
        let config = config_payload(now);
        assert_eq!(
            service_kit::jsjson::stringify(&config),
            r#"{"gauntlet_id":"ESC_SEASON_1","start_at":"2026-07-30T17:00:00+00:00","end_at":"2026-10-22T17:00:00+00:00"}"#
        );
        let start = service_kit::js::date_parse(config["start_at"].as_str().unwrap()).unwrap();
        let end = service_kit::js::date_parse(config["end_at"].as_str().unwrap()).unwrap();
        assert!(start <= now && now < end);
    }
}
