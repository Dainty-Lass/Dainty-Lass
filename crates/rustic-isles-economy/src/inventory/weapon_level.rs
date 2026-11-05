// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Weapons at base level (spec §7.7, Reforged patch 0036): the 1.14.7 paks carry no per-level power tables, so a
//! weapon is served at item level 0; the stored level is untouched.

use serde_json::Value;
use service_kit::jsjson;

/// A levelled `WP_` item's data with `CurrentLevel` set to 0 in place; anything else none.
pub fn weapon_item_data_at_base_level(catalog_id: &Value, item_data: &Value) -> Option<String> {
    let catalog = catalog_id.as_str()?;
    let text = item_data.as_str()?;
    if !catalog.starts_with("WP_") || !text.contains("CurrentLevel") {
        return None;
    }
    let Ok(Value::Object(mut data)) = jsjson::parse(text) else {
        return None;
    };
    let level = data.get("CurrentLevel")?.as_f64()?;
    if level <= 0.0 {
        return None;
    }
    data.insert("CurrentLevel".into(), Value::from(0));
    Some(jsjson::stringify(&Value::Object(data)))
}

/// A copy with every item (an object with `catalogId` and `itemData`) up to depth 4 served at base level.
pub fn serve_weapons_at_base_level(value: &Value, enabled: bool) -> Value {
    if !enabled {
        return value.clone();
    }
    let mut out = value.clone();
    serve(&mut out, 0);
    out
}

fn serve(value: &mut Value, depth: usize) {
    if depth > 4 {
        return;
    }
    match value {
        Value::Array(items) => items.iter_mut().for_each(|item| serve(item, depth + 1)),
        Value::Object(map) => {
            if let (Some(catalog), Some(data)) = (map.get("catalogId"), map.get("itemData"))
                && let Some(served) = weapon_item_data_at_base_level(catalog, data)
            {
                map.insert("itemData".into(), Value::String(served));
            }
            for (_, field) in map.iter_mut() {
                serve(field, depth + 1);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn item(level: Value) -> Value {
        let data = jsjson::stringify(&json!({"A": 1, "CurrentLevel": level, "Z": 2}));
        json!({"catalogId": "WP_AXE", "itemData": data})
    }

    #[test]
    fn levels() {
        let served = serve_weapons_at_base_level(&item(json!(5)), true);
        assert_eq!(served["itemData"], r#"{"A":1,"CurrentLevel":0,"Z":2}"#);
        for unchanged in [
            item(json!(0)),
            json!({"catalogId": "WP_A", "itemData": "{\"x\":1}"}),
            json!({"catalogId": "LT_A", "itemData": "{\"CurrentLevel\":3}"}),
            json!({"catalogId": "WP_A", "itemData": "CurrentLevel{"}),
            json!({"catalogId": 5, "itemData": "{\"CurrentLevel\":3}"}),
        ] {
            assert_eq!(serve_weapons_at_base_level(&unchanged, true), unchanged);
        }
        assert_eq!(serve_weapons_at_base_level(&item(json!(5)), false), item(json!(5)));
    }

    #[test]
    fn depth() {
        // depth 0 is the response, so an item inside one array at each level: [[[[item]]]] is depth 4
        let four = json!([[[[item(json!(5))]]]]);
        assert_eq!(
            serve_weapons_at_base_level(&four, true)[0][0][0][0]["itemData"],
            r#"{"A":1,"CurrentLevel":0,"Z":2}"#
        );
        let five = json!([[[[[item(json!(5))]]]]]);
        assert_eq!(serve_weapons_at_base_level(&five, true), five);
        let nested = json!({"instancedItems": [item(json!(2))]});
        assert_eq!(
            serve_weapons_at_base_level(&nested, true)["instancedItems"][0]["itemData"],
            r#"{"A":1,"CurrentLevel":0,"Z":2}"#
        );
    }
}
