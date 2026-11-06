//! `generate_reward_cache_store.ts`'s selection plan, the `rewardCachePlan` of `version_data.json`: which catalog ids
//! the Reward Cache sells, in which tab, at what price, and the family orders it sorts by (from isles-core's
//! generators, retired for the MIT module, RD27).

use serde_json::{Map, Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum RewardCacheCategory {
    Armour,
    Weapon,
    Style,
    Event,
    Limited,
    General,
}

impl RewardCacheCategory {
    fn as_str(self) -> &'static str {
        match self {
            Self::Armour => "armour",
            Self::Weapon => "weapon",
            Self::Style => "style",
            Self::Event => "event",
            Self::Limited => "limited",
            Self::General => "general",
        }
    }

    fn expected_count(self) -> usize {
        match self {
            Self::Armour => 48,
            Self::Weapon => 34,
            Self::Style => 23,
            Self::Event => 55,
            Self::Limited => 10,
            Self::General => 8,
        }
    }
}

#[derive(Clone, Copy)]
struct RewardCacheSelection {
    catalog_id: &'static str,
    category: RewardCacheCategory,
    price: i64,
    featured: bool,
    allow_rarity3: bool,
    display_name_override: Option<&'static str>,
    description_override: Option<&'static str>,
}

const fn sel(catalog_id: &'static str, category: RewardCacheCategory, price: i64) -> RewardCacheSelection {
    RewardCacheSelection {
        catalog_id,
        category,
        price,
        featured: false,
        allow_rarity3: false,
        display_name_override: None,
        description_override: None,
    }
}

const fn sel_featured(catalog_id: &'static str, category: RewardCacheCategory, price: i64) -> RewardCacheSelection {
    RewardCacheSelection {
        featured: true,
        ..sel(catalog_id, category, price)
    }
}

const fn sel_r3(catalog_id: &'static str, category: RewardCacheCategory, price: i64) -> RewardCacheSelection {
    RewardCacheSelection {
        allow_rarity3: true,
        ..sel(catalog_id, category, price)
    }
}

const fn sel_override(
    catalog_id: &'static str,
    category: RewardCacheCategory,
    price: i64,
    name: &'static str,
    desc: &'static str,
) -> RewardCacheSelection {
    RewardCacheSelection {
        catalog_id,
        category,
        price,
        featured: true,
        allow_rarity3: false,
        display_name_override: Some(name),
        description_override: Some(desc),
    }
}

use RewardCacheCategory::{Armour, Event, General, Limited, Style, Weapon};

const REWARD_CACHE_SELECTIONS: &[RewardCacheSelection] = &[
    // Four marquee offers also populate the FEATURED tab.
    sel_featured("AR_HP08A_HEROES_HELM_00_ULTRA", Armour, 3000),
    sel_featured("WP_EB_HP08A_HEROES_01", Weapon, 3000),
    sel_featured("EM_INTRO_HP10A_CELESTIAL_00", Style, 3000),
    sel_featured("AC_HEAD_UNSEEN_00", Limited, 3000),
    // Armour
    sel("AR_HP08A_HEROES_CHEST_ULTRA", Armour, 2500),
    sel("AR_HP08A_HEROES_LEGS_ULTRA", Armour, 2500),
    sel("AR_HP07B_WITCH_HELM_ULTRA", Armour, 2500),
    sel("AR_HP07A_MONK_HELM_ULTRA", Armour, 2500),
    sel("AR_HP11A_ALCHEMIST_HELM_00_ULTRA", Armour, 2500),
    sel("AR_HP10A_CELESTIAL_HELM_02", Armour, 2500),
    sel("AR_HP09A_ASSASSINS_CHEST_00_ULTRA", Armour, 2500),
    sel("AR_HP08B_PUNK_CHEST_ULTRA", Armour, 2500),
    sel("AR_ARMSTRONG_HELM_ULTRA", Armour, 2000),
    sel("AR_HP10A_CELESTIAL_ARMS_00_ULTRA", Armour, 2500),
    sel("AR_HP10B_RAIDERS_ARMS_01", Armour, 2500),
    sel("AR_HP11A_ALCHEMIST_ARMS_00_ULTRA", Armour, 2500),
    sel("AR_HP07A_MONK_ARMS_ULTRA", Armour, 2500),
    // Complete premium armour-set pieces.
    sel("AR_HP11A_ALCHEMIST_CHEST_00_ULTRA", Armour, 2500),
    sel("AR_HP11A_ALCHEMIST_LEGS_00_ULTRA", Armour, 2500),
    sel("AR_HP10A_CELESTIAL_HELM_00_ULTRA", Armour, 2500),
    sel("AR_HP10A_CELESTIAL_CHEST_00_ULTRA", Armour, 2500),
    sel("AR_HP10A_CELESTIAL_LEGS_00_ULTRA", Armour, 2500),
    sel("AR_HP07B_WITCH_ARMS_ULTRA", Armour, 2500),
    sel("AR_HP07B_WITCH_CHEST_ULTRA", Armour, 2500),
    sel("AR_HP07B_WITCH_LEGS_ULTRA", Armour, 2500),
    sel("AR_HP12A_FROSTWARDEN_HELM_00_ULTRA", Armour, 2500),
    sel("AR_HP12A_FROSTWARDEN_ARMS_00_ULTRA", Armour, 2500),
    sel("AR_HP12A_FROSTWARDEN_CHEST_00_ULTRA", Armour, 2500),
    sel("AR_HP12A_FROSTWARDEN_LEGS_00_ULTRA", Armour, 2500),
    // Additional premium complete-set pieces.
    sel("AR_HP08A_HEROES_ARMS_ULTRA", Armour, 2500),
    sel("AR_HP09A_ASSASSINS_HELM_00_ULTRA", Armour, 2500),
    sel("AR_HP09A_ASSASSINS_ARMS_00_ULTRA", Armour, 2500),
    sel("AR_HP09A_ASSASSINS_LEGS_00_ULTRA", Armour, 2500),
    sel("AR_HP08B_PUNK_HELM_ULTRA", Armour, 2500),
    sel("AR_HP08B_PUNK_ARMS_ULTRA", Armour, 2500),
    sel("AR_HP08B_PUNK_LEGS_ULTRA", Armour, 2500),
    sel("AR_HP10B_RAIDERS_HELM_00_ULTRA", Armour, 2500),
    sel("AR_HP10B_RAIDERS_ARMS_00_ULTRA", Armour, 2500),
    sel("AR_HP10B_RAIDERS_CHEST_00_ULTRA", Armour, 2500),
    sel("AR_HP10B_RAIDERS_LEGS_00_ULTRA", Armour, 2500),
    sel("AR_HP09B_COMMANDO_HELM_00_ULTRA", Armour, 2500),
    sel("AR_HP09B_COMMANDO_ARMS_00_ULTRA", Armour, 2500),
    sel("AR_HP09B_COMMANDO_CHEST_00_ULTRA", Armour, 2500),
    sel("AR_HP09B_COMMANDO_LEGS_00_ULTRA", Armour, 2500),
    sel("AR_ARMSTRONG_ARMS_ULTRA", Armour, 2000),
    sel("AR_ARMSTRONG_CHEST_ULTRA", Armour, 2000),
    sel("AR_ARMSTRONG_LEGS_ULTRA", Armour, 2000),
    // Rogue Elements standard tactical set.
    sel("AR_HP09B_COMMANDO_HELM_00", Armour, 2000),
    sel("AR_HP09B_COMMANDO_CHEST_00", Armour, 2000),
    sel("AR_HP09B_COMMANDO_ARMS_00", Armour, 2000),
    sel("AR_HP09B_COMMANDO_LEGS_00", Armour, 2000),
    // Weapons
    sel("WP_IH_HP10B_RAIDERS_01", Weapon, 2500),
    sel("WP_EB_HP12A_FROSTWARDEN_01", Weapon, 2500),
    sel("WP_AC_HP10A_CELESTIAL_01", Weapon, 2500),
    sel("WP_DP_HP10A_CELESTIAL_01", Weapon, 2500),
    sel("WP_MS_HP11A_ALCHEMIST_01", Weapon, 2500),
    sel("WP_EB_HP10A_CELESTIAL_00", Weapon, 2000),
    sel("WP_IH_HP08B_PUNK_00", Weapon, 2000),
    sel("WP_AC_HP10A_CELESTIAL_00", Weapon, 2000),
    sel("WP_AC_HP11A_ALCHEMIST_00", Weapon, 2000),
    sel("WP_AC_HP07A_MONK_00", Weapon, 2000),
    sel("WP_AC_HP08B_PUNK_00", Weapon, 2000),
    // Axe transmogs
    sel("WP_GA_HP11A_ALCHEMIST_00", Weapon, 2000),
    sel("WP_GA_ARCSLAYER_00", Weapon, 2000),
    sel("WP_GA_HP12A_FROSTWARDEN_00", Weapon, 2000),
    sel("WP_GA_HP08A_HEROES_00", Weapon, 2000),
    sel("WP_GA_HP09A_ASSASSINS_00", Weapon, 2000),
    sel("WP_GA_HP10A_CELESTIAL_00", Weapon, 2000),
    sel("WP_GA_HEADSMAN", Weapon, 2000),
    sel("WP_GA_HP07B_WITCH_00", Weapon, 2000),
    // Remaining resolved premium axe transmogs.
    sel("WP_GA_HP10B_RAIDERS_00", Weapon, 2500),
    sel("WP_GA_HP06_PIRATE_00", Weapon, 2500),
    sel("WP_GA_HP07A_MONK_00", Weapon, 2500),
    sel("WP_GA_HP08B_PUNK_00", Weapon, 2500),
    sel("WP_GA_HP09B_COMMANDO_00", Weapon, 2500),
    sel("WP_GA_ROMANTIC", Weapon, 2500),
    sel("WP_GA_HP06B_ACADEMY_00", Weapon, 2500),
    sel_r3("WP_GA_HP04_ARID", Weapon, 2000),
    sel_r3("WP_GA_HP03_MOSSY", Weapon, 2000),
    sel_r3("WP_GA_HP05_NINJA_00", Weapon, 2000),
    sel_r3("WP_GA_HP05_NINJA_02", Weapon, 2000),
    // Legendary premium axes.
    sel_override(
        "WP_GA_ELVEN_00",
        Weapon,
        3000,
        "Lightsworn Benediction",
        "A radiant Lightsworn axe transmog forged for a Slayer of Andar.",
    ),
    sel_override(
        "WP_GA_HP16A_MARAUDERS_01",
        Weapon,
        3000,
        "Hellion's Foil",
        "A legendary blaze axe transmog inspired by Hellion.",
    ),
    sel_override(
        "WP_GA_METAL_00",
        Weapon,
        3000,
        "The Debt Collector",
        "A heavy legendary metal axe transmog built to collect what is owed.",
    ),
    // Style
    sel("DYE_RED01_DURABLE", Style, 1000),
    sel("DYE_GRAY05_DURABLE", Style, 1000),
    sel("DYE_GREEN14_DURABLE", Style, 1000),
    sel("DYE_BLUE13_DURABLE", Style, 1000),
    sel("EM_PLAYER_HP07A_FIREBALL_00", Style, 2000),
    sel("EM_PLAYER_HP10A_CELESTIAL_04", Style, 2000),
    sel("EM_INTRO_HP08A_HEROIC_00", Style, 2500),
    // Premium player and arrival emotes.
    sel("EM_INTRO_HP12A_FROSTWARDEN_00", Style, 2500),
    sel("EM_INTRO_HP09A_ASSASSINS_00", Style, 2500),
    sel("EM_INTRO_HP03_VINE_00", Style, 2500),
    sel("EM_INTRO_HP08B_SKATEBOARD_00", Style, 2500),
    sel("EM_PLAYER_HP11A_ALCHEMIST_00", Style, 2500),
    sel("EM_INTRO_HP10B_RAIDERS_01", Style, 2500),
    sel("EM_INTRO_HP07B_COFFINDROP_BAT_00", Style, 2500),
    sel("EM_INTRO_HP07B_COFFINDROP_DANCE_00", Style, 2500),
    sel("EM_INTRO_HP09B_COMMANDO_00", Style, 2500),
    sel("EM_INTRO_HP10B_RAIDERS_00", Style, 2500),
    sel("EM_INTRO_HP08A_WARCRY_00", Style, 2500),
    // Flares.
    sel("QI_FLARE_FUNGUY", Style, 1000),
    sel("QI_FLARE_HP11A_ALCHEMIST_00", Style, 1000),
    sel("QI_FLARE_HP08B_03", Style, 1000),
    sel("QI_FLARE_SAINTS19_06", Style, 1000),
    // Event
    sel("WP_DP_EVENT_SPRING_00", Event, 2500),
    sel("EM_PLAYER_EVENT_FROST17_002", Event, 2000),
    sel("BNC_ANIMATION_EVENT_FROST17_00", Event, 1000),
    sel("BNC_SIGIL_EVENT_FROST17_00", Event, 800),
    // Banner standards / flagstaff skins.
    sel("BNC_STANDARD_HP10A_CELESTIAL_00", Event, 1000),
    sel("BNC_STANDARD_HP09A_ASSASSINS_00", Event, 1000),
    sel("BNC_STANDARD_HP10B_RAIDERS_00", Event, 1000),
    sel("BNC_STANDARD_HP07A_MONK_01", Event, 1000),
    sel("BNC_STANDARD_HP08B_PUNK_00", Event, 1000),
    // Premium banner-plant animations.
    sel("BNC_ANIMATION_HP05_FLAMES_00", Event, 2000),
    sel("BNC_ANIMATION_HP05_PETALS_00", Event, 2000),
    sel("BNC_ANIMATION_HP05_NINJASTARS_00", Event, 2000),
    sel("BNC_ANIMATION_HP09A_ASSASSINS_00", Event, 2000),
    sel("BNC_ANIMATION_HP11A_ALCHEMIST_00", Event, 2000),
    sel("BNC_ANIMATION_HP12A_FROSTWARDEN_00", Event, 2000),
    sel("BNC_ANIMATION_AUTUMN_FOX_00", Event, 2000),
    sel("BNC_ANIMATION_SAINTS_BOND_00", Event, 2000),
    // Additional premium flagstaff skins.
    sel("BNC_MESH_EVENT_FROST17_00", Event, 2500),
    sel("BNC_STANDARD_OSTIA_00", Event, 2500),
    sel("BNC_STANDARD_OSTIA_01", Event, 2500),
    sel("BNC_STANDARD_FUNGUY_00", Event, 2000),
    // Additional premium banner standards.
    sel("BNC_STANDARD_HP09B_COMMANDO_00", Event, 2000),
    sel("BNC_STANDARD_HP05_NINJA_00", Event, 2000),
    sel("BNC_STANDARD_HP05_NINJA_01", Event, 2000),
    sel("BNC_STANDARD_HP05_NINJA_02", Event, 2000),
    sel("BNC_STANDARD_1P_ARCSLAYER", Event, 2000),
    sel("BNC_STANDARD_1P_UNSEEN", Event, 2000),
    sel("BNC_STANDARD_HP03_MOSSY_00", Event, 2000),
    // Banner fabrics / shields.
    sel("BNC_FABRIC_EVENT_HP09A_ASSASSINS_01", Event, 1000),
    sel("BNC_FABRIC_EVENT_HP09B_COMMANDO_00", Event, 1000),
    sel("BNC_FABRIC_EVENT_HP10A_CELESTIAL_00", Event, 1000),
    sel("BNC_FABRIC_EVENT_HP10B_RAIDERS_00", Event, 1000),
    sel("BNC_FABRIC_EVENT_HP08B_00", Event, 1000),
    sel("BNC_FABRIC_EVENT_HP08B_01", Event, 1000),
    sel("BNC_FABRIC_EVENT_HP08B_02", Event, 1000),
    sel("BNC_FABRIC_EVENT_HP11A_ALCHEMIST_00", Event, 1000),
    sel("BNC_FABRIC_EVENT_HP12A_FROSTWARDEN_00", Event, 1000),
    sel("BNC_FABRIC_AUTUMN_FOX", Event, 1000),
    sel("BNC_FABRIC_FUNGUY", Event, 1000),
    sel("BNC_FABRIC_ARMSTRONG", Event, 1000),
    sel("BNC_FABRIC_EVENT_DARK17_000", Event, 1000),
    sel("BNC_FABRIC_EVENT_FROST18_01", Event, 1000),
    sel("BNC_FABRIC_EVENT_FROST18_02", Event, 1000),
    // Banner sigils / logos.
    sel("BNC_SIGIL_EVENT_HP09A_ASSASSINS_00", Event, 1000),
    sel("BNC_SIGIL_HP09B_COMMANDO_00", Event, 1000),
    sel("BNC_SIGIL_HP10B_RAIDERS_00", Event, 1000),
    sel("BNC_SIGIL_EVENT_HP08B_01", Event, 1000),
    sel("BNC_SIGIL_EVENT_HP08B_02", Event, 1000),
    sel("BNC_SIGIL_EVENT_HP08B_03", Event, 1000),
    sel("BNC_SIGIL_FUNGUY", Event, 1000),
    sel("BNC_SIGIL_EVENT_FROST17_01", Event, 1000),
    sel("BNC_SIGIL_EVENT_DARK17_000", Event, 1000),
    sel("BNC_SIGIL_RANDALL", Event, 1000),
    sel("BNC_SIGIL_EVENT_HP08A_00", Event, 1000),
    sel("BNC_SIGIL_036", Event, 2000),
    // Limited
    sel("AC_BACK_HP06-1_01", Limited, 2500),
    sel("AC_BACK_HP06-1_00", Limited, 2500),
    sel("GD_FRAME_A_REDSTRIPE", Limited, 2500),
    sel("GD_FRAME_A_BASE", Limited, 2500),
    sel("GD_FRAME_B_BLUEWING", Limited, 2000),
    sel("GD_FRAME_B_FLUTTERHEART", Limited, 2000),
    sel("GD_FRAME_B_SPRINGTIDE", Limited, 2000),
    sel("GD_FRAME_B_BASE", Limited, 2000),
    sel("GD_FRAME_C_BASE", Limited, 2000),
    // General
    sel("TITLE_HP10A_CELESTIAL_01", General, 700),
    sel("BNC_STANDARD_HP08A_HEROES_00", General, 800),
    sel("TITLE_HP08B_01", General, 800),
    sel("TITLE_HP09A_ASSASSINS_01", General, 800),
    sel("TITLE_HP07B_00", General, 800),
    sel("TITLE_HP08A_01", General, 800),
    sel("TITLE_HP09B_COMMANDO_01", General, 800),
    sel("TITLE_HP10B_RAIDERS_01", General, 800),
];

const ARMOUR_FAMILY_ORDER: &[&str] = &[
    "AR_HP08A_HEROES",
    "AR_HP09A_ASSASSINS",
    "AR_HP08B_PUNK",
    "AR_HP10B_RAIDERS",
    "AR_HP09B_COMMANDO",
    "AR_HP11A_ALCHEMIST",
    "AR_HP10A_CELESTIAL",
    "AR_HP07B_WITCH",
    "AR_HP12A_FROSTWARDEN",
    "AR_ARMSTRONG",
    "AR_HP07A_MONK",
];

const BANNER_FAMILY_ORDER: &[&str] = &[
    "HP09A_ASSASSINS",
    "HP09B_COMMANDO",
    "HP10B_RAIDERS",
    "HP08B",
    "HP11A_ALCHEMIST",
    "HP12A_FROSTWARDEN",
    "HP10A_CELESTIAL",
    "HP08A",
    "HP05_NINJA",
    "FROST17",
    "FROST18",
    "DARK17",
    "AUTUMN_FOX",
    "FUNGUY",
    "ARMSTRONG",
    "ARCSLAYER",
    "UNSEEN",
    "MOSSY",
    "RANDALL",
];

/// The plan as dainty-lass's `RewardCachePlan::from_json` reads it, for restricted's `version_data.json` (RD27).
pub fn reward_cache_plan_document() -> Value {
    let selections: Vec<Value> = REWARD_CACHE_SELECTIONS
        .iter()
        .map(|s| {
            let mut entry = json!({
                "catalogId": s.catalog_id,
                "category": s.category.as_str(),
                "price": s.price,
                "featured": s.featured,
                "allowRarity3": s.allow_rarity3,
            });
            if let Some(name) = s.display_name_override {
                entry["displayName"] = json!(name);
            }
            if let Some(description) = s.description_override {
                entry["description"] = json!(description);
            }
            entry
        })
        .collect();
    let expected: Map<String, Value> = [Armour, Weapon, Style, Event, Limited, General]
        .iter()
        .map(|c| (c.as_str().to_owned(), json!(c.expected_count())))
        .collect();
    json!({
        "selections": selections,
        "armourFamilies": ARMOUR_FAMILY_ORDER,
        "bannerFamilies": BANNER_FAMILY_ORDER,
        "expected": expected,
        "expectedFeatured": 7,
    })
}
