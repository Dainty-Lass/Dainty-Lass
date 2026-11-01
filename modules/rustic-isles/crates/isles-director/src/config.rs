//! The Director's settings, read as the Node Director reads `process.env` (`src/server.ts`,
//! `src/controllers/gameservers.ts`): numbers with JavaScript's `Number()`, so an empty value is 0 and
//! text is NaN, which the comparisons treat as Node does. Two things differ, both because the Director
//! is a module of the controller now: `PORT`, `BIND_HOST` and `PORT_RANGE_*` are the controller's
//! (the manifest's `director` and `game` ports), so the env file's are ignored; and the game servers
//! inherit the controller's environment with the env file over it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use isles_core::Settings;
use module_api::PortRange;
use serde_json::Value;
use service_kit::js;

use crate::DirectorConfig;

/// 1, true, yes or on, any case, trimmed.
pub fn enabled(value: Option<&str>) -> bool {
    value
        .map(|v| js::trim(v).to_ascii_lowercase())
        .is_some_and(|v| matches!(v.as_str(), "1" | "true" | "yes" | "on"))
}

fn off(value: &str) -> bool {
    matches!(
        js::trim(value).to_ascii_lowercase().as_str(),
        "0" | "false" | "no" | "off"
    )
}

/// `Number(process.env[name] ?? fallback)`.
fn number(settings: &Settings, name: &str, fallback: &str) -> f64 {
    js::number(Some(&Value::String(settings.raw(name).unwrap_or(fallback).to_owned())))
}

#[derive(Clone)]
pub struct Config {
    pub port_range_begin: u16,
    pub port_range_end: u16,
    pub gameserver_binary: Option<String>,
    /// The switches after the positional arguments, `-LogCmds=...` last (`DirectorConfig::launch_args`)
    pub switches: Vec<String>,
    /// `GAMESERVER_LOG_CMDS` is set but not well formed; a warning is logged at start
    pub log_cmds_invalid: bool,
    pub metagame_api_key: Option<String>,
    pub my_ip: Option<String>,
    pub seconds_between_startups: f64,
    pub ready_timeout_ms: f64,
    pub hub_prompt_restart: bool,
    pub training_hub_at_boot: bool,
    pub ramsgate_hub_enabled: bool,
    pub training_idle_stop_minutes: Option<String>,
    pub runtime_log_dir: String,
    pub max_concurrent_hunt_servers: f64,
    pub enable_public_hunt_reuse: bool,
    pub public_hunt_max_players: f64,
    pub gameserver_log_dir: PathBuf,
    pub windows_hide: bool,
    pub trials_rotation_minutes: Option<String>,
    pub memory: crate::memory::Options,
    pub memory_problems: Vec<String>,
    /// What every game server inherits: the controller's environment and the env file
    pub child_env: Vec<(String, String)>,
    /// How long a stopped game server gets before its process tree is killed
    pub stop_grace: Duration,
    /// Replaces the hub supervisors' restart backoff (the tests)
    pub hub_backoff_ms: Option<Vec<u64>>,
}

// The API key and the environment hold secrets: nothing of them in a log line.
impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("ports", &(self.port_range_begin, self.port_range_end))
            .field("gameserver_binary", &self.gameserver_binary)
            .field("my_ip", &self.my_ip)
            .finish_non_exhaustive()
    }
}

fn resolve(cwd: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}

impl Config {
    /// The Director's configuration for one module: `settings` are its env file, `ports` the game range the
    /// controller gave it.
    pub fn from_settings(
        settings: &Settings,
        director: &DirectorConfig,
        ports: PortRange,
        process_env: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Self, String> {
        let cwd = std::env::current_dir().map_err(|e| format!("the working folder: {e}"))?;
        let training_on_demand = enabled(settings.raw("TRAINING_ON_DEMAND"));
        let vars = settings.vars();
        // [2026-10-08] MYSTPAX_ became MYSTICPARADOX_; the game servers inherit this environment, so a stale
        // key would silently switch a runtime feature off.
        let legacy: Vec<&str> = vars
            .keys()
            .map(String::as_str)
            .filter(|k| k.starts_with("MYSTPAX_"))
            .collect();
        if !legacy.is_empty() {
            return Err(format!(
                "Rename these environment keys to the MYSTICPARADOX_ prefix: {}",
                legacy.join(", ")
            ));
        }
        // The process environment wins over the env file, as Node's --env-file does
        let mut child_env = vars.clone();
        child_env.extend(process_env);
        let (memory, memory_problems) = crate::memory::options_from_settings(settings);
        Ok(Self {
            port_range_begin: ports.first,
            port_range_end: ports.last,
            gameserver_binary: settings.get("GAMESERVER_BINARY_PATH").map(str::to_owned),
            switches: director.launch_args(settings),
            log_cmds_invalid: director.log_cmds_invalid(settings),
            metagame_api_key: settings.raw("METAGAME_API_KEY").map(str::to_owned),
            my_ip: settings.raw("MY_IP").map(str::to_owned),
            seconds_between_startups: number(settings, "SECONDS_TO_WAIT_BETWEEN_GAMESERVER_STARTUP", "undefined"),
            ready_timeout_ms: number(settings, "GAMESERVER_READY_TIMEOUT_MS", "30000"),
            hub_prompt_restart: !off(settings.raw("HUB_PROMPT_RESTART").unwrap_or("1")),
            training_hub_at_boot: crate::hub::switch_on(settings.raw("TRAINING_HUB"), !training_on_demand),
            ramsgate_hub_enabled: crate::hub::switch_on(settings.raw("RAMSGATE_HUB"), true),
            training_idle_stop_minutes: settings.raw("TRAINING_IDLE_STOP_MINUTES").map(str::to_owned),
            runtime_log_dir: js::trim(settings.raw("GAMESERVER_RUNTIME_LOG_DIR").unwrap_or("")).to_owned(),
            max_concurrent_hunt_servers: number(settings, "MAX_CONCURRENT_HUNT_SERVERS", "3"),
            enable_public_hunt_reuse: settings.raw("ENABLE_PUBLIC_HUNT_REUSE").unwrap_or("0") == "1",
            public_hunt_max_players: number(settings, "PUBLIC_HUNT_MAX_PLAYERS", "4"),
            gameserver_log_dir: settings.raw("GAMESERVER_LOG_DIR").map_or_else(
                || cwd.join("..").join("debug").join("gameservers"),
                |dir| resolve(&cwd, dir),
            ),
            windows_hide: director.hide_window(settings),
            trials_rotation_minutes: settings.raw("TRIALS_ROTATION_MINUTES").map(str::to_owned),
            memory,
            memory_problems,
            child_env: child_env.into_iter().collect(),
            stop_grace: crate::process::STOP_GRACE,
            hub_backoff_ms: None,
        })
    }

    /// Whether the Director has what it needs to start a game server at all.
    pub fn can_launch(&self) -> bool {
        self.gameserver_binary.is_some() && self.metagame_api_key.is_some()
    }
}
