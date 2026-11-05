//! `controllers/starterManifest.ts`: what a new account is seeded with and the repair a loadout or inventory read does
//! when a record is missing. The instanced items and the loadout are fixed; the stacked list,
//! the bootstrap version and the default quick-emoji wheel are the version's table (RU3c `STARTER_MANIFEST`).
//! Inventory reads reuse [`ensure_in_transaction`] and [`needs_repair`].

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value, json};
use service_kit::jsjson;

use crate::repo::bounties::WalletRecord;
use crate::repo::characters::LoadoutRecord;
use crate::repo::inventory::InventoryRecord;
use crate::repo::{AccountId, RepoError, RepoResult, Repositories, Tx};
use crate::tables::VersionTables;

/// 26 uppercase alphanumerics, the shape of every captured instance id
const INSTANCE_ID_ALPHABET: &[u8; 36] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
const INSTANCE_ID_LENGTH: usize = 26;

/// A fresh per-item instance id. Each character is drawn uniformly (a byte modulo 36 would favour the first four).
pub fn generate_instance_id() -> String {
    let mut id = String::with_capacity(INSTANCE_ID_LENGTH);
    while id.len() < INSTANCE_ID_LENGTH {
        for byte in crate::game_data::season_window::next_uuid().as_bytes() {
            // 252 = 7 * 36: the bytes above it would bias the draw
            if *byte < 252 && id.len() < INSTANCE_ID_LENGTH {
                id.push(INSTANCE_ID_ALPHABET[usize::from(*byte) % 36] as char);
            }
        }
    }
    id
}

/// Each owned starter weapon must also own the default part its slot data names, or the client shows no mod row.
pub const STARTER_DEFAULT_WEAPON_PART_CATALOG_IDS: [&str; 8] = [
    "PART_EB_SPECIAL_DEFAULT",
    "PART_IH_SPECIAL_DEFAULT",
    "PART_GA_SPECIAL_DEFAULT",
    "PART_CB_SPECIAL_DEFAULT",
    "PART_MS_SPECIAL_PROJECTILE",
    "PART_DP_RECEIVER_DEFAULT",
    "PART_DP_GRIP_DEFAULT",
    "PART_AC_SPECIAL_DEFAULT",
];

/// The 23 instanced items every new account owns.
pub const STARTER_INSTANCED_CATALOG_IDS: [&str; 23] = [
    "WP_EB_BEGINNER",
    "WP_GA_BEGINNER",
    "WP_IH_BEGINNER",
    "WP_CB_BEGINNER",
    "WP_MS_BEGINNER",
    "WP_DP_BEGINNER",
    "WP_AC_BEGINNER",
    "PART_EB_SPECIAL_DEFAULT",
    "PART_IH_SPECIAL_DEFAULT",
    "PART_GA_SPECIAL_DEFAULT",
    "PART_CB_SPECIAL_DEFAULT",
    "PART_MS_SPECIAL_PROJECTILE",
    "PART_DP_RECEIVER_DEFAULT",
    "PART_DP_GRIP_DEFAULT",
    "PART_AC_SPECIAL_DEFAULT",
    "AR_UNEQUIPPED_HELM",
    "AR_BEGINNER_CHEST",
    "AR_BEGINNER_ARMS",
    "AR_BEGINNER_LEGS",
    "LT_BASIC",
    "FL_HEALING_DEFAULT",
    "GD_FRAME_STARTER_BASE",
    "BN_BEGINNER_00",
];

pub const STARTER_WEAPON_CATALOG_ID: &str = "WP_EB_BEGINNER";
const STARTER_WEAPON_PART_CATALOG_ID: &str = "PART_EB_SPECIAL_DEFAULT";
pub const STARTER_BANNER_CATALOG_ID: &str = "BN_BEGINNER_00";
pub const STARTER_FLASK_CATALOG_ID: &str = "FL_HEALING_DEFAULT";
pub const STARTER_GLIDER_CATALOG_ID: &str = "GD_FRAME_STARTER_BASE";

const CHAT_WHEEL_SLOTS: usize = 9;
pub const DEFAULT_QUICK_CHATS: [&str; CHAT_WHEEL_SLOTS] = [
    "qc_hello",
    "qc_thankyou",
    "qc_goodjob",
    "qc_help",
    "qc_follow_me",
    "qc_onmyway",
    "qc_yes",
    "qc_no",
    "qc_sorry",
];

/// `STARTER_WALLET` (`controllers/wallet.ts`; the bounties-store area owns the wallet routes).
pub const STARTER_WALLET: [(&str, f64); 6] = [
    ("CURRENCY_NOTES", 5000.0),
    ("CURRENCY_PJM_WEAPON", 50.0),
    ("CURRENCY_PJM_ARMOR", 50.0),
    ("CURRENCY_PJM_ALCHEMY", 50.0),
    ("CURRENCY_PJM_AIRSHIP", 50.0),
    ("CURRENCY_PJM_FISHING", 50.0),
];

/// Weapon, then the armor pieces and lantern: the loadout's equipment, by key.
pub const LOADOUT_EQUIPMENT: [(&str, &str); 6] = [
    ("weapon", STARTER_WEAPON_CATALOG_ID),
    ("helmet", "AR_UNEQUIPPED_HELM"),
    ("chest", "AR_BEGINNER_CHEST"),
    ("arms", "AR_BEGINNER_ARMS"),
    ("legs", "AR_BEGINNER_LEGS"),
    ("lantern", "LT_BASIC"),
];

const STARTER_APPEARANCE: &str = "{\"CreationState\":\"EArchonCharacterCreationState::FaceComplete\",\"Data\":[{\"SkeletalMeshComponentName\":\"Head Slot\",\"MorphData\":[]}],\"AssetReferences\":[],\"StringData\":[{\"Key\":\"BodyType\",\"Data\":\"Feminine\"},{\"Key\":\"Hair\",\"Data\":\"Hair12\"},{\"Key\":\"SkinName\",\"Data\":\"Tan\"},{\"Key\":\"SkinValue\",\"Data\":\"(R=0.610496,G=0.417885,B=0.254152,A=1.000000)\"},{\"Key\":\"Hair_Color\",\"Data\":\"(R=0.196843,G=0.042531,B=0.019093,A=1.000000)\"},{\"Key\":\"Beard\",\"Data\":\"NoBeard\"},{\"Key\":\"Facepaint\",\"Data\":\"NoFacepaint\"},{\"Key\":\"Makeup\",\"Data\":\"NoMakeup\"}]}";

/// Thrown by Node when a bootstrap would run for a character the account doesn't own (`CharacterOwnershipError`).
#[derive(Debug, thiserror::Error)]
pub enum EnsureError {
    #[error("Character {character_id} does not belong to user {user}")]
    NotOwned { character_id: String, user: String },
    #[error(transparent)]
    Repo(#[from] RepoError),
}

fn is_empty_wheel(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Array(items)) => items.iter().all(|e| e.as_str().is_none_or(str::is_empty)),
        _ => true,
    }
}

/// `WithDefaultChatWheels` ([Reforged 0017]): fills a wheel with nothing on it with the defaults on the way out. Only
/// a version whose catalog has the emojis does it.
pub fn with_default_chat_wheels(persistent: Value, tables: &VersionTables) -> Value {
    let Some(emojis) = tables.starter_manifest.quick_emojis else {
        return persistent;
    };
    let Value::Object(mut map) = persistent else {
        return persistent;
    };
    if is_empty_wheel(map.get("quick_chats")) {
        map.insert("quick_chats".into(), json!(DEFAULT_QUICK_CHATS));
    }
    if is_empty_wheel(map.get("emojis")) {
        map.insert("emojis".into(), json!(emojis));
    }
    Value::Object(map)
}

pub fn build_instance_data() -> String {
    jsjson::stringify(&json!({
        "SheenType": 73,
        "IsPrimarySheenActive": true,
        "PrimaryDyeId": "None",
        "IsSecondarySheenActive": true,
        "SecondaryDyeId": "None",
        "IsTertiarySheenActive": false,
        "TertiaryDyeId": "None",
        "TransmogCatalogId": "None",
        "TransmogEnabled": false,
        "EquippedCells": [],
        "EquippedCellsv2": [],
        "SubTypeMetadataArray": {
            "ItemSubType": "subtype_eblade",
            "EquippedItemParts": [{ "WeaponPartId": STARTER_WEAPON_PART_CATALOG_ID, "SlotIndex": 0 }]
        }
    }))
}

fn missing(what: &str, catalog_id: &str) -> RepoError {
    RepoError::Data(format!("Missing inventory instance ID for starter {what} {catalog_id}"))
}

/// `persistent`: the banner and glider are inventory instance references, not catalog ids.
fn build_starter_persistent(ids: &HashMap<String, String>, tables: &VersionTables) -> RepoResult<Value> {
    let banner = ids
        .get(STARTER_BANNER_CATALOG_ID)
        .ok_or_else(|| missing("banner", STARTER_BANNER_CATALOG_ID))?;
    let glider = ids
        .get(STARTER_GLIDER_CATALOG_ID)
        .ok_or_else(|| missing("glider", STARTER_GLIDER_CATALOG_ID))?;
    let customization = jsjson::stringify(&json!({
        "BannerMeshItemID": "BNC_MESH_BEGINNER_00", "FabricMaterialItemID": "BNC_FABRIC_BEGINNER_00",
        "SigilTextureItemID": "BNC_SIGIL_BEGINNER_00", "PlantVFXItemID": "", "PersistantStandardVFXItemID": "",
        "AnimationItemID": "BNC_ANIMATION_BEGINNER_00", "BackgroundColourItemID": "DYE_GREEN04_DURABLE",
        "BorderColourItemID": "DYE_BROWN07_DURABLE", "SigilColourItemID": "DYE_BROWN07_DURABLE",
        "BackgroundSheenType": 0, "BorderSheenType": 0, "SigilSheenType": 0
    }));
    let (quick_chats, emojis): (Vec<&str>, Vec<&str>) = match tables.starter_manifest.quick_emojis {
        Some(emojis) => (DEFAULT_QUICK_CHATS.to_vec(), emojis.to_vec()),
        None => (vec![""; CHAT_WHEEL_SLOTS], vec![""; CHAT_WHEEL_SLOTS]),
    };
    let curiosities: Vec<Value> = (0..8)
        .map(|item_index| json!({ "item_index": item_index, "item_id": "", "instance_id": "" }))
        .collect();
    Ok(json!({
        "manual_emotes": ["", "", "", "", "", ""],
        "intro_emote": "EM_INTRO_BEGINNER_01",
        "banner": banner,
        "bannerCustomization": customization,
        "flare": "QI_BASIC_FLARE_DURABLE", "title": "", "head_accessory": "", "back_accessory": "", "pet": "",
        "glider": glider, "update_version": 0,
        "quick_chats": quick_chats,
        "emojis": emojis,
        "quick_curiosities_items": curiosities,
        "quickwheel": []
    }))
}

/// A slot built from the inventory ids the caller holds, so a repair never needs a shared id.
pub fn build_starter_loadout_slot(ids: &HashMap<String, String>, persistent: &Value) -> RepoResult<Value> {
    let instance_data = build_instance_data();
    let mut slot = Map::new();
    for (key, catalog_id) in LOADOUT_EQUIPMENT {
        let instance_id = ids.get(catalog_id).ok_or_else(|| {
            RepoError::Data(format!(
                "Missing inventory instance ID for starter loadout item {catalog_id}"
            ))
        })?;
        slot.insert(
            key.into(),
            json!({
                "item_id": catalog_id,
                "instance_id": instance_id,
                "instance_data": if catalog_id == STARTER_WEAPON_CATALOG_ID { Value::String(instance_data.clone()) } else { Value::Null }
            }),
        );
    }
    // The flask is the owned flask's instance id, like the banner (the catalog id resolves to no owned item)
    let flask = ids
        .get(STARTER_FLASK_CATALOG_ID)
        .ok_or_else(|| missing("flask", STARTER_FLASK_CATALOG_ID))?;
    slot.insert("player_role".into(), Value::Null);
    slot.insert("subweapon".into(), Value::Null);
    slot.insert("appearance".into(), json!(STARTER_APPEARANCE));
    slot.insert("flask".into(), json!(flask));
    slot.insert("quick_items".into(), json!([]));
    slot.insert("slot_index".into(), json!(0));
    slot.insert("update_version".into(), json!(0));
    slot.insert("custom_name".into(), json!(""));
    slot.insert("persistent".into(), persistent.clone());
    Ok(Value::Object(slot))
}

/// One complete manifest with fresh ids.
#[derive(Debug, Clone)]
pub struct Manifest {
    pub instanced_items: Vec<Value>,
    pub stacked_items: Vec<Value>,
    pub loadout_slot: Value,
    pub persistent: Value,
}

pub fn build_manifest(tables: &VersionTables) -> RepoResult<Manifest> {
    let ids: HashMap<String, String> = STARTER_INSTANCED_CATALOG_IDS
        .iter()
        .map(|id| ((*id).to_owned(), generate_instance_id()))
        .collect();
    let persistent = build_starter_persistent(&ids, tables)?;
    let instanced_items = STARTER_INSTANCED_CATALOG_IDS
        .iter()
        .map(|id| json!({ "catalogId": id, "instanceId": ids[*id], "itemData": null, "updateVersion": 0 }))
        .collect();
    Ok(Manifest {
        instanced_items,
        stacked_items: tables
            .starter_manifest
            .stacked
            .iter()
            .map(|id| json!({ "catalogId": id, "quantity": 1 }))
            .collect(),
        loadout_slot: build_starter_loadout_slot(&ids, &persistent)?,
        persistent,
    })
}

fn parse_array(raw: &str) -> Vec<Value> {
    match jsjson::parse(raw) {
        Ok(Value::Array(items)) => items,
        _ => Vec::new(),
    }
}

fn non_empty_str<'a>(item: &'a Value, key: &str) -> Option<&'a str> {
    item.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

fn fresh_instance_id(used: &mut HashSet<String>) -> String {
    loop {
        let id = generate_instance_id();
        if used.insert(id.clone()) {
            return id;
        }
    }
}

/// Appends only the absent default weapon parts; a normal read never restores removed gear or rewrites item data.
fn reconcile_default_weapon_parts(existing: &[Value]) -> (Vec<Value>, Vec<&'static str>) {
    let mut items = existing.to_vec();
    let owned: HashSet<String> = items
        .iter()
        .filter_map(|item| non_empty_str(item, "catalogId").map(str::to_owned))
        .collect();
    let mut used: HashSet<String> = items
        .iter()
        .filter_map(|item| non_empty_str(item, "instanceId").map(str::to_owned))
        .collect();
    let mut added = Vec::new();
    for id in STARTER_DEFAULT_WEAPON_PART_CATALOG_IDS {
        if owned.contains(id) {
            continue;
        }
        let instance_id = fresh_instance_id(&mut used);
        items.push(json!({ "catalogId": id, "instanceId": instance_id, "itemData": null, "updateVersion": 0 }));
        added.push(id);
    }
    (items, added)
}

/// The stacked sibling: appends only absent starter entries (the daily-bounty draft token reached old accounts so).
fn reconcile_missing_stacked(existing: &[Value], tables: &VersionTables) -> (Vec<Value>, Vec<&'static str>) {
    let mut items = existing.to_vec();
    let owned: HashSet<String> = items
        .iter()
        .filter_map(|item| non_empty_str(item, "catalogId").map(str::to_owned))
        .collect();
    let mut added = Vec::new();
    for id in tables.starter_manifest.stacked {
        if owned.contains(*id) {
            continue;
        }
        items.push(json!({ "catalogId": id, "quantity": 1 }));
        added.push(*id);
    }
    (items, added)
}

fn is_instance_id(text: &str) -> bool {
    text.len() == INSTANCE_ID_LENGTH && text.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

struct Repaired {
    items: Vec<Value>,
    ids: HashMap<String, String>,
    changed: bool,
}

/// Keeps every valid item; only what a missing inventory or loadout needs is repaired, each with a fresh unique id.
fn repair_instanced_items(existing: &[Value], source_loadout: Option<&Value>) -> Repaired {
    let mut items: Vec<Value> = existing.iter().filter(|item| item.is_object()).cloned().collect();
    let mut changed = items.len() != existing.len();
    let mut used = HashSet::new();
    let mut ids: HashMap<String, String> = HashMap::new();
    for item in &items {
        if let Some(instance_id) = non_empty_str(item, "instanceId")
            && used.insert(instance_id.to_owned())
            && let Some(catalog_id) = item.get("catalogId").and_then(Value::as_str)
        {
            ids.entry(catalog_id.to_owned())
                .or_insert_with(|| instance_id.to_owned());
        }
    }
    // A surviving loadout is an id source only for ids in the observed 26-character format
    let mut loadout_ids: HashMap<&str, String> = HashMap::new();
    for (key, catalog_id) in LOADOUT_EQUIPMENT {
        let equipment = source_loadout.and_then(|l| l.get(key));
        let item_id = equipment.and_then(|e| e.get("item_id")).and_then(Value::as_str);
        let instance_id = equipment.and_then(|e| e.get("instance_id")).and_then(Value::as_str);
        if let (Some(item_id), Some(instance_id)) = (item_id, instance_id)
            && item_id == catalog_id
            && is_instance_id(instance_id)
        {
            loadout_ids.insert(catalog_id, instance_id.to_owned());
        }
    }
    if let Some(flask) = source_loadout.and_then(|l| l.get("flask")).and_then(Value::as_str)
        && is_instance_id(flask)
    {
        loadout_ids.insert(STARTER_FLASK_CATALOG_ID, flask.to_owned());
    }
    for catalog_id in STARTER_INSTANCED_CATALOG_IDS {
        if ids.contains_key(catalog_id) {
            continue;
        }
        let instance_id = match loadout_ids.get(catalog_id) {
            Some(candidate) if !used.contains(candidate) => {
                used.insert(candidate.clone());
                candidate.clone()
            }
            _ => fresh_instance_id(&mut used),
        };
        let existing_item = items
            .iter_mut()
            .find(|item| item.get("catalogId").and_then(Value::as_str) == Some(catalog_id));
        if let Some(Value::Object(item)) = existing_item {
            item.insert("instanceId".into(), json!(instance_id));
            if item.get("itemData").is_none_or(Value::is_null) {
                item.insert("itemData".into(), Value::Null);
            }
            if !item.get("updateVersion").is_some_and(Value::is_number) {
                item.insert("updateVersion".into(), json!(0));
            }
        } else {
            items.push(
                json!({ "catalogId": catalog_id, "instanceId": instance_id, "itemData": null, "updateVersion": 0 }),
            );
        }
        ids.insert(catalog_id.to_owned(), instance_id);
        changed = true;
    }
    Repaired { items, ids, changed }
}

fn dump(items: &[Value]) -> String {
    jsjson::stringify(&Value::Array(items.to_vec()))
}

/// Whether a read must open a write: a record is missing, or a reconcile would add something.
pub fn needs_repair(
    inventory: Option<&InventoryRecord>,
    loadout: Option<&LoadoutRecord>,
    tables: &VersionTables,
) -> bool {
    let (Some(inventory), Some(_)) = (inventory, loadout) else {
        return true;
    };
    !reconcile_default_weapon_parts(&parse_array(&inventory.instanced_items))
        .1
        .is_empty()
        || !reconcile_missing_stacked(&parse_array(&inventory.stacked_items), tables)
            .1
            .is_empty()
}

/// `EnsureStarterBootstrapRecordsInTransaction`: the ownership gate first (recovery can create records), then the
/// repair of whichever record is missing. A complete account gets only the two additive reconciles.
pub fn ensure_in_transaction(
    tx: &mut Tx<'_>,
    repos: &Repositories,
    tables: &VersionTables,
    account: &AccountId,
    user: &str,
    character_id: &str,
) -> Result<(InventoryRecord, LoadoutRecord), EnsureError> {
    if repos.characters.find(tx.conn(), character_id, account)?.is_none() {
        return Err(EnsureError::NotOwned {
            character_id: character_id.to_owned(),
            user: user.to_owned(),
        });
    }
    let version = tables.starter_manifest.bootstrap_version;
    let mut inventory = repos.inventories.find_by_character_id(tx.conn(), character_id)?;
    let loadout = repos.loadouts.find(tx.conn(), character_id, account)?;
    match (inventory.take(), loadout) {
        (Some(mut inventory), Some(loadout)) => {
            let (instanced, added_parts) = reconcile_default_weapon_parts(&parse_array(&inventory.instanced_items));
            let (stacked, added_stacked) = reconcile_missing_stacked(&parse_array(&inventory.stacked_items), tables);
            if !added_parts.is_empty() || !added_stacked.is_empty() {
                inventory = repos
                    .inventories
                    .update_both_if_revision_matches(
                        tx,
                        character_id,
                        &dump(&instanced),
                        &dump(&stacked),
                        inventory.revision.unwrap_or(0),
                    )?
                    .ok_or_else(|| {
                        RepoError::Data(format!(
                            "Default weapon-part reconciliation conflict for inventory {character_id}"
                        ))
                    })?;
                if !added_parts.is_empty() {
                    tracing::info!(
                        "[inventory] reconciled missing default weapon parts [{}] for characterId {character_id} (userId {user})",
                        added_parts.join(", ")
                    );
                }
                if !added_stacked.is_empty() {
                    tracing::info!(
                        "[inventory] reconciled missing starter stacked items [{}] for characterId {character_id} (userId {user})",
                        added_stacked.join(", ")
                    );
                }
            }
            Ok((inventory, loadout))
        }
        (None, None) => {
            let manifest = build_manifest(tables)?;
            let inventory = InventoryRecord {
                character_id: character_id.to_owned(),
                account: Some(account.clone()),
                instanced_items: dump(&manifest.instanced_items),
                stacked_items: dump(&manifest.stacked_items),
                revision: Some(0),
                bootstrap_version: Some(version.to_owned()),
            };
            let loadout = new_loadout(
                account,
                character_id,
                dump(&[manifest.loadout_slot]),
                jsjson::stringify(&manifest.persistent),
                version,
            );
            repos.inventories.create(tx, &inventory)?;
            repos.loadouts.create(tx, &loadout)?;
            Ok((inventory, loadout))
        }
        (None, Some(loadout)) => {
            let source = parse_array(&loadout.loadouts).into_iter().next();
            let repaired = repair_instanced_items(&[], source.as_ref());
            let stacked: Vec<Value> = tables
                .starter_manifest
                .stacked
                .iter()
                .map(|id| json!({ "catalogId": id, "quantity": 1 }))
                .collect();
            let inventory = InventoryRecord {
                character_id: character_id.to_owned(),
                account: Some(account.clone()),
                instanced_items: dump(&repaired.items),
                stacked_items: dump(&stacked),
                revision: Some(0),
                bootstrap_version: Some(version.to_owned()),
            };
            repos.inventories.create(tx, &inventory)?;
            Ok((inventory, loadout))
        }
        (Some(mut inventory), None) => {
            let repaired = repair_instanced_items(&parse_array(&inventory.instanced_items), None);
            if repaired.changed {
                inventory = repos
                    .inventories
                    .update_both_if_revision_matches(
                        tx,
                        character_id,
                        &dump(&repaired.items),
                        &inventory.stacked_items,
                        inventory.revision.unwrap_or(0),
                    )?
                    .ok_or_else(|| {
                        RepoError::Data(format!("Starter recovery conflict for inventory {character_id}"))
                    })?;
            }
            let persistent = build_starter_persistent(&repaired.ids, tables)?;
            let slot = build_starter_loadout_slot(&repaired.ids, &persistent)?;
            let loadout = new_loadout(
                account,
                character_id,
                dump(&[slot]),
                jsjson::stringify(&persistent),
                version,
            );
            repos.loadouts.create(tx, &loadout)?;
            Ok((inventory, loadout))
        }
    }
}

fn new_loadout(
    account: &AccountId,
    character_id: &str,
    loadouts: String,
    persistent: String,
    version: &str,
) -> LoadoutRecord {
    LoadoutRecord {
        character_id: character_id.to_owned(),
        account: account.clone(),
        loadouts,
        persistent,
        active_index: None,
        unlocked_total_slots: None,
        revision: Some(0),
        bootstrap_version: Some(version.to_owned()),
    }
}

/// `SeedNewAccountAtomically`: inventory, loadout and wallet together, tagged with the bootstrap version. A wallet a
/// balance request created first is adopted unchanged.
pub fn seed_new_account(
    tx: &mut Tx<'_>,
    repos: &Repositories,
    tables: &VersionTables,
    account: &AccountId,
    character_id: &str,
) -> RepoResult<Manifest> {
    let manifest = build_manifest(tables)?;
    let version = tables.starter_manifest.bootstrap_version;
    repos.inventories.create(
        tx,
        &InventoryRecord {
            character_id: character_id.to_owned(),
            account: Some(account.clone()),
            instanced_items: dump(&manifest.instanced_items),
            stacked_items: dump(&manifest.stacked_items),
            revision: Some(0),
            bootstrap_version: Some(version.to_owned()),
        },
    )?;
    repos.loadouts.create(
        tx,
        &new_loadout(
            account,
            character_id,
            dump(std::slice::from_ref(&manifest.loadout_slot)),
            jsjson::stringify(&manifest.persistent),
            version,
        ),
    )?;
    repos.wallets.create_if_missing(
        tx,
        &WalletRecord {
            account: account.clone(),
            balances: STARTER_WALLET.iter().map(|(id, n)| ((*id).to_owned(), *n)).collect(),
            bootstrap_version: Some(version.to_owned()),
        },
    )?;
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tables(emojis: bool) -> VersionTables {
        crate::areas::characters::test_support::sample_tables(emojis)
    }

    #[test]
    fn instance_ids_have_the_captured_shape_and_do_not_repeat() {
        let ids: HashSet<String> = (0..200).map(|_| generate_instance_id()).collect();
        assert_eq!(ids.len(), 200);
        assert!(ids.iter().all(|id| is_instance_id(id)));
    }

    #[test]
    fn a_manifest_is_internally_consistent() {
        let manifest = build_manifest(&tables(true)).unwrap();
        assert_eq!(manifest.instanced_items.len(), 23);
        let by_catalog: HashMap<&str, &str> = manifest
            .instanced_items
            .iter()
            .map(|i| (i["catalogId"].as_str().unwrap(), i["instanceId"].as_str().unwrap()))
            .collect();
        let slot = &manifest.loadout_slot;
        for (key, catalog_id) in LOADOUT_EQUIPMENT {
            assert_eq!(slot[key]["item_id"], catalog_id);
            assert_eq!(slot[key]["instance_id"], by_catalog[catalog_id]);
        }
        assert_eq!(slot["flask"], by_catalog["FL_HEALING_DEFAULT"]);
        assert_eq!(manifest.persistent["banner"], by_catalog["BN_BEGINNER_00"]);
        assert_eq!(manifest.persistent["glider"], by_catalog["GD_FRAME_STARTER_BASE"]);
        assert_eq!(slot["persistent"], manifest.persistent);
        assert!(
            slot["weapon"]["instance_data"]
                .as_str()
                .unwrap()
                .contains("PART_EB_SPECIAL_DEFAULT")
        );
        assert!(slot["helmet"]["instance_data"].is_null());
        assert_eq!(manifest.persistent["quick_chats"][0], "qc_hello");
        assert_eq!(manifest.persistent["emojis"][8], "");
    }

    #[test]
    fn a_version_without_the_emojis_seeds_empty_wheels_and_leaves_them() {
        let manifest = build_manifest(&tables(false)).unwrap();
        assert_eq!(manifest.persistent["quick_chats"], json!(vec![""; 9]));
        assert_eq!(manifest.persistent["emojis"], json!(vec![""; 9]));
        let persistent = json!({ "quick_chats": [], "emojis": [""] });
        assert_eq!(with_default_chat_wheels(persistent.clone(), &tables(false)), persistent);
    }

    #[test]
    fn an_empty_wheel_gets_the_defaults_and_a_chosen_one_is_kept() {
        let persistent = json!({ "quick_chats": ["", ""], "emojis": ["EJ_PLAYER_ZAI_ANGRY"], "other": 1 });
        let filled = with_default_chat_wheels(persistent, &tables(true));
        assert_eq!(filled["quick_chats"][1], "qc_thankyou");
        assert_eq!(filled["emojis"], json!(["EJ_PLAYER_ZAI_ANGRY"]));
        assert_eq!(filled["other"], 1);
        let not_an_object = json!([1]);
        assert_eq!(
            with_default_chat_wheels(not_an_object.clone(), &tables(true)),
            not_an_object
        );
    }

    #[test]
    fn reconcile_adds_only_what_is_absent() {
        let manifest = build_manifest(&tables(true)).unwrap();
        let (same, added) = reconcile_default_weapon_parts(&manifest.instanced_items);
        assert!(added.is_empty());
        assert_eq!(same, manifest.instanced_items);
        let trimmed: Vec<Value> = manifest
            .instanced_items
            .iter()
            .filter(|i| i["catalogId"] != "PART_DP_GRIP_DEFAULT")
            .cloned()
            .collect();
        let (items, added) = reconcile_default_weapon_parts(&trimmed);
        assert_eq!(added, ["PART_DP_GRIP_DEFAULT"]);
        assert_eq!(items.len(), 23);

        let (stacked, added) = reconcile_missing_stacked(
            &[json!({ "catalogId": "QI_BASIC_FLARE_DURABLE", "quantity": 9 })],
            &tables(true),
        );
        assert_eq!(stacked[0]["quantity"], 9);
        assert_eq!(added.len(), tables(true).starter_manifest.stacked.len() - 1);
    }

    #[test]
    fn a_missing_inventory_keeps_the_loadouts_valid_instance_ids() {
        let manifest = build_manifest(&tables(true)).unwrap();
        let repaired = repair_instanced_items(&[], Some(&manifest.loadout_slot));
        let weapon = manifest.loadout_slot["weapon"]["instance_id"].as_str().unwrap();
        assert_eq!(repaired.ids["WP_EB_BEGINNER"], weapon);
        assert_eq!(
            repaired.ids["FL_HEALING_DEFAULT"],
            manifest.loadout_slot["flask"].as_str().unwrap()
        );
        assert_eq!(repaired.items.len(), 23);
        assert!(repaired.changed);
        // A placeholder that isn't a 26-character id is never propagated
        let mut placeholder = manifest.loadout_slot.clone();
        placeholder["weapon"]["instance_id"] = json!("RUSTIC_ISLES_STARTER_WEAPON");
        assert_ne!(
            repair_instanced_items(&[], Some(&placeholder)).ids["WP_EB_BEGINNER"],
            "RUSTIC_ISLES_STARTER_WEAPON"
        );
    }

    #[test]
    fn a_repair_gives_an_existing_item_a_new_id_and_fills_its_blanks() {
        let items = vec![
            json!({ "catalogId": "LT_BASIC", "instanceId": "", "updateVersion": "x" }),
            json!("junk"),
        ];
        let repaired = repair_instanced_items(&items, None);
        let lantern = repaired.items.iter().find(|i| i["catalogId"] == "LT_BASIC").unwrap();
        assert!(is_instance_id(lantern["instanceId"].as_str().unwrap()));
        assert_eq!(lantern["itemData"], Value::Null);
        assert_eq!(lantern["updateVersion"], 0);
        assert_eq!(repaired.items.len(), 23);
    }
}
