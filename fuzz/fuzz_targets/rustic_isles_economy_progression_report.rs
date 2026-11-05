//! What Rustic Isles' progression reads in an objective report (spec-rustic-isles-economy-progression §14): arbitrary
//! JSON through the report check and the mastery plan with arbitrary stored state and host limits, and arbitrary text
//! through `Number(text)` and the grant-id check. Nothing panics; credits never exceed what the report claims for a
//! track, nor a host's room under a cap.
#![no_main]

use std::collections::HashMap;

use libfuzzer_sys::fuzz_target;
use rustic_isles_economy::js_number_text;
use rustic_isles_economy::progression::mastery::{HostLimits, plan_mastery_report, report_is_valid};
use rustic_isles_economy::progression::tracks::is_valid_grant_id;
use serde_json::Value;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let _ = js_number_text(&text);
    let _ = is_valid_grant_id(&text);
    let (report_text, stored_text) = text.split_once('\u{1}').unwrap_or((&text, ""));
    let Ok(report) = serde_json::from_str::<Value>(report_text) else {
        return;
    };
    let _ = report_is_valid(&report);
    let stored_value: Value = serde_json::from_str(stored_text).unwrap_or(Value::Null);
    let stored: HashMap<String, (i64, i64)> = stored_value
        .as_object()
        .map(|m| {
            m.iter()
                .map(|(k, v)| (k.clone(), (v.as_i64().unwrap_or(0), v.as_i64().unwrap_or(0) % 3)))
                .collect()
        })
        .unwrap_or_default();
    let caps = HashMap::from([("MasteryTrack_PlayerLevel".to_owned(), (3, (data.len() % 5) as i64))]);
    let limits = HostLimits {
        max_new: data.len() % 7,
        max_value: 100,
        max_count: 10,
        known: None,
        caps,
    };
    for host in [None, Some(&limits)] {
        let plan = plan_mastery_report(&report, &stored, host);
        for (track, amount) in &plan.credits {
            assert!(*amount > 0);
            let claimed = plan.claimed.iter().find(|(t, _)| t == track).map_or(0, |(_, n)| *n);
            assert!(*amount <= claimed);
            if let Some(limits) = host
                && let Some((cap, total)) = limits.caps.get(track)
            {
                assert!(*amount <= (cap - total).max(0));
            }
        }
    }
});
