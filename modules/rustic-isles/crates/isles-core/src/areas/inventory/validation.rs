//! `validation.ts`'s inventory validators: each returns the 400 message or `None` when the body is valid.

use serde_json::Value;
use service_kit::js;

const MAX_ID_LENGTH: usize = 256;
const MAX_ITEMS_PER_ARRAY: usize = 4096;
const MAX_QUANTITY: f64 = 1_000_000_000.0;
const MAX_ITEM_DATA_LENGTH: usize = 262_144;
const MAX_UPDATE_VERSION: f64 = 1_000_000_000.0;

/// JavaScript's `String.length`: UTF-16 code units.
fn js_len(text: &str) -> usize {
    text.encode_utf16().count()
}

fn is_non_empty_string(value: Option<&Value>, max: usize) -> bool {
    matches!(value, Some(Value::String(text)) if (1..=max).contains(&js_len(text)))
}

/// `typeof x === "number" && Number.isInteger(x)`.
fn integer(value: Option<&Value>) -> Option<f64> {
    let n = value?.as_f64()?;
    (n.is_finite() && n.fract() == 0.0).then_some(n)
}

fn is_bounded_non_negative_integer(value: Option<&Value>) -> bool {
    integer(value).is_some_and(|n| (0.0..=MAX_UPDATE_VERSION).contains(&n))
}

fn item_data_is_valid(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => true,
        Some(Value::String(data)) => js_len(data) <= MAX_ITEM_DATA_LENGTH,
        Some(_) => false,
    }
}

fn collection<'a>(value: Option<&'a Value>, field: &str) -> Result<Option<&'a Vec<Value>>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(items)) if items.len() > MAX_ITEMS_PER_ARRAY => Err(format!("{field} has too many items")),
        Some(Value::Array(items)) => Ok(Some(items)),
        Some(_) => Err(format!("{field} must be an array")),
    }
}

fn instanced_array(value: Option<&Value>, field: &str) -> Option<String> {
    let items = match collection(value, field) {
        Ok(Some(items)) => items,
        Ok(None) => return None,
        Err(error) => return Some(error),
    };
    for item in items {
        let Value::Object(item) = item else {
            return Some(format!("{field} contains a non-object entry"));
        };
        if !is_non_empty_string(item.get("catalogId"), MAX_ID_LENGTH) {
            return Some(format!("{field} entry has an invalid catalogId"));
        }
        if !is_non_empty_string(item.get("instanceId"), MAX_ID_LENGTH) {
            return Some(format!("{field} entry has an invalid instanceId"));
        }
        if !item_data_is_valid(item.get("itemData")) {
            return Some(format!("{field} entry has invalid itemData"));
        }
        let version = item.get("updateVersion");
        if !js::is_nullish(version) && !is_bounded_non_negative_integer(version) {
            return Some(format!("{field} entry has an invalid updateVersion"));
        }
    }
    None
}

fn stacked_array(value: Option<&Value>, field: &str) -> Option<String> {
    let items = match collection(value, field) {
        Ok(Some(items)) => items,
        Ok(None) => return None,
        Err(error) => return Some(error),
    };
    for item in items {
        let Value::Object(item) = item else {
            return Some(format!("{field} contains a non-object entry"));
        };
        if !is_non_empty_string(item.get("catalogId"), MAX_ID_LENGTH) {
            return Some(format!("{field} entry has an invalid catalogId"));
        }
        if !integer(item.get("quantity")).is_some_and(|q| (0.0..=MAX_QUANTITY).contains(&q)) {
            return Some(format!("{field} entry has an invalid quantity"));
        }
    }
    None
}

/// `ValidateInventoryTransactionBody`.
pub fn validate_inventory_transaction_body(body: &Value) -> Option<String> {
    let Value::Object(fields) = body else {
        return Some("request body must be an object".into());
    };
    if !is_non_empty_string(fields.get("characterId"), MAX_ID_LENGTH) {
        return Some("characterId must be a non-empty string".into());
    }
    if !is_non_empty_string(fields.get("transactionId"), MAX_ID_LENGTH) {
        return Some("transactionId must be a non-empty string".into());
    }
    for field in ["addInstancedItems", "removeInstancedItems", "saveInstancedItems"] {
        if let Some(error) = instanced_array(fields.get(field), field) {
            return Some(error);
        }
    }
    for field in ["addStackedItems", "removeStackedItems"] {
        if let Some(error) = stacked_array(fields.get(field), field) {
            return Some(error);
        }
    }
    None
}

/// `InventoryBodyHasGrantOrSpend`: the transaction grants or spends, rather than only saving owned items' data.
pub fn inventory_body_has_grant_or_spend(body: &Value) -> bool {
    let Value::Object(fields) = body else {
        return false;
    };
    [
        "addInstancedItems",
        "addStackedItems",
        "removeInstancedItems",
        "removeStackedItems",
    ]
    .iter()
    .any(|field| matches!(fields.get(*field), Some(Value::Array(items)) if !items.is_empty()))
}

/// `ValidateInstancedItemUpdateBody`.
pub fn validate_instanced_item_update_body(body: &Value) -> Option<String> {
    let Value::Object(fields) = body else {
        return Some("request body must be an object".into());
    };
    for field in ["characterId", "instanceId", "catalogId"] {
        if !is_non_empty_string(fields.get(field), MAX_ID_LENGTH) {
            return Some(format!("{field} must be a non-empty string"));
        }
    }
    if !item_data_is_valid(fields.get("itemData")) {
        return Some("itemData must be a string".into());
    }
    let version = fields.get("updateVersion");
    if !js::is_nullish(version) && !is_bounded_non_negative_integer(version) {
        return Some("updateVersion must be a bounded non-negative integer".into());
    }
    None
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_transaction_body_is_checked_field_by_field() {
        let ok =
            json!({"characterId": "c", "transactionId": "t", "addStackedItems": [{"catalogId": "X", "quantity": 0}]});
        assert_eq!(validate_inventory_transaction_body(&ok), None);
        let check = |patch: Value| {
            let mut body = ok.clone();
            for (key, value) in patch.as_object().unwrap() {
                body[key] = value.clone();
            }
            validate_inventory_transaction_body(&body)
        };
        assert_eq!(
            check(json!({"characterId": ""})).as_deref(),
            Some("characterId must be a non-empty string")
        );
        assert_eq!(
            check(json!({"transactionId": 5})).as_deref(),
            Some("transactionId must be a non-empty string")
        );
        assert_eq!(
            check(json!({"addStackedItems": "x"})).as_deref(),
            Some("addStackedItems must be an array")
        );
        assert_eq!(
            check(json!({"addStackedItems": [[]]})).as_deref(),
            Some("addStackedItems contains a non-object entry")
        );
        assert_eq!(
            check(json!({"addStackedItems": [{"catalogId": "X", "quantity": 1.5}]})).as_deref(),
            Some("addStackedItems entry has an invalid quantity")
        );
        assert_eq!(
            check(json!({"removeStackedItems": [{"catalogId": "X", "quantity": -1}]})).as_deref(),
            Some("removeStackedItems entry has an invalid quantity")
        );
        assert_eq!(
            check(json!({"addInstancedItems": [{"catalogId": "X"}]})).as_deref(),
            Some("addInstancedItems entry has an invalid instanceId")
        );
        assert_eq!(
            check(json!({"saveInstancedItems": [{"catalogId": "X", "instanceId": "i", "itemData": 3}]})).as_deref(),
            Some("saveInstancedItems entry has invalid itemData")
        );
        assert_eq!(
            check(json!({"saveInstancedItems": [{"catalogId": "X", "instanceId": "i", "updateVersion": -1}]}))
                .as_deref(),
            Some("saveInstancedItems entry has an invalid updateVersion")
        );
        // null reads as absent
        assert_eq!(check(json!({"addStackedItems": null})), None);
        let too_long = "x".repeat(257);
        assert_eq!(
            check(json!({"addInstancedItems": [{"catalogId": too_long, "instanceId": "i"}]})).as_deref(),
            Some("addInstancedItems entry has an invalid catalogId")
        );
    }

    #[test]
    fn grants_and_spends_are_told_from_saves() {
        assert!(!inventory_body_has_grant_or_spend(
            &json!({"saveInstancedItems": [{"a": 1}]})
        ));
        assert!(!inventory_body_has_grant_or_spend(&json!({"addStackedItems": []})));
        assert!(inventory_body_has_grant_or_spend(
            &json!({"removeInstancedItems": [{}]})
        ));
        assert!(!inventory_body_has_grant_or_spend(&json!(null)));
    }

    #[test]
    fn an_item_update_body_is_checked() {
        let body =
            json!({"characterId": "c", "instanceId": "i", "catalogId": "X", "itemData": "{}", "updateVersion": 3});
        assert_eq!(validate_instanced_item_update_body(&body), None);
        let base = json!({"characterId": "c", "instanceId": "i", "catalogId": "X"});
        let with = |key: &str, value: Value| {
            let mut body = base.clone();
            body[key] = value;
            validate_instanced_item_update_body(&body)
        };
        assert_eq!(with("itemData", json!(1)).as_deref(), Some("itemData must be a string"));
        assert_eq!(
            with("updateVersion", json!(1e10)).as_deref(),
            Some("updateVersion must be a bounded non-negative integer")
        );
        assert_eq!(
            validate_instanced_item_update_body(&json!({"characterId": "c"})).as_deref(),
            Some("instanceId must be a non-empty string")
        );
    }
}
