//! `[modules]` and `[network]` (DF3, DN2): which game modules and plugins the server loads, and who it is
//! in a network. Both are new to host.toml, so a file without them runs today's server: every compiled-in
//! module (this build has none yet), no external one, a node outside any network.
//!
//! ```toml
//! [modules]
//! compiled = ["isles-1-14-7"]          # left out: every compiled-in module
//!
//! [[modules.external]]                 # an executable, pinned by its SHA-256 (DD5)
//! id = "rustic-freeza"
//! path = "modules/rustic-freeza.exe"
//! sha256 = "<64 lowercase hex>"
//! args = []                            # optional
//! autoload = true                      # optional; false: listed, loaded from the console only
//!
//! [modules.rustic-freeza.tuning]       # a module's own settings, as it registered them
//! greeting = "hi"
//!
//! [network]
//! node = "oregon-1"
//! region = "oregon"
//! ```

use std::collections::{BTreeMap, BTreeSet};

use module_api::{ModuleId, NodeId, Region, Sha256Hex};
use serde_json::{Map, Value};

use crate::error::HostError;

/// Keys of `[modules]` that aren't a module's settings, so no module can be named after them.
pub const RESERVED_KEYS: [&str; 2] = ["compiled", "external"];
pub const MAX_EXTERNAL: usize = 32;
pub const MAX_ARGS: usize = 64;
pub const MAX_ARG: usize = 1024;
pub const MAX_SECTIONS: usize = 32;
pub const MAX_KEYS: usize = 64;
pub const MAX_VALUE: usize = 4096;

#[derive(Debug, Clone, PartialEq)]
pub struct ExternalModuleEntry {
    pub id: ModuleId,
    /// As written; relative to the server folder (the one that holds `.workspace/`)
    pub path: String,
    pub sha256: Sha256Hex,
    pub args: Vec<String>,
    pub autoload: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModulesConfig {
    /// The compiled-in modules to load at start; `None` loads every one this binary has
    pub compiled: Option<Vec<ModuleId>>,
    pub external: Vec<ExternalModuleEntry>,
    /// `[modules.<id>.<section>]`, by module id
    pub settings: BTreeMap<ModuleId, BTreeMap<String, BTreeMap<String, Value>>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkConfig {
    pub node: NodeId,
    pub region: Region,
}

fn bad(text: impl Into<String>) -> HostError {
    HostError::message(text)
}

fn module_id(value: &Value, what: &str) -> Result<ModuleId, HostError> {
    let text = value
        .as_str()
        .ok_or_else(|| bad(format!("{what} must be a module id")))?;
    let id: ModuleId = text.parse().map_err(|e| bad(format!("{what}: {e}")))?;
    if RESERVED_KEYS.contains(&id.as_str()) {
        return Err(bad(format!("{what}: {id} can't be a module id")));
    }
    Ok(id)
}

/// `[modules.<id>.<section>]` key names are snake_case, as the module registered them.
fn is_key(text: &str) -> bool {
    let bytes = text.as_bytes();
    (1..=64).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|&b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn setting(value: &Value, path: &str) -> Result<(), HostError> {
    let ok = match value {
        Value::String(s) => s.len() <= MAX_VALUE && !s.contains('\0'),
        Value::Bool(_) | Value::Number(_) => true,
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err(bad(format!("{path} must be a string, a number or true/false")))
    }
}

impl ModulesConfig {
    /// `modules` in host.json's form (the same shape as the TOML), checked.
    pub fn from_json(value: &Value) -> Result<Self, HostError> {
        let object = value
            .as_object()
            .ok_or_else(|| bad("modules must be a table ([modules])"))?;
        let mut config = Self::default();
        let mut listed = BTreeSet::new();
        if let Some(compiled) = object.get("compiled") {
            let list = compiled
                .as_array()
                .ok_or_else(|| bad("modules.compiled must be a list of module ids"))?;
            let mut ids = Vec::new();
            for item in list {
                let id = module_id(item, "modules.compiled")?;
                if !listed.insert(id.clone()) {
                    return Err(bad(format!("modules.compiled lists {id} twice")));
                }
                ids.push(id);
            }
            config.compiled = Some(ids);
        }
        if let Some(external) = object.get("external") {
            let list = external.as_array().filter(|l| l.len() <= MAX_EXTERNAL).ok_or_else(|| {
                bad(format!(
                    "modules.external must be at most {MAX_EXTERNAL} [[modules.external]] tables"
                ))
            })?;
            for item in list {
                let entry = external_entry(item)?;
                if !listed.insert(entry.id.clone()) {
                    return Err(bad(format!("module {} is listed twice in [modules]", entry.id)));
                }
                config.external.push(entry);
            }
        }
        for (key, sections) in object.iter().filter(|(k, _)| !RESERVED_KEYS.contains(&k.as_str())) {
            let id = module_id(&Value::String(key.clone()), "a [modules.<id>] table")?;
            let sections = sections
                .as_object()
                .filter(|s| s.len() <= MAX_SECTIONS)
                .ok_or_else(|| {
                    bad(format!(
                        "modules.{id} must hold at most {MAX_SECTIONS} [modules.{id}.<section>] tables"
                    ))
                })?;
            let mut by_section = BTreeMap::new();
            for (section, values) in sections {
                let path = format!("modules.{id}.{section}");
                let values = values
                    .as_object()
                    .filter(|v| is_key(section) && v.len() <= MAX_KEYS)
                    .ok_or_else(|| {
                        bad(format!(
                            "{path} must be a [{path}] table with a snake_case name and at most {MAX_KEYS} keys"
                        ))
                    })?;
                let mut keys = BTreeMap::new();
                for (key, value) in values {
                    if !is_key(key) {
                        return Err(bad(format!("{path}.{key}: a setting's name is snake_case")));
                    }
                    setting(value, &format!("{path}.{key}"))?;
                    keys.insert(key.clone(), value.clone());
                }
                by_section.insert(section.clone(), keys);
            }
            config.settings.insert(id, by_section);
        }
        Ok(config)
    }

    /// host.json's form: what [`from_json`](Self::from_json) reads.
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        if let Some(compiled) = &self.compiled {
            out.insert(
                "compiled".into(),
                Value::Array(compiled.iter().map(|id| Value::String(id.to_string())).collect()),
            );
        }
        if !self.external.is_empty() {
            let list = self
                .external
                .iter()
                .map(|e| {
                    let mut m = Map::new();
                    m.insert("id".into(), Value::String(e.id.to_string()));
                    m.insert("path".into(), Value::String(e.path.clone()));
                    m.insert("sha256".into(), Value::String(e.sha256.to_string()));
                    if !e.args.is_empty() {
                        m.insert(
                            "args".into(),
                            Value::Array(e.args.iter().cloned().map(Value::String).collect()),
                        );
                    }
                    if !e.autoload {
                        m.insert("autoload".into(), Value::Bool(false));
                    }
                    Value::Object(m)
                })
                .collect();
            out.insert("external".into(), Value::Array(list));
        }
        for (id, sections) in &self.settings {
            let sections = sections
                .iter()
                .map(|(name, keys)| {
                    let keys = keys.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                    (name.clone(), Value::Object(keys))
                })
                .collect();
            out.insert(id.to_string(), Value::Object(sections));
        }
        Value::Object(out)
    }
}

fn external_entry(item: &Value) -> Result<ExternalModuleEntry, HostError> {
    let m = item
        .as_object()
        .ok_or_else(|| bad("each [[modules.external]] must be a table"))?;
    for key in m.keys() {
        if !["id", "path", "sha256", "args", "autoload"].contains(&key.as_str()) {
            return Err(bad(format!(
                "[[modules.external]] has no setting {key} (the settings there are id, path, sha256, args, autoload)"
            )));
        }
    }
    let id = module_id(m.get("id").unwrap_or(&Value::Null), "[[modules.external]] id")?;
    let path = m
        .get("path")
        .and_then(Value::as_str)
        .filter(|p| !p.trim().is_empty() && p.len() <= 1024 && !p.chars().any(char::is_control))
        .ok_or_else(|| {
            bad(format!(
                "[[modules.external]] {id}: path must be the module's executable"
            ))
        })?
        .to_owned();
    let sha256 = m
        .get("sha256")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<Sha256Hex>().ok())
        .ok_or_else(|| {
            bad(format!(
                "[[modules.external]] {id}: sha256 must be the executable's SHA-256, 64 lowercase hex characters"
            ))
        })?;
    let args = match m.get("args") {
        None => Vec::new(),
        Some(Value::Array(list))
            if list.len() <= MAX_ARGS
                && list
                    .iter()
                    .all(|a| a.as_str().is_some_and(|s| s.len() <= MAX_ARG && !s.contains('\0'))) =>
        {
            list.iter().filter_map(Value::as_str).map(str::to_owned).collect()
        }
        Some(_) => {
            return Err(bad(format!(
                "[[modules.external]] {id}: args must be a list of at most {MAX_ARGS} strings"
            )));
        }
    };
    let autoload = match m.get("autoload") {
        None => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => {
            return Err(bad(format!(
                "[[modules.external]] {id}: autoload must be true or false"
            )));
        }
    };
    Ok(ExternalModuleEntry {
        id,
        path,
        sha256,
        args,
        autoload,
    })
}

impl NetworkConfig {
    pub fn from_json(value: &Value) -> Result<Self, HostError> {
        let m = value
            .as_object()
            .ok_or_else(|| bad("network must be a table ([network])"))?;
        let name = |key: &str| -> Result<String, HostError> {
            m.get(key)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| bad(format!("network.{key} must be set: a name of a-z, 0-9 and -")))
        };
        Ok(Self {
            node: name("node")?.parse().map_err(|e| bad(format!("network.node: {e}")))?,
            region: name("region")?
                .parse()
                .map_err(|e| bad(format!("network.region: {e}")))?,
        })
    }

    pub fn to_json(&self) -> Value {
        serde_json::json!({ "node": self.node.to_string(), "region": self.region.to_string() })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::hostconfig::HostConfig;

    const BASE: &str = "version = 1\nmode = \"private\"\nname = \"Friday Hunts\"\nhost = \"100.101.102.103\"\n\
        game_address = \"100.101.102.103\"\nregistration = \"INVITECODE\"\nmax_hunt_servers = 3\n\
        [mongo]\nuri = \"mongodb://127.0.0.1:27017\"\nmanage = false\n";

    fn digest() -> String {
        "ab".repeat(32)
    }

    fn with_modules() -> String {
        format!(
            "{BASE}\n[network]\nnode = \"oregon-1\"\nregion = \"oregon\"\n\n[modules]\ncompiled = [\"isles-1-14-7\"]\n\n\
             [[modules.external]]\nid = \"rustic-freeza\"\npath = \"modules/rustic-freeza.exe\"\nsha256 = \"{}\"\n\
             args = [\"--quiet\"]\nautoload = false\n\n[modules.rustic-freeza.tuning]\ngreeting = \"hi\"\nlevel = 3\n",
            digest()
        )
    }

    #[test]
    fn a_file_without_them_runs_todays_server() {
        let config = HostConfig::from_toml(BASE).unwrap();
        assert_eq!((config.modules.as_ref(), config.network.as_ref()), (None, None));
        let written = config.to_toml().unwrap();
        assert!(
            !written.contains("modules") && !written.contains("[network]"),
            "{written}"
        );
        // A Node kit's host.json, which has neither, reads the same
        let json = config.to_json().unwrap();
        assert_eq!(HostConfig::from_json(&json).unwrap().modules, None);
    }

    #[test]
    fn modules_and_network_round_trip_through_host_toml_and_host_json() {
        let config = HostConfig::from_toml(&with_modules()).unwrap();
        let modules = config.modules.clone().unwrap();
        assert_eq!(modules.compiled, Some(vec!["isles-1-14-7".parse().unwrap()]));
        let freeza = &modules.external[0];
        assert_eq!(
            (
                freeza.id.as_str(),
                freeza.path.as_str(),
                freeza.args.as_slice(),
                freeza.autoload
            ),
            (
                "rustic-freeza",
                "modules/rustic-freeza.exe",
                &["--quiet".to_string()][..],
                false
            )
        );
        assert_eq!(freeza.sha256.to_string(), digest());
        let tuning = &modules.settings[&"rustic-freeza".parse::<ModuleId>().unwrap()]["tuning"];
        assert_eq!(
            (tuning["greeting"].clone(), tuning["level"].clone()),
            (Value::from("hi"), Value::from(3))
        );
        assert_eq!(config.network.as_ref().unwrap().node.as_str(), "oregon-1");

        // Written back as read when nothing changed, and read back the same when it did
        assert_eq!(config.to_toml().unwrap(), with_modules());
        let mut changed = config.clone();
        changed.modules.as_mut().unwrap().external[0].autoload = true;
        let text = changed.to_toml().unwrap();
        assert!(
            text.contains("[[modules.external]]") && !text.contains("autoload"),
            "{text}"
        );
        let back = HostConfig::from_toml(&text).unwrap();
        assert_eq!(
            (back.modules, back.network),
            (changed.modules.clone(), changed.network.clone())
        );
        // A new file (setup from scratch, or a host.json moved over) says what [modules] is
        let fresh = HostConfig::from_json(&changed.to_json().unwrap()).unwrap();
        let text = fresh.to_toml().unwrap();
        assert!(text.contains("# The game modules and plugins to load"), "{text}");
        let back = HostConfig::from_toml(&text).unwrap();
        assert_eq!((back.modules, back.network), (changed.modules, changed.network));
    }

    #[test]
    fn what_modules_holds_is_checked() {
        let digest = digest();
        let external = |rest: &str| format!("[[modules.external]]\nid = \"a\"\npath = \"a\"\n{rest}");
        for (bad, said) in [
            ("[modules]\ncompiled = [\"Isles\"]\n".to_string(), "modules.compiled"),
            ("[modules]\ncompiled = [\"a\", \"a\"]\n".to_string(), "twice"),
            (
                "[modules]\ncompiled = [\"external\"]\n".to_string(),
                "can't be a module id",
            ),
            (external(&format!("sha256 = \"{}\"\n", digest.to_uppercase())), "sha256"),
            ("[[modules.external]]\nid = \"a\"\nsha256 = \"x\"\n".to_string(), "path"),
            (
                external(&format!("sha256 = \"{digest}\"\nurl = \"x\"\n")),
                "no setting url",
            ),
            (
                format!(
                    "[modules]\ncompiled = [\"a\"]\n{}",
                    external(&format!("sha256 = \"{digest}\"\n"))
                ),
                "listed twice",
            ),
            ("[modules.a.Tuning]\nx = 1\n".to_string(), "snake_case"),
            ("[modules.a.tuning]\nx = [1]\n".to_string(), "a string, a number"),
            (
                "[network]\nnode = \"Oregon\"\nregion = \"oregon\"\n".to_string(),
                "network.node",
            ),
            ("[network]\nnode = \"oregon-1\"\n".to_string(), "network.region"),
        ] {
            let error = HostConfig::from_toml(&format!("{BASE}{bad}")).unwrap_err().to_string();
            assert!(error.contains(said), "{bad:?}: {error}");
        }
    }
}
