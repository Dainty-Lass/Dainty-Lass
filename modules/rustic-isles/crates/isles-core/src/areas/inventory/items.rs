//! The item rules of `controllers/inventory.ts`, on the item lists as JSON values: how a transaction changes an
//! inventory's instanced and stacked items, and what it reports back. Pure, so the transaction around it (the ledger,
//! the wallet, the revision-guarded write) stays the only place that touches the database.

use std::collections::HashMap;

use serde_json::{Map, Value};
use service_kit::{js, jsjson};

use super::sha256::{sha256, sha256_hex};

/// Catalog ids that must never be stored (the 1.12 starter dyes the live service doesn't grant).
const INVALID_STARTER_CATALOG_IDS: [&str; 2] = ["DYE_BANNER_BACKGROUND_DEFAULT", "DYE_BANNER_SIGIL_DEFAULT"];

const INSTANCE_ID_ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

fn is_invalid_starter(catalog_id: Option<&Value>) -> bool {
    matches!(catalog_id, Some(Value::String(id)) if INVALID_STARTER_CATALOG_IDS.contains(&id.as_str()))
}

/// `catalogId.startsWith("CURRENCY_")`: every such add and remove is applied to the wallet; the stacked blob holds none.
pub fn is_wallet_currency_id(catalog_id: Option<&Value>) -> bool {
    matches!(catalog_id, Some(Value::String(id)) if id.starts_with("CURRENCY_"))
}

/// A JavaScript number as a JSON value: whole numbers without a fraction.
pub fn number_value(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0 {
        Value::from(n as i64)
    } else {
        Value::from(n)
    }
}

fn is_safe_integer(value: Option<&Value>) -> bool {
    js::safe_integer(value).is_some()
}

/// `StableStringify`: the value with its object keys sorted (by UTF-16 code unit, as `Array.sort` does), so a body
/// hashes the same whatever order its client wrote the keys in.
pub fn stable_stringify(value: &Value) -> String {
    match value {
        Value::Array(items) => format!("[{}]", items.iter().map(stable_stringify).collect::<Vec<_>>().join(",")),
        Value::Object(fields) => {
            let mut keys: Vec<&String> = fields.keys().collect();
            keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));
            let parts: Vec<String> = keys
                .into_iter()
                .map(|key| {
                    format!(
                        "{}:{}",
                        jsjson::stringify(&Value::String(key.clone())),
                        stable_stringify(&fields[key])
                    )
                })
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        other => jsjson::stringify(other),
    }
}

/// `ComputeInventoryRequestHash`: the canonical SHA-256 of the whole mutation body, bound to the ledger row so a
/// transaction id replayed with the same body returns the stored result and one reused with another body is refused.
/// A missing collection hashes like an empty one.
pub fn compute_inventory_request_hash(
    add_instanced: &Value,
    add_stacked: &Value,
    remove_instanced: &Value,
    remove_stacked: &Value,
    save_instanced: &Value,
) -> String {
    let or_empty = |value: &Value| {
        if value.is_null() {
            Value::Array(Vec::new())
        } else {
            value.clone()
        }
    };
    let mut canonical = Map::new();
    canonical.insert("addInstancedItems".into(), or_empty(add_instanced));
    canonical.insert("addStackedItems".into(), or_empty(add_stacked));
    canonical.insert("removeInstancedItems".into(), or_empty(remove_instanced));
    canonical.insert("removeStackedItems".into(), or_empty(remove_stacked));
    canonical.insert("saveInstancedItems".into(), or_empty(save_instanced));
    sha256_hex(&stable_stringify(&Value::Object(canonical)))
}

/// `MintInstanceId`: 26 characters of base32, 130 random bits. Mixed from several v4 UUIDs through SHA-256, since a
/// v4 UUID alone has fixed bits.
pub fn mint_instance_id() -> String {
    let mut seed = Vec::with_capacity(48);
    for _ in 0..3 {
        seed.extend_from_slice(crate::game_data::season_window::next_uuid().as_bytes());
    }
    let digest = sha256(&seed);
    let mut bits = String::with_capacity(136);
    for byte in &digest[..17] {
        bits.push_str(&format!("{byte:08b}"));
    }
    (0..26)
        .map(|i| {
            let index = usize::from_str_radix(&bits[i * 5..i * 5 + 5], 2).unwrap_or(0);
            char::from(INSTANCE_ID_ALPHABET[index])
        })
        .collect()
}

/// `NeedsMintedInstanceId`: a game server creates instanced items with a placeholder instanceId (none, empty, or the
/// catalogId) and expects the service to assign the real one.
pub fn needs_minted_instance_id(item: &Value) -> bool {
    let Value::Object(fields) = item else {
        return false;
    };
    match fields.get("instanceId") {
        Some(Value::String(id)) => {
            id.is_empty() || fields.get("catalogId").and_then(Value::as_str) == Some(id.as_str())
        }
        _ => true,
    }
}

/// One element of `NormalizeInstancedItems`: `None` when the item is dropped (not an object, no catalogId or
/// instanceId, a starter id that is never stored).
pub fn normalize_instanced_item(item: &Value) -> Option<Value> {
    let Value::Object(fields) = item else {
        return None;
    };
    let catalog_ok = matches!(fields.get("catalogId"), Some(Value::String(id)) if !id.is_empty())
        && !is_invalid_starter(fields.get("catalogId"));
    let instance_ok = matches!(fields.get("instanceId"), Some(Value::String(id)) if !id.is_empty());
    if !catalog_ok || !instance_ok {
        return None;
    }
    let mut normalized = fields.clone();
    if js::is_nullish(fields.get("itemData")) {
        normalized.insert("itemData".into(), Value::Null);
    }
    if !js::is_number(fields.get("updateVersion")) {
        normalized.insert("updateVersion".into(), Value::from(0));
    }
    Some(Value::Object(normalized))
}

fn remove_instanced_items(instanced: &mut Vec<Value>, to_remove: &[Value]) {
    let ids: Vec<&str> = to_remove
        .iter()
        .filter_map(|item| item.get("instanceId").and_then(Value::as_str))
        .filter(|id| !id.is_empty())
        .collect();
    if ids.is_empty() {
        return;
    }
    for item in to_remove {
        let id = item.get("instanceId").and_then(Value::as_str);
        if id.is_some()
            && id == item.get("catalogId").and_then(Value::as_str)
            && !instanced
                .iter()
                .any(|stored| stored.get("instanceId").and_then(Value::as_str) == id)
        {
            tracing::warn!(
                "[InstanceId] remove references placeholder {} that matches no stored item - the server did not adopt a minted id",
                id.unwrap_or_default()
            );
        }
    }
    instanced.retain(|item| {
        item.get("instanceId")
            .and_then(Value::as_str)
            .is_none_or(|id| !ids.contains(&id))
    });
}

/// `ApplyItemDataSave`: the monotonic updateVersion guard shared by the batch save and the single-item endpoint.
/// `Some(item)` is the item to store; `None` means do not write (stale, or nothing changed).
pub fn apply_item_data_save(
    existing: &Value,
    incoming_item_data: Option<&Value>,
    incoming_update_version: Option<&Value>,
) -> Option<Value> {
    let existing_version = existing
        .get("updateVersion")
        .filter(|v| v.is_number())
        .map_or(0.0, |v| js::number(Some(v)));
    let incoming_version = match incoming_update_version {
        Some(version) if version.is_number() => js::number(Some(version)),
        _ => existing_version,
    };
    let null = Value::Null;
    let next_item_data = match incoming_item_data {
        None => &null,
        Some(data) => data,
    };
    let existing_item_data = match existing.get("itemData") {
        None => &null,
        Some(data) => data,
    };
    let id = js::string(existing.get("instanceId"));
    let catalog = js::string(existing.get("catalogId"));
    if incoming_version < existing_version {
        tracing::warn!(
            "Ignoring stale itemData save: updateVersion {incoming_version} < stored {existing_version} for instanceId {id} (catalogId {catalog})"
        );
        return None;
    }
    if incoming_version == existing_version {
        if js::strict_eq(Some(next_item_data), Some(existing_item_data)) {
            return None;
        }
        tracing::warn!(
            "Same-version divergent itemData save for instanceId {id} (catalogId {catalog}) at updateVersion {incoming_version} - applying, flagged for review"
        );
    }
    let mut next = existing.as_object()?.clone();
    next.insert("itemData".into(), next_item_data.clone());
    next.insert("updateVersion".into(), number_value(incoming_version));
    Some(Value::Object(next))
}

/// Thrown when a removeStackedItems entry asks to remove more of a non-currency catalogId than the character owns.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("Insufficient stacked item: user {user_id} has {owned} of {catalog_id}, cannot remove {requested}")]
pub struct InsufficientStackedItem {
    pub user_id: String,
    pub catalog_id: String,
    pub requested: f64,
    pub owned: f64,
}

fn quantity_or_zero(value: Option<&Value>) -> f64 {
    let n = js::number(value);
    if n.is_nan() { 0.0 } else { n }
}

/// `ValidateStackedItemRemovals`, against the pre-transaction snapshot so a request can never fund its own removal.
fn validate_stacked_item_removals(
    user_id: &str,
    stacked: &[Value],
    to_remove: &[Value],
) -> Result<(), InsufficientStackedItem> {
    let mut owned: HashMap<&str, f64> = HashMap::new();
    for item in stacked {
        if let Some(id) = item.get("catalogId").and_then(Value::as_str) {
            *owned.entry(id).or_default() += quantity_or_zero(item.get("quantity"));
        }
    }
    let mut requested: Vec<(&str, f64)> = Vec::new();
    for item in to_remove {
        let Some(id) = item.get("catalogId").and_then(Value::as_str) else {
            continue;
        };
        if id.starts_with("CURRENCY_") {
            continue;
        }
        let quantity = if js::is_number(item.get("quantity")) {
            js::number(item.get("quantity"))
        } else {
            0.0
        };
        match requested.iter_mut().find(|(known, _)| *known == id) {
            Some((_, total)) => *total += quantity,
            None => requested.push((id, quantity)),
        }
    }
    for (id, quantity) in requested {
        let have = owned.get(id).copied().unwrap_or(0.0);
        if quantity > have {
            return Err(InsufficientStackedItem {
                user_id: user_id.to_owned(),
                catalog_id: id.to_owned(),
                requested: quantity,
                owned: have,
            });
        }
    }
    Ok(())
}

/// What a transaction asks, after the grant normalisation and the test multiplier.
pub struct Delta<'a> {
    pub add_instanced: &'a [Value],
    pub add_stacked: &'a [Value],
    pub remove_instanced: &'a [Value],
    pub remove_stacked: &'a [Value],
    pub save_instanced: &'a [Value],
}

#[derive(Debug, Clone, PartialEq)]
pub struct Applied {
    pub instanced: Vec<Value>,
    pub stacked: Vec<Value>,
    pub created_instanced: Vec<Value>,
    pub updated_instanced: Vec<Value>,
    pub updated_stacked: Vec<Value>,
    /// The stacked adds that persisted (for the ledger annotator)
    pub accepted_stacked_adds: Vec<Value>,
}

enum Updated {
    /// Node pushes the stored object itself, so a later removal in the same call shows in what was reported
    Stored(usize),
    Plain(Value),
}

fn catalog_id_of(item: &Value) -> Option<&str> {
    item.get("catalogId").and_then(Value::as_str)
}

/// The mutation step of `RunInventoryTransaction`, in Node's order: instanced adds, stacked adds, removals, the
/// removed-stack and wallet reports, the stale currency rows dropped, then the saves. `touched_balances` is what the
/// wallet step returned (post-transaction balance of every currency the call touched, in the order it touched them).
pub fn apply_transaction(
    user_id: &str,
    mut instanced: Vec<Value>,
    stacked: Vec<Value>,
    delta: &Delta<'_>,
    touched_balances: &Map<String, Value>,
    mint: &mut dyn FnMut() -> String,
    transaction_id: &str,
) -> Result<Applied, InsufficientStackedItem> {
    validate_stacked_item_removals(user_id, &stacked, delta.remove_stacked)?;

    let mut created = Vec::new();
    let mut updated_instanced = Vec::new();
    let mut accepted = Vec::new();

    for new_item in delta.add_instanced {
        let mut item = new_item.clone();
        if needs_minted_instance_id(&item) && matches!(item.get("catalogId"), Some(Value::String(_))) {
            let placeholder = jsjson::stringify(item.get("instanceId").unwrap_or(&Value::Null));
            let minted = mint();
            if let Value::Object(fields) = &mut item {
                fields.insert("instanceId".into(), Value::String(minted.clone()));
            }
            tracing::info!(
                "[InstanceId] minted {minted} for {} (server sent {}) transactionId {transaction_id}",
                js::string(item.get("catalogId")),
                placeholder
            );
        }
        if let Some(normalized) = normalize_instanced_item(&item) {
            remove_instanced_items(&mut instanced, std::slice::from_ref(&normalized));
            instanced.push(normalized.clone());
            created.push(normalized);
        }
    }

    // The stacked blob as objects that stay addressable by index while they are reported
    let mut arena: Vec<Value> = stacked;
    let mut order: Vec<usize> = (0..arena.len()).collect();
    let mut by_catalog: HashMap<String, usize> = HashMap::new();
    for (index, item) in arena.iter().enumerate() {
        if let Some(id) = catalog_id_of(item) {
            by_catalog.insert(id.to_owned(), index);
        }
    }
    let mut updated: Vec<Updated> = Vec::new();

    for new_stacked in delta.add_stacked {
        if is_invalid_starter(new_stacked.get("catalogId")) {
            continue;
        }
        if matches!(new_stacked.get("catalogId"), Some(Value::String(_)))
            && is_safe_integer(new_stacked.get("quantity"))
            && js::number(new_stacked.get("quantity")) > 0.0
        {
            let mut entry = Map::new();
            entry.insert("catalogId".into(), new_stacked["catalogId"].clone());
            entry.insert("quantity".into(), new_stacked["quantity"].clone());
            accepted.push(Value::Object(entry));
        }
        // Currency was applied to the wallet and is reported from there
        if is_wallet_currency_id(new_stacked.get("catalogId")) {
            continue;
        }
        let Some(id) = catalog_id_of(new_stacked) else {
            // `Map.get(undefined)` finds nothing: a stack with no usable catalogId is stored as it came
            arena.push(new_stacked.clone());
            order.push(arena.len() - 1);
            updated.push(Updated::Stored(arena.len() - 1));
            continue;
        };
        match by_catalog.get(id).copied() {
            Some(index) => {
                let sum = js::number(arena[index].get("quantity")) + js::number(new_stacked.get("quantity"));
                if let Value::Object(fields) = &mut arena[index] {
                    fields.insert("quantity".into(), number_value(sum));
                }
                updated.push(Updated::Stored(index));
            }
            None => {
                arena.push(new_stacked.clone());
                let index = arena.len() - 1;
                order.push(index);
                by_catalog.insert(id.to_owned(), index);
                updated.push(Updated::Stored(index));
            }
        }
    }

    remove_instanced_items(&mut instanced, delta.remove_instanced);

    if !delta.remove_stacked.is_empty() {
        for to_remove in delta.remove_stacked {
            let Some(id) = catalog_id_of(to_remove) else {
                continue;
            };
            let Some(&index) = order.iter().find(|&&i| catalog_id_of(&arena[i]) == Some(id)) else {
                continue;
            };
            let current = js::number(arena[index].get("quantity"));
            let by = if js::is_number(to_remove.get("quantity")) {
                js::number(to_remove.get("quantity"))
            } else {
                current
            };
            if let Value::Object(fields) = &mut arena[index] {
                fields.insert("quantity".into(), number_value(current - by));
            }
        }
        order.retain(|&i| js::number(arena[i].get("quantity")) > 0.0);
    }

    // The post-removal quantity of every non-currency stack this call removed, so the client updates its counts
    let mut reported: Vec<String> = updated
        .iter()
        .filter_map(|entry| match entry {
            Updated::Stored(i) => catalog_id_of(&arena[*i]).map(str::to_owned),
            Updated::Plain(value) => catalog_id_of(value).map(str::to_owned),
        })
        .collect();
    let mut after_removal: HashMap<&str, usize> = HashMap::new();
    for &i in &order {
        if let Some(id) = catalog_id_of(&arena[i]) {
            after_removal.insert(id, i);
        }
    }
    let mut report_removed = Vec::new();
    for removed in delta.remove_stacked {
        let Some(id) = catalog_id_of(removed) else {
            continue;
        };
        if id.is_empty() || id.starts_with("CURRENCY_") || reported.iter().any(|known| known == id) {
            continue;
        }
        reported.push(id.to_owned());
        let quantity = after_removal.get(id).map_or(Value::from(0), |&i| {
            arena[i].get("quantity").cloned().unwrap_or(Value::Null)
        });
        let mut entry = Map::new();
        entry.insert("catalogId".into(), Value::String(id.to_owned()));
        entry.insert("quantity".into(), quantity);
        report_removed.push(Updated::Plain(Value::Object(entry)));
    }
    updated.extend(report_removed);

    for (catalog_id, balance) in touched_balances {
        let mut entry = Map::new();
        entry.insert("catalogId".into(), Value::String(catalog_id.clone()));
        entry.insert("quantity".into(), balance.clone());
        updated.push(Updated::Plain(Value::Object(entry)));
    }

    // The currency rows older transactions left in the blob; the wallet holds currency
    order.retain(|&i| !is_wallet_currency_id(arena[i].get("catalogId")));

    for saved in delta.save_instanced {
        let Some(item) = normalize_instanced_item(saved) else {
            continue;
        };
        let instance_id = item.get("instanceId");
        let Some(index) = instanced
            .iter()
            .position(|stored| js::strict_eq(stored.get("instanceId"), instance_id))
        else {
            // A save must reference an item the character already owns; it is never an implicit grant
            tracing::warn!(
                "Ignoring save for unknown instanceId {} (catalogId {}) - not owned by this character (userId {user_id})",
                js::string(instance_id),
                js::string(item.get("catalogId"))
            );
            continue;
        };
        if !js::strict_eq(instanced[index].get("catalogId"), item.get("catalogId")) {
            tracing::warn!(
                "Rejecting save that would change catalogId {} -> {} for instanceId {} (userId {user_id})",
                js::string(instanced[index].get("catalogId")),
                js::string(item.get("catalogId")),
                js::string(instanced[index].get("instanceId"))
            );
            continue;
        }
        if let Some(next) = apply_item_data_save(&instanced[index], item.get("itemData"), item.get("updateVersion")) {
            instanced[index] = next.clone();
            updated_instanced.push(next);
        }
    }

    let updated_stacked = updated
        .into_iter()
        .map(|entry| match entry {
            Updated::Stored(i) => arena[i].clone(),
            Updated::Plain(value) => value,
        })
        .collect();
    let stacked = order.into_iter().map(|i| arena[i].clone()).collect();
    Ok(Applied {
        instanced,
        stacked,
        created_instanced: created,
        updated_instanced,
        updated_stacked,
        accepted_stacked_adds: accepted,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn apply(instanced: Vec<Value>, stacked: Vec<Value>, delta: Delta<'_>) -> Result<Applied, InsufficientStackedItem> {
        let mut next = 0;
        apply_transaction(
            "u",
            instanced,
            stacked,
            &delta,
            &Map::new(),
            &mut || {
                next += 1;
                format!("MINTED{next}")
            },
            "tx",
        )
    }

    const NONE: &[Value] = &[];

    fn delta<'a>(
        add_instanced: &'a [Value],
        add_stacked: &'a [Value],
        remove_instanced: &'a [Value],
        remove_stacked: &'a [Value],
        save_instanced: &'a [Value],
    ) -> Delta<'a> {
        Delta {
            add_instanced,
            add_stacked,
            remove_instanced,
            remove_stacked,
            save_instanced,
        }
    }

    #[test]
    fn the_request_hash_ignores_key_order_and_treats_missing_as_empty() {
        let a = compute_inventory_request_hash(
            &json!([{"catalogId": "X", "instanceId": "i", "itemData": null}]),
            &Value::Null,
            &Value::Null,
            &Value::Null,
            &Value::Null,
        );
        let b = compute_inventory_request_hash(
            &json!([{"itemData": null, "instanceId": "i", "catalogId": "X"}]),
            &json!([]),
            &json!([]),
            &json!([]),
            &json!([]),
        );
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        // Node: sha256 of the sorted-key JSON text
        assert_eq!(
            compute_inventory_request_hash(&Value::Null, &Value::Null, &Value::Null, &Value::Null, &Value::Null),
            sha256_hex(
                r#"{"addInstancedItems":[],"addStackedItems":[],"removeInstancedItems":[],"removeStackedItems":[],"saveInstancedItems":[]}"#
            )
        );
        assert_eq!(
            stable_stringify(&json!({"b": 1, "a": {"d": [2, {"z": 1, "y": null}], "c": "é"}})),
            r#"{"a":{"c":"é","d":[2,{"y":null,"z":1}]},"b":1}"#
        );
    }

    #[test]
    fn a_minted_id_is_26_base32_characters() {
        let id = mint_instance_id();
        assert_eq!(id.len(), 26);
        assert!(id.bytes().all(|b| INSTANCE_ID_ALPHABET.contains(&b)));
        assert_ne!(id, mint_instance_id());
    }

    #[test]
    fn placeholder_ids_are_told() {
        assert!(needs_minted_instance_id(
            &json!({"catalogId": "TOKEN_CELL_EXCHANGE", "instanceId": "TOKEN_CELL_EXCHANGE"})
        ));
        assert!(needs_minted_instance_id(&json!({"catalogId": "X"})));
        assert!(needs_minted_instance_id(&json!({"catalogId": "X", "instanceId": ""})));
        assert!(!needs_minted_instance_id(
            &json!({"catalogId": "X", "instanceId": "REAL"})
        ));
        assert!(!needs_minted_instance_id(&json!("x")));
    }

    #[test]
    fn instanced_adds_get_minted_ids_and_replace_a_same_id_item() {
        let add = [
            json!({"catalogId": "WP_X", "instanceId": "WP_X"}),
            json!({"catalogId": "WP_X", "instanceId": "WP_X", "extra": true}),
            json!({"catalogId": "LT", "instanceId": "KEEP", "itemData": "d", "updateVersion": 4}),
            json!({"catalogId": "DYE_BANNER_SIGIL_DEFAULT", "instanceId": "S"}),
            json!({"catalogId": "", "instanceId": "S2"}),
            json!(7),
        ];
        let applied = apply(
            vec![json!({"catalogId": "OLD", "instanceId": "KEEP", "itemData": null, "updateVersion": 0})],
            vec![],
            delta(&add, NONE, NONE, NONE, NONE),
        )
        .unwrap();
        assert_eq!(
            applied.created_instanced,
            vec![
                json!({"catalogId": "WP_X", "instanceId": "MINTED1", "itemData": null, "updateVersion": 0}),
                json!({"catalogId": "WP_X", "instanceId": "MINTED2", "extra": true, "itemData": null, "updateVersion": 0}),
                json!({"catalogId": "LT", "instanceId": "KEEP", "itemData": "d", "updateVersion": 4}),
            ]
        );
        // KEEP was replaced, and moved to the end
        let ids: Vec<&str> = applied
            .instanced
            .iter()
            .map(|i| i["instanceId"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["MINTED1", "MINTED2", "KEEP"]);
        assert_eq!(applied.instanced[2]["catalogId"], "LT");
    }

    #[test]
    fn stacked_adds_merge_and_report_the_stack_that_holds_them() {
        let add = [
            json!({"catalogId": "MAT", "quantity": 3}),
            json!({"catalogId": "NEW", "quantity": 2, "source": "x"}),
            json!({"catalogId": "CURRENCY_NOTES", "quantity": 50}),
            json!({"catalogId": "DYE_BANNER_BACKGROUND_DEFAULT", "quantity": 1}),
        ];
        let applied = apply(
            vec![],
            vec![
                json!({"catalogId": "MAT", "quantity": 4}),
                json!({"catalogId": "CURRENCY_NOTES", "quantity": 9}),
            ],
            delta(NONE, &add, NONE, NONE, NONE),
        )
        .unwrap();
        // the stale currency row is dropped, a new stack keeps the request's extra fields
        assert_eq!(
            applied.stacked,
            vec![
                json!({"catalogId": "MAT", "quantity": 7}),
                json!({"catalogId": "NEW", "quantity": 2, "source": "x"})
            ]
        );
        assert_eq!(applied.updated_stacked, applied.stacked);
        assert_eq!(applied.accepted_stacked_adds.len(), 3);
    }

    #[test]
    fn a_removal_in_the_same_call_shows_in_what_an_add_reported() {
        // Node pushes the stored object, which the removal then changes
        let add = [json!({"catalogId": "MAT", "quantity": 3})];
        let remove = [json!({"catalogId": "MAT", "quantity": 5})];
        let applied = apply(
            vec![],
            vec![json!({"catalogId": "MAT", "quantity": 6})],
            delta(NONE, &add, NONE, &remove, NONE),
        )
        .unwrap();
        assert_eq!(applied.stacked, vec![json!({"catalogId": "MAT", "quantity": 4})]);
        assert_eq!(
            applied.updated_stacked,
            vec![json!({"catalogId": "MAT", "quantity": 4})]
        );

        // consumed in full: the report says 0 and the stack goes
        let remove = [
            json!({"catalogId": "CORE", "quantity": 1}),
            json!({"catalogId": "CURRENCY_NOTES", "quantity": 10}),
        ];
        let applied = apply(
            vec![],
            vec![json!({"catalogId": "CORE", "quantity": 1})],
            delta(NONE, NONE, NONE, &remove, NONE),
        )
        .unwrap();
        assert!(applied.stacked.is_empty());
        assert_eq!(
            applied.updated_stacked,
            vec![json!({"catalogId": "CORE", "quantity": 0})]
        );
    }

    #[test]
    fn removing_more_than_is_owned_is_refused_against_the_pre_transaction_snapshot() {
        let add = [json!({"catalogId": "CORE", "quantity": 5})];
        let remove = [
            json!({"catalogId": "CORE", "quantity": 2}),
            json!({"catalogId": "CORE", "quantity": 1}),
        ];
        // owns 2, asks 3: the add in the same request doesn't fund it
        let error = apply(
            vec![],
            vec![json!({"catalogId": "CORE", "quantity": 2})],
            delta(NONE, &add, NONE, &remove, NONE),
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "Insufficient stacked item: user u has 2 of CORE, cannot remove 3"
        );
        // currency is the wallet's to guard; an entry without a quantity removes the whole stack
        let remove = [
            json!({"catalogId": "CURRENCY_NOTES", "quantity": 99999}),
            json!({"catalogId": "OWN"}),
        ];
        assert!(
            apply(
                vec![],
                vec![json!({"catalogId": "OWN", "quantity": 3})],
                delta(NONE, NONE, NONE, &remove, NONE)
            )
            .is_ok()
        );
    }

    #[test]
    fn saves_are_monotonic_and_never_change_what_an_item_is() {
        let stored = json!({"catalogId": "WP", "instanceId": "i", "itemData": "old", "updateVersion": 3});
        let save = |item: Value| {
            let items = [item];
            apply(vec![stored.clone()], vec![], delta(NONE, NONE, NONE, NONE, &items)).unwrap()
        };
        // newer applies
        let applied = save(json!({"catalogId": "WP", "instanceId": "i", "itemData": "new", "updateVersion": 4}));
        assert_eq!(
            applied.updated_instanced,
            vec![json!({"catalogId": "WP", "instanceId": "i", "itemData": "new", "updateVersion": 4})]
        );
        // stale ignored
        assert!(
            save(json!({"catalogId": "WP", "instanceId": "i", "itemData": "new", "updateVersion": 2}))
                .updated_instanced
                .is_empty()
        );
        // equal version and identical data: nothing to report
        assert!(
            save(json!({"catalogId": "WP", "instanceId": "i", "itemData": "old", "updateVersion": 3}))
                .updated_instanced
                .is_empty()
        );
        // equal version, divergent data: applied
        assert_eq!(
            save(json!({"catalogId": "WP", "instanceId": "i", "itemData": "div", "updateVersion": 3}))
                .updated_instanced
                .len(),
            1
        );
        // no version in a save normalises to 0, which is stale against 3
        assert!(
            save(json!({"catalogId": "WP", "instanceId": "i", "itemData": "x"}))
                .updated_instanced
                .is_empty()
        );
        // a catalogId change (transmutation) and an unknown instance are skipped
        assert!(
            save(json!({"catalogId": "BETTER", "instanceId": "i", "itemData": "x", "updateVersion": 9}))
                .updated_instanced
                .is_empty()
        );
        assert!(
            save(json!({"catalogId": "WP", "instanceId": "other", "itemData": "x", "updateVersion": 9}))
                .updated_instanced
                .is_empty()
        );
    }

    #[test]
    fn the_wallets_touched_balances_are_reported_last_in_touch_order() {
        let add = [json!({"catalogId": "MAT", "quantity": 1})];
        let mut touched = Map::new();
        touched.insert("CURRENCY_B".into(), json!(5));
        touched.insert("CURRENCY_A".into(), json!(7));
        let applied = apply_transaction(
            "u",
            vec![],
            vec![],
            &delta(NONE, &add, NONE, NONE, NONE),
            &touched,
            &mut || "M".into(),
            "t",
        )
        .unwrap();
        assert_eq!(
            applied.updated_stacked,
            vec![
                json!({"catalogId": "MAT", "quantity": 1}),
                json!({"catalogId": "CURRENCY_B", "quantity": 5}),
                json!({"catalogId": "CURRENCY_A", "quantity": 7}),
            ]
        );
    }
}
