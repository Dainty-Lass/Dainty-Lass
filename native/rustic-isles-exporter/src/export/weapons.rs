//! Weapon slots (`EXPORT_WEAPON_SLOTS`, spec §6.3): the live weapons' part slots, the live parts, the part catalog
//! rows and the loadout screen's view models. Only the first reaches a reader: the 1.14.7 module takes
//! `starterKit.defaultWeaponParts` from its `defaultWeaponPartId`s (ruling V-4).

use super::{Ctx, ROW_LIMIT, Row, TextFile, class_name};
use crate::engine::Engine;
use crate::json::Json;
use crate::sdk;
use crate::sdk_parse::{Kind, Member};

pub const SLOTS_FILE: &str = "weapon_slots_1_14_7.jsonl";
pub const PARTS_FILE: &str = "live_weapon_parts_1_14_7.jsonl";
pub const CATALOG_ROWS_FILE: &str = "weapon_part_catalog_rows_1_14_7.jsonl";
pub const LOADOUT_FILE: &str = "loadout_gear_ui_1_14_7.jsonl";

const WEAPON_CLASS: &str = "ArchonInventoryItem_Weapon";
const PART_CLASS: &str = "ArchonInventoryItem_WeaponPart";
const PART_ROW_STRUCT: &str = "PlayFabWeaponPartCatalogTableData";
/// A weapon has a handful of slots; more is a corrupt array.
const MAX_SLOTS: i32 = 4096;

/// Writes the four files in the spec's order; returns the live weapons written (the manifest's
/// `liveWeaponSlotItems`), -1 when that file could not be written at all.
pub fn export(ctx: &Ctx) -> i64 {
    let weapons = weapon_slots(ctx);
    live_parts(ctx);
    catalog_rows(ctx);
    loadout_ui(ctx);
    weapons
}

/// `EWeaponPartType`'s text by enumerator name (spec §6.3).
pub fn part_type_name(value: i64) -> &'static str {
    sdk::WEAPON_PART_TYPES
        .iter()
        .find(|(known, _)| *known == value)
        .map_or("unknown", |(_, text)| text)
}

/// Opens `file` (always, spec §6.3) unless this build's SDK lacks a fact the file needs.
fn open(ctx: &Ctx, tag: &str, file: &str, missing: &[&str]) -> Option<TextFile> {
    let opened = TextFile::create(&ctx.out.join(file));
    if !missing.is_empty() {
        if let Some(opened) = opened {
            opened.finish();
        }
        ctx.status.line(
            "",
            &format!(
                "{tag}: the SDK this DLL was built on lacks {}, skipping {file}",
                missing.join(", ")
            ),
        );
        return None;
    }
    if opened.is_none() {
        ctx.status.line("", &format!("{tag}: cannot open {file}"));
    }
    opened
}

/// The live (non-default) objects that are a `class`, in object order; empty when the class is not loaded.
fn live(e: &Engine, class: &str) -> Option<Vec<u64>> {
    let class = e.find_class(class);
    if class == 0 {
        return None;
    }
    let objects = e
        .objects()
        .into_iter()
        .map(|(_, object)| object)
        .filter(|&object| e.is_a(object, class) && !e.is_default(object))
        .collect();
    e.take_fault();
    Some(objects)
}

/// Calls `function` on `object` through `ProcessEvent` with a parameter block of `size` bytes holding `args` at their
/// offsets; the block afterwards, or `None` when the call could not be made.
fn call(e: &Engine, object: u64, function: u64, size: u32, args: &[(u32, &[u8])]) -> Option<Vec<u8>> {
    let words = usize::try_from(size).unwrap_or(0).div_ceil(8).max(2);
    let mut bytes = vec![0u8; words * 8];
    for (offset, value) in args {
        let at = usize::try_from(*offset).unwrap_or(usize::MAX);
        if let Some(slot) = at.checked_add(value.len()).and_then(|end| bytes.get_mut(at..end)) {
            slot.copy_from_slice(value);
        }
    }
    let mut params: Vec<u64> = bytes
        .as_chunks::<8>()
        .0
        .iter()
        .map(|word| u64::from_le_bytes(*word))
        .collect();
    if !e.process_event(object, function, &mut params) {
        return None;
    }
    Some(params.iter().flat_map(|word| word.to_le_bytes()).collect())
}

/// The elements of the `TArray` a call returned in its block.
fn returned(block: &[u8], array: &Member) -> Vec<u64> {
    let at = usize::try_from(array.offset).unwrap_or(usize::MAX);
    let word = |from: usize, len: usize| -> Option<&[u8]> { block.get(from..from.checked_add(len)?) };
    let (Some(data), Some(num)) = (word(at, 8), at.checked_add(8).and_then(|next| word(next, 4))) else {
        return Vec::new();
    };
    let data = u64::from_le_bytes(data.try_into().unwrap_or_default());
    let num = i32::from_le_bytes(num.try_into().unwrap_or_default());
    if data == 0 || !(0..=MAX_SLOTS).contains(&num) {
        return Vec::new();
    }
    (0..u64::from(num.unsigned_abs()))
        .map(|index| data.wrapping_add(index * u64::from(array.stride)))
        .collect()
}

/// An array member's elements; for an array of object pointers, the objects.
fn entries(e: &Engine, base: u64, array: &Member) -> Vec<u64> {
    let items = e.items(base, array);
    if array.elem == Kind::ObjPtr {
        items.into_iter().map(|item| e.ptr(item)).collect()
    } else {
        items
    }
}

fn part_type(j: &mut Json, value: i64) {
    j.kint("weaponPartType", value)
        .kstr("weaponPartTypeName", part_type_name(value));
}

/// `weapon_slots_1_14_7.jsonl`.
fn weapon_slots(ctx: &Ctx) -> i64 {
    let e = ctx.engine;
    let Some(mut file) = open(ctx, "weapon-slots", SLOTS_FILE, sdk::WEAPON_SLOTS_MISSING) else {
        return -1;
    };
    let Some(weapons) = live(e, WEAPON_CLASS) else {
        file.finish();
        ctx.status
            .line("", "weapon-slots: UArchonInventoryItem_Weapon class unavailable");
        return 0;
    };
    let get_slots = e.find_function(WEAPON_CLASS, "GetItemPartSlots");
    let get_equipped = e.find_function(WEAPON_CLASS, "GetEquippedItemParts");
    let is_locked = e.find_function(WEAPON_CLASS, "IsItemPartSlotLocked");
    let mut written = 0i64;
    for weapon in weapons {
        e.take_fault();
        let id = e.text(weapon, &sdk::WS_ITEM_ID);
        if id.is_empty() {
            continue;
        }
        let mut j = Json::new();
        j.obj();
        Row {
            j: &mut j,
            e,
            at: weapon,
        }
        .s("itemId", &sdk::WS_ITEM_ID)
        .s("instanceId", &sdk::WS_INSTANCE_ID)
        .n("weaponType", &sdk::WS_WEAPON_TYPE);
        let slots = call(e, weapon, get_slots, sdk::WS_SLOTS_PARAMS, &[])
            .map(|block| returned(&block, &sdk::WS_SLOTS_OUT))
            .unwrap_or_default();
        j.key("slots").arr();
        for (index, slot) in slots.into_iter().enumerate() {
            let kind = e.int(slot, &sdk::WS_SLOT_TYPE);
            let size = usize::try_from(sdk::WS_LOCKED_TYPE.size).unwrap_or(1).clamp(1, 8);
            let value = kind.to_le_bytes();
            let effective = call(
                e,
                weapon,
                is_locked,
                sdk::WS_LOCKED_PARAMS,
                &[(sdk::WS_LOCKED_TYPE.offset, &value[..size])],
            )
            .and_then(|block| {
                let at = usize::try_from(sdk::WS_LOCKED_RETURN.offset).ok()?;
                block.get(at).copied()
            })
            .is_some_and(|byte| byte != 0);
            j.obj().kint("slotIndex", index as i64);
            part_type(&mut j, kind);
            Row { j: &mut j, e, at: slot }
                .s("defaultWeaponPartId", &sdk::WS_SLOT_DEFAULT)
                .b("catalogLocked", &sdk::WS_SLOT_LOCKED)
                .b("hideFromUI", &sdk::WS_SLOT_HIDE);
            j.kbool("effectiveLocked", effective).end_obj();
        }
        j.end_arr();
        let equipped = call(e, weapon, get_equipped, sdk::WS_EQUIPPED_PARAMS, &[])
            .map(|block| returned(&block, &sdk::WS_EQUIPPED_OUT))
            .unwrap_or_default();
        j.key("equippedParts").arr();
        for (index, part) in equipped.into_iter().enumerate() {
            j.obj().kint("slotIndex", index as i64);
            part_type(&mut j, e.int(part, &sdk::WS_EQUIPPED_TYPE));
            Row { j: &mut j, e, at: part }.s("weaponPartId", &sdk::WS_EQUIPPED_ID);
            j.end_obj();
        }
        j.end_arr().end_obj();
        if e.take_fault() {
            ctx.status
                .line("", &format!("weapon-slots: weapon {id} unreadable, skipped"));
            continue;
        }
        file.line(&j.into_bytes());
        written += 1;
    }
    file.finish();
    ctx.status.line(
        "",
        &format!("weapon-slots: wrote {written} live weapons -> {SLOTS_FILE}"),
    );
    written
}

/// `live_weapon_parts_1_14_7.jsonl`.
fn live_parts(ctx: &Ctx) -> i64 {
    let e = ctx.engine;
    let Some(mut file) = open(ctx, "weapon-parts", PARTS_FILE, sdk::LIVE_PARTS_MISSING) else {
        return -1;
    };
    let Some(parts) = live(e, PART_CLASS) else {
        file.finish();
        ctx.status
            .line("", "weapon-parts: UArchonInventoryItem_WeaponPart class unavailable");
        return 0;
    };
    let mut written = 0i64;
    for part in parts {
        e.take_fault();
        let id = e.text(part, &sdk::WP_ITEM_ID);
        if id.is_empty() {
            continue;
        }
        let table = e.ptr(part.wrapping_add(u64::from(sdk::WP_CATALOG.offset)));
        let (table_name, row_struct, row) = if table == 0 {
            (String::new(), String::new(), None)
        } else {
            let (count, rows) = e.rows(table);
            let row = (0..ROW_LIMIT)
                .contains(&count)
                .then(|| rows.into_iter().find(|(name, data)| *name == id && *data != 0))
                .flatten()
                .map(|(_, data)| data);
            (e.name_of(table), e.row_struct_name(table).unwrap_or_default(), row)
        };
        let (kind, unassignable) = match row {
            Some(row) => (e.int(row, &sdk::PC_TYPE), i64::from(e.flag(row, &sdk::PC_UNASSIGNABLE))),
            None => (-1, -1),
        };
        let mut j = Json::new();
        j.obj();
        Row { j: &mut j, e, at: part }
            .s("itemId", &sdk::WP_ITEM_ID)
            .s("instanceId", &sdk::WP_INSTANCE_ID)
            .n("ownerWeaponType", &sdk::WP_OWNER_TYPE);
        j.kstr("objectClass", &class_name(e, part))
            .kstr("catalogName", &table_name)
            .kstr("catalogRowStruct", &row_struct)
            .kbool("catalogRowFound", row.is_some())
            .kint("catalogWeaponPartType", kind)
            .kint("catalogIsUnassignable", unassignable)
            .kbool("clientIsAssignable", unassignable == 0)
            .end_obj();
        if e.take_fault() {
            ctx.status
                .line("", &format!("weapon-parts: part {id} unreadable, skipped"));
            continue;
        }
        file.line(&j.into_bytes());
        written += 1;
    }
    file.finish();
    ctx.status
        .line("", &format!("weapon-parts: wrote {written} live parts -> {PARTS_FILE}"));
    written
}

/// `weapon_part_catalog_rows_1_14_7.jsonl`: rows in map order, not sorted.
fn catalog_rows(ctx: &Ctx) -> i64 {
    let e = ctx.engine;
    let Some(mut file) = open(ctx, "weapon-part-catalog", CATALOG_ROWS_FILE, sdk::PART_CATALOG_MISSING) else {
        return -1;
    };
    let mut written = 0i64;
    for table in e.tables_by_row_struct(PART_ROW_STRUCT) {
        let table_name = e.name_of(table);
        let (count, rows) = e.rows(table);
        if !(0..ROW_LIMIT).contains(&count) {
            ctx.status.line(
                "",
                &format!("weapon-part-catalog: {table_name}: row count {count} out of range, skipping"),
            );
            continue;
        }
        for (row_name, data) in rows {
            e.take_fault();
            if data == 0 {
                continue;
            }
            let kind = e.int(data, &sdk::PC_TYPE);
            let unassignable = e.flag(data, &sdk::PC_UNASSIGNABLE);
            if e.take_fault() {
                ctx.status.line(
                    "",
                    &format!("weapon-part-catalog: {table_name}/{row_name}: unreadable row skipped"),
                );
                continue;
            }
            let mut j = Json::new();
            j.obj().kstr("tableName", &table_name).kstr("rowName", &row_name);
            part_type(&mut j, kind);
            j.kbool("isUnassignable", unassignable)
                .kbool("clientIsAssignable", !unassignable)
                .end_obj();
            file.line(&j.into_bytes());
            written += 1;
        }
    }
    file.finish();
    ctx.status.line(
        "",
        &format!("weapon-part-catalog: wrote {written} rows -> {CATALOG_ROWS_FILE}"),
    );
    written
}

/// `loadout_gear_ui_1_14_7.jsonl`: a research capture, so a read that fails writes its default, never skips the line.
fn loadout_ui(ctx: &Ctx) -> i64 {
    let e = ctx.engine;
    let Some(mut file) = open(ctx, "loadout-gear-ui", LOADOUT_FILE, sdk::LOADOUT_UI_MISSING) else {
        return -1;
    };
    let view_model = e.find_class("LoadoutGearViewModel");
    let screen = e.find_class("LoadoutGearScreen");
    let mut written = 0i64;
    for (_, object) in e.objects() {
        if e.is_default(object) {
            continue;
        }
        if e.is_a(object, view_model) {
            file.line(&gear_view_model(e, object));
            written += 1;
        }
        if e.is_a(object, screen) {
            let mut j = Json::new();
            j.obj()
                .kstr("kind", "screen")
                .kstr("objectName", &e.name_of(object))
                .kstr("objectClass", &class_name(e, object));
            let gear = e.ptr(object.wrapping_add(u64::from(sdk::LG_SCREEN_VM.offset)));
            let slot_box = e.ptr(object.wrapping_add(u64::from(sdk::LG_SCREEN_SLOT_BOX.offset)));
            j.kbool("hasGearViewModel", gear != 0)
                .kbool("hasPartSlotBox", slot_box != 0);
            Row {
                j: &mut j,
                e,
                at: object,
            }
            .b("isAssignPanelOpen", &sdk::LG_SCREEN_ASSIGN_OPEN);
            j.end_obj();
            file.line(&j.into_bytes());
            written += 1;
        }
    }
    e.take_fault();
    file.finish();
    ctx.status
        .line("", &format!("loadout-gear-ui: wrote {written} rows -> {LOADOUT_FILE}"));
    written
}

fn gear_view_model(e: &Engine, object: u64) -> Vec<u8> {
    let mut j = Json::new();
    j.obj()
        .kstr("kind", "gearViewModel")
        .kstr("objectName", &e.name_of(object))
        .kstr("objectClass", &class_name(e, object));
    let item = e.ptr(object.wrapping_add(u64::from(sdk::LG_ITEM_VM.offset)));
    Row { j: &mut j, e, at: item }
        .s("itemId", &sdk::LG_VM_ITEM_ID)
        .s("itemInstanceId", &sdk::LG_VM_INSTANCE_ID)
        .n("weaponType", &sdk::LG_VM_WEAPON_TYPE)
        .n("itemWeaponPartType", &sdk::LG_VM_PART_TYPE)
        .s("requiredSlayerNodeId", &sdk::LG_VM_NODE)
        .b("isRequiredNodeUnlocked", &sdk::LG_VM_NODE_UNLOCKED);
    let slots = entries(e, object, &sdk::LG_SLOTS);
    let candidates = entries(e, object, &sdk::LG_CANDIDATES);
    j.kint("partSlotCount", slots.len() as i64)
        .kint("partCandidateCount", candidates.len() as i64);
    j.key("partSlots").arr();
    for (index, slot) in slots.into_iter().enumerate() {
        j.obj().kint("arrayIndex", index as i64);
        Row { j: &mut j, e, at: slot }
            .b("isLocked", &sdk::LG_SLOT_LOCKED)
            .b("hasPartEquipped", &sdk::LG_SLOT_EQUIPPED)
            .n("rarity", &sdk::LG_SLOT_RARITY);
        j.end_obj();
    }
    j.end_arr().key("partCandidates").arr();
    for (index, candidate) in candidates.into_iter().enumerate() {
        j.obj().kint("arrayIndex", index as i64);
        Row {
            j: &mut j,
            e,
            at: candidate,
        }
        .s("itemId", &sdk::LG_CAND_ITEM_ID)
        .s("itemInstanceId", &sdk::LG_CAND_INSTANCE_ID)
        .n("weaponType", &sdk::LG_CAND_WEAPON_TYPE);
        part_type(&mut j, e.int(candidate, &sdk::LG_CAND_PART_TYPE));
        Row {
            j: &mut j,
            e,
            at: candidate,
        }
        .b("isEquipped", &sdk::LG_CAND_EQUIPPED)
        .b("hasBeenCrafted", &sdk::LG_CAND_CRAFTED)
        .b("canBeCrafted", &sdk::LG_CAND_CAN_CRAFT)
        .s("requiredSlayerNodeId", &sdk::LG_CAND_NODE)
        .b("isRequiredNodeUnlocked", &sdk::LG_CAND_NODE_UNLOCKED);
        j.end_obj();
    }
    j.end_arr().end_obj();
    j.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Engine;
    use crate::memory::{NoCalls, Synthetic};
    use crate::status::{Status, no_debug};

    #[test]
    fn a_fact_the_sdk_lacks_empties_the_file_and_names_the_fact() {
        let out = crate::testdir::new("weapons-missing");
        let _ = std::fs::write(out.join(SLOTS_FILE), "stale\r\n");
        let mem = Synthetic::new();
        let engine = Engine::new(&mem, &NoCalls, 0, 0);
        let status = Status::new(&out, no_debug);
        let ctx = Ctx {
            engine: &engine,
            out: out.clone(),
            status: &status,
        };
        assert!(open(&ctx, "weapon-slots", SLOTS_FILE, &["WS_SLOT_DEFAULT", "WS_SLOT_HIDE"]).is_none());
        assert_eq!(std::fs::read(out.join(SLOTS_FILE)).unwrap_or_default(), b"");
        let log = std::fs::read_to_string(out.join(crate::status::FILE)).unwrap_or_default();
        assert_eq!(
            log,
            "weapon-slots: the SDK this DLL was built on lacks WS_SLOT_DEFAULT, WS_SLOT_HIDE, skipping \
             weapon_slots_1_14_7.jsonl\r\n"
        );
    }
}
