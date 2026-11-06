//! The Reforged API's text rules (spec-rustic-isles-reforged-public §19): the log URL sanitizer, metric names and
//! labels, the notices validation and instant parser, the username, code, note, key and segment checks, lenient
//! base64, the Director's instances and the launcher feed's URL rule. Nothing panics; no `token=` value or JWT shape
//! survives the sanitizer; the exposition stays one sample per line; at most 100 instances, titles of 64 characters.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rustic_isles_reforged::metrics::Metrics;
use rustic_isles_reforged::notices::{parse_instant, validate_input};
use rustic_isles_reforged::reforged_api::{
    invite_note, is_invite_code, looks_like_account_key, normalize_username, status_instances,
};
use rustic_isles_reforged::updates::is_allowed_url;
use rustic_isles_reforged::util::{is_segment, lenient_base64};
use rustic_isles_reforged::{metric_class, sanitize_url_for_log};
use serde_json::Value;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let clean = sanitize_url_for_log(&text);
    if let Some(at) = clean.find("?token=").or_else(|| clean.find("&token=")) {
        assert!(clean[at + 7..].starts_with("[REDACTED]"));
    }
    let _ = metric_class(&text);
    let metrics = Metrics::default();
    let (name, label) = text.split_once(' ').unwrap_or((&text, ""));
    metrics.increment(name, 1.0, &[("class", label), (label, name)]);
    metrics.set(label, 2.0, &[("status", name)]);
    metrics.observe(name, 3.0, &[("method", label)]);
    let rendered = metrics.render(label, true, 1.5);
    assert!(
        rendered
            .lines()
            .all(|l| l.starts_with('#') || l.rsplit_once(' ').is_some())
    );
    let _ = (
        is_invite_code(&text),
        looks_like_account_key(&text),
        is_segment(&text),
        lenient_base64(&text),
    );
    let _ = is_allowed_url(&text, &["https://x.example/".to_owned()]);
    let value = Value::String(text.to_string());
    let _ = (
        normalize_username(Some(&value)),
        invite_note(Some(&value)),
        parse_instant(&value),
    );
    if let Ok(json) = serde_json::from_slice::<Value>(data) {
        let mut n = 0;
        let mut ids = || {
            n += 1;
            format!("id-{n}")
        };
        let _ = validate_input(Some(&json), &mut ids);
        let (list, _) = status_instances(&json);
        assert!(list.len() <= 100);
        assert!(
            list.iter()
                .all(|i| i["title"].as_str().is_some_and(|t| t.chars().count() <= 64))
        );
    }
});
