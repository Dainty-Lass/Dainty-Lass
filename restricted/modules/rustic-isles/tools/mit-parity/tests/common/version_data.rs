//! `version_data.json` as restricted holds it: the values of isles-core's `areas/characters/data.rs` and `starter.rs`
//! (1.14.7's character data and starter kit) in the MIT document's shape (spec-rustic-isles-1-14-7 §4.3).
#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)] // test helpers

use serde_json::{Value, json};

fn default_appearance() -> Value {
    json!({
        "CreationState": "EArchonCharacterCreationState::FaceComplete",
        "Data": [{ "SkeletalMeshComponentName": "Head Slot", "MorphData": [] }],
        "AssetReferences": [],
        "StringData": [
            { "Key": "BodyType", "Data": "Feminine" },
            { "Key": "Hair", "Data": "Hair12" },
            { "Key": "SkinName", "Data": "Tan" },
            { "Key": "SkinValue", "Data": "(R=0.610496,G=0.417885,B=0.254152,A=1.000000)" },
            { "Key": "Hair_Color", "Data": "(R=0.196843,G=0.042531,B=0.019093,A=1.000000)" },
            { "Key": "Beard", "Data": "NoBeard" },
            { "Key": "Facepaint", "Data": "NoFacepaint" },
            { "Key": "Makeup", "Data": "NoMakeup" }
        ]
    })
}

fn dojo_series() -> Value {
    json!({
        "ID": "Dojo",
        "Status": 0,
        "62B91BD94558409B4F7352B5B96F3ED7": {
            "Status": 0,
            "6CA2C43B46334BC06F73DEB5F2BFFEC1": { "Status": 0, "CurrentAmount": 0, "LastUpdateAmount": 0 },
            "3A7241AA43743647D3C1E39E8976E4F3": { "Status": 0, "CurrentAmount": 0, "LastUpdateAmount": 0 }
        },
        "816CBFD94D16EDA252BD1D8461209568": { "Status": 1 },
        "B152371947599B3C2D55BE9B91439C37": {
            "Status": 0,
            "D3F19E2248AECEF5C5C3C8B9E2AC2C67": { "Status": 0, "CurrentAmount": 0, "LastUpdateAmount": 0 },
            "0407C2134FE0BEFE3EC791999632D2BC": { "Status": 0, "CurrentAmount": 0, "LastUpdateAmount": 0 }
        },
        "DFE54F884C6FC60688B6C494D79ADD29": {
            "Status": 0,
            "9D1B0D754DBC896034F942AD625F9D93": { "Status": 0, "CurrentAmount": 0, "LastUpdateAmount": 0 }
        }
    })
}

const TUTORIAL_SLATE_FLAGS: [&str; 17] = [
    "TutorialSlate_LanternAbility",
    "TutorialSlate_QuickAttack",
    "TutorialSlate_HeavyAttack",
    "TutorialSlate_Dodge",
    "TutorialSlate_LockOn",
    "TutorialSlate_Sprint",
    "TutorialSlate_Consumables",
    "TutorialSlate_Interact",
    "TutorialSlate_Chat",
    "TutorialSlate_Emote",
    "TutorialSlate_Menu",
    "TutorialSlate_Inventory",
    "TutorialSlate_Loadout",
    "TutorialSlate_Store",
    "TutorialSlate_HuntPass",
    "TutorialSlate_Map",
    "TutorialSlate_Party",
];

pub fn document() -> Value {
    let mut forced: Vec<Value> = TUTORIAL_SLATE_FLAGS.iter().map(|f| json!([f, "True"])).collect();
    forced.push(json!(["CharacterFlag_CityGatherableTutorialShown", "True"]));
    let weapon_data = json!({
        "SheenType": 73,
        "IsPrimarySheenActive": true,
        "PrimaryDyeId": "None",
        "IsSecondarySheenActive": true,
        "SecondaryDyeId": "None",
        "IsTertiarySheenActive": false,
        "TertiaryDyeId": "None",
        "TransmogCatalogId": "None",
        "TransmogEnabled": false,
        "EquippedCells": [],
        "EquippedCellsv2": [],
        "SubTypeMetadataArray": {
            "ItemSubType": "subtype_eblade",
            "EquippedItemParts": [{ "WeaponPartId": "PART_EB_SPECIAL_DEFAULT", "SlotIndex": 0 }]
        }
    });
    json!({
        "format": 1,
        "gameVersion": "1.14.7",
        "characterData": {
            "fill": [
                ["RecentPlayers", "{\"RecentPlayers\":[],\"Version\":0}"],
                ["AppearanceData", default_appearance().to_string()],
                ["PlayerAccountProgressStep", "New"],
                ["CharacterFlagData", "{\"Flags\":[]}"],
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
            "dojoSeries": dojo_series(),
            "dojoNodeId": "Dojo",
            "triggerSeriesKey": "SERIE_cr19_series_1_ftue",
            "triggerNode": "929A333B40E413C41E47B0A425EC3349",
            "triggerStatus": 3
        },
        "starterKit": {
            "instanced": [
                "WP_EB_BEGINNER", "WP_GA_BEGINNER", "WP_IH_BEGINNER", "WP_CB_BEGINNER", "WP_MS_BEGINNER",
                "WP_DP_BEGINNER", "WP_AC_BEGINNER", "PART_EB_SPECIAL_DEFAULT", "PART_IH_SPECIAL_DEFAULT",
                "PART_GA_SPECIAL_DEFAULT", "PART_CB_SPECIAL_DEFAULT", "PART_MS_SPECIAL_PROJECTILE",
                "PART_DP_RECEIVER_DEFAULT", "PART_DP_GRIP_DEFAULT", "PART_AC_SPECIAL_DEFAULT", "AR_UNEQUIPPED_HELM",
                "AR_BEGINNER_CHEST", "AR_BEGINNER_ARMS", "AR_BEGINNER_LEGS", "LT_BASIC", "FL_HEALING_DEFAULT",
                "GD_FRAME_STARTER_BASE", "BN_BEGINNER_00"
            ],
            "defaultWeaponParts": [
                "PART_EB_SPECIAL_DEFAULT", "PART_IH_SPECIAL_DEFAULT", "PART_GA_SPECIAL_DEFAULT",
                "PART_CB_SPECIAL_DEFAULT", "PART_MS_SPECIAL_PROJECTILE", "PART_DP_RECEIVER_DEFAULT",
                "PART_DP_GRIP_DEFAULT", "PART_AC_SPECIAL_DEFAULT"
            ],
            "equipment": [
                ["weapon", "WP_EB_BEGINNER"], ["helmet", "AR_UNEQUIPPED_HELM"], ["chest", "AR_BEGINNER_CHEST"],
                ["arms", "AR_BEGINNER_ARMS"], ["legs", "AR_BEGINNER_LEGS"], ["lantern", "LT_BASIC"]
            ],
            "instanceData": [["WP_EB_BEGINNER", weapon_data]],
            "flask": "FL_HEALING_DEFAULT",
            "banner": "BN_BEGINNER_00",
            "glider": "GD_FRAME_STARTER_BASE",
            "bannerCustomization": {
                "BannerMeshItemID": "BNC_MESH_BEGINNER_00", "FabricMaterialItemID": "BNC_FABRIC_BEGINNER_00",
                "SigilTextureItemID": "BNC_SIGIL_BEGINNER_00", "PlantVFXItemID": "", "PersistantStandardVFXItemID": "",
                "AnimationItemID": "BNC_ANIMATION_BEGINNER_00", "BackgroundColourItemID": "DYE_GREEN04_DURABLE",
                "BorderColourItemID": "DYE_BROWN07_DURABLE", "SigilColourItemID": "DYE_BROWN07_DURABLE",
                "BackgroundSheenType": 0, "BorderSheenType": 0, "SigilSheenType": 0
            },
            "introEmote": "EM_INTRO_BEGINNER_01",
            "flare": "QI_BASIC_FLARE_DURABLE",
            "defaultQuickChats": [
                "qc_hello", "qc_thankyou", "qc_goodjob", "qc_help", "qc_follow_me", "qc_onmyway", "qc_yes", "qc_no",
                "qc_sorry"
            ],
            "wallet": [
                ["CURRENCY_NOTES", 5000], ["CURRENCY_PJM_WEAPON", 50], ["CURRENCY_PJM_ARMOR", 50],
                ["CURRENCY_PJM_ALCHEMY", 50], ["CURRENCY_PJM_AIRSHIP", 50], ["CURRENCY_PJM_FISHING", 50]
            ]
        }
    })
}

pub fn restricted() -> rustic_isles_1_14_7::VersionData {
    rustic_isles_1_14_7::VersionData::from_json(&document()).unwrap()
}
