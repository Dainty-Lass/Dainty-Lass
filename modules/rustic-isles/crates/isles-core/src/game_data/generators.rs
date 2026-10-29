//! RB11 Rust ports of the `ParadoxBackend/scripts` game-data generators:
//! - [`generate_progression_config`] (`generate_progression_config.cjs`)
//! - [`generate_slayers_path`] (`generate_slayers_path.cjs`)
//! - [`generate_slayers_path_definitions`] (`generate_slayers_path_definitions.cjs`)
//! - [`generate_reward_cache_store`] (`generate_reward_cache_store.ts`)
//! - [`generate_ladyluck_store`] (`generate_ladyluck_store.cjs`)
//! - [`enrich_ladyluck_store`] (`enrich_ladyluck_store.ts`)
//! - [`generate_platinum_store`] (`generate_platinum_store.ts`)
//! - [`generate_inventory_storage_policy`] (`generate_inventory_storage_policy.ts`)

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::Utc;
use regex::Regex;
use serde_json::{Map, Value, json};
use service_kit::collate::locale_compare;
use service_kit::jsjson;
use sha2::{Digest, Sha256};

const EMBEDDED_SRC_TOKENS: &str = include_str!("src_tokens.txt");

/// `JSON.stringify(value, null, indent)` matching V8's number formatting, string escaping and key
/// ordering.
pub fn stringify_pretty(value: &Value, indent: usize) -> String {
    let mut out = String::new();
    write_pretty(&mut out, value, indent, 0);
    out
}

fn array_index(key: &str) -> Option<u32> {
    let n: u32 = key.parse().ok()?;
    (n != u32::MAX && n.to_string() == key).then_some(n)
}

fn js_order(map: &Map<String, Value>) -> Vec<(&String, &Value)> {
    let mut indexed: Vec<(u32, &String, &Value)> = map
        .iter()
        .filter_map(|(k, v)| array_index(k).map(|n| (n, k, v)))
        .collect();
    indexed.sort_by_key(|(n, _, _)| *n);
    let mut out: Vec<(&String, &Value)> = Vec::with_capacity(map.len());
    for (_, k, v) in indexed {
        out.push((k, v));
    }
    for (k, v) in map {
        if array_index(k).is_none() {
            out.push((k, v));
        }
    }
    out
}

fn write_string(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn push_indent(out: &mut String, spaces: usize) {
    for _ in 0..spaces {
        out.push(' ');
    }
}

fn write_pretty(out: &mut String, value: &Value, indent: usize, depth: usize) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&jsjson::number(n.as_f64().unwrap_or(f64::NAN))),
        Value::String(s) => write_string(out, s),
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            if indent == 0 {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_pretty(out, item, 0, depth + 1);
                }
                out.push(']');
                return;
            }
            out.push_str("[\n");
            let child_spaces = (depth + 1) * indent;
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(",\n");
                }
                push_indent(out, child_spaces);
                write_pretty(out, item, indent, depth + 1);
            }
            out.push('\n');
            push_indent(out, depth * indent);
            out.push(']');
        }
        Value::Object(map) => {
            if map.is_empty() {
                out.push_str("{}");
                return;
            }
            let ordered = js_order(map);
            if indent == 0 {
                out.push('{');
                for (i, (k, v)) in ordered.into_iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_string(out, k);
                    out.push(':');
                    write_pretty(out, v, 0, depth + 1);
                }
                out.push('}');
                return;
            }
            out.push_str("{\n");
            let child_spaces = (depth + 1) * indent;
            for (i, (k, v)) in ordered.into_iter().enumerate() {
                if i > 0 {
                    out.push_str(",\n");
                }
                push_indent(out, child_spaces);
                write_string(out, k);
                out.push_str(": ");
                write_pretty(out, v, indent, depth + 1);
            }
            out.push('\n');
            push_indent(out, depth * indent);
            out.push('}');
        }
    }
}

fn backup_stamp() -> String {
    Utc::now()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        .replace([':', '.'], "-")
}

fn backup_if_exists(target_path: &Path) -> Result<Option<String>, String> {
    if !target_path.exists() {
        return Ok(None);
    }
    let stamp = backup_stamp();
    let file_name = target_path.file_name().and_then(|s| s.to_str()).unwrap_or("file.json");
    let bak_name = format!("{file_name}.{stamp}.bak");
    let bak_path = target_path.with_file_name(&bak_name);
    std::fs::copy(target_path, &bak_path).map_err(|e| {
        format!(
            "failed to back up {} to {}: {e}",
            target_path.display(),
            bak_path.display()
        )
    })?;
    Ok(Some(bak_name))
}

// ---------------------------------------------------------------------------
// 1. generate_progression_config.cjs
// ---------------------------------------------------------------------------

pub fn generate_progression_config(raw_dir: &Path, out_dir: &Path, apply: bool) -> Result<String, String> {
    let source_path = raw_dir.join("progression_config.source.json");
    let target_path = out_dir.join("progression_config.json");

    if !source_path.exists() {
        return Err(format!(
            "ERROR: source not found at {}\nProvide the progression payload from your own installation (see docs/GENERATING_GAME_DATA.md).",
            source_path.display()
        ));
    }

    let raw_text =
        std::fs::read_to_string(&source_path).map_err(|e| format!("ERROR: source is not valid JSON: {e}"))?;
    let stripped = raw_text.strip_prefix('\u{feff}').unwrap_or(&raw_text);
    let source = jsjson::parse(stripped).map_err(|e| format!("ERROR: source is not valid JSON: SyntaxError: {e}"))?;

    let paths = source
        .get("payload")
        .and_then(|p| p.get("paths"))
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| {
            "ERROR: source has no payload.paths[] array — refusing to write a shape the backend cannot read.".to_owned()
        })?;

    let code = source
        .get("code")
        .filter(|v| !v.is_null())
        .cloned()
        .unwrap_or(Value::Null);
    let message = source.get("message").and_then(Value::as_str).unwrap_or("OK").to_owned();
    let mut payload = source
        .get("payload")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    payload.insert("paths".into(), Value::Array(paths.clone()));

    let mut output_map = Map::new();
    output_map.insert("code".into(), code);
    output_map.insert("message".into(), Value::String(message));
    output_map.insert("payload".into(), Value::Object(payload));
    let output = Value::Object(output_map);

    let total_rewards: usize = paths
        .iter()
        .map(|p| p.get("free_rewards").and_then(Value::as_array).map_or(0, Vec::len))
        .sum();

    let mut lines = vec![
        format!("source paths      : {}", paths.len()),
        format!("free_rewards tiers : {total_rewards}"),
        format!(
            "mode              : {}",
            if apply {
                "APPLY"
            } else {
                "DRY RUN (pass --apply to write)"
            }
        ),
    ];

    if !apply {
        lines.push("\nDry run complete. Nothing written.".into());
        return Ok(lines.join("\n"));
    }

    std::fs::create_dir_all(out_dir).map_err(|e| format!("ERROR: failed to create {}: {e}", out_dir.display()))?;
    let _ = backup_if_exists(&target_path)?;
    let json_text = stringify_pretty(&output, 2);
    std::fs::write(&target_path, json_text.as_bytes())
        .map_err(|e| format!("ERROR: failed to write {}: {e}", target_path.display()))?;
    lines.push(format!("\nWrote progression_config.json ({} paths)", paths.len()));
    Ok(lines.join("\n"))
}

// ---------------------------------------------------------------------------
// 2. generate_slayers_path.cjs
// ---------------------------------------------------------------------------

pub fn generate_slayers_path(export_path: &Path, target_path: &Path, apply: bool) -> Result<String, String> {
    if !export_path.exists() {
        return Err(format!(
            "ERROR: export not found at {} — run CatalogExporter with EXPORT_SLAYERS_PATH=1 first",
            export_path.display()
        ));
    }
    if !target_path.exists() {
        return Err(format!("ERROR: target not found at {}", target_path.display()));
    }

    let raw_export = std::fs::read_to_string(export_path)
        .map_err(|e| format!("ERROR: failed to read {}: {e}", export_path.display()))?;
    let mut live_rows = Vec::new();
    for (idx, line) in raw_export
        .strip_prefix('\u{feff}')
        .unwrap_or(&raw_export)
        .trim()
        .split('\n')
        .enumerate()
    {
        if line.trim().is_empty() {
            continue;
        }
        let parsed =
            jsjson::parse(line).map_err(|_| format!("ERROR: malformed JSON on line {} of the export", idx + 1))?;
        live_rows.push(parsed);
    }

    let raw_target = std::fs::read_to_string(target_path)
        .map_err(|e| format!("ERROR: failed to read {}: {e}", target_path.display()))?;
    let existing = jsjson::parse(raw_target.strip_prefix('\u{feff}').unwrap_or(&raw_target))
        .map_err(|e| format!("ERROR: invalid JSON in {}: {e}", target_path.display()))?;
    let existing_nodes = existing
        .get("payload")
        .and_then(|p| p.get("nodes"))
        .and_then(Value::as_object)
        .ok_or_else(|| {
            "ERROR: existing slayers_path.json has no payload.nodes — refusing to guess at the shape".to_owned()
        })?;

    let mut next_nodes = Map::new();
    for row in &live_rows {
        let Some(node_id) = row.get("nodeId").and_then(Value::as_str).filter(|s| !s.is_empty()) else {
            continue;
        };
        next_nodes.insert(
            node_id.to_owned(),
            json!({
                "node_id": node_id,
                "node_status": 0,
                "objectives": []
            }),
        );
    }

    let existing_ids: HashSet<&String> = existing_nodes.keys().collect();
    let next_ids: HashSet<&String> = next_nodes.keys().collect();

    let mut added: Vec<&str> = next_ids.difference(&existing_ids).map(|s| s.as_str()).collect();
    added.sort_unstable();
    let mut dropped: Vec<&str> = existing_ids.difference(&next_ids).map(|s| s.as_str()).collect();
    dropped.sort_unstable();

    let mut lines = vec![
        format!("existing baseline : {} nodes", existing_ids.len()),
        format!("live 1.12 table   : {} nodes", next_ids.len()),
        format!("  added   : {}", added.len()),
        format!("  dropped : {}  (1.4.4 leftovers / renamed)", dropped.len()),
        "\ncritical unlock gates:".into(),
    ];

    const CRITICAL_GATES: &[&str] = &[
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
    for gate in CRITICAL_GATES {
        let before = if existing_nodes.contains_key(*gate) {
            "present"
        } else {
            "MISSING"
        };
        let after = if next_nodes.contains_key(*gate) {
            "present"
        } else {
            "MISSING"
        };
        lines.push(format!("  {gate:<30} before={before:<8} after={after}"));
    }

    if !dropped.is_empty() {
        lines.push("\nsample dropped ids (stored player unlocks keyed on these become orphans —".into());
        lines.push(
            "SavePlayerJourney merges stored over baseline, so they persist harmlessly but read as locked):".into(),
        );
        for id in dropped.iter().take(10) {
            lines.push(format!("  {id}"));
        }
        if dropped.len() > 10 {
            lines.push(format!("  ...and {} more", dropped.len() - 10));
        }
    }

    if !apply {
        lines.push("\nDry run. Nothing written. Re-run with --apply to commit.".into());
        return Ok(lines.join("\n"));
    }

    let next_count = next_nodes.len();
    let mut output = existing.clone();
    if let Some(payload) = output.get_mut("payload").and_then(Value::as_object_mut) {
        payload.insert("nodes".into(), Value::Object(next_nodes));
    }
    let bak_name = backup_if_exists(target_path)?.unwrap_or_default();
    let json_text = stringify_pretty(&output, 2);
    std::fs::write(target_path, json_text.as_bytes())
        .map_err(|e| format!("ERROR: failed to write {}: {e}", target_path.display()))?;

    lines.push(format!("\nBacked up: {bak_name}"));
    lines.push(format!("Wrote    : slayers_path.json  ({next_count} nodes)"));
    lines.push("\nRestart the metagame, then verify in game that Trials, Grim Onslaught crafting and".into());
    lines.push("Island S/T/U activities unlock. Existing characters may need to re-unlock renamed nodes.".into());
    Ok(lines.join("\n"))
}

// ---------------------------------------------------------------------------
// 3. generate_slayers_path_definitions.cjs
// ---------------------------------------------------------------------------

pub fn generate_slayers_path_definitions(
    export_path: &Path,
    target_path: &Path,
    source_version: &str,
    apply: bool,
) -> Result<String, String> {
    if !export_path.exists() {
        return Err(format!(
            "ERROR: export not found at {} — run CatalogExporter with EXPORT_SLAYERS_PATH=1 first",
            export_path.display()
        ));
    }

    let raw_export = std::fs::read_to_string(export_path)
        .map_err(|e| format!("ERROR: failed to read {}: {e}", export_path.display()))?;
    let mut rows = Vec::new();
    for (idx, line) in raw_export
        .strip_prefix('\u{feff}')
        .unwrap_or(&raw_export)
        .split('\n')
        .enumerate()
    {
        if line.trim().is_empty() {
            continue;
        }
        let parsed =
            jsjson::parse(line).map_err(|_| format!("ERROR: malformed JSON on line {} of the export", idx + 1))?;
        rows.push(parsed);
    }

    let mut next: BTreeMap<String, Value> = BTreeMap::new();
    for row in &rows {
        let Some(node_id) = row.get("nodeId").and_then(Value::as_str).filter(|s| !s.is_empty()) else {
            continue;
        };
        let child_nodes: Vec<Value> = row
            .get("childNodes")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .filter(|s| !s.is_empty() && *s != "None")
                    .map(|s| Value::String(s.to_owned()))
                    .collect()
            })
            .unwrap_or_default();
        let currency_costs: Vec<Value> = row
            .get("currencyCosts")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_object)
                    .filter_map(|cost| {
                        let currency = cost.get("currency")?.as_str()?;
                        if currency == "None" {
                            return None;
                        }
                        let mut m = Map::new();
                        if let Some(amount) = cost.get("amount") {
                            m.insert("amount".into(), amount.clone());
                        }
                        m.insert("currency".into(), Value::String(currency.to_owned()));
                        Some(Value::Object(m))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let rewards: Vec<Value> = row
            .get("rewards")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|r| r.get("itemId").and_then(Value::as_str))
                    .filter(|s| !s.is_empty() && *s != "None")
                    .map(|s| Value::String(s.to_owned()))
                    .collect()
            })
            .unwrap_or_default();

        if child_nodes.is_empty() && currency_costs.is_empty() && rewards.is_empty() {
            continue;
        }

        let mut def = Map::new();
        if row.get("autoUnlockIfParentUnlocked") == Some(&Value::Bool(true)) {
            def.insert("autoUnlockIfParentUnlocked".into(), Value::Bool(true));
        }
        if !child_nodes.is_empty() {
            def.insert("childNodes".into(), Value::Array(child_nodes));
        }
        if !currency_costs.is_empty() {
            def.insert("currencyCosts".into(), Value::Array(currency_costs));
        }
        if !rewards.is_empty() {
            def.insert("rewards".into(), Value::Array(rewards));
        }
        next.insert(node_id.to_owned(), Value::Object(def));
    }

    let mut sorted_map = Map::new();
    for (k, v) in &next {
        sorted_map.insert(k.clone(), v.clone());
    }
    let next_json = stringify_pretty(&Value::Object(sorted_map), 1);

    let (existing_map, existing_json) = if target_path.exists() {
        let text = std::fs::read_to_string(target_path)
            .map_err(|e| format!("ERROR: failed to read {}: {e}", target_path.display()))?;
        let parsed = jsjson::parse(text.strip_prefix('\u{feff}').unwrap_or(&text))
            .ok()
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        (parsed, text.replace("\r\n", "\n"))
    } else {
        (Map::new(), String::new())
    };

    let added: Vec<&str> = next
        .keys()
        .filter(|id| !existing_map.contains_key(*id))
        .map(String::as_str)
        .collect();
    let mut removed: Vec<&str> = existing_map
        .keys()
        .filter(|id| !next.contains_key(*id))
        .map(String::as_str)
        .collect();
    removed.sort_unstable();
    let changed: Vec<&str> = next
        .iter()
        .filter(|(id, val)| {
            existing_map
                .get(*id)
                .is_some_and(|ex| jsjson::stringify(ex) != jsjson::stringify(val))
        })
        .map(|(id, _)| id.as_str())
        .collect();

    let mut lines = vec![
        format!("source  : slayers_path_{source_version} ({} nodes)", rows.len()),
        format!("existing: {} definitions; next: {}", existing_map.len(), next.len()),
        format!(
            "  added   : {}{}",
            added.len(),
            if added.is_empty() {
                String::new()
            } else {
                format!("  {}", added.join(", "))
            }
        ),
        format!(
            "  removed : {}{}",
            removed.len(),
            if removed.is_empty() {
                String::new()
            } else {
                format!("  {}", removed.join(", "))
            }
        ),
        format!(
            "  changed : {}{}",
            changed.len(),
            if changed.is_empty() {
                String::new()
            } else {
                format!("  {}", changed.join(", "))
            }
        ),
        format!("  identical bytes: {}", next_json == existing_json),
    ];

    if !apply {
        lines.push("\nDry run. Nothing written. Re-run with --apply to write.".into());
        return Ok(lines.join("\n"));
    }

    if let Some(parent) = target_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("ERROR: failed to create {}: {e}", parent.display()))?;
    }
    std::fs::write(target_path, next_json.as_bytes())
        .map_err(|e| format!("ERROR: failed to write {}: {e}", target_path.display()))?;
    lines.push("\nWrote slayers_path_definitions.json. Restart the metagame to load it.".into());
    Ok(lines.join("\n"))
}

// ---------------------------------------------------------------------------
// 4. generate_reward_cache_store.ts
// ---------------------------------------------------------------------------

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

    fn tag(self) -> &'static str {
        match self {
            Self::Armour => "tab_season_armour",
            Self::Weapon => "tab_season_weapon",
            Self::Style => "tab_season_style",
            Self::Event => "season_store_event",
            Self::Limited => "tab_season_limited",
            Self::General => "season_store_general",
        }
    }

    fn order(self) -> usize {
        match self {
            Self::Armour => 0,
            Self::Weapon => 1,
            Self::Style => 2,
            Self::Event => 3,
            Self::Limited => 4,
            Self::General => 5,
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

#[derive(Default)]
struct ParsedItemData {
    rarity: i64,
    icon: String,
    large_icon: String,
    custom_icon: String,
}

fn parse_item_data(row: Option<&Value>) -> ParsedItemData {
    let Some(custom_str) = row.and_then(|r| r.get("customData")).and_then(Value::as_str) else {
        return ParsedItemData::default();
    };
    let Ok(custom) = jsjson::parse(custom_str) else {
        return ParsedItemData::default();
    };
    let Some(item_data_str) = custom.get("ItemData").and_then(Value::as_str) else {
        return ParsedItemData::default();
    };
    let Ok(item_data) = jsjson::parse(item_data_str) else {
        return ParsedItemData::default();
    };
    let rarity = item_data
        .get("RarityLevel")
        .and_then(Value::as_f64)
        .filter(|f| f.is_finite())
        .map_or(0, |f| f as i64);
    let get_str = |key: &str| item_data.get(key).and_then(Value::as_str).unwrap_or("").to_owned();
    ParsedItemData {
        rarity,
        icon: get_str("Icon"),
        large_icon: get_str("LargeIcon"),
        custom_icon: get_str("CustomIcon"),
    }
}

fn is_valid_cooked_texture_path(asset_path: &str) -> bool {
    let Some(rest) = asset_path.strip_prefix("/Game/") else {
        return false;
    };
    !rest.is_empty()
        && rest
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'/' | b'-'))
}

fn texture_reference(asset_path: &str) -> Result<String, String> {
    if !is_valid_cooked_texture_path(asset_path) {
        return Err(format!("Invalid cooked texture path: {asset_path}"));
    }
    Ok(format!("Engine.Texture2D'{asset_path}'"))
}

fn slug(value: &str) -> String {
    let mut out = String::new();
    let mut prev_underscore = false;
    for c in value.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_underscore = false;
        } else if !prev_underscore && !out.is_empty() {
            out.push('_');
            prev_underscore = true;
        }
    }
    if out.ends_with('_') {
        out.pop();
    }
    out
}

fn prefix_rank(value: &str, prefixes: &[&str]) -> usize {
    prefixes
        .iter()
        .position(|p| value.contains(p))
        .unwrap_or(prefixes.len())
}

fn armour_slot_rank(row: Option<&Value>) -> usize {
    let Some(tags) = row.and_then(|r| r.get("tags")).and_then(Value::as_array) else {
        return 4;
    };
    let has = |name: &str| tags.iter().any(|t| t.as_str() == Some(name));
    if has("Helm") {
        0
    } else if has("Chest") {
        1
    } else if has("Arms") {
        2
    } else if has("Legs") {
        3
    } else {
        4
    }
}

fn banner_component_rank(catalog_id: &str) -> usize {
    if catalog_id.starts_with("BNC_STANDARD_") || catalog_id.starts_with("BNC_MESH_") {
        0
    } else if catalog_id.starts_with("BNC_FABRIC_") {
        1
    } else if catalog_id.starts_with("BNC_SIGIL_") {
        2
    } else if catalog_id.starts_with("BNC_ANIMATION_") {
        3
    } else {
        4
    }
}

fn load_catalog_map(catalog_path: &Path) -> Result<HashMap<String, Value>, String> {
    if !catalog_path.exists() {
        return Err(format!("Catalog dump not found: {}", catalog_path.display()));
    }
    let text =
        std::fs::read_to_string(catalog_path).map_err(|e| format!("failed to read {}: {e}", catalog_path.display()))?;
    let mut catalog = HashMap::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let row = jsjson::parse(line).map_err(|e| format!("invalid JSON in {}: {e}", catalog_path.display()))?;
        if let Some(item_id) = row.get("itemId").and_then(Value::as_str).filter(|s| !s.is_empty()) {
            catalog.insert(item_id.to_owned(), row);
        }
    }
    Ok(catalog)
}

pub fn reward_cache_selection_ids() -> Vec<&'static str> {
    REWARD_CACHE_SELECTIONS.iter().map(|s| s.catalog_id).collect()
}

pub fn generate_reward_cache_store(catalog_path: &Path, output_path: &Path) -> Result<String, String> {
    let catalog = load_catalog_map(catalog_path)?;
    let original_order: HashMap<&str, usize> = REWARD_CACHE_SELECTIONS
        .iter()
        .enumerate()
        .map(|(i, s)| (s.catalog_id, i))
        .collect();

    let mut ordered = REWARD_CACHE_SELECTIONS.to_vec();
    ordered.sort_by(|left, right| {
        if left.featured != right.featured {
            return if left.featured {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            };
        }
        let cat_cmp = left.category.order().cmp(&right.category.order());
        if cat_cmp != std::cmp::Ordering::Equal {
            return cat_cmp;
        }
        if left.category == Armour {
            let fam_cmp = prefix_rank(left.catalog_id, ARMOUR_FAMILY_ORDER)
                .cmp(&prefix_rank(right.catalog_id, ARMOUR_FAMILY_ORDER));
            if fam_cmp != std::cmp::Ordering::Equal {
                return fam_cmp;
            }
            let var_l = usize::from(!left.catalog_id.ends_with("_ULTRA"));
            let var_r = usize::from(!right.catalog_id.ends_with("_ULTRA"));
            if var_l != var_r {
                return var_l.cmp(&var_r);
            }
            let slot_cmp =
                armour_slot_rank(catalog.get(left.catalog_id)).cmp(&armour_slot_rank(catalog.get(right.catalog_id)));
            if slot_cmp != std::cmp::Ordering::Equal {
                return slot_cmp;
            }
        }
        if left.category == Weapon {
            let axe_l = usize::from(!left.catalog_id.starts_with("WP_GA_"));
            let axe_r = usize::from(!right.catalog_id.starts_with("WP_GA_"));
            if axe_l != axe_r {
                return axe_l.cmp(&axe_r);
            }
            if left.catalog_id.starts_with("WP_GA_") && right.catalog_id.starts_with("WP_GA_") {
                let rar_l = parse_item_data(catalog.get(left.catalog_id)).rarity;
                let rar_r = parse_item_data(catalog.get(right.catalog_id)).rarity;
                if rar_r != rar_l {
                    return rar_r.cmp(&rar_l);
                }
            }
        }
        if left.category == Event {
            let ban_l = left.catalog_id.starts_with("BNC_");
            let ban_r = right.catalog_id.starts_with("BNC_");
            if ban_l != ban_r {
                return if ban_l {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                };
            }
            if ban_l && ban_r {
                let fam_cmp = prefix_rank(left.catalog_id, BANNER_FAMILY_ORDER)
                    .cmp(&prefix_rank(right.catalog_id, BANNER_FAMILY_ORDER));
                if fam_cmp != std::cmp::Ordering::Equal {
                    return fam_cmp;
                }
                let comp_cmp = banner_component_rank(left.catalog_id).cmp(&banner_component_rank(right.catalog_id));
                if comp_cmp != std::cmp::Ordering::Equal {
                    return comp_cmp;
                }
            }
        }
        original_order
            .get(left.catalog_id)
            .copied()
            .unwrap_or(usize::MAX)
            .cmp(&original_order.get(right.catalog_id).copied().unwrap_or(usize::MAX))
    });

    const PRICE_TIERS: &[i64] = &[700, 800, 1000, 2000, 2500, 3000];
    const FORBIDDEN_TAGS: &[&str] = &["deprecated", "hidden", "initial", "masteryreward", "trialstore", "wip"];
    let mut seen_catalog_ids = HashSet::new();
    let mut seen_sku_ids = HashSet::new();
    let mut category_counts: HashMap<RewardCacheCategory, usize> = HashMap::new();
    let mut featured_count = 0usize;
    let mut store = Vec::with_capacity(ordered.len());

    for (index, selection) in ordered.iter().enumerate() {
        if !seen_catalog_ids.insert(selection.catalog_id) {
            return Err(format!("Duplicate catalog selection: {}", selection.catalog_id));
        }
        if !PRICE_TIERS.contains(&selection.price) {
            return Err(format!(
                "Unsupported price {} for {}",
                selection.price, selection.catalog_id
            ));
        }
        let upper_id = selection.catalog_id.to_ascii_uppercase();
        if ["FOUNDER", "TRANSLATOR", "TRIALS", "MASTERY"]
            .iter()
            .any(|p| upper_id.contains(p))
        {
            return Err(format!(
                "Exclusive/progression catalog id is not eligible: {}",
                selection.catalog_id
            ));
        }
        if selection.catalog_id == "WP_GA_SPOOKY_01" {
            return Err(format!(
                "Quest/Rumour-only catalog item is not eligible: {}",
                selection.catalog_id
            ));
        }
        let row = catalog
            .get(selection.catalog_id)
            .ok_or_else(|| format!("Catalog item not found: {}", selection.catalog_id))?;

        let catalog_name = row
            .get("displayName")
            .and_then(Value::as_str)
            .map(service_kit::js::trim)
            .unwrap_or("");
        let name = selection
            .display_name_override
            .map(service_kit::js::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(catalog_name);
        if name.is_empty() {
            return Err(format!("Missing display name: {}", selection.catalog_id));
        }
        let lower_name = name.to_ascii_lowercase();
        if selection.display_name_override.is_none()
            && (lower_name.starts_with("~dnt~")
                || lower_name.starts_with("<missing")
                || lower_name == "tbd"
                || lower_name == "empty slot")
        {
            return Err(format!("Unresolved display name for {}: {name}", selection.catalog_id));
        }

        let lower_tags: HashSet<String> = row
            .get("tags")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(Value::as_str).map(str::to_lowercase).collect())
            .unwrap_or_default();
        for forbidden in FORBIDDEN_TAGS {
            if lower_tags.contains(*forbidden) {
                return Err(format!(
                    "Forbidden catalog tag '{forbidden}' on {}",
                    selection.catalog_id
                ));
            }
        }

        let item_data = parse_item_data(Some(row));
        let rarity = item_data.rarity;
        let is_resolved_glider = lower_tags.contains("glider") && rarity >= 3;
        let is_explicit_r3 = selection.allow_rarity3 && rarity == 3;
        if !(rarity >= 4 || is_resolved_glider || is_explicit_r3) {
            return Err(format!(
                "Selected item must be rarity 4+, a resolved rarity-3 glider, or an explicit rarity-3 premium selection: {} has rarity {rarity}",
                selection.catalog_id
            ));
        }

        let standard_image_path = [&item_data.custom_icon, &item_data.large_icon, &item_data.icon]
            .into_iter()
            .find(|s| !s.is_empty())
            .ok_or_else(|| format!("Missing catalog image path: {}", selection.catalog_id))?;

        *category_counts.entry(selection.category).or_insert(0) += 1;
        if selection.featured {
            featured_count += 1;
        }

        let sku_id = format!(
            "season19_rewardcache_{}_{}",
            selection.category.as_str(),
            slug(selection.catalog_id)
        );
        if !seen_sku_ids.insert(sku_id.clone()) {
            return Err(format!("Duplicate generated SKU id: {sku_id}"));
        }

        let mut store_tags = vec![
            Value::String("season_store".into()),
            Value::String(selection.category.tag().into()),
        ];
        if selection.featured {
            store_tags.push(Value::String("season_store_feature".into()));
        }
        store_tags.push(Value::String("season19".into()));
        store_tags.push(Value::String("reward_cache_reconstructed".into()));
        store_tags.push(Value::String(format!("reward_cache_rarity_{rarity}")));

        let default_desc = format!("A premium Reward Cache cosmetic: {name}.");
        let desc = selection
            .description_override
            .map(service_kit::js::trim)
            .filter(|s| !s.is_empty())
            .or_else(|| {
                row.get("description")
                    .and_then(Value::as_str)
                    .map(service_kit::js::trim)
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or(&default_desc);
        let instanced = row.get("isStackable") != Some(&Value::Bool(true));

        store.push(json!({
            "id": sku_id,
            "displayName": name,
            "displayDescription": desc,
            "displayPriority": index + 1,
            "prices": [{ "currencyId": "id_currency_seasonal_coin", "price": selection.price }],
            "maxAllowed": 1,
            "images": { "standard": texture_reference(standard_image_path)? },
            "tags": store_tags,
            "items": [{ "catalogId": selection.catalog_id, "quantity": 1, "instanced": instanced }],
            "duplicateInstancedItems": [selection.catalog_id]
        }));
    }

    for cat in [Armour, Weapon, Style, Event, Limited, General] {
        let count = category_counts.get(&cat).copied().unwrap_or(0);
        if count != cat.expected_count() {
            return Err(format!(
                "{} count {count} != expected {}",
                cat.as_str(),
                cat.expected_count()
            ));
        }
    }
    if featured_count != 7 {
        return Err(format!("Featured count {featured_count} != expected 7"));
    }

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    }
    let json_text = format!("{}\n", stringify_pretty(&Value::Array(store.clone()), 2));
    std::fs::write(output_path, json_text.as_bytes())
        .map_err(|e| format!("failed to write {}: {e}", output_path.display()))?;

    let mut price_counts: BTreeMap<i64, usize> = BTreeMap::new();
    for entry in &store {
        if let Some(p) = entry
            .get("prices")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(|p| p.get("price"))
            .and_then(Value::as_i64)
        {
            *price_counts.entry(p).or_insert(0) += 1;
        }
    }

    let cats_str = [Armour, Weapon, Style, Event, Limited, General]
        .iter()
        .map(|c| format!("{}={}", c.as_str(), category_counts.get(c).copied().unwrap_or(0)))
        .collect::<Vec<_>>()
        .join(", ");
    let prices_str = price_counts
        .iter()
        .map(|(p, c)| format!("{p}={c}"))
        .collect::<Vec<_>>()
        .join(", ");

    Ok([
        format!("Wrote {}", output_path.display()),
        format!("SKUs: {}; featured: {featured_count}", store.len()),
        format!("Categories: {cats_str}"),
        format!("Prices: {prices_str}"),
    ]
    .join("\n"))
}

// ---------------------------------------------------------------------------
// 5. generate_ladyluck_store.cjs
// ---------------------------------------------------------------------------

pub fn generate_ladyluck_store(raw_dir: &Path, out_dir: &Path, apply: bool) -> Result<String, String> {
    let source_path = raw_dir.join("ladyluck_store.source.json");
    let target_path = out_dir.join("ladyluck_store.json");

    if !source_path.exists() {
        return Err(format!(
            "ERROR: source not found at {}\nProvide the store payload from your own installation (see docs/GENERATING_GAME_DATA.md).",
            source_path.display()
        ));
    }

    let raw_text =
        std::fs::read_to_string(&source_path).map_err(|e| format!("ERROR: source is not valid JSON: {e}"))?;
    let stripped = raw_text.strip_prefix('\u{feff}').unwrap_or(&raw_text);
    let source = jsjson::parse(stripped).map_err(|e| format!("ERROR: source is not valid JSON: SyntaxError: {e}"))?;
    let entries = source
        .as_array()
        .ok_or_else(|| "ERROR: source must be a JSON array of store entries.".to_owned())?;

    let mut problems = Vec::new();
    let mut output = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let obj = entry.as_object();
        let id_val = obj.and_then(|m| m.get("id"));
        if !id_val.is_some_and(Value::is_string) {
            problems.push(format!("entry {index}: missing string \"id\""));
        }
        let id_disp = id_val
            .map(|v| service_kit::js::string(Some(v)))
            .unwrap_or_else(|| "undefined".into());
        if !obj.and_then(|m| m.get("items")).is_some_and(Value::is_array) {
            problems.push(format!("entry {index} ({id_disp}): missing items[]"));
        }
        if !obj.and_then(|m| m.get("prices")).is_some_and(Value::is_array) {
            problems.push(format!("entry {index} ({id_disp}): missing prices[]"));
        }

        let get_or = |key: &str, default: Value| {
            obj.and_then(|m| m.get(key))
                .filter(|v| !v.is_null())
                .cloned()
                .unwrap_or(default)
        };
        let mut item_map = Map::new();
        if let Some(id) = id_val {
            item_map.insert("id".into(), id.clone());
        }
        item_map.insert("displayName".into(), get_or("displayName", json!("")));
        item_map.insert("displayDescription".into(), get_or("displayDescription", json!("")));
        item_map.insert("displayPriority".into(), get_or("displayPriority", json!(0)));
        item_map.insert("prices".into(), get_or("prices", json!([])));
        item_map.insert("maxAllowed".into(), get_or("maxAllowed", json!(0)));
        item_map.insert("tags".into(), get_or("tags", json!([])));
        item_map.insert("items".into(), get_or("items", json!([])));
        item_map.insert(
            "duplicateInstancedItems".into(),
            get_or("duplicateInstancedItems", json!([])),
        );
        output.push(Value::Object(item_map));
    }

    let mut lines = vec![
        format!("store entries : {}", output.len()),
        format!(
            "mode          : {}",
            if apply {
                "APPLY"
            } else {
                "DRY RUN (pass --apply to write)"
            }
        ),
    ];

    if !problems.is_empty() {
        return Err("ERROR: refusing to proceed until the source entries are well-formed.".to_owned());
    }

    if !apply {
        lines.push("\nDry run complete. Nothing written.".into());
        return Ok(lines.join("\n"));
    }

    std::fs::create_dir_all(out_dir).map_err(|e| format!("ERROR: failed to create {}: {e}", out_dir.display()))?;
    let _ = backup_if_exists(&target_path)?;
    let json_text = stringify_pretty(&Value::Array(output.clone()), 2);
    std::fs::write(&target_path, json_text.as_bytes())
        .map_err(|e| format!("ERROR: failed to write {}: {e}", target_path.display()))?;
    lines.push(format!("\nWrote ladyluck_store.json ({} entries)", output.len()));
    Ok(lines.join("\n"))
}

// ---------------------------------------------------------------------------
// 6. enrich_ladyluck_store.ts
// ---------------------------------------------------------------------------

pub fn enrich_ladyluck_store(catalog_path: &Path, live_path: &Path, store_path: &Path) -> Result<String, String> {
    let catalog = load_catalog_map(catalog_path)?;
    if !live_path.exists() {
        return Err(format!("Live Lady Luck capture not found: {}", live_path.display()));
    }
    if !store_path.exists() {
        return Err(format!("Lady Luck store data not found: {}", store_path.display()));
    }

    let raw_live =
        std::fs::read_to_string(live_path).map_err(|e| format!("failed to read {}: {e}", live_path.display()))?;
    let live_val = jsjson::parse(raw_live.strip_prefix('\u{feff}').unwrap_or(&raw_live))
        .map_err(|e| format!("invalid JSON in {}: {e}", live_path.display()))?;
    let live = live_val
        .as_array()
        .filter(|a| !a.is_empty())
        .ok_or_else(|| "Live Lady Luck capture is not a SKU list".to_owned())?;

    let raw_store =
        std::fs::read_to_string(store_path).map_err(|e| format!("failed to read {}: {e}", store_path.display()))?;
    let existing_val = jsjson::parse(raw_store.strip_prefix('\u{feff}').unwrap_or(&raw_store))
        .map_err(|e| format!("invalid JSON in {}: {e}", store_path.display()))?;
    let existing_arr = existing_val
        .as_array()
        .ok_or_else(|| "Existing ladyluck_store.json is not a JSON array".to_owned())?;
    let mut existing: HashMap<String, &Map<String, Value>> = HashMap::new();
    for sku in existing_arr {
        if let Some(obj) = sku.as_object()
            && let Some(id) = obj.get("id").and_then(Value::as_str)
        {
            existing.insert(id.to_owned(), obj);
        }
    }

    let same_items = |a: &[Value], b: &[Value]| -> bool {
        a.len() == b.len()
            && a.iter()
                .zip(b.iter())
                .all(|(ia, ib)| ia.get("catalogId") == ib.get("catalogId") && ia.get("quantity") == ib.get("quantity"))
    };

    let mut store: Vec<Map<String, Value>> = Vec::new();
    let mut skipped = Vec::new();

    for live_offer_val in live {
        let Some(live_offer) = live_offer_val.as_object() else {
            continue;
        };
        let offer_id = live_offer.get("id").and_then(Value::as_str).unwrap_or("").to_owned();
        let live_items = live_offer
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let missing: Vec<&str> = live_items
            .iter()
            .filter_map(|i| i.get("catalogId").and_then(Value::as_str))
            .filter(|cid| !catalog.contains_key(*cid))
            .collect();
        if !missing.is_empty() {
            skipped.push(format!("{offer_id} ({} not in the catalogue)", missing.join(", ")));
            continue;
        }
        let previous = existing.get(&offer_id).copied();
        let prev_items = previous.and_then(|p| p.get("items")).and_then(Value::as_array);
        let keep_previous = prev_items.is_some_and(|pi| same_items(pi, &live_items));

        let prices: Vec<Value> = live_offer
            .get("prices")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .map(|p| {
                        json!({
                            "currencyId": p.get("currencyId").cloned().unwrap_or(Value::Null),
                            "price": p.get("price").cloned().unwrap_or(Value::Null)
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let items: Vec<Value> = if keep_previous {
            prev_items.cloned().unwrap_or_default()
        } else {
            live_items
                .iter()
                .map(|item| {
                    let cid = item.get("catalogId").and_then(Value::as_str).unwrap_or("");
                    let instanced = catalog.get(cid).and_then(|r| r.get("isStackable")) != Some(&Value::Bool(true));
                    json!({
                        "catalogId": cid,
                        "quantity": item.get("quantity").cloned().unwrap_or_else(|| json!(1)),
                        "instanced": instanced
                    })
                })
                .collect()
        };

        let dup_items: Vec<Value> = if keep_previous {
            previous
                .and_then(|p| p.get("duplicateInstancedItems"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        } else {
            live_offer
                .get("duplicateInstancedItems")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        };

        let first_cid = items.first().and_then(|i| i.get("catalogId")).and_then(Value::as_str);
        let image_path = first_cid
            .and_then(|cid| catalog.get(cid))
            .map(|row| {
                let d = parse_item_data(Some(row));
                [d.custom_icon, d.large_icon, d.icon]
                    .into_iter()
                    .find(|s| !s.is_empty())
                    .unwrap_or_default()
            })
            .unwrap_or_default();

        let prev_images = previous
            .and_then(|p| p.get("images"))
            .and_then(Value::as_object)
            .cloned();
        let final_images = if !image_path.is_empty() {
            let mut img = prev_images.unwrap_or_default();
            img.insert("standard".into(), Value::String(texture_reference(&image_path)?));
            Some((Value::Object(img), true))
        } else {
            prev_images.map(|img| (Value::Object(img), false))
        };

        let mut sku = Map::new();
        sku.insert("id".into(), Value::String(offer_id));
        if let Some(dn) = live_offer.get("displayName") {
            sku.insert("displayName".into(), dn.clone());
        }
        if let Some(dd) = live_offer.get("displayDescription") {
            sku.insert("displayDescription".into(), dd.clone());
        }
        if let Some(dp) = live_offer.get("displayPriority") {
            sku.insert("displayPriority".into(), dp.clone());
        }
        sku.insert("prices".into(), Value::Array(prices));
        sku.insert(
            "maxAllowed".into(),
            live_offer.get("maxAllowed").cloned().unwrap_or(Value::Null),
        );
        if let Some((img_val, _)) = final_images {
            sku.insert("images".into(), img_val);
        }
        sku.insert(
            "tags".into(),
            live_offer
                .get("tags")
                .and_then(Value::as_array)
                .cloned()
                .map(Value::Array)
                .unwrap_or_else(|| json!([])),
        );
        sku.insert("items".into(), Value::Array(items));
        sku.insert("duplicateInstancedItems".into(), Value::Array(dup_items));
        store.push(sku);
    }

    let image_count = store
        .iter()
        .filter(|sku| {
            let first_cid = sku
                .get("items")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(|i| i.get("catalogId"))
                .and_then(Value::as_str);
            first_cid.and_then(|cid| catalog.get(cid)).is_some_and(|row| {
                let d = parse_item_data(Some(row));
                !d.custom_icon.is_empty() || !d.large_icon.is_empty() || !d.icon.is_empty()
            })
        })
        .count();

    const STEEL: &str = "id_currency_marks_steel";
    const GILDED: &str = "id_currency_marks_gilded";
    let mut seen_sku_ids = HashSet::new();
    let mut steel_count = 0usize;
    let mut gilded_count = 0usize;
    for sku in &store {
        let id = sku.get("id").and_then(Value::as_str).unwrap_or("");
        if !seen_sku_ids.insert(id.to_owned()) {
            return Err(format!("Duplicate Lady Luck SKU id: {id}"));
        }
        let prices = sku
            .get("prices")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if prices.len() != 1 {
            return Err(format!("Expected exactly one price for {id}"));
        }
        let cid = prices[0].get("currencyId").and_then(Value::as_str).unwrap_or("");
        if cid == STEEL {
            steel_count += 1;
        } else if cid == GILDED {
            gilded_count += 1;
        } else {
            return Err(format!("Unsupported Lady Luck currency for {id}"));
        }
        let has_tag = sku
            .get("tags")
            .and_then(Value::as_array)
            .is_some_and(|arr| arr.iter().any(|t| t.as_str() == Some("ladyluckstore")));
        if !has_tag {
            return Err(format!("Missing ladyluckstore tag on {id}"));
        }
    }

    let mut retired: Vec<&str> = existing
        .keys()
        .filter(|id| !seen_sku_ids.contains(*id))
        .map(String::as_str)
        .collect();
    retired.sort_unstable();

    let store_values: Vec<Value> = store.into_iter().map(Value::Object).collect();
    let json_text = format!("{}\n", stringify_pretty(&Value::Array(store_values.clone()), 2));
    std::fs::write(store_path, json_text.as_bytes())
        .map_err(|e| format!("failed to write {}: {e}", store_path.display()))?;

    Ok([
        format!("Wrote {}", store_path.display()),
        format!(
            "SKUs: {} of {} live; explicit catalog images: {image_count}",
            store_values.len(),
            live.len()
        ),
        format!("Steel offers: {steel_count}; Gilded offers: {gilded_count}"),
        format!(
            "Skipped live SKUs: {}",
            if skipped.is_empty() {
                "none".into()
            } else {
                skipped.join("; ")
            }
        ),
        format!(
            "Retired SKUs: {}",
            if retired.is_empty() {
                "none".into()
            } else {
                retired.join(", ")
            }
        ),
    ]
    .join("\n"))
}

// ---------------------------------------------------------------------------
// 7. generate_platinum_store.ts
// ---------------------------------------------------------------------------

const PLATINUM_CATEGORY_TAGS: &[(&str, &str)] = &[
    ("weapon_ac", "skin_weapon_ac"),
    ("weapon_eb", "skin_weapon_eb"),
    ("weapon_dp", "skin_weapon_dp"),
    ("weapon_ih", "skin_weapon_ih"),
    ("weapon_ga", "skin_weapon_ga"),
    ("weapon_ms", "skin_weapon_ms"),
    ("weapon_cb", "skin_weapon_cb"),
    ("armour", "skin_armour"),
    ("lantern", "skin_lantern"),
    ("dye", "dye_armour"),
    ("slayer", "personality_character"),
    ("emote", "social_emote"),
    ("emoji", "social_emojis"),
    ("arrival", "social_arrival"),
    ("fabric", "personality_fabric"),
    ("standard", "personality_standard"),
    ("flaresigil", "personality_flaresigil"),
    ("glider", "hunting_gliders"),
    ("tonics", "supplies_supplies"),
    ("boosts", "supplies_boost"),
];

fn platinum_category_tag(category: &str) -> Option<&'static str> {
    PLATINUM_CATEGORY_TAGS
        .iter()
        .find(|(k, _)| *k == category)
        .map(|(_, v)| *v)
}

fn reconstructed_price(category: &str, rarity: i64) -> i64 {
    if category.starts_with("weapon_") {
        return match rarity {
            3 => 500,
            4 => 800,
            5 => 1200,
            _ => 800,
        };
    }
    match category {
        "armour" => match rarity {
            3 => 400,
            4 => 600,
            5 => 800,
            _ => 600,
        },
        "lantern" => match rarity {
            4 => 600,
            5 => 800,
            _ => 600,
        },
        "dye" | "slayer" | "tonics" | "boosts" => 100,
        "emote" => match rarity {
            3 => 300,
            4 => 500,
            5 => 700,
            _ => 500,
        },
        "emoji" => 200,
        "arrival" => 1000,
        "fabric" => 300,
        "standard" => 400,
        "flaresigil" => 250,
        "glider" => 800,
        _ => 100,
    }
}

fn category_for_catalog_id(catalog_id: &str) -> Option<&'static str> {
    if catalog_id.starts_with("WP_") {
        let code = catalog_id.split('_').nth(1)?.to_ascii_lowercase();
        let key = format!("weapon_{code}");
        return PLATINUM_CATEGORY_TAGS.iter().find(|(k, _)| *k == key).map(|(k, _)| *k);
    }
    if catalog_id.starts_with("AR_") {
        return Some("armour");
    }
    if catalog_id.starts_with("LT_") {
        return Some("lantern");
    }
    if catalog_id.starts_with("DYE_") {
        return Some("dye");
    }
    if catalog_id.starts_with("EM_INTRO") {
        return Some("arrival");
    }
    if catalog_id.starts_with("EM_") {
        return Some("emote");
    }
    if catalog_id.starts_with("EJ_") {
        return Some("emoji");
    }
    if catalog_id.starts_with("BNC_FABRIC") {
        return Some("fabric");
    }
    if catalog_id.starts_with("BNC_STANDARD") {
        return Some("standard");
    }
    if catalog_id.starts_with("BNC_SIGIL") || catalog_id.starts_with("BNC_ANIMATION") || catalog_id.starts_with("FL_") {
        return Some("flaresigil");
    }
    if catalog_id.starts_with("GD_") {
        return Some("glider");
    }
    if catalog_id.starts_with("QI_") {
        return Some("tonics");
    }
    None
}

fn is_grantable(catalog: &HashMap<String, Value>, catalog_id: &str) -> bool {
    const EXCLUDED: &[&str] = &[
        "deprecated",
        "hidden",
        "wip",
        "initial",
        "founder",
        "translator",
        "mastery",
        "partner",
    ];
    let Some(row) = catalog.get(catalog_id) else {
        return false;
    };
    let tags: HashSet<String> = row
        .get("tags")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).map(str::to_lowercase).collect())
        .unwrap_or_default();
    !EXCLUDED.iter().any(|ex| tags.contains(*ex))
}

fn has_resolved_name(catalog: &HashMap<String, Value>, catalog_id: &str) -> bool {
    let name = catalog
        .get(catalog_id)
        .and_then(|r| r.get("displayName"))
        .and_then(Value::as_str)
        .unwrap_or("");
    !name.is_empty() && !name.contains("MISSING STRING TABLE") && !name.starts_with("~DNT~")
}

fn is_instanced(catalog: &HashMap<String, Value>, catalog_id: &str) -> bool {
    catalog.get(catalog_id).and_then(|r| r.get("isStackable")) == Some(&Value::Bool(false))
}

fn duplicate_instanced_items_for(catalog_ids: &[String]) -> Vec<Value> {
    catalog_ids
        .iter()
        .filter(|cid| !cid.starts_with("QI_") && !cid.starts_with("TOKEN_") && !cid.starts_with("CURRENCY_"))
        .map(|cid| Value::String(cid.clone()))
        .collect()
}

fn decode_row_name_to_catalog_id(catalog: &HashMap<String, Value>, row_name: &str) -> Option<String> {
    const DECODABLE_PREFIXES: &[&str] = &["sku_", "single_", "contract_"];
    const TYPE_PREFIXES: &[&str] = &[
        "AR_", "WP_", "DYE_", "EM_", "TITLE_", "BNC_", "GD_", "LT_", "AC_", "QI_", "EJ_", "FL_",
    ];
    let mut base = row_name;
    for prefix in DECODABLE_PREFIXES {
        if let Some(stripped) = base.strip_prefix(prefix) {
            base = stripped;
            break;
        }
    }
    let upper = base.to_ascii_uppercase();
    if catalog.contains_key(&upper) {
        return Some(upper);
    }
    for tp in TYPE_PREFIXES {
        let candidate = format!("{tp}{upper}");
        if catalog.contains_key(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn resolve_images(
    baked_rows: &HashMap<String, Value>,
    catalog: &HashMap<String, Value>,
    row_name: &str,
    catalog_ids: &[String],
) -> Option<Value> {
    let baked = baked_rows.get(row_name);
    let has_baked_standard = baked
        .and_then(|b| b.get("standardImage"))
        .and_then(Value::as_str)
        .is_some_and(|s| s != "None");
    let has_baked_feature = baked
        .and_then(|b| b.get("featureImage"))
        .and_then(Value::as_str)
        .is_some_and(|s| s != "None");

    if has_baked_standard {
        return Some(json!({}));
    }
    for cid in catalog_ids {
        let Some(row) = catalog.get(cid) else {
            continue;
        };
        let d = parse_item_data(Some(row));
        for asset in [&d.custom_icon, &d.large_icon, &d.icon] {
            if asset.starts_with("/Game/") {
                return Some(json!({ "standard": format!("Engine.Texture2D'{asset}'") }));
            }
        }
    }
    has_baked_feature.then(|| json!({}))
}

pub fn resolve_platinum_export_file(exports_dir: &Path, base_name: &str) -> PathBuf {
    let legacy = exports_dir.join(format!("{base_name}_1_12.jsonl"));
    let current = exports_dir.join(format!("{base_name}_1_14_7.jsonl"));
    if legacy.exists() || !current.exists() {
        legacy
    } else {
        current
    }
}

pub fn generate_platinum_store(
    catalog_path: &Path,
    store_images_path: &Path,
    capture_dir: &Path,
    data_dir: &Path,
) -> Result<String, String> {
    let catalog = load_catalog_map(catalog_path)?;
    if !store_images_path.exists() {
        return Err(format!("Store item images not found: {}", store_images_path.display()));
    }
    let images_text = std::fs::read_to_string(store_images_path)
        .map_err(|e| format!("failed to read {}: {e}", store_images_path.display()))?;
    let mut baked_rows = HashMap::new();
    for line in images_text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let row = jsjson::parse(line).map_err(|e| format!("invalid JSON in {}: {e}", store_images_path.display()))?;
        if let Some(rn) = row.get("rowName").and_then(Value::as_str)
            && !rn.starts_with("NewRow")
        {
            baked_rows.insert(rn.to_owned(), row);
        }
    }

    let mut already_sold = HashSet::new();
    for file_name in ["reward_cache_store.json", "ladyluck_store.json"] {
        let path = data_dir.join(file_name);
        let text = std::fs::read_to_string(&path).map_err(|e| format!("failed to read {}: {e}", path.display()))?;
        let skus = jsjson::parse(text.strip_prefix('\u{feff}').unwrap_or(&text))
            .map_err(|e| format!("invalid JSON in {}: {e}", path.display()))?;
        if let Some(arr) = skus.as_array() {
            for sku in arr {
                if let Some(items) = sku.get("items").and_then(Value::as_array) {
                    for item in items {
                        if let Some(cid) = item.get("catalogId").and_then(Value::as_str) {
                            already_sold.insert(cid.to_owned());
                        }
                    }
                }
            }
        }
    }

    let rumour_re = Regex::new(r"^quest_rumour_(.+?)(?:_\d+)?$").map_err(|e| format!("regex error: {e}"))?;
    let mut quest_earned = HashSet::new();
    for row_name in baked_rows.keys() {
        if let Some(caps) = rumour_re.captures(row_name)
            && let Some(m) = caps.get(1)
        {
            quest_earned.insert(m.as_str().to_ascii_uppercase());
        }
    }

    let read_capture = |file_name: &str| -> Result<Vec<Value>, String> {
        let full = capture_dir.join(file_name);
        if !full.exists() {
            return Ok(Vec::new());
        }
        let text = std::fs::read_to_string(&full).map_err(|e| format!("failed to read {}: {e}", full.display()))?;
        let parsed = jsjson::parse(text.strip_prefix('\u{feff}').unwrap_or(&text))
            .map_err(|e| format!("invalid JSON in {}: {e}", full.display()))?;
        Ok(parsed.as_array().cloned().unwrap_or_default())
    };

    const CURRENCY_ID: &str = "id_currency_platinum";
    const SECTION_WEBSTORE: &str = "webstore";
    const SECTION_DYES: &str = "dyes";
    const SECTION_RANK_SKIP: &str = "hp_level_skip_rank";
    const SECTION_HUNTPASS: &str = "huntpass_store";

    let mut output: Vec<Value> = Vec::new();
    let mut claimed_sku_ids: HashSet<String> = HashSet::new();
    let mut claimed_catalog_ids: HashSet<String> = HashSet::new();
    let mut skipped: Vec<(String, usize)> = Vec::new();
    let mut skip = |reason: &str| {
        if let Some(entry) = skipped.iter_mut().find(|(r, _)| r == reason) {
            entry.1 += 1;
        } else {
            skipped.push((reason.to_owned(), 1));
        }
    };

    let cc_re = Regex::new(r"^(ent_)?(ccfp|cchd|cchs|ccbd)_").map_err(|e| format!("regex error: {e}"))?;

    let mut webstore_capture = read_capture("awakening_webstore.json")?;
    webstore_capture.extend(read_capture("dyes.json")?);

    for sku_val in &webstore_capture {
        let Some(sku) = sku_val.as_object() else {
            continue;
        };
        let sku_id = sku.get("id").and_then(Value::as_str).unwrap_or("");
        if claimed_sku_ids.contains(sku_id) {
            continue;
        }
        if !baked_rows.contains_key(sku_id) {
            skip("tierA: sku id not baked into 1.12 StoreItemsTable");
            continue;
        }
        if sku
            .get("platformOfferId")
            .is_some_and(|v| service_kit::js::truthy(Some(v)))
        {
            skip("tierA: real-money platform offer");
            continue;
        }
        if sku
            .get("loadoutSlots")
            .is_some_and(|v| service_kit::js::truthy(Some(v)))
        {
            skip("tierA: loadoutSlots grant unimplemented");
            continue;
        }

        let price = sku
            .get("prices")
            .and_then(Value::as_array)
            .and_then(|arr| {
                arr.iter()
                    .find(|e| e.get("currencyId").and_then(Value::as_str) == Some(CURRENCY_ID))
            })
            .and_then(|e| e.get("price"))
            .cloned();
        let Some(price) = price else {
            skip("tierA: not platinum-priced");
            continue;
        };

        let items = sku.get("items").and_then(Value::as_array).cloned().unwrap_or_default();
        let entitlements = sku
            .get("entitlements")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let sku_progression = sku.get("skuProgression").filter(|v| !v.is_null()).cloned();

        if items.is_empty() && entitlements.is_empty() && sku_progression.is_none() {
            skip("tierA: no payload of any kind");
            continue;
        }

        let mut bad_item = false;
        for item in &items {
            let cid = item.get("catalogId").and_then(Value::as_str).unwrap_or("");
            if !is_grantable(&catalog, cid) {
                skip("tierA: item absent from 1.12 or excluded content");
                bad_item = true;
                break;
            }
            if already_sold.contains(cid) {
                skip("tierA: overlaps Lady Luck / Reward Cache");
                bad_item = true;
                break;
            }
            if claimed_catalog_ids.contains(cid) {
                skip("tierA: catalog id already sold by another platinum SKU");
                bad_item = true;
                break;
            }
            if quest_earned.contains(cid) {
                skip("tierA: quest/Rumour-earned");
                bad_item = true;
                break;
            }
        }
        if bad_item {
            continue;
        }

        let mut categories = BTreeSet::new();
        for item in &items {
            let cid = item.get("catalogId").and_then(Value::as_str).unwrap_or("");
            if let Some(cat) = category_for_catalog_id(cid)
                && let Some(tag) = platinum_category_tag(cat)
            {
                categories.insert(tag.to_owned());
            }
        }

        let mut forced_section: Option<&str> = None;
        if items.is_empty() && !entitlements.is_empty() {
            let is_char_cust = entitlements.iter().all(|ent| {
                let name = ent.get("name").and_then(Value::as_str).unwrap_or("");
                cc_re.is_match(name) || name.starts_with("character_facepaint_")
            });
            if is_char_cust {
                categories.insert("personality_character".into());
            } else {
                forced_section = Some(SECTION_HUNTPASS);
            }
        }
        if categories.is_empty() && forced_section.is_none() {
            skip("tierA: content type has no 1.12 store tab");
            continue;
        }

        let catalog_ids: Vec<String> = items
            .iter()
            .filter_map(|i| i.get("catalogId").and_then(Value::as_str).map(str::to_owned))
            .collect();
        let Some(images) = resolve_images(&baked_rows, &catalog, sku_id, &catalog_ids) else {
            skip("tierA: no baked or catalog artwork");
            continue;
        };

        for cid in &catalog_ids {
            claimed_catalog_ids.insert(cid.clone());
        }
        claimed_sku_ids.insert(sku_id.to_owned());

        let is_dye = catalog_ids.iter().any(|cid| cid.starts_with("DYE_"));
        let mut tags: Vec<Value> = if let Some(sec) = forced_section {
            vec![Value::String(sec.into())]
        } else if is_dye {
            vec![
                Value::String(SECTION_WEBSTORE.into()),
                Value::String(SECTION_DYES.into()),
            ]
        } else {
            vec![Value::String(SECTION_WEBSTORE.into())]
        };
        for c in categories {
            tags.push(Value::String(c));
        }
        tags.push(Value::String("platinum_store_captured".into()));

        let display_priority = sku
            .get("displayPriority")
            .and_then(Value::as_f64)
            .filter(|n| *n != 0.0 && n.is_finite())
            .map_or(1000, |n| n as i64);
        let max_allowed = if !entitlements.is_empty() {
            json!(1)
        } else {
            sku.get("maxAllowed")
                .filter(|v| !v.is_null())
                .cloned()
                .unwrap_or(Value::Null)
        };
        let mapped_items: Vec<Value> = items
            .iter()
            .map(|i| {
                let cid = i.get("catalogId").and_then(Value::as_str).unwrap_or("");
                json!({
                    "catalogId": cid,
                    "quantity": i.get("quantity").cloned().unwrap_or_else(|| json!(1)),
                    "instanced": is_instanced(&catalog, cid)
                })
            })
            .collect();

        let mut out_sku = Map::new();
        out_sku.insert("id".into(), Value::String(sku_id.to_owned()));
        out_sku.insert(
            "displayName".into(),
            sku.get("displayName")
                .filter(|v| !v.is_null())
                .cloned()
                .unwrap_or_else(|| Value::String(sku_id.to_owned())),
        );
        out_sku.insert(
            "displayDescription".into(),
            sku.get("displayDescription")
                .filter(|v| !v.is_null())
                .cloned()
                .unwrap_or_else(|| json!("")),
        );
        out_sku.insert("displayPriority".into(), json!(display_priority));
        out_sku.insert("prices".into(), json!([{ "currencyId": CURRENCY_ID, "price": price }]));
        out_sku.insert("maxAllowed".into(), max_allowed);
        out_sku.insert("images".into(), images);
        out_sku.insert("tags".into(), Value::Array(tags));
        out_sku.insert("items".into(), Value::Array(mapped_items));
        out_sku.insert(
            "duplicateInstancedItems".into(),
            Value::Array(duplicate_instanced_items_for(&catalog_ids)),
        );
        if !entitlements.is_empty() {
            let ents: Vec<Value> = entitlements
                .iter()
                .map(|e| {
                    let dur = e
                        .get("duration")
                        .and_then(Value::as_f64)
                        .filter(|n| *n != 0.0 && n.is_finite())
                        .map_or(0, |n| n as i64);
                    json!({
                        "name": e.get("name").cloned().unwrap_or(Value::Null),
                        "duration": dur
                    })
                })
                .collect();
            out_sku.insert("entitlements".into(), Value::Array(ents));
        }
        if let Some(sp) = sku_progression
            && service_kit::js::truthy(Some(&sp))
        {
            out_sku.insert("skuProgression".into(), sp);
        }
        output.push(Value::Object(out_sku));
    }

    // Hunt Pass rank skips
    for sku_val in &read_capture("hp_level_skip_rank.json")? {
        let Some(sku) = sku_val.as_object() else {
            continue;
        };
        let sku_id = sku.get("id").and_then(Value::as_str).unwrap_or("");
        if !baked_rows.contains_key(sku_id) {
            skip("rank skip: sku id not baked");
            continue;
        }
        let price = sku
            .get("prices")
            .and_then(Value::as_array)
            .and_then(|arr| {
                arr.iter()
                    .find(|e| e.get("currencyId").and_then(Value::as_str) == Some(CURRENCY_ID))
            })
            .and_then(|e| e.get("price"))
            .cloned();
        let sku_prog = sku
            .get("skuProgression")
            .filter(|v| service_kit::js::truthy(Some(v)))
            .cloned();
        let (Some(price), Some(sku_prog)) = (price, sku_prog) else {
            skip("rank skip: missing price or progression");
            continue;
        };
        if !claimed_sku_ids.insert(sku_id.to_owned()) {
            continue;
        }
        let display_priority = sku
            .get("displayPriority")
            .and_then(Value::as_f64)
            .filter(|n| *n != 0.0 && n.is_finite())
            .map_or(1000, |n| n as i64);
        let images = resolve_images(&baked_rows, &catalog, sku_id, &[]).unwrap_or_else(|| json!({}));
        output.push(json!({
            "id": sku_id,
            "displayName": sku.get("displayName").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| json!("Hunt Pass Level Skip")),
            "displayDescription": sku.get("displayDescription").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| json!("")),
            "displayPriority": display_priority,
            "prices": [{ "currencyId": CURRENCY_ID, "price": price }],
            "maxAllowed": sku.get("maxAllowed").filter(|v| !v.is_null()).cloned().unwrap_or(Value::Null),
            "images": images,
            "tags": [SECTION_RANK_SKIP, "event", "platinum_store_captured"],
            "items": [],
            "duplicateInstancedItems": [],
            "skuProgression": sku_prog
        }));
    }

    // Hunt Pass premium passes
    for sku_val in &read_capture("huntpass_store.json")? {
        let Some(sku) = sku_val.as_object() else {
            continue;
        };
        let sku_id = sku.get("id").and_then(Value::as_str).unwrap_or("");
        if !baked_rows.contains_key(sku_id) {
            skip("huntpass pass: sku id not baked");
            continue;
        }
        let price = sku
            .get("prices")
            .and_then(Value::as_array)
            .and_then(|arr| {
                arr.iter()
                    .find(|e| e.get("currencyId").and_then(Value::as_str) == Some(CURRENCY_ID))
            })
            .and_then(|e| e.get("price"))
            .cloned();
        let Some(price) = price else {
            skip("huntpass pass: no platinum price (already-owned capture)");
            continue;
        };
        let entitlements = sku
            .get("entitlements")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if entitlements.is_empty() {
            skip("huntpass pass: no entitlement payload");
            continue;
        }
        if !claimed_sku_ids.insert(sku_id.to_owned()) {
            continue;
        }
        let display_priority = sku
            .get("displayPriority")
            .and_then(Value::as_f64)
            .filter(|n| *n != 0.0 && n.is_finite())
            .map_or(1000, |n| n as i64);
        let images = resolve_images(&baked_rows, &catalog, sku_id, &[]).unwrap_or_else(|| json!({}));
        let ents: Vec<Value> = entitlements
            .iter()
            .map(|e| {
                let dur = e
                    .get("duration")
                    .and_then(Value::as_f64)
                    .filter(|n| *n != 0.0 && n.is_finite())
                    .map_or(0, |n| n as i64);
                json!({
                    "name": e.get("name").cloned().unwrap_or(Value::Null),
                    "duration": dur
                })
            })
            .collect();
        output.push(json!({
            "id": sku_id,
            "displayName": sku.get("displayName").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| Value::String(sku_id.to_owned())),
            "displayDescription": sku.get("displayDescription").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| json!("")),
            "displayPriority": display_priority,
            "prices": [{ "currencyId": CURRENCY_ID, "price": price }],
            "maxAllowed": sku.get("maxAllowed").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| json!(1)),
            "images": images,
            "tags": [SECTION_HUNTPASS, "event_pass", "platinum_store_captured"],
            "items": [],
            "duplicateInstancedItems": [],
            "entitlements": ents
        }));
    }

    // Tier B
    let mut sorted_baked_keys: Vec<String> = baked_rows.keys().cloned().collect();
    sorted_baked_keys.sort_unstable();
    for row_name in sorted_baked_keys {
        if claimed_sku_ids.contains(&row_name) {
            continue;
        }
        if !["sku_", "single_", "contract_"].iter().any(|p| row_name.starts_with(p)) {
            skip("tierB: row family is a bundle/quest/pass, not a single decodable offer");
            continue;
        }
        let baked = &baked_rows[&row_name];
        let std_img = baked.get("standardImage").and_then(Value::as_str).unwrap_or("None");
        let feat_img = baked.get("featureImage").and_then(Value::as_str).unwrap_or("None");
        if std_img == "None" && feat_img == "None" {
            skip("tierB: baked row has no artwork");
            continue;
        }
        let Some(catalog_id) = decode_row_name_to_catalog_id(&catalog, &row_name) else {
            skip("tierB: row name does not decode to a 1.12 catalog id");
            continue;
        };
        if already_sold.contains(&catalog_id) {
            skip("tierB: overlaps Lady Luck / Reward Cache");
            continue;
        }
        if claimed_catalog_ids.contains(&catalog_id) {
            skip("tierB: catalog id already sold by another platinum SKU");
            continue;
        }
        if !is_grantable(&catalog, &catalog_id) {
            skip("tierB: excluded catalog content");
            continue;
        }
        if quest_earned.contains(&catalog_id) {
            skip("tierB: quest/Rumour-earned");
            continue;
        }
        if !has_resolved_name(&catalog, &catalog_id) {
            skip("tierB: unresolved display name");
            continue;
        }
        let Some(category) = category_for_catalog_id(&catalog_id) else {
            skip("tierB: content type has no 1.12 store tab");
            continue;
        };
        let Some(cat_tag) = platinum_category_tag(category) else {
            skip("tierB: content type has no 1.12 store tab");
            continue;
        };
        let row = &catalog[&catalog_id];
        let rarity = parse_item_data(Some(row)).rarity;
        let price = reconstructed_price(category, rarity);
        let cids = vec![catalog_id.clone()];
        let Some(images) = resolve_images(&baked_rows, &catalog, &row_name, &cids) else {
            skip("tierB: no artwork resolved");
            continue;
        };

        let mut tags = if category == "dye" {
            vec![
                Value::String(SECTION_WEBSTORE.into()),
                Value::String(SECTION_DYES.into()),
            ]
        } else {
            vec![Value::String(SECTION_WEBSTORE.into())]
        };
        tags.push(Value::String(cat_tag.into()));
        tags.push(Value::String("platinum_store_reconstructed".into()));
        tags.push(Value::String(format!("platinum_store_rarity_{rarity}")));

        claimed_sku_ids.insert(row_name.clone());
        claimed_catalog_ids.insert(catalog_id.clone());

        let max_allowed = if category == "tonics" || category == "boosts" {
            Value::Null
        } else {
            json!(1)
        };

        output.push(json!({
            "id": row_name,
            "displayName": row.get("displayName").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| Value::String(catalog_id.clone())),
            "displayDescription": row.get("description").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| json!("")),
            "displayPriority": 1000,
            "prices": [{ "currencyId": CURRENCY_ID, "price": price }],
            "maxAllowed": max_allowed,
            "images": images,
            "tags": tags,
            "items": [{ "catalogId": catalog_id, "quantity": 1, "instanced": is_instanced(&catalog, &catalog_id) }],
            "duplicateInstancedItems": duplicate_instanced_items_for(&cids)
        }));
    }

    let sort_key = |sku: &Value| -> String {
        let tags: Vec<&str> = sku
            .get("tags")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let section = if tags.contains(&SECTION_RANK_SKIP) {
            "2"
        } else if tags.contains(&SECTION_HUNTPASS) {
            "3"
        } else {
            "1"
        };
        let category_tag = PLATINUM_CATEGORY_TAGS
            .iter()
            .map(|(_, v)| *v)
            .find(|t| tags.contains(t))
            .unwrap_or("zzz");
        let price = sku
            .get("prices")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(|p| p.get("price"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let id = sku.get("id").and_then(Value::as_str).unwrap_or("");
        format!("{section}|{category_tag}|{price:06}|{id}")
    };

    output.sort_by(|left, right| locale_compare(&sort_key(left), &sort_key(right)));

    std::fs::create_dir_all(data_dir).map_err(|e| format!("failed to create {}: {e}", data_dir.display()))?;
    let output_path = data_dir.join("platinum_store.json");
    let json_text = format!("{}\n", stringify_pretty(&Value::Array(output.clone()), 2));
    std::fs::write(&output_path, json_text.as_bytes())
        .map_err(|e| format!("failed to write {}: {e}", output_path.display()))?;

    let captured_count = output
        .iter()
        .filter(|s| {
            s.get("tags")
                .and_then(Value::as_array)
                .is_some_and(|a| a.iter().any(|t| t.as_str() == Some("platinum_store_captured")))
        })
        .count();
    let reconstructed_count = output
        .iter()
        .filter(|s| {
            s.get("tags")
                .and_then(Value::as_array)
                .is_some_and(|a| a.iter().any(|t| t.as_str() == Some("platinum_store_reconstructed")))
        })
        .count();

    Ok(format!(
        "platinum_store.json: {} SKUs ({captured_count} captured price, {reconstructed_count} reconstructed price)",
        output.len()
    ))
}

// ---------------------------------------------------------------------------
// 8. generate_inventory_storage_policy.ts
// ---------------------------------------------------------------------------

fn walk_source_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    if !dir.exists() {
        return Ok(());
    }
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|e| format!("failed to read {}: {e}", dir.display()))? {
        let entry = entry.map_err(|e| format!("failed to read dir entry in {}: {e}", dir.display()))?;
        entries.push(entry);
    }
    for entry in entries {
        let full_path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|e| format!("failed to stat {}: {e}", full_path.display()))?;
        if file_type.is_dir() {
            walk_source_files(&full_path, out)?;
        } else if let Some(name) = full_path.file_name().and_then(|s| s.to_str()) {
            let lower = name.to_ascii_lowercase();
            if (Path::new(&lower)
                .extension()
                .is_some_and(|ext| ext == "ts" || ext == "json"))
                && name != "inventory_storage_policy.json"
                && !name.ends_with(".example.json")
            {
                out.push(full_path);
            }
        }
    }
    Ok(())
}

pub fn generate_inventory_storage_policy(
    catalog_path: &Path,
    src_dir: Option<&Path>,
    game_data_dir: &Path,
) -> Result<String, String> {
    if !catalog_path.exists() {
        return Err(format!("Catalog dump not found: {}", catalog_path.display()));
    }
    let catalog_text =
        std::fs::read_to_string(catalog_path).map_err(|e| format!("failed to read {}: {e}", catalog_path.display()))?;
    let mut catalog = HashMap::new();
    for line in catalog_text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let row = jsjson::parse(line).map_err(|e| format!("invalid JSON in {}: {e}", catalog_path.display()))?;
        if let Some(item_id) = row.get("itemId").and_then(Value::as_str).filter(|s| !s.is_empty()) {
            catalog.insert(item_id.to_owned(), row);
        }
    }

    let token_re = Regex::new(r"[A-Z][A-Z0-9_-]{2,}").map_err(|e| format!("regex error: {e}"))?;
    let mut referenced_ids = BTreeSet::new();

    if let Some(src) = src_dir.filter(|d| d.exists()) {
        let mut src_files = Vec::new();
        walk_source_files(src, &mut src_files)?;
        src_files.sort();
        for file in src_files {
            let text = std::fs::read_to_string(&file).map_err(|e| format!("failed to read {}: {e}", file.display()))?;
            for m in token_re.find_iter(&text) {
                if catalog.contains_key(m.as_str()) {
                    referenced_ids.insert(m.as_str().to_owned());
                }
            }
        }
    } else {
        for token in EMBEDDED_SRC_TOKENS.lines() {
            if catalog.contains_key(token) {
                referenced_ids.insert(token.to_owned());
            }
        }
    }

    let mut gd_files = Vec::new();
    walk_source_files(game_data_dir, &mut gd_files)?;
    gd_files.sort();
    for file in gd_files {
        let text = std::fs::read_to_string(&file).map_err(|e| format!("failed to read {}: {e}", file.display()))?;
        for m in token_re.find_iter(&text) {
            if catalog.contains_key(m.as_str()) {
                referenced_ids.insert(m.as_str().to_owned());
            }
        }
    }

    let mut entries = Map::new();
    for cid in &referenced_ids {
        let Some(row) = catalog.get(cid) else {
            continue;
        };
        let storage = if row.get("isStackable") == Some(&Value::Bool(true)) {
            "stacked"
        } else {
            "instanced"
        };
        let max_quantity = row
            .get("maxQuantity")
            .and_then(Value::as_f64)
            .filter(|n| n.is_finite())
            .map_or_else(|| json!(0), |n| jsjson::to_js(&json!(n)));
        let tags: Vec<Value> = row
            .get("tags")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(|s| Value::String(s.to_owned()))
                    .collect()
            })
            .unwrap_or_default();
        entries.insert(
            cid.clone(),
            json!({
                "storage": storage,
                "maxQuantity": max_quantity,
                "tags": tags
            }),
        );
    }

    let mut hasher = Sha256::new();
    hasher.update(catalog_text.as_bytes());
    let digest = hasher.finalize();
    let mut sha_hex = String::with_capacity(64);
    for b in digest {
        sha_hex.push_str(&format!("{b:02x}"));
    }

    let document = json!({
        "schema": 1,
        "targetVersion": "1.14.7",
        "catalogSha256": sha_hex,
        "generatedAt": "2026-10-08",
        "referencedCatalogIds": entries.len(),
        "sourceScope": ["src/**/*.ts", "src/**/*.json", "game-data/*.json"],
        "entries": entries
    });

    std::fs::create_dir_all(game_data_dir).map_err(|e| format!("failed to create {}: {e}", game_data_dir.display()))?;
    let output_path = game_data_dir.join("inventory_storage_policy.json");
    let json_text = format!("{}\n", stringify_pretty(&document, 2));
    std::fs::write(&output_path, json_text.as_bytes())
        .map_err(|e| format!("failed to write {}: {e}", output_path.display()))?;

    Ok([
        format!("Wrote {}", output_path.display()),
        format!("Referenced catalog ids: {}", entries.len()),
        format!("Catalog SHA-256: {sha_hex}"),
    ]
    .join("\n"))
}
