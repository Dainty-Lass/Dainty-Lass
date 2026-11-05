//! What Rustic Isles' escalation reads (spec-rustic-isles-economy-progression §14): an arbitrary stored record,
//! update and tuning file through the dedicated and host merges. Nothing panics; a host merge never lowers the level
//! nor the XP at an unchanged level.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rustic_isles_economy::progression::escalation::{merge_escalation, merge_host_escalation, parse_tuning};
use rustic_isles_store::AccountId;
use serde_json::Value;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let mut parts = text.splitn(3, '\u{1}');
    let parse = |t: Option<&str>| {
        t.and_then(|t| serde_json::from_str::<Value>(t).ok())
            .unwrap_or(Value::Null)
    };
    let (previous, update, tuning) = (parse(parts.next()), parse(parts.next()), parse(parts.next()));
    let Ok(account) = "account:01ARZ3NDEKTSV4RRFFQ69G5FAV@n".parse::<AccountId>() else {
        return;
    };
    let prior = merge_escalation(None, &previous, "T0", &account, "s");
    let dedicated = merge_escalation(Some(&prior), &update, "T1", &account, "s");
    assert!(dedicated.escalation_level >= prior.escalation_level);
    let table = parse_tuning(&tuning).ok();
    let season = table.as_ref().and_then(|t| t.seasons.values().next());
    for s in [None, season] {
        let merged = merge_host_escalation(Some(&prior), &update, "T2", &account, "s", s);
        assert!(merged.record.escalation_level >= prior.escalation_level);
        if merged.record.escalation_level == prior.escalation_level {
            assert!(merged.record.next_level_xp >= prior.next_level_xp);
        }
    }
});
