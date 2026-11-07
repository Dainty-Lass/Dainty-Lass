//! What the game-data generators read from host files (spec-rustic-isles-gamedata §4, §11): arbitrary bytes as a
//! catalog line, a nested `customData`, a store-image row name, a journey row and a Lady Luck source. The readers
//! and the slug, decode and category rules never panic, a checked texture reference cannot close its quote, and
//! the pretty writer round-trips through `JSON.parse` at every indent from 0 to 4.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rustic_isles_gamedata::generators::fuzzing::{
    catalog_line, category, checked_texture, decode, journey_definition, ladyluck_source, reconstructed_price, slug,
};
use rustic_isles_gamedata::generators::stringify_pretty;
use serde_json::{Value, json};

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);

    let _ = catalog_line(&text);
    let item_data = json!({ "ItemData": text }).to_string();
    let nested = json!({ "itemId": "X", "customData": item_data }).to_string();
    if let Some((rarity, image)) = catalog_line(&nested) {
        assert!(image.is_none_or(|i| !i.is_empty()));
        let _ = rarity;
    }

    let s = slug(&text);
    assert!(!s.starts_with('_') && !s.ends_with('_') && !s.contains("__"));
    assert!(s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'));
    let _ = decode(&text, &["AR_X", &text.to_ascii_uppercase()]);
    if let Some(c) = category(&text) {
        let _ = reconstructed_price(&c, i64::from(data.first().copied().unwrap_or(0)));
    }
    if let Ok(reference) = checked_texture(&text) {
        assert_eq!(reference.matches('\'').count(), 2);
    }

    let Ok(value) = dainty_service_kit::jsjson::parse(&text) else {
        return;
    };
    let _ = journey_definition(&value);
    if let Value::Array(entries) = &value {
        let (skus, _) = ladyluck_source(entries);
        assert_eq!(skus.len(), entries.len());
    }
    for indent in 0..=4 {
        let back = dainty_service_kit::jsjson::parse(&stringify_pretty(&value, indent));
        assert!(back.is_ok_and(|b| dainty_service_kit::jsjson::deep_equal(&b, &value)));
    }
});
