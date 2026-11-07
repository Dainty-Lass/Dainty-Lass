//! The Director's tag queries (spec-rustic-isles-director §9.4, §15): any JSON query against any row tags. Never
//! panics or overflows the stack, and stops at the end of the stream whatever the counts say.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rustic_isles_director::hunt::evaluate_tag_query;
use serde_json::Value;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let (query, tags) = text.split_once('\n').unwrap_or((&text, ""));
    let Ok(query) = dainty_service_kit::jsjson::parse(query) else {
        return;
    };
    let tags: Vec<&str> = tags.split(',').collect();
    let first = evaluate_tag_query(&query, &tags);
    assert_eq!(first, evaluate_tag_query(&query, &tags));
    if let Value::Object(map) = &query {
        assert!(!map.is_empty() || !first);
    }
});
