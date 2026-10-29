//! The server's configuration (`legacy/ReforgedHost/src/hostconfig.ts`): `.workspace/config/host.toml`
//! (what the host chose; [`crate::hosttoml`]) and one env file per service, written from it. Secrets are made once and
//! kept: running setup again never rotates a key (that would log everyone out, or break every
//! invite already sent).
//!
//! A Node kit's `host.json` is still read when there is no host.toml, and moved over by the first
//! write ([`save_host_config`]). Everything is checked in host.json's form ([`validate`]).
//!
//! `host.json` is read and written as the Node kit does (`JSON.stringify(c, null, 2)` and a newline).
//! A [`HostConfig`] that was read remembers the file, so writing it back gives the same bytes, and a
//! change moves no other key: a changed key keeps its place, a new one goes at the end, as with
//! `{ ...config, multipliers }` in JavaScript. A config made from scratch (setup) has its keys in
//! the order the Node kit's setup writes them.

use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};

use protocol::GameVersion;
use protocol::invite::{played_versions, reforged_version, revived_version};
use serde_json::{Map, Value};
use service_kit::{js, jsjson};

use crate::envfile::{EnvMap, format_env, parse_env};
use crate::error::HostError;
use crate::paths::{BOTH_LEGACY_PORTS, DEFAULT_LEGACY_PUBLIC_PORT};
use crate::paths::{GAME_CHANGELIST, MAX_HUNTS, PINNED_EXE_SHA256, PORTS, Paths, game_exe_relative, game_port_range};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerMode {
    Private,
    Public,
}

impl ServerMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Public => "public",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Registration {
    Invitecode,
    Open,
    None,
}

impl Registration {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Invitecode => "INVITECODE",
            Self::Open => "OPEN",
            Self::None => "NONE",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "INVITECODE" => Some(Self::Invitecode),
            "OPEN" => Some(Self::Open),
            "NONE" => Some(Self::None),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MongoConfig {
    pub uri: String,
    /// The host kit starts mongod itself.
    pub manage: bool,
    /// Path to mongod(.exe) when managed.
    pub mongod: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Multipliers {
    pub xp: f64,
    pub rewards: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinuxOptions {
    /// The wine program; `None`: the first wine on PATH when a game server starts.
    pub wine: Option<String>,
    /// A Proton (or GE-Proton) script to use instead of Wine.
    pub proton: Option<String>,
    /// A virtual X display (xvfb-run) for the game servers when there is no DISPLAY.
    pub xvfb: bool,
    /// Public: the allowlist filters the game ports only on this interface (eth0). `None`: every interface.
    pub firewall_interface: Option<String>,
}

/// host.toml `[database]`: the engine of the Rust services' database (DD15, `store`). Absent: SQLite, as
/// before it existed. Each field is kept as read, so writing the file back changes nothing unasked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DatabaseSettings {
    /// `sqlite` or `postgres`; absent: sqlite.
    pub engine: Option<String>,
    pub postgres_host: Option<String>,
    pub postgres_port: Option<u16>,
    pub postgres_user: Option<String>,
    pub postgres_database: Option<String>,
    /// A Postgres server off this machine: refused unless true, since libpq is built without TLS (DF7).
    pub allow_remote: Option<bool>,
    /// Start on a new database without migrating existing MongoDB data (RU9/M1).
    pub fresh_start: Option<bool>,
}

/// The SQLite database's file, in the config folder: kept with secrets.env in the host's backups.
pub const DATABASE_FILE: &str = "dainty-lass.db";

impl DatabaseSettings {
    pub fn postgres(&self) -> bool {
        self.engine.as_deref() == Some("postgres")
    }

    fn from_json(d: &Map<String, Value>) -> Self {
        let text = |key: &str| d.get(key).and_then(Value::as_str).map(str::to_owned);
        Self {
            engine: text("engine"),
            postgres_host: text("postgresHost"),
            postgres_port: d.get("postgresPort").and_then(Value::as_u64).map(|p| p as u16),
            postgres_user: text("postgresUser"),
            postgres_database: text("postgresDatabase"),
            allow_remote: d.get("allowRemote").and_then(Value::as_bool),
            fresh_start: d.get("freshStart").and_then(Value::as_bool),
        }
    }

    /// host.json's form: the keys that were set, in host.toml's order.
    pub fn to_json(&self) -> Value {
        let text = |value: &Option<String>| value.clone().map(Value::String);
        let fields = [
            ("engine", text(&self.engine)),
            ("postgresHost", text(&self.postgres_host)),
            ("postgresPort", self.postgres_port.map(Value::from)),
            ("postgresUser", text(&self.postgres_user)),
            ("postgresDatabase", text(&self.postgres_database)),
            ("allowRemote", self.allow_remote.map(Value::from)),
            ("freshStart", self.fresh_start.map(Value::from)),
        ];
        Value::Object(
            fields
                .into_iter()
                .filter_map(|(key, value)| value.map(|v| (key.to_owned(), v)))
                .collect(),
        )
    }
}

/// `localhost`, a loopback address or a Unix socket folder, as `store::is_local_address` says.
pub fn is_local_database_host(host: &str) -> bool {
    host == "localhost"
        || host.starts_with('/')
        || host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

pub const DEFAULT_LINUX: LinuxOptions = LinuxOptions {
    wine: None,
    proton: None,
    xvfb: true,
    firewall_interface: None,
};

/// How much CPU and RAM the game servers may use (upstream/README.md, patches 0020 and 0021).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PerformanceOptions {
    /// Frames per second of each game server's game thread while a player is connected
    /// (`REFORGED_SERVER_MAX_FPS`, runtime patch 0020): 10 to 240, or 0 for no cap.
    pub server_fps: u32,
    /// Frames per second once a game server has had no player for 15 s (`REFORGED_SERVER_IDLE_FPS`):
    /// 1 to 240, or 0 for the same as `server_fps`.
    pub idle_fps: u32,
    /// The Training Grounds start the first time someone goes there, not at boot
    /// (`TRAINING_ON_DEMAND`, Director patch 0021).
    pub training_on_demand: bool,
    /// On demand: they stop again after this many minutes without a player
    /// (`TRAINING_IDLE_STOP_MINUTES`, Director patch 0033); 0 keeps them running once started.
    pub training_idle_minutes: u32,
    /// true: the game servers keep upstream's diagnostic log levels; false: warnings and errors only
    /// (`GAMESERVER_LOG_CMDS`).
    pub verbose_game_logs: bool,
    /// A new hunt starts only when the machine has memory for it (`GAMESERVER_MEMORY_GUARD`, Director
    /// patch 0022): `hunt_memory_mb` for the hunt, and `min_free_memory_mb` left free after it.
    pub memory_guard: bool,
    pub min_free_memory_mb: u32,
    pub hunt_memory_mb: u32,
}

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

/// The game servers' log levels when `verbose_game_logs` is off: warnings and errors, and LogLoad off
/// as upstream has it.
pub const QUIET_GAME_LOG_CMDS: &str = "LogPhoenixCharacter Warning, LogOnline Warning, LogLoad Off";

pub const MAX_MULTIPLIER: f64 = 100.0;

pub const DEFAULT_MONGO_URI: &str = "mongodb://127.0.0.1:27017/?replicaSet=rs0";

/// Ports the server uses itself: never the public gateway's.
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

/// Which Dauntless the server runs (RD26): 1.14.7 on Mystic Paradox, Dauntless Revived's 1.4.4
/// (`legacy` feature), or both side by side from one controller.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Edition {
    #[default]
    Reborn,
    Legacy,
    Both,
}

impl Edition {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reborn => "reborn",
            Self::Legacy => "legacy",
            Self::Both => "both",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        [Self::Reborn, Self::Legacy, Self::Both]
            .into_iter()
            .find(|e| e.as_str() == text)
    }

    /// The games whose stacks the server runs (DD4): 1.14.7, 1.4.4 or both. Their versions are taken,
    /// so the module registry refuses a module for either.
    pub fn versions(self) -> Vec<GameVersion> {
        match self {
            Self::Reborn => vec![reforged_version()],
            Self::Legacy => vec![revived_version()],
            Self::Both => vec![reforged_version(), revived_version()],
        }
    }

    /// Whether the server runs `version`'s stack.
    pub fn runs(self, version: &GameVersion) -> bool {
        self.versions().contains(version)
    }

    pub fn runs_reborn(self) -> bool {
        self != Self::Legacy
    }

    pub fn runs_legacy(self) -> bool {
        self != Self::Reborn
    }

    /// Whether this build carries every game the server runs.
    pub fn supported(self) -> bool {
        let played = played_versions();
        self.versions().iter().all(|version| played.contains(version))
    }

    /// The game an invite names unless it asks for the other: the server's only one, or 1.14.7.
    pub fn primary(self) -> GameVersion {
        if self == Self::Legacy {
            revived_version()
        } else {
            reforged_version()
        }
    }
}

/// `host.json`.
#[derive(Debug, Clone, PartialEq)]
pub struct HostConfig {
    pub version: u32,
    pub mode: ServerMode,
    /// Absent from the file: reborn, so every file written before it keeps today's server.
    pub edition: Edition,
    /// Private: this PC's Tailscale IPv4 (or MagicDNS name). Public: the public IPv4 or DNS name
    /// players connect to (the invite's host).
    pub host: String,
    /// The address the game servers announce for UDP (the Director's `MY_IP`): the Tailscale IPv4 or
    /// the public IPv4.
    pub game_address: String,
    pub name: String,
    /// The Dauntless 1.14.7 folder the dedicated game servers run from (the one with `Archon`), as written.
    pub game_dir: Option<String>,
    pub registration: Registration,
    /// Private mode: a Tailscale share link for invites.
    pub share_url: Option<String>,
    pub mongo: MongoConfig,
    pub max_hunt_servers: u32,
    /// Private servers only: multiply the XP and the loot the game servers grant, to reach late-game
    /// content quickly when testing. Absent or 1 is off.
    pub multipliers: Option<Multipliers>,
    /// Public servers: the gateway's TCP port when 443 is taken. Absent: 443. It goes into every invite.
    pub public_port: Option<u16>,
    /// Both games: the Dauntless 1.4.4 folder its game servers run from (`game_dir` is 1.14.7's).
    pub legacy_game_dir: Option<String>,
    /// Both games, public: the 1.4.4 gateway's TCP port. Absent: [`DEFAULT_LEGACY_PUBLIC_PORT`].
    pub legacy_public_port: Option<u16>,
    /// Absent: [`DEFAULT_PERFORMANCE`]. A file from an older setup may hold only some fields; the
    /// defaults fill the rest here and the file keeps what it had.
    pub performance: Option<PerformanceOptions>,
    /// Linux only: how the Windows game servers run and what the allowlist filters.
    pub linux: Option<LinuxOptions>,
    /// Absent: SQLite in the config folder.
    pub database: Option<DatabaseSettings>,
    /// `[guard]` (DH6). Absent: the admin API defaults.
    pub guard: Option<GuardConfig>,
    /// `[history]` (DH7). Absent: off.
    pub history: Option<HistoryConfig>,
    /// `[switches]` (RU3c, RU5): backend persistence and rule switches. Absent: each version's own defaults.
    pub switches: Option<SwitchesConfig>,
    /// `[modules]` (DF3). Absent: every compiled-in module and no external one, which is today's server.
    pub modules: Option<ModulesConfig>,
    /// `[network]` (DN2). Absent: a server outside any network.
    pub network: Option<NetworkConfig>,
    /// The file this config was read from; `FileForm::default()` for a new one.
    pub file: FileForm,
}

/// `[switches]` in `host.toml`: backend persistence and rule switches (`MYSTICPARADOX_*`).
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
    pub fn from_json(s: &Map<String, Value>) -> Result<Self, HostError> {
        let on_off = |key: &str, allow_log: bool| -> Result<Option<String>, HostError> {
            let Some(v) = s.get(key) else {
                return Ok(None);
            };
            match v {
                Value::Bool(b) => Ok(Some(if *b { "on" } else { "off" }.into())),
                Value::String(text) => {
                    let lower = text.trim().to_ascii_lowercase();
                    if lower == "on" || lower == "off" || (allow_log && lower == "log") {
                        Ok(Some(lower))
                    } else if allow_log {
                        Err(bad(format!(
                            "switches.{key} must be \"on\", \"log\" or \"off\", not {}",
                            shown(Some(v))
                        )))
                    } else {
                        Err(bad(format!(
                            "switches.{key} must be \"on\" or \"off\", not {}",
                            shown(Some(v))
                        )))
                    }
                }
                _ => Err(bad(format!(
                    "switches.{key} must be \"on\" or \"off\", not {}",
                    shown(Some(v))
                ))),
            }
        };
        let positive_int = |key: &str| -> Result<Option<u32>, HostError> {
            let Some(v) = s.get(key) else {
                return Ok(None);
            };
            let Some(n) = integer(Some(v)).filter(|&n| (1.0..=f64::from(u32::MAX)).contains(&n)) else {
                return Err(bad(format!(
                    "switches.{key} must be a whole number of at least 1, not {}",
                    shown(Some(v))
                )));
            };
            Ok(Some(n as u32))
        };
        Ok(Self {
            reforge: on_off("reforge", true)?,
            mastery_persistence: on_off("masteryPersistence", false)?,
            bounty_persistence: on_off("bountyPersistence", false)?,
            cooldown_persistence: on_off("cooldownPersistence", false)?,
            account_erasure: on_off("accountErasure", false)?,
            log_purge: on_off("logPurge", false)?,
            rate_limit: on_off("rateLimit", false)?,
            rate_limit_per_minute: positive_int("rateLimitPerMinute")?,
            gameserver_rate_limit_per_minute: positive_int("gameserverRateLimitPerMinute")?,
            host_rate_limit_per_minute: positive_int("hostRateLimitPerMinute")?,
        })
    }

    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        for (key, value) in [
            ("reforge", &self.reforge),
            ("masteryPersistence", &self.mastery_persistence),
            ("bountyPersistence", &self.bounty_persistence),
            ("cooldownPersistence", &self.cooldown_persistence),
            ("accountErasure", &self.account_erasure),
            ("logPurge", &self.log_purge),
            ("rateLimit", &self.rate_limit),
        ] {
            if let Some(v) = value {
                out.insert(key.into(), Value::String(v.clone()));
            }
        }
        for (key, value) in [
            ("rateLimitPerMinute", self.rate_limit_per_minute),
            ("gameserverRateLimitPerMinute", self.gameserver_rate_limit_per_minute),
            ("hostRateLimitPerMinute", self.host_rate_limit_per_minute),
        ] {
            if let Some(v) = value {
                out.insert(key.into(), Value::Number(v.into()));
            }
        }
        Value::Object(out)
    }

    pub fn env(&self) -> Vec<(&'static str, String)> {
        let mut out = Vec::new();
        for (env_key, value) in [
            ("MYSTICPARADOX_REFORGE", &self.reforge),
            ("MYSTICPARADOX_MASTERY_PERSISTENCE", &self.mastery_persistence),
            ("MYSTICPARADOX_BOUNTY_PERSISTENCE", &self.bounty_persistence),
            ("MYSTICPARADOX_COOLDOWN_PERSISTENCE", &self.cooldown_persistence),
            ("MYSTICPARADOX_ACCOUNT_ERASURE", &self.account_erasure),
            ("MYSTICPARADOX_LOG_PURGE", &self.log_purge),
            ("MYSTICPARADOX_RATE_LIMIT", &self.rate_limit),
        ] {
            if let Some(v) = value {
                out.push((env_key, v.clone()));
            }
        }
        for (env_key, value) in [
            ("MYSTICPARADOX_RATE_LIMIT_PER_MINUTE", self.rate_limit_per_minute),
            (
                "MYSTICPARADOX_GAMESERVER_RATE_LIMIT_PER_MINUTE",
                self.gameserver_rate_limit_per_minute,
            ),
            (
                "MYSTICPARADOX_HOST_RATE_LIMIT_PER_MINUTE",
                self.host_rate_limit_per_minute,
            ),
        ] {
            if let Some(v) = value {
                out.push((env_key, v.to_string()));
            }
        }
        out
    }

    /// Key-value pairs in `[modules.<id>.switches]` form (`snake_case` key -> JSON value).
    pub fn module_switches(&self) -> Vec<(&'static str, Value)> {
        let mut out = Vec::new();
        for (key, value) in [
            ("reforge", &self.reforge),
            ("mastery_persistence", &self.mastery_persistence),
            ("bounty_persistence", &self.bounty_persistence),
            ("cooldown_persistence", &self.cooldown_persistence),
            ("account_erasure", &self.account_erasure),
            ("log_purge", &self.log_purge),
            ("rate_limit", &self.rate_limit),
        ] {
            if let Some(v) = value {
                out.push((key, Value::String(v.clone())));
            }
        }
        for (key, value) in [
            ("rate_limit_per_minute", self.rate_limit_per_minute),
            (
                "gameserver_rate_limit_per_minute",
                self.gameserver_rate_limit_per_minute,
            ),
            ("host_rate_limit_per_minute", self.host_rate_limit_per_minute),
        ] {
            if let Some(v) = value {
                out.push((key, Value::Number(v.into())));
            }
        }
        out
    }
}

/// `[history]` (DD23 layer 2, DH7): off-server change log and snapshot shipping to a peer node.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HistoryConfig {
    /// A peer node id in `[[network.nodes]]` to ship this node's change log and snapshots to (`Service::LogShip`, DH7).
    pub ship_to_peer: Option<module_api::network::NodeId>,
}

impl HistoryConfig {
    pub fn from_json(h: &Map<String, Value>) -> Result<Self, HostError> {
        let ship_to_peer = match h.get("shipToPeer") {
            None => None,
            Some(v) => {
                let s = v
                    .as_str()
                    .ok_or_else(|| bad("history.shipToPeer must be a node id (a-z, 0-9 and -)"))?;
                Some(s.parse().map_err(|e| bad(format!("history.shipToPeer: {e}")))?)
            }
        };
        Ok(Self { ship_to_peer })
    }

    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        if let Some(peer) = &self.ship_to_peer {
            out.insert("shipToPeer".into(), Value::String(peer.to_string()));
        }
        Value::Object(out)
    }
}

/// What a read `host.json` or `host.toml` looked like, so that writing it back changes only what was
/// changed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FileForm {
    /// The file as read (and normalized, as the Node kit's `validateHostConfig` returns it).
    document: Map<String, Value>,
    /// The fields as they were read, in [`HostConfig::fields`]'s form.
    read: Map<String, Value>,
    /// host.toml's text, when it was read from one.
    toml: Option<String>,
}

/// What every command says to a legacy server in a build without the `legacy` feature.
pub const LEGACY_UNSUPPORTED: &str = "this server runs Dauntless Revived's 1.4.4 (edition = \"legacy\" or \"both\"), and this reforged-host was built without it: use a build with the legacy feature (cargo build --release -p server-console --features legacy), or setup --edition reborn";

/// Refuses a legacy server in a build without the `legacy` feature.
pub fn check_edition(c: &HostConfig) -> Result<(), HostError> {
    if c.edition.supported() {
        Ok(())
    } else {
        Err(bad(LEGACY_UNSUPPORTED))
    }
}

pub fn performance_options(c: &HostConfig) -> PerformanceOptions {
    c.performance.unwrap_or(DEFAULT_PERFORMANCE)
}

pub fn valid_minutes(value: f64) -> bool {
    value.fract() == 0.0 && (0.0..=1440.0).contains(&value)
}

pub fn valid_megabytes(value: f64, min: f64) -> bool {
    value.fract() == 0.0 && value >= min && value <= 1_048_576.0
}

pub fn valid_fps(value: f64, min: f64) -> bool {
    value.fract() == 0.0 && (value == 0.0 || (value >= min && value <= 240.0))
}

/// The game servers' UDP ports for this server's hunt cap (8770-8777 up to 6 hunts, down to 8766 at 10).
pub fn game_ports(c: &HostConfig) -> (u16, u16) {
    game_port_range(c.max_hunt_servers)
}

pub fn game_ports_text(c: &HostConfig, separator: &str) -> String {
    let (first, last) = game_ports(c);
    format!("{first}{separator}{last}")
}

/// The gateway's port: what players' launchers connect to.
pub fn gateway_port(c: &HostConfig) -> u16 {
    match c.mode {
        ServerMode::Public => c.public_port.unwrap_or(PORTS.public_gateway),
        ServerMode::Private => PORTS.private_gateway,
    }
}

/// The 1.4.4 gateway's port: the server's own on a 1.4.4-only server, its second one on a server running
/// both games (RD26).
pub fn legacy_gateway_port(c: &HostConfig) -> u16 {
    match (c.edition, c.mode) {
        (Edition::Both, ServerMode::Public) => c.legacy_public_port.unwrap_or(DEFAULT_LEGACY_PUBLIC_PORT),
        (Edition::Both, ServerMode::Private) => BOTH_LEGACY_PORTS.private_gateway,
        _ => gateway_port(c),
    }
}

/// The 1.4.4 game servers' UDP ports, `(first, last)`.
pub fn legacy_game_ports(c: &HostConfig) -> (u16, u16) {
    if c.edition == Edition::Both {
        (BOTH_LEGACY_PORTS.game_first, BOTH_LEGACY_PORTS.game_last)
    } else {
        game_ports(c)
    }
}

/// The 1.4.4 allowlist's Windows Firewall rule on a server running both games (`ALLOWLIST_RULE_NAME`).
pub const LEGACY_RULE_NAME: &str = "Dauntless Reforged Revival 1.4.4 game ports (allowlist)";

pub fn multipliers_on(c: &HostConfig) -> bool {
    c.mode == ServerMode::Private && c.multipliers.is_some_and(|m| m.xp != 1.0 || m.rewards != 1.0)
}

pub fn linux_options(c: &HostConfig) -> LinuxOptions {
    c.linux.clone().unwrap_or(DEFAULT_LINUX)
}

// ------------------------------------------------------------------ checks

pub fn is_ipv4(host: &str) -> bool {
    host.parse::<Ipv4Addr>().is_ok()
}

pub fn is_tailscale_ipv4(host: &str) -> bool {
    host.parse::<Ipv4Addr>().is_ok_and(|ip| {
        let [a, b, _, _] = ip.octets();
        a == 100 && (64..=127).contains(&b)
    })
}

pub fn is_public_ipv4(host: &str) -> bool {
    let Ok(ip) = host.parse::<Ipv4Addr>() else {
        return false;
    };
    let [a, b, _, _] = ip.octets();
    !(a == 10
        || a == 127
        || a == 0
        || a >= 224
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 168))
}

/// `/^(?=.{1,253}$)([a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?\.)+[a-z]{2,63}$/i`
pub fn is_dns_name(host: &str) -> bool {
    if host.is_empty() || host.len() > 253 || !host.is_ascii() {
        return false;
    }
    let labels: Vec<&str> = host.split('.').collect();
    let Some((top, rest)) = labels.split_last() else {
        return false;
    };
    let label_ok = |l: &str| {
        let b = l.as_bytes();
        (1..=63).contains(&b.len())
            && b[0].is_ascii_alphanumeric()
            && b[b.len() - 1].is_ascii_alphanumeric()
            && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'-')
    };
    !rest.is_empty()
        && rest.iter().all(|l| label_ok(l))
        && (2..=63).contains(&top.len())
        && top.bytes().all(|c| c.is_ascii_alphabetic())
}

/// `/^[A-Za-z0-9][A-Za-z0-9_.@-]{0,14}$/`: a network interface name like eth0.
pub fn is_linux_interface(name: &str) -> bool {
    let b = name.as_bytes();
    (1..=15).contains(&b.len())
        && b[0].is_ascii_alphanumeric()
        && b[1..].iter().all(|c| c.is_ascii_alphanumeric() || b"_.@-".contains(c))
}

/// `/^https:\/\/login\.tailscale\.com\/[A-Za-z0-9/_?=&.%-]+$/`
fn is_share_url(url: &str) -> bool {
    url.strip_prefix("https://login.tailscale.com/").is_some_and(|rest| {
        !rest.is_empty()
            && rest
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "/_?=&.%-".contains(c))
    })
}

/// Control characters become spaces, runs of white space one space, and at most 48 characters
/// (UTF-16 units, as JavaScript counts them) are kept.
pub fn clean_server_name(name: &str) -> String {
    let spaced: String = name
        .chars()
        .map(|c| {
            if c <= '\u{1f}' || ('\u{7f}'..='\u{9f}').contains(&c) {
                ' '
            } else {
                c
            }
        })
        .collect();
    let mut collapsed = String::with_capacity(spaced.len());
    let mut in_space = false;
    for c in spaced.chars() {
        if js::is_space(c) {
            if !in_space {
                collapsed.push(' ');
            }
            in_space = true;
        } else {
            collapsed.push(c);
            in_space = false;
        }
    }
    let mut out = String::new();
    let mut units = 0;
    for c in js::trim(&collapsed).chars() {
        units += c.len_utf16();
        if units > 48 {
            break;
        }
        out.push(c);
    }
    out
}

fn bad(text: impl Into<String>) -> HostError {
    HostError::message(text)
}

/// `String(value)` for an error message.
fn shown(value: Option<&Value>) -> String {
    js::string(value)
}

fn integer(value: Option<&Value>) -> Option<f64> {
    value.and_then(Value::as_f64).filter(|n| n.fract() == 0.0)
}

fn string_field<'a>(object: &'a Map<String, Value>, key: &str, what: &str) -> Result<&'a str, HostError> {
    object
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| bad(format!("{what} must be a string")))
}

fn nullable_string(object: &Map<String, Value>, key: &str, what: &str) -> Result<Option<String>, HostError> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(other) => Err(bad(format!(
            "{what} must be a string or null, not {}",
            shown(Some(other))
        ))),
    }
}

/// Checks a host.json (`validateHostConfig`), returning the first problem. On success the result is
/// the same document with `host` trimmed and lower-cased and `name` cleaned, in their places.
pub fn validate(document: &Value) -> Result<Value, HostError> {
    let c = document
        .as_object()
        .ok_or_else(|| bad("host.json must hold a JSON object"))?;
    let mode = match c.get("mode").and_then(Value::as_str) {
        Some("private") => ServerMode::Private,
        Some("public") => ServerMode::Public,
        _ => return Err(bad("mode must be private or public")),
    };
    if !c
        .get("edition")
        .is_none_or(|e| e.as_str().and_then(Edition::parse).is_some())
    {
        return Err(bad("edition must be reborn, legacy or both"));
    }
    let both = c.get("edition").and_then(Value::as_str) == Some("both");
    let raw_host = string_field(c, "host", "host")?;
    let host = js::trim(raw_host).to_lowercase();
    match mode {
        ServerMode::Private => {
            if !is_tailscale_ipv4(&host) && !host.ends_with(".ts.net") {
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
    let game_address = c.get("gameAddress").and_then(Value::as_str).filter(|a| is_ipv4(a));
    let Some(game_address) = game_address else {
        return Err(bad(format!(
            "gameAddress must be an IPv4 address (the one players' game traffic reaches), not {}",
            shown(c.get("gameAddress"))
        )));
    };
    if mode == ServerMode::Private && !is_tailscale_ipv4(game_address) {
        return Err(bad("a private server's game address must be its Tailscale IPv4"));
    }
    let name = clean_server_name(string_field(c, "name", "name")?);
    if name.is_empty() {
        return Err(bad("the server needs a name"));
    }
    if !c
        .get("registration")
        .and_then(Value::as_str)
        .is_some_and(|r| Registration::parse(r).is_some())
    {
        return Err(bad("registration must be INVITECODE, OPEN or NONE"));
    }
    match c.get("shareUrl") {
        Some(Value::Null) => {}
        Some(Value::String(url)) if is_share_url(url) => {}
        _ => return Err(bad("shareUrl must be a https://login.tailscale.com/ link")),
    }
    if let Some(mongo) = c.get("mongo") {
        let mongo = mongo.as_object().ok_or_else(|| bad("mongo must be an object"))?;
        if !mongo
            .get("uri")
            .and_then(Value::as_str)
            .is_some_and(|u| u.starts_with("mongodb://") || u.starts_with("mongodb+srv://"))
        {
            return Err(bad("mongo.uri must be a mongodb:// connection string"));
        }
    }
    if !integer(c.get("maxHuntServers")).is_some_and(|n| (1.0..=f64::from(MAX_HUNTS)).contains(&n)) {
        return Err(bad(format!(
            "maxHuntServers must be 1 to {MAX_HUNTS} (UDP {}-{} up to 6 hunts, {}-{} at {MAX_HUNTS})",
            PORTS.game_first, PORTS.game_last, PORTS.game_lowest, PORTS.game_last
        )));
    }
    if let Some(multipliers) = c.get("multipliers") {
        let m = multipliers
            .as_object()
            .ok_or_else(|| bad("multipliers must be an object with xp and rewards"))?;
        for (what, value) in m {
            if !value.as_f64().is_some_and(|v| (1.0..=MAX_MULTIPLIER).contains(&v)) {
                return Err(bad(format!(
                    "the {what} multiplier must be a number from 1 to {}, not {}",
                    jsjson::number(MAX_MULTIPLIER),
                    shown(Some(value))
                )));
            }
        }
        let one = |key: &str| m.get(key).and_then(Value::as_f64) == Some(1.0);
        if mode == ServerMode::Public && !(one("xp") && one("rewards")) {
            return Err(bad(
                "XP and reward multipliers are for private (Tailscale) test servers only",
            ));
        }
    }
    if let Some(port) = c.get("publicPort") {
        if mode != ServerMode::Public {
            return Err(bad("publicPort is for public servers only"));
        }
        let Some(n) = integer(Some(port)).filter(|n| (1.0..=65535.0).contains(n)) else {
            return Err(bad(format!(
                "publicPort must be a TCP port from 1 to 65535, not {}",
                shown(Some(port))
            )));
        };
        let n = n as u16;
        if RESERVED_PORTS.contains(&n) || (PORTS.game_lowest..=PORTS.game_last).contains(&n) {
            return Err(bad(format!(
                "publicPort {n} is one the server uses itself: pick another (4443, for example)"
            )));
        }
    }
    match c.get("legacyGameDir") {
        None | Some(Value::Null) => {}
        Some(Value::String(_)) if both => {}
        Some(Value::String(_)) => return Err(bad("legacyGameDir is for a server running both games (edition both)")),
        Some(other) => {
            return Err(bad(format!(
                "legacyGameDir must be a folder, not {}",
                shown(Some(other))
            )));
        }
    }
    let reborn_port = integer(c.get("publicPort")).map_or(PORTS.public_gateway, |n| n as u16);
    let legacy_ports = [
        BOTH_LEGACY_PORTS.metagame,
        BOTH_LEGACY_PORTS.deploy,
        BOTH_LEGACY_PORTS.content,
        BOTH_LEGACY_PORTS.allowlist,
        BOTH_LEGACY_PORTS.chat,
    ];
    let legacy_game = BOTH_LEGACY_PORTS.game_first..=BOTH_LEGACY_PORTS.game_last;
    if both && mode == ServerMode::Public && (legacy_ports.contains(&reborn_port) || legacy_game.contains(&reborn_port))
    {
        return Err(bad(format!(
            "publicPort {reborn_port} is one the 1.4.4 stack uses on a server running both games: pick another"
        )));
    }
    if let Some(port) = c.get("legacyPublicPort") {
        if !both || mode != ServerMode::Public {
            return Err(bad(
                "legacyPublicPort is for a public server running both games (edition both)",
            ));
        }
        let Some(n) = integer(Some(port)).filter(|n| (1.0..=65535.0).contains(n)) else {
            return Err(bad(format!(
                "legacyPublicPort must be a TCP port from 1 to 65535, not {}",
                shown(Some(port))
            )));
        };
        let n = n as u16;
        if RESERVED_PORTS.contains(&n)
            || legacy_ports.contains(&n)
            || (PORTS.game_lowest..=PORTS.game_last).contains(&n)
            || legacy_game.contains(&n)
        {
            return Err(bad(format!(
                "legacyPublicPort {n} is one the server uses itself: pick another (4443, for example)"
            )));
        }
    }
    if both && mode == ServerMode::Public {
        let legacy_port = integer(c.get("legacyPublicPort")).map_or(DEFAULT_LEGACY_PUBLIC_PORT, |n| n as u16);
        if legacy_port == reborn_port {
            return Err(bad(format!(
                "both gateways would listen on {reborn_port}: give the 1.4.4 one another port (legacyPublicPort)"
            )));
        }
    }
    if let Some(performance) = c.get("performance") {
        // Checked with the defaults filled in: a host.json from an older setup lacks the newer fields.
        let p = performance.as_object().cloned().unwrap_or_default();
        let number = |key: &str, default: u32| match p.get(key) {
            None => Some(f64::from(default)),
            Some(v) => v.as_f64(),
        };
        let flag = |key: &str, default: bool| match p.get(key) {
            None => Some(default),
            Some(v) => v.as_bool(),
        };
        let shown_field = |key: &str, default: u32| match p.get(key) {
            None => default.to_string(),
            v => shown(v),
        };
        let d = DEFAULT_PERFORMANCE;
        if !number("serverFps", d.server_fps).is_some_and(|v| valid_fps(v, 10.0)) {
            return Err(bad(format!(
                "performance.serverFps must be 0 (no cap) or 10 to 240 frames per second, not {}",
                shown_field("serverFps", d.server_fps)
            )));
        }
        if !number("idleFps", d.idle_fps).is_some_and(|v| valid_fps(v, 1.0)) {
            return Err(bad(format!(
                "performance.idleFps must be 0 (same as serverFps) or 1 to 240 frames per second, not {}",
                shown_field("idleFps", d.idle_fps)
            )));
        }
        if flag("trainingOnDemand", d.training_on_demand).is_none() {
            return Err(bad("performance.trainingOnDemand must be true or false"));
        }
        if !number("trainingIdleMinutes", d.training_idle_minutes).is_some_and(valid_minutes) {
            return Err(bad(format!(
                "performance.trainingIdleMinutes must be 0 (keep them running) or 1 to 1440 minutes, not {}",
                shown_field("trainingIdleMinutes", d.training_idle_minutes)
            )));
        }
        if flag("verboseGameLogs", d.verbose_game_logs).is_none() {
            return Err(bad("performance.verboseGameLogs must be true or false"));
        }
        if flag("memoryGuard", d.memory_guard).is_none() {
            return Err(bad("performance.memoryGuard must be true or false"));
        }
        if !number("minFreeMemoryMb", d.min_free_memory_mb).is_some_and(|v| valid_megabytes(v, 0.0)) {
            return Err(bad(format!(
                "performance.minFreeMemoryMb must be a whole number of MB, not {}",
                shown_field("minFreeMemoryMb", d.min_free_memory_mb)
            )));
        }
        if !number("huntMemoryMb", d.hunt_memory_mb).is_some_and(|v| valid_megabytes(v, 256.0)) {
            return Err(bad(format!(
                "performance.huntMemoryMb must be at least 256 MB, not {}",
                shown_field("huntMemoryMb", d.hunt_memory_mb)
            )));
        }
    }
    if let Some(linux) = c.get("linux") {
        let lx = linux.as_object().ok_or_else(|| bad("linux must be an object"))?;
        for what in ["wine", "proton"] {
            match lx.get(what) {
                Some(Value::Null) => {}
                Some(Value::String(path)) if path.starts_with('/') => {}
                _ => {
                    return Err(bad(format!(
                        "linux.{what} must be the full path of the program, or null"
                    )));
                }
            }
        }
        if !lx.get("xvfb").is_some_and(Value::is_boolean) {
            return Err(bad("linux.xvfb must be true or false"));
        }
        match lx.get("firewallInterface") {
            Some(Value::Null) => {}
            Some(Value::String(name)) if is_linux_interface(name) => {}
            other => {
                return Err(bad(format!(
                    "linux.firewallInterface must be a network interface name like eth0, not {}",
                    shown(other)
                )));
            }
        }
    }
    if let Some(database) = c.get("database") {
        validate_database(database)?;
    }
    if let Some(guard) = c.get("guard") {
        guard::validate(guard)?;
    }
    if let Some(switches) = c.get("switches") {
        let s = switches.as_object().ok_or_else(|| bad("switches must be an object"))?;
        SwitchesConfig::from_json(s)?;
    }
    if let Some(modules) = c.get("modules") {
        ModulesConfig::from_json(modules)?;
    }
    let parsed_network = match c.get("network") {
        None => None,
        Some(network) => {
            let network = NetworkConfig::from_json(network)?;
            // Node links are mutual TLS on the gateway's public port (DN3)
            if !network.nodes.is_empty() && c.get("mode").and_then(Value::as_str) != Some("public") {
                return Err(bad(
                    "network.nodes needs mode = \"public\": node links run over the gateway's TLS port",
                ));
            }
            Some(network)
        }
    };
    if let Some(history) = c.get("history") {
        let h = history.as_object().ok_or_else(|| bad("history must be an object"))?;
        let parsed = HistoryConfig::from_json(h)?;
        if let Some(peer) = &parsed.ship_to_peer {
            let Some(network) = &parsed_network else {
                return Err(bad(format!(
                    "history.shipToPeer: {peer} requires [network] and [[network.nodes]]"
                )));
            };
            if *peer == network.node {
                return Err(bad(format!(
                    "history.shipToPeer: {peer} is this node itself (network.node); pick a peer in [[network.nodes]]"
                )));
            }
            if !network.nodes.iter().any(|n| n.id == *peer) {
                return Err(bad(format!(
                    "history.shipToPeer: {peer} is not listed in [[network.nodes]]"
                )));
            }
        }
    }
    let mut out = c.clone();
    out.insert("host".into(), Value::String(host));
    out.insert("name".into(), Value::String(name));
    Ok(Value::Object(out))
}

fn validate_database(database: &Value) -> Result<(), HostError> {
    let d = database.as_object().ok_or_else(|| bad("database must be an object"))?;
    let engine = match d.get("engine") {
        None => "sqlite",
        Some(Value::String(e)) if e == "sqlite" || e == "postgres" => e.as_str(),
        other => {
            return Err(bad(format!(
                "database.engine must be sqlite or postgres, not {}",
                shown(other)
            )));
        }
    };
    for key in ["postgresHost", "postgresUser", "postgresDatabase"] {
        match d.get(key) {
            None => {}
            Some(Value::String(v)) if !js::trim(v).is_empty() && !v.chars().any(char::is_control) => {}
            other => return Err(bad(format!("database.{key} must be a name, not {}", shown(other)))),
        }
    }
    if let Some(port) = d.get("postgresPort")
        && !integer(Some(port)).is_some_and(|n| (1.0..=65535.0).contains(&n))
    {
        return Err(bad(format!(
            "database.postgresPort must be a TCP port from 1 to 65535, not {}",
            shown(Some(port))
        )));
    }
    if !d.get("allowRemote").is_none_or(Value::is_boolean) {
        return Err(bad("database.allowRemote must be true or false"));
    }
    if !d.get("freshStart").is_none_or(Value::is_boolean) {
        return Err(bad("database.freshStart must be true or false"));
    }
    let host = d.get("postgresHost").and_then(Value::as_str).unwrap_or("127.0.0.1");
    let allow_remote = d.get("allowRemote").and_then(Value::as_bool) == Some(true);
    // libpq is built without TLS (DF7 spike): a password and every query would cross the network in clear.
    if engine == "postgres" && !allow_remote && !is_local_database_host(host) {
        return Err(bad(format!(
            "database.postgresHost {host} is not on this machine, and connections to Postgres are not encrypted: \
             use 127.0.0.1, or set database.allowRemote = true if the network between them is private"
        )));
    }
    Ok(())
}

impl HostConfig {
    /// A config from a host.json document, checked as the Node kit checks it.
    pub fn from_json(document: &Value) -> Result<Self, HostError> {
        let checked = validate(&jsjson::to_js(document))?;
        let Value::Object(c) = checked else {
            return Err(bad("host.json must hold a JSON object"));
        };
        let mongo = c.get("mongo").and_then(Value::as_object);
        let num = |o: &Map<String, Value>, key: &str| o.get(key).and_then(Value::as_f64);
        let performance = match c.get("performance") {
            None | Some(Value::Null) => None,
            Some(p) => {
                let p = p.as_object().cloned().unwrap_or_default();
                let d = DEFAULT_PERFORMANCE;
                let n = |key: &str, default: u32| num(&p, key).map_or(default, |v| v as u32);
                let b = |key: &str, default: bool| p.get(key).and_then(Value::as_bool).unwrap_or(default);
                Some(PerformanceOptions {
                    server_fps: n("serverFps", d.server_fps),
                    idle_fps: n("idleFps", d.idle_fps),
                    training_on_demand: b("trainingOnDemand", d.training_on_demand),
                    training_idle_minutes: n("trainingIdleMinutes", d.training_idle_minutes),
                    verbose_game_logs: b("verboseGameLogs", d.verbose_game_logs),
                    memory_guard: b("memoryGuard", d.memory_guard),
                    min_free_memory_mb: n("minFreeMemoryMb", d.min_free_memory_mb),
                    hunt_memory_mb: n("huntMemoryMb", d.hunt_memory_mb),
                })
            }
        };
        let linux = match c.get("linux").and_then(Value::as_object) {
            None => None,
            Some(lx) => Some(LinuxOptions {
                wine: nullable_string(lx, "wine", "linux.wine")?,
                proton: nullable_string(lx, "proton", "linux.proton")?,
                xvfb: lx.get("xvfb").and_then(Value::as_bool).unwrap_or(true),
                firewall_interface: nullable_string(lx, "firewallInterface", "linux.firewallInterface")?,
            }),
        };
        let database = c
            .get("database")
            .and_then(Value::as_object)
            .map(DatabaseSettings::from_json);
        let guard = c.get("guard").and_then(Value::as_object).map(GuardConfig::from_json);
        let history = c
            .get("history")
            .and_then(Value::as_object)
            .map(HistoryConfig::from_json)
            .transpose()?;
        let switches = c
            .get("switches")
            .and_then(Value::as_object)
            .map(SwitchesConfig::from_json)
            .transpose()?;
        let modules = c.get("modules").map(ModulesConfig::from_json).transpose()?;
        let network = c.get("network").map(NetworkConfig::from_json).transpose()?;
        let multipliers = c.get("multipliers").and_then(Value::as_object).map(|m| Multipliers {
            xp: num(m, "xp").unwrap_or(1.0),
            rewards: num(m, "rewards").unwrap_or(1.0),
        });
        let mut config = Self {
            version: c.get("version").and_then(Value::as_u64).map_or(1, |v| v as u32),
            mode: if c.get("mode").and_then(Value::as_str) == Some("public") {
                ServerMode::Public
            } else {
                ServerMode::Private
            },
            edition: c
                .get("edition")
                .and_then(Value::as_str)
                .and_then(Edition::parse)
                .unwrap_or_default(),
            host: string_field(&c, "host", "host")?.to_owned(),
            game_address: string_field(&c, "gameAddress", "gameAddress")?.to_owned(),
            name: string_field(&c, "name", "name")?.to_owned(),
            game_dir: nullable_string(&c, "gameDir", "gameDir")?,
            registration: c
                .get("registration")
                .and_then(Value::as_str)
                .and_then(Registration::parse)
                .unwrap_or(Registration::Invitecode),
            share_url: nullable_string(&c, "shareUrl", "shareUrl")?,
            mongo: match mongo {
                Some(mongo) => MongoConfig {
                    uri: string_field(mongo, "uri", "mongo.uri")?.to_owned(),
                    manage: mongo.get("manage").and_then(Value::as_bool).unwrap_or(false),
                    mongod: nullable_string(mongo, "mongod", "mongo.mongod")?,
                },
                None => MongoConfig {
                    uri: DEFAULT_MONGO_URI.to_owned(),
                    manage: true,
                    mongod: None,
                },
            },
            max_hunt_servers: num(&c, "maxHuntServers").map_or(3, |v| v as u32),
            multipliers,
            public_port: num(&c, "publicPort").map(|v| v as u16),
            legacy_game_dir: nullable_string(&c, "legacyGameDir", "legacyGameDir")?,
            legacy_public_port: num(&c, "legacyPublicPort").map(|v| v as u16),
            performance,
            linux,
            database,
            guard,
            history,
            switches,
            modules,
            network,
            file: FileForm::default(),
        };
        config.file = FileForm {
            read: config.fields(),
            document: c,
            toml: None,
        };
        Ok(config)
    }

    /// A config from a host.toml's text, checked as [`from_json`](Self::from_json) checks host.json;
    /// the messages name the TOML keys.
    pub fn from_toml(text: &str) -> Result<Self, HostError> {
        let document = crate::hosttoml::document(text)?;
        let mut config = Self::from_json(&document).map_err(|e| bad(crate::hosttoml::message(&e.to_string())))?;
        config.file.toml = Some(text.to_owned());
        Ok(config)
    }

    /// This config, to be written over `previous`'s host.toml. Setup builds its config anew from the
    /// options; this keeps the host's comments and layout for every value that did not change.
    pub fn written_over(mut self, previous: Option<&HostConfig>) -> Self {
        if let Some(previous) = previous.filter(|p| p.file.toml.is_some()) {
            self.file = previous.file.clone();
        }
        self
    }

    /// Whether the host opted out of migrating existing MongoDB data (`setup --fresh-start` or `database.fresh_start = true`).
    pub fn fresh_start(&self) -> bool {
        self.database.as_ref().is_some_and(|d| d.fresh_start == Some(true))
    }

    /// The host.toml to write, checked: the file as read with only the changed values replaced, or a
    /// new file (from scratch, or from a host.json) that says what each key does.
    pub fn to_toml(&self) -> Result<String, HostError> {
        let text = crate::hosttoml::text(&self.fields(), &self.file.read, self.file.toml.as_deref())?;
        Self::from_toml(&text)?;
        Ok(text)
    }

    /// The fields in the order the Node kit's setup writes a new host.json.
    fn fields(&self) -> Map<String, Value> {
        let text = |s: &str| Value::String(s.to_owned());
        let opt = |s: &Option<String>| s.as_deref().map_or(Value::Null, text);
        let mut m = Map::new();
        m.insert("version".into(), self.version.into());
        m.insert("mode".into(), text(self.mode.as_str()));
        // Left out for reborn: a reborn server's file stays as the Node kit writes it.
        if self.edition != Edition::Reborn {
            m.insert("edition".into(), text(self.edition.as_str()));
        }
        m.insert("host".into(), text(&self.host));
        m.insert("gameAddress".into(), text(&self.game_address));
        m.insert("name".into(), text(&self.name));
        m.insert("gameDir".into(), opt(&self.game_dir));
        m.insert("registration".into(), text(self.registration.as_str()));
        m.insert("shareUrl".into(), opt(&self.share_url));
        let mut mongo = Map::new();
        mongo.insert("uri".into(), text(&self.mongo.uri));
        mongo.insert("manage".into(), self.mongo.manage.into());
        mongo.insert("mongod".into(), opt(&self.mongo.mongod));
        m.insert("mongo".into(), Value::Object(mongo));
        m.insert("maxHuntServers".into(), self.max_hunt_servers.into());
        if let Some(x) = self.multipliers {
            let mut mm = Map::new();
            mm.insert("xp".into(), js_number(x.xp));
            mm.insert("rewards".into(), js_number(x.rewards));
            m.insert("multipliers".into(), Value::Object(mm));
        }
        if let Some(port) = self.public_port {
            m.insert("publicPort".into(), port.into());
        }
        if let Some(dir) = &self.legacy_game_dir {
            m.insert("legacyGameDir".into(), text(dir));
        }
        if let Some(port) = self.legacy_public_port {
            m.insert("legacyPublicPort".into(), port.into());
        }
        if let Some(p) = self.performance {
            let mut pm = Map::new();
            pm.insert("serverFps".into(), p.server_fps.into());
            pm.insert("idleFps".into(), p.idle_fps.into());
            pm.insert("trainingOnDemand".into(), p.training_on_demand.into());
            pm.insert("trainingIdleMinutes".into(), p.training_idle_minutes.into());
            pm.insert("verboseGameLogs".into(), p.verbose_game_logs.into());
            pm.insert("memoryGuard".into(), p.memory_guard.into());
            pm.insert("minFreeMemoryMb".into(), p.min_free_memory_mb.into());
            pm.insert("huntMemoryMb".into(), p.hunt_memory_mb.into());
            m.insert("performance".into(), Value::Object(pm));
        }
        if let Some(lx) = &self.linux {
            let mut lm = Map::new();
            lm.insert("wine".into(), opt(&lx.wine));
            lm.insert("proton".into(), opt(&lx.proton));
            lm.insert("xvfb".into(), lx.xvfb.into());
            lm.insert("firewallInterface".into(), opt(&lx.firewall_interface));
            m.insert("linux".into(), Value::Object(lm));
        }
        if let Some(database) = &self.database {
            m.insert("database".into(), database.to_json());
        }
        if let Some(guard) = &self.guard {
            m.insert("guard".into(), guard.to_json());
        }
        if let Some(history) = &self.history {
            m.insert("history".into(), history.to_json());
        }
        if let Some(switches) = &self.switches {
            m.insert("switches".into(), switches.to_json());
        }
        if let Some(modules) = &self.modules {
            m.insert("modules".into(), modules.to_json());
        }
        if let Some(network) = &self.network {
            m.insert("network".into(), network.to_json());
        }
        m
    }

    /// The document to write, checked: the file as read with only the changed fields replaced.
    pub fn to_json(&self) -> Result<Value, HostError> {
        let fields = self.fields();
        let mut document = self.file.document.clone();
        for (key, value) in &fields {
            if self.file.read.get(key) != Some(value) {
                document.insert(key.clone(), value.clone());
            }
        }
        for key in self.file.read.keys() {
            if !fields.contains_key(key) {
                document.shift_remove(key);
            }
        }
        validate(&Value::Object(document))
    }
}

fn js_number(value: f64) -> Value {
    jsjson::to_js(&serde_json::Number::from_f64(value).map_or(Value::Null, Value::Number))
}

/// `JSON.stringify(value, null, 2)`.
pub fn stringify_pretty(value: &Value) -> String {
    let mut out = String::new();
    write_pretty(&mut out, &jsjson::to_js(value), 0);
    out
}

fn write_pretty(out: &mut String, value: &Value, depth: usize) {
    let indent = |out: &mut String, depth: usize| out.push_str(&"  ".repeat(depth));
    match value {
        Value::Array(items) if !items.is_empty() => {
            out.push_str("[\n");
            for (index, item) in items.iter().enumerate() {
                indent(out, depth + 1);
                write_pretty(out, item, depth + 1);
                out.push_str(if index + 1 < items.len() { ",\n" } else { "\n" });
            }
            indent(out, depth);
            out.push(']');
        }
        Value::Object(map) if !map.is_empty() => {
            out.push_str("{\n");
            for (index, (key, item)) in map.iter().enumerate() {
                indent(out, depth + 1);
                out.push_str(&jsjson::stringify(&Value::String(key.clone())));
                out.push_str(": ");
                write_pretty(out, item, depth + 1);
                out.push_str(if index + 1 < map.len() { ",\n" } else { "\n" });
            }
            indent(out, depth);
            out.push('}');
        }
        other => out.push_str(&jsjson::stringify(other)),
    }
}

pub mod guard;
pub mod modules;
pub mod yaml;

pub use guard::GuardConfig;
pub use modules::{ExternalModuleEntry, ModulesConfig, NetworkConfig};

/// host.toml, host.yaml or host.json: far more than any needs.
pub const MAX_HOST_CONFIG_BYTES: u64 = 256 * 1024;

fn is_toml(file: &Path) -> bool {
    file.extension().is_some_and(|e| e.eq_ignore_ascii_case("toml"))
}

fn is_yaml(file: &Path) -> bool {
    file.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("yaml") || e.eq_ignore_ascii_case("yml"))
}

/// The file at `file`: TOML when it ends in `.toml`, host.json's document in YAML when it ends in
/// `.yaml` or `.yml` (RD26: one schema, three forms), host.json otherwise. `None` when it does not exist.
pub fn read_host_config(file: &Path) -> Result<Option<HostConfig>, HostError> {
    // Settings are a few hundred bytes; a huge file is refused before it is read
    if std::fs::metadata(file).is_ok_and(|m| m.len() > MAX_HOST_CONFIG_BYTES) {
        return Err(bad(format!(
            "{} is larger than {} KiB: not a settings file",
            file.display(),
            MAX_HOST_CONFIG_BYTES / 1024
        )));
    }
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(HostError::io(file, e)),
    };
    if is_toml(file) {
        return HostConfig::from_toml(&text)
            .map(Some)
            .map_err(|e| bad(format!("{}: {e}", file.display())));
    }
    let document = if is_yaml(file) {
        yaml::parse_yaml_to_json(&text).map_err(|e| bad(format!("{}: {e}", file.display())))?
    } else {
        jsjson::parse(&text).map_err(|e| bad(format!("{}: {e}", file.display())))?
    };
    HostConfig::from_json(&document).map(Some)
}

/// The server's settings: host.toml, else host.yaml or host.yml, else a Node kit's host.json while
/// there is no host.toml yet. `None` when there is none. Never writes anything.
pub fn load_host_config(p: &Paths) -> Result<Option<HostConfig>, HostError> {
    let yaml = [p.config.join("host.yaml"), p.config.join("host.yml")];
    for file in [&p.host_toml, &yaml[0], &yaml[1], &p.host_json] {
        if let Some(c) = read_host_config(file)? {
            return Ok(Some(c));
        }
    }
    Ok(None)
}

/// Whether the server has its settings yet (host.toml, or a Node kit's host.json).
pub fn has_host_config(p: &Paths) -> bool {
    p.host_toml.exists() || p.host_json.exists()
}

/// Writes host.toml. A Node kit's host.json is renamed host.json.bak once host.toml holds its
/// settings, so it is never read again; returns true when that happened.
pub fn save_host_config(p: &Paths, c: &HostConfig) -> Result<bool, HostError> {
    write_host_config(&p.host_toml, c)?;
    if !p.host_json.exists() {
        return Ok(false);
    }
    let backup = p.host_json.with_extension("json.bak");
    std::fs::rename(&p.host_json, &backup).map_err(|e| HostError::io(&p.host_json, e))?;
    Ok(true)
}

/// The host.json text (`JSON.stringify(c, null, 2)` and a newline).
pub fn host_config_text(c: &HostConfig) -> Result<String, HostError> {
    Ok(stringify_pretty(&c.to_json()?) + "\n")
}

/// Writes `file` as TOML when it ends in `.toml` and as host.json otherwise.
pub fn write_host_config(file: &Path, c: &HostConfig) -> Result<(), HostError> {
    let text = if is_toml(file) {
        c.to_toml()?
    } else {
        host_config_text(c)?
    };
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| HostError::io(dir, e))?;
    }
    std::fs::write(file, text).map_err(|e| HostError::io(file, e))
}

// ------------------------------------------------------------------ secrets

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
    /// What the admin API's callers (a Discord bot) send it.
    pub admin_api_key: String,
    /// Signs the admin API's calls to the backend's `/admin/v1` (Paradox's manager-operator credential).
    pub operator_secret: String,
    /// The Postgres role's password when `[database]` picks Postgres (DF7); made for every server, so a
    /// later switch finds it there.
    pub database_password: String,
}

pub const SECRET_FILE: &str = "secrets.env";

/// The variable names in `secrets.env`, in the order they are written.
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

impl Secrets {
    /// The field written under `SECRET_KEYS[index]`.
    fn slot(&mut self, index: usize) -> &mut String {
        match index {
            0 => &mut self.gateway_secret,
            1 => &mut self.allowlist_secret,
            2 => &mut self.admin_key,
            3 => &mut self.gameserver_key,
            4 => &mut self.api_key_hash_secret,
            5 => &mut self.metrics_token,
            6 => &mut self.jwt_private_key_b64,
            7 => &mut self.jwt_public_key_b64,
            8 => &mut self.admin_api_key,
            9 => &mut self.operator_secret,
            _ => &mut self.database_password,
        }
    }
}

/// New secrets: eight 32-byte hex keys and a 4096-bit RSA pair (PKCS#8 and SPKI PEM, base64) for the
/// backend's tokens. Filled by RH3, which picks the RSA implementation (D10: aws-lc-rs) and ports
/// the "4096-bit RSA pair" test.
pub fn new_secrets() -> Result<Secrets, HostError> {
    crate::commands::setup::secrets::new_secrets()
}

/// Reads the secrets file, adding any secret that is missing (an older setup, or the first run).
/// Returns the secrets and whether any was made.
pub fn load_or_create_secrets(
    config_dir: &Path,
    make: impl FnOnce() -> Result<Secrets, HostError>,
) -> Result<(Secrets, bool), HostError> {
    let file = config_dir.join(SECRET_FILE);
    let existing = read_env_file(&file)?;
    let mut out = Secrets::default();
    let mut fresh: Option<Secrets> = None;
    let mut make = Some(make);
    for (index, key) in SECRET_KEYS.iter().enumerate() {
        match existing.get(key) {
            Some(value) if value.encode_utf16().count() >= 32 => *out.slot(index) = value.to_owned(),
            _ => {
                if fresh.is_none() {
                    let make = make.take().ok_or_else(|| bad("secrets made twice"))?;
                    fresh = Some(make()?);
                }
                if let Some(fresh) = fresh.as_mut() {
                    *out.slot(index) = fresh.slot(index).clone();
                }
            }
        }
    }
    let created = fresh.is_some();
    if created {
        std::fs::create_dir_all(config_dir).map_err(|e| HostError::io(config_dir, e))?;
        let mut copy = out.clone();
        let entries: Vec<(&str, String)> = SECRET_KEYS
            .iter()
            .enumerate()
            .map(|(index, key)| (*key, copy.slot(index).clone()))
            .collect();
        write_secret_file(
            &file,
            &format_env(
                &[
                    "DaintyLass server secrets. SECRET: never share or commit this file.",
                    "Made once by `npm run setup`; changing one logs everyone out or breaks the invites already sent.",
                ],
                &entries,
            )?,
        )?;
    }
    Ok((out, created))
}

/// Owner-only where the platform supports it: written to `<file>.tmp` and renamed over the file.
pub fn write_secret_file(file: &Path, text: &str) -> Result<(), HostError> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| HostError::io(dir, e))?;
    }
    let mut tmp = file.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    {
        use std::io::Write;
        let mut handle = options.open(&tmp).map_err(|e| HostError::io(&tmp, e))?;
        handle.write_all(text.as_bytes()).map_err(|e| HostError::io(&tmp, e))?;
    }
    std::fs::rename(&tmp, file).map_err(|e| HostError::io(file, e))
}

// ------------------------------------------------------------------ the env files

/// The platform the env files are written for (Node's `process.platform`, as far as it matters here).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Windows,
    Linux,
    /// Anything else (macOS): the allowlist only writes what it would do.
    Other,
}

impl Platform {
    pub fn current() -> Self {
        if cfg!(windows) {
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
    /// Public mode only.
    pub allowlist: Option<EnvEntries>,
}

impl EnvFiles {
    /// `(name, entries)` in the order the files are written.
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

pub(crate) fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

pub fn build_env_files(c: &HostConfig, s: &Secrets, p: &Paths, source_commit: &str, platform: Platform) -> EnvFiles {
    let linux = platform == Platform::Linux;
    let lx = linux_options(c);
    let mut backend: Vec<(&str, String)> = vec![
        ("NODE_ENV", "production".into()),
        ("MYSTICPARADOX_SERVICE_ROLE", "api".into()),
        ("MYSTICPARADOX_METRICS_TOKEN", s.metrics_token.clone()),
        // Plain HTTP on this machine only; the gateway is the front door (it terminates TLS in public mode).
        ("PORT", PORTS.backend.to_string()),
        ("BIND_HOST", "127.0.0.1".into()),
        ("REFORGED_HTTP_ONLY", "true".into()),
        // The launcher answers the game's QoS ping itself, on its relay.
        ("QOS_PORT", "0".into()),
        ("QOS_TARGET_URL", format!("http://127.0.0.1:{}/QoS", PORTS.relay)),
        // Account keys from /reforged/api/Register are the game's AUTH_PASSWORD.
        ("AUTH_MODE", "APIKEY".into()),
        ("ALLOW_NO_AUTH_DEV_MODE", "false".into()),
        ("AUTH_SIGNING_PRIVKEY_B64", s.jwt_private_key_b64.clone()),
        ("AUTH_SIGNING_PUBKEY_B64", s.jwt_public_key_b64.clone()),
        ("API_KEY_HASH_SECRET", s.api_key_hash_secret.clone()),
        ("GAMESERVER_API_KEYS", s.gameserver_key.clone()),
        // Written only for a paradox rollback build; the default build uses [database] and migrates from [mongo].
        #[cfg(feature = "paradox")]
        ("MONGODB_URI", c.mongo.uri.clone()),
        #[cfg(feature = "paradox")]
        ("MONGODB_DB", "reforged".into()),
        #[cfg(feature = "paradox")]
        ("MONGODB_APP_NAME", "reforged-backend".into()),
        // A transaction commit may wait up to 15 s (default 2 s). On the VPS (2026-10-23 23:02-23:04) the disk
        // stalled for about a minute; every commit hit the 2 s limit, POST /inventory answered 500, and the
        // patrol chests, gathering and potions of that minute were lost.
        #[cfg(feature = "paradox")]
        ("MONGODB_TRANSACTION_MAX_COMMIT_TIME_MS", "15000".into()),
        ("TARGET_CHANGELIST", GAME_CHANGELIST.to_string()),
        ("APPROVED_EXECUTABLE_SHA256", PINNED_EXE_SHA256.into()),
        ("MATCHMAKING_MODE", "DEPLOYSERVER".into()),
        ("DEPLOYSERVER_URL", format!("127.0.0.1:{}", PORTS.director)),
        // After a hunt server is up, the backend holds the player a further random 10-20 s "Go Now"
        // countdown by default, with the build locked. The Director only answers once the server is
        // ready, so use the hub's 0-1 s window instead (the backend ignores 0, so the minimum is 1 ms).
        ("HUNT_GO_NOW_MIN_DELAY_MS", "1".into()),
        ("HUNT_GO_NOW_MAX_DELAY_MS", "1000".into()),
        ("REALTIME_XMPP_ENABLED", "true".into()),
        ("REALTIME_XMPP_CAPTURE", "false".into()),
        // The launcher's relay sends the invite's host as Host.
        ("REALTIME_XMPP_ALLOWED_HOSTS", c.host.clone()),
        ("REALTIME_XMPP_DEV_CITY_MUC", "false".into()),
        ("REGISTRATION_MODE", c.registration.as_str().into()),
        ("REFORGED_ADMIN_KEY", s.admin_key.clone()),
        // The admin API's signed calls to /admin/v1 (security/managerOperator.ts; docs/admin-api.md).
        ("MYSTICPARADOX_MANAGER_ADMIN", "on".into()),
        ("MYSTICPARADOX_MANAGER_OPERATOR_SECRET", s.operator_secret.clone()),
        ("REFORGED_SERVER_NAME", c.name.clone()),
        ("REFORGED_SERVER_VERSION", "0.1.0".into()),
        (
            "REFORGED_SOURCE_URL",
            "https://github.com/FreezaMotoqueiro/Dauntless-Reforged-Revival".into(),
        ),
        ("REFORGED_SOURCE_COMMIT", source_commit.into()),
        // The backend multiplies XP and rewards only as a private server on a Tailscale address.
        ("REFORGED_SERVER_MODE", c.mode.as_str().into()),
        ("REFORGED_SERVER_HOST", c.host.clone()),
    ];
    if let Some(m) = c.multipliers.filter(|_| multipliers_on(c)) {
        backend.push(("REFORGED_XP_MULTIPLIER", jsjson::number(m.xp)));
        backend.push(("REFORGED_REWARD_MULTIPLIER", jsjson::number(m.rewards)));
    }
    let mut switch_env: Vec<(&'static str, String)> = Vec::new();
    if let Some(mod_switches) = c
        .modules
        .as_ref()
        .and_then(|m| m.settings.iter().find(|(id, _)| id.as_str() == "isles-1-14-7"))
        .and_then(|(_, sections)| sections.get("switches"))
    {
        for (key, env_key) in [
            ("reforge", "MYSTICPARADOX_REFORGE"),
            ("mastery_persistence", "MYSTICPARADOX_MASTERY_PERSISTENCE"),
            ("bounty_persistence", "MYSTICPARADOX_BOUNTY_PERSISTENCE"),
            ("cooldown_persistence", "MYSTICPARADOX_COOLDOWN_PERSISTENCE"),
            ("account_erasure", "MYSTICPARADOX_ACCOUNT_ERASURE"),
            ("log_purge", "MYSTICPARADOX_LOG_PURGE"),
            ("rate_limit", "MYSTICPARADOX_RATE_LIMIT"),
            ("rate_limit_per_minute", "MYSTICPARADOX_RATE_LIMIT_PER_MINUTE"),
            (
                "gameserver_rate_limit_per_minute",
                "MYSTICPARADOX_GAMESERVER_RATE_LIMIT_PER_MINUTE",
            ),
            ("host_rate_limit_per_minute", "MYSTICPARADOX_HOST_RATE_LIMIT_PER_MINUTE"),
        ] {
            if let Some(v) = mod_switches.get(key) {
                let text = match v {
                    Value::String(s) => s.clone(),
                    Value::Bool(b) => if *b { "on" } else { "off" }.into(),
                    other => other.to_string(),
                };
                switch_env.push((env_key, text));
            }
        }
    }
    if let Some(switches) = &c.switches {
        for (env_key, val) in switches.env() {
            if let Some(slot) = switch_env.iter_mut().find(|(k, _)| *k == env_key) {
                slot.1 = val;
            } else {
                switch_env.push((env_key, val));
            }
        }
    }
    backend.extend(switch_env);
    backend.extend([
        // Stacked items given back to a fixed quantity at each login, per account (upstream patch 0023).
        (
            "REFORGED_SESSION_SUPPLIES_FILE",
            path_text(&p.config.join("session-supplies.json")),
        ),
        ("LOG_LEVEL", "info".into()),
    ]);

    let exe = c.game_dir.as_ref().map(|dir| Path::new(dir).join(game_exe_relative()));
    let exe_text = exe.as_deref().map(path_text).unwrap_or_default();
    let perf = performance_options(c);
    let (first, last) = game_ports(c);
    let mut director: Vec<(&str, String)> = vec![
        ("NODE_ENV", "production".into()),
        ("PORT", PORTS.director.to_string()),
        ("BIND_HOST", "127.0.0.1".into()),
        ("MY_IP", c.game_address.clone()),
        // Ramsgate on the last port, the Training Grounds on the one before, hunts below.
        ("PORT_RANGE_BEGIN", first.to_string()),
        ("PORT_RANGE_END", last.to_string()),
        // Linux: the launcher that runs the Windows executable through Wine.
        (
            "GAMESERVER_BINARY_PATH",
            if linux {
                path_text(&p.gameserver_wrapper)
            } else {
                exe_text.clone()
            },
        ),
    ];
    if linux {
        director.push(("REFORGED_GAME_EXE", exe_text.clone()));
        if let Some(wine) = &lx.wine {
            director.push(("REFORGED_WINE", wine.clone()));
        }
        if let Some(proton) = &lx.proton {
            director.push(("REFORGED_PROTON", proton.clone()));
        }
        director.extend([
            ("REFORGED_WINEPREFIX", path_text(&p.wine_prefix)),
            ("REFORGED_PROTON_DATA", path_text(&p.proton_data)),
            ("REFORGED_XVFB", if lx.xvfb { "1" } else { "0" }.into()),
        ]);
    }
    director.extend([
        ("METAGAME_API_KEY", s.gameserver_key.clone()),
        ("SECONDS_TO_WAIT_BETWEEN_GAMESERVER_STARTUP", "3".into()),
        // Under Wine a game server takes longer to load its first map.
        (
            "GAMESERVER_READY_TIMEOUT_MS",
            if linux { "90000" } else { "30000" }.into(),
        ),
        ("MAX_CONCURRENT_HUNT_SERVERS", c.max_hunt_servers.to_string()),
        // The Training Grounds start the first time someone goes there (Director patch 0021) ...
        (
            "TRAINING_ON_DEMAND",
            if perf.training_on_demand { "1" } else { "0" }.into(),
        ),
        // ... and stop again once empty for this long (Director patch 0033), told by the runtime's
        // log beside the game executable.
        (
            "TRAINING_IDLE_STOP_MINUTES",
            if perf.training_on_demand {
                perf.training_idle_minutes
            } else {
                0
            }
            .to_string(),
        ),
    ]);
    if let Some(dir) = exe.as_deref().and_then(Path::parent) {
        director.push(("GAMESERVER_RUNTIME_LOG_DIR", path_text(dir)));
    }
    director.extend([
        // A new hunt only when the memory for it is free (Director patch 0022).
        (
            "GAMESERVER_MEMORY_GUARD",
            if perf.memory_guard { "1" } else { "0" }.into(),
        ),
        ("GAMESERVER_MIN_FREE_MB", perf.min_free_memory_mb.to_string()),
        ("GAMESERVER_HUNT_MEMORY_MB", perf.hunt_memory_mb.to_string()),
        // The game servers inherit these two: the runtime caps each one's frame rate (runtime patch 0020).
        ("REFORGED_SERVER_MAX_FPS", perf.server_fps.to_string()),
        ("REFORGED_SERVER_IDLE_FPS", perf.idle_fps.to_string()),
    ]);
    // The game servers' -LogCmds (Director patch 0021); absent: upstream's diagnostic levels.
    if !perf.verbose_game_logs {
        director.push(("GAMESERVER_LOG_CMDS", QUIET_GAME_LOG_CMDS.into()));
    }
    director.extend([
        ("SERVER_RUNTIME_AUTO_UPDATE", "false".into()),
        ("SERVER_RUNTIME_UPDATE_REQUIRED", "false".into()),
        ("GAMESERVER_LOG_DIR", path_text(&p.gameserver_logs)),
        // The game servers inherit these: the runtime in each of them talks to the backend directly.
        ("REFORGED_BACKEND_URL", format!("http://127.0.0.1:{}", PORTS.backend)),
        ("MYSTICPARADOX_RUNTIME_PROFILE", "production".into()),
        ("LOG_LEVEL", "info".into()),
    ]);

    let content: Vec<(&str, String)> = vec![
        ("PORT", PORTS.content.to_string()),
        ("BIND_HOST", "127.0.0.1".into()),
        ("BACKEND_URL", format!("http://127.0.0.1:{}", PORTS.backend)),
        ("CONTENT_RUNTIME_DIR", path_text(&p.runtime_publish)),
        ("CONTENT_BRANDING_DIR", path_text(&p.branding)),
        ("CONTENT_NEWS_FILE", path_text(&p.news_file)),
    ];
    // Loopback only, never behind the gateway (docs/security/admin-api.md).
    let admin: Vec<(&str, String)> = vec![
        ("PORT", PORTS.admin.to_string()),
        ("BIND_HOST", "127.0.0.1".into()),
        ("BACKEND_URL", format!("http://127.0.0.1:{}", PORTS.backend)),
        ("REFORGED_DIRECTOR_URL", format!("http://127.0.0.1:{}", PORTS.director)),
        ("REFORGED_ADMIN_API_KEY", s.admin_api_key.clone()),
        ("REFORGED_ADMIN_KEY", s.admin_key.clone()),
        ("MYSTICPARADOX_MANAGER_OPERATOR_SECRET", s.operator_secret.clone()),
        (
            "REFORGED_ADMIN_API_TIMEOUTS_FILE",
            path_text(&p.config.join("admin-timeouts.json")),
        ),
        ("REFORGED_MAX_HUNT_SERVERS", c.max_hunt_servers.to_string()),
        ("REFORGED_SERVER_MODE", c.mode.as_str().into()),
    ];
    let guard = c.guard.as_ref().map(GuardConfig::env).unwrap_or_default();
    let admin = [admin, database_env(c, s, p), guard].concat();
    let common: [(&str, String); 4] = [
        ("GATEWAY_SECRET", s.gateway_secret.clone()),
        ("GATEWAY_BACKEND_URL", format!("http://127.0.0.1:{}", PORTS.backend)),
        ("GATEWAY_CONTENT_URL", format!("http://127.0.0.1:{}", PORTS.content)),
        ("GATEWAY_WS_URL", format!("http://127.0.0.1:{}", PORTS.backend)),
    ];
    let (gateway, allowlist) = match c.mode {
        ServerMode::Public => {
            let mut gateway: Vec<(&str, String)> = vec![
                ("GATEWAY_TLS", "1".into()),
                ("GATEWAY_BIND", "0.0.0.0".into()),
                ("GATEWAY_PORT", gateway_port(c).to_string()),
                ("GATEWAY_CERT", path_text(&p.cert_file)),
                ("GATEWAY_KEY", path_text(&p.key_file)),
            ];
            // Peers listed: this node answers them over mutual TLS and calls them (DN3). The files are the ones
            // `network issue-node-cert` wrote, copied to the TLS folder
            if let Some(network) = c.network.as_ref().filter(|n| !n.nodes.is_empty()) {
                gateway.extend(node_link_env(network, p));
            }
            gateway.extend(common);
            gateway.extend([
                ("ALLOWLIST_URL", format!("http://127.0.0.1:{}", PORTS.allowlist)),
                ("ALLOWLIST_SECRET", s.allowlist_secret.clone()),
            ]);
            // Windows Firewall on Windows, nftables on Linux; anywhere else it only writes what it would do.
            let mut a: Vec<(&str, String)> = vec![
                (
                    "ALLOWLIST_DRY_RUN",
                    if platform == Platform::Other { "1" } else { "0" }.into(),
                ),
                ("ALLOWLIST_FIREWALL", if linux { "nftables" } else { "windows" }.into()),
            ];
            if linux {
                a.push(("ALLOWLIST_NFT_TABLE", "reforged_revival".into()));
                if let Some(iface) = &lx.firewall_interface {
                    a.push(("ALLOWLIST_INTERFACE", iface.clone()));
                }
            }
            a.extend([
                ("ALLOWLIST_BIND", "127.0.0.1".into()),
                ("ALLOWLIST_PORT", PORTS.allowlist.to_string()),
                ("ALLOWLIST_SECRET", s.allowlist_secret.clone()),
                ("ALLOWLIST_PORTS", game_ports_text(c, "-")),
                ("ALLOWLIST_AUDIT_LOG", path_text(&p.logs.join("allowlist-audit.log"))),
                (
                    "ALLOWLIST_STATE_FILE",
                    path_text(&p.config.join("allowlist-state.json")),
                ),
            ]);
            (gateway, Some(a))
        }
        ServerMode::Private => {
            let mut gateway: Vec<(&str, String)> = vec![
                ("GATEWAY_TLS", "0".into()),
                ("GATEWAY_BIND", c.game_address.clone()),
                ("GATEWAY_PORT", PORTS.private_gateway.to_string()),
            ];
            gateway.extend(common);
            // Tailscale decides who can reach the game ports.
            gateway.push(("GATEWAY_ALLOWLIST", "0".into()));
            (gateway, None)
        }
    };
    let owned =
        |entries: Vec<(&str, String)>| -> EnvEntries { entries.into_iter().map(|(k, v)| (k.to_owned(), v)).collect() };
    EnvFiles {
        backend: owned(backend),
        director: owned(director),
        content: owned(content),
        gateway: owned(gateway),
        admin: owned(admin),
        allowlist: allowlist.map(owned),
    }
}

/// The node's files in the TLS folder, next to the gateway's own certificate (DN3).
pub const NODE_CERT_FILE: &str = "node-cert.pem";
pub const NODE_KEY_FILE: &str = "node-key.pem";
pub const CLUSTER_CA_FILE: &str = "cluster-ca-cert.pem";
/// The Ed25519 signing key for home-server session tickets in the TLS folder (DN5).
pub const SESSION_KEY_FILE: &str = "session-key";

/// The node database configuration (`store::DatabaseConfig`) read from `c`, `p`, and `secrets.env` (if present).
pub fn node_database_config(c: &HostConfig, p: &Paths) -> Result<store::DatabaseConfig, HostError> {
    let existing = read_env_file(&p.config.join(SECRET_FILE))?;
    let mut secrets = Secrets::default();
    if let Some(pw) = existing.get("REFORGED_DB_PASSWORD") {
        secrets.database_password = pw.to_string();
    }
    let entries = database_env(c, &secrets, p);
    store::DatabaseConfig::from_lookup(
        |key| entries.iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone()),
        p.config.join(DATABASE_FILE),
    )
    .map_err(HostError::message)
}

/// What the gateway reads to serve node links (`GATEWAY_NODE_*`, `GATEWAY_NETWORK_NODES`, DN3).
fn node_link_env(network: &NetworkConfig, p: &Paths) -> Vec<(&'static str, String)> {
    vec![
        ("GATEWAY_NODE_ID", network.node.to_string()),
        ("GATEWAY_NODE_REGION", network.region.to_string()),
        ("GATEWAY_NODE_CERT", path_text(&p.tls.join(NODE_CERT_FILE))),
        ("GATEWAY_NODE_KEY", path_text(&p.tls.join(NODE_KEY_FILE))),
        ("GATEWAY_NODE_CA", path_text(&p.tls.join(CLUSTER_CA_FILE))),
        (
            "GATEWAY_NETWORK_NODES",
            serde_json::to_string(&network.nodes).unwrap_or_default(),
        ),
    ]
}

/// The database settings `store::DatabaseConfig::from_lookup` reads, for a service that keeps the node's
/// database (DD15).
fn database_env(c: &HostConfig, s: &Secrets, p: &Paths) -> Vec<(&'static str, String)> {
    let d = c.database.clone().unwrap_or_default();
    if !d.postgres() {
        return vec![
            ("REFORGED_DB_ENGINE", "sqlite".into()),
            ("REFORGED_DB_FILE", path_text(&p.config.join(DATABASE_FILE))),
        ];
    }
    let mut out = vec![
        ("REFORGED_DB_ENGINE", "postgres".into()),
        (
            "REFORGED_DB_HOST",
            d.postgres_host.unwrap_or_else(|| "127.0.0.1".into()),
        ),
        ("REFORGED_DB_PORT", d.postgres_port.unwrap_or(5432).to_string()),
        ("REFORGED_DB_USER", d.postgres_user.unwrap_or_else(|| "dainty".into())),
        (
            "REFORGED_DB_NAME",
            d.postgres_database.unwrap_or_else(|| "dainty".into()),
        ),
        ("REFORGED_DB_PASSWORD", s.database_password.clone()),
    ];
    if d.allow_remote == Some(true) {
        out.push(("REFORGED_DB_ALLOW_REMOTE", "1".into()));
    }
    out
}

pub const ENV_FILE_HEADER: [&str; 2] = [
    "Written by ReforgedHost (`npm run setup`) from host.json and secrets.env. SECRET: never share or commit.",
    "Run setup again instead of editing: it rewrites this file.",
];

/// The env files only a legacy server has (`crate::legacy`).
pub const LEGACY_ENV_FILES: [&str; 2] = ["metagame", "deploy"];

/// Writes `<name>.env` in the config folder for each service, owner-only; the allowlist's file is
/// removed when there is none (private mode), and a legacy server's when it went back to reborn.
/// Returns the files written.
pub fn write_env_files(p: &Paths, files: &EnvFiles) -> Result<Vec<PathBuf>, HostError> {
    let mut list = files.files().to_vec();
    list.extend(LEGACY_ENV_FILES.map(|name| (name, None)));
    write_named_env_files(p, &list)
}

/// [`write_env_files`] for any set of files: `None` removes that one.
pub fn write_named_env_files(p: &Paths, files: &[(&str, Option<&EnvEntries>)]) -> Result<Vec<PathBuf>, HostError> {
    let mut written = Vec::new();
    for &(name, entries) in files {
        let file = p.config.join(format!("{name}.env"));
        match entries {
            None => match std::fs::remove_file(&file) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(HostError::io(&file, e)),
            },
            Some(entries) => {
                write_secret_file(&file, &format_env(&ENV_FILE_HEADER, entries)?)?;
                written.push(file);
            }
        }
    }
    Ok(written)
}

/// The file's variables; empty when there is no such file.
pub fn read_env_file(file: &Path) -> Result<EnvMap, HostError> {
    match std::fs::read_to_string(file) {
        Ok(text) => Ok(parse_env(&text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(EnvMap::default()),
        Err(e) => Err(HostError::io(file, e)),
    }
}
