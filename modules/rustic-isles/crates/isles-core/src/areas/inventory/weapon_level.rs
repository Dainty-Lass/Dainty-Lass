//! `weaponBaseLevel.ts` (Reforged patch 0036): weapons served at item level 0, because the 1.14.7 paks carry no
//! per-level power tables. The stored level is left as it is; only what the game reads is set to 0. Whether it runs is
//! the `weapons_at_base_level` hook (1.14.7: unless `KEEP_WEAPON_ITEM_LEVELS=true`; 1.12.0: never).

use serde_json::{Map, Value};
use service_kit::jsjson;

/// `WeaponItemDataAtBaseLevel`: itemData with `CurrentLevel` 0 for a weapon (`WP_...`) whose level is above 0;
/// anything else (other items, no level, unparsable data) comes back unchanged (`None`).
pub fn weapon_item_data_at_base_level(catalog_id: Option<&Value>, item_data: Option<&Value>) -> Option<String> {
    let (Some(Value::String(catalog_id)), Some(Value::String(item_data))) = (catalog_id, item_data) else {
        return None;
    };
    if !catalog_id.starts_with("WP_") || !item_data.contains("CurrentLevel") {
        return None;
    }
    let Ok(Value::Object(mut parsed)) = jsjson::parse(item_data) else {
        return None;
    };
    let level = parsed.get("CurrentLevel").and_then(Value::as_f64)?;
    if level <= 0.0 {
        return None;
    }
    parsed.insert("CurrentLevel".into(), Value::from(0));
    Some(jsjson::stringify(&Value::Object(parsed)))
}

/// `ServeWeaponsAtBaseLevel`'s copy: `None` when nothing in `value` changes.
fn serve(value: &Value, depth: u32) -> Option<Value> {
    if depth > 4 {
        return None;
    }
    match value {
        Value::Array(entries) => {
            let served: Vec<Option<Value>> = entries.iter().map(|entry| serve(entry, depth + 1)).collect();
            served.iter().any(Option::is_some).then(|| {
                Value::Array(
                    served
                        .into_iter()
                        .zip(entries)
                        .map(|(served, entry)| served.unwrap_or_else(|| entry.clone()))
                        .collect(),
                )
            })
        }
        Value::Object(source) => {
            let mut out: Option<Map<String, Value>> = None;
            if source.contains_key("catalogId")
                && source.contains_key("itemData")
                && let Some(data) = weapon_item_data_at_base_level(source.get("catalogId"), source.get("itemData"))
            {
                let mut copy = source.clone();
                copy.insert("itemData".into(), Value::String(data));
                out = Some(copy);
            }
            for (key, field) in source {
                if !matches!(field, Value::Array(_) | Value::Object(_)) {
                    continue;
                }
                if let Some(next) = serve(field, depth + 1) {
                    out.get_or_insert_with(|| source.clone()).insert(key.clone(), next);
                }
            }
            out.map(Value::Object)
        }
        _ => None,
    }
}

/// A copy of `value` in which every item object found in it, in arrays or object fields up to a few levels deep, has
/// its weapon level served as 0. `enabled` is the version's `weapons_at_base_level` hook.
pub fn serve_weapons_at_base_level(value: Value, enabled: bool) -> Value {
    if !enabled {
        return value;
    }
    serve(&value, 0).unwrap_or(value)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn levelled() -> String {
        json!({"TransmogCatalogID": "None", "UnlockedAsTransmog": false, "CurrentLevel": 5, "DataVersion": 1,
            "EquippedCells": [], "EquippedWeaponParts": [], "CurrentXP": 0})
        .to_string()
    }

    fn level_of(item_data: &Value) -> i64 {
        serde_json::from_str::<Value>(item_data.as_str().unwrap()).unwrap()["CurrentLevel"]
            .as_i64()
            .unwrap()
    }

    // weaponBaseLevel.test.ts
    #[test]
    fn a_levelled_weapon_is_served_at_level_0_with_the_rest_of_its_data_kept() {
        let text = weapon_item_data_at_base_level(Some(&json!("WP_MS_MCROLLIN")), Some(&json!(levelled()))).unwrap();
        let out: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(out["CurrentLevel"], 0);
        assert_eq!(out["DataVersion"], 1);
        assert_eq!(out["EquippedWeaponParts"], json!([]));
        // JSON.stringify keeps CurrentLevel where it was
        assert!(
            text.contains(r#""UnlockedAsTransmog":false,"CurrentLevel":0,"DataVersion":1"#),
            "{text}"
        );
    }

    #[test]
    fn level_0_missing_level_other_items_and_bad_data_are_unchanged() {
        let zero = json!(json!({"CurrentLevel": 0}).to_string());
        assert_eq!(
            weapon_item_data_at_base_level(Some(&json!("WP_MS_MCROLLIN")), Some(&zero)),
            None
        );
        assert_eq!(
            weapon_item_data_at_base_level(Some(&json!("WP_MS_BEGINNER")), Some(&Value::Null)),
            None
        );
        let five = json!(json!({"CurrentLevel": 5}).to_string());
        assert_eq!(
            weapon_item_data_at_base_level(Some(&json!("AR_ROCK_HELM")), Some(&five)),
            None
        );
        assert_eq!(
            weapon_item_data_at_base_level(Some(&json!("WP_CB_MCROLLIN")), Some(&json!("{CurrentLevel"))),
            None
        );
        assert_eq!(weapon_item_data_at_base_level(Some(&json!(7)), Some(&five)), None);
    }

    #[test]
    fn an_inventory_response_is_copied_with_its_weapons_at_level_0() {
        let inventory = json!({
            "characterId": "c",
            "instancedItems": [
                {"catalogId": "WP_CB_MCROLLIN", "instanceId": "WP_CB_MCROLLIN", "itemData": levelled(), "updateVersion": 5},
                {"catalogId": "LT_BASIC", "instanceId": "x", "itemData": null, "updateVersion": 0}
            ],
            "stackedItems": [{"catalogId": "CURRENCY_NOTES", "quantity": 3}]
        });
        let out = serve_weapons_at_base_level(inventory.clone(), true);
        assert_ne!(out, inventory);
        assert_eq!(level_of(&out["instancedItems"][0]["itemData"]), 0);
        assert_eq!(out["instancedItems"][0]["updateVersion"], 5);
        assert_eq!(out["instancedItems"][1], inventory["instancedItems"][1]);
        assert_eq!(out["stackedItems"], inventory["stackedItems"]);
        assert_eq!(level_of(&inventory["instancedItems"][0]["itemData"]), 5);
    }

    #[test]
    fn a_single_item_and_a_nested_transaction_result_are_handled() {
        let item = json!({"catalogId": "WP_MS_FULGAR", "instanceId": "i", "itemData": levelled(), "updateVersion": 2});
        assert_eq!(
            level_of(&serve_weapons_at_base_level(item.clone(), true)["itemData"]),
            0
        );
        let result = json!({"payload": {"instancedItemsAdded": [item]}});
        let out = serve_weapons_at_base_level(result, true);
        assert_eq!(level_of(&out["payload"]["instancedItemsAdded"][0]["itemData"]), 0);
        let plain = json!({"a": 1, "b": [1, 2]});
        assert_eq!(serve_weapons_at_base_level(plain.clone(), true), plain);
    }

    #[test]
    fn the_hook_off_serves_the_stored_level() {
        let item = json!({"catalogId": "WP_MS_FULGAR", "instanceId": "i", "itemData": levelled(), "updateVersion": 2});
        assert_eq!(serve_weapons_at_base_level(item.clone(), false), item);
    }

    #[test]
    fn nothing_is_served_past_four_levels_down() {
        let item = json!({"catalogId": "WP_MS_FULGAR", "itemData": levelled()});
        let deep = json!({"a": {"b": {"c": {"d": {"e": item}}}}});
        assert_eq!(serve_weapons_at_base_level(deep.clone(), true), deep);
        let reachable = json!({"a": {"b": {"c": {"d": item}}}});
        assert_ne!(serve_weapons_at_base_level(reachable.clone(), true), reachable);
    }
}
