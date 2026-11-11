// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The version's values (spec §12.2): manifest, tables, Director line, version data and route coverage.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use dainty_module_api::GameModule;
use dainty_module_api::manifest::{PatchSetCapability, Transport};
use rustic_isles::routes::all;
use rustic_isles::{Group, Service, Settings, When};
use rustic_isles_1_14_7::test_support::{area_handlers, synthetic, synthetic_document};
use rustic_isles_1_14_7::{DIRECTOR, TABLES, VersionData, module, spec};
use serde_json::{Value, json};

#[test]
fn manifest() {
    let module = module().unwrap();
    let manifest = module.manifest();
    manifest.validate().unwrap();
    assert_eq!(manifest.id.as_str(), "isles-1-14-7");
    assert_eq!(manifest.display_name, "Rustic Isles 1.14.7");
    assert_eq!(manifest.game_version.to_string(), "1.14.7");
    assert_eq!(manifest.invite_aliases, ["reforged"]);
    let caps = &manifest.capabilities;
    assert_eq!(caps.realms, cfg!(feature = "realms"));
    assert!(caps.network && caps.chat);
    assert_eq!(caps.patch_set, Some(PatchSetCapability::Accepts));
    assert!(caps.theme.is_none());
    assert!(!manifest.unverified);
    let ports: Vec<_> = manifest
        .ports
        .iter()
        .map(|p| (p.name.as_str(), p.transport, p.range.first, p.range.last))
        .collect();
    assert_eq!(
        ports,
        [
            ("backend", Transport::Tcp, 41000, 41000),
            ("director", Transport::Tcp, 41001, 41001),
            ("qos", Transport::Tcp, 41010, 41010),
            ("game", Transport::Udp, 8770, 8777),
        ]
    );
    let spec = spec().unwrap();
    let services: Vec<&str> = spec.services.iter().map(|s| s.port()).collect();
    assert_eq!(services, ["backend", "director"]);
}

#[cfg(not(feature = "realms"))]
#[test]
fn without_realms_the_module_is_dedicated_only() {
    let rifts =
        rustic_isles_1_14_7::rifts_with_core(&Settings::default(), std::sync::Arc::new(rustic_rifts_seam::NoCore));
    assert!(!rifts.loaded());
    assert!(!rustic_isles_1_14_7::capabilities().realms);
}

#[test]
fn tables() {
    let t = &TABLES;
    assert_eq!(t.target_changelist, 647_472);
    assert_eq!(t.build_id(), "647472_1.14.7_shipping");
    assert_eq!(t.active_huntpass_id, "season43");
    assert_eq!(t.recent_huntpass_seasons.len(), 8);
    assert_eq!(t.recent_huntpass_seasons[7], "season50");
    assert_eq!(t.patchnotes_release_version, "");
    let grants: Vec<_> = t
        .bounty_item_grant_data
        .iter()
        .map(|g| (g.item_id, g.amount, g.start, g.end))
        .collect();
    assert_eq!(
        grants,
        [
            (
                "TOKEN_BOUNTY_DRAFT_PREMIUM",
                1,
                "2026-01-01T00:00:00.000Z",
                "2027-12-31T00:00:00.000Z"
            ),
            (
                "CURRENCY_PRESTIGE",
                5,
                "2026-01-01T00:00:00.000Z",
                "2027-12-31T00:00:00.000Z"
            ),
        ]
    );
    assert_eq!(t.store_sections.len(), 9);
    assert_eq!(t.store_sections[0], "ladyluckstore");
    assert_eq!(t.store_sections[8], "fountain_daily_free_bundle");
    let coin = t.seasonal_coin.unwrap();
    assert_eq!((coin.catalog_id, coin.max_per_claim), ("CURRENCY_SEASONAL_COIN", 1000));
    assert_eq!(t.masked_journey_nodes[0].node, "Gauntlet_Unlock");
    assert_eq!(t.masked_journey_nodes[0].switch, "RUSTIC_ISLES_GAUNTLET");
    assert_eq!(t.starter_manifest.bootstrap_version, "starter-1.12-v5");
    assert_eq!(t.switch_default("qos_port"), "80");
    assert_eq!(t.switch_default("reforge"), "on");
    assert_eq!(t.switch_default("cooldown_persistence"), "on");
    assert_eq!(t.switch_default("nope"), "");
    assert_eq!(
        dainty_service_kit::jsjson::stringify(&(t.session_payload)("abc")),
        r#"{"error_code":null,"sessionid":"abc","sessiontoken":"abc","sessionToken":"abc"}"#
    );
}

#[test]
fn starter_manifest_wire_facts() {
    let manifest = &TABLES.starter_manifest;
    assert_eq!(manifest.bootstrap_version, "starter-1.12-v5");
    let stacked = manifest.stacked_item_ids;
    assert_eq!(stacked.len(), 24);
    assert_eq!(stacked[0], "TOKEN_DAILY_CHALLENGE_DRAFT");
    assert_eq!(stacked[13..15], ["DYE_GREEN04_DURABLE", "DYE_BROWN07_DURABLE"]);
    assert_eq!(stacked[15], "EJ_SYSTEM_TYPING");
    assert_eq!(stacked[23], "EJ_PLAYER_ZAI_SWEAT");
    let unique: std::collections::HashSet<_> = stacked.iter().collect();
    assert_eq!(unique.len(), 24);
    let wheel = manifest.quick_emoji_wheel;
    assert_eq!(wheel.len(), 9);
    assert_eq!(wheel[0], "EJ_PLAYER_BOSUN_JOY");
    assert_eq!(wheel[8], "");
    // Every wheel id is a stacked id (17 to 24, in the same order), so a new account holds what its wheel shows
    assert_eq!(wheel[..8], stacked[16..24]);
}

#[test]
fn director_line() {
    assert_eq!(
        DIRECTOR.launch_args(&Settings::default()).join(" "),
        r#"-EpicPortal -server -nullrhi -warp -nosound -RepDriverEnable -LogCmds="LogPhoenixCharacter VeryVerbose, LogOnline Verbose, LogLoad Off""#
    );
    assert_eq!(DIRECTOR.target_changelist, 647_472);
}

/// The synthetic document with `edit` applied, parsed; the error text.
fn refused(edit: impl FnOnce(&mut Value)) -> String {
    let mut doc = synthetic_document();
    edit(&mut doc);
    VersionData::from_json(&doc).unwrap_err()
}

#[test]
fn version_data_parses() {
    let data = synthetic();
    assert_eq!(data.starter_kit().instanced.len(), 6);
    assert_eq!(data.character_data().trigger_status, 2.0);
    assert_eq!(
        data.starter_kit().default_appearance,
        data.character_data().default_appearance
    );
    let legacy = data.legacy_starter().unwrap();
    assert_eq!(
        (legacy.default_dye.as_str(), legacy.banner.as_str()),
        ("TEST_DYE", "TEST_ITEM_5")
    );
    assert!(data.generator_inputs().is_none());
    let shown = format!("{data:?}");
    assert!(
        shown.contains("instanced: 6") && !shown.contains("TEST_ITEM"),
        "{shown}"
    );
}

#[test]
fn version_data_rules() {
    let cases: Vec<(String, &str)> = vec![
        (refused(|d| d["format"] = json!(2)), "format must be 1"),
        (
            refused(|d| d["gameVersion"] = json!("1.14.6")),
            "gameVersion 1.14.6 is not 1.14.7",
        ),
        (
            refused(|d| {
                d.as_object_mut().unwrap().remove("starterKit");
            }),
            "starterKit is missing",
        ),
        (
            refused(|d| d["characterData"]["triggerStatus"] = json!("x")),
            "characterData.triggerStatus must be a finite number",
        ),
        (
            refused(|d| d["starterKit"]["instanced"] = json!(["A", "A"])),
            "starterKit.instanced repeats A",
        ),
        (
            refused(|d| d["starterKit"]["equipment"] = json!([["weapon", "NOPE"]])),
            "starterKit.equipment NOPE is not in starterKit.instanced",
        ),
        (
            refused(|d| d["starterKit"]["flask"] = json!("NOPE")),
            "starterKit.flask NOPE is not in starterKit.instanced",
        ),
        (
            refused(|d| d["starterKit"]["banner"] = json!("NOPE")),
            "starterKit.banner NOPE is not in starterKit.instanced",
        ),
        (
            refused(|d| d["starterKit"]["glider"] = json!("NOPE")),
            "starterKit.glider NOPE is not in starterKit.instanced",
        ),
        (
            refused(|d| d["starterKit"]["defaultWeaponParts"] = json!(["NOPE"])),
            "starterKit.defaultWeaponParts NOPE is not in starterKit.instanced",
        ),
        (
            refused(|d| d["starterKit"]["equipment"] = json!([["", "TEST_ITEM_1"]])),
            "starterKit.equipment has an empty slot key",
        ),
        (
            refused(|d| d["starterKit"]["equipment"] = json!([["w", "TEST_ITEM_1"], ["w", "TEST_ITEM_2"]])),
            "starterKit.equipment repeats the slot w",
        ),
        (
            refused(|d| d["starterKit"]["defaultQuickChats"] = json!(["a"])),
            "starterKit.defaultQuickChats must be exactly nine texts",
        ),
        (
            refused(|d| d["starterKit"]["wallet"] = json!([["C", "x"]])),
            "starterKit.wallet must be an array of [catalog id, finite number] pairs",
        ),
        (refused(|d| d["typo"] = json!(1)), "unknown member typo"),
        (
            refused(|d| d["starterManifest"] = json!({})),
            "unknown member starterManifest",
        ),
    ];
    for (got, reason) in cases {
        assert_eq!(got, format!("version_data.json: {reason}"));
    }
    let mut doc = synthetic_document();
    doc["rewardCachePlan"] = json!({
        "selections": [], "armourFamilies": [], "bannerFamilies": [], "expectedFeatured": 0
    });
    let only_plan = VersionData::from_json(&doc).unwrap();
    assert!(only_plan.generator_inputs().is_none());
    doc["storageTokenSeeds"] = json!(["A"]);
    assert!(VersionData::from_json(&doc).unwrap().generator_inputs().is_some());
}

#[test]
fn every_isles_row_is_registered_once() {
    let areas = area_handlers();
    areas.check().unwrap();
    let mut isles = 0;
    for row in all() {
        let registered = areas.contains(row.service, row.method, row.path);
        let auth = row.group == Group::Auth;
        let expected = row.service == Service::Backend && row.when != When::Rifts && !auth;
        assert_eq!(registered, expected, "{}", row.name());
        if expected || auth {
            isles += 1;
        }
    }
    assert_eq!(isles, 227);
    // Each area's own list is disjoint from the others' and from auth's
    let lists: Vec<(&str, Vec<(rustic_isles::Method, &str)>)> = vec![
        ("gamedata", rustic_isles_gamedata::routes().to_vec()),
        ("characters", rustic_isles_characters::routes().to_vec()),
        ("inventory", rustic_isles_economy::inventory::routes().to_vec()),
        ("progression", rustic_isles_economy::progression::routes().to_vec()),
        ("bounties", rustic_isles_economy::bounties::routes().to_vec()),
        (
            "auth",
            all()
                .iter()
                .filter(|r| r.group == Group::Auth)
                .map(|r| (r.method, r.path))
                .collect(),
        ),
    ];
    let mut seen = std::collections::HashMap::new();
    for (area, list) in &lists {
        for pair in list {
            if let Some(other) = seen.insert(*pair, *area) {
                panic!("{pair:?} is both {other}'s and {area}'s");
            }
        }
    }
    let counts: Vec<usize> = lists.iter().map(|(_, l)| l.len()).collect();
    assert_eq!(counts, [29, 8, 8, 25, 17, 49]);
    // Rifts' rows: 59 on the backend (the admission key's has no Node route) and one on the Director
    let rifts: Vec<_> = all().iter().filter(|r| r.when == When::Rifts).collect();
    assert_eq!(rifts.len(), 60);
    assert_eq!(
        all()
            .iter()
            .filter(|r| r.service == Service::Director && r.when != When::Rifts)
            .count(),
        4
    );
}
