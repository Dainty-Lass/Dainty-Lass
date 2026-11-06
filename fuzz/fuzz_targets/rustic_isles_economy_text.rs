//! The short texts Rustic Isles' bounties and store read (spec-rustic-isles-economy-bounties §13): arbitrary text as a
//! weekly challenge id, a season cooldown id, a cooldown id, a query string and a log token. Nothing panics; a parsed
//! weekly id rebuilds to the same text; a plain log token never holds a control character.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rustic_isles_economy::bounties::cooldowns::{is_season_id, is_valid_cooldown_id};
use rustic_isles_economy::bounties::state::parse_weekly_id;
use rustic_isles_economy::bounties::store::{query_values, single_query_value};
use rustic_isles_economy::log_token;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    if let Some(id) = parse_weekly_id(&text) {
        assert_eq!(id.text(), text);
    }
    let _ = is_season_id(&text);
    if is_valid_cooldown_id(&text) {
        assert!(text.bytes().all(|b| b.is_ascii_graphic()));
    }
    let _ = query_values(&text, "token");
    let _ = single_query_value(&text, "requiredTags");
    let token = log_token(&text);
    if token == text {
        assert!(!token.chars().any(char::is_control));
    }
});
