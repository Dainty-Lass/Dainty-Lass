// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `[env.<service>]` (RD24 phase 2, docs/config-toml-plan.md): variables appended to a service's env file for a
//! tunable host.toml has no key of its own. A key host.toml already sets, a secret, a port or a bind address is
//! refused, so the passthrough can't turn a public server's safety settings off.
//!
//! ```toml
//! [env.gateway]
//! GATEWAY_RATE_WS = "40,24"
//! GATEWAY_MAX_CONNECTIONS = 4096
//! ```

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use super::{EnvEntries, EnvFiles, SECRET_KEYS};
use crate::error::HostError;

/// The env files `setup` writes, in [`EnvFiles::files`]'s order.
pub const SERVICES: [&str; 6] = ["backend", "director", "content", "gateway", "admin", "allowlist"];

/// The most variables one `[env.<service>]` table may hold.
pub const MAX_KEYS: usize = 64;
const MAX_VALUE_BYTES: usize = 4096;

/// Never through the passthrough, on any service: what decides who may connect or what a public server allows.
const DENIED: [&str; 10] = [
    "DAINTY_SERVER_MODE",
    "APPROVED_EXECUTABLE_SHA256",
    "DAINTY_XP_MULTIPLIER",
    "DAINTY_REWARD_MULTIPLIER",
    "REGISTRATION_MODE",
    "MONGODB_URI",
    "NODE_ENV",
    "PORT",
    "BIND_HOST",
    "MY_IP",
];
const DENIED_PREFIXES: [&str; 2] = ["AUTH_", "DAINTY_DB_"];
const DENIED_SUFFIXES: [&str; 8] = [
    "_SECRET",
    "_KEY",
    "_KEYS",
    "_TOKEN",
    "_PASSWORD",
    "_B64",
    "_PORT",
    "_BIND",
];

/// `[env]`, by service, each key to its value as written (a string, number or bool), so a rewrite keeps its type.
pub type EnvPassthrough = BTreeMap<String, BTreeMap<String, Value>>;

/// The env file's text for a value [`from_json`] accepted.
#[must_use]
pub fn text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn bad(text: impl Into<String>) -> HostError {
    HostError::message(text)
}

fn is_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    (1..=64).contains(&bytes.len())
        && bytes[0].is_ascii_uppercase()
        && bytes
            .iter()
            .all(|&b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

/// Why `key` can't be set through the passthrough, if it can't.
#[must_use]
pub fn denied(key: &str) -> Option<&'static str> {
    if SECRET_KEYS.contains(&key) {
        return Some("it is a secret, kept in secrets.env");
    }
    if DENIED.contains(&key)
        || DENIED_PREFIXES.iter().any(|p| key.starts_with(p))
        || DENIED_SUFFIXES.iter().any(|s| key.ends_with(s))
    {
        return Some("host.toml's own settings decide it");
    }
    None
}

/// `env` in host.json's form (the same shape as the TOML), checked.
pub fn from_json(value: &Value) -> Result<EnvPassthrough, HostError> {
    let object = value
        .as_object()
        .ok_or_else(|| bad("env must hold [env.<service>] tables"))?;
    let mut out = BTreeMap::new();
    for (service, table) in object {
        if !SERVICES.contains(&service.as_str()) {
            return Err(bad(format!(
                "env.{service}: there is no {service} service (the services are {})",
                SERVICES.join(", ")
            )));
        }
        let table = table.as_object().filter(|t| t.len() <= MAX_KEYS).ok_or_else(|| {
            bad(format!(
                "env.{service} must be a table of at most {MAX_KEYS} KEY = value lines"
            ))
        })?;
        let mut entries = BTreeMap::new();
        for (key, value) in table {
            if !is_key(key) {
                return Err(bad(format!(
                    "env.{service}.{key}: a variable's name is A-Z, 0-9 and _, starting with a letter"
                )));
            }
            if let Some(why) = denied(key) {
                return Err(bad(format!("env.{service}.{key} can't be set here: {why}")));
            }
            let text = match value {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => {
                    return Err(bad(format!(
                        "env.{service}.{key} must be a string, a number or true/false"
                    )));
                }
            };
            if text.len() > MAX_VALUE_BYTES || text.chars().any(char::is_control) {
                return Err(bad(format!(
                    "env.{service}.{key} must be one line of at most {MAX_VALUE_BYTES} bytes"
                )));
            }
            entries.insert(key.clone(), value.clone());
        }
        if !entries.is_empty() {
            out.insert(service.clone(), entries);
        }
    }
    Ok(out)
}

/// host.json's form: what [`from_json`] reads.
#[must_use]
pub fn to_json(env: &EnvPassthrough) -> Value {
    let mut out = Map::new();
    for (service, entries) in env {
        let table = entries.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        out.insert(service.clone(), Value::Object(table));
    }
    Value::Object(out)
}

fn file_mut<'a>(files: &'a mut EnvFiles, service: &str) -> Option<&'a mut EnvEntries> {
    match service {
        "backend" => Some(&mut files.backend),
        "director" => Some(&mut files.director),
        "content" => Some(&mut files.content),
        "gateway" => Some(&mut files.gateway),
        "admin" => Some(&mut files.admin),
        "allowlist" => files.allowlist.as_mut(),
        _ => None,
    }
}

/// Appends each passthrough to its service's file. A key the file already has stays as setup wrote it, and the
/// returned lines say why each skipped one was skipped (`setup` refuses to write while there are any).
pub fn apply(files: &mut EnvFiles, env: &EnvPassthrough) -> Vec<String> {
    let mut refused = Vec::new();
    for (service, entries) in env {
        let Some(file) = file_mut(files, service) else {
            refused.push(format!(
                "env.{service}: this server writes no {service}.env (the allowlist helper runs only on a public server)"
            ));
            continue;
        };
        for (key, value) in entries {
            if file.iter().any(|(k, _)| k == key) {
                refused.push(format!(
                    "env.{service}.{key}: setup already writes {key} from host.toml's own settings; set it there"
                ));
            } else {
                file.push((key.clone(), text(value)));
            }
        }
    }
    refused
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use super::*;
    use serde_json::json;

    fn files() -> EnvFiles {
        let one = |k: &str| vec![(k.to_owned(), "1".to_owned())];
        EnvFiles {
            backend: one("LOG_LEVEL"),
            director: one("LOG_LEVEL"),
            content: one("CONTENT_NEWS_FILE"),
            gateway: one("GATEWAY_TLS"),
            admin: one("DAINTY_MAX_HUNT_SERVERS"),
            allowlist: None,
            discord: None,
            passthrough_refused: Vec::new(),
        }
    }

    #[test]
    fn values_become_env_text() {
        let env = from_json(&json!({
            "gateway": { "GATEWAY_RATE_WS": "40,24", "GATEWAY_MAX_CONNECTIONS": 4096 },
            "backend": { "REALTIME_XMPP_CAPTURE": true },
            "content": {}
        }))
        .unwrap();
        assert_eq!(text(&env["gateway"]["GATEWAY_MAX_CONNECTIONS"]), "4096");
        assert_eq!(text(&env["backend"]["REALTIME_XMPP_CAPTURE"]), "true");
        assert!(!env.contains_key("content"));
        assert_eq!(from_json(&to_json(&env)).unwrap(), env);
    }

    #[test]
    fn secrets_ports_and_safety_keys_are_refused() {
        for key in [
            "GATEWAY_SECRET",
            "DASHBOARD_INGEST_KEY",
            "GATEWAY_PORT",
            "GATEWAY_BIND",
            "AUTH_MODE",
            "DAINTY_SERVER_MODE",
            "APPROVED_EXECUTABLE_SHA256",
            "PORT",
            "DAINTY_DB_PASSWORD",
        ] {
            let err = from_json(&json!({ "gateway": { key: "x" } })).unwrap_err().to_string();
            assert!(
                err.contains(&format!("env.gateway.{key} can't be set here")),
                "{key}: {err}"
            );
        }
    }

    #[test]
    fn bad_names_shapes_and_values_are_refused() {
        for (doc, says) in [
            (json!({ "game": { "A": "1" } }), "there is no game service"),
            (json!({ "gateway": { "lower_case": "1" } }), "A-Z, 0-9 and _"),
            (
                json!({ "gateway": { "GATEWAY_X": ["a"] } }),
                "a string, a number or true/false",
            ),
            (json!({ "gateway": { "GATEWAY_X": "a\nB=c" } }), "one line"),
            (json!({ "gateway": "GATEWAY_X=1" }), "must be a table"),
        ] {
            let err = from_json(&doc).unwrap_err().to_string();
            assert!(err.contains(says), "{doc}: {err}");
        }
    }

    #[test]
    fn apply_appends_and_never_overrides_what_setup_wrote() {
        let env = from_json(&json!({
            "gateway": { "GATEWAY_RATE_WS": "40,24", "GATEWAY_TLS": "0" },
            "allowlist": { "ALLOWLIST_TTL_SECONDS": 60 }
        }))
        .unwrap();
        let mut f = files();
        let refused = apply(&mut f, &env);
        assert_eq!(
            f.gateway,
            [
                ("GATEWAY_TLS".into(), "1".into()),
                ("GATEWAY_RATE_WS".into(), "40,24".into())
            ]
        );
        assert_eq!(refused.len(), 2);
        assert!(refused[0].contains("writes no allowlist.env"));
        assert!(refused[1].contains("env.gateway.GATEWAY_TLS: setup already writes"));
    }
}
