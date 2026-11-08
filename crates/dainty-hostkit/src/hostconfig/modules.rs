// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `[modules]` and `[network]` (DF3, DN2): which game modules and plugins the server loads, and who it is
//! in a network. Both are new to host.toml, so a file without them runs today's server: every compiled-in
//! module (this build has none yet), no external one, a node outside any network.
//!
//! ```toml
//! [modules]
//! compiled = ["isles-1-14-7"]          # left out: every compiled-in module
//!
//! [[modules.external]]                 # an executable, pinned by its SHA-256 (DD5)
//! id = "rustic-example"
//! path = "modules/rustic-example.exe"
//! sha256 = "<64 lowercase hex>"
//! args = []                            # optional
//! autoload = true                      # optional; false: listed, loaded from the console only
//!
//! [modules.rustic-example.tuning]      # a module's own settings, as it registered them
//! greeting = "hi"
//!
//! [network]
//! node = "oregon-1"
//! region = "oregon"
//!
//! [[network.nodes]]                    # the other nodes of the network (DN3); left out: no peers
//! id = "uk-1"
//! region = "uk"
//! host = "uk.example.org"              # an IP address or a lower-case DNS name
//! port = 443
//! fingerprint = "<64 lowercase hex>"   # optional: the peer's public certificate, listed to the launcher (DN7)
//! ```

use std::collections::{BTreeMap, BTreeSet};

use dainty_module_api::{ModuleId, NodeId, Region, Sha256Hex};
use network_plugin::cluster::{Peer, check_peers};
use serde_json::{Map, Value};

use crate::error::HostError;

/// Keys of `[modules]` that aren't a module's settings, so no module can be named after them.
pub const RESERVED_KEYS: [&str; 2] = ["compiled", "external"];
/// The most `[[modules.external]]` tables.
pub const MAX_EXTERNAL: usize = 32;
/// The most arguments one external module may be given.
pub const MAX_ARGS: usize = 64;
/// The most bytes one argument may have.
pub const MAX_ARG: usize = 1024;
/// The most `[modules.<id>.<section>]` tables one module may have.
pub const MAX_SECTIONS: usize = 32;
/// The most keys one section may hold.
pub const MAX_KEYS: usize = 64;
/// The most bytes one string setting may have.
pub const MAX_VALUE: usize = 4096;

/// One `[[modules.external]]`: a program the server runs as a module.
#[derive(Debug, Clone, PartialEq)]
pub struct ExternalModuleEntry {
    /// The module's id.
    pub id: ModuleId,
    /// As written; relative to the server folder (the one that holds `.workspace/`)
    pub path: String,
    /// The SHA-256 the file must have (DD5).
    pub sha256: Sha256Hex,
    /// The arguments it is started with.
    pub args: Vec<String>,
    /// Loaded when the server starts; otherwise from the console only.
    pub autoload: bool,
}

/// `[modules]`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModulesConfig {
    /// The compiled-in modules to load at start; `None` loads every one this binary has
    pub compiled: Option<Vec<ModuleId>>,
    /// The external modules.
    pub external: Vec<ExternalModuleEntry>,
    /// `[modules.<id>.<section>]`, by module id
    pub settings: BTreeMap<ModuleId, BTreeMap<String, BTreeMap<String, Value>>>,
}

/// `[network]`: this node and the other nodes it links to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetworkConfig {
    /// This node's id.
    pub node: NodeId,
    /// This node's region.
    pub region: Region,
    /// Optional explicit directory leader; when `None`, [`Self::effective_leader`] picks the
    /// lexicographically smallest node id across `node` and `nodes`.
    pub leader: Option<NodeId>,
    /// Optional policy when a client asks to join an instance hosted on another node
    /// (`"refuse"` or `"redirect"`, default `"refuse"` per DN1 / DN8).
    pub wrong_node_policy: Option<network_plugin::ring::WrongNodePolicy>,
    /// The other nodes (`[[network.nodes]]`): the only ones this node answers, and the ones it calls
    pub nodes: Vec<Peer>,
    /// `failover = true`: the nodes elect the directory's leader and replace it when it goes down
    /// (`docs/distributed-mode.md`); `leader` is left out then. Off by default: today's fixed leader
    pub failover: bool,
    /// How often the elected leader tells the others it leads, in ms (1000 when left out)
    pub heartbeat_ms: Option<u64>,
    /// How long followers wait without a heartbeat before electing, in ms: the shortest of a range twice as wide
    /// (5000 when left out)
    pub election_timeout_ms: Option<u64>,
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
    /// Reads `[network]`; fails on a missing or malformed field.
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
        let node: NodeId = name("node")?.parse().map_err(|e| bad(format!("network.node: {e}")))?;
        let region = name("region")?
            .parse()
            .map_err(|e| bad(format!("network.region: {e}")))?;
        let nodes = match m.get("nodes") {
            None => Vec::new(),
            Some(list) => list
                .as_array()
                .ok_or_else(|| bad("network.nodes must be a list of [[network.nodes]] tables"))?
                .iter()
                .map(|item| {
                    serde_json::from_value::<Peer>(item.clone()).map_err(|e| {
                        bad(format!(
                            "[[network.nodes]]: {e} (the settings there are id, region, host, port, fingerprint, link)"
                        ))
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
        };
        check_peers(&node, &nodes).map_err(|e| bad(format!("network.nodes: {e}")))?;
        let leader = match m.get("leader") {
            None => None,
            Some(v) => {
                let s = v
                    .as_str()
                    .ok_or_else(|| bad("network.leader must be a node id (a-z, 0-9 and -)"))?;
                let id: NodeId = s.parse().map_err(|e| bad(format!("network.leader: {e}")))?;
                if id != node && !nodes.iter().any(|peer| peer.id == id) {
                    return Err(bad(format!(
                        "network.leader: {id} is neither network.node ({node}) nor in [[network.nodes]]"
                    )));
                }
                Some(id)
            }
        };
        let flag = |key: &str| -> Result<bool, HostError> {
            match m.get(key) {
                None => Ok(false),
                Some(Value::Bool(b)) => Ok(*b),
                Some(_) => Err(bad(format!("network.{key} must be true or false"))),
            }
        };
        let millis = |key: &str, name: &str| -> Result<Option<u64>, HostError> {
            match m.get(key) {
                None => Ok(None),
                Some(v) => v
                    .as_u64()
                    .filter(|ms| (10..=600_000).contains(ms))
                    .map(Some)
                    .ok_or_else(|| bad(format!("network.{name} must be milliseconds from 10 to 600000"))),
            }
        };
        let failover = flag("failover")?;
        let heartbeat_ms = millis("heartbeatMs", "heartbeat_ms")?;
        let election_timeout_ms = millis("electionTimeoutMs", "election_timeout_ms")?;
        if failover && leader.is_some() {
            return Err(bad(
                "network.leader and network.failover = true can't both be set: with failover the nodes elect the leader",
            ));
        }
        let wrong_node_policy = match m.get("wrongNodePolicy") {
            None => None,
            Some(v) => match v.as_str() {
                Some("refuse") => Some(network_plugin::ring::WrongNodePolicy::Refuse),
                Some("redirect") => Some(network_plugin::ring::WrongNodePolicy::Redirect),
                _ => {
                    return Err(bad("network.wrong_node_policy must be \"refuse\" or \"redirect\""));
                }
            },
        };
        let config = Self {
            node,
            region,
            leader,
            wrong_node_policy,
            nodes,
            failover,
            heartbeat_ms,
            election_timeout_ms,
        };
        if let Some(election) = config.election_config() {
            election.check().map_err(|e| bad(format!("network: {e}")))?;
        }
        Ok(config)
    }

    /// The election this node runs under `failover = true`; none with a fixed leader.
    pub fn election_config(&self) -> Option<network_plugin::ElectionConfig> {
        if !self.failover {
            return None;
        }
        let mut config = network_plugin::ElectionConfig::new(
            self.node.clone(),
            self.nodes.iter().map(|peer| peer.id.clone()).collect(),
        );
        if let Some(ms) = self.heartbeat_ms {
            config.heartbeat = std::time::Duration::from_millis(ms);
        }
        if let Some(ms) = self.election_timeout_ms {
            let low = std::time::Duration::from_millis(ms);
            config.election_timeout = (low, low * 2);
        }
        // A vote that can't come back before the next election starts is wasted
        config.call_timeout = (config.heartbeat * 2).min(config.election_timeout.0 / 2);
        Some(config)
    }

    /// The directory leader for this network: `leader` when set in `[network]`, or the
    /// lexicographically smallest node id across this node and `nodes` (so every node in the same
    /// peer set agrees without an extra setting).
    pub fn effective_leader(&self) -> NodeId {
        if let Some(leader) = &self.leader {
            return leader.clone();
        }
        std::iter::once(&self.node)
            .chain(self.nodes.iter().map(|peer| &peer.id))
            .min_by(|a, b| a.as_str().cmp(b.as_str()))
            .cloned()
            .unwrap_or_else(|| self.node.clone())
    }

    /// The table host.toml writes.
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        out.insert("node".into(), Value::String(self.node.to_string()));
        out.insert("region".into(), Value::String(self.region.to_string()));
        if let Some(leader) = &self.leader {
            out.insert("leader".into(), Value::String(leader.to_string()));
        }
        if self.failover {
            out.insert("failover".into(), Value::Bool(true));
        }
        if let Some(ms) = self.heartbeat_ms {
            out.insert("heartbeatMs".into(), Value::from(ms));
        }
        if let Some(ms) = self.election_timeout_ms {
            out.insert("electionTimeoutMs".into(), Value::from(ms));
        }
        if let Some(policy) = &self.wrong_node_policy {
            let s = match policy {
                network_plugin::ring::WrongNodePolicy::Refuse => "refuse",
                network_plugin::ring::WrongNodePolicy::Redirect => "redirect",
            };
            out.insert("wrongNodePolicy".into(), Value::String(s.to_owned()));
        }
        if !self.nodes.is_empty() {
            let nodes = self.nodes.iter().filter_map(|peer| serde_json::to_value(peer).ok());
            out.insert("nodes".into(), Value::Array(nodes.collect()));
        }
        Value::Object(out)
    }
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

    fn with_modules() -> String {
        format!(
            "{BASE}\n[network]\nnode = \"oregon-1\"\nregion = \"oregon\"\n\n[modules]\ncompiled = [\"isles-1-14-7\"]\n\n\
             [[modules.external]]\nid = \"rustic-example\"\npath = \"modules/rustic-example.exe\"\nsha256 = \"{}\"\n\
             args = [\"--quiet\"]\nautoload = false\n\n[modules.rustic-example.tuning]\ngreeting = \"hi\"\nlevel = 3\n",
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
        // A legacy host.json, which has neither, reads the same
        let json = config.to_json().unwrap();
        assert_eq!(HostConfig::from_json(&json).unwrap().modules, None);
    }

    #[test]
    fn modules_and_network_round_trip_through_host_toml_and_host_json() {
        let config = HostConfig::from_toml(&with_modules()).unwrap();
        let modules = config.modules.clone().unwrap();
        assert_eq!(modules.compiled, Some(vec!["isles-1-14-7".parse().unwrap()]));
        let example = &modules.external[0];
        assert_eq!(
            (
                example.id.as_str(),
                example.path.as_str(),
                example.args.as_slice(),
                example.autoload
            ),
            (
                "rustic-example",
                "modules/rustic-example.exe",
                &["--quiet".to_string()][..],
                false
            )
        );
        assert_eq!(example.sha256.to_string(), digest());
        let tuning = &modules.settings[&"rustic-example".parse::<ModuleId>().unwrap()]["tuning"];
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
            (
                "[network]\nnode = \"oregon-1\"\nregion = \"oregon\"\nleader = \"uk-1\"\n".to_string(),
                "network.leader",
            ),
        ] {
            let error = HostConfig::from_toml(&format!("{BASE}{bad}")).unwrap_err().to_string();
            assert!(error.contains(said), "{bad:?}: {error}");
        }
    }

    #[test]
    fn network_leader_defaults_to_smallest_node_id_and_round_trips_when_set() {
        let public = BASE
            .replace("mode = \"private\"", "mode = \"public\"")
            .replace("100.101.102.103", "203.0.113.10");
        let toml = format!(
            "{public}\n[network]\nnode = \"uk-1\"\nregion = \"uk\"\n\n\
             [[network.nodes]]\nid = \"oregon-1\"\nregion = \"oregon\"\nhost = \"198.51.100.1\"\nport = 443\n"
        );
        let config = HostConfig::from_toml(&toml).unwrap();
        let net = config.network.as_ref().unwrap();
        assert_eq!(net.leader, None);
        assert_eq!(net.effective_leader().as_str(), "oregon-1");

        let explicit = format!(
            "{public}\n[network]\nnode = \"oregon-1\"\nregion = \"oregon\"\nleader = \"uk-1\"\n\n\
             [[network.nodes]]\nid = \"uk-1\"\nregion = \"uk\"\nhost = \"198.51.100.2\"\nport = 443\n"
        );
        let config = HostConfig::from_toml(&explicit).unwrap();
        let net = config.network.as_ref().unwrap();
        assert_eq!(net.leader, Some("uk-1".parse().unwrap()));
        assert_eq!(net.effective_leader().as_str(), "uk-1");
        let round_trip = HostConfig::from_toml(&config.to_toml().unwrap()).unwrap();
        assert_eq!(round_trip.network, config.network);
    }

    #[test]
    fn failover_reads_its_timings_and_a_back_channel_link_and_round_trips() {
        let public = BASE
            .replace("mode = \"private\"", "mode = \"public\"")
            .replace("100.101.102.103", "203.0.113.10");
        let nodes = "[[network.nodes]]\nid = \"uk-1\"\nregion = \"uk\"\nhost = \"198.51.100.2\"\nport = 443\n\
                     link = \"100.64.0.2:443\"\n\n\
                     [[network.nodes]]\nid = \"germany-1\"\nregion = \"germany\"\nhost = \"198.51.100.3\"\nport = 443\n";
        let toml = format!(
            "{public}\n[network]\nnode = \"oregon-1\"\nregion = \"oregon\"\nfailover = true\nheartbeat_ms = 500\n\
             election_timeout_ms = 3000\n\n{nodes}"
        );
        let config = HostConfig::from_toml(&toml).unwrap();
        let net = config.network.as_ref().unwrap();
        assert!(net.failover);
        assert_eq!(net.nodes[0].dial(), ("100.64.0.2", 443));
        assert_eq!(net.nodes[1].dial(), ("198.51.100.3", 443));
        let election = net.election_config().unwrap();
        assert_eq!(election.peers.len(), 2);
        assert_eq!(election.quorum(), 2);
        assert_eq!(election.heartbeat, std::time::Duration::from_millis(500));
        assert_eq!(
            election.election_timeout,
            (std::time::Duration::from_secs(3), std::time::Duration::from_secs(6))
        );
        election.check().unwrap();
        let round_trip = HostConfig::from_toml(&config.to_toml().unwrap()).unwrap();
        assert_eq!(round_trip.network, config.network);

        // Off by default, and refused beside a fixed leader or with timings that can't work
        let plain = format!("{public}\n[network]\nnode = \"oregon-1\"\nregion = \"oregon\"\n\n{nodes}");
        assert_eq!(
            HostConfig::from_toml(&plain)
                .unwrap()
                .network
                .unwrap()
                .election_config(),
            None
        );
        for (bad, said) in [
            ("failover = true\nleader = \"uk-1\"\n", "can't both be set"),
            ("failover = \"yes\"\n", "network.failover must be true or false"),
            ("failover = true\nheartbeat_ms = 4000\n", "three heartbeats"),
            (
                "failover = true\nelection_timeout_ms = 5\n",
                "network.election_timeout_ms",
            ),
        ] {
            let toml = format!("{public}\n[network]\nnode = \"oregon-1\"\nregion = \"oregon\"\n{bad}\n{nodes}");
            let error = HostConfig::from_toml(&toml).unwrap_err().to_string();
            assert!(error.contains(said), "{bad:?}: {error}");
        }
        let link = nodes.replace("100.64.0.2:443", "100.64.0.2");
        let toml = format!("{public}\n[network]\nnode = \"oregon-1\"\nregion = \"oregon\"\n\n{link}");
        let error = HostConfig::from_toml(&toml).unwrap_err().to_string();
        assert!(error.contains("link must be host:port"), "{error}");
    }

    #[test]
    fn history_ship_to_peer_round_trips_and_validates_against_peers() {
        let public = BASE
            .replace("mode = \"private\"", "mode = \"public\"")
            .replace("100.101.102.103", "203.0.113.10");
        let toml = format!(
            "{public}\n[history]\nship_to_peer = \"uk-1\"\n\n[network]\nnode = \"oregon-1\"\nregion = \"oregon\"\n\n\
             [[network.nodes]]\nid = \"uk-1\"\nregion = \"uk\"\nhost = \"198.51.100.2\"\nport = 443\n"
        );
        let config = HostConfig::from_toml(&toml).unwrap();
        assert_eq!(
            config.history.as_ref().and_then(|h| h.ship_to_peer.as_ref()),
            Some(&"uk-1".parse().unwrap())
        );
        let round_trip = HostConfig::from_toml(&config.to_toml().unwrap()).unwrap();
        assert_eq!(round_trip.history, config.history);
        let from_json = HostConfig::from_json(&config.to_json().unwrap()).unwrap();
        assert_eq!(from_json.history, config.history);
        assert!(from_json.to_toml().unwrap().contains("ship_to_peer = \"uk-1\""));

        // Refused without [network] or when naming itself or an unlisted peer
        for (bad_toml, expected) in [
            (
                format!("{public}\n[history]\nship_to_peer = \"uk-1\"\n"),
                "requires [network]",
            ),
            (
                format!(
                    "{public}\n[history]\nship_to_peer = \"oregon-1\"\n\n[network]\nnode = \"oregon-1\"\nregion = \"oregon\"\n\n\
                     [[network.nodes]]\nid = \"uk-1\"\nregion = \"uk\"\nhost = \"198.51.100.2\"\nport = 443\n"
                ),
                "is this node itself",
            ),
            (
                format!(
                    "{public}\n[history]\nship_to_peer = \"de-1\"\n\n[network]\nnode = \"oregon-1\"\nregion = \"oregon\"\n\n\
                     [[network.nodes]]\nid = \"uk-1\"\nregion = \"uk\"\nhost = \"198.51.100.2\"\nport = 443\n"
                ),
                "not listed in [[network.nodes]]",
            ),
        ] {
            let err = HostConfig::from_toml(&bad_toml).unwrap_err().to_string();
            assert!(err.contains(expected), "{err}");
        }
    }
}
