// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The pure rules: normalise, the progression guard, triggers, fresh onboarding and the starter manifest
//! (spec §13.2).

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use common::{TABLES, character_data, kit, parse};
use rustic_isles_characters::data::{
    FACE_COMPLETE, guard_progression, is_fresh, normalize, process_triggers, read_form,
};
use rustic_isles_characters::starter::{build_manifest, is_instance_id, repair, with_default_chat_wheels};
use serde_json::{Value, json};

const NOW: i64 = 1_792_000_000;
const CL: u32 = 647_472;

fn norm(text: &str) -> Value {
    parse(&normalize(text, &character_data(), CL, NOW).unwrap())
}

fn inner(value: &Value, key: &str) -> Value {
    parse(value[key].as_str().unwrap())
}

#[test]
fn normalise_an_empty_blob() {
    let value = norm("{}");
    let keys: Vec<&str> = value.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "RecentPlayers",
            "AppearanceData",
            "PlayerAccountProgressStep",
            "CharacterFlagData",
            "RepairData",
            "LoginTime",
            "LastLoginTime",
            "LastChangelist",
            "LoginFlagData"
        ]
    );
    assert_eq!(value["PlayerAccountProgressStep"], "EnteredRamsgate");
    assert_eq!(value["LastChangelist"], "647472");
    assert_eq!(value["LoginTime"], "2026.10.14-17.46.40");
    let flags = inner(&value, "CharacterFlagData");
    assert_eq!(
        flags,
        json!({"Flags": [{"FlagKey": "Flag_A", "FlagValue": "True"}, {"FlagKey": "Flag_B", "FlagValue": "True"}]})
    );
    assert_eq!(
        inner(&value, "LoginFlagData"),
        json!({"Flags": [{"FlagKey": "Login_Seen", "FlagValue": "true"}]})
    );
    let look = inner(&value, "AppearanceData");
    assert_eq!(look["CreationState"], FACE_COMPLETE);
    assert_eq!(look["Data"], json!([7, 8]));
    assert_eq!(norm(""), value);

    // A fill list without them: they are appended in rule order
    let mut short = character_data();
    short.fill.truncate(1);
    let value = parse(&normalize("{}", &short, CL, NOW).unwrap());
    let keys: Vec<&str> = value.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "RecentPlayers",
            "LoginTime",
            "LastLoginTime",
            "LastChangelist",
            "PlayerAccountProgressStep",
            "AppearanceData",
            "CharacterFlagData",
            "LoginFlagData"
        ]
    );
    assert_eq!(inner(&value, "AppearanceData")["AssetReferences"], json!([]));
}

#[test]
fn normalise_is_idempotent() {
    let data = character_data();
    for text in ["{}", "{\"PlayerAccountProgressStep\":\"Final\",\"Extra\":[1]}"] {
        let once = normalize(text, &data, CL, NOW).unwrap();
        assert_eq!(normalize(&once, &data, CL, NOW + 86_400).unwrap(), once);
    }
}

#[test]
fn normalise_refuses_what_is_not_an_object() {
    let data = character_data();
    assert_eq!(
        normalize("[1]", &data, CL, NOW).unwrap_err().to_string(),
        "character data is not a JSON object"
    );
    assert!(
        normalize("not json", &data, CL, NOW)
            .unwrap_err()
            .to_string()
            .starts_with("character data: ")
    );
}

#[test]
fn normalise_keeps_and_heals() {
    let look = json!({"Data": [1], "StringData": [{"Key": "K", "Value": "V"}], "Mine": true}).to_string();
    let blob = json!({
        "PlayerAccountProgressStep": "FinishedFirstHunt",
        "LoginTime": "2020.01.01-00.00.00",
        "CharacterFlagData": "{\"Flags\":[{\"FlagKey\":\"Flag_A\",\"FlagValue\":\"False\"}],\"Other\":1}",
        "AppearanceData": look,
        "Extra": {"x": 1},
    });
    let value = norm(&blob.to_string());
    assert_eq!(value["PlayerAccountProgressStep"], "FinishedFirstHunt");
    assert_eq!(value["LoginTime"], "2020.01.01-00.00.00");
    assert_eq!(value["LastLoginTime"], "2026.10.14-17.46.40");
    assert_eq!(value["Extra"], json!({"x": 1}));
    let flags = inner(&value, "CharacterFlagData");
    assert_eq!(
        flags,
        json!({"Flags": [{"FlagKey": "Flag_A", "FlagValue": "False"}, {"FlagKey": "Flag_B", "FlagValue": "True"}], "Other": 1})
    );
    let look = inner(&value, "AppearanceData");
    assert_eq!(look["Data"], json!([1]));
    assert_eq!(look["StringData"], json!([{"Key": "K", "Value": "V"}]));
    assert_eq!(look["CreationState"], FACE_COMPLETE);
    assert_eq!(look["Mine"], true);
    assert_eq!(look["AssetReferences"], json!([]));

    let default_strings = json!([{"Key": "Hair", "Value": "Short"}]);
    for legacy in [
        json!({"Key": "K", "CustomizationName": "c"}),
        json!({"Value": "v"}),
        json!(0),
        json!("s"),
    ] {
        let look = json!({"Data": [1], "StringData": [{"Key": "K"}, legacy]}).to_string();
        let value = norm(&json!({"AppearanceData": look}).to_string());
        assert_eq!(
            inner(&value, "AppearanceData")["StringData"],
            default_strings,
            "{legacy}"
        );
    }
    let look = json!({"StringData": [null, false, ""]}).to_string();
    let value = norm(&json!({"AppearanceData": look}).to_string());
    assert_eq!(inner(&value, "AppearanceData")["StringData"], json!([null, false, ""]));
    // An appearance that is no object is the default's
    let value = norm("{\"AppearanceData\":5}");
    assert_eq!(inner(&value, "AppearanceData")["Data"], json!([7, 8]));
}

#[test]
fn the_progression_guard() {
    let existing = "{\"PlayerAccountProgressStep\":\"SavedCharacter\"}";
    let step = |incoming: &str| parse(&guard_progression(incoming, existing))["PlayerAccountProgressStep"].clone();
    assert_eq!(step("{\"PlayerAccountProgressStep\":\"New\"}"), "SavedCharacter");
    assert_eq!(
        step("{\"PlayerAccountProgressStep\":\"EnteredRamsgate\"}"),
        "EnteredRamsgate"
    );
    assert_eq!(
        step("{\"PlayerAccountProgressStep\":\"FinishedSecondHunt\"}"),
        "EnteredRamsgate"
    );
    assert_eq!(step("{\"PlayerAccountProgressStep\":\"Bogus\"}"), "SavedCharacter");
    assert_eq!(step("{}"), "SavedCharacter");
    assert_eq!(guard_progression("not json", existing), "not json");
    assert_eq!(guard_progression("{\"a\":1}", "[1]"), "{\"a\":1}");
}

#[test]
fn triggers() {
    let data = character_data();
    let series = "{\"Series\":\"DOJO_TEST\",\"Steps\":[1,2]}";
    let run = |blob: Value| process_triggers(&blob.to_string(), &data, CL, NOW);
    let fired = parse(&run(json!({"SERIE_trigger": "{\"Node_T\":{\"Status\":3}}"})).unwrap());
    assert_eq!(fired["SERIE_dojo"], series);
    let kept = parse(&run(json!({"SERIE_trigger": "{\"Node_T\":{\"Status\":3}}", "SERIE_dojo": "mine"})).unwrap());
    assert_eq!(kept["SERIE_dojo"], "mine");
    let not_yet = parse(&run(json!({"SERIE_trigger": "{\"Node_T\":{\"Status\":2}}"})).unwrap());
    assert_eq!(not_yet["SERIE_dojo"], Value::Null);
    assert!(run(json!({"SERIE_trigger": "nope"})).is_err());
    assert!(run(json!({"SERIE_trigger": {"Node_T": {"Status": 3}}})).is_err());
    assert!(run(json!({"SERIE_trigger": 5})).is_ok());
    assert!(run(json!({"SERIE_trigger": null})).is_ok());

    let (text, added) = read_form("{}", &data, CL, NOW, true).unwrap();
    assert!(added);
    assert_eq!(parse(&text)["SERIE_dojo"], series);
    let (_, added) = read_form(&text, &data, CL, NOW, true).unwrap();
    assert!(!added);
    let (text, added) = read_form("{}", &data, CL, NOW, false).unwrap();
    assert!(!added && !text.contains("SERIE_dojo"));
}

#[test]
fn fresh_onboarding() {
    assert!(is_fresh([]));
    assert!(is_fresh(["{}", "garbage"]));
    let saved = normalize(
        "{\"PlayerAccountProgressStep\":\"SavedCharacter\"}",
        &character_data(),
        CL,
        NOW,
    )
    .unwrap();
    assert!(!is_fresh(["{}", saved.as_str()]));
    let incomplete = json!({
        "PlayerAccountProgressStep": "Final",
        "AppearanceData": "{\"CreationState\":\"EArchonCharacterCreationState::None\"}",
    });
    assert!(is_fresh([incomplete.to_string().as_str()]));
    let object_look =
        json!({"PlayerAccountProgressStep": "SavedCharacter", "AppearanceData": {"CreationState": FACE_COMPLETE}});
    assert!(!is_fresh([object_look.to_string().as_str()]));
}

#[test]
fn the_manifest() {
    let kit = kit();
    let manifest = build_manifest(&kit, &TABLES).unwrap();
    assert_eq!(manifest.instanced_items.len(), kit.instanced.len());
    let mut ids = std::collections::HashMap::new();
    for (item, catalog) in manifest.instanced_items.iter().zip(&kit.instanced) {
        assert_eq!(&item["catalogId"], catalog);
        let id = item["instanceId"].as_str().unwrap();
        assert!(is_instance_id(id), "{id}");
        assert_eq!((&item["itemData"], &item["updateVersion"]), (&Value::Null, &json!(0)));
        ids.insert(catalog.clone(), id.to_owned());
    }
    let keys: Vec<&str> = manifest.instanced_items[0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, ["catalogId", "instanceId", "itemData", "updateVersion"]);
    assert_eq!(
        manifest.stacked_items,
        vec![
            json!({"catalogId": "STACK_A", "quantity": 1}),
            json!({"catalogId": "STACK_B", "quantity": 1})
        ]
    );
    let slot = &manifest.loadout_slot;
    let slot_keys: Vec<&str> = slot.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(
        slot_keys,
        [
            "weapon",
            "helmet",
            "chest",
            "lantern",
            "player_role",
            "subweapon",
            "appearance",
            "flask",
            "quick_items",
            "slot_index",
            "update_version",
            "custom_name",
            "persistent"
        ]
    );
    for (key, catalog) in &kit.equipment {
        assert_eq!(slot[key]["item_id"], *catalog);
        assert_eq!(slot[key]["instance_id"], ids[catalog]);
    }
    assert_eq!(slot["weapon"]["instance_data"], "{\"Level\":1}");
    assert_eq!(slot["chest"]["instance_data"], Value::Null);
    assert_eq!(slot["flask"], ids["FLASK_TEST"]);
    let persistent = &manifest.persistent;
    let persistent_keys: Vec<&str> = persistent.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(
        persistent_keys,
        [
            "manual_emotes",
            "intro_emote",
            "banner",
            "bannerCustomization",
            "flare",
            "title",
            "head_accessory",
            "back_accessory",
            "pet",
            "glider",
            "update_version",
            "quick_chats",
            "emojis",
            "quick_curiosities_items",
            "quickwheel"
        ]
    );
    assert_eq!(persistent["banner"], ids["BANNER_TEST"]);
    assert_eq!(persistent["glider"], ids["GLIDER_TEST"]);
    assert_eq!(persistent["bannerCustomization"], "{\"Color\":\"Red\"}");
    assert_eq!(
        persistent["quick_curiosities_items"][7],
        json!({"item_index": 7, "item_id": "", "instance_id": ""})
    );
    assert_eq!(slot["persistent"], *persistent);

    let other = build_manifest(&kit, &TABLES).unwrap();
    let theirs: Vec<&Value> = other.instanced_items.iter().map(|i| &i["instanceId"]).collect();
    assert!(
        manifest
            .instanced_items
            .iter()
            .all(|i| !theirs.contains(&&i["instanceId"]))
    );

    let mut broken = kit.clone();
    broken.banner = "NOT_THERE".into();
    assert_eq!(
        build_manifest(&broken, &TABLES).unwrap_err().to_string(),
        "stored data: Missing inventory instance ID for starter banner NOT_THERE"
    );
    let mut broken = kit.clone();
    broken.equipment.push(("legs".into(), "LEGS_NOT_THERE".into()));
    assert!(
        build_manifest(&broken, &TABLES)
            .unwrap_err()
            .to_string()
            .ends_with("Missing inventory instance ID for starter loadout item LEGS_NOT_THERE")
    );
}

#[test]
fn chat_wheels() {
    let kit = kit();
    let filled = with_default_chat_wheels(json!({"quick_chats": ["", 5], "emojis": ["a"]}), &kit, &TABLES);
    assert_eq!(filled["quick_chats"][8], "chat_9");
    assert_eq!(filled["emojis"], json!(["a"]));
    assert_eq!(with_default_chat_wheels(json!([1]), &kit, &TABLES), json!([1]));
    let filled = with_default_chat_wheels(json!({}), &kit, &TABLES);
    assert_eq!(filled["emojis"][0], "e1");
    let untouched = with_default_chat_wheels(json!({}), &kit, &common::NO_EMOJI_TABLES);
    assert_eq!(untouched, json!({}));
}

#[test]
fn repair_keeps_valid_items() {
    let kit = kit();
    let items = vec![
        json!({"catalogId": "HELM_TEST", "instanceId": "KEEPTHISIDAAAAAAAAAAAAAAAA", "itemData": {"x": 1}}),
        json!({"catalogId": "HELM_TEST", "instanceId": "SECONDHELMAAAAAAAAAAAAAAAA"}),
        json!("junk"),
    ];
    let source =
        json!({"weapon": {"item_id": "WP_TEST_00", "instance_id": "FROMTHESLOTAAAAAAAAAAAAAAA"}, "flask": "bad"});
    let repaired = repair(items, Some(&source), &kit);
    assert!(repaired.changed);
    assert_eq!(repaired.ids["HELM_TEST"], "KEEPTHISIDAAAAAAAAAAAAAAAA");
    assert_eq!(repaired.ids["WP_TEST_00"], "FROMTHESLOTAAAAAAAAAAAAAAA");
    assert!(is_instance_id(&repaired.ids["FLASK_TEST"]));
    assert_eq!(repaired.items[0]["itemData"], json!({"x": 1}));
    assert!(repaired.items.iter().all(Value::is_object));
    for catalog in &kit.instanced {
        assert!(repaired.ids.contains_key(catalog), "{catalog}");
    }
    let again = repair(repaired.items.clone(), None, &kit);
    assert!(!again.changed);
    assert_eq!(again.ids, repaired.ids);
}
