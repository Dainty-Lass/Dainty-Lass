// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The RB11 repairs that need the starter values (spec §8.3, §8.4): instance ids for starter items, the
//! banner and glider a loadout points at, and the loadout slot count. Named after Node's
//! `migrate_starter_instance_refs`, `migrate_banner_instance_ids` and `migrate_loadout_slot_entitlements`.

use std::collections::{HashMap, HashSet};

use rustic_isles_characters::StarterKit;
use rustic_isles_store::repo::characters::slots::{
    DEFAULT_ACCOUNT_LOADOUT_SLOTS, MAX_TOTAL_LOADOUT_SLOTS, resolve_player_journey_total, resolve_requested_total,
};
use serde_json::{Map, Value, json};

use crate::ids::{is_instance_id, repair_instance_id};

/// The game-derived values the repairs need (§14.3); the caller builds them for its game version.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LegacyStarter {
    /// Catalog ids a new character gets an instance of
    pub instanced: Vec<String>,
    /// `(slot key, catalog id)`: the kit's equipment, then the flask
    pub equipment: Vec<(String, String)>,
    /// The starter banner's catalog id.
    pub banner: String,
    /// The starter glider's catalog id.
    pub glider: String,
    /// The dye a loadout's empty or missing dye slots get.
    pub default_dye: String,
    /// The sigil a loadout banner without one gets.
    pub default_sigil: String,
}

impl LegacyStarter {
    /// From the characters kit plus the two catalog ids the kit does not carry.
    pub fn from_kit(kit: &StarterKit, default_dye: &str, default_sigil: &str) -> Self {
        let mut equipment = kit.equipment.clone();
        equipment.push(("flask".into(), kit.flask.clone()));
        Self {
            instanced: kit.instanced.clone(),
            equipment,
            banner: kit.banner.clone(),
            glider: kit.glider.clone(),
            default_dye: default_dye.to_owned(),
            default_sigil: default_sigil.to_owned(),
        }
    }
}

/// A loadout's side of the repair: its slots and `persistent`.
pub(crate) struct LoadoutSide<'a> {
    pub slots: &'a mut [Value],
    pub persistent: &'a mut Value,
}

/// §8.3 for one character: item ids, then (with a loadout) seeded banner and glider, slots and `persistent`.
pub(crate) fn repair_starter(
    starter: &LegacyStarter,
    user: &str,
    character: &str,
    items: &mut Vec<Value>,
    loadout: Option<LoadoutSide<'_>>,
) {
    let mut seen: HashSet<String> = HashSet::new();
    let mut by_catalog: HashMap<String, String> = HashMap::new();
    let fresh = |seen: &HashSet<String>, catalog: &str, mut ordinal: usize| loop {
        let id = repair_instance_id(user, character, catalog, ordinal);
        if !seen.contains(&id) {
            break id;
        }
        ordinal += 1;
    };
    for (position, item) in items.iter_mut().enumerate() {
        let Some(object) = item.as_object_mut() else { continue };
        let Some(catalog) = object.get("catalogId").and_then(Value::as_str).map(str::to_owned) else {
            continue;
        };
        let current = object
            .get("instanceId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let valid = is_instance_id(&current);
        let id = if valid && !seen.contains(&current) {
            current
        } else if !valid || starter.instanced.contains(&catalog) {
            fresh(&seen, &catalog, position)
        } else {
            current
        };
        object.insert("instanceId".into(), Value::String(id.clone()));
        seen.insert(id.clone());
        by_catalog.entry(catalog).or_insert(id);
    }
    let Some(loadout) = loadout else { return };
    for catalog in [&starter.banner, &starter.glider] {
        // A version with no such item (an empty id) seeds nothing
        let present = items
            .iter()
            .any(|i| i.get("catalogId").and_then(Value::as_str) == Some(catalog.as_str()));
        if catalog.is_empty() || present {
            continue;
        }
        let id = fresh(&seen, catalog, items.len());
        items.push(json!({
            "instanceId": id,
            "catalogId": catalog,
            "grantDate": crate::read::DEFAULT_TIME,
            "markedNew": false,
            "craftingTrackId": "",
            "durability": 100,
            "currentXp": 0,
            "surged": 0,
            "talents": vec![0; 25],
        }));
        seen.insert(id.clone());
        by_catalog.entry(catalog.clone()).or_insert(id);
    }
    let valid_seen = |v: Option<&Value>| {
        v.and_then(Value::as_str)
            .is_some_and(|s| is_instance_id(s) && seen.contains(s))
    };
    for slot in loadout.slots.iter_mut() {
        let Some(slot) = slot.as_object_mut() else { continue };
        for (key, catalog) in &starter.equipment {
            let Some(id) = by_catalog.get(catalog) else { continue };
            if let Some(Value::Object(member)) = slot.get_mut(key)
                && !valid_seen(member.get("instance_id"))
            {
                member.insert("instance_id".into(), Value::String(id.clone()));
            }
        }
    }
    let Some(persistent) = loadout.persistent.as_object_mut() else {
        return;
    };
    let non_empty = |v: Option<&Value>| v.and_then(Value::as_str).is_some_and(|s| !s.is_empty());
    if let Some(banner) = by_catalog.get(&starter.banner) {
        with_object(persistent, "banner", |object| {
            for key in ["fabric_id", "pole_id"] {
                if !valid_seen(object.get(key)) {
                    object.insert(key.into(), Value::String(banner.clone()));
                }
            }
            if !non_empty(object.get("sigil_id")) {
                object.insert("sigil_id".into(), Value::String(starter.default_sigil.clone()));
            }
            for key in [
                "dye_aspect1_id",
                "dye_aspect2_id",
                "sigil_PrimaryDyeId",
                "sigil_SecondaryDyeId",
            ] {
                if !non_empty(object.get(key)) {
                    object.insert(key.into(), Value::String(starter.default_dye.clone()));
                }
            }
            for key in ["border_id", "title_id", "landing_id"] {
                object.entry(key).or_insert_with(|| Value::String(String::new()));
            }
            object.entry("update_version").or_insert(json!(0));
        });
    }
    if let Some(glider) = by_catalog.get(&starter.glider) {
        with_object(persistent, "glider", |object| {
            if !valid_seen(object.get("instance_id")) {
                object.insert("instance_id".into(), Value::String(glider.clone()));
            }
            for key in ["PrimaryDyeId", "SecondaryDyeId"] {
                if !non_empty(object.get(key)) {
                    object.insert(key.into(), Value::String(starter.default_dye.clone()));
                }
            }
            object
                .entry("Aspect1Id")
                .or_insert_with(|| Value::String(String::new()));
            object.entry("update_version").or_insert(json!(0));
        });
    }
}

/// `f` on `persistent[key]` as an object, made `{}` when absent or not one: the legacy 0.1.x shape, so a 1.14.7
/// instance id string is replaced (parity, §17 item 2).
fn with_object(persistent: &mut Map<String, Value>, key: &str, f: impl FnOnce(&mut Map<String, Value>)) {
    let slot = persistent.entry(key).or_insert_with(|| Value::Object(Map::new()));
    if !slot.is_object() {
        *slot = Value::Object(Map::new());
    }
    if let Value::Object(map) = slot {
        f(map);
    }
}

/// §8.4: the slot count the journey and the persisted total allow, the list grown to it, the index clamped.
/// Returns `(unlocked_total_slots, active_index)`.
pub(crate) fn reconcile_slots(
    slots: &mut Vec<Value>,
    journey: Option<&Value>,
    persisted: Option<f64>,
    active_index: i64,
) -> (i64, i64) {
    let max = i64::from(MAX_TOTAL_LOADOUT_SLOTS);
    let journey_total = i64::from(resolve_player_journey_total(journey));
    let persisted = persisted.map(|p| (p as i64).clamp(1, max));
    let target = resolve_requested_total(persisted, journey_total)
        .map_or(i64::from(DEFAULT_ACCOUNT_LOADOUT_SLOTS), i64::from)
        .clamp(1, max);
    if let Some(first) = slots.first().cloned() {
        while i64::try_from(slots.len()).unwrap_or(i64::MAX) < target {
            let mut copy = first.clone();
            if let Some(object) = copy.as_object_mut() {
                object.insert("custom_name".into(), json!(""));
                object.insert("update_version".into(), json!(0));
                object.insert("slot_index".into(), json!(slots.len()));
            }
            slots.push(copy);
        }
    }
    let len = i64::try_from(slots.len()).unwrap_or(i64::MAX);
    let active = if len == 0 { 0 } else { active_index.clamp(0, len - 1) };
    (len.max(target), active)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn starter() -> LegacyStarter {
        LegacyStarter {
            instanced: vec!["WEAPON".into()],
            equipment: vec![("weapon".into(), "WEAPON".into())],
            banner: "BANNER".into(),
            glider: "GLIDER".into(),
            default_dye: "DYE".into(),
            default_sigil: "SIGIL".into(),
        }
    }

    #[test]
    fn starter_items_get_ids_and_slots_follow() {
        let mut items = vec![
            json!({"catalogId": "WEAPON", "instanceId": "bad-id"}),
            json!({"catalogId": "OTHER", "instanceId": "AAAAAAAAAAAAAAAAAAAAAAAAAA"}),
            json!({"catalogId": "OTHER", "instanceId": "AAAAAAAAAAAAAAAAAAAAAAAAAA"}),
            json!("not an item"),
        ];
        let mut slots = vec![json!({"weapon": {"instance_id": "stale"}, "flask": "kept"})];
        let mut persistent = json!({"banner": "STRINGID"});
        repair_starter(
            &starter(),
            "u1",
            "c1",
            &mut items,
            Some(LoadoutSide {
                slots: &mut slots,
                persistent: &mut persistent,
            }),
        );
        let weapon = items[0]["instanceId"].as_str().unwrap().to_owned();
        assert_eq!(weapon, repair_instance_id("u1", "c1", "WEAPON", 0));
        // A valid duplicate of a non-starter item is kept
        assert_eq!(items[2]["instanceId"], "AAAAAAAAAAAAAAAAAAAAAAAAAA");
        assert_eq!(items.len(), 6);
        assert_eq!(items[4]["catalogId"], "BANNER");
        assert_eq!(items[4]["instanceId"], repair_instance_id("u1", "c1", "BANNER", 4));
        assert_eq!(slots[0]["weapon"]["instance_id"], Value::String(weapon));
        assert_eq!(persistent["banner"]["fabric_id"], items[4]["instanceId"]);
        assert_eq!(persistent["banner"]["sigil_id"], "SIGIL");
        assert_eq!(persistent["glider"]["PrimaryDyeId"], "DYE");
        assert_eq!(persistent["glider"]["instance_id"], items[5]["instanceId"]);
    }

    #[test]
    fn slots_follow_the_journey() {
        let journey = json!({"LoadoutSlot_01": {"node_status": 2}, "LoadoutSlot_02": {"node_status": 2}});
        let mut slots = vec![json!({"slot_index": 0, "custom_name": "Main", "update_version": 4})];
        let (unlocked, active) = reconcile_slots(&mut slots, Some(&journey), None, 99);
        assert_eq!((unlocked, active), (3, 2));
        assert_eq!(slots.len(), 3);
        assert_eq!(
            slots[2],
            json!({"slot_index": 2, "custom_name": "", "update_version": 0})
        );
        let mut empty = Vec::new();
        assert_eq!(reconcile_slots(&mut empty, None, Some(9.7), 3), (6, 0));
        assert!(empty.is_empty());
    }
}
