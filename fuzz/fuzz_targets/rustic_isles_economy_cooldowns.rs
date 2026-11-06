//! What Rustic Isles' cooldowns read from game servers and hosts (spec-rustic-isles-economy-bounties §13): arbitrary
//! bodies and stored entries through validation and both merges. Nothing panics; at most 1024 entries; a host's batch
//! never moves a stored readable date earlier, never removes an id, and never accepts a date past the latest allowed
//! unless it is the stored one.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rustic_isles_economy::bounties::cooldowns::{
    latest_allowed, merge_cooldowns, plan_host_cooldowns, validate_cooldown_save,
};
use rustic_isles_store::AccountId;
use serde_json::Value;
use service_kit::js::date_parse;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let (body_text, stored_text) = text.split_once('\u{1}').unwrap_or((&text, ""));
    let body: Option<Value> = serde_json::from_str(body_text).ok();
    let stored_body: Option<Value> = serde_json::from_str(stored_text).ok();
    let Ok(account) = "account:01ARZ3NDEKTSV4RRFFQ69G5FAV@n".parse::<AccountId>() else {
        return;
    };
    let now_ms = 1_792_000_000_000 + i64::from(data.first().copied().unwrap_or(0)) * 3_600_000;
    let stored_entries = validate_cooldown_save(stored_body.as_ref()).unwrap_or_default();
    let stored = merge_cooldowns(None, &account, &stored_entries, now_ms);
    let Ok(incoming) = validate_cooldown_save(body.as_ref()) else {
        return;
    };
    let dedicated = merge_cooldowns(Some(&stored), &account, &incoming, now_ms);
    assert!(dedicated.entries.len() <= 1024);
    let (accepted, _) = plan_host_cooldowns(Some(&stored), &incoming, now_ms);
    let host = merge_cooldowns(Some(&stored), &account, &accepted, now_ms);
    assert!(host.entries.len() <= 1024);
    for (id, date) in &accepted {
        let attempted = date_parse(date);
        let before = stored
            .entries
            .iter()
            .find(|e| e.id == *id)
            .map(|e| e.started_date.clone());
        let before_ms = before.as_deref().and_then(date_parse);
        if attempted.is_some_and(|a| a > latest_allowed(id, now_ms)) {
            assert_eq!(attempted, before_ms, "{id}");
        }
    }
    for entry in &stored.entries {
        let Some(after) = host.entries.iter().find(|e| e.id == entry.id) else {
            // Only a stale gatherable (pruned by the dedicated rule) may go
            assert!(entry.id.len() == 32);
            continue;
        };
        if let (Some(old), Some(new)) = (date_parse(&entry.started_date), date_parse(&after.started_date)) {
            assert!(new >= old, "{}", entry.id);
        }
    }
});
