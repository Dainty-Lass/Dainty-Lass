// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The Director's settings (spec §11), read once per module start.

use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use dainty_module_api::PortRange;
use rustic_isles::Settings;
use serde_json::Value;

use crate::hub::switch_on;
use crate::{DirectorConfig, memory};

const LAUNCH_ID: &str = "RUSTIC_ISLES_GAMESERVER_LAUNCH_ID";
const TRIALS_WEEK: &str = "RUSTIC_ISLES_TRIALS_WEEK";
/// How long a stopped server's process tree may take to exit before it is killed.
pub const STOP_GRACE: Duration = Duration::from_secs(10);

fn js_number(settings: &Settings, name: &str) -> f64 {
    match settings.raw(name) {
        Some(text) => dainty_service_kit::js::number(Some(&Value::String(text.to_owned()))),
        None => f64::NAN,
    }
}

fn js_number_or(settings: &Settings, name: &str, unset: f64) -> f64 {
    if settings.raw(name).is_none() {
        unset
    } else {
        js_number(settings, name)
    }
}

fn trimmed_lower(settings: &Settings, name: &str) -> Option<String> {
    settings.raw(name).map(|v| v.trim().to_ascii_lowercase())
}

/// `PORT_RANGE_BEGIN`-`PORT_RANGE_END`, both required (core §6.4 forces them to the assigned game range).
pub fn game_range(settings: &Settings) -> Result<PortRange, String> {
    let port = |name: &str| {
        settings
            .get(name)
            .and_then(|v| v.trim().parse::<u16>().ok())
            .ok_or_else(|| format!("the Director has no {name}"))
    };
    Ok(PortRange {
        first: port("PORT_RANGE_BEGIN")?,
        last: port("PORT_RANGE_END")?,
    })
}

/// The Director's settings for one module start. `Debug` leaves out the key and the environment.
pub struct Config {
    /// The game server ports the module was given.
    pub port_range: PortRange,
    /// The game server executable (`GAMESERVER_BINARY_PATH`); none means no server can start.
    pub binary_path: Option<String>,
    api_key: Option<String>,
    /// The address players are sent to (`MY_IP`).
    pub my_ip: Option<String>,
    /// Wait between two server starts (`SECONDS_TO_WAIT_BETWEEN_GAMESERVER_STARTUP`), at least 1 ms.
    pub stagger: Duration,
    /// How long a server has to report ready (`GAMESERVER_READY_TIMEOUT_MS`, default 30 s).
    pub ready_timeout: Duration,
    /// `GAMESERVER_READY_TIMEOUT_MS` as a number, for the error text
    pub ready_timeout_setting: f64,
    /// Most hunt servers at once (`MAX_CONCURRENT_HUNT_SERVERS`, default 3).
    pub max_hunts: f64,
    /// Players a reused public hunt may hold (`PUBLIC_HUNT_MAX_PLAYERS`, default 4).
    pub public_max_players: f64,
    /// Join players to a running public hunt (`ENABLE_PUBLIC_HUNT_REUSE=1`).
    pub public_reuse: bool,
    /// Restart a hub as soon as it exits or is found gone (`HUB_PROMPT_RESTART`, default on).
    pub hub_prompt_restart: bool,
    /// Keep one hunt server booted and held before its map load (`GAMESERVER_WARM_SPARE=1`; prototype, default off).
    pub warm_spare: bool,
    /// Keep the Ramsgate hub running (`RAMSGATE_HUB`, default on).
    pub ramsgate_hub: bool,
    /// Keep the Training Grounds running (`TRAINING_HUB`); off by default under `TRAINING_ON_DEMAND`.
    pub training_hub: bool,
    /// 0 is off
    pub training_idle_minutes: u64,
    /// Where the runtime writes its per-port occupancy logs (`GAMESERVER_RUNTIME_LOG_DIR`).
    pub runtime_log_dir: Option<PathBuf>,
    /// The host's published runtime (`GAMESERVER_RUNTIME_DIR`), injected into every server; none loads nothing.
    pub runtime_dir: Option<PathBuf>,
    /// Where each server's output is written (`GAMESERVER_LOG_DIR`, default `../debug/gameservers`).
    pub log_dir: PathBuf,
    /// Hide the server's window on Windows (`GAMESERVER_WINDOWS_HIDE`).
    pub windows_hide: bool,
    /// The switches every server is started with, `-LogCmds` last.
    pub launch_args: Vec<String>,
    /// Minutes each Trials week lasts (`TRIALS_ROTATION_MINUTES`, default a week).
    pub trials_rotation_minutes: u64,
    /// The memory guard's settings.
    pub memory: memory::Options,
    /// Warned when the service is built
    pub warnings: Vec<String>,
    base_env: Vec<(String, String)>,
    /// Wait before a stopped server's tree is killed, [`STOP_GRACE`] outside tests.
    pub stop_grace: Duration,
    /// The hubs' restart backoff in ms; none uses the supervisor's default.
    pub hub_backoff_ms: Option<Vec<u64>>,
}

// Never the key or the environment (§15)
impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("port_range", &self.port_range)
            .field("binary_path", &self.binary_path)
            .field("my_ip", &self.my_ip)
            .finish_non_exhaustive()
    }
}

fn idle_minutes(settings: &Settings, warnings: &mut Vec<String>) -> u64 {
    let raw = settings.raw("TRAINING_IDLE_STOP_MINUTES").map(str::trim).unwrap_or("");
    if raw.is_empty() {
        return 0;
    }
    let value = raw
        .bytes()
        .all(|b| b.is_ascii_digit())
        .then(|| raw.parse::<u64>().ok())
        .flatten()
        .filter(|m| *m <= 1440);
    value.unwrap_or_else(|| {
        warnings.push(format!(
            "[TrainingOnDemand] TRAINING_IDLE_STOP_MINUTES='{raw}' is not a whole number from 0 to 1440; the \
             Training Grounds keep running"
        ));
        0
    })
}

impl Config {
    /// Reads the settings; an old `MYSTPAX_` key is an error. `process_env` wins over the settings in the child's
    /// environment.
    pub fn from_settings(
        settings: &Settings,
        director: &DirectorConfig,
        port_range: PortRange,
        process_env: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Config, String> {
        let vars = settings.vars();
        let stale: Vec<&str> = vars
            .keys()
            .map(String::as_str)
            .filter(|k| k.starts_with("MYSTPAX_"))
            .collect();
        if !stale.is_empty() {
            return Err(format!(
                "Rename these environment keys to the RUSTIC_ISLES_ prefix: {}",
                stale.join(", ")
            ));
        }
        let mut warnings = Vec::new();
        let stagger_s = js_number(settings, "SECONDS_TO_WAIT_BETWEEN_GAMESERVER_STARTUP");
        // setTimeout of a non-positive or NaN delay is 1 ms
        let stagger = if stagger_s.is_finite() && stagger_s >= 1.0 {
            Duration::from_secs_f64(stagger_s)
        } else {
            Duration::from_millis(1)
        };
        let ready_timeout_setting = js_number_or(settings, "GAMESERVER_READY_TIMEOUT_MS", 30_000.0);
        let ready_timeout = if (1.0..=2_147_483_647.0).contains(&ready_timeout_setting) {
            Duration::from_millis(ready_timeout_setting as u64)
        } else {
            Duration::from_millis(1)
        };
        let training_on_demand = trimmed_lower(settings, "TRAINING_ON_DEMAND")
            .is_some_and(|v| matches!(v.as_str(), "1" | "true" | "yes" | "on"));
        let training_hub = switch_on(settings.raw("TRAINING_HUB"), !training_on_demand);
        let mut training_idle_minutes = idle_minutes(settings, &mut warnings);
        let runtime_log_dir = settings
            .raw("GAMESERVER_RUNTIME_LOG_DIR")
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from);
        if training_idle_minutes > 0 && runtime_log_dir.is_none() {
            warnings.push(
                "[TrainingOnDemand] TRAINING_IDLE_STOP_MINUTES is set but GAMESERVER_RUNTIME_LOG_DIR is not: the \
                 Training Grounds keep running"
                    .to_owned(),
            );
            training_idle_minutes = 0;
        }
        let cwd = std::env::current_dir().unwrap_or_default();
        let log_dir = match settings.raw("GAMESERVER_LOG_DIR") {
            Some(dir) => cwd.join(dir),
            None => cwd.join("..").join("debug").join("gameservers"),
        };
        if director.args.log_cmds_invalid(settings) {
            warnings.push(format!(
                "GAMESERVER_LOG_CMDS must be \"Category Verbosity\" pairs separated by commas; using the default ({})",
                director.args.log_cmds
            ));
        }
        let (trials_rotation_minutes, trials_warning) =
            crate::hunt::parse_trials_rotation_minutes(settings.raw("TRIALS_ROTATION_MINUTES"));
        warnings.extend(trials_warning);
        let (memory, problems) = memory::options_from_settings(settings);
        warnings.extend(problems.into_iter().map(|p| format!("[MemoryGuard] {p}")));
        // The process wins over the settings, as Node's --env-file; Windows names are case-insensitive
        let key = |name: &str| {
            if cfg!(windows) {
                name.to_uppercase()
            } else {
                name.to_owned()
            }
        };
        let mut env: Vec<(String, String)> = vars.into_iter().collect();
        for (name, value) in process_env {
            env.retain(|(k, _)| key(k) != key(&name));
            env.push((name, value));
        }
        env.retain(|(k, _)| key(k) != key(LAUNCH_ID) && key(k) != key(TRIALS_WEEK));
        Ok(Config {
            port_range,
            binary_path: settings.get("GAMESERVER_BINARY_PATH").map(str::to_owned),
            api_key: settings.raw("METAGAME_API_KEY").map(str::to_owned),
            my_ip: settings.raw("MY_IP").map(str::to_owned),
            stagger,
            ready_timeout,
            ready_timeout_setting,
            max_hunts: js_number_or(settings, "MAX_CONCURRENT_HUNT_SERVERS", 3.0),
            public_max_players: js_number_or(settings, "PUBLIC_HUNT_MAX_PLAYERS", 4.0),
            public_reuse: settings.raw("ENABLE_PUBLIC_HUNT_REUSE") == Some("1"),
            hub_prompt_restart: !trimmed_lower(settings, "HUB_PROMPT_RESTART")
                .is_some_and(|v| matches!(v.as_str(), "0" | "false" | "no" | "off")),
            warm_spare: settings.raw("GAMESERVER_WARM_SPARE") == Some("1"),
            ramsgate_hub: switch_on(settings.raw("RAMSGATE_HUB"), true),
            training_hub,
            training_idle_minutes,
            runtime_log_dir,
            runtime_dir: settings
                .raw("GAMESERVER_RUNTIME_DIR")
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
            log_dir,
            windows_hide: director.args.hide_window(settings),
            launch_args: director.args.launch_args(settings),
            trials_rotation_minutes,
            memory,
            warnings,
            base_env: env,
            stop_grace: STOP_GRACE,
            hub_backoff_ms: None,
        })
    }

    /// `GAMESERVER_BINARY_PATH` and `METAGAME_API_KEY` set (an empty key counts, §16 item 10).
    pub fn can_launch(&self) -> bool {
        self.binary_path.is_some() && self.api_key.is_some()
    }

    pub(crate) fn api_key(&self) -> Option<&str> {
        self.api_key.as_deref()
    }

    /// The child's whole environment for one launch.
    pub(crate) fn child_env(&self, launch_id: &str, matchmaker_hunt_id: Option<&str>) -> Vec<(String, String)> {
        let mut env = self.base_env.clone();
        env.push((LAUNCH_ID.to_owned(), launch_id.to_owned()));
        // An arena hunt keeps the week it was launched for; the hubs follow the runtime's own rotation
        if let Some(week) = matchmaker_hunt_id.and_then(crate::hunt::arena_week) {
            env.push((TRIALS_WEEK.to_owned(), week.to_owned()));
        }
        env
    }

    /// [`Config::my_ip`] for formatted addresses and log text, `undefined` when unset.
    pub fn my_ip_text(&self) -> &str {
        self.my_ip.as_deref().unwrap_or("undefined")
    }
}
