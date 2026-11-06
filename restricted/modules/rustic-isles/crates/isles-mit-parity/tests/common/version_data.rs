//! `version_data.json` as restricted holds it: 1.14.7's character data and starter kit (`starter.rs`), the legacy
//! starter's banner defaults, the Reward Cache plan (`reward_cache.rs`) and `src_tokens.txt`, in the MIT document's
//! shape (spec-rustic-isles-1-14-7 §4.3), all from the retired isles-core and isles-db (RD27). The values isles-core
//! kept inline in its normalise and bootstrap code (the series fill, the trigger node, the banner customization) are
//! written out here. `tests/version_data.rs` keeps the copy the host ships equal to it.
#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)] // test helpers

use serde_json::{Value, json};

use super::reward_cache::reward_cache_plan_document;
use super::starter::{
    DEFAULT_CHARACTER_FLAGS, DEFAULT_QUICK_CHATS, LOADOUT_EQUIPMENT, NO_RECENT_PLAYERS, STARTER_BANNER_CATALOG_ID,
    STARTER_DEFAULT_WEAPON_PART_CATALOG_IDS, STARTER_FLASK_CATALOG_ID, STARTER_GLIDER_CATALOG_ID,
    STARTER_INSTANCED_CATALOG_IDS, STARTER_WALLET, STARTER_WEAPON_CATALOG_ID, TUTORIAL_SLATE_FLAGS,
    build_instance_data, default_appearance, dojo_series_data,
};

// isles-db's (`transforms.rs`): what a migrated banner without a dye or sigil gets
const DEFAULT_BANNER_DYE_ID: &str = "DYE_DEFAULT";
const DEFAULT_BANNER_SIGIL_ID: &str = "BN_SIGIL_BEGINNER_00";
const SRC_TOKENS: &str = include_str!("src_tokens.txt");

fn parsed(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

pub fn document() -> Value {
    let mut forced: Vec<Value> = TUTORIAL_SLATE_FLAGS.iter().map(|f| json!([f, "True"])).collect();
    forced.push(json!(["CharacterFlag_CityGatherableTutorialShown", "True"]));
    let equipment: Vec<Value> = LOADOUT_EQUIPMENT.iter().map(|(slot, id)| json!([slot, id])).collect();
    // Whole amounts, as Node's STARTER_WALLET writes them
    let wallet: Vec<Value> = STARTER_WALLET
        .iter()
        .map(|(currency, amount)| json!([currency, *amount as i64]))
        .collect();
    json!({
        "format": 1,
        "gameVersion": "1.14.7",
        "characterData": {
            "fill": [
                ["RecentPlayers", NO_RECENT_PLAYERS],
                ["AppearanceData", default_appearance().to_string()],
                ["PlayerAccountProgressStep", "New"],
                ["CharacterFlagData", DEFAULT_CHARACTER_FLAGS],
                ["PlayerDataRepair", "{\"Data\":[]}"],
                ["SERIE_cr20_pjm_quests", "{\"ID\":\"CR20_PJM_Quests\"}"],
                ["SERIE_d24_a_main_quests", "{\"ID\":\"D24_A_MAIN_QUESTS\"}"],
                ["SERIE_d24_b_side_quests", "{\"ID\":\"D24_B_SIDE_QUESTS\"}"],
                ["SERIE_d24_d_tutorials", "{\"ID\":\"D24_D_TUTORIALS\"}"]
            ],
            "defaultAppearance": default_appearance(),
            "forcedCharacterFlags": forced,
            "forcedLoginFlag": ["WatchedNewRamsgateCinematic", "true"],
            "dojoSeriesKey": "SERIE_dojo",
            "dojoSeries": parsed(&dojo_series_data()),
            "dojoNodeId": "Dojo",
            "triggerSeriesKey": "SERIE_cr19_series_1_ftue",
            "triggerNode": "929A333B40E413C41E47B0A425EC3349",
            "triggerStatus": 3
        },
        "starterKit": {
            "instanced": STARTER_INSTANCED_CATALOG_IDS,
            "defaultWeaponParts": STARTER_DEFAULT_WEAPON_PART_CATALOG_IDS,
            "equipment": equipment,
            "instanceData": [[STARTER_WEAPON_CATALOG_ID, parsed(&build_instance_data())]],
            "flask": STARTER_FLASK_CATALOG_ID,
            "banner": STARTER_BANNER_CATALOG_ID,
            "glider": STARTER_GLIDER_CATALOG_ID,
            "bannerCustomization": {
                "BannerMeshItemID": "BNC_MESH_BEGINNER_00", "FabricMaterialItemID": "BNC_FABRIC_BEGINNER_00",
                "SigilTextureItemID": "BNC_SIGIL_BEGINNER_00", "PlantVFXItemID": "", "PersistantStandardVFXItemID": "",
                "AnimationItemID": "BNC_ANIMATION_BEGINNER_00", "BackgroundColourItemID": "DYE_GREEN04_DURABLE",
                "BorderColourItemID": "DYE_BROWN07_DURABLE", "SigilColourItemID": "DYE_BROWN07_DURABLE",
                "BackgroundSheenType": 0, "BorderSheenType": 0, "SigilSheenType": 0
            },
            "introEmote": "EM_INTRO_BEGINNER_01",
            "flare": "QI_BASIC_FLARE_DURABLE",
            "defaultQuickChats": DEFAULT_QUICK_CHATS,
            "wallet": wallet
        },
        "legacyStarter": {
            "defaultDye": DEFAULT_BANNER_DYE_ID,
            "defaultSigil": DEFAULT_BANNER_SIGIL_ID
        },
        "rewardCachePlan": reward_cache_plan_document(),
        "storageTokenSeeds": SRC_TOKENS.lines().collect::<Vec<_>>()
    })
}

pub fn restricted() -> rustic_isles_1_14_7::VersionData {
    rustic_isles_1_14_7::VersionData::from_json(&document()).unwrap()
}
