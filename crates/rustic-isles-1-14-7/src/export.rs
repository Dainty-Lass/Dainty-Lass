// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! What the host's own game export supplies to the version data (ruling V-4): the starter weapons' default parts
//! from the weapon-slot export, and a check that every catalog id the version data names is in the catalog export.
//! Both are optional; without the files the shipped facts stand.

use std::collections::HashSet;

use serde_json::{Map, Value};

/// The catalog export's file name in the export folder (spec-catalog-exporter §6.1).
pub const CATALOG_EXPORT: &str = "catalog_1_14_7.jsonl";

/// The weapon-slot export's file name (spec-catalog-exporter §6.3), written only with `EXPORT_WEAPON_SLOTS=1`.
pub const WEAPON_SLOTS_EXPORT: &str = "weapon_slots_1_14_7.jsonl";

fn lines(text: &str) -> impl Iterator<Item = Map<String, Value>> + '_ {
    text.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
        .filter_map(|value| match value {
            Value::Object(row) => Some(row),
            _ => None,
        })
}

/// The catalog export's item ids.
pub fn catalog_ids(catalog: &str) -> HashSet<String> {
    lines(catalog)
        .filter_map(|row| row.get("itemId").and_then(Value::as_str).map(str::to_owned))
        .collect()
}

fn texts(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).map(str::to_owned).collect())
        .unwrap_or_default()
}

/// Sets the facts' `starterKit.defaultWeaponParts` to the default part each starter weapon's slots name in the
/// weapon-slot export, adding any part the kit lacks to `instanced` (a weapon must own its slots' defaults). Starter
/// weapons are the `instanced` ids the catalog names `WP_*`; when the export lacks one, the facts stand. The answer
/// says what happened, for the host's report.
pub fn default_weapon_parts(facts: &mut Value, weapon_slots: &str) -> String {
    let Some(kit) = facts.get_mut("starterKit").and_then(Value::as_object_mut) else {
        return "the facts have no starterKit; the weapon-slot export is not used".to_owned();
    };
    let instanced = texts(kit.get("instanced"));
    let weapons: Vec<&String> = instanced.iter().filter(|id| id.starts_with("WP_")).collect();
    let rows: Vec<Map<String, Value>> = lines(weapon_slots).collect();
    let mut parts: Vec<String> = Vec::new();
    let mut uncovered = Vec::new();
    for weapon in &weapons {
        let Some(row) = rows
            .iter()
            .find(|row| row.get("itemId").and_then(Value::as_str) == Some(weapon.as_str()))
        else {
            uncovered.push(weapon.as_str());
            continue;
        };
        let slots = row
            .get("slots")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        for slot in slots {
            let part = slot
                .get("defaultWeaponPartId")
                .and_then(Value::as_str)
                .unwrap_or_default();
            // The engine writes an unset FName as None
            if !part.is_empty() && part != "None" && !parts.iter().any(|p| p == part) {
                parts.push(part.to_owned());
            }
        }
    }
    if weapons.is_empty() || !uncovered.is_empty() {
        return format!(
            "the weapon-slot export does not cover {}; the shipped default weapon parts stand",
            if uncovered.is_empty() {
                "any starter weapon".to_owned()
            } else {
                uncovered.join(", ")
            }
        );
    }
    let added: Vec<String> = parts.iter().filter(|p| !instanced.contains(p)).cloned().collect();
    if let Some(Value::Array(list)) = kit.get_mut("instanced") {
        list.extend(added.iter().cloned().map(Value::String));
    }
    kit.insert(
        "defaultWeaponParts".to_owned(),
        Value::Array(parts.iter().cloned().map(Value::String).collect()),
    );
    if added.is_empty() {
        format!("default weapon parts from the weapon-slot export: {}", parts.len())
    } else {
        format!(
            "default weapon parts from the weapon-slot export: {} ({} added to the kit: {})",
            parts.len(),
            added.len(),
            added.join(", ")
        )
    }
}

/// Every catalog id the merged version data names that the catalog export lacks, each as `<member>: <id>`.
pub fn missing_ids(document: &Value, catalog: &HashSet<String>) -> Vec<String> {
    let mut named: Vec<(&str, String)> = Vec::new();
    let kit = document.get("starterKit");
    let member = |key: &str| kit.and_then(|k| k.get(key));
    for key in ["instanced", "defaultWeaponParts"] {
        named.extend(texts(member(key)).into_iter().map(|id| (key, id)));
    }
    for key in ["flask", "banner", "glider", "introEmote", "flare"] {
        named.extend(member(key).and_then(Value::as_str).map(|id| (key, id.to_owned())));
    }
    let pair_seconds = |key: &'static str, second: bool| {
        member(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(move |pair| pair.get(usize::from(second)).and_then(Value::as_str))
            .map(move |id| (key, id.to_owned()))
    };
    named.extend(pair_seconds("equipment", true));
    named.extend(pair_seconds("wallet", false));
    if let Some(custom) = member("bannerCustomization").and_then(Value::as_object) {
        named.extend(
            custom
                .values()
                .filter_map(Value::as_str)
                .filter(|id| !id.is_empty())
                .map(|id| ("bannerCustomization", id.to_owned())),
        );
    }
    if let Some(legacy) = document.get("legacyStarter").and_then(Value::as_object) {
        named.extend(
            legacy
                .values()
                .filter_map(Value::as_str)
                .map(|id| ("legacyStarter", id.to_owned())),
        );
    }
    let mut seen = HashSet::new();
    named
        .into_iter()
        .filter(|(_, id)| !catalog.contains(id) && seen.insert(id.clone()))
        .map(|(key, id)| format!("{key}: {id}"))
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a panic is how a test fails")]
mod tests {
    use serde_json::json;

    use super::*;

    fn facts() -> Value {
        json!({ "starterKit": {
            "instanced": ["WP_A", "WP_B", "PART_A_OLD", "OTHER"],
            "defaultWeaponParts": ["PART_A_OLD"],
        }})
    }

    fn slots(weapon: &str, parts: &[&str]) -> String {
        let slots: Vec<Value> = parts.iter().map(|p| json!({ "defaultWeaponPartId": p })).collect();
        format!("{}\n", json!({ "itemId": weapon, "slots": slots }))
    }

    #[test]
    fn the_export_sets_the_parts_when_it_covers_every_starter_weapon() {
        let mut doc = facts();
        let export = slots("WP_A", &["PART_A_OLD", "None", ""]) + &slots("WP_B", &["PART_B_NEW"]) + "not json\n";
        let note = default_weapon_parts(&mut doc, &export);
        assert!(note.contains("1 added to the kit: PART_B_NEW"), "{note}");
        assert_eq!(
            doc["starterKit"]["defaultWeaponParts"],
            json!(["PART_A_OLD", "PART_B_NEW"])
        );
        assert_eq!(doc["starterKit"]["instanced"][4], json!("PART_B_NEW"));
    }

    /// Two lines byte for byte as the MIT exporter writes them, CRLF included (`native/rustic-isles-exporter`,
    /// `tests/weapons.rs`): an unset slot reads `None`, and a slot's default may already be in the kit.
    const EXPORTER_LINES: &str = concat!(
        r#"{"itemId":"WP_TEST_SWORD","instanceId":"inst-1","weaponType":1,"slots":[{"slotIndex":0,"weaponPartType":7,"weaponPartTypeName":"eb_special","defaultWeaponPartId":"PART_TEST_SPECIAL_DEFAULT","catalogLocked":false,"hideFromUI":false,"effectiveLocked":false},{"slotIndex":1,"weaponPartType":8,"weaponPartTypeName":"eb_passive","defaultWeaponPartId":"None","catalogLocked":true,"hideFromUI":true,"effectiveLocked":true}],"equippedParts":[{"slotIndex":0,"weaponPartType":7,"weaponPartTypeName":"eb_special","weaponPartId":"PART_TEST_SPECIAL_DEFAULT"}]}"#,
        "\r\n",
        r#"{"itemId":"WP_TEST_AXE","instanceId":"","weaponType":2,"slots":[{"slotIndex":0,"weaponPartType":32,"weaponPartTypeName":"soul","defaultWeaponPartId":"PART_TEST_SOUL","catalogLocked":false,"hideFromUI":false,"effectiveLocked":false},{"slotIndex":1,"weaponPartType":99,"weaponPartTypeName":"unknown","defaultWeaponPartId":"None","catalogLocked":false,"hideFromUI":false,"effectiveLocked":false}],"equippedParts":[]}"#,
        "\r\n",
    );

    #[test]
    fn reads_the_exporters_own_lines() {
        let mut doc = json!({ "starterKit": {
            "instanced": ["WP_TEST_AXE", "WP_TEST_SWORD", "PART_TEST_SOUL"],
            "defaultWeaponParts": ["PART_OLD"],
        }});
        let note = default_weapon_parts(&mut doc, EXPORTER_LINES);
        assert!(note.contains("1 added to the kit: PART_TEST_SPECIAL_DEFAULT"), "{note}");
        // In the kit's weapon order, not the export's.
        assert_eq!(
            doc["starterKit"]["defaultWeaponParts"],
            json!(["PART_TEST_SOUL", "PART_TEST_SPECIAL_DEFAULT"])
        );
    }

    #[test]
    fn a_partial_export_leaves_the_facts() {
        let mut doc = facts();
        let note = default_weapon_parts(&mut doc, &slots("WP_A", &["PART_X"]));
        assert!(note.contains("does not cover WP_B"), "{note}");
        assert_eq!(doc, facts());
    }

    #[test]
    fn missing_ids_name_their_member_once() {
        let doc = json!({
            "starterKit": { "instanced": ["A", "B"], "equipment": [["weapon", "B"]], "flask": "C",
                "wallet": [["CUR", 5]], "bannerCustomization": { "x": "D", "y": "", "n": 0 } },
            "legacyStarter": { "defaultDye": "A" },
        });
        let catalog = catalog_ids("{\"itemId\":\"A\"}\n{\"itemId\":\"CUR\"}\n");
        assert_eq!(
            missing_ids(&doc, &catalog),
            ["instanced: B", "flask: C", "bannerCustomization: D"]
        );
    }
}
