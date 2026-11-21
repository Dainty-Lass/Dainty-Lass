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

/// With Gauntlet on, the level a player may enter is the one after their best: the hunt menu keeps Gauntlet locked at
/// 0 even with the Slayer's Path node bought (first Gauntlet test, 2026-11-19).
pub fn level_access(gauntlet_on: bool, best_level: i64) -> i64 {
    if gauntlet_on {
        best_level.max(0).saturating_add(1)
    } else {
        0
    }
}

/// Whether the request is the level-access query (`/progression/level_access/<season>`).
pub fn is_level_access(first_segment: &str) -> bool {
    first_segment.to_lowercase() == "level_access"
}

/// The level-access request is an integer (0: no access, else `level_access`); any other is both progressions at the
/// player's best level, as there are no guilds yet.
pub fn progression_payload(first_segment: &str, level_access: i64, best_level: i64) -> Value {
    if is_level_access(first_segment) {
        json!(level_access)
    } else {
        json!({ "personal_progression": best_level, "guild_progression": best_level })
    }
}

/// The personal rewards answer: always an empty object.
pub fn personal_rewards_payload() -> Value {
    json!({})
}

/// The guild rewards answer: always 0.
pub fn guild_rewards_payload() -> Value {
    json!(0)
}

/// An array, not an object.
pub fn leaderboard_payload() -> Value {
    json!([])
}

/// The value without any field whose name looks like a credential, at any depth, for the log.
pub fn redacted(value: Value) -> Value {
    const SECRET: [&str; 6] = ["session", "token", "key", "secret", "password", "auth"];
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter(|(k, _)| {
                    let k = k.to_ascii_lowercase();
                    !SECRET.iter().any(|s| k.contains(s))
                })
                .map(|(k, v)| (k, redacted(v)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(redacted).collect()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redaction() {
        let body = json!({"level": 3, "session_id": "s", "x": [{"AuthToken": "t", "ok": 1}], "apiKey": "k"});
        assert_eq!(redacted(body), json!({"level": 3, "x": [{"ok": 1}]}));
    }

    #[test]
    fn payloads() {
        assert!(leaderboard_payload().is_array());
        assert_eq!(progression_payload("LEVEL_ACCESS", 0, 0), json!(0));
        assert_eq!(progression_payload("level_access", level_access(true, 0), 0), json!(1));
        assert_eq!(level_access(true, 4), 5);
        assert_eq!(level_access(false, 4), 0);
        assert_eq!(
            progression_payload("ESC_SEASON_1", 0, 3),
            json!({"personal_progression": 3, "guild_progression": 3})
        );
        assert_eq!(
            progression_payload("", 1, 0),
            json!({"personal_progression": 0, "guild_progression": 0})
        );
        assert_eq!(guild_rewards_payload(), json!(0));
        let now = dainty_service_kit::js::date_parse("2026-10-16T08:30:00Z").unwrap();
        let config = config_payload(now);
        assert_eq!(
            dainty_service_kit::jsjson::stringify(&config),
            r#"{"gauntlet_id":"ESC_SEASON_1","start_at":"2026-07-30T17:00:00+00:00","end_at":"2026-10-22T17:00:00+00:00"}"#
        );
        let start = dainty_service_kit::js::date_parse(config["start_at"].as_str().unwrap()).unwrap();
        let end = dainty_service_kit::js::date_parse(config["end_at"].as_str().unwrap()).unwrap();
        assert!(start <= now && now < end);
    }
}
