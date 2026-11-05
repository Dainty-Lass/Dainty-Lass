//! `fountainDailyBundle.gamedata.test.ts` and `storeRegistry.gamedata.test.ts`, the parts that test code rather than the
//! host's exported data (the counts and prices of the real Lady Luck and Reward Cache files are a host's own check), on
//! synthetic game data in a temporary folder.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;

use serde_json::{Value, json};

use super::store::*;
use crate::game_data::GameDataRoot;

const TAG: &str = FOUNTAIN_DAILY_BUNDLE_TAG;

fn root(dir: &Path) -> GameDataRoot {
    GameDataRoot {
        backend: Some(dir.to_path_buf()),
        director: None,
    }
}

fn write(dir: &Path, name: &str, value: &Value) {
    std::fs::write(dir.join(name), value.to_string()).unwrap();
}

/// A storage policy and a progression config that make `CURRENCY_NOTES` a plain stacked grant, `AC_BACK_HP06-1_00`
/// an instanced item and `CURRENCY_SEASONAL_COIN` a prestige-gated reward.
fn game_data(dir: &Path) -> GameDataRoot {
    write(
        dir,
        "inventory_storage_policy.json",
        &json!({ "schema": 1, "entries": {
            "CURRENCY_NOTES": { "storage": "stacked", "maxQuantity": 0, "tags": ["Currency"] },
            "CURRENCY_SEASONAL_COIN": { "storage": "stacked", "maxQuantity": 0, "tags": ["Currency"] },
            "DYE_ONE": { "storage": "stacked", "maxQuantity": 1, "tags": ["Dye"] },
            "AC_BACK_HP06-1_00": { "storage": "instanced", "maxQuantity": 1, "tags": [] },
        } }),
    );
    write(
        dir,
        "progression_config.json",
        &json!({ "payload": { "paths": [{ "progression_id": "season43", "requirements": [{ "xp_required": 100 }],
            "prestige": { "xp_per_level": 100, "free_rewards": { "stacked_items": [
                { "catalog_id": "CURRENCY_SEASONAL_COIN", "quantity": 5 }] } } }] } }),
    );
    root(dir)
}

fn base() -> Value {
    json!({
        "id": "test_fountain_bundle", "displayName": "Test", "displayDescription": "Test", "displayPriority": 0,
        "prices": [{ "currencyId": "id_currency_platinum", "price": 0 }], "maxAllowed": 1, "images": {},
        "tags": [TAG],
        "items": [{ "catalogId": "CURRENCY_NOTES", "quantity": 500, "instanced": false }],
        "duplicateInstancedItems": [],
    })
}

fn with(key: &str, value: Value) -> Value {
    let mut sku = base();
    sku[key] = value;
    sku
}

fn problems(root: &GameDataRoot, sku: &Value) -> Vec<String> {
    parse_fountain_daily_bundle(&json!({ "comment": "x", "skus": [sku] }), root).1
}

#[test]
fn the_data_file_is_validated_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let root = game_data(dir.path());
    assert_eq!(problems(&root, &base()), Vec::<String>::new());
    assert_eq!(
        parse_fountain_daily_bundle(&json!([base()]), &root).0.len(),
        1,
        "a bare array is accepted too"
    );
    let (skus, _) = parse_fountain_daily_bundle(&json!({ "skus": [base()] }), &root);
    assert_eq!(skus[0].max_allowed, json!(1));

    let two = json!({ "skus": [base(), with("id", json!("second"))] });
    assert!(!parse_fountain_daily_bundle(&two, &root).1.is_empty(), "one offer only");
    assert!(!parse_fountain_daily_bundle(&json!({ "skus": [] }), &root).1.is_empty());
    assert!(!parse_fountain_daily_bundle(&json!({}), &root).1.is_empty());
    assert!(!parse_fountain_daily_bundle(&Value::Null, &root).1.is_empty());
    let bad = |sku: Value| !problems(&root, &sku).is_empty();
    assert!(bad(with("id", json!("bad/id"))));
    assert!(bad(with("tags", json!(["webstore"]))));
    assert!(bad(with(
        "prices",
        json!([{ "currencyId": "id_currency_platinum", "price": 1 }])
    )));
    assert!(bad(with(
        "prices",
        json!([{ "currencyId": "id_currency_marks_steel", "price": 0 }])
    )));
    assert!(bad(with("prices", json!([]))));
    assert!(bad(with(
        "entitlements",
        json!([{ "name": "exchange_slot_2", "duration": 0 }])
    )));
    assert!(bad(with(
        "skuProgression",
        json!({ "progressionId": "selected_huntpass", "xp": null, "ranks": 1 })
    )));
    assert!(bad(with("items", json!([]))));
    assert!(bad(with(
        "items",
        json!([{ "catalogId": "CURRENCY_NOTES", "quantity": 500, "instanced": true }])
    )));
    assert!(
        bad(with(
            "items",
            json!([{ "catalogId": "CURRENCY_NOTES", "quantity": 500 }])
        )),
        "instanced must be explicitly false"
    );
    assert!(bad(with(
        "items",
        json!([{ "catalogId": "CURRENCY_NOTES", "quantity": 0, "instanced": false }])
    )));
    assert!(bad(with(
        "items",
        json!([{ "catalogId": "CURRENCY_NOTES", "quantity": 1.5, "instanced": false }])
    )));
    assert!(bad(with(
        "items",
        json!([{ "catalogId": "NOT_A_REAL_CATALOG_ID", "quantity": 1, "instanced": false }])
    )));
    assert!(bad(with(
        "items",
        json!([{ "catalogId": "AC_BACK_HP06-1_00", "quantity": 1, "instanced": false }])
    )));
    assert!(
        bad(with(
            "items",
            json!([{ "catalogId": "DYE_ONE", "quantity": 1, "instanced": false }])
        )),
        "a capped item can't repeat daily"
    );
    let gated = problems(
        &root,
        &with(
            "items",
            json!([{ "catalogId": "CURRENCY_SEASONAL_COIN", "quantity": 5, "instanced": false }]),
        ),
    );
    assert!(gated.iter().any(|p| p.contains("prestige-gated")), "{gated:?}");
}

#[test]
fn a_missing_data_file_leaves_the_section_empty() {
    let dir = tempfile::tempdir().unwrap();
    let root = game_data(dir.path());
    let catalog = StoreCatalog::load(&root, &["exchange_vendor_slot_2", TAG]);
    assert!(catalog.skus(TAG).is_empty());
    assert!(parse_fountain_daily_bundle(&Value::Null, &root).0.is_empty());
}

#[test]
fn a_valid_bundle_loads_free_stacked_and_reachable_only_by_its_own_id() {
    let dir = tempfile::tempdir().unwrap();
    let root = game_data(dir.path());
    write(dir.path(), "fountain_daily_bundle.json", &json!({ "skus": [base()] }));
    let catalog = StoreCatalog::load(&root, &["exchange_vendor_slot_2", "exchange_vendor_slot_3", TAG]);
    let offer = &catalog.skus(TAG)[0];
    assert_eq!(catalog.skus(TAG).len(), 1);
    assert_eq!(offer.prices[0].currency_id, "id_currency_platinum");
    assert_eq!(offer.prices[0].price, json!(0));
    assert!(offer.items.iter().all(|i| !i.instanced));
    assert_eq!(catalog.find_by_id_or_catalog_id(&offer.id).unwrap().0, TAG);
    // the fountain grants plain currency other stores may sell too: never the fallback of a bare catalog id
    assert!(catalog.find_by_id_or_catalog_id("CURRENCY_NOTES").is_none());
}

#[test]
fn a_fountain_offer_sharing_an_id_with_another_store_is_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let root = game_data(dir.path());
    write(
        dir.path(),
        "fountain_daily_bundle.json",
        &json!({ "skus": [with("id", json!("single_exchange_slot_2"))] }),
    );
    let catalog = StoreCatalog::load(&root, &["exchange_vendor_slot_2", TAG]);
    assert!(catalog.skus(TAG).is_empty());
    assert_eq!(
        catalog.find_by_id_or_catalog_id("single_exchange_slot_2").unwrap().0,
        "exchange_vendor_slot_2"
    );
}

#[test]
fn middleman_fusion_slots_2_and_3_are_sold_for_1000_platinum_as_permanent_entitlements() {
    let dir = tempfile::tempdir().unwrap();
    let catalog = StoreCatalog::load(
        &game_data(dir.path()),
        &["exchange_vendor_slot_2", "exchange_vendor_slot_3"],
    );
    for slot in [2, 3] {
        let skus = catalog.skus(&format!("exchange_vendor_slot_{slot}"));
        assert_eq!(skus.len(), 1);
        let sku = &skus[0];
        assert_eq!(sku.id, format!("single_exchange_slot_{slot}"));
        assert_eq!(sku.prices[0].currency_id, "id_currency_platinum");
        assert_eq!(sku.prices[0].price, json!(1000));
        assert_eq!(sku.entitlements, [(format!("exchange_slot_{slot}"), json!(0))]);
        assert_eq!(sku.max_allowed, json!(1));
        assert!(sku.items.is_empty());
        assert_eq!(
            catalog.find_by_id_or_catalog_id(&sku.id).unwrap().0,
            format!("exchange_vendor_slot_{slot}")
        );
    }
}

#[test]
fn a_version_serves_only_the_sections_its_table_names() {
    let dir = tempfile::tempdir().unwrap();
    let root = game_data(dir.path());
    write(
        dir.path(),
        "ladyluck_store.json",
        &json!([{ "id": "ll1", "displayName": "L", "displayDescription": "", "displayPriority": 1,
            "prices": [{ "currencyId": "id_currency_marks_steel", "price": 5 }], "maxAllowed": 1, "tags": ["ladyluckstore"],
            "items": [{ "catalogId": "WP_X", "quantity": 1, "instanced": true }], "duplicateInstancedItems": [] }]),
    );
    write(
        dir.path(),
        "platinum_store.json",
        &json!([{ "id": "dye1", "displayName": "D", "displayDescription": "", "displayPriority": 1,
            "prices": [{ "currencyId": "id_currency_platinum", "price": 5 }], "maxAllowed": 1, "tags": ["webstore", "dyes"],
            "items": [{ "catalogId": "DYE_ONE", "quantity": 1, "instanced": false }], "duplicateInstancedItems": [] }]),
    );
    let v112 = StoreCatalog::load(&root, &["ladyluckstore"]);
    assert_eq!(v112.skus("ladyluckstore").len(), 1);
    assert!(v112.skus("webstore").is_empty());
    assert!(v112.find_by_id_or_catalog_id("dye1").is_none());
    let v1147 = StoreCatalog::load(&root, &["ladyluckstore", "webstore", "dyes", "hp_level_skip_rank"]);
    assert_eq!(v1147.skus("webstore").len(), 1);
    assert_eq!(v1147.skus("dyes").len(), 1);
    assert!(v1147.skus("hp_level_skip_rank").is_empty());
    // one SKU reachable from two sections resolves to one offer, the first section's
    assert_eq!(v1147.find_by_id_or_catalog_id("DYE_ONE").unwrap().0, "webstore");
}
