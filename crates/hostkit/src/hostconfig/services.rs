// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `[services.<name>]` (RD27): a service the core loads as a library, pinned by its SHA-256, in place of the one
//! compiled into this binary. A file without it runs today's composition.
//!
//! ```toml
//! [services.content]
//! library = "plugins/content_plugin.dll"   # relative to the server folder (the one that holds .workspace/)
//! sha256 = "<64 lowercase hex>"
//!
//! [services.example]                       # a service this core doesn't know by name
//! library = "plugins/example.so"
//! sha256 = "<64 lowercase hex>"
//! serves = ["content_files"]               # what it serves as a core plugin; known names have their own
//! ```

use std::collections::BTreeMap;

use module_api::{CoreCapability, Sha256Hex};
use serde_json::{Map, Value};

use crate::error::HostError;

/// The most `[services.<name>]` tables.
pub const MAX_SERVICES: usize = 16;

/// Modules this core composes itself under these names; a library can't take one.
pub const RESERVED_NAMES: [&str; 8] = [
    "admin",
    "allowlist",
    "backend",
    "director",
    "mongod",
    "network",
    "remote",
    "remote-agent",
];

/// The services the core knows by name, with what each serves as a core plugin.
pub const KNOWN: [(&str, &[CoreCapability]); 3] = [
    ("content", &[CoreCapability::ContentFiles]),
    ("chat", &[CoreCapability::Chat]),
    ("gateway", &[CoreCapability::Routes, CoreCapability::UdpPorts]),
];

/// One `[services.<name>]`: a service the core loads as a library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceEntry {
    /// As written; relative to the server folder
    pub library: String,
    /// The SHA-256 the library must have.
    pub sha256: Sha256Hex,
    /// Only for a name not in [`KNOWN`]
    pub serves: Option<Vec<CoreCapability>>,
}

impl ServiceEntry {
    /// What the library must say it serves.
    #[must_use]
    pub fn serves_for(&self, name: &str) -> Vec<CoreCapability> {
        known(name)
            .map(<[CoreCapability]>::to_vec)
            .or_else(|| self.serves.clone())
            .unwrap_or_default()
    }
}

/// What the known service `name` serves as a core plugin; `None` for a name not in [`KNOWN`].
#[must_use]
pub fn known(name: &str) -> Option<&'static [CoreCapability]> {
    KNOWN
        .iter()
        .find(|(known, _)| *known == name)
        .map(|(_, serves)| *serves)
}

/// `[services]`, by service name.
pub type ServicesConfig = BTreeMap<String, ServiceEntry>;

fn bad(text: impl Into<String>) -> HostError {
    HostError::message(text)
}

fn is_name(text: &str) -> bool {
    let bytes = text.as_bytes();
    (1..=32).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|&b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// `services` in host.json's form (the same shape as the TOML), checked.
pub fn from_json(value: &Value) -> Result<ServicesConfig, HostError> {
    let object = value.as_object().filter(|o| o.len() <= MAX_SERVICES).ok_or_else(|| {
        bad(format!(
            "services must hold at most {MAX_SERVICES} [services.<name>] tables"
        ))
    })?;
    let mut out = BTreeMap::new();
    for (name, item) in object {
        if !is_name(name) {
            return Err(bad(format!(
                "services.{name}: a service's name is a-z, 0-9 and -, starting with a letter"
            )));
        }
        if RESERVED_NAMES.contains(&name.as_str()) {
            return Err(bad(format!(
                "services.{name}: this core runs {name} itself, so it can't be a library"
            )));
        }
        out.insert(name.clone(), entry(name, item)?);
    }
    Ok(out)
}

fn entry(name: &str, item: &Value) -> Result<ServiceEntry, HostError> {
    let m = item
        .as_object()
        .ok_or_else(|| bad(format!("services.{name} must be a table ([services.{name}])")))?;
    for key in m.keys() {
        if !["library", "sha256", "serves"].contains(&key.as_str()) {
            return Err(bad(format!(
                "services.{name} has no setting {key} (the settings there are library, sha256, serves)"
            )));
        }
    }
    let library = m
        .get("library")
        .and_then(Value::as_str)
        .filter(|p| !p.trim().is_empty() && p.len() <= 1024 && !p.chars().any(char::is_control))
        .ok_or_else(|| bad(format!("services.{name}.library must be the library's path")))?
        .to_owned();
    let sha256 = m
        .get("sha256")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<Sha256Hex>().ok())
        .ok_or_else(|| {
            bad(format!(
                "services.{name}.sha256 must be the library's SHA-256, 64 lowercase hex characters"
            ))
        })?;
    let serves = match m.get("serves") {
        None => None,
        Some(_) if known(name).is_some() => {
            return Err(bad(format!(
                "services.{name}.serves: the core knows what {name} serves; leave it out"
            )));
        }
        Some(list) => Some(
            serde_json::from_value::<Vec<CoreCapability>>(list.clone())
                .map_err(|e| bad(format!("services.{name}.serves must be a list of capabilities: {e}")))?,
        ),
    };
    Ok(ServiceEntry {
        library,
        sha256,
        serves,
    })
}

/// host.json's form: what [`from_json`] reads.
#[must_use]
pub fn to_json(services: &ServicesConfig) -> Value {
    let mut out = Map::new();
    for (name, entry) in services {
        let mut m = Map::new();
        m.insert("library".into(), Value::String(entry.library.clone()));
        m.insert("sha256".into(), Value::String(entry.sha256.to_string()));
        if let Some(serves) = &entry.serves {
            m.insert(
                "serves".into(),
                serde_json::to_value(serves).unwrap_or_else(|_| Value::Array(Vec::new())),
            );
        }
        out.insert(name.clone(), Value::Object(m));
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use super::*;
    use crate::hostconfig::HostConfig;

    const BASE: &str = "version = 1\nmode = \"private\"\nname = \"Friday Hunts\"\nhost = \"100.101.102.103\"\n\
        game_address = \"100.101.102.103\"\nregistration = \"INVITECODE\"\nmax_hunt_servers = 3\n\
        [mongo]\nuri = \"mongodb://127.0.0.1:27017\"\nmanage = false\n";

    fn digest() -> String {
        "ab".repeat(32)
    }

    fn with_services() -> String {
        format!(
            "{BASE}\n[services.content]\nlibrary = \"plugins/content_plugin.dll\"\nsha256 = \"{d}\"\n\n\
             [services.echo]\nlibrary = \"plugins/echo.so\"\nsha256 = \"{d}\"\nserves = [\"content_files\"]\n",
            d = digest()
        )
    }

    #[test]
    fn a_file_without_them_composes_as_today() {
        let config = HostConfig::from_toml(BASE).unwrap();
        assert_eq!(config.services, None);
        assert!(!config.to_toml().unwrap().contains("services"));
        assert_eq!(
            HostConfig::from_json(&config.to_json().unwrap()).unwrap().services,
            None
        );
    }

    #[test]
    fn services_round_trip_through_host_toml_and_host_json() {
        let config = HostConfig::from_toml(&with_services()).unwrap();
        let services = config.services.clone().unwrap();
        let content = &services["content"];
        assert_eq!(
            (
                content.library.as_str(),
                content.sha256.to_string(),
                content.serves.clone()
            ),
            ("plugins/content_plugin.dll", digest(), None)
        );
        assert_eq!(content.serves_for("content"), [CoreCapability::ContentFiles]);
        assert_eq!(services["echo"].serves_for("echo"), [CoreCapability::ContentFiles]);

        assert_eq!(config.to_toml().unwrap(), with_services());
        let mut changed = config.clone();
        changed.services.as_mut().unwrap().remove("echo");
        let text = changed.to_toml().unwrap();
        assert!(text.contains("[services.content]") && !text.contains("echo"), "{text}");
        assert_eq!(HostConfig::from_toml(&text).unwrap().services, changed.services);

        let fresh = HostConfig::from_json(&config.to_json().unwrap()).unwrap();
        let text = fresh.to_toml().unwrap();
        assert!(text.contains("# Services the core loads as libraries"), "{text}");
        assert_eq!(HostConfig::from_toml(&text).unwrap().services, config.services);
    }

    #[test]
    fn setup_keeps_them() {
        let previous = HostConfig::from_toml(&with_services()).unwrap();
        let rewritten = HostConfig::from_toml(BASE).unwrap().written_over(Some(&previous));
        assert_eq!(rewritten.services, previous.services);
    }

    #[test]
    fn what_services_holds_is_checked() {
        let d = digest();
        for (bad, said) in [
            (
                format!(
                    "[services.content]\nlibrary = \"a.dll\"\nsha256 = \"{}\"\n",
                    d.to_uppercase()
                ),
                "services.content.sha256",
            ),
            (
                "[services.content]\nlibrary = \"a.dll\"\nsha256 = \"abc\"\n".to_string(),
                "sha256",
            ),
            (
                format!("[services.content]\nsha256 = \"{d}\"\n"),
                "services.content.library",
            ),
            (
                format!("[services.content]\nlibrary = \"a.dll\"\nsha256 = \"{d}\"\npath = \"x\"\n"),
                "no setting path",
            ),
            (
                format!("[services.Content]\nlibrary = \"a.dll\"\nsha256 = \"{d}\"\n"),
                "a service's name",
            ),
            (
                format!("[services.admin]\nlibrary = \"a.dll\"\nsha256 = \"{d}\"\n"),
                "runs admin itself",
            ),
            (
                format!("[services.chat]\nlibrary = \"a.dll\"\nsha256 = \"{d}\"\nserves = [\"chat\"]\n"),
                "leave it out",
            ),
            (
                format!("[services.x]\nlibrary = \"a.dll\"\nsha256 = \"{d}\"\nserves = [\"everything\"]\n"),
                "services.x.serves",
            ),
            (
                "[services]\ncontent = 1\n".to_string(),
                "services.content must be a table",
            ),
        ] {
            let error = HostConfig::from_toml(&format!("{BASE}{bad}")).unwrap_err().to_string();
            assert!(error.contains(said), "{bad:?}: {error}");
        }
    }
}
