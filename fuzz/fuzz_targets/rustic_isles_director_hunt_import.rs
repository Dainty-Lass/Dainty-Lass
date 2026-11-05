//! A CatalogExporter capture's rows (spec-rustic-isles-director §10, §15): any JSONL through the converters and the
//! merge, in memory. Never panics, and every converted row keeps its fixed key order.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rustic_isles_director::hunt_tables::{convert_rows_for_fuzz, key_order};
use serde_json::Value;

fn in_order(row: &Value, order: &[&str]) -> bool {
    let Some(map) = row.as_object() else {
        return false;
    };
    let mut at = 0;
    for key in map.keys() {
        match order[at..].iter().position(|k| k == key) {
            Some(found) => at += found + 1,
            None if key == "_importedFrom" => {}
            None => return false,
        }
    }
    true
}

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let rows: Vec<Value> = text.lines().filter_map(|l| service_kit::jsjson::parse(l).ok()).collect();
    let (players, matchmakers) = rows.split_at(rows.len() / 2);
    let player_count = players.iter().filter(|r| r.is_object()).count();
    let converted = convert_rows_for_fuzz(players, matchmakers, "1.14.7");
    for (n, row) in converted.iter().enumerate() {
        assert!(in_order(row, key_order(n < player_count)), "{row}");
    }
});
