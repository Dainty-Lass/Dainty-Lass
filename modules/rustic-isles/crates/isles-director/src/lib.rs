//! The Director both Rustic Isles modules share (RU6 ports it from `ParadoxDirector`, starting from `cdc0a87`), with
//! its per-version config (RU3c, seam 5): the game servers' launch arguments, the changelist the runtime updater
//! checks, and the hunt tables in the module's game data root. Its routes are the route table's `Director` rows,
//! served on the module's `director` port, which no gateway route reaches: the backend calls it on loopback.
//!
//! It starts and stops the game servers (Ramsgate, the Training Grounds, one process per hunt) on the module's
//! UDP range, each as the leader of a process tree it ends with it: a job object on Windows, a process group on
//! Linux, as the host kit does. Starting is behind [`Launcher`], so the tests run a fake game server.
//!
//! Not ported yet: the signed server-runtime update (`runtimeUpdater.ts`, `SERVER_RUNTIME_AUTO_UPDATE`, off as
//! `setup` writes it). `target_changelist` is what it will check.

pub mod api;
pub mod cmdline;
pub mod config;
pub mod gameservers;
pub mod hub;
pub mod hunt;
pub mod hunt_tables;
pub mod log;
pub mod matchmaker;
pub mod memory;
pub mod occupancy;
pub mod os;
pub mod port_pool;
pub mod process;
pub mod random;
#[cfg(windows)]
mod win_spawn;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use isles_core::PortService;
use isles_core::areas::matchmaking::deploy::{DedicatedServers, Launch, LaunchRequest, LiveHub, LiveHubStatus};
use isles_core::game_data::GameDataRoot;
use isles_core::http::Shared;
use isles_core::settings::Settings;
use module_api::{BoxFuture, ModuleError, PortRange};
use serde_json::Value;
use tokio::task::JoinHandle;

pub use api::{register, router};
pub use config::Config;
pub use gameservers::Director;
pub use os::OsLauncher;
pub use process::Launcher;

/// The persistent hubs.
pub const RAMSGATE_MAP: &str = "/Game/Maps/ramsgate/ramsgate_01_persistent";
pub const TRAINING_DOJO_MAP: &str = "/Game/Maps/islands/dojo/training_dojo_persistent";

/// `GAMESERVER_ARGS`: what comes before the positional launch arguments (`controllers/gameservers.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameserverArgs {
    pub standard: &'static [&'static str],
    /// The `-LogCmds` value, unless `GAMESERVER_LOG_CMDS` replaces it
    pub log_cmds: &'static str,
}

/// One version's Director.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DirectorConfig {
    /// `runtimeUpdater.ts`'s `TARGET_CHANGELIST`
    pub target_changelist: u32,
    pub args: GameserverArgs,
}

/// `GAMESERVER_LOG_CMDS`'s shape: "Category Verbosity" pairs separated by commas.
fn valid_log_cmds(value: &str) -> bool {
    (1..=512).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b',' | b' '))
}

impl DirectorConfig {
    fn configured_log_cmds<'a>(&self, settings: &'a Settings) -> &'a str {
        service_kit::js::trim(settings.raw("GAMESERVER_LOG_CMDS").unwrap_or_default())
    }

    /// `GAMESERVER_LOG_CMDS` is set but is not "Category Verbosity" pairs: the default is used and the start warns.
    pub fn log_cmds_invalid(&self, settings: &Settings) -> bool {
        let configured = self.configured_log_cmds(settings);
        !configured.is_empty() && !valid_log_cmds(configured)
    }

    /// The switches every game server starts with, after the positional arguments.
    pub fn launch_args(&self, settings: &Settings) -> Vec<String> {
        let mut args: Vec<String> = self.args.standard.iter().map(|a| (*a).to_owned()).collect();
        let configured = Some(self.configured_log_cmds(settings)).filter(|value| valid_log_cmds(value));
        args.push(format!("-LogCmds=\"{}\"", configured.unwrap_or(self.args.log_cmds)));
        args
    }

    /// `windowsHide`: `(GAMESERVER_WINDOWS_HIDE ?? "1") !== "0"`.
    pub fn hide_window(&self, settings: &Settings) -> bool {
        settings.raw("GAMESERVER_WINDOWS_HIDE").unwrap_or("1") != "0"
    }

    /// A hunt table (`isles_core::game_data::DIRECTOR_FILES`) in the module's root.
    pub fn hunt_table(&self, game_data: &GameDataRoot, name: &str) -> Option<PathBuf> {
        game_data.director_file(name)
    }
}

/// How often the watchdog looks for dead game servers (`setInterval(RunWatchdog, 60 * 1000)`).
const WATCHDOG_EVERY: Duration = Duration::from_secs(60);

struct Running {
    director: Arc<Director>,
    tasks: Vec<JoinHandle<()>>,
}

/// The Director as one of the module's listeners.
pub struct DirectorService {
    config: Arc<DirectorConfig>,
    launcher: Arc<dyn Launcher>,
    running: Mutex<Option<Running>>,
}

impl DirectorService {
    pub fn new(config: DirectorConfig) -> Self {
        Self::with_launcher(config, Arc::new(OsLauncher))
    }

    /// With the game servers started some other way (the tests).
    pub fn with_launcher(config: DirectorConfig, launcher: Arc<dyn Launcher>) -> Self {
        Self {
            config: Arc::new(config),
            launcher,
            running: Mutex::new(None),
        }
    }

    pub fn config(&self) -> &DirectorConfig {
        &self.config
    }

    /// The Director of a started module.
    pub fn director(&self) -> Option<Arc<Director>> {
        self.running
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|running| running.director.clone())
    }

    /// The Director over `shared`: its settings, the controller's game port range and the hunt tables.
    pub fn build(&self, shared: &Shared) -> Result<Arc<Director>, ModuleError> {
        // The controller's game range, set over the env file by the module (isles-core's `Settings::forced`)
        let port = |name: &str| {
            shared
                .settings
                .get(name)
                .and_then(|value| value.parse::<u16>().ok())
                .ok_or_else(|| ModuleError::failed(format!("the Director has no {name}")))
        };
        let ports = PortRange {
            first: port("PORT_RANGE_BEGIN")?,
            last: port("PORT_RANGE_END")?,
        };
        let config = Config::from_settings(&shared.settings, &self.config, ports, std::env::vars())
            .map_err(ModuleError::failed)?;
        for problem in &config.memory_problems {
            log::warn(&format!("[MemoryGuard] {problem}"));
        }
        if config.log_cmds_invalid {
            log::warn(&format!(
                "GAMESERVER_LOG_CMDS must be \"Category Verbosity\" pairs separated by commas; using the default ({})",
                self.config.args.log_cmds
            ));
        }
        // A missing table is fatal to the Node Director; here the hunts fail and the hubs still run
        let tables = match &shared.game_data.director {
            Some(dir) => hunt::load_tables(dir).unwrap_or_else(|error| {
                log::error(&format!("Hunt tables not loaded, no hunt can start: {error}"));
                hunt::Tables::default()
            }),
            None => {
                log::error(
                    "No Director game data folder is configured ([game_data] director_dir), so no hunt can start",
                );
                hunt::Tables::default()
            }
        };
        let memory = memory::MemoryAdmission::new(config.memory.clone());
        Ok(Director::new(config, tables, memory, self.launcher.clone()))
    }

    fn set_running(&self, running: Option<Running>) -> Option<Running> {
        std::mem::replace(
            &mut *self.running.lock().unwrap_or_else(PoisonError::into_inner),
            running,
        )
    }
}

/// An in-process `DedicatedServers` adapter backed directly by a [`Director`] instance,
/// used when the Director can launch game servers in-process or when `DEPLOYSERVER_URL` is unset.
#[derive(Clone)]
pub struct InProcessDirector(pub Arc<Director>);

fn live_hub(value: Option<&Value>) -> Option<LiveHub> {
    let obj = value?.as_object()?;
    Some(LiveHub {
        ready: obj.get("ready").and_then(Value::as_bool).unwrap_or(false),
        host: obj.get("host").and_then(Value::as_str).unwrap_or_default().to_owned(),
        port: obj.get("port").and_then(Value::as_i64).unwrap_or(0),
    })
}

impl DedicatedServers for InProcessDirector {
    fn configured(&self) -> bool {
        self.0.config.can_launch()
    }

    fn launch(&self, request: LaunchRequest) -> BoxFuture<'_, Result<Launch, String>> {
        let director = self.0.clone();
        Box::pin(async move {
            let body: Value = serde_json::from_str(&request.body()).unwrap_or(Value::Null);
            match matchmaker::handle(&director, Some(&body)).await {
                Ok(data) => {
                    let host = data.get("host").and_then(Value::as_str).unwrap_or_default().to_owned();
                    let port = data.get("port").and_then(Value::as_i64).unwrap_or(0);
                    Ok(Launch::Online { host, port })
                }
                Err(_) => Ok(Launch::Refused { reason: None }),
            }
        })
    }

    fn hub_status(&self) -> BoxFuture<'_, Option<LiveHubStatus>> {
        let data = self.0.persistent_hub_status();
        Box::pin(async move {
            Some(LiveHubStatus {
                ramsgate: live_hub(data.get("ramsgate")),
                training_dojo: live_hub(data.get("trainingDojo")),
            })
        })
    }
}

impl PortService for DirectorService {
    fn port(&self) -> &'static str {
        "director"
    }

    fn app(&self, shared: Arc<Shared>) -> Result<axum::Router, ModuleError> {
        let director = self.build(&shared)?;
        if director.config.can_launch()
            || shared
                .settings
                .raw("DEPLOYSERVER_URL")
                .is_none_or(|url| url.trim().is_empty())
        {
            shared
                .matchmaking
                .set_dedicated_servers(Arc::new(InProcessDirector(director.clone())));
        }
        let app = router(shared, &director).map_err(|e| ModuleError::failed(e.to_string()))?;
        self.set_running(Some(Running {
            director,
            tasks: Vec::new(),
        }));
        Ok(app)
    }

    /// `app.listen`'s callback: the hubs boot in the background, and the watchdog runs once a minute.
    fn started(&self) {
        let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(running) = running.as_mut() else {
            return;
        };
        let director = running.director.clone();
        if !director.config.can_launch() {
            log::warn(
                "GAMESERVER_BINARY_PATH and METAGAME_API_KEY are not set in the env file: the Director starts no game server",
            );
            return;
        }
        let booting = director.clone();
        running.tasks.push(tokio::spawn(async move {
            booting.startup().await;
            booting.set_persistent_hubs_ready(true);
        }));
        running.tasks.push(tokio::spawn(async move {
            let mut every = tokio::time::interval(WATCHDOG_EVERY);
            every.tick().await;
            loop {
                every.tick().await;
                // A pass that fails never takes matchmaking down with it.
                let pass = director.clone();
                if tokio::spawn(async move { pass.run_watchdog().await }).await.is_err() {
                    log::error("Gameserver watchdog pass failed");
                }
            }
        }));
        log::info("Mystic Paradox DeployServer started");
        log::info("Clear Skies, Slayer.");
    }

    fn stopping(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            let Some(running) = self.set_running(None) else {
                return;
            };
            for task in &running.tasks {
                task.abort();
            }
            running.director.shutdown().await;
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const V1147: DirectorConfig = DirectorConfig {
        target_changelist: 647472,
        args: GameserverArgs {
            standard: &[
                "-EpicPortal",
                "-server",
                "-nullrhi",
                "-warp",
                "-nosound",
                "-RepDriverEnable",
            ],
            log_cmds: "LogPhoenixCharacter VeryVerbose, LogOnline Verbose, LogLoad Off",
        },
    };

    #[test]
    fn launch_args_take_the_hosts_log_cmds_when_well_formed() {
        let args = V1147.launch_args(&Settings::default());
        assert_eq!(
            args.last().map(String::as_str),
            Some("-LogCmds=\"LogPhoenixCharacter VeryVerbose, LogOnline Verbose, LogLoad Off\"")
        );
        let quiet = Settings::from_pairs([("GAMESERVER_LOG_CMDS", " LogOnline Warning ")]);
        assert_eq!(
            V1147.launch_args(&quiet).last().map(String::as_str),
            Some("-LogCmds=\"LogOnline Warning\"")
        );
        let bad = Settings::from_pairs([("GAMESERVER_LOG_CMDS", "x\" -evil")]);
        assert!(
            V1147
                .launch_args(&bad)
                .last()
                .is_some_and(|a| a.contains("LogLoad Off"))
        );
        assert!(V1147.log_cmds_invalid(&bad) && !V1147.log_cmds_invalid(&quiet));
        assert!(V1147.hide_window(&Settings::default()));
        assert!(!V1147.hide_window(&Settings::from_pairs([("GAMESERVER_WINDOWS_HIDE", "0")])));
    }

    #[tokio::test]
    async fn director_service_wires_in_process_director_when_launch_is_configured_even_with_deployserver_url() {
        let mut shared = Shared::bare(isles_1_14_7::version());
        shared.settings = Settings::from_pairs([
            ("DEPLOYSERVER_URL", "127.0.0.1:1"),
            ("GAMESERVER_BINARY_PATH", "/srv/game"),
            ("METAGAME_API_KEY", "gs-secret"),
            ("PORT_RANGE_BEGIN", "8766"),
            ("PORT_RANGE_END", "8777"),
        ]);
        shared.matchmaking = Arc::new(isles_core::areas::matchmaking::state::MatchmakingState::from_settings(
            &shared.settings,
        ));
        let shared = Arc::new(shared);
        // Before DirectorService::app, HttpDeployServer points to dead 127.0.0.1:1 and hub_status is None.
        assert_eq!(shared.matchmaking.dedicated_servers().hub_status().await, None);
        let service = DirectorService::new(V1147);
        assert!(service.app(shared.clone()).is_ok());
        // After DirectorService::app, InProcessDirector answers directly in memory without hitting 127.0.0.1:1.
        assert!(shared.matchmaking.dedicated_servers().configured());
        assert_eq!(
            shared.matchmaking.dedicated_servers().hub_status().await,
            Some(LiveHubStatus {
                ramsgate: None,
                training_dojo: None,
            })
        );
    }
}
