// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The session supplies refilled at every game login (progression spec §4.14): a host's file of stacked items each
//! listed player is topped back up to.

use serde_json::Value;

use crate::util::obj;

/// `userId` to `(catalog id, quantity)` targets, in file order.
pub type SuppliesFile = Vec<(String, Vec<(String, i64)>)>;

const SHAPE: &str = "expected {\"accounts\": {\"<user id>\": {\"<catalog id>\": <quantity>}}}";

fn catalog_ok(id: &str) -> bool {
    (1..=128).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// The file's text (one BOM already dropped); `instanced` says whether the storage policy keeps an id instanced.
pub fn parse_session_supplies(text: &str, instanced: &dyn Fn(&str) -> bool) -> Result<SuppliesFile, String> {
    let document = dainty_service_kit::jsjson::parse(text).map_err(|e| e.to_string())?;
    let Some(accounts) = document.get("accounts").and_then(Value::as_object) else {
        return Err(SHAPE.into());
    };
    let mut out = Vec::new();
    for (user, items) in accounts {
        let quoted = dainty_service_kit::jsjson::stringify(&Value::String(user.clone()));
        let Some(items) = items.as_object().filter(|_| !user.is_empty()) else {
            return Err(format!(
                "account {quoted}: expected an object of catalog ids and quantities"
            ));
        };
        let mut targets = Vec::new();
        for (id, quantity) in items {
            if !catalog_ok(id) {
                return Err(format!(
                    "account {user}: invalid catalog id {}",
                    dainty_service_kit::jsjson::stringify(&Value::String(id.clone()))
                ));
            }
            if id.starts_with("CURRENCY_") {
                return Err(format!(
                    "account {user}: {id} is a currency (wallet), not a stacked item"
                ));
            }
            if instanced(id) {
                return Err(format!("account {user}: {id} is an instanced item, not a stacked one"));
            }
            let q = quantity
                .as_f64()
                .filter(|q| q.fract() == 0.0 && (0.0..=1_000_000.0).contains(q));
            let Some(q) = q else {
                return Err(format!(
                    "account {user}: quantity of {id} must be a whole number from 0 to 1000000"
                ));
            };
            targets.push((id.clone(), crate::util::whole(q)));
        }
        out.push((user.clone(), targets));
    }
    Ok(out)
}

/// The stacked list topped up to `targets`; none when nothing changed.
pub fn apply_session_supplies(stacked: &[Value], targets: &[(String, i64)]) -> Option<Vec<Value>> {
    let mut out = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    let mut changed = false;
    for entry in stacked {
        let id = entry.get("catalogId").and_then(Value::as_str);
        let Some((target_id, target)) = id.and_then(|id| targets.iter().find(|(t, _)| t == id)) else {
            out.push(entry.clone());
            continue;
        };
        if seen.contains(&target_id.as_str()) {
            changed = true;
            continue;
        }
        seen.push(target_id);
        if *target == 0 {
            changed = true;
            continue;
        }
        #[allow(clippy::cast_precision_loss, reason = "targets are at most 1000000")]
        let target_f = *target as f64;
        if entry.get("quantity").and_then(Value::as_f64) != Some(target_f) {
            changed = true;
        }
        let mut kept = entry.clone();
        if let Some(map) = kept.as_object_mut() {
            map.insert("quantity".into(), Value::from(*target));
        }
        out.push(kept);
    }
    for (id, target) in targets {
        if !seen.contains(&id.as_str()) && *target > 0 {
            out.push(obj(vec![
                ("catalogId", Value::from(id.as_str())),
                ("quantity", Value::from(*target)),
            ]));
            changed = true;
        }
    }
    changed.then_some(out)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn parse(value: Value) -> Result<SuppliesFile, String> {
        parse_session_supplies(&value.to_string(), &|id| id == "WP_X")
    }

    #[test]
    fn parsing() {
        assert!(parse_session_supplies("{", &|_| false).is_err());
        assert_eq!(parse(json!({"x": 1})).unwrap_err(), SHAPE);
        assert_eq!(
            parse(json!({"accounts": {"u": 5}})).unwrap_err(),
            "account \"u\": expected an object of catalog ids and quantities"
        );
        assert_eq!(
            parse(json!({"accounts": {"": {}}})).unwrap_err(),
            "account \"\": expected an object of catalog ids and quantities"
        );
        assert_eq!(
            parse(json!({"accounts": {"u": {"a b": 1}}})).unwrap_err(),
            "account u: invalid catalog id \"a b\""
        );
        assert_eq!(
            parse(json!({"accounts": {"u": {"CURRENCY_NOTES": 1}}})).unwrap_err(),
            "account u: CURRENCY_NOTES is a currency (wallet), not a stacked item"
        );
        assert_eq!(
            parse(json!({"accounts": {"u": {"WP_X": 1}}})).unwrap_err(),
            "account u: WP_X is an instanced item, not a stacked one"
        );
        assert_eq!(
            parse(json!({"accounts": {"u": {"A": 1.5}}})).unwrap_err(),
            "account u: quantity of A must be a whole number from 0 to 1000000"
        );
        assert_eq!(
            parse(json!({"accounts": {"u": {"A": 3}}})).unwrap(),
            vec![("u".to_owned(), vec![("A".to_owned(), 3)])]
        );
    }

    #[test]
    fn applying() {
        let targets = vec![("A".to_owned(), 10), ("B".to_owned(), 0), ("C".to_owned(), 3)];
        let stacked = vec![
            json!({"catalogId": "A", "quantity": 4}),
            json!({"catalogId": "B", "quantity": 9}),
            json!({"catalogId": "Z", "quantity": 1}),
            json!({"catalogId": "A", "quantity": 7}),
        ];
        let out = apply_session_supplies(&stacked, &targets).unwrap();
        assert_eq!(
            out,
            vec![
                json!({"catalogId": "A", "quantity": 10}),
                json!({"catalogId": "Z", "quantity": 1}),
                json!({"catalogId": "C", "quantity": 3})
            ]
        );
        assert_eq!(apply_session_supplies(&out, &targets), None);
        assert_eq!(apply_session_supplies(&[], &[("Q".to_owned(), 0)]), None);
    }
}
