// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The storage policy (spec §8): which catalog ids the game keeps instanced and which stacked, read from the host's
//! `inventory_storage_policy.json`, and the two rules built on it.

use std::collections::HashMap;
use std::sync::Arc;

use rustic_isles::{LoadError, Shared};
use serde_json::Value;
use service_kit::js;

use crate::util::{MAX_SAFE, num, num0, number_value, obj, sha_upper};

pub const POLICY_FILE: &str = "inventory_storage_policy.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Storage {
    Instanced,
    Stacked,
    /// Present but neither: not acted on
    Other,
}

impl Storage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Instanced => "instanced",
            Self::Stacked => "stacked",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PolicyEntry {
    pub storage: Storage,
    /// NaN when not a number
    pub max_quantity: f64,
    pub tags: Vec<String>,
    /// The `storage` text as written (none when absent), for messages
    pub storage_text: Option<String>,
}

/// A game-data or policy failure: the transaction fails with a 500 page.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct PolicyError(pub String);

#[derive(Debug, Clone, Default, PartialEq)]
pub struct StoragePolicy {
    entries: HashMap<String, PolicyEntry>,
}

/// A backend game-data file read with core's loader, its errors as `[game-data] ...` texts.
pub fn read_game_data(shared: &Shared, name: &str) -> Result<Value, PolicyError> {
    let root = shared.game_data();
    let Some(path) = root.backend_file(name) else {
        return Err(PolicyError(format!(
            "[game-data] this module has no game data folder for {name}"
        )));
    };
    match root.try_load_backend(name) {
        Ok(Some(value)) => Ok(value),
        Ok(None) => Err(PolicyError(format!(
            "[game-data] Required game data file is missing: {}\nThis project does not ship game data. Generate it \
             from your own installation (see docs/GENERATING_GAME_DATA.md), or copy the matching *.example.json for \
             a smoke test.",
            path.display()
        ))),
        Err(LoadError::Unreadable(text) | LoadError::Missing(text)) => Err(PolicyError(format!("[game-data] {text}"))),
    }
}

impl StoragePolicy {
    /// `{"schema":1,"entries":{"<catalogId>":{"storage":..,"maxQuantity":..,"tags":[..]}}}`; unknown fields ignored.
    pub fn from_document(document: &Value) -> Self {
        let mut entries = HashMap::new();
        if let Some(map) = document.get("entries").and_then(Value::as_object) {
            for (id, entry) in map {
                let storage_text = entry.get("storage").and_then(Value::as_str).map(str::to_owned);
                let storage = match storage_text.as_deref() {
                    Some("instanced") => Storage::Instanced,
                    Some("stacked") => Storage::Stacked,
                    _ => Storage::Other,
                };
                let max_quantity = match entry.get("maxQuantity") {
                    Some(v @ Value::Number(_)) => num(Some(v)),
                    _ => f64::NAN,
                };
                let tags = entry
                    .get("tags")
                    .and_then(Value::as_array)
                    .map(|tags| tags.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                    .unwrap_or_default();
                entries.insert(
                    id.clone(),
                    PolicyEntry {
                        storage,
                        max_quantity,
                        tags,
                        storage_text,
                    },
                );
            }
        }
        Self { entries }
    }

    /// Loaded on first use and kept for the module's life; a failed load is tried again next time.
    pub fn load(shared: &Shared) -> Result<Arc<StoragePolicy>, PolicyError> {
        let loaded = crate::caches::cached(shared, "inventory.storage_policy", || {
            match read_game_data(shared, POLICY_FILE) {
                Ok(document) => (Arc::new(Ok(Arc::new(Self::from_document(&document)))), true),
                Err(error) => (Arc::new(Err(error)), false),
            }
        });
        (*loaded).clone()
    }

    pub fn entry(&self, catalog_id: &str) -> Option<&PolicyEntry> {
        self.entries.get(catalog_id)
    }

    fn storage_of(&self, item: &Value) -> Option<Storage> {
        let id = item.get("catalogId").and_then(Value::as_str)?;
        self.entry(id).map(|e| e.storage)
    }

    pub fn is_instanced(&self, catalog_id: &str) -> bool {
        self.entry(catalog_id).is_some_and(|e| e.storage == Storage::Instanced)
    }
}

const UNLOCK_TAGS: [&str; 7] = [
    "transmog",
    "owned",
    "accessory",
    "dye",
    "bannercustomization",
    "emoji",
    "emote",
];

/// `IsOwnershipUnlock`: a stacked entry that is owned rather than counted.
pub fn is_ownership_unlock(entry: &PolicyEntry) -> bool {
    entry.storage == Storage::Stacked
        && (entry.max_quantity == 1.0
            || entry
                .tags
                .iter()
                .any(|t| UNLOCK_TAGS.contains(&t.to_lowercase().as_str())))
}

/// One grant moved to the representation the policy says.
#[derive(Debug, Clone, PartialEq)]
pub struct Correction {
    pub catalog_id: String,
    pub from: Storage,
    pub to: Storage,
    pub quantity: f64,
}

impl Correction {
    pub fn describe(&self) -> String {
        format!(
            "{}:{}->{}x{}",
            self.catalog_id,
            self.from.as_str(),
            self.to.as_str(),
            service_kit::jsjson::number(self.quantity)
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedGrants {
    pub add_instanced: Vec<Value>,
    pub add_stacked: Vec<Value>,
    pub corrections: Vec<Correction>,
}

/// The most instances one stacked entry may become.
pub const MAX_NORMALIZED_INSTANCES: f64 = 1000.0;

fn list(value: &Value) -> &[Value] {
    value.as_array().map_or(&[], Vec::as_slice)
}

/// `NormalizeInventoryGrantCollections` (§8.3): only the two add collections, after the hash.
pub fn normalize_inventory_grant_collections(
    policy: &StoragePolicy,
    transaction_id: &str,
    add_instanced: &Value,
    add_stacked: &Value,
) -> Result<NormalizedGrants, PolicyError> {
    let mut converted_instanced = Vec::new();
    let mut stacked = Vec::new();
    let mut corrections = Vec::new();
    let mut ordinal = 0u64;
    for add in list(add_stacked) {
        if policy.storage_of(add) != Some(Storage::Instanced) {
            stacked.push(add.clone());
            continue;
        }
        let catalog_id = add.get("catalogId").and_then(Value::as_str).unwrap_or_default();
        let q = num(add.get("quantity"));
        if !(q.is_finite() && q.fract() == 0.0 && q.abs() <= MAX_SAFE && q > 0.0 && q <= MAX_NORMALIZED_INSTANCES) {
            return Err(PolicyError(format!(
                "Invalid quantity {} for non-stackable catalog item {catalog_id}",
                js::string(add.get("quantity"))
            )));
        }
        for _ in 0..crate::util::whole(q) {
            let id = sha_upper(
                &format!("inventory-storage-normalize:{transaction_id}:{catalog_id}:{ordinal}"),
                26,
            );
            ordinal += 1;
            converted_instanced.push(obj(vec![
                ("catalogId", Value::from(catalog_id)),
                ("instanceId", Value::from(id)),
                ("itemData", Value::Null),
                ("updateVersion", Value::from(0)),
            ]));
        }
        corrections.push(Correction {
            catalog_id: catalog_id.to_owned(),
            from: Storage::Stacked,
            to: Storage::Instanced,
            quantity: q,
        });
    }
    let mut kept_instanced = Vec::new();
    for add in list(add_instanced) {
        if policy.storage_of(add) != Some(Storage::Stacked) {
            kept_instanced.push(add.clone());
            continue;
        }
        let catalog_id = add.get("catalogId").and_then(Value::as_str).unwrap_or_default();
        match stacked
            .iter_mut()
            .find(|s| s.get("catalogId").and_then(Value::as_str) == Some(catalog_id))
        {
            Some(existing) => {
                let sum = num0(existing.get("quantity")) + 1.0;
                if let Some(map) = existing.as_object_mut() {
                    map.insert("quantity".into(), number_value(sum));
                }
            }
            None => stacked.push(obj(vec![
                ("catalogId", Value::from(catalog_id)),
                ("quantity", Value::from(1)),
            ])),
        }
        corrections.push(Correction {
            catalog_id: catalog_id.to_owned(),
            from: Storage::Instanced,
            to: Storage::Stacked,
            quantity: 1.0,
        });
    }
    converted_instanced.extend(kept_instanced);
    Ok(NormalizedGrants {
        add_instanced: converted_instanced,
        add_stacked: stacked,
        corrections,
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct MigrationChange {
    pub catalog_id: String,
    pub migrated: usize,
    pub existing: f64,
    pub final_quantity: f64,
    /// `moved` or `deduplicated-to-one`
    pub result: &'static str,
}

/// A stacked entry the policy says is instanced, left for manual review.
#[derive(Debug, Clone, PartialEq)]
pub struct MigrationIssue {
    pub catalog_id: String,
    pub quantity: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Migration {
    pub instanced: Vec<Value>,
    pub stacked: Vec<Value>,
    pub changes: Vec<MigrationChange>,
    pub issues: Vec<MigrationIssue>,
}

impl Migration {
    pub fn changed(&self) -> bool {
        !self.changes.is_empty()
    }

    pub fn describe_changes(&self) -> String {
        self.changes
            .iter()
            .map(|c| format!("{}:instanced->stacked x{} ({})", c.catalog_id, c.migrated, c.result))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// `MigrateInventoryStorageArrays` (§8.4): instanced rows of stackable items become one stack.
pub fn migrate_inventory_storage_arrays(
    policy: &StoragePolicy,
    instanced: Vec<Value>,
    stacked: Vec<Value>,
) -> Migration {
    let mut counts: Vec<(String, usize)> = Vec::new();
    let mut kept = Vec::new();
    for item in instanced {
        if policy.storage_of(&item) == Some(Storage::Stacked)
            && let Some(id) = item.get("catalogId").and_then(Value::as_str)
        {
            match counts.iter_mut().find(|(seen, _)| seen == id) {
                Some((_, n)) => *n += 1,
                None => counts.push((id.to_owned(), 1)),
            }
            continue;
        }
        kept.push(item);
    }
    let mut stacked = stacked;
    let mut changes = Vec::new();
    for (id, count) in counts {
        let mut existing = 0.0;
        stacked.retain(|s| {
            if s.get("catalogId").and_then(Value::as_str) == Some(id.as_str()) {
                existing += num0(s.get("quantity")).max(0.0);
                false
            } else {
                true
            }
        });
        let unlock = policy.entry(&id).is_some_and(is_ownership_unlock);
        // `count` is bounded by the stored list's length
        #[allow(clippy::cast_precision_loss)]
        let total = existing + count as f64;
        let final_quantity = if unlock { 1.0 } else { total };
        stacked.push(obj(vec![
            ("catalogId", Value::from(id.as_str())),
            ("quantity", number_value(final_quantity)),
        ]));
        changes.push(MigrationChange {
            catalog_id: id,
            migrated: count,
            existing,
            final_quantity,
            result: if unlock && total > 1.0 {
                "deduplicated-to-one"
            } else {
                "moved"
            },
        });
    }
    let issues = stacked
        .iter()
        .filter(|s| policy.storage_of(s) == Some(Storage::Instanced))
        .map(|s| MigrationIssue {
            catalog_id: s
                .get("catalogId")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            quantity: s.get("quantity").cloned().unwrap_or(Value::Null),
        })
        .collect();
    Migration {
        instanced: kept,
        stacked,
        changes,
        issues,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn policy() -> StoragePolicy {
        StoragePolicy::from_document(&json!({"schema": 1, "entries": {
            "WP_AC_TRIALS_00": {"storage": "stacked", "maxQuantity": 1, "tags": ["Transmog"]},
            "LT_TRIALS_00": {"storage": "instanced", "maxQuantity": 1, "tags": []},
            "MAT_STACK": {"storage": "stacked", "maxQuantity": 99, "tags": ["Material", 5]},
            "ODD": {"storage": "weird"},
        }}))
    }

    #[test]
    fn entries_and_unlocks() {
        let p = policy();
        assert_eq!(p.entry("ODD").unwrap().storage, Storage::Other);
        assert!(p.entry("NOPE").is_none());
        assert!(is_ownership_unlock(p.entry("WP_AC_TRIALS_00").unwrap()));
        assert!(!is_ownership_unlock(p.entry("MAT_STACK").unwrap()));
        assert!(!is_ownership_unlock(p.entry("LT_TRIALS_00").unwrap()));
        assert_eq!(p.entry("MAT_STACK").unwrap().tags, ["Material"]);
        assert!(p.storage_of(&json!({"catalogId": 5})).is_none());
    }

    #[test]
    fn migrations() {
        let p = policy();
        let out = migrate_inventory_storage_arrays(
            &p,
            vec![
                json!({"catalogId": "WP_AC_TRIALS_00", "instanceId": "A"}),
                json!({"catalogId": "WP_AC_TRIALS_00", "instanceId": "B"}),
                json!({"catalogId": "LT_TRIALS_00", "instanceId": "C"}),
            ],
            vec![json!({"catalogId": "WP_AC_TRIALS_00", "quantity": 1})],
        );
        assert_eq!(
            out.instanced,
            vec![json!({"catalogId": "LT_TRIALS_00", "instanceId": "C"})]
        );
        assert_eq!(
            out.stacked,
            vec![json!({"catalogId": "WP_AC_TRIALS_00", "quantity": 1})]
        );
        assert_eq!(
            (out.changes[0].migrated, out.changes[0].existing, out.changes[0].result),
            (2, 1.0, "deduplicated-to-one")
        );
        let out = migrate_inventory_storage_arrays(
            &p,
            vec![json!({"catalogId": "MAT_STACK", "instanceId": "m1"})],
            vec![
                json!({"catalogId": "MAT_STACK", "quantity": 4}),
                json!({"catalogId": "LT_TRIALS_00", "quantity": 2}),
            ],
        );
        assert_eq!(
            out.stacked,
            vec![
                json!({"catalogId": "LT_TRIALS_00", "quantity": 2}),
                json!({"catalogId": "MAT_STACK", "quantity": 5})
            ]
        );
        assert_eq!(out.changes[0].result, "moved");
        assert_eq!(
            out.issues,
            vec![MigrationIssue {
                catalog_id: "LT_TRIALS_00".into(),
                quantity: json!(2)
            }]
        );
    }

    #[test]
    fn normalization() {
        let p = policy();
        let out = normalize_inventory_grant_collections(
            &p,
            "tx1",
            &json!([{"catalogId": "MAT_STACK", "instanceId": "x"}, {"catalogId": "OTHER", "instanceId": "y"}]),
            &json!([{"catalogId": "LT_TRIALS_00", "quantity": 2}, {"catalogId": "MAT_STACK", "quantity": 3}]),
        )
        .unwrap();
        assert_eq!(out.add_instanced.len(), 3);
        let a = out.add_instanced[0]["instanceId"].as_str().unwrap();
        assert_eq!(a.len(), 26);
        assert!(a.bytes().all(|b| b.is_ascii_digit() || b.is_ascii_uppercase()));
        assert_ne!(out.add_instanced[0]["instanceId"], out.add_instanced[1]["instanceId"]);
        assert_eq!(out.add_instanced[2]["catalogId"], "OTHER");
        assert_eq!(out.add_stacked, vec![json!({"catalogId": "MAT_STACK", "quantity": 4})]);
        assert_eq!(
            out.corrections.iter().map(Correction::describe).collect::<Vec<_>>(),
            ["LT_TRIALS_00:stacked->instancedx2", "MAT_STACK:instanced->stackedx1"]
        );
        let again = normalize_inventory_grant_collections(
            &p,
            "tx1",
            &json!([]),
            &json!([{"catalogId": "LT_TRIALS_00", "quantity": 1}]),
        )
        .unwrap();
        assert_eq!(again.add_instanced[0]["instanceId"], out.add_instanced[0]["instanceId"]);
        let other = normalize_inventory_grant_collections(
            &p,
            "tx2",
            &json!([]),
            &json!([{"catalogId": "LT_TRIALS_00", "quantity": 1}]),
        )
        .unwrap();
        assert_ne!(other.add_instanced[0]["instanceId"], out.add_instanced[0]["instanceId"]);
        for bad in [json!(0), json!(1001), json!(1.5), json!("x")] {
            let err = normalize_inventory_grant_collections(
                &p,
                "t",
                &json!([]),
                &json!([{"catalogId": "LT_TRIALS_00", "quantity": bad}]),
            );
            assert!(err.unwrap_err().0.starts_with("Invalid quantity "));
        }
    }
}
