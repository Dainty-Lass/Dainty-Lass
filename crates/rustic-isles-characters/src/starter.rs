// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The starter records (spec §6.2 to §6.4, §7.1, §7.2): instance ids, the manifest a new character is seeded with,
//! and the repair and reconcile a read runs when a starter record or item is missing.

use std::collections::{HashMap, HashSet};

use dainty_service_kit::jsjson;
use rustic_isles::{VersionTables, replay};
use rustic_isles_store::{RepoError, RepoResult};
use serde_json::{Map, Value, json};

use crate::StarterKit;

const ALPHABET: &[u8; 36] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
/// The length of an instance id: 26 characters from A-Z and 0-9.
pub const INSTANCE_ID_LEN: usize = 26;
/// 7 x 36: a byte at or above it is skipped, so every character is equally likely
const REJECT_AT: u8 = 252;

/// What a new character starts with.
#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    /// One item per starter catalog id, each with a fresh instance id.
    pub instanced_items: Vec<Value>,
    /// The version's stacked starter items.
    pub stacked_items: Vec<Value>,
    /// The one starter loadout slot, `persistent` nested.
    pub loadout_slot: Value,
    /// The account-wide `persistent` object.
    pub persistent: Value,
}

fn uuid_bytes(uuid: &str) -> Vec<u8> {
    let hex: Vec<u8> = uuid.bytes().filter(u8::is_ascii_hexdigit).collect();
    hex.as_chunks::<2>()
        .0
        .iter()
        .filter_map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect()
}

/// 26 of `A-Z0-9` from the replay id source, byte by byte in RFC 4122 order (the goldens pin this order of draws).
pub fn generate_instance_id() -> String {
    let mut id = String::with_capacity(INSTANCE_ID_LEN);
    while id.len() < INSTANCE_ID_LEN {
        for byte in uuid_bytes(&replay::next_uuid()) {
            if id.len() == INSTANCE_ID_LEN {
                break;
            }
            if byte < REJECT_AT {
                id.push(char::from(ALPHABET[usize::from(byte % 36)]));
            }
        }
    }
    id
}

/// Exactly 26 bytes of ASCII upper-case letters and digits.
pub fn is_instance_id(text: &str) -> bool {
    text.len() == INSTANCE_ID_LEN && text.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

fn fresh_id(used: &HashSet<String>) -> String {
    loop {
        let id = generate_instance_id();
        if !used.contains(&id) {
            return id;
        }
    }
}

fn item(catalog_id: &str, instance_id: &str) -> Value {
    json!({"catalogId": catalog_id, "instanceId": instance_id, "itemData": null, "updateVersion": 0})
}

fn stacked_item(catalog_id: &str) -> Value {
    json!({"catalogId": catalog_id, "quantity": 1})
}

fn blanks(n: usize) -> Value {
    Value::Array(vec![Value::from(""); n])
}

fn texts<S: AsRef<str>>(items: &[S]) -> Value {
    Value::Array(items.iter().map(|s| Value::from(s.as_ref())).collect())
}

fn instance_of<'a>(ids: &'a HashMap<String, String>, catalog_id: &str, what: &str) -> RepoResult<&'a str> {
    ids.get(catalog_id)
        .map(String::as_str)
        .ok_or_else(|| RepoError::Data(format!("Missing inventory instance ID for starter {what} {catalog_id}")))
}

/// `persistent` for the chosen instance ids (§6.4).
pub fn persistent_for(kit: &StarterKit, tables: &VersionTables, ids: &HashMap<String, String>) -> RepoResult<Value> {
    let banner = instance_of(ids, &kit.banner, "banner")?;
    let glider = instance_of(ids, &kit.glider, "glider")?;
    let emojis = tables.starter_manifest.quick_emoji_wheel;
    let (quick_chats, emojis) = if emojis.is_empty() {
        (blanks(9), blanks(9))
    } else {
        (texts(&kit.default_quick_chats), texts(emojis))
    };
    let curiosities: Vec<Value> = (0..8)
        .map(|i| json!({"item_index": i, "item_id": "", "instance_id": ""}))
        .collect();
    Ok(json!({
        "manual_emotes": blanks(6),
        "intro_emote": kit.intro_emote,
        "banner": banner,
        "bannerCustomization": jsjson::stringify(&Value::Object(kit.banner_customization.clone())),
        "flare": kit.flare,
        "title": "",
        "head_accessory": "",
        "back_accessory": "",
        "pet": "",
        "glider": glider,
        "update_version": 0,
        "quick_chats": quick_chats,
        "emojis": emojis,
        "quick_curiosities_items": curiosities,
        "quickwheel": [],
    }))
}

/// The one starter slot (§6.4), `persistent` nested.
pub fn slot_for(kit: &StarterKit, ids: &HashMap<String, String>, persistent: Value) -> RepoResult<Value> {
    let mut slot = Map::new();
    for (key, catalog_id) in &kit.equipment {
        let instance = instance_of(ids, catalog_id, "loadout item")?;
        let data = kit
            .instance_data
            .iter()
            .find(|(id, _)| id == catalog_id)
            .map_or(Value::Null, |(_, data)| {
                Value::from(jsjson::stringify(&Value::Object(data.clone())))
            });
        let member = json!({"item_id": catalog_id, "instance_id": instance, "instance_data": data});
        slot.insert(key.clone(), member);
    }
    let flask = instance_of(ids, &kit.flask, "flask")?;
    slot.insert("player_role".into(), Value::Null);
    slot.insert("subweapon".into(), Value::Null);
    let appearance = jsjson::stringify(&Value::Object(kit.default_appearance.clone()));
    slot.insert("appearance".into(), Value::from(appearance));
    slot.insert("flask".into(), Value::from(flask));
    slot.insert("quick_items".into(), Value::Array(Vec::new()));
    slot.insert("slot_index".into(), Value::from(0));
    slot.insert("update_version".into(), Value::from(0));
    slot.insert("custom_name".into(), Value::from(""));
    slot.insert("persistent".into(), persistent);
    Ok(Value::Object(slot))
}

/// The version's stacked starter items, in its table's order.
pub fn stacked_items(tables: &VersionTables) -> Vec<Value> {
    tables
        .starter_manifest
        .stacked_item_ids
        .iter()
        .map(|id| stacked_item(id))
        .collect()
}

/// One instance id per `instanced` catalog id, in order, then the stacked items, `persistent` and one slot.
pub fn build_manifest(kit: &StarterKit, tables: &VersionTables) -> RepoResult<Manifest> {
    let mut ids = HashMap::new();
    let mut instanced_items = Vec::with_capacity(kit.instanced.len());
    for catalog_id in &kit.instanced {
        let instance = generate_instance_id();
        instanced_items.push(item(catalog_id, &instance));
        ids.entry(catalog_id.clone()).or_insert(instance);
    }
    let persistent = persistent_for(kit, tables, &ids)?;
    let loadout_slot = slot_for(kit, &ids, persistent.clone())?;
    Ok(Manifest {
        instanced_items,
        stacked_items: stacked_items(tables),
        loadout_slot,
        persistent,
    })
}

/// "Empty": absent, not an array, or every entry `""` or not a string.
fn empty_wheel(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Array(items)) => items.iter().all(|i| i.as_str().is_none_or(str::is_empty)),
        _ => true,
    }
}

/// Fills an emptied `quick_chats` or `emojis` wheel on the way out, only when the version has quick emojis.
pub fn with_default_chat_wheels(persistent: Value, kit: &StarterKit, tables: &VersionTables) -> Value {
    let emojis = tables.starter_manifest.quick_emoji_wheel;
    let Value::Object(mut map) = persistent else {
        return persistent;
    };
    if emojis.is_empty() {
        return Value::Object(map);
    }
    if empty_wheel(map.get("quick_chats")) {
        map.insert("quick_chats".into(), texts(&kit.default_quick_chats));
    }
    if empty_wheel(map.get("emojis")) {
        map.insert("emojis".into(), texts(emojis));
    }
    Value::Object(map)
}

/// A stored item list; anything but an array reads as `[]` (spec §14 item 8).
pub fn parse_items(text: &str) -> Vec<Value> {
    match jsjson::parse(text) {
        Ok(Value::Array(items)) => items,
        _ => Vec::new(),
    }
}

fn catalog_of(item: &Value) -> Option<&str> {
    item.get("catalogId").and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// The default weapon parts no item carries, in the kit's order.
pub fn missing_parts<'a>(items: &[Value], kit: &'a StarterKit) -> Vec<&'a str> {
    let held: HashSet<&str> = items.iter().filter_map(catalog_of).collect();
    let mut missing: Vec<&str> = Vec::new();
    for id in &kit.default_weapon_parts {
        if !held.contains(id.as_str()) && !missing.contains(&id.as_str()) {
            missing.push(id);
        }
    }
    missing
}

/// The version's stacked ids the list lacks, in the table's order.
pub fn missing_stacked(stacked: &[Value], tables: &VersionTables) -> Vec<&'static str> {
    let held: HashSet<&str> = stacked.iter().filter_map(catalog_of).collect();
    let mut missing: Vec<&'static str> = Vec::new();
    for id in tables.starter_manifest.stacked_item_ids {
        if !held.contains(id) && !missing.contains(id) {
            missing.push(id);
        }
    }
    missing
}

/// Both lists with the missing defaults appended (fresh ids unique against the list's), and what was added.
#[derive(Debug, Clone, PartialEq)]
pub struct Reconciled {
    /// The instanced items, missing default weapon parts appended.
    pub instanced: Vec<Value>,
    /// The stacked items, missing starter stacks appended.
    pub stacked: Vec<Value>,
    /// The catalog ids of the weapon parts appended.
    pub added_parts: Vec<String>,
    /// The catalog ids of the stacked items appended.
    pub added_stacked: Vec<String>,
}

/// Both stored lists (JSON text) with the kit's missing default weapon parts and stacked items appended.
pub fn reconcile(instanced: &str, stacked: &str, kit: &StarterKit, tables: &VersionTables) -> Reconciled {
    let mut items = parse_items(instanced);
    let mut stacks = parse_items(stacked);
    let parts: Vec<String> = missing_parts(&items, kit).into_iter().map(str::to_owned).collect();
    let extra: Vec<String> = missing_stacked(&stacks, tables)
        .into_iter()
        .map(str::to_owned)
        .collect();
    let mut used: HashSet<String> = items
        .iter()
        .filter_map(|i| i.get("instanceId").and_then(Value::as_str).map(str::to_owned))
        .collect();
    for catalog_id in &parts {
        let instance = fresh_id(&used);
        items.push(item(catalog_id, &instance));
        used.insert(instance);
    }
    stacks.extend(extra.iter().map(|id| stacked_item(id)));
    Reconciled {
        instanced: items,
        stacked: stacks,
        added_parts: parts,
        added_stacked: extra,
    }
}

/// The repaired list, the instance id of every `instanced` catalog id, and whether anything changed.
#[derive(Debug, Clone, PartialEq)]
pub struct Repaired {
    /// The items after the repair.
    pub items: Vec<Value>,
    /// Catalog id to instance id for every `instanced` catalog id.
    pub ids: HashMap<String, String>,
    /// Whether the repair changed the list.
    pub changed: bool,
}

fn candidates(source: Option<&Value>, kit: &StarterKit) -> HashMap<String, String> {
    let mut found = HashMap::new();
    let Some(source) = source else {
        return found;
    };
    for (key, catalog_id) in &kit.equipment {
        let Some(member) = source.get(key) else {
            continue;
        };
        let instance = member.get("instance_id").and_then(Value::as_str);
        if member.get("item_id").and_then(Value::as_str) == Some(catalog_id)
            && let Some(instance) = instance.filter(|i| is_instance_id(i))
        {
            found.entry(catalog_id.clone()).or_insert_with(|| instance.to_owned());
        }
    }
    if let Some(flask) = source
        .get("flask")
        .and_then(Value::as_str)
        .filter(|i| is_instance_id(i))
    {
        found.entry(kit.flask.clone()).or_insert_with(|| flask.to_owned());
    }
    found
}

/// Keeps every valid item and gives each `instanced` catalog id without one an id: the source slot's when it is a
/// valid unused instance id, else a fresh unique one.
pub fn repair(items: Vec<Value>, source: Option<&Value>, kit: &StarterKit) -> Repaired {
    let before = items.len();
    let mut items: Vec<Value> = items.into_iter().filter(Value::is_object).collect();
    let mut changed = items.len() != before;
    let mut ids: HashMap<String, String> = HashMap::new();
    let mut used: HashSet<String> = HashSet::new();
    for entry in &items {
        let Some(instance) = entry
            .get("instanceId")
            .and_then(Value::as_str)
            .filter(|i| !i.is_empty())
        else {
            continue;
        };
        if !used.insert(instance.to_owned()) {
            continue;
        }
        if let Some(catalog_id) = entry.get("catalogId").and_then(Value::as_str) {
            ids.entry(catalog_id.to_owned()).or_insert_with(|| instance.to_owned());
        }
    }
    let offered = candidates(source, kit);
    for catalog_id in &kit.instanced {
        if ids.contains_key(catalog_id) {
            continue;
        }
        let instance = match offered.get(catalog_id) {
            Some(candidate) if !used.contains(candidate) => candidate.clone(),
            _ => fresh_id(&used),
        };
        used.insert(instance.clone());
        let target = items
            .iter_mut()
            .find(|entry| entry.get("catalogId").and_then(Value::as_str) == Some(catalog_id));
        match target.and_then(Value::as_object_mut) {
            Some(entry) => {
                entry.insert("instanceId".into(), Value::from(instance.as_str()));
                if !entry.contains_key("itemData") {
                    entry.insert("itemData".into(), Value::Null);
                }
                if !entry.get("updateVersion").is_some_and(Value::is_number) {
                    entry.insert("updateVersion".into(), Value::from(0));
                }
            }
            None => items.push(item(catalog_id, &instance)),
        }
        ids.insert(catalog_id.clone(), instance);
        changed = true;
    }
    Repaired { items, ids, changed }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_ids() {
        for _ in 0..50 {
            let id = generate_instance_id();
            assert!(is_instance_id(&id), "{id}");
        }
        assert!(is_instance_id("ABCDEFGHIJKLMNOPQRSTUVWXY9"));
        assert!(!is_instance_id("ABCDEFGHIJKLMNOPQRSTUVWXYz"));
        assert!(!is_instance_id("ABCDEFGHIJKLMNOPQRSTUVWXY"));
        assert!(!is_instance_id(""));
    }

    #[test]
    fn bytes_of_a_uuid() {
        let bytes = uuid_bytes("00ff10fb-fc00-4000-8000-000000000001");
        assert_eq!(bytes.len(), 16);
        assert_eq!(&bytes[..4], &[0x00, 0xff, 0x10, 0xfb]);
    }
}
