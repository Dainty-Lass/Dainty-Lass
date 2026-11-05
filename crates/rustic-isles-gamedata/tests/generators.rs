// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The generators on synthetic fixtures (spec §10.2): made-up ids and paths, never a row of a real export.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use regex::Regex;
use rustic_isles_gamedata::generators::{
    enrich_ladyluck_store, generate_inventory_storage_policy, generate_ladyluck_store, generate_platinum_store,
    generate_progression_config, generate_reward_cache_store, generate_slayers_path, generate_slayers_path_definitions,
    resolve_platinum_export_file, stringify_pretty,
};
use rustic_isles_gamedata::{
    BackendStep, GeneratorPaths, RewardCacheCategory, RewardCachePlan, RewardCacheSelection, TokenSource,
    VersionInputs, run,
};
use serde_json::{Value, json};

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

fn read_json(path: &Path) -> Value {
    service_kit::jsjson::parse(&read(path)).unwrap()
}

fn backups(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".bak"))
        .collect();
    names.sort();
    names
}

/// A catalog row; `icon` goes in `ItemData.Icon`.
fn item(id: &str, rarity: f64, icon: &str, tags: &[&str], stackable: Option<bool>) -> Value {
    let data = json!({"RarityLevel": rarity, "Icon": icon, "LargeIcon": "", "CustomIcon": ""});
    let mut row = json!({
        "itemId": id,
        "displayName": format!("{id} name"),
        "description": format!("{id} text"),
        "tags": tags,
        "maxQuantity": 1,
        "customData": json!({"ItemData": data.to_string()}).to_string(),
    });
    if let Some(s) = stackable {
        row["isStackable"] = json!(s);
    }
    row
}

fn jsonl(rows: &[Value]) -> String {
    rows.iter().map(Value::to_string).collect::<Vec<_>>().join("\n") + "\n"
}

const ICON: &str = "/Game/UI/Icons/Icon_0.Icon_0";

#[test]
fn progression_generator() {
    let dir = tempfile::tempdir().unwrap();
    let raw = dir.path().join("raw");
    let out = dir.path().join("out");
    let err = generate_progression_config(&raw, &out, true).unwrap_err();
    assert!(err.starts_with("ERROR: source not found at "), "{err}");
    assert!(
        err.ends_with(
            "\nProvide the progression payload from your own installation (see docs/GENERATING_GAME_DATA.md)."
        )
    );
    let source = raw.join("progression_config.source.json");
    write(&source, "not json");
    assert!(
        generate_progression_config(&raw, &out, true)
            .unwrap_err()
            .starts_with("ERROR: source is not valid JSON: SyntaxError: ")
    );
    write(&source, r#"{"payload":{}}"#);
    assert_eq!(
        generate_progression_config(&raw, &out, true).unwrap_err(),
        "ERROR: source has no payload.paths[] array — refusing to write a shape the backend cannot read."
    );
    write(
        &source,
        "\u{feff}{\"extra\":1,\"message\":5,\"payload\":{\"v\":2,\"paths\":[{\"progression_id\":\"ExperienceTrack_PlayerLevel\",\"free_rewards\":[{\"tier\":1}]}]}}",
    );
    let dry = generate_progression_config(&raw, &out, false).unwrap();
    assert_eq!(
        dry,
        "source paths      : 1\nfree_rewards tiers : 1\nmode              : DRY RUN (pass --apply to write)\n\nDry run complete. Nothing written."
    );
    assert!(!out.exists());
    let report = generate_progression_config(&raw, &out, true).unwrap();
    assert!(report.ends_with("mode              : APPLY\n\nWrote progression_config.json (1 paths)"));
    let target = out.join("progression_config.json");
    let text = read(&target);
    assert!(!text.ends_with('\n'));
    assert_eq!(
        text,
        stringify_pretty(
            &json!({"code": null, "message": "OK", "payload": {"v": 2, "paths": [{"progression_id": "ExperienceTrack_PlayerLevel", "free_rewards": [{"tier": 1}]}]}}),
            2
        )
    );
    generate_progression_config(&raw, &out, true).unwrap();
    let names = backups(&out);
    assert_eq!(names.len(), 1);
    let stamp = Regex::new(r"^progression_config\.json\.\d{4}-\d\d-\d\dT\d\d-\d\d-\d\d-\d{3}Z\.bak$").unwrap();
    assert!(stamp.is_match(&names[0]), "{}", names[0]);
}

fn journey_rows() -> String {
    [
        json!({"nodeId": "Slayer_09", "autoUnlockIfParentUnlocked": true, "childNodes": ["Slayer_10", "None", ""],
               "currencyCosts": [{"currency": "CURRENCY_RAMS", "amount": 100}, {"currency": "None", "amount": 5}],
               "rewards": [{"itemId": "WP_SWORD_00"}, {"itemId": "None"}]}),
        json!({"nodeId": "Slayer_10", "autoUnlockIfParentUnlocked": 1, "childNodes": [], "currencyCosts": [{"currency": "CURRENCY_RAMS"}], "rewards": []}),
        json!({"nodeId": "Slayer_11", "childNodes": [], "currencyCosts": [], "rewards": [{"itemId": "None"}]}),
        json!({"nodeId": ""}),
    ]
    .iter()
    .map(Value::to_string)
    .collect::<Vec<_>>()
    .join("\n")
}

#[test]
fn slayers_path() {
    let dir = tempfile::tempdir().unwrap();
    let export = dir.path().join("exports/player_journey_nodes.jsonl");
    let target = dir.path().join("backend/slayers_path.json");
    let err = generate_slayers_path(&export, &target, true).unwrap_err();
    assert!(
        err.ends_with(" — run CatalogExporter with EXPORT_SLAYERS_PATH=1 first"),
        "{err}"
    );
    write(&export, &format!("\n\n{}\n", journey_rows()));
    assert!(
        generate_slayers_path(&export, &target, true)
            .unwrap_err()
            .starts_with("ERROR: target not found at ")
    );
    write(&target, r#"{"payload":{}}"#);
    assert_eq!(
        generate_slayers_path(&export, &target, true).unwrap_err(),
        "ERROR: existing slayers_path.json has no payload.nodes — refusing to guess at the shape"
    );
    write(
        &target,
        r#"{"code":null,"payload":{"nodes":{"Old_01":{},"Slayer_09":{}},"x":1}}"#,
    );
    let dry = generate_slayers_path(&export, &target, false).unwrap();
    assert!(dry.contains("existing baseline : 2 nodes\nlive 1.12 table   : 3 nodes\n  added   : 2\n  dropped : 1"));
    assert!(dry.contains(&format!("  {:<30} before=present  after=present", "Slayer_09")));
    assert!(dry.contains(&format!("  {:<30} before=MISSING  after=MISSING", "Lantern.Unlock")));
    assert!(dry.contains("    Old_01"));
    assert!(dry.ends_with("Dry run. Nothing written. Re-run with --apply to commit."));
    let report = generate_slayers_path(&export, &target, true).unwrap();
    assert!(report.contains("Backed up: slayers_path.json."));
    assert!(report.contains("Wrote    : slayers_path.json (3 nodes)"));
    let doc = read_json(&target);
    assert_eq!(
        service_kit::jsjson::stringify(&doc),
        r#"{"code":null,"payload":{"nodes":{"Slayer_09":{"node_id":"Slayer_09","node_status":0,"objectives":[]},"Slayer_10":{"node_id":"Slayer_10","node_status":0,"objectives":[]},"Slayer_11":{"node_id":"Slayer_11","node_status":0,"objectives":[]}},"x":1}}"#
    );

    let defs = dir.path().join("backend/defs/slayers_path_definitions.json");
    let report = generate_slayers_path_definitions(&export, &defs, "1_14_7", true).unwrap();
    assert!(report.starts_with("source  : slayers_path_1_14_7 (4 nodes)\nexisting: 0 definitions; next: 2\n  added   : 2\n  Slayer_09, Slayer_10"), "{report}");
    let text = read(&defs);
    assert_eq!(
        text,
        "{\n \"Slayer_09\": {\n  \"autoUnlockIfParentUnlocked\": true,\n  \"childNodes\": [\n   \"Slayer_10\"\n  ],\n  \"currencyCosts\": [\n   {\n    \"amount\": 100,\n    \"currency\": \"CURRENCY_RAMS\"\n   }\n  ],\n  \"rewards\": [\n   \"WP_SWORD_00\"\n  ]\n },\n \"Slayer_10\": {\n  \"currencyCosts\": [\n   {\n    \"currency\": \"CURRENCY_RAMS\"\n   }\n  ]\n }\n}"
    );
    write(&defs, &text.replace('\n', "\r\n"));
    let again = generate_slayers_path_definitions(&export, &defs, "1_14_7", false).unwrap();
    assert!(
        again.contains("  added   : 0\n  removed : 0\n  changed : 0\n  identical bytes: true"),
        "{again}"
    );
    assert!(again.ends_with("Dry run. Nothing written. Re-run with --apply to write."));

    // The graph trims the whole text first, so leading blank lines shift its numbering only
    write(&export, &format!("\n\n{}\n{{bad\n", journey_rows()));
    assert_eq!(
        generate_slayers_path(&export, &target, false).unwrap_err(),
        "ERROR: malformed JSON on line 5 of the export"
    );
    assert_eq!(
        generate_slayers_path_definitions(&export, &defs, "1_14_7", false).unwrap_err(),
        "ERROR: malformed JSON on line 7 of the export"
    );
    write(&export, "{}\n{bad");
    assert!(
        generate_slayers_path(&export, &target, false)
            .unwrap_err()
            .contains("line 2")
    );
}

fn selection(id: &str, category: RewardCacheCategory, price: i64, featured: bool) -> RewardCacheSelection {
    RewardCacheSelection {
        catalog_id: id.into(),
        category,
        price,
        featured,
        allow_rarity3: false,
        display_name: None,
        description: None,
    }
}

fn reward_catalog() -> Vec<Value> {
    vec![
        item("AR_B_LEGS_00", 4.0, ICON, &["Legs"], None),
        item("AR_B_HELM_00", 4.0, ICON, &["Helm"], None),
        item("AR_A_CHEST_00", 4.0, ICON, &["Chest"], None),
        item("AR_A_HELM_00_ULTRA", 5.0, ICON, &["Helm"], Some(false)),
        item("WP_SW_X", 4.0, ICON, &[], None),
        item("WP_GA_LOW", 4.0, ICON, &[], None),
        item("WP_GA_HIGH", 5.0, ICON, &[], None),
        item("GD_GLIDE", 3.0, ICON, &["Glider"], None),
        item("BNC_SIGIL_FAM1", 4.0, ICON, &[], None),
        item("BNC_STANDARD_FAM2", 4.0, ICON, &[], None),
        item("BNC_FABRIC_FAM1", 4.0, ICON, &[], None),
        item("EM_EVENT", 4.0, ICON, &[], Some(true)),
        item("EJ_R3", 3.0, ICON, &[], None),
    ]
}

fn reward_plan() -> RewardCachePlan {
    use RewardCacheCategory::*;
    let mut featured = selection("EJ_R3", General, 700, true);
    featured.allow_rarity3 = true;
    featured.display_name = Some("  Custom name ".into());
    RewardCachePlan {
        selections: vec![
            selection("AR_B_LEGS_00", Armour, 1000, false),
            selection("WP_SW_X", Weapon, 800, false),
            selection("AR_B_HELM_00", Armour, 1000, false),
            selection("WP_GA_LOW", Weapon, 800, false),
            selection("AR_A_CHEST_00", Armour, 1000, false),
            selection("AR_A_HELM_00_ULTRA", Armour, 2000, false),
            selection("WP_GA_HIGH", Weapon, 800, false),
            selection("EM_EVENT", Event, 700, false),
            selection("BNC_SIGIL_FAM1", Event, 700, false),
            selection("BNC_FABRIC_FAM1", Event, 700, false),
            selection("BNC_STANDARD_FAM2", Event, 700, false),
            selection("GD_GLIDE", Style, 2500, false),
            featured,
        ],
        armour_families: vec!["_A_".into(), "_B_".into()],
        banner_families: vec!["FAM2".into(), "FAM1".into()],
        expected: vec![(Armour, 4), (Weapon, 3), (Style, 1), (Event, 4), (General, 1)],
        expected_featured: 1,
    }
}

#[test]
fn reward_cache() {
    let dir = tempfile::tempdir().unwrap();
    let catalog = dir.path().join("catalog.jsonl");
    let output = dir.path().join("out/reward_cache_store.json");
    write(&catalog, &jsonl(&reward_catalog()));
    let report = generate_reward_cache_store(&catalog, &output, &reward_plan()).unwrap();
    assert_eq!(
        report,
        format!(
            "Wrote {}\nSKUs: 13; featured: 1\nCategories: armour=4, weapon=3, style=1, event=4, limited=0, general=1\nPrices: 700=5, 800=3, 1000=3, 2000=1, 2500=1",
            output.display()
        )
    );
    let text = read(&output);
    assert!(text.ends_with("]\n"));
    let skus = read_json(&output);
    let ids: Vec<&str> = skus
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        [
            "season19_rewardcache_general_ej_r3",
            "season19_rewardcache_armour_ar_a_helm_00_ultra",
            "season19_rewardcache_armour_ar_a_chest_00",
            "season19_rewardcache_armour_ar_b_helm_00",
            "season19_rewardcache_armour_ar_b_legs_00",
            "season19_rewardcache_weapon_wp_ga_high",
            "season19_rewardcache_weapon_wp_ga_low",
            "season19_rewardcache_weapon_wp_sw_x",
            "season19_rewardcache_style_gd_glide",
            "season19_rewardcache_event_bnc_standard_fam2",
            "season19_rewardcache_event_bnc_fabric_fam1",
            "season19_rewardcache_event_bnc_sigil_fam1",
            "season19_rewardcache_event_em_event",
        ]
    );
    assert_eq!(
        service_kit::jsjson::stringify(&skus[0]),
        format!(
            r#"{{"id":"season19_rewardcache_general_ej_r3","displayName":"Custom name","displayDescription":"EJ_R3 text","displayPriority":1,"prices":[{{"currencyId":"id_currency_seasonal_coin","price":700}}],"maxAllowed":1,"images":{{"standard":"Engine.Texture2D'{ICON}'"}},"tags":["season_store","season_store_general","season_store_feature","season19","reward_cache_reconstructed","reward_cache_rarity_3"],"items":[{{"catalogId":"EJ_R3","quantity":1,"instanced":true}}],"duplicateInstancedItems":["EJ_R3"]}}"#
        )
    );
    assert_eq!(skus[1]["items"][0]["instanced"], true);
    assert_eq!(skus[12]["items"][0]["instanced"], false);
    assert_eq!(skus[12]["displayPriority"], 13);

    let fails = |plan: &RewardCachePlan, rows: Vec<Value>| {
        let catalog = dir.path().join("fail.jsonl");
        write(&catalog, &jsonl(&rows));
        let target = dir.path().join("fail/out.json");
        let err = generate_reward_cache_store(&catalog, &target, plan).unwrap_err();
        assert!(!target.exists());
        err
    };
    let one = |s: RewardCacheSelection| RewardCachePlan {
        selections: vec![s],
        expected: vec![(RewardCacheCategory::Armour, 1)],
        ..RewardCachePlan::default()
    };
    let a = |id: &str| selection(id, RewardCacheCategory::Armour, 1000, false);
    let good = || item("AR_OK", 4.0, ICON, &[], None);
    let mut dup = one(a("AR_OK"));
    dup.selections.push(a("AR_OK"));
    assert_eq!(fails(&dup, vec![good()]), "Duplicate catalog selection: AR_OK");
    let mut priced = a("AR_OK");
    priced.price = 900;
    assert_eq!(fails(&one(priced), vec![good()]), "Unsupported price 900 for AR_OK");
    assert_eq!(
        fails(&one(a("ar_founder_x")), vec![good()]),
        "Exclusive/progression catalog id is not eligible: ar_founder_x"
    );
    assert_eq!(
        fails(&one(a("WP_GA_SPOOKY_01")), vec![good()]),
        "Quest/Rumour-only catalog item is not eligible: WP_GA_SPOOKY_01"
    );
    assert_eq!(
        fails(&one(a("AR_NONE")), vec![good()]),
        "Catalog item not found: AR_NONE"
    );
    let mut nameless = good();
    nameless["displayName"] = json!("  ");
    assert_eq!(fails(&one(a("AR_OK")), vec![nameless]), "Missing display name: AR_OK");
    let mut dnt = good();
    dnt["displayName"] = json!("~DNT~ x");
    assert_eq!(
        fails(&one(a("AR_OK")), vec![dnt]),
        "Unresolved display name for AR_OK: ~DNT~ x"
    );
    assert_eq!(
        fails(&one(a("AR_OK")), vec![item("AR_OK", 4.0, ICON, &["x", "WIP"], None)]),
        "Forbidden catalog tag 'wip' on AR_OK"
    );
    assert_eq!(
        fails(&one(a("AR_OK")), vec![item("AR_OK", 3.0, ICON, &[], None)]),
        "Selected item must be rarity 4+, a resolved rarity-3 glider, or an explicit rarity-3 premium selection: AR_OK has rarity 3"
    );
    assert_eq!(
        fails(&one(a("AR_OK")), vec![item("AR_OK", 4.0, "", &[], None)]),
        "Missing catalog image path: AR_OK"
    );
    let mut clash = one(a("AR_X-1"));
    clash.selections.push(a("AR_X_1"));
    assert_eq!(
        fails(
            &clash,
            vec![
                item("AR_X-1", 4.0, ICON, &[], None),
                item("AR_X_1", 4.0, ICON, &[], None)
            ]
        ),
        "Duplicate generated SKU id: season19_rewardcache_armour_ar_x_1"
    );
    assert_eq!(
        fails(&one(a("AR_OK")), vec![item("AR_OK", 4.0, "/Game/x'y", &[], None)]),
        "Invalid cooked texture path: /Game/x'y"
    );
    let mut counted = one(a("AR_OK"));
    counted.expected = vec![(RewardCacheCategory::Armour, 2)];
    assert_eq!(fails(&counted, vec![good()]), "armour count 1 != expected 2");
    let mut featured = one(a("AR_OK"));
    featured.expected_featured = 1;
    assert_eq!(fails(&featured, vec![good()]), "Featured count 0 != expected 1");

    let plan = RewardCachePlan::from_json(&json!({
        "selections": [{"catalogId": "AR_OK", "category": "armour", "price": 1000, "featured": true, "allowRarity3": true, "displayName": "N"}],
        "armourFamilies": ["_A_"], "bannerFamilies": [], "expected": {"armour": 1}, "expectedFeatured": 1
    }))
    .unwrap();
    assert_eq!(plan.selections[0].display_name.as_deref(), Some("N"));
    assert!(plan.selections[0].allow_rarity3 && plan.selections[0].featured);
    assert_eq!(plan.expected, [(RewardCacheCategory::Armour, 1)]);
    assert!(
        RewardCachePlan::from_json(&json!({"selections": [{"catalogId": "x", "category": "hats", "price": 1}]}))
            .is_err()
    );
}

#[test]
fn ladyluck_from_source() {
    let dir = tempfile::tempdir().unwrap();
    let raw = dir.path().join("raw");
    let out = dir.path().join("out");
    let source = raw.join("ladyluck_store.source.json");
    assert!(
        generate_ladyluck_store(&raw, &out, true)
            .unwrap_err()
            .ends_with("Provide the store payload from your own installation (see docs/GENERATING_GAME_DATA.md).")
    );
    write(&source, "{}");
    assert_eq!(
        generate_ladyluck_store(&raw, &out, true).unwrap_err(),
        "ERROR: source must be a JSON array of store entries."
    );
    write(&source, r#"[{"items":[]},{"id":"b","prices":[]}]"#);
    assert_eq!(
        generate_ladyluck_store(&raw, &out, false).unwrap_err(),
        "store entries : 2\nmode          : DRY RUN (pass --apply to write)\n\nVALIDATION PROBLEMS (3):\n  entry 0: missing string \"id\"\n  entry 0 (undefined): missing prices[]\n  entry 1 (b): missing items[]\nERROR: refusing to proceed until the source entries are well-formed."
    );
    let many: Vec<Value> = (0..20).map(|i| json!({"id": format!("x{i}"), "prices": []})).collect();
    write(&source, &Value::Array(many).to_string());
    let err = generate_ladyluck_store(&raw, &out, true).unwrap_err();
    assert!(err.contains("VALIDATION PROBLEMS (20):") && err.contains("  ...and 5 more\nERROR: refusing"));
    assert!(!out.exists());
    write(
        &source,
        r#"[{"id":"a","images":{"standard":"x"},"prices":[{"currencyId":"c","price":1}],"items":[],"extra":1,"displayName":null}]"#,
    );
    assert_eq!(
        generate_ladyluck_store(&raw, &out, false).unwrap(),
        "store entries : 1\nmode          : DRY RUN (pass --apply to write)\nDry run complete. Nothing written."
    );
    assert_eq!(
        generate_ladyluck_store(&raw, &out, true).unwrap(),
        "store entries : 1\nmode          : APPLY\nWrote ladyluck_store.json (1 entries)"
    );
    let text = read(&out.join("ladyluck_store.json"));
    assert!(!text.ends_with('\n'));
    assert_eq!(
        service_kit::jsjson::stringify(&service_kit::jsjson::parse(&text).unwrap()),
        r#"[{"id":"a","displayName":"","displayDescription":"","displayPriority":0,"prices":[{"currencyId":"c","price":1}],"maxAllowed":0,"tags":[],"items":[],"duplicateInstancedItems":[]}]"#
    );
    generate_ladyluck_store(&raw, &out, true).unwrap();
    assert_eq!(backups(&out).len(), 1);
}

fn steel(price: i64) -> Value {
    json!([{"currencyId": "id_currency_marks_steel", "price": price}])
}

#[test]
fn ladyluck_from_live() {
    let dir = tempfile::tempdir().unwrap();
    let catalog = dir.path().join("catalog.jsonl");
    let live = dir.path().join("captures/ladyluckstore.json");
    let store = dir.path().join("backend/ladyluck_store.json");
    write(
        &catalog,
        &jsonl(&[
            item("DYE_KEEP", 2.0, "", &[], Some(true)),
            item("AR_ART", 4.0, ICON, &[], None),
            item("EM_NEW", 3.0, "", &[], None),
        ]),
    );
    assert!(
        enrich_ladyluck_store(&catalog, &live, &store)
            .unwrap_err()
            .starts_with("Live Lady Luck capture not found: ")
    );
    write(&live, "[]");
    assert!(
        enrich_ladyluck_store(&catalog, &live, &store)
            .unwrap_err()
            .starts_with("Lady Luck store data not found: ")
    );
    write(&store, "{}");
    assert_eq!(
        enrich_ladyluck_store(&catalog, &live, &store).unwrap_err(),
        "Live Lady Luck capture is not a SKU list"
    );
    let offers = json!([
        {"id": "keep", "displayName": "Keep", "prices": steel(10), "tags": ["ladyluckstore"], "items": [{"catalogId": "DYE_KEEP", "quantity": 2}], "duplicateInstancedItems": ["live"]},
        {"id": "art", "prices": [{"currencyId": "id_currency_marks_gilded", "price": 5}], "tags": ["ladyluckstore"], "items": [{"catalogId": "AR_ART"}]},
        {"id": "gone", "prices": steel(1), "tags": ["ladyluckstore"], "items": [{"catalogId": "AR_MISSING"}, {"catalogId": "X_MISSING"}]},
        {"id": "fresh", "displayPriority": 3, "prices": steel(7), "tags": ["ladyluckstore"], "items": [{"catalogId": "EM_NEW", "quantity": 1}]},
        "not an object"
    ]);
    write(&live, &format!("\u{feff}{offers}"));
    assert_eq!(
        enrich_ladyluck_store(&catalog, &live, &store).unwrap_err(),
        "Existing ladyluck_store.json is not a JSON array"
    );
    let existing = json!([
        {"id": "keep", "items": [{"catalogId": "DYE_KEEP", "quantity": 2, "instanced": false}], "duplicateInstancedItems": [], "images": {"standard": "old"}},
        {"id": "art", "items": [{"catalogId": "OTHER", "quantity": 1}], "images": {"feature": "f"}},
        {"id": "retired", "items": []}
    ]);
    write(&store, &existing.to_string());
    let report = enrich_ladyluck_store(&catalog, &live, &store).unwrap();
    assert_eq!(
        report,
        format!(
            "Wrote {}\nSKUs: 3 of 5 live; explicit catalog images: 1\nSteel offers: 2; Gilded offers: 1\nSkipped live SKUs: gone (AR_MISSING, X_MISSING not in the catalogue)\nRetired SKUs: retired",
            store.display()
        )
    );
    let text = read(&store);
    assert!(text.ends_with("]\n"));
    let skus = service_kit::jsjson::parse(&text).unwrap();
    assert_eq!(
        service_kit::jsjson::stringify(&skus[0]),
        r#"{"id":"keep","displayName":"Keep","prices":[{"currencyId":"id_currency_marks_steel","price":10}],"maxAllowed":null,"images":{"standard":"old"},"tags":["ladyluckstore"],"items":[{"catalogId":"DYE_KEEP","quantity":2,"instanced":false}],"duplicateInstancedItems":[]}"#
    );
    assert_eq!(
        service_kit::jsjson::stringify(&skus[1]),
        format!(
            r#"{{"id":"art","prices":[{{"currencyId":"id_currency_marks_gilded","price":5}}],"maxAllowed":null,"images":{{"feature":"f","standard":"Engine.Texture2D'{ICON}'"}},"tags":["ladyluckstore"],"items":[{{"catalogId":"AR_ART","instanced":true}}],"duplicateInstancedItems":[]}}"#
        )
    );
    assert_eq!(
        service_kit::jsjson::stringify(&skus[2]),
        r#"{"id":"fresh","displayPriority":3,"prices":[{"currencyId":"id_currency_marks_steel","price":7}],"maxAllowed":null,"tags":["ladyluckstore"],"items":[{"catalogId":"EM_NEW","quantity":1,"instanced":true}],"duplicateInstancedItems":[]}"#
    );

    let check = |offer: Value, want: &str| {
        write(&live, &json!([offer]).to_string());
        write(&store, "[]");
        assert_eq!(enrich_ladyluck_store(&catalog, &live, &store).unwrap_err(), want);
    };
    let base = json!({"id": "s", "prices": steel(1), "tags": ["ladyluckstore"], "items": []});
    let mut two = base.clone();
    two["prices"] = json!([{"currencyId": "id_currency_marks_steel", "price": 1}, {"currencyId": "id_currency_marks_steel", "price": 2}]);
    check(two, "Expected exactly one price for s");
    let mut coin = base.clone();
    coin["prices"] = json!([{"currencyId": "id_currency_platinum", "price": 1}]);
    check(coin, "Unsupported Lady Luck currency for s");
    let mut untagged = base.clone();
    untagged["tags"] = json!(["x"]);
    check(untagged, "Missing ladyluckstore tag on s");
    write(&live, &json!([base.clone(), base]).to_string());
    assert_eq!(
        enrich_ladyluck_store(&catalog, &live, &store).unwrap_err(),
        "Duplicate Lady Luck SKU id: s"
    );
    let mut bad_art =
        json!({"id": "t", "prices": steel(1), "tags": ["ladyluckstore"], "items": [{"catalogId": "BAD"}]});
    write(&catalog, &jsonl(&[item("BAD", 4.0, "/Game/a b", &[], None)]));
    write(&live, &json!([bad_art.take()]).to_string());
    assert_eq!(
        enrich_ladyluck_store(&catalog, &live, &store).unwrap_err(),
        "Invalid cooked texture path: /Game/a b"
    );
}

fn platinum_fixture(dir: &Path) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
    let catalog = dir.join("exports/catalog_1_14_7.jsonl");
    let images = dir.join("exports/store_item_images_1_14_7.jsonl");
    let captures = dir.join("captures");
    let data = dir.join("backend");
    write(
        &catalog,
        &jsonl(&[
            item("AR_CAP", 4.0, "", &[], Some(false)),
            item("DYE_RED", 1.0, "", &[], None),
            item("AR_HIDDEN", 4.0, "", &["Hidden"], None),
            item("AR_SOLD", 4.0, "", &[], None),
            item("EM_QUEST", 4.0, "", &[], None),
            item("WP_AC_ROW", 5.0, ICON, &[], Some(false)),
            item("QI_TONIC", 1.0, ICON, &[], Some(true)),
            item("GD_RAW", 3.0, "", &[], None),
            item("TOKEN_X", 1.0, "", &[], None),
            json!({"itemId": "LT_NONAME", "displayName": "MISSING STRING TABLE ENTRY"}),
            item("AR_TWICE", 4.0, "", &[], None),
        ]),
    );
    let row = |name: &str, standard: &str, feature: &str| json!({"rowName": name, "standardImage": standard, "featureImage": feature});
    write(
        &images,
        &jsonl(&[
            row("cap", "/Game/s", "None"),
            row("dye_offer", "None", "/Game/f"),
            row("real_money", "/Game/s", "None"),
            row("hidden", "/Game/s", "None"),
            row("sold", "/Game/s", "None"),
            row("quest", "/Game/s", "None"),
            row("quest_rumour_em_quest_2", "None", "None"),
            row("cosmetic_ent", "/Game/s", "None"),
            row("pass_ent", "/Game/s", "None"),
            row("noart", "None", "None"),
            row("rank1", "None", "None"),
            row("pass1", "/Game/s", "None"),
            row("pass_free", "/Game/s", "None"),
            row("sku_wp_ac_row", "None", "/Game/f"),
            row("single_qi_tonic", "/Game/s", "None"),
            row("sku_raw", "/Game/s", "None"),
            row("sku_noname", "/Game/s", "None"),
            row("bundle_x", "/Game/s", "None"),
            row("sku_bare", "None", "None"),
            row("contract_unknown", "/Game/s", "None"),
            row("sku_twice", "/Game/s", "None"),
            row("twice_offer", "/Game/s", "None"),
            json!({"rowName": "NewRow_1", "standardImage": "/Game/s"}),
        ]),
    );
    write(
        &data.join("reward_cache_store.json"),
        r#"[{"items":[{"catalogId":"AR_SOLD"}]}]"#,
    );
    write(&data.join("ladyluck_store.json"), "[]");
    let plat = |price: Value| json!([{"currencyId": "id_currency_platinum", "price": price}]);
    write(
        &captures.join("awakening_webstore.json"),
        &json!([
            {"id": "cap", "displayPriority": "12", "prices": plat(json!(900)), "items": [{"catalogId": "AR_CAP", "quantity": 1}], "skuProgression": {"x": 1}},
            {"id": "cap", "prices": plat(json!(1)), "items": [{"catalogId": "AR_CAP"}]},
            {"id": "unbaked", "prices": plat(json!(1)), "items": [{"catalogId": "AR_CAP"}]},
            {"id": "real_money", "platformOfferId": "p", "prices": plat(json!(1)), "items": [{"catalogId": "DYE_RED"}]},
            {"id": "hidden", "prices": plat(json!(1)), "items": [{"catalogId": "AR_HIDDEN"}]},
            {"id": "sold", "prices": plat(json!(1)), "items": [{"catalogId": "AR_SOLD"}]},
            {"id": "quest", "prices": plat(json!(1)), "items": [{"catalogId": "EM_QUEST"}]},
            {"id": "cosmetic_ent", "displayPriority": 0, "prices": plat(json!(300)), "entitlements": [{"name": "ent_ccfp_a", "duration": "2.5"}, {"name": "character_facepaint_b"}]},
            {"id": "pass_ent", "prices": plat(Value::Null), "entitlements": [{"name": "other"}]},
            {"id": "noart", "prices": plat(json!(1)), "items": [{"catalogId": "AR_TWICE"}]},
            {"id": "twice_offer", "prices": plat(json!(1)), "items": [{"catalogId": "AR_TWICE"}]},
            "x"
        ])
        .to_string(),
    );
    write(
        &captures.join("dyes.json"),
        &json!([{"id": "dye_offer", "displayName": "Red", "maxAllowed": 3, "prices": plat(json!(150)), "items": [{"catalogId": "DYE_RED", "quantity": 1}]}]).to_string(),
    );
    write(
        &captures.join("hp_level_skip_rank.json"),
        &json!([
            {"id": "rank1", "prices": plat(json!(200)), "skuProgression": {"levels": 1}},
            {"id": "rank2", "prices": plat(json!(200)), "skuProgression": {"levels": 1}},
            {"id": "pass1", "prices": plat(json!(200))}
        ])
        .to_string(),
    );
    write(
        &captures.join("huntpass_store.json"),
        &json!([
            {"id": "pass1", "displayName": "Pass", "prices": plat(json!(1500)), "entitlements": [{"name": "hp", "duration": 7}]},
            {"id": "pass_free", "prices": [], "entitlements": [{"name": "hp"}]}
        ])
        .to_string(),
    );
    (catalog, images, captures, data)
}

#[test]
fn platinum_store() {
    let dir = tempfile::tempdir().unwrap();
    let (catalog, images, captures, data) = platinum_fixture(dir.path());
    let report = generate_platinum_store(&catalog, &images, &captures, &data).unwrap();
    let text = read(&data.join("platinum_store.json"));
    assert!(text.ends_with("]\n"));
    let skus = service_kit::jsjson::parse(&text).unwrap();
    let ids: Vec<&str> = skus
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        [
            "dye_offer",
            "sku_raw",
            "cosmetic_ent",
            "twice_offer",
            "cap",
            "sku_wp_ac_row",
            "single_qi_tonic",
            "rank1",
            "pass_ent",
            "pass1"
        ]
    );
    assert_eq!(
        report,
        "platinum_store.json: 10 SKUs (7 captured price, 3 reconstructed price)"
    );
    let find = |id: &str| skus.as_array().unwrap().iter().find(|s| s["id"] == id).unwrap().clone();
    let compact = |id: &str| service_kit::jsjson::stringify(&find(id));
    assert_eq!(
        compact("cap"),
        r#"{"id":"cap","displayName":"cap","displayDescription":"","displayPriority":12,"prices":[{"currencyId":"id_currency_platinum","price":900}],"maxAllowed":null,"images":{},"tags":["webstore","skin_armour","platinum_store_captured"],"items":[{"catalogId":"AR_CAP","quantity":1,"instanced":true}],"duplicateInstancedItems":["AR_CAP"],"skuProgression":{"x":1}}"#
    );
    assert_eq!(
        compact("dye_offer"),
        r#"{"id":"dye_offer","displayName":"Red","displayDescription":"","displayPriority":1000,"prices":[{"currencyId":"id_currency_platinum","price":150}],"maxAllowed":3,"images":{},"tags":["webstore","dyes","dye_armour","platinum_store_captured"],"items":[{"catalogId":"DYE_RED","quantity":1,"instanced":false}],"duplicateInstancedItems":["DYE_RED"]}"#
    );
    assert_eq!(
        compact("cosmetic_ent"),
        r#"{"id":"cosmetic_ent","displayName":"cosmetic_ent","displayDescription":"","displayPriority":1000,"prices":[{"currencyId":"id_currency_platinum","price":300}],"maxAllowed":1,"images":{},"tags":["webstore","personality_character","platinum_store_captured"],"items":[],"duplicateInstancedItems":[],"entitlements":[{"name":"ent_ccfp_a","duration":2.5},{"name":"character_facepaint_b","duration":0}]}"#
    );
    assert_eq!(
        find("pass_ent")["tags"],
        json!(["huntpass_store", "platinum_store_captured"])
    );
    assert_eq!(find("pass_ent")["prices"][0]["price"], Value::Null);
    assert_eq!(
        compact("rank1"),
        r#"{"id":"rank1","displayName":"Hunt Pass Level Skip","displayDescription":"","displayPriority":1000,"prices":[{"currencyId":"id_currency_platinum","price":200}],"maxAllowed":null,"images":{},"tags":["hp_level_skip_rank","event","platinum_store_captured"],"items":[],"duplicateInstancedItems":[],"skuProgression":{"levels":1}}"#
    );
    assert_eq!(
        compact("pass1"),
        r#"{"id":"pass1","displayName":"Pass","displayDescription":"","displayPriority":1000,"prices":[{"currencyId":"id_currency_platinum","price":1500}],"maxAllowed":1,"images":{},"tags":["huntpass_store","event_pass","platinum_store_captured"],"items":[],"duplicateInstancedItems":[],"entitlements":[{"name":"hp","duration":7}]}"#
    );
    assert_eq!(
        compact("sku_wp_ac_row"),
        format!(
            r#"{{"id":"sku_wp_ac_row","displayName":"WP_AC_ROW name","displayDescription":"WP_AC_ROW text","displayPriority":1000,"prices":[{{"currencyId":"id_currency_platinum","price":1200}}],"maxAllowed":1,"images":{{"standard":"Engine.Texture2D'{ICON}'"}},"tags":["webstore","skin_weapon_ac","platinum_store_reconstructed","platinum_store_rarity_5"],"items":[{{"catalogId":"WP_AC_ROW","quantity":1,"instanced":true}}],"duplicateInstancedItems":["WP_AC_ROW"]}}"#
        )
    );
    let tonic = find("single_qi_tonic");
    assert_eq!(
        (&tonic["maxAllowed"], &tonic["prices"][0]["price"], &tonic["images"]),
        (&Value::Null, &json!(100), &json!({}))
    );
    assert_eq!(tonic["duplicateInstancedItems"], json!([]));
    assert_eq!(tonic["items"][0]["instanced"], false);
    // twice_offer claimed AR_TWICE first in its pass; the row that decodes to it is not offered again
    assert!(!ids.contains(&"sku_twice"));

    let raw = find("sku_raw");
    assert_eq!(
        (&raw["prices"][0]["price"], &raw["tags"][1]),
        (&json!(800), &json!("hunting_gliders"))
    );

    std::fs::remove_file(data.join("ladyluck_store.json")).unwrap();
    assert!(
        generate_platinum_store(&catalog, &images, &captures, &data)
            .unwrap_err()
            .starts_with("Lady Luck store data not found: ")
    );
    write(&data.join("ladyluck_store.json"), "[]");
    write(&captures.join("dyes.json"), "{");
    assert!(
        generate_platinum_store(&catalog, &images, &captures, &data)
            .unwrap_err()
            .starts_with("invalid JSON in ")
    );
    assert!(
        generate_platinum_store(&catalog, &dir.path().join("none.jsonl"), &captures, &data)
            .unwrap_err()
            .starts_with("Store item images not found: ")
    );
}

#[test]
fn platinum_export_choice() {
    let dir = tempfile::tempdir().unwrap();
    let new = dir.path().join("catalog_1_14_7.jsonl");
    let old = dir.path().join("catalog_1_12.jsonl");
    assert_eq!(resolve_platinum_export_file(dir.path(), "catalog"), old);
    write(&new, "");
    assert_eq!(resolve_platinum_export_file(dir.path(), "catalog"), new);
    write(&old, "");
    assert_eq!(resolve_platinum_export_file(dir.path(), "catalog"), old);
}

#[test]
fn storage_policy() {
    let dir = tempfile::tempdir().unwrap();
    let catalog = dir.path().join("catalog.jsonl");
    let data = dir.path().join("game-data");
    let text = jsonl(&[
        json!({"itemId": "CURRENCY_RAMS", "isStackable": true, "maxQuantity": 999999999, "tags": ["Currency", 5]}),
        json!({"itemId": "WP_SWORD_00", "maxQuantity": 1.0, "tags": []}),
        json!({"itemId": "AR_EXAMPLE", "isStackable": "true", "maxQuantity": "9"}),
        json!({"itemId": "AR_POLICY"}),
    ]);
    write(&catalog, &text);
    write(&data.join("raw/hunt.source.json"), r#"{"x":"WP_SWORD_00"}"#);
    write(&data.join("thing.example.json"), "AR_EXAMPLE");
    write(&data.join("inventory_storage_policy.json"), "AR_POLICY");
    write(&data.join("notes.txt"), "AR_POLICY");
    let seeds = TokenSource::Seeds(vec!["CURRENCY_RAMS".into(), "NOT_IN_CATALOG".into()]);
    let report = generate_inventory_storage_policy(&catalog, &seeds, &data).unwrap();
    let sha = {
        use sha2::Digest;
        sha2::Sha256::digest(text.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    };
    let target = data.join("inventory_storage_policy.json");
    assert_eq!(
        report,
        format!(
            "Wrote {}\nReferenced catalog ids: 2\nCatalog SHA-256: {sha}",
            target.display()
        )
    );
    let written = read(&target);
    assert!(written.ends_with("}\n"));
    assert_eq!(
        service_kit::jsjson::stringify(&service_kit::jsjson::parse(&written).unwrap()),
        format!(
            r#"{{"schema":1,"targetVersion":"1.14.7","catalogSha256":"{sha}","generatedAt":"2026-10-08","referencedCatalogIds":2,"sourceScope":["src/**/*.ts","src/**/*.json","game-data/*.json"],"entries":{{"CURRENCY_RAMS":{{"storage":"stacked","maxQuantity":999999999,"tags":["Currency"]}},"WP_SWORD_00":{{"storage":"instanced","maxQuantity":1,"tags":[]}}}}}}"#
        )
    );
    // The policy just written is not scanned on the next run
    let walk = TokenSource::Walk(dir.path().join("src"));
    write(&dir.path().join("src/a.ts"), "const x = 'AR_EXAMPLE';");
    generate_inventory_storage_policy(&catalog, &walk, &data).unwrap();
    let entries = read_json(&target)["entries"].clone();
    let keys: Vec<&String> = entries.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["AR_EXAMPLE", "WP_SWORD_00"]);
    assert_eq!(entries["AR_EXAMPLE"]["storage"], "instanced");
    assert_eq!(entries["AR_EXAMPLE"]["maxQuantity"], 0);
    assert!(
        generate_inventory_storage_policy(&dir.path().join("none"), &walk, &data)
            .unwrap_err()
            .starts_with("Catalog dump not found: ")
    );
}

#[test]
fn run_steps() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let paths = GeneratorPaths {
        raw_game_data: root.join("backend/game-data/raw"),
        backend_game_data: root.join("backend/game-data"),
        exports: root.join("Items_Analysis"),
        store_captures: root.join("captures"),
    };
    let mut plan = reward_plan();
    let inputs = VersionInputs {
        reward_cache: plan.clone(),
        storage_tokens: TokenSource::Seeds(vec!["EJ_R3".into()]),
    };
    let backend = &paths.backend_game_data;

    let err = run(BackendStep::ProgressionConfig, &paths, &inputs).unwrap_err();
    assert!(err.contains("progression_config.source.json"));
    write(
        &paths.raw_game_data.join("progression_config.source.json"),
        r#"{"payload":{"paths":[]}}"#,
    );
    assert!(
        run(BackendStep::ProgressionConfig, &paths, &inputs)
            .unwrap()
            .ends_with("(0 paths)")
    );
    assert!(backend.join("progression_config.json").is_file());

    let journey = paths.exports.join("slayers_path_1_14_7/player_journey_nodes.jsonl");
    write(&journey, &journey_rows());
    write(&backend.join("slayers_path.json"), r#"{"payload":{"nodes":{}}}"#);
    assert!(
        run(BackendStep::SlayersPath, &paths, &inputs)
            .unwrap()
            .contains("(3 nodes)")
    );
    let defs = run(BackendStep::SlayersPathDefinitions, &paths, &inputs).unwrap();
    assert!(defs.starts_with("source  : slayers_path_1_14_7 "));
    assert!(backend.join("slayers_path_definitions.json").is_file());

    // A failing first generator stops the step before the policy
    let err = run(BackendStep::RewardCache, &paths, &inputs).unwrap_err();
    assert!(err.starts_with("Catalog dump not found: "));
    assert!(!backend.join("inventory_storage_policy.json").exists());
    write(&paths.exports.join("catalog_1_14_7.jsonl"), &jsonl(&reward_catalog()));
    let report = run(BackendStep::RewardCache, &paths, &inputs).unwrap();
    // The store just written names all 13 ids, so the walk finds them besides the seed
    assert!(
        report.contains("\nPrices: ") && report.contains("\nReferenced catalog ids: 13\nCatalog SHA-256: "),
        "{report}"
    );
    assert!(backend.join("reward_cache_store.json").is_file());

    write(&paths.raw_game_data.join("ladyluck_store.source.json"), "[]");
    assert!(
        run(BackendStep::LadyLuckGenerate, &paths, &inputs)
            .unwrap()
            .ends_with("(0 entries)")
    );
    let skus = paths.store_captures.join("Store/Product/Skus");
    write(
        &skus.join("ladyluckstore.json"),
        &json!([{"id": "l", "prices": steel(1), "tags": ["ladyluckstore"], "items": [{"catalogId": "EJ_R3"}]}])
            .to_string(),
    );
    let report = run(BackendStep::LadyLuckEnrich, &paths, &inputs).unwrap();
    assert!(report.contains("SKUs: 1 of 1 live") && report.contains("Referenced catalog ids: "));

    write(&paths.exports.join("store_item_images_1_14_7.jsonl"), "");
    let report = run(BackendStep::PlatinumAndStoragePolicy, &paths, &inputs).unwrap();
    assert!(report.starts_with("platinum_store.json: 0 SKUs") && report.contains("\nWrote "));
    assert!(backend.join("platinum_store.json").is_file());

    plan.expected_featured = 9;
    let failing = VersionInputs {
        reward_cache: plan,
        ..inputs
    };
    std::fs::remove_file(backend.join("inventory_storage_policy.json")).unwrap();
    assert!(
        run(BackendStep::RewardCache, &paths, &failing)
            .unwrap_err()
            .starts_with("Featured count")
    );
    assert!(!backend.join("inventory_storage_policy.json").exists());
}
