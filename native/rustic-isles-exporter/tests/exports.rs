//! Each phase-1 export run on a synthetic image, its files checked byte for byte (spec §5, §6).
#![allow(clippy::unwrap_used, reason = "test helpers fail the test")]

mod common;

use std::cell::Cell;

use catalog_exporter::export::{catalog, hunts, mastery, progression, slayers, store};
use catalog_exporter::memory::NoCalls;
use catalog_exporter::sdk;
use catalog_exporter::sdk_parse::Member;
use common::{FakeCatalog, World, ctx, read, status, status_text, temp_dir};

fn crlf(lines: &[&str]) -> String {
    lines.iter().map(|line| format!("{line}\r\n")).collect()
}

/// Writes the quantity list `(item, amount)` into an array member.
fn quantities(world: &mut World, item: u64, member: &Member, entries: &[(&str, i64)]) {
    let elements = world.set_items(item, member, entries.len());
    for (at, (id, amount)) in elements.into_iter().zip(entries) {
        world.set_text(at, &sdk::CAT_QUANTITY_ITEM, id);
        world.set_int(at, &sdk::CAT_QUANTITY_AMOUNT, *amount);
    }
}

/// A live catalog whose `GetAllItems` returns `count` zeroed items of the SDK's size.
fn catalog_world(count: usize) -> (World, FakeCatalog, Vec<u64>) {
    let mut world = World::new();
    world.class("ArchonCatalog", Some("Object"));
    world.default_object("Default__ArchonCatalog", "ArchonCatalog", 0x80);
    let live = world.object("ArchonCatalog_0", "ArchonCatalog", 0x80);
    let function_class = world.class_ptr("Function");
    let owner = world.class_ptr("ArchonCatalog");
    let function = world.raw_object("GetAllItems", function_class, owner, 0x80);
    world.register(function);
    let stride = sdk::GET_ALL_ITEMS_OUT.stride;
    let data = world.mem.alloc(count * stride as usize);
    let items = (0..count as u64).map(|i| data + i * u64::from(stride)).collect();
    let fake = FakeCatalog {
        catalog: live,
        function,
        data,
        count: i32::try_from(count).unwrap(),
        calls: Cell::new(0),
    };
    (world, fake, items)
}

#[test]
fn catalog_writes_every_item_with_an_id() {
    let (mut world, fake, items) = catalog_world(3);
    let item = items[0];
    world.set_text(item, &sdk::CAT_ITEM_ID, "WP_TEST_AXE");
    world.set_text(item, &sdk::CAT_ITEM_CLASS, "/Game/Items/WP_TEST_AXE.WP_TEST_AXE_C");
    world.set_text(item, &sdk::CAT_DISPLAY_NAME, "Test \"Axe\"");
    world.set_text(item, &sdk::CAT_DESCRIPTION, "Line1\nLine2\t\u{e9}\u{1}");
    world.set_texts(item, &sdk::CAT_TAGS, &["weapon", "axe"]);
    world.set_text(item, &sdk::CAT_CUSTOM_DATA, r#"{"ItemData":{"RarityLevel":3}}"#);
    world.set_flag(item, &sdk::CAT_IS_STACKABLE);
    world.set_flag(item, &sdk::CAT_IS_CONTAINER);
    world.set_int(item, &sdk::CAT_MAX_QUANTITY, 1);
    world.set_texts(item, &sdk::CAT_CONTAINER_CONTENTS, &["A"]);
    quantities(&mut world, item, &sdk::CAT_CONTAINER_CONTENTS_Q, &[("Q1", 2)]);
    quantities(&mut world, item, &sdk::CAT_ALTERNATE_ITEMS, &[("ALT", -1)]);
    let prices = world.set_items(item, &sdk::CAT_PRICES, 1);
    world.set_int(prices[0], &sdk::CAT_PRICE_CURRENCY, 2);
    world.set_int(prices[0], &sdk::CAT_PRICE_AMOUNT, 700);
    world.set_text(item, &sdk::CAT_JOURNEY_UNLOCK_ROW, "Node_1");
    // Item 1 has no id and is skipped; item 2 has nothing else.
    world.set_text(items[2], &sdk::CAT_ITEM_ID, "CURRENCY_TEST");

    let out = temp_dir("catalog");
    let engine = world.engine(&fake);
    let status = status(&out);
    let ctx = ctx(&engine, &out, &status);
    assert_eq!(catalog::catalog(&ctx), 2);
    assert_eq!(fake.calls.get(), 1);
    let expected = crlf(&[
        r#"{"itemId":"WP_TEST_AXE","itemClass":"WP_TEST_AXE.WP_TEST_AXE_C","displayName":"Test \"Axe\"","description":"Line1\nLine2\té\u0001","tags":["weapon","axe"],"customData":"{\"ItemData\":{\"RarityLevel\":3}}","entitlements":[],"isStackable":true,"isBundle":false,"isContainer":true,"maxQuantity":1,"containerItemContents":["A"],"bundledItems":[],"containerItemContentsWithQuantity":[{"itemId":"Q1","quantity":2}],"containerResultTableContents":[],"containerResultTableContentsWithQuantity":[],"bundledItemsWithQuantity":[],"bundledResultTables":[],"bundledResultTablesWithQuantity":[],"otherItemQuantities":[],"alternateItemsToGrant":[{"itemId":"ALT","quantity":-1}],"virtualCurrencyPrices":[{"currency":2,"amount":700}],"journeyUnlockRow":"Node_1"}"#,
        r#"{"itemId":"CURRENCY_TEST","itemClass":"None","displayName":"","description":"","tags":[],"customData":"","entitlements":[],"isStackable":false,"isBundle":false,"isContainer":false,"maxQuantity":0,"containerItemContents":[],"bundledItems":[],"containerItemContentsWithQuantity":[],"containerResultTableContents":[],"containerResultTableContentsWithQuantity":[],"bundledItemsWithQuantity":[],"bundledResultTables":[],"bundledResultTablesWithQuantity":[],"otherItemQuantities":[],"alternateItemsToGrant":[],"virtualCurrencyPrices":[],"journeyUnlockRow":"None"}"#,
    ])
    // The é is written as its UTF-8 bytes, not escaped.
    .replace("\\u00e9", "\u{e9}");
    assert_eq!(read(&out.join(catalog::CATALOG_FILE)), expected);
    let log = status_text(&out);
    assert!(log.contains("general: GetAllItems returned 3 items\r\n"), "{log}");
    assert!(
        log.contains("general: wrote 2 definitions -> catalog_1_14_7.jsonl\r\n"),
        "{log}"
    );
}

#[test]
fn catalog_without_items_opens_no_file() {
    let (world, fake, _) = catalog_world(0);
    let out = temp_dir("catalog-empty");
    let engine = world.engine(&fake);
    let status = status(&out);
    assert_eq!(catalog::catalog(&ctx(&engine, &out, &status)), 0);
    assert!(!out.join(catalog::CATALOG_FILE).exists());

    let none = World::new();
    let engine = none.engine(&NoCalls);
    assert_eq!(catalog::catalog(&ctx(&engine, &out, &status)), 0);
    assert!(status_text(&out).contains("general: UArchonCatalog instance NOT found yet"));
}

#[test]
fn equipment_reads_the_id_at_its_fixed_offset() {
    let mut world = World::new();
    world.class("EquipmentCatalogItem_Weapon", Some("Object"));
    world.class("EquipmentCatalogItem_Lantern", Some("Object"));
    world.class("SomethingElse", Some("Object"));
    let skipped = world.object(
        "Default__EquipmentCatalogItem_Weapon",
        "EquipmentCatalogItem_Weapon",
        0x40,
    );
    world.write_name(skipped + 0x28, "WP_DEFAULT", 0);
    let weapon = world.object("WP_A", "EquipmentCatalogItem_Weapon", 0x40);
    world.write_name(weapon + 0x28, "WP_TEST_AXE", 0);
    world.register_null();
    let lantern = world.object("LT_A", "EquipmentCatalogItem_Lantern", 0x40);
    world.write_name(lantern + 0x28, "Lanterns/LT_TEST", 2);
    world.object("WP_B", "EquipmentCatalogItem_Weapon", 0x40);
    let other = world.object("X", "SomethingElse", 0x40);
    world.write_name(other + 0x28, "NOT_EQUIPMENT", 0);

    let out = temp_dir("equipment");
    let engine = world.engine(&NoCalls);
    let status = status(&out);
    assert_eq!(catalog::equipment(&ctx(&engine, &out, &status)), 2);
    assert_eq!(
        read(&out.join(catalog::EQUIPMENT_FILE)),
        crlf(&[
            r#"{"itemId":"WP_TEST_AXE","type":"weapon","class":"EquipmentCatalogItem_Weapon"}"#,
            r#"{"itemId":"LT_TEST_1","type":"lantern","class":"EquipmentCatalogItem_Lantern"}"#,
        ])
    );
}

#[test]
fn slayers_path_sorts_rows_skips_free_slots_and_faulting_rows() {
    let mut world = World::new();
    let size = 0x200;
    let (table, rows) = world.table_with_holes(
        "PlayerJourneyNodes",
        "PlayerJourneyNodeData",
        &[("Node_B", size), ("Node_A", size), ("Node_Bad", size)],
        &[1],
    );
    let node = rows[1];
    world.set_int(node, &sdk::SP_NODE_TYPE, 2);
    world.set_int(node, &sdk::SP_NODE_LEVEL, 3);
    world.set_texts(node, &sdk::SP_CHILD_NODES, &["Node_B"]);
    world.set_flag(node, &sdk::SP_AUTO_UNLOCK);
    let costs = world.set_items(node, &sdk::SP_CURRENCY_COSTS, 1);
    world.set_text(costs[0], &sdk::SP_CURRENCY_COST_CURRENCY, "CURRENCY_PJM");
    world.set_int(costs[0], &sdk::SP_CURRENCY_COST_AMOUNT, 50);
    let costs = world.set_items(node, &sdk::SP_EXPERIENCE_COSTS, 1);
    world.set_int(costs[0], &sdk::SP_EXPERIENCE_COST_TYPE, 1);
    world.set_int(costs[0], &sdk::SP_EXPERIENCE_COST_AMOUNT, 100);
    let rewards = world.set_items(node, &sdk::SP_REWARDS, 1);
    world.set_text(rewards[0], &sdk::SP_REWARD_ITEM_ID, "WP_X");
    world.set_int(rewards[0], &sdk::SP_REWARD_AMOUNT, 1);
    world.set_flag(rewards[0], &sdk::SP_REWARD_AUTO_EQUIP);
    world.set_items(node, &sdk::SP_SYSTEM_REWARDS, 2);
    world.set_text(node, &sdk::SP_ACTIVATION_EVENT_ID, "Evt");
    world.set_texts(node, &sdk::SP_QUEST_IDS, &["Q1", "Q2"]);
    world.set_text(node, &sdk::SP_TUTORIAL, "Tut");
    let map = sdk::SP_ATTRIBUTE_VALUES;
    let entries = world.write_sparse(node + u64::from(map.offset), 3, map.stride, &[1]);
    for (at, (name, value)) in [entries[0], entries[2]]
        .into_iter()
        .zip([("Health", 1.5f32), ("Stamina", -0.25)])
    {
        world.write_fstring(at + u64::from(map.key_offset), name);
        world.mem.write_f32(at + u64::from(map.value_offset), value);
    }
    // Node_Bad's row data points at unmapped memory.
    let elements = sdk::DATATABLE_ROW_MAP;
    let (data, _) = {
        let engine = world.engine(&NoCalls);
        engine.array(table + u64::from(elements.offset))
    };
    world.mem.write_u64(
        data + 3 * u64::from(elements.stride) + u64::from(elements.value_offset),
        0xDEAD_0000,
    );

    let out = temp_dir("slayers");
    let engine = world.engine(&NoCalls);
    let status = status(&out);
    assert_eq!(slayers::export(&ctx(&engine, &out, &status)), 2);
    assert_eq!(
        read(&out.join(slayers::FOLDER).join(slayers::FILE)),
        crlf(&[
            r#"{"nodeId":"Node_A","table":"PlayerJourneyNodes","nodeType":2,"nodeLevel":3,"childNodes":["Node_B"],"autoUnlockIfParentUnlocked":true,"currencyCosts":[{"currency":"CURRENCY_PJM","amount":50}],"experienceCosts":[{"experienceType":1,"amount":100}],"rewards":[{"itemId":"WP_X","amount":1,"entitlementId":"None","duration":0,"passPrefix":"","experienceType":0,"autoEquip":true}],"systemRewardCount":2,"activationEventId":"Evt","questIds":["Q1","Q2"],"questIdToNotifyOnUnlock":"None","tutorialToShowWhenUnlocked":"Tut","objectiveCount":0,"gameplayAttributeValues":{"Health":1.500000,"Stamina":-0.250000}}"#,
            r#"{"nodeId":"Node_B","table":"PlayerJourneyNodes","nodeType":0,"nodeLevel":0,"childNodes":[],"autoUnlockIfParentUnlocked":false,"currencyCosts":[],"experienceCosts":[],"rewards":[],"systemRewardCount":0,"activationEventId":"None","questIds":[],"questIdToNotifyOnUnlock":"None","tutorialToShowWhenUnlocked":"None","objectiveCount":0,"gameplayAttributeValues":{}}"#,
        ])
    );
    let log = status_text(&out);
    assert!(
        log.contains("[SlayersPath] PlayerJourneyNodes/Node_Bad: unreadable row skipped\r\n"),
        "{log}"
    );
    assert!(
        log.contains("[SlayersPath] wrote 2/3 PlayerJourneyNodeData rows"),
        "{log}"
    );
}

#[test]
fn slayers_path_without_a_table_opens_no_file() {
    let world = World::new();
    let out = temp_dir("slayers-none");
    let engine = world.engine(&NoCalls);
    let status = status(&out);
    assert_eq!(slayers::export(&ctx(&engine, &out, &status)), 0);
    assert!(!out.join(slayers::FOLDER).join(slayers::FILE).exists());
    assert!(status_text(&out).contains(
        "[SlayersPath] no tables found with RowStruct=PlayerJourneyNodeData (is the Slayer's Path screen loaded? \
         open it once, then re-run)"
    ));
}

#[test]
fn hunts_write_their_tables_and_manifest() {
    let mut world = World::new();
    let (_, rows) = world.table("PlayerHunts", "PlayerHuntTableData", &[("Hunt_1", 0x400)]);
    let hunt = rows[0];
    world.set_text(hunt, &sdk::PH_ID, "PH_1");
    let region_table = world.raw_object("RegionTable", 0, 0, 0x40);
    world.mem.write_u64(
        hunt + u64::from(sdk::PH_REGION.offset + sdk::ROW_HANDLE_TABLE.offset),
        region_table,
    );
    world.set_text(hunt, &sdk::PH_REGION, "Region_A");
    world.set_int(hunt, &sdk::PH_MATCHMAKING_TYPE, 1);
    world.set_texts(
        hunt + u64::from(sdk::PH_HUNT_TAGS.offset),
        &sdk::TAG_CONTAINER_TAGS,
        &["Hunt.Patrol", "Hunt.Escalation"],
    );
    let lists = world.set_items(hunt, &sdk::PH_BY_TAG, 1);
    world.set_text(lists[0], &sdk::PH_BY_TAG_TABLE, "MMTable");
    let queries = world.set_items(lists[0], &sdk::PH_BY_TAG_QUERIES, 1);
    world.set_texts(queries[0], &sdk::TAG_QUERY_DICTIONARY, &["Hunt.Patrol"]);
    let stream = world.set_items(queries[0], &sdk::TAG_QUERY_STREAM, 4);
    for (at, byte) in stream.into_iter().zip([0u8, 1, 2, 255]) {
        world.mem.write_u8(at, byte);
    }
    world.set_text(queries[0], &sdk::TAG_QUERY_AUTO_DESCRIPTION, "auto");
    world.set_flag(hunt, &sdk::PH_HAS_PORTALS);
    world.set_text(hunt, &sdk::PH_SUCCESS_REWARD, "R1");
    world.set_int(hunt, &sdk::PH_MIN_SKILL, 5);
    world.set_int(hunt, &sdk::PH_RECOMMENDED_SKILL, 10);

    let shrike = world.object("Shrike", "Object", 0x40);
    let (_, rows) = world.table("MatchmakerHunts", "MatchmakerHuntTableData", &[("MM_1", 0x400)]);
    let mm = rows[0];
    world.set_int(mm, &sdk::MH_THREAT, 3);
    world.set_float(mm, &sdk::MH_DANGER, 0.5);
    world.set_text(mm, &sdk::MH_BEHEMOTH_NAME, "Shrike");
    world.set_text(mm, &sdk::MH_BEHEMOTH_ASSET, "/Game/Behemoths/Shrike.Shrike");
    let asset = mm + u64::from(sdk::MH_BEHEMOTH_ASSET.offset);
    let shrike_index = {
        let engine = world.engine(&NoCalls);
        engine
            .objects()
            .into_iter()
            .find(|(_, object)| *object == shrike)
            .unwrap()
            .0
    };
    world.mem.write_i32(asset, shrike_index);
    world.mem.write_i32(asset + 4, world.serial(shrike_index));
    world.set_float(mm, &sdk::MH_BEHEMOTH_WEIGHT, 1.0);
    let extra = world.set_items(mm, &sdk::MH_EXTRA_BEHEMOTHS, 1);
    let inner = |member: Member| Member {
        offset: member.offset - sdk::MH_BEHEMOTH.offset,
        ..member
    };
    world.set_text(extra[0], &inner(sdk::MH_BEHEMOTH_NAME), "Skraev");
    world.set_int(extra[0], &inner(sdk::MH_BEHEMOTH_POWER), 2);
    world.set_float(extra[0], &inner(sdk::MH_BEHEMOTH_WEIGHT), 0.5);
    let maps = world.set_items(mm, &sdk::MH_MAP_LIST, 1);
    world.set_text(maps[0], &sdk::MH_MAP_NAME, "Map_A");
    world.set_text(maps[0], &sdk::MH_MAP_ASSET_NAME, "Map_A_Asset");
    world.set_text(maps[0], &sdk::MH_MAP_ASSET, "/Game/Maps/Map_A.Map_A");
    world.set_int(maps[0], &sdk::MH_MAP_BIOME, 4);
    world.set_float(maps[0], &sdk::MH_MAP_WEIGHT, 2.0);
    world.set_texts(mm, &sdk::MH_MODIFIERS, &["Mod_1"]);
    world.set_int(mm, &sdk::MH_GATHERABLE_TIER, 2);
    world.set_int(mm, &sdk::MH_MAX_PLAYERS, 4);

    let (_, rows) = world.table("Schedules", "ScheduleData", &[("Sched_1", 0x100)]);
    let schedule = rows[0];
    world.set_int(schedule, &sdk::SD_START, 638_000_000_000_000_000);
    world.set_flag(schedule, &sdk::SD_REPEATABLE);
    world.set_flag(schedule, &sdk::SD_EACH_WEEK);
    let items = world.set_items(schedule, &sdk::SD_ITEMS, 1);
    world.set_text(items[0], &sdk::SD_ITEM_ID, "Trial_1");
    world.set_int(items[0], &sdk::SD_ITEM_MAX, 3);
    let left_out = world.set_items(schedule, &sdk::SD_LEFT_OUT, 2);
    world.mem.write_i32(left_out[0], 1);
    world.mem.write_i32(left_out[1], -2);

    let out = temp_dir("hunts");
    let engine = world.engine(&NoCalls);
    let status = status(&out);
    assert_eq!(hunts::export(&ctx(&engine, &out, &status)), 3);
    let folder = out.join(hunts::FOLDER);
    assert_eq!(
        read(&folder.join("player_hunts.jsonl")),
        crlf(&[
            r#"{"sourceTable":"PlayerHunts","rowName":"Hunt_1","playerHuntId":"PH_1","region":{"table":"RegionTable","rowName":"Region_A"},"matchmakingType":1,"matchmakingGameType":0,"huntTags":["Hunt.Patrol","Hunt.Escalation"],"matchmakerHuntIDs":[],"matchmakerHuntsByTag":[{"matchmakerTable":"MMTable","queries":[{"tokenStreamVersion":0,"tagDictionary":["Hunt.Patrol"],"tokenStream":[0,1,2,255],"userDescription":"","autoDescription":"auto"}]}],"escalationModeSpecification":{"table":"","rowName":"None"},"escalationPatrolInitialChallengeLevel":0,"escalationPatrolMaxChallengeLevel":0,"recommendedEscalationLevel":0,"hasPortals":true,"hasGlitterEvent":false,"hasPhaelanxEvent":false,"huntSuccessReward":"R1","firstCompletionSuccessReward":"","targetedHuntSuccessReward":"","huntFailureReward":"","targetedHuntFailureReward":"","minWeaponSkillLevel":5,"recommendedWeaponSkillLevel":10,"visibleWhileLocked":false}"#
        ])
    );
    assert_eq!(
        read(&folder.join("matchmaker_hunts.jsonl")),
        crlf(&[
            r#"{"sourceTable":"MatchmakerHunts","rowName":"MM_1","region":{"table":"","rowName":"None"},"huntTags":[],"huntThreatLevel":3,"gameModeOverride":"","dangerPerSecOverride":0.500000,"specificBehemoth":{"behemothName":"Shrike","behemothAssetPath":"/Game/Behemoths/Shrike.Shrike","behemothAssetResolvedName":"Shrike","powerOverride":0,"weighting":1.000000},"additionalSpecificBehemoths":[{"behemothName":"Skraev","behemothAssetPath":"","behemothAssetResolvedName":"","powerOverride":2,"weighting":0.500000}],"mapMetaData":{"table":"","rowName":"None"},"mapList":[{"mapName":"Map_A","mapAssetName":"Map_A_Asset","mapAssetPath":"/Game/Maps/Map_A.Map_A","mapAssetResolvedName":"","biome":4,"weighting":2.000000}],"specificModifier":"","modifiers":["Mod_1"],"gatherableTier":2,"maxPlayers":4,"isGeneratedEncounter":false,"gameModeSpecificData":{"table":"","rowName":"None"}}"#
        ])
    );
    assert_eq!(
        read(&folder.join("schedule_data.jsonl")),
        crlf(&[
            r#"{"sourceTable":"Schedules","rowName":"Sched_1","startTimeTicks":638000000000000000,"endTimeTicks":0,"isRepeatable":true,"eachDay":false,"eachWeek":true,"shouldQueue":false,"scheduledItems":[{"id":"Trial_1","maxCompletionPerInterval":3}],"itemsLeftOut":[1,-2]}"#
        ])
    );
    assert!(!folder.join("hunt_regions.jsonl").exists());
    let manifest = read(&folder.join(hunts::MANIFEST));
    assert!(
        manifest.starts_with(
            "{\r\n  \"gameVersion\": \"1.14.7\",\r\n  \"changelist\": 647472,\r\n  \"format\": \"jsonl\",\r\n  \
         \"playerHuntRows\": 1,\r\n  \"matchmakerHuntRows\": 1,\r\n  \"huntRegionRows\": 0,\r\n"
        ),
        "{manifest}"
    );
    assert!(manifest.contains("  \"scheduleRows\": 1,\r\n  \"notes\": \"Read-only live table export."));
    assert!(manifest.ends_with("importer validation is complete.\"\r\n}\r\n"));
    let log = status_text(&out);
    assert!(
        log.starts_with("[Hunts] starting read-only 1.12 hunt export\r\n"),
        "{log}"
    );
    assert!(log.contains("[Hunts] no loaded table with RowStruct=Hunt_Region; open the hunt/map UI and re-inject\r\n"));
}

#[test]
fn store_runtime_writes_view_models_and_images() {
    let mut world = World::new();
    world.class("StoreViewModel", Some("Object"));
    world.class("StoreItemViewModel", Some("Object"));
    world.default_object("Default__StoreViewModel", "StoreViewModel", 0x200);
    let view = world.object("SeasonStore", "StoreViewModel", 0x200);
    world.set_text(view, &sdk::SV_TAG_FILTER, "season");
    world.set_text(view, &sdk::SV_ITEMS_TABLE, "StoreItems");
    let categories = world.set_items(view, &sdk::SV_CATEGORIES, 1);
    let category = categories[0];
    world.set_text(category, &sdk::SV_CAT_NAME, "Featured");
    world.set_text(category, &sdk::SV_CAT_BREADCRUMB, "Store.Featured");
    world.set_flag(category, &sdk::SV_CAT_REQUIRES);
    world.set_text(category, &sdk::SV_CAT_SKU, "SKU1");
    let subs = world.set_items(category, &sdk::SV_CAT_SUBCATEGORIES, 1);
    world.set_text(subs[0], &sdk::SV_SUB_NAME, "Sub");
    world.set_text(subs[0], &sdk::SV_SUB_BREADCRUMB, "Store.Sub");
    world.set_texts(subs[0], &sdk::SV_SUB_TAG_IDS, &["t1"]);
    world.set_flag(subs[0], &sdk::SV_SUB_HIDE_PURCHASED);

    let item = world.object("Offer_1", "StoreItemViewModel", 0x400);
    world.set_text(item, &sdk::SI_DISPLAY_NAME, "Armour");
    world.set_text(item, &sdk::SI_DISPLAY_ITEM_TYPE, "Set");
    world.set_int(item, &sdk::SI_PRICE, 1000);
    world.set_int(item, &sdk::SI_CURRENT_PRICE, 800);
    world.set_int(item, &sdk::SI_CURRENCY_ENUM, 2);
    world.set_text(item, &sdk::SI_CURRENCY_CODE, "PLAT");
    world.set_int(item, &sdk::SI_PRIORITY, 5);
    world.set_flag(item, &sdk::SI_CAN_PURCHASE_AGAIN);
    world.set_int(item, &sdk::SI_RARITY, 3);
    world.set_text(item, &sdk::SI_CATALOG_ITEM_ID, "AR_SET");
    let offer = item + u64::from(sdk::SI_OFFER.offset);
    world.set_text(offer, &sdk::OF_SKU, "SKU_AR");
    world.set_text(offer, &sdk::OF_DISPLAY_NAME, "Armour");
    world.set_int(offer, &sdk::OF_PLAT_PRICE, 800);
    world.set_int(offer, &sdk::OF_REGULAR_PLAT_PRICE, 1000);
    world.set_int(offer, &sdk::OF_MAX_ALLOWED, 1);
    world.set_int(offer, &sdk::OF_REMAINING, 1);
    world.set_texts(offer, &sdk::OF_TAGS, &["tab_season_armour"]);
    let granted = world.set_items(offer, &sdk::OF_GRANTED_IDS, 1);
    world.set_text(granted[0], &sdk::OF_GRANTED_ID_ITEM, "AR_HEAD");
    world.set_int(granted[0], &sdk::OF_GRANTED_ID_AMOUNT, 1);
    world.set_text(offer, &sdk::OF_CURRENCY_CODE, "PLAT");
    world.set_int(offer, &sdk::OF_CURRENCY_ENUM, 2);
    world.set_texts(offer, &sdk::OF_PREVIEW_IDS, &["AR_HEAD"]);

    let (_, rows) = world.table("StoreItems", "StoreItemTable", &[("B_Row", 0x80), ("A_Row", 0x80)]);
    world.set_text(rows[0], &sdk::IMG_FEATURE, "/Game/UI/Feature_B.Feature_B");

    let out = temp_dir("store");
    let engine = world.engine(&NoCalls);
    let status = status(&out);
    assert_eq!(store::export(&ctx(&engine, &out, &status)), 5);
    assert_eq!(
        read(&out.join(store::VIEW_MODELS)),
        crlf(&[
            r#"{"objectName":"Default__StoreViewModel","objectClass":"StoreViewModel","defaultObject":true,"tagFilter":"","storeItemsTable":"","currencyDataTable":"","categories":[]}"#,
            r#"{"objectName":"SeasonStore","objectClass":"StoreViewModel","defaultObject":false,"tagFilter":"season","storeItemsTable":"StoreItems","currencyDataTable":"","categories":[{"name":"Featured","breadcrumbCategory":"Store.Featured","requiresEntitlement":true,"entitlementId":"","skuId":"SKU1","subcategories":[{"name":"Sub","breadcrumbCategory":"Store.Sub","requiresEntitlement":false,"entitlementId":"","skuId":"","subTagIds":["t1"],"hidePurchasedItems":true,"shouldHideTab":false,"hideTab":false}]}]}"#,
        ])
    );
    assert_eq!(
        read(&out.join(store::ITEM_VIEW_MODELS)),
        crlf(&[
            r#"{"objectName":"Offer_1","objectClass":"StoreItemViewModel","defaultObject":false,"displayName":"Armour","displayDescription":"","displayItemType":"Set","itemPriceNumber":1000,"itemCurrentPriceNumber":800,"storeCurrencyEnum":2,"currencyCode":"PLAT","displayPriority":5,"purchased":false,"canPurchaseAgain":true,"expired":false,"prestigeItemElite":false,"rewardRarityLevel":3,"catalogItemId":"AR_SET","offer":{"skuId":"SKU_AR","displayName":"Armour","displayDescription":"","displayPriority":0,"catalogId":"","imageUrl":"","platPrice":800,"regularPlatPrice":1000,"dustPrice":0,"maxAllowed":1,"remaining":1,"tags":["tab_season_armour"],"grantedCatalogIds":[{"itemId":"AR_HEAD","amount":1}],"grantedEntitlements":[],"grantedProgressInTracks":[],"grantedProgress":{"trackId":"","amount":0,"ranks":0},"grantedLoadoutSlots":0,"payWithAllCurrencies":false,"currencyCode":"PLAT","currencyEnum":2,"inUserCollection":false,"platformOfferId":"","timeAvailabilityReason":"","missingEntitlementNames":[],"previewItemIds":["AR_HEAD"]}}"#,
        ])
    );
    assert_eq!(
        read(&out.join(store::IMAGES)),
        crlf(&[
            r#"{"table":"StoreItems","rowName":"A_Row","featureImage":"None","standardImage":"None"}"#,
            r#"{"table":"StoreItems","rowName":"B_Row","featureImage":"Feature_B.Feature_B","standardImage":"None"}"#,
        ])
    );
    assert!(status_text(&out).contains("[StoreRuntime] DONE storeViewModels=2 itemViewModels=1 imageRows=2 skipped=0"));
}

#[test]
fn progression_checks_each_table_before_reading_it() {
    let mut world = World::new();
    let (_, rows) = world.table(
        "player_experience_track_table",
        "ExperienceTrackTableData",
        &[("Track_A", 0x200)],
    );
    let row = rows[0];
    let base = &sdk::EXP_BASE;
    use catalog_exporter::facts::*;
    world.set_text(row, &base[B_TRACK], "PlayerLevel");
    world.set_flag(row, &base[B_RESET]);
    world.set_float(row, &base[B_MULTIPLIER], 1.0);
    let requirements = world.set_items(row, &base[B_REQUIREMENTS], 2);
    for (at, (rank, xp)) in requirements.into_iter().zip([(1, 0), (2, 100)]) {
        world.set_int(at, &base[B_REQ_RANK], rank);
        world.set_int(at, &base[B_REQ_XP], xp);
    }
    let rewards = world.set_items(row, &base[B_FREE], 1);
    let reward = rewards[0];
    world.set_int(reward, &base[B_RW_RANK], 2);
    world.set_text(reward, &base[B_RW_CUSTOM], "Reward!");
    let stacked = world.set_items(reward, &base[B_RW_STACKED], 1);
    world.set_text(stacked[0], &base[B_ST_ID], "CURRENCY_NOTES");
    world.set_int(stacked[0], &base[B_ST_QUANTITY], 500);
    world.set_texts(reward, &base[B_RW_INSTANCED], &["WP_X"]);
    let entitlements = world.set_items(reward, &base[B_RW_ENTITLEMENTS], 1);
    world.set_text(entitlements[0], &base[B_EN_ID], "ENT_1");
    world.set_text(row, &sdk::EXP_PRESTIGE_TRACK, "Prestige_A");
    world.set_text(row, &sdk::EXP_SHORT_NAME, "XP");
    world.set_text(row, &sdk::EXP_NAME, "Experience");
    world.set_text(row, &sdk::EXP_DESCRIPTION, "desc");
    world.set_int(row, &sdk::EXP_BANKING_LEVELS, 3);
    world.table("weapon_experience_track_table", "WrongStruct", &[("Ignored", 0x200)]);
    let (_, awards) = world.table("ExpAwards", "ExperienceTableData", &[("Kill", 0x80)]);
    world.set_int(awards[0], &sdk::AW_TYPE, 1);
    world.set_int(awards[0], &sdk::AW_AMOUNT, 10);

    let out = temp_dir("progression");
    let engine = world.engine(&NoCalls);
    let status = status(&out);
    assert_eq!(progression::export(&ctx(&engine, &out, &status)), 2);
    let folder = out.join(progression::FOLDER);
    assert_eq!(
        read(&folder.join("player-experience-tracks.jsonl")),
        crlf(&[
            r#"{"rowName":"Track_A","base":{"progressionTrack":"PlayerLevel","premiumGatingEntitlement":"","resetWithCharacter":true,"progressionMultiplier":1.000000,"requirementCount":2,"requirements":[{"rankId":1,"requiredXP":0},{"rankId":2,"requiredXP":100}],"freeRewards":[{"rankId":2,"customRewardString":"Reward!","stackedItems":[{"catalogId":"CURRENCY_NOTES","quantity":500,"priority":0}],"instancedItems":["WP_X"],"orderedInstancedItems":[],"entitlements":[{"entitlementId":"ENT_1","duration":0}],"buffCount":0}],"premiumRewards":[]},"prestigeTrack":"Prestige_A","expDisplayShortName":"XP","expDisplayName":"Experience","expDisplayDescription":"desc","experienceBankingLevelCount":3}"#
        ])
    );
    assert_eq!(
        read(&folder.join("experience-awards.jsonl")),
        crlf(&[r#"{"rowName":"Kill","experienceType":1,"amount":10,"objectiveCount":0,"table":"ExpAwards"}"#])
    );
    for missing in [
        "weapon-experience-tracks.jsonl",
        "experience-tracks.jsonl",
        "mastery-tracks.jsonl",
    ] {
        assert!(!folder.join(missing).exists(), "{missing}");
    }
    let log = status_text(&out);
    assert!(log.contains(
        "[Progression] RowStruct mismatch on weapon_experience_track_table: expected ExperienceTrackTableData got \
         WrongStruct - skipping (never guessing a wrong cast)\r\n"
    ));
    assert!(log.contains("[Progression] experience_track_table: table not found, skipping\r\n"));
}

#[test]
fn progression_falls_back_to_the_experience_component() {
    let mut world = World::new();
    world.class("PlayerExperienceComponent", Some("Object"));
    let (table, rows) = world.table("Composite", "ExperienceTableData", &[("Craft", 0x80)]);
    world.set_int(rows[0], &sdk::AW_AMOUNT, 7);
    // Out of the object array, so only the component leads to it.
    world.unregister(table);
    let component = world.object("Component_0", "PlayerExperienceComponent", 0x100);
    world.mem.write_u64(component + u64::from(sdk::PEC_TABLE.offset), table);

    let out = temp_dir("awards");
    let engine = world.engine(&NoCalls);
    let status = status(&out);
    progression::export(&ctx(&engine, &out, &status));
    assert_eq!(
        read(&out.join(progression::FOLDER).join("experience-awards.jsonl")),
        crlf(&[r#"{"rowName":"Craft","experienceType":0,"amount":7,"objectiveCount":0,"table":"Composite"}"#])
    );
    assert!(status_text(&out).contains(
        "experience awards: using UPlayerExperienceComponent::ExperienceTable composite fallback (Composite)"
    ));
}

#[test]
fn mastery_writes_its_pretty_layout_with_lf() {
    let mut world = World::new();
    let meta = world.class_ptr("Class");
    let kill = world.raw_object("ObjectiveData_Kill", meta, 0, 0x100);
    let hunt = world.raw_object("ObjectiveData_Hunt", meta, 0, 0x100);
    let (_, cards) = world.table(
        "mastery_card_table",
        "MasteryCardTableData",
        &[("Card_B", 0x100), ("Card_A", 0x100)],
    );
    let card = cards[1];
    let objectives = world.set_items(card, &sdk::MC_OBJECTIVES, 3);
    world.set_text(objectives[0], &sdk::MC_OBJ_ID, "obj_kill_10");
    world.set_int(objectives[0], &sdk::MC_OBJ_TARGET, 10);
    world
        .mem
        .write_u64(objectives[0] + u64::from(sdk::MC_OBJ_DATA.offset), kill);
    world.set_text(objectives[0], &sdk::MC_OBJ_COMPOUND, "None");
    world.set_int(objectives[1], &sdk::MC_OBJ_TARGET, 99);
    world.set_text(objectives[2], &sdk::MC_OBJ_ID, "obj_rank");
    world.set_int(objectives[2], &sdk::MC_OBJ_TARGET, 1);
    world.set_text(objectives[2], &sdk::MC_OBJ_COMPOUND, "cmp_1");
    world.set_int(objectives[2], &sdk::MC_OBJ_RANK, 2);
    world.set_text(objectives[2], &sdk::MC_OBJ_TRACK, "MasteryTrack_Axe");
    world.set_texts(
        card,
        &sdk::MC_REWARD_TRACKS,
        &["MasteryTrack_PlayerLevel", "MasteryTrack_Axe"],
    );
    world.set_int(card, &sdk::MC_CARD_TYPE, sdk::CARD_TYPE_WEAPON);
    world.set_text(card, &sdk::MC_GAME_OBJECT, "WP_AXE");
    world.set_text(card, &sdk::MC_BASE_XP, "obj_base_xp");
    world.set_text(card, &sdk::MC_PRESTIGE_XP, "None");
    world.table("mastery_card_table_extra", "MasteryCardTableData", &[("Card_A", 0x100)]);
    let (_, achievements) = world.table("achievements_table", "AchievementTableData", &[("Ach_1", 0x100)]);
    let objectives = world.set_items(achievements[0], &sdk::AC_OBJECTIVES, 1);
    world.set_text(objectives[0], &sdk::AC_OBJ_ID, "obj_ach");
    world.set_int(objectives[0], &sdk::AC_OBJ_TARGET, 5);
    world
        .mem
        .write_u64(objectives[0] + u64::from(sdk::AC_OBJ_DATA.offset), hunt);

    let out = temp_dir("mastery");
    let engine = world.engine(&NoCalls);
    let status = status(&out);
    assert_eq!(mastery::export_dated(&ctx(&engine, &out, &status), "2026-11-11"), 3);
    let expected = r#"{
  "schema": 1,
  "source": "Dauntless 1.14.7 CL 647472: mastery_card_table, achievements_table (CatalogExporter live export of mastery_card_table, mastery_card_table_extra, achievements_table), extracted 2026-11-11",
  "objectiveIds": [
    "obj_ach",
    "obj_base_xp",
    "obj_kill_10",
    "obj_rank"
  ],
  "cards": [
    {
      "cardId": "Card_A", "track": "MasteryTrack_Axe",
      "objectives": [
        {"id":"obj_kill_10","target":10,"type":"Kill"},
        {"id":"obj_rank","target":1,"compoundId":"cmp_1","rank":2,"progressTrack":"MasteryTrack_Axe"}
      ],
      "type": "Weapon", "gameObjectId": "WP_AXE", "baseXpObjectiveId": "obj_base_xp"
    },
    {
      "cardId": "Card_B", "track": null,
      "objectives": [],
      "type": null
    }
  ],
  "achievements": [
    {
      "id": "Ach_1",
      "objectives": [
        {"id":"obj_ach","target":5,"type":"Hunt"}
      ]
    }
  ]
}
"#;
    assert_eq!(read(&out.join(mastery::FOLDER).join(mastery::FILE)), expected);
    assert!(status_text(&out).contains("[Mastery] mastery_card_table_extra: duplicate row Card_A dropped"));
}

#[test]
fn mastery_without_tables_writes_nothing() {
    let world = World::new();
    let out = temp_dir("mastery-none");
    let engine = world.engine(&NoCalls);
    let status = status(&out);
    assert_eq!(mastery::export_dated(&ctx(&engine, &out, &status), "2026-11-11"), 0);
    assert!(!out.join(mastery::FOLDER).join(mastery::FILE).exists());
    assert!(status_text(&out).contains("[Mastery] neither table is loaded; nothing written"));
}
