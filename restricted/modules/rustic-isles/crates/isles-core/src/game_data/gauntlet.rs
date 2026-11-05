//! The Gauntlet stubs' payloads (`gauntletPayloads.ts`), in the shapes the 1.14.7 client parses (2026-10-16).

use serde_json::{Value, json};

use super::season_window::active_season_window;

/// The season id the client then asks progression and rewards for.
pub const GAUNTLET_SEASON_ID: &str = "ESC_SEASON_1";

/// `GET /config`: `{gauntlet_id, start_at, end_at}` on the served Hunt Pass's rolling window.
pub fn config_payload(now_ms: i64) -> Value {
    let (start, end) = active_season_window(now_ms);
    json!({ "gauntlet_id": GAUNTLET_SEASON_ID, "start_at": start, "end_at": end })
}

/// `GET /progression/<season>/<user>`: the level-access request is an integer payload, 0 for no access.
pub fn progression_payload(first_segment: &str) -> Value {
    if first_segment.to_lowercase() == "level_access" {
        json!(0)
    } else {
        json!({ "personal_progression": 0, "guild_progression": 0 })
    }
}

/// `GET /rewards/guild/<season>/<user>`: an integer payload.
pub fn guild_rewards_payload() -> Value {
    json!(0)
}

/// `GET /leaderboard/get_leaderboard/...`: an empty array.
pub fn leaderboard_payload() -> Value {
    json!([])
}

/// The `{code, message, payload}` envelope every Phoenix stub uses.
pub fn envelope(payload: Value) -> Value {
    json!({ "code": null, "message": "OK", "payload": payload })
}

#[cfg(test)]
mod tests {
    use service_kit::js::date_parse;

    use super::*;

    // gauntletPayloads.test.ts
    #[test]
    fn the_leaderboard_answer_is_an_array() {
        assert_eq!(leaderboard_payload(), json!([]));
        assert!(leaderboard_payload().is_array());
    }

    #[test]
    fn the_level_access_answer_is_an_integer_payload() {
        assert_eq!(progression_payload("level_access"), json!(0));
        // the route is case-insensitive, so the check is too
        assert_eq!(progression_payload("LEVEL_ACCESS"), json!(0));
    }

    #[test]
    fn a_progression_answer_has_both_levels_with_or_without_a_season() {
        let zero = json!({ "personal_progression": 0, "guild_progression": 0 });
        assert_eq!(progression_payload("ESC_SEASON_1"), zero);
        assert_eq!(progression_payload(""), zero);
    }

    #[test]
    fn the_guild_rewards_answer_is_an_integer_payload() {
        assert_eq!(guild_rewards_payload(), json!(0));
    }

    #[test]
    fn the_season_config_names_the_season_and_a_current_window() {
        let now = date_parse("2026-10-16T08:30:00Z").unwrap();
        let config = config_payload(now);
        assert_eq!(config["gauntlet_id"], GAUNTLET_SEASON_ID);
        let start = date_parse(config["start_at"].as_str().unwrap()).unwrap();
        let end = date_parse(config["end_at"].as_str().unwrap()).unwrap();
        assert!(start <= now && now < end, "the window covers the present");
        assert_eq!(config["end_at"], "2026-10-22T17:00:00+00:00");
        assert_eq!(
            serde_json::to_string(&config).unwrap(),
            r#"{"gauntlet_id":"ESC_SEASON_1","start_at":"2026-07-30T17:00:00+00:00","end_at":"2026-10-22T17:00:00+00:00"}"#
        );
    }
}
