// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The Slayer's Path graph and node definitions from the journey-node export (spec §5.2, §5.3).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use dainty_service_kit::js;
use serde_json::{Map, Value, json};

use super::io::{back_up, create_parent, drop_bom, parse, read_text, show, write_file};
use super::pretty::stringify_pretty;

/// Node ids the client gates unlocks on.
const GATES: [&str; 11] = [
    "Slayer_09",
    "Slayer_10",
    "Slayer_11",
    "WeaponTypeGA_Spc_00",
    "WeaponTypeEB_Spc_00",
    "WeaponTypeCB_Spc_00",
    "AirshipUpgrades_Islands_S",
    "AirshipUpgrades_Islands_T",
    "AirshipUpgrades_Islands_U",
    "Escalation.Hard.Unlock",
    "Lantern.Unlock",
];

fn missing_export(path: &Path) -> String {
    format!(
        "ERROR: export not found at {} — run CatalogExporter with EXPORT_SLAYERS_PATH=1 first",
        show(path)
    )
}

/// The export's rows; `trimmed` trims the whole text first, which shifts the line numbers.
fn read_export(path: &Path, trimmed: bool) -> Result<Vec<Value>, String> {
    if !path.is_file() {
        return Err(missing_export(path));
    }
    let text = read_text(path).map_err(|_| missing_export(path))?;
    let text = drop_bom(&text);
    let text = if trimmed { js::trim(text) } else { text };
    let mut rows = Vec::new();
    for (i, line) in text.split('\n').enumerate() {
        if js::trim(line).is_empty() {
            continue;
        }
        rows.push(parse(line).map_err(|_| format!("ERROR: malformed JSON on line {} of the export", i + 1))?);
    }
    Ok(rows)
}

fn node_id(row: &Value) -> Option<&str> {
    row.get("nodeId").and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// Replaces `target`'s `payload.nodes` with one locked node per exported journey node; without `apply` it is a dry run that only reports.
pub fn generate_slayers_path(export: &Path, target: &Path, apply: bool) -> Result<String, String> {
    let rows = read_export(export, true)?;
    if !target.is_file() {
        return Err(format!("ERROR: target not found at {}", show(target)));
    }
    let invalid = |e: String| format!("ERROR: invalid JSON in {}: {e}", show(target));
    let text = read_text(target).map_err(|e| invalid(e.to_string()))?;
    let mut document = parse(&text).map_err(invalid)?;
    let Some(existing) = document.pointer("/payload/nodes").and_then(Value::as_object) else {
        return Err("ERROR: existing slayers_path.json has no payload.nodes — refusing to guess at the shape".into());
    };
    let mut next = Map::new();
    for id in rows.iter().filter_map(node_id) {
        if !next.contains_key(id) {
            next.insert(
                id.to_owned(),
                json!({ "node_id": id, "node_status": 0, "objectives": [] }),
            );
        }
    }
    let added = next.keys().filter(|k| !existing.contains_key(*k)).count();
    let mut dropped: Vec<&String> = existing.keys().filter(|k| !next.contains_key(*k)).collect();
    dropped.sort();
    let present = |map: &Map<String, Value>, gate: &str| if map.contains_key(gate) { "present" } else { "MISSING" };

    let mut report = vec![
        format!("existing baseline : {} nodes", existing.len()),
        format!("live 1.12 table   : {} nodes", next.len()),
        format!("  added   : {added}"),
        format!("  dropped : {}  (1.4.4 leftovers / renamed)", dropped.len()),
        "critical unlock gates:".to_owned(),
    ];
    for gate in GATES {
        report.push(format!(
            "  {gate:<30} before={:<8} after={}",
            present(existing, gate),
            present(&next, gate)
        ));
    }
    if !dropped.is_empty() {
        report.push("Dropped nodes stay in players' saves as orphans the client reads as locked.".into());
        report.push("They no longer exist in the export, so nothing can unlock them:".into());
        for id in dropped.iter().take(10) {
            report.push(format!("    {id}"));
        }
        if dropped.len() > 10 {
            report.push(format!("  ...and {} more", dropped.len() - 10));
        }
    }
    if !apply {
        report.push("Dry run. Nothing written. Re-run with --apply to commit.".into());
        return Ok(report.join("\n"));
    }
    let count = next.len();
    if let Some(payload) = document.get_mut("payload").and_then(Value::as_object_mut) {
        payload.insert("nodes".into(), Value::Object(next));
    }
    let backup = back_up(target)?;
    write_file(target, &stringify_pretty(&document, 2))?;
    let name = backup
        .as_deref()
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    report.push(format!("Backed up: {name}"));
    report.push(format!("Wrote    : slayers_path.json ({count} nodes)"));
    report.push("Restart the metagame so it loads the new graph, then check in game that Trials,".into());
    report.push("Grim Onslaught crafting and Islands S, T and U unlock as they should.".into());
    Ok(report.join("\n"))
}

/// One node's definition, or none when its three lists are empty.
pub(crate) fn definition(row: &Value) -> Option<Value> {
    let list = |name: &str| row.get(name).and_then(Value::as_array).cloned().unwrap_or_default();
    let usable = |s: &str| !s.is_empty() && s != "None";
    let children: Vec<Value> = list("childNodes")
        .into_iter()
        .filter(|c| c.as_str().is_some_and(usable))
        .collect();
    let costs: Vec<Value> = list("currencyCosts")
        .into_iter()
        .filter_map(|c| {
            let cost = c.as_object()?;
            let currency = cost.get("currency")?.as_str().filter(|s| *s != "None")?;
            let mut out = Map::new();
            if let Some(amount) = cost.get("amount") {
                out.insert("amount".into(), amount.clone());
            }
            out.insert("currency".into(), Value::from(currency));
            Some(Value::Object(out))
        })
        .collect();
    let rewards: Vec<Value> = list("rewards")
        .into_iter()
        .filter_map(|r| r.get("itemId")?.as_str().filter(|s| usable(s)).map(Value::from))
        .collect();
    if children.is_empty() && costs.is_empty() && rewards.is_empty() {
        return None;
    }
    let mut out = Map::new();
    if row.get("autoUnlockIfParentUnlocked") == Some(&Value::Bool(true)) {
        out.insert("autoUnlockIfParentUnlocked".into(), Value::Bool(true));
    }
    for (name, items) in [("childNodes", children), ("currencyCosts", costs), ("rewards", rewards)] {
        if !items.is_empty() {
            out.insert(name.into(), Value::Array(items));
        }
    }
    Some(Value::Object(out))
}

/// Writes the journey node definitions to `target`, naming the export `source_version` in the report; without `apply` it is a dry run that only reports.
pub fn generate_slayers_path_definitions(
    export: &Path,
    target: &Path,
    source_version: &str,
    apply: bool,
) -> Result<String, String> {
    let rows = read_export(export, false)?;
    let mut sorted = BTreeMap::new();
    for row in &rows {
        if let Some(id) = node_id(row)
            && let Some(def) = definition(row)
        {
            sorted.insert(id.to_owned(), def);
        }
    }
    let next: Map<String, Value> = sorted.into_iter().collect();
    let text = stringify_pretty(&Value::Object(next.clone()), 1);

    let existing_text = read_text(target).ok().map(|t| t.replace("\r\n", "\n"));
    let existing = existing_text
        .as_deref()
        .and_then(|t| parse(t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    let added: Vec<&String> = next.keys().filter(|k| !existing.contains_key(*k)).collect();
    let removed: BTreeSet<&String> = existing.keys().filter(|k| !next.contains_key(*k)).collect();
    let compact = dainty_service_kit::jsjson::stringify;
    let changed: Vec<&String> = next
        .iter()
        .filter(|(k, v)| existing.get(*k).is_some_and(|old| compact(old) != compact(v)))
        .map(|(k, _)| k)
        .collect();
    let mut report = vec![
        format!("source  : slayers_path_{source_version} ({} nodes)", rows.len()),
        format!("existing: {} definitions; next: {}", existing.len(), next.len()),
    ];
    let mut list = |label: &str, ids: Vec<&String>| {
        report.push(format!("  {label} : {}", ids.len()));
        if !ids.is_empty() {
            let ids: Vec<&str> = ids.into_iter().map(String::as_str).collect();
            report.push(format!("  {}", ids.join(", ")));
        }
    };
    list("added  ", added);
    list("removed", removed.into_iter().collect());
    list("changed", changed);
    report.push(format!(
        "  identical bytes: {}",
        existing_text.as_deref() == Some(text.as_str())
    ));
    if !apply {
        report.push("Dry run. Nothing written. Re-run with --apply to write.".into());
        return Ok(report.join("\n"));
    }
    create_parent(target)?;
    write_file(target, &text)?;
    report.push("Wrote slayers_path_definitions.json. Restart the metagame to load it.".into());
    Ok(report.join("\n"))
}
