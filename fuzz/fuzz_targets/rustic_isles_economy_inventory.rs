//! What Rustic Isles' inventory reads from game servers and its own rows (spec-rustic-isles-economy-inventory §15):
//! arbitrary bytes as a `POST /inventory` body and an item update body, and as the stored arrays. For a body that
//! validates, the request hash, the grant normalization (a fixed small policy) and the item rules never panic; the
//! hash ignores key order; a normalized stack never becomes more than 1000 instances or an instance without a
//! 26-character id; a transaction never stores a `CURRENCY_` row nor leaves a stack at or below 0 after removals.
//! Weapon serving changes nothing but `CurrentLevel`; the multiplier reader and the Tailscale check never panic.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rustic_isles_economy::inventory::items::{Delta, apply_transaction, compute_inventory_request_hash};
use rustic_isles_economy::inventory::storage_policy::{StoragePolicy, normalize_inventory_grant_collections};
use rustic_isles_economy::inventory::validation::{
    validate_instanced_item_update_body, validate_inventory_transaction_body,
};
use rustic_isles_economy::inventory::weapon_level::serve_weapons_at_base_level;
use rustic_isles_economy::multipliers::{is_tailscale_host, read_test_multipliers};
use serde_json::{Map, Value, json};

fn reversed(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = Map::new();
            for (k, v) in map.iter().rev() {
                out.insert(k.clone(), reversed(v));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(reversed).collect()),
        other => other.clone(),
    }
}

/// JSON equality with numbers compared as JavaScript does (`-0 === 0`), keys in any order.
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => x.len() == y.len() && x.iter().zip(y).all(|(p, q)| same(p, q)),
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| same(v, w)))
        }
        _ => a == b,
    }
}

fn level_free(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| match (k.as_str(), v) {
                    ("itemData", Value::String(text)) => {
                        let parsed = dainty_service_kit::jsjson::parse(text).ok();
                        let cleared = parsed.map(|mut p| {
                            if let Some(m) = p.as_object_mut() {
                                m.remove("CurrentLevel");
                            }
                            p
                        });
                        (k.clone(), cleared.unwrap_or_else(|| v.clone()))
                    }
                    _ => (k.clone(), level_free(v)),
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(level_free).collect()),
        other => other.clone(),
    }
}

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let mut parts = text.splitn(3, '\u{1}');
    let (body_text, stored_text, extra) = (
        parts.next().unwrap_or(""),
        parts.next().unwrap_or(""),
        parts.next().unwrap_or(""),
    );
    let _ = read_test_multipliers(&rustic_isles::Settings::from_pairs([
        ("REFORGED_XP_MULTIPLIER", extra),
        ("REFORGED_SERVER_MODE", "private"),
        ("REFORGED_SERVER_HOST", extra),
    ]));
    let _ = is_tailscale_host(extra);
    let Ok(body) = serde_json::from_str::<Value>(body_text) else {
        return;
    };
    let _ = validate_instanced_item_update_body(&body);
    let served = serve_weapons_at_base_level(&body, true);
    assert!(same(&level_free(&served), &level_free(&body)));
    if validate_inventory_transaction_body(&body).is_some() {
        return;
    }
    let field = |v: &Value, k: &str| v.get(k).cloned().unwrap_or(Value::Null);
    let hash = |v: &Value| {
        compute_inventory_request_hash(
            &field(v, "addInstancedItems"),
            &field(v, "addStackedItems"),
            &field(v, "removeInstancedItems"),
            &field(v, "removeStackedItems"),
            &field(v, "saveInstancedItems"),
        )
    };
    assert_eq!(hash(&body), hash(&reversed(&body)));
    let policy = StoragePolicy::from_document(&json!({"entries": {
        "WP_A": {"storage": "instanced", "maxQuantity": 1}, "MAT": {"storage": "stacked", "maxQuantity": 99},
    }}));
    let Ok(normalized) = normalize_inventory_grant_collections(
        &policy,
        "tx",
        &field(&body, "addInstancedItems"),
        &field(&body, "addStackedItems"),
    ) else {
        return;
    };
    let sent = |k: &str| body.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
    let (sent_instanced, sent_stacked) = (sent("addInstancedItems"), sent("addStackedItems"));
    assert!(normalized.add_instanced.len() <= sent_instanced.len() + 1000 * sent_stacked.len());
    for item in normalized.add_instanced.iter().filter(|i| !sent_instanced.contains(i)) {
        let id = item.get("instanceId").and_then(Value::as_str).unwrap_or("");
        assert_eq!(id.len(), 26);
    }
    let stored: Value = serde_json::from_str(stored_text).unwrap_or(Value::Null);
    let list = |k: &str| stored.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
    let removes = field(&body, "removeStackedItems");
    let mut n = 0u32;
    let mut minter = move || {
        n += 1;
        format!("{n:036}")
    };
    let Ok(applied) = apply_transaction(Delta {
        user_id: "u",
        transaction_id: "t",
        instanced: list("instanced"),
        stacked: list("stacked"),
        add_instanced: &normalized.add_instanced,
        add_stacked: &normalized.add_stacked,
        remove_instanced: &field(&body, "removeInstancedItems"),
        remove_stacked: &removes,
        save_instanced: &field(&body, "saveInstancedItems"),
        touched: &[],
        minter: &mut minter,
    }) else {
        return;
    };
    for entry in &applied.stacked {
        let id = entry.get("catalogId").and_then(Value::as_str).unwrap_or("");
        assert!(!id.starts_with("CURRENCY_"));
        if removes.as_array().is_some_and(|r| !r.is_empty()) {
            assert!(dainty_service_kit::js::number(entry.get("quantity")) > 0.0);
        }
    }
});
