// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The body checks of `POST /inventory` and `POST /inventory/instanceditem` (spec §4.6). Lengths are UTF-16 units.

use serde_json::Value;

use crate::util::utf16_len;

const MAX_ENTRIES: usize = 4096;
const MAX_ITEM_DATA: usize = 262_144;
const MAX_BOUNDED: f64 = 1_000_000_000.0;

fn non_empty_string(value: Option<&Value>) -> bool {
    value
        .and_then(Value::as_str)
        .is_some_and(|s| (1..=256).contains(&utf16_len(s)))
}

fn is_integer(value: &Value) -> bool {
    value.as_f64().is_some_and(|n| n.is_finite() && n.fract() == 0.0)
}

fn bounded(value: &Value) -> bool {
    is_integer(value) && value.as_f64().is_some_and(|n| (0.0..=MAX_BOUNDED).contains(&n))
}

fn item_data_ok(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => true,
        Some(Value::String(s)) => utf16_len(s) <= MAX_ITEM_DATA,
        Some(_) => false,
    }
}

fn version_ok(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => true,
        Some(v) => bounded(v),
    }
}

fn check_list(body: &Value, field: &str, instanced: bool) -> Option<String> {
    let value = match body.get(field) {
        None | Some(Value::Null) => return None,
        Some(v) => v,
    };
    let Some(entries) = value.as_array() else {
        return Some(format!("{field} must be an array"));
    };
    if entries.len() > MAX_ENTRIES {
        return Some(format!("{field} has too many items"));
    }
    for entry in entries {
        if !entry.is_object() {
            return Some(format!("{field} contains a non-object entry"));
        }
        if !non_empty_string(entry.get("catalogId")) {
            return Some(format!("{field} entry has an invalid catalogId"));
        }
        if instanced {
            if !non_empty_string(entry.get("instanceId")) {
                return Some(format!("{field} entry has an invalid instanceId"));
            }
            if !item_data_ok(entry.get("itemData")) {
                return Some(format!("{field} entry has invalid itemData"));
            }
            if !version_ok(entry.get("updateVersion")) {
                return Some(format!("{field} entry has an invalid updateVersion"));
            }
        } else if !entry.get("quantity").is_some_and(bounded) {
            return Some(format!("{field} entry has an invalid quantity"));
        }
    }
    None
}

/// The first failure's message, or none.
pub fn validate_inventory_transaction_body(body: &Value) -> Option<String> {
    if !body.is_object() {
        return Some("request body must be an object".into());
    }
    if !non_empty_string(body.get("characterId")) {
        return Some("characterId must be a non-empty string".into());
    }
    if !non_empty_string(body.get("transactionId")) {
        return Some("transactionId must be a non-empty string".into());
    }
    for field in ["addInstancedItems", "removeInstancedItems", "saveInstancedItems"] {
        if let Some(message) = check_list(body, field, true) {
            return Some(message);
        }
    }
    for field in ["addStackedItems", "removeStackedItems"] {
        if let Some(message) = check_list(body, field, false) {
            return Some(message);
        }
    }
    None
}

/// The instanced item update body's first problem, as the 400's text; None when it passes.
pub fn validate_instanced_item_update_body(body: &Value) -> Option<String> {
    for field in ["characterId", "instanceId", "catalogId"] {
        if !non_empty_string(body.get(field)) {
            return Some(format!("{field} must be a non-empty string"));
        }
    }
    if !item_data_ok(body.get("itemData")) {
        return Some("itemData must be a string".into());
    }
    if !version_ok(body.get("updateVersion")) {
        return Some("updateVersion must be a bounded non-negative integer".into());
    }
    None
}

/// Whether the body grants or spends anything (a player may only save).
pub fn inventory_body_has_grant_or_spend(body: &Value) -> bool {
    [
        "addInstancedItems",
        "addStackedItems",
        "removeInstancedItems",
        "removeStackedItems",
    ]
    .iter()
    .any(|field| {
        body.get(*field)
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty())
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn transaction_rows() {
        let base = json!({"characterId": "c", "transactionId": "t"});
        assert_eq!(validate_inventory_transaction_body(&base), None);
        let with = |key: &str, value: Value| {
            let mut body = base.clone();
            body[key] = value;
            validate_inventory_transaction_body(&body)
        };
        assert_eq!(
            with("characterId", json!("")).unwrap(),
            "characterId must be a non-empty string"
        );
        assert_eq!(
            with("characterId", json!("x".repeat(257))).unwrap(),
            "characterId must be a non-empty string"
        );
        assert_eq!(
            with("transactionId", Value::Null).unwrap(),
            "transactionId must be a non-empty string"
        );
        assert_eq!(with("addInstancedItems", Value::Null), None);
        assert_eq!(
            with("addInstancedItems", json!("x")).unwrap(),
            "addInstancedItems must be an array"
        );
        assert_eq!(
            with("removeInstancedItems", json!([1])).unwrap(),
            "removeInstancedItems contains a non-object entry"
        );
        assert_eq!(
            with("saveInstancedItems", json!([{"catalogId": "a"}])).unwrap(),
            "saveInstancedItems entry has an invalid instanceId"
        );
        assert_eq!(
            with(
                "addInstancedItems",
                json!([{"catalogId": "a", "instanceId": "i", "itemData": 5}])
            )
            .unwrap(),
            "addInstancedItems entry has invalid itemData"
        );
        assert_eq!(
            with(
                "addInstancedItems",
                json!([{"catalogId": "a", "instanceId": "i", "updateVersion": 1.5}])
            )
            .unwrap(),
            "addInstancedItems entry has an invalid updateVersion"
        );
        assert_eq!(
            with("addStackedItems", json!([{"catalogId": "a", "quantity": 1.5}])).unwrap(),
            "addStackedItems entry has an invalid quantity"
        );
        assert_eq!(
            with("addStackedItems", json!([{"catalogId": "a", "quantity": "x"}])).unwrap(),
            "addStackedItems entry has an invalid quantity"
        );
        assert_eq!(
            with("removeStackedItems", json!([{"catalogId": "", "quantity": 1}])).unwrap(),
            "removeStackedItems entry has an invalid catalogId"
        );
        assert_eq!(
            with("addStackedItems", json!([{"catalogId": "a", "quantity": 0}])),
            None
        );
        let many: Vec<Value> = (0..4097).map(|_| json!({"catalogId": "a", "quantity": 1})).collect();
        assert_eq!(
            with("addStackedItems", Value::Array(many)).unwrap(),
            "addStackedItems has too many items"
        );
    }

    #[test]
    fn update_rows() {
        let ok = json!({"characterId": "c", "instanceId": "i", "catalogId": "a"});
        assert_eq!(validate_instanced_item_update_body(&ok), None);
        assert_eq!(
            validate_instanced_item_update_body(&json!({"characterId": "c", "catalogId": "a"})).unwrap(),
            "instanceId must be a non-empty string"
        );
        let mut big = ok.clone();
        big["updateVersion"] = json!(1e10);
        assert_eq!(
            validate_instanced_item_update_body(&big).unwrap(),
            "updateVersion must be a bounded non-negative integer"
        );
        let mut data = ok;
        data["itemData"] = json!(1);
        assert_eq!(
            validate_instanced_item_update_body(&data).unwrap(),
            "itemData must be a string"
        );
    }
}
