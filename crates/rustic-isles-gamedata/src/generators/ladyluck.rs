// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Lady Luck's store: from a host-supplied response (spec §5.5) or from the live rotation capture (§5.6).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

use serde_json::{Map, Value, json};
use service_kit::js;

use super::io::{
    Catalog, back_up, checked_texture, create_dir, drop_bom, parse, read_catalog, read_text, show, write_file,
};
use super::pretty::stringify_pretty;
use super::progression::read_source;

/// `value ?? default`
fn or(entry: &Value, name: &str, default: Value) -> Value {
    match entry.get(name) {
        None | Some(Value::Null) => default,
        Some(v) => v.clone(),
    }
}

/// The source's entries as SKUs, and the problems that refuse them.
pub(crate) fn shape_source(entries: &[Value]) -> (Vec<Value>, Vec<String>) {
    let mut problems = Vec::new();
    let mut out = Vec::with_capacity(entries.len());
    for (i, entry) in entries.iter().enumerate() {
        let id = entry.get("id");
        let label = js::string(id);
        if !id.is_some_and(Value::is_string) {
            problems.push(format!("entry {i}: missing string \"id\""));
        }
        if !entry.get("items").is_some_and(Value::is_array) {
            problems.push(format!("entry {i} ({label}): missing items[]"));
        }
        if !entry.get("prices").is_some_and(Value::is_array) {
            problems.push(format!("entry {i} ({label}): missing prices[]"));
        }
        let mut sku = Map::new();
        if let Some(id) = id {
            sku.insert("id".into(), id.clone());
        }
        sku.insert("displayName".into(), or(entry, "displayName", json!("")));
        sku.insert("displayDescription".into(), or(entry, "displayDescription", json!("")));
        sku.insert("displayPriority".into(), or(entry, "displayPriority", json!(0)));
        sku.insert("prices".into(), or(entry, "prices", json!([])));
        sku.insert("maxAllowed".into(), or(entry, "maxAllowed", json!(0)));
        sku.insert("tags".into(), or(entry, "tags", json!([])));
        sku.insert("items".into(), or(entry, "items", json!([])));
        sku.insert(
            "duplicateInstancedItems".into(),
            or(entry, "duplicateInstancedItems", json!([])),
        );
        out.push(Value::Object(sku));
    }
    (out, problems)
}

pub fn generate_ladyluck_store(raw_dir: &Path, out_dir: &Path, apply: bool) -> Result<String, String> {
    let source = read_source(&raw_dir.join("ladyluck_store.source.json"), "store")?;
    let Some(entries) = source.as_array() else {
        return Err("ERROR: source must be a JSON array of store entries.".into());
    };
    let (out, problems) = shape_source(entries);
    let mut report = vec![
        format!("store entries : {}", entries.len()),
        if apply {
            "mode          : APPLY".to_owned()
        } else {
            "mode          : DRY RUN (pass --apply to write)".to_owned()
        },
    ];
    if !problems.is_empty() {
        // The refusal first, since hostkit shows an error's first line; then what Node printed before it
        report.insert(
            0,
            "ERROR: refusing to proceed until the source entries are well-formed.".into(),
        );
        report.push(String::new());
        report.push(format!("VALIDATION PROBLEMS ({}):", problems.len()));
        for problem in problems.iter().take(15) {
            report.push(format!("  {problem}"));
        }
        if problems.len() > 15 {
            report.push(format!("  ...and {} more", problems.len() - 15));
        }
        return Err(report.join("\n"));
    }
    report.push(String::new());
    if !apply {
        report.push("Dry run complete. Nothing written.".into());
        return Ok(report.join("\n"));
    }
    create_dir(out_dir)?;
    let target = out_dir.join("ladyluck_store.json");
    back_up(&target)?;
    let count = out.len();
    write_file(&target, &stringify_pretty(&Value::Array(out), 2))?;
    report.push(format!("Wrote ladyluck_store.json ({count} entries)"));
    Ok(report.join("\n"))
}

fn items_of(sku: &Value) -> Vec<Value> {
    sku.get("items").and_then(Value::as_array).cloned().unwrap_or_default()
}

/// Same length, and pairwise the same `catalogId` and `quantity`.
fn same_items(previous: &Value, live: &[Value]) -> bool {
    let old = items_of(previous);
    old.len() == live.len()
        && old.iter().zip(live).all(|(a, b)| {
            js::strict_eq(a.get("catalogId"), b.get("catalogId")) && js::strict_eq(a.get("quantity"), b.get("quantity"))
        })
}

fn live_item(item: &Value, catalog: &Catalog) -> Value {
    let mut out = Map::new();
    if let Some(id) = item.get("catalogId") {
        out.insert("catalogId".into(), id.clone());
    }
    if let Some(quantity) = item.get("quantity") {
        out.insert("quantity".into(), quantity.clone());
    }
    let id = item.get("catalogId").and_then(Value::as_str).unwrap_or("");
    let stackable = catalog.get(id).is_some_and(|i| i.stackable());
    out.insert("instanced".into(), Value::Bool(!stackable));
    Value::Object(out)
}

pub fn enrich_ladyluck_store(catalog: &Path, live: &Path, store: &Path) -> Result<String, String> {
    let catalog = read_catalog(catalog)?;
    if !live.is_file() {
        return Err(format!("Live Lady Luck capture not found: {}", show(live)));
    }
    if !store.is_file() {
        return Err(format!("Lady Luck store data not found: {}", show(store)));
    }
    let invalid = |path: &Path, e: String| format!("invalid JSON in {}: {e}", show(path));
    let live_text = read_text(live).map_err(|e| invalid(live, e.to_string()))?;
    let offers = parse(drop_bom(&live_text)).map_err(|e| invalid(live, e))?;
    let offers = offers
        .as_array()
        .filter(|a| !a.is_empty())
        .ok_or("Live Lady Luck capture is not a SKU list")?;
    let store_text = read_text(store).map_err(|e| invalid(store, e.to_string()))?;
    let existing = parse(&store_text).map_err(|e| invalid(store, e))?;
    let existing = existing
        .as_array()
        .ok_or("Existing ladyluck_store.json is not a JSON array")?;
    let mut previous: HashMap<&str, &Value> = HashMap::new();
    for sku in existing {
        if let Some(id) = sku.get("id").and_then(Value::as_str) {
            previous.insert(id, sku);
        }
    }

    let mut skus = Vec::new();
    let mut skipped = Vec::new();
    let mut explicit_images = 0;
    for offer in offers.iter().filter(|o| o.is_object()) {
        let id = offer.get("id").cloned().unwrap_or(Value::Null);
        let label = js::string(offer.get("id"));
        let items: Vec<Value> = items_of(offer);
        let missing: Vec<&str> = items
            .iter()
            .filter_map(|i| i.get("catalogId").and_then(Value::as_str))
            .filter(|c| !catalog.contains_key(*c))
            .collect();
        if !missing.is_empty() {
            skipped.push(format!("{label} ({} not in the catalogue)", missing.join(", ")));
            continue;
        }
        let prev = id.as_str().and_then(|i| previous.get(i)).copied();
        let kept = prev.filter(|p| same_items(p, &items));
        let mut sku = Map::new();
        sku.insert("id".into(), id.clone());
        for name in ["displayName", "displayDescription", "displayPriority"] {
            if let Some(v) = offer.get(name) {
                sku.insert(name.into(), v.clone());
            }
        }
        let prices: Vec<Value> = offer
            .get("prices")
            .and_then(Value::as_array)
            .map(|ps| {
                ps.iter()
                    .map(|p| {
                        let field = |n: &str| p.get(n).cloned().unwrap_or(Value::Null);
                        json!({ "currencyId": field("currencyId"), "price": field("price") })
                    })
                    .collect()
            })
            .unwrap_or_default();
        sku.insert("prices".into(), Value::Array(prices));
        sku.insert(
            "maxAllowed".into(),
            offer.get("maxAllowed").cloned().unwrap_or(Value::Null),
        );
        let first = items.first().and_then(|i| i.get("catalogId")).and_then(Value::as_str);
        let image = first.and_then(|c| catalog.get(c)).and_then(|i| i.image());
        let previous_images = prev.and_then(|p| p.get("images")).filter(|v| !v.is_null());
        if let Some(path) = image {
            // Server-authored art, independent of how the client's store table names its rows
            let mut images = previous_images.and_then(Value::as_object).cloned().unwrap_or_default();
            images.insert("standard".into(), Value::from(checked_texture(path)?));
            sku.insert("images".into(), Value::Object(images));
            explicit_images += 1;
        } else if let Some(images) = previous_images {
            sku.insert("images".into(), images.clone());
        }
        let tags = offer.get("tags").filter(|t| t.is_array()).cloned().unwrap_or(json!([]));
        sku.insert("tags".into(), tags);
        match kept {
            Some(p) => {
                sku.insert("items".into(), p.get("items").cloned().unwrap_or(json!([])));
                let dups = p.get("duplicateInstancedItems").cloned().unwrap_or(json!([]));
                sku.insert("duplicateInstancedItems".into(), dups);
            }
            None => {
                let built: Vec<Value> = items.iter().map(|i| live_item(i, &catalog)).collect();
                sku.insert("items".into(), Value::Array(built));
                let dups = offer.get("duplicateInstancedItems").cloned().unwrap_or(json!([]));
                sku.insert("duplicateInstancedItems".into(), dups);
            }
        }
        skus.push(Value::Object(sku));
    }

    let mut ids = HashSet::new();
    let (mut steel, mut gilded) = (0, 0);
    for sku in &skus {
        let label = js::string(sku.get("id"));
        if !ids.insert(service_kit::jsjson::stringify(sku.get("id").unwrap_or(&Value::Null))) {
            return Err(format!("Duplicate Lady Luck SKU id: {label}"));
        }
        let prices = sku
            .get("prices")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        let [price] = prices else {
            return Err(format!("Expected exactly one price for {label}"));
        };
        match price.get("currencyId").and_then(Value::as_str) {
            Some("id_currency_marks_steel") => steel += 1,
            Some("id_currency_marks_gilded") => gilded += 1,
            _ => return Err(format!("Unsupported Lady Luck currency for {label}")),
        }
        let tagged = sku
            .get("tags")
            .and_then(Value::as_array)
            .is_some_and(|t| t.iter().any(|t| t.as_str() == Some("ladyluckstore")));
        if !tagged {
            return Err(format!("Missing ladyluckstore tag on {label}"));
        }
    }
    let kept_ids: HashSet<&str> = skus
        .iter()
        .filter_map(|s| s.get("id").and_then(Value::as_str))
        .collect();
    let retired: BTreeSet<&str> = previous.keys().copied().filter(|id| !kept_ids.contains(id)).collect();
    let count = skus.len();
    write_file(store, &format!("{}\n", stringify_pretty(&Value::Array(skus), 2)))?;
    let none_or = |list: Vec<String>, sep: &str| {
        if list.is_empty() {
            "none".to_owned()
        } else {
            list.join(sep)
        }
    };
    Ok([
        format!("Wrote {}", show(store)),
        format!(
            "SKUs: {count} of {} live; explicit catalog images: {explicit_images}",
            offers.len()
        ),
        format!("Steel offers: {steel}; Gilded offers: {gilded}"),
        format!("Skipped live SKUs: {}", none_or(skipped, "; ")),
        format!(
            "Retired SKUs: {}",
            none_or(retired.into_iter().map(str::to_owned).collect(), ", ")
        ),
    ]
    .join("\n"))
}
