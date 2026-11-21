// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The store catalog (bounties spec §4.9 to §4.13): the sections, their offers as read from the host's store files,
//! the lookup, the listing, the purchase plan and the daily window.

use std::collections::HashSet;
use std::sync::Arc;

use dainty_service_kit::{js, jsjson};
use rustic_isles::{GameDataRoot, Settings, Shared};
use serde_json::{Map, Value};

use crate::bounties::platinum::effective_price;
use crate::util::{iso, num, obj, sha_upper};

/// The lookup's order.
pub const SECTION_ORDER: [&str; 9] = [
    "exchange_vendor_slot_2",
    "exchange_vendor_slot_3",
    "ladyluckstore",
    "season_store",
    "webstore",
    "dyes",
    "hp_level_skip_rank",
    "huntpass_store",
    "fountain_daily_free_bundle",
];

/// The daily free bundle's section, claimed once per game day.
pub const DAILY_SECTION: &str = "fountain_daily_free_bundle";
const PLATINUM_SECTIONS: [&str; 4] = ["webstore", "dyes", "hp_level_skip_rank", "huntpass_store"];
const DAY_MS: i64 = 86_400_000;
const RESET_OFFSET_MS: i64 = 17 * 3_600_000;

/// Whether the section is the daily free bundle.
pub fn is_daily_section(tag: &str) -> bool {
    tag == DAILY_SECTION
}

/// One item an offer grants.
#[derive(Debug, Clone, PartialEq)]
pub struct SkuItem {
    /// The item's catalog id.
    pub catalog_id: String,
    /// As written, or `null`
    pub quantity: Value,
    /// Whether each unit is its own instance rather than a stack.
    pub instanced: bool,
}

/// One offer: the file's object kept whole for the listing, and the fields the purchase reads.
#[derive(Debug, Clone, PartialEq)]
pub struct Sku {
    /// The SKU id.
    pub id: String,
    /// The file's object, kept whole for the listing.
    pub raw: Map<String, Value>,
    /// The section tags it is listed under.
    pub tags: Vec<String>,
    /// `(currencyId as String(..), price as written or null)`
    pub prices: Vec<(String, Value)>,
    /// `maxAllowed` as written; 1 is a one-time purchase.
    pub max_allowed: Value,
    /// The items it grants.
    pub items: Vec<SkuItem>,
    /// `(name, duration as written)` of each entitlement it grants.
    pub entitlements: Vec<(String, Value)>,
    /// `skuProgression`, when set and not null.
    pub sku_progression: Option<Value>,
}

fn list_of<'a>(raw: &'a Map<String, Value>, key: &str) -> &'a [Value] {
    raw.get(key).and_then(Value::as_array).map_or(&[], Vec::as_slice)
}

impl Sku {
    /// An object with a string `id`, else none.
    pub fn read(value: &Value) -> Option<Self> {
        let raw = value.as_object()?.clone();
        let id = raw.get("id")?.as_str()?.to_owned();
        let tags = list_of(&raw, "tags")
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
        let prices = list_of(&raw, "prices")
            .iter()
            .map(|p| {
                (
                    js::string(p.get("currencyId")),
                    p.get("price").cloned().unwrap_or(Value::Null),
                )
            })
            .collect();
        let items = list_of(&raw, "items")
            .iter()
            .map(|i| SkuItem {
                catalog_id: js::string(i.get("catalogId")),
                quantity: i.get("quantity").cloned().unwrap_or(Value::Null),
                instanced: i.get("instanced") == Some(&Value::Bool(true)),
            })
            .collect();
        let entitlements = list_of(&raw, "entitlements")
            .iter()
            .map(|e| {
                (
                    js::string(e.get("name")),
                    e.get("duration").cloned().unwrap_or(Value::Null),
                )
            })
            .collect();
        let sku_progression = raw.get("skuProgression").filter(|p| !p.is_null()).cloned();
        Some(Self {
            id,
            max_allowed: raw.get("maxAllowed").cloned().unwrap_or(Value::Null),
            tags,
            prices,
            items,
            entitlements,
            sku_progression,
            raw,
        })
    }

    /// Whether the offer can be bought once only (`maxAllowed` 1).
    pub fn is_one_time(&self) -> bool {
        self.max_allowed.as_f64() == Some(1.0)
    }
}

/// `(tag, offers)` per served section, in section order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Catalog {
    sections: Vec<(String, Vec<Sku>)>,
}

fn read_offers(root: &GameDataRoot, file: &str) -> Vec<Sku> {
    match root.try_load_backend(file) {
        Ok(None) => {
            tracing::warn!(
                "[Store] game data file {file} is missing (generate it from your own installation) - its sections are \
                 empty"
            );
            Vec::new()
        }
        Ok(Some(Value::Array(list))) => list.iter().filter_map(Sku::read).collect(),
        Ok(Some(_)) => {
            tracing::warn!("[Store] {file} is not a list of offers - its sections are empty");
            Vec::new()
        }
        Err(e) => {
            tracing::warn!("[Store] {file} could not be read - its sections are empty: {e}");
            Vec::new()
        }
    }
}

/// The Middleman's slot `n` (no live capture exists).
pub fn exchange_slot(n: u8) -> Sku {
    let value = serde_json::json!({
        "id": format!("single_exchange_slot_{n}"),
        "displayName": format!("Cell Fusion Slot {n}"),
        "displayDescription": format!("Unlocks cell fusion slot {n} at the Middleman permanently."),
        "displayPriority": n,
        "prices": [{"currencyId": "id_currency_platinum", "price": 1000}],
        "maxAllowed": 1,
        "tags": [format!("exchange_vendor_slot_{n}")],
        "items": [],
        "entitlements": [{"name": format!("exchange_slot_{n}"), "duration": 0}],
        "duplicateInstancedItems": [],
    });
    Sku::read(&value).unwrap_or_else(|| Sku {
        id: String::new(),
        raw: Map::new(),
        tags: Vec::new(),
        prices: Vec::new(),
        max_allowed: Value::Null,
        items: Vec::new(),
        entitlements: Vec::new(),
        sku_progression: None,
    })
}

fn v(value: Option<&Value>) -> String {
    value.map_or_else(|| String::from("undefined"), jsjson::stringify)
}

fn prestige_gated(root: &GameDataRoot) -> HashSet<String> {
    let Ok(Some(config)) = root.try_load_backend("progression_config.json") else {
        return HashSet::new();
    };
    let doc = crate::inventory::prestige::ProgressionConfig::from_document(&config);
    let mut out = HashSet::new();
    let paths = config
        .get("payload")
        .and_then(|p| p.get("paths"))
        .and_then(Value::as_array);
    for path in paths.into_iter().flatten() {
        let rewards = path
            .get("prestige")
            .and_then(|p| p.get("free_rewards"))
            .and_then(|r| r.get("stacked_items"))
            .and_then(Value::as_array);
        for reward in rewards.into_iter().flatten() {
            if let Some(id) = reward.get("catalog_id").and_then(Value::as_str)
                && doc.is_prestige_gated_reward(id)
            {
                out.insert(id.to_owned());
            }
        }
    }
    out
}

/// `ParseFountainDailyBundle` (§4.9.4, fail-closed): the offer served, or the problems.
pub fn parse_fountain_bundle(raw: &Value, root: &GameDataRoot) -> (Vec<Sku>, Vec<String>) {
    let offers = match raw {
        Value::Array(list) => list.as_slice(),
        Value::Object(map) => match map.get("skus") {
            Some(Value::Array(list)) => list.as_slice(),
            _ => {
                return (
                    Vec::new(),
                    vec!["expected { \"skus\": [ <one offer> ] } or an array holding one offer".into()],
                );
            }
        },
        _ => {
            return (
                Vec::new(),
                vec!["expected { \"skus\": [ <one offer> ] } or an array holding one offer".into()],
            );
        }
    };
    if offers.len() != 1 {
        return (
            Vec::new(),
            vec![format!(
                "expected exactly one offer (the client only uses the first), found {}",
                offers.len()
            )],
        );
    }
    let offer = &offers[0];
    let get = |key: &str| offer.get(key);
    let mut problems = Vec::new();
    let id_ok = get("id").and_then(Value::as_str).is_some_and(|id| {
        (1..=128).contains(&id.len())
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
    });
    if !id_ok {
        problems.push(format!(
            "invalid id {} (letters, digits, _ . - only: it is a URL segment of /token/<slug>/<sku>)",
            v(get("id"))
        ));
    }
    let tagged = get("tags")
        .and_then(Value::as_array)
        .is_some_and(|t| t.iter().any(|x| x.as_str() == Some(DAILY_SECTION)));
    if !tagged {
        problems.push("tags must include fountain_daily_free_bundle".into());
    }
    let free = get("prices").and_then(Value::as_array).is_some_and(|p| {
        p.len() == 1
            && p[0].as_object().is_some_and(|m| {
                m.len() == 2
                    && m.get("currencyId").and_then(Value::as_str) == Some("id_currency_platinum")
                    && m.get("price").and_then(Value::as_f64) == Some(0.0)
            })
    });
    if !free {
        problems.push(
            "prices must be exactly [{ \"currencyId\": \"id_currency_platinum\", \"price\": 0 }] (the client opens the \
             coin toss only for PlatPrice 0)"
                .into(),
        );
    }
    let empty_list = |value: Option<&Value>| match value {
        None | Some(Value::Null) => true,
        Some(Value::Array(list)) => list.is_empty(),
        _ => false,
    };
    if !empty_list(get("entitlements")) {
        problems.push("entitlements are not allowed (stacked items only)".into());
    }
    if !js::is_nullish(get("skuProgression")) {
        problems.push("skuProgression is not allowed (stacked items only)".into());
    }
    if !js::is_nullish(get("loadoutSlots")) {
        problems.push("loadoutSlots are not allowed (stacked items only)".into());
    }
    let items = get("items").and_then(Value::as_array).filter(|i| !i.is_empty());
    if items.is_none() {
        problems.push("items must list at least one stacked item".into());
    }
    let policy = root
        .try_load_backend("inventory_storage_policy.json")
        .ok()
        .flatten()
        .map(|d| crate::inventory::storage_policy::StoragePolicy::from_document(&d));
    let gated = prestige_gated(root);
    for item in items.into_iter().flatten() {
        let Some(id) = item.get("catalogId").and_then(Value::as_str).filter(|s| !s.is_empty()) else {
            problems.push(format!("item with invalid catalogId {}", v(item.get("catalogId"))));
            continue;
        };
        if item.get("instanced") != Some(&Value::Bool(false)) {
            problems.push(format!(
                "{id}: instanced items are not allowed (set \"instanced\": false; stacked items only)"
            ));
        }
        let q = item.get("quantity").and_then(Value::as_f64);
        if !q.is_some_and(|q| q.fract() == 0.0 && (1.0..=crate::util::MAX_SAFE).contains(&q)) {
            problems.push(format!("{id}: quantity must be a positive integer"));
        }
        let entry = policy.as_ref().and_then(|p| p.entry(id));
        match entry {
            Some(e) if e.storage == crate::inventory::storage_policy::Storage::Stacked => {
                if e.max_quantity > 0.0 {
                    problems.push(format!(
                        "{id}: capped at maxQuantity {}; a daily offer may only grant uncapped stacked items",
                        jsjson::number(e.max_quantity)
                    ));
                }
            }
            other => problems.push(format!(
                "{id}: inventory_storage_policy.json says {}, not stacked (regenerate it after choosing a new item)",
                other
                    .and_then(|e| e.storage_text.clone())
                    .unwrap_or_else(|| String::from("nothing"))
            )),
        }
        if gated.contains(id) {
            problems.push(format!(
                "{id}: a prestige-gated reward currency cannot be granted by the store"
            ));
        }
    }
    if !problems.is_empty() {
        return (Vec::new(), problems);
    }
    let text = |key: &str, default: &str| get(key).and_then(Value::as_str).unwrap_or(default).to_owned();
    let priority = get("displayPriority")
        .and_then(Value::as_f64)
        .filter(|p| p.is_finite())
        .map_or(Value::from(0), crate::util::number_value);
    let items: Vec<Value> = items
        .into_iter()
        .flatten()
        .map(|i| {
            obj(vec![
                ("catalogId", i.get("catalogId").cloned().unwrap_or(Value::Null)),
                ("quantity", i.get("quantity").cloned().unwrap_or(Value::Null)),
                ("instanced", Value::Bool(false)),
            ])
        })
        .collect();
    let served = obj(vec![
        ("id", get("id").cloned().unwrap_or(Value::Null)),
        ("displayName", Value::from(text("displayName", "Daily Free Bundle"))),
        ("displayDescription", Value::from(text("displayDescription", ""))),
        ("displayPriority", priority),
        (
            "prices",
            serde_json::json!([{"currencyId": "id_currency_platinum", "price": 0}]),
        ),
        ("maxAllowed", Value::from(1)),
        (
            "images",
            get("images")
                .filter(|i| i.is_object())
                .cloned()
                .unwrap_or(Value::Object(Map::new())),
        ),
        (
            "tags",
            Value::Array(
                get("tags")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter(|t| t.is_string())
                    .cloned()
                    .collect(),
            ),
        ),
        ("items", Value::Array(items)),
        ("duplicateInstancedItems", Value::Array(Vec::new())),
    ]);
    (Sku::read(&served).into_iter().collect(), Vec::new())
}

impl Catalog {
    /// Reads the served sections' files from `root` (Node reads them at start).
    pub fn load(root: &GameDataRoot, served: &[&str]) -> Self {
        let serves = |tag: &str| served.contains(&tag);
        let ladyluck = if serves("ladyluckstore") {
            read_offers(root, "ladyluck_store.json")
        } else {
            Vec::new()
        };
        let reward_cache = if serves("season_store") {
            read_offers(root, "reward_cache_store.json")
        } else {
            Vec::new()
        };
        let platinum = if PLATINUM_SECTIONS.iter().any(|t| serves(t)) {
            read_offers(root, "platinum_store.json")
        } else {
            Vec::new()
        };
        let slots = [exchange_slot(2), exchange_slot(3)];
        let mut fountain = Vec::new();
        if serves(DAILY_SECTION) {
            match root.try_load_backend("fountain_daily_bundle.json") {
                Ok(None) => tracing::info!(
                    "[Store:fountain_daily_free_bundle] no fountain_daily_bundle.json - the fountain stays unavailable \
                     (optional)"
                ),
                Ok(Some(raw)) => {
                    let (offers, problems) = parse_fountain_bundle(&raw, root);
                    if problems.is_empty() {
                        fountain = offers;
                    } else {
                        tracing::warn!(
                            "[Store:fountain_daily_free_bundle] fountain_daily_bundle.json rejected - the fountain stays \
                             unavailable: {}",
                            problems.join("; ")
                        );
                    }
                }
                Err(e) => tracing::warn!(
                    "[Store:fountain_daily_free_bundle] fountain_daily_bundle.json could not be read - the fountain \
                     stays unavailable: {e}"
                ),
            }
            let used: HashSet<&str> = slots
                .iter()
                .chain(&ladyluck)
                .chain(&reward_cache)
                .chain(&platinum)
                .map(|s| s.id.as_str())
                .collect();
            let clash: Vec<String> = fountain
                .iter()
                .filter(|s| used.contains(s.id.as_str()))
                .map(|s| s.id.clone())
                .collect();
            if !clash.is_empty() {
                tracing::warn!(
                    "[Store:fountain_daily_free_bundle] SKU id {} is already used by another store - section disabled",
                    clash.join(", ")
                );
                fountain.clear();
            }
        }
        let mut sections = Vec::new();
        for tag in SECTION_ORDER {
            if !serves(tag) {
                continue;
            }
            let offers = match tag {
                "exchange_vendor_slot_2" => vec![slots[0].clone()],
                "exchange_vendor_slot_3" => vec![slots[1].clone()],
                "ladyluckstore" => ladyluck.clone(),
                "season_store" => reward_cache.clone(),
                DAILY_SECTION => fountain.clone(),
                platinum_tag => platinum
                    .iter()
                    .filter(|s| s.tags.iter().any(|t| t == platinum_tag))
                    .cloned()
                    .collect(),
            };
            sections.push((tag.to_owned(), offers));
        }
        Self { sections }
    }

    /// Read once per module, on first use, never refreshed.
    pub fn cached(shared: &Shared) -> Arc<Catalog> {
        crate::caches::cached(shared, "bounties.catalog", || {
            let catalog = Catalog::load(shared.game_data(), shared.version().tables.store_sections);
            (Arc::new(catalog), true)
        })
    }

    /// The offers in a section; empty for an unknown tag.
    pub fn skus(&self, tag: &str) -> &[Sku] {
        self.sections
            .iter()
            .find(|(t, _)| t == tag)
            .map_or(&[], |(_, offers)| offers.as_slice())
    }

    /// The offer with that id in that section.
    pub fn from_section(&self, tag: &str, sku_id: &str) -> Option<&Sku> {
        self.skus(tag).iter().find(|s| s.id == sku_id)
    }

    /// `FindStoreSkuByIdOrCatalogId`: the exact id first (daily sections included), else a unique item catalog id
    /// across the non-daily sections.
    pub fn lookup(&self, text: &str) -> Option<(&str, &Sku)> {
        for (tag, offers) in &self.sections {
            if let Some(sku) = offers.iter().find(|s| s.id == text) {
                return Some((tag, sku));
            }
        }
        let mut found: Option<(&str, &Sku)> = None;
        for (tag, offers) in &self.sections {
            if is_daily_section(tag) {
                continue;
            }
            for sku in offers.iter().filter(|s| s.items.iter().any(|i| i.catalog_id == text)) {
                match found {
                    None => found = Some((tag, sku)),
                    Some((_, first)) if first.id != sku.id => return None,
                    Some(_) => {}
                }
            }
        }
        found
    }
}

/// One SKU in `GET /product/skus/public` (§4.11), keys in this order. `owned`: the caller already bought this
/// one-time SKU, so `remaining` is 0, as the live service answered and the client reads to show it as owned.
pub fn listing(settings: &Settings, sku: &Sku, owned: bool) -> Value {
    let raw = &sku.raw;
    let mut out = Map::new();
    for key in ["id", "displayName", "displayDescription", "displayPriority"] {
        if let Some(value) = raw.get(key) {
            out.insert(key.into(), value.clone());
        }
    }
    let prices: Vec<Value> = list_of(raw, "prices")
        .iter()
        .map(|p| {
            let currency = js::string(p.get("currencyId"));
            let price = if effective_price(settings, &currency, 1.0) == 0.0 {
                Value::from(0)
            } else {
                p.get("price").cloned().unwrap_or(Value::Null)
            };
            obj(vec![
                ("currencyId", p.get("currencyId").cloned().unwrap_or(Value::Null)),
                ("price", price),
                ("salesPrice", Value::Null),
                ("multiPrice", Value::Null),
            ])
        })
        .collect();
    out.insert("prices".into(), Value::Array(prices));
    out.insert("maxAllowed".into(), sku.max_allowed.clone());
    let remaining = if owned {
        Value::from(0)
    } else if sku.max_allowed.is_null() {
        Value::from(999)
    } else {
        sku.max_allowed.clone()
    };
    out.insert("remaining".into(), remaining);
    out.insert(
        "duplicateInstancedItems".into(),
        raw.get("duplicateInstancedItems").cloned().unwrap_or(Value::Null),
    );
    out.insert(
        "images".into(),
        raw.get("images")
            .filter(|i| !i.is_null())
            .cloned()
            .unwrap_or(Value::Object(Map::new())),
    );
    out.insert("tags".into(), raw.get("tags").cloned().unwrap_or(Value::Null));
    out.insert("scheduledTags".into(), Value::Null);
    let items: Vec<Value> = list_of(raw, "items")
        .iter()
        .map(|i| {
            obj(vec![
                ("catalogId", i.get("catalogId").cloned().unwrap_or(Value::Null)),
                ("quantity", i.get("quantity").cloned().unwrap_or(Value::Null)),
            ])
        })
        .collect();
    out.insert("items".into(), Value::Array(items));
    let entitlements: Vec<Value> = list_of(raw, "entitlements")
        .iter()
        .map(|e| {
            obj(vec![
                ("name", e.get("name").cloned().unwrap_or(Value::Null)),
                ("duration", e.get("duration").cloned().unwrap_or(Value::Null)),
            ])
        })
        .collect();
    out.insert("entitlements".into(), Value::Array(entitlements));
    let progression = match raw.get("skuProgression") {
        None | Some(Value::Null) => Value::Null,
        Some(p) => {
            let mut m = Map::new();
            for key in ["progressionId", "ranks", "xp"] {
                if let Some(v) = p.get(key) {
                    m.insert(key.into(), v.clone());
                }
            }
            Value::Object(m)
        }
    };
    out.insert("skuProgression".into(), progression);
    for key in [
        "loadoutSlots",
        "availableFrom",
        "availableTo",
        "timeAvailabilityReason",
        "platformOfferId",
        "missingEntitlementNames",
    ] {
        out.insert(key.into(), Value::Null);
    }
    Value::Object(out)
}

/// The daily window at `now_ms`: `(start, end, key)`, opening at 17:00 UTC.
pub fn daily_window(now_ms: i64) -> (i64, i64, String) {
    let start = (now_ms - RESET_OFFSET_MS).div_euclid(DAY_MS) * DAY_MS + RESET_OFFSET_MS;
    (start, start + DAY_MS, iso(start))
}

/// `(transaction scope, instance scope)` of a section.
pub fn transaction_scope(tag: &str) -> (String, String) {
    match tag {
        "ladyluckstore" => ("ladyluck-purchase".into(), "ladyluck-purchase-instance".into()),
        t if PLATINUM_SECTIONS.contains(&t) => (
            "store-purchase:platinum".into(),
            "store-purchase-instance:platinum".into(),
        ),
        t => (format!("store-purchase:{t}"), format!("store-purchase-instance:{t}")),
    }
}

/// The deterministic transaction id of a one-time SKU's purchase: its ledger row is the ownership record.
pub fn one_time_transaction_id(tag: &str, user_id: &str, sku_id: &str) -> String {
    sha_upper(&format!("{}:{user_id}:{sku_id}", transaction_scope(tag).0), 32)
}

/// The deterministic transaction id of one daily claim: one per section, user, SKU and window.
pub fn daily_claim_transaction_id(tag: &str, user_id: &str, sku_id: &str, window_key: &str) -> String {
    sha_upper(
        &format!("{}:{user_id}:{sku_id}:{window_key}", transaction_scope(tag).0),
        32,
    )
}

/// Store currency ids to wallet currencies; anything else upper-cased (spec §14 item 6).
pub fn wallet_currency(currency_id: &str) -> String {
    match currency_id {
        "id_currency_marks_gilded" => "CURRENCY_MARKS_GILDED".into(),
        "id_currency_marks_steel" => "CURRENCY_MARKS_STEEL".into(),
        "id_currency_s19_coin" => "CURRENCY_S19_COIN".into(),
        "id_currency_seasonal_coin" => "CURRENCY_SEASONAL_COIN".into(),
        "id_currency_rewardcache" => "CURRENCY_REWARDCACHE".into(),
        "id_currency_platinum" => "CURRENCY_PLATINUM".into(),
        other => other.to_uppercase(),
    }
}

/// Why a store purchase failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PurchaseReason {
    /// No offer with that id.
    UnknownSku,
    /// The wallet cannot pay the price.
    InsufficientBalance,
    /// Another purchase changed the inventory first.
    Conflict,
    /// The transaction id was used by a purchase with a different request.
    AlreadyPurchasedDifferentRequest,
    /// Anything else, such as an invalid price or a store failure.
    TransactionFailed,
}

impl PurchaseReason {
    /// The reason as the answer writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnknownSku => "unknown_sku",
            Self::InsufficientBalance => "insufficient_balance",
            Self::Conflict => "conflict",
            Self::AlreadyPurchasedDifferentRequest => "already_purchased_different_request",
            Self::TransactionFailed => "transaction_failed",
        }
    }

    /// The HTTP status the reason is answered with.
    pub fn status(self) -> u16 {
        match self {
            Self::UnknownSku => 404,
            Self::InsufficientBalance => 402,
            Self::Conflict | Self::AlreadyPurchasedDifferentRequest => 409,
            Self::TransactionFailed => 500,
        }
    }
}

/// The SKU's entitlement and progression grants.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SkuGrants {
    /// `(name, duration)` of each entitlement; an unreadable duration is 0.
    pub entitlements: Vec<(String, f64)>,
    /// `skuProgression`, when the offer has one.
    pub progression: Option<Value>,
}

impl SkuGrants {
    /// True when the offer grants neither.
    pub fn is_empty(&self) -> bool {
        self.entitlements.is_empty() && self.progression.is_none()
    }
}

/// A store purchase planned as one inventory transaction.
#[derive(Debug, Clone, PartialEq)]
pub struct PurchasePlan {
    /// The transaction id: derived for a daily claim or a one-time offer, fresh otherwise.
    pub transaction_id: String,
    /// Instances to add, each with an id derived from the purchase.
    pub add_instanced: Vec<Value>,
    /// Stacks to add.
    pub add_stacked: Vec<Value>,
    /// The price, as stacks to remove from the wallet.
    pub remove_stacked: Vec<Value>,
    /// The entitlement and progression grants.
    pub grants: SkuGrants,
    /// A daily section's window key
    pub window_key: Option<String>,
}

/// The pure plan of a purchase (§4.12 step 2); `fresh_id` is a repeatable purchase's id.
pub fn plan_purchase(
    settings: &Settings,
    sku: &Sku,
    tag: &str,
    user_id: &str,
    now_ms: i64,
    fresh_id: &dyn Fn() -> String,
) -> Result<PurchasePlan, PurchaseReason> {
    let instance_scope = transaction_scope(tag).1;
    let window_key = is_daily_section(tag).then(|| daily_window(now_ms).2);
    let transaction_id = match &window_key {
        Some(key) => daily_claim_transaction_id(tag, user_id, &sku.id, key),
        None if sku.is_one_time() => one_time_transaction_id(tag, user_id, &sku.id),
        None => fresh_id(),
    };
    let mut remove_stacked = Vec::new();
    for (currency, price) in &sku.prices {
        let Some(p) = price.as_f64().filter(|p| p.is_finite() && *p >= 0.0) else {
            tracing::warn!("[Store:{tag}] {} has an invalid price - purchase refused", sku.id);
            return Err(PurchaseReason::TransactionFailed);
        };
        if effective_price(settings, currency, p) > 0.0 {
            remove_stacked.push(obj(vec![
                ("catalogId", Value::from(wallet_currency(currency))),
                ("quantity", price.clone()),
            ]));
        }
    }
    let mut add_instanced = Vec::new();
    let mut add_stacked = Vec::new();
    for item in &sku.items {
        if item.instanced {
            let count = item.quantity.as_f64().unwrap_or(0.0);
            let mut i = 0u64;
            // Each instance has its own derived id, so a replayed purchase builds the identical request
            #[allow(clippy::cast_precision_loss, reason = "the index stops at 1000")]
            while (i as f64) < count && i < 1000 {
                let mut stable = format!("{instance_scope}:{user_id}:{}:{}:{i}", sku.id, item.catalog_id);
                if let Some(key) = &window_key {
                    stable.push(':');
                    stable.push_str(key);
                }
                add_instanced.push(obj(vec![
                    ("catalogId", Value::from(item.catalog_id.as_str())),
                    ("instanceId", Value::from(sha_upper(&stable, 26))),
                    ("itemData", Value::Null),
                    ("updateVersion", Value::from(0)),
                ]));
                i += 1;
            }
        } else {
            add_stacked.push(obj(vec![
                ("catalogId", Value::from(item.catalog_id.as_str())),
                ("quantity", item.quantity.clone()),
            ]));
        }
    }
    let entitlements = sku
        .entitlements
        .iter()
        .map(|(name, duration)| {
            let d = num(Some(duration));
            (name.clone(), if d.is_nan() { 0.0 } else { d })
        })
        .collect();
    Ok(PurchasePlan {
        transaction_id,
        add_instanced,
        add_stacked,
        remove_stacked,
        grants: SkuGrants {
            entitlements,
            progression: sku.sku_progression.clone(),
        },
        window_key,
    })
}

/// The query reader of §3.14: `&`-split, first `=`, `+` as space, `decodeURIComponent` (raw text on failure).
pub fn query_values(query: &str, name: &str) -> Vec<String> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            let decode = |t: &str| {
                let spaced = t.replace('+', " ");
                js::decode_uri_component(&spaced).unwrap_or(spaced)
            };
            (decode(key) == name).then(|| decode(value))
        })
        .collect()
}

/// The single non-empty value of `name`, else none (absent, empty or repeated).
pub fn single_query_value(query: &str, name: &str) -> Option<String> {
    let mut values = query_values(query, name);
    if values.len() != 1 {
        return None;
    }
    values.pop().filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn windows() {
        let at = js::date_parse("2026-10-10T16:59:59.999Z").unwrap();
        assert_eq!(daily_window(at).2, "2026-10-09T17:00:00.000Z");
        assert_eq!(daily_window(at + 1).2, "2026-10-10T17:00:00.000Z");
        let id = daily_claim_transaction_id(DAILY_SECTION, "u", "s", "k");
        assert_eq!(id.len(), 32);
        assert!(id.bytes().all(|b| b.is_ascii_digit() || b.is_ascii_uppercase()));
    }

    #[test]
    fn scopes_and_currencies() {
        assert_eq!(transaction_scope("webstore"), transaction_scope("dyes"));
        assert_eq!(transaction_scope("ladyluckstore").0, "ladyluck-purchase");
        assert_eq!(
            transaction_scope("season_store").1,
            "store-purchase-instance:season_store"
        );
        assert_eq!(wallet_currency("id_currency_marks_steel"), "CURRENCY_MARKS_STEEL");
        assert_eq!(wallet_currency("id_currency_s20_coin"), "ID_CURRENCY_S20_COIN");
    }

    fn sku(value: Value) -> Sku {
        Sku::read(&value).unwrap()
    }

    #[test]
    fn plans() {
        let free = Settings::default();
        let one = sku(
            json!({"id": "s1", "maxAllowed": 1, "prices": [{"currencyId": "id_currency_marks_steel", "price": 10}],
            "items": [{"catalogId": "WP_A", "quantity": 2, "instanced": true}, {"catalogId": "MAT", "quantity": 3}],
            "entitlements": [{"name": "e1", "duration": "x"}]}),
        );
        let fresh = || String::from("FRESH");
        let plan = plan_purchase(&free, &one, "ladyluckstore", "u", 0, &fresh).unwrap();
        assert_eq!(plan.transaction_id, sha_upper("ladyluck-purchase:u:s1", 32));
        assert_eq!(
            plan.remove_stacked,
            vec![json!({"catalogId": "CURRENCY_MARKS_STEEL", "quantity": 10})]
        );
        assert_eq!(plan.add_instanced.len(), 2);
        assert_eq!(plan.add_instanced[0]["instanceId"].as_str().unwrap().len(), 26);
        assert_eq!(plan.add_stacked, vec![json!({"catalogId": "MAT", "quantity": 3})]);
        assert_eq!(plan.grants.entitlements, vec![("e1".to_owned(), 0.0)]);
        let repeat = sku(json!({"id": "s2", "prices": [{"currencyId": "id_currency_platinum", "price": 5}]}));
        let plan = plan_purchase(&free, &repeat, "webstore", "u", 0, &fresh).unwrap();
        assert_eq!((plan.transaction_id.as_str(), plan.remove_stacked.len()), ("FRESH", 0));
        for price in [json!(-1), json!("10"), Value::Null] {
            let bad = sku(json!({"id": "s3", "prices": [{"currencyId": "x", "price": price}]}));
            assert_eq!(
                plan_purchase(&free, &bad, "webstore", "u", 0, &fresh).unwrap_err(),
                PurchaseReason::TransactionFailed
            );
        }
    }

    #[test]
    fn lookups() {
        let root = GameDataRoot::default();
        let mut catalog = Catalog::load(&root, &SECTION_ORDER);
        let dye =
            sku(json!({"id": "dye1", "tags": ["webstore", "dyes"], "items": [{"catalogId": "DYE_X", "quantity": 1}]}));
        catalog.sections = vec![
            (
                "webstore".into(),
                vec![
                    dye.clone(),
                    sku(json!({"id": "a", "items": [{"catalogId": "SHARED", "quantity": 1}]})),
                ],
            ),
            (
                "dyes".into(),
                vec![
                    dye,
                    sku(json!({"id": "b", "items": [{"catalogId": "SHARED", "quantity": 1}]})),
                ],
            ),
        ];
        assert_eq!(catalog.lookup("dye1").unwrap().0, "webstore");
        assert_eq!(catalog.lookup("DYE_X").unwrap().1.id, "dye1");
        assert!(catalog.lookup("SHARED").is_none());
        assert!(catalog.lookup("nothing").is_none());
    }

    #[test]
    fn listing_keys() {
        let s = sku(
            json!({"id": "x", "displayName": "X", "prices": [{"currencyId": "id_currency_platinum", "price": 500}], "items": [{"catalogId": "A", "quantity": 1, "instanced": true}]}),
        );
        let l = listing(&Settings::default(), &s, false);
        let keys: Vec<_> = l.as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            keys,
            [
                "id",
                "displayName",
                "prices",
                "maxAllowed",
                "remaining",
                "duplicateInstancedItems",
                "images",
                "tags",
                "scheduledTags",
                "items",
                "entitlements",
                "skuProgression",
                "loadoutSlots",
                "availableFrom",
                "availableTo",
                "timeAvailabilityReason",
                "platformOfferId",
                "missingEntitlementNames"
            ]
        );
        assert_eq!(l["prices"][0]["price"], 0);
        assert_eq!(l["remaining"], 999);
        assert_eq!(listing(&Settings::default(), &s, true)["remaining"], 0);
        assert_eq!(l["items"][0], json!({"catalogId": "A", "quantity": 1}));
        let paid = listing(&Settings::from_pairs([("PLATINUM_STORE_FREE", "0")]), &s, false);
        assert_eq!(paid["prices"][0]["price"], 500);
    }

    #[test]
    fn fountain() {
        let dir = tempfile::tempdir().unwrap();
        let root = GameDataRoot::new(Some(dir.path().to_path_buf()), None);
        let policy = json!({"entries": {"MAT": {"storage": "stacked", "maxQuantity": 0}, "CAP": {"storage": "stacked", "maxQuantity": 5}}});
        std::fs::write(dir.path().join("inventory_storage_policy.json"), policy.to_string()).unwrap();
        let good = json!({"skus": [{"id": "f1", "tags": [DAILY_SECTION], "prices": [{"currencyId": "id_currency_platinum", "price": 0}], "items": [{"catalogId": "MAT", "quantity": 2, "instanced": false}]}]});
        let (offers, problems) = parse_fountain_bundle(&good, &root);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(offers[0].raw["displayName"], "Daily Free Bundle");
        let bad = json!([{"id": "a b", "tags": [], "prices": [], "entitlements": [{}], "items": [{"catalogId": "CAP", "quantity": 0, "instanced": true}, {"catalogId": ""}]}]);
        let (_, problems) = parse_fountain_bundle(&bad, &root);
        assert_eq!(problems.len(), 8, "{problems:?}");
        assert_eq!(
            parse_fountain_bundle(&json!([]), &root).1,
            vec!["expected exactly one offer (the client only uses the first), found 0"]
        );
    }

    #[test]
    fn queries() {
        assert_eq!(
            single_query_value("requiredTags=ladyluckstore", "requiredTags").as_deref(),
            Some("ladyluckstore")
        );
        assert_eq!(single_query_value("requiredTags=", "requiredTags"), None);
        assert_eq!(
            single_query_value("requiredTags=a&requiredTags=b", "requiredTags"),
            None
        );
        assert_eq!(single_query_value("x=1", "requiredTags"), None);
        assert_eq!(single_query_value("token=a+b%2Fc", "token").as_deref(), Some("a b/c"));
        assert_eq!(
            single_query_value("token=%E0%A4%A", "token").as_deref(),
            Some("%E0%A4%A")
        );
    }
}
