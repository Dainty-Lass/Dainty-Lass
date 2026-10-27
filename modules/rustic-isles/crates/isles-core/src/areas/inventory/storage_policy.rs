//! `inventoryStoragePolicy.ts`: which catalog ids are quantity records (`stacked`) and which are items with an instance
//! (`instanced`), from the version's own catalog (`inventory_storage_policy.json`, the module's `GAME_DATA`). A grant
//! is corrected to the catalog's representation before it touches the wallet or the inventory; legacy rows are
//! repaired on read.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::{Map, Value};
use service_kit::js;

use super::sha256::sha256_hex;

#[derive(Debug, thiserror::Error)]
pub enum PolicyError {
    /// `loadGameData` throws for a missing file, and `JSON.parse` for a bad one
    #[error("[game-data] {0}")]
    GameData(String),
    /// `RequireGrantQuantity`
    #[error("Invalid quantity {quantity} for non-stackable catalog item {catalog_id}")]
    GrantQuantity { quantity: String, catalog_id: String },
}

/// The file as `loadGameData` reads it: UTF-8, a leading BOM dropped.
pub fn read_game_data(path: &Path) -> Result<Value, PolicyError> {
    let text = std::fs::read_to_string(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            PolicyError::GameData(format!(
                "Required game data file is missing: {}\nThis project does not ship game data. Generate it from your \
                 own installation (see docs/GENERATING_GAME_DATA.md), or copy the matching *.example.json for a smoke \
                 test.",
                path.display()
            ))
        } else {
            PolicyError::GameData(format!("{}: {error}", path.display()))
        }
    })?;
    serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|error| PolicyError::GameData(format!("{}: {error}", path.display())))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Storage {
    Instanced,
    Stacked,
    /// Present in the file with a value the code doesn't act on
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PolicyEntry {
    pub storage: Storage,
    pub max_quantity: f64,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct StoragePolicy {
    entries: HashMap<String, PolicyEntry>,
}

type Cache = Mutex<HashMap<PathBuf, Arc<StoragePolicy>>>;

impl StoragePolicy {
    pub fn from_document(document: &Value) -> Self {
        let entries = document
            .get("entries")
            .and_then(Value::as_object)
            .map(|entries| {
                entries
                    .iter()
                    .map(|(catalog_id, entry)| {
                        let storage = match entry.get("storage").and_then(Value::as_str) {
                            Some("instanced") => Storage::Instanced,
                            Some("stacked") => Storage::Stacked,
                            _ => Storage::Other,
                        };
                        let tags = entry
                            .get("tags")
                            .and_then(Value::as_array)
                            .map(|tags| tags.iter().filter_map(|t| t.as_str().map(str::to_owned)).collect())
                            .unwrap_or_default();
                        let max_quantity = entry.get("maxQuantity").and_then(Value::as_f64).unwrap_or(f64::NAN);
                        (
                            catalog_id.clone(),
                            PolicyEntry {
                                storage,
                                max_quantity,
                                tags,
                            },
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self { entries }
    }

    /// Loaded on first use and kept, as Node does (`Policy()`), so a module without game data fails when first asked.
    pub fn load(path: &Path) -> Result<Arc<Self>, PolicyError> {
        static CACHE: OnceLock<Cache> = OnceLock::new();
        let cache = CACHE.get_or_init(Mutex::default);
        if let Some(policy) = cache.lock().ok().and_then(|cache| cache.get(path).cloned()) {
            return Ok(policy);
        }
        let policy = Arc::new(Self::from_document(&read_game_data(path)?));
        if let Ok(mut cache) = cache.lock() {
            cache.insert(path.to_path_buf(), policy.clone());
        }
        Ok(policy)
    }

    /// `GetInventoryStoragePolicy`.
    pub fn entry(&self, catalog_id: Option<&Value>) -> Option<&PolicyEntry> {
        match catalog_id {
            Some(Value::String(id)) => self.entries.get(id),
            _ => None,
        }
    }

    fn storage(&self, catalog_id: Option<&Value>) -> Option<Storage> {
        self.entry(catalog_id).map(|entry| entry.storage)
    }
}

/// `IsOwnershipUnlock`: a stacked entry that is a single owned unlock rather than a count.
pub fn is_ownership_unlock(entry: Option<&PolicyEntry>) -> bool {
    let Some(entry) = entry else {
        return false;
    };
    if entry.storage != Storage::Stacked {
        return false;
    }
    entry.max_quantity == 1.0
        || entry.tags.iter().any(|tag| {
            matches!(
                tag.to_lowercase().as_str(),
                "transmog" | "owned" | "accessory" | "dye" | "bannercustomization" | "emoji" | "emote"
            )
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Correction {
    pub catalog_id: String,
    pub from: &'static str,
    pub to: &'static str,
    pub quantity: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedGrants {
    pub instanced_items_to_add: Vec<Value>,
    pub stacked_items_to_add: Vec<Value>,
    pub corrections: Vec<Correction>,
}

fn deterministic_instance_id(transaction_id: &str, catalog_id: &str, ordinal: usize) -> String {
    sha256_hex(&format!(
        "inventory-storage-normalize:{transaction_id}:{catalog_id}:{ordinal}"
    ))[..26]
        .to_uppercase()
}

fn quantity_value(value: Option<&Value>) -> f64 {
    js::number(value)
}

/// `NormalizeInventoryGrantCollections`: corrects only the ADD collections at the authoritative transaction boundary.
/// The request hash is computed from the original wire body before this runs, which keeps replays of old ledger rows
/// matching. Unknown catalog ids keep their incoming representation.
pub fn normalize_inventory_grant_collections(
    policy: &StoragePolicy,
    transaction_id: &str,
    instanced_items_to_add: &[Value],
    stacked_items_to_add: &[Value],
) -> Result<NormalizedGrants, PolicyError> {
    let mut instanced = Vec::new();
    let mut stacked: Vec<Value> = Vec::new();
    let mut corrections = Vec::new();
    let mut ordinal = 0;

    for item in stacked_items_to_add {
        let catalog_id = item.get("catalogId");
        if policy.storage(catalog_id) != Some(Storage::Instanced) {
            stacked.push(item.clone());
            continue;
        }
        let catalog_id = js::string(catalog_id);
        let raw = item.get("quantity");
        let quantity = quantity_value(raw);
        if !(quantity.fract() == 0.0
            && quantity.abs() <= 9_007_199_254_740_991.0
            && quantity > 0.0
            && quantity <= 1000.0)
        {
            return Err(PolicyError::GrantQuantity {
                quantity: js::string(raw),
                catalog_id,
            });
        }
        let quantity = quantity as i64;
        for _ in 0..quantity {
            let mut entry = Map::new();
            entry.insert("catalogId".into(), Value::String(catalog_id.clone()));
            entry.insert(
                "instanceId".into(),
                Value::String(deterministic_instance_id(transaction_id, &catalog_id, ordinal)),
            );
            entry.insert("itemData".into(), Value::Null);
            entry.insert("updateVersion".into(), Value::from(0));
            instanced.push(Value::Object(entry));
            ordinal += 1;
        }
        corrections.push(Correction {
            catalog_id,
            from: "stacked",
            to: "instanced",
            quantity,
        });
    }

    for item in instanced_items_to_add {
        let catalog_id = item.get("catalogId");
        if policy.storage(catalog_id) != Some(Storage::Stacked) {
            instanced.push(item.clone());
            continue;
        }
        let catalog_id = js::string(catalog_id);
        add_stacked_grant(&mut stacked, &catalog_id, 1.0);
        corrections.push(Correction {
            catalog_id,
            from: "instanced",
            to: "stacked",
            quantity: 1,
        });
    }

    Ok(NormalizedGrants {
        instanced_items_to_add: instanced,
        stacked_items_to_add: stacked,
        corrections,
    })
}

fn number_value(n: f64) -> Value {
    if n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0 {
        Value::from(n as i64)
    } else {
        Value::from(n)
    }
}

fn add_stacked_grant(stacked: &mut Vec<Value>, catalog_id: &str, quantity: f64) {
    if let Some(existing) = stacked
        .iter_mut()
        .find(|item| item.get("catalogId").and_then(Value::as_str) == Some(catalog_id))
    {
        let current = js::number(existing.get("quantity"));
        let current = if current.is_nan() { 0.0 } else { current };
        if let Some(fields) = existing.as_object_mut() {
            fields.insert("quantity".into(), number_value(current + quantity));
        }
        return;
    }
    let mut entry = Map::new();
    entry.insert("catalogId".into(), Value::String(catalog_id.to_owned()));
    entry.insert("quantity".into(), number_value(quantity));
    stacked.push(Value::Object(entry));
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationChange {
    pub catalog_id: String,
    pub migrated_instances: i64,
    pub existing_quantity: i64,
    pub final_quantity: i64,
    pub result: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationIssue {
    pub catalog_id: String,
    pub quantity: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Migration {
    pub instanced_items: Vec<Value>,
    pub stacked_items: Vec<Value>,
    pub changes: Vec<MigrationChange>,
    pub issues: Vec<MigrationIssue>,
}

/// `MigrateInventoryStorageArrays`: stackable instances move to quantity ownership, unknown ids are preserved, and
/// non-stackables found in stacked storage stay for manual review.
pub fn migrate_inventory_storage_arrays(policy: &StoragePolicy, instanced: &[Value], stacked: &[Value]) -> Migration {
    let mut kept = Vec::new();
    let mut moved: Vec<(String, i64)> = Vec::new();
    for item in instanced {
        let catalog_id = item.get("catalogId");
        if policy.storage(catalog_id) == Some(Storage::Stacked) {
            let id = js::string(catalog_id);
            match moved.iter_mut().find(|(known, _)| *known == id) {
                Some((_, count)) => *count += 1,
                None => moved.push((id, 1)),
            }
        } else {
            kept.push(item.clone());
        }
    }

    let mut next: Vec<Value> = stacked.to_vec();
    let mut changes = Vec::new();
    for (catalog_id, migrated) in moved {
        let entry = policy.entry(Some(&Value::String(catalog_id.clone())));
        let existing: f64 = next
            .iter()
            .filter(|item| item.get("catalogId").and_then(Value::as_str) == Some(catalog_id.as_str()))
            .map(|item| {
                let q = js::number(item.get("quantity"));
                if q.is_nan() { 0.0 } else { q.max(0.0) }
            })
            .sum();
        next.retain(|item| item.get("catalogId").and_then(Value::as_str) != Some(catalog_id.as_str()));
        let ownership = is_ownership_unlock(entry);
        let final_quantity = if ownership { 1.0 } else { existing + migrated as f64 };
        let mut stack = Map::new();
        stack.insert("catalogId".into(), Value::String(catalog_id.clone()));
        stack.insert("quantity".into(), number_value(final_quantity));
        next.push(Value::Object(stack));
        changes.push(MigrationChange {
            catalog_id,
            migrated_instances: migrated,
            existing_quantity: existing as i64,
            final_quantity: final_quantity as i64,
            result: if ownership && existing + migrated as f64 > 1.0 {
                "deduplicated-to-one"
            } else {
                "moved"
            },
        });
    }

    let issues = next
        .iter()
        .filter(|item| policy.storage(item.get("catalogId")) == Some(Storage::Instanced))
        .map(|item| {
            let q = js::number(item.get("quantity"));
            MigrationIssue {
                catalog_id: js::string(item.get("catalogId")),
                quantity: if q.is_nan() { 0 } else { q as i64 },
            }
        })
        .collect();

    Migration {
        instanced_items: kept,
        stacked_items: next,
        changes,
        issues,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// The shape of `inventory_storage_policy.json`, with the catalog ids the Node test (a `.gamedata.test.ts`, which
    /// reads the real export) asserts on.
    fn policy() -> StoragePolicy {
        let entry =
            |storage: &str, max: i64, tags: &[&str]| json!({"storage": storage, "maxQuantity": max, "tags": tags});
        StoragePolicy::from_document(&json!({
            "schema": 1,
            "entries": {
                "WP_AC_TRIALS_00": entry("stacked", 1, &["Transmog"]),
                "WP_EB_HP09A_ASSASSINS_00": entry("stacked", 1, &["Transmog"]),
                "LT_TRIALS_01": entry("stacked", 1, &["Owned"]),
                "LT_TRIALS_00": entry("instanced", 1, &[]),
                "PART_CB_PASSIVE_TRIALS_02": entry("instanced", 1, &[]),
                "WP_GA_BEGINNER": entry("instanced", 1, &[]),
                "AC_HEAD_TRIALS_00": entry("stacked", 1, &["Accessory"]),
                "AR_TRIALS_CHEST_00": entry("stacked", 1, &["Transmog"]),
                "MAT_STACK": entry("stacked", 99, &["Material"]),
            }
        }))
    }

    fn storage(policy: &StoragePolicy, id: &str) -> Option<Storage> {
        policy.storage(Some(&json!(id)))
    }

    // inventoryStoragePolicy.gamedata.test.ts, on a policy with the same ids
    #[test]
    fn policy_distinguishes_transmog_unlocks_from_functional_gear() {
        let policy = policy();
        assert_eq!(storage(&policy, "WP_AC_TRIALS_00"), Some(Storage::Stacked));
        assert_eq!(storage(&policy, "LT_TRIALS_01"), Some(Storage::Stacked));
        assert_eq!(storage(&policy, "LT_TRIALS_00"), Some(Storage::Instanced));
        assert_eq!(storage(&policy, "WP_GA_BEGINNER"), Some(Storage::Instanced));
        assert_eq!(storage(&policy, "UNKNOWN"), None);
        assert_eq!(policy.storage(Some(&json!(5))), None);
    }

    #[test]
    fn grant_normalization_moves_catalog_declared_stackables_out_of_instanced_adds() {
        let result = normalize_inventory_grant_collections(
            &policy(),
            "TX-1",
            &[
                json!({"catalogId": "WP_AC_TRIALS_00", "instanceId": "BAD-COSMETIC", "itemData": null, "updateVersion": 0}),
                json!({"catalogId": "LT_TRIALS_00", "instanceId": "REAL-LANTERN", "itemData": null, "updateVersion": 0}),
            ],
            &[],
        )
        .unwrap();
        assert_eq!(
            result.instanced_items_to_add,
            vec![
                json!({"catalogId": "LT_TRIALS_00", "instanceId": "REAL-LANTERN", "itemData": null, "updateVersion": 0})
            ]
        );
        assert_eq!(
            result.stacked_items_to_add,
            vec![json!({"catalogId": "WP_AC_TRIALS_00", "quantity": 1})]
        );
        assert_eq!(
            result.corrections,
            vec![Correction {
                catalog_id: "WP_AC_TRIALS_00".into(),
                from: "instanced",
                to: "stacked",
                quantity: 1
            }]
        );
    }

    #[test]
    fn normalization_creates_deterministic_instances_for_a_known_non_stackable_sent_as_stacked() {
        let stacked = [json!({"catalogId": "LT_TRIALS_00", "quantity": 2})];
        let first = normalize_inventory_grant_collections(&policy(), "TX-2", &[], &stacked).unwrap();
        let second = normalize_inventory_grant_collections(&policy(), "TX-2", &[], &stacked).unwrap();
        assert_eq!(first.stacked_items_to_add.len(), 0);
        assert_eq!(first.instanced_items_to_add.len(), 2);
        assert_eq!(first.instanced_items_to_add, second.instanced_items_to_add);
        assert_ne!(
            first.instanced_items_to_add[0]["instanceId"],
            first.instanced_items_to_add[1]["instanceId"]
        );
        // createHash("sha256").update("inventory-storage-normalize:TX-2:LT_TRIALS_00:0").digest("hex").slice(0, 26).toUpperCase()
        assert_eq!(
            first.instanced_items_to_add[0]["instanceId"].as_str().unwrap().len(),
            26
        );
    }

    #[test]
    fn an_impossible_quantity_for_a_non_stackable_throws() {
        for quantity in [json!(0), json!(1001), json!(1.5), json!("x")] {
            let error = normalize_inventory_grant_collections(
                &policy(),
                "T",
                &[],
                &[json!({"catalogId": "LT_TRIALS_00", "quantity": quantity})],
            )
            .unwrap_err();
            assert!(error.to_string().starts_with("Invalid quantity "), "{error}");
        }
    }

    #[test]
    fn migration_removes_malformed_cosmetic_instances_and_deduplicates_ownership() {
        let result = migrate_inventory_storage_arrays(
            &policy(),
            &[
                json!({"catalogId": "WP_AC_TRIALS_00", "instanceId": "A"}),
                json!({"catalogId": "WP_AC_TRIALS_00", "instanceId": "B"}),
                json!({"catalogId": "LT_TRIALS_00", "instanceId": "C"}),
            ],
            &[json!({"catalogId": "WP_AC_TRIALS_00", "quantity": 1})],
        );
        assert_eq!(
            result.instanced_items,
            vec![json!({"catalogId": "LT_TRIALS_00", "instanceId": "C"})]
        );
        assert_eq!(
            result.stacked_items,
            vec![json!({"catalogId": "WP_AC_TRIALS_00", "quantity": 1})]
        );
        assert_eq!(result.changes[0].migrated_instances, 2);
        assert_eq!(result.changes[0].existing_quantity, 1);
        assert_eq!(result.changes[0].result, "deduplicated-to-one");
    }

    #[test]
    fn migration_repairs_cosmetics_the_parser_expects_as_quantity_records() {
        let result = migrate_inventory_storage_arrays(
            &policy(),
            &[
                json!({"catalogId": "AC_HEAD_TRIALS_00", "instanceId": "HEAD"}),
                json!({"catalogId": "WP_AC_TRIALS_00", "instanceId": "STRIKERS"}),
                json!({"catalogId": "AR_TRIALS_CHEST_00", "instanceId": "CHEST"}),
                json!({"catalogId": "LT_TRIALS_00", "instanceId": "FUNCTIONAL-LANTERN"}),
            ],
            &[],
        );
        assert_eq!(
            result.instanced_items,
            vec![json!({"catalogId": "LT_TRIALS_00", "instanceId": "FUNCTIONAL-LANTERN"})]
        );
        assert_eq!(
            result.stacked_items,
            vec![
                json!({"catalogId": "AC_HEAD_TRIALS_00", "quantity": 1}),
                json!({"catalogId": "WP_AC_TRIALS_00", "quantity": 1}),
                json!({"catalogId": "AR_TRIALS_CHEST_00", "quantity": 1}),
            ]
        );
        assert!(result.issues.is_empty());
    }

    #[test]
    fn a_counted_stack_adds_up_and_a_non_stackable_in_stacked_storage_is_left_for_review() {
        let result = migrate_inventory_storage_arrays(
            &policy(),
            &[json!({"catalogId": "MAT_STACK", "instanceId": "m1"})],
            &[
                json!({"catalogId": "MAT_STACK", "quantity": 4}),
                json!({"catalogId": "LT_TRIALS_00", "quantity": 2}),
            ],
        );
        assert_eq!(result.changes[0].final_quantity, 5);
        assert_eq!(result.changes[0].result, "moved");
        assert_eq!(
            result.issues,
            vec![MigrationIssue {
                catalog_id: "LT_TRIALS_00".into(),
                quantity: 2
            }]
        );
        assert!(is_ownership_unlock(policy().entry(Some(&json!("LT_TRIALS_01")))));
        assert!(!is_ownership_unlock(policy().entry(Some(&json!("MAT_STACK")))));
        assert!(!is_ownership_unlock(policy().entry(Some(&json!("LT_TRIALS_00")))));
    }

    #[test]
    fn the_file_is_read_like_load_game_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("inventory_storage_policy.json");
        assert!(matches!(StoragePolicy::load(&path), Err(PolicyError::GameData(m)) if m.contains("missing")));
        std::fs::write(
            &path,
            "\u{feff}{\"entries\":{\"X\":{\"storage\":\"stacked\",\"maxQuantity\":1,\"tags\":[]}}}",
        )
        .unwrap();
        let loaded = StoragePolicy::load(&path).unwrap();
        assert_eq!(loaded.storage(Some(&json!("X"))), Some(Storage::Stacked));
    }
}
