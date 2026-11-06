//! 1.14.7's character-data defaults (`controllers/character.ts`) and starter kit (`controllers/starterManifest.ts`,
//! `STARTER_WALLET`), the `characterData` and `starterKit` of `version_data.json` (from isles-core's
//! `areas/characters/data.rs` and `starter.rs`, retired for the MIT module, RD27).

use serde_json::{Value, json};
use service_kit::jsjson;

pub const DEFAULT_CHARACTER_FLAGS: &str = "{\"Flags\":[]}";
pub const NO_RECENT_PLAYERS: &str = "{\"RecentPlayers\":[],\"Version\":0}";

pub fn default_appearance() -> Value {
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

/// 1.12 still consults this blob for the Training Grounds destination (character.ts:48-70).
pub fn dojo_series_data() -> String {
    jsjson::stringify(&json!({
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
    }))
}

/// The 17 tutorial popups the client suppresses in Ramsgate when set.
pub const TUTORIAL_SLATE_FLAGS: [&str; 17] = [
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

/// Each owned starter weapon must also own the default part its slot data names, or the client shows no mod row.
pub const STARTER_DEFAULT_WEAPON_PART_CATALOG_IDS: [&str; 8] = [
    "PART_EB_SPECIAL_DEFAULT",
    "PART_IH_SPECIAL_DEFAULT",
    "PART_GA_SPECIAL_DEFAULT",
    "PART_CB_SPECIAL_DEFAULT",
    "PART_MS_SPECIAL_PROJECTILE",
    "PART_DP_RECEIVER_DEFAULT",
    "PART_DP_GRIP_DEFAULT",
    "PART_AC_SPECIAL_DEFAULT",
];

/// The 23 instanced items every new account owns.
pub const STARTER_INSTANCED_CATALOG_IDS: [&str; 23] = [
    "WP_EB_BEGINNER",
    "WP_GA_BEGINNER",
    "WP_IH_BEGINNER",
    "WP_CB_BEGINNER",
    "WP_MS_BEGINNER",
    "WP_DP_BEGINNER",
    "WP_AC_BEGINNER",
    "PART_EB_SPECIAL_DEFAULT",
    "PART_IH_SPECIAL_DEFAULT",
    "PART_GA_SPECIAL_DEFAULT",
    "PART_CB_SPECIAL_DEFAULT",
    "PART_MS_SPECIAL_PROJECTILE",
    "PART_DP_RECEIVER_DEFAULT",
    "PART_DP_GRIP_DEFAULT",
    "PART_AC_SPECIAL_DEFAULT",
    "AR_UNEQUIPPED_HELM",
    "AR_BEGINNER_CHEST",
    "AR_BEGINNER_ARMS",
    "AR_BEGINNER_LEGS",
    "LT_BASIC",
    "FL_HEALING_DEFAULT",
    "GD_FRAME_STARTER_BASE",
    "BN_BEGINNER_00",
];

pub const STARTER_WEAPON_CATALOG_ID: &str = "WP_EB_BEGINNER";
const STARTER_WEAPON_PART_CATALOG_ID: &str = "PART_EB_SPECIAL_DEFAULT";
pub const STARTER_BANNER_CATALOG_ID: &str = "BN_BEGINNER_00";
pub const STARTER_FLASK_CATALOG_ID: &str = "FL_HEALING_DEFAULT";
pub const STARTER_GLIDER_CATALOG_ID: &str = "GD_FRAME_STARTER_BASE";

const CHAT_WHEEL_SLOTS: usize = 9;
pub const DEFAULT_QUICK_CHATS: [&str; CHAT_WHEEL_SLOTS] = [
    "qc_hello",
    "qc_thankyou",
    "qc_goodjob",
    "qc_help",
    "qc_follow_me",
    "qc_onmyway",
    "qc_yes",
    "qc_no",
    "qc_sorry",
];

/// `STARTER_WALLET` (`controllers/wallet.ts`; the bounties-store area owns the wallet routes).
pub const STARTER_WALLET: [(&str, f64); 6] = [
    ("CURRENCY_NOTES", 5000.0),
    ("CURRENCY_PJM_WEAPON", 50.0),
    ("CURRENCY_PJM_ARMOR", 50.0),
    ("CURRENCY_PJM_ALCHEMY", 50.0),
    ("CURRENCY_PJM_AIRSHIP", 50.0),
    ("CURRENCY_PJM_FISHING", 50.0),
];

/// Weapon, then the armor pieces and lantern: the loadout's equipment, by key.
pub const LOADOUT_EQUIPMENT: [(&str, &str); 6] = [
    ("weapon", STARTER_WEAPON_CATALOG_ID),
    ("helmet", "AR_UNEQUIPPED_HELM"),
    ("chest", "AR_BEGINNER_CHEST"),
    ("arms", "AR_BEGINNER_ARMS"),
    ("legs", "AR_BEGINNER_LEGS"),
    ("lantern", "LT_BASIC"),
];

pub fn build_instance_data() -> String {
    jsjson::stringify(&json!({
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
            "EquippedItemParts": [{ "WeaponPartId": STARTER_WEAPON_PART_CATALOG_ID, "SlotIndex": 0 }]
        }
    }))
}
