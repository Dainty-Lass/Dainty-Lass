// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

pub mod guard;
pub mod modules;
pub mod yaml;

pub use guard::GuardConfig;
pub use modules::{ExternalModuleEntry, ModulesConfig, NetworkConfig};

use std::path::{Path, PathBuf};

use serde_json::{Map, Number, Value};
use service_kit::{js, jsjson};

use crate::envfile::{EnvDocument, EnvMap, format_env, parse_env};
use crate::error::HostError;
use crate::paths::{MAX_HUNTS, PINNED_EXE_SHA256, PORTS, Paths, game_exe_relative, game_port_range};

pub const DATABASE_FILE: &str = "dainty-lass.db";
pub const DEFAULT_LINUX: LinuxOptions = LinuxOptions {
    wine: None,
    proton: None,
    xvfb: true,
    firewall_interface: None,
};
pub const DEFAULT_PERFORMANCE: PerformanceOptions = PerformanceOptions {
    server_fps: 30,
    idle_fps: 10,
    training_on_demand: true,
    training_idle_minutes: 5,
    verbose_game_logs: false,
    memory_guard: true,
    min_free_memory_mb: 1024,
    hunt_memory_mb: 1536,
};
pub const QUIET_GAME_LOG_CMDS: &str = "LogPhoenixCharacter Warning, LogOnline Warning, LogLoad Off";
pub const MAX_MULTIPLIER: f64 = 100.0;
pub const DEFAULT_MONGO_URI: &str = "mongodb://127.0.0.1:27017/?replicaSet=rs0";
pub const RESERVED_PORTS: [u16; 8] = [
    PORTS.backend,
    PORTS.director,
    PORTS.content,
    PORTS.allowlist,
    PORTS.relay,
    PORTS.mongo,
    61003,
    61004,
];
/// `edition` is read only to refuse the 1.4.4 ones; `reborn` is dropped when the file is next written
pub const EDITION_REMOVED: &str = "edition \"legacy\" and \"both\" ran Dauntless Revived's 1.4.4, which this server no longer runs: remove edition from host.toml (an old 1.4.4 server's accounts import with reforged-host legacy import revived)";
pub const MAX_HOST_CONFIG_BYTES: u64 = 256 * 1024;
pub const SECRET_FILE: &str = "secrets.env";
pub const SECRET_KEYS: [&str; 11] = [
    "GATEWAY_SECRET",
    "ALLOWLIST_SECRET",
    "REFORGED_ADMIN_KEY",
    "GAMESERVER_API_KEY",
    "API_KEY_HASH_SECRET",
    "MYSTICPARADOX_METRICS_TOKEN",
    "AUTH_SIGNING_PRIVKEY_B64",
    "AUTH_SIGNING_PUBKEY_B64",
    "REFORGED_ADMIN_API_KEY",
    "MYSTICPARADOX_MANAGER_OPERATOR_SECRET",
    "REFORGED_DB_PASSWORD",
];
pub const NODE_CERT_FILE: &str = "node-cert.pem";
pub const NODE_KEY_FILE: &str = "node-key.pem";
pub const CLUSTER_CA_FILE: &str = "cluster-ca-cert.pem";
pub const SESSION_KEY_FILE: &str = "session-key";
pub const ENV_FILE_HEADER: [&str; 2] = [
    "Written by ReforgedHost (`npm run setup`) from host.json and secrets.env. SECRET: never share or commit.",
    "Run setup again instead of editing: it rewrites this file.",
];

pub(crate) fn bad(text: impl Into<String>) -> HostError {
    HostError::message(text)
}

pub(crate) fn shown(value: Option<&Value>) -> String {
    match value {
        None => "undefined".to_owned(),
        Some(v) => jsjson::stringify(v),
    }
}

pub(crate) fn integer(value: Option<&Value>) -> Option<f64> {
    js::safe_integer(value).map(|n| n as f64)
}

pub fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerMode {
    Private,
    Public,
}

impl ServerMode {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Public => "public",
        }
    }

    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "private" => Some(Self::Private),
            "public" => Some(Self::Public),
            _ => None,
        }
    }
}

impl std::fmt::Display for ServerMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Registration {
    Invitecode,
    Open,
    None,
}

impl Registration {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Invitecode => "INVITECODE",
            Self::Open => "OPEN",
            Self::None => "NONE",
        }
    }

    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "INVITECODE" => Some(Self::Invitecode),
            "OPEN" => Some(Self::Open),
            "NONE" => Some(Self::None),
            _ => None,
        }
    }
}

impl std::fmt::Display for Registration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MongoConfig {
    pub uri: String,
    pub manage: bool,
    pub mongod: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Multipliers {
    pub xp: f64,
    pub rewards: f64,
}

impl Default for Multipliers {
    fn default() -> Self {
        Self { xp: 1.0, rewards: 1.0 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinuxOptions {
    pub wine: Option<String>,
    pub proton: Option<String>,
    pub xvfb: bool,
    pub firewall_interface: Option<String>,
}

impl Default for LinuxOptions {
    fn default() -> Self {
        DEFAULT_LINUX
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DatabaseSettings {
    pub engine: Option<String>,
    pub postgres_host: Option<String>,
    pub postgres_port: Option<u16>,
    pub postgres_user: Option<String>,
    pub postgres_database: Option<String>,
    pub allow_remote: Option<bool>,
    pub fresh_start: Option<bool>,
}

impl DatabaseSettings {
    #[must_use]
    pub fn postgres(&self) -> bool {
        self.engine.as_deref() == Some("postgres")
    }

    #[must_use]
    pub fn from_json(d: &Map<String, Value>) -> Self {
        Self {
            engine: d.get("engine").and_then(Value::as_str).map(str::to_owned),
            postgres_host: d.get("postgresHost").and_then(Value::as_str).map(str::to_owned),
            postgres_port: d.get("postgresPort").and_then(Value::as_u64).map(|n| n as u16),
            postgres_user: d.get("postgresUser").and_then(Value::as_str).map(str::to_owned),
            postgres_database: d.get("postgresDatabase").and_then(Value::as_str).map(str::to_owned),
            allow_remote: d.get("allowRemote").and_then(Value::as_bool),
            fresh_start: d.get("freshStart").and_then(Value::as_bool),
        }
    }

    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        if let Some(engine) = &self.engine {
            out.insert("engine".into(), Value::String(engine.clone()));
        }
        if let Some(host) = &self.postgres_host {
            out.insert("postgresHost".into(), Value::String(host.clone()));
        }
        if let Some(port) = self.postgres_port {
            out.insert("postgresPort".into(), Value::from(port));
        }
        if let Some(user) = &self.postgres_user {
            out.insert("postgresUser".into(), Value::String(user.clone()));
        }
        if let Some(db) = &self.postgres_database {
            out.insert("postgresDatabase".into(), Value::String(db.clone()));
        }
        if let Some(allow) = self.allow_remote {
            out.insert("allowRemote".into(), Value::Bool(allow));
        }
        if let Some(fresh) = self.fresh_start {
            out.insert("freshStart".into(), Value::Bool(fresh));
        }
        Value::Object(out)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PerformanceOptions {
    pub server_fps: u32,
    pub idle_fps: u32,
    pub training_on_demand: bool,
    pub training_idle_minutes: u32,
    pub verbose_game_logs: bool,
    pub memory_guard: bool,
    pub min_free_memory_mb: u32,
    pub hunt_memory_mb: u32,
}

impl Default for PerformanceOptions {
    fn default() -> Self {
        DEFAULT_PERFORMANCE
    }
}

const SWITCH_STRINGS: [(&str, &str, &str, &[&str]); 7] = [
    ("reforge", "reforge", "MYSTICPARADOX_REFORGE", &["on", "log", "off"]),
    (
        "masteryPersistence",
        "mastery_persistence",
        "MYSTICPARADOX_MASTERY_PERSISTENCE",
        &["on", "off"],
    ),
    (
        "bountyPersistence",
        "bounty_persistence",
        "MYSTICPARADOX_BOUNTY_PERSISTENCE",
        &["on", "off"],
    ),
    (
        "cooldownPersistence",
        "cooldown_persistence",
        "MYSTICPARADOX_COOLDOWN_PERSISTENCE",
        &["on", "off"],
    ),
    (
        "accountErasure",
        "account_erasure",
        "MYSTICPARADOX_ACCOUNT_ERASURE",
        &["on", "off"],
    ),
    ("logPurge", "log_purge", "MYSTICPARADOX_LOG_PURGE", &["on", "off"]),
    ("rateLimit", "rate_limit", "MYSTICPARADOX_RATE_LIMIT", &["on", "off"]),
];

const SWITCH_NUMBERS: [(&str, &str, &str); 3] = [
    (
        "rateLimitPerMinute",
        "rate_limit_per_minute",
        "MYSTICPARADOX_RATE_LIMIT_PER_MINUTE",
    ),
    (
        "gameserverRateLimitPerMinute",
        "gameserver_rate_limit_per_minute",
        "MYSTICPARADOX_GAMESERVER_RATE_LIMIT_PER_MINUTE",
    ),
    (
        "hostRateLimitPerMinute",
        "host_rate_limit_per_minute",
        "MYSTICPARADOX_HOST_RATE_LIMIT_PER_MINUTE",
    ),
];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SwitchesConfig {
    pub reforge: Option<String>,
    pub mastery_persistence: Option<String>,
    pub bounty_persistence: Option<String>,
    pub cooldown_persistence: Option<String>,
    pub account_erasure: Option<String>,
    pub log_purge: Option<String>,
    pub rate_limit: Option<String>,
    pub rate_limit_per_minute: Option<u32>,
    pub gameserver_rate_limit_per_minute: Option<u32>,
    pub host_rate_limit_per_minute: Option<u32>,
}

impl SwitchesConfig {
    pub fn validate(value: &Value) -> Result<(), HostError> {
        let s = value.as_object().ok_or_else(|| bad("switches must be an object"))?;
        for (key, _, _, allowed) in SWITCH_STRINGS {
            if let Some(v) = s.get(key)
                && !v.as_str().is_some_and(|w| allowed.contains(&w))
            {
                let choices = allowed
                    .iter()
                    .map(|w| format!("\"{w}\""))
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(bad(format!(
                    "switches.{key} must be one of {choices}, not {}",
                    shown(Some(v))
                )));
            }
        }
        for (key, _, _) in SWITCH_NUMBERS {
            if let Some(v) = s.get(key)
                && !integer(Some(v)).is_some_and(|n| (1.0..=1_000_000.0).contains(&n))
            {
                return Err(bad(format!(
                    "switches.{key} must be a whole number from 1 to 1000000, not {}",
                    shown(Some(v))
                )));
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn from_json(s: &Map<String, Value>) -> Self {
        let word = |key: &str| s.get(key).and_then(Value::as_str).map(str::to_owned);
        let num = |key: &str| s.get(key).and_then(Value::as_u64).map(|n| n as u32);
        Self {
            reforge: word("reforge"),
            mastery_persistence: word("masteryPersistence"),
            bounty_persistence: word("bountyPersistence"),
            cooldown_persistence: word("cooldownPersistence"),
            account_erasure: word("accountErasure"),
            log_purge: word("logPurge"),
            rate_limit: word("rateLimit"),
            rate_limit_per_minute: num("rateLimitPerMinute"),
            gameserver_rate_limit_per_minute: num("gameserverRateLimitPerMinute"),
            host_rate_limit_per_minute: num("hostRateLimitPerMinute"),
        }
    }

    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        let words = [
            ("reforge", &self.reforge),
            ("masteryPersistence", &self.mastery_persistence),
            ("bountyPersistence", &self.bounty_persistence),
            ("cooldownPersistence", &self.cooldown_persistence),
            ("accountErasure", &self.account_erasure),
            ("logPurge", &self.log_purge),
            ("rateLimit", &self.rate_limit),
        ];
        for (k, v) in words {
            if let Some(s) = v {
                out.insert(k.to_owned(), Value::String(s.clone()));
            }
        }
        let nums = [
            ("rateLimitPerMinute", self.rate_limit_per_minute),
            ("gameserverRateLimitPerMinute", self.gameserver_rate_limit_per_minute),
            ("hostRateLimitPerMinute", self.host_rate_limit_per_minute),
        ];
        for (k, v) in nums {
            if let Some(n) = v {
                out.insert(k.to_owned(), Value::from(n));
            }
        }
        Value::Object(out)
    }

    #[must_use]
    pub fn env(&self) -> Vec<(&'static str, String)> {
        let json = self.to_json();
        let mut out = Vec::new();
        for (json_key, _, env_key, _) in SWITCH_STRINGS {
            if let Some(val) = json.get(json_key).and_then(Value::as_str) {
                out.push((env_key, val.to_owned()));
            }
        }
        for (json_key, _, env_key) in SWITCH_NUMBERS {
            if let Some(val) = json.get(json_key).and_then(Value::as_u64) {
                out.push((env_key, val.to_string()));
            }
        }
        out
    }

    #[must_use]
    pub fn module_switches(&self) -> Vec<(&'static str, Value)> {
        let json = self.to_json();
        let mut out = Vec::new();
        for (json_key, mod_key, _, _) in SWITCH_STRINGS {
            if let Some(val) = json.get(json_key) {
                out.push((mod_key, val.clone()));
            }
        }
        for (json_key, mod_key, _) in SWITCH_NUMBERS {
            if let Some(val) = json.get(json_key) {
                out.push((mod_key, val.clone()));
            }
        }
        out
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HistoryConfig {
    pub ship_to_peer: Option<module_api::network::NodeId>,
}

impl HistoryConfig {
    pub fn from_json(value: &Value) -> Result<Self, HostError> {
        let obj = value.as_object().ok_or_else(|| bad("history must be an object"))?;
        let ship_to_peer = match obj.get("shipToPeer") {
            None | Some(Value::Null) => None,
            Some(v) => {
                let s = v
                    .as_str()
                    .ok_or_else(|| bad("history.shipToPeer must be a peer node id (a-z, 0-9 and -)"))?;
                let id: module_api::network::NodeId = s.parse().map_err(|e| bad(format!("history.shipToPeer: {e}")))?;
                Some(id)
            }
        };
        Ok(Self { ship_to_peer })
    }

    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        if let Some(peer) = &self.ship_to_peer {
            out.insert("shipToPeer".into(), Value::String(peer.to_string()));
        }
        Value::Object(out)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoteConfig {
    pub role: Option<String>,
    pub node: Option<module_api::network::NodeId>,
    pub region: Option<module_api::network::Region>,
    pub controller_url: Option<String>,
    pub controller_node: Option<module_api::network::NodeId>,
    pub controller_cert_fp: Option<String>,
    pub sync_interval_secs: Option<u64>,
    pub label: Option<String>,
    pub local_overrides: Vec<String>,
}

impl RemoteConfig {
    pub fn from_json(value: &Value) -> Result<Self, HostError> {
        let obj = value.as_object().ok_or_else(|| bad("remote must be an object"))?;
        let role = match obj.get("role") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) if matches!(s.as_str(), "standalone" | "controller" | "managed") => Some(s.clone()),
            Some(v) => {
                return Err(bad(format!(
                    "remote.role must be \"standalone\", \"controller\" or \"managed\", not {}",
                    shown(Some(v))
                )));
            }
        };
        let node = match obj.get("node") {
            None | Some(Value::Null) => None,
            Some(v) => {
                let s = v
                    .as_str()
                    .ok_or_else(|| bad("remote.node must be a node id (a-z, 0-9 and -)"))?;
                let id: module_api::network::NodeId = s.parse().map_err(|e| bad(format!("remote.node: {e}")))?;
                Some(id)
            }
        };
        let region = match obj.get("region") {
            None | Some(Value::Null) => None,
            Some(v) => {
                let s = v
                    .as_str()
                    .ok_or_else(|| bad("remote.region must be a region name (a-z, 0-9 and -)"))?;
                let id: module_api::network::Region = s.parse().map_err(|e| bad(format!("remote.region: {e}")))?;
                Some(id)
            }
        };
        let controller_url = match obj.get("controllerUrl") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => {
                let trimmed = js::trim(s);
                if trimmed.is_empty() {
                    None
                } else {
                    let parsed = url::Url::parse(trimmed)
                        .map_err(|e| bad(format!("remote.controllerUrl must be an https:// URL: {e}")))?;
                    if parsed.scheme() != "https" && parsed.scheme() != "http" {
                        return Err(bad("remote.controllerUrl must use https:// (or http:// on loopback)"));
                    }
                    Some(trimmed.to_owned())
                }
            }
            Some(v) => {
                return Err(bad(format!(
                    "remote.controllerUrl must be a URL string, not {}",
                    shown(Some(v))
                )));
            }
        };
        let controller_node = match obj.get("controllerNode") {
            None | Some(Value::Null) => None,
            Some(v) => {
                let s = v
                    .as_str()
                    .ok_or_else(|| bad("remote.controllerNode must be a node id (a-z, 0-9 and -)"))?;
                let id: module_api::network::NodeId =
                    s.parse().map_err(|e| bad(format!("remote.controllerNode: {e}")))?;
                Some(id)
            }
        };
        let controller_cert_fp = match obj.get("controllerCertFp") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => {
                let trimmed = js::trim(s);
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_owned())
                }
            }
            Some(v) => {
                return Err(bad(format!(
                    "remote.controllerCertFp must be a SHA-256 fingerprint string, not {}",
                    shown(Some(v))
                )));
            }
        };
        let sync_interval_secs = match obj.get("syncIntervalSecs") {
            None | Some(Value::Null) => None,
            Some(v) => {
                let Some(n) = integer(Some(v)).filter(|&n| (1.0..=86400.0).contains(&n)) else {
                    return Err(bad(format!(
                        "remote.syncIntervalSecs must be a whole number from 1 to 86400, not {}",
                        shown(Some(v))
                    )));
                };
                Some(n as u64)
            }
        };
        let label = match obj.get("label") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => {
                let trimmed = js::trim(s);
                if trimmed.is_empty() {
                    None
                } else if trimmed.len() > 64 || trimmed.chars().any(char::is_control) {
                    return Err(bad(
                        "remote.label must be 1 to 64 printable characters without control characters",
                    ));
                } else {
                    Some(trimmed.to_owned())
                }
            }
            Some(v) => {
                return Err(bad(format!("remote.label must be a string, not {}", shown(Some(v)))));
            }
        };
        let local_overrides = match obj.get("localOverrides") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(items)) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    let s = item
                        .as_str()
                        .map(js::trim)
                        .filter(|s| !s.is_empty())
                        .ok_or_else(|| bad("remote.localOverrides must be a list of non-empty setting keys"))?;
                    out.push(s.to_owned());
                }
                out
            }
            Some(v) => {
                return Err(bad(format!(
                    "remote.localOverrides must be a list of setting keys, not {}",
                    shown(Some(v))
                )));
            }
        };
        if role.as_deref() == Some("managed") {
            if controller_url.is_none() {
                return Err(bad("remote.controllerUrl is required when remote.role = \"managed\""));
            }
            if controller_node.is_none() {
                return Err(bad("remote.controllerNode is required when remote.role = \"managed\""));
            }
            if controller_cert_fp.is_none() {
                return Err(bad(
                    "remote.controllerCertFp is required when remote.role = \"managed\"",
                ));
            }
        }
        Ok(Self {
            role,
            node,
            region,
            controller_url,
            controller_node,
            controller_cert_fp,
            sync_interval_secs,
            label,
            local_overrides,
        })
    }

    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        if let Some(role) = &self.role {
            out.insert("role".into(), Value::String(role.clone()));
        }
        if let Some(node) = &self.node {
            out.insert("node".into(), Value::String(node.to_string()));
        }
        if let Some(region) = &self.region {
            out.insert("region".into(), Value::String(region.to_string()));
        }
        if let Some(url) = &self.controller_url {
            out.insert("controllerUrl".into(), Value::String(url.clone()));
        }
        if let Some(cnode) = &self.controller_node {
            out.insert("controllerNode".into(), Value::String(cnode.to_string()));
        }
        if let Some(fp) = &self.controller_cert_fp {
            out.insert("controllerCertFp".into(), Value::String(fp.clone()));
        }
        if let Some(secs) = self.sync_interval_secs {
            out.insert("syncIntervalSecs".into(), Value::from(secs));
        }
        if let Some(label) = &self.label {
            out.insert("label".into(), Value::String(label.clone()));
        }
        if !self.local_overrides.is_empty() {
            out.insert(
                "localOverrides".into(),
                Value::Array(self.local_overrides.iter().cloned().map(Value::String).collect()),
            );
        }
        Value::Object(out)
    }

    #[must_use]
    pub fn effective_node(&self) -> module_api::network::NodeId {
        self.node.clone().unwrap_or_else(|| {
            let default_name = match self.role.as_deref() {
                Some("controller") => "controller",
                _ => "managed",
            };
            default_name
                .parse()
                .unwrap_or_else(|_| module_api::NodeIdentity::standalone().node)
        })
    }

    #[must_use]
    pub fn effective_region(&self) -> module_api::network::Region {
        self.region
            .clone()
            .unwrap_or_else(|| module_api::NodeIdentity::standalone().region)
    }

    #[must_use]
    pub fn controller_peer(&self, local_node: &module_api::network::NodeId) -> Option<network_plugin::cluster::Peer> {
        if self.role.as_deref() != Some("managed") {
            return None;
        }
        let cnode = self.controller_node.as_ref()?;
        if cnode == local_node {
            return None;
        }
        let url_str = self.controller_url.as_deref()?;
        let parsed = url::Url::parse(url_str).ok()?;
        let host = parsed.host_str()?.to_owned();
        let port = parsed.port_or_known_default().unwrap_or(61000);
        Some(network_plugin::cluster::Peer {
            id: cnode.clone(),
            region: self.effective_region(),
            host,
            port,
            fingerprint: None,
        })
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileForm {
    pub document: Map<String, Value>,
    pub read: Map<String, Value>,
    pub toml: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HostConfig {
    pub version: u32,
    pub mode: ServerMode,
    pub host: String,
    pub game_address: String,
    pub name: String,
    pub game_dir: Option<String>,
    pub registration: Registration,
    pub share_url: Option<String>,
    pub mongo: MongoConfig,
    pub max_hunt_servers: u32,
    pub multipliers: Option<Multipliers>,
    pub public_port: Option<u16>,
    pub performance: Option<PerformanceOptions>,
    pub linux: Option<LinuxOptions>,
    pub database: Option<DatabaseSettings>,
    pub guard: Option<GuardConfig>,
    pub history: Option<HistoryConfig>,
    pub switches: Option<SwitchesConfig>,
    pub modules: Option<ModulesConfig>,
    pub network: Option<NetworkConfig>,
    pub remote: Option<RemoteConfig>,
    pub file: FileForm,
}

impl HostConfig {
    pub fn from_json(document: &Value) -> Result<Self, HostError> {
        let validated = validate(document)?;
        let read = document.as_object().cloned().unwrap_or_default();
        Self::from_validated(validated, read, None)
    }

    pub fn from_toml(text: &str) -> Result<Self, HostError> {
        let doc = crate::hosttoml::document(text)?;
        let validated = validate(&doc).map_err(|e| bad(crate::hosttoml::message(&e.to_string())))?;
        let read = doc.as_object().cloned().unwrap_or_default();
        Self::from_validated(validated, read, Some(text.to_owned()))
    }

    fn from_validated(validated: Value, read: Map<String, Value>, toml: Option<String>) -> Result<Self, HostError> {
        let obj = validated
            .as_object()
            .cloned()
            .ok_or_else(|| bad("host.json must hold a JSON object"))?;
        let version = obj.get("version").and_then(Value::as_u64).unwrap_or(1) as u32;
        let mode = obj
            .get("mode")
            .and_then(Value::as_str)
            .and_then(ServerMode::parse)
            .unwrap_or(ServerMode::Private);
        let host = obj.get("host").and_then(Value::as_str).unwrap_or_default().to_owned();
        let game_address = obj
            .get("gameAddress")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let name = obj.get("name").and_then(Value::as_str).unwrap_or_default().to_owned();
        let game_dir = obj.get("gameDir").and_then(Value::as_str).map(str::to_owned);
        let registration = obj
            .get("registration")
            .and_then(Value::as_str)
            .and_then(Registration::parse)
            .unwrap_or(Registration::Invitecode);
        let share_url = obj.get("shareUrl").and_then(Value::as_str).map(str::to_owned);
        let mongo = match obj.get("mongo").and_then(Value::as_object) {
            Some(m) => MongoConfig {
                uri: m
                    .get("uri")
                    .and_then(Value::as_str)
                    .unwrap_or(DEFAULT_MONGO_URI)
                    .to_owned(),
                manage: m.get("manage").and_then(Value::as_bool).unwrap_or(true),
                mongod: m.get("mongod").and_then(Value::as_str).map(str::to_owned),
            },
            None => MongoConfig {
                uri: DEFAULT_MONGO_URI.to_owned(),
                manage: true,
                mongod: None,
            },
        };
        let max_hunt_servers = obj.get("maxHuntServers").and_then(Value::as_u64).unwrap_or(4) as u32;
        let multipliers = obj.get("multipliers").and_then(Value::as_object).and_then(|m| {
            Some(Multipliers {
                xp: m.get("xp")?.as_f64()?,
                rewards: m.get("rewards")?.as_f64()?,
            })
        });
        let public_port = obj.get("publicPort").and_then(Value::as_u64).map(|n| n as u16);
        let performance = obj
            .get("performance")
            .and_then(Value::as_object)
            .map(|p| PerformanceOptions {
                server_fps: p
                    .get("serverFps")
                    .and_then(Value::as_u64)
                    .map(|n| n as u32)
                    .unwrap_or(DEFAULT_PERFORMANCE.server_fps),
                idle_fps: p
                    .get("idleFps")
                    .and_then(Value::as_u64)
                    .map(|n| n as u32)
                    .unwrap_or(DEFAULT_PERFORMANCE.idle_fps),
                training_on_demand: p
                    .get("trainingOnDemand")
                    .and_then(Value::as_bool)
                    .unwrap_or(DEFAULT_PERFORMANCE.training_on_demand),
                training_idle_minutes: p
                    .get("trainingIdleMinutes")
                    .and_then(Value::as_u64)
                    .map(|n| n as u32)
                    .unwrap_or(DEFAULT_PERFORMANCE.training_idle_minutes),
                verbose_game_logs: p
                    .get("verboseGameLogs")
                    .and_then(Value::as_bool)
                    .unwrap_or(DEFAULT_PERFORMANCE.verbose_game_logs),
                memory_guard: p
                    .get("memoryGuard")
                    .and_then(Value::as_bool)
                    .unwrap_or(DEFAULT_PERFORMANCE.memory_guard),
                min_free_memory_mb: p
                    .get("minFreeMemoryMb")
                    .and_then(Value::as_u64)
                    .map(|n| n as u32)
                    .unwrap_or(DEFAULT_PERFORMANCE.min_free_memory_mb),
                hunt_memory_mb: p
                    .get("huntMemoryMb")
                    .and_then(Value::as_u64)
                    .map(|n| n as u32)
                    .unwrap_or(DEFAULT_PERFORMANCE.hunt_memory_mb),
            });
        let linux = obj.get("linux").and_then(Value::as_object).map(|l| LinuxOptions {
            wine: l.get("wine").and_then(Value::as_str).map(str::to_owned),
            proton: l.get("proton").and_then(Value::as_str).map(str::to_owned),
            xvfb: l.get("xvfb").and_then(Value::as_bool).unwrap_or(DEFAULT_LINUX.xvfb),
            firewall_interface: l.get("firewallInterface").and_then(Value::as_str).map(str::to_owned),
        });
        let database = obj
            .get("database")
            .and_then(Value::as_object)
            .map(DatabaseSettings::from_json);
        let guard = obj.get("guard").and_then(Value::as_object).map(GuardConfig::from_json);
        let history = match obj.get("history") {
            Some(h) if !h.is_null() => Some(HistoryConfig::from_json(h)?),
            _ => None,
        };
        let switches = obj
            .get("switches")
            .and_then(Value::as_object)
            .map(SwitchesConfig::from_json);
        let modules = match obj.get("modules") {
            Some(m) if !m.is_null() => Some(ModulesConfig::from_json(m)?),
            _ => None,
        };
        let network = match obj.get("network") {
            Some(n) if !n.is_null() => Some(NetworkConfig::from_json(n)?),
            _ => None,
        };
        let remote = match obj.get("remote") {
            Some(r) if !r.is_null() => Some(RemoteConfig::from_json(r)?),
            _ => None,
        };
        Ok(Self {
            version,
            mode,
            host,
            game_address,
            name,
            game_dir,
            registration,
            share_url,
            mongo,
            max_hunt_servers,
            multipliers,
            public_port,
            performance,
            linux,
            database,
            guard,
            history,
            switches,
            modules,
            network,
            remote,
            file: FileForm {
                document: obj,
                read,
                toml,
            },
        })
    }

    #[must_use]
    pub fn written_over(mut self, previous: Option<&HostConfig>) -> Self {
        if let Some(prev) = previous {
            if self.database.is_none() {
                self.database = prev.database.clone();
            }
            if self.guard.is_none() {
                self.guard = prev.guard.clone();
            }
            if self.history.is_none() {
                self.history = prev.history.clone();
            }
            if self.switches.is_none() {
                self.switches = prev.switches.clone();
            }
            if self.modules.is_none() {
                self.modules = prev.modules.clone();
            }
            if self.network.is_none() {
                self.network = prev.network.clone();
            }
            if self.remote.is_none() {
                self.remote = prev.remote.clone();
            }
            self.file.read = prev.file.read.clone();
            self.file.toml = prev.file.toml.clone();
        }
        self
    }

    #[must_use]
    pub fn fresh_start(&self) -> bool {
        self.database.as_ref().and_then(|d| d.fresh_start).unwrap_or(false)
    }

    fn fields_map(&self) -> Map<String, Value> {
        let mut canonical = Map::new();
        canonical.insert("version".into(), Value::from(self.version));
        canonical.insert("mode".into(), Value::String(self.mode.as_str().to_owned()));
        canonical.insert("host".into(), Value::String(self.host.clone()));
        canonical.insert("gameAddress".into(), Value::String(self.game_address.clone()));
        canonical.insert("name".into(), Value::String(self.name.clone()));
        canonical.insert(
            "gameDir".into(),
            self.game_dir.clone().map_or(Value::Null, Value::String),
        );
        canonical.insert(
            "registration".into(),
            Value::String(self.registration.as_str().to_owned()),
        );
        canonical.insert(
            "shareUrl".into(),
            self.share_url.clone().map_or(Value::Null, Value::String),
        );
        let mut mongo = self
            .file
            .document
            .get("mongo")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        mongo.insert("uri".into(), Value::String(self.mongo.uri.clone()));
        mongo.insert("manage".into(), Value::Bool(self.mongo.manage));
        mongo.insert(
            "mongod".into(),
            self.mongo.mongod.clone().map_or(Value::Null, Value::String),
        );
        canonical.insert("mongo".into(), Value::Object(mongo));
        canonical.insert("maxHuntServers".into(), Value::from(self.max_hunt_servers));
        if let Some(port) = self.public_port {
            canonical.insert("publicPort".into(), Value::from(port));
        }
        if let Some(m) = self.multipliers {
            let mut mult = Map::new();
            if let Some(n) = Number::from_f64(m.xp) {
                mult.insert("xp".into(), Value::Number(format_multiplier_num(n, m.xp)));
            }
            if let Some(n) = Number::from_f64(m.rewards) {
                mult.insert("rewards".into(), Value::Number(format_multiplier_num(n, m.rewards)));
            }
            canonical.insert("multipliers".into(), Value::Object(mult));
        }
        if let Some(p) = &self.performance {
            let all_perf = [
                (
                    "serverFps",
                    Value::from(p.server_fps),
                    p.server_fps != DEFAULT_PERFORMANCE.server_fps,
                ),
                (
                    "idleFps",
                    Value::from(p.idle_fps),
                    p.idle_fps != DEFAULT_PERFORMANCE.idle_fps,
                ),
                (
                    "trainingOnDemand",
                    Value::Bool(p.training_on_demand),
                    p.training_on_demand != DEFAULT_PERFORMANCE.training_on_demand,
                ),
                (
                    "trainingIdleMinutes",
                    Value::from(p.training_idle_minutes),
                    p.training_idle_minutes != DEFAULT_PERFORMANCE.training_idle_minutes,
                ),
                (
                    "verboseGameLogs",
                    Value::Bool(p.verbose_game_logs),
                    p.verbose_game_logs != DEFAULT_PERFORMANCE.verbose_game_logs,
                ),
                (
                    "memoryGuard",
                    Value::Bool(p.memory_guard),
                    p.memory_guard != DEFAULT_PERFORMANCE.memory_guard,
                ),
                (
                    "minFreeMemoryMb",
                    Value::from(p.min_free_memory_mb),
                    p.min_free_memory_mb != DEFAULT_PERFORMANCE.min_free_memory_mb,
                ),
                (
                    "huntMemoryMb",
                    Value::from(p.hunt_memory_mb),
                    p.hunt_memory_mb != DEFAULT_PERFORMANCE.hunt_memory_mb,
                ),
            ];
            let mut perf = Map::new();
            if let Some(orig_p) = self
                .file
                .document
                .get("performance")
                .and_then(Value::as_object)
                .filter(|m| !m.is_empty())
            {
                for (k, v) in orig_p {
                    if let Some((_, new_v, _)) = all_perf.iter().find(|(pk, _, _)| *pk == k.as_str()) {
                        perf.insert(k.clone(), new_v.clone());
                    } else {
                        perf.insert(k.clone(), v.clone());
                    }
                }
                for (k, new_v, changed) in all_perf {
                    if !perf.contains_key(k) && changed {
                        perf.insert(k.into(), new_v);
                    }
                }
            } else {
                for (k, new_v, _) in all_perf {
                    perf.insert(k.into(), new_v);
                }
            }
            canonical.insert("performance".into(), Value::Object(perf));
        }
        if let Some(l) = &self.linux {
            let mut linux = Map::new();
            linux.insert("wine".into(), l.wine.clone().map_or(Value::Null, Value::String));
            linux.insert("proton".into(), l.proton.clone().map_or(Value::Null, Value::String));
            linux.insert("xvfb".into(), Value::Bool(l.xvfb));
            linux.insert(
                "firewallInterface".into(),
                l.firewall_interface.clone().map_or(Value::Null, Value::String),
            );
            canonical.insert("linux".into(), Value::Object(linux));
        }
        if let Some(d) = &self.database {
            canonical.insert("database".into(), d.to_json());
        }
        if let Some(g) = &self.guard {
            canonical.insert("guard".into(), g.to_json());
        }
        if let Some(h) = &self.history {
            canonical.insert("history".into(), h.to_json());
        }
        if let Some(s) = &self.switches {
            canonical.insert("switches".into(), s.to_json());
        }
        if let Some(n) = &self.network {
            canonical.insert("network".into(), n.to_json());
        }
        if let Some(r) = &self.remote {
            canonical.insert("remote".into(), r.to_json());
        }
        if let Some(m) = &self.modules {
            canonical.insert("modules".into(), m.to_json());
        }
        merge_document_order(&self.file.document, canonical)
    }

    pub fn to_toml(&self) -> Result<String, HostError> {
        let fields = self.fields_map();
        let validated = validate(&Value::Object(fields)).map_err(|e| bad(crate::hosttoml::message(&e.to_string())))?;
        let obj = validated
            .as_object()
            .ok_or_else(|| bad("host.toml must hold a table"))?;
        let mut merged = obj.clone();
        // Preserve nullable fields that are None as Null so hosttoml::patch removes them if they were set before
        for k in ["gameDir", "shareUrl"] {
            if !merged.contains_key(k) {
                merged.insert(k.to_owned(), Value::Null);
            }
        }
        crate::hosttoml::text(&merged, &self.file.read, self.file.toml.as_deref())
    }

    pub fn to_json(&self) -> Result<Value, HostError> {
        let fields = self.fields_map();
        validate(&Value::Object(fields))
    }
}

fn format_multiplier_num(n: Number, raw: f64) -> Number {
    if raw.fract() == 0.0 && (0.0..=(i64::MAX as f64)).contains(&raw) {
        Number::from(raw as i64)
    } else {
        n
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Secrets {
    pub gateway_secret: String,
    pub allowlist_secret: String,
    pub admin_key: String,
    pub gameserver_key: String,
    pub api_key_hash_secret: String,
    pub metrics_token: String,
    pub jwt_private_key_b64: String,
    pub jwt_public_key_b64: String,
    pub admin_api_key: String,
    pub operator_secret: String,
    pub database_password: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Windows,
    Linux,
    Other,
}

impl Platform {
    #[must_use]
    pub fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else {
            Self::Other
        }
    }
}

pub type EnvEntries = Vec<(String, String)>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvFiles {
    pub backend: EnvEntries,
    pub director: EnvEntries,
    pub content: EnvEntries,
    pub gateway: EnvEntries,
    pub admin: EnvEntries,
    pub allowlist: Option<EnvEntries>,
}

impl EnvFiles {
    #[must_use]
    pub fn files(&self) -> [(&'static str, Option<&EnvEntries>); 6] {
        [
            ("backend", Some(&self.backend)),
            ("director", Some(&self.director)),
            ("content", Some(&self.content)),
            ("gateway", Some(&self.gateway)),
            ("admin", Some(&self.admin)),
            ("allowlist", self.allowlist.as_ref()),
        ]
    }
}

#[must_use]
pub fn is_local_database_host(host: &str) -> bool {
    let trimmed = host.trim();
    trimmed.eq_ignore_ascii_case("localhost") || trimmed == "127.0.0.1" || trimmed == "::1"
}

#[must_use]
pub fn performance_options(c: &HostConfig) -> PerformanceOptions {
    c.performance.unwrap_or(DEFAULT_PERFORMANCE)
}

#[must_use]
pub fn valid_minutes(value: f64) -> bool {
    value.is_finite() && value.fract() == 0.0 && (0.0..=1440.0).contains(&value)
}

#[must_use]
pub fn valid_megabytes(value: f64, min: f64) -> bool {
    value.is_finite() && value.fract() == 0.0 && value >= min && value <= 1_048_576.0
}

#[must_use]
pub fn valid_fps(value: f64, min: f64) -> bool {
    value.is_finite() && value.fract() == 0.0 && (value == 0.0 || (value >= min && value <= 240.0))
}

#[must_use]
pub fn game_ports(c: &HostConfig) -> (u16, u16) {
    game_port_range(c.max_hunt_servers)
}

#[must_use]
pub fn game_ports_text(c: &HostConfig, separator: &str) -> String {
    let (first, last) = game_ports(c);
    format!("{first}{separator}{last}")
}

#[must_use]
pub fn gateway_port(c: &HostConfig) -> u16 {
    match c.mode {
        ServerMode::Public => c.public_port.unwrap_or(PORTS.public_gateway),
        ServerMode::Private => PORTS.private_gateway,
    }
}

#[must_use]
pub fn multipliers_on(c: &HostConfig) -> bool {
    c.multipliers
        .is_some_and(|m| (m.xp - 1.0).abs() > f64::EPSILON || (m.rewards - 1.0).abs() > f64::EPSILON)
}

#[must_use]
pub fn linux_options(c: &HostConfig) -> LinuxOptions {
    c.linux.clone().unwrap_or(DEFAULT_LINUX)
}

fn parse_ipv4_octets(host: &str) -> Option<[u8; 4]> {
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() != 4 {
        return None;
    }
    let mut octets = [0u8; 4];
    for (idx, part) in parts.into_iter().enumerate() {
        if part.is_empty() || (part.len() > 1 && part.starts_with('0')) || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        octets[idx] = part.parse::<u8>().ok()?;
    }
    Some(octets)
}

#[must_use]
pub fn is_ipv4(host: &str) -> bool {
    parse_ipv4_octets(host).is_some()
}

#[must_use]
pub fn is_tailscale_ipv4(host: &str) -> bool {
    let Some([a, b, _, _]) = parse_ipv4_octets(host) else {
        return false;
    };
    a == 100 && (64..=127).contains(&b)
}

#[must_use]
pub fn is_public_ipv4(host: &str) -> bool {
    let Some([a, b, _, _]) = parse_ipv4_octets(host) else {
        return false;
    };
    if a == 0 || a == 10 || a == 127 || a >= 224 {
        return false;
    }
    if a == 100 && (64..=127).contains(&b) {
        return false;
    }
    if a == 169 && b == 254 {
        return false;
    }
    if a == 172 && (16..=31).contains(&b) {
        return false;
    }
    if a == 192 && b == 168 {
        return false;
    }
    true
}

#[must_use]
pub fn is_dns_name(host: &str) -> bool {
    if host.is_empty() || host.len() > 253 || is_ipv4(host) || !host.contains('.') {
        return false;
    }
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2 {
        return false;
    }
    for label in &labels {
        let bytes = label.as_bytes();
        if bytes.is_empty()
            || bytes.len() > 63
            || !bytes[0].is_ascii_alphanumeric()
            || !bytes[bytes.len() - 1].is_ascii_alphanumeric()
            || !bytes.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'-')
        {
            return false;
        }
    }
    let tld = labels[labels.len() - 1];
    tld.len() >= 2 && !tld.bytes().all(|b| b.is_ascii_digit())
}

#[must_use]
pub fn is_linux_interface(name: &str) -> bool {
    let bytes = name.as_bytes();
    (1..=15).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(*b, b'_' | b'.' | b'-'))
}

#[must_use]
pub fn clean_server_name(name: &str) -> String {
    let mut replaced = String::with_capacity(name.len());
    for ch in name.chars() {
        let code = ch as u32;
        if code <= 0x1f || (0x7f..=0x9f).contains(&code) {
            replaced.push(' ');
        } else {
            replaced.push(ch);
        }
    }
    let mut collapsed = String::with_capacity(replaced.len());
    let mut in_space = false;
    for ch in js::trim(&replaced).chars() {
        if js::is_space(ch) {
            if !in_space {
                collapsed.push(' ');
                in_space = true;
            }
        } else {
            collapsed.push(ch);
            in_space = false;
        }
    }
    let mut utf16_units = 0usize;
    let mut truncated = String::with_capacity(collapsed.len());
    for ch in collapsed.chars() {
        let len16 = ch.len_utf16();
        if utf16_units + len16 > 48 {
            break;
        }
        utf16_units += len16;
        truncated.push(ch);
    }
    js::trim(&truncated).to_owned()
}

#[must_use]
pub fn is_valid_share_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://login.tailscale.com/") else {
        return false;
    };
    !rest.is_empty()
        && rest
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'_' | b'?' | b'=' | b'&' | b'.' | b'%' | b'-'))
}

fn is_reserved_or_game_port(port: u16) -> bool {
    RESERVED_PORTS.contains(&port) || (PORTS.game_lowest..=PORTS.game_last).contains(&port)
}

pub fn validate(document: &Value) -> Result<Value, HostError> {
    let Some(obj) = document.as_object() else {
        return Err(bad("host.json must hold a JSON object"));
    };
    let mode_str = obj
        .get("mode")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("mode must be private or public"))?;
    let mode = ServerMode::parse(mode_str).ok_or_else(|| bad("mode must be private or public"))?;

    match obj.get("edition") {
        None | Some(Value::Null) => {}
        Some(Value::String(s)) if s == "reborn" => {}
        Some(Value::String(s)) if s == "legacy" || s == "both" => return Err(bad(EDITION_REMOVED)),
        Some(_) => return Err(bad("edition must be reborn")),
    }

    let raw_host = obj
        .get("host")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("host must be a string"))?;
    let host = js::trim(raw_host).to_ascii_lowercase();
    match mode {
        ServerMode::Private => {
            if !is_tailscale_ipv4(&host) && (!is_dns_name(&host) || !host.ends_with(".ts.net")) {
                return Err(bad(format!(
                    "a private server's host must be this PC's Tailscale address (100.64.0.0/10) or MagicDNS name, not {raw_host}"
                )));
            }
        }
        ServerMode::Public => {
            if !is_public_ipv4(&host) && !is_dns_name(&host) {
                return Err(bad(format!(
                    "a public server's host must be a public IPv4 address or a DNS name, not {raw_host}"
                )));
            }
        }
    }

    let raw_game_addr = obj.get("gameAddress");
    let Some(game_addr_str) = raw_game_addr.and_then(Value::as_str) else {
        return Err(bad(format!(
            "gameAddress must be an IPv4 address (the one players' game traffic reaches), not {}",
            shown(raw_game_addr)
        )));
    };
    let game_address = js::trim(game_addr_str).to_owned();
    if !is_ipv4(&game_address) {
        return Err(bad(format!(
            "gameAddress must be an IPv4 address (the one players' game traffic reaches), not {}",
            shown(raw_game_addr)
        )));
    }
    if mode == ServerMode::Private && !is_tailscale_ipv4(&game_address) {
        return Err(bad("a private server's game address must be its Tailscale IPv4"));
    }

    let raw_name = obj
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("the server needs a name"))?;
    let name = clean_server_name(raw_name);
    if name.is_empty() {
        return Err(bad("the server needs a name"));
    }

    let game_dir = match obj.get("gameDir") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            let trimmed = js::trim(s);
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_owned())
            }
        }
        Some(other) => {
            return Err(bad(format!(
                "gameDir must be a folder or null, not {}",
                shown(Some(other))
            )));
        }
    };

    let registration_str = obj
        .get("registration")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("registration must be INVITECODE, OPEN or NONE"))?;
    let registration =
        Registration::parse(registration_str).ok_or_else(|| bad("registration must be INVITECODE, OPEN or NONE"))?;

    let share_url = match obj.get("shareUrl") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            let trimmed = js::trim(s);
            if trimmed.is_empty() {
                None
            } else if !is_valid_share_url(trimmed) {
                return Err(bad("shareUrl must be a https://login.tailscale.com/ link"));
            } else {
                Some(trimmed.to_owned())
            }
        }
        Some(_) => return Err(bad("shareUrl must be a https://login.tailscale.com/ link")),
    };

    let (mongo_uri, mongo_manage, mongo_mongod) = match obj.get("mongo") {
        None => (DEFAULT_MONGO_URI.to_owned(), true, None),
        Some(m_val) => {
            let m = m_val.as_object().ok_or_else(|| bad("mongo must be an object"))?;
            let uri_raw = m
                .get("uri")
                .and_then(Value::as_str)
                .ok_or_else(|| bad("mongo.uri must be a mongodb:// connection string"))?;
            let uri = js::trim(uri_raw);
            if !uri.starts_with("mongodb://") && !uri.starts_with("mongodb+srv://") {
                return Err(bad("mongo.uri must be a mongodb:// connection string"));
            }
            let manage = match m.get("manage") {
                None => true,
                Some(Value::Bool(b)) => *b,
                Some(_) => return Err(bad("mongo.manage must be true or false")),
            };
            let mongod = match m.get("mongod") {
                None | Some(Value::Null) => None,
                Some(Value::String(s)) => {
                    let trimmed = js::trim(s);
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_owned())
                    }
                }
                Some(_) => return Err(bad("mongo.mongod must be a string or null")),
            };
            (uri.to_owned(), manage, mongod)
        }
    };

    let hunts_val = obj.get("maxHuntServers");
    let max_hunts_f = integer(hunts_val)
        .ok_or_else(|| bad("maxHuntServers must be 1 to 10 (UDP 8770-8777 up to 6 hunts, 8766-8777 at 10)"))?;
    if !(1.0..=(MAX_HUNTS as f64)).contains(&max_hunts_f) {
        return Err(bad(
            "maxHuntServers must be 1 to 10 (UDP 8770-8777 up to 6 hunts, 8766-8777 at 10)",
        ));
    }
    let max_hunt_servers = max_hunts_f as u32;

    let multipliers = match obj.get("multipliers") {
        None | Some(Value::Null) => None,
        Some(m_val) => {
            let m = m_val
                .as_object()
                .ok_or_else(|| bad("multipliers must be an object with xp and rewards"))?;
            let check_mult = |key: &str, what: &str| -> Result<f64, HostError> {
                let v = m.get(key);
                let Some(num) = v.and_then(Value::as_f64).filter(|n| n.is_finite()) else {
                    return Err(bad(format!(
                        "the {what} multiplier must be a number from 1 to 100, not {}",
                        shown(v)
                    )));
                };
                if !(1.0..=MAX_MULTIPLIER).contains(&num) {
                    return Err(bad(format!(
                        "the {what} multiplier must be a number from 1 to 100, not {}",
                        shown(v)
                    )));
                }
                Ok((num * 100.0).round() / 100.0)
            };
            let xp = check_mult("xp", "XP")?;
            let rewards = check_mult("rewards", "reward")?;
            if mode == ServerMode::Public && ((xp - 1.0).abs() > f64::EPSILON || (rewards - 1.0).abs() > f64::EPSILON) {
                return Err(bad(
                    "XP and reward multipliers are for private (Tailscale) test servers only",
                ));
            }
            Some(Multipliers { xp, rewards })
        }
    };

    let public_port = match obj.get("publicPort") {
        None | Some(Value::Null) => None,
        Some(v) => {
            if mode != ServerMode::Public {
                return Err(bad("publicPort is for public servers only"));
            }
            let Some(n) = integer(Some(v)).filter(|&n| (1.0..=65535.0).contains(&n)) else {
                return Err(bad(format!(
                    "publicPort must be a TCP port from 1 to 65535, not {}",
                    shown(Some(v))
                )));
            };
            let port = n as u16;
            if is_reserved_or_game_port(port) {
                return Err(bad(format!(
                    "publicPort {port} is one the server uses itself: pick another (4443, for example)"
                )));
            }
            Some(port)
        }
    };

    let performance = match obj.get("performance") {
        None | Some(Value::Null) => None,
        Some(p_val) => {
            let p = p_val.as_object().ok_or_else(|| bad("performance must be an object"))?;
            let server_fps = match p.get("serverFps") {
                None => DEFAULT_PERFORMANCE.server_fps,
                Some(v) => {
                    let Some(n) = integer(Some(v)).filter(|&n| valid_fps(n, 10.0)) else {
                        return Err(bad(format!(
                            "performance.serverFps must be 0 (no cap) or 10 to 240, not {}",
                            shown(Some(v))
                        )));
                    };
                    n as u32
                }
            };
            let idle_fps = match p.get("idleFps") {
                None => DEFAULT_PERFORMANCE.idle_fps,
                Some(v) => {
                    let Some(n) = integer(Some(v)).filter(|&n| valid_fps(n, 1.0)) else {
                        return Err(bad(format!(
                            "performance.idleFps must be 0 (same as serverFps) or an integer from 1 to 240, not {}",
                            shown(Some(v))
                        )));
                    };
                    n as u32
                }
            };
            let training_on_demand = match p.get("trainingOnDemand") {
                None => DEFAULT_PERFORMANCE.training_on_demand,
                Some(Value::Bool(b)) => *b,
                Some(v) => {
                    return Err(bad(format!(
                        "performance.trainingOnDemand must be true or false, not {}",
                        shown(Some(v))
                    )));
                }
            };
            let training_idle_minutes = match p.get("trainingIdleMinutes") {
                None => DEFAULT_PERFORMANCE.training_idle_minutes,
                Some(v) => {
                    let Some(n) = integer(Some(v)).filter(|&n| valid_minutes(n)) else {
                        return Err(bad(format!(
                            "performance.trainingIdleMinutes must be an integer from 0 to 1440, not {}",
                            shown(Some(v))
                        )));
                    };
                    n as u32
                }
            };
            let verbose_game_logs = match p.get("verboseGameLogs") {
                None => DEFAULT_PERFORMANCE.verbose_game_logs,
                Some(Value::Bool(b)) => *b,
                Some(v) => {
                    return Err(bad(format!(
                        "performance.verboseGameLogs must be true or false, not {}",
                        shown(Some(v))
                    )));
                }
            };
            let memory_guard = match p.get("memoryGuard") {
                None => DEFAULT_PERFORMANCE.memory_guard,
                Some(Value::Bool(b)) => *b,
                Some(v) => {
                    return Err(bad(format!(
                        "performance.memoryGuard must be true or false, not {}",
                        shown(Some(v))
                    )));
                }
            };
            let min_free_memory_mb = match p.get("minFreeMemoryMb") {
                None => DEFAULT_PERFORMANCE.min_free_memory_mb,
                Some(v) => {
                    let Some(n) = integer(Some(v)).filter(|&n| valid_megabytes(n, 0.0)) else {
                        return Err(bad(format!(
                            "performance.minFreeMemoryMb must be an integer from 0 to 1048576, not {}",
                            shown(Some(v))
                        )));
                    };
                    n as u32
                }
            };
            let hunt_memory_mb = match p.get("huntMemoryMb") {
                None => DEFAULT_PERFORMANCE.hunt_memory_mb,
                Some(v) => {
                    let Some(n) = integer(Some(v)).filter(|&n| valid_megabytes(n, 256.0)) else {
                        return Err(bad(format!(
                            "performance.huntMemoryMb must be an integer from 256 to 1048576, not {}",
                            shown(Some(v))
                        )));
                    };
                    n as u32
                }
            };
            Some(PerformanceOptions {
                server_fps,
                idle_fps,
                training_on_demand,
                training_idle_minutes,
                verbose_game_logs,
                memory_guard,
                min_free_memory_mb,
                hunt_memory_mb,
            })
        }
    };

    let linux = match obj.get("linux") {
        None | Some(Value::Null) => None,
        Some(l_val) => {
            let l = l_val.as_object().ok_or_else(|| bad("linux must be an object"))?;
            let prog = |key: &str| -> Result<Option<String>, HostError> {
                match l.get(key) {
                    None | Some(Value::Null) => Ok(None),
                    Some(Value::String(s)) => {
                        let trimmed = js::trim(s);
                        if trimmed.is_empty() {
                            Ok(None)
                        } else if !trimmed.starts_with('/') {
                            Err(bad(format!(
                                "linux.{key} must be the full path of the program, or null"
                            )))
                        } else {
                            Ok(Some(trimmed.to_owned()))
                        }
                    }
                    Some(_) => Err(bad(format!(
                        "linux.{key} must be the full path of the program, or null"
                    ))),
                }
            };
            let wine = prog("wine")?;
            let proton = prog("proton")?;
            let xvfb = match l.get("xvfb") {
                None => DEFAULT_LINUX.xvfb,
                Some(Value::Bool(b)) => *b,
                Some(_) => return Err(bad("linux.xvfb must be true or false")),
            };
            let firewall_interface = match l.get("firewallInterface") {
                None | Some(Value::Null) => None,
                Some(Value::String(s)) => {
                    let trimmed = js::trim(s);
                    if trimmed.is_empty() {
                        None
                    } else if !is_linux_interface(trimmed) {
                        return Err(bad(format!(
                            "linux.firewallInterface must be a network interface name like eth0, not {}",
                            shown(l.get("firewallInterface"))
                        )));
                    } else {
                        Some(trimmed.to_owned())
                    }
                }
                Some(v) => {
                    return Err(bad(format!(
                        "linux.firewallInterface must be a network interface name like eth0, not {}",
                        shown(Some(v))
                    )));
                }
            };
            Some(LinuxOptions {
                wine,
                proton,
                xvfb,
                firewall_interface,
            })
        }
    };

    let database = match obj.get("database") {
        None | Some(Value::Null) => None,
        Some(d_val) => {
            let d = d_val.as_object().ok_or_else(|| bad("database must be an object"))?;
            let engine = match d.get("engine") {
                None | Some(Value::Null) => None,
                Some(Value::String(s)) if s == "sqlite" || s == "postgres" => Some(s.clone()),
                Some(v) => {
                    return Err(bad(format!(
                        "database.engine must be \"sqlite\" or \"postgres\", not {}",
                        shown(Some(v))
                    )));
                }
            };
            let clean_str = |key: &str| -> Result<Option<String>, HostError> {
                match d.get(key) {
                    None | Some(Value::Null) => Ok(None),
                    Some(Value::String(s)) => {
                        let trimmed = js::trim(s);
                        if trimmed.is_empty() || trimmed.chars().any(char::is_control) {
                            Err(bad(format!("database.{key} must be a non-empty string")))
                        } else {
                            Ok(Some(trimmed.to_owned()))
                        }
                    }
                    Some(v) => Err(bad(format!(
                        "database.{key} must be a non-empty string, not {}",
                        shown(Some(v))
                    ))),
                }
            };
            let postgres_host = clean_str("postgresHost")?;
            let postgres_user = clean_str("postgresUser")?;
            let postgres_database = clean_str("postgresDatabase")?;
            let postgres_port = match d.get("postgresPort") {
                None | Some(Value::Null) => None,
                Some(v) => {
                    let Some(n) = integer(Some(v)).filter(|&n| (1.0..=65535.0).contains(&n)) else {
                        return Err(bad(format!(
                            "database.postgresPort must be a TCP port from 1 to 65535, not {}",
                            shown(Some(v))
                        )));
                    };
                    Some(n as u16)
                }
            };
            let allow_remote = match d.get("allowRemote") {
                None | Some(Value::Null) => None,
                Some(Value::Bool(b)) => Some(*b),
                Some(v) => {
                    return Err(bad(format!(
                        "database.allowRemote must be true or false, not {}",
                        shown(Some(v))
                    )));
                }
            };
            let fresh_start = match d.get("freshStart") {
                None | Some(Value::Null) => None,
                Some(Value::Bool(b)) => Some(*b),
                Some(v) => {
                    return Err(bad(format!(
                        "database.freshStart must be true or false, not {}",
                        shown(Some(v))
                    )));
                }
            };
            if engine.as_deref() == Some("postgres") {
                let db_host = postgres_host.as_deref().unwrap_or("127.0.0.1");
                if !allow_remote.unwrap_or(false) && !is_local_database_host(db_host) {
                    return Err(bad(format!(
                        "database.postgresHost {db_host} is not on this machine, and connections to Postgres are not encrypted: use 127.0.0.1, or set database.allowRemote = true if the network between them is private"
                    )));
                }
            }
            Some(DatabaseSettings {
                engine,
                postgres_host,
                postgres_port,
                postgres_user,
                postgres_database,
                allow_remote,
                fresh_start,
            })
        }
    };

    if let Some(g) = obj.get("guard").filter(|v| !v.is_null()) {
        guard::validate(g)?;
    }
    if let Some(s) = obj.get("switches").filter(|v| !v.is_null()) {
        SwitchesConfig::validate(s)?;
    }
    let modules_cfg = match obj.get("modules").filter(|v| !v.is_null()) {
        Some(m) => Some(ModulesConfig::from_json(m)?),
        None => None,
    };
    let network_cfg = match obj.get("network").filter(|v| !v.is_null()) {
        Some(n) => {
            let net = NetworkConfig::from_json(n)?;
            if !net.nodes.is_empty() && mode != ServerMode::Public {
                return Err(bad(
                    "network.nodes needs mode = \"public\": node links run over the gateway's TLS port",
                ));
            }
            Some(net)
        }
        None => None,
    };
    let remote_cfg = match obj.get("remote").filter(|v| !v.is_null()) {
        Some(r) => Some(RemoteConfig::from_json(r)?),
        None => None,
    };
    let history_cfg = match obj.get("history").filter(|v| !v.is_null()) {
        Some(h) => {
            let hist = HistoryConfig::from_json(h)?;
            if let Some(peer) = &hist.ship_to_peer {
                let Some(net) = &network_cfg else {
                    return Err(bad("history.shipToPeer requires [network]"));
                };
                if *peer == net.node {
                    return Err(bad(format!("history.shipToPeer: {peer} is this node itself")));
                }
                if !net.nodes.iter().any(|n| n.id == *peer) {
                    return Err(bad(format!(
                        "history.shipToPeer: {peer} is not listed in [[network.nodes]]"
                    )));
                }
            }
            Some(hist)
        }
        None => None,
    };

    let mut canonical = Map::new();
    canonical.insert("version".into(), Value::from(1));
    canonical.insert("mode".into(), Value::String(mode.as_str().to_owned()));
    canonical.insert("host".into(), Value::String(host));
    canonical.insert("gameAddress".into(), Value::String(game_address));
    canonical.insert("name".into(), Value::String(name));
    canonical.insert("gameDir".into(), game_dir.map_or(Value::Null, Value::String));
    canonical.insert("registration".into(), Value::String(registration.as_str().to_owned()));
    canonical.insert("shareUrl".into(), share_url.map_or(Value::Null, Value::String));
    let mut mongo_out = obj.get("mongo").and_then(Value::as_object).cloned().unwrap_or_default();
    mongo_out.insert("uri".into(), Value::String(mongo_uri));
    mongo_out.insert("manage".into(), Value::Bool(mongo_manage));
    mongo_out.insert("mongod".into(), mongo_mongod.map_or(Value::Null, Value::String));
    canonical.insert("mongo".into(), Value::Object(mongo_out));
    canonical.insert("maxHuntServers".into(), Value::from(max_hunt_servers));
    if let Some(port) = public_port {
        canonical.insert("publicPort".into(), Value::from(port));
    }
    if let Some(m) = multipliers {
        let mut mult_out = Map::new();
        if let Some(n) = Number::from_f64(m.xp) {
            mult_out.insert("xp".into(), Value::Number(format_multiplier_num(n, m.xp)));
        }
        if let Some(n) = Number::from_f64(m.rewards) {
            mult_out.insert("rewards".into(), Value::Number(format_multiplier_num(n, m.rewards)));
        }
        canonical.insert("multipliers".into(), Value::Object(mult_out));
    }
    if let Some(p) = performance {
        let all_perf = [
            ("serverFps", Value::from(p.server_fps)),
            ("idleFps", Value::from(p.idle_fps)),
            ("trainingOnDemand", Value::Bool(p.training_on_demand)),
            ("trainingIdleMinutes", Value::from(p.training_idle_minutes)),
            ("verboseGameLogs", Value::Bool(p.verbose_game_logs)),
            ("memoryGuard", Value::Bool(p.memory_guard)),
            ("minFreeMemoryMb", Value::from(p.min_free_memory_mb)),
            ("huntMemoryMb", Value::from(p.hunt_memory_mb)),
        ];
        let mut perf_out = Map::new();
        if let Some(orig_p) = obj
            .get("performance")
            .and_then(Value::as_object)
            .filter(|m| !m.is_empty())
        {
            for (k, v) in orig_p {
                if let Some((_, new_v)) = all_perf.iter().find(|(pk, _)| *pk == k.as_str()) {
                    perf_out.insert(k.clone(), new_v.clone());
                } else {
                    perf_out.insert(k.clone(), v.clone());
                }
            }
        } else {
            for (k, new_v) in all_perf {
                perf_out.insert(k.into(), new_v);
            }
        }
        canonical.insert("performance".into(), Value::Object(perf_out));
    }
    if let Some(l) = linux {
        let mut linux_out = Map::new();
        linux_out.insert("wine".into(), l.wine.map_or(Value::Null, Value::String));
        linux_out.insert("proton".into(), l.proton.map_or(Value::Null, Value::String));
        linux_out.insert("xvfb".into(), Value::Bool(l.xvfb));
        linux_out.insert(
            "firewallInterface".into(),
            l.firewall_interface.map_or(Value::Null, Value::String),
        );
        canonical.insert("linux".into(), Value::Object(linux_out));
    }
    if let Some(d) = database {
        canonical.insert("database".into(), d.to_json());
    }
    if let Some(g) = obj.get("guard").and_then(Value::as_object) {
        canonical.insert("guard".into(), GuardConfig::from_json(g).to_json());
    }
    if let Some(h) = history_cfg {
        canonical.insert("history".into(), h.to_json());
    }
    if let Some(s) = obj.get("switches").and_then(Value::as_object) {
        canonical.insert("switches".into(), SwitchesConfig::from_json(s).to_json());
    }
    if let Some(n) = network_cfg {
        canonical.insert("network".into(), n.to_json());
    }
    if let Some(r) = remote_cfg {
        canonical.insert("remote".into(), r.to_json());
    }
    if let Some(m) = modules_cfg {
        canonical.insert("modules".into(), m.to_json());
    }
    Ok(Value::Object(merge_document_order(obj, canonical)))
}

fn merge_document_order(orig: &Map<String, Value>, canonical: Map<String, Value>) -> Map<String, Value> {
    if orig.is_empty() {
        return canonical;
    }
    const OPTIONAL_KEYS: &[&str] = &[
        "edition",
        "publicPort",
        "multipliers",
        "performance",
        "linux",
        "database",
        "guard",
        "history",
        "switches",
        "network",
        "remote",
        "modules",
    ];
    let mut out = Map::new();
    for (k, orig_v) in orig {
        if let Some(new_v) = canonical.get(k) {
            out.insert(k.clone(), new_v.clone());
        } else if OPTIONAL_KEYS.contains(&k.as_str()) {
            continue;
        } else {
            out.insert(k.clone(), orig_v.clone());
        }
    }
    for (k, new_v) in canonical {
        if !out.contains_key(&k) {
            out.insert(k, new_v);
        }
    }
    out
}

fn format_js_pretty(value: &Value, indent: usize) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.as_f64().map_or_else(|| "null".to_owned(), jsjson::number),
        Value::String(s) => jsjson::stringify(&Value::String(s.clone())),
        Value::Array(items) => {
            if items.is_empty() {
                return "[]".to_owned();
            }
            let pad = " ".repeat(indent + 2);
            let close_pad = " ".repeat(indent);
            let inner = items
                .iter()
                .map(|item| format!("{pad}{}", format_js_pretty(item, indent + 2)))
                .collect::<Vec<_>>()
                .join(",\n");
            format!("[\n{inner}\n{close_pad}]")
        }
        Value::Object(entries) => {
            if entries.is_empty() {
                return "{}".to_owned();
            }
            let pad = " ".repeat(indent + 2);
            let close_pad = " ".repeat(indent);
            let inner = entries
                .iter()
                .map(|(k, v)| {
                    format!(
                        "{pad}{}: {}",
                        jsjson::stringify(&Value::String(k.clone())),
                        format_js_pretty(v, indent + 2)
                    )
                })
                .collect::<Vec<_>>()
                .join(",\n");
            format!("{{\n{inner}\n{close_pad}}}")
        }
    }
}

#[must_use]
pub fn stringify_pretty(value: &Value) -> String {
    format_js_pretty(&jsjson::to_js(value), 0)
}

pub fn read_host_config(file: &Path) -> Result<Option<HostConfig>, HostError> {
    let meta = match std::fs::metadata(file) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(HostError::io(file, e)),
    };
    if meta.len() > MAX_HOST_CONFIG_BYTES {
        return Err(bad(format!(
            "{} is larger than {} KiB",
            file.display(),
            MAX_HOST_CONFIG_BYTES / 1024
        )));
    }
    let text = std::fs::read_to_string(file).map_err(|e| HostError::io(file, e))?;
    let ext = file
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        "toml" => HostConfig::from_toml(&text).map(Some),
        "yaml" | "yml" => {
            let doc = yaml::parse_yaml_to_json(&text)
                .map_err(|e| bad(format!("{} is not valid YAML: {e}", file.display())))?;
            HostConfig::from_json(&doc).map(Some)
        }
        _ => {
            let doc: Value =
                serde_json::from_str(&text).map_err(|e| bad(format!("{} is not valid JSON: {e}", file.display())))?;
            HostConfig::from_json(&doc).map(Some)
        }
    }
}

pub fn load_host_config(p: &Paths) -> Result<Option<HostConfig>, HostError> {
    if let Some(cfg) = read_host_config(&p.host_toml)? {
        return Ok(Some(cfg));
    }
    read_host_config(&p.host_json)
}

#[must_use]
pub fn has_host_config(p: &Paths) -> bool {
    p.host_toml.is_file() || p.host_json.is_file()
}

pub fn host_config_text(c: &HostConfig) -> Result<String, HostError> {
    Ok(format!("{}\n", stringify_pretty(&c.to_json()?)))
}

pub fn write_host_config(file: &Path, c: &HostConfig) -> Result<(), HostError> {
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| HostError::io(parent, e))?;
    }
    let ext = file
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let text = if ext == "json" {
        format!("{}\n", stringify_pretty(&c.to_json()?))
    } else {
        c.to_toml()?
    };
    std::fs::write(file, text).map_err(|e| HostError::io(file, e))
}

pub fn save_host_config(p: &Paths, c: &HostConfig) -> Result<bool, HostError> {
    let had_json_only = !p.host_toml.exists() && p.host_json.is_file();
    write_host_config(&p.host_toml, c)?;
    if p.host_json.is_file() {
        let bak = p.config.join("host.json.bak");
        let _ = std::fs::remove_file(&bak);
        std::fs::rename(&p.host_json, &bak).map_err(|e| HostError::io(&p.host_json, e))?;
    }
    Ok(had_json_only)
}

pub fn new_secrets() -> Result<Secrets, HostError> {
    crate::commands::setup::secrets::new_secrets()
}

pub fn write_secret_file(file: &Path, text: &str) -> Result<(), HostError> {
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent).map_err(|e| HostError::io(parent, e))?;
    }
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut handle = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(file)
            .map_err(|e| HostError::io(file, e))?;
        handle.write_all(text.as_bytes()).map_err(|e| HostError::io(file, e))?;
        let _ = std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o600));
        return Ok(());
    }
    #[cfg(not(unix))]
    {
        std::fs::write(file, text).map_err(|e| HostError::io(file, e))
    }
}

pub fn load_or_create_secrets(
    config_dir: &Path,
    make: impl FnOnce() -> Result<Secrets, HostError>,
) -> Result<(Secrets, bool), HostError> {
    let file = config_dir.join(SECRET_FILE);
    if !file.exists() {
        let s = make()?;
        let entries = [
            ("GATEWAY_SECRET", s.gateway_secret.as_str()),
            ("ALLOWLIST_SECRET", s.allowlist_secret.as_str()),
            ("REFORGED_ADMIN_KEY", s.admin_key.as_str()),
            ("GAMESERVER_API_KEY", s.gameserver_key.as_str()),
            ("API_KEY_HASH_SECRET", s.api_key_hash_secret.as_str()),
            ("MYSTICPARADOX_METRICS_TOKEN", s.metrics_token.as_str()),
            ("AUTH_SIGNING_PRIVKEY_B64", s.jwt_private_key_b64.as_str()),
            ("AUTH_SIGNING_PUBKEY_B64", s.jwt_public_key_b64.as_str()),
            ("REFORGED_ADMIN_API_KEY", s.admin_api_key.as_str()),
            ("MYSTICPARADOX_MANAGER_OPERATOR_SECRET", s.operator_secret.as_str()),
            ("REFORGED_DB_PASSWORD", s.database_password.as_str()),
        ];
        let header = [
            "DaintyLass server secrets. SECRET: never share or commit this file.",
            "Made once by `npm run setup`; changing one logs everyone out or breaks the invites already sent.",
        ];
        let text = format_env(&header, &entries)?;
        write_secret_file(&file, &text)?;
        return Ok((s, true));
    }

    let raw = std::fs::read_to_string(&file).map_err(|e| HostError::io(&file, e))?;
    let mut doc = EnvDocument::parse(&raw);
    let existing = doc.values();
    let missing = SECRET_KEYS
        .iter()
        .any(|k| existing.get(k).is_none_or(|v| v.trim().is_empty()));
    if missing {
        let fresh = make()?;
        let fresh_pairs = [
            ("GATEWAY_SECRET", fresh.gateway_secret.as_str()),
            ("ALLOWLIST_SECRET", fresh.allowlist_secret.as_str()),
            ("REFORGED_ADMIN_KEY", fresh.admin_key.as_str()),
            ("GAMESERVER_API_KEY", fresh.gameserver_key.as_str()),
            ("API_KEY_HASH_SECRET", fresh.api_key_hash_secret.as_str()),
            ("MYSTICPARADOX_METRICS_TOKEN", fresh.metrics_token.as_str()),
            ("AUTH_SIGNING_PRIVKEY_B64", fresh.jwt_private_key_b64.as_str()),
            ("AUTH_SIGNING_PUBKEY_B64", fresh.jwt_public_key_b64.as_str()),
            ("REFORGED_ADMIN_API_KEY", fresh.admin_api_key.as_str()),
            ("MYSTICPARADOX_MANAGER_OPERATOR_SECRET", fresh.operator_secret.as_str()),
            ("REFORGED_DB_PASSWORD", fresh.database_password.as_str()),
        ];
        for (k, v) in fresh_pairs {
            if existing.get(k).is_none_or(|cur| cur.trim().is_empty()) {
                doc.set(k, v)?;
            }
        }
        write_secret_file(&file, &doc.to_text())?;
    }
    let map = doc.values();
    let get = |k: &str| map.get(k).unwrap_or_default().to_owned();
    Ok((
        Secrets {
            gateway_secret: get("GATEWAY_SECRET"),
            allowlist_secret: get("ALLOWLIST_SECRET"),
            admin_key: get("REFORGED_ADMIN_KEY"),
            gameserver_key: get("GAMESERVER_API_KEY"),
            api_key_hash_secret: get("API_KEY_HASH_SECRET"),
            metrics_token: get("MYSTICPARADOX_METRICS_TOKEN"),
            jwt_private_key_b64: get("AUTH_SIGNING_PRIVKEY_B64"),
            jwt_public_key_b64: get("AUTH_SIGNING_PUBKEY_B64"),
            admin_api_key: get("REFORGED_ADMIN_API_KEY"),
            operator_secret: get("MYSTICPARADOX_MANAGER_OPERATOR_SECRET"),
            database_password: get("REFORGED_DB_PASSWORD"),
        },
        missing,
    ))
}

fn database_env(c: &HostConfig, s: &Secrets, p: &Paths) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(db) = &c.database
        && db.postgres()
    {
        out.push(("REFORGED_DB_ENGINE".into(), "postgres".into()));
        out.push((
            "REFORGED_DB_HOST".into(),
            db.postgres_host.clone().unwrap_or_else(|| "127.0.0.1".into()),
        ));
        out.push(("REFORGED_DB_PORT".into(), db.postgres_port.unwrap_or(5432).to_string()));
        out.push((
            "REFORGED_DB_USER".into(),
            db.postgres_user.clone().unwrap_or_else(|| "dainty".into()),
        ));
        out.push((
            "REFORGED_DB_NAME".into(),
            db.postgres_database.clone().unwrap_or_else(|| "dainty".into()),
        ));
        out.push(("REFORGED_DB_PASSWORD".into(), s.database_password.clone()));
        if db.allow_remote == Some(true) {
            out.push(("REFORGED_DB_ALLOW_REMOTE".into(), "1".into()));
        }
    } else {
        out.push(("REFORGED_DB_ENGINE".into(), "sqlite".into()));
        out.push(("REFORGED_DB_FILE".into(), path_text(&p.config.join(DATABASE_FILE))));
    }
    out
}

fn module_switch_env(c: &HostConfig) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let set = |out: &mut Vec<(String, String)>, k: &str, v: String| {
        if let Some((_, existing)) = out.iter_mut().find(|(ek, _)| ek == k) {
            *existing = v;
        } else {
            out.push((k.to_owned(), v));
        }
    };
    if let Some(modules) = &c.modules
        && let Ok(isles_id) = "isles-1-14-7".parse::<module_api::ModuleId>()
        && let Some(sections) = modules.settings.get(&isles_id)
        && let Some(switches) = sections.get("switches")
    {
        for (_, mod_key, env_key, _) in SWITCH_STRINGS {
            if let Some(val) = switches.get(mod_key).and_then(Value::as_str) {
                set(&mut out, env_key, val.to_owned());
            }
        }
        for (_, mod_key, env_key) in SWITCH_NUMBERS {
            if let Some(val) = switches.get(mod_key).and_then(Value::as_u64) {
                set(&mut out, env_key, val.to_string());
            }
        }
    }
    if let Some(switches) = &c.switches {
        for (env_key, val) in switches.env() {
            set(&mut out, env_key, val);
        }
    }
    out
}

#[must_use]
pub fn build_env_files(c: &HostConfig, s: &Secrets, p: &Paths, source_commit: &str, platform: Platform) -> EnvFiles {
    fn push(vec: &mut EnvEntries, k: &str, v: impl Into<String>) {
        vec.push((k.to_owned(), v.into()));
    }

    // 1. backend
    let mut backend = EnvEntries::new();
    push(&mut backend, "NODE_ENV", "production");
    push(&mut backend, "MYSTICPARADOX_SERVICE_ROLE", "api");
    push(&mut backend, "MYSTICPARADOX_METRICS_TOKEN", &s.metrics_token);
    push(&mut backend, "PORT", PORTS.backend.to_string());
    push(&mut backend, "BIND_HOST", "127.0.0.1");
    push(&mut backend, "REFORGED_HTTP_ONLY", "true");
    push(&mut backend, "QOS_PORT", "0");
    push(
        &mut backend,
        "QOS_TARGET_URL",
        format!("http://127.0.0.1:{}/QoS", PORTS.relay),
    );
    push(&mut backend, "AUTH_MODE", "APIKEY");
    push(&mut backend, "ALLOW_NO_AUTH_DEV_MODE", "false");
    push(&mut backend, "AUTH_SIGNING_PRIVKEY_B64", &s.jwt_private_key_b64);
    push(&mut backend, "AUTH_SIGNING_PUBKEY_B64", &s.jwt_public_key_b64);
    push(&mut backend, "API_KEY_HASH_SECRET", &s.api_key_hash_secret);
    push(&mut backend, "GAMESERVER_API_KEYS", &s.gameserver_key);
    if cfg!(feature = "paradox") {
        push(&mut backend, "MONGODB_URI", &c.mongo.uri);
        push(&mut backend, "MONGODB_DB", "reforged");
        push(&mut backend, "MONGODB_APP_NAME", "reforged-backend");
        push(&mut backend, "MONGODB_TRANSACTION_MAX_COMMIT_TIME_MS", "15000");
    }
    push(&mut backend, "TARGET_CHANGELIST", "647472");
    push(&mut backend, "APPROVED_EXECUTABLE_SHA256", PINNED_EXE_SHA256);
    push(&mut backend, "MATCHMAKING_MODE", "DEPLOYSERVER");
    push(
        &mut backend,
        "DEPLOYSERVER_URL",
        format!("127.0.0.1:{}", PORTS.director),
    );
    push(&mut backend, "HUNT_GO_NOW_MIN_DELAY_MS", "1");
    push(&mut backend, "HUNT_GO_NOW_MAX_DELAY_MS", "1000");
    push(&mut backend, "REALTIME_XMPP_ENABLED", "true");
    push(&mut backend, "REALTIME_XMPP_CAPTURE", "false");
    push(&mut backend, "REALTIME_XMPP_ALLOWED_HOSTS", &c.host);
    push(&mut backend, "REALTIME_XMPP_DEV_CITY_MUC", "false");
    push(&mut backend, "REGISTRATION_MODE", c.registration.as_str());
    push(&mut backend, "REFORGED_ADMIN_KEY", &s.admin_key);
    push(&mut backend, "MYSTICPARADOX_MANAGER_ADMIN", "on");
    push(
        &mut backend,
        "MYSTICPARADOX_MANAGER_OPERATOR_SECRET",
        &s.operator_secret,
    );
    push(&mut backend, "REFORGED_SERVER_NAME", &c.name);
    push(&mut backend, "REFORGED_SERVER_VERSION", "0.1.0");
    push(
        &mut backend,
        "REFORGED_SOURCE_URL",
        "https://github.com/FreezaMotoqueiro/Dauntless-Reforged-Revival",
    );
    push(&mut backend, "REFORGED_SOURCE_COMMIT", source_commit);
    push(&mut backend, "REFORGED_SERVER_MODE", c.mode.as_str());
    push(&mut backend, "REFORGED_SERVER_HOST", &c.host);
    if multipliers_on(c)
        && let Some(m) = c.multipliers
    {
        push(&mut backend, "REFORGED_XP_MULTIPLIER", jsjson::number(m.xp));
        push(&mut backend, "REFORGED_REWARD_MULTIPLIER", jsjson::number(m.rewards));
    }
    for (k, v) in module_switch_env(c) {
        push(&mut backend, &k, v);
    }
    push(
        &mut backend,
        "REFORGED_SESSION_SUPPLIES_FILE",
        path_text(&p.config.join("session-supplies.json")),
    );
    push(&mut backend, "LOG_LEVEL", "info");

    // 2. director
    let mut director = EnvEntries::new();
    let (first_port, last_port) = game_ports(c);
    let linux = platform == Platform::Linux;
    let perf = performance_options(c);
    let exe_rel = game_exe_relative();
    let exe_path = c.game_dir.as_deref().map(|d| Path::new(d).join(&exe_rel));
    let exe_text = exe_path.as_deref().map(path_text).unwrap_or_default();

    push(&mut director, "NODE_ENV", "production");
    push(&mut director, "PORT", PORTS.director.to_string());
    push(&mut director, "BIND_HOST", "127.0.0.1");
    push(&mut director, "MY_IP", &c.game_address);
    push(&mut director, "PORT_RANGE_BEGIN", first_port.to_string());
    push(&mut director, "PORT_RANGE_END", last_port.to_string());
    if linux {
        push(
            &mut director,
            "GAMESERVER_BINARY_PATH",
            path_text(&p.gameserver_wrapper),
        );
        let l_opts = linux_options(c);
        push(&mut director, "REFORGED_GAME_EXE", &exe_text);
        if let Some(wine) = l_opts.wine {
            push(&mut director, "REFORGED_WINE", wine);
        }
        if let Some(proton) = l_opts.proton {
            push(&mut director, "REFORGED_PROTON", proton);
        }
        push(&mut director, "REFORGED_WINEPREFIX", path_text(&p.wine_prefix));
        push(&mut director, "REFORGED_PROTON_DATA", path_text(&p.proton_data));
        push(&mut director, "REFORGED_XVFB", if l_opts.xvfb { "1" } else { "0" });
    } else {
        push(&mut director, "GAMESERVER_BINARY_PATH", &exe_text);
    }
    push(&mut director, "METAGAME_API_KEY", &s.gameserver_key);
    push(&mut director, "SECONDS_TO_WAIT_BETWEEN_GAMESERVER_STARTUP", "3");
    push(
        &mut director,
        "GAMESERVER_READY_TIMEOUT_MS",
        if linux { "90000" } else { "30000" },
    );
    push(
        &mut director,
        "MAX_CONCURRENT_HUNT_SERVERS",
        c.max_hunt_servers.to_string(),
    );
    push(
        &mut director,
        "TRAINING_ON_DEMAND",
        if perf.training_on_demand { "1" } else { "0" },
    );
    push(
        &mut director,
        "TRAINING_IDLE_STOP_MINUTES",
        if perf.training_on_demand {
            perf.training_idle_minutes.to_string()
        } else {
            "0".to_owned()
        },
    );
    if let Some(parent) = exe_path.as_deref().and_then(Path::parent) {
        push(&mut director, "GAMESERVER_RUNTIME_LOG_DIR", path_text(parent));
    }
    push(
        &mut director,
        "GAMESERVER_MEMORY_GUARD",
        if perf.memory_guard { "1" } else { "0" },
    );
    push(
        &mut director,
        "GAMESERVER_MIN_FREE_MB",
        perf.min_free_memory_mb.to_string(),
    );
    push(
        &mut director,
        "GAMESERVER_HUNT_MEMORY_MB",
        perf.hunt_memory_mb.to_string(),
    );
    push(&mut director, "REFORGED_SERVER_MAX_FPS", perf.server_fps.to_string());
    push(&mut director, "REFORGED_SERVER_IDLE_FPS", perf.idle_fps.to_string());
    if !perf.verbose_game_logs {
        push(&mut director, "GAMESERVER_LOG_CMDS", QUIET_GAME_LOG_CMDS);
    }
    push(&mut director, "SERVER_RUNTIME_AUTO_UPDATE", "false");
    push(&mut director, "SERVER_RUNTIME_UPDATE_REQUIRED", "false");
    push(&mut director, "GAMESERVER_LOG_DIR", path_text(&p.gameserver_logs));
    push(
        &mut director,
        "REFORGED_BACKEND_URL",
        format!("http://127.0.0.1:{}", PORTS.backend),
    );
    push(&mut director, "MYSTICPARADOX_RUNTIME_PROFILE", "production");
    push(&mut director, "LOG_LEVEL", "info");

    // 3. content
    let mut content = EnvEntries::new();
    push(&mut content, "PORT", PORTS.content.to_string());
    push(&mut content, "BIND_HOST", "127.0.0.1");
    push(
        &mut content,
        "BACKEND_URL",
        format!("http://127.0.0.1:{}", PORTS.backend),
    );
    push(&mut content, "CONTENT_RUNTIME_DIR", path_text(&p.runtime_publish));
    push(&mut content, "CONTENT_BRANDING_DIR", path_text(&p.branding));
    push(&mut content, "CONTENT_NEWS_FILE", path_text(&p.news_file));

    // 4. admin
    let mut admin = EnvEntries::new();
    push(&mut admin, "PORT", PORTS.admin.to_string());
    push(&mut admin, "BIND_HOST", "127.0.0.1");
    push(&mut admin, "BACKEND_URL", format!("http://127.0.0.1:{}", PORTS.backend));
    push(
        &mut admin,
        "REFORGED_DIRECTOR_URL",
        format!("http://127.0.0.1:{}", PORTS.director),
    );
    push(&mut admin, "REFORGED_ADMIN_API_KEY", &s.admin_api_key);
    push(&mut admin, "REFORGED_ADMIN_KEY", &s.admin_key);
    push(&mut admin, "MYSTICPARADOX_MANAGER_OPERATOR_SECRET", &s.operator_secret);
    push(
        &mut admin,
        "REFORGED_ADMIN_API_TIMEOUTS_FILE",
        path_text(&p.config.join("admin-timeouts.json")),
    );
    push(&mut admin, "REFORGED_MAX_HUNT_SERVERS", c.max_hunt_servers.to_string());
    push(&mut admin, "REFORGED_SERVER_MODE", c.mode.as_str());
    for (k, v) in database_env(c, s, p) {
        push(&mut admin, &k, v);
    }
    if let Some(g) = &c.guard {
        for (k, v) in g.env() {
            push(&mut admin, k, v);
        }
    }

    // 5. gateway & allowlist
    let mut gateway = EnvEntries::new();
    let allowlist = match c.mode {
        ServerMode::Public => {
            push(&mut gateway, "GATEWAY_TLS", "1");
            push(&mut gateway, "GATEWAY_BIND", "0.0.0.0");
            push(&mut gateway, "GATEWAY_PORT", gateway_port(c).to_string());
            push(&mut gateway, "GATEWAY_CERT", path_text(&p.cert_file));
            push(&mut gateway, "GATEWAY_KEY", path_text(&p.key_file));
            if let Some(net) = &c.network
                && (!net.nodes.is_empty()
                    || c.remote
                        .as_ref()
                        .is_some_and(|r| matches!(r.role.as_deref(), Some("controller" | "managed"))))
            {
                push(&mut gateway, "GATEWAY_NODE_ID", net.node.to_string());
                push(&mut gateway, "GATEWAY_NODE_REGION", net.region.to_string());
                push(
                    &mut gateway,
                    "GATEWAY_NODE_CERT",
                    path_text(&p.tls.join(NODE_CERT_FILE)),
                );
                push(&mut gateway, "GATEWAY_NODE_KEY", path_text(&p.tls.join(NODE_KEY_FILE)));
                push(&mut gateway, "GATEWAY_NODE_CA", path_text(&p.tls.join(CLUSTER_CA_FILE)));
                let mut peers = net.nodes.clone();
                if let Some(rem) = &c.remote
                    && let Some(peer) = rem.controller_peer(&net.node)
                    && !peers.iter().any(|existing| existing.id == peer.id)
                {
                    peers.push(peer);
                }
                if !peers.is_empty()
                    && let Ok(nodes_json) = serde_json::to_string(&peers)
                {
                    push(&mut gateway, "GATEWAY_NETWORK_NODES", nodes_json);
                }
            } else if let Some(rem) = &c.remote
                && matches!(rem.role.as_deref(), Some("controller" | "managed"))
            {
                let node = rem.effective_node();
                let region = rem.effective_region();
                push(&mut gateway, "GATEWAY_NODE_ID", node.to_string());
                push(&mut gateway, "GATEWAY_NODE_REGION", region.to_string());
                push(
                    &mut gateway,
                    "GATEWAY_NODE_CERT",
                    path_text(&p.tls.join(NODE_CERT_FILE)),
                );
                push(&mut gateway, "GATEWAY_NODE_KEY", path_text(&p.tls.join(NODE_KEY_FILE)));
                push(&mut gateway, "GATEWAY_NODE_CA", path_text(&p.tls.join(CLUSTER_CA_FILE)));
                if let Some(peer) = rem.controller_peer(&node)
                    && let Ok(nodes_json) = serde_json::to_string(&[peer])
                {
                    push(&mut gateway, "GATEWAY_NETWORK_NODES", nodes_json);
                }
            }
            push(&mut gateway, "GATEWAY_SECRET", &s.gateway_secret);
            push(
                &mut gateway,
                "GATEWAY_BACKEND_URL",
                format!("http://127.0.0.1:{}", PORTS.backend),
            );
            push(
                &mut gateway,
                "GATEWAY_CONTENT_URL",
                format!("http://127.0.0.1:{}", PORTS.content),
            );
            push(
                &mut gateway,
                "GATEWAY_WS_URL",
                format!("http://127.0.0.1:{}", PORTS.backend),
            );
            push(
                &mut gateway,
                "ALLOWLIST_URL",
                format!("http://127.0.0.1:{}", PORTS.allowlist),
            );
            push(&mut gateway, "ALLOWLIST_SECRET", &s.allowlist_secret);

            let mut al = EnvEntries::new();
            push(
                &mut al,
                "ALLOWLIST_DRY_RUN",
                if platform == Platform::Other { "1" } else { "0" },
            );
            push(
                &mut al,
                "ALLOWLIST_FIREWALL",
                if linux { "nftables" } else { "windows" },
            );
            if linux {
                push(&mut al, "ALLOWLIST_NFT_TABLE", "reforged_revival");
                if let Some(iface) = linux_options(c).firewall_interface {
                    push(&mut al, "ALLOWLIST_INTERFACE", iface);
                }
            }
            push(&mut al, "ALLOWLIST_BIND", "127.0.0.1");
            push(&mut al, "ALLOWLIST_PORT", PORTS.allowlist.to_string());
            push(&mut al, "ALLOWLIST_SECRET", &s.allowlist_secret);
            push(&mut al, "ALLOWLIST_PORTS", game_ports_text(c, "-"));
            push(
                &mut al,
                "ALLOWLIST_AUDIT_LOG",
                path_text(&p.logs.join("allowlist-audit.log")),
            );
            push(
                &mut al,
                "ALLOWLIST_STATE_FILE",
                path_text(&p.config.join("allowlist-state.json")),
            );
            Some(al)
        }
        ServerMode::Private => {
            push(&mut gateway, "GATEWAY_TLS", "0");
            push(&mut gateway, "GATEWAY_BIND", &c.game_address);
            push(&mut gateway, "GATEWAY_PORT", PORTS.private_gateway.to_string());
            push(&mut gateway, "GATEWAY_SECRET", &s.gateway_secret);
            push(
                &mut gateway,
                "GATEWAY_BACKEND_URL",
                format!("http://127.0.0.1:{}", PORTS.backend),
            );
            push(
                &mut gateway,
                "GATEWAY_CONTENT_URL",
                format!("http://127.0.0.1:{}", PORTS.content),
            );
            push(
                &mut gateway,
                "GATEWAY_WS_URL",
                format!("http://127.0.0.1:{}", PORTS.backend),
            );
            push(&mut gateway, "GATEWAY_ALLOWLIST", "0");
            None
        }
    };

    EnvFiles {
        backend,
        director,
        content,
        gateway,
        admin,
        allowlist,
    }
}

pub fn node_database_config(c: &HostConfig, p: &Paths) -> Result<store::DatabaseConfig, HostError> {
    if let Some(db) = &c.database
        && db.postgres()
    {
        let (secrets, _) = load_or_create_secrets(&p.config, new_secrets)?;
        return Ok(store::DatabaseConfig::Postgres(store::PostgresConfig {
            host: db.postgres_host.clone().unwrap_or_else(|| "127.0.0.1".to_owned()),
            port: db.postgres_port.unwrap_or(5432),
            user: db.postgres_user.clone().unwrap_or_else(|| "dainty".to_owned()),
            password: secrets.database_password,
            database: db.postgres_database.clone().unwrap_or_else(|| "dainty".to_owned()),
            allow_remote: db.allow_remote.unwrap_or(false),
        }));
    }
    Ok(store::DatabaseConfig::Sqlite {
        file: p.config.join(DATABASE_FILE),
    })
}

pub fn write_named_env_files(p: &Paths, files: &[(&str, Option<&EnvEntries>)]) -> Result<Vec<PathBuf>, HostError> {
    std::fs::create_dir_all(&p.config).map_err(|e| HostError::io(&p.config, e))?;
    let mut written = Vec::new();
    for &(name, entries) in files {
        let path = p.config.join(format!("{name}.env"));
        match entries {
            Some(list) => {
                let text = format_env(&ENV_FILE_HEADER, list)?;
                write_secret_file(&path, &text)?;
                written.push(path);
            }
            None => {
                if path.exists() {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    }
    Ok(written)
}

pub fn write_env_files(p: &Paths, files: &EnvFiles) -> Result<Vec<PathBuf>, HostError> {
    let named = files.files();
    write_named_env_files(p, &named)
}

pub fn read_env_file(file: &Path) -> Result<EnvMap, HostError> {
    let text = std::fs::read_to_string(file).map_err(|e| HostError::io(file, e))?;
    Ok(parse_env(&text))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use serde_json::json;

    #[test]
    fn ip_and_dns_validators_work() {
        assert!(is_ipv4("127.0.0.1"));
        assert!(!is_ipv4("127.0.0.01"));
        assert!(is_tailscale_ipv4("100.64.0.1"));
        assert!(is_tailscale_ipv4("100.127.255.254"));
        assert!(!is_tailscale_ipv4("100.63.0.1"));
        assert!(is_public_ipv4("203.0.113.10"));
        assert!(is_public_ipv4("8.8.8.8"));
        assert!(!is_public_ipv4("10.0.0.1"));
        assert!(!is_public_ipv4("192.168.1.1"));
        assert!(!is_public_ipv4("100.100.100.100"));
        assert!(is_dns_name("hunts.example.com"));
        assert!(is_dns_name("node-1.tailnet.ts.net"));
        assert!(!is_dns_name("localhost"));
        assert!(!is_dns_name("1.2.3.4"));
    }

    #[test]
    fn clean_server_name_strips_controls_and_truncates_utf16() {
        assert_eq!(clean_server_name("  Friday\t\nHunts  "), "Friday Hunts");
        let long = "a".repeat(60);
        assert_eq!(clean_server_name(&long).len(), 48);
    }

    #[test]
    fn validates_and_round_trips_private_and_public_configs() {
        let doc = json!({
            "version": 1,
            "mode": "private",
            "host": "100.101.102.103",
            "gameAddress": "100.101.102.103",
            "name": "Friday Hunts",
            "registration": "INVITECODE",
            "maxHuntServers": 4
        });
        let cfg = HostConfig::from_json(&doc).unwrap();
        assert_eq!(cfg.mode, ServerMode::Private);
        assert_eq!(gateway_port(&cfg), 61000);
        let toml = cfg.to_toml().unwrap();
        let back = HostConfig::from_toml(&toml).unwrap();
        assert_eq!(back.host, "100.101.102.103");
        assert_eq!(back.max_hunt_servers, 4);
    }
}
