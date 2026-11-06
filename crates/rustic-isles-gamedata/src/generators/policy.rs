// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The inventory storage policy (spec §5.8, §6): every catalog id the backend's code or game data names, and
//! whether it is stored stacked or instanced.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use super::io::{Catalog, catalog_from_rows, create_dir, read_text, show, write_file};
use super::pretty::stringify_pretty;

static TOKEN: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"[A-Z][A-Z0-9_-]{2,}").ok());

/// Where the candidate ids come from besides the game data folder (spec §6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenSource {
    /// Every `.ts` and `.json` file under a folder, as the game data walk reads them
    Walk(PathBuf),
    /// An explicit list of candidate ids the version supplies
    Seeds(Vec<String>),
}

fn ends_with_ci(name: &str, suffix: &str) -> bool {
    name.len() >= suffix.len() && name.as_bytes()[name.len() - suffix.len()..].eq_ignore_ascii_case(suffix.as_bytes())
}

fn scanned(name: &str) -> bool {
    (ends_with_ci(name, ".ts") || ends_with_ci(name, ".json"))
        && name != "inventory_storage_policy.json"
        && !ends_with_ci(name, ".example.json")
}

/// The files to scan under `dir`, in sorted path order. Symbolic links are followed (a listed weak spot); a folder
/// already seen is not entered twice.
fn walk(dir: &Path, seen: &mut HashSet<PathBuf>, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let real = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    if !seen.insert(real) {
        return Ok(());
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(format!("failed to read {}: {e}", show(dir))),
    };
    for entry in entries {
        let entry = entry.map_err(|e| format!("failed to read {}: {e}", show(dir)))?;
        let path = entry.path();
        if path.is_dir() {
            walk(&path, seen, out)?;
        } else if scanned(&entry.file_name().to_string_lossy()) {
            out.push(path);
        }
    }
    Ok(())
}

fn scan(dir: &Path, catalog: &Catalog, found: &mut HashSet<String>) -> Result<(), String> {
    let mut files = Vec::new();
    walk(dir, &mut HashSet::new(), &mut files)?;
    files.sort();
    for file in files {
        let text = read_text(&file).map_err(|e| format!("failed to read {}: {e}", show(&file)))?;
        add_tokens(&text, catalog, found);
    }
    Ok(())
}

fn add_tokens(text: &str, catalog: &Catalog, found: &mut HashSet<String>) {
    let Some(re) = TOKEN.as_ref() else {
        return;
    };
    for token in re.find_iter(text) {
        if catalog.contains_key(token.as_str()) {
            found.insert(token.as_str().to_owned());
        }
    }
}

fn entry(item: &super::io::Item) -> Value {
    let storage = if item.stackable() { "stacked" } else { "instanced" };
    let max = item
        .row
        .get("maxQuantity")
        .filter(|v| v.as_f64().is_some_and(f64::is_finite))
        .cloned()
        .unwrap_or(json!(0));
    json!({ "storage": storage, "maxQuantity": max, "tags": item.tags() })
}

pub fn generate_inventory_storage_policy(
    catalog: &Path,
    tokens: &TokenSource,
    game_data_dir: &Path,
) -> Result<String, String> {
    let Ok(bytes) = std::fs::read(catalog) else {
        return Err(format!("Catalog dump not found: {}", show(catalog)));
    };
    let sha: String = Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    let catalog_rows = super::io::read_json_lines(catalog)?;
    let catalog_map = catalog_from_rows(catalog_rows);

    let mut found = HashSet::new();
    match tokens {
        TokenSource::Walk(dir) => scan(dir, &catalog_map, &mut found)?,
        TokenSource::Seeds(seeds) => {
            found.extend(seeds.iter().filter(|s| catalog_map.contains_key(s.as_str())).cloned());
        }
    }
    scan(game_data_dir, &catalog_map, &mut found)?;

    let sorted: BTreeMap<&String, Value> = found
        .iter()
        .filter_map(|id| catalog_map.get(id).map(|item| (id, entry(item))))
        .collect();
    let count = sorted.len();
    let entries: Map<String, Value> = sorted.into_iter().map(|(k, v)| (k.clone(), v)).collect();
    let document = json!({
        "schema": 1,
        "targetVersion": "1.14.7",
        "catalogSha256": sha,
        // Fixed texts, so a regenerated file compares byte for byte (spec §12 item 14)
        "generatedAt": "2026-10-08",
        "referencedCatalogIds": count,
        "sourceScope": ["src/**/*.ts", "src/**/*.json", "game-data/*.json"],
        "entries": entries,
    });
    create_dir(game_data_dir)?;
    let target = game_data_dir.join("inventory_storage_policy.json");
    write_file(&target, &format!("{}\n", stringify_pretty(&document, 2)))?;
    Ok([
        format!("Wrote {}", show(&target)),
        format!("Referenced catalog ids: {count}"),
        format!("Catalog SHA-256: {sha}"),
    ]
    .join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scanned_names() {
        assert!(scanned("a.JSON") && scanned("x.ts") && scanned("raw.source.json"));
        assert!(!scanned("inventory_storage_policy.json") && !scanned("a.example.json") && !scanned("a.json.bak"));
    }
}
