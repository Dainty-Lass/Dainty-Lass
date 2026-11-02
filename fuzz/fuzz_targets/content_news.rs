// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The news document the content server reads from the host's file or GitHub (RX3, docs/security/content.md).
#![no_main]

use std::sync::Once;

use content::input::{MAX_BODY, MAX_NEWS_ITEMS, MAX_TITLE, parse_news};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    static QUIET: Once = Once::new();
    QUIET.call_once(|| content::log::set_sink(Some(Box::new(|_, _| {}))));
    let Ok(doc) = serde_json::from_slice::<serde_json::Value>(data) else {
        return;
    };
    let Ok(items) = parse_news(&doc) else {
        return;
    };
    assert!(items.len() <= MAX_NEWS_ITEMS);
    assert!(items.windows(2).all(|w| w[0].date >= w[1].date));
    for item in &items {
        // What the launcher shows: one-line titles, bodies with only tabs and newlines, both bounded
        assert!(!item.title.is_empty() && item.title.encode_utf16().count() <= MAX_TITLE);
        assert!(!item.title.chars().any(|c| c < ' ' || c == '\u{7f}'));
        assert!(item.body.encode_utf16().count() <= MAX_BODY);
        assert!(!item.body.chars().any(|c| (c < ' ' && c != '\t' && c != '\n') || c == '\u{7f}' || c == '\r'));
        assert!(item.date.ends_with('Z'));
    }
});
