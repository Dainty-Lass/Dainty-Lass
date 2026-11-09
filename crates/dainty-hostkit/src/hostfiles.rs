// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The other files a host writes, in TOML (RD24 phase 3, docs/config-toml-plan.md). `news.toml` is read by content
//! itself, which picks the format by the file's extension. `session-supplies.toml` is turned into the JSON the
//! backend reads, `.workspace/generated/session-supplies.json`, at every setup. An older install's JSON file is
//! converted once and kept beside it as `.bak`.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};
use toml_edit::{DocumentMut, Item};

use crate::error::HostError;
use crate::paths::Paths;

/// The news file a new install starts with.
pub const NEWS_TEMPLATE: &str = "\
# The news the launcher shows, newest first (dainty-host). Content reads this file again within a few seconds
# of a change: no restart. One [[items]] each, with a date, a title and a body; a body over several lines goes
# between triple quotes.
#
# [[items]]
# date = 2026-11-08
# title = \"Welcome\"
# body = \"\"\"
# The server is up.
# Invite your friends.
# \"\"\"
";

const SUPPLIES_HEADER: &str = "\
# Supplies topped up at each sign-in, by account (dainty-host). Run `dainty-host setup` after editing: it turns
# this file into the JSON the backend reads. One [accounts.<username>] table each, an item's catalog id to the
# number the account should hold (0 to 1000000):
#
# [accounts.alice]
# Consumable_Potion = 10
";

fn parse_toml(text: &str) -> Result<DocumentMut, String> {
    text.parse::<DocumentMut>().map_err(|e| format!("not valid TOML: {e}"))
}

fn bad(text: impl Into<String>) -> HostError {
    HostError::message(text)
}

/// `.workspace/config/session-supplies.toml`.
#[must_use]
pub fn supplies_toml(p: &Paths) -> PathBuf {
    p.config.join("session-supplies.toml")
}

/// `.workspace/generated/session-supplies.json`, what `DAINTY_SESSION_SUPPLIES_FILE` names.
#[must_use]
pub fn supplies_json(p: &Paths) -> PathBuf {
    p.workspace.join("generated").join("session-supplies.json")
}

fn legacy(p: &Paths, name: &str) -> PathBuf {
    p.config.join(name)
}

fn read(path: &Path) -> Result<String, HostError> {
    let text = std::fs::read_to_string(path).map_err(|e| HostError::io(path, e))?;
    Ok(text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned())
}

fn write(path: &Path, text: &str) -> Result<(), HostError> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| HostError::io(dir, e))?;
    }
    std::fs::write(path, text).map_err(|e| HostError::io(path, e))
}

/// Writes `toml` from the JSON file `json` once, then renames that to `.bak`; `Ok(true)` when it did.
fn convert_once(json: &Path, toml: &Path, to_toml: fn(&str) -> Result<String, String>) -> Result<bool, HostError> {
    if toml.exists() || !json.is_file() {
        return Ok(false);
    }
    let text = to_toml(&read(json)?).map_err(|e| bad(format!("{}: {e}", json.display())))?;
    write(toml, &text)?;
    let mut bak = json.as_os_str().to_owned();
    bak.push(".bak");
    std::fs::rename(json, &bak).map_err(|e| HostError::io(json, e))?;
    Ok(true)
}

// ------------------------------------------------------------------ news

/// A string as TOML: between triple quotes when it has lines and nothing that would need escaping there.
fn text_value(text: &str) -> String {
    let plain = !text.contains("\"\"\"")
        && !text.contains('\\')
        && !text.ends_with('"')
        && text.chars().all(|c| c == '\n' || !c.is_control());
    if text.contains('\n') && plain {
        format!("\"\"\"\n{text}\"\"\"")
    } else {
        toml_edit::Value::from(text).to_string().trim().to_owned()
    }
}

/// news.json's text as news.toml: `{"items": [...]}` or a bare array, each item's date, title and body kept.
pub fn news_toml(json: &str) -> Result<String, String> {
    let doc: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let items = match &doc {
        Value::Array(items) => items,
        Value::Object(map) => map
            .get("items")
            .and_then(Value::as_array)
            .ok_or("news file must be {\"items\": [...]} or an array")?,
        _ => return Err("news file must be {\"items\": [...]} or an array".to_owned()),
    };
    let mut out = String::from(NEWS_TEMPLATE);
    for item in items {
        let Some(item) = item.as_object() else { continue };
        out.push_str("\n[[items]]\n");
        for key in ["date", "title", "body"] {
            match item.get(key) {
                Some(Value::String(s)) => out.push_str(&format!("{key} = {}\n", text_value(s))),
                Some(Value::Null) | None => {}
                Some(other) => out.push_str(&format!("{key} = {}\n", text_value(&other.to_string()))),
            }
        }
    }
    parse_toml(&out)?;
    Ok(out)
}

/// news.toml, made once: from an older install's news.json, or the commented template.
pub fn ensure_news(p: &Paths) -> Result<Option<String>, HostError> {
    if convert_once(&legacy(p, "news.json"), &p.news_file, news_toml)? {
        return Ok(Some(format!(
            "- news: news.json moved to {} (the old file is news.json.bak)",
            p.news_file.display()
        )));
    }
    if !p.news_file.exists() {
        write(&p.news_file, NEWS_TEMPLATE)?;
    }
    Ok(None)
}

// ------------------------------------------------------------------ session supplies

/// session-supplies.json's text as TOML: `{"accounts": {"<user>": {"<catalog id>": n}}}`.
pub fn supplies_toml_text(json: &str) -> Result<String, String> {
    let doc: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let accounts = doc
        .get("accounts")
        .and_then(Value::as_object)
        .ok_or("expected {\"accounts\": {\"<username>\": {\"<catalog id>\": <quantity>}}}")?;
    let mut table = toml_edit::Table::new();
    table.set_implicit(true);
    for (user, items) in accounts {
        let items = items
            .as_object()
            .ok_or_else(|| format!("account {user}: expected an object of catalog ids and quantities"))?;
        let mut inner = toml_edit::Table::new();
        for (id, quantity) in items {
            let n = quantity
                .as_i64()
                .ok_or_else(|| format!("account {user}: quantity of {id} must be a whole number"))?;
            inner.insert(id, toml_edit::value(n));
        }
        table.insert(user, Item::Table(inner));
    }
    let mut doc = DocumentMut::new();
    doc.insert("accounts", Item::Table(table));
    Ok(format!("{SUPPLIES_HEADER}\n{doc}"))
}

/// session-supplies.toml as the JSON the backend reads; the backend checks the ids and quantities itself.
pub fn supplies_json_text(toml: &str) -> Result<String, String> {
    let doc = parse_toml(toml)?;
    for (key, _) in doc.iter() {
        if key != "accounts" {
            return Err(format!("{key}: the file holds only [accounts.<username>] tables"));
        }
    }
    let mut accounts = Map::new();
    if let Some(item) = doc.get("accounts") {
        let table = item.as_table_like().ok_or("accounts must be a table")?;
        for (user, items) in table.iter() {
            let items = items
                .as_table_like()
                .ok_or_else(|| format!("accounts.{user} must be a table of catalog ids and quantities"))?;
            let mut out = Map::new();
            for (id, quantity) in items.iter() {
                let n = quantity
                    .as_integer()
                    .ok_or_else(|| format!("accounts.{user}.{id} must be a whole number"))?;
                out.insert(id.to_owned(), Value::from(n));
            }
            accounts.insert(user.to_owned(), Value::Object(out));
        }
    }
    let mut root = Map::new();
    root.insert("accounts".to_owned(), Value::Object(accounts));
    serde_json::to_string_pretty(&Value::Object(root)).map_err(|e| e.to_string())
}

/// Converts an older session-supplies.json once, then writes the backend's JSON from the TOML; with neither, removes
/// a stale generated file so no account is topped up.
pub fn render_supplies(p: &Paths) -> Result<Option<String>, HostError> {
    let toml_path = supplies_toml(p);
    let moved = convert_once(&legacy(p, "session-supplies.json"), &toml_path, supplies_toml_text)?;
    let out = supplies_json(p);
    if !toml_path.is_file() {
        if out.exists() {
            std::fs::remove_file(&out).map_err(|e| HostError::io(&out, e))?;
        }
        return Ok(None);
    }
    let json = supplies_json_text(&read(&toml_path)?).map_err(|e| bad(format!("{}: {e}", toml_path.display())))?;
    write(&out, &format!("{json}\n"))?;
    Ok(moved.then(|| {
        format!(
            "- session supplies: session-supplies.json moved to {} (the old file is session-supplies.json.bak)",
            toml_path.display()
        )
    }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use super::*;

    #[test]
    fn news_json_becomes_news_toml_with_multi_line_bodies() {
        let toml = news_toml(
            r#"{"items": [{"date": "2026-11-08", "title": "Up \"again\"", "body": "Line one\nLine two\n"},
                          {"date": "2026-11-01", "title": "Back\\slash", "body": "a\\b\nc"}]}"#,
        )
        .unwrap();
        assert!(
            toml.contains("title = 'Up \"again\"'") || toml.contains("title = \"Up \\\"again\\\"\""),
            "{toml}"
        );
        assert!(toml.contains("body = \"\"\"\nLine one\nLine two\n\"\"\""), "{toml}");
        let doc: DocumentMut = toml.parse().unwrap();
        let items = doc["items"].as_array_of_tables().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items.get(1).unwrap()["body"].as_str(), Some("a\\b\nc"));
        assert!(news_toml("3").is_err());
    }

    #[test]
    fn supplies_round_trip_and_setup_converts_once() {
        let json = r#"{"accounts": {"alice": {"Consumable_Potion": 10}, "bob smith": {"Ammo_X": 0}}}"#;
        let toml = supplies_toml_text(json).unwrap();
        assert!(toml.contains("[accounts.alice]\nConsumable_Potion = 10"), "{toml}");
        let back: Value = serde_json::from_str(&supplies_json_text(&toml).unwrap()).unwrap();
        assert_eq!(back, serde_json::from_str::<Value>(json).unwrap());
        assert!(
            supplies_json_text("[accounts.a]\nX = 1.5\n")
                .unwrap_err()
                .contains("whole number")
        );
        assert!(supplies_json_text("extra = 1\n").is_err());

        let dir = tempfile::tempdir().unwrap();
        let p = Paths::new(dir.path());
        std::fs::create_dir_all(&p.config).unwrap();
        assert_eq!(render_supplies(&p).unwrap(), None);
        assert!(!supplies_json(&p).exists());
        std::fs::write(p.config.join("session-supplies.json"), json).unwrap();
        assert!(render_supplies(&p).unwrap().is_some());
        assert!(p.config.join("session-supplies.json.bak").is_file());
        let written: Value = serde_json::from_str(&std::fs::read_to_string(supplies_json(&p)).unwrap()).unwrap();
        assert_eq!(written["accounts"]["alice"]["Consumable_Potion"], 10);
        assert_eq!(render_supplies(&p).unwrap(), None, "once");
    }

    #[test]
    fn news_is_made_once() {
        let dir = tempfile::tempdir().unwrap();
        let p = Paths::new(dir.path());
        std::fs::create_dir_all(&p.config).unwrap();
        std::fs::write(p.config.join("news.json"), r#"{"items": []}"#).unwrap();
        assert!(ensure_news(&p).unwrap().is_some());
        assert!(p.news_file.ends_with("news.toml") && p.news_file.is_file());
        std::fs::write(&p.news_file, "# mine\n").unwrap();
        assert_eq!(ensure_news(&p).unwrap(), None);
        assert_eq!(std::fs::read_to_string(&p.news_file).unwrap(), "# mine\n");
    }
}
