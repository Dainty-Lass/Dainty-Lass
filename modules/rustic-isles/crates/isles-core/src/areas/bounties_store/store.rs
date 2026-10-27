//! The store (`controllers/store.ts`): Lady Luck's store, the Reward Cache, the Platinum store's four sections, the
//! Middleman's cell-fusion slots and the Ramsgate fountain's daily bundle, as sections the client asks for by tag.
//!
//! The offers are the host's own game data (`ladyluck_store.json`, `reward_cache_store.json`, `platinum_store.json`,
//! the optional `fountain_daily_bundle.json`), built from the host's install and from the store captures
//! `store-captures.json` names; nothing of them is committed. Which sections a version serves is its `STORE_SECTIONS`
//! table: 1.12.0 has `ladyluckstore` only.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::{Map, Value, json};
use service_kit::js;
use sha2::{Digest, Sha256};

use super::authn::SkuGrants;
use super::platinum::effective_price;
use super::util::{DAY_MS, num};
use crate::game_data::GameDataRoot;
use crate::settings::Settings;

pub const FOUNTAIN_DAILY_BUNDLE_TAG: &str = "fountain_daily_free_bundle";
const FOUNTAIN_DAILY_BUNDLE_FILE: &str = "fountain_daily_bundle.json";
/// The Platinum data set spans four sections the client requests separately; one idempotency scope covers them all.
const PLATINUM_SECTION_TAGS: [&str; 4] = ["webstore", "dyes", "hp_level_skip_rank", "huntpass_store"];

/// `Object.entries(STORE_SKUS)` order: the first section holding an id answers a lookup.
pub const SECTION_ORDER: [&str; 9] = [
    "exchange_vendor_slot_2",
    "exchange_vendor_slot_3",
    "ladyluckstore",
    "season_store",
    "webstore",
    "dyes",
    "hp_level_skip_rank",
    "huntpass_store",
    FOUNTAIN_DAILY_BUNDLE_TAG,
];

#[derive(Debug, Clone, PartialEq)]
pub struct SkuPrice {
    pub currency_id: String,
    /// As the data file wrote it; a non-number is refused at purchase
    pub price: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SkuItem {
    pub catalog_id: String,
    pub quantity: Value,
    pub instanced: bool,
}

/// `StoreSku`: one offer. The file's fields are kept as written, since the listing echoes them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StoreSku {
    pub id: String,
    pub raw: Map<String, Value>,
    pub tags: Vec<String>,
    pub prices: Vec<SkuPrice>,
    pub max_allowed: Value,
    pub items: Vec<SkuItem>,
    /// `{name, duration}`
    pub entitlements: Vec<(String, Value)>,
    pub sku_progression: Option<Value>,
}

impl StoreSku {
    fn from_value(value: &Value) -> Option<Self> {
        let raw = value.as_object()?.clone();
        let list = |key: &str| raw.get(key).and_then(Value::as_array).cloned().unwrap_or_default();
        Some(Self {
            id: raw.get("id").and_then(Value::as_str)?.to_owned(),
            tags: list("tags")
                .iter()
                .filter_map(|t| t.as_str().map(str::to_owned))
                .collect(),
            prices: list("prices")
                .iter()
                .map(|p| SkuPrice {
                    currency_id: js::string(p.get("currencyId")),
                    price: p.get("price").cloned().unwrap_or(Value::Null),
                })
                .collect(),
            max_allowed: raw.get("maxAllowed").cloned().unwrap_or(Value::Null),
            items: list("items")
                .iter()
                .map(|i| SkuItem {
                    catalog_id: js::string(i.get("catalogId")),
                    quantity: i.get("quantity").cloned().unwrap_or(Value::Null),
                    instanced: i.get("instanced") == Some(&Value::Bool(true)),
                })
                .collect(),
            entitlements: list("entitlements")
                .iter()
                .map(|e| {
                    (
                        js::string(e.get("name")),
                        e.get("duration").cloned().unwrap_or(Value::Null),
                    )
                })
                .collect(),
            sku_progression: raw.get("skuProgression").filter(|p| !p.is_null()).cloned(),
            raw,
        })
    }

    pub fn is_one_time(&self) -> bool {
        self.max_allowed.as_f64() == Some(1.0)
    }
}

/// Store-service currency ids (`id_currency_x`) to the wallet's catalog ids (`CURRENCY_X`).
fn wallet_currency(currency_id: &str) -> String {
    match currency_id {
        "id_currency_marks_gilded" => "CURRENCY_MARKS_GILDED".into(),
        "id_currency_marks_steel" => "CURRENCY_MARKS_STEEL".into(),
        "id_currency_s19_coin" => "CURRENCY_S19_COIN".into(),
        // The Reward Cache is priced in the 1.14.7 seasonal coin (the HUD's blue-star "Cache Coins")
        "id_currency_seasonal_coin" => "CURRENCY_SEASONAL_COIN".into(),
        "id_currency_rewardcache" => "CURRENCY_REWARDCACHE".into(),
        // The AGGREGATE id: the wallet splits the debit across the per-storefront buckets
        "id_currency_platinum" => "CURRENCY_PLATINUM".into(),
        other => other.to_uppercase(),
    }
}

/// The Middleman's cell-fusion slots: slot 1 is free, 2 and 3 are unlocked by the entitlements `exchange_slot_2` and
/// `exchange_slot_3`. No live capture exists: the operator's price, 1,000 Platinum each, permanent.
fn exchange_slot_sku(slot: u32) -> StoreSku {
    let value = json!({
        "id": format!("single_exchange_slot_{slot}"),
        "displayName": format!("Cell Fusion Slot {slot}"),
        "displayDescription": format!("Unlocks cell fusion slot {slot} at the Middleman permanently."),
        "displayPriority": slot,
        "prices": [{ "currencyId": "id_currency_platinum", "price": 1000 }],
        "maxAllowed": 1,
        "tags": [format!("exchange_vendor_slot_{slot}")],
        "items": [],
        "entitlements": [{ "name": format!("exchange_slot_{slot}"), "duration": 0 }],
        "duplicateInstancedItems": [],
    });
    StoreSku::from_value(&value).unwrap_or_default()
}

/// What a data file read gives: `None` when the file is missing.
fn read_game_file(root: &GameDataRoot, name: &str) -> Result<Option<Value>, String> {
    let Some(path) = root.backend_file(name) else {
        return Ok(None);
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => service_kit::jsjson::parse(text.trim_start_matches('\u{feff}'))
            .map(Some)
            .map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// `inventory_storage_policy.json`'s `entries`: storage and `maxQuantity` per catalog id.
fn storage_policy(root: &GameDataRoot) -> Option<HashMap<String, (String, f64)>> {
    let file = read_game_file(root, "inventory_storage_policy.json").ok().flatten()?;
    Some(
        file.get("entries")?
            .as_object()?
            .iter()
            .map(|(id, entry)| {
                (
                    id.clone(),
                    (js::string(entry.get("storage")), js::number(entry.get("maxQuantity"))),
                )
            })
            .collect(),
    )
}

/// `IsPrestigeGatedReward`: a reward currency some progression track pays per prestige level (`progression_config.json`).
fn prestige_gated_rewards(root: &GameDataRoot) -> HashSet<String> {
    let mut gated = HashSet::new();
    let Ok(Some(file)) = read_game_file(root, "progression_config.json") else {
        return gated;
    };
    let paths = file
        .pointer("/payload/paths")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    for path in paths {
        let per_level = js::number(path.pointer("/prestige/xp_per_level"));
        if path.get("progression_id").and_then(Value::as_str).is_none() || !per_level.is_finite() || per_level <= 0.0 {
            continue;
        }
        let rewards = path
            .pointer("/prestige/free_rewards/stacked_items")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        for reward in rewards {
            let quantity = js::number(reward.get("quantity"));
            if let (Some(id), true) = (
                reward.get("catalog_id").and_then(Value::as_str),
                quantity.is_finite() && quantity > 0.0,
            ) {
                gated.insert(id.to_owned());
            }
        }
    }
    gated
}

/// `ParseFountainDailyBundle`: fail-closed, any problem refuses the whole offer. Stacked items only (a stacked grant
/// carries no instance id, so a daily claim never re-mints a previous day's item), no entitlement or progression.
pub fn parse_fountain_daily_bundle(raw: &Value, root: &GameDataRoot) -> (Vec<StoreSku>, Vec<String>) {
    let list = match raw {
        Value::Array(list) => Some(list),
        other => other.get("skus").and_then(Value::as_array),
    };
    let Some(list) = list else {
        return (
            Vec::new(),
            vec!["expected { \"skus\": [ <one offer> ] } or an array holding one offer".into()],
        );
    };
    if list.len() != 1 {
        return (
            Vec::new(),
            vec![format!(
                "expected exactly one offer (the client only uses the first), found {}",
                list.len()
            )],
        );
    }
    let sku = &list[0];
    let mut problems: Vec<String> = Vec::new();
    let id_ok = sku.get("id").and_then(Value::as_str).is_some_and(|id| {
        (1..=128).contains(&id.len())
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
    });
    if !id_ok {
        problems.push(format!(
            "invalid id {} (letters, digits, _ . - only: it is a URL segment of /token/<slug>/<sku>)",
            sku.get("id").map_or_else(|| "undefined".to_owned(), Value::to_string)
        ));
    }
    let tags = sku.get("tags").and_then(Value::as_array);
    if !tags.is_some_and(|t| t.iter().any(|t| t.as_str() == Some(FOUNTAIN_DAILY_BUNDLE_TAG))) {
        problems.push(format!("tags must include {FOUNTAIN_DAILY_BUNDLE_TAG}"));
    }
    let free_platinum = sku.get("prices").and_then(Value::as_array).is_some_and(|prices| {
        prices.len() == 1
            && prices[0].get("currencyId").and_then(Value::as_str) == Some("id_currency_platinum")
            && prices[0].get("price").and_then(Value::as_f64) == Some(0.0)
            && prices[0].get("price").is_some_and(Value::is_number)
    });
    if !free_platinum {
        problems.push(
            "prices must be exactly [{ \"currencyId\": \"id_currency_platinum\", \"price\": 0 }] (the client opens the coin toss only for PlatPrice 0)"
                .into(),
        );
    }
    let has_entitlements = match sku.get("entitlements") {
        Some(Value::Array(list)) => !list.is_empty(),
        None | Some(Value::Null) => false,
        Some(_) => true,
    };
    if has_entitlements {
        problems.push("entitlements are not allowed (stacked items only)".into());
    }
    if sku.get("skuProgression").is_some_and(|v| !v.is_null()) {
        problems.push("skuProgression is not allowed (stacked items only)".into());
    }
    if sku.get("loadoutSlots").is_some_and(|v| !v.is_null()) {
        problems.push("loadoutSlots are not allowed (stacked items only)".into());
    }
    let items = sku.get("items").and_then(Value::as_array);
    if items.is_none_or(Vec::is_empty) {
        problems.push("items must list at least one stacked item".into());
    }
    let policy = storage_policy(root);
    let gated = prestige_gated_rewards(root);
    for item in items.map_or(&[][..], Vec::as_slice) {
        let Some(catalog_id) = item
            .get("catalogId")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
        else {
            problems.push(format!(
                "item with invalid catalogId {}",
                item.get("catalogId")
                    .map_or_else(|| "undefined".to_owned(), Value::to_string)
            ));
            continue;
        };
        if item.get("instanced") != Some(&Value::Bool(false)) {
            problems.push(format!(
                "{catalog_id}: instanced items are not allowed (set \"instanced\": false; stacked items only)"
            ));
        }
        let quantity_ok = item
            .get("quantity")
            .and_then(Value::as_f64)
            .is_some_and(|q| q.fract() == 0.0 && q > 0.0 && q <= 9_007_199_254_740_991.0);
        if !quantity_ok {
            problems.push(format!("{catalog_id}: quantity must be a positive integer"));
        }
        // The grant path re-applies the storage policy, so the policy, not the file's own flag, decides what is minted
        match policy.as_ref().and_then(|p| p.get(catalog_id)) {
            Some((storage, max)) if storage == "stacked" => {
                if *max > 0.0 {
                    // A capped item (an ownership unlock, a dye) would pass day 1 and exceed its cap from day 2 on
                    problems.push(format!(
                        "{catalog_id}: capped at maxQuantity {}; a daily offer may only grant uncapped stacked items",
                        js::string(Some(&num(*max)))
                    ));
                }
            }
            other => problems.push(format!(
                "{catalog_id}: inventory_storage_policy.json says {}, not stacked (regenerate it after choosing a new item)",
                other.map_or("nothing", |(storage, _)| storage.as_str())
            )),
        }
        if gated.contains(catalog_id) {
            problems.push(format!(
                "{catalog_id}: a prestige-gated reward currency cannot be granted by the store"
            ));
        }
    }
    if !problems.is_empty() {
        return (Vec::new(), problems);
    }
    let mut offer = Map::new();
    let text = |key: &str, default: &str| {
        sku.get(key)
            .and_then(Value::as_str)
            .map_or_else(|| default.to_owned(), str::to_owned)
    };
    offer.insert("id".into(), sku["id"].clone());
    offer.insert("displayName".into(), text("displayName", "Daily Free Bundle").into());
    offer.insert("displayDescription".into(), text("displayDescription", "").into());
    offer.insert(
        "displayPriority".into(),
        sku.get("displayPriority")
            .filter(|p| p.as_f64().is_some_and(f64::is_finite))
            .cloned()
            .unwrap_or_else(|| 0.into()),
    );
    offer.insert(
        "prices".into(),
        json!([{ "currencyId": "id_currency_platinum", "price": 0 }]),
    );
    offer.insert("maxAllowed".into(), 1.into());
    offer.insert(
        "images".into(),
        sku.get("images")
            .filter(|i| i.is_object())
            .cloned()
            .unwrap_or_else(|| json!({})),
    );
    offer.insert(
        "tags".into(),
        Value::Array(
            sku["tags"]
                .as_array()
                .map_or(&[][..], Vec::as_slice)
                .iter()
                .filter(|t| t.is_string())
                .cloned()
                .collect(),
        ),
    );
    offer.insert(
        "items".into(),
        Value::Array(
            items
                .map_or(&[][..], Vec::as_slice)
                .iter()
                .map(|i| json!({ "catalogId": i["catalogId"], "quantity": i["quantity"], "instanced": false }))
                .collect(),
        ),
    );
    offer.insert("duplicateInstancedItems".into(), json!([]));
    (
        StoreSku::from_value(&Value::Object(offer)).into_iter().collect(),
        Vec::new(),
    )
}

/// `LoadFountainDailyBundleSkus`: the file is optional; missing or rejected, the fountain simply stays unavailable.
fn load_fountain_daily_bundle(root: &GameDataRoot) -> Vec<StoreSku> {
    let raw = match read_game_file(root, FOUNTAIN_DAILY_BUNDLE_FILE) {
        Ok(Some(raw)) => raw,
        Ok(None) => {
            tracing::info!(
                "[Store:{FOUNTAIN_DAILY_BUNDLE_TAG}] no {FOUNTAIN_DAILY_BUNDLE_FILE} - the fountain stays unavailable (optional)"
            );
            return Vec::new();
        }
        Err(error) => {
            tracing::error!(
                "[Store:{FOUNTAIN_DAILY_BUNDLE_TAG}] {FOUNTAIN_DAILY_BUNDLE_FILE} could not be read - the fountain stays unavailable: {error}"
            );
            return Vec::new();
        }
    };
    let (skus, problems) = parse_fountain_daily_bundle(&raw, root);
    if !problems.is_empty() {
        tracing::error!(
            "[Store:{FOUNTAIN_DAILY_BUNDLE_TAG}] {FOUNTAIN_DAILY_BUNDLE_FILE} rejected - the fountain stays unavailable: {}",
            problems.join("; ")
        );
        return Vec::new();
    }
    skus
}

fn load_skus(root: &GameDataRoot, file: &str) -> Vec<StoreSku> {
    match read_game_file(root, file) {
        Ok(Some(Value::Array(list))) => list.iter().filter_map(StoreSku::from_value).collect(),
        Ok(Some(_)) => {
            tracing::error!("[Store] {file} is not a list of offers - its sections are empty");
            Vec::new()
        }
        Ok(None) => {
            tracing::error!(
                "[Store] game data file {file} is missing (generate it from your own installation) - its sections are empty"
            );
            Vec::new()
        }
        Err(error) => {
            tracing::error!("[Store] {file} could not be read - its sections are empty: {error}");
            Vec::new()
        }
    }
}

/// The sections a version serves, with their offers.
#[derive(Debug, Clone, Default)]
pub struct StoreCatalog {
    sections: Vec<(&'static str, Vec<StoreSku>)>,
}

/// A fountain offer must not share an id with another store's: a token names only the SKU id, and the lookup returns the
/// first section holding it.
fn without_sku_id_clashes(tag: &str, candidates: Vec<StoreSku>, others: &[&[StoreSku]]) -> Vec<StoreSku> {
    let other_ids: HashSet<&str> = others.iter().flat_map(|s| s.iter()).map(|s| s.id.as_str()).collect();
    let clashes: Vec<&str> = candidates
        .iter()
        .filter(|s| other_ids.contains(s.id.as_str()))
        .map(|s| s.id.as_str())
        .collect();
    if clashes.is_empty() {
        return candidates;
    }
    tracing::error!(
        "[Store:{tag}] SKU id {} is already used by another store - section disabled",
        clashes.join(", ")
    );
    Vec::new()
}

impl StoreCatalog {
    /// Reads the data files of the sections `served` names (`STORE_SECTIONS`).
    pub fn load(root: &GameDataRoot, served: &[&str]) -> Self {
        let serves = |tag: &str| served.contains(&tag);
        let platinum_wanted = PLATINUM_SECTION_TAGS.iter().any(|t| serves(t));
        let ladyluck = if serves("ladyluckstore") {
            load_skus(root, "ladyluck_store.json")
        } else {
            Vec::new()
        };
        let reward_cache = if serves("season_store") {
            load_skus(root, "reward_cache_store.json")
        } else {
            Vec::new()
        };
        let platinum = if platinum_wanted {
            load_skus(root, "platinum_store.json")
        } else {
            Vec::new()
        };
        let slot2 = vec![exchange_slot_sku(2)];
        let slot3 = vec![exchange_slot_sku(3)];
        let fountain = if serves(FOUNTAIN_DAILY_BUNDLE_TAG) {
            without_sku_id_clashes(
                FOUNTAIN_DAILY_BUNDLE_TAG,
                load_fountain_daily_bundle(root),
                &[&slot2, &slot3, &ladyluck, &reward_cache, &platinum],
            )
        } else {
            Vec::new()
        };
        let platinum_section = |tag: &str| -> Vec<StoreSku> {
            platinum
                .iter()
                .filter(|s| s.tags.iter().any(|t| t == tag))
                .cloned()
                .collect()
        };
        let mut sections: Vec<(&'static str, Vec<StoreSku>)> = Vec::new();
        for tag in SECTION_ORDER {
            if !serves(tag) {
                continue;
            }
            let skus = match tag {
                "exchange_vendor_slot_2" => slot2.clone(),
                "exchange_vendor_slot_3" => slot3.clone(),
                "ladyluckstore" => ladyluck.clone(),
                "season_store" => reward_cache.clone(),
                FOUNTAIN_DAILY_BUNDLE_TAG => fountain.clone(),
                other => platinum_section(other),
            };
            sections.push((tag, skus));
        }
        Self { sections }
    }

    /// From offers directly, in the order given (tests).
    pub fn from_sections(sections: Vec<(&'static str, Vec<StoreSku>)>) -> Self {
        Self { sections }
    }

    /// `GetStoreSkus`: an unserved tag is empty.
    pub fn skus(&self, store_tag: &str) -> &[StoreSku] {
        self.sections
            .iter()
            .find(|(tag, _)| *tag == store_tag)
            .map_or(&[][..], |(_, skus)| skus.as_slice())
    }

    pub fn from_sku(&self, store_tag: &str, sku_id: &str) -> Option<&StoreSku> {
        self.skus(store_tag).iter().find(|s| s.id == sku_id)
    }

    /// `FindStoreSkuByIdOrCatalogId`: the exact SKU id in the first section holding it, else the one SKU a bare item
    /// catalog id belongs to. One SKU in several sections (a dye is in `webstore` and `dyes`) is not ambiguous; two
    /// different offers are, and are refused. Daily sections are excluded from the catalog-id fallback.
    ///
    /// `first_match` is 1.12.0's lookup, which had Lady Luck alone: an exact id, else the first SKU holding the item,
    /// ambiguous or not.
    pub fn find_by_id_or_catalog_id(
        &self,
        id_or_catalog_id: &str,
        first_match: bool,
    ) -> Option<(&'static str, &StoreSku)> {
        for (tag, skus) in &self.sections {
            if let Some(exact) = skus.iter().find(|s| s.id == id_or_catalog_id) {
                return Some((tag, exact));
            }
        }
        let matches: Vec<(&'static str, &StoreSku)> = self
            .sections
            .iter()
            .filter(|(tag, _)| !is_daily_claim_section(tag))
            .flat_map(|(tag, skus)| {
                skus.iter()
                    .filter(|s| s.items.iter().any(|i| i.catalog_id == id_or_catalog_id))
                    .map(move |s| (*tag, s))
            })
            .collect();
        let distinct: HashSet<&str> = matches.iter().map(|(_, s)| s.id.as_str()).collect();
        if first_match || distinct.len() == 1 {
            matches.first().copied()
        } else {
            None
        }
    }
}

type CatalogCache = Mutex<HashMap<(Option<PathBuf>, Vec<String>), Arc<StoreCatalog>>>;

/// The catalog of one module, read once as Node reads its files at start.
pub fn catalog_for(root: &GameDataRoot, served: &[&str]) -> Arc<StoreCatalog> {
    static CACHE: OnceLock<CatalogCache> = OnceLock::new();
    let key = (
        root.backend.clone(),
        served.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>(),
    );
    let cache = CACHE.get_or_init(Mutex::default);
    let Ok(mut cache) = cache.lock() else {
        return Arc::new(StoreCatalog::load(root, served));
    };
    cache
        .entry(key)
        .or_insert_with(|| Arc::new(StoreCatalog::load(root, served)))
        .clone()
}

pub fn is_daily_claim_section(store_tag: &str) -> bool {
    store_tag == FOUNTAIN_DAILY_BUNDLE_TAG
}

// ---- Daily claims: once per reset window, which opens at 17:00 UTC, the instant the client itself stores for its
// daily bounty cooldown.

const DAILY_RESET_OFFSET_MS: i64 = 17 * 60 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DailyWindow {
    /// The window's start as an ISO string
    pub key: String,
    pub start_ms: i64,
    pub end_ms: i64,
}

pub fn daily_reset_window(now_ms: i64) -> DailyWindow {
    let start_ms = (now_ms - DAILY_RESET_OFFSET_MS).div_euclid(DAY_MS) * DAY_MS + DAILY_RESET_OFFSET_MS;
    DailyWindow {
        key: super::util::iso(start_ms),
        start_ms,
        end_ms: start_ms + DAY_MS,
    }
}

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// `{transaction, instance}` idempotency scopes; Lady Luck's strings are exactly the deployed ones.
pub fn transaction_scope_for(store_tag: &str) -> (String, String) {
    if store_tag == "ladyluckstore" {
        ("ladyluck-purchase".into(), "ladyluck-purchase-instance".into())
    } else if PLATINUM_SECTION_TAGS.contains(&store_tag) {
        (
            "store-purchase:platinum".into(),
            "store-purchase-instance:platinum".into(),
        )
    } else {
        (
            format!("store-purchase:{store_tag}"),
            format!("store-purchase-instance:{store_tag}"),
        )
    }
}

/// `DailyClaimTransactionId`: deterministic in (store scope, user, SKU, window), so a second claim in a window replays
/// the ledger row and the listing finds this window's claim without a separate cooldown record.
pub fn daily_claim_transaction_id(store_tag: &str, user_id: &str, sku_id: &str, window_key: &str) -> String {
    let (scope, _) = transaction_scope_for(store_tag);
    sha256_hex(&format!("{scope}:{user_id}:{sku_id}:{window_key}"))[..32].to_uppercase()
}

// ---- Purchases

/// Why `PurchaseStoreSku` refused; the route maps each to a status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PurchaseReason {
    UnknownSku,
    InsufficientBalance,
    Conflict,
    AlreadyPurchasedDifferentRequest,
    TransactionFailed,
}

impl PurchaseReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnknownSku => "unknown_sku",
            Self::InsufficientBalance => "insufficient_balance",
            Self::Conflict => "conflict",
            Self::AlreadyPurchasedDifferentRequest => "already_purchased_different_request",
            Self::TransactionFailed => "transaction_failed",
        }
    }

    /// `StatusByReason`; anything else is 400.
    pub const fn status(self) -> u16 {
        match self {
            Self::UnknownSku => 404,
            Self::InsufficientBalance => 402,
            Self::Conflict | Self::AlreadyPurchasedDifferentRequest => 409,
            Self::TransactionFailed => 500,
        }
    }
}

/// One purchase, planned: what `RunInventoryTransaction` is asked to apply.
#[derive(Debug, Clone, PartialEq)]
pub struct PurchasePlan {
    pub transaction_id: String,
    pub instanced_to_add: Vec<Value>,
    pub stacked_to_add: Vec<Value>,
    pub stacked_to_remove: Vec<Value>,
    pub grants: SkuGrants,
    pub daily_window: Option<DailyWindow>,
}

/// The first half of `PurchaseStoreSku`: the SKU's transaction id and item lists. A one-time SKU (`maxAllowed` 1) gets
/// a transaction id keyed on (store scope, user, SKU), which makes the inventory ledger the ownership check itself; a
/// repeatable one a fresh id per call; a daily one the window as well. Only a positive price is a debit.
pub fn plan_purchase(
    settings: &Settings,
    sku: &StoreSku,
    store_tag: &str,
    user_id: &str,
    now_ms: i64,
    fresh_id: &str,
) -> Result<PurchasePlan, PurchaseReason> {
    let window = is_daily_claim_section(store_tag).then(|| daily_reset_window(now_ms));
    let (transaction_scope, instance_scope) = transaction_scope_for(store_tag);
    let transaction_id = if let Some(window) = &window {
        daily_claim_transaction_id(store_tag, user_id, &sku.id, &window.key)
    } else if sku.is_one_time() {
        sha256_hex(&format!("{transaction_scope}:{user_id}:{}", sku.id))[..32].to_uppercase()
    } else {
        fresh_id.to_uppercase()
    };

    // A negative or non-numeric price would be a credit through the debit path: refuse the purchase
    if sku
        .prices
        .iter()
        .any(|p| !p.price.as_f64().is_some_and(|price| price.is_finite() && price >= 0.0))
    {
        tracing::error!("[Store:{store_tag}] {} has an invalid price - purchase refused", sku.id);
        return Err(PurchaseReason::TransactionFailed);
    }
    let stacked_to_remove: Vec<Value> = sku
        .prices
        .iter()
        .filter_map(|p| {
            let price = effective_price(settings, &p.currency_id, p.price.as_f64().unwrap_or_default());
            (price > 0.0).then(|| json!({ "catalogId": wallet_currency(&p.currency_id), "quantity": num(price) }))
        })
        .collect();

    let mut instanced_to_add: Vec<Value> = Vec::new();
    let mut stacked_to_add: Vec<Value> = Vec::new();
    for item in &sku.items {
        if item.instanced {
            // Deterministic per (user, SKU, item, index): a replayed purchase must build the identical list, or the
            // ledger's request hash rejects it as a mismatch
            let count = item.quantity.as_f64().unwrap_or_default();
            let mut i = 0.0;
            while i < count {
                let window_part = window.as_ref().map_or_else(String::new, |w| format!(":{}", w.key));
                let instance_id = sha256_hex(&format!(
                    "{instance_scope}:{user_id}:{}:{}:{}{window_part}",
                    sku.id,
                    item.catalog_id,
                    js::string(Some(&num(i)))
                ))[..26]
                    .to_uppercase();
                instanced_to_add.push(json!({
                    "catalogId": item.catalog_id,
                    "instanceId": instance_id,
                    "itemData": null,
                    "updateVersion": 0,
                }));
                i += 1.0;
            }
        } else {
            stacked_to_add.push(json!({ "catalogId": item.catalog_id, "quantity": item.quantity }));
        }
    }
    let grants = SkuGrants {
        entitlements: sku
            .entitlements
            .iter()
            .map(|(name, duration)| (name.clone(), super::util::number_or_zero(Some(duration))))
            .collect(),
        progression: sku.sku_progression.clone(),
    };
    Ok(PurchasePlan {
        transaction_id,
        instanced_to_add,
        stacked_to_add,
        stacked_to_remove,
        grants,
        daily_window: window,
    })
}

// ---- The listing

/// One offer as `GET /product/skus/public` writes it. `price_of` is `EffectivePrice`.
pub fn sku_listing(settings: &Settings, sku: &StoreSku) -> Value {
    let raw = &sku.raw;
    let mut out = Map::new();
    let copy = |out: &mut Map<String, Value>, key: &str| {
        if let Some(value) = raw.get(key) {
            out.insert(key.into(), value.clone());
        }
    };
    copy(&mut out, "id");
    copy(&mut out, "displayName");
    copy(&mut out, "displayDescription");
    copy(&mut out, "displayPriority");
    out.insert(
        "prices".into(),
        Value::Array(
            sku.prices
                .iter()
                .map(|p| {
                    let price = if effective_price(settings, &p.currency_id, 1.0) == 0.0 {
                        Value::from(0)
                    } else {
                        p.price.clone()
                    };
                    json!({ "currencyId": p.currency_id, "price": price, "salesPrice": null, "multiPrice": null })
                })
                .collect(),
        ),
    );
    out.insert("maxAllowed".into(), sku.max_allowed.clone());
    out.insert(
        "remaining".into(),
        if sku.max_allowed.is_null() {
            999.into()
        } else {
            sku.max_allowed.clone()
        },
    );
    out.insert(
        "duplicateInstancedItems".into(),
        raw.get("duplicateInstancedItems").cloned().unwrap_or(Value::Null),
    );
    out.insert(
        "images".into(),
        raw.get("images")
            .filter(|i| !i.is_null())
            .cloned()
            .unwrap_or_else(|| json!({})),
    );
    out.insert("tags".into(), raw.get("tags").cloned().unwrap_or(Value::Null));
    out.insert("scheduledTags".into(), Value::Null);
    out.insert(
        "items".into(),
        Value::Array(
            sku.items
                .iter()
                .map(|i| json!({ "catalogId": i.catalog_id, "quantity": i.quantity }))
                .collect(),
        ),
    );
    out.insert(
        "entitlements".into(),
        Value::Array(
            sku.entitlements
                .iter()
                .map(|(name, duration)| json!({ "name": name, "duration": duration }))
                .collect(),
        ),
    );
    out.insert(
        "skuProgression".into(),
        sku.sku_progression.as_ref().map_or(Value::Null, |p| {
            let mut m = Map::new();
            for key in ["progressionId", "ranks", "xp"] {
                if let Some(value) = p.get(key) {
                    m.insert(key.into(), value.clone());
                }
            }
            Value::Object(m)
        }),
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sku(value: Value) -> StoreSku {
        StoreSku::from_value(&value).unwrap()
    }

    fn ladyluck(id: &str, currency: &str, price: u32) -> StoreSku {
        sku(json!({
            "id": id, "displayName": "N", "displayDescription": "D", "displayPriority": 1,
            "prices": [{ "currencyId": currency, "price": price }], "maxAllowed": 1,
            "tags": ["ladyluckstore"],
            "items": [{ "catalogId": "WP_X", "quantity": 1, "instanced": true }],
            "duplicateInstancedItems": [],
        }))
    }

    #[test]
    fn the_daily_window_opens_at_17_utc() {
        let before = super::super::util::parse_ms("2026-10-10T16:59:59.999Z").unwrap();
        let at = super::super::util::parse_ms("2026-10-10T17:00:00.000Z").unwrap();
        assert_eq!(daily_reset_window(before).key, "2026-10-09T17:00:00.000Z");
        let window = daily_reset_window(at);
        assert_eq!(window.key, "2026-10-10T17:00:00.000Z");
        assert_eq!(window.end_ms - window.start_ms, DAY_MS);
        assert_eq!(daily_reset_window(at + 3_600_000).key, window.key);
    }

    #[test]
    fn transaction_ids_are_deterministic_node_hashes() {
        // sha256("ladyluck-purchase:user-1:sku-1").slice(0, 32).toUpperCase()
        let settings = Settings::default();
        let plan = plan_purchase(
            &settings,
            &ladyluck("sku-1", "id_currency_marks_steel", 5),
            "ladyluckstore",
            "user-1",
            0,
            "ff",
        )
        .unwrap();
        assert_eq!(plan.transaction_id.len(), 32);
        assert!(
            plan.transaction_id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b))
        );
        assert_eq!(
            plan.stacked_to_remove,
            vec![json!({ "catalogId": "CURRENCY_MARKS_STEEL", "quantity": 5 })]
        );
        assert_eq!(plan.instanced_to_add[0]["instanceId"].as_str().unwrap().len(), 26);
    }

    #[test]
    fn platinum_prices_are_free_and_never_a_debit() {
        let free = Settings::default();
        let paid = Settings::from_pairs([("PLATINUM_STORE_FREE", "0")]);
        let offer = ladyluck("p1", "id_currency_platinum", 100);
        assert!(
            plan_purchase(&free, &offer, "webstore", "u", 0, "a")
                .unwrap()
                .stacked_to_remove
                .is_empty()
        );
        let plan = plan_purchase(&paid, &offer, "webstore", "u", 0, "a").unwrap();
        assert_eq!(
            plan.stacked_to_remove,
            vec![json!({ "catalogId": "CURRENCY_PLATINUM", "quantity": 100 })]
        );
    }

    #[test]
    fn an_invalid_price_is_refused() {
        let settings = Settings::default();
        let mut offer = ladyluck("p1", "id_currency_marks_steel", 1);
        offer.prices[0].price = json!(-1);
        assert_eq!(
            plan_purchase(&settings, &offer, "ladyluckstore", "u", 0, "a"),
            Err(PurchaseReason::TransactionFailed)
        );
        offer.prices[0].price = json!("5");
        assert_eq!(
            plan_purchase(&settings, &offer, "ladyluckstore", "u", 0, "a"),
            Err(PurchaseReason::TransactionFailed)
        );
    }

    #[test]
    fn sections_of_one_store_share_one_scope() {
        let settings = Settings::default();
        let offer = ladyluck("dye1", "id_currency_platinum", 10);
        let a = plan_purchase(&settings, &offer, "webstore", "u", 0, "a").unwrap();
        let b = plan_purchase(&settings, &offer, "dyes", "u", 0, "b").unwrap();
        assert_eq!(a.transaction_id, b.transaction_id);
        assert_eq!(transaction_scope_for("ladyluckstore").0, "ladyluck-purchase");
        assert_eq!(transaction_scope_for("season_store").0, "store-purchase:season_store");
    }

    #[test]
    fn a_daily_claim_adds_its_window() {
        let settings = Settings::default();
        let mut offer = ladyluck("fountain", "id_currency_platinum", 0);
        offer.items[0].instanced = false;
        let one = super::super::util::parse_ms("2026-10-10T16:30:00.000Z").unwrap();
        let next = super::super::util::parse_ms("2026-10-10T17:00:00.000Z").unwrap();
        let first = plan_purchase(&settings, &offer, FOUNTAIN_DAILY_BUNDLE_TAG, "u", one, "a").unwrap();
        let same = plan_purchase(&settings, &offer, FOUNTAIN_DAILY_BUNDLE_TAG, "u", one + 1000, "b").unwrap();
        let later = plan_purchase(&settings, &offer, FOUNTAIN_DAILY_BUNDLE_TAG, "u", next, "c").unwrap();
        assert_eq!(first.transaction_id, same.transaction_id);
        assert_ne!(first.transaction_id, later.transaction_id);
        assert_eq!(
            first.transaction_id,
            daily_claim_transaction_id(FOUNTAIN_DAILY_BUNDLE_TAG, "u", "fountain", "2026-10-09T17:00:00.000Z")
        );
    }

    #[test]
    fn lookups_find_the_exact_id_first_then_a_unique_catalog_id() {
        let a = ladyluck("a", "id_currency_marks_steel", 1);
        let mut b = ladyluck("b", "id_currency_marks_steel", 1);
        b.items[0].catalog_id = "WP_Y".into();
        let mut dye = ladyluck("dye", "id_currency_platinum", 1);
        dye.items[0].catalog_id = "DYE_1".into();
        let catalog = StoreCatalog::from_sections(vec![
            ("ladyluckstore", vec![a, b]),
            ("webstore", vec![dye.clone()]),
            ("dyes", vec![dye]),
        ]);
        assert_eq!(catalog.find_by_id_or_catalog_id("b", false).unwrap().0, "ladyluckstore");
        assert_eq!(catalog.find_by_id_or_catalog_id("WP_Y", false).unwrap().1.id, "b");
        // the same dye in two sections is one offer
        assert_eq!(catalog.find_by_id_or_catalog_id("DYE_1", false).unwrap().1.id, "dye");
        // WP_X belongs to "a" only; "nope" to nothing
        assert!(catalog.find_by_id_or_catalog_id("nope", false).is_none());
    }

    #[test]
    fn two_offers_with_one_item_are_ambiguous() {
        let a = ladyluck("a", "id_currency_marks_steel", 1);
        let b = ladyluck("b", "id_currency_marks_steel", 1);
        let catalog = StoreCatalog::from_sections(vec![("ladyluckstore", vec![a, b])]);
        assert!(catalog.find_by_id_or_catalog_id("WP_X", false).is_none());
        // 1.12.0 took the first SKU holding the item
        assert_eq!(catalog.find_by_id_or_catalog_id("WP_X", true).unwrap().1.id, "a");
        assert!(catalog.find_by_id_or_catalog_id("a", false).is_some());
    }
}
