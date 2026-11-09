// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Rustic Isles' Director (RD27, written clean-room from `docs/port/spec-rustic-isles-director.md`): the service on
//! the module's `director` port that starts, watches and stops the game servers (Ramsgate, the Training Grounds, one
//! process per hunt), leases the hunt UDP ports, guards memory, resolves hunts from the hunt tables and imports
//! those tables from a CatalogExporter capture. It plugs into `rustic-isles` as a `PortService` and as the module's
//! dedicated-servers slot.

#![cfg_attr(
    test,
    allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")
)]

mod api;
pub mod cmdline;
mod config;
mod gameservers;
pub mod hub;
pub mod hunt;
pub mod hunt_tables;
pub mod launcher;
mod matchmaker;
pub mod memory;
pub mod occupancy;
pub mod os;
pub mod port_pool;
pub mod process;

use std::sync::{Arc, Mutex, PoisonError};

use dainty_module_api::{BoxFuture, ModuleError};
use rustic_isles::{DedicatedServers, Launch, LaunchRequest, LiveHub, LiveHubStatus, PortService, Settings, Shared};
use tokio::task::JoinHandle;

pub use api::{register, router};
pub use config::{Config, STOP_GRACE, game_range};
pub use gameservers::{
    Details, Director, GameServer, RAMSGATE_MAP, RosterEntry, ServerKind, ServerStatus, StartError, StartErrorKind,
    Supervisor, TRAINING_DOJO_HUNT_ID, TRAINING_DOJO_MAP, label_for, roster_argument,
};
pub use launcher::{Command, Exit, Launcher, OrNull, OsLauncher, Started, TreeControl};
pub use matchmaker::TravelArgs;
pub use memory::MemoryAdmission;

/// A version's game server switches and default `-LogCmds` (§12.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameserverArgs {
    /// The switches every server gets.
    pub standard: &'static [&'static str],
    /// The default `-LogCmds` value, used unless `GAMESERVER_LOG_CMDS` is valid.
    pub log_cmds: &'static str,
}

fn log_cmds_ok(value: &str) -> bool {
    (1..=512).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b',' | b' '))
}

impl GameserverArgs {
    /// A version's switches and default `-LogCmds`.
    pub const fn new(standard: &'static [&'static str], log_cmds: &'static str) -> Self {
        Self { standard, log_cmds }
    }

    /// `GAMESERVER_LOG_CMDS` trimmed when it is `[A-Za-z0-9_, ]{1,512}`; a host typo or a quote cannot add a switch.
    fn log_cmds_value<'a>(&self, settings: &'a Settings) -> Option<&'a str> {
        settings
            .raw("GAMESERVER_LOG_CMDS")
            .map(str::trim)
            .filter(|v| log_cmds_ok(v))
    }

    /// The switches, `-LogCmds="..."` last.
    pub fn launch_args(&self, settings: &Settings) -> Vec<String> {
        let log_cmds = self.log_cmds_value(settings).unwrap_or(self.log_cmds);
        self.standard
            .iter()
            .map(|s| (*s).to_owned())
            .chain(std::iter::once(format!("-LogCmds=\"{log_cmds}\"")))
            .collect()
    }

    /// Set (and not blank) but not a list of pairs.
    pub fn log_cmds_invalid(&self, settings: &Settings) -> bool {
        settings
            .raw("GAMESERVER_LOG_CMDS")
            .is_some_and(|v| !v.trim().is_empty() && self.log_cmds_value(settings).is_none())
    }

    /// Hidden unless `GAMESERVER_WINDOWS_HIDE` is exactly `0`.
    pub fn hide_window(&self, settings: &Settings) -> bool {
        settings.raw("GAMESERVER_WINDOWS_HIDE") != Some("0")
    }
}

/// A version's Director values (unit 12's version crate carries 1.14.7's).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectorConfig {
    /// For the not-yet-ported runtime updater
    pub target_changelist: u32,
    /// The game server switches.
    pub args: GameserverArgs,
}

impl DirectorConfig {
    /// A version's values.
    pub const fn new(target_changelist: u32, args: GameserverArgs) -> Self {
        Self {
            target_changelist,
            args,
        }
    }

    /// [`GameserverArgs::launch_args`].
    pub fn launch_args(&self, settings: &Settings) -> Vec<String> {
        self.args.launch_args(settings)
    }

    /// [`GameserverArgs::log_cmds_invalid`].
    pub fn log_cmds_invalid(&self, settings: &Settings) -> bool {
        self.args.log_cmds_invalid(settings)
    }

    /// [`GameserverArgs::hide_window`].
    pub fn hide_window(&self, settings: &Settings) -> bool {
        self.args.hide_window(settings)
    }
}

/// The controller's environment, skipping a name or value that is not Unicode (§16 item 13).
fn process_environment() -> Vec<(String, String)> {
    std::env::vars_os()
        .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
        .collect()
}

/// The in-memory Director in the module's dedicated-servers slot.
pub struct InProcessDirector(pub Arc<Director>);

fn live(details: Option<Details>) -> Option<LiveHub> {
    details.map(|(host, port)| LiveHub {
        ready: true,
        host: host.unwrap_or_default(),
        port: i64::from(port),
    })
}

impl DedicatedServers for InProcessDirector {
    fn configured(&self) -> bool {
        self.0.config().can_launch()
    }

    fn launch(&self, request: LaunchRequest) -> BoxFuture<'_, Result<Launch, String>> {
        Box::pin(async move {
            let body = request.body();
            match self.0.handle_matchmaking(Some(&body)).await {
                Ok((host, port)) => Ok(Launch::Online {
                    host: host.unwrap_or_default(),
                    port: i64::from(port),
                }),
                Err(error) => {
                    tracing::error!("handle-matchmaking-for-player failed: {error}");
                    Ok(Launch::Refused { reason: None })
                }
            }
        })
    }

    fn hub_status(&self) -> BoxFuture<'_, Option<LiveHubStatus>> {
        Box::pin(async move {
            Some(LiveHubStatus {
                ramsgate: live(self.0.live_hub_details(ServerKind::Ramsgate)),
                training: live(self.0.live_hub_details(ServerKind::Training)),
            })
        })
    }

    fn servers(&self) -> BoxFuture<'_, Option<Vec<dainty_module_api::game::GameServer>>> {
        Box::pin(async move { Some(self.0.game_servers()) })
    }
}

/// The Director as the module's `director` port service (§12.3).
pub struct DirectorService {
    config: DirectorConfig,
    launcher: Arc<dyn Launcher>,
    director: Mutex<Option<Arc<Director>>>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

impl DirectorService {
    /// The service, starting real processes.
    pub fn new(config: DirectorConfig) -> Self {
        Self::with_launcher(config, Arc::new(OsLauncher))
    }

    /// The service with another launcher (tests).
    pub fn with_launcher(config: DirectorConfig, launcher: Arc<dyn Launcher>) -> Self {
        Self {
            config,
            launcher,
            director: Mutex::new(None),
            tasks: Mutex::new(Vec::new()),
        }
    }

    /// The version's values.
    pub fn config(&self) -> &DirectorConfig {
        &self.config
    }

    /// The running Director; none before start-up.
    pub fn director(&self) -> Option<Arc<Director>> {
        self.director.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// The Director over the module's settings and hunt tables; its warnings are logged here.
    pub fn build(&self, shared: &Shared) -> Result<Arc<Director>, ModuleError> {
        let settings = shared.settings();
        let range = game_range(settings).map_err(ModuleError::failed)?;
        let config =
            Config::from_settings(settings, &self.config, range, process_environment()).map_err(ModuleError::failed)?;
        for warning in &config.warnings {
            tracing::warn!("{warning}");
        }
        let tables = match &shared.game_data().director {
            None => {
                tracing::error!(
                    "No Director game data folder is configured ([game_data] director_dir), so no hunt can start"
                );
                hunt::Tables::default()
            }
            Some(dir) => hunt::load_tables(dir).unwrap_or_else(|error| {
                tracing::error!("Hunt tables not loaded, no hunt can start: {error}");
                hunt::Tables::default()
            }),
        };
        let memory = MemoryAdmission::new(config.memory);
        Ok(Director::new(config, tables, memory, Arc::clone(&self.launcher)))
    }

    fn tasks(&self) -> std::sync::MutexGuard<'_, Vec<JoinHandle<()>>> {
        self.tasks.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl PortService for DirectorService {
    fn port(&self) -> &'static str {
        "director"
    }

    fn app(&self, shared: Arc<Shared>) -> Result<axum::Router, ModuleError> {
        let director = self.build(&shared)?;
        let url_unset = shared
            .settings()
            .raw("DEPLOYSERVER_URL")
            .is_none_or(|url| url.trim().is_empty());
        if director.config().can_launch() || url_unset {
            shared.set_dedicated_servers(Arc::new(InProcessDirector(Arc::clone(&director))));
        }
        let router = router(shared, &director).map_err(|e| ModuleError::failed(e.to_string()))?;
        *self.director.lock().unwrap_or_else(PoisonError::into_inner) = Some(director);
        Ok(router)
    }

    fn started(&self) {
        let Some(director) = self.director() else {
            return;
        };
        if !director.config().can_launch() {
            tracing::warn!(
                "GAMESERVER_BINARY_PATH and METAGAME_API_KEY are not set in the env file: the Director starts no game \
                 server"
            );
            return;
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let boot = Arc::clone(&director);
        let boot = handle.spawn(async move { boot.startup().await });
        let watchdog = Arc::clone(&director);
        let watchdog = handle.spawn(async move {
            let period = Director::watchdog_period();
            let mut ticks = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
            loop {
                ticks.tick().await;
                watchdog.run_watchdog().await;
            }
        });
        self.tasks().extend([boot, watchdog]);
        tracing::info!("Rustic Isles DeployServer started");
        tracing::info!("Clear Skies, Slayer.");
    }

    fn stopping(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let tasks: Vec<JoinHandle<()>> = self.tasks().drain(..).collect();
            for task in tasks {
                task.abort();
            }
            let director = self.director.lock().unwrap_or_else(PoisonError::into_inner).take();
            if let Some(director) = director {
                director.shutdown().await;
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ARGS: GameserverArgs = GameserverArgs::new(
        &[
            "-EpicPortal",
            "-server",
            "-nullrhi",
            "-warp",
            "-nosound",
            "-RepDriverEnable",
        ],
        "LogPhoenixCharacter VeryVerbose, LogOnline Verbose, LogLoad Off",
    );

    #[test]
    fn launch_settings() {
        let none = Settings::default();
        assert_eq!(
            ARGS.launch_args(&none).last().unwrap(),
            "-LogCmds=\"LogPhoenixCharacter VeryVerbose, LogOnline Verbose, LogLoad Off\""
        );
        assert_eq!(ARGS.launch_args(&none).len(), 7);
        let set = Settings::from_pairs([("GAMESERVER_LOG_CMDS", " LogOnline Warning ")]);
        assert_eq!(ARGS.launch_args(&set).last().unwrap(), "-LogCmds=\"LogOnline Warning\"");
        assert!(!ARGS.log_cmds_invalid(&set));
        let evil = Settings::from_pairs([("GAMESERVER_LOG_CMDS", "x\" -evil")]);
        assert_eq!(ARGS.launch_args(&evil), ARGS.launch_args(&none));
        assert!(ARGS.log_cmds_invalid(&evil));
        assert!(ARGS.hide_window(&none));
        assert!(!ARGS.hide_window(&Settings::from_pairs([("GAMESERVER_WINDOWS_HIDE", "0")])));
        assert!(ARGS.hide_window(&Settings::from_pairs([("GAMESERVER_WINDOWS_HIDE", " 0")])));
    }
}
