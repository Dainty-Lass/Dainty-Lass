//! The environment as the Node deploy server reads it: most settings are read again on every use
//! (`process.env.X` inside the function), so a host can change a drain file or a limit without a
//! restart and tests can flip them. A few are read once at start, as Node read them when the module
//! loaded; those live in [`Fixed`].
//!
//! Every DR variable is set as `DEPLOY_<name>` and kept here under DR's name, so lookups and the game
//! process (which reads `DR_SERVER_READY_FILE`, `DR_GAME_EXE`...) see DR's names. Other variables are
//! the system's own, passed to the game process and never read as settings.

use std::collections::HashMap;
use std::sync::{Arc, PoisonError, RwLock};

use service_kit::env::Env;

/// What the deploy server's variables start with in its env file.
pub const PREFIX: &str = "DEPLOY_";

#[derive(Clone, Default)]
pub struct Settings {
    vars: Arc<RwLock<HashMap<String, String>>>,
    system: Arc<Vec<(String, String)>>,
}

impl Settings {
    pub fn from_env(env: &Env) -> Self {
        let (mut vars, mut system) = (HashMap::new(), Vec::new());
        for (name, value) in env.vars() {
            match name.strip_prefix(PREFIX) {
                Some(name) if !name.is_empty() => {
                    vars.insert(name.to_owned(), value.to_owned());
                }
                _ => system.push((name.to_owned(), value.to_owned())),
            }
        }
        Self {
            vars: Arc::new(RwLock::new(vars)),
            system: Arc::new(system),
        }
    }

    /// Settings by DR's names, as a test sets them.
    pub fn from_pairs<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let vars = pairs.into_iter().map(|(k, v)| (k.to_owned(), v.to_owned())).collect();
        Self {
            vars: Arc::new(RwLock::new(vars)),
            system: Arc::default(),
        }
    }

    /// `process.env[name]`: the value as set, also when empty.
    pub fn get(&self, name: &str) -> Option<String> {
        self.vars
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(name)
            .cloned()
    }

    /// `process.env[name] || undefined`: unset and empty alike.
    pub fn non_empty(&self, name: &str) -> Option<String> {
        self.get(name).filter(|v| !v.is_empty())
    }

    pub fn is(&self, name: &str, value: &str) -> bool {
        self.get(name).as_deref() == Some(value)
    }

    /// Sets (or with `None` removes) a variable, as `process.env[name] = ...` did.
    pub fn set(&self, name: &str, value: Option<&str>) {
        let mut vars = self.vars.write().unwrap_or_else(PoisonError::into_inner);
        match value {
            Some(value) => vars.insert(name.to_owned(), value.to_owned()),
            None => vars.remove(name),
        };
    }

    /// The game process's environment: the system's variables, then every setting by DR's name.
    pub fn snapshot(&self) -> Vec<(String, String)> {
        let vars = self.vars.read().unwrap_or_else(PoisonError::into_inner);
        let settings = vars.iter().map(|(k, v)| (k.clone(), v.clone()));
        self.system.iter().cloned().chain(settings).collect()
    }

    /// `Number(value)` for a set value, `None` when unset.
    pub fn number(&self, name: &str) -> Option<f64> {
        self.get(name)
            .map(|v| service_kit::js::number(Some(&serde_json::Value::String(v))))
    }

    /// The deploy server's `setting(name, fallback)`: a whole number of at least 1, digits only.
    pub fn whole(&self, name: &str, fallback: u64) -> Result<u64, String> {
        let Some(value) = self.get(name) else {
            return Ok(fallback);
        };
        let digits = !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit());
        match value.parse::<u64>() {
            Ok(number) if digits && (1..=9_007_199_254_740_991).contains(&number) => Ok(number),
            _ => Err(format!("Invalid {PREFIX}{name}")),
        }
    }
}

/// What Node read once, when `gameservers.ts` loaded.
#[derive(Debug, Clone)]
pub struct Fixed {
    pub port_range_begin: u16,
    pub port_range_end: u16,
    pub binary: String,
    pub metagame_api_key: String,
    pub my_ip: String,
    /// `SECONDS_TO_WAIT_BETWEEN_GAMESERVER_STARTUP`, in milliseconds
    pub launch_spacing_ms: u64,
}

impl Fixed {
    pub fn from_settings(settings: &Settings) -> Result<Self, String> {
        let port = |name: &str| -> Result<u16, String> {
            settings
                .number(name)
                .filter(|n| n.fract() == 0.0 && (1.0..=65535.0).contains(n))
                .map(|n| n as u16)
                .ok_or_else(|| format!("{PREFIX}{name} must be a port number"))
        };
        let spacing = settings
            .number("SECONDS_TO_WAIT_BETWEEN_GAMESERVER_STARTUP")
            .unwrap_or(f64::NAN);
        Ok(Self {
            port_range_begin: port("PORT_RANGE_BEGIN")?,
            port_range_end: port("PORT_RANGE_END")?,
            binary: settings.get("GAMESERVER_BINARY_PATH").unwrap_or_default(),
            metagame_api_key: settings.get("METAGAME_API_KEY").unwrap_or_default(),
            my_ip: settings.get("MY_IP").unwrap_or_default(),
            // Node: a NaN wait never delays (Date.now() + NaN compares false)
            launch_spacing_ms: if spacing.is_finite() && spacing > 0.0 {
                (spacing * 1000.0) as u64
            } else {
                0
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deploy_variables_are_read_by_their_dr_names() {
        let env = Env::from_pairs([
            ("DEPLOY_PORT", "61001"),
            ("DEPLOY_DR_SHOW_CONSOLE", "1"),
            ("PATH", "/usr/bin"),
            ("PORT", "9"),
            ("DEPLOY_", "x"),
        ]);
        let settings = Settings::from_env(&env);
        assert_eq!(settings.get("PORT").as_deref(), Some("61001"));
        assert_eq!(settings.get("DR_SHOW_CONSOLE").as_deref(), Some("1"));
        assert_eq!(settings.get("PATH"), None, "the system's variables are not settings");
        let mut child = std::collections::BTreeMap::new();
        child.extend(settings.snapshot());
        assert_eq!(child["PORT"], "61001", "a setting wins over the system's variable");
        assert_eq!(child["PATH"], "/usr/bin");
        assert_eq!(child["DR_SHOW_CONSOLE"], "1");
        assert!(!child.contains_key("DEPLOY_PORT"));
        assert_eq!(settings.whole("MAX_HUNTS", 4), Ok(4));
        settings.set("MAX_HUNTS", Some("x"));
        assert_eq!(settings.whole("MAX_HUNTS", 4), Err("Invalid DEPLOY_MAX_HUNTS".into()));
    }
}
