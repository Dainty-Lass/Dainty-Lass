//! What the services read in place of Node's `process.env` (RU3c, seam 6).
//!
//! Two layers. The env file the module's config names comes first, read under Node's variable names. Under it, the
//! shared switches whose default differs by version ([`SWITCHES`]): their value is the module's `[switches]` config
//! section, which the registration fills with the version's default and `host.toml` overrides. So a host's own
//! setting always wins, and a host that sets nothing runs each version as its Node service did. The process
//! environment is never read: a compiled-in module shares the controller's.

use std::collections::BTreeMap;
use std::path::Path;

/// A setting both versions read whose default is the version's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedSwitch {
    /// Node's variable
    pub env: &'static str,
    /// The `[switches]` key in `host.toml` (snake_case)
    pub key: &'static str,
    pub help: &'static str,
}

pub const QOS_PORT: SharedSwitch = SharedSwitch {
    env: "QOS_PORT",
    key: "qos_port",
    help: "The region-ping listener's port, or off",
};
pub const REFORGE: SharedSwitch = SharedSwitch {
    env: "MYSTICPARADOX_REFORGE",
    key: "reforge",
    help: "Weapon reforge rules: on, log or off",
};
pub const MASTERY_PERSISTENCE: SharedSwitch = SharedSwitch {
    env: "MYSTICPARADOX_MASTERY_PERSISTENCE",
    key: "mastery_persistence",
    help: "Whether a game server's mastery report is saved: on or off",
};

pub const BOUNTY_PERSISTENCE: SharedSwitch = SharedSwitch {
    env: "MYSTICPARADOX_BOUNTY_PERSISTENCE",
    key: "bounty_persistence",
    help: "Whether bounty saves are stored (off is 1.12.0's empty stub): on or off",
};
pub const COOLDOWN_PERSISTENCE: SharedSwitch = SharedSwitch {
    env: "MYSTICPARADOX_COOLDOWN_PERSISTENCE",
    key: "cooldown_persistence",
    help: "Whether cooldown saves are stored (off is 1.12.0's empty stub): on or off",
};

pub const SWITCHES: [SharedSwitch; 5] = [
    QOS_PORT,
    REFORGE,
    MASTERY_PERSISTENCE,
    BOUNTY_PERSISTENCE,
    COOLDOWN_PERSISTENCE,
];

/// Additional `[switches]` keys in `host.toml` whose defaults are not version-table overrides.
pub const OPTIONAL_SWITCHES: [(&str, &str); 6] = [
    ("account_erasure", "MYSTICPARADOX_ACCOUNT_ERASURE"),
    ("log_purge", "MYSTICPARADOX_LOG_PURGE"),
    ("rate_limit", "MYSTICPARADOX_RATE_LIMIT"),
    ("rate_limit_per_minute", "MYSTICPARADOX_RATE_LIMIT_PER_MINUTE"),
    (
        "gameserver_rate_limit_per_minute",
        "MYSTICPARADOX_GAMESERVER_RATE_LIMIT_PER_MINUTE",
    ),
    ("host_rate_limit_per_minute", "MYSTICPARADOX_HOST_RATE_LIMIT_PER_MINUTE"),
];

fn parse_positive_int(raw: Option<&str>, fallback: u32) -> u32 {
    let trimmed = raw.unwrap_or_default().trim();
    let digits = trimmed.strip_prefix('+').unwrap_or(trimmed);
    let prefix: String = digits.chars().take_while(char::is_ascii_digit).collect();
    match prefix.parse::<u32>() {
        Ok(value) if value > 0 => value,
        _ => fallback,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("{path}: {message}")]
    File { path: String, message: String },
    #[error("{name}: {message}")]
    Value { name: String, message: String },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    env: BTreeMap<String, String>,
    switches: BTreeMap<String, String>,
    /// Set by the controller's config, over the env file: the game servers' port range
    forced: BTreeMap<String, String>,
}

impl Settings {
    /// `env` from the env file; `switches` the `[switches]` section as config text.
    pub fn new(env: BTreeMap<String, String>, switches: BTreeMap<String, String>) -> Self {
        Self {
            env,
            switches,
            forced: BTreeMap::new(),
        }
    }

    pub fn from_pairs<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        Self {
            env: pairs.into_iter().map(|(k, v)| (k.to_owned(), v.to_owned())).collect(),
            switches: BTreeMap::new(),
            forced: BTreeMap::new(),
        }
    }

    /// A setting the module's config decides: it reads as set, whatever the env file says.
    pub fn forced(mut self, name: &str, value: impl ToString) -> Self {
        self.forced.insert(name.to_owned(), value.to_string());
        self
    }

    /// These settings with `over`'s on top: its env file values win, then its switches.
    pub fn overlay(mut self, over: &Settings) -> Self {
        self.env.extend(over.env.clone());
        self.switches.extend(over.switches.clone());
        self.forced.extend(over.forced.clone());
        self
    }

    /// The `[switches]` section of a module config, keyed by Node's variable names.
    pub fn switches_from_config(section: Option<&BTreeMap<String, serde_json::Value>>) -> BTreeMap<String, String> {
        SWITCHES
            .iter()
            .map(|switch| (switch.key, switch.env))
            .chain(OPTIONAL_SWITCHES.iter().copied())
            .filter_map(|(key, env)| {
                let value = section?.get(key)?;
                let text = match value {
                    serde_json::Value::String(text) => text.clone(),
                    serde_json::Value::Null => return None,
                    other => other.to_string(),
                };
                Some((env.to_owned(), text))
            })
            .collect()
    }

    /// Reads a dotenv file, and nothing else.
    pub fn read_env_file(path: &Path) -> Result<BTreeMap<String, String>, SettingsError> {
        let error = |message: String| SettingsError::File {
            path: path.display().to_string(),
            message,
        };
        let mut env = BTreeMap::new();
        for item in dotenvy::from_path_iter(path).map_err(|e| error(e.to_string()))? {
            let (name, value) = item.map_err(|e| error(e.to_string()))?;
            env.insert(name, value);
        }
        Ok(env)
    }

    /// Every setting as the module sees it: the switches' defaults under the env file's values. What the Director's
    /// game servers inherit (Node's `...process.env`), the process environment aside.
    pub fn vars(&self) -> BTreeMap<String, String> {
        let mut vars = self.switches.clone();
        vars.extend(self.env.iter().map(|(k, v)| (k.clone(), v.clone())));
        vars.extend(self.forced.iter().map(|(k, v)| (k.clone(), v.clone())));
        vars
    }

    /// The env file's variables (the release gates look at their names).
    pub fn env_map(&self) -> &BTreeMap<String, String> {
        &self.env
    }

    /// The value as set, also when empty: Node's `process.env.X`, which `??` treats as set.
    pub fn raw(&self, name: &str) -> Option<&str> {
        self.forced
            .get(name)
            .or_else(|| self.env.get(name))
            .or_else(|| self.switches.get(name))
            .map(String::as_str)
    }

    /// The value, or `None` when unset or empty.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.raw(name).filter(|value| !value.is_empty())
    }

    /// Node's `!/^(0|false|off|no)$/i.test(value ?? "")`: on unless switched off.
    pub fn on_unless_off(&self, name: &str) -> bool {
        let value = self.raw(name).unwrap_or_default();
        !["0", "false", "off", "no"]
            .iter()
            .any(|off| value.eq_ignore_ascii_case(off))
    }

    /// `MYSTICPARADOX_RATE_LIMIT` (`security/rateLimit.ts`): on unless `0`, `off`, `false` or `no` (trimmed, any case).
    pub fn rate_limit_enabled(&self) -> bool {
        let raw = self.raw("MYSTICPARADOX_RATE_LIMIT").unwrap_or("on").trim();
        !["0", "off", "false", "no"]
            .iter()
            .any(|off| raw.eq_ignore_ascii_case(off))
    }

    /// `MYSTICPARADOX_RATE_LIMIT_PER_MINUTE` (`security/rateLimit.ts`), default 600.
    pub fn rate_limit_per_minute(&self) -> u32 {
        parse_positive_int(self.raw("MYSTICPARADOX_RATE_LIMIT_PER_MINUTE"), 600)
    }

    /// `MYSTICPARADOX_GAMESERVER_RATE_LIMIT_PER_MINUTE` (`security/rateLimit.ts`), default 6000.
    pub fn gameserver_rate_limit_per_minute(&self) -> u32 {
        parse_positive_int(self.raw("MYSTICPARADOX_GAMESERVER_RATE_LIMIT_PER_MINUTE"), 6000)
    }

    /// `MYSTICPARADOX_HOST_RATE_LIMIT_PER_MINUTE` (`security/rateLimit.ts`), default 6000.
    pub fn host_rate_limit_per_minute(&self) -> u32 {
        parse_positive_int(self.raw("MYSTICPARADOX_HOST_RATE_LIMIT_PER_MINUTE"), 6000)
    }

    /// `MYSTICPARADOX_ADMIN_API` (`config/adminApi.ts`): `on`, `off`, or blank to follow the manager bridge.
    pub fn admin_api(&self) -> Result<bool, SettingsError> {
        let raw = self
            .raw("MYSTICPARADOX_ADMIN_API")
            .unwrap_or_default()
            .trim()
            .to_lowercase();
        match raw.as_str() {
            "on" => Ok(true),
            "off" => Ok(false),
            "" => {
                // ReadManagerOperatorSwitch (security/managerOperator.ts)
                let manager = self
                    .raw("MYSTICPARADOX_MANAGER_ADMIN")
                    .unwrap_or_default()
                    .trim()
                    .to_lowercase();
                match manager.as_str() {
                    "" | "off" => Ok(false),
                    "on" => Ok(true),
                    _ => Err(SettingsError::Value {
                        name: "MYSTICPARADOX_MANAGER_ADMIN".into(),
                        message: "must be on or off (blank means off)".into(),
                    }),
                }
            }
            _ => Err(SettingsError::Value {
                name: "MYSTICPARADOX_ADMIN_API".into(),
                message: "must be on or off (blank follows MYSTICPARADOX_MANAGER_ADMIN)".into(),
            }),
        }
    }
}

impl rifts_seam::Settings for Settings {
    fn get(&self, name: &str) -> Option<&str> {
        self.raw(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_env_file_wins_over_the_switches_section() {
        let section = BTreeMap::from([("qos_port".to_owned(), json!(80)), ("reforge".to_owned(), json!("off"))]);
        let switches = Settings::switches_from_config(Some(&section));
        assert_eq!(switches.get("QOS_PORT").map(String::as_str), Some("80"));
        let env = BTreeMap::from([("MYSTICPARADOX_REFORGE".to_owned(), "log".to_owned())]);
        let settings = Settings::new(env, switches);
        assert_eq!(settings.get("MYSTICPARADOX_REFORGE"), Some("log"));
        assert_eq!(settings.get("QOS_PORT"), Some("80"));
        assert_eq!(settings.get("MYSTICPARADOX_MASTERY_PERSISTENCE"), None);
    }

    #[test]
    fn switches_read_like_node() {
        assert!(Settings::default().on_unless_off("X"));
        assert!(!Settings::from_pairs([("X", "OFF")]).on_unless_off("X"));
        assert!(!Settings::default().admin_api().unwrap());
        assert!(
            Settings::from_pairs([("MYSTICPARADOX_ADMIN_API", " On ")])
                .admin_api()
                .unwrap()
        );
        assert!(
            Settings::from_pairs([("MYSTICPARADOX_MANAGER_ADMIN", "on")])
                .admin_api()
                .unwrap()
        );
        assert!(
            Settings::from_pairs([("MYSTICPARADOX_ADMIN_API", "yes")])
                .admin_api()
                .is_err()
        );
    }
}
