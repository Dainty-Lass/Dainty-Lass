// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The item rules (spec §7): pure functions over JSON item lists; the transaction around them is the only place
//! that touches the database.

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value};
use service_kit::{js, jsjson};
use sha2::{Digest, Sha256};

use crate::util::{num, num0};

/// 1.12's starter dyes, never stored.
pub const STARTER_DYES: [&str; 2] = ["DYE_BANNER_BACKGROUND_DEFAULT", "DYE_BANNER_SIGIL_DEFAULT"];

const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// Whether the id is a wallet currency (`CURRENCY_`), which the wallet holds instead of the stacked list.
pub fn is_wallet_currency_id(catalog_id: &str) -> bool {
    catalog_id.starts_with("CURRENCY_")
}

pub use crate::util::number_value;

fn is_starter_dye(id: &str) -> bool {
    STARTER_DYES.contains(&id)
}

/// Objects with their keys sorted by UTF-16 code units, every key and scalar as `JSON.stringify` writes it.
pub fn stable_stringify(value: &Value) -> String {
    let mut out = String::new();
    write_stable(&mut out, value);
    out
}

fn write_stable(out: &mut String, value: &Value) {
    match value {
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_stable(out, item);
            }
            out.push(']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            out.push('{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&jsjson::stringify(&Value::String(key.clone())));
                out.push(':');
                write_stable(out, &map[key]);
            }
            out.push('}');
        }
        scalar => out.push_str(&jsjson::stringify(scalar)),
    }
}

fn or_empty(value: &Value) -> Value {
    match value {
        Value::Null => Value::Array(Vec::new()),
        other => other.clone(),
    }
}

/// `ComputeInventoryRequestHash` (§7.1): binds a transaction id to its five collections.
pub fn compute_inventory_request_hash(
    add_instanced: &Value,
    add_stacked: &Value,
    remove_instanced: &Value,
    remove_stacked: &Value,
    save_instanced: &Value,
) -> String {
    let mut map = Map::new();
    map.insert("addInstancedItems".into(), or_empty(add_instanced));
    map.insert("addStackedItems".into(), or_empty(add_stacked));
    map.insert("removeInstancedItems".into(), or_empty(remove_instanced));
    map.insert("removeStackedItems".into(), or_empty(remove_stacked));
    map.insert("saveInstancedItems".into(), or_empty(save_instanced));
    crate::util::sha256_hex(&stable_stringify(&Value::Object(map)))
}

/// `MintInstanceId`: three UUID texts hashed, the first 130 bits as 26 base-32 letters.
pub fn mint_instance_id(next_uuid: &mut dyn FnMut() -> String) -> String {
    let mut hasher = Sha256::new();
    for _ in 0..3 {
        hasher.update(next_uuid().as_bytes());
    }
    let digest = hasher.finalize();
    let mut out = String::with_capacity(26);
    for group in 0..26 {
        let mut index = 0usize;
        for bit in group * 5..group * 5 + 5 {
            let set = digest[bit / 8] >> (7 - bit % 8) & 1;
            index = index << 1 | usize::from(set);
        }
        out.push(char::from(ALPHABET[index]));
    }
    out
}

/// `NeedsMintedInstanceId`: an object whose `instanceId` is not a non-empty string or equals its `catalogId`.
pub fn needs_minted_instance_id(item: &Value) -> bool {
    let Some(map) = item.as_object() else {
        return false;
    };
    match map.get("instanceId") {
        Some(Value::String(id)) => id.is_empty() || map.get("catalogId").and_then(Value::as_str) == Some(id),
        _ => true,
    }
}

/// `NormalizeInstancedItems` for one entry: none when it is dropped.
pub fn normalize_instanced_item(item: &Value) -> Option<Value> {
    let map = item.as_object()?;
    let catalog = map.get("catalogId").and_then(Value::as_str)?;
    if catalog.is_empty() || is_starter_dye(catalog) {
        return None;
    }
    if !map
        .get("instanceId")
        .and_then(Value::as_str)
        .is_some_and(|id| !id.is_empty())
    {
        return None;
    }
    let mut out = map.clone();
    if js::is_nullish(out.get("itemData")) {
        out.insert("itemData".into(), Value::Null);
    }
    if !js::is_number(out.get("updateVersion")) {
        out.insert("updateVersion".into(), Value::from(0));
    }
    Some(Value::Object(out))
}

/// `ApplyItemDataSave` (§7.6): the item to store, or none for "do not write".
pub fn apply_item_data_save(
    stored: &Value,
    item_data: Option<&Value>,
    update_version: Option<&Value>,
) -> Option<Value> {
    let stored_version = match stored.get("updateVersion") {
        Some(v @ Value::Number(_)) => num(Some(v)),
        _ => 0.0,
    };
    let incoming_version = match update_version {
        Some(v @ Value::Number(_)) => num(Some(v)),
        _ => stored_version,
    };
    let incoming = item_data.cloned().unwrap_or(Value::Null);
    let stored_data = stored.get("itemData").cloned().unwrap_or(Value::Null);
    let id = stored
        .get("instanceId")
        .map(|v| js::string(Some(v)))
        .unwrap_or_default();
    if incoming_version < stored_version {
        tracing::warn!(
            "Ignoring stale itemData save for instanceId {id}: version {} is older than the stored {}",
            jsjson::number(incoming_version),
            jsjson::number(stored_version)
        );
        return None;
    }
    if incoming_version == stored_version {
        if js::strict_eq(Some(&incoming), Some(&stored_data)) {
            return None;
        }
        tracing::warn!(
            "Same-version divergent itemData save for instanceId {id} at version {}; applying it for review",
            jsjson::number(incoming_version)
        );
    }
    let mut out = stored.as_object().cloned().unwrap_or_default();
    out.insert("itemData".into(), incoming);
    out.insert("updateVersion".into(), number_value(incoming_version));
    Some(Value::Object(out))
}

/// A removal asked for more of a stacked item than is owned.
#[derive(Debug, Clone, PartialEq)]
pub struct InsufficientStackedItem {
    /// The item's catalog id.
    pub catalog_id: String,
    /// How many are owned.
    pub owned: f64,
    /// How many the removal asked for.
    pub requested: f64,
}

/// What [`apply_transaction`] works on.
pub struct Delta<'a> {
    /// The owner's userId, for the log.
    pub user_id: &'a str,
    /// The transaction id, for the log.
    pub transaction_id: &'a str,
    /// The stored instanced items.
    pub instanced: Vec<Value>,
    /// The stored stacked items.
    pub stacked: Vec<Value>,
    /// Normalized (§8.3)
    pub add_instanced: &'a [Value],
    /// Stacked items to add.
    pub add_stacked: &'a [Value],
    /// As sent (`null` or a non-array is empty)
    pub remove_instanced: &'a Value,
    /// Stacked items to remove, as sent.
    pub remove_stacked: &'a Value,
    /// Item data saves on owned instances, as sent; a save is never a grant.
    pub save_instanced: &'a Value,
    /// The currency step's balances, in touch order
    pub touched: &'a [(String, f64)],
    /// Mints an instance id for an add that needs one.
    pub minter: &'a mut dyn FnMut() -> String,
}

/// A transaction's result.
#[derive(Debug, Clone, PartialEq)]
pub struct Applied {
    /// The instanced items to store.
    pub instanced: Vec<Value>,
    /// The stacked items to store, wallet currencies left out.
    pub stacked: Vec<Value>,
    /// The instanced items added, normalized.
    pub created: Vec<Value>,
    /// The instances whose item data a save changed.
    pub updated_instanced: Vec<Value>,
    /// The stacks the answer reports: those changed, those removed, then the touched balances.
    pub updated_stacked: Vec<Value>,
    /// `{catalogId, quantity}` of each stacked add with a positive safe-integer quantity (the annotator's input)
    pub accepted: Vec<Value>,
}

fn list(value: &Value) -> &[Value] {
    value.as_array().map_or(&[], Vec::as_slice)
}

fn catalog(entry: &Value) -> Option<&str> {
    entry.get("catalogId").and_then(Value::as_str)
}

/// `ApplyInventoryTransaction` (§7.3), in Node's order.
pub fn apply_transaction(delta: Delta<'_>) -> Result<Applied, InsufficientStackedItem> {
    let removes = list(delta.remove_stacked);
    check_removals(&delta.stacked, removes)?;
    let mut instanced = delta.instanced;
    let mut created = Vec::new();
    for add in delta.add_instanced {
        let mut add = add.clone();
        if needs_minted_instance_id(&add)
            && let Some(map) = add.as_object_mut()
            && let Some(catalog_id) = map.get("catalogId").and_then(Value::as_str).map(str::to_owned)
        {
            let minted = mint_instance_id(delta.minter);
            let old = map
                .get("instanceId")
                .map_or_else(|| String::from("undefined"), jsjson::stringify);
            tracing::info!(
                "[InstanceId] minted {minted} for {catalog_id} (server sent {old}) transactionId {}",
                delta.transaction_id
            );
            map.insert("instanceId".into(), Value::String(minted));
        }
        let Some(kept) = normalize_instanced_item(&add) else {
            continue;
        };
        let id = kept.get("instanceId").cloned();
        instanced.retain(|stored| stored.get("instanceId") != id.as_ref());
        instanced.push(kept.clone());
        created.push(kept);
    }

    // The stacked list as an arena, so a report keeps pointing at its entry through later changes (as Node's
    // object references do)
    let mut arena: Vec<Value> = delta.stacked;
    let mut live: Vec<usize> = (0..arena.len()).collect();
    let mut index: HashMap<String, usize> = HashMap::new();
    for &i in &live {
        if let Some(id) = catalog(&arena[i]) {
            index.insert(id.to_owned(), i);
        }
    }
    let mut reports: Vec<usize> = Vec::new();
    let mut accepted = Vec::new();
    for add in delta.add_stacked {
        let id = catalog(add);
        if id.is_some_and(is_starter_dye) {
            continue;
        }
        if let Some(id) = id
            && let Some(q) = js::safe_integer(add.get("quantity"))
            && q > 0
        {
            accepted.push(crate::util::obj(vec![
                ("catalogId", Value::from(id)),
                ("quantity", Value::from(q)),
            ]));
        }
        match id {
            Some(id) if is_wallet_currency_id(id) => {}
            None => {
                arena.push(add.clone());
                live.push(arena.len() - 1);
                reports.push(arena.len() - 1);
            }
            Some(id) => match index.get(id) {
                Some(&at) => {
                    let sum = num(arena[at].get("quantity")) + num(add.get("quantity"));
                    if let Some(map) = arena[at].as_object_mut() {
                        map.insert("quantity".into(), number_value(sum));
                    }
                    reports.push(at);
                }
                None => {
                    arena.push(add.clone());
                    let at = arena.len() - 1;
                    live.push(at);
                    index.insert(id.to_owned(), at);
                    reports.push(at);
                }
            },
        }
    }

    remove_instanced(&mut instanced, list(delta.remove_instanced));

    if !removes.is_empty() {
        for removal in removes {
            let Some(id) = catalog(removal) else { continue };
            let Some(&at) = live.iter().find(|&&i| catalog(&arena[i]) == Some(id)) else {
                continue;
            };
            let stored = num(arena[at].get("quantity"));
            let by = match removal.get("quantity") {
                Some(v @ Value::Number(_)) => num(Some(v)),
                _ => stored,
            };
            if let Some(map) = arena[at].as_object_mut() {
                map.insert("quantity".into(), number_value(stored - by));
            }
        }
        live.retain(|&i| num(arena[i].get("quantity")) > 0.0);
    }

    let mut updated_stacked: Vec<Value> = reports.iter().map(|&i| arena[i].clone()).collect();
    let mut reported: HashSet<String> = reports
        .iter()
        .filter_map(|&i| catalog(&arena[i]).map(str::to_owned))
        .collect();
    for removal in removes {
        let Some(id) = catalog(removal).filter(|id| !id.is_empty() && !is_wallet_currency_id(id)) else {
            continue;
        };
        if !reported.insert(id.to_owned()) {
            continue;
        }
        let remaining = live
            .iter()
            .find(|&&i| catalog(&arena[i]) == Some(id))
            .map_or(Value::from(0), |&i| {
                arena[i].get("quantity").cloned().unwrap_or(Value::Null)
            });
        updated_stacked.push(crate::util::obj(vec![
            ("catalogId", Value::from(id)),
            ("quantity", remaining),
        ]));
    }
    for (id, balance) in delta.touched {
        updated_stacked.push(crate::util::obj(vec![
            ("catalogId", Value::from(id.as_str())),
            ("quantity", number_value(*balance)),
        ]));
    }
    live.retain(|&i| !catalog(&arena[i]).is_some_and(is_wallet_currency_id));
    let stacked = live.iter().map(|&i| arena[i].clone()).collect();

    let mut updated_instanced = Vec::new();
    for save in list(delta.save_instanced) {
        let Some(save) = normalize_instanced_item(save) else {
            continue;
        };
        let id = save.get("instanceId");
        let Some(at) = instanced.iter().position(|s| js::strict_eq(s.get("instanceId"), id)) else {
            tracing::warn!(
                "Ignoring save for unknown instanceId {} for userId {} (a save is never a grant)",
                js::string(id),
                delta.user_id
            );
            continue;
        };
        if !js::strict_eq(instanced[at].get("catalogId"), save.get("catalogId")) {
            tracing::warn!(
                "Rejecting save that would change catalogId of instanceId {} from {} to {}",
                js::string(id),
                js::string(instanced[at].get("catalogId")),
                js::string(save.get("catalogId"))
            );
            continue;
        }
        if let Some(next) = apply_item_data_save(&instanced[at], save.get("itemData"), save.get("updateVersion")) {
            instanced[at] = next.clone();
            updated_instanced.push(next);
        }
    }

    Ok(Applied {
        instanced,
        stacked,
        created,
        updated_instanced,
        updated_stacked,
        accepted,
    })
}

/// Removals checked against the stored stacks before any add, so a request never funds its own removal.
fn check_removals(stored: &[Value], removes: &[Value]) -> Result<(), InsufficientStackedItem> {
    let mut owned: HashMap<&str, f64> = HashMap::new();
    for entry in stored {
        if let Some(id) = catalog(entry) {
            *owned.entry(id).or_default() += num0(entry.get("quantity"));
        }
    }
    let mut requested: Vec<(&str, f64)> = Vec::new();
    for removal in removes {
        let Some(id) = catalog(removal).filter(|id| !is_wallet_currency_id(id)) else {
            continue;
        };
        let q = match removal.get("quantity") {
            Some(v @ Value::Number(_)) => num(Some(v)),
            _ => 0.0,
        };
        match requested.iter_mut().find(|(seen, _)| *seen == id) {
            Some((_, sum)) => *sum += q,
            None => requested.push((id, q)),
        }
    }
    for (id, want) in requested {
        let have = owned.get(id).copied().unwrap_or(0.0);
        if want > have {
            return Err(InsufficientStackedItem {
                catalog_id: id.to_owned(),
                owned: have,
                requested: want,
            });
        }
    }
    Ok(())
}

fn remove_instanced(instanced: &mut Vec<Value>, removes: &[Value]) {
    let ids: Vec<&str> = removes
        .iter()
        .filter_map(|r| r.get("instanceId").and_then(Value::as_str))
        .filter(|id| !id.is_empty())
        .collect();
    if ids.is_empty() {
        return;
    }
    for removal in removes {
        let (Some(id), Some(catalog_id)) = (removal.get("instanceId").and_then(Value::as_str), catalog(removal)) else {
            continue;
        };
        if id == catalog_id
            && !instanced
                .iter()
                .any(|s| s.get("instanceId").and_then(Value::as_str) == Some(id))
        {
            tracing::warn!(
                "[InstanceId] remove references placeholder {id} that matches no stored item - the server did not \
                 adopt a minted id"
            );
        }
    }
    instanced.retain(|s| {
        !s.get("instanceId")
            .and_then(Value::as_str)
            .is_some_and(|id| ids.contains(&id))
    });
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn run(
        stacked: Value,
        add: Value,
        remove: Value,
        touched: &[(String, f64)],
    ) -> Result<Applied, InsufficientStackedItem> {
        let mut n = 0;
        let mut minter = move || {
            n += 1;
            format!("00000000-0000-4000-8000-{n:012}")
        };
        apply_transaction(Delta {
            user_id: "u",
            transaction_id: "t",
            instanced: Vec::new(),
            stacked: stacked.as_array().cloned().unwrap(),
            add_instanced: &[],
            add_stacked: add.as_array().unwrap(),
            remove_instanced: &Value::Null,
            remove_stacked: &remove,
            save_instanced: &Value::Null,
            touched,
            minter: &mut minter,
        })
    }

    #[test]
    fn hash() {
        let a = compute_inventory_request_hash(
            &json!([{"b": 1, "a": 2}]),
            &Value::Null,
            &json!([]),
            &Value::Null,
            &Value::Null,
        );
        let b = compute_inventory_request_hash(
            &json!([{"a": 2, "b": 1}]),
            &json!([]),
            &Value::Null,
            &json!([]),
            &json!([]),
        );
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        let empty =
            compute_inventory_request_hash(&Value::Null, &Value::Null, &Value::Null, &Value::Null, &Value::Null);
        assert_eq!(
            empty,
            crate::util::sha256_hex(
                r#"{"addInstancedItems":[],"addStackedItems":[],"removeInstancedItems":[],"removeStackedItems":[],"saveInstancedItems":[]}"#
            )
        );
        assert_eq!(
            stable_stringify(&json!({"é": 1, "b": {"z": [1.5, "ü"], "a": null}})),
            r#"{"b":{"a":null,"z":[1.5,"ü"]},"é":1}"#
        );
    }

    #[test]
    fn mint() {
        let mut fixed = || String::from("6f9619ff-8b86-4d01-b42d-00c04fc964ff");
        let a = mint_instance_id(&mut fixed);
        assert_eq!(a.len(), 26);
        assert!(a.bytes().all(|b| ALPHABET.contains(&b)));
        assert_eq!(a, mint_instance_id(&mut fixed));
        let mut n = 0;
        let mut other = move || {
            n += 1;
            format!("{n}")
        };
        assert_ne!(mint_instance_id(&mut other), mint_instance_id(&mut other));
    }

    #[test]
    fn placeholders() {
        assert!(needs_minted_instance_id(&json!({"catalogId": "A", "instanceId": "A"})));
        assert!(needs_minted_instance_id(&json!({"catalogId": "A"})));
        assert!(needs_minted_instance_id(&json!({"catalogId": "A", "instanceId": ""})));
        assert!(!needs_minted_instance_id(&json!({"catalogId": "A", "instanceId": "B"})));
        assert!(!needs_minted_instance_id(&json!(7)));
    }

    #[test]
    fn stacked_table() {
        let touched = [("CURRENCY_NOTES".to_owned(), 59.0)];
        let out = run(
            json!([{"catalogId": "MAT", "quantity": 4}, {"catalogId": "CURRENCY_NOTES", "quantity": 9}]),
            json!([
                {"catalogId": "MAT", "quantity": 3},
                {"catalogId": "NEW", "quantity": 2, "source": "x"},
                {"catalogId": "CURRENCY_NOTES", "quantity": 50},
                {"catalogId": "DYE_BANNER_BACKGROUND_DEFAULT", "quantity": 1}
            ]),
            Value::Null,
            &touched,
        )
        .unwrap();
        assert_eq!(
            out.stacked,
            vec![
                json!({"catalogId": "MAT", "quantity": 7}),
                json!({"catalogId": "NEW", "quantity": 2, "source": "x"})
            ]
        );
        assert_eq!(
            out.updated_stacked,
            vec![
                json!({"catalogId": "MAT", "quantity": 7}),
                json!({"catalogId": "NEW", "quantity": 2, "source": "x"}),
                json!({"catalogId": "CURRENCY_NOTES", "quantity": 59}),
            ]
        );
        let out = run(
            json!([{"catalogId": "MAT", "quantity": 6}]),
            json!([{"catalogId": "MAT", "quantity": 3}]),
            json!([{"catalogId": "MAT", "quantity": 5}]),
            &[],
        )
        .unwrap();
        assert_eq!(out.stacked, vec![json!({"catalogId": "MAT", "quantity": 4})]);
        assert_eq!(out.updated_stacked, vec![json!({"catalogId": "MAT", "quantity": 4})]);
        let out = run(
            json!([{"catalogId": "CORE", "quantity": 1}]),
            json!([]),
            json!([{"catalogId": "CORE", "quantity": 1}, {"catalogId": "CURRENCY_NOTES", "quantity": 10}]),
            &[],
        )
        .unwrap();
        assert!(out.stacked.is_empty());
        assert_eq!(out.updated_stacked, vec![json!({"catalogId": "CORE", "quantity": 0})]);
        let err = run(
            json!([{"catalogId": "CORE", "quantity": 2}]),
            json!([{"catalogId": "CORE", "quantity": 5}]),
            json!([{"catalogId": "CORE", "quantity": 2}, {"catalogId": "CORE", "quantity": 1}]),
            &[],
        )
        .unwrap_err();
        assert_eq!((err.owned, err.requested), (2.0, 3.0));
        let out = run(
            json!([{"catalogId": "OWN", "quantity": 3}]),
            json!([]),
            json!([{"catalogId": "OWN"}, {"catalogId": "CURRENCY_NOTES", "quantity": 99999}]),
            &[],
        )
        .unwrap();
        assert!(out.stacked.is_empty());
        assert_eq!(out.updated_stacked, vec![json!({"catalogId": "OWN", "quantity": 0})]);
        let touched = [("CURRENCY_B".to_owned(), 5.0), ("CURRENCY_A".to_owned(), 7.0)];
        let out = run(
            json!([]),
            json!([{"catalogId": "MAT", "quantity": 1}]),
            Value::Null,
            &touched,
        )
        .unwrap();
        assert_eq!(
            out.updated_stacked,
            vec![
                json!({"catalogId": "MAT", "quantity": 1}),
                json!({"catalogId": "CURRENCY_B", "quantity": 5}),
                json!({"catalogId": "CURRENCY_A", "quantity": 7}),
            ]
        );
    }

    #[test]
    fn instanced_table() {
        let mut n = 0;
        let mut minter = move || {
            n += 1;
            format!("uuid-{n}")
        };
        let adds = vec![
            json!({"catalogId": "WP_X", "instanceId": "WP_X"}),
            json!({"catalogId": "WP_X", "instanceId": "WP_X", "extra": true}),
            json!({"catalogId": "LT", "instanceId": "KEEP", "itemData": "d", "updateVersion": 4}),
            json!({"catalogId": "DYE_BANNER_SIGIL_DEFAULT", "instanceId": "dye"}),
            json!({"catalogId": "", "instanceId": "x"}),
            json!(7),
        ];
        let out = apply_transaction(Delta {
            user_id: "u",
            transaction_id: "t",
            instanced: vec![json!({"catalogId": "OLD", "instanceId": "KEEP"})],
            stacked: Vec::new(),
            add_instanced: &adds,
            add_stacked: &[],
            remove_instanced: &Value::Null,
            remove_stacked: &Value::Null,
            save_instanced: &Value::Null,
            touched: &[],
            minter: &mut minter,
        })
        .unwrap();
        assert_eq!(out.created.len(), 3);
        let keys: Vec<Vec<&str>> = out
            .created
            .iter()
            .map(|c| c.as_object().unwrap().keys().map(String::as_str).collect())
            .collect();
        assert_eq!(
            keys[1],
            ["catalogId", "instanceId", "extra", "itemData", "updateVersion"]
        );
        assert_eq!(out.created[0]["instanceId"].as_str().unwrap().len(), 26);
        assert_ne!(out.created[0]["instanceId"], out.created[1]["instanceId"]);
        assert_eq!(out.instanced.len(), 3);
        assert_eq!(
            out.instanced[2],
            json!({"catalogId": "LT", "instanceId": "KEEP", "itemData": "d", "updateVersion": 4})
        );
    }

    #[test]
    fn saves() {
        let stored = json!({"catalogId": "WP", "instanceId": "i", "itemData": "old", "updateVersion": 3});
        let save = |data: Option<Value>, v: Option<Value>| apply_item_data_save(&stored, data.as_ref(), v.as_ref());
        assert_eq!(
            save(Some(json!("new")), Some(json!(4))).unwrap()["updateVersion"],
            json!(4)
        );
        assert_eq!(save(Some(json!("new")), Some(json!(2))), None);
        assert_eq!(save(Some(json!("old")), Some(json!(3))), None);
        assert_eq!(
            save(Some(json!("div")), Some(json!(3))).unwrap()["itemData"],
            json!("div")
        );
        assert_eq!(save(Some(json!("new")), Some(json!(0))), None);
        // The single-item route: no data and no version over stored data writes null (§14 item 12)
        assert_eq!(save(None, None).unwrap()["itemData"], Value::Null);
        let mut minter = String::new;
        let mut apply = |s: Value| {
            apply_transaction(Delta {
                user_id: "u",
                transaction_id: "t",
                instanced: vec![stored.clone()],
                stacked: Vec::new(),
                add_instanced: &[],
                add_stacked: &[],
                remove_instanced: &Value::Null,
                remove_stacked: &Value::Null,
                save_instanced: &json!([s]),
                touched: &[],
                minter: &mut minter,
            })
            .unwrap()
            .updated_instanced
        };
        assert_eq!(
            apply(json!({"catalogId": "WP", "instanceId": "i", "itemData": "old"})).len(),
            0
        );
        assert_eq!(
            apply(json!({"catalogId": "BETTER", "instanceId": "i", "updateVersion": 9})).len(),
            0
        );
        assert_eq!(
            apply(json!({"catalogId": "WP", "instanceId": "other", "updateVersion": 9})).len(),
            0
        );
        assert_eq!(
            apply(json!({"catalogId": "WP", "instanceId": "i", "itemData": "new", "updateVersion": 4})).len(),
            1
        );
    }
}
