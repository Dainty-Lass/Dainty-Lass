// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The input readers, the item data of a catalog row, texture references and backups (spec §4).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use dainty_service_kit::js;
use regex::Regex;
use serde_json::Value;

pub(crate) fn show(path: &Path) -> String {
    path.display().to_string()
}

/// The file as text (lossy, as Node's `utf8` read).
pub(crate) fn read_text(path: &Path) -> std::io::Result<String> {
    std::fs::read(path).map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

pub(crate) fn drop_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

pub(crate) fn parse(text: &str) -> Result<Value, String> {
    dainty_service_kit::jsjson::parse(text).map_err(|e| e.to_string())
}

/// JSON lines: split on `\n`, a trailing `\r` dropped, blank lines skipped, no BOM stripping.
pub(crate) fn read_json_lines(path: &Path) -> Result<Vec<Value>, String> {
    let text = read_text(path).map_err(|e| format!("invalid JSON in {}: {e}", show(path)))?;
    let mut rows = Vec::new();
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if js::trim(line).is_empty() {
            continue;
        }
        rows.push(parse(line).map_err(|e| format!("invalid JSON in {}: {e}", show(path)))?);
    }
    Ok(rows)
}

/// A catalog row with its item data read once.
#[derive(Debug, Clone)]
pub(crate) struct Item {
    pub row: Value,
    pub rarity: i64,
    pub custom_icon: String,
    pub large_icon: String,
    pub icon: String,
}

impl Item {
    fn new(row: Value) -> Self {
        let data = row
            .get("customData")
            .and_then(Value::as_str)
            .and_then(|t| parse(t).ok())
            .and_then(|custom| {
                custom
                    .get("ItemData")
                    .and_then(Value::as_str)
                    .and_then(|t| parse(t).ok())
            });
        let field = |name: &str| {
            data.as_ref()
                .and_then(|d| d.get(name))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned()
        };
        let rarity = data
            .as_ref()
            .and_then(|d| d.get("RarityLevel"))
            .and_then(Value::as_f64)
            .filter(|r| r.is_finite())
            .map_or(0, |r| r.trunc() as i64);
        Self {
            rarity,
            custom_icon: field("CustomIcon"),
            large_icon: field("LargeIcon"),
            icon: field("Icon"),
            row,
        }
    }

    /// The first non-empty of the custom, large and small icons.
    pub fn image(&self) -> Option<&str> {
        [&self.custom_icon, &self.large_icon, &self.icon]
            .into_iter()
            .find(|s| !s.is_empty())
            .map(String::as_str)
    }

    pub fn str(&self, name: &str) -> Option<&str> {
        self.row.get(name).and_then(Value::as_str)
    }

    pub fn tags(&self) -> Vec<&str> {
        self.row
            .get("tags")
            .and_then(Value::as_array)
            .map(|t| t.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default()
    }

    pub fn has_tag_ci(&self, tags: &[&str]) -> Option<String> {
        self.tags()
            .into_iter()
            .map(str::to_lowercase)
            .find(|t| tags.contains(&t.as_str()))
    }

    /// `isStackable` exactly `true`.
    pub fn stackable(&self) -> bool {
        self.row.get("isStackable") == Some(&Value::Bool(true))
    }
}

pub(crate) type Catalog = HashMap<String, Item>;

/// Rows with a non-empty string `itemId`; a later row replaces an earlier one.
pub(crate) fn read_catalog(path: &Path) -> Result<Catalog, String> {
    if !path.is_file() {
        return Err(format!("Catalog dump not found: {}", show(path)));
    }
    Ok(catalog_from_rows(read_json_lines(path)?))
}

pub(crate) fn catalog_from_rows(rows: Vec<Value>) -> Catalog {
    let mut catalog = Catalog::new();
    for row in rows {
        if let Some(id) = row.get("itemId").and_then(Value::as_str).filter(|s| !s.is_empty()) {
            catalog.insert(id.to_owned(), Item::new(row));
        }
    }
    catalog
}

/// The store image rows whose `rowName` is a string not starting `NewRow`.
pub(crate) fn store_image_rows(rows: Vec<Value>) -> Vec<Value> {
    rows.into_iter()
        .filter(|r| {
            r.get("rowName")
                .and_then(Value::as_str)
                .is_some_and(|n| !n.starts_with("NewRow"))
        })
        .collect()
}

static TEXTURE_PATH: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"^/Game/[A-Za-z0-9_./-]+$").ok());

pub(crate) fn texture(path: &str) -> String {
    format!("Engine.Texture2D'{path}'")
}

/// A reference whose path cannot close the quote (spec §11).
pub(crate) fn checked_texture(path: &str) -> Result<String, String> {
    match TEXTURE_PATH.as_ref() {
        Some(re) if re.is_match(path) => Ok(texture(path)),
        _ => Err(format!("Invalid cooked texture path: {path}")),
    }
}

/// `toISOString()` with every `:` and `.` replaced by `-`.
pub(crate) fn backup_stamp() -> String {
    rustic_isles::season::to_iso_string(chrono::Utc::now().timestamp_millis()).replace([':', '.'], "-")
}

/// Copies an existing `target` beside itself; the backup's path, or none when there was nothing to back up.
pub(crate) fn back_up(target: &Path) -> Result<Option<PathBuf>, String> {
    if !target.exists() {
        return Ok(None);
    }
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let backup = target.with_file_name(format!("{name}.{}.bak", backup_stamp()));
    std::fs::copy(target, &backup)
        .map_err(|e| format!("failed to back up {} to {}: {e}", show(target), show(&backup)))?;
    Ok(Some(backup))
}

pub(crate) fn write_file(path: &Path, text: &str) -> Result<(), String> {
    std::fs::write(path, text).map_err(|e| format!("failed to write {}: {e}", show(path)))
}

pub(crate) fn create_dir(path: &Path) -> Result<(), String> {
    std::fs::create_dir_all(path).map_err(|e| format!("failed to create {}: {e}", show(path)))
}

pub(crate) fn create_parent(path: &Path) -> Result<(), String> {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => create_dir(parent),
        _ => Ok(()),
    }
}

/// A JS number as a JSON value, an integer when it is one.
pub(crate) fn number(n: f64) -> Value {
    dainty_service_kit::jsjson::to_js(&serde_json::Number::from_f64(n).map_or(Value::Null, Value::Number))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn item_data() {
        let custom = json!({"ItemData": json!({"RarityLevel": 4.7, "Icon": "/Game/a", "LargeIcon": ""}).to_string()});
        let item = Item::new(json!({"itemId": "X", "customData": custom.to_string()}));
        assert_eq!((item.rarity, item.image()), (4, Some("/Game/a")));
        let bad = Item::new(json!({"itemId": "X", "customData": "{"}));
        assert_eq!((bad.rarity, bad.image()), (0, None));
        let object = Item::new(json!({"itemId": "X", "customData": {"ItemData": "{}"}}));
        assert_eq!(object.rarity, 0);
        assert!(checked_texture("/Game/UI/Icons/Icon_0.Icon_0").is_ok());
        assert_eq!(
            checked_texture("/Game/x'y").unwrap_err(),
            "Invalid cooked texture path: /Game/x'y"
        );
        assert!(checked_texture("/Game/").is_err());
        let stamp = backup_stamp();
        let re = Regex::new(r"^\d{4}-\d\d-\d\dT\d\d-\d\d-\d\d-\d{3}Z$").unwrap();
        assert!(re.is_match(&stamp), "{stamp}");
        assert_eq!(number(1000.0), json!(1000));
        assert_eq!(number(2.5), json!(2.5));
    }

    #[test]
    fn json_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("c.jsonl");
        std::fs::write(
            &path,
            "{\"itemId\":\"A\",\"n\":1}\r\n\n  \n{\"itemId\":\"A\",\"n\":2}\n{\"itemId\":\"\"}\n",
        )
        .unwrap();
        let catalog = read_catalog(&path).unwrap();
        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog["A"].row["n"], 2);
        std::fs::write(&path, "\u{feff}{}").unwrap();
        assert!(read_catalog(&path).unwrap_err().starts_with("invalid JSON in "));
        assert!(
            read_catalog(&dir.path().join("none"))
                .unwrap_err()
                .starts_with("Catalog dump not found: ")
        );
    }
}
