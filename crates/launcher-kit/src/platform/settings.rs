// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The launcher's `settings.json` (spec §2.7.2): defaults, sanitising and the store.
//!
//! The file is player data older launchers wrote, so every write keeps their bytes: the key order, two-space
//! indent and no trailing newline (the plan's section 3).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use protocol::account::is_valid_username;
use protocol::invite::{
    clean_server_name, is_private_mode_host, is_valid_fingerprint, is_valid_host, is_valid_invite_code,
    is_valid_share_url,
};
use serde_json::{Map, Value, json};

use crate::core::game_language::{AUTO, GAME_LOCALES};
use crate::core::i18n::Language;
use crate::core::text::utf16_len;

/// The settings file's name in `userData`.
pub const SETTINGS_FILE: &str = "settings.json";
/// The temporary file a write goes through before the rename.
pub const SETTINGS_TMP_FILE: &str = "settings.json.tmp";
/// The only file version written.
pub const VERSION: u64 = 1;
/// The most saved servers kept.
pub const MAX_SAVED_SERVERS: usize = 20;
/// The most trusted runtime ids kept per server.
pub const MAX_TRUSTED_RUNTIMES: usize = 8;
/// UTF-16 limits of the folder-path fields (exclusive).
pub const GAME_ROOT_LIMIT: usize = 200;
/// See [`GAME_ROOT_LIMIT`].
pub const LOCAL_RUNTIME_DIR_LIMIT: usize = 260;
/// See [`GAME_ROOT_LIMIT`].
pub const HOST_WORKSPACE_LIMIT: usize = 200;
/// See [`GAME_ROOT_LIMIT`].
pub const DUMPER_DLL_LIMIT: usize = 260;
/// The runtime profiles, the default first.
pub const RUNTIME_PROFILES: [&str; 2] = ["production", "development"];
/// The flags written only while `true`, after `dumperDll` and in this order, so older files keep their bytes.
pub const OPTIONAL_FLAGS: [&str; 3] = ["minimizeToTray", "closeToTray", "showGameConsole"];

/// Which folder-path rule applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    /// A drive letter, `:` and a separator.
    Windows,
    /// An absolute path.
    Linux,
}

impl Os {
    /// The OS this build runs on.
    #[must_use]
    pub const fn current() -> Self {
        if cfg!(windows) { Self::Windows } else { Self::Linux }
    }
}

/// The settings a fresh install starts with, `language` being the OS language.
#[must_use]
pub fn defaults(language: Language) -> Map<String, Value> {
    let Value::Object(map) = json!({
        "version": VERSION,
        "server": null,
        "savedServers": [],
        "gameRoot": null,
        "graphics": -1,
        "windowed": false,
        "streamerMode": false,
        "language": language.code(),
        "gameLanguage": AUTO,
        "runtimeProfile": RUNTIME_PROFILES[0],
        "hostTools": false,
        "usernames": {},
        "backupOffered": {},
        "trustedRuntimes": {},
        "localRuntimeDir": null,
        "hostWorkspace": null,
        "dumperDll": null,
    }) else {
        return Map::new();
    };
    map
}

/// `raw` made safe to keep: every field checked, unknown keys dropped, the key order fixed. `language` fills in an
/// absent or unknown `language`.
#[must_use]
pub fn sanitize(raw: &Value, language: Language, os: Os) -> Map<String, Value> {
    let empty = Map::new();
    let raw = raw.as_object().unwrap_or(&empty);
    let field = |key: &str| raw.get(key).unwrap_or(&Value::Null);

    let current = sanitize_server(field("server"));
    let saved = saved_servers(field("savedServers"), current.as_ref());
    // A 1.4.4 server can't be joined any more (RD5), but the list still shows it.
    let current = current.filter(|server| server.get("game").and_then(Value::as_str) != Some("revived"));

    let language = field("language")
        .as_str()
        .and_then(Language::from_code)
        .unwrap_or(language);
    let game_language = one_of(field("gameLanguage"), &[AUTO], &GAME_LOCALES).unwrap_or(AUTO);
    let runtime_profile = one_of(field("runtimeProfile"), &RUNTIME_PROFILES, &[]).unwrap_or(RUNTIME_PROFILES[0]);
    let graphics = integral(field("graphics"))
        .filter(|n| (-1..=4).contains(n))
        .unwrap_or(-1);

    let mut out = Map::new();
    out.insert("version".into(), json!(VERSION));
    out.insert("server".into(), current.map_or(Value::Null, Value::Object));
    out.insert(
        "savedServers".into(),
        Value::Array(saved.into_iter().map(Value::Object).collect()),
    );
    out.insert("gameRoot".into(), folder(field("gameRoot"), GAME_ROOT_LIMIT, os));
    out.insert("graphics".into(), json!(graphics));
    out.insert("windowed".into(), json!(is_true(field("windowed"))));
    out.insert("streamerMode".into(), json!(is_true(field("streamerMode"))));
    out.insert("language".into(), json!(language.code()));
    out.insert("gameLanguage".into(), json!(game_language));
    out.insert("runtimeProfile".into(), json!(runtime_profile));
    out.insert("hostTools".into(), json!(is_true(field("hostTools"))));
    out.insert("usernames".into(), Value::Object(usernames(field("usernames"))));
    out.insert(
        "backupOffered".into(),
        Value::Object(backup_offered(field("backupOffered"))),
    );
    out.insert(
        "trustedRuntimes".into(),
        Value::Object(trusted_runtimes(field("trustedRuntimes"))),
    );
    out.insert(
        "localRuntimeDir".into(),
        folder(field("localRuntimeDir"), LOCAL_RUNTIME_DIR_LIMIT, os),
    );
    out.insert(
        "hostWorkspace".into(),
        folder(field("hostWorkspace"), HOST_WORKSPACE_LIMIT, os),
    );
    let dumper = folder(field("dumperDll"), DUMPER_DLL_LIMIT, os);
    let dumper = match dumper.as_str() {
        Some(path) if path.to_ascii_lowercase().ends_with(".dll") => dumper,
        _ => Value::Null,
    };
    out.insert("dumperDll".into(), dumper);
    for flag in OPTIONAL_FLAGS {
        if is_true(field(flag)) {
            out.insert(flag.into(), Value::Bool(true));
        }
    }
    out
}

/// A server entry as stored, or `None` when it is refused.
#[must_use]
pub fn sanitize_server(raw: &Value) -> Option<Map<String, Value>> {
    let raw = raw.as_object()?;
    let name = clean_server_name(raw.get("name")?.as_str()?)?;
    let public = raw.get("mode").and_then(Value::as_str) == Some("public");
    let host = raw.get("host")?.as_str()?;
    if !is_valid_host(host) || (!public && !is_private_mode_host(host)) {
        return None;
    }
    let port = integral(raw.get("port")?).filter(|p| (1..=65535).contains(p))?;
    let fingerprint = raw
        .get("fp")
        .and_then(Value::as_str)
        .filter(|fp| is_valid_fingerprint(fp));
    if public && fingerprint.is_none() {
        return None;
    }
    let game = if raw.get("game").and_then(Value::as_str) == Some("revived") {
        "revived"
    } else {
        "reforged"
    };
    let share = raw
        .get("share")
        .and_then(Value::as_str)
        .filter(|share| !public && is_valid_share_url(share));
    let code = raw
        .get("code")
        .and_then(Value::as_str)
        .filter(|code| is_valid_invite_code(code));

    let mut out = Map::new();
    out.insert("game".into(), json!(game));
    out.insert("mode".into(), json!(if public { "public" } else { "private" }));
    out.insert("host".into(), json!(host));
    out.insert("port".into(), json!(port));
    out.insert("name".into(), json!(name));
    out.insert("share".into(), json!(share));
    out.insert("fp".into(), json!(fingerprint.filter(|_| public)));
    out.insert("code".into(), json!(code));
    Some(out)
}

/// The saved-servers list: valid entries, unique by mode, host and port, at most [`MAX_SAVED_SERVERS`]; `current`
/// replaces its own entry or goes first, and the list is cut again.
#[must_use]
pub fn saved_servers(raw: &Value, current: Option<&Map<String, Value>>) -> Vec<Map<String, Value>> {
    let mut list: Vec<Map<String, Value>> = Vec::new();
    for entry in raw.as_array().map(Vec::as_slice).unwrap_or_default() {
        let Some(entry) = sanitize_server(entry) else { continue };
        if list.len() < MAX_SAVED_SERVERS && !list.iter().any(|kept| same_server(kept, &entry)) {
            list.push(entry);
        }
    }
    if let Some(current) = current {
        match list.iter_mut().find(|kept| same_server(kept, current)) {
            Some(kept) => kept.clone_from(current),
            None => list.insert(0, current.clone()),
        }
        list.truncate(MAX_SAVED_SERVERS);
    }
    list
}

/// Whether two stored entries are the same server: mode, host and port.
#[must_use]
pub fn same_server(a: &Map<String, Value>, b: &Map<String, Value>) -> bool {
    ["mode", "host", "port"].iter().all(|key| a.get(*key) == b.get(*key))
}

/// Whether `text` passes the folder-path rule for `os` with an exclusive UTF-16 `limit`.
#[must_use]
pub fn is_folder_path(text: &str, limit: usize, os: Os) -> bool {
    let units = utf16_len(text);
    if units <= 3 || units >= limit || text.contains('\0') {
        return false;
    }
    let bytes = text.as_bytes();
    match os {
        Os::Windows => bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && matches!(bytes[2], b'\\' | b'/'),
        Os::Linux => bytes[0] == b'/',
    }
}

/// The settings file, kept sanitised in memory and on disk.
#[derive(Debug, Clone)]
pub struct SettingsStore {
    path: PathBuf,
    os: Os,
    map: Map<String, Value>,
}

impl SettingsStore {
    /// Reads `<user_data>/settings.json`; a missing, unreadable or non-JSON file gives the defaults for
    /// `os_language`. Nothing is written until the first change.
    #[must_use]
    pub fn load(user_data: &Path, os_language: Language, os: Os) -> Self {
        let path = user_data.join(SETTINGS_FILE);
        let raw = fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
            .unwrap_or(Value::Null);
        let map = sanitize(&raw, os_language, os);
        Self { path, os, map }
    }

    /// The file's path.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The sanitised settings.
    #[must_use]
    pub fn map(&self) -> &Map<String, Value> {
        &self.map
    }

    /// Applies `edit` to a copy of the map, sanitises it with the language it names (English when it names
    /// none) and writes it. On a failed write the map in memory still takes the change.
    pub fn update(&mut self, edit: impl FnOnce(&mut Map<String, Value>)) -> io::Result<()> {
        let mut next = self.map.clone();
        edit(&mut next);
        self.replace(Value::Object(next))
    }

    /// Replaces the settings with `raw`, sanitised as [`update`](Self::update) does, and writes them.
    pub fn replace(&mut self, raw: Value) -> io::Result<()> {
        self.map = sanitize(&raw, Language::En, self.os);
        self.write()
    }

    /// Writes the map through `settings.json.tmp` and a rename, so a crash never leaves half a file.
    pub fn write(&self) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = self.path.with_file_name(SETTINGS_TMP_FILE);
        fs::write(&tmp, to_bytes(&self.map))?;
        fs::rename(&tmp, &self.path)
    }
}

/// The bytes written for `map`: two-space indent, keys in the map's order, no trailing newline.
#[must_use]
pub fn to_bytes(map: &Map<String, Value>) -> Vec<u8> {
    serde_json::to_vec_pretty(map).unwrap_or_default()
}

fn is_true(value: &Value) -> bool {
    value == &Value::Bool(true)
}

/// A JSON number with no fraction, as JavaScript's `Number.isInteger` sees it (`2.0` and `-0` count).
fn integral(value: &Value) -> Option<i64> {
    let number = value.as_number()?;
    if let Some(n) = number.as_i64() {
        return Some(n);
    }
    let f = number.as_f64()?;
    // Only small values matter here; anything this large fails every range check anyway.
    (f.is_finite() && f.fract() == 0.0 && f.abs() < 1e15).then_some(f as i64)
}

fn one_of(value: &Value, first: &[&'static str], rest: &[&'static str]) -> Option<&'static str> {
    let text = value.as_str()?;
    first.iter().chain(rest).copied().find(|known| *known == text)
}

fn folder(value: &Value, limit: usize, os: Os) -> Value {
    match value.as_str() {
        Some(path) if is_folder_path(path, limit, os) => json!(path),
        _ => Value::Null,
    }
}

fn is_server_id(key: &str) -> bool {
    key.len() == 24 && key.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn is_runtime_id(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn entries(value: &Value) -> impl Iterator<Item = (&String, &Value)> {
    value
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(key, _)| is_server_id(key))
}

fn usernames(value: &Value) -> Map<String, Value> {
    entries(value)
        .filter(|(_, name)| name.as_str().is_some_and(is_valid_username))
        .map(|(key, name)| (key.clone(), name.clone()))
        .collect()
}

fn backup_offered(value: &Value) -> Map<String, Value> {
    entries(value)
        .filter(|(_, offered)| is_true(offered))
        .map(|(key, _)| (key.clone(), Value::Bool(true)))
        .collect()
}

fn trusted_runtimes(value: &Value) -> Map<String, Value> {
    entries(value)
        .filter_map(|(key, ids)| {
            let mut kept: Vec<&str> = Vec::new();
            for id in ids.as_array()?.iter().filter_map(Value::as_str) {
                if is_runtime_id(id) && !kept.contains(&id) {
                    kept.push(id);
                }
            }
            let newest = &kept[kept.len().saturating_sub(MAX_TRUSTED_RUNTIMES)..];
            (!newest.is_empty()).then(|| (key.clone(), json!(newest)))
        })
        .collect()
}

#[cfg(test)]
mod tests;
