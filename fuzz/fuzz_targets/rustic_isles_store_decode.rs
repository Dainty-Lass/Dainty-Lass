//! What Rustic Isles' store reads back from its own columns (spec-rustic-isles-store §14): arbitrary text as stored
//! roles, a Guard detail, bounty and draft lists, cooldowns, a reforge, a grant result and notices lists, and
//! arbitrary dotted paths and amounts into the rollup document and the daily counters. Nothing panics, a decoder
//! answers a value or a data error, the rollup stays an object, and the slot rules hold for any number.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rustic_isles_store::fuzzing::{decode_grant_result, decode_roles};
use rustic_isles_store::repo::bounties::{decode_bounties, decode_cooldowns, decode_draft_data, rollup_document};
use rustic_isles_store::repo::characters::slots;
use rustic_isles_store::repo::progression::WeaponReforge;
use rustic_isles_store::repo::reforged::{add_daily, decode_daily, decode_guard_detail, decode_notice_list};
use serde_json::Value;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);

    let _ = decode_roles(&text);
    let _ = decode_guard_detail(&text);
    let _ = decode_bounties(&text);
    let _ = decode_draft_data(&text);
    let _ = decode_cooldowns(&text);
    let _ = decode_grant_result(&text);
    let _ = decode_notice_list(&text);
    if let Ok(value) = serde_json::from_str::<Value>(&text)
        && let Some(reforge) = WeaponReforge::from_json(&value)
    {
        assert_eq!(WeaponReforge::from_json(&reforge.to_json()), Some(reforge));
    }

    let numbers: Vec<i64> = data
        .chunks(8)
        .map(|c| {
            let mut b = [0u8; 8];
            b[..c.len()].copy_from_slice(c);
            i64::from_le_bytes(b)
        })
        .collect();
    let first = numbers.first().copied().unwrap_or(0);
    let second = numbers.get(1).copied();
    let amount = f64::from_bits(first.cast_unsigned());
    let counters: Vec<(String, f64)> = text.split('\n').map(|p| (p.to_owned(), amount)).collect();
    assert!(rollup_document(&counters).is_object());
    let mut doc = decode_daily(&text).unwrap_or_default();
    for (path, value) in &counters {
        add_daily(&mut doc, path, *value);
    }

    let _ = slots::resolve_visible_total(first, second);
    let _ = slots::resolve_requested_total(second, first);
    let _ = slots::resolve_active_index(second, u32::try_from(first & 0xff).unwrap_or(0));
    assert!((1..=6).contains(&slots::wire_counts(amount).2));
});
