// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The server's configuration: `settings.toml` (or a host.json, .yaml, .yml) read, validated and written back,
//! `secrets.env`, and the per-service env files `setup` builds from the two.

pub mod check;
pub mod discord;
pub mod env;
pub mod events;
pub mod guard;
pub mod logs;
pub mod modules;
pub mod overrides;
pub mod ports;
pub mod services;
pub mod split;
pub mod tunables;
pub mod yaml;

pub use guard::GuardConfig;
pub use modules::{ExternalModuleEntry, ModulesConfig, NetworkConfig};
pub use services::{ServiceEntry, ServicesConfig};

use std::path::{Path, PathBuf};

use dainty_service_kit::{js, jsjson};
use serde_json::{Map, Number, Value};

use crate::envfile::{EnvDocument, EnvMap, format_env, parse_env};
use crate::error::HostError;
use crate::paths::{MAX_HUNTS, PINNED_EXE_SHA256, PORTS, Paths, game_exe_relative, game_port_range};

/// The SQLite store's file name, in the config folder.
pub const DATABASE_FILE: &str = "dainty-lass.db";
/// The Linux options when settings.toml has no `linux` section.
pub const DEFAULT_LINUX: LinuxOptions = LinuxOptions {
    wine: None,
    proton: None,
    xvfb: true,
    firewall_interface: None,
};
/// The game-server options when settings.toml has no `performance` section.
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
/// `GAMESERVER_LOG_CMDS` for quiet game logs: the noisy categories turned down.
pub const QUIET_GAME_LOG_CMDS: &str = "LogPhoenixCharacter Warning, LogOnline Warning, LogLoad Off";
/// The largest XP or reward multiplier settings.toml takes.
pub const MAX_MULTIPLIER: f64 = 100.0;
/// The MongoDB a server migrates pre-cutover data from when settings.toml names none.
pub const DEFAULT_MONGO_URI: &str = "mongodb://127.0.0.1:27017/?replicaSet=rs0";
/// TCP ports the server's own services hold with `ports`; no module or public port may take one.
#[must_use]
pub fn reserved_ports(ports: &crate::paths::Ports) -> [u16; 8] {
    [
        ports.backend,
        ports.director,
        ports.content,
        ports.admin,
        ports.allowlist,
        ports.chat,
        ports.relay,
        ports.mongo,
    ]
}

/// The ports `c`'s services listen on: [`PORTS`] with settings.toml's `[ports]` over it.
#[must_use]
pub fn service_ports(c: &HostConfig) -> crate::paths::Ports {
    c.ports.unwrap_or_default().resolve()
}
/// `edition` is read only to refuse the 1.4.4 ones; `reborn` is dropped when the file is next written
pub const EDITION_REMOVED: &str = "edition \"legacy\" and \"both\" ran Dauntless Revived's 1.4.4, which this server no longer runs: remove edition from settings.toml (an old 1.4.4 server's accounts import with dainty-host legacy import revived)";
/// The largest settings.toml (or .json, .yaml) read, in bytes.
pub const MAX_HOST_CONFIG_BYTES: u64 = 256 * 1024;
/// The secrets file's name, in the config folder.
pub const SECRET_FILE: &str = "secrets.env";
/// Every key `secrets.env` holds; one missing or empty is made on the next setup, but for
/// [`PROVIDED_SECRET_KEYS`].
pub const SECRET_KEYS: [&str; 13] = [
    "GATEWAY_SECRET",
    "ALLOWLIST_SECRET",
    "DAINTY_ADMIN_KEY",
    "GAMESERVER_API_KEY",
    "API_KEY_HASH_SECRET",
    "RUSTIC_ISLES_METRICS_TOKEN",
    "AUTH_SIGNING_PRIVKEY_B64",
    "AUTH_SIGNING_PUBKEY_B64",
    "DAINTY_ADMIN_API_KEY",
    "RUSTIC_ISLES_MANAGER_OPERATOR_SECRET",
    "DAINTY_DB_PASSWORD",
    "DASHBOARD_INGEST_KEY",
    "DISCORD_BOT_TOKEN",
];
/// Secrets the host brings rather than setup makes: left empty until set (`setup --discord-bot-token`).
pub const PROVIDED_SECRET_KEYS: [&str; 1] = ["DISCORD_BOT_TOKEN"];
/// Secrets under their names from before the Rustic Isles rename: `setup` moves each to its new name with the same value,
/// never a fresh one, since a changed secret breaks every signed call (DX4)
pub const RENAMED_SECRET_KEYS: [(&str, &str); 5] = [
    // The prefix before the 2026-11-06 rename (a server the legacy importer carried over)
    ("REFORGED_ADMIN_KEY", "DAINTY_ADMIN_KEY"),
    ("REFORGED_ADMIN_API_KEY", "DAINTY_ADMIN_API_KEY"),
    ("REFORGED_DB_PASSWORD", "DAINTY_DB_PASSWORD"),
    ("MYSTICPARADOX_METRICS_TOKEN", "RUSTIC_ISLES_METRICS_TOKEN"),
    (
        "MYSTICPARADOX_MANAGER_OPERATOR_SECRET",
        "RUSTIC_ISLES_MANAGER_OPERATOR_SECRET",
    ),
];
/// The node certificate's file name, in `.workspace/tls`.
pub const NODE_CERT_FILE: &str = "node-cert.pem";
/// The node key's file name, in `.workspace/tls`.
pub const NODE_KEY_FILE: &str = "node-key.pem";
/// The cluster CA certificate's file name, in `.workspace/tls`.
pub const CLUSTER_CA_FILE: &str = "cluster-ca-cert.pem";
/// The session key's file name.
pub const SESSION_KEY_FILE: &str = "session-key";
/// The comment lines at the top of every env file setup writes.
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

/// `path` as env-file text, lossy where it is not UTF-8.
pub fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Whom the server is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerMode {
    /// Friends over Tailscale.
    Private,
    /// A server on the internet.
    Public,
}

impl ServerMode {
    /// The name settings.toml spells it with.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Public => "public",
        }
    }

    /// The mode settings.toml's `mode` names; `None` for anything else.
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

/// Who may register an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Registration {
    /// With an invite code.
    Invitecode,
    /// Anyone.
    Open,
    /// No one.
    None,
}

impl Registration {
    /// The name settings.toml spells it with.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Invitecode => "INVITECODE",
            Self::Open => "OPEN",
            Self::None => "NONE",
        }
    }

    /// The policy settings.toml's `registration` names (upper case); `None` for anything else.
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

/// The MongoDB a server migrates its pre-cutover player data from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MongoConfig {
    /// Its connection string.
    pub uri: String,
    /// Whether the server starts its own mongod.
    pub manage: bool,
    /// The mongod executable it starts; found on PATH or under Program Files when `None`.
    pub mongod: Option<String>,
}

/// A private test server's multipliers; 1.0 is the game's own rate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Multipliers {
    /// Multiplies the XP awarded.
    pub xp: f64,
    /// Multiplies the rewards given.
    pub rewards: f64,
}

impl Default for Multipliers {
    fn default() -> Self {
        Self { xp: 1.0, rewards: 1.0 }
    }
}

/// How the game servers run on Linux.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinuxOptions {
    /// The Wine executable; found on PATH when `None`.
    pub wine: Option<String>,
    /// The Proton executable, which runs them instead of Wine.
    pub proton: Option<String>,
    /// Whether they run under xvfb-run when there is no display.
    pub xvfb: bool,
    /// The network interface the firewall script guards; every interface when `None`.
    pub firewall_interface: Option<String>,
}

impl Default for LinuxOptions {
    fn default() -> Self {
        DEFAULT_LINUX
    }
}

/// settings.toml's `database`: where the store is, and whether to start empty.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DatabaseSettings {
    /// `postgres`, or SQLite when `None` or anything else.
    pub engine: Option<String>,
    /// The Postgres host; 127.0.0.1 when `None`.
    pub postgres_host: Option<String>,
    /// The Postgres port; 5432 when `None`.
    pub postgres_port: Option<u16>,
    /// The Postgres user; `dainty` when `None`.
    pub postgres_user: Option<String>,
    /// The Postgres database; `dainty` when `None`.
    pub postgres_database: Option<String>,
    /// Allows a Postgres host off this machine.
    pub allow_remote: Option<bool>,
    /// Starts with an empty store instead of migrating the MongoDB data.
    pub fresh_start: Option<bool>,
}

impl DatabaseSettings {
    /// Whether the store is Postgres.
    #[must_use]
    pub fn postgres(&self) -> bool {
        self.engine.as_deref() == Some("postgres")
    }

    /// Reads settings.toml's `database` table; keys it doesn't know are ignored.
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

    /// The table settings.toml writes, without the keys that are `None`.
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

/// How the game servers run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PerformanceOptions {
    /// The frame cap: 0 for none, or 10 to 240.
    pub server_fps: u32,
    /// The frame cap after 15 s without players: 0 for the same as `server_fps`.
    pub idle_fps: u32,
    /// Starts the Training Grounds when a player asks for them, not at boot.
    pub training_on_demand: bool,
    /// Minutes without players before an on-demand Training Grounds stops; 0 keeps it running.
    pub training_idle_minutes: u32,
    /// Keeps the game servers' full logs.
    pub verbose_game_logs: bool,
    /// Starts a new hunt only with `min_free_memory_mb + hunt_memory_mb` free.
    pub memory_guard: bool,
    /// Memory kept free for the system, in MB.
    pub min_free_memory_mb: u32,
    /// Memory one hunt needs, in MB.
    pub hunt_memory_mb: u32,
}

impl Default for PerformanceOptions {
    fn default() -> Self {
        DEFAULT_PERFORMANCE
    }
}

const SWITCH_STRINGS: [(&str, &str, &str, &[&str]); 8] = [
    ("reforge", "reforge", "RUSTIC_ISLES_REFORGE", &["on", "log", "off"]),
    (
        "masteryPersistence",
        "mastery_persistence",
        "RUSTIC_ISLES_MASTERY_PERSISTENCE",
        &["on", "off"],
    ),
    (
        "bountyPersistence",
        "bounty_persistence",
        "RUSTIC_ISLES_BOUNTY_PERSISTENCE",
        &["on", "off"],
    ),
    (
        "cooldownPersistence",
        "cooldown_persistence",
        "RUSTIC_ISLES_COOLDOWN_PERSISTENCE",
        &["on", "off"],
    ),
    (
        "accountErasure",
        "account_erasure",
        "RUSTIC_ISLES_ACCOUNT_ERASURE",
        &["on", "off"],
    ),
    ("logPurge", "log_purge", "RUSTIC_ISLES_LOG_PURGE", &["on", "off"]),
    ("rateLimit", "rate_limit", "RUSTIC_ISLES_RATE_LIMIT", &["on", "off"]),
    ("gauntlet", "gauntlet", "RUSTIC_ISLES_GAUNTLET", &["on", "off"]),
];

const SWITCH_NUMBERS: [(&str, &str, &str); 3] = [
    (
        "rateLimitPerMinute",
        "rate_limit_per_minute",
        "RUSTIC_ISLES_RATE_LIMIT_PER_MINUTE",
    ),
    (
        "gameserverRateLimitPerMinute",
        "gameserver_rate_limit_per_minute",
        "RUSTIC_ISLES_GAMESERVER_RATE_LIMIT_PER_MINUTE",
    ),
    (
        "hostRateLimitPerMinute",
        "host_rate_limit_per_minute",
        "RUSTIC_ISLES_HOST_RATE_LIMIT_PER_MINUTE",
    ),
];

/// settings.toml's `switches`: Rustic Isles' feature switches, each `None` for the module's default.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SwitchesConfig {
    /// `on`, `log` or `off`.
    pub reforge: Option<String>,
    /// `on` or `off`.
    pub mastery_persistence: Option<String>,
    /// `on` or `off`.
    pub bounty_persistence: Option<String>,
    /// `on` or `off`.
    pub cooldown_persistence: Option<String>,
    /// `on` or `off`.
    pub account_erasure: Option<String>,
    /// `on` or `off`.
    pub log_purge: Option<String>,
    /// `on` or `off`.
    pub rate_limit: Option<String>,
    /// Requests a player may make a minute.
    pub rate_limit_per_minute: Option<u32>,
    /// Requests a game server may make a minute.
    pub gameserver_rate_limit_per_minute: Option<u32>,
    /// Requests the host's own tools may make a minute.
    pub host_rate_limit_per_minute: Option<u32>,
    /// `on` or `off`: whether Gauntlet hunts are matched and its Slayer's Path node shown.
    pub gauntlet: Option<String>,
    /// `on`, `kills` or `off`: the game servers' play-test cheats (DAINTY_TEST_CHEATS), private servers only.
    pub test_cheats: Option<String>,
}

/// The words `test_cheats` takes: `on` is every cheat, `kills` one-hit Behemoth kills only.
const TEST_CHEATS_WORDS: [&str; 3] = ["on", "kills", "off"];

impl SwitchesConfig {
    /// Fails on a `switches` value that is not a table, or on a word or number a switch does not take.
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
        if let Some(v) = s.get("testCheats")
            && !v.as_str().is_some_and(|w| TEST_CHEATS_WORDS.contains(&w))
        {
            return Err(bad(format!(
                "switches.test_cheats must be one of \"on\", \"kills\", \"off\", not {}",
                shown(Some(v))
            )));
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

    /// Reads a validated `switches` table.
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
            gauntlet: word("gauntlet"),
            test_cheats: word("testCheats"),
        }
    }

    /// The table settings.toml writes, without the switches that are `None`.
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
            ("gauntlet", &self.gauntlet),
            ("testCheats", &self.test_cheats),
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

    /// The backend's `RUSTIC_ISLES_*` variables for the switches that are set.
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

    /// The Director's `DAINTY_TEST_CHEATS`, which every game server it starts inherits; `None` when off.
    #[must_use]
    pub fn test_cheats_env(&self) -> Option<&str> {
        self.test_cheats.as_deref().filter(|w| *w != "off")
    }

    /// The module's setting names and values for the switches that are set.
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

/// settings.toml's `history`: where the change log is shipped.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HistoryConfig {
    /// The peer node each change is shipped to; kept only here when `None`.
    pub ship_to_peer: Option<dainty_module_api::network::NodeId>,
}

impl HistoryConfig {
    /// Fails on a value that is not a table, or a `shipToPeer` that is no node id.
    pub fn from_json(value: &Value) -> Result<Self, HostError> {
        let obj = value.as_object().ok_or_else(|| bad("history must be an object"))?;
        let ship_to_peer = match obj.get("shipToPeer") {
            None | Some(Value::Null) => None,
            Some(v) => {
                let s = v
                    .as_str()
                    .ok_or_else(|| bad("history.shipToPeer must be a peer node id (a-z, 0-9 and -)"))?;
                let id: dainty_module_api::network::NodeId =
                    s.parse().map_err(|e| bad(format!("history.shipToPeer: {e}")))?;
                Some(id)
            }
        };
        Ok(Self { ship_to_peer })
    }

    /// The table settings.toml writes.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        if let Some(peer) = &self.ship_to_peer {
            out.insert("shipToPeer".into(), Value::String(peer.to_string()));
        }
        Value::Object(out)
    }
}

/// settings.toml's `remote`: this install's place in remote management (DD27).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoteConfig {
    /// `standalone`, `controller` or `managed`; standalone when `None`.
    pub role: Option<String>,
    /// This install's node id.
    pub node: Option<dainty_module_api::network::NodeId>,
    /// This install's region.
    pub region: Option<dainty_module_api::network::Region>,
    /// The controller's https:// URL; required when managed.
    pub controller_url: Option<String>,
    /// The controller's node id; required when managed.
    pub controller_node: Option<dainty_module_api::network::NodeId>,
    /// The controller certificate's SHA-256 fingerprint; required when managed.
    pub controller_cert_fp: Option<String>,
    /// Seconds between syncs with the controller, 1 to 86400.
    pub sync_interval_secs: Option<u64>,
    /// This install's label, 1 to 64 printable characters.
    pub label: Option<String>,
    /// Setting keys this install keeps its own value for over the controller's shared configuration.
    pub local_overrides: Vec<String>,
}

impl RemoteConfig {
    /// Fails on a value that is not a table, a field of the wrong form, or a managed install without its controller.
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
                let id: dainty_module_api::network::NodeId = s.parse().map_err(|e| bad(format!("remote.node: {e}")))?;
                Some(id)
            }
        };
        let region = match obj.get("region") {
            None | Some(Value::Null) => None,
            Some(v) => {
                let s = v
                    .as_str()
                    .ok_or_else(|| bad("remote.region must be a region name (a-z, 0-9 and -)"))?;
                let id: dainty_module_api::network::Region =
                    s.parse().map_err(|e| bad(format!("remote.region: {e}")))?;
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
                let id: dainty_module_api::network::NodeId =
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

    /// The table settings.toml writes, without the fields that are `None`.
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

    /// The node id, or `controller` or `managed` by role when none is set.
    #[must_use]
    pub fn effective_node(&self) -> dainty_module_api::network::NodeId {
        self.node.clone().unwrap_or_else(|| {
            let default_name = match self.role.as_deref() {
                Some("controller") => "controller",
                _ => "managed",
            };
            default_name
                .parse()
                .unwrap_or_else(|_| dainty_module_api::NodeIdentity::standalone().node)
        })
    }

    /// The region, or the standalone default.
    #[must_use]
    pub fn effective_region(&self) -> dainty_module_api::network::Region {
        self.region
            .clone()
            .unwrap_or_else(|| dainty_module_api::NodeIdentity::standalone().region)
    }

    /// The controller as a node-link peer, for a managed install that is not the controller itself.
    #[must_use]
    pub fn controller_peer(
        &self,
        local_node: &dainty_module_api::network::NodeId,
    ) -> Option<network_plugin::cluster::Peer> {
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
            link: None,
        })
    }
}

/// The file a [`HostConfig`] was read from, kept so writing it back keeps the host's order and comments.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileForm {
    /// The validated document, in the file's key order.
    pub document: Map<String, Value>,
    /// The document as read, before validation.
    pub read: Map<String, Value>,
    /// The settings.toml text as read; `None` for a JSON or YAML file.
    pub toml: Option<String>,
}

/// settings.toml: the server's configuration, validated.
#[derive(Debug, Clone, PartialEq)]
pub struct HostConfig {
    /// The file's format version.
    pub version: u32,
    /// Whom the server is for.
    pub mode: ServerMode,
    /// The address players reach: a Tailscale address or .ts.net name, or a public IPv4 or domain name.
    pub host: String,
    /// The IPv4 address the game servers announce.
    pub game_address: String,
    /// The server's name, as the launcher shows it.
    pub name: String,
    /// The Dauntless 1.14.7 folder the game servers run from.
    pub game_dir: Option<String>,
    /// Who may register.
    pub registration: Registration,
    /// A private server's Tailscale share link.
    pub share_url: Option<String>,
    /// The MongoDB pre-cutover data is migrated from.
    pub mongo: MongoConfig,
    /// The most hunts that run at once, 1 to 10.
    pub max_hunt_servers: u32,
    /// A private test server's multipliers; `None` when off.
    pub multipliers: Option<Multipliers>,
    /// The TCP port a public server's gateway listens on; 443 when `None`.
    pub public_port: Option<u16>,
    /// The staff dashboard chat sends its log and reports to, and content players' crash reports, with
    /// `DASHBOARD_INGEST_KEY` (dainty-dash)
    pub dashboard_url: Option<String>,
    /// The 1.14.7 version data the server reads at start; `None` is [`DEFAULT_VERSION_DATA`]. Relative to the
    /// server folder
    pub version_data: Option<String>,
    /// The 1.14.7 version facts beside it (rulings V-1 to V-3); `None` is [`DEFAULT_VERSION_FACTS`]. Relative to
    /// the server folder
    pub version_facts: Option<String>,
    /// How the game servers run; [`DEFAULT_PERFORMANCE`] when `None`.
    pub performance: Option<PerformanceOptions>,
    /// How the game servers run on Linux; [`DEFAULT_LINUX`] when `None`.
    pub linux: Option<LinuxOptions>,
    /// Where the store is.
    pub database: Option<DatabaseSettings>,
    /// `[guard]`: when a destructive or bulk admin action waits for a delay or a second operator (DH6).
    pub guard: Option<GuardConfig>,
    /// Where the change log is shipped.
    pub history: Option<HistoryConfig>,
    /// Rustic Isles' feature switches.
    pub switches: Option<SwitchesConfig>,
    /// `[modules]`: the game modules and plugins the server loads.
    pub modules: Option<ModulesConfig>,
    /// `[services.<name>]`: services loaded as libraries (RD27); `None` composes the compiled-in ones
    pub services: Option<ServicesConfig>,
    /// `[env.<service>]`: variables appended to a service's env file (RD24 phase 2)
    pub env: Option<env::EnvPassthrough>,
    /// `[gateway.limits]`, `[gateway.rate]`, `[gateway.timeouts]` and `[gateway]`: the gateway's tunables (RD24)
    pub gateway: Option<tunables::Tunables>,
    /// `[content]`: the content server's tunables (RD24)
    pub content: Option<tunables::Tunables>,
    /// `[allowlist]`: the allowlist helper's tunables, on a public server (RD24)
    pub allowlist: Option<tunables::Tunables>,
    /// `[backend]`: the backend's log level, go-now window and XMPP capture (RD24)
    pub backend: Option<tunables::Tunables>,
    /// `[director]`: the Director's log level and game server start timing (RD24)
    pub director: Option<tunables::Tunables>,
    /// `[network]`: the other nodes this server links to.
    pub network: Option<NetworkConfig>,
    /// `[remote]`: this install's place in remote management.
    pub remote: Option<RemoteConfig>,
    /// `[split]`: this server is one half of two, the metagame or the game servers (`docs/split-roles.md`)
    pub split: Option<split::SplitConfig>,
    /// `[logs]`: the console's format and color, and the GELF collector (`docs/logging.md`)
    pub logs: Option<logs::LogsConfig>,
    /// `[events]`: how long the registry's events are kept (module sharing A5)
    pub events: Option<events::EventsConfig>,
    /// `[discord]`: the Discord bot (module sharing A7); its token is in secrets.env
    pub discord: Option<discord::DiscordConfig>,
    /// `[ports]`: the loopback ports the services listen on, for a machine that holds a default
    pub ports: Option<ports::PortsConfig>,
    /// The file it was read from.
    pub file: FileForm,
}

impl HostConfig {
    /// Validates a host.json or host.yaml document.
    pub fn from_json(document: &Value) -> Result<Self, HostError> {
        let validated = validate(document)?;
        let read = document.as_object().cloned().unwrap_or_default();
        Self::from_validated(validated, read, None)
    }

    /// Validates settings.toml's text; the error names the key and the problem as settings.toml spells them.
    pub fn from_toml(text: &str) -> Result<Self, HostError> {
        let doc = crate::settingstoml::document(text)?;
        let validated = validate(&doc).map_err(|e| bad(crate::settingstoml::message(&e.to_string())))?;
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
        let dashboard_url = obj.get("dashboardUrl").and_then(Value::as_str).map(str::to_owned);
        let version_data = obj.get("versionData").and_then(Value::as_str).map(str::to_owned);
        let version_facts = obj.get("versionFacts").and_then(Value::as_str).map(str::to_owned);
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
        let services = match obj.get("services") {
            Some(s) if !s.is_null() => Some(services::from_json(s)?),
            _ => None,
        };
        let env = match obj.get("env") {
            Some(e) if !e.is_null() => Some(env::from_json(e)?),
            _ => None,
        };
        let tunable = |name: &str| match obj.get(name) {
            Some(t) if !t.is_null() => tunables::Tunables::from_json(name, t).map(Some),
            _ => Ok(None),
        };
        let (gateway, content, allowlist) = (tunable("gateway")?, tunable("content")?, tunable("allowlist")?);
        let (backend, director) = (tunable("backend")?, tunable("director")?);
        let network = match obj.get("network") {
            Some(n) if !n.is_null() => Some(NetworkConfig::from_json(n)?),
            _ => None,
        };
        let remote = match obj.get("remote") {
            Some(r) if !r.is_null() => Some(RemoteConfig::from_json(r)?),
            _ => None,
        };
        let split = match obj.get("split") {
            Some(s) if !s.is_null() => Some(split::SplitConfig::from_json(s)?),
            _ => None,
        };
        let logs = match obj.get("logs") {
            Some(l) if !l.is_null() => Some(logs::LogsConfig::from_json(l)?),
            _ => None,
        };
        let events = match obj.get("events") {
            Some(e) if !e.is_null() => Some(events::EventsConfig::from_json(e)?),
            _ => None,
        };
        let ports = match obj.get("ports") {
            Some(p) if !p.is_null() => Some(ports::PortsConfig::from_json(p)?),
            _ => None,
        };
        let discord = match obj.get("discord") {
            Some(d) if !d.is_null() => Some(discord::DiscordConfig::from_json(d)?),
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
            dashboard_url,
            version_data,
            version_facts,
            performance,
            linux,
            database,
            guard,
            history,
            switches,
            modules,
            services,
            env,
            gateway,
            content,
            allowlist,
            backend,
            director,
            network,
            remote,
            split,
            logs,
            events,
            discord,
            ports,
            file: FileForm {
                document: obj,
                read,
                toml,
            },
        })
    }

    /// Fills the sections this config leaves out from `previous`, and takes its file form, so writing it keeps them.
    #[must_use]
    pub fn written_over(mut self, previous: Option<&HostConfig>) -> Self {
        if let Some(prev) = previous {
            if self.database.is_none() {
                self.database = prev.database.clone();
            }
            if self.guard.is_none() {
                self.guard = prev.guard.clone();
            }
            for (mine, theirs) in [
                (&mut self.gateway, &prev.gateway),
                (&mut self.content, &prev.content),
                (&mut self.allowlist, &prev.allowlist),
                (&mut self.backend, &prev.backend),
                (&mut self.director, &prev.director),
            ] {
                if mine.is_none() {
                    mine.clone_from(theirs);
                }
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
            if self.services.is_none() {
                self.services = prev.services.clone();
            }
            if self.env.is_none() {
                self.env = prev.env.clone();
            }
            if self.network.is_none() {
                self.network = prev.network.clone();
            }
            if self.remote.is_none() {
                self.remote = prev.remote.clone();
            }
            if self.split.is_none() {
                self.split = prev.split.clone();
            }
            if self.logs.is_none() {
                self.logs = prev.logs.clone();
            }
            if self.events.is_none() {
                self.events = prev.events.clone();
            }
            if self.ports.is_none() {
                self.ports = prev.ports;
            }
            if self.discord.is_none() {
                self.discord = prev.discord.clone();
            }
            if self.dashboard_url.is_none() {
                self.dashboard_url = prev.dashboard_url.clone();
            }
            if self.version_data.is_none() {
                self.version_data = prev.version_data.clone();
            }
            if self.version_facts.is_none() {
                self.version_facts = prev.version_facts.clone();
            }
            self.file.read = prev.file.read.clone();
            self.file.toml = prev.file.toml.clone();
        }
        self
    }

    /// Whether the store starts empty instead of migrating the MongoDB data.
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
        if let Some(url) = &self.dashboard_url {
            canonical.insert("dashboardUrl".into(), Value::String(url.clone()));
        }
        if let Some(path) = &self.version_data {
            canonical.insert("versionData".into(), Value::String(path.clone()));
        }
        if let Some(path) = &self.version_facts {
            canonical.insert("versionFacts".into(), Value::String(path.clone()));
        }
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
        if let Some(s) = &self.services {
            canonical.insert("services".into(), services::to_json(s));
        }
        if let Some(e) = &self.env {
            canonical.insert("env".into(), env::to_json(e));
        }
        for (name, t) in self.tunables() {
            if let Some(t) = t {
                canonical.insert(name.into(), t.to_json());
            }
        }
        if let Some(sp) = &self.split {
            canonical.insert("split".into(), sp.to_json());
        }
        if let Some(l) = &self.logs {
            canonical.insert("logs".into(), l.to_json());
        }
        if let Some(e) = &self.events {
            canonical.insert("events".into(), e.to_json());
        }
        if let Some(p) = &self.ports {
            canonical.insert("ports".into(), p.to_json());
        }
        if let Some(d) = &self.discord {
            canonical.insert("discord".into(), d.to_json());
        }
        merge_document_order(&self.file.document, canonical)
    }

    /// settings.toml's text, laid over the file it was read from so the host's order and comments stay.
    pub fn to_toml(&self) -> Result<String, HostError> {
        let fields = self.fields_map();
        let validated =
            validate(&Value::Object(fields)).map_err(|e| bad(crate::settingstoml::message(&e.to_string())))?;
        let obj = validated
            .as_object()
            .ok_or_else(|| bad("settings.toml must hold a table"))?;
        let mut merged = obj.clone();
        // Preserve nullable fields that are None as Null so settingstoml::patch removes them if they were set before
        for k in ["gameDir", "shareUrl"] {
            if !merged.contains_key(k) {
                merged.insert(k.to_owned(), Value::Null);
            }
        }
        crate::settingstoml::text(&merged, &self.file.read, self.file.toml.as_deref())
    }

    /// The services' tunable tables, by name, in settings.toml's order.
    #[must_use]
    pub fn tunables(&self) -> [(&'static str, Option<&tunables::Tunables>); 5] {
        [
            ("gateway", self.gateway.as_ref()),
            ("content", self.content.as_ref()),
            ("allowlist", self.allowlist.as_ref()),
            ("backend", self.backend.as_ref()),
            ("director", self.director.as_ref()),
        ]
    }

    /// The validated document, as host.json holds it.
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

/// The server's secrets, as `secrets.env` holds them; never logged.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Secrets {
    /// `GATEWAY_SECRET`.
    pub gateway_secret: String,
    /// `ALLOWLIST_SECRET`.
    pub allowlist_secret: String,
    /// `DAINTY_ADMIN_KEY`.
    pub admin_key: String,
    /// `GAMESERVER_API_KEY`.
    pub gameserver_key: String,
    /// `API_KEY_HASH_SECRET`.
    pub api_key_hash_secret: String,
    /// `RUSTIC_ISLES_METRICS_TOKEN`.
    pub metrics_token: String,
    /// `AUTH_SIGNING_PRIVKEY_B64`: the private key tokens are signed with, base64.
    pub jwt_private_key_b64: String,
    /// `AUTH_SIGNING_PUBKEY_B64`: its public key, base64.
    pub jwt_public_key_b64: String,
    /// `DAINTY_ADMIN_API_KEY`.
    pub admin_api_key: String,
    /// `RUSTIC_ISLES_MANAGER_OPERATOR_SECRET`.
    pub operator_secret: String,
    /// `DAINTY_DB_PASSWORD`: the Postgres password.
    pub database_password: String,
    /// What chat sends the staff dashboard with; the dashboard's own `DASHBOARD_INGEST_KEY` is set to it
    pub dashboard_ingest_key: String,
    /// `DISCORD_BOT_TOKEN`: the Discord bot's (A7), from the host; empty until set.
    pub discord_bot_token: String,
}

/// The OS the env files are written for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// Windows.
    Windows,
    /// Linux.
    Linux,
    /// Anything else: no firewall to drive, so the allowlist runs dry.
    Other,
}

impl Platform {
    /// The OS this binary was built for.
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

/// An env file's variables, in the order written.
pub type EnvEntries = Vec<(String, String)>;

/// Every env file setup writes, by service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvFiles {
    /// backend.env.
    pub backend: EnvEntries,
    /// director.env.
    pub director: EnvEntries,
    /// content.env.
    pub content: EnvEntries,
    /// gateway.env.
    pub gateway: EnvEntries,
    /// admin.env.
    pub admin: EnvEntries,
    /// allowlist.env; `None` when the server runs no allowlist, and the file is then removed.
    pub allowlist: Option<EnvEntries>,
    /// discord.env; `None` unless `[discord]` is enabled and the token is set, and the file is then removed.
    pub discord: Option<EnvEntries>,
    /// `[env.<service>]` lines not written, each with why; `setup` refuses while there are any
    pub passthrough_refused: Vec<String>,
}

impl EnvFiles {
    /// Each file's name without `.env`, with its variables.
    #[must_use]
    pub fn files(&self) -> [(&'static str, Option<&EnvEntries>); 7] {
        [
            ("backend", Some(&self.backend)),
            ("director", Some(&self.director)),
            ("content", Some(&self.content)),
            ("gateway", Some(&self.gateway)),
            ("admin", Some(&self.admin)),
            ("allowlist", self.allowlist.as_ref()),
            ("discord", self.discord.as_ref()),
        ]
    }
}

/// Whether `host` names this machine: localhost, 127.0.0.1 or ::1.
#[must_use]
pub fn is_local_database_host(host: &str) -> bool {
    let trimmed = host.trim();
    trimmed.eq_ignore_ascii_case("localhost") || trimmed == "127.0.0.1" || trimmed == "::1"
}

/// The config's game-server options, or [`DEFAULT_PERFORMANCE`].
#[must_use]
pub fn performance_options(c: &HostConfig) -> PerformanceOptions {
    c.performance.unwrap_or(DEFAULT_PERFORMANCE)
}

/// Whether `value` is a whole number of minutes from 0 to 1440.
#[must_use]
pub fn valid_minutes(value: f64) -> bool {
    value.is_finite() && value.fract() == 0.0 && (0.0..=1440.0).contains(&value)
}

/// Whether `value` is a whole number of megabytes from `min` to 1048576.
#[must_use]
pub fn valid_megabytes(value: f64, min: f64) -> bool {
    value.is_finite() && value.fract() == 0.0 && value >= min && value <= 1_048_576.0
}

/// Whether `value` is 0 (no cap) or a whole frame rate from `min` to 240.
#[must_use]
pub fn valid_fps(value: f64, min: f64) -> bool {
    value.is_finite() && value.fract() == 0.0 && (value == 0.0 || (value >= min && value <= 240.0))
}

/// The first and last UDP port the game servers may use.
#[must_use]
pub fn game_ports(c: &HostConfig) -> (u16, u16) {
    game_port_range(c.max_hunt_servers)
}

/// The game ports as `first<separator>last`.
#[must_use]
pub fn game_ports_text(c: &HostConfig, separator: &str) -> String {
    let (first, last) = game_ports(c);
    format!("{first}{separator}{last}")
}

/// The 1.14.7 version data a release ships, in the server folder, when settings.toml has no `version_data` (RD27).
pub const DEFAULT_VERSION_DATA: &str = "data/1.14.7/version_data.json";

/// The 1.14.7 version facts a release ships beside [`DEFAULT_VERSION_DATA`] (rulings V-1 to V-3).
pub const DEFAULT_VERSION_FACTS: &str = "data/1.14.7/version_facts.json";

/// The version facts file the server reads: settings.toml's `version_facts` (relative to the server folder), else
/// [`DEFAULT_VERSION_FACTS`]; in a checkout, which has no `data/`, the MIT module's own copy. A host whose
/// `version_data` still names a whole document needs no facts file.
#[must_use]
pub fn version_facts_path(paths: &Paths, config: Option<&HostConfig>) -> PathBuf {
    if let Some(named) = config.and_then(|c| c.version_facts.as_deref()) {
        return paths.repo.join(named);
    }
    let shipped = paths.repo.join(DEFAULT_VERSION_FACTS);
    if shipped.exists() {
        return shipped;
    }
    // The checkout's root, from restricted/'s modules/rustic-isles
    let root = paths.isles.join("..").join("..").join("..");
    [paths.repo.as_path(), root.as_path()]
        .into_iter()
        .map(|dir| {
            dir.join("crates")
                .join("rustic-isles-1-14-7")
                .join("data")
                .join("version_facts.json")
        })
        .find(|file| file.is_file())
        .unwrap_or(shipped)
}

/// The version data file the server reads: settings.toml's `version_data` (relative to the server folder), else
/// [`DEFAULT_VERSION_DATA`]; in a checkout, which has no `data/`, restricted's own copy.
#[must_use]
pub fn version_data_path(paths: &Paths, config: Option<&HostConfig>) -> PathBuf {
    if let Some(named) = config.and_then(|c| c.version_data.as_deref()) {
        return paths.repo.join(named);
    }
    let shipped = paths.repo.join(DEFAULT_VERSION_DATA);
    let checkout = paths
        .isles
        .join("version-data")
        .join("1.14.7")
        .join("version_data.json");
    if !shipped.exists() && checkout.is_file() {
        checkout
    } else {
        shipped
    }
}

/// The TCP port the gateway listens on.
#[must_use]
pub fn gateway_port(c: &HostConfig) -> u16 {
    match c.mode {
        ServerMode::Public => c.public_port.unwrap_or(PORTS.public_gateway),
        ServerMode::Private => service_ports(c).private_gateway,
    }
}

/// Whether a multiplier other than 1.0 is set.
#[must_use]
pub fn multipliers_on(c: &HostConfig) -> bool {
    c.multipliers
        .is_some_and(|m| (m.xp - 1.0).abs() > f64::EPSILON || (m.rewards - 1.0).abs() > f64::EPSILON)
}

/// The config's Linux options, or [`DEFAULT_LINUX`].
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

/// Whether `host` is a dotted IPv4 address, without leading zeros.
#[must_use]
pub fn is_ipv4(host: &str) -> bool {
    parse_ipv4_octets(host).is_some()
}

/// Whether `host` is a Tailscale address (100.64.0.0/10).
#[must_use]
pub fn is_tailscale_ipv4(host: &str) -> bool {
    let Some([a, b, _, _]) = parse_ipv4_octets(host) else {
        return false;
    };
    a == 100 && (64..=127).contains(&b)
}

/// Whether `host` is an IPv4 address outside the private, loopback, link-local, Tailscale and multicast ranges.
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

/// Whether `host` is a DNS name: two labels or more, and a top-level label that is not all digits.
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

/// Whether `name` can be a Linux network interface name.
#[must_use]
pub fn is_linux_interface(name: &str) -> bool {
    let bytes = name.as_bytes();
    (1..=15).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || matches!(*b, b'_' | b'.' | b'-'))
}

/// `name` with control characters replaced, white space collapsed and cut to 48 UTF-16 units, as the launcher shows it.
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

/// Whether `url` is a Tailscale link on login.tailscale.com.
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

fn is_reserved_or_game_port(own: &ports::PortsConfig, port: u16) -> bool {
    let ports = own.resolve();
    // The private gateway's too, so a server moved between modes keeps its ports apart
    reserved_ports(&ports).contains(&port)
        || port == ports.qos
        || port == ports.private_gateway
        || (PORTS.game_lowest..=PORTS.game_last).contains(&port)
}

/// Checks a host.json document and returns it in canonical form, keeping its key order; the error says what is wrong.
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

    let ports_cfg = match obj.get("ports").filter(|v| !v.is_null()) {
        Some(p) => Some(ports::PortsConfig::from_json(p)?),
        None => None,
    };
    let own = ports_cfg.unwrap_or_default();

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
            if is_reserved_or_game_port(&own, port) {
                return Err(bad(format!(
                    "publicPort {port} is one the server uses itself: pick another (4443, for example)"
                )));
            }
            Some(port)
        }
    };

    if mode == ServerMode::Public && own.gateway.is_some() {
        return Err(bad(
            "ports.gateway is a private server's: a public one's is public_port",
        ));
    }
    let gateway = public_port.unwrap_or(match mode {
        ServerMode::Public => PORTS.public_gateway,
        ServerMode::Private => own.resolve().private_gateway,
    });
    for (key, port) in own.named() {
        let taken = if key == "gateway" {
            None
        } else if port == gateway {
            Some("the gateway's")
        } else if port == PORTS.relay {
            Some("the relay's")
        } else if port == PORTS.mongo {
            Some("MongoDB's")
        } else if (PORTS.game_lowest..=PORTS.game_last).contains(&port) {
            Some("a game server's")
        } else {
            None
        };
        if let Some(owner) = taken {
            return Err(bad(format!("ports.{key} {port} is {owner} port: pick another")));
        }
    }

    let dashboard_url = match obj.get("dashboardUrl") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if js::trim(s).is_empty() => None,
        Some(Value::String(s))
            if url::Url::parse(js::trim(s)).is_ok_and(|u| {
                matches!(u.scheme(), "http" | "https") && u.host().is_some() && u.username().is_empty()
            }) =>
        {
            Some(js::trim(s).trim_end_matches('/').to_owned())
        }
        Some(v) => {
            return Err(bad(format!(
                "dashboardUrl must be the dashboard's http:// or https:// address, not {}",
                shown(Some(v))
            )));
        }
    };

    let version_data = match obj.get("versionData") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if !js::trim(s).is_empty() => Some(js::trim(s).to_owned()),
        Some(v) => {
            return Err(bad(format!(
                "versionData must be the path of the 1.14.7 version_data.json, not {}",
                shown(Some(v))
            )));
        }
    };
    let version_facts = match obj.get("versionFacts") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if !js::trim(s).is_empty() => Some(js::trim(s).to_owned()),
        Some(v) => {
            return Err(bad(format!(
                "versionFacts must be the path of the 1.14.7 version_facts.json, not {}",
                shown(Some(v))
            )));
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
        if mode == ServerMode::Public && s.get("testCheats").and_then(Value::as_str).is_some_and(|w| w != "off") {
            return Err(bad("switches.test_cheats is for private (Tailscale) test servers only"));
        }
    }
    let modules_cfg = match obj.get("modules").filter(|v| !v.is_null()) {
        Some(m) => Some(ModulesConfig::from_json(m)?),
        None => None,
    };
    let services_cfg = match obj.get("services").filter(|v| !v.is_null()) {
        Some(s) => Some(services::from_json(s)?),
        None => None,
    };
    let env_cfg = match obj.get("env").filter(|v| !v.is_null()) {
        Some(e) => Some(env::from_json(e)?),
        None => None,
    };
    let mut tunables_cfg = Vec::new();
    for t in tunables::TABLES {
        if let Some(v) = obj.get(t.name).filter(|v| !v.is_null()) {
            tunables_cfg.push((t.name, tunables::Tunables::from_json(t.name, v)?));
        }
    }
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

    let split_cfg = match obj.get("split").filter(|v| !v.is_null()) {
        Some(sp) => {
            let split = split::SplitConfig::from_json(sp)?;
            split.check(mode, network_cfg.as_ref())?;
            Some(split)
        }
        None => None,
    };

    let logs_cfg = match obj.get("logs").filter(|v| !v.is_null()) {
        Some(l) => Some(logs::LogsConfig::from_json(l)?),
        None => None,
    };
    let events_cfg = match obj.get("events").filter(|v| !v.is_null()) {
        Some(e) => Some(events::EventsConfig::from_json(e)?),
        None => None,
    };
    let discord_cfg = match obj.get("discord").filter(|v| !v.is_null()) {
        Some(d) => Some(discord::DiscordConfig::from_json(d)?),
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
    if let Some(url) = dashboard_url {
        canonical.insert("dashboardUrl".into(), Value::String(url));
    }
    if let Some(path) = version_data {
        canonical.insert("versionData".into(), Value::String(path));
    }
    if let Some(path) = version_facts {
        canonical.insert("versionFacts".into(), Value::String(path));
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
    if let Some(s) = services_cfg {
        canonical.insert("services".into(), services::to_json(&s));
    }
    if let Some(e) = env_cfg {
        canonical.insert("env".into(), env::to_json(&e));
    }
    for (name, t) in tunables_cfg {
        canonical.insert(name.into(), t.to_json());
    }
    if let Some(sp) = split_cfg {
        canonical.insert("split".into(), sp.to_json());
    }
    if let Some(l) = logs_cfg {
        canonical.insert("logs".into(), l.to_json());
    }
    if let Some(e) = events_cfg {
        canonical.insert("events".into(), e.to_json());
    }
    if let Some(p) = ports_cfg {
        canonical.insert("ports".into(), p.to_json());
    }
    if let Some(d) = discord_cfg {
        canonical.insert("discord".into(), d.to_json());
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
        "dashboardUrl",
        "versionData",
        "versionFacts",
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
        "services",
        "env",
        "gateway",
        "content",
        "allowlist",
        "backend",
        "director",
        "split",
        "logs",
        "events",
        "discord",
        "ports",
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

/// `value` as `JSON.stringify(value, null, 2)` writes it.
#[must_use]
pub fn stringify_pretty(value: &Value) -> String {
    format_js_pretty(&jsjson::to_js(value), 0)
}

/// Reads the config at `file`, by its extension; `None` when there is no file.
/// Fails on a file over [`MAX_HOST_CONFIG_BYTES`] or one that doesn't parse or validate.
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

/// Reads the workspace's settings.toml; `None` before the first setup. A Node kit's host.json alone is refused.
pub fn load_host_config(p: &Paths) -> Result<Option<HostConfig>, HostError> {
    let old = p.config.join("host.json");
    if !p.settings_toml.is_file() && old.is_file() {
        return Err(HostError::message(format!(
            "{} is a Node kit's: run `dainty-legacy import reforged <checkout>` first",
            old.display()
        )));
    }
    read_host_config(&p.settings_toml)
}

/// Whether the workspace has a settings.toml.
#[must_use]
pub fn has_host_config(p: &Paths) -> bool {
    p.settings_toml.is_file()
}

/// The config as host.json text.
pub fn host_config_text(c: &HostConfig) -> Result<String, HostError> {
    Ok(format!("{}\n", stringify_pretty(&c.to_json()?)))
}

/// Writes the config to `file`, as JSON for a `.json` file and as settings.toml otherwise.
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

/// Writes the workspace's settings.toml.
pub fn save_host_config(p: &Paths, c: &HostConfig) -> Result<(), HostError> {
    write_host_config(&p.settings_toml, c)
}

/// Fresh random secrets.
pub fn new_secrets() -> Result<Secrets, HostError> {
    crate::commands::setup::secrets::new_secrets()
}

/// Writes `text` to `file`, readable by its owner only on Unix.
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
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::write(file, text).map_err(|e| HostError::io(file, e))
    }
}

/// The secrets in `config_dir`'s `secrets.env`, and whether the file was made now.
/// A new file gets `make`'s secrets; a missing or empty key is filled from it and old names are renamed (DX4).
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
            ("DAINTY_ADMIN_KEY", s.admin_key.as_str()),
            ("GAMESERVER_API_KEY", s.gameserver_key.as_str()),
            ("API_KEY_HASH_SECRET", s.api_key_hash_secret.as_str()),
            ("RUSTIC_ISLES_METRICS_TOKEN", s.metrics_token.as_str()),
            ("AUTH_SIGNING_PRIVKEY_B64", s.jwt_private_key_b64.as_str()),
            ("AUTH_SIGNING_PUBKEY_B64", s.jwt_public_key_b64.as_str()),
            ("DAINTY_ADMIN_API_KEY", s.admin_api_key.as_str()),
            ("RUSTIC_ISLES_MANAGER_OPERATOR_SECRET", s.operator_secret.as_str()),
            ("DAINTY_DB_PASSWORD", s.database_password.as_str()),
            ("DASHBOARD_INGEST_KEY", s.dashboard_ingest_key.as_str()),
        ];
        let header = [
            "Dainty Lass server secrets. SECRET: never share or commit this file.",
            "Made once by `npm run setup`; changing one logs everyone out or breaks the invites already sent.",
        ];
        let text = format_env(&header, &entries)?;
        write_secret_file(&file, &text)?;
        return Ok((s, true));
    }

    let raw = std::fs::read_to_string(&file).map_err(|e| HostError::io(&file, e))?;
    let mut doc = EnvDocument::parse(&raw);
    let renamed = rename_secrets(&mut doc)?;
    let existing = doc.values();
    let missing = SECRET_KEYS
        .iter()
        .filter(|k| !PROVIDED_SECRET_KEYS.contains(k))
        .any(|k| existing.get(k).is_none_or(|v| v.trim().is_empty()));
    if renamed && !missing {
        write_secret_file(&file, &doc.to_text())?;
    }
    if missing {
        let fresh = make()?;
        let fresh_pairs = [
            ("GATEWAY_SECRET", fresh.gateway_secret.as_str()),
            ("ALLOWLIST_SECRET", fresh.allowlist_secret.as_str()),
            ("DAINTY_ADMIN_KEY", fresh.admin_key.as_str()),
            ("GAMESERVER_API_KEY", fresh.gameserver_key.as_str()),
            ("API_KEY_HASH_SECRET", fresh.api_key_hash_secret.as_str()),
            ("RUSTIC_ISLES_METRICS_TOKEN", fresh.metrics_token.as_str()),
            ("AUTH_SIGNING_PRIVKEY_B64", fresh.jwt_private_key_b64.as_str()),
            ("AUTH_SIGNING_PUBKEY_B64", fresh.jwt_public_key_b64.as_str()),
            ("DAINTY_ADMIN_API_KEY", fresh.admin_api_key.as_str()),
            ("RUSTIC_ISLES_MANAGER_OPERATOR_SECRET", fresh.operator_secret.as_str()),
            ("DAINTY_DB_PASSWORD", fresh.database_password.as_str()),
            ("DASHBOARD_INGEST_KEY", fresh.dashboard_ingest_key.as_str()),
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
            admin_key: get("DAINTY_ADMIN_KEY"),
            gameserver_key: get("GAMESERVER_API_KEY"),
            api_key_hash_secret: get("API_KEY_HASH_SECRET"),
            metrics_token: get("RUSTIC_ISLES_METRICS_TOKEN"),
            jwt_private_key_b64: get("AUTH_SIGNING_PRIVKEY_B64"),
            jwt_public_key_b64: get("AUTH_SIGNING_PUBKEY_B64"),
            admin_api_key: get("DAINTY_ADMIN_API_KEY"),
            operator_secret: get("RUSTIC_ISLES_MANAGER_OPERATOR_SECRET"),
            database_password: get("DAINTY_DB_PASSWORD"),
            dashboard_ingest_key: get("DASHBOARD_INGEST_KEY"),
            discord_bot_token: get("DISCORD_BOT_TOKEN"),
        },
        missing,
    ))
}

/// Sets one of [`PROVIDED_SECRET_KEYS`] in `config_dir`'s `secrets.env`, keeping the rest of the file as it is.
pub fn set_provided_secret(config_dir: &Path, key: &str, value: &str) -> Result<(), HostError> {
    if !PROVIDED_SECRET_KEYS.contains(&key) {
        return Err(bad(format!("{key} is not a secret the host sets")));
    }
    let file = config_dir.join(SECRET_FILE);
    let raw = std::fs::read_to_string(&file).map_err(|e| HostError::io(&file, e))?;
    let mut doc = EnvDocument::parse(&raw);
    doc.set(key, value)?;
    write_secret_file(&file, &doc.to_text())
}

/// Moves each of [`RENAMED_SECRET_KEYS`] the file has only under its old name to the new one; whether any moved.
fn rename_secrets(doc: &mut EnvDocument) -> Result<bool, HostError> {
    let mut renamed = false;
    for (old, new) in RENAMED_SECRET_KEYS {
        let present = |key: &str| doc.get(key).filter(|v| !v.trim().is_empty());
        if present(new).is_some() {
            continue;
        }
        if present(old).is_some() {
            // In place, so the file keeps its order (and the Node kit's bytes, but for the name)
            doc.remove(new);
            doc.rename(old, new)?;
            renamed = true;
        }
    }
    Ok(renamed)
}

fn database_env(c: &HostConfig, s: &Secrets, p: &Paths) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(db) = &c.database
        && db.postgres()
    {
        out.push(("DAINTY_DB_ENGINE".into(), "postgres".into()));
        out.push((
            "DAINTY_DB_HOST".into(),
            db.postgres_host.clone().unwrap_or_else(|| "127.0.0.1".into()),
        ));
        out.push(("DAINTY_DB_PORT".into(), db.postgres_port.unwrap_or(5432).to_string()));
        out.push((
            "DAINTY_DB_USER".into(),
            db.postgres_user.clone().unwrap_or_else(|| "dainty".into()),
        ));
        out.push((
            "DAINTY_DB_NAME".into(),
            db.postgres_database.clone().unwrap_or_else(|| "dainty".into()),
        ));
        out.push(("DAINTY_DB_PASSWORD".into(), s.database_password.clone()));
        if db.allow_remote == Some(true) {
            out.push(("DAINTY_DB_ALLOW_REMOTE".into(), "1".into()));
        }
    } else {
        out.push(("DAINTY_DB_ENGINE".into(), "sqlite".into()));
        out.push(("DAINTY_DB_FILE".into(), path_text(&p.config.join(DATABASE_FILE))));
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
        && let Ok(isles_id) = "isles-1-14-7".parse::<dainty_module_api::ModuleId>()
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

/// Every env file's variables for this config and these secrets.
#[must_use]
pub fn build_env_files(c: &HostConfig, s: &Secrets, p: &Paths, source_commit: &str, platform: Platform) -> EnvFiles {
    let own = service_ports(c);
    fn push(vec: &mut EnvEntries, k: &str, v: impl Into<String>) {
        vec.push((k.to_owned(), v.into()));
    }

    // 1. backend
    let mut backend = EnvEntries::new();
    push(&mut backend, "NODE_ENV", "production");
    push(&mut backend, "RUSTIC_ISLES_SERVICE_ROLE", "api");
    push(&mut backend, "RUSTIC_ISLES_METRICS_TOKEN", &s.metrics_token);
    push(&mut backend, "PORT", own.backend.to_string());
    push(&mut backend, "BIND_HOST", "127.0.0.1");
    push(&mut backend, "DAINTY_HTTP_ONLY", "true");
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
        push(&mut backend, "MONGODB_APP_NAME", "dainty-backend");
        push(&mut backend, "MONGODB_TRANSACTION_MAX_COMMIT_TIME_MS", "15000");
    }
    push(&mut backend, "TARGET_CHANGELIST", "647472");
    push(&mut backend, "APPROVED_EXECUTABLE_SHA256", PINNED_EXE_SHA256);
    push(&mut backend, "MATCHMAKING_MODE", "DEPLOYSERVER");
    push(&mut backend, "DEPLOYSERVER_URL", format!("127.0.0.1:{}", own.director));
    push(&mut backend, "HUNT_GO_NOW_MIN_DELAY_MS", "1");
    push(&mut backend, "HUNT_GO_NOW_MAX_DELAY_MS", "1000");
    push(&mut backend, "REALTIME_XMPP_ENABLED", "true");
    push(&mut backend, "REALTIME_XMPP_CAPTURE", "false");
    push(&mut backend, "REALTIME_XMPP_ALLOWED_HOSTS", &c.host);
    push(&mut backend, "REALTIME_XMPP_DEV_CITY_MUC", "false");
    push(&mut backend, "REGISTRATION_MODE", c.registration.as_str());
    push(&mut backend, "DAINTY_ADMIN_KEY", &s.admin_key);
    push(&mut backend, "RUSTIC_ISLES_MANAGER_ADMIN", "on");
    push(&mut backend, "RUSTIC_ISLES_MANAGER_OPERATOR_SECRET", &s.operator_secret);
    push(&mut backend, "DAINTY_SERVER_NAME", &c.name);
    push(&mut backend, "DAINTY_SERVER_VERSION", "0.1.0");
    push(
        &mut backend,
        "DAINTY_SOURCE_URL",
        "https://github.com/SilentSpud/DaintyLass",
    );
    push(&mut backend, "DAINTY_SOURCE_COMMIT", source_commit);
    push(&mut backend, "DAINTY_SERVER_MODE", c.mode.as_str());
    push(&mut backend, "DAINTY_SERVER_HOST", &c.host);
    if multipliers_on(c)
        && let Some(m) = c.multipliers
    {
        push(&mut backend, "DAINTY_XP_MULTIPLIER", jsjson::number(m.xp));
        push(&mut backend, "DAINTY_REWARD_MULTIPLIER", jsjson::number(m.rewards));
    }
    for (k, v) in module_switch_env(c) {
        push(&mut backend, &k, v);
    }
    push(
        &mut backend,
        "DAINTY_SESSION_SUPPLIES_FILE",
        path_text(&crate::hostfiles::supplies_json(p)),
    );
    // The other half's port is the node-link tunnel (docs/split-roles.md)
    if let Some(split) = &c.split {
        push(&mut backend, "RUSTIC_ISLES_PORT_SERVICES", split.role.port_service());
    }
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
    push(&mut director, "PORT", own.director.to_string());
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
        push(&mut director, "DAINTY_GAME_EXE", &exe_text);
        if let Some(wine) = l_opts.wine {
            push(&mut director, "DAINTY_WINE", wine);
        }
        if let Some(proton) = l_opts.proton {
            push(&mut director, "DAINTY_PROTON", proton);
        }
        push(&mut director, "DAINTY_WINEPREFIX", path_text(&p.wine_prefix));
        push(&mut director, "DAINTY_PROTON_DATA", path_text(&p.proton_data));
        push(&mut director, "DAINTY_XVFB", if l_opts.xvfb { "1" } else { "0" });
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
    // The Director injects the published runtime into every game server (RD9 as of 2026-11-13)
    push(&mut director, "GAMESERVER_RUNTIME_DIR", path_text(&p.runtime_publish));
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
    push(&mut director, "DAINTY_SERVER_MAX_FPS", perf.server_fps.to_string());
    push(&mut director, "DAINTY_SERVER_IDLE_FPS", perf.idle_fps.to_string());
    // The names a runtime built before the 2026-11-06 rename reads; they go once a rebuilt one is published (P5)
    push(&mut director, "REFORGED_SERVER_MAX_FPS", perf.server_fps.to_string());
    push(&mut director, "REFORGED_SERVER_IDLE_FPS", perf.idle_fps.to_string());
    if !perf.verbose_game_logs {
        push(&mut director, "GAMESERVER_LOG_CMDS", QUIET_GAME_LOG_CMDS);
    }
    // Game servers inherit the Director's environment; validate refused it on a public server
    if let Some(cheats) = c.switches.as_ref().and_then(SwitchesConfig::test_cheats_env) {
        push(&mut director, "DAINTY_TEST_CHEATS", cheats);
    }
    push(&mut director, "SERVER_RUNTIME_AUTO_UPDATE", "false");
    push(&mut director, "SERVER_RUNTIME_UPDATE_REQUIRED", "false");
    push(&mut director, "GAMESERVER_LOG_DIR", path_text(&p.gameserver_logs));
    push(
        &mut director,
        "DAINTY_BACKEND_URL",
        format!("http://127.0.0.1:{}", own.backend),
    );
    // What the runtime reads first (rustic-isles-runtime-core's config::backend::PRIMARY)
    push(
        &mut director,
        "RUSTIC_ISLES_BACKEND_URL",
        format!("http://127.0.0.1:{}", own.backend),
    );
    push(&mut director, "RUSTIC_ISLES_RUNTIME_PROFILE", "production");
    push(&mut director, "LOG_LEVEL", "info");

    // 3. content
    let mut content = EnvEntries::new();
    push(&mut content, "PORT", own.content.to_string());
    push(&mut content, "BIND_HOST", "127.0.0.1");
    push(&mut content, "BACKEND_URL", format!("http://127.0.0.1:{}", own.backend));
    push(&mut content, "CONTENT_RUNTIME_DIR", path_text(&p.runtime_publish));
    push(&mut content, "CONTENT_BRANDING_DIR", path_text(&p.branding));
    push(&mut content, "CONTENT_NEWS_FILE", path_text(&p.news_file));
    push(&mut content, "CONTENT_CRASH_DIR", path_text(&p.crash_reports()));

    // 4. admin
    let mut admin = EnvEntries::new();
    push(&mut admin, "PORT", own.admin.to_string());
    push(&mut admin, "BIND_HOST", "127.0.0.1");
    push(&mut admin, "BACKEND_URL", format!("http://127.0.0.1:{}", own.backend));
    push(
        &mut admin,
        "DAINTY_DIRECTOR_URL",
        format!("http://127.0.0.1:{}", own.director),
    );
    push(&mut admin, "DAINTY_ADMIN_API_KEY", &s.admin_api_key);
    push(&mut admin, "DAINTY_ADMIN_KEY", &s.admin_key);
    push(&mut admin, "RUSTIC_ISLES_MANAGER_OPERATOR_SECRET", &s.operator_secret);
    // `/metrics` adds the backend's own with it
    push(&mut admin, "RUSTIC_ISLES_METRICS_TOKEN", &s.metrics_token);
    push(
        &mut admin,
        "DAINTY_ADMIN_API_TIMEOUTS_FILE",
        path_text(&p.config.join("admin-timeouts.json")),
    );
    push(&mut admin, "DAINTY_MAX_HUNT_SERVERS", c.max_hunt_servers.to_string());
    push(&mut admin, "DAINTY_SERVER_MODE", c.mode.as_str());
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
            push(&mut gateway, "CHAT_PORT", own.chat.to_string());
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
                format!("http://127.0.0.1:{}", own.backend),
            );
            push(
                &mut gateway,
                "GATEWAY_CONTENT_URL",
                format!("http://127.0.0.1:{}", own.content),
            );
            push(
                &mut gateway,
                "GATEWAY_WS_URL",
                format!("http://127.0.0.1:{}", own.backend),
            );
            push(
                &mut gateway,
                "ALLOWLIST_URL",
                format!("http://127.0.0.1:{}", own.allowlist),
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
                push(&mut al, "ALLOWLIST_NFT_TABLE", "dainty_lass");
                if let Some(iface) = linux_options(c).firewall_interface {
                    push(&mut al, "ALLOWLIST_INTERFACE", iface);
                }
            }
            push(&mut al, "ALLOWLIST_BIND", "127.0.0.1");
            push(&mut al, "ALLOWLIST_PORT", own.allowlist.to_string());
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
            push(&mut gateway, "GATEWAY_PORT", own.private_gateway.to_string());
            push(&mut gateway, "CHAT_PORT", own.chat.to_string());
            push(&mut gateway, "GATEWAY_SECRET", &s.gateway_secret);
            push(
                &mut gateway,
                "GATEWAY_BACKEND_URL",
                format!("http://127.0.0.1:{}", own.backend),
            );
            push(
                &mut gateway,
                "GATEWAY_CONTENT_URL",
                format!("http://127.0.0.1:{}", own.content),
            );
            push(
                &mut gateway,
                "GATEWAY_WS_URL",
                format!("http://127.0.0.1:{}", own.backend),
            );
            push(&mut gateway, "GATEWAY_ALLOWLIST", "0");
            None
        }
    };
    // Chat runs with the gateway's file (`host::serve`)
    if let Some(url) = &c.dashboard_url {
        push(&mut gateway, "CHAT_DASHBOARD_URL", url);
        push(&mut gateway, "CHAT_DASHBOARD_INGEST_KEY", &s.dashboard_ingest_key);
    }
    // Players' crash reports are copied there too
    if let Some(url) = &c.dashboard_url {
        push(&mut content, "CONTENT_DASHBOARD_URL", url);
        push(&mut content, "CONTENT_DASHBOARD_INGEST_KEY", &s.dashboard_ingest_key);
    }

    // The bot's own file, so its token reaches no other service (D5)
    let discord = c
        .discord
        .as_ref()
        .filter(|d| d.enabled && !s.discord_bot_token.trim().is_empty())
        .map(|d| d.env(s.discord_bot_token.trim(), c.dashboard_url.as_deref()));

    let mut files = EnvFiles {
        backend,
        director,
        content,
        gateway,
        admin,
        allowlist,
        discord,
        passthrough_refused: Vec::new(),
    };
    tunables::apply(&mut files, c.tunables());
    if let Some(passthrough) = &c.env {
        files.passthrough_refused = env::apply(&mut files, passthrough);
    }
    files
}

/// The store this node opens: Postgres as `database` says, or the SQLite file in the config folder.
pub fn node_database_config(c: &HostConfig, p: &Paths) -> Result<dainty_store::DatabaseConfig, HostError> {
    if let Some(db) = &c.database
        && db.postgres()
    {
        let (secrets, _) = load_or_create_secrets(&p.config, new_secrets)?;
        return Ok(dainty_store::DatabaseConfig::Postgres(dainty_store::PostgresConfig {
            host: db.postgres_host.clone().unwrap_or_else(|| "127.0.0.1".to_owned()),
            port: db.postgres_port.unwrap_or(5432),
            user: db.postgres_user.clone().unwrap_or_else(|| "dainty".to_owned()),
            password: secrets.database_password,
            database: db.postgres_database.clone().unwrap_or_else(|| "dainty".to_owned()),
            allow_remote: db.allow_remote.unwrap_or(false),
        }));
    }
    Ok(dainty_store::DatabaseConfig::Sqlite {
        file: p.config.join(DATABASE_FILE),
    })
}

/// Writes each `<name>.env` that has variables and removes each that has `None`; returns the files written.
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

/// Writes every env file in `files`; returns the files written.
pub fn write_env_files(p: &Paths, files: &EnvFiles) -> Result<Vec<PathBuf>, HostError> {
    let named = files.files();
    write_named_env_files(p, &named)
}

/// Reads an env file's variables.
pub fn read_env_file(file: &Path) -> Result<EnvMap, HostError> {
    let text = std::fs::read_to_string(file).map_err(|e| HostError::io(file, e))?;
    Ok(parse_env(&text))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

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
    fn gauntlet_switch_round_trips() {
        let table = json!({"gauntlet": "on"});
        SwitchesConfig::validate(&table).unwrap();
        let switches = SwitchesConfig::from_json(table.as_object().unwrap());
        assert_eq!(switches.gauntlet.as_deref(), Some("on"));
        assert_eq!(switches.to_json(), table);
        assert_eq!(switches.env(), [("RUSTIC_ISLES_GAUNTLET", "on".to_owned())]);
        assert_eq!(switches.module_switches(), [("gauntlet", json!("on"))]);
        assert!(SwitchesConfig::validate(&json!({"gauntlet": "yes"})).is_err());
        // An older settings.toml has no key: the module's default, off
        assert!(SwitchesConfig::from_json(&Map::new()).env().is_empty());
    }

    #[test]
    fn test_cheats_switch_round_trips_and_stays_off_the_backend() {
        let table = json!({"testCheats": "kills"});
        SwitchesConfig::validate(&table).unwrap();
        let switches = SwitchesConfig::from_json(table.as_object().unwrap());
        assert_eq!(switches.to_json(), table);
        assert_eq!(switches.test_cheats_env(), Some("kills"));
        assert!(switches.env().is_empty());
        assert!(switches.module_switches().is_empty());
        let off = SwitchesConfig::from_json(json!({"testCheats": "off"}).as_object().unwrap());
        assert_eq!(off.test_cheats_env(), None);
        assert!(SwitchesConfig::validate(&json!({"testCheats": "yes"})).is_err());
        assert_eq!(SwitchesConfig::from_json(&Map::new()).test_cheats_env(), None);
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

    #[test]
    fn version_data_round_trips_and_defaults_to_the_shipped_file() {
        let mut doc = json!({
            "version": 1,
            "mode": "private",
            "host": "100.101.102.103",
            "gameAddress": "100.101.102.103",
            "name": "Friday Hunts",
            "registration": "INVITECODE",
            "maxHuntServers": 4
        });
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path());
        let old = HostConfig::from_json(&doc).unwrap();
        assert_eq!(old.version_data, None);
        assert!(!old.to_toml().unwrap().contains("version_data"));
        assert_eq!(
            version_data_path(&paths, Some(&old)),
            dir.path().join(DEFAULT_VERSION_DATA)
        );
        assert_eq!(version_data_path(&paths, None), dir.path().join(DEFAULT_VERSION_DATA));

        doc["versionData"] = json!(" /srv/data/version_data.json ");
        let cfg = HostConfig::from_json(&doc).unwrap();
        assert_eq!(cfg.version_data.as_deref(), Some("/srv/data/version_data.json"));
        let toml = cfg.to_toml().unwrap();
        assert!(
            toml.contains("version_data = \"/srv/data/version_data.json\""),
            "{toml}"
        );
        let back = HostConfig::from_toml(&toml).unwrap();
        assert_eq!(back.version_data, cfg.version_data);
        // update keeps a host's setting when setup's own options leave it out
        assert_eq!(old.clone().written_over(Some(&back)).version_data, cfg.version_data);

        assert_eq!(cfg.version_facts, None);
        doc["versionFacts"] = json!(" srv/data/version_facts.json ");
        let facts = HostConfig::from_json(&doc).unwrap();
        let toml = facts.to_toml().unwrap();
        assert!(
            toml.contains("version_facts = \"srv/data/version_facts.json\""),
            "{toml}"
        );
        let back = HostConfig::from_toml(&toml).unwrap();
        assert_eq!(back.version_facts.as_deref(), Some("srv/data/version_facts.json"));
        assert_eq!(old.clone().written_over(Some(&back)).version_facts, back.version_facts);
        assert_eq!(
            version_facts_path(&paths, Some(&back)),
            dir.path().join("srv/data/version_facts.json")
        );
        assert_eq!(version_facts_path(&paths, None), dir.path().join(DEFAULT_VERSION_FACTS));
        for bad in [json!(""), json!(7)] {
            doc["versionFacts"] = bad;
            assert!(HostConfig::from_json(&doc).is_err(), "{doc}");
        }
        doc.as_object_mut().unwrap().remove("versionFacts");

        doc["versionData"] = json!("mine/version_data.json");
        let relative = HostConfig::from_json(&doc).unwrap();
        assert_eq!(
            version_data_path(&paths, Some(&relative)),
            dir.path().join("mine/version_data.json")
        );
        for bad in [json!(""), json!(7), json!(["a"])] {
            doc["versionData"] = bad;
            assert!(HostConfig::from_json(&doc).is_err(), "{doc}");
        }
    }

    #[test]
    fn the_dashboard_url_round_trips_and_must_be_http() {
        let mut doc = json!({
            "version": 1,
            "mode": "private",
            "host": "100.101.102.103",
            "gameAddress": "100.101.102.103",
            "name": "Friday Hunts",
            "registration": "INVITECODE",
            "maxHuntServers": 4,
            "dashboardUrl": " https://dash.example/ "
        });
        let cfg = HostConfig::from_json(&doc).unwrap();
        assert_eq!(cfg.dashboard_url.as_deref(), Some("https://dash.example"));
        let toml = cfg.to_toml().unwrap();
        assert!(toml.contains("dashboard_url = \"https://dash.example\""), "{toml}");
        assert_eq!(
            HostConfig::from_toml(&toml).unwrap().dashboard_url.as_deref(),
            Some("https://dash.example")
        );
        for bad in [json!("dash.example"), json!("ftp://dash.example"), json!(7)] {
            doc["dashboardUrl"] = bad;
            assert!(HostConfig::from_json(&doc).is_err(), "{doc}");
        }
        doc["dashboardUrl"] = json!("");
        assert_eq!(HostConfig::from_json(&doc).unwrap().dashboard_url, None);
    }

    /// D3: the rifts crates read only `RUSTIC_RIFTS_*`, so setup must leave no `MYSTICPARADOX_*` in an env file, even
    /// over one an older kit or a hand edit left it in.
    #[test]
    fn setup_rewrites_env_files_without_the_names_from_before_the_rename() {
        let dir = tempfile::tempdir().unwrap();
        let p = Paths::new(dir.path());
        std::fs::create_dir_all(&p.config).unwrap();
        for name in ["backend", "director", "admin"] {
            std::fs::write(
                p.config.join(format!("{name}.env")),
                "MYSTICPARADOX_REALMS_MODULE=on\nMYSTICPARADOX_HOST_POLICY=enforce\n",
            )
            .unwrap();
        }
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
        let secrets = Secrets {
            metrics_token: "m".repeat(64),
            operator_secret: "o".repeat(64),
            ..Secrets::default()
        };
        let files = build_env_files(&cfg, &secrets, &p, "", Platform::Windows);
        for path in write_env_files(&p, &files).unwrap() {
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(!text.contains("MYSTICPARADOX_"), "{}: {text}", path.display());
        }
        let backend = read_env_file(&p.config.join("backend.env")).unwrap();
        assert_eq!(backend.get("RUSTIC_ISLES_METRICS_TOKEN").map(str::len), Some(64));
    }

    /// RD24 phase 2: `[env.<service>]` reaches the service's file, keeps its comments and types through a rewrite,
    /// and never replaces a line setup writes.
    #[test]
    fn env_passthrough_reaches_the_service_file() {
        let text = "version = 1
mode = \"private\"
name = \"Friday Hunts\"
host = \"100.101.102.103\"
            game_address = \"100.101.102.103\"
registration = \"INVITECODE\"
max_hunt_servers = 3

            [env.gateway]
# busy VPS
GATEWAY_RATE_WS = \"40,24\"
GATEWAY_MAX_CONNECTIONS = 4096
";
        let cfg = HostConfig::from_toml(text).unwrap();
        let mut renamed = cfg.clone();
        renamed.name = "Saturday Hunts".into();
        let rewritten = renamed.to_toml().unwrap();
        assert!(
            rewritten.contains(
                "# busy VPS
GATEWAY_RATE_WS = \"40,24\"
GATEWAY_MAX_CONNECTIONS = 4096"
            ),
            "{rewritten}"
        );

        let dir = tempfile::tempdir().unwrap();
        let p = Paths::new(dir.path());
        let files = build_env_files(&cfg, &Secrets::default(), &p, "", Platform::Windows);
        assert!(files.passthrough_refused.is_empty(), "{:?}", files.passthrough_refused);
        let gateway: Vec<_> = files.gateway.iter().map(|(k, v)| format!("{k}={v}")).collect();
        assert!(gateway.contains(&"GATEWAY_RATE_WS=40,24".to_owned()), "{gateway:?}");
        assert!(
            gateway.contains(&"GATEWAY_MAX_CONNECTIONS=4096".to_owned()),
            "{gateway:?}"
        );

        let clash = HostConfig::from_toml(&text.replace("GATEWAY_RATE_WS", "GATEWAY_TLS")).unwrap();
        let files = build_env_files(&clash, &Secrets::default(), &p, "", Platform::Windows);
        assert_eq!(files.gateway.iter().filter(|(k, _)| k == "GATEWAY_TLS").count(), 1);
        assert!(
            files.passthrough_refused[0].contains("env.gateway.GATEWAY_TLS"),
            "{:?}",
            files.passthrough_refused
        );

        let secret = text.replace("GATEWAY_RATE_WS", "GATEWAY_SECRET");
        assert!(
            HostConfig::from_toml(&secret)
                .unwrap_err()
                .to_string()
                .contains("env.gateway.GATEWAY_SECRET")
        );
    }

    /// RD24 phase 2: the typed tables reach their services' files over setup's defaults, survive a rewrite with
    /// the host's comments, read back from a new file, and name the TOML key when wrong.
    #[test]
    fn service_tables_round_trip_into_the_env_files() {
        let text = "version = 1
mode = \"private\"
name = \"Friday Hunts\"
host = \"100.101.102.103\"
game_address = \"100.101.102.103\"
registration = \"INVITECODE\"
max_hunt_servers = 3

[gateway.limits]
max_connections = 4096 # a busy VPS

[gateway.rate]
ws = { burst = 40, per_minute = 24 }
register = { burst = 5, per_minute = 0.2 }

[backend]
log_level = \"debug\"
hunt_go_now_max_ms = 2000

[director]
gameserver_ready_timeout_ms = 60000
";
        let cfg = HostConfig::from_toml(text).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let p = Paths::new(dir.path());
        let files = build_env_files(&cfg, &Secrets::default(), &p, "", Platform::Windows);
        let has = |file: &EnvEntries, k: &str, v: &str| file.iter().filter(|(fk, fv)| fk == k && fv == v).count() == 1;
        assert!(
            has(&files.gateway, "GATEWAY_MAX_CONNECTIONS", "4096"),
            "{:?}",
            files.gateway
        );
        assert!(has(&files.gateway, "GATEWAY_RATE_WS", "40,24"));
        assert!(has(&files.gateway, "GATEWAY_RATE_REGISTER", "5,0.2"));
        assert!(has(&files.backend, "LOG_LEVEL", "debug"), "{:?}", files.backend);
        assert!(has(&files.backend, "HUNT_GO_NOW_MAX_DELAY_MS", "2000"));
        assert!(
            has(&files.backend, "HUNT_GO_NOW_MIN_DELAY_MS", "1"),
            "the default stays"
        );
        assert!(has(&files.director, "GAMESERVER_READY_TIMEOUT_MS", "60000"));
        assert!(has(
            &files.director,
            "GAMESERVER_RUNTIME_DIR",
            &path_text(&p.runtime_publish)
        ));
        assert!(has(&files.director, "LOG_LEVEL", "info"));

        let mut changed = cfg.clone();
        changed.name = "Saturday Hunts".into();
        changed.gateway = Some(
            tunables::Tunables::from_json(
                "gateway",
                &json!({
                    "limits": { "maxConnections": 4096 },
                    "rate": { "ws": { "burst": 60, "perMinute": 24 }, "register": { "burst": 5, "perMinute": 0.2 } },
                    "timeouts": { "wsIdleMs": 120_000 }
                }),
            )
            .unwrap(),
        );
        let rewritten = changed.to_toml().unwrap();
        assert!(rewritten.contains("max_connections = 4096 # a busy VPS"), "{rewritten}");
        assert!(
            rewritten.contains("ws = { burst = 60, per_minute = 24 }"),
            "{rewritten}"
        );
        assert!(
            rewritten.contains("[gateway.timeouts]\nws_idle_ms = 120000"),
            "{rewritten}"
        );
        assert_eq!(HostConfig::from_toml(&rewritten).unwrap().gateway, changed.gateway);

        // A new file (no source): every table written under what it does, and read back the same
        let mut fresh = changed.clone();
        fresh.file = FileForm::default();
        let new_text = fresh.to_toml().unwrap();
        assert!(new_text.contains("[gateway.rate]\n"), "{new_text}");
        let back = HostConfig::from_toml(&new_text).unwrap();
        assert_eq!(back.tunables(), changed.tunables());

        for (bad, says) in [
            ("max_connections = 4096", "max_connections = 1.5"),
            ("ws = { burst = 40, per_minute = 24 }", "ws = 40"),
            ("log_level = \"debug\"", "log_level = \"loud\""),
            ("[backend]", "[backend]\nhunt_go_now_min_ms = 3000"),
            ("max_connections = 4096", "maxConnections = 4096"),
        ] {
            let err = HostConfig::from_toml(&text.replace(bad, says)).unwrap_err().to_string();
            assert!(
                err.contains("gateway.limits.max_connections")
                    || err.contains("gateway.rate.ws")
                    || err.contains("backend.log_level")
                    || err.contains("backend.hunt_go_now_min_ms")
                    || err.contains("did you mean gateway.limits.max_connections?"),
                "{says}: {err}"
            );
        }
    }

    /// The login queue's cap reaches the backend's file only when set, survives a rewrite, and is range-checked.
    #[test]
    fn max_online_players_reaches_the_backend_file() {
        let base = "version = 1
mode = \"private\"
name = \"Friday Hunts\"
host = \"100.101.102.103\"
game_address = \"100.101.102.103\"
registration = \"INVITECODE\"
max_hunt_servers = 3
";
        let dir = tempfile::tempdir().unwrap();
        let p = Paths::new(dir.path());
        let backend = |cfg: &HostConfig| build_env_files(cfg, &Secrets::default(), &p, "", Platform::Windows).backend;
        let unset = HostConfig::from_toml(base).unwrap();
        assert!(
            !backend(&unset).iter().any(|(k, _)| k == "DAINTY_MAX_ONLINE_PLAYERS"),
            "no cap, no queue"
        );
        let capped = HostConfig::from_toml(&format!(
            "{base}
[backend]
max_online_players = 40
"
        ))
        .unwrap();
        let file = backend(&capped);
        assert_eq!(
            file.iter()
                .filter(|(k, v)| k == "DAINTY_MAX_ONLINE_PLAYERS" && v == "40")
                .count(),
            1,
            "{file:?}"
        );
        let rewritten = capped.to_toml().unwrap();
        assert!(rewritten.contains("max_online_players = 40"), "{rewritten}");
        assert_eq!(HostConfig::from_toml(&rewritten).unwrap().tunables(), capped.tunables());
        for bad in ["0", "10001", "\"many\""] {
            let text = format!(
                "{base}
[backend]
max_online_players = {bad}
"
            );
            let err = HostConfig::from_toml(&text).unwrap_err().to_string();
            assert!(err.contains("backend.max_online_players"), "{bad}: {err}");
        }
    }

    /// `[split]` needs the other half in `[network]`, guards a public game servers half's ports, and tells the module
    /// which port service it runs.
    #[test]
    fn split_reaches_the_backend_file() {
        let text = "version = 1
mode = \"public\"
name = \"Friday Hunts\"
host = \"play.example.org\"
game_address = \"203.0.113.7\"
registration = \"INVITECODE\"
max_hunt_servers = 3

[network]
node = \"meta-1\"
region = \"oregon\"

[[network.nodes]]
id = \"games-1\"
region = \"oregon\"
host = \"games.internal\"
port = 443

[split]
role = \"metagame\"
peer = \"games-1\"
";
        let cfg = HostConfig::from_toml(text).unwrap();
        let split = cfg.split.clone().unwrap();
        assert_eq!(
            (split.role, split.peer.as_str()),
            (split::SplitRole::Metagame, "games-1")
        );
        let rewritten = cfg.to_toml().unwrap();
        assert!(
            rewritten.contains("[split]") && rewritten.contains("peer = \"games-1\""),
            "{rewritten}"
        );

        let dir = tempfile::tempdir().unwrap();
        let p = Paths::new(dir.path());
        let files = build_env_files(&cfg, &Secrets::default(), &p, "", Platform::Windows);
        assert!(
            files
                .backend
                .contains(&("RUSTIC_ISLES_PORT_SERVICES".to_owned(), "backend".to_owned()))
        );
        let none = HostConfig::from_toml(&text.replace(
            "[split]
role = \"metagame\"
peer = \"games-1\"
",
            "",
        ))
        .unwrap();
        assert!(none.split.is_none());
        let files = build_env_files(&none, &Secrets::default(), &p, "", Platform::Windows);
        assert!(!files.backend.iter().any(|(k, _)| k == "RUSTIC_ISLES_PORT_SERVICES"));

        let refused = |text: &str| HostConfig::from_toml(text).unwrap_err().to_string();
        assert!(refused(&text.replace("peer = \"games-1\"", "peer = \"games-2\"")).contains("[[network.nodes]]"));
        assert!(refused(&text.replace("peer = \"games-1\"", "peer = \"meta-1\"")).contains("this node"));
        let games = text.replace("role = \"metagame\"", "role = \"gameservers\"");
        assert!(refused(&games).contains("open_game_ports"));
        let open = HostConfig::from_toml(&format!(
            "{games}open_game_ports = true
"
        ))
        .unwrap();
        let files = build_env_files(&open, &Secrets::default(), &p, "", Platform::Windows);
        assert!(
            files
                .backend
                .contains(&("RUSTIC_ISLES_PORT_SERVICES".to_owned(), "director".to_owned()))
        );
    }

    /// `[logs]` reads from settings.toml's snake_case keys and survives a rewrite.
    #[test]
    fn logs_round_trip_through_toml() {
        let text = "version = 1
mode = \"private\"
name = \"Friday Hunts\"
host = \"100.101.102.103\"
game_address = \"100.101.102.103\"
registration = \"INVITECODE\"
max_hunt_servers = 3

[logs]
format = \"json\"
color = \"never\"
gelf_url = \"tcp://graylog:12201\"
";
        let cfg = HostConfig::from_toml(text).unwrap();
        let logs = cfg.logs.clone().unwrap();
        assert_eq!(logs.gelf_url.as_deref(), Some("tcp://graylog:12201"));
        assert!(logs.gelf_target().is_some());
        let rewritten = cfg.to_toml().unwrap();
        assert!(rewritten.contains("gelf_url = \"tcp://graylog:12201\""), "{rewritten}");
        assert_eq!(HostConfig::from_toml(&rewritten).unwrap().logs, cfg.logs);
        assert!(HostConfig::from_toml(&text.replace("tcp://graylog", "tcp://logs.example.org")).is_err());
    }

    /// `[events]` reads from settings.toml, is written into a file that lacked it, and left out keeps the default.
    #[test]
    fn events_round_trip_through_toml() {
        let text = "version = 1
mode = \"private\"
name = \"Friday Hunts\"
host = \"100.101.102.103\"
game_address = \"100.101.102.103\"
registration = \"INVITECODE\"
max_hunt_servers = 3
";
        let mut cfg = HostConfig::from_toml(text).unwrap();
        assert_eq!(cfg.events, None);
        assert_eq!(
            cfg.events.clone().unwrap_or_default().options(),
            dainty_controller::events::EventsOptions::default()
        );
        cfg.events = Some(events::EventsConfig {
            retention_days: Some(30),
        });
        let written = cfg.to_toml().unwrap();
        assert!(
            written.contains(
                "[events]
retention_days = 30"
            ),
            "{written}"
        );
        let read = HostConfig::from_toml(&written).unwrap();
        assert_eq!(read.events, cfg.events);
        assert_eq!(
            HostConfig::from_toml(text).unwrap().written_over(Some(&read)).events,
            cfg.events
        );
        assert!(HostConfig::from_toml(&written.replace("= 30", "= -3")).is_err());
        assert!(HostConfig::from_toml(&written.replace("retention_days", "retention")).is_err());
    }

    /// `[ports]` round-trips through settings.toml and moves each service, and whoever reaches it, to its port; left
    /// out, the defaults stay; a port the gateway, a game server or another service holds is refused.
    #[test]
    fn ports_round_trip_and_reach_every_env_file() {
        let text = "version = 1
mode = \"public\"
name = \"Friday Hunts\"
host = \"play.example.org\"
game_address = \"203.0.113.7\"
registration = \"INVITECODE\"
max_hunt_servers = 3
";
        let mut cfg = HostConfig::from_toml(text).unwrap();
        assert_eq!((cfg.ports, service_ports(&cfg)), (None, PORTS));
        cfg.ports = Some(ports::PortsConfig {
            backend: Some(42000),
            content: Some(42002),
            allowlist: Some(42005),
            ..ports::PortsConfig::default()
        });
        let written = cfg.to_toml().unwrap();
        assert!(
            written.contains(
                "[ports]
backend = 42000
content = 42002
allowlist = 42005"
            ),
            "{written}"
        );
        let read = HostConfig::from_toml(&written).unwrap();
        assert_eq!(read.ports, cfg.ports);
        assert_eq!(
            HostConfig::from_toml(text).unwrap().written_over(Some(&read)).ports,
            cfg.ports
        );

        let dir = tempfile::tempdir().unwrap();
        let p = Paths::new(dir.path());
        let files = build_env_files(&read, &Secrets::default(), &p, "", Platform::Windows);
        let lines = |entries: &EnvEntries| entries.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>();
        let has = |entries: &EnvEntries, line: &str| assert!(lines(entries).contains(&line.to_owned()), "{line}");
        has(&files.backend, "PORT=42000");
        has(&files.director, &format!("PORT={}", PORTS.director));
        has(&files.content, "PORT=42002");
        has(&files.admin, "BACKEND_URL=http://127.0.0.1:42000");
        has(&files.gateway, "GATEWAY_BACKEND_URL=http://127.0.0.1:42000");
        has(&files.gateway, "GATEWAY_CONTENT_URL=http://127.0.0.1:42002");
        has(&files.gateway, &format!("CHAT_PORT={}", PORTS.chat));
        has(&files.allowlist.unwrap(), "ALLOWLIST_PORT=42005");
        has(
            &files.content,
            &format!("CONTENT_CRASH_DIR={}", path_text(&p.crash_reports())),
        );
        assert!(!lines(&files.content).iter().any(|l| l.starts_with("CONTENT_DASHBOARD")));
        let mut dashed = read.clone();
        dashed.dashboard_url = Some("https://dash.example".into());
        let secrets = Secrets {
            dashboard_ingest_key: "d".repeat(64),
            ..Secrets::default()
        };
        let files = build_env_files(&dashed, &secrets, &p, "", Platform::Windows);
        has(&files.content, "CONTENT_DASHBOARD_URL=https://dash.example");
        has(
            &files.content,
            &format!("CONTENT_DASHBOARD_INGEST_KEY={}", "d".repeat(64)),
        );

        for bad in [
            "backend = 443",
            "backend = 8770",
            "backend = 27017",
            "director = 42002",
            "gateway = 1",
        ] {
            let refused = format!(
                "{text}[ports]
content = 42002
{bad}
"
            );
            assert!(HostConfig::from_toml(&refused).is_err(), "{bad}");
        }
        let public_port = format!(
            "{text}public_port = 42000
[ports]
backend = 42000
"
        );
        assert!(HostConfig::from_toml(&public_port).is_err());
        assert!(
            HostConfig::from_toml(&format!(
                "{text}public_port = {}
",
                PORTS.backend
            ))
            .is_err()
        );
    }

    /// `[discord]` round-trips through settings.toml, and discord.env is written only with the bot on and its token
    /// set, which then reaches that file alone (D5).
    #[test]
    fn discord_round_trips_and_its_token_reaches_only_its_env_file() {
        let text = "version = 1
mode = \"private\"
name = \"Friday Hunts\"
host = \"100.101.102.103\"
game_address = \"100.101.102.103\"
registration = \"INVITECODE\"
max_hunt_servers = 3
dashboard_url = \"https://dash.example\"
";
        let mut cfg = HostConfig::from_toml(text).unwrap();
        assert_eq!(cfg.discord, None);
        cfg.discord = Some(discord::DiscordConfig {
            enabled: true,
            application_id: None,
            guild_ids: vec!["123456789012345678".into()],
            dashboard_url: None,
        });
        let written = cfg.to_toml().unwrap();
        assert!(written.contains("[discord]"), "{written}");
        assert!(written.contains("guild_ids = [\"123456789012345678\"]"), "{written}");
        let read = HostConfig::from_toml(&written).unwrap();
        assert_eq!(read.discord, cfg.discord);
        assert_eq!(
            HostConfig::from_toml(text).unwrap().written_over(Some(&read)).discord,
            cfg.discord
        );
        assert!(HostConfig::from_toml(&written.replace("guild_ids", "guilds")).is_err());
        assert!(HostConfig::from_toml(&format!("{written}bot_token = \"x\"\n")).is_err());

        let dir = tempfile::tempdir().unwrap();
        let p = Paths::new(dir.path());
        let none = build_env_files(&read, &Secrets::default(), &p, "", Platform::Windows);
        assert_eq!(none.discord, None, "no token, no file");
        let secrets = Secrets {
            discord_bot_token: "bot-token".into(),
            ..Secrets::default()
        };
        let files = build_env_files(&read, &secrets, &p, "", Platform::Windows);
        let discord = files.discord.clone().unwrap();
        assert!(discord.contains(&("DISCORD_BOT_TOKEN".into(), "bot-token".into())));
        assert!(discord.contains(&("DISCORD_DASHBOARD_URL".into(), "https://dash.example".into())));
        for (name, entries) in files.files() {
            if name != "discord" {
                assert!(
                    !entries.into_iter().flatten().any(|(_, v)| v.contains("bot-token")),
                    "{name}.env holds the token"
                );
            }
        }
        let mut off = read.clone();
        off.discord.as_mut().unwrap().enabled = false;
        assert_eq!(build_env_files(&off, &secrets, &p, "", Platform::Windows).discord, None);
    }

    /// The token is the host's to bring: an empty one never makes setup rewrite secrets.env, and setting it keeps
    /// every other secret.
    #[test]
    fn the_discord_token_is_set_not_made() {
        let dir = tempfile::tempdir().unwrap();
        let (made, created) = load_or_create_secrets(dir.path(), || Ok(sample_secrets())).unwrap();
        assert!(created && made.discord_bot_token.is_empty());
        let (_, missing) = load_or_create_secrets(dir.path(), || Ok(sample_secrets())).unwrap();
        assert!(!missing, "an empty token is not a missing secret");
        set_provided_secret(dir.path(), "DISCORD_BOT_TOKEN", "bot-token").unwrap();
        let (read, _) = load_or_create_secrets(dir.path(), || Ok(sample_secrets())).unwrap();
        assert_eq!(read.discord_bot_token, "bot-token");
        assert_eq!(read.admin_key, made.admin_key);
        assert!(set_provided_secret(dir.path(), "DAINTY_ADMIN_KEY", "x").is_err());
        assert!(overrides::sensitive("DISCORD_BOT_TOKEN"));
        assert!(env::denied("DISCORD_BOT_TOKEN").is_some());
    }

    /// Node kit host.json shapes from before RD24's tables: private with nothing optional, and public with the
    /// optional tables a host could already set.
    fn node_kit_documents() -> [Value; 2] {
        [
            json!({
                "version": 1,
                "mode": "private",
                "host": "100.101.102.103",
                "gameAddress": "100.101.102.103",
                "name": "Friday Hunts",
                "registration": "INVITECODE",
                "maxHuntServers": 4
            }),
            json!({
                "version": 1,
                "mode": "public",
                "host": "play.example.org",
                "gameAddress": "203.0.113.10",
                "name": "Public Hunts",
                "registration": "OPEN",
                "maxHuntServers": 6,
                "publicPort": 443,
                "performance": {
                    "serverFps": 30,
                    "idleFps": 10,
                    "trainingOnDemand": true,
                    "trainingIdleMinutes": 5,
                    "verboseGameLogs": false,
                    "memoryGuard": true,
                    "minFreeMemoryMb": 1024,
                    "huntMemoryMb": 1536
                }
            }),
        ]
    }

    /// What every service table looks like named but empty, as a host who opened the template might leave it.
    const EMPTY_TABLES: &str = "
[gateway]
[gateway.limits]
[gateway.rate]
[gateway.timeouts]
[content]
                                [allowlist]
[backend]
[director]
";

    /// RD24: a host.json, the settings.toml it becomes, that file with every service table named but empty, and setup's
    /// rewrite of it all write `service`'s env file the same, on both platforms; and no table's setting is written
    /// unless it is set, so the file is what setup wrote before the tables.
    fn every_form_writes_the_same_env(service: &str) {
        let dir = tempfile::tempdir().unwrap();
        let p = Paths::new(dir.path());
        let secrets = sample_secrets();
        let file_of = |c: &HostConfig, platform: Platform| -> Option<EnvEntries> {
            let files = build_env_files(c, &secrets, &p, "abc123", platform);
            files
                .files()
                .into_iter()
                .find(|(name, _)| *name == service)
                .and_then(|(_, f)| f.cloned())
        };
        for doc in node_kit_documents() {
            let from_json = HostConfig::from_json(&doc).unwrap();
            let toml = from_json.to_toml().unwrap();
            let from_toml = HostConfig::from_toml(&toml).unwrap();
            let with_tables = HostConfig::from_toml(&format!("{toml}{EMPTY_TABLES}")).unwrap();
            let rewritten = HostConfig::from_toml(&with_tables.to_toml().unwrap()).unwrap();
            assert_eq!(
                HostConfig::from_json(&from_toml.to_json().unwrap())
                    .unwrap()
                    .to_json()
                    .unwrap(),
                from_json.to_json().unwrap(),
                "settings.toml reads back as the host.json it came from"
            );
            for platform in [Platform::Windows, Platform::Linux] {
                let want = file_of(&from_json, platform);
                for (form, c) in [
                    ("settings.toml", &from_toml),
                    ("empty tables", &with_tables),
                    ("rewritten", &rewritten),
                ] {
                    assert_eq!(
                        file_of(c, platform),
                        want,
                        "{service} from {form} ({platform:?}), {doc}"
                    );
                }
                let Some(file) = want else { continue };
                let value = |k: &str| file.iter().find(|(fk, _)| fk == k).map(|(_, v)| v.as_str());
                for setting in tunables::table(service).map_or(&[][..], |t| t.settings) {
                    let written = value(setting.env);
                    let default = match (service, setting.env) {
                        ("backend" | "director", "LOG_LEVEL") => Some("info"),
                        ("backend", "MONGODB_TRANSACTION_MAX_COMMIT_TIME_MS") => written,
                        ("backend", "HUNT_GO_NOW_MIN_DELAY_MS") => Some("1"),
                        ("backend", "HUNT_GO_NOW_MAX_DELAY_MS") => Some("1000"),
                        ("director", "SECONDS_TO_WAIT_BETWEEN_GAMESERVER_STARTUP") => Some("3"),
                        ("backend", "REALTIME_XMPP_CAPTURE") => Some("false"),
                        ("allowlist", "ALLOWLIST_NFT_TABLE") => (platform == Platform::Linux).then_some("dainty_lass"),
                        // Setup's own defaults: the platform picks the timeout, the build the commit time
                        ("director", "GAMESERVER_READY_TIMEOUT_MS") => written,
                        _ => None,
                    };
                    assert_eq!(written, default, "{service}: {} with no table", setting.env);
                }
            }
        }
    }

    #[test]
    fn backend_env_is_the_same_from_every_form() {
        every_form_writes_the_same_env("backend");
    }

    #[test]
    fn director_env_is_the_same_from_every_form() {
        every_form_writes_the_same_env("director");
    }

    #[test]
    fn content_env_is_the_same_from_every_form() {
        every_form_writes_the_same_env("content");
    }

    #[test]
    fn gateway_env_is_the_same_from_every_form() {
        every_form_writes_the_same_env("gateway");
    }

    #[test]
    fn admin_env_is_the_same_from_every_form() {
        every_form_writes_the_same_env("admin");
    }

    #[test]
    fn allowlist_env_is_the_same_from_every_form() {
        every_form_writes_the_same_env("allowlist");
    }

    /// The play-test cheats reach only the Director's file, which every game server inherits, and never a public one.
    #[test]
    fn test_cheats_reach_only_the_director_file_on_a_private_server() {
        let text = |mode: &str, host: &str| {
            format!(
                "version = 1
mode = \"{mode}\"
name = \"Friday Hunts\"
host = \"{host}\"
                 game_address = \"{host}\"
registration = \"INVITECODE\"
max_hunt_servers = 3

[switches]
test_cheats = \"on\"
"
            )
        };
        let cfg = HostConfig::from_toml(&text("private", "100.101.102.103")).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let files = build_env_files(&cfg, &sample_secrets(), &Paths::new(dir.path()), "", Platform::Windows);
        assert!(
            files
                .director
                .iter()
                .any(|(k, v)| k == "DAINTY_TEST_CHEATS" && v == "on")
        );
        for (name, file) in files.files() {
            if name != "director" {
                assert!(
                    !file.into_iter().flatten().any(|(k, _)| k.contains("TEST_CHEATS")),
                    "{name}"
                );
            }
        }
        assert!(cfg.to_toml().unwrap().contains("test_cheats = \"on\""));
        let public = HostConfig::from_toml(&text("public", "203.0.113.10")).unwrap_err();
        assert!(public.to_string().contains("test_cheats"), "{public}");
        // Off, or an older file without the key: nothing is written
        let off = HostConfig::from_toml(&text("private", "100.101.102.103").replace("\"on\"", "\"off\"")).unwrap();
        let files = build_env_files(&off, &sample_secrets(), &Paths::new(dir.path()), "", Platform::Windows);
        assert!(!files.director.iter().any(|(k, _)| k == "DAINTY_TEST_CHEATS"));
    }

    /// RT-11: the hub cap and AFK timeouts reach only the Director's file, which it hands every game server; the
    /// supply caps and fusion time also the backend's, whose status hands them to players' games (R15 Q-1).
    #[test]
    fn rt11_settings_reach_the_director_file() {
        let text = "version = 1
mode = \"private\"
name = \"Friday Hunts\"
host = \"100.101.102.103\"
game_address = \"100.101.102.103\"
registration = \"INVITECODE\"
max_hunt_servers = 3

[director]
hub_max_players = 8 # a bigger Ramsgate
hub_afk_seconds = 0
hunt_afk_seconds = 900
keep_supply_stack_caps = true
cell_fusion_minutes = 45
";
        let cfg = HostConfig::from_toml(text).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let files = build_env_files(&cfg, &sample_secrets(), &Paths::new(dir.path()), "", Platform::Windows);
        let has = |k: &str, v: &str| files.director.iter().any(|(fk, fv)| fk == k && fv == v);
        assert!(has("DAINTY_HUB_MAX_PLAYERS", "8"), "{:?}", files.director);
        assert!(has("DAINTY_HUB_AFK_SECONDS", "0"));
        assert!(has("DAINTY_HUNT_AFK_SECONDS", "900"));
        assert!(has("DAINTY_KEEP_SUPPLY_STACK_CAPS", "true"));
        assert!(has("DAINTY_CELL_FUSION_MINUTES", "45"));
        let economy: Vec<_> = files
            .backend
            .iter()
            .filter(|(k, _)| k.starts_with("DAINTY_KEEP_SUPPLY") || k.starts_with("DAINTY_CELL_FUSION"))
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert_eq!(
            economy,
            [
                ("DAINTY_KEEP_SUPPLY_STACK_CAPS", "true"),
                ("DAINTY_CELL_FUSION_MINUTES", "45")
            ]
        );
        for (name, file) in files.files() {
            if name != "director" {
                let economy_ok = name == "backend";
                assert!(
                    !file.into_iter().flatten().any(|(k, _)| k.starts_with("DAINTY_HUB_")
                        || k.starts_with("DAINTY_HUNT_AFK")
                        || (!economy_ok
                            && (k.starts_with("DAINTY_KEEP_SUPPLY") || k.starts_with("DAINTY_CELL_FUSION")))),
                    "{name}"
                );
            }
        }
        let rewritten = cfg.to_toml().unwrap();
        assert!(
            rewritten.contains("hub_max_players = 8 # a bigger Ramsgate"),
            "{rewritten}"
        );
        let again = HostConfig::from_toml(&rewritten).unwrap();
        let again = build_env_files(
            &again,
            &sample_secrets(),
            &Paths::new(dir.path()),
            "",
            Platform::Windows,
        );
        assert_eq!(again.director, files.director);
        let err = HostConfig::from_toml(&text.replace("= 8 #", "= 65 #"))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("director.hub_max_players") || err.contains("hubMaxPlayers"),
            "{err}"
        );
    }

    fn sample_secrets() -> Secrets {
        Secrets {
            gateway_secret: "g".into(),
            allowlist_secret: "a".into(),
            admin_key: "k".into(),
            gameserver_key: "s".into(),
            api_key_hash_secret: "h".into(),
            metrics_token: "m".into(),
            jwt_private_key_b64: "p".into(),
            jwt_public_key_b64: "u".into(),
            admin_api_key: "i".into(),
            operator_secret: "o".into(),
            database_password: "d".into(),
            dashboard_ingest_key: "e".into(),
            discord_bot_token: String::new(),
        }
    }
}
