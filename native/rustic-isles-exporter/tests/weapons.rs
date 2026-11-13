//! The weapon-slot export (spec §6.3) on a synthetic image: its four files byte for byte.
#![allow(clippy::unwrap_used, reason = "test helpers fail the test")]

mod common;

use std::collections::HashMap;

use catalog_exporter::export::weapons;
use catalog_exporter::memory::{Calls, NoCalls};
use catalog_exporter::sdk;
use common::{World, ctx, read, status, status_text, temp_dir};

fn crlf(lines: &[&str]) -> String {
    lines.iter().map(|line| format!("{line}\r\n")).collect()
}

/// `ProcessEvent` answering the three weapon functions (spec §4.2) from prepared arrays.
struct FakeWeapons {
    slots: u64,
    equipped: u64,
    locked: u64,
    /// weapon -> (slots data, count, equipped data, count)
    arrays: HashMap<u64, (u64, i32, u64, i32)>,
    /// The part types `IsItemPartSlotLocked` answers true for.
    locked_types: Vec<u8>,
}

impl Calls for FakeWeapons {
    fn process_event(&self, object: u64, function: u64, params: &mut [u64]) -> bool {
        let Some(&(slots, slot_count, equipped, equipped_count)) = self.arrays.get(&object) else {
            return false;
        };
        let (out, data, count) = if function == self.slots {
            (sdk::WS_SLOTS_OUT.offset, slots, slot_count)
        } else if function == self.equipped {
            (sdk::WS_EQUIPPED_OUT.offset, equipped, equipped_count)
        } else if function == self.locked {
            let mut bytes: Vec<u8> = params.iter().flat_map(|word| word.to_le_bytes()).collect();
            let kind = bytes[sdk::WS_LOCKED_TYPE.offset as usize];
            bytes[sdk::WS_LOCKED_RETURN.offset as usize] = u8::from(self.locked_types.contains(&kind));
            for (word, chunk) in params.iter_mut().zip(bytes.chunks(8)) {
                *word = u64::from_le_bytes(chunk.try_into().unwrap());
            }
            return true;
        } else {
            return false;
        };
        let at = out as usize / 8;
        params[at] = data;
        params[at + 1] = u64::from(count as u32) | u64::from(count as u32) << 32;
        true
    }
}

fn function(world: &mut World, name: &str, owner: &str) -> u64 {
    let class = world.class_ptr("Function");
    let owner = world.class_ptr(owner);
    let function = world.raw_object(name, class, owner, 0x80);
    world.register(function);
    function
}

/// Slots `(type, default part, catalog locked, hidden)`.
fn slots(world: &mut World, entries: &[(i64, &str, bool, bool)]) -> (u64, i32) {
    let stride = sdk::WS_SLOTS_OUT.stride;
    let data = world.mem.alloc(entries.len().max(1) * stride as usize);
    for (index, (kind, part, locked, hidden)) in entries.iter().enumerate() {
        let slot = data + index as u64 * u64::from(stride);
        world.set_int(slot, &sdk::WS_SLOT_TYPE, *kind);
        // An empty part is a name left unset: index 0, which reads as None.
        if !part.is_empty() {
            world.set_text(slot, &sdk::WS_SLOT_DEFAULT, part);
        }
        if *locked {
            world.set_flag(slot, &sdk::WS_SLOT_LOCKED);
        }
        if *hidden {
            world.set_flag(slot, &sdk::WS_SLOT_HIDE);
        }
    }
    (data, i32::try_from(entries.len()).unwrap())
}

fn equipped(world: &mut World, entries: &[(i64, &str)]) -> (u64, i32) {
    let stride = sdk::WS_EQUIPPED_OUT.stride;
    let data = world.mem.alloc(entries.len().max(1) * stride as usize);
    for (index, (kind, part)) in entries.iter().enumerate() {
        let at = data + index as u64 * u64::from(stride);
        world.set_int(at, &sdk::WS_EQUIPPED_TYPE, *kind);
        world.set_text(at, &sdk::WS_EQUIPPED_ID, part);
    }
    (data, i32::try_from(entries.len()).unwrap())
}

fn weapon(world: &mut World, id: &str, instance: &str, weapon_type: i64) -> u64 {
    let weapon = world.object(&format!("Weapon_{id}"), "ArchonInventoryItem_Weapon", 0x60);
    world.set_text(weapon, &sdk::WS_ITEM_ID, id);
    world.set_text(weapon, &sdk::WS_INSTANCE_ID, instance);
    world.set_int(weapon, &sdk::WS_WEAPON_TYPE, weapon_type);
    weapon
}

/// The made-up starter weapon line the 1.14.7 module's test also reads (`rustic-isles-1-14-7` `export.rs`).
pub const STARTER_LINE: &str = r#"{"itemId":"WP_TEST_SWORD","instanceId":"inst-1","weaponType":1,"slots":[{"slotIndex":0,"weaponPartType":7,"weaponPartTypeName":"eb_special","defaultWeaponPartId":"PART_TEST_SPECIAL_DEFAULT","catalogLocked":false,"hideFromUI":false,"effectiveLocked":false},{"slotIndex":1,"weaponPartType":8,"weaponPartTypeName":"eb_passive","defaultWeaponPartId":"None","catalogLocked":true,"hideFromUI":true,"effectiveLocked":true}],"equippedParts":[{"slotIndex":0,"weaponPartType":7,"weaponPartTypeName":"eb_special","weaponPartId":"PART_TEST_SPECIAL_DEFAULT"}]}"#;

#[test]
fn weapon_slots_write_each_live_weapon_with_its_calls() {
    let mut world = World::new();
    world.class("ArchonInventoryItem", Some("Object"));
    world.class("ArchonInventoryItem_Weapon", Some("ArchonInventoryItem"));
    let get_slots = function(&mut world, "GetItemPartSlots", "ArchonInventoryItem_Weapon");
    let get_equipped = function(&mut world, "GetEquippedItemParts", "ArchonInventoryItem_Weapon");
    let is_locked = function(&mut world, "IsItemPartSlotLocked", "ArchonInventoryItem_Weapon");
    world.default_object(
        "Default__ArchonInventoryItem_Weapon",
        "ArchonInventoryItem_Weapon",
        0x60,
    );
    let sword = weapon(&mut world, "WP_TEST_SWORD", "inst-1", 1);
    // An unset FName reads as None (spec §5); the part type comes from the SDK's enum by name.
    let sword_slots = slots(
        &mut world,
        &[(7, "PART_TEST_SPECIAL_DEFAULT", false, false), (8, "None", true, true)],
    );
    let sword_parts = equipped(&mut world, &[(7, "PART_TEST_SPECIAL_DEFAULT")]);
    let nameless = world.object("Weapon_nameless", "ArchonInventoryItem_Weapon", 0x60);
    let axe = weapon(&mut world, "WP_TEST_AXE", "", 2);
    // 32 is Weapon_Part_Soul in the synthetic enum, 99 no enumerator at all.
    let axe_slots = slots(
        &mut world,
        &[(32, "PART_TEST_SOUL", false, false), (99, "", false, false)],
    );
    let fake = FakeWeapons {
        slots: get_slots,
        equipped: get_equipped,
        locked: is_locked,
        arrays: HashMap::from([
            (sword, (sword_slots.0, sword_slots.1, sword_parts.0, sword_parts.1)),
            (nameless, (sword_slots.0, sword_slots.1, 0, 0)),
            (axe, (axe_slots.0, axe_slots.1, 0, 0)),
        ]),
        locked_types: vec![8],
    };

    let out = temp_dir("weapon-slots");
    let engine = world.engine(&fake);
    let status = status(&out);
    assert_eq!(weapons::export(&ctx(&engine, &out, &status)), 2);
    assert_eq!(
        read(&out.join(weapons::SLOTS_FILE)),
        crlf(&[
            STARTER_LINE,
            r#"{"itemId":"WP_TEST_AXE","instanceId":"","weaponType":2,"slots":[{"slotIndex":0,"weaponPartType":32,"weaponPartTypeName":"soul","defaultWeaponPartId":"PART_TEST_SOUL","catalogLocked":false,"hideFromUI":false,"effectiveLocked":false},{"slotIndex":1,"weaponPartType":99,"weaponPartTypeName":"unknown","defaultWeaponPartId":"None","catalogLocked":false,"hideFromUI":false,"effectiveLocked":false}],"equippedParts":[]}"#,
        ])
    );
    let log = status_text(&out);
    assert!(
        log.contains("weapon-slots: wrote 2 live weapons -> weapon_slots_1_14_7.jsonl\r\n"),
        "{log}"
    );
    // The other three are always opened, empty here.
    for file in [weapons::PARTS_FILE, weapons::CATALOG_ROWS_FILE, weapons::LOADOUT_FILE] {
        assert_eq!(read(&out.join(file)), "", "{file}");
    }
    assert!(
        log.contains("weapon-parts: UArchonInventoryItem_WeaponPart class unavailable"),
        "{log}"
    );
    assert!(log.contains("weapon-part-catalog: wrote 0 rows -> weapon_part_catalog_rows_1_14_7.jsonl"));
    assert!(log.contains("loadout-gear-ui: wrote 0 rows -> loadout_gear_ui_1_14_7.jsonl"));
}

#[test]
fn weapon_slots_without_the_class_write_an_empty_file() {
    let world = World::new();
    let out = temp_dir("weapon-slots-none");
    std::fs::write(out.join(weapons::SLOTS_FILE), "stale\r\n").unwrap();
    let engine = world.engine(&NoCalls);
    let status = status(&out);
    assert_eq!(weapons::export(&ctx(&engine, &out, &status)), 0);
    assert_eq!(read(&out.join(weapons::SLOTS_FILE)), "");
    assert!(status_text(&out).contains("weapon-slots: UArchonInventoryItem_Weapon class unavailable\r\n"));
}

#[test]
fn live_parts_and_catalog_rows_read_the_part_catalog() {
    let mut world = World::new();
    world.class("ArchonInventoryItem", Some("Object"));
    world.class("ArchonInventoryItem_WeaponPart", Some("ArchonInventoryItem"));
    let (table, rows) = world.table_with_holes(
        "DT_TestPartCatalog",
        "PlayFabWeaponPartCatalogTableData",
        &[("PART_B", 0x10), ("PART_A", 0x10)],
        &[0],
    );
    world.set_int(rows[0], &sdk::PC_TYPE, 1);
    world.set_int(rows[1], &sdk::PC_TYPE, 32);
    world.set_flag(rows[1], &sdk::PC_UNASSIGNABLE);
    let found = world.object("Part_A", "ArchonInventoryItem_WeaponPart", 0x60);
    world.set_text(found, &sdk::WP_ITEM_ID, "PART_A");
    world.set_text(found, &sdk::WP_INSTANCE_ID, "inst-a");
    world.set_int(found, &sdk::WP_OWNER_TYPE, 3);
    world.mem.write_u64(found + u64::from(sdk::WP_CATALOG.offset), table);
    let assignable = world.object("Part_B", "ArchonInventoryItem_WeaponPart", 0x60);
    world.set_text(assignable, &sdk::WP_ITEM_ID, "PART_B");
    world
        .mem
        .write_u64(assignable + u64::from(sdk::WP_CATALOG.offset), table);
    let orphan = world.object("Part_C", "ArchonInventoryItem_WeaponPart", 0x60);
    world.set_text(orphan, &sdk::WP_ITEM_ID, "PART_C");

    let out = temp_dir("weapon-parts");
    let engine = world.engine(&NoCalls);
    let status = status(&out);
    weapons::export(&ctx(&engine, &out, &status));
    assert_eq!(
        read(&out.join(weapons::PARTS_FILE)),
        crlf(&[
            r#"{"itemId":"PART_A","instanceId":"inst-a","ownerWeaponType":3,"objectClass":"ArchonInventoryItem_WeaponPart","catalogName":"DT_TestPartCatalog","catalogRowStruct":"PlayFabWeaponPartCatalogTableData","catalogRowFound":true,"catalogWeaponPartType":32,"catalogIsUnassignable":1,"clientIsAssignable":false}"#,
            r#"{"itemId":"PART_B","instanceId":"","ownerWeaponType":0,"objectClass":"ArchonInventoryItem_WeaponPart","catalogName":"DT_TestPartCatalog","catalogRowStruct":"PlayFabWeaponPartCatalogTableData","catalogRowFound":true,"catalogWeaponPartType":1,"catalogIsUnassignable":0,"clientIsAssignable":true}"#,
            r#"{"itemId":"PART_C","instanceId":"","ownerWeaponType":0,"objectClass":"ArchonInventoryItem_WeaponPart","catalogName":"","catalogRowStruct":"","catalogRowFound":false,"catalogWeaponPartType":-1,"catalogIsUnassignable":-1,"clientIsAssignable":false}"#,
        ])
    );
    // Map order, not sorted (spec §6.3).
    assert_eq!(
        read(&out.join(weapons::CATALOG_ROWS_FILE)),
        crlf(&[
            r#"{"tableName":"DT_TestPartCatalog","rowName":"PART_B","weaponPartType":1,"weaponPartTypeName":"dp_barrel","isUnassignable":false,"clientIsAssignable":true}"#,
            r#"{"tableName":"DT_TestPartCatalog","rowName":"PART_A","weaponPartType":32,"weaponPartTypeName":"soul","isUnassignable":true,"clientIsAssignable":false}"#,
        ])
    );
    let log = status_text(&out);
    assert!(
        log.contains("weapon-parts: wrote 3 live parts -> live_weapon_parts_1_14_7.jsonl\r\n"),
        "{log}"
    );
    assert!(log.contains("weapon-part-catalog: wrote 2 rows -> weapon_part_catalog_rows_1_14_7.jsonl\r\n"));
}

#[test]
fn loadout_ui_writes_view_models_and_screens() {
    let mut world = World::new();
    world.class("LoadoutGearViewModel", Some("Object"));
    world.class("LoadoutGearScreen", Some("Object"));
    let object = world.class_ptr("Object");
    let item = world.raw_object("ItemVM", object, 0, 0x60);
    world.set_text(item, &sdk::LG_VM_ITEM_ID, "WP_TEST_SWORD");
    world.set_text(item, &sdk::LG_VM_INSTANCE_ID, "inst-1");
    world.set_int(item, &sdk::LG_VM_WEAPON_TYPE, 1);
    world.set_text(item, &sdk::LG_VM_NODE, "Node_1");
    world.set_flag(item, &sdk::LG_VM_NODE_UNLOCKED);
    let slot = world.raw_object("SlotVM", object, 0, 0x40);
    world.set_flag(slot, &sdk::LG_SLOT_EQUIPPED);
    world.set_int(slot, &sdk::LG_SLOT_RARITY, 3);
    let candidate = world.raw_object("CandidateVM", object, 0, 0x60);
    world.set_text(candidate, &sdk::LG_CAND_ITEM_ID, "PART_TEST");
    world.set_int(candidate, &sdk::LG_CAND_PART_TYPE, 7);
    world.set_flag(candidate, &sdk::LG_CAND_EQUIPPED);
    world.set_flag(candidate, &sdk::LG_CAND_CAN_CRAFT);

    world.default_object("Default__LoadoutGearViewModel", "LoadoutGearViewModel", 0x60);
    let gear = world.object("GearVM_0", "LoadoutGearViewModel", 0x60);
    world.mem.write_u64(gear + u64::from(sdk::LG_ITEM_VM.offset), item);
    let slots = world.set_items(gear, &sdk::LG_SLOTS, 1);
    world.mem.write_u64(slots[0], slot);
    let candidates = world.set_items(gear, &sdk::LG_CANDIDATES, 1);
    world.mem.write_u64(candidates[0], candidate);
    let screen = world.object("GearScreen_0", "LoadoutGearScreen", 0x60);
    world.mem.write_u64(screen + u64::from(sdk::LG_SCREEN_VM.offset), gear);
    world.set_flag(screen, &sdk::LG_SCREEN_ASSIGN_OPEN);

    let out = temp_dir("loadout-ui");
    let engine = world.engine(&NoCalls);
    let status = status(&out);
    weapons::export(&ctx(&engine, &out, &status));
    assert_eq!(
        read(&out.join(weapons::LOADOUT_FILE)),
        crlf(&[
            r#"{"kind":"gearViewModel","objectName":"GearVM_0","objectClass":"LoadoutGearViewModel","itemId":"WP_TEST_SWORD","itemInstanceId":"inst-1","weaponType":1,"itemWeaponPartType":0,"requiredSlayerNodeId":"Node_1","isRequiredNodeUnlocked":true,"partSlotCount":1,"partCandidateCount":1,"partSlots":[{"arrayIndex":0,"isLocked":false,"hasPartEquipped":true,"rarity":3}],"partCandidates":[{"arrayIndex":0,"itemId":"PART_TEST","itemInstanceId":"","weaponType":0,"weaponPartType":7,"weaponPartTypeName":"eb_special","isEquipped":true,"hasBeenCrafted":false,"canBeCrafted":true,"requiredSlayerNodeId":"None","isRequiredNodeUnlocked":false}]}"#,
            r#"{"kind":"screen","objectName":"GearScreen_0","objectClass":"LoadoutGearScreen","hasGearViewModel":true,"hasPartSlotBox":false,"isAssignPanelOpen":true}"#,
        ])
    );
    assert!(status_text(&out).contains("loadout-gear-ui: wrote 2 rows -> loadout_gear_ui_1_14_7.jsonl\r\n"));
}

#[test]
fn the_part_type_names_follow_the_enumerator_names() {
    // The synthetic enum puts the lantern parts before Soul; the text follows the name, not the position.
    assert_eq!(weapons::part_type_name(0), "default");
    assert_eq!(weapons::part_type_name(24), "ac_legendary");
    assert_eq!(weapons::part_type_name(30), "lantern_capacitor");
    assert_eq!(weapons::part_type_name(31), "lantern_cell");
    assert_eq!(weapons::part_type_name(32), "soul");
    assert_eq!(weapons::part_type_name(33), "tracker");
    assert_eq!(weapons::part_type_name(34), "unknown");
    assert!(sdk::WEAPON_SLOTS_MISSING.is_empty());
}
