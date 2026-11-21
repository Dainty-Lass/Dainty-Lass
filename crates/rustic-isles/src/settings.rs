// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! A module's settings (core spec §6): forced values over the env file over the `[switches]` section. The process
//! environment is never read, since a compiled-in module shares the controller's.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use rustic_isles_auth::{AuthSettings, BudgetConfig};
use serde_json::Value;

/// A `[switches]` key and the setting it writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedSwitch {
    /// The setting it writes, such as `QOS_PORT`.
    pub env: &'static str,
    /// The `[switches]` key.
    pub key: &'static str,
    /// One line for the host kit's help.
    pub help: &'static str,
}

const fn switch(key: &'static str, env: &'static str, help: &'static str) -> SharedSwitch {
    SharedSwitch { env, key, help }
}

/// The version's switches, with their defaults in the version's `switch_defaults`.
pub const SWITCHES: [SharedSwitch; 5] = [
    switch("qos_port", "QOS_PORT", "The region-ping listener's port, or off"),
    switch(
        "reforge",
        "RUSTIC_ISLES_REFORGE",
        "Weapon reforge rules: on, log or off",
    ),
    switch(
        "mastery_persistence",
        "RUSTIC_ISLES_MASTERY_PERSISTENCE",
        "Whether a game server's mastery report is saved: on or off",
    ),
    switch(
        "bounty_persistence",
        "RUSTIC_ISLES_BOUNTY_PERSISTENCE",
        "Whether bounty saves are stored: on or off",
    ),
    switch(
        "cooldown_persistence",
        "RUSTIC_ISLES_COOLDOWN_PERSISTENCE",
        "Whether cooldown saves are stored: on or off",
    ),
];

/// Keys with a `null` default; their help is `Optional host switch ({env})`.
pub const OPTIONAL_SWITCHES: [SharedSwitch; 7] = [
    switch("account_erasure", "RUSTIC_ISLES_ACCOUNT_ERASURE", ""),
    switch("log_purge", "RUSTIC_ISLES_LOG_PURGE", ""),
    switch("rate_limit", "RUSTIC_ISLES_RATE_LIMIT", ""),
    switch("rate_limit_per_minute", "RUSTIC_ISLES_RATE_LIMIT_PER_MINUTE", ""),
    switch(
        "gameserver_rate_limit_per_minute",
        "RUSTIC_ISLES_GAMESERVER_RATE_LIMIT_PER_MINUTE",
        "",
    ),
    switch(
        "host_rate_limit_per_minute",
        "RUSTIC_ISLES_HOST_RATE_LIMIT_PER_MINUTE",
        "",
    ),
    switch("gauntlet", "RUSTIC_ISLES_GAUNTLET", ""),
];

/// A settings file could not be read or a value is not allowed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct SettingsError(pub String);

type Map = BTreeMap<String, String>;

/// A module's settings: forced values over the env file over the `[switches]` section. `Debug` shows names only.
#[derive(Clone, Default)]
pub struct Settings {
    forced: Map,
    env: Map,
    switches: Map,
}

// Names only: values hold signing keys and secrets (core spec §17)
impl fmt::Debug for Settings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Settings")
            .field("forced", &self.forced.keys().collect::<Vec<_>>())
            .field("env", &self.env.keys().collect::<Vec<_>>())
            .field("switches", &self.switches.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// A JSON config value as text: strings as they are, `null` none, anything else its JSON text.
pub fn config_text(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        other => Some(dainty_service_kit::jsjson::stringify(other)),
    }
}

fn is_off_word(value: &str) -> bool {
    ["0", "false", "off", "no"]
        .iter()
        .any(|word| word.eq_ignore_ascii_case(value))
}

impl Settings {
    /// Settings from the env file and the switches, nothing forced.
    pub fn new(env: BTreeMap<String, String>, switches: BTreeMap<String, String>) -> Self {
        Self {
            forced: Map::new(),
            env,
            switches,
        }
    }

    /// Env-file values only (tests and small hosts).
    pub fn from_pairs<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let env = pairs.into_iter().map(|(k, v)| (k.to_owned(), v.to_owned())).collect();
        Self::new(env, Map::new())
    }

    /// Sets a value over both other layers (the module's own config, §6.4).
    pub fn forced(mut self, name: &str, value: &str) -> Self {
        self.forced.insert(name.to_owned(), value.to_owned());
        self
    }

    /// `other`'s three maps over these.
    pub fn overlay(&self, other: &Settings) -> Self {
        let merge = |base: &Map, top: &Map| {
            let mut merged = base.clone();
            merged.extend(top.iter().map(|(k, v)| (k.clone(), v.clone())));
            merged
        };
        Self {
            forced: merge(&self.forced, &other.forced),
            env: merge(&self.env, &other.env),
            switches: merge(&self.switches, &other.switches),
        }
    }

    /// The `[switches]` section as settings: known keys only, `null` skipped.
    pub fn switches_from_config(section: Option<&BTreeMap<String, Value>>) -> BTreeMap<String, String> {
        let mut out = Map::new();
        let Some(section) = section else {
            return out;
        };
        for switch in SWITCHES.iter().chain(OPTIONAL_SWITCHES.iter()) {
            if let Some(text) = section.get(switch.key).and_then(config_text) {
                out.insert(switch.env.to_owned(), text);
            }
        }
        out
    }

    /// A dotenv file as a map; a failure is `{path}: {message}`.
    pub fn read_env_file(path: &Path) -> Result<BTreeMap<String, String>, SettingsError> {
        let fail = |message: String| SettingsError(format!("{}: {message}", path.display()));
        let mut out = Map::new();
        for item in dotenvy::from_path_iter(path).map_err(|e| fail(e.to_string()))? {
            let (name, value) = item.map_err(|e| fail(e.to_string()))?;
            out.insert(name, value);
        }
        Ok(out)
    }

    /// Every setting as one map (switches, then the env file, then forced): what the Director's game servers
    /// inherit.
    pub fn vars(&self) -> BTreeMap<String, String> {
        let mut out = self.switches.clone();
        out.extend(self.env.iter().map(|(k, v)| (k.clone(), v.clone())));
        out.extend(self.forced.iter().map(|(k, v)| (k.clone(), v.clone())));
        out
    }

    /// The env file's values alone.
    pub fn env_map(&self) -> &BTreeMap<String, String> {
        &self.env
    }

    /// The value as set, empty included (Node's `process.env.X`).
    pub fn raw(&self, name: &str) -> Option<&str> {
        self.forced
            .get(name)
            .or_else(|| self.env.get(name))
            .or_else(|| self.switches.get(name))
            .map(String::as_str)
    }

    /// `raw`, but none when empty.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.raw(name).filter(|v| !v.is_empty())
    }

    /// False only for `0`, `false`, `off` or `no` as set (not trimmed); unset is on.
    pub fn on_unless_off(&self, name: &str) -> bool {
        self.raw(name).is_none_or(|value| !is_off_word(value))
    }

    fn budget(&self) -> BudgetConfig {
        BudgetConfig::from_settings(self)
    }

    /// Whether the request budget is on (`RUSTIC_ISLES_RATE_LIMIT`).
    pub fn rate_limit_enabled(&self) -> bool {
        self.budget().enabled
    }

    /// A client's requests per minute (`RUSTIC_ISLES_RATE_LIMIT_PER_MINUTE`, default 600).
    pub fn rate_limit_per_minute(&self) -> u64 {
        self.budget().client_per_minute
    }

    /// A game server's requests per minute (default 6000).
    pub fn gameserver_rate_limit_per_minute(&self) -> u64 {
        self.budget().gameserver_per_minute
    }

    /// A player host's requests per minute (default 6000).
    pub fn host_rate_limit_per_minute(&self) -> u64 {
        self.budget().host_per_minute
    }

    /// `RUSTIC_ISLES_ADMIN_API`: on, off, or blank to follow `RUSTIC_ISLES_MANAGER_ADMIN`.
    pub fn admin_api(&self) -> Result<bool, SettingsError> {
        let read = |name: &str| self.raw(name).unwrap_or("").trim().to_lowercase();
        match read("RUSTIC_ISLES_ADMIN_API").as_str() {
            "on" => Ok(true),
            "off" => Ok(false),
            "" => match read("RUSTIC_ISLES_MANAGER_ADMIN").as_str() {
                "" | "off" => Ok(false),
                "on" => Ok(true),
                _ => Err(SettingsError(
                    "RUSTIC_ISLES_MANAGER_ADMIN: must be on or off (blank means off)".to_owned(),
                )),
            },
            _ => Err(SettingsError(
                "RUSTIC_ISLES_ADMIN_API: must be on or off (blank follows RUSTIC_ISLES_MANAGER_ADMIN)".to_owned(),
            )),
        }
    }

    /// Replay headers count only with `RUSTIC_ISLES_REPLAY_HEADERS` on (core spec §17, a departure from Node).
    pub fn replay_headers(&self) -> bool {
        let value = self
            .raw("RUSTIC_ISLES_REPLAY_HEADERS")
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        matches!(value.as_str(), "on" | "true" | "1" | "yes")
    }

    /// Whether the composer should open a database.
    pub fn database_wanted(&self) -> bool {
        self.get("DAINTY_DB_FILE").is_some() || self.get("DAINTY_DB_ENGINE").is_some()
    }
}

impl AuthSettings for Settings {
    fn raw(&self, name: &str) -> Option<&str> {
        Settings::raw(self, name)
    }
}

impl rustic_rifts_seam::Settings for Settings {
    fn get(&self, name: &str) -> Option<&str> {
        Settings::raw(self, name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layers_and_readers() {
        let section = BTreeMap::from([
            ("reforge".to_owned(), Value::from("off")),
            ("qos_port".to_owned(), Value::from(80)),
            ("rate_limit".to_owned(), Value::Null),
            ("unknown".to_owned(), Value::from(true)),
        ]);
        let switches = Settings::switches_from_config(Some(&section));
        assert_eq!(switches.len(), 2);
        let env = BTreeMap::from([("RUSTIC_ISLES_REFORGE".to_owned(), "log".to_owned())]);
        let settings = Settings::new(env, switches);
        assert_eq!(settings.raw("RUSTIC_ISLES_REFORGE"), Some("log"));
        assert_eq!(settings.raw("QOS_PORT"), Some("80"));
        assert_eq!(settings.raw("RUSTIC_ISLES_RATE_LIMIT"), None);
        assert!(settings.on_unless_off("X"));
        let forced = settings.clone().forced("RUSTIC_ISLES_REFORGE", "on");
        assert_eq!(forced.raw("RUSTIC_ISLES_REFORGE"), Some("on"));
        assert_eq!(forced.vars()["QOS_PORT"], "80");
        assert!(!Settings::from_pairs([("A", "OFF")]).on_unless_off("A"));
        assert!(Settings::from_pairs([("A", " off")]).on_unless_off("A"));
        assert_eq!(Settings::from_pairs([("A", "")]).get("A"), None);
        let over = Settings::from_pairs([("A", "1")]).overlay(&Settings::from_pairs([("A", "2")]));
        assert_eq!(over.raw("A"), Some("2"));
    }

    #[test]
    fn admin_api() {
        assert_eq!(Settings::default().admin_api(), Ok(false));
        assert_eq!(
            Settings::from_pairs([("RUSTIC_ISLES_ADMIN_API", " On ")]).admin_api(),
            Ok(true)
        );
        assert_eq!(
            Settings::from_pairs([("RUSTIC_ISLES_MANAGER_ADMIN", "on")]).admin_api(),
            Ok(true)
        );
        assert!(
            Settings::from_pairs([("RUSTIC_ISLES_ADMIN_API", "yes")])
                .admin_api()
                .is_err()
        );
        assert!(
            Settings::from_pairs([("RUSTIC_ISLES_MANAGER_ADMIN", "yes")])
                .admin_api()
                .is_err()
        );
    }

    #[test]
    fn debug_names_only() {
        let settings = Settings::from_pairs([("AUTH_SIGNING_PRIVKEY_B64", "very-secret")]);
        let text = format!("{settings:?}");
        assert!(text.contains("AUTH_SIGNING_PRIVKEY_B64") && !text.contains("very-secret"));
    }

    #[test]
    fn budget_readers() {
        for off in ["0", "off", "OFF", " false ", "No"] {
            assert!(!Settings::from_pairs([("RUSTIC_ISLES_RATE_LIMIT", off)]).rate_limit_enabled());
        }
        for bad in ["0", "-10", "not-a-number"] {
            let s = Settings::from_pairs([
                ("RUSTIC_ISLES_RATE_LIMIT_PER_MINUTE", bad),
                ("RUSTIC_ISLES_GAMESERVER_RATE_LIMIT_PER_MINUTE", bad),
                ("RUSTIC_ISLES_HOST_RATE_LIMIT_PER_MINUTE", bad),
            ]);
            assert_eq!(
                (
                    s.rate_limit_per_minute(),
                    s.gameserver_rate_limit_per_minute(),
                    s.host_rate_limit_per_minute()
                ),
                (600, 6000, 6000)
            );
        }
        assert!(Settings::from_pairs([("RUSTIC_ISLES_REPLAY_HEADERS", " YES ")]).replay_headers());
        assert!(!Settings::default().replay_headers());
    }
}
