// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The services' tunables as settings.toml tables (RD24 phase 2, docs/config-toml-plan.md): `[gateway.limits]`,
//! `[gateway.rate]`, `[gateway.timeouts]`, `[gateway]`, `[content]`, `[allowlist]`, `[backend]` and `[director]`.
//! Every setting is optional and only a set one reaches its service's env file, so a file without them writes
//! what setup wrote before. The defaults stay in each service's own loader. The gateway's, content's and the
//! allowlist helper's ranges are checked by those loaders on the rendered env (`hostconfig/check.rs`); the
//! backend's and the Director's loaders take any value, so their ranges are checked here.
//!
//! ```toml
//! [gateway.limits]
//! max_connections = 4096
//!
//! [gateway.rate]
//! ws = { burst = 40, per_minute = 24 }
//!
//! [backend]
//! log_level = "debug"
//! ```

use serde_json::{Map, Value};

use super::{EnvEntries, EnvFiles, bad, integer, shown};
use crate::error::HostError;
use dainty_service_kit::jsjson;

/// What a setting holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A whole number from `min` to `max`.
    Whole {
        /// The smallest value.
        min: u64,
        /// The largest value.
        max: u64,
    },
    /// A gateway token bucket: `{ burst, per_minute }`, written as `burst,per_minute` (`parse_bucket_spec`).
    Bucket,
    /// `debug`, `info`, `warn` or `error`.
    Level,
    /// `true` or `false`.
    Switch,
    /// One line of text.
    Text,
}

/// One tunable: its path in the table (host.json's names), its env variable and what it holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Setting {
    /// `limits.maxBodyBytes`.
    pub path: &'static str,
    /// `GATEWAY_MAX_BODY_BYTES`.
    pub env: &'static str,
    /// What it holds.
    pub kind: Kind,
}

/// A table: its name in settings.toml, which is also the env file it writes to, and its settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Table {
    /// `gateway`.
    pub name: &'static str,
    /// Its settings.
    pub settings: &'static [Setting],
}

/// The service's own loader checks the range on the rendered env, so only the type is checked here.
const ANY: Kind = Kind::Whole { min: 0, max: ANY_MAX };
/// `Number.MAX_SAFE_INTEGER`: what an `ANY` setting may be at most.
const ANY_MAX: u64 = 9_007_199_254_740_991;

const fn whole(path: &'static str, env: &'static str) -> Setting {
    Setting { path, env, kind: ANY }
}

const fn ranged(path: &'static str, env: &'static str, min: u64, max: u64) -> Setting {
    Setting {
        path,
        env,
        kind: Kind::Whole { min, max },
    }
}

const fn bucket(path: &'static str, env: &'static str) -> Setting {
    Setting {
        path,
        env,
        kind: Kind::Bucket,
    }
}

const fn of(path: &'static str, env: &'static str, kind: Kind) -> Setting {
    Setting { path, env, kind }
}

const GATEWAY: &[Setting] = &[
    whole("limits.maxBodyBytes", "GATEWAY_MAX_BODY_BYTES"),
    whole("limits.maxConnections", "GATEWAY_MAX_CONNECTIONS"),
    whole("limits.maxConnectionsPerIp", "GATEWAY_MAX_CONNECTIONS_PER_IP"),
    bucket("rate.general", "GATEWAY_RATE_GENERAL"),
    bucket("rate.content", "GATEWAY_RATE_CONTENT"),
    bucket("rate.register", "GATEWAY_RATE_REGISTER"),
    bucket("rate.token", "GATEWAY_RATE_TOKEN"),
    bucket("rate.ws", "GATEWAY_RATE_WS"),
    bucket("rate.connect", "GATEWAY_RATE_CONNECT"),
    whole("timeouts.handshakeMs", "GATEWAY_HANDSHAKE_TIMEOUT_MS"),
    whole("timeouts.headersMs", "GATEWAY_HEADERS_TIMEOUT_MS"),
    whole("timeouts.requestMs", "GATEWAY_REQUEST_TIMEOUT_MS"),
    whole("timeouts.keepAliveMs", "GATEWAY_KEEPALIVE_TIMEOUT_MS"),
    whole("timeouts.idleMs", "GATEWAY_IDLE_TIMEOUT_MS"),
    whole("timeouts.upstreamIdleMs", "GATEWAY_UPSTREAM_IDLE_TIMEOUT_MS"),
    whole("timeouts.wsIdleMs", "GATEWAY_WS_IDLE_TIMEOUT_MS"),
    whole("timeouts.shutdownGraceMs", "GATEWAY_SHUTDOWN_GRACE_MS"),
    whole("allowlistRefreshSeconds", "GATEWAY_ALLOWLIST_REFRESH_SECONDS"),
];

const CONTENT: &[Setting] = &[
    whole("maxStreamsPerAccount", "CONTENT_MAX_STREAMS_PER_ACCOUNT"),
    whole("maxStreamsTotal", "CONTENT_MAX_STREAMS_TOTAL"),
    whole("authCacheSeconds", "CONTENT_AUTH_CACHE_SECONDS"),
    of("newsUrl", "CONTENT_NEWS_URL", Kind::Text),
];

const ALLOWLIST: &[Setting] = &[
    whole("ttlSeconds", "ALLOWLIST_TTL_SECONDS"),
    whole("minIntervalMs", "ALLOWLIST_MIN_INTERVAL_MS"),
    whole("maxEntries", "ALLOWLIST_MAX_ENTRIES"),
    of("nftTable", "ALLOWLIST_NFT_TABLE", Kind::Text),
];

/// The most a delay or timeout here may be: ten minutes.
const TEN_MINUTES_MS: u64 = 600_000;

const BACKEND: &[Setting] = &[
    of("logLevel", "LOG_LEVEL", Kind::Level),
    ranged(
        "transactionMaxCommitMs",
        "MONGODB_TRANSACTION_MAX_COMMIT_TIME_MS",
        1,
        TEN_MINUTES_MS,
    ),
    ranged("huntGoNowMinMs", "HUNT_GO_NOW_MIN_DELAY_MS", 1, TEN_MINUTES_MS),
    ranged("huntGoNowMaxMs", "HUNT_GO_NOW_MAX_DELAY_MS", 1, TEN_MINUTES_MS),
    of("xmppCapture", "REALTIME_XMPP_CAPTURE", Kind::Switch),
];

const DIRECTOR: &[Setting] = &[
    of("logLevel", "LOG_LEVEL", Kind::Level),
    ranged(
        "gameserverStartupGapSeconds",
        "SECONDS_TO_WAIT_BETWEEN_GAMESERVER_STARTUP",
        0,
        600,
    ),
    ranged(
        "gameserverReadyTimeoutMs",
        "GAMESERVER_READY_TIMEOUT_MS",
        1000,
        TEN_MINUTES_MS,
    ),
    // The Director hands its env to every game server, which reads these before the number files (RT-11)
    ranged("hubMaxPlayers", "DAINTY_HUB_MAX_PLAYERS", 1, 64),
    ranged("hubAfkSeconds", "DAINTY_HUB_AFK_SECONDS", 0, AFK_MAX_SECONDS),
    ranged("huntAfkSeconds", "DAINTY_HUNT_AFK_SECONDS", 0, AFK_MAX_SECONDS),
    // Patches 0024 and 0029; the backend's status hands them to players' launchers too (BACKEND_MIRRORED)
    of("keepSupplyStackCaps", "DAINTY_KEEP_SUPPLY_STACK_CAPS", Kind::Switch),
    ranged("cellFusionMinutes", "DAINTY_CELL_FUSION_MINUTES", 0, 10_080),
];

/// The Director's settings the backend also gets, to send in its status for players' games (spec-runtime-r15 Q-1).
const BACKEND_MIRRORED: [&str; 2] = ["DAINTY_KEEP_SUPPLY_STACK_CAPS", "DAINTY_CELL_FUSION_MINUTES"];

/// The runtime's ceiling for an AFK timeout: a week (spec-runtime-r1 10.2).
const AFK_MAX_SECONDS: u64 = 604_800;

/// Every table, in settings.toml's order.
pub const TABLES: [Table; 5] = [
    Table {
        name: "gateway",
        settings: GATEWAY,
    },
    Table {
        name: "content",
        settings: CONTENT,
    },
    Table {
        name: "allowlist",
        settings: ALLOWLIST,
    },
    Table {
        name: "backend",
        settings: BACKEND,
    },
    Table {
        name: "director",
        settings: DIRECTOR,
    },
];

/// What setup writes when `[backend]` leaves the go-now window out.
pub const HUNT_GO_NOW_DEFAULTS: (u64, u64) = (1, 1000);

/// The levels `LOG_LEVEL` takes.
pub const LEVELS: [&str; 4] = ["debug", "info", "warn", "error"];

const MAX_TEXT_BYTES: usize = 2048;

/// The table named `name`.
#[must_use]
pub fn table(name: &str) -> Option<&'static Table> {
    TABLES.iter().find(|t| t.name == name)
}

/// The env variables `service`'s table may set, for the overrides (`hostconfig/overrides.rs`).
#[must_use]
pub fn env_names(service: &str) -> Vec<&'static str> {
    table(service).map_or_else(Vec::new, |t| t.settings.iter().map(|s| s.env).collect())
}

/// `gateway.rate.ws` for `GATEWAY_RATE_WS` in `service`'s file, as settings.toml spells it.
#[must_use]
pub fn toml_key(service: &str, env: &str) -> Option<String> {
    let t = table(service)?;
    let s = t.settings.iter().find(|s| s.env == env)?;
    let path: Vec<String> = s.path.split('.').map(crate::settingstoml::snake_case).collect();
    Some(format!("{}.{}", t.name, path.join(".")))
}

/// One table's settings, as host.json holds them (nested objects, camelCase keys), checked.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tunables {
    values: Map<String, Value>,
}

fn lookup<'a>(values: &'a Map<String, Value>, path: &str) -> Option<&'a Value> {
    let mut parts = path.split('.');
    let mut here = values.get(parts.next()?)?;
    for part in parts {
        here = here.as_object()?.get(part)?;
    }
    Some(here).filter(|v| !v.is_null())
}

fn check(name: &str, s: &Setting, value: &Value) -> Result<(), HostError> {
    let at = format!("{name}.{}", s.path);
    match s.kind {
        Kind::Whole { min, max } => {
            if !integer(Some(value)).is_some_and(|n| (min as f64..=max as f64).contains(&n)) {
                return Err(bad(if max == ANY_MAX {
                    format!("{at} must be a whole number, not {}", shown(Some(value)))
                } else {
                    format!(
                        "{at} must be a whole number from {min} to {max}, not {}",
                        shown(Some(value))
                    )
                }));
            }
        }
        Kind::Bucket => {
            let shape = || {
                bad(format!(
                    "{at} must be {{ burst = <whole number>, perMinute = <number> }}, not {}",
                    shown(Some(value))
                ))
            };
            let b = value.as_object().ok_or_else(shape)?;
            if b.keys().any(|k| k != "burst" && k != "perMinute")
                || !integer(b.get("burst")).is_some_and(|n| n >= 1.0)
                || !b.get("perMinute").and_then(Value::as_f64).is_some_and(|n| n >= 0.0)
            {
                return Err(shape());
            }
        }
        Kind::Level => {
            if !value.as_str().is_some_and(|w| LEVELS.contains(&w)) {
                return Err(bad(format!(
                    "{at} must be \"debug\", \"info\", \"warn\" or \"error\", not {}",
                    shown(Some(value))
                )));
            }
        }
        Kind::Switch => {
            if !value.is_boolean() {
                return Err(bad(format!("{at} must be true or false, not {}", shown(Some(value)))));
            }
        }
        Kind::Text => {
            if !value
                .as_str()
                .is_some_and(|t| t.len() <= MAX_TEXT_BYTES && !t.chars().any(char::is_control))
            {
                return Err(bad(format!(
                    "{at} must be one line of text of at most {MAX_TEXT_BYTES} bytes, not {}",
                    shown(Some(value))
                )));
            }
        }
    }
    Ok(())
}

/// Refuses a key that is neither a setting nor a group of them, so a typo isn't silently ignored.
fn known(name: &str, t: &Table, values: &Map<String, Value>, prefix: &str) -> Result<(), HostError> {
    for (key, value) in values {
        let path = format!("{prefix}{key}");
        if t.settings.iter().any(|s| s.path == path) {
            continue;
        }
        let group = format!("{path}.");
        if t.settings.iter().any(|s| s.path.starts_with(&group)) {
            let inner = value
                .as_object()
                .ok_or_else(|| bad(format!("{name}.{path} must be a table")))?;
            known(name, t, inner, &group)?;
            continue;
        }
        return Err(bad(format!("{name} has no setting {path}")));
    }
    Ok(())
}

impl Tunables {
    /// Reads and checks the table `name` (`gateway`, `content`, …) as host.json holds it.
    pub fn from_json(name: &str, value: &Value) -> Result<Self, HostError> {
        let t = table(name).ok_or_else(|| bad(format!("{name} is not a service table")))?;
        let values = value
            .as_object()
            .ok_or_else(|| bad(format!("{name} must be a table ([{name}])")))?;
        known(name, t, values, "")?;
        for s in t.settings {
            if let Some(v) = lookup(values, s.path) {
                check(name, s, v)?;
            }
        }
        if name == "backend" {
            let (dmin, dmax) = HUNT_GO_NOW_DEFAULTS;
            let get = |p: &str, d: u64| lookup(values, p).and_then(Value::as_u64).unwrap_or(d);
            let (min, max) = (get("huntGoNowMinMs", dmin), get("huntGoNowMaxMs", dmax));
            if min > max {
                return Err(bad(format!(
                    "backend.huntGoNowMinMs ({min}) must not be more than backend.huntGoNowMaxMs ({max})"
                )));
            }
        }
        Ok(Self { values: values.clone() })
    }

    /// host.json's form: what [`from_json`](Self::from_json) reads.
    #[must_use]
    pub fn to_json(&self) -> Value {
        Value::Object(self.values.clone())
    }

    /// The value at `path` (`rate.ws`), if set.
    #[must_use]
    pub fn get(&self, path: &str) -> Option<&Value> {
        lookup(&self.values, path)
    }

    /// The env lines of table `name`: only what is set, in the table's order.
    #[must_use]
    pub fn env(&self, name: &str) -> Vec<(&'static str, String)> {
        let Some(t) = table(name) else {
            return Vec::new();
        };
        t.settings
            .iter()
            .filter_map(|s| self.get(s.path).map(|v| (s.env, env_text(s.kind, v))))
            .collect()
    }
}

fn env_text(kind: Kind, value: &Value) -> String {
    match (kind, value) {
        (Kind::Bucket, Value::Object(b)) => {
            let burst = b.get("burst").and_then(Value::as_u64).unwrap_or(1);
            let per_minute = b.get("perMinute").and_then(Value::as_f64).unwrap_or(0.0);
            format!("{burst},{}", jsjson::number(per_minute))
        }
        (_, Value::String(s)) => s.clone(),
        (_, other) => other.to_string(),
    }
}

fn upsert(file: &mut EnvEntries, key: &str, value: String) {
    match file.iter_mut().find(|(k, _)| k == key) {
        Some((_, existing)) => *existing = value,
        None => file.push((key.to_owned(), value)),
    }
}

/// Writes each table's settings into its service's file, over what setup wrote by default. `[allowlist]` on a
/// server without the helper writes nothing.
pub fn apply(files: &mut EnvFiles, tables: [(&str, Option<&Tunables>); 5]) {
    for (name, tunables) in tables {
        let Some(tunables) = tunables else { continue };
        let file = match name {
            "gateway" => &mut files.gateway,
            "content" => &mut files.content,
            "backend" => &mut files.backend,
            "director" => &mut files.director,
            "allowlist" => match files.allowlist.as_mut() {
                Some(file) => file,
                None => continue,
            },
            _ => continue,
        };
        let env = tunables.env(name);
        for (key, value) in &env {
            upsert(file, key, value.clone());
        }
        if name == "director" {
            for (key, value) in env.into_iter().filter(|(key, _)| BACKEND_MIRRORED.contains(key)) {
                upsert(&mut files.backend, key, value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use super::*;
    use serde_json::json;

    #[test]
    fn set_settings_become_env_lines_in_table_order() {
        let g = Tunables::from_json(
            "gateway",
            &json!({
                "rate": { "ws": { "burst": 40, "perMinute": 24 }, "register": { "burst": 5, "perMinute": 0.2 } },
                "limits": { "maxConnections": 4096 },
                "allowlistRefreshSeconds": 30
            }),
        )
        .unwrap();
        assert_eq!(
            g.env("gateway"),
            [
                ("GATEWAY_MAX_CONNECTIONS", "4096".to_owned()),
                ("GATEWAY_RATE_REGISTER", "5,0.2".to_owned()),
                ("GATEWAY_RATE_WS", "40,24".to_owned()),
                ("GATEWAY_ALLOWLIST_REFRESH_SECONDS", "30".to_owned()),
            ]
        );
        let b = Tunables::from_json("backend", &json!({ "logLevel": "debug", "xmppCapture": true })).unwrap();
        assert_eq!(
            b.env("backend"),
            [
                ("LOG_LEVEL", "debug".to_owned()),
                ("REALTIME_XMPP_CAPTURE", "true".to_owned())
            ]
        );
        assert_eq!(Tunables::from_json("backend", &b.to_json()).unwrap(), b);
    }

    #[test]
    fn bad_values_and_unknown_keys_are_refused() {
        for (name, doc, says) in [
            (
                "gateway",
                json!({ "limits": { "maxBody": 1 } }),
                "gateway has no setting limits.maxBody",
            ),
            ("gateway", json!({ "limits": 3 }), "gateway.limits must be a table"),
            (
                "gateway",
                json!({ "limits": { "maxConnections": 1.5 } }),
                "must be a whole number",
            ),
            (
                "gateway",
                json!({ "rate": { "ws": "40,24" } }),
                "gateway.rate.ws must be { burst",
            ),
            (
                "gateway",
                json!({ "rate": { "ws": { "burst": 0, "perMinute": 1 } } }),
                "burst",
            ),
            ("content", json!({ "newsUrl": "a\nb" }), "one line"),
            ("backend", json!({ "logLevel": "loud" }), "\"debug\", \"info\""),
            ("backend", json!({ "xmppCapture": "yes" }), "true or false"),
            ("backend", json!({ "huntGoNowMinMs": 0 }), "from 1 to 600000"),
            ("backend", json!({ "huntGoNowMinMs": 2000 }), "must not be more than"),
            (
                "director",
                json!({ "gameserverReadyTimeoutMs": 10 }),
                "from 1000 to 600000",
            ),
            ("director", json!({ "hubMaxPlayers": 0 }), "from 1 to 64"),
            ("director", json!({ "hubMaxPlayers": 65 }), "from 1 to 64"),
            ("director", json!({ "hubAfkSeconds": -1 }), "from 0 to 604800"),
            ("director", json!({ "huntAfkSeconds": 604_801 }), "from 0 to 604800"),
        ] {
            let err = Tunables::from_json(name, &doc).unwrap_err().to_string();
            assert!(err.contains(says), "{doc}: {err}");
        }
    }

    #[test]
    fn apply_replaces_what_setup_wrote_and_skips_a_missing_allowlist() {
        let mut files = EnvFiles {
            backend: vec![("LOG_LEVEL".into(), "info".into())],
            director: Vec::new(),
            content: Vec::new(),
            gateway: Vec::new(),
            admin: Vec::new(),
            allowlist: None,
            discord: None,
            passthrough_refused: Vec::new(),
        };
        let b = Tunables::from_json("backend", &json!({ "logLevel": "warn" })).unwrap();
        let a = Tunables::from_json("allowlist", &json!({ "ttlSeconds": 60 })).unwrap();
        apply(
            &mut files,
            [
                ("gateway", None),
                ("content", None),
                ("allowlist", Some(&a)),
                ("backend", Some(&b)),
                ("director", None),
            ],
        );
        assert_eq!(files.backend, [("LOG_LEVEL".to_owned(), "warn".to_owned())]);
        assert!(files.allowlist.is_none());
    }

    #[test]
    fn rt11_settings_reach_the_director_only_when_set() {
        let d = Tunables::from_json(
            "director",
            &json!({
                "hubMaxPlayers": 8,
                "hubAfkSeconds": 0,
                "huntAfkSeconds": 900,
                "keepSupplyStackCaps": true,
                "cellFusionMinutes": 0,
            }),
        )
        .unwrap();
        assert_eq!(
            d.env("director"),
            [
                ("DAINTY_HUB_MAX_PLAYERS", "8".to_owned()),
                ("DAINTY_HUB_AFK_SECONDS", "0".to_owned()),
                ("DAINTY_HUNT_AFK_SECONDS", "900".to_owned()),
                ("DAINTY_KEEP_SUPPLY_STACK_CAPS", "true".to_owned()),
                ("DAINTY_CELL_FUSION_MINUTES", "0".to_owned()),
            ]
        );
        for bad in [
            json!({ "cellFusionMinutes": 10_081 }),
            json!({ "keepSupplyStackCaps": "yes" }),
        ] {
            assert!(Tunables::from_json("director", &bad).is_err(), "{bad}");
        }
        assert!(
            Tunables::from_json("director", &json!({}))
                .unwrap()
                .env("director")
                .is_empty()
        );
        assert_eq!(
            toml_key("director", "DAINTY_HUNT_AFK_SECONDS").as_deref(),
            Some("director.hunt_afk_seconds")
        );
    }

    #[test]
    fn toml_keys_name_the_env_variables() {
        assert_eq!(
            toml_key("gateway", "GATEWAY_RATE_WS").as_deref(),
            Some("gateway.rate.ws")
        );
        assert_eq!(
            toml_key("gateway", "GATEWAY_MAX_CONNECTIONS_PER_IP").as_deref(),
            Some("gateway.limits.max_connections_per_ip")
        );
        assert_eq!(toml_key("gateway", "GATEWAY_SECRET"), None);
        assert!(env_names("director").contains(&"GAMESERVER_READY_TIMEOUT_MS"));
    }
}
