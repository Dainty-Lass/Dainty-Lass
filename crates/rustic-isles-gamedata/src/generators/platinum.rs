// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The platinum store (spec §5.7): captured offers keep their captured price; the client's store table rows that
//! decode to one catalog item get a reconstructed one.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use dainty_service_kit::js;
use regex::Regex;
use serde_json::{Map, Value, json};

use super::io::{
    Catalog, Item, create_dir, drop_bom, number, parse, read_catalog, read_json_lines, read_text, show,
    store_image_rows, texture, write_file,
};
use super::pretty::stringify_pretty;

const PLATINUM: &str = "id_currency_platinum";
const WEAPON_CODES: [&str; 7] = ["ac", "eb", "dp", "ih", "ga", "ms", "cb"];
const EXCLUDED_TAGS: [&str; 8] = [
    "deprecated",
    "hidden",
    "wip",
    "initial",
    "founder",
    "translator",
    "mastery",
    "partner",
];
const ROW_FAMILIES: [&str; 3] = ["sku_", "single_", "contract_"];
const TYPE_PREFIXES: [&str; 12] = [
    "AR_", "WP_", "DYE_", "EM_", "TITLE_", "BNC_", "GD_", "LT_", "AC_", "QI_", "EJ_", "FL_",
];

/// The tab table, in output order: (tag, category).
const TABS: [(&str, &str); 20] = [
    ("skin_weapon_ac", "weapon_ac"),
    ("skin_weapon_eb", "weapon_eb"),
    ("skin_weapon_dp", "weapon_dp"),
    ("skin_weapon_ih", "weapon_ih"),
    ("skin_weapon_ga", "weapon_ga"),
    ("skin_weapon_ms", "weapon_ms"),
    ("skin_weapon_cb", "weapon_cb"),
    ("skin_armour", "armour"),
    ("skin_lantern", "lantern"),
    ("dye_armour", "dye"),
    ("personality_character", "slayer"),
    ("social_emote", "emote"),
    ("social_emojis", "emoji"),
    ("social_arrival", "arrival"),
    ("personality_fabric", "fabric"),
    ("personality_standard", "standard"),
    ("personality_flaresigil", "flaresigil"),
    ("hunting_gliders", "glider"),
    ("supplies_supplies", "tonics"),
    // No id maps to boosts; kept with its price (spec §12 item 11)
    ("supplies_boost", "boosts"),
];

static QUEST_ROW: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"^quest_rumour_(.+?)(?:_\d+)?$").ok());
static CHARACTER_ENTITLEMENT: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^(ent_)?(ccfp|cchd|cchs|ccbd)_").ok());

/// The category of a catalog id, or none.
pub fn category(id: &str) -> Option<String> {
    if let Some(rest) = id.strip_prefix("WP_") {
        let code = rest.split_once('_').map(|(code, _)| code.to_lowercase())?;
        return WEAPON_CODES.contains(&code.as_str()).then(|| format!("weapon_{code}"));
    }
    let table: [(&[&str], &str); 11] = [
        (&["AR_"], "armour"),
        (&["LT_"], "lantern"),
        (&["DYE_"], "dye"),
        (&["EM_INTRO"], "arrival"),
        (&["EM_"], "emote"),
        (&["EJ_"], "emoji"),
        (&["BNC_FABRIC"], "fabric"),
        (&["BNC_STANDARD"], "standard"),
        (&["BNC_SIGIL", "BNC_ANIMATION", "FL_"], "flaresigil"),
        (&["GD_"], "glider"),
        (&["QI_"], "tonics"),
    ];
    table
        .iter()
        .find(|(prefixes, _)| prefixes.iter().any(|p| id.starts_with(p)))
        .map(|(_, c)| (*c).to_owned())
}

fn tag_of(category: &str) -> Option<&'static str> {
    TABS.iter().find(|(_, c)| *c == category).map(|(t, _)| *t)
}

/// The platinum price of an item with no captured offer, by store category and rarity.
pub fn reconstructed_price(category: &str, rarity: i64) -> i64 {
    match (category, rarity) {
        (c, r) if c.starts_with("weapon_") => match r {
            3 => 500,
            5 => 1200,
            _ => 800,
        },
        ("armour", 3) => 400,
        ("armour", 5) | ("lantern", 5) => 800,
        ("armour" | "lantern", _) => 600,
        ("dye" | "slayer" | "tonics" | "boosts", _) => 100,
        ("emote", 3) => 300,
        ("emote", 5) => 700,
        ("emote", _) => 500,
        ("emoji", _) => 200,
        ("arrival", _) => 1000,
        ("fabric", _) => 300,
        ("standard", _) => 400,
        ("flaresigil", _) => 250,
        ("glider", _) => 800,
        _ => 100,
    }
}

/// A row name's catalog id: the family prefix removed, the rest upper-cased, then that id or a type prefix plus it.
pub(crate) fn decode(row: &str, catalog: &Catalog) -> Option<String> {
    let rest = ROW_FAMILIES.iter().find_map(|p| row.strip_prefix(p))?;
    let upper = rest.to_ascii_uppercase();
    if catalog.contains_key(&upper) {
        return Some(upper);
    }
    TYPE_PREFIXES
        .iter()
        .map(|p| format!("{p}{upper}"))
        .find(|id| catalog.contains_key(id))
}

/// `JSON.stringify` drops a key whose value is `undefined`.
fn copy(out: &mut Map<String, Value>, name: &str, from: &Value) {
    if let Some(v) = from.get(name) {
        out.insert(name.into(), v.clone());
    }
}

fn baked_art(value: Option<&Value>) -> bool {
    value.and_then(Value::as_str).is_some_and(|s| s != "None")
}

struct Inputs {
    catalog: Catalog,
    baked: BTreeMap<String, Value>,
    sold: HashSet<String>,
    quest: HashSet<String>,
}

impl Inputs {
    fn grantable(&self, id: &str) -> bool {
        self.catalog
            .get(id)
            .is_some_and(|i| i.has_tag_ci(&EXCLUDED_TAGS).is_none())
    }

    /// `{}` when the client draws its own art, a catalog texture, or none.
    fn images(&self, row: &str, ids: &[&str]) -> Option<Value> {
        let baked = self.baked.get(row);
        if baked_art(baked.and_then(|r| r.get("standardImage"))) {
            return Some(json!({}));
        }
        // Only the /Game/ prefix is required here (spec §11's weak spot): the client parses the reference
        let icon = ids.iter().filter_map(|id| self.catalog.get(*id)).find_map(|item| {
            [&item.custom_icon, &item.large_icon, &item.icon]
                .into_iter()
                .find(|icon| icon.starts_with("/Game/"))
        });
        if let Some(icon) = icon {
            return Some(json!({ "standard": texture(icon) }));
        }
        baked_art(baked.and_then(|r| r.get("featureImage"))).then(|| json!({}))
    }
}

fn platinum_price(sku: &Value) -> Option<Value> {
    sku.get("prices")?
        .as_array()?
        .iter()
        .find(|p| p.get("currencyId").and_then(Value::as_str) == Some(PLATINUM))?
        .get("price")
        .filter(|p| !p.is_null())
        .cloned()
}

/// `Number(x ?? default) || default`
fn number_or(value: Option<&Value>, default: f64) -> Value {
    let n = match value {
        None | Some(Value::Null) => default,
        v => js::number(v),
    };
    number(if n == 0.0 || n.is_nan() { default } else { n })
}

fn entitlements(sku: &Value) -> Vec<Value> {
    sku.get("entitlements")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn entitlement_list(list: &[Value]) -> Value {
    Value::Array(
        list.iter()
            .map(|e| {
                let mut out = Map::new();
                copy(&mut out, "name", e);
                out.insert("duration".into(), number_or(e.get("duration"), 0.0));
                Value::Object(out)
            })
            .collect(),
    )
}

fn dup_instanced(ids: &[&str]) -> Value {
    json!(
        ids.iter()
            .filter(|id| !["QI_", "TOKEN_", "CURRENCY_"].iter().any(|p| id.starts_with(p)))
            .collect::<Vec<_>>()
    )
}

/// A SKU's fixed head, in store order.
fn head(sku: &Value, id: &str, name: Value, priority: Value, price: Value, max: Value) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("id".into(), Value::from(id));
    out.insert("displayName".into(), name);
    out.insert("displayDescription".into(), or(sku, "displayDescription", json!("")));
    out.insert("displayPriority".into(), priority);
    out.insert("prices".into(), json!([{ "currencyId": PLATINUM, "price": price }]));
    out.insert("maxAllowed".into(), max);
    out
}

fn or(sku: &Value, name: &str, default: Value) -> Value {
    match sku.get(name) {
        None | Some(Value::Null) => default,
        Some(v) => v.clone(),
    }
}

/// Skips counted by reason (report text; the counts are not reported).
#[derive(Default)]
struct Skips(BTreeMap<&'static str, usize>);

impl Skips {
    fn add(&mut self, reason: &'static str) {
        *self.0.entry(reason).or_default() += 1;
    }
}

struct Build {
    inputs: Inputs,
    skus: Vec<Value>,
    claimed_skus: HashSet<String>,
    claimed_items: HashSet<String>,
    skips: Skips,
    captured: usize,
}

impl Build {
    fn read_capture(dir: &Path, name: &str) -> Result<Vec<Value>, String> {
        let path = dir.join(name);
        let Ok(text) = read_text(&path) else {
            return Ok(Vec::new());
        };
        let value = parse(drop_bom(&text)).map_err(|e| format!("invalid JSON in {}: {e}", show(&path)))?;
        Ok(match value {
            Value::Array(items) => items.into_iter().filter(Value::is_object).collect(),
            _ => Vec::new(),
        })
    }

    /// The reason an item can't be offered, or none.
    fn item_refusal(&self, id: &str, tier_a: bool) -> Option<&'static str> {
        let (grant, sold, claimed, quest) = if tier_a {
            (
                "tierA: item absent from 1.12 or excluded content",
                "tierA: overlaps Lady Luck / Reward Cache",
                "tierA: catalog id already sold by another platinum SKU",
                "tierA: quest/Rumour-earned",
            )
        } else {
            (
                "tierB: excluded catalog content",
                "tierB: overlaps Lady Luck / Reward Cache",
                "tierB: catalog id already sold by another platinum SKU",
                "tierB: quest/Rumour-earned",
            )
        };
        if tier_a && !self.inputs.grantable(id) {
            return Some(grant);
        }
        if self.inputs.sold.contains(id) {
            return Some(sold);
        }
        if self.claimed_items.contains(id) {
            return Some(claimed);
        }
        if !tier_a && !self.inputs.grantable(id) {
            return Some(grant);
        }
        self.inputs.quest.contains(id).then_some(quest)
    }

    fn captured_offer(&mut self, sku: &Value) {
        let id = sku.get("id").and_then(Value::as_str).unwrap_or("");
        if self.claimed_skus.contains(id) {
            return;
        }
        if !self.inputs.baked.contains_key(id) {
            return self.skips.add("tierA: sku id not baked into 1.12 StoreItemsTable");
        }
        if js::truthy(sku.get("platformOfferId")) {
            return self.skips.add("tierA: real-money platform offer");
        }
        if js::truthy(sku.get("loadoutSlots")) {
            return self.skips.add("tierA: loadoutSlots grant unimplemented");
        }
        let Some(price) = platinum_price(sku) else {
            return self.skips.add("tierA: not platinum-priced");
        };
        let items: Vec<Value> = sku.get("items").and_then(Value::as_array).cloned().unwrap_or_default();
        let ents = entitlements(sku);
        let progression = sku.get("skuProgression").filter(|p| !p.is_null());
        if items.is_empty() && ents.is_empty() && progression.is_none() {
            return self.skips.add("tierA: no payload of any kind");
        }
        let ids: Vec<&str> = items
            .iter()
            .map(|i| i.get("catalogId").and_then(Value::as_str).unwrap_or(""))
            .collect();
        for item in &ids {
            if let Some(reason) = self.item_refusal(item, true) {
                return self.skips.add(reason);
            }
        }
        let mut tags: BTreeSet<&str> = ids
            .iter()
            .filter_map(|i| category(i))
            .filter_map(|c| tag_of(&c))
            .collect();
        let mut section = false;
        if items.is_empty() && !ents.is_empty() {
            let character = ents.iter().all(|e| {
                let name = e.get("name").and_then(Value::as_str).unwrap_or("");
                CHARACTER_ENTITLEMENT.as_ref().is_some_and(|re| re.is_match(name))
                    || name.starts_with("character_facepaint_")
            });
            if character {
                tags.insert("personality_character");
            } else {
                section = true;
            }
        }
        if tags.is_empty() && !section {
            return self.skips.add("tierA: content type has no 1.12 store tab");
        }
        let Some(images) = self.inputs.images(id, &ids) else {
            return self.skips.add("tierA: no baked or catalog artwork");
        };
        self.claimed_skus.insert(id.to_owned());
        self.claimed_items.extend(ids.iter().map(|i| (*i).to_owned()));

        let name = or(sku, "displayName", Value::from(id));
        let priority = number_or(sku.get("displayPriority"), 1000.0);
        let max = if ents.is_empty() {
            or(sku, "maxAllowed", Value::Null)
        } else {
            json!(1)
        };
        let mut out = head(sku, id, name, priority, price, max);
        out.insert("images".into(), images);
        let mut all_tags: Vec<&str> = if section {
            vec!["huntpass_store"]
        } else if ids.iter().any(|i| i.starts_with("DYE_")) {
            vec!["webstore", "dyes"]
        } else {
            vec!["webstore"]
        };
        all_tags.extend(tags);
        all_tags.push("platinum_store_captured");
        out.insert("tags".into(), json!(all_tags));
        let built: Vec<Value> = items
            .iter()
            .zip(&ids)
            .map(|(item, id)| {
                let mut entry = Map::new();
                copy(&mut entry, "catalogId", item);
                copy(&mut entry, "quantity", item);
                let instanced = self
                    .inputs
                    .catalog
                    .get(*id)
                    .is_some_and(|i| i.row.get("isStackable") == Some(&Value::Bool(false)));
                entry.insert("instanced".into(), Value::Bool(instanced));
                Value::Object(entry)
            })
            .collect();
        out.insert("items".into(), Value::Array(built));
        out.insert("duplicateInstancedItems".into(), dup_instanced(&ids));
        if !ents.is_empty() {
            out.insert("entitlements".into(), entitlement_list(&ents));
        }
        if js::truthy(sku.get("skuProgression")) {
            copy(&mut out, "skuProgression", sku);
        }
        self.skus.push(Value::Object(out));
        self.captured += 1;
    }

    fn rank_skip(&mut self, sku: &Value) {
        let id = sku.get("id").and_then(Value::as_str).unwrap_or("");
        if !self.inputs.baked.contains_key(id) {
            return self.skips.add("rank skip: sku id not baked");
        }
        let price = platinum_price(sku);
        let (Some(price), true) = (price, js::truthy(sku.get("skuProgression"))) else {
            return self.skips.add("rank skip: missing price or progression");
        };
        if !self.claimed_skus.insert(id.to_owned()) {
            return;
        }
        let name = or(sku, "displayName", json!("Hunt Pass Level Skip"));
        let priority = number_or(sku.get("displayPriority"), 1000.0);
        let mut out = head(sku, id, name, priority, price, or(sku, "maxAllowed", Value::Null));
        out.insert("images".into(), self.inputs.images(id, &[]).unwrap_or(json!({})));
        out.insert(
            "tags".into(),
            json!(["hp_level_skip_rank", "event", "platinum_store_captured"]),
        );
        out.insert("items".into(), json!([]));
        out.insert("duplicateInstancedItems".into(), json!([]));
        copy(&mut out, "skuProgression", sku);
        self.skus.push(Value::Object(out));
        self.captured += 1;
    }

    fn pass(&mut self, sku: &Value) {
        let id = sku.get("id").and_then(Value::as_str).unwrap_or("");
        if !self.inputs.baked.contains_key(id) {
            return self.skips.add("huntpass pass: sku id not baked");
        }
        let Some(price) = platinum_price(sku) else {
            return self
                .skips
                .add("huntpass pass: no platinum price (already-owned capture)");
        };
        let ents = entitlements(sku);
        if ents.is_empty() {
            return self.skips.add("huntpass pass: no entitlement payload");
        }
        if !self.claimed_skus.insert(id.to_owned()) {
            return;
        }
        let name = or(sku, "displayName", Value::from(id));
        let priority = number_or(sku.get("displayPriority"), 1000.0);
        let mut out = head(sku, id, name, priority, price, or(sku, "maxAllowed", json!(1)));
        out.insert("images".into(), self.inputs.images(id, &[]).unwrap_or(json!({})));
        out.insert(
            "tags".into(),
            json!(["huntpass_store", "event_pass", "platinum_store_captured"]),
        );
        out.insert("items".into(), json!([]));
        out.insert("duplicateInstancedItems".into(), json!([]));
        out.insert("entitlements".into(), entitlement_list(&ents));
        self.skus.push(Value::Object(out));
        self.captured += 1;
    }

    fn reconstructed(&mut self, row: &str) -> bool {
        if self.claimed_skus.contains(row) {
            return false;
        }
        if !ROW_FAMILIES.iter().any(|p| row.starts_with(p)) {
            self.skips
                .add("tierB: row family is a bundle/quest/pass, not a single decodable offer");
            return false;
        }
        let baked = self.inputs.baked.get(row);
        let art = |name: &str| baked_art(baked.and_then(|r| r.get(name)));
        if !art("standardImage") && !art("featureImage") {
            self.skips.add("tierB: baked row has no artwork");
            return false;
        }
        let Some(id) = decode(row, &self.inputs.catalog) else {
            self.skips.add("tierB: row name does not decode to a 1.12 catalog id");
            return false;
        };
        if let Some(reason) = self.item_refusal(&id, false) {
            self.skips.add(reason);
            return false;
        }
        let Some(item) = self.inputs.catalog.get(&id) else {
            return false;
        };
        let display = item.str("displayName").unwrap_or("");
        if display.is_empty() || display.contains("MISSING STRING TABLE") || display.starts_with("~DNT~") {
            self.skips.add("tierB: unresolved display name");
            return false;
        }
        let Some((category, tag)) = category(&id).and_then(|c| tag_of(&c).map(|t| (c, t))) else {
            self.skips.add("tierB: content type has no 1.12 store tab");
            return false;
        };
        let Some(images) = self.inputs.images(row, &[id.as_str()]) else {
            self.skips.add("tierB: no artwork resolved");
            return false;
        };
        let sku = self.reconstructed_sku(row, &id, item, &category, tag, images);
        self.claimed_skus.insert(row.to_owned());
        self.claimed_items.insert(id);
        self.skus.push(sku);
        true
    }

    fn reconstructed_sku(&self, row: &str, id: &str, item: &Item, category: &str, tag: &str, images: Value) -> Value {
        let name = or(&item.row, "displayName", Value::from(row));
        let max = if matches!(category, "tonics" | "boosts") {
            Value::Null
        } else {
            json!(1)
        };
        let price = json!(reconstructed_price(category, item.rarity));
        let mut out = head(&Value::Null, row, name, json!(1000), price, max);
        out.insert("displayDescription".into(), or(&item.row, "description", json!("")));
        out.insert("images".into(), images);
        let mut tags = vec!["webstore".to_owned()];
        if category == "dye" {
            tags.push("dyes".into());
        }
        tags.push(tag.to_owned());
        tags.push("platinum_store_reconstructed".into());
        tags.push(format!("platinum_store_rarity_{}", item.rarity));
        out.insert("tags".into(), json!(tags));
        let instanced = item.row.get("isStackable") == Some(&Value::Bool(false));
        out.insert(
            "items".into(),
            json!([{ "catalogId": id, "quantity": 1, "instanced": instanced }]),
        );
        out.insert("duplicateInstancedItems".into(), dup_instanced(&[id]));
        Value::Object(out)
    }
}

/// The SKU's sort text: `{section}|{tag}|{price}|{id}`.
fn sort_text(sku: &Value) -> String {
    let tags: Vec<&str> = sku
        .get("tags")
        .and_then(Value::as_array)
        .map(|t| t.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let section = if tags.contains(&"hp_level_skip_rank") {
        2
    } else if tags.contains(&"huntpass_store") {
        3
    } else {
        1
    };
    let tag = TABS.iter().map(|(t, _)| *t).find(|t| tags.contains(t)).unwrap_or("zzz");
    // `String(price).padStart(6, "0")`: compared as text, never cut
    let price = js::string(sku.pointer("/prices/0/price"));
    let id = js::string(sku.get("id"));
    format!("{section}|{tag}|{price:0>6}|{id}")
}

fn sold_ids(path: &Path, what: &str) -> Result<HashSet<String>, String> {
    if !path.is_file() {
        return Err(format!("{what} not found: {}", show(path)));
    }
    let text = read_text(path).map_err(|e| format!("invalid JSON in {}: {e}", show(path)))?;
    let value = parse(drop_bom(&text)).map_err(|e| format!("invalid JSON in {}: {e}", show(path)))?;
    let mut ids = HashSet::new();
    for sku in value.as_array().map_or(&[][..], Vec::as_slice) {
        for item in sku
            .get("items")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice)
        {
            if let Some(id) = item.get("catalogId").and_then(Value::as_str) {
                ids.insert(id.to_owned());
            }
        }
    }
    Ok(ids)
}

/// Builds the platinum store (§5.6) from the catalog, the store image rows and the captures, minus items the Reward Cache and Lady Luck stores already sell, and writes `platinum_store.json` in `data_dir`.
pub fn generate_platinum_store(
    catalog: &Path,
    store_images: &Path,
    capture_dir: &Path,
    data_dir: &Path,
) -> Result<String, String> {
    let catalog = read_catalog(catalog)?;
    if !store_images.is_file() {
        return Err(format!("Store item images not found: {}", show(store_images)));
    }
    let rows = store_image_rows(read_json_lines(store_images)?);
    let mut baked = BTreeMap::new();
    let mut quest = HashSet::new();
    for row in rows {
        let Some(name) = row.get("rowName").and_then(Value::as_str).map(str::to_owned) else {
            continue;
        };
        if let Some(captures) = QUEST_ROW.as_ref().and_then(|re| re.captures(&name))
            && let Some(id) = captures.get(1)
        {
            quest.insert(id.as_str().to_uppercase());
        }
        baked.insert(name, row);
    }
    let mut sold = sold_ids(&data_dir.join("reward_cache_store.json"), "Reward Cache store")?;
    sold.extend(sold_ids(&data_dir.join("ladyluck_store.json"), "Lady Luck store data")?);
    let mut build = Build {
        inputs: Inputs {
            catalog,
            baked,
            sold,
            quest,
        },
        skus: Vec::new(),
        claimed_skus: HashSet::new(),
        claimed_items: HashSet::new(),
        skips: Skips::default(),
        captured: 0,
    };
    for file in ["awakening_webstore.json", "dyes.json"] {
        for sku in Build::read_capture(capture_dir, file)? {
            build.captured_offer(&sku);
        }
    }
    for sku in Build::read_capture(capture_dir, "hp_level_skip_rank.json")? {
        build.rank_skip(&sku);
    }
    for sku in Build::read_capture(capture_dir, "huntpass_store.json")? {
        build.pass(&sku);
    }
    let rows: Vec<String> = build.inputs.baked.keys().cloned().collect();
    let mut reconstructed = 0;
    for row in rows {
        reconstructed += usize::from(build.reconstructed(&row));
    }
    tracing::debug!(skips = ?build.skips.0, "platinum store skips");

    let mut keyed: Vec<(String, Value)> = build.skus.into_iter().map(|s| (sort_text(&s), s)).collect();
    keyed.sort_by(|a, b| dainty_service_kit::collate::locale_compare(&a.0, &b.0));
    let sorted: Vec<Value> = keyed.into_iter().map(|(_, s)| s).collect();
    create_dir(data_dir)?;
    let target = data_dir.join("platinum_store.json");
    let total = sorted.len();
    write_file(&target, &format!("{}\n", stringify_pretty(&Value::Array(sorted), 2)))?;
    Ok(format!(
        "platinum_store.json: {total} SKUs ({} captured price, {reconstructed} reconstructed price)",
        build.captured
    ))
}

/// `{base}_1_12.jsonl` when it exists or the 1.14.7 one does not (spec §12 item 6).
pub fn resolve_platinum_export_file(exports: &Path, base: &str) -> PathBuf {
    let old = exports.join(format!("{base}_1_12.jsonl"));
    let new = exports.join(format!("{base}_1_14_7.jsonl"));
    if old.exists() || !new.exists() { old } else { new }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generators::io::catalog_from_rows;

    #[test]
    fn categories_and_prices() {
        let c = |id: &str| category(id);
        assert_eq!(c("WP_GA_X_00").as_deref(), Some("weapon_ga"));
        assert_eq!(c("WP_Sw_X").as_deref(), None);
        assert_eq!(c("WP_ZZ_X"), None);
        assert_eq!(c("WP_GA"), None);
        assert_eq!(c("EM_INTRO_X").as_deref(), Some("arrival"));
        assert_eq!(c("EM_WAVE").as_deref(), Some("emote"));
        assert_eq!(c("BNC_SIGIL_A").as_deref(), Some("flaresigil"));
        assert_eq!(c("FL_A").as_deref(), Some("flaresigil"));
        assert_eq!(c("BNC_MESH_A"), None);
        assert_eq!(c("QI_TONIC").as_deref(), Some("tonics"));
        assert_eq!(c("TOKEN_X"), None);
        let p = reconstructed_price;
        assert_eq!(
            [
                p("weapon_ac", 3),
                p("weapon_ac", 4),
                p("weapon_ac", 5),
                p("weapon_ac", 1)
            ],
            [500, 800, 1200, 800]
        );
        assert_eq!(
            [p("armour", 3), p("armour", 4), p("armour", 5), p("armour", 0)],
            [400, 600, 800, 600]
        );
        assert_eq!([p("lantern", 3), p("lantern", 4), p("lantern", 5)], [600, 600, 800]);
        assert_eq!([p("dye", 5), p("slayer", 1), p("tonics", 1), p("boosts", 1)], [100; 4]);
        assert_eq!(
            [p("emote", 3), p("emote", 4), p("emote", 5), p("emote", 2)],
            [300, 500, 700, 500]
        );
        assert_eq!(
            [
                p("emoji", 1),
                p("arrival", 1),
                p("fabric", 1),
                p("standard", 1),
                p("flaresigil", 1),
                p("glider", 1)
            ],
            [200, 1000, 300, 400, 250, 800]
        );
        assert_eq!(p("other", 5), 100);
    }

    #[test]
    fn decoding() {
        let catalog = catalog_from_rows(vec![json!({"itemId": "AR_FOO"}), json!({"itemId": "BAR"})]);
        assert_eq!(decode("sku_foo", &catalog).as_deref(), Some("AR_FOO"));
        assert_eq!(decode("single_bar", &catalog).as_deref(), Some("BAR"));
        assert_eq!(decode("contract_baz", &catalog), None);
        assert_eq!(decode("bundle_bar", &catalog), None);
    }

    #[test]
    fn sort_keys() {
        let sku = json!({"id": "x", "tags": ["webstore", "skin_armour", "skin_weapon_cb"], "prices": [{"price": 800}]});
        assert_eq!(sort_text(&sku), "1|skin_weapon_cb|000800|x");
        let priced =
            |price: Value| sort_text(&json!({"id": "p", "tags": ["huntpass_store"], "prices": [{"price": price}]}));
        assert_eq!(priced(json!(12.5)), "3|zzz|0012.5|p");
        assert_eq!(priced(json!(-5)), "3|zzz|0000-5|p");
        assert_eq!(priced(json!("500")), "3|zzz|000500|p");
        assert_eq!(priced(json!(1_200_000)), "3|zzz|1200000|p");
        assert_eq!(priced(json!(500)), "3|zzz|000500|p");
        assert_eq!(
            sort_text(&json!({"id": "r", "tags": ["hp_level_skip_rank"], "prices": [{"price": 0}]})),
            "2|zzz|000000|r"
        );
    }
}
