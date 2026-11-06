// The key tables, `scalar`, `free_item` and `fresh` are DaintyLass's `rust/crates/hostkit/src/hosttoml.rs` (plan
// `fb46a4a`), MIT, Copyright (c) 2026 Silent Spud; LICENSE-MIT.txt there. Changed for 1.14.7 alone.

//! `host.json` → `host.toml`: the Node kit's settings file moved to the one DaintyLass reads, with the keys of the
//! games DaintyLass no longer runs (Dauntless Revived 1.4.4 and Rustic Isles' 1.12.0) left out. The values are
//! carried as they are: `dainty-host setup`, run next, checks them and fills in the rest with its defaults.

use serde_json::{Map, Value};
use toml_edit::{DocumentMut, Item, Table};

use crate::error::{ImportError, Result};

/// A key: its name in host.json, its name in host.toml, and whether it may be null (left out).
type Field = (&'static str, &'static str, bool);

/// host.json's keys without `edition`, `legacyGameDir` and `legacyPublicPort`, which only ran 1.4.4 beside 1.14.7.
const TOP: &[Field] = &[
    ("version", "version", false),
    ("mode", "mode", false),
    ("host", "host", false),
    ("gameAddress", "game_address", false),
    ("name", "name", false),
    ("gameDir", "game_dir", true),
    ("registration", "registration", false),
    ("shareUrl", "share_url", true),
    ("mongo", "mongo", false),
    ("maxHuntServers", "max_hunt_servers", false),
    ("multipliers", "multipliers", false),
    ("publicPort", "public_port", false),
    ("performance", "performance", false),
    ("linux", "linux", false),
    ("database", "database", false),
    ("guard", "guard", false),
    ("history", "history", false),
    ("switches", "switches", false),
    ("network", "network", false),
    ("remote", "remote", false),
    ("modules", "modules", false),
];

/// The keys that ran 1.4.4 beside 1.14.7: dropped, and reported.
pub const RETIRED_KEYS: [&str; 3] = ["edition", "legacyGameDir", "legacyPublicPort"];

/// The game modules DaintyLass no longer has: their `[modules]` entries are dropped.
pub const RETIRED_MODULES: [&str; 2] = ["revived-1-4-4", "isles-1-12-0"];

/// The top-level key whose tables and lists are read as they are.
const FREE_FORM: &str = "modules";

const NETWORK: &[Field] = &[
    ("node", "node", false),
    ("region", "region", false),
    ("leader", "leader", false),
    ("wrongNodePolicy", "wrong_node_policy", false),
    ("nodes", "nodes", false),
];

const REMOTE: &[Field] = &[
    ("role", "role", false),
    ("node", "node", false),
    ("region", "region", false),
    ("controllerUrl", "controller_url", false),
    ("controllerNode", "controller_node", false),
    ("controllerCertFp", "controller_cert_fp", false),
    ("syncIntervalSecs", "sync_interval_secs", false),
    ("label", "label", false),
    ("localOverrides", "local_overrides", false),
];

fn is_peer_list(prefix: &str, json: &str) -> bool {
    prefix == "network." && json == "nodes"
}

const MONGO: &[Field] = &[
    ("uri", "uri", false),
    ("manage", "manage", false),
    ("mongod", "mongod", true),
];

const MULTIPLIERS: &[Field] = &[("xp", "xp", false), ("rewards", "rewards", false)];

const PERFORMANCE: &[Field] = &[
    ("serverFps", "server_fps", false),
    ("idleFps", "idle_fps", false),
    ("trainingOnDemand", "training_on_demand", false),
    ("trainingIdleMinutes", "training_idle_minutes", false),
    ("verboseGameLogs", "verbose_game_logs", false),
    ("memoryGuard", "memory_guard", false),
    ("minFreeMemoryMb", "min_free_memory_mb", false),
    ("huntMemoryMb", "hunt_memory_mb", false),
];

const LINUX: &[Field] = &[
    ("wine", "wine", true),
    ("proton", "proton", true),
    ("xvfb", "xvfb", false),
    ("firewallInterface", "firewall_interface", true),
];

const DATABASE: &[Field] = &[
    ("engine", "engine", false),
    ("postgresHost", "postgres_host", false),
    ("postgresPort", "postgres_port", false),
    ("postgresUser", "postgres_user", false),
    ("postgresDatabase", "postgres_database", false),
    ("allowRemote", "allow_remote", false),
    ("freshStart", "fresh_start", false),
];

const GUARD: &[Field] = &[
    ("banLimit", "ban_limit", false),
    ("banWindowMinutes", "ban_window_minutes", false),
    ("banGuard", "ban_guard", false),
    ("revertLimit", "revert_limit", false),
    ("revertGuard", "revert_guard", false),
    ("revokeGuard", "revoke_guard", false),
    ("delayMinutes", "delay_minutes", false),
    ("expireHours", "expire_hours", false),
];

const HISTORY: &[Field] = &[("shipToPeer", "ship_to_peer", false)];

const SWITCHES: &[Field] = &[
    ("reforge", "reforge", false),
    ("masteryPersistence", "mastery_persistence", false),
    ("bountyPersistence", "bounty_persistence", false),
    ("cooldownPersistence", "cooldown_persistence", false),
    ("accountErasure", "account_erasure", false),
    ("logPurge", "log_purge", false),
    ("rateLimit", "rate_limit", false),
    ("rateLimitPerMinute", "rate_limit_per_minute", false),
    (
        "gameserverRateLimitPerMinute",
        "gameserver_rate_limit_per_minute",
        false,
    ),
    ("hostRateLimitPerMinute", "host_rate_limit_per_minute", false),
];

/// The keys of a table at the top of the file; `None` for a single value.
fn table_fields(json_key: &str) -> Option<&'static [Field]> {
    match json_key {
        "mongo" => Some(MONGO),
        "multipliers" => Some(MULTIPLIERS),
        "performance" => Some(PERFORMANCE),
        "linux" => Some(LINUX),
        "database" => Some(DATABASE),
        "guard" => Some(GUARD),
        "history" => Some(HISTORY),
        "switches" => Some(SWITCHES),
        "network" => Some(NETWORK),
        "remote" => Some(REMOTE),
        _ => None,
    }
}

fn bad(text: impl Into<String>) -> ImportError {
    ImportError::invalid(text)
}

fn scalar(value: &Value, key: &str) -> Result<toml_edit::Value> {
    match value {
        Value::String(s) => Ok(s.as_str().into()),
        Value::Bool(b) => Ok((*b).into()),
        Value::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => Ok(i.into()),
            (None, Some(f)) => Ok(f.into()),
            (None, None) => Err(bad(format!("{key}: {n} is not a number host.toml can hold"))),
        },
        _ => Err(bad(format!("{key} must be a single value"))),
    }
}

/// A free-form part as TOML: an object of objects as `[a.b]` tables, a list of objects as `[[a.b]]`.
fn free_item(value: &Value, path: &str) -> Result<Item> {
    match value {
        Value::Object(map) => {
            let mut table = Table::new();
            for (key, inner) in map {
                table.insert(key, free_item(inner, &format!("{path}.{key}"))?);
            }
            // A table that holds only tables prints no header of its own
            if !table.is_empty()
                && table
                    .iter()
                    .all(|(_, item)| item.is_table() || item.is_array_of_tables())
            {
                table.set_implicit(true);
            }
            Ok(Item::Table(table))
        }
        Value::Array(items) if !items.is_empty() && items.iter().all(Value::is_object) => {
            let mut tables = toml_edit::ArrayOfTables::new();
            for item in items {
                if let Item::Table(table) = free_item(item, path)? {
                    tables.push(table);
                }
            }
            Ok(Item::ArrayOfTables(tables))
        }
        Value::Array(items) => {
            let mut array = toml_edit::Array::new();
            for item in items {
                array.push(scalar(item, path)?);
            }
            Ok(Item::Value(toml_edit::Value::Array(array)))
        }
        other => Ok(Item::Value(scalar(other, path)?)),
    }
}

const HEADER: &str = "\
# This server's settings (dainty-host), moved here from the Node kit's host.json by dainty-legacy. Edit them
# here, then run `dainty-host setup` again to apply them; setup keeps your comments and every value it does not
# change. A setting that is left out takes its default. The secrets are not here: they are in secrets.env.
";

/// A new host.toml: the header, then the single values, then the tables, as TOML needs.
fn fresh(fields: &Map<String, Value>) -> Result<String> {
    let mut out = String::from(HEADER);
    for &(json, toml, _) in TOP {
        if table_fields(json).is_some() || json == FREE_FORM {
            continue;
        }
        if let Some(v) = fields.get(json).filter(|v| !v.is_null()) {
            out.push_str(&format!("{toml} = {}\n", scalar(v, toml)?.to_string().trim()));
        }
    }
    for &(json, toml, _) in TOP {
        let (Some(sub), Some(Value::Object(values))) = (table_fields(json), fields.get(json)) else {
            continue;
        };
        out.push_str(&format!("\n[{toml}]\n"));
        for &(sub_json, sub_toml, _) in sub {
            if is_peer_list(&format!("{toml}."), sub_json) {
                continue;
            }
            match values.get(sub_json) {
                None | Some(Value::Null) => {}
                Some(Value::Array(items)) => {
                    let mut arr = toml_edit::Array::new();
                    for item in items {
                        arr.push(scalar(item, sub_toml)?);
                    }
                    out.push_str(&format!("{sub_toml} = {}\n", arr.to_string().trim()));
                }
                Some(v) => {
                    out.push_str(&format!("{sub_toml} = {}\n", scalar(v, sub_toml)?.to_string().trim()));
                }
            }
        }
        if let Some(nodes) = values.get("nodes").filter(|v| !v.is_null()) {
            let mut doc = DocumentMut::new();
            let mut parent = Table::new();
            parent.insert("nodes", free_item(nodes, "network.nodes")?);
            parent.set_implicit(true);
            doc.insert(toml, Item::Table(parent));
            out.push('\n');
            out.push_str(&doc.to_string());
        }
    }
    if let Some(modules) = fields.get(FREE_FORM).filter(|v| !v.is_null()) {
        let mut doc = DocumentMut::new();
        doc.insert(FREE_FORM, free_item(modules, FREE_FORM)?);
        out.push('\n');
        out.push_str(&doc.to_string());
    }
    Ok(out)
}

/// `[modules]` without the retired game modules: out of `compiled`, `external` and the `[modules.<id>]` tables.
fn strip_retired_modules(modules: &mut Value, dropped: &mut Vec<String>) {
    let Some(object) = modules.as_object_mut() else {
        return;
    };
    let retired = |value: &Value| value.as_str().is_some_and(|id| RETIRED_MODULES.contains(&id));
    if let Some(Value::Array(list)) = object.get_mut("compiled") {
        list.retain(|id| {
            let keep = !retired(id);
            if !keep {
                dropped.push(format!("modules.compiled: {}", id.as_str().unwrap_or_default()));
            }
            keep
        });
    }
    if let Some(Value::Array(list)) = object.get_mut("external") {
        list.retain(|entry| {
            let keep = !entry.get("id").is_some_and(retired);
            if !keep {
                dropped.push(format!(
                    "modules.external: {}",
                    entry["id"].as_str().unwrap_or_default()
                ));
            }
            keep
        });
    }
    // A list emptied here goes, so the default applies rather than "none" (an empty `compiled` loads nothing)
    for list in ["compiled", "external"] {
        if object.get(list).and_then(Value::as_array).is_some_and(Vec::is_empty) {
            object.remove(list);
        }
    }
    for id in RETIRED_MODULES {
        if object.remove(id).is_some() {
            dropped.push(format!("modules.{id}"));
        }
    }
}

/// What a conversion made of a host.json.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Converted {
    pub toml: String,
    /// Keys left out: the retired ones, the retired modules' entries and any key host.toml doesn't have
    pub dropped: Vec<String>,
    /// The server ran only 1.4.4 (`edition = "legacy"`): it has no 1.14.7 game folder yet
    pub was_legacy_only: bool,
}

/// The host.toml text for a host.json document.
pub fn convert(text: &str) -> Result<Converted> {
    let mut document: Value =
        serde_json::from_str(text).map_err(|e| bad(format!("host.json is not valid JSON: {e}")))?;
    let fields = document
        .as_object_mut()
        .ok_or_else(|| bad("host.json must hold a JSON object"))?;
    let was_legacy_only = fields.get("edition").and_then(Value::as_str) == Some("legacy");
    let mut dropped = Vec::new();
    for key in RETIRED_KEYS {
        if fields.remove(key).is_some() {
            dropped.push(key.to_owned());
        }
    }
    if let Some(modules) = fields.get_mut(FREE_FORM) {
        strip_retired_modules(modules, &mut dropped);
    }
    for key in fields.keys() {
        if !TOP.iter().any(|(json, _, _)| json == key) {
            dropped.push(key.clone());
        }
    }
    for &(json, toml, _) in TOP {
        let (Some(sub), Some(Value::Object(values))) = (table_fields(json), fields.get(json)) else {
            continue;
        };
        for key in values.keys() {
            if !sub.iter().any(|(sub_json, _, _)| sub_json == key) {
                dropped.push(format!("{toml}.{key}"));
            }
        }
    }
    Ok(Converted {
        toml: fresh(fields)?,
        dropped,
        was_legacy_only,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_host_json_becomes_host_toml_without_the_retired_games() {
        let text = json!({
            "version": 1,
            "mode": "public",
            "edition": "both",
            "host": "play.example.org",
            "gameAddress": "203.0.113.7",
            "name": "Old Kit",
            "gameDir": "C:/Games/Dauntless",
            "legacyGameDir": "C:/Games/Dauntless-1.4.4",
            "legacyPublicPort": 4443,
            "publicPort": 443,
            "shareUrl": null,
            "mongo": { "uri": "mongodb://127.0.0.1:27017/reforged", "manage": true, "mongod": null },
            "performance": { "serverFps": 30, "verboseGameLogs": false, "turbo": true },
            "network": { "node": "eu-1", "nodes": [{ "node": "us-1", "address": "198.51.100.2" }] },
            "modules": {
                "compiled": ["isles-1-14-7", "revived-1-4-4"],
                "external": [{ "id": "isles-1-12-0", "path": "/srv/x" }],
                "revived-1-4-4": { "deploy": { "enabled": false } },
                "isles-1-14-7": { "backend": { "level": "info" } }
            },
            "somethingElse": 1
        })
        .to_string();
        let converted = convert(&text).unwrap();
        let doc: DocumentMut = converted.toml.parse().unwrap();
        assert_eq!(doc["host"].as_str(), Some("play.example.org"));
        assert_eq!(doc["game_address"].as_str(), Some("203.0.113.7"));
        assert_eq!(doc["public_port"].as_integer(), Some(443));
        assert!(doc.get("share_url").is_none());
        assert_eq!(doc["mongo"]["uri"].as_str(), Some("mongodb://127.0.0.1:27017/reforged"));
        assert!(doc["mongo"].get("mongod").is_none());
        assert_eq!(doc["performance"]["server_fps"].as_integer(), Some(30));
        assert_eq!(doc["network"]["nodes"][0]["node"].as_str(), Some("us-1"));
        assert_eq!(doc["modules"]["compiled"].as_array().unwrap().len(), 1);
        assert_eq!(
            doc["modules"]["isles-1-14-7"]["backend"]["level"].as_str(),
            Some("info")
        );
        for gone in ["edition", "legacy_game_dir", "legacy_public_port", "somethingElse"] {
            assert!(doc.get(gone).is_none(), "{gone}");
        }
        assert!(doc["modules"].get("revived-1-4-4").is_none());
        assert!(doc["modules"].get("external").is_none());
        for key in [
            "edition",
            "legacyGameDir",
            "legacyPublicPort",
            "modules.compiled: revived-1-4-4",
            "modules.external: isles-1-12-0",
            "modules.revived-1-4-4",
            "somethingElse",
            "performance.turbo",
        ] {
            assert!(
                converted.dropped.iter().any(|d| d == key),
                "{key}: {:?}",
                converted.dropped
            );
        }
        assert!(!converted.was_legacy_only);
    }

    #[test]
    fn a_legacy_only_server_is_flagged_and_bad_files_are_refused() {
        let converted = convert(r#"{"version":1,"edition":"legacy","legacyGameDir":"/srv/dauntless"}"#).unwrap();
        assert!(converted.was_legacy_only);
        let doc: DocumentMut = converted.toml.parse().unwrap();
        assert!(doc.get("edition").is_none() && doc.get("legacy_game_dir").is_none());
        assert!(convert("[1, 2]").is_err());
        assert!(convert("{").is_err());
        assert!(convert(r#"{"name": {"nested": [[1]]}}"#).is_err());
    }
}
