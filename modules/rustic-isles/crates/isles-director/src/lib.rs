//! The Director both Rustic Isles modules share (RU6 ports it from `ParadoxDirector`, starting from `cdc0a87`), with
//! its per-version config (RU3c, seam 5): the game servers' launch arguments, the changelist the runtime updater
//! checks, and the hunt tables in the module's game data root. Its routes are the route table's `Director` rows,
//! served on the module's `director` port, which no gateway route reaches: the backend calls it on loopback.

use std::path::PathBuf;
use std::sync::Arc;

use isles_core::PortService;
use isles_core::game_data::GameDataRoot;
use isles_core::http::{Handlers, Shared};
use isles_core::routes::Service;
use isles_core::settings::Settings;
use module_api::ModuleError;

/// The persistent hubs, the same maps in both versions (RU3c).
pub const RAMSGATE_MAP: &str = "/Game/Maps/ramsgate/ramsgate_01_persistent";
pub const TRAINING_DOJO_MAP: &str = "/Game/Maps/islands/dojo/training_dojo_persistent";

/// `GAMESERVER_ARGS`: what comes before the positional launch arguments (`controllers/gameservers.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameserverArgs {
    pub standard: &'static [&'static str],
    /// The `-LogCmds` value
    pub log_cmds: &'static str,
    /// Whether `GAMESERVER_LOG_CMDS` may replace it (1.14.7)
    pub log_cmds_setting: bool,
    /// Whether the window is hidden unless `GAMESERVER_WINDOWS_HIDE=0` (1.14.7); shown otherwise
    pub windows_hide: bool,
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
    /// The arguments every game server starts with, before the positional ones.
    pub fn launch_args(&self, settings: &Settings) -> Vec<String> {
        let mut args: Vec<String> = self.args.standard.iter().map(|a| (*a).to_owned()).collect();
        let configured = self
            .args
            .log_cmds_setting
            .then(|| settings.raw("GAMESERVER_LOG_CMDS").unwrap_or_default().trim())
            .filter(|value| valid_log_cmds(value));
        args.push(format!("-LogCmds=\"{}\"", configured.unwrap_or(self.args.log_cmds)));
        args
    }

    /// `windowsHide`: `(GAMESERVER_WINDOWS_HIDE ?? "1") !== "0"` on 1.14.7.
    pub fn hide_window(&self, settings: &Settings) -> bool {
        self.args.windows_hide && settings.raw("GAMESERVER_WINDOWS_HIDE").unwrap_or("1") != "0"
    }

    /// A hunt table (`isles_core::game_data::DIRECTOR_FILES`) in the module's root.
    pub fn hunt_table(&self, game_data: &GameDataRoot, name: &str) -> Option<PathBuf> {
        game_data.director_file(name)
    }
}

/// The Director's handlers; RU6 fills them. A route with none answers 501.
pub fn register(_config: &Arc<DirectorConfig>, _handlers: &mut Handlers) {}

/// The Director as one of the module's listeners.
#[derive(Debug, Clone)]
pub struct DirectorService {
    config: Arc<DirectorConfig>,
}

impl DirectorService {
    pub fn new(config: DirectorConfig) -> Self {
        Self {
            config: Arc::new(config),
        }
    }

    pub fn config(&self) -> &DirectorConfig {
        &self.config
    }
}

impl PortService for DirectorService {
    fn port(&self) -> &'static str {
        "director"
    }

    fn app(&self, shared: Arc<Shared>) -> Result<axum::Router, ModuleError> {
        let mut handlers = Handlers::default();
        register(&self.config, &mut handlers);
        isles_core::http::app(shared, Service::Director, &handlers).map_err(|e| ModuleError::failed(e.to_string()))
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
            log_cmds_setting: true,
            windows_hide: true,
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
        assert!(V1147.hide_window(&Settings::default()));
        assert!(!V1147.hide_window(&Settings::from_pairs([("GAMESERVER_WINDOWS_HIDE", "0")])));
    }
}
