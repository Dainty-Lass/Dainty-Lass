// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Which items a player host may grant in an inventory transaction, and the daily premium draft token's cap.
//! Pure functions over the body of `POST /inventory`.

use rifts_seam::HostGrantCatalog;
use serde_json::Value;

use crate::json::{js_trim, safe_integer};

pub const HOST_DRAFT_PREMIUM_ID: &str = "TOKEN_DAILY_CHALLENGE_DRAFT_PREMIUM";
pub const HOST_DRAFT_PREMIUM_BALANCE_CAP: i128 = 1;

/// The cosmetics every new character gets, which a host may hand out again.
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

/// What a hunt's challenge claim may pay out, so a host may grant it.
pub const CHALLENGE_REWARD_CURRENCIES: [&str; 1] = ["CURRENCY_SEASONAL_COIN"];
pub const MAX_CHALLENGE_REWARD_PER_CLAIM: i64 = 1000;

const PLATINUM_IDS: [&str; 13] = [
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
const HUNT_PASS_PREFIXES: [&str; 3] = ["TOKEN_HUNTPASS", "TOKEN_HUNT_PASS", "CONTAINER_CORE_PRESTIGE_"];

const COSMETIC_IDS: [&str; 3] = [
    "CHANGE_APPEARANCE_TOKEN",
    "CURRENCY_TOKEN_TRANSMOG",
    "CONTAINER_CORE_SLAYERPACK",
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
const ENTITLEMENT_HEADS: [&str; 5] = ["ent", "pass", "eventpass", "minipass", "exchange_slot"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DenyClass {
    PremiumCurrency,
    HuntPass,
    Cosmetic,
    Entitlement,
}

impl DenyClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PremiumCurrency => "PREMIUM_CURRENCY",
            Self::HuntPass => "HUNT_PASS",
            Self::Cosmetic => "COSMETIC",
            Self::Entitlement => "ENTITLEMENT",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GrantField {
    AddStackedItems,
    AddInstancedItems,
    RemoveStackedItems,
    RemoveInstancedItems,
}

impl GrantField {
    pub const ALL: [Self; 4] = [
        Self::AddStackedItems,
        Self::AddInstancedItems,
        Self::RemoveStackedItems,
        Self::RemoveInstancedItems,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::AddStackedItems => "addStackedItems",
            Self::AddInstancedItems => "addInstancedItems",
            Self::RemoveStackedItems => "removeStackedItems",
            Self::RemoveInstancedItems => "removeInstancedItems",
        }
    }

    fn is_add(self) -> bool {
        matches!(self, Self::AddStackedItems | Self::AddInstancedItems)
    }
}

/// A catalogue that knows nothing: no tags, nothing prestige, no entitlement names.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoGameData;

impl HostGrantCatalog for NoGameData {
    fn tags_of(&self, _catalog_id: &str) -> Option<Vec<String>> {
        None
    }

    fn is_prestige_reward(&self, _catalog_id: &str) -> bool {
        false
    }

    fn is_prestige_gated(&self, _catalog_id: &str) -> bool {
        false
    }

    fn is_entitlement_name(&self, _name: &str) -> bool {
        false
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    pub catalog_id: String,
    pub denied_class: DenyClass,
    pub field: GrantField,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantDenial {
    pub classes: Vec<&'static str>,
    pub violations: Vec<Violation>,
    pub summary: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GrantSession {
    pub no_completion: bool,
    pub realm_hub: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Allowances {
    pub draft_premium: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DraftPremiumDecision {
    pub allowed: bool,
    pub reason: &'static str,
    /// `(before, after)` when a balance was read.
    pub balance: Option<(i128, i128)>,
}

impl DraftPremiumDecision {
    pub fn balance_tags(&self) -> String {
        self.balance
            .map(|(before, after)| format!(" balance={before}->{after} cap={HOST_DRAFT_PREMIUM_BALANCE_CAP}"))
            .unwrap_or_default()
    }
}

fn starts_with_ignore_case(text: &str, prefix: &str) -> bool {
    text.len() >= prefix.len()
        && text.is_char_boundary(prefix.len())
        && text[..prefix.len()].eq_ignore_ascii_case(prefix)
}

/// `id_currency_x` is the same item as `CURRENCY_X`.
fn canonical_id(catalog_id: &str) -> String {
    let id = js_trim(catalog_id);
    if starts_with_ignore_case(id, "id_currency_") {
        format!("CURRENCY_{}", &id["id_currency_".len()..]).to_uppercase()
    } else {
        id.to_owned()
    }
}

/// `CURRENCY_S<digits>_COIN` or `_DAILY`: a season's currencies.
fn is_season_currency(up: &str) -> bool {
    let Some(rest) = up.strip_prefix("CURRENCY_S") else {
        return false;
    };
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    digits > 0 && matches!(&rest[digits..], "_COIN" | "_DAILY")
}

fn entitlement_tail(tail: &str) -> bool {
    !tail.is_empty()
        && tail
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'.' | b'-'))
}

fn is_entitlement_id(id: &str) -> bool {
    let headed = ENTITLEMENT_HEADS.iter().any(|head| {
        id.strip_prefix(head)
            .and_then(|rest| rest.strip_prefix('_'))
            .is_some_and(entitlement_tail)
    });
    headed || id.strip_suffix("_premium").is_some_and(entitlement_tail)
}

pub fn classify(catalog_id: &str, catalog: &dyn HostGrantCatalog) -> Option<DenyClass> {
    let id = canonical_id(catalog_id);
    if id.is_empty() {
        return None;
    }
    let up = id.to_uppercase();
    let tags: Vec<String> = catalog
        .tags_of(&id)
        .unwrap_or_default()
        .iter()
        .map(|tag| tag.to_lowercase())
        .collect();
    let tagged = |names: &[&str]| tags.iter().any(|tag| names.contains(&tag.as_str()));

    if up.starts_with("CURRENCY_PLATINUM")
        || up.starts_with("DEBUG_")
        || PLATINUM_IDS.contains(&id.as_str())
        || tagged(&["platinum"])
    {
        return Some(DenyClass::PremiumCurrency);
    }
    if HUNT_PASS_IDS.contains(&up.as_str())
        || is_season_currency(&up)
        || HUNT_PASS_PREFIXES.iter().any(|p| up.starts_with(p))
        || up.ends_with("_DRAFT_PREMIUM")
        || catalog.is_prestige_reward(&id)
        || tagged(&["seasonal_currency"])
    {
        return Some(DenyClass::HuntPass);
    }
    if COSMETIC_IDS.contains(&up.as_str())
        || COSMETIC_PREFIXES.iter().any(|p| up.starts_with(p))
        || tagged(&COSMETIC_TAGS)
    {
        return Some(DenyClass::Cosmetic);
    }
    if catalog.is_entitlement_name(&id.to_lowercase()) || is_entitlement_id(&id) {
        return Some(DenyClass::Entitlement);
    }
    None
}

fn items(body: &Value, field: GrantField) -> &[Value] {
    body.get(field.as_str())
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn catalog_id(item: &Value) -> Option<&str> {
    item.get("catalogId").and_then(Value::as_str)
}

/// `quantity` only when it is a JSON number.
fn quantity(item: &Value) -> Option<&Value> {
    item.get("quantity").filter(|q| q.is_number())
}

pub fn is_claim_funded_coin_grant(item: &Value) -> bool {
    catalog_id(item).is_some_and(|id| CHALLENGE_REWARD_CURRENCIES.contains(&id))
        && quantity(item)
            .and_then(safe_integer)
            .is_some_and(|q| (1..=MAX_CHALLENGE_REWARD_PER_CLAIM).contains(&q))
}

pub fn is_prestige_funded_grant(item: &Value, catalog: &dyn HostGrantCatalog) -> bool {
    let Some(id) = catalog_id(item) else {
        return false;
    };
    catalog.is_prestige_gated(id)
        && quantity(item).and_then(safe_integer).is_some_and(|q| q > 0)
        && classify(id, catalog) == Some(DenyClass::HuntPass)
}

pub fn is_host_starter_cosmetic_add(item: &Value) -> bool {
    catalog_id(item).is_some_and(|id| HOST_STARTER_COSMETIC_IDS.contains(&id))
        && quantity(item).is_none_or(|q| q.as_f64() == Some(1.0))
}

pub fn is_host_draft_premium_add(item: &Value) -> bool {
    catalog_id(item) == Some(HOST_DRAFT_PREMIUM_ID)
}

pub fn body_adds_host_draft_premium(body: &Value) -> bool {
    items(body, GrantField::AddStackedItems)
        .iter()
        .any(is_host_draft_premium_add)
}

pub fn is_platinum_spend_item(item: &Value) -> bool {
    catalog_id(item).is_some_and(|id| {
        let id = canonical_id(id);
        PLATINUM_IDS.contains(&id.as_str())
    })
}

/// Any item list with an entry in it.
pub fn body_has_grant_or_spend(body: &Value) -> bool {
    body.is_object() && GrantField::ALL.iter().any(|field| !items(body, *field).is_empty())
}

fn is_exempt(item: &Value, field: GrantField, body: &Value, catalog: &dyn HostGrantCatalog, allow: Allowances) -> bool {
    if field == GrantField::AddStackedItems {
        if is_claim_funded_coin_grant(item) || is_prestige_funded_grant(item, catalog) {
            return true;
        }
        let draft_adds = items(body, field)
            .iter()
            .filter(|i| is_host_draft_premium_add(i))
            .count();
        if allow.draft_premium
            && draft_adds == 1
            && is_host_draft_premium_add(item)
            && quantity(item).and_then(Value::as_f64) == Some(1.0)
        {
            return true;
        }
    }
    if field == GrantField::RemoveStackedItems && is_platinum_spend_item(item) {
        return true;
    }
    field.is_add() && is_host_starter_cosmetic_add(item)
}

pub fn violations(body: &Value, catalog: &dyn HostGrantCatalog, allowances: Allowances) -> Vec<Violation> {
    let mut found = Vec::new();
    for field in GrantField::ALL {
        for item in items(body, field) {
            let Some(id) = catalog_id(item) else { continue };
            if is_exempt(item, field, body, catalog, allowances) {
                continue;
            }
            let Some(class) = classify(id, catalog) else { continue };
            // taking items away only matters for what costs money or unlocks content
            if !field.is_add() && !matches!(class, DenyClass::PremiumCurrency | DenyClass::Entitlement) {
                continue;
            }
            found.push(Violation {
                catalog_id: id.to_owned(),
                denied_class: class,
                field,
            });
        }
    }
    found
}

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
    let violations = violations(body, catalog, allowances);
    if violations.is_empty() {
        return None;
    }
    let mut classes: Vec<&'static str> = Vec::new();
    for violation in &violations {
        let name = violation.denied_class.as_str();
        if !classes.contains(&name) {
            classes.push(name);
        }
    }
    let mut summary = violations
        .iter()
        .take(16)
        .map(|v| format!("{}({},{})", v.catalog_id, v.denied_class.as_str(), v.field.as_str()))
        .collect::<Vec<_>>()
        .join(",");
    if violations.len() > 16 {
        summary.push_str(&format!(",+{}", violations.len() - 16));
    }
    Some(GrantDenial {
        classes,
        violations,
        summary,
    })
}

/// The stored stacked items' balance of `id`; `None` when the list cannot be read.
pub fn stacked_balance_of(stored: Option<&str>, id: &str) -> Option<i128> {
    let Some(text) = stored else {
        return Some(0);
    };
    let Ok(Value::Array(entries)) = serde_json::from_str::<Value>(text) else {
        return None;
    };
    let mut total: i128 = 0;
    for entry in &entries {
        if catalog_id(entry) != Some(id) {
            continue;
        }
        let quantity = entry.get("quantity").and_then(safe_integer).filter(|q| *q >= 0)?;
        total += i128::from(quantity);
    }
    Some(total)
}

pub fn decide_draft_premium(body: &Value, balance_before: Option<i128>) -> Option<DraftPremiumDecision> {
    let adds: Vec<&Value> = items(body, GrantField::AddStackedItems)
        .iter()
        .filter(|item| is_host_draft_premium_add(item))
        .collect();
    let refused = |reason, balance| DraftPremiumDecision {
        allowed: false,
        reason,
        balance,
    };
    match adds.as_slice() {
        [] => return None,
        [one] if quantity(one).and_then(Value::as_f64) == Some(1.0) => {}
        _ => return Some(refused("ONE_WHOLE_TOKEN_PER_TRANSACTION", None)),
    }
    let Some(before) = balance_before else {
        return Some(refused("BALANCE_UNREADABLE", None));
    };
    let spent: i128 = items(body, GrantField::RemoveStackedItems)
        .iter()
        .filter(|item| is_host_draft_premium_add(item))
        .filter_map(|item| quantity(item).and_then(safe_integer).filter(|q| *q > 0))
        .map(i128::from)
        .sum();
    let after = before - spent + 1;
    Some(DraftPremiumDecision {
        allowed: after <= HOST_DRAFT_PREMIUM_BALANCE_CAP,
        reason: if after <= HOST_DRAFT_PREMIUM_BALANCE_CAP {
            "WITHIN_BALANCE_CAP"
        } else {
            "OVER_BALANCE_CAP"
        },
        balance: Some((before, after)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A catalogue whose answers a test sets.
    #[derive(Default)]
    struct Catalog {
        tags: Vec<(&'static str, Vec<&'static str>)>,
        prestige_reward: Vec<&'static str>,
        prestige_gated: Vec<&'static str>,
        entitlements: Vec<&'static str>,
    }

    impl HostGrantCatalog for Catalog {
        fn tags_of(&self, catalog_id: &str) -> Option<Vec<String>> {
            self.tags
                .iter()
                .find(|(id, _)| *id == catalog_id)
                .map(|(_, tags)| tags.iter().map(|t| (*t).to_owned()).collect())
        }

        fn is_prestige_reward(&self, catalog_id: &str) -> bool {
            self.prestige_reward.contains(&catalog_id)
        }

        fn is_prestige_gated(&self, catalog_id: &str) -> bool {
            self.prestige_gated.contains(&catalog_id)
        }

        fn is_entitlement_name(&self, name: &str) -> bool {
            self.entitlements.contains(&name)
        }
    }

    fn class(id: &str) -> Option<&'static str> {
        classify(id, &NoGameData).map(DenyClass::as_str)
    }

    #[test]
    fn premium_currency() {
        for id in PLATINUM_IDS {
            assert_eq!(class(id), Some("PREMIUM_CURRENCY"), "{id}");
        }
        for id in [
            "currency_platinum_new",
            "Debug_Gold",
            " id_currency_platinum ",
            "ID_CURRENCY_platinum_cs",
        ] {
            assert_eq!(class(id), Some("PREMIUM_CURRENCY"), "{id}");
        }
        let tagged = Catalog {
            tags: vec![("GEM", vec!["Platinum"])],
            ..Catalog::default()
        };
        assert_eq!(classify("GEM", &tagged), Some(DenyClass::PremiumCurrency));
    }

    #[test]
    fn hunt_pass() {
        for id in [
            "CURRENCY_SEASONAL_COIN",
            "currency_rewardcache",
            "CURRENCY_PRESTIGE",
            "GRANTPROGRESSITEM_HUNTPASS",
            "CURRENCY_S12_COIN",
            "currency_s3_daily",
            "TOKEN_HUNTPASS_SKIP",
            "TOKEN_HUNT_PASS_X",
            "CONTAINER_CORE_PRESTIGE_01",
            "TOKEN_DAILY_CHALLENGE_DRAFT_PREMIUM",
            "id_currency_seasonal_coin",
        ] {
            assert_eq!(class(id), Some("HUNT_PASS"), "{id}");
        }
        for id in ["CURRENCY_S_COIN", "CURRENCY_S1_COINS", "CURRENCY_SX_DAILY"] {
            assert_eq!(class(id), None, "{id}");
        }
        let catalog = Catalog {
            tags: vec![("ORB", vec!["SEASONAL_CURRENCY"])],
            prestige_reward: vec!["CELL"],
            ..Catalog::default()
        };
        assert_eq!(classify("ORB", &catalog), Some(DenyClass::HuntPass));
        assert_eq!(classify("CELL", &catalog), Some(DenyClass::HuntPass));
    }

    #[test]
    fn cosmetic() {
        for id in COSMETIC_IDS
            .iter()
            .chain(&["dye_red", "EM_WAVE", "BNC_X", "BN_X", "CONTAINER_1P_A"])
        {
            assert_eq!(class(id), Some("COSMETIC"), "{id}");
        }
        for prefix in COSMETIC_PREFIXES {
            assert_eq!(class(&format!("{prefix}Z")), Some("COSMETIC"), "{prefix}");
        }
        for tag in COSMETIC_TAGS {
            let catalog = Catalog {
                tags: vec![("THING", vec![tag])],
                ..Catalog::default()
            };
            assert_eq!(classify("THING", &catalog), Some(DenyClass::Cosmetic), "{tag}");
        }
    }

    #[test]
    fn entitlement() {
        for id in [
            "ent_x",
            "pass_s1",
            "eventpass_a.b",
            "minipass_1-2",
            "exchange_slot_3",
            "season1_premium",
        ] {
            assert_eq!(class(id), Some("ENTITLEMENT"), "{id}");
        }
        for id in ["ent_", "ENT_X", "pass", "_premium", "x_Premium", "ent_A"] {
            assert_eq!(class(id), None, "{id}");
        }
        let catalog = Catalog {
            entitlements: vec!["deluxe"],
            ..Catalog::default()
        };
        assert_eq!(classify("DELUXE", &catalog), Some(DenyClass::Entitlement));
    }

    #[test]
    fn no_game_data_leaves_plain_items_alone() {
        for id in ["", "   ", "WEAPON_SWORD_01", "CURRENCY_GOLD", "id_currency_gold"] {
            assert_eq!(class(id), None, "{id:?}");
        }
    }

    #[test]
    fn violations_and_summary() {
        let adds: Vec<Value> = (0..18).map(|i| json!({"catalogId": format!("DYE_{i:02}")})).collect();
        let body = json!({"addStackedItems": adds});
        let denial = grant_denial(GrantSession::default(), &body, &NoGameData, Allowances::default()).unwrap();
        assert_eq!(denial.classes, ["COSMETIC"]);
        assert_eq!(denial.violations.len(), 18);
        assert!(
            denial
                .summary
                .starts_with("DYE_00(COSMETIC,addStackedItems),DYE_01(COSMETIC,addStackedItems)")
        );
        assert!(denial.summary.ends_with("DYE_15(COSMETIC,addStackedItems),+2"));

        let mixed = json!({
            "addInstancedItems": [{"catalogId": "ent_x"}, {"catalogId": 7}, "x", null],
            "addStackedItems": [{"catalogId": "CURRENCY_PLATINUM", "quantity": 5}],
            "removeStackedItems": [{"catalogId": "DYE_A"}, {"catalogId": "pass_a"}],
            "removeInstancedItems": {"catalogId": "CURRENCY_PLATINUM"}
        });
        let found = violations(&mixed, &NoGameData, Allowances::default());
        let fields: Vec<(&str, &str, &str)> = found
            .iter()
            .map(|v| (v.catalog_id.as_str(), v.denied_class.as_str(), v.field.as_str()))
            .collect();
        assert_eq!(
            fields,
            [
                ("CURRENCY_PLATINUM", "PREMIUM_CURRENCY", "addStackedItems"),
                ("ent_x", "ENTITLEMENT", "addInstancedItems"),
                ("pass_a", "ENTITLEMENT", "removeStackedItems"),
            ]
        );
        let denial = grant_denial(GrantSession::default(), &mixed, &NoGameData, Allowances::default()).unwrap();
        assert_eq!(denial.classes, ["PREMIUM_CURRENCY", "ENTITLEMENT"]);
        assert_eq!(
            grant_denial(GrantSession::default(), &json!({}), &NoGameData, Allowances::default()),
            None
        );
    }

    #[test]
    fn no_reward_sessions_refuse_any_grant() {
        let session = GrantSession {
            no_completion: true,
            realm_hub: false,
        };
        let body = json!({"addStackedItems": [{"catalogId": "WEAPON"}]});
        let denial = grant_denial(session, &body, &NoGameData, Allowances::default()).unwrap();
        assert_eq!(
            (denial.classes, denial.summary.as_str()),
            (vec!["NO_REWARD_SESSION"], "any grant or spend")
        );
        assert!(
            grant_denial(
                session,
                &json!({"addStackedItems": []}),
                &NoGameData,
                Allowances::default()
            )
            .is_none()
        );
        let hub = GrantSession {
            no_completion: true,
            realm_hub: true,
        };
        assert!(grant_denial(hub, &body, &NoGameData, Allowances::default()).is_none());
        assert!(!body_has_grant_or_spend(&json!([1])));
    }

    #[test]
    fn claim_funded_coin_bounds_and_case() {
        let coin = |q: Value| json!({"catalogId": "CURRENCY_SEASONAL_COIN", "quantity": q});
        assert!(is_claim_funded_coin_grant(&coin(json!(1))));
        assert!(is_claim_funded_coin_grant(&coin(json!(1000))));
        for bad in [json!(0), json!(1001), json!(1.5), json!("5"), json!(-1)] {
            assert!(!is_claim_funded_coin_grant(&coin(bad.clone())), "{bad}");
        }
        assert!(!is_claim_funded_coin_grant(
            &json!({"catalogId": "currency_seasonal_coin", "quantity": 1})
        ));
        let body = json!({"addStackedItems": [coin(json!(5)), coin(json!(5000))]});
        let found = violations(&body, &NoGameData, Allowances::default());
        assert_eq!(found.len(), 1);
        // only in addStackedItems
        let instanced = json!({"addInstancedItems": [coin(json!(5))]});
        assert_eq!(violations(&instanced, &NoGameData, Allowances::default()).len(), 1);
    }

    #[test]
    fn prestige_gated_grants() {
        let catalog = Catalog {
            prestige_gated: vec!["TOKEN_HUNTPASS_GATED", "DYE_GATED"],
            ..Catalog::default()
        };
        let item = |id: &str, q: Value| json!({"catalogId": id, "quantity": q});
        assert!(is_prestige_funded_grant(
            &item("TOKEN_HUNTPASS_GATED", json!(2)),
            &catalog
        ));
        assert!(!is_prestige_funded_grant(
            &item("TOKEN_HUNTPASS_GATED", json!(0)),
            &catalog
        ));
        assert!(!is_prestige_funded_grant(&item("DYE_GATED", json!(1)), &catalog));
        assert!(!is_prestige_funded_grant(
            &item("TOKEN_HUNTPASS_OTHER", json!(1)),
            &catalog
        ));
        assert!(!is_prestige_funded_grant(
            &item("TOKEN_HUNTPASS_GATED", json!(1)),
            &NoGameData
        ));
    }

    #[test]
    fn platinum_spend_is_exempt_only_when_removed() {
        let spend = json!({"removeStackedItems": [{"catalogId": "id_currency_platinum", "quantity": 100}]});
        assert!(violations(&spend, &NoGameData, Allowances::default()).is_empty());
        let other = json!({"removeInstancedItems": [{"catalogId": "CURRENCY_PLATINUM"}]});
        assert_eq!(violations(&other, &NoGameData, Allowances::default()).len(), 1);
        assert!(!is_platinum_spend_item(&json!({"catalogId": "CURRENCY_PLATINUM_NEW"})));
    }

    #[test]
    fn draft_token_exemption_needs_the_allowance_and_one_whole_token() {
        let one = json!({"addStackedItems": [{"catalogId": HOST_DRAFT_PREMIUM_ID, "quantity": 1}]});
        let allow = Allowances { draft_premium: true };
        assert_eq!(violations(&one, &NoGameData, Allowances::default()).len(), 1);
        assert!(violations(&one, &NoGameData, allow).is_empty());
        let two = json!({"addStackedItems": [
            {"catalogId": HOST_DRAFT_PREMIUM_ID, "quantity": 1},
            {"catalogId": HOST_DRAFT_PREMIUM_ID, "quantity": 1}
        ]});
        assert_eq!(violations(&two, &NoGameData, allow).len(), 2);
        let many = json!({"addStackedItems": [{"catalogId": HOST_DRAFT_PREMIUM_ID, "quantity": 2}]});
        assert_eq!(violations(&many, &NoGameData, allow).len(), 1);
        // next to another Hunt Pass item the token is still exempt and the other is not
        let beside = json!({"addStackedItems": [
            {"catalogId": HOST_DRAFT_PREMIUM_ID, "quantity": 1},
            {"catalogId": "TOKEN_HUNTPASS_SKIP", "quantity": 1}
        ]});
        let found = violations(&beside, &NoGameData, allow);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].catalog_id, "TOKEN_HUNTPASS_SKIP");
        assert!(body_adds_host_draft_premium(&one));
        assert!(!body_adds_host_draft_premium(
            &json!({"removeStackedItems": [{"catalogId": HOST_DRAFT_PREMIUM_ID}]})
        ));
    }

    #[test]
    fn starter_cosmetics() {
        for id in HOST_STARTER_COSMETIC_IDS {
            assert!(is_host_starter_cosmetic_add(&json!({"catalogId": id})), "{id}");
            assert!(is_host_starter_cosmetic_add(&json!({"catalogId": id, "quantity": 1})));
            assert!(!is_host_starter_cosmetic_add(&json!({"catalogId": id, "quantity": 2})));
        }
        assert!(!is_host_starter_cosmetic_add(&json!({"catalogId": "title_blank"})));
        let body = json!({"addInstancedItems": [{"catalogId": "TITLE_BLANK"}], "addStackedItems": [{"catalogId": "EJ_SYSTEM_TYPING", "quantity": 1}]});
        assert!(violations(&body, &NoGameData, Allowances::default()).is_empty());
    }

    #[test]
    fn stacked_balance() {
        let id = HOST_DRAFT_PREMIUM_ID;
        assert_eq!(stacked_balance_of(None, id), Some(0));
        assert_eq!(stacked_balance_of(Some("{}"), id), None);
        assert_eq!(stacked_balance_of(Some("nope"), id), None);
        let list = format!(
            r#"[null, {{"catalogId":"{id}","quantity":1}}, {{"catalogId":"OTHER","quantity":-5}}, {{"catalogId":"{id}","quantity":2}}]"#
        );
        assert_eq!(stacked_balance_of(Some(&list), id), Some(3));
        let bad = format!(r#"[{{"catalogId":"{id}","quantity":-1}}]"#);
        assert_eq!(stacked_balance_of(Some(&bad), id), None);
        let missing = format!(r#"[{{"catalogId":"{id}"}}]"#);
        assert_eq!(stacked_balance_of(Some(&missing), id), None);
    }

    #[test]
    fn draft_decision_table() {
        let add = |q: Value| json!({"addStackedItems": [{"catalogId": HOST_DRAFT_PREMIUM_ID, "quantity": q}]});
        assert_eq!(decide_draft_premium(&json!({}), Some(0)), None);
        let within = decide_draft_premium(&add(json!(1)), Some(0)).unwrap();
        assert_eq!(
            (within.allowed, within.reason, within.balance),
            (true, "WITHIN_BALANCE_CAP", Some((0, 1)))
        );
        assert_eq!(within.balance_tags(), " balance=0->1 cap=1");
        let over = decide_draft_premium(&add(json!(1)), Some(1)).unwrap();
        assert_eq!(
            (over.allowed, over.reason, over.balance),
            (false, "OVER_BALANCE_CAP", Some((1, 2)))
        );
        let unreadable = decide_draft_premium(&add(json!(1)), None).unwrap();
        assert_eq!(
            (unreadable.reason, unreadable.balance_tags()),
            ("BALANCE_UNREADABLE", String::new())
        );
        assert_eq!(
            decide_draft_premium(&add(json!(2)), Some(0)).unwrap().reason,
            "ONE_WHOLE_TOKEN_PER_TRANSACTION"
        );
        let spent = json!({
            "addStackedItems": [{"catalogId": HOST_DRAFT_PREMIUM_ID, "quantity": 1}],
            "removeStackedItems": [{"catalogId": HOST_DRAFT_PREMIUM_ID, "quantity": 1}, {"catalogId": HOST_DRAFT_PREMIUM_ID, "quantity": -4}]
        });
        let swap = decide_draft_premium(&spent, Some(1)).unwrap();
        assert_eq!((swap.allowed, swap.balance), (true, Some((1, 1))));
    }
}
