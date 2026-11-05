//! What a player host may grant (`p2p/security/hostGrantPolicy.ts`) and the daily premium draft token's balance cap
//! (`hostDraftPremium.ts`). Pure: the game data comes in as a [`HostGrantCatalog`] and the balance as the character's
//! stored `stackedItems`. Both refuse only in enforce mode; observe logs them (`hostPolicyMode.ts`).

use rifts_seam::HostGrantCatalog;
use serde_json::Value;

use crate::json::js_trim;

pub const HOST_DRAFT_PREMIUM_ID: &str = "TOKEN_DAILY_CHALLENGE_DRAFT_PREMIUM";
/// The owner's fallback: the catalogue gives the token no cap (`maxQuantity` 0).
pub const HOST_DRAFT_PREMIUM_BALANCE_CAP: i128 = 1;

/// The game's starter grant on every hub arrival; refusing it sends a rank-20 player back to the menu.
pub const HOST_STARTER_COSMETIC_IDS: [&str; 13] = [
    "EM_INTRO_GENERIC_001",
    "EM_INTRO_GENERIC_002",
    "EM_INTRO_GENERIC_003",
    "TITLE_BLANK",
    "EJ_SYSTEM_TYPING",
    "EJ_PLAYER_BOSUN_JOY",
    "EJ_PLAYER_KAT_PRAY",
    "EJ_PLAYER_MOYRA_MUSCLE",
    "EJ_PLAYER_GREGARIO_QUESTION",
    "EJ_PLAYER_MIDDLEMAN_HMM",
    "EJ_PLAYER_PRIYANI_WINK",
    "EJ_PLAYER_ZAI_ANGRY",
    "EJ_PLAYER_ZAI_SWEAT",
];

/// `bountyState.ts`: what one season-challenge claim funds.
pub const CHALLENGE_REWARD_CURRENCIES: [&str; 1] = ["CURRENCY_SEASONAL_COIN"];
pub const MAX_CHALLENGE_REWARD_PER_CLAIM: f64 = 1000.0;

/// `platinumWallet.ts`: the 13 catalogue rows of Platinum.
const PLATINUM_CATALOG_IDS: [&str; 13] = [
    "CURRENCY_PLATINUM_CS",
    "CURRENCY_PLATINUM_UNIV",
    "CURRENCY_PLATINUM_PLATFORM_BONUS",
    "CURRENCY_PLATINUM_EPIC_BONUS",
    "CURRENCY_PLATINUM_XBOX_BONUS",
    "CURRENCY_PLATINUM_PSN_BONUS",
    "CURRENCY_PLATINUM_SWITCH_BONUS",
    "CURRENCY_PLATINUM",
    "CURRENCY_PLATINUM_PLATFORM",
    "CURRENCY_PLATINUM_EPIC",
    "CURRENCY_PLATINUM_XBOX",
    "CURRENCY_PLATINUM_PSN",
    "CURRENCY_PLATINUM_SWITCH",
];

const HUNT_PASS_IDS: [&str; 4] = [
    "CURRENCY_SEASONAL_COIN",
    "CURRENCY_REWARDCACHE",
    "CURRENCY_PRESTIGE",
    "GRANTPROGRESSITEM_HUNTPASS",
];
const COSMETIC_PREFIXES: [&str; 14] = [
    "DYE_",
    "EM_",
    "EJ_",
    "TITLE_",
    "BNC_",
    "BN_",
    "AC_",
    "GD_",
    "CC_",
    "TY_",
    "MP_",
    "CONTAINER_PROMO_",
    "CONTAINER_1P_",
    "CONTAINER_CORE_FOUNDER_",
];
const COSMETIC_IDS: [&str; 3] = [
    "CHANGE_APPEARANCE_TOKEN",
    "CURRENCY_TOKEN_TRANSMOG",
    "CONTAINER_CORE_SLAYERPACK",
];
const COSMETIC_TAGS: [&str; 13] = [
    "dye",
    "emote",
    "emoji",
    "title",
    "bannercustomization",
    "banner",
    "accessory",
    "glider",
    "toy",
    "companion",
    "stylehunter",
    "founder",
    "transmogtoken",
];
const ENTITLEMENT_PREFIXES: [&str; 5] = ["ent", "pass", "eventpass", "minipass", "exchange_slot"];

/// `DefaultHostGrantCatalog` with no game-data files: only the id rules apply.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoGameData;

impl HostGrantCatalog for NoGameData {
    fn tags_of(&self, _: &str) -> Option<Vec<String>> {
        None
    }
    fn is_prestige_reward(&self, _: &str) -> bool {
        false
    }
    fn is_prestige_gated(&self, _: &str) -> bool {
        false
    }
    fn is_entitlement_name(&self, _: &str) -> bool {
        false
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenyClass {
    PremiumCurrency,
    HuntPass,
    Cosmetic,
    Entitlement,
}

impl DenyClass {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PremiumCurrency => "PREMIUM_CURRENCY",
            Self::HuntPass => "HUNT_PASS",
            Self::Cosmetic => "COSMETIC",
            Self::Entitlement => "ENTITLEMENT",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantField {
    AddStacked,
    AddInstanced,
    RemoveStacked,
    RemoveInstanced,
}

impl GrantField {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AddStacked => "addStackedItems",
            Self::AddInstanced => "addInstancedItems",
            Self::RemoveStacked => "removeStackedItems",
            Self::RemoveInstanced => "removeInstancedItems",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub catalog_id: String,
    pub denied_class: DenyClass,
    pub field: GrantField,
}

/// `HostGrantDenial`'s answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantDenial {
    /// The denied classes in order of appearance, or `NO_REWARD_SESSION`.
    pub classes: Vec<&'static str>,
    pub violations: Vec<Violation>,
    pub summary: String,
}

/// What the session the grant is for looks like to `HostGrantDenial`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GrantSession {
    /// `guardCompletionMode === "NO_COMPLETION"`.
    pub no_completion: bool,
    /// A realm hub of a known kind (Ramsgate or Training).
    pub realm_hub: bool,
}

/// `HostGrantAllowances`: `draft_premium` once `hostDraftPremium.ts` allowed the body's one token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Allowances {
    pub draft_premium: bool,
}

fn catalog_id(item: &Value) -> Option<&str> {
    item.get("catalogId").and_then(Value::as_str)
}

fn quantity(item: &Value) -> Option<f64> {
    item.get("quantity").and_then(Value::as_f64)
}

/// `Number.isSafeInteger`.
fn safe_integer(value: f64) -> bool {
    value.is_finite() && value.trunc() == value && value.abs() <= 9_007_199_254_740_991.0
}

fn items<'a>(body: &'a Value, field: &str) -> &'a [Value] {
    body.get(field).and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

/// `CanonicalId`: `id_currency_platinum` (the store's spelling) is `CURRENCY_PLATINUM`.
fn canonical_id(catalog_id: &str) -> String {
    let trimmed = js_trim(catalog_id);
    match trimmed.get(..12) {
        Some(prefix) if prefix.eq_ignore_ascii_case("id_currency_") => {
            format!("CURRENCY_{}", &trimmed[12..]).to_uppercase()
        }
        _ => trimmed.to_owned(),
    }
}

fn is_platinum_catalog_id(id: &str) -> bool {
    PLATINUM_CATALOG_IDS.contains(&id)
}

/// `/^CURRENCY_S\d+_(COIN|DAILY)$/`.
fn is_season_currency(upper: &str) -> bool {
    upper.strip_prefix("CURRENCY_S").is_some_and(|rest| {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        digits > 0 && matches!(&rest[digits..], "_COIN" | "_DAILY")
    })
}

fn entitlement_tail(rest: &str) -> bool {
    !rest.is_empty()
        && rest
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'.' | b'-'))
}

/// `ENTITLEMENT_PATTERNS`.
fn looks_like_entitlement(id: &str) -> bool {
    ENTITLEMENT_PREFIXES.iter().any(|prefix| {
        id.strip_prefix(prefix)
            .and_then(|rest| rest.strip_prefix('_'))
            .is_some_and(entitlement_tail)
    }) || id.strip_suffix("_premium").is_some_and(entitlement_tail)
}

/// `ClassifyHostGrant`.
pub fn classify(catalog_id: &str, catalog: &dyn HostGrantCatalog) -> Option<DenyClass> {
    if js_trim(catalog_id).is_empty() {
        return None;
    }
    let id = canonical_id(catalog_id);
    let upper = id.to_uppercase();
    let tags: Vec<String> = catalog
        .tags_of(&id)
        .unwrap_or_default()
        .iter()
        .map(|tag| tag.to_lowercase())
        .collect();
    let tagged = |tag: &str| tags.iter().any(|own| own == tag);
    if upper.starts_with("CURRENCY_PLATINUM")
        || upper.starts_with("DEBUG_")
        || is_platinum_catalog_id(&id)
        || tagged("platinum")
    {
        return Some(DenyClass::PremiumCurrency);
    }
    if HUNT_PASS_IDS.contains(&upper.as_str())
        || is_season_currency(&upper)
        || upper.starts_with("TOKEN_HUNTPASS")
        || upper.starts_with("TOKEN_HUNT_PASS")
        || upper.ends_with("_DRAFT_PREMIUM")
        || upper.starts_with("CONTAINER_CORE_PRESTIGE_")
        || catalog.is_prestige_reward(&id)
        || tagged("seasonal_currency")
    {
        return Some(DenyClass::HuntPass);
    }
    if COSMETIC_IDS.contains(&upper.as_str())
        || COSMETIC_PREFIXES.iter().any(|prefix| upper.starts_with(prefix))
        || tags.iter().any(|tag| COSMETIC_TAGS.contains(&tag.as_str()))
    {
        return Some(DenyClass::Cosmetic);
    }
    if catalog.is_entitlement_name(&id.to_lowercase()) || looks_like_entitlement(&id) {
        return Some(DenyClass::Entitlement);
    }
    None
}

/// `IsClaimFundedCoinGrant`: the exact id and a whole 1..=1000, what one claim funds.
pub fn is_claim_funded_coin_grant(item: &Value) -> bool {
    catalog_id(item).is_some_and(|id| CHALLENGE_REWARD_CURRENCIES.contains(&id))
        && quantity(item).is_some_and(|q| safe_integer(q) && q > 0.0 && q <= MAX_CHALLENGE_REWARD_PER_CLAIM)
}

/// `IsPrestigeFundedGrant`: the wallet's prestige gate pays it, and only a HUNT_PASS-class id.
pub fn is_prestige_funded_grant(item: &Value, catalog: &dyn HostGrantCatalog) -> bool {
    catalog_id(item).is_some_and(|id| {
        catalog.is_prestige_gated(id)
            && quantity(item).is_some_and(|q| safe_integer(q) && q > 0.0)
            && classify(id, catalog) == Some(DenyClass::HuntPass)
    })
}

/// `IsHostStarterCosmeticAdd`: one of the starter ids, quantity 1 or none.
pub fn is_host_starter_cosmetic_add(item: &Value) -> bool {
    catalog_id(item).is_some_and(|id| HOST_STARTER_COSMETIC_IDS.contains(&id))
        && item.get("quantity").is_none_or(|q| q.as_f64() == Some(1.0))
}

/// `IsHostDraftPremiumAdd`: exactly the id (case included) and one whole token.
pub fn is_host_draft_premium_add(item: &Value) -> bool {
    catalog_id(item) == Some(HOST_DRAFT_PREMIUM_ID) && quantity(item) == Some(1.0)
}

/// `BodyAddsHostDraftPremium`.
pub fn body_adds_host_draft_premium(body: &Value) -> bool {
    items(body, "addStackedItems")
        .iter()
        .any(|item| catalog_id(item) == Some(HOST_DRAFT_PREMIUM_ID))
}

/// `IsPlatinumSpendItem`: a stacked removal of Platinum is a spend (owner decision, 14 Oct).
pub fn is_platinum_spend_item(catalog_id: &str) -> bool {
    let id = canonical_id(catalog_id);
    id == "CURRENCY_PLATINUM" || is_platinum_catalog_id(&id)
}

/// `InventoryBodyHasGrantOrSpend` (`validation.ts`).
pub fn body_has_grant_or_spend(body: &Value) -> bool {
    body.is_object()
        && [
            "addInstancedItems",
            "addStackedItems",
            "removeInstancedItems",
            "removeStackedItems",
        ]
        .iter()
        .any(|field| !items(body, field).is_empty())
}

/// `HostGrantViolations`.
pub fn violations(body: &Value, catalog: &dyn HostGrantCatalog, allowances: Allowances) -> Vec<Violation> {
    let mut found = Vec::new();
    // A body that names the token twice gets no exemption at all
    let draft_premium_adds = items(body, "addStackedItems")
        .iter()
        .filter(|item| catalog_id(item) == Some(HOST_DRAFT_PREMIUM_ID))
        .count();
    let mut scan = |field: GrantField, removal: bool| {
        for item in items(body, field.as_str()) {
            let Some(id) = catalog_id(item) else {
                continue;
            };
            if field == GrantField::AddStacked
                && (is_claim_funded_coin_grant(item)
                    || is_prestige_funded_grant(item, catalog)
                    || (allowances.draft_premium && draft_premium_adds == 1 && is_host_draft_premium_add(item)))
            {
                continue;
            }
            if field == GrantField::RemoveStacked && is_platinum_spend_item(id) {
                continue;
            }
            if matches!(field, GrantField::AddStacked | GrantField::AddInstanced) && is_host_starter_cosmetic_add(item)
            {
                continue;
            }
            // A host may not take premium currency or an entitlement away; other removals are its own
            match classify(id, catalog) {
                Some(class) if !removal || matches!(class, DenyClass::PremiumCurrency | DenyClass::Entitlement) => {
                    found.push(Violation {
                        catalog_id: id.to_owned(),
                        denied_class: class,
                        field,
                    });
                }
                _ => {}
            }
        }
    };
    scan(GrantField::AddStacked, false);
    scan(GrantField::AddInstanced, false);
    scan(GrantField::RemoveStacked, true);
    scan(GrantField::RemoveInstanced, true);
    found
}

/// `HostGrantDenial`: a forbidden-class entry, or any grant or spend in a no-reward session (a hosted Training hunt;
/// a realm's hubs grant as a hunt does).
pub fn grant_denial(
    session: GrantSession,
    body: &Value,
    catalog: &dyn HostGrantCatalog,
    allowances: Allowances,
) -> Option<GrantDenial> {
    if session.no_completion && !session.realm_hub && body_has_grant_or_spend(body) {
        return Some(GrantDenial {
            classes: vec!["NO_REWARD_SESSION"],
            violations: Vec::new(),
            summary: "any grant or spend".to_owned(),
        });
    }
    let found = violations(body, catalog, allowances);
    if found.is_empty() {
        return None;
    }
    let mut summary = found
        .iter()
        .take(16)
        .map(|entry| {
            format!(
                "{}({},{})",
                entry.catalog_id,
                entry.denied_class.as_str(),
                entry.field.as_str()
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    if found.len() > 16 {
        summary.push_str(&format!(",+{}", found.len() - 16));
    }
    let mut classes: Vec<&'static str> = Vec::new();
    for entry in &found {
        if !classes.contains(&entry.denied_class.as_str()) {
            classes.push(entry.denied_class.as_str());
        }
    }
    Some(GrantDenial {
        classes,
        violations: found,
        summary,
    })
}

/// `StackedBalanceOf` on the stored `stackedItems` text: no inventory is 0, an unreadable list is `None` (refused).
pub fn stacked_balance_of(stacked_items: Option<&str>, catalog_id: &str) -> Option<i128> {
    let Some(text) = stacked_items else {
        return Some(0);
    };
    let parsed: Value = serde_json::from_str(text).ok()?;
    let mut total: i128 = 0;
    for item in parsed.as_array()? {
        if self::catalog_id(item) != Some(catalog_id) {
            continue;
        }
        let quantity = quantity(item).filter(|q| safe_integer(*q) && *q >= 0.0)?;
        total += quantity as i128;
    }
    Some(total)
}

/// `HostDraftPremiumDecision`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DraftPremiumDecision {
    pub allowed: bool,
    pub reason: &'static str,
    /// Before and after this transaction, when the balance was read.
    pub balance: Option<(i128, i128)>,
}

impl DraftPremiumDecision {
    const fn refused(reason: &'static str) -> Self {
        Self {
            allowed: false,
            reason,
            balance: None,
        }
    }

    /// ` balance=0->1 cap=1`, or nothing when the balance was not read.
    pub fn balance_tags(&self) -> String {
        self.balance.map_or_else(String::new, |(before, after)| {
            format!(" balance={before}->{after} cap={HOST_DRAFT_PREMIUM_BALANCE_CAP}")
        })
    }
}

/// `DecideHostDraftPremium`: `None` when the body does not add the token. Only the balance cap decides (no pass, no
/// once-per-day rule since 2026-10-16).
pub fn decide_draft_premium(body: &Value, balance_before: Option<i128>) -> Option<DraftPremiumDecision> {
    if !body_adds_host_draft_premium(body) {
        return None;
    }
    let adds: Vec<&Value> = items(body, "addStackedItems")
        .iter()
        .filter(|item| catalog_id(item) == Some(HOST_DRAFT_PREMIUM_ID))
        .collect();
    if adds.len() != 1 || !is_host_draft_premium_add(adds[0]) {
        return Some(DraftPremiumDecision::refused("ONE_WHOLE_TOKEN_PER_TRANSACTION"));
    }
    let Some(before) = balance_before else {
        return Some(DraftPremiumDecision::refused("BALANCE_UNREADABLE"));
    };
    let removed: i128 = items(body, "removeStackedItems")
        .iter()
        .filter(|item| catalog_id(item) == Some(HOST_DRAFT_PREMIUM_ID))
        .filter_map(quantity)
        .filter(|q| safe_integer(*q) && *q > 0.0)
        .map(|q| q as i128)
        .sum();
    let after = before - removed + 1;
    // A token spent and refilled in one transaction (the game's refund and re-draft) ends at the cap
    Some(DraftPremiumDecision {
        allowed: after <= HOST_DRAFT_PREMIUM_BALANCE_CAP,
        reason: if after > HOST_DRAFT_PREMIUM_BALANCE_CAP {
            "OVER_BALANCE_CAP"
        } else {
            "WITHIN_BALANCE_CAP"
        },
        balance: Some((before, after)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// hostGrantPolicy.test.ts's synthetic catalogue.
    struct Catalog {
        gated: Vec<&'static str>,
    }

    impl HostGrantCatalog for Catalog {
        fn tags_of(&self, catalog_id: &str) -> Option<Vec<String>> {
            let tags: &[&str] = match catalog_id {
                "WP_AC_ARMSTRONG" => &["WEAPON", "transmog", "owned"],
                "STRANGE_NEW_EMOTE" => &["EMOTE", "emote"],
                "CURRENCY_NEW_SEASON_COIN" => &["currency", "seasonal_currency"],
                _ => return None,
            };
            Some(tags.iter().map(|tag| (*tag).to_owned()).collect())
        }
        fn is_prestige_reward(&self, catalog_id: &str) -> bool {
            matches!(catalog_id, "CURRENCY_SEASONAL_COIN" | "CURRENCY_PJM_PRESTIGE_FILLED")
        }
        fn is_prestige_gated(&self, catalog_id: &str) -> bool {
            self.gated.contains(&catalog_id)
        }
        fn is_entitlement_name(&self, name: &str) -> bool {
            matches!(name, "season43_premium" | "character_facepaint_furious")
        }
    }

    const CATALOG: Catalog = Catalog { gated: Vec::new() };

    fn catalog() -> Catalog {
        Catalog {
            gated: vec!["CURRENCY_PRESTIGE", "CURRENCY_PJM_PRESTIGE_FILLED"],
        }
    }

    const HUNT: GrantSession = GrantSession {
        no_completion: false,
        realm_hub: false,
    };
    const TRAINING_HUNT: GrantSession = GrantSession {
        no_completion: true,
        realm_hub: false,
    };
    const HUB: GrantSession = GrantSession {
        no_completion: true,
        realm_hub: true,
    };
    const NONE: Allowances = Allowances { draft_premium: false };
    const ALLOWED: Allowances = Allowances { draft_premium: true };

    fn classes(session: GrantSession, body: &Value, allowances: Allowances) -> Option<Vec<&'static str>> {
        grant_denial(session, body, &catalog(), allowances).map(|denial| denial.classes)
    }

    fn found(body: &Value, allowances: Allowances) -> Vec<String> {
        violations(body, &catalog(), allowances)
            .iter()
            .map(|v| format!("{}:{}:{}", v.catalog_id, v.denied_class.as_str(), v.field.as_str()))
            .collect()
    }

    #[test]
    fn hosts_may_never_grant_platinum_hunt_pass_items_cosmetics_or_entitlements() {
        let catalog = catalog();
        for id in [
            "CURRENCY_PLATINUM",
            "CURRENCY_PLATINUM_EPIC_BONUS",
            "id_currency_platinum",
            "DEBUG_CURRENCY_PLATINUM_10K",
        ] {
            assert_eq!(classify(id, &catalog), Some(DenyClass::PremiumCurrency), "{id}");
        }
        for id in [
            "CURRENCY_SEASONAL_COIN",
            "CURRENCY_REWARDCACHE",
            "CURRENCY_S19_COIN",
            "CURRENCY_S13_DAILY",
            "CURRENCY_PRESTIGE",
            "CURRENCY_PJM_PRESTIGE_FILLED",
            "GrantProgressItem_HuntPass",
            "TOKEN_HUNTPASS_PRESTIGE_S08A",
            "TOKEN_HUNT_PASS_TUTORIAL",
            "TOKEN_DAILY_CHALLENGE_DRAFT_PREMIUM",
            "CONTAINER_CORE_PRESTIGE_HP08A_00",
            "CURRENCY_NEW_SEASON_COIN",
        ] {
            assert_eq!(classify(id, &catalog), Some(DenyClass::HuntPass), "{id}");
        }
        for id in [
            "DYE_BABYBLUE_DURABLE",
            "EM_INTRO_3POINT_00",
            "EJ_PLAYER_OMG",
            "TITLE_PHOENIX_00",
            "BNC_ANIMATION_001",
            "BN_BEGINNER_00",
            "AC_BACK_TRIALS_00",
            "GD_FRAME_A_BASE",
            "CC_HAIR_SPIKED",
            "TY_FIREWORK_BASE",
            "MP_PUFFLEHOP_SPRING_BASE",
            "CHANGE_APPEARANCE_TOKEN",
            "CURRENCY_TOKEN_TRANSMOG",
            "CONTAINER_PROMO_KIWO_00",
            "CONTAINER_CORE_FOUNDER_SLAYER",
            "CONTAINER_CORE_SLAYERPACK",
            "STRANGE_NEW_EMOTE",
        ] {
            assert_eq!(classify(id, &catalog), Some(DenyClass::Cosmetic), "{id}");
        }
        for id in [
            "season43_premium",
            "exchange_slot_2",
            "ent_cchd_hp09a_assassins_00",
            "pass_rewardcache_season15a",
            "character_facepaint_furious",
        ] {
            assert_eq!(classify(id, &catalog), Some(DenyClass::Entitlement), "{id}");
        }
    }

    #[test]
    fn the_loot_a_hunt_really_produces_stays_allowed() {
        for id in [
            "CURRENCY_NOTES",
            "CURRENCY_PJM_PRESTIGE_EMPTY",
            "CURRENCY_PJM_WEAPON",
            "CURRENCY_CELLDUST",
            "CURRENCY_EVENT_FROSTFALL",
            "BREAK_LERAWR_HIDE",
            "TOKEN_KILL_COUNT_LERAWR_BETA",
            "TOKEN_DAILY_CHALLENGE_DRAFT",
            "GATHER_DEFENSE_01",
            "CELL_PREVENT_STAGGER_UC",
            "CONTAINER_CORE_REWARD_DAILY",
            "Container_Core_Slayer_Bronze",
            "ARC_ZONE_0",
            "QI_ATTACK_SPEED_POTION",
            "LT_UNSEEN_01",
            "WP_AC_ARMSTRONG",
            "PART_AC_LEGENDARY_SHOCK",
            "",
        ] {
            assert_eq!(classify(id, &catalog()), None, "{id}");
        }
    }

    #[test]
    fn without_game_data_only_the_id_rules_apply() {
        assert_eq!(classify("STRANGE_NEW_EMOTE", &NoGameData), None);
        assert_eq!(classify("character_facepaint_furious", &NoGameData), None);
        assert_eq!(
            classify("CURRENCY_PLATINUM", &NoGameData),
            Some(DenyClass::PremiumCurrency)
        );
        assert_eq!(classify("DYE_BABYBLUE_DURABLE", &NoGameData), Some(DenyClass::Cosmetic));
        assert_eq!(classify("ent_x", &NoGameData), Some(DenyClass::Entitlement));
        assert_eq!(
            classify("ENT_X", &NoGameData),
            None,
            "the entitlement patterns are lower case"
        );
        assert_eq!(classify("_premium", &NoGameData), None);
        assert_eq!(classify("CURRENCY_S_COIN", &NoGameData), None);
        assert_eq!(classify("CURRENCY_S19_COINS", &NoGameData), None);
    }

    #[test]
    fn violations_cover_grants_and_removals_of_premium_currency_and_entitlements() {
        let body = json!({
            "addStackedItems": [{"catalogId": "CURRENCY_NOTES", "quantity": 100}, {"catalogId": "CURRENCY_PLATINUM", "quantity": 1000}],
            "addInstancedItems": [{"catalogId": "DYE_BABYBLUE_DURABLE", "instanceId": "i"}],
            "removeStackedItems": [{"catalogId": "CURRENCY_PLATINUM_EPIC", "quantity": 5},
                {"catalogId": "TOKEN_DAILY_CHALLENGE_DRAFT_PREMIUM", "quantity": 1}],
            "removeInstancedItems": [{"catalogId": "exchange_slot_2", "instanceId": "j"}]
        });
        assert_eq!(
            found(&body, NONE),
            [
                "CURRENCY_PLATINUM:PREMIUM_CURRENCY:addStackedItems",
                "DYE_BABYBLUE_DURABLE:COSMETIC:addInstancedItems",
                "exchange_slot_2:ENTITLEMENT:removeInstancedItems"
            ]
        );
        let loot = json!({"addStackedItems": [{"catalogId": "CURRENCY_NOTES", "quantity": 100},
            {"catalogId": "BREAK_LERAWR_HIDE", "quantity": 4}]});
        assert!(found(&loot, NONE).is_empty());
        assert_eq!(classes(HUNT, &loot, NONE), None);
        assert_eq!(
            classes(HUNT, &body, NONE),
            Some(vec!["PREMIUM_CURRENCY", "COSMETIC", "ENTITLEMENT"])
        );
        let denial = grant_denial(HUNT, &body, &catalog(), NONE).unwrap();
        assert_eq!(
            denial.summary,
            "CURRENCY_PLATINUM(PREMIUM_CURRENCY,addStackedItems),DYE_BABYBLUE_DURABLE(COSMETIC,addInstancedItems),\
             exchange_slot_2(ENTITLEMENT,removeInstancedItems)"
        );
    }

    #[test]
    fn the_summary_names_sixteen_entries_and_counts_the_rest() {
        let adds: Vec<Value> = (0..18)
            .map(|n| json!({"catalogId": format!("DYE_{n}"), "quantity": 1}))
            .collect();
        let denial = grant_denial(HUNT, &json!({ "addStackedItems": adds }), &catalog(), NONE).unwrap();
        assert!(
            denial.summary.ends_with("DYE_15(COSMETIC,addStackedItems),+2"),
            "{}",
            denial.summary
        );
        assert_eq!(denial.classes, ["COSMETIC"]);
    }

    #[test]
    fn a_no_reward_session_refuses_every_grant_and_spend_but_keeps_saves() {
        let loot = json!({"addStackedItems": [{"catalogId": "CURRENCY_NOTES", "quantity": 100}]});
        assert_eq!(classes(TRAINING_HUNT, &loot, NONE), Some(vec!["NO_REWARD_SESSION"]));
        let spend = json!({"removeStackedItems": [{"catalogId": "QI_ATTACK_SPEED_POTION", "quantity": 1}]});
        assert_eq!(classes(TRAINING_HUNT, &spend, NONE), Some(vec!["NO_REWARD_SESSION"]));
        let save =
            json!({"saveInstancedItems": [{"catalogId": "WP_AC_ARMSTRONG", "instanceId": "w", "itemData": "{}"}]});
        assert_eq!(classes(TRAINING_HUNT, &save, NONE), None);
        let coin = json!({"addStackedItems": [{"catalogId": "CURRENCY_SEASONAL_COIN", "quantity": 400}]});
        assert_eq!(classes(TRAINING_HUNT, &coin, NONE), Some(vec!["NO_REWARD_SESSION"]));
    }

    #[test]
    fn a_claim_funded_season_coin_is_a_whole_stacked_add_of_one_to_a_thousand() {
        for quantity in [1, 400, 1000] {
            let body = json!({"addStackedItems": [{"catalogId": "CURRENCY_SEASONAL_COIN", "quantity": quantity}]});
            assert!(is_claim_funded_coin_grant(&body["addStackedItems"][0]));
            assert!(found(&body, NONE).is_empty());
            assert_eq!(classes(HUNT, &body, NONE), None);
            assert_eq!(classes(HUB, &body, NONE), None);
        }
        let with_loot = json!({"addStackedItems": [{"catalogId": "CURRENCY_NOTES", "quantity": 100},
            {"catalogId": "CURRENCY_SEASONAL_COIN", "quantity": 400}]});
        assert!(found(&with_loot, NONE).is_empty());
        for quantity in [
            json!(0),
            json!(-5),
            json!(1001),
            json!(100_000),
            json!(1.5),
            json!(9_007_199_254_740_993_u64),
            json!("400"),
            Value::Null,
        ] {
            let body = json!({"addStackedItems": [{"catalogId": "CURRENCY_SEASONAL_COIN", "quantity": quantity}]});
            assert!(!is_claim_funded_coin_grant(&body["addStackedItems"][0]), "{quantity}");
            assert_eq!(found(&body, NONE), ["CURRENCY_SEASONAL_COIN:HUNT_PASS:addStackedItems"]);
        }
        let missing = json!({"addStackedItems": [{"catalogId": "CURRENCY_SEASONAL_COIN"}]});
        assert_eq!(found(&missing, NONE).len(), 1);
        let instanced = json!({"addInstancedItems": [{"catalogId": "CURRENCY_SEASONAL_COIN", "instanceId": "i"}]});
        assert_eq!(
            found(&instanced, NONE),
            ["CURRENCY_SEASONAL_COIN:HUNT_PASS:addInstancedItems"]
        );
        for id in [
            "currency_seasonal_coin",
            "id_currency_seasonal_coin",
            "Currency_Seasonal_Coin",
            " CURRENCY_SEASONAL_COIN",
        ] {
            let body = json!({"addStackedItems": [{"catalogId": id, "quantity": 400}]});
            assert!(!is_claim_funded_coin_grant(&body["addStackedItems"][0]), "{id}");
            assert_eq!(found(&body, NONE).len(), 1, "{id}");
        }
        assert_eq!(
            classes(
                HUB,
                &json!({"addStackedItems": [{"catalogId": "CURRENCY_SEASONAL_COIN", "quantity": 1001}]}),
                NONE
            ),
            Some(vec!["HUNT_PASS"])
        );
    }

    #[test]
    fn every_other_hunt_pass_item_stays_denied_next_to_funded_grants() {
        let body = json!({
            "addStackedItems": [
                {"catalogId": "CURRENCY_SEASONAL_COIN", "quantity": 400},
                {"catalogId": "CURRENCY_REWARDCACHE", "quantity": 400},
                {"catalogId": "CURRENCY_S19_COIN", "quantity": 10},
                {"catalogId": "CURRENCY_PRESTIGE", "quantity": 10},
                {"catalogId": "CURRENCY_PJM_PRESTIGE_FILLED", "quantity": 1},
                {"catalogId": "TOKEN_DAILY_CHALLENGE_DRAFT_PREMIUM", "quantity": 1},
                {"catalogId": "CURRENCY_PLATINUM", "quantity": 1000},
                {"catalogId": "DYE_BABYBLUE_DURABLE", "quantity": 1}
            ],
            "addInstancedItems": [{"catalogId": "exchange_slot_2", "instanceId": "i"}]
        });
        let classes: Vec<String> = violations(&body, &catalog(), NONE)
            .iter()
            .map(|v| format!("{}:{}", v.catalog_id, v.denied_class.as_str()))
            .collect();
        assert_eq!(
            classes,
            [
                "CURRENCY_REWARDCACHE:HUNT_PASS",
                "CURRENCY_S19_COIN:HUNT_PASS",
                "TOKEN_DAILY_CHALLENGE_DRAFT_PREMIUM:HUNT_PASS",
                "CURRENCY_PLATINUM:PREMIUM_CURRENCY",
                "DYE_BABYBLUE_DURABLE:COSMETIC",
                "exchange_slot_2:ENTITLEMENT"
            ]
        );
    }

    #[test]
    fn a_prestige_gated_reward_passes_as_a_whole_positive_stacked_add_of_a_hunt_pass_id() {
        for id in ["CURRENCY_PRESTIGE", "CURRENCY_PJM_PRESTIGE_FILLED"] {
            for quantity in [1, 5, 100] {
                let item = json!({"catalogId": id, "quantity": quantity});
                assert!(is_prestige_funded_grant(&item, &catalog()));
                let body = json!({ "addStackedItems": [item] });
                assert_eq!(classes(HUNT, &body, NONE), None);
            }
        }
        assert!(!is_prestige_funded_grant(
            &json!({"catalogId": "CURRENCY_PRESTIGE", "quantity": 1}),
            &CATALOG
        ));
        for quantity in [json!(0), json!(-1), json!(1.5), json!("1"), Value::Null] {
            let body = json!({"addStackedItems": [{"catalogId": "CURRENCY_PRESTIGE", "quantity": quantity}]});
            assert_eq!(
                found(&body, NONE),
                ["CURRENCY_PRESTIGE:HUNT_PASS:addStackedItems"],
                "{quantity}"
            );
        }
        let instanced =
            json!({"addInstancedItems": [{"catalogId": "CURRENCY_PJM_PRESTIGE_FILLED", "instanceId": "i"}]});
        assert_eq!(
            found(&instanced, NONE),
            ["CURRENCY_PJM_PRESTIGE_FILLED:HUNT_PASS:addInstancedItems"]
        );
        for id in ["currency_prestige", "id_currency_prestige", "CURRENCY_PRESTIGE "] {
            assert!(
                !is_prestige_funded_grant(&json!({"catalogId": id, "quantity": 1}), &catalog()),
                "{id}"
            );
        }
        // Removals are never waived by it, and a hosted Training hunt still refuses it
        assert!(
            found(
                &json!({"removeStackedItems": [{"catalogId": "CURRENCY_PRESTIGE", "quantity": 2}]}),
                NONE
            )
            .is_empty()
        );
        let filled = json!({"addStackedItems": [{"catalogId": "CURRENCY_PJM_PRESTIGE_FILLED", "quantity": 1}]});
        assert_eq!(classes(TRAINING_HUNT, &filled, NONE), Some(vec!["NO_REWARD_SESSION"]));
        assert_eq!(classes(HUB, &filled, NONE), None);
    }

    #[test]
    fn a_gated_id_classed_otherwise_stays_denied() {
        let odd = Catalog {
            gated: vec!["CURRENCY_PLATINUM", "DYE_BABYBLUE_DURABLE", "season43_premium"],
        };
        for (id, class) in [
            ("CURRENCY_PLATINUM", DenyClass::PremiumCurrency),
            ("DYE_BABYBLUE_DURABLE", DenyClass::Cosmetic),
            ("season43_premium", DenyClass::Entitlement),
        ] {
            let body = json!({"addStackedItems": [{"catalogId": id, "quantity": 1}]});
            let found = violations(&body, &odd, NONE);
            assert_eq!(found.len(), 1);
            assert_eq!(found[0].denied_class, class);
        }
    }

    #[test]
    fn a_realm_hub_grants_as_a_hunt_does_and_keeps_the_class_deny_list() {
        let loot = json!({"addStackedItems": [{"catalogId": "CURRENCY_NOTES", "quantity": 100}]});
        assert_eq!(classes(HUB, &loot, NONE), None);
        let spend = json!({"removeStackedItems": [{"catalogId": "BREAK_LERAWR_HIDE", "quantity": 3}],
            "addStackedItems": [{"catalogId": "CURRENCY_NOTES", "quantity": 2}]});
        assert_eq!(classes(HUB, &spend, NONE), None);
        assert_eq!(
            classes(
                HUB,
                &json!({"addStackedItems": [{"catalogId": "CURRENCY_PLATINUM", "quantity": 1000}]}),
                NONE
            ),
            Some(vec!["PREMIUM_CURRENCY"])
        );
        assert_eq!(
            classes(
                HUB,
                &json!({"addInstancedItems": [{"catalogId": "DYE_BABYBLUE_DURABLE", "instanceId": "d"}]}),
                NONE
            ),
            Some(vec!["COSMETIC"])
        );
        assert_eq!(
            classes(
                HUB,
                &json!({"removeInstancedItems": [{"catalogId": "exchange_slot_2", "instanceId": "j"}]}),
                NONE
            ),
            Some(vec!["ENTITLEMENT"])
        );
    }

    #[test]
    fn a_stacked_platinum_removal_is_a_spend_and_a_platinum_grant_is_denied_everywhere() {
        let spend = json!({"removeStackedItems": [{"catalogId": "CURRENCY_PLATINUM_EPIC", "quantity": 5},
            {"catalogId": "CURRENCY_PLATINUM", "quantity": 40}, {"catalogId": "id_currency_platinum", "quantity": 1}]});
        assert!(found(&spend, NONE).is_empty());
        assert_eq!(classes(HUNT, &spend, NONE), None);
        assert_eq!(classes(HUB, &spend, NONE), None);
        assert_eq!(classes(TRAINING_HUNT, &spend, NONE), Some(vec!["NO_REWARD_SESSION"]));
        for id in ["CURRENCY_PLATINUM_EPIC", "CURRENCY_PLATINUM", "id_currency_platinum"] {
            let grant = json!({"addStackedItems": [{"catalogId": id, "quantity": 1}]});
            assert_eq!(classes(HUB, &grant, NONE), Some(vec!["PREMIUM_CURRENCY"]), "{id}");
        }
        let debug = json!({"removeStackedItems": [{"catalogId": "DEBUG_CURRENCY_PLATINUM_10K", "quantity": 1}]});
        assert_eq!(classes(HUNT, &debug, NONE), Some(vec!["PREMIUM_CURRENCY"]));
        let instanced = json!({"removeInstancedItems": [{"catalogId": "CURRENCY_PLATINUM_EPIC", "instanceId": "x"}]});
        assert_eq!(classes(HUNT, &instanced, NONE), Some(vec!["PREMIUM_CURRENCY"]));
    }

    #[test]
    fn the_draft_premium_token_is_exempt_only_as_one_whole_stacked_add_the_decision_allowed() {
        let token = json!({"catalogId": HOST_DRAFT_PREMIUM_ID, "quantity": 1});
        assert!(is_host_draft_premium_add(&token));
        assert!(is_host_draft_premium_add(
            &json!({"catalogId": HOST_DRAFT_PREMIUM_ID, "quantity": 1.0})
        ));
        assert!(!is_host_draft_premium_add(
            &json!({"catalogId": HOST_DRAFT_PREMIUM_ID, "quantity": 2})
        ));
        assert!(!is_host_draft_premium_add(
            &json!({"catalogId": "token_daily_challenge_draft_premium", "quantity": 1})
        ));
        let body = json!({ "addStackedItems": [token] });
        assert_eq!(
            found(&body, NONE),
            ["TOKEN_DAILY_CHALLENGE_DRAFT_PREMIUM:HUNT_PASS:addStackedItems"]
        );
        assert!(found(&body, ALLOWED).is_empty());
        assert_eq!(classes(HUNT, &body, ALLOWED), None);
        assert_eq!(classes(HUB, &body, ALLOWED), None);
        assert_eq!(classes(HUB, &body, NONE), Some(vec!["HUNT_PASS"]));
        assert_eq!(classes(TRAINING_HUNT, &body, ALLOWED), Some(vec!["NO_REWARD_SESSION"]));

        let with_cache = json!({"addStackedItems": [token, {"catalogId": "CURRENCY_REWARDCACHE", "quantity": 10}]});
        assert_eq!(
            found(&with_cache, ALLOWED),
            ["CURRENCY_REWARDCACHE:HUNT_PASS:addStackedItems"]
        );
        assert_eq!(found(&json!({"addStackedItems": [token, token]}), ALLOWED).len(), 2);
        for quantity in [json!(2), json!(0), json!(-1), json!(1.5)] {
            let body = json!({"addStackedItems": [{"catalogId": HOST_DRAFT_PREMIUM_ID, "quantity": quantity}]});
            assert_eq!(found(&body, ALLOWED).len(), 1, "{quantity}");
        }
        let instanced = json!({"addInstancedItems": [{"catalogId": HOST_DRAFT_PREMIUM_ID, "instanceId": "i"}]});
        assert_eq!(found(&instanced, ALLOWED).len(), 1);
        assert!(found(&json!({ "removeStackedItems": [token] }), ALLOWED).is_empty());
    }

    #[test]
    fn the_starter_grant_passes_and_only_its_ids_one_each() {
        let mut adds: Vec<Value> = HOST_STARTER_COSMETIC_IDS
            .iter()
            .map(|id| json!({"catalogId": id, "quantity": 1}))
            .collect();
        adds.push(json!({"catalogId": "TOKEN_OB_PROGRESSION", "quantity": 1}));
        adds.push(json!({"catalogId": "TOKEN_DAILY_CHALLENGE_DRAFT", "quantity": 4}));
        adds.push(json!({"catalogId": "TOKEN_WEEKLY_CHALLENGE_DRAFT", "quantity": 4}));
        let grant = json!({ "addStackedItems": adds });
        assert!(found(&grant, NONE).is_empty());
        assert_eq!(classes(HUB, &grant, NONE), None);
        let instanced =
            json!({"addInstancedItems": [{"catalogId": "EM_INTRO_GENERIC_001", "instanceId": "EM_INTRO_GENERIC_001"}]});
        assert!(found(&instanced, NONE).is_empty());
        let refused = json!({"addStackedItems": [
            {"catalogId": "EM_INTRO_GENERIC_001", "quantity": 2},
            {"catalogId": "EM_INTRO_3POINT_00", "quantity": 1},
            {"catalogId": "TITLE_PHOENIX_00", "quantity": 1},
            {"catalogId": "em_intro_generic_001", "quantity": 1},
            {"catalogId": "EM_INTRO_GENERIC_002", "quantity": null}
        ]});
        let refused = violations(&refused, &catalog(), NONE);
        assert_eq!(refused.len(), 5);
        assert!(refused.iter().all(|v| v.denied_class == DenyClass::Cosmetic));
    }

    #[test]
    fn malformed_entries_are_skipped_not_trusted() {
        let body = json!({"addStackedItems": [null, 3, "DYE_X", [], {"catalogId": 5}, {"catalogId": "DYE_X"}],
            "addInstancedItems": {"catalogId": "DYE_Y"}});
        assert_eq!(found(&body, NONE), ["DYE_X:COSMETIC:addStackedItems"]);
        assert!(!body_has_grant_or_spend(&json!([])));
        assert!(!body_has_grant_or_spend(&json!({"addStackedItems": []})));
        assert!(body_has_grant_or_spend(&json!({"removeInstancedItems": [null]})));
    }

    #[test]
    fn a_balance_is_the_stacked_quantity_and_an_unreadable_list_is_none() {
        let id = HOST_DRAFT_PREMIUM_ID;
        assert_eq!(stacked_balance_of(None, id), Some(0));
        assert_eq!(
            stacked_balance_of(Some(r#"[{"catalogId":"CURRENCY_NOTES","quantity":5}]"#), id),
            Some(0)
        );
        let one = format!(r#"[{{"catalogId":"{id}","quantity":1}}]"#);
        assert_eq!(stacked_balance_of(Some(&one), id), Some(1));
        let five = format!(r#"[{{"catalogId":"{id}","quantity":2}},{{"catalogId":"{id}","quantity":3}}]"#);
        assert_eq!(stacked_balance_of(Some(&five), id), Some(5));
        assert_eq!(stacked_balance_of(Some("not json"), id), None);
        assert_eq!(
            stacked_balance_of(Some(&format!(r#"{{"catalogId":"{id}"}}"#)), id),
            None
        );
        assert_eq!(
            stacked_balance_of(Some(&format!(r#"[{{"catalogId":"{id}","quantity":1.5}}]"#)), id),
            None
        );
        assert_eq!(
            stacked_balance_of(Some(&format!(r#"[{{"catalogId":"{id}","quantity":-1}}]"#)), id),
            None
        );
        assert_eq!(
            stacked_balance_of(Some(&format!(r#"[null,{{"catalogId":"{id}","quantity":1}}]"#)), id),
            Some(1)
        );
    }

    fn decide(body: &Value, before: Option<i128>) -> Option<DraftPremiumDecision> {
        decide_draft_premium(body, before)
    }

    #[test]
    fn the_draft_premium_decision_is_the_balance_cap() {
        let token = json!({"catalogId": HOST_DRAFT_PREMIUM_ID, "quantity": 1});
        assert_eq!(
            decide(
                &json!({"addStackedItems": [{"catalogId": "CURRENCY_NOTES", "quantity": 100}]}),
                Some(0)
            ),
            None
        );
        assert_eq!(
            decide(&json!({ "removeStackedItems": [token] }), Some(0)),
            None,
            "a removal is not a grant"
        );
        let instanced = json!({"addInstancedItems": [{"catalogId": HOST_DRAFT_PREMIUM_ID, "instanceId": "i"}]});
        assert_eq!(
            decide(&instanced, Some(0)),
            None,
            "the policy refuses it, not the decision"
        );

        let body = json!({ "addStackedItems": [token] });
        assert_eq!(
            decide(&body, Some(0)),
            Some(DraftPremiumDecision {
                allowed: true,
                reason: "WITHIN_BALANCE_CAP",
                balance: Some((0, 1))
            })
        );
        assert_eq!(
            decide(&body, Some(1)),
            Some(DraftPremiumDecision {
                allowed: false,
                reason: "OVER_BALANCE_CAP",
                balance: Some((1, 2))
            })
        );
        let refill = json!({"addStackedItems": [token], "removeStackedItems": [{"catalogId": HOST_DRAFT_PREMIUM_ID, "quantity": 1}]});
        assert_eq!(
            decide(&refill, Some(1)).map(|d| (d.allowed, d.balance)),
            Some((true, Some((1, 1))))
        );
        assert_eq!(
            decide(&body, None),
            Some(DraftPremiumDecision {
                allowed: false,
                reason: "BALANCE_UNREADABLE",
                balance: None
            }),
            "never taken as zero"
        );
        for body in [
            json!({"addStackedItems": [token, token]}),
            json!({"addStackedItems": [{"catalogId": HOST_DRAFT_PREMIUM_ID, "quantity": 2}]}),
            json!({"addStackedItems": [{"catalogId": HOST_DRAFT_PREMIUM_ID, "quantity": 0}]}),
            json!({"addStackedItems": [{"catalogId": HOST_DRAFT_PREMIUM_ID, "quantity": 1.5}]}),
        ] {
            assert_eq!(
                decide(&body, Some(0)).map(|d| (d.allowed, d.reason)),
                Some((false, "ONE_WHOLE_TOKEN_PER_TRANSACTION")),
                "{body}"
            );
        }
        assert_eq!(decide(&body, Some(0)).unwrap().balance_tags(), " balance=0->1 cap=1");
    }
}
