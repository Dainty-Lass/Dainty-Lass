// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Pure parsers for `reforged-host` output, `host.toml`/`host.json`, and `vswhere` (Spec §2.3).
//! Compiled directly into the `launcher_host` fuzz target.

use serde_json::{Map, Value};
use toml_edit::{DocumentMut, Item};

pub const PINNED_MP_COMMIT: &str = "b4d12f0";
pub const PATCH_SENTINEL: &str = ".reforged-patches.json";
pub const EXPORT_FLAGS: &str = "# Written by the DaintyLass launcher (Host page). See docs/sdk-and-game-data.md.\r\nEXPORT_CATALOG=1\r\nEXPORT_SLAYERS_PATH=1\r\nEXPORT_HUNTS=1\r\nEXPORT_STORE_RUNTIME=1\r\nEXPORT_PROGRESSION=1\r\nEXPORT_COMBAT=0\r\nEXPORT_SKINS=0\r\nEXPORT_SKINS_RESOLVE_STRINGTABLE=0\r\nEXPORT_WEAPON_SLOTS=0\r\nEXPORT_TABLE_INVENTORY=0\r\nEXPORT_CELLS=0\r\nEXPORT_DROP_TABLES=0\r\nEXPORT_CURRENCY_NAMES=0\r\n";
pub const SERVER_PORT: u16 = 61000;
pub const MAX_SERVER_NAME_LEN: usize = 80;
pub const MIN_MULTIPLIER: f64 = 1.0;
pub const MAX_MULTIPLIER: f64 = 100.0;

pub const INVITE_LINK_PREFIXES: [&str; 3] = ["dauntless://", "reforged-revival://", "dauntless-revived://"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteLine {
    pub link: String,
    pub for_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateCheck {
    pub behind: u64,
    pub head: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateDone {
    pub to: String,
    pub files: u32,
    pub runtime_changed: bool,
}

pub type UpdateSummary = UpdateDone;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerMode {
    Private,
    Public,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ServerConfig {
    pub name: Option<String>,
    pub mode: ServerMode,
    pub host: Option<String>,
    pub multipliers: Option<(f64, f64)>,
}

pub type ServerConfigValue = ServerConfig;
pub type ServerMultipliersValue = (f64, f64);

/// Truncates `text` to at most `max` Unicode scalar values.
#[must_use]
pub fn cut(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

fn strip_ansi_csi(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            let mut j = i + 2;
            while j < bytes.len() && (0x20..=0x3f).contains(&bytes[j]) {
                j += 1;
            }
            if j < bytes.len() && (0x40..=0x7e).contains(&bytes[j]) {
                i = j + 1;
                continue;
            }
        }
        let Some(ch) = raw[i..].chars().next() else {
            break;
        };
        if ch != '\r' {
            out.push(ch);
        }
        i += ch.len_utf8();
    }
    out
}

/// Strips ANSI CSI sequences and carriage returns, trims trailing whitespace, and caps at 400 characters.
#[must_use]
pub fn clean_line(raw: &str) -> String {
    let stripped = strip_ansi_csi(raw);
    cut(stripped.trim_end(), 400)
}

/// Cleans a line and returns `Some` if non-empty.
#[must_use]
pub fn sanitize_output_line(raw: &str) -> Option<String> {
    let cleaned = clean_line(raw);
    let trimmed = cleaned.trim();
    (!trimmed.is_empty()).then_some(cleaned)
}

/// Returns the last non-empty cleaned line of `text`, capped at 300 characters.
#[must_use]
pub fn last_output_line(text: &str) -> Option<String> {
    text.lines()
        .rev()
        .map(clean_line)
        .find(|l| !l.trim().is_empty())
        .map(|l| cut(l.trim(), 300))
}

/// Finds the last `Error:` line (returning its message) or the last non-empty non-JSON line, capped at 300 chars.
#[must_use]
pub fn failure_line(lines: &[String]) -> Option<String> {
    for raw in lines.iter().rev() {
        let cleaned = clean_line(raw);
        let trimmed = cleaned.trim();
        if let Some(rest) = trimmed.strip_prefix("Error:") {
            let msg = rest.trim();
            if !msg.is_empty() {
                return Some(cut(msg, 300));
            }
        }
    }
    for raw in lines.iter().rev() {
        let cleaned = clean_line(raw);
        let trimmed = cleaned.trim();
        if !trimmed.is_empty() && !trimmed.starts_with('{') {
            return Some(cut(trimmed, 300));
        }
    }
    None
}

/// Parses the last JSON object line in `lines`.
#[must_use]
pub fn last_json(lines: &[String]) -> Option<Map<String, Value>> {
    for raw in lines.iter().rev() {
        let cleaned = clean_line(raw);
        let trimmed = cleaned.trim();
        if trimmed.starts_with('{')
            && trimmed.ends_with('}')
            && let Ok(Value::Object(map)) = serde_json::from_str::<Value>(trimmed)
        {
            return Some(map);
        }
    }
    None
}

/// Validates and cleans a raw invite link string.
#[must_use]
pub fn clean_invite_link(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > 4096 || trimmed.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return None;
    }
    if !INVITE_LINK_PREFIXES.iter().any(|prefix| trimmed.starts_with(prefix)) {
        return None;
    }
    Some(trimmed.to_owned())
}

/// Extracts an [`InviteLine`] from the `--json` output of `reforged-host invite`.
#[must_use]
pub fn invite_line(lines: &[String]) -> Option<InviteLine> {
    let json = last_json(lines)?;
    let raw_link = json
        .get("link")
        .or_else(|| json.get("invite"))
        .and_then(Value::as_str)?;
    let link = clean_invite_link(raw_link)?;
    let for_name = json
        .get("for")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.chars().count() <= 40 && !s.chars().any(char::is_control))
        .map(str::to_owned);
    Some(InviteLine { link, for_name })
}

fn clean_short_token(raw: &str, max: usize) -> Option<String> {
    let cleaned: String = raw.trim().chars().filter(|c| !c.is_control()).take(max).collect();
    (!cleaned.is_empty()).then_some(cleaned)
}

/// Parses `reforged-host update --check --json` output.
#[must_use]
pub fn update_check(json: &Map<String, Value>) -> Option<UpdateCheck> {
    let behind = json.get("behind")?.as_u64()?;
    let head = json
        .get("head")
        .and_then(Value::as_str)
        .and_then(|s| clean_short_token(s, 40))
        .unwrap_or_default();
    Some(UpdateCheck { behind, head })
}

/// Parses `reforged-host update --json` completion output.
#[must_use]
pub fn update_done(json: &Map<String, Value>) -> Option<UpdateDone> {
    if json.get("error").is_some() {
        return None;
    }
    let to = json
        .get("to")
        .and_then(Value::as_str)
        .and_then(|s| clean_short_token(s, 40))?;
    let files = json
        .get("files")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(0);
    let runtime_changed = json
        .get("runtimeChanged")
        .or_else(|| json.get("runtime_changed"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Some(UpdateDone {
        to,
        files,
        runtime_changed,
    })
}

/// Parses update output lines into [`UpdateDone`].
#[must_use]
pub fn parse_update_output(lines: &[String]) -> Option<UpdateDone> {
    let json = last_json(lines)?;
    update_done(&json)
}

/// Extracts an update failure code or message from `reforged-host update --json`.
#[must_use]
pub fn update_failure(json: Option<&Map<String, Value>>, fallback: Option<&str>) -> String {
    if let Some(map) = json {
        if let Some(code) = map.get("error").and_then(Value::as_str) {
            let trimmed = code.trim();
            if !trimmed.is_empty()
                && trimmed.len() <= 48
                && trimmed
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
            {
                return format!("update_{trimmed}");
            }
        }
        if let Some(msg) = map.get("message").and_then(Value::as_str) {
            let cleaned = clean_line(msg);
            let trimmed = cleaned.trim();
            if !trimmed.is_empty() {
                return cut(trimmed, 300);
            }
        }
    }
    if let Some(fb) = fallback {
        let cleaned = clean_line(fb);
        let trimmed = cleaned.trim();
        let stripped = trimmed.strip_prefix("Error:").map_or(trimmed, str::trim);
        if !stripped.is_empty() {
            return cut(stripped, 300);
        }
    }
    "update".to_owned()
}

/// Extracts the published runtime version from `reforged-host publish-runtime` output lines.
#[must_use]
pub fn published_version(lines: &[String]) -> Option<String> {
    for raw in lines.iter().rev() {
        let cleaned = clean_line(raw);
        let trimmed = cleaned.trim();
        if let Some(rest) = trimmed.strip_prefix("Published runtime ") {
            let version = rest.split_whitespace().next().unwrap_or("");
            if !version.is_empty()
                && version.len() <= 48
                && version
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
            {
                return Some(version.to_owned());
            }
        }
    }
    None
}

/// Extracts the exported game-data pack file name from `reforged-host gamedata --export` output lines.
#[must_use]
pub fn pack_written(lines: &[String]) -> Option<String> {
    for raw in lines.iter().rev() {
        let cleaned = clean_line(raw);
        let trimmed = cleaned.trim();
        if trimmed.starts_with("Exported ")
            && let Some((_, tail)) = trimmed.rsplit_once(" to ")
        {
            let path = tail.trim_end_matches('.');
            let file_name = path.rsplit(['\\', '/']).next().unwrap_or(path).trim();
            if !file_name.is_empty()
                && !file_name.contains(['\\', '/'])
                && !file_name.chars().any(char::is_control)
                && file_name.chars().count() <= 120
            {
                return Some(file_name.to_owned());
            }
        }
    }
    None
}

/// Rounds a multiplier to 2 decimal places if it is finite and in `1.0..=100.0`.
#[must_use]
pub fn round_multiplier(n: f64) -> Option<f64> {
    if !n.is_finite() || !(MIN_MULTIPLIER..=MAX_MULTIPLIER).contains(&n) {
        return None;
    }
    let rounded = (n * 100.0).round() / 100.0;
    (MIN_MULTIPLIER..=MAX_MULTIPLIER).contains(&rounded).then_some(rounded)
}

fn clean_config_field(raw: &str, max_chars: usize) -> Option<String> {
    let cleaned: String = raw.trim().chars().filter(|c| !c.is_control()).take(max_chars).collect();
    let trimmed = cleaned.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn clean_host_field(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return None;
    }
    Some(cut(trimmed, 255))
}

fn parse_mode(raw: &str) -> Option<ServerMode> {
    match raw.trim() {
        "private" => Some(ServerMode::Private),
        "public" => Some(ServerMode::Public),
        _ => None,
    }
}

/// Parses legacy `.workspace/config/host.json`.
#[must_use]
pub fn server_config(raw: &str) -> Option<ServerConfig> {
    if raw.len() > 1024 * 1024 {
        return None;
    }
    let root: Value = serde_json::from_str(raw).ok()?;
    let obj = root.as_object()?;
    let srv = obj.get("server").and_then(Value::as_object);

    let mode_str = obj
        .get("mode")
        .or_else(|| srv.and_then(|s| s.get("mode")))
        .and_then(Value::as_str)?;
    let mode = parse_mode(mode_str)?;

    let name = obj
        .get("name")
        .or_else(|| srv.and_then(|s| s.get("name")))
        .and_then(Value::as_str)
        .and_then(|s| clean_config_field(s, MAX_SERVER_NAME_LEN));

    let host = obj
        .get("host")
        .or_else(|| srv.and_then(|s| s.get("public_host").or_else(|| s.get("host"))))
        .and_then(Value::as_str)
        .and_then(clean_host_field);

    let multipliers = if mode == ServerMode::Private {
        let mult_obj = obj
            .get("multipliers")
            .or_else(|| obj.get("test_multipliers"))
            .or_else(|| obj.get("gameplay"))
            .and_then(Value::as_object);
        mult_obj.and_then(|m| {
            let xp = m
                .get("xp")
                .or_else(|| m.get("xp_multiplier"))
                .and_then(Value::as_f64)
                .and_then(round_multiplier)?;
            let rewards = m
                .get("rewards")
                .or_else(|| m.get("reward_multiplier"))
                .and_then(Value::as_f64)
                .and_then(round_multiplier)?;
            (xp > 1.0 || rewards > 1.0).then_some((xp, rewards))
        })
    } else {
        None
    };

    Some(ServerConfig {
        name,
        mode,
        host,
        multipliers,
    })
}

fn toml_number(item: &Item) -> Option<f64> {
    if let Some(f) = item.as_float() {
        Some(f)
    } else {
        item.as_integer().map(|i| i as f64)
    }
}

/// Parses `.workspace/config/host.toml`.
#[must_use]
pub fn server_config_toml(raw: &str) -> Option<ServerConfig> {
    if raw.len() > 1024 * 1024 {
        return None;
    }
    let doc: DocumentMut = raw.parse().ok()?;
    let srv = doc.get("server").and_then(Item::as_table_like);

    let mode_str = srv
        .and_then(|s| s.get("mode"))
        .or_else(|| doc.get("mode"))
        .and_then(Item::as_str)?;
    let mode = parse_mode(mode_str)?;

    let name = srv
        .and_then(|s| s.get("name"))
        .or_else(|| doc.get("name"))
        .and_then(Item::as_str)
        .and_then(|s| clean_config_field(s, MAX_SERVER_NAME_LEN));

    let host = srv
        .and_then(|s| s.get("public_host").or_else(|| s.get("host")))
        .or_else(|| doc.get("host"))
        .and_then(Item::as_str)
        .and_then(clean_host_field);

    let multipliers = if mode == ServerMode::Private {
        let mult_tbl = doc
            .get("test_multipliers")
            .or_else(|| doc.get("multipliers"))
            .or_else(|| doc.get("gameplay"))
            .and_then(Item::as_table_like);
        mult_tbl.and_then(|m| {
            let xp = m
                .get("xp")
                .or_else(|| m.get("xp_multiplier"))
                .and_then(toml_number)
                .and_then(round_multiplier)?;
            let rewards = m
                .get("rewards")
                .or_else(|| m.get("reward_multiplier"))
                .and_then(toml_number)
                .and_then(round_multiplier)?;
            (xp > 1.0 || rewards > 1.0).then_some((xp, rewards))
        })
    } else {
        None
    };

    Some(ServerConfig {
        name,
        mode,
        host,
        multipliers,
    })
}

/// Parses `host.toml` when present, falling back to `host.json`.
#[must_use]
pub fn parse_server_config_files(toml_raw: Option<&str>, json_raw: Option<&str>) -> Option<ServerConfig> {
    toml_raw
        .and_then(server_config_toml)
        .or_else(|| json_raw.and_then(server_config))
}

/// Checks if a CMake/source text carries the DaintyLass patch marker.
#[must_use]
pub fn is_patched_source(cmake: &str) -> bool {
    cmake.contains(PATCH_SENTINEL) || cmake.contains("MysticParadox")
}

/// Classifies a set of `(real_content, example_content)` pairs as `"missing"`, `"synthetic"`, or `"real"`.
#[must_use]
pub fn classify_game_data(files: &[(&str, &str)]) -> &'static str {
    if files.is_empty() || files.iter().any(|(real, _)| real.is_empty()) {
        return "missing";
    }
    if files
        .iter()
        .any(|(real, example)| !example.is_empty() && real == example)
    {
        return "synthetic";
    }
    "real"
}

/// Parses `XP_MULTIPLIER` and `REWARD_MULTIPLIER` from a `.env` file text.
#[must_use]
pub fn parse_env_multipliers(raw: &str) -> Option<ServerMultipliersValue> {
    let mut xp: Option<f64> = None;
    let mut rewards: Option<f64> = None;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((k, v)) = trimmed.split_once('=') else {
            continue;
        };
        let key = k.trim();
        let val = v.trim().trim_matches(['"', '\'']);
        if key == "XP_MULTIPLIER" {
            xp = val.parse::<f64>().ok().and_then(round_multiplier);
        } else if key == "REWARD_MULTIPLIER" || key == "REWARDS_MULTIPLIER" {
            rewards = val.parse::<f64>().ok().and_then(round_multiplier);
        }
    }
    let xp_val = xp.unwrap_or(1.0);
    let rew_val = rewards.unwrap_or(1.0);
    (xp_val > 1.0 || rew_val > 1.0).then_some((xp_val, rew_val))
}

/// Updates or removes `XP_MULTIPLIER` and `REWARD_MULTIPLIER` in an `.env` text.
#[must_use]
pub fn set_env_multipliers(existing: &str, multipliers: Option<ServerMultipliersValue>) -> String {
    let mut out: Vec<String> = existing
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !trimmed.starts_with("XP_MULTIPLIER=")
                && !trimmed.starts_with("REWARD_MULTIPLIER=")
                && !trimmed.starts_with("REWARDS_MULTIPLIER=")
        })
        .map(str::to_owned)
        .collect();
    if let Some((xp, rewards)) = multipliers
        && let (Some(x), Some(r)) = (round_multiplier(xp), round_multiplier(rewards))
        && (x > 1.0 || r > 1.0)
    {
        out.push(format!("XP_MULTIPLIER={x}"));
        out.push(format!("REWARD_MULTIPLIER={r}"));
    }
    let mut joined = out.join("\n");
    if !joined.is_empty() {
        joined.push('\n');
    }
    joined
}

/// Parses `vswhere -format json` output for the Visual Studio display name and major version.
#[must_use]
pub fn vswhere_install(raw: &str) -> Option<(String, Option<u32>)> {
    if raw.len() > 4 * 1024 * 1024 {
        return None;
    }
    let list: Value = serde_json::from_str(raw.trim_start_matches('\u{feff}')).ok()?;
    let first = list.as_array()?.first()?.as_object()?;
    let name = first
        .get("displayName")
        .or_else(|| first.get("installationName"))
        .and_then(Value::as_str)
        .and_then(|s| clean_config_field(s, 80))?;
    let major = first
        .get("installationVersion")
        .and_then(Value::as_str)
        .and_then(|ver| ver.split('.').next())
        .and_then(|maj| maj.trim().parse::<u32>().ok());
    Some((name, major))
}

/// Parses `vswhere -find MSBuild\**\Bin\MSBuild.exe` output for the first valid path.
#[must_use]
pub fn vswhere_msbuild(raw: &str) -> Option<String> {
    for line in raw.trim_start_matches('\u{feff}').lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty()
            && trimmed.len() <= 520
            && !trimmed.chars().any(char::is_control)
            && trimmed.to_ascii_lowercase().ends_with("msbuild.exe")
        {
            return Some(trimmed.to_owned());
        }
    }
    None
}

/// Selects the MSVC platform toolset for the detected Visual Studio major version.
#[must_use]
pub const fn platform_toolset(vs_major: Option<u32>) -> &'static str {
    match vs_major {
        Some(major) if major >= 18 => "v145",
        _ => "v143",
    }
}
