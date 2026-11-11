// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! A Node kit's `host.json` as `host.toml` (spec §11.2): known keys renamed and in host.toml's order, retired
//! keys and modules left out, `modules` carried over as it is.

use std::fmt::Write as _;

use serde_json::{Map, Value};

use crate::data::Reforged;

/// `host.json` converted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Converted {
    /// The new `host.toml`, header first.
    pub toml: String,
    /// What was left out, by JSON name (`<table>.<key>` inside a known table).
    pub dropped: Vec<String>,
    /// `edition` was `"legacy"`: the server ran only 1.4.4.
    pub legacy_only: bool,
}

fn bare_key(key: &str) -> bool {
    !key.is_empty() && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn key(name: &str) -> String {
    if bare_key(name) { name.to_owned() } else { string(name) }
}

fn path(parts: &[String]) -> String {
    parts.iter().map(|p| key(p)).collect::<Vec<_>>().join(".")
}

fn string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04X}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A single value or an inline array; `name` is the dotted key for the error.
fn value(name: &str, v: &Value) -> Result<String, String> {
    match v {
        Value::String(s) => Ok(string(s)),
        Value::Bool(b) => Ok(b.to_string()),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(i.to_string())
            } else if n.is_f64()
                && let Some(f) = n.as_f64().filter(|f| f.is_finite())
            {
                // Debug keeps the `.0` and writes exponents the way TOML reads them
                Ok(format!("{f:?}"))
            } else {
                Err(format!("{name}: {n} is not a number host.toml can hold"))
            }
        }
        Value::Array(items) => {
            let items = items
                .iter()
                .map(|item| match item {
                    Value::Array(_) | Value::Object(_) | Value::Null => Err(format!("{name} must be a single value")),
                    other => value(name, other),
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(format!("[{}]", items.join(", ")))
        }
        Value::Null | Value::Object(_) => Err(format!("{name} must be a single value")),
    }
}

fn is_table_list(v: &Value) -> bool {
    matches!(v, Value::Array(items) if !items.is_empty() && items.iter().all(Value::is_object))
}

/// Writes `object` as the table at `at`, freely: objects are tables, nonempty lists of objects arrays of tables.
fn free_table(out: &mut String, at: &[String], object: &Map<String, Value>, header: Header) -> Result<(), String> {
    let nested = |v: &Value| v.is_object() || is_table_list(v);
    let direct: Vec<(&String, &Value)> = object.iter().filter(|(_, v)| !v.is_null() && !nested(v)).collect();
    // A table holding only tables gets no header of its own
    let only_tables = direct.is_empty() && object.values().any(nested);
    match header {
        Header::Table if !only_tables => {
            let _ = write!(out, "\n[{}]\n", path(at));
        }
        Header::Element => {
            let _ = write!(out, "\n[[{}]]\n", path(at));
        }
        Header::Table => {}
    }
    for (k, v) in direct {
        let name = path(&[at, std::slice::from_ref(k)].concat());
        let _ = writeln!(out, "{} = {}", key(k), value(&name, v)?);
    }
    for (k, v) in object {
        let inner = [at, std::slice::from_ref(k)].concat();
        match v {
            Value::Object(o) => free_table(out, &inner, o, Header::Table)?,
            Value::Array(items) if is_table_list(v) => {
                for item in items {
                    if let Value::Object(o) = item {
                        free_table(out, &inner, o, Header::Element)?;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Header {
    Table,
    Element,
}

fn retired_id(entry: &Value) -> Option<&str> {
    entry.as_str().or_else(|| entry.get("id").and_then(Value::as_str))
}

/// Takes the retired modules out of `modules`, noting each in `dropped`.
fn prune_modules(modules: &mut Map<String, Value>, retired: &[String], dropped: &mut Vec<String>) {
    for (list, label) in [("compiled", "modules.compiled"), ("external", "modules.external")] {
        let Some(Value::Array(items)) = modules.get_mut(list) else {
            continue;
        };
        items.retain(|entry| match retired_id(entry) {
            Some(id) if retired.iter().any(|r| r == id) => {
                dropped.push(format!("{label}: {id}"));
                false
            }
            _ => true,
        });
        if items.is_empty() {
            modules.remove(list);
        }
    }
    for id in retired {
        if modules.remove(id).is_some() {
            dropped.push(format!("modules.{id}"));
        }
    }
}

/// Converts `host.json`'s text.
pub fn convert(text: &str, data: &Reforged) -> Result<Converted, String> {
    let parsed: Value = serde_json::from_str(text).map_err(|e| format!("host.json is not valid JSON: {e}"))?;
    let Value::Object(mut json) = parsed else {
        return Err("host.json must hold a JSON object".into());
    };
    let legacy_only = json.get("edition").and_then(Value::as_str) == Some("legacy");
    let mut dropped = Vec::new();
    for retired in &data.retired_keys {
        if json.remove(retired).is_some() {
            dropped.push(retired.clone());
        }
    }
    let unknown: Vec<String> = json
        .keys()
        .filter(|k| !data.top.iter().any(|(j, _)| j == *k))
        .cloned()
        .collect();
    for k in unknown {
        json.remove(&k);
        dropped.push(k);
    }

    let mut out = data.header.clone();
    let tables = |json_name: &str| json_name == "modules" || data.keys.contains_key(json_name);
    for (json_name, toml_name) in &data.top {
        let Some(v) = json.get(json_name) else {
            continue;
        };
        if v.is_null() || tables(json_name) {
            continue;
        }
        let _ = writeln!(out, "{} = {}", key(toml_name), value(toml_name, v)?);
    }

    for (json_name, toml_name) in &data.top {
        let Some(keys) = data.keys.get(json_name) else {
            continue;
        };
        let Some(v) = json.get(json_name) else {
            continue;
        };
        if v.is_null() {
            continue;
        }
        let Value::Object(table) = v else {
            return Err(format!("{toml_name} must be a table"));
        };
        let _ = write!(out, "\n[{}]\n", key(toml_name));
        let mut tables_after = Vec::new();
        for k in table.keys() {
            if !keys.iter().any(|(j, _)| j == k) {
                dropped.push(format!("{toml_name}.{k}"));
            }
        }
        for (jk, tk) in keys {
            let Some(v) = table.get(jk) else {
                continue;
            };
            if v.is_null() {
                continue;
            }
            if is_table_list(v) {
                tables_after.push((tk, v));
                continue;
            }
            let name = format!("{toml_name}.{tk}");
            let _ = writeln!(out, "{} = {}", key(tk), value(&name, v)?);
        }
        // network.nodes and the like: an array of tables, last in its table
        for (tk, v) in tables_after {
            if let Value::Array(items) = v {
                for item in items {
                    if let Value::Object(o) = item {
                        free_table(&mut out, &[toml_name.clone(), tk.clone()], o, Header::Element)?;
                    }
                }
            }
        }
    }

    if let Some(modules) = json.get_mut("modules") {
        match modules {
            Value::Null => {}
            Value::Object(m) => {
                prune_modules(m, &data.retired_modules, &mut dropped);
                let toml_name = data
                    .top
                    .iter()
                    .find(|(j, _)| j == "modules")
                    .map_or("modules", |(_, t)| t.as_str());
                free_table(&mut out, &[toml_name.to_owned()], m, Header::Table)?;
            }
            _ => return Err("modules must be a table".into()),
        }
    }
    Ok(Converted {
        toml: out,
        dropped,
        legacy_only,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data;

    fn convert_ok(text: &str) -> Converted {
        let out = convert(text, data::reforged().unwrap()).unwrap();
        toml::from_str::<toml::Table>(&out.toml).unwrap_or_else(|e| panic!("{e}\n{}", out.toml));
        out
    }

    #[test]
    fn converts_a_node_kit() {
        let out = convert_ok(
            r#"{
              "version": 3, "mode": "public", "edition": "legacy", "legacyGameDir": "C:/old",
              "gameDir": null, "name": "Spud's \"isle\"", "maxHuntServers": 4, "surprise": true,
              "mongo": {"uri": "mongodb://127.0.0.1:27017", "mongod": null, "extra": 1},
              "multipliers": {"xp": 1.5, "rewards": 2.0},
              "performance": {"serverFps": 30, "memoryGuard": true},
              "network": {"nodes": [{"id": "a", "url": "https://a"}, {"id": "b"}], "node": "a", "leader": true},
              "switches": {"rateLimitPerMinute": 120},
              "modules": {
                "compiled": ["revived-1-4-4", "isles-1-14-7"],
                "external": [{"id": "isles-1-12-0", "path": "x"}],
                "revived-1-4-4": {"port": 1},
                "isles-1-14-7": {"ports": {"backend": 41000}, "flags": ["a", "b"]}
              }
            }"#,
        );
        assert!(out.legacy_only);
        assert_eq!(
            out.dropped,
            [
                "edition",
                "legacyGameDir",
                "surprise",
                "mongo.extra",
                "modules.compiled: revived-1-4-4",
                "modules.external: isles-1-12-0",
                "modules.revived-1-4-4"
            ]
        );
        let header = &data::reforged().unwrap().header;
        let body = out.toml.strip_prefix(header.as_str()).unwrap();
        assert_eq!(
            body,
            "version = 3\nmode = \"public\"\nname = \"Spud's \\\"isle\\\"\"\nmax_hunt_servers = 4\n\
             \n[mongo]\nuri = \"mongodb://127.0.0.1:27017\"\n\
             \n[multipliers]\nxp = 1.5\nrewards = 2.0\n\
             \n[performance]\nserver_fps = 30\nmemory_guard = true\n\
             \n[switches]\nrate_limit_per_minute = 120\n\
             \n[network]\nnode = \"a\"\nleader = true\n\
             \n[[network.nodes]]\nid = \"a\"\nurl = \"https://a\"\n\n[[network.nodes]]\nid = \"b\"\n\
             \n[modules]\ncompiled = [\"isles-1-14-7\"]\n\
             \n[modules.isles-1-14-7]\nflags = [\"a\", \"b\"]\n\n[modules.isles-1-14-7.ports]\nbackend = 41000\n"
        );
    }

    #[test]
    fn a_table_of_tables_has_no_header() {
        let out = convert_ok(r#"{"modules": {"isles-1-14-7": {"ports": {"backend": 1}}}}"#);
        assert!(
            out.toml.ends_with("\n[modules.isles-1-14-7.ports]\nbackend = 1\n"),
            "{}",
            out.toml
        );
        assert!(!out.toml.contains("[modules]\n"));
    }

    #[test]
    fn refuses_what_toml_cannot_hold() {
        let data = data::reforged().unwrap();
        let err = |text: &str| convert(text, data).unwrap_err();
        assert!(err("{").starts_with("host.json is not valid JSON: "));
        assert_eq!(err("[]"), "host.json must hold a JSON object");
        assert_eq!(err(r#"{"name": {"a": 1}}"#), "name must be a single value");
        assert_eq!(err(r#"{"name": [null]}"#), "name must be a single value");
        assert_eq!(err(r#"{"name": [[1]]}"#), "name must be a single value");
        assert_eq!(
            err(r#"{"mongo": {"uri": {"a": 1}}}"#),
            "mongo.uri must be a single value"
        );
        assert_eq!(
            err(r#"{"version": 18446744073709551615}"#),
            "version: 18446744073709551615 is not a number host.toml can hold"
        );
        assert_eq!(
            err(r#"{"modules": {"x": [1, {"a": 2}]}}"#),
            "modules.x must be a single value"
        );
    }

    #[test]
    fn empty_host_json_is_the_header() {
        let out = convert_ok("{}");
        assert_eq!(out.toml, data::reforged().unwrap().header);
        assert!(out.dropped.is_empty() && !out.legacy_only);
    }
}
