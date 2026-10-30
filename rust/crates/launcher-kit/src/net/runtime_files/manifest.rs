// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

use protocol::manifest::{
    DEFAULT_CHANGELIST, MAX_RUNTIME_FILE_BYTES, RUNTIME_FILE_NAMES, RuntimeFile, RuntimeManifest,
};
use protocol::text::{is_js_space, is_unsafe_char, js_trim};
use serde_json::Value;

#[allow(dead_code)]
pub const RUNTIME_MARKER_FILE: &str = ".reforged-runtime.json";
pub const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

fn clean_version_text(value: &str, max_len: usize) -> String {
    let mut filtered = String::with_capacity(value.len());
    let mut last_was_space = false;

    for ch in value.chars() {
        if is_unsafe_char(ch) {
            continue;
        }
        if is_js_space(ch) {
            if !last_was_space {
                filtered.push(' ');
                last_was_space = true;
            }
        } else {
            filtered.push(ch);
            last_was_space = false;
        }
    }

    let trimmed = js_trim(&filtered);
    let mut out = String::new();
    let mut units = 0usize;
    for ch in trimmed.chars() {
        let len_utf16 = ch.len_utf16();
        if units + len_utf16 > max_len {
            break;
        }
        out.push(ch);
        units += len_utf16;
    }
    js_trim(&out).to_owned()
}

fn is_lower_hex_64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn parse_runtime_manifest(raw: &Value) -> Option<RuntimeManifest> {
    let obj = raw.as_object()?;
    let raw_version = obj.get("version")?.as_str()?;
    let version = clean_version_text(raw_version, 48);
    if version.is_empty() {
        return None;
    }

    let files_arr = obj.get("files")?.as_array()?;
    if files_arr.len() != RUNTIME_FILE_NAMES.len() {
        return None;
    }

    let mut files = Vec::with_capacity(RUNTIME_FILE_NAMES.len());
    for item in files_arr {
        let f_obj = item.as_object()?;
        let name = f_obj.get("name")?.as_str()?;
        if !RUNTIME_FILE_NAMES.contains(&name) {
            return None;
        }
        if files.iter().any(|existing: &RuntimeFile| existing.name == name) {
            return None;
        }
        let sha256 = f_obj.get("sha256")?.as_str()?;
        if !is_lower_hex_64(sha256) {
            return None;
        }
        let size = f_obj.get("size")?.as_u64()?;
        if !(1..=MAX_RUNTIME_FILE_BYTES).contains(&size) {
            return None;
        }
        files.push(RuntimeFile {
            name: name.to_owned(),
            size,
            sha256: sha256.to_owned(),
        });
    }

    files.sort_by(|a, b| a.name.encode_utf16().cmp(b.name.encode_utf16()));

    let changelist = obj
        .get("build")
        .and_then(Value::as_object)
        .and_then(|b| b.get("changelist"))
        .and_then(Value::as_u64)
        .filter(|&n| (1..=MAX_SAFE_INTEGER).contains(&n) && u32::try_from(n).is_ok())
        .unwrap_or(DEFAULT_CHANGELIST);

    Some(RuntimeManifest::new(version, changelist, files))
}
