// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! A synthetic `version_data.json`: made-up ids that satisfy §4.3's rules, never the real values.

use serde_json::{Value, json};

use crate::VersionData;

/// The synthetic document, which tests change one member at a time.
pub fn synthetic_document() -> Value {
    let instanced: Vec<String> = (1..=6).map(|n| format!("TEST_ITEM_{n}")).collect();
    json!({
        "format": 1,
        "gameVersion": "1.14.7",
        "characterData": {
            "fill": [["TestFlags", {}], ["TestCount", 0]],
            "defaultAppearance": { "Data": [1, 2], "StringData": ["a"] },
            "forcedCharacterFlags": [["TEST_FLAG_A", "1"]],
            "forcedLoginFlag": ["TEST_LOGIN_FLAG", "1"],
            "dojoSeriesKey": "SERIE_test",
            "dojoSeries": { "Nodes": [] },
            "dojoNodeId": "TestDojoNode",
            "triggerSeriesKey": "SERIE_trigger",
            "triggerNode": "TestTriggerNode",
            "triggerStatus": 2
        },
        "starterKit": {
            "instanced": instanced,
            "defaultWeaponParts": ["TEST_ITEM_1", "TEST_ITEM_2"],
            "equipment": [["weapon", "TEST_ITEM_1"], ["head", "TEST_ITEM_3"]],
            "instanceData": [["TEST_ITEM_1", { "level": 1 }]],
            "flask": "TEST_ITEM_4",
            "banner": "TEST_ITEM_5",
            "glider": "TEST_ITEM_6",
            "bannerCustomization": { "pattern": "test" },
            "introEmote": "TEST_EMOTE",
            "flare": "TEST_FLARE",
            "defaultQuickChats": ["q1", "q2", "q3", "q4", "q5", "q6", "q7", "q8", "q9"],
            "wallet": [["TEST_CURRENCY", 100]]
        },
        "legacyStarter": { "defaultDye": "TEST_DYE", "defaultSigil": "TEST_SIGIL" }
    })
}

/// [`synthetic_document`] split as a release ships it: the facts and the design (rulings V-1, V-2).
pub fn synthetic_split() -> (Value, Value) {
    let mut facts = synthetic_document();
    let design = json!({
        "format": 1,
        "gameVersion": "1.14.7",
        "characterData": { "defaultAppearance": facts["characterData"]["defaultAppearance"].take() },
        "starterKit": { "wallet": facts["starterKit"]["wallet"].take() },
        "legacyStarter": facts["legacyStarter"].take(),
    });
    if let Some(object) = facts.as_object_mut() {
        object.remove("legacyStarter");
    }
    for (member, key) in [("characterData", "defaultAppearance"), ("starterKit", "wallet")] {
        if let Some(object) = facts[member].as_object_mut() {
            object.remove(key);
        }
    }
    (facts, design)
}

/// The areas' backend handlers as the composer registers them (auth's are core's).
pub fn area_handlers() -> rustic_isles::Handlers {
    crate::composer::handlers(Default::default(), Default::default())
}

/// [`synthetic_document`] parsed.
pub fn synthetic() -> VersionData {
    match VersionData::from_json(&synthetic_document()) {
        Ok(data) => data,
        // The document above is fixed; a failure here is a bug in this file
        Err(error) => unreachable!("the synthetic version data parses: {error}"),
    }
}
