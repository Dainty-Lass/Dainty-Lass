// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The Reward Cache store, rebuilt from a curated plan the version supplies (spec §5.4). The client no longer
//! receives the seasonal store, so the server serves it from catalog items the plan names.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use serde_json::{Map, Value, json};

use super::io::{Catalog, Item, checked_texture, create_parent, read_catalog, show, write_file};
use super::pretty::stringify_pretty;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RewardCacheCategory {
    Armour,
    Weapon,
    Style,
    Event,
    Limited,
    General,
}

impl RewardCacheCategory {
    /// In the store's order.
    pub const ALL: [Self; 6] = [
        Self::Armour,
        Self::Weapon,
        Self::Style,
        Self::Event,
        Self::Limited,
        Self::General,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Armour => "armour",
            Self::Weapon => "weapon",
            Self::Style => "style",
            Self::Event => "event",
            Self::Limited => "limited",
            Self::General => "general",
        }
    }

    /// The client's tab tag.
    pub const fn tag(self) -> &'static str {
        match self {
            Self::Armour => "tab_season_armour",
            Self::Weapon => "tab_season_weapon",
            Self::Style => "tab_season_style",
            Self::Event => "season_store_event",
            Self::Limited => "tab_season_limited",
            Self::General => "season_store_general",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.as_str() == text)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewardCacheSelection {
    pub catalog_id: String,
    pub category: RewardCacheCategory,
    pub price: i64,
    pub featured: bool,
    /// An explicit rarity-3 premium selection
    pub allow_rarity3: bool,
    pub display_name: Option<String>,
    pub description: Option<String>,
}

/// Authored data over game identifiers: an input the version crate supplies (spec §13, item 2).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RewardCachePlan {
    pub selections: Vec<RewardCacheSelection>,
    /// Ordered catalog-id substrings ranking armour sets
    pub armour_families: Vec<String>,
    /// Ordered substrings ranking banner families
    pub banner_families: Vec<String>,
    pub expected: Vec<(RewardCacheCategory, usize)>,
    pub expected_featured: usize,
}

fn strings(value: Option<&Value>, what: &str) -> Result<Vec<String>, String> {
    let Some(items) = value.and_then(Value::as_array) else {
        return Err(format!("Reward Cache plan: {what} must be an array of strings"));
    };
    items
        .iter()
        .map(|v| v.as_str().map(str::to_owned))
        .collect::<Option<_>>()
        .ok_or_else(|| format!("Reward Cache plan: {what} must be an array of strings"))
}

fn count(value: &Value, what: &str) -> Result<usize, String> {
    value
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| format!("Reward Cache plan: {what} must be a whole number"))
}

impl RewardCachePlan {
    /// `{"selections":[{"catalogId","category","price","featured","allowRarity3","displayName","description"}],
    /// "armourFamilies":[..],"bannerFamilies":[..],"expected":{"armour":n,...},"expectedFeatured":n}`
    pub fn from_json(value: &Value) -> Result<Self, String> {
        let Some(list) = value.get("selections").and_then(Value::as_array) else {
            return Err("Reward Cache plan: selections must be an array".into());
        };
        let mut selections = Vec::with_capacity(list.len());
        for (i, s) in list.iter().enumerate() {
            let bad = |what: &str| format!("Reward Cache plan: selection {i}: {what}");
            let catalog_id = s
                .get("catalogId")
                .and_then(Value::as_str)
                .ok_or_else(|| bad("no catalogId"))?;
            let category = s
                .get("category")
                .and_then(Value::as_str)
                .and_then(RewardCacheCategory::parse)
                .ok_or_else(|| bad("unknown category"))?;
            let price = s
                .get("price")
                .and_then(Value::as_i64)
                .ok_or_else(|| bad("no whole price"))?;
            let flag = |name: &str| s.get(name).and_then(Value::as_bool).unwrap_or(false);
            let text = |name: &str| s.get(name).and_then(Value::as_str).map(str::to_owned);
            selections.push(RewardCacheSelection {
                catalog_id: catalog_id.to_owned(),
                category,
                price,
                featured: flag("featured"),
                allow_rarity3: flag("allowRarity3"),
                display_name: text("displayName"),
                description: text("description"),
            });
        }
        let mut expected = Vec::new();
        if let Some(map) = value.get("expected").and_then(Value::as_object) {
            for (name, n) in map {
                let category = RewardCacheCategory::parse(name)
                    .ok_or_else(|| format!("Reward Cache plan: unknown category {name}"))?;
                expected.push((category, count(n, "expected counts")?));
            }
        }
        Ok(Self {
            selections,
            armour_families: strings(value.get("armourFamilies"), "armourFamilies")?,
            banner_families: strings(value.get("bannerFamilies"), "bannerFamilies")?,
            expected,
            expected_featured: count(
                value.get("expectedFeatured").unwrap_or(&Value::Null),
                "expectedFeatured",
            )?,
        })
    }

    fn expected(&self, category: RewardCacheCategory) -> usize {
        self.expected
            .iter()
            .find(|(c, _)| *c == category)
            .map_or(0, |(_, n)| *n)
    }
}

const PRICES: [i64; 6] = [700, 800, 1000, 2000, 2500, 3000];
const EXCLUSIVE: [&str; 4] = ["FOUNDER", "TRANSLATOR", "TRIALS", "MASTERY"];
const FORBIDDEN_TAGS: [&str; 6] = ["deprecated", "hidden", "initial", "masteryreward", "trialstore", "wip"];
const SLOTS: [&str; 4] = ["Helm", "Chest", "Arms", "Legs"];

fn family(id: &str, families: &[String]) -> usize {
    families
        .iter()
        .position(|f| id.contains(f.as_str()))
        .unwrap_or(families.len())
}

fn banner_component(id: &str) -> usize {
    let parts: [&[&str]; 4] = [
        &["BNC_STANDARD_", "BNC_MESH_"],
        &["BNC_FABRIC_"],
        &["BNC_SIGIL_"],
        &["BNC_ANIMATION_"],
    ];
    parts
        .iter()
        .position(|prefixes| prefixes.iter().any(|p| id.starts_with(p)))
        .unwrap_or(parts.len())
}

/// The store's order; the sort is stable, so ties keep the authored order.
fn sort_key(s: &RewardCacheSelection, plan: &RewardCachePlan, catalog: &Catalog) -> [i64; 10] {
    let id = s.catalog_id.as_str();
    let item = catalog.get(id);
    let mut key = [0i64; 10];
    key[0] = i64::from(!s.featured);
    key[1] = s.category as i64;
    let index = |n: usize| i64::try_from(n).unwrap_or(i64::MAX);
    match s.category {
        RewardCacheCategory::Armour => {
            key[2] = index(family(id, &plan.armour_families));
            key[3] = i64::from(!id.ends_with("_ULTRA"));
            let tags = item.map(Item::tags).unwrap_or_default();
            key[4] = index(SLOTS.iter().position(|slot| tags.contains(slot)).unwrap_or(SLOTS.len()));
        }
        RewardCacheCategory::Weapon if id.starts_with("WP_GA_") => {
            key[5] = -1;
            key[6] = -item.map_or(0, |i| i.rarity);
        }
        RewardCacheCategory::Event if id.starts_with("BNC_") => {
            key[7] = -1;
            key[8] = index(family(id, &plan.banner_families));
            key[9] = index(banner_component(id));
        }
        _ => {}
    }
    key
}

/// The catalog id lower-cased, each run of other characters than ASCII letters and digits one `_`.
pub(crate) fn slug(id: &str) -> String {
    let lower = id.to_lowercase();
    let mut out = String::new();
    let mut gap = false;
    for c in lower.chars() {
        if c.is_ascii_alphanumeric() {
            if gap && !out.is_empty() {
                out.push('_');
            }
            gap = false;
            out.push(c);
        } else {
            gap = true;
        }
    }
    out
}

fn trimmed(text: Option<&str>) -> Option<String> {
    text.map(|t| service_kit::js::trim(t).to_owned())
        .filter(|t| !t.is_empty())
}

fn sku(s: &RewardCacheSelection, i: usize, item: &Item, used_skus: &mut HashSet<String>) -> Result<Value, String> {
    let id = s.catalog_id.as_str();
    let override_name = trimmed(s.display_name.as_deref());
    let name = override_name
        .clone()
        .or_else(|| trimmed(item.str("displayName")))
        .ok_or_else(|| format!("Missing display name: {id}"))?;
    if override_name.is_none() {
        let lower = name.to_lowercase();
        if lower.starts_with("~dnt~") || lower.starts_with("<missing") || lower == "tbd" || lower == "empty slot" {
            return Err(format!("Unresolved display name for {id}: {name}"));
        }
    }
    if let Some(tag) = item.has_tag_ci(&FORBIDDEN_TAGS) {
        return Err(format!("Forbidden catalog tag '{tag}' on {id}"));
    }
    let rarity = item.rarity;
    let glider = item.has_tag_ci(&["glider"]).is_some();
    if !(rarity >= 4 || (glider && rarity >= 3) || (s.allow_rarity3 && rarity == 3)) {
        return Err(format!(
            "Selected item must be rarity 4+, a resolved rarity-3 glider, or an explicit rarity-3 premium selection: \
             {id} has rarity {rarity}"
        ));
    }
    let image = item
        .image()
        .ok_or_else(|| format!("Missing catalog image path: {id}"))?;
    let sku_id = format!("season19_rewardcache_{}_{}", s.category.as_str(), slug(id));
    if !used_skus.insert(sku_id.clone()) {
        return Err(format!("Duplicate generated SKU id: {sku_id}"));
    }
    let texture = checked_texture(image)?;
    let description = trimmed(s.description.as_deref())
        .or_else(|| trimmed(item.str("description")))
        .unwrap_or_else(|| format!("A premium Reward Cache cosmetic: {name}."));
    let mut tags = vec!["season_store", s.category.tag()];
    if s.featured {
        tags.push("season_store_feature");
    }
    tags.extend(["season19", "reward_cache_reconstructed"]);
    let mut tags: Vec<String> = tags.into_iter().map(str::to_owned).collect();
    tags.push(format!("reward_cache_rarity_{rarity}"));
    let mut out = Map::new();
    out.insert("id".into(), Value::from(sku_id));
    out.insert("displayName".into(), Value::from(name));
    out.insert("displayDescription".into(), Value::from(description));
    out.insert("displayPriority".into(), Value::from(i + 1));
    out.insert(
        "prices".into(),
        json!([{ "currencyId": "id_currency_seasonal_coin", "price": s.price }]),
    );
    out.insert("maxAllowed".into(), json!(1));
    out.insert("images".into(), json!({ "standard": texture }));
    out.insert("tags".into(), json!(tags));
    out.insert(
        "items".into(),
        json!([{ "catalogId": id, "quantity": 1, "instanced": !item.stackable() }]),
    );
    out.insert("duplicateInstancedItems".into(), json!([id]));
    Ok(Value::Object(out))
}

pub fn generate_reward_cache_store(catalog: &Path, output: &Path, plan: &RewardCachePlan) -> Result<String, String> {
    let catalog = read_catalog(catalog)?;
    let mut order: Vec<&RewardCacheSelection> = plan.selections.iter().collect();
    order.sort_by_cached_key(|s| sort_key(s, plan, &catalog));

    let mut used = HashSet::new();
    let mut used_skus = HashSet::new();
    let mut skus = Vec::with_capacity(order.len());
    let mut counts: BTreeMap<RewardCacheCategory, usize> = BTreeMap::new();
    let mut prices: BTreeMap<i64, usize> = BTreeMap::new();
    let mut featured = 0;
    for (i, s) in order.into_iter().enumerate() {
        let id = s.catalog_id.as_str();
        if !used.insert(id) {
            return Err(format!("Duplicate catalog selection: {id}"));
        }
        if !PRICES.contains(&s.price) {
            return Err(format!("Unsupported price {} for {id}", s.price));
        }
        let upper = id.to_uppercase();
        if EXCLUSIVE.iter().any(|word| upper.contains(word)) {
            return Err(format!("Exclusive/progression catalog id is not eligible: {id}"));
        }
        if id == "WP_GA_SPOOKY_01" {
            return Err(format!("Quest/Rumour-only catalog item is not eligible: {id}"));
        }
        let item = catalog.get(id).ok_or_else(|| format!("Catalog item not found: {id}"))?;
        skus.push(sku(s, i, item, &mut used_skus)?);
        *counts.entry(s.category).or_default() += 1;
        *prices.entry(s.price).or_default() += 1;
        featured += usize::from(s.featured);
    }
    for category in RewardCacheCategory::ALL {
        let (n, m) = (counts.get(&category).copied().unwrap_or(0), plan.expected(category));
        if n != m {
            return Err(format!("{} count {n} != expected {m}", category.as_str()));
        }
    }
    if featured != plan.expected_featured {
        return Err(format!(
            "Featured count {featured} != expected {}",
            plan.expected_featured
        ));
    }
    create_parent(output)?;
    let total = skus.len();
    write_file(output, &format!("{}\n", stringify_pretty(&Value::Array(skus), 2)))?;
    let categories: Vec<String> = RewardCacheCategory::ALL
        .iter()
        .map(|c| format!("{}={}", c.as_str(), counts.get(c).copied().unwrap_or(0)))
        .collect();
    let prices: Vec<String> = prices.iter().map(|(p, n)| format!("{p}={n}")).collect();
    Ok([
        format!("Wrote {}", show(output)),
        format!("SKUs: {total}; featured: {featured}"),
        format!("Categories: {}", categories.join(", ")),
        format!("Prices: {}", prices.join(", ")),
    ]
    .join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slug("AR_X_Helm_00_ULTRA"), "ar_x_helm_00_ultra");
        assert_eq!(slug("__A--b..9__"), "a_b_9");
        assert_eq!(slug("É_x"), "x");
    }
}
