// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

use serde_json::{Map, Value, json};

pub const MAX_ENTRIES: usize = 32;
const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

#[derive(Debug, Clone, PartialEq)]
pub struct CacheEntry {
    pub size: u64,
    pub mtime_ms: f64,
    pub sha256: String,
}

fn is_lower_hex_64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn parse_cache_file(raw: &str) -> Vec<(String, CacheEntry)> {
    let Ok(parsed) = serde_json::from_str::<Value>(raw) else {
        return Vec::new();
    };
    let Some(files) = parsed.get("files").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    for (key, value) in files {
        if key.is_empty() {
            continue;
        }
        let Some(obj) = value.as_object() else {
            continue;
        };
        let Some(size) = obj
            .get("size")
            .and_then(Value::as_u64)
            .filter(|&n| n <= MAX_SAFE_INTEGER)
        else {
            continue;
        };
        let Some(mtime_ms) = obj
            .get("mtimeMs")
            .and_then(Value::as_f64)
            .filter(|&n| n.is_finite() && n > 0.0)
        else {
            continue;
        };
        let Some(sha256) = obj.get("sha256").and_then(Value::as_str).filter(|s| is_lower_hex_64(s)) else {
            continue;
        };
        entries.push((
            key.clone(),
            CacheEntry {
                size,
                mtime_ms,
                sha256: sha256.to_owned(),
            },
        ));
    }
    if entries.len() > MAX_ENTRIES {
        entries.drain(0..(entries.len() - MAX_ENTRIES));
    }
    entries
}

pub fn write_cache_file(entries: &[(String, CacheEntry)]) -> String {
    let start = entries.len().saturating_sub(MAX_ENTRIES);
    let mut files = Map::new();
    for (key, entry) in &entries[start..] {
        if key.is_empty() {
            continue;
        }
        files.insert(
            key.clone(),
            json!({
                "size": entry.size,
                "mtimeMs": entry.mtime_ms,
                "sha256": entry.sha256,
            }),
        );
    }
    let root = json!({ "files": Value::Object(files) });
    serde_json::to_string_pretty(&root).unwrap_or_else(|_| "{\"files\":{}}".to_owned())
}
