//! `catalog_1_14_7.jsonl` and `equipment_1_14_7.jsonl` (spec §6.1, §6.2).

use super::{Ctx, Row, TextFile};
use crate::engine::{Engine, short};
use crate::json::Json;
use crate::sdk;
use crate::sdk_parse::Member;

pub const CATALOG_FILE: &str = "catalog_1_14_7.jsonl";
pub const EQUIPMENT_FILE: &str = "equipment_1_14_7.jsonl";

/// The equipment item id, a name read at fixed offsets of the object (spec §4.5).
const EQUIPMENT_ID_INDEX: u64 = 0x28;
const EQUIPMENT_ID_NUMBER: u64 = 0x2C;

const EQUIPMENT: [(&str, &str); 5] = [
    ("EquipmentCatalogItem_Weapon", "weapon"),
    ("EquipmentCatalogItem_Armour", "armour"),
    ("EquipmentCatalogItem_Lantern", "lantern"),
    ("EquipmentCatalogItem_PlayerRole", "player_role"),
    ("EquipmentCatalogItem_WeaponPart", "weapon_part"),
];

/// The live catalog: the first non-default `ArchonCatalog` in object order (spec §4.2).
pub fn live_catalog(e: &Engine) -> u64 {
    let class = e.find_class("ArchonCatalog");
    if class == 0 {
        return 0;
    }
    let found = e
        .objects()
        .into_iter()
        .map(|(_, object)| object)
        .find(|&object| e.is_a(object, class) && !e.is_default(object))
        .unwrap_or(0);
    e.take_fault();
    found
}

/// `UArchonCatalog::GetAllItems` through `ProcessEvent`: the returned array's data and count.
pub fn get_all_items(e: &Engine, catalog: u64) -> (u64, i32) {
    let function = e.find_function("ArchonCatalog", "GetAllItems");
    let words = usize::try_from(sdk::GET_ALL_ITEMS_PARAMS)
        .unwrap_or(0)
        .div_ceil(8)
        .max(2);
    let mut params = vec![0u64; words];
    let at = usize::try_from(sdk::GET_ALL_ITEMS_OUT.offset).unwrap_or(usize::MAX);
    if at % 8 != 0 || !e.process_event(catalog, function, &mut params) {
        return (0, 0);
    }
    let (Some(&data), Some(&num)) = (params.get(at / 8), params.get(at / 8 + 1)) else {
        return (0, 0);
    };
    // The count is the low half of the word after the data pointer (`ArrayNum`, then `ArrayMax`).
    let num = (num & 0xFFFF_FFFF) as u32 as i32;
    if data == 0 { (0, 0) } else { (data, num) }
}

/// Returns the definitions written.
pub fn catalog(ctx: &Ctx) -> i64 {
    let e = ctx.engine;
    let catalog = live_catalog(e);
    if catalog == 0 {
        ctx.status.line("", "general: UArchonCatalog instance NOT found yet");
        return 0;
    }
    let (data, count) = get_all_items(e, catalog);
    ctx.status
        .line("", &format!("general: GetAllItems returned {count} items"));
    if count <= 0 {
        return 0;
    }
    let Some(mut file) = TextFile::create(&ctx.out.join(CATALOG_FILE)) else {
        ctx.status.line("", &format!("general: cannot open {CATALOG_FILE}"));
        return 0;
    };
    let stride = u64::from(sdk::GET_ALL_ITEMS_OUT.stride);
    let mut written = 0i64;
    for index in 0..u64::try_from(count).unwrap_or(0).min(4_000_000) {
        let item = data.wrapping_add(index * stride);
        e.take_fault();
        let id = e.text(item, &sdk::CAT_ITEM_ID);
        if id.is_empty() {
            continue;
        }
        let mut j = Json::new();
        item_line(&mut j, e, item);
        if e.take_fault() {
            ctx.status.line("", &format!("general: item {id} unreadable, skipped"));
            continue;
        }
        file.line(&j.into_bytes());
        written += 1;
    }
    file.finish();
    ctx.status
        .line("", &format!("general: wrote {written} definitions -> {CATALOG_FILE}"));
    written
}

fn quantities(row: &mut Row, key: &str, array: &Member) {
    let elements = row.e.items(row.at, array);
    row.j.key(key).arr();
    for element in elements {
        let item = row.e.text(element, &sdk::CAT_QUANTITY_ITEM);
        let quantity = row.e.number(element, &sdk::CAT_QUANTITY_AMOUNT);
        row.j
            .obj()
            .kstr("itemId", &item)
            .key("quantity")
            .num(quantity)
            .end_obj();
    }
    row.j.end_arr();
}

fn item_line(j: &mut Json, e: &Engine, item: u64) {
    j.obj();
    let mut row = Row { j, e, at: item };
    row.s("itemId", &sdk::CAT_ITEM_ID)
        .s("itemClass", &sdk::CAT_ITEM_CLASS)
        .s("displayName", &sdk::CAT_DISPLAY_NAME)
        .s("description", &sdk::CAT_DESCRIPTION)
        .list("tags", &sdk::CAT_TAGS)
        .s("customData", &sdk::CAT_CUSTOM_DATA)
        .list("entitlements", &sdk::CAT_ENTITLEMENTS)
        .b("isStackable", &sdk::CAT_IS_STACKABLE)
        .b("isBundle", &sdk::CAT_IS_BUNDLE)
        .b("isContainer", &sdk::CAT_IS_CONTAINER)
        .n("maxQuantity", &sdk::CAT_MAX_QUANTITY)
        .list("containerItemContents", &sdk::CAT_CONTAINER_CONTENTS)
        .list("bundledItems", &sdk::CAT_BUNDLED_ITEMS);
    quantities(
        &mut row,
        "containerItemContentsWithQuantity",
        &sdk::CAT_CONTAINER_CONTENTS_Q,
    );
    row.list("containerResultTableContents", &sdk::CAT_CONTAINER_TABLES);
    quantities(
        &mut row,
        "containerResultTableContentsWithQuantity",
        &sdk::CAT_CONTAINER_TABLES_Q,
    );
    quantities(&mut row, "bundledItemsWithQuantity", &sdk::CAT_BUNDLED_ITEMS_Q);
    row.list("bundledResultTables", &sdk::CAT_BUNDLED_TABLES);
    quantities(&mut row, "bundledResultTablesWithQuantity", &sdk::CAT_BUNDLED_TABLES_Q);
    quantities(&mut row, "otherItemQuantities", &sdk::CAT_OTHER_QUANTITIES);
    quantities(&mut row, "alternateItemsToGrant", &sdk::CAT_ALTERNATE_ITEMS);
    let prices = e.items(item, &sdk::CAT_PRICES);
    row.j.key("virtualCurrencyPrices").arr();
    for price in prices {
        let currency = e.number(price, &sdk::CAT_PRICE_CURRENCY);
        let amount = e.number(price, &sdk::CAT_PRICE_AMOUNT);
        row.j
            .obj()
            .key("currency")
            .num(currency)
            .key("amount")
            .num(amount)
            .end_obj();
    }
    row.j.end_arr();
    row.s("journeyUnlockRow", &sdk::CAT_JOURNEY_UNLOCK_ROW);
    j.end_obj();
}

/// Returns the definitions written.
pub fn equipment(ctx: &Ctx) -> i64 {
    let e = ctx.engine;
    let Some(mut file) = TextFile::create(&ctx.out.join(EQUIPMENT_FILE)) else {
        ctx.status.line("", &format!("equipment: cannot open {EQUIPMENT_FILE}"));
        return 0;
    };
    let mut written = 0i64;
    for (_, object) in e.objects() {
        let class = e.class_name_of(object);
        let Some((_, kind)) = EQUIPMENT.iter().find(|(name, _)| *name == class) else {
            continue;
        };
        if e.raw_name_of(object).starts_with("Default__") {
            continue;
        }
        let index = e.i32(object.wrapping_add(EQUIPMENT_ID_INDEX));
        let number = e.i32(object.wrapping_add(EQUIPMENT_ID_NUMBER));
        e.take_fault();
        if index <= 0 {
            continue;
        }
        let id = short(&e.name_text(index, number)).to_owned();
        if id.is_empty() {
            continue;
        }
        let mut j = Json::new();
        j.obj()
            .kstr("itemId", &id)
            .kstr("type", kind)
            .kstr("class", &class)
            .end_obj();
        file.line(&j.into_bytes());
        written += 1;
    }
    file.finish();
    ctx.status.line(
        "",
        &format!("equipment: wrote {written} definitions -> {EQUIPMENT_FILE}"),
    );
    written
}
