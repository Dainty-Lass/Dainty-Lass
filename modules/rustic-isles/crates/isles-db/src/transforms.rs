//! Deterministic DD18 global ID derivation, BSON/JSON field normalization, and the five RB11
//! data transforms (`migrate_launcher_approval`, `migrate_inventory_storage_arrays`,
//! `migrate_starter_instances`, `migrate_banner_instances`, `migrate_loadout_slots`).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use std::sync::Arc;

use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use isles_core::areas::characters::starter::STARTER_INSTANCED_CATALOG_IDS;
use isles_core::areas::inventory::storage_policy::{StoragePolicy, migrate_inventory_storage_arrays};
use isles_core::ids::AccountId;
use isles_core::repo::characters::slots::{
    DEFAULT_ACCOUNT_LOADOUT_SLOTS, MAX_TOTAL_LOADOUT_SLOTS, resolve_player_journey_total, resolve_requested_total,
};
use module_api::{GlobalId, NodeId};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

const STARTER_BANNER_CATALOG_ID: &str = "BN_BEGINNER_00";
const STARTER_FLASK_CATALOG_ID: &str = "FL_HEALING_DEFAULT";
const STARTER_GLIDER_CATALOG_ID: &str = "GD_FRAME_STARTER_BASE";
const DEFAULT_BANNER_DYE_ID: &str = "DYE_DEFAULT";
const DEFAULT_BANNER_SIGIL_ID: &str = "BN_SIGIL_BEGINNER_00";

pub fn is_instance_id(text: &str) -> bool {
    text.len() == 26 && text.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const UPPER_ALNUM: &[u8; 36] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";

/// Deterministic 26-character Crockford Base32 ULID derived from `SHA-256("isles-db:<ns>:<node>:<key>")`.
pub fn deterministic_ulid(namespace: &str, node: &NodeId, key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"isles-db:");
    hasher.update(namespace.as_bytes());
    hasher.update(b":");
    hasher.update(node.as_str().as_bytes());
    hasher.update(b":");
    hasher.update(key.as_bytes());
    let digest = hasher.finalize();

    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    let val = u128::from_be_bytes(bytes);

    let mut chars = [b'0'; 26];
    // Top 2 bits of a 26-char Base32 ULID must be 0 so the first char is in '0'..='7'
    chars[0] = CROCKFORD[((val >> 125) & 0x07) as usize];
    for (idx, slot) in chars.iter_mut().enumerate().skip(1) {
        let shift = 125 - (idx * 5);
        *slot = CROCKFORD[((val >> shift) & 0x1F) as usize];
    }
    String::from_utf8(chars.to_vec()).unwrap_or_default()
}

pub fn deterministic_account_id(node: &NodeId, user_id: &str) -> Result<AccountId, String> {
    let ulid = deterministic_ulid("account", node, user_id);
    let raw = format!("account:{ulid}@{node}");
    raw.parse::<AccountId>()
        .map_err(|e| format!("invalid derived account id {raw:?}: {e}"))
}

pub fn deterministic_party_id(node: &NodeId, party_id: &str) -> Result<GlobalId, String> {
    let ulid = deterministic_ulid("party", node, party_id);
    let raw = format!("party:{ulid}@{node}");
    raw.parse::<GlobalId>()
        .map_err(|e| format!("invalid derived party id {raw:?}: {e}"))
}

/// Deterministic 26-character `[0-9A-Z]` instance ID used when repairing missing/invalid starter instances.
pub fn deterministic_instance_id(user_id: &str, character_id: &str, catalog_id: &str, ordinal: usize) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"isles-db:instance:");
    hasher.update(user_id.as_bytes());
    hasher.update(b":");
    hasher.update(character_id.as_bytes());
    hasher.update(b":");
    hasher.update(catalog_id.as_bytes());
    hasher.update(b":");
    hasher.update(ordinal.to_string().as_bytes());
    let digest = hasher.finalize();

    let mut out = String::with_capacity(26);
    for i in 0..26 {
        let b = digest[i % digest.len()];
        let idx = ((b as usize).wrapping_add(i * 7)) % UPPER_ALNUM.len();
        out.push(UPPER_ALNUM[idx] as char);
    }
    out
}

pub fn as_str_opt(val: Option<&Value>) -> Option<String> {
    match val {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    }
}

pub fn as_str_or(val: Option<&Value>, default: &str) -> String {
    match val {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => default.to_owned(),
    }
}

pub fn as_i64_or(val: Option<&Value>, default: i64) -> i64 {
    match val {
        Some(Value::Number(n)) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)).unwrap_or(default),
        Some(Value::String(s)) => s.parse::<i64>().unwrap_or(default),
        Some(Value::Bool(b)) => i64::from(*b),
        _ => default,
    }
}

pub fn as_f64_or(val: Option<&Value>, default: f64) -> f64 {
    match val {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(default),
        Some(Value::String(s)) => s.parse::<f64>().unwrap_or(default),
        _ => default,
    }
}

pub fn as_bool_i64(val: Option<&Value>, default: bool) -> i64 {
    match val {
        Some(Value::Bool(b)) => i64::from(*b),
        Some(Value::Number(n)) => i64::from(n.as_i64().unwrap_or(0) != 0),
        Some(Value::String(s)) => i64::from(s.eq_ignore_ascii_case("true") || s == "1"),
        _ => i64::from(default),
    }
}

pub fn as_bool(val: Option<&Value>, default: bool) -> bool {
    as_bool_i64(val, default) != 0
}

pub fn as_iso_string(val: Option<&Value>, fallback: &str) -> String {
    as_opt_iso_string(val).unwrap_or_else(|| fallback.to_owned())
}

pub fn as_opt_iso_string(val: Option<&Value>) -> Option<String> {
    match val {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return None;
            }
            if let Ok(dt) = DateTime::parse_from_rfc3339(trimmed) {
                return Some(dt.with_timezone(&Utc).to_rfc3339_opts(SecondsFormat::Millis, true));
            }
            if let Ok(ms) = trimmed.parse::<i64>() {
                let dt = Utc
                    .timestamp_millis_opt(ms)
                    .single()
                    .unwrap_or(DateTime::<Utc>::UNIX_EPOCH);
                return Some(dt.to_rfc3339_opts(SecondsFormat::Millis, true));
            }
            Some(trimmed.to_owned())
        }
        Some(Value::Number(n)) => {
            let raw = n.as_i64().or_else(|| n.as_f64().map(|f| f as i64))?;
            let ms = if raw.abs() < 10_000_000_000 { raw * 1000 } else { raw };
            let dt = Utc
                .timestamp_millis_opt(ms)
                .single()
                .unwrap_or(DateTime::<Utc>::UNIX_EPOCH);
            Some(dt.to_rfc3339_opts(SecondsFormat::Millis, true))
        }
        Some(Value::Object(map)) => {
            if let Some(d) = map.get("$date") {
                return as_opt_iso_string(Some(d));
            }
            None
        }
        _ => None,
    }
}

pub fn as_epoch_ms(val: Option<&Value>, fallback: i64) -> i64 {
    match val {
        None | Some(Value::Null) => fallback,
        Some(Value::Number(n)) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)).unwrap_or(fallback),
        Some(Value::String(s)) => {
            if let Ok(dt) = DateTime::parse_from_rfc3339(s.trim()) {
                return dt.timestamp_millis();
            }
            s.trim().parse::<i64>().unwrap_or(fallback)
        }
        Some(Value::Object(map)) => {
            if let Some(d) = map.get("$date") {
                return as_epoch_ms(Some(d), fallback);
            }
            fallback
        }
        _ => fallback,
    }
}

/// Parses a field that may be stored in Mongo either as a serialized JSON string or as a native BSON value.
pub fn parse_json_or_value(val: Option<&Value>, default: Value) -> Value {
    match val {
        None | Some(Value::Null) => default,
        Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(default),
        Some(other) => other.clone(),
    }
}

/// Formats a field that may be a JSON string or native BSON object/array into a compact JSON string.
pub fn to_json_text(val: Option<&Value>, default_json: &str) -> String {
    match val {
        None | Some(Value::Null) => default_json.to_owned(),
        Some(Value::String(s)) => {
            if serde_json::from_str::<Value>(s).is_ok() {
                s.clone()
            } else {
                default_json.to_owned()
            }
        }
        Some(other) => serde_json::to_string(other).unwrap_or_else(|_| default_json.to_owned()),
    }
}

/// RB11 `migrate_launcher_approval.ts`:
/// Backfills missing `approvalStatus` on existing launcher accounts to `"approved"` with
/// `updatedBy = "migration:0.1.73"` and `updatedAt = createdAt`.
pub fn migrate_launcher_approval(
    doc: &Map<String, Value>,
    created_at: &str,
) -> (String, String, String, Option<String>) {
    let nested = doc.get("approval").and_then(Value::as_object);
    let raw_status = doc
        .get("approvalStatus")
        .and_then(Value::as_str)
        .or_else(|| nested.and_then(|n| n.get("status")).and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let Some(status) = raw_status else {
        return (
            "approved".to_owned(),
            created_at.to_owned(),
            "migration:0.1.73".to_owned(),
            None,
        );
    };

    let updated_at = as_opt_iso_string(
        doc.get("approvalUpdatedAt")
            .or_else(|| nested.and_then(|n| n.get("updatedAt"))),
    )
    .unwrap_or_else(|| created_at.to_owned());

    let updated_by = doc
        .get("approvalUpdatedBy")
        .and_then(Value::as_str)
        .or_else(|| nested.and_then(|n| n.get("updatedBy")).and_then(Value::as_str))
        .filter(|s| !s.is_empty())
        .unwrap_or("migration:0.1.73")
        .to_owned();

    let reason = doc
        .get("approvalReason")
        .and_then(Value::as_str)
        .or_else(|| nested.and_then(|n| n.get("reason")).and_then(Value::as_str))
        .filter(|s| !s.is_empty())
        .map(str::to_owned);

    (status.to_owned(), updated_at, updated_by, reason)
}

/// Loads `StoragePolicy` from `<game_data_dir>/inventory_storage_policy.json` if available.
pub fn try_load_storage_policy(game_data_dir: Option<&Path>) -> Option<Arc<StoragePolicy>> {
    let dir = game_data_dir?;
    let candidates = [
        dir.join("inventory_storage_policy.json"),
        dir.join("data").join("inventory_storage_policy.json"),
    ];
    for path in candidates {
        if let Ok(policy) = StoragePolicy::load(&path) {
            return Some(policy);
        }
    }
    None
}

/// RB11 `migrate_inventory_storage_arrays.ts`:
/// Normalizes `instancedItems` and `stackedItems` (whether stored as JSON strings or BSON arrays)
/// and applies `StoragePolicy` relocation when a policy is provided.
pub fn transform_inventory_arrays(
    instanced_raw: Option<&Value>,
    stacked_raw: Option<&Value>,
    policy: Option<&StoragePolicy>,
) -> (Vec<Value>, Vec<Value>) {
    let instanced_val = parse_json_or_value(instanced_raw, Value::Array(Vec::new()));
    let stacked_val = parse_json_or_value(stacked_raw, Value::Array(Vec::new()));
    let instanced = instanced_val.as_array().cloned().unwrap_or_default();
    let stacked = stacked_val.as_array().cloned().unwrap_or_default();

    if let Some(policy) = policy {
        let migrated = migrate_inventory_storage_arrays(policy, &instanced, &stacked);
        return (migrated.instanced_items, migrated.stacked_items);
    }

    (instanced, stacked)
}

/// Starter equipment slot -> starter catalog_id mapping (`starterManifest.ts`).
const STARTER_EQUIPMENT_SLOTS: [(&str, &str); 7] = [
    ("weapon", "WP_EB_BEGINNER"),
    ("helmet", "AR_UNEQUIPPED_HELM"),
    ("chest", "AR_BEGINNER_CHEST"),
    ("arms", "AR_BEGINNER_ARMS"),
    ("legs", "AR_BEGINNER_LEGS"),
    ("lantern", "LT_BASIC"),
    ("flask", STARTER_FLASK_CATALOG_ID),
];

/// RB11 `migrate_starter_instances.ts` + `migrate_banner_instances.ts`:
/// 1. Repairs invalid or duplicate `instanceId`s on starter items in `instanced_items` and seeds any
///    missing starter instanced items when the character has loadouts referencing them.
/// 2. Rewrites loadout slots (`weapon`, `helmet`, `chest`, `arms`, `legs`, `lantern`, `flask`) and
///    `persistent` (`banner`, `glider`) so every starter reference points to a valid owned `instanceId`.
pub fn repair_starter_and_banner_instances(
    user_id: &str,
    character_id: &str,
    instanced_items: &mut Vec<Value>,
    loadouts_opt: Option<&mut Vec<Value>>,
    persistent_opt: Option<&mut Value>,
) {
    let starter_set: HashSet<&str> = STARTER_INSTANCED_CATALOG_IDS.iter().copied().collect();
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut catalog_to_instance: HashMap<String, String> = HashMap::new();

    for (idx, item) in instanced_items.iter_mut().enumerate() {
        let Some(obj) = item.as_object_mut() else {
            continue;
        };
        let Some(catalog_id) = obj.get("catalogId").and_then(Value::as_str).map(str::to_owned) else {
            continue;
        };
        let current_id = obj
            .get("instanceId")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_default();

        let valid_and_unique = is_instance_id(&current_id) && !seen_ids.contains(&current_id);
        let final_id = if valid_and_unique {
            current_id
        } else if starter_set.contains(catalog_id.as_str()) || !is_instance_id(&current_id) {
            let mut candidate_ord = idx;
            loop {
                let candidate = deterministic_instance_id(user_id, character_id, &catalog_id, candidate_ord);
                if !seen_ids.contains(&candidate) {
                    break candidate;
                }
                candidate_ord += 1;
            }
        } else {
            current_id
        };

        seen_ids.insert(final_id.clone());
        catalog_to_instance
            .entry(catalog_id)
            .or_insert_with(|| final_id.clone());
        obj.insert("instanceId".into(), Value::String(final_id));
    }

    // If loadouts or persistent exist, ensure starter banner, glider, and equipment exist in inventory
    // when referenced or missing, matching migrate_starter_instances.ts + migrate_banner_instances.ts.
    let has_loadout_context = loadouts_opt.is_some() || persistent_opt.is_some();
    if has_loadout_context {
        let now_iso = "2026-01-01T00:00:00.000Z";
        for &starter_cid in &[STARTER_BANNER_CATALOG_ID, STARTER_GLIDER_CATALOG_ID] {
            if !catalog_to_instance.contains_key(starter_cid) {
                let mut ord = instanced_items.len();
                let new_id = loop {
                    let cand = deterministic_instance_id(user_id, character_id, starter_cid, ord);
                    if !seen_ids.contains(&cand) {
                        break cand;
                    }
                    ord += 1;
                };
                seen_ids.insert(new_id.clone());
                catalog_to_instance.insert(starter_cid.to_owned(), new_id.clone());
                instanced_items.push(json!({
                    "instanceId": new_id,
                    "catalogId": starter_cid,
                    "grantDate": now_iso,
                    "markedNew": false,
                    "craftingTrackId": "",
                    "durability": 100,
                    "currentXp": 0,
                    "surged": 0,
                    "talents": "0000000000000000000000000"
                }));
            }
        }
    }

    if let Some(loadouts) = loadouts_opt {
        for slot in loadouts.iter_mut() {
            let Some(slot_obj) = slot.as_object_mut() else {
                continue;
            };
            for &(slot_key, starter_cid) in &STARTER_EQUIPMENT_SLOTS {
                let Some(starter_inst_id) = catalog_to_instance.get(starter_cid) else {
                    continue;
                };
                if let Some(Value::Object(equip)) = slot_obj.get_mut(slot_key) {
                    let cur = equip.get("instance_id").and_then(Value::as_str).unwrap_or_default();
                    if !is_instance_id(cur) || !seen_ids.contains(cur) {
                        equip.insert("instance_id".into(), Value::String(starter_inst_id.clone()));
                    }
                }
            }
        }
    }

    if let Some(Value::Object(pers)) = persistent_opt {
        if let Some(banner_inst_id) = catalog_to_instance.get(STARTER_BANNER_CATALOG_ID) {
            let banner_entry = pers.entry("banner".to_owned()).or_insert_with(|| json!({}));
            if !banner_entry.is_object() {
                *banner_entry = json!({});
            }
            if let Some(banner) = banner_entry.as_object_mut() {
                let fabric = banner.get("fabric_id").and_then(Value::as_str).unwrap_or_default();
                if !is_instance_id(fabric) || !seen_ids.contains(fabric) {
                    banner.insert("fabric_id".into(), Value::String(banner_inst_id.clone()));
                }
                let pole = banner.get("pole_id").and_then(Value::as_str).unwrap_or_default();
                if !is_instance_id(pole) || !seen_ids.contains(pole) {
                    banner.insert("pole_id".into(), Value::String(banner_inst_id.clone()));
                }
                if banner
                    .get("sigil_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .is_empty()
                {
                    banner.insert("sigil_id".into(), Value::String(DEFAULT_BANNER_SIGIL_ID.into()));
                }
                if banner
                    .get("dye_aspect1_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .is_empty()
                {
                    banner.insert("dye_aspect1_id".into(), Value::String(DEFAULT_BANNER_DYE_ID.into()));
                }
                if banner
                    .get("dye_aspect2_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .is_empty()
                {
                    banner.insert("dye_aspect2_id".into(), Value::String(DEFAULT_BANNER_DYE_ID.into()));
                }
                if banner
                    .get("sigil_PrimaryDyeId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .is_empty()
                {
                    banner.insert("sigil_PrimaryDyeId".into(), Value::String(DEFAULT_BANNER_DYE_ID.into()));
                }
                if banner
                    .get("sigil_SecondaryDyeId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .is_empty()
                {
                    banner.insert(
                        "sigil_SecondaryDyeId".into(),
                        Value::String(DEFAULT_BANNER_DYE_ID.into()),
                    );
                }
                banner
                    .entry("border_id")
                    .or_insert_with(|| Value::String(String::new()));
                banner.entry("title_id").or_insert_with(|| Value::String(String::new()));
                banner
                    .entry("landing_id")
                    .or_insert_with(|| Value::String(String::new()));
                banner.entry("update_version").or_insert_with(|| json!(0));
            }
        }

        if let Some(glider_inst_id) = catalog_to_instance.get(STARTER_GLIDER_CATALOG_ID) {
            let glider_entry = pers.entry("glider".to_owned()).or_insert_with(|| json!({}));
            if !glider_entry.is_object() {
                *glider_entry = json!({});
            }
            if let Some(glider) = glider_entry.as_object_mut() {
                let inst = glider.get("instance_id").and_then(Value::as_str).unwrap_or_default();
                if !is_instance_id(inst) || !seen_ids.contains(inst) {
                    glider.insert("instance_id".into(), Value::String(glider_inst_id.clone()));
                }
                if glider
                    .get("PrimaryDyeId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .is_empty()
                {
                    glider.insert("PrimaryDyeId".into(), Value::String(DEFAULT_BANNER_DYE_ID.into()));
                }
                if glider
                    .get("SecondaryDyeId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .is_empty()
                {
                    glider.insert("SecondaryDyeId".into(), Value::String(DEFAULT_BANNER_DYE_ID.into()));
                }
                glider
                    .entry("Aspect1Id")
                    .or_insert_with(|| Value::String(String::new()));
                glider.entry("update_version").or_insert_with(|| json!(0));
            }
        }
    }
}

/// RB11 `migrate_loadout_slots.ts`:
/// Reconciles `unlocked_total_slots`, `loadouts` array length, and `active_index` against the
/// account's `player_journey` nodes.
pub fn reconcile_loadout_slots(
    loadouts: &mut Vec<Value>,
    unlocked_total_slots: Option<f64>,
    active_index: Option<i64>,
    journey_nodes: Option<&Value>,
) -> (i64, i64) {
    let journey_total = i64::from(resolve_player_journey_total(journey_nodes));
    let persisted = unlocked_total_slots.map(|f| (f as i64).clamp(1, i64::from(MAX_TOTAL_LOADOUT_SLOTS)));
    let target_total = resolve_requested_total(persisted, journey_total)
        .unwrap_or(DEFAULT_ACCOUNT_LOADOUT_SLOTS)
        .clamp(1, MAX_TOTAL_LOADOUT_SLOTS) as usize;

    if !loadouts.is_empty() {
        let template = loadouts[0].clone();
        while loadouts.len() < target_total {
            let idx = loadouts.len();
            let mut cloned = template.clone();
            if let Some(obj) = cloned.as_object_mut() {
                obj.insert("custom_name".into(), Value::String(String::new()));
                obj.insert("update_version".into(), json!(0));
                obj.insert("slot_index".into(), json!(idx));
            }
            loadouts.push(cloned);
        }
    }

    let final_total = loadouts.len().max(target_total) as i64;
    let raw_active = active_index.unwrap_or(0);
    let clamped_active = if loadouts.is_empty() {
        0
    } else {
        raw_active.clamp(0, (loadouts.len() as i64) - 1)
    };
    (final_total, clamped_active)
}

/// Flattens a Mongo `economyDaily` document into sorted `(dotted_path, f64)` pairs for `economy_rollup_counters`.
pub fn flatten_economy_rollup(doc: &Map<String, Value>) -> Vec<(String, f64)> {
    let mut out = BTreeMap::new();
    for (k, v) in doc {
        if matches!(
            k.as_str(),
            "_id" | "userId" | "day" | "currency" | "ttlAt" | "expiresAt" | "__v"
        ) {
            continue;
        }
        flatten_value(k, v, &mut out);
    }
    out.into_iter().collect()
}

fn flatten_value(prefix: &str, val: &Value, out: &mut BTreeMap<String, f64>) {
    match val {
        Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                out.insert(prefix.to_owned(), f);
            }
        }
        Value::Object(map) => {
            for (k, v) in map {
                let next = format!("{prefix}.{k}");
                flatten_value(&next, v, out);
            }
        }
        _ => {}
    }
}
