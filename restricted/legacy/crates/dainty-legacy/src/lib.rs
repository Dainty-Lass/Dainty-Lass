//! `legacy`, a Dainty Lass plugin for Dauntless 1.14.7 (DD25): the imports of `legacy-import` as console commands.
//! It runs as its own process (DD5, `dainty_module_api::child::main`), so its AGPL code never links into Dainty Lass's MIT
//! core, and it serves nothing: it loads beside `isles-1-14-7` and waits for a command.
//!
//! An import reads files on the host's disk by path and writes accounts, so only the host's console may run one; an
//! admin API caller is refused (docs/security/importer.md).

use std::sync::Mutex;

use dainty_legacy_import::cli::{self, Kind};
use dainty_module_api::actor::Actor;
use dainty_module_api::registrar::CommandSpec;
use dainty_module_api::{
    API_VERSION, BoxFuture, Capabilities, CommandOutput, CommandRequest, ErrorKind, GameModule, ModuleConfig,
    ModuleContext, ModuleError, ModuleKind, ModuleManifest, ModuleStatus, Phase, Registrar,
};

pub const MODULE_ID: &str = "legacy";
pub const DISPLAY_NAME: &str = "Legacy import";
/// The game version the plugin extends: the one an import upgrades to
pub const GAME_VERSION: &str = "1.14.7";

/// The commands, `module run legacy <command> ...`; `dainty-host legacy import <kind>` maps onto them
pub mod commands {
    pub const REVIVED: &str = "import-revived";
    pub const OFFLINE_SAVE: &str = "import-offline-save";
    pub const REFORGED: &str = "import-reforged";
}

pub fn manifest() -> ModuleManifest {
    let (Ok(id), Ok(game_version)) = (MODULE_ID.parse(), GAME_VERSION.parse()) else {
        unreachable!("the module's constants parse")
    };
    ModuleManifest {
        id,
        kind: ModuleKind::Plugin,
        display_name: DISPLAY_NAME.to_owned(),
        game_version,
        invite_aliases: Vec::new(),
        api: API_VERSION,
        ports: Vec::new(),
        capabilities: Capabilities::default(),
        unverified: false,
        client: None,
    }
}

fn kind_of(command: &str) -> Option<Kind> {
    match command {
        commands::REVIVED => Some(Kind::Revived),
        commands::OFFLINE_SAVE => Some(Kind::OfflineSave),
        commands::REFORGED => Some(Kind::Reforged),
        _ => None,
    }
}

fn error(error: dainty_legacy_import::ImportError) -> ModuleError {
    use dainty_legacy_import::ImportError as E;
    let kind = match &error {
        E::Invalid(_) | E::Usage(_) => ErrorKind::Invalid,
        E::Target(_) | E::Io(_) => ErrorKind::Failed,
    };
    ModuleError::new(kind, error.to_string())
}

pub struct LegacyPlugin {
    phase: Mutex<Phase>,
}

impl Default for LegacyPlugin {
    fn default() -> Self {
        Self {
            phase: Mutex::new(Phase::Loaded),
        }
    }
}

impl LegacyPlugin {
    fn set(&self, phase: Phase) {
        *self.phase.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = phase;
    }
}

pub fn module() -> LegacyPlugin {
    LegacyPlugin::default()
}

impl GameModule for LegacyPlugin {
    fn manifest(&self) -> ModuleManifest {
        manifest()
    }

    fn register(&self, registrar: &mut Registrar) -> Result<(), ModuleError> {
        let target = "--to <1.14.7 database> or --env-file <the 1.14.7 backend's env file>";
        registrar
            .command(
                CommandSpec::new(commands::REVIVED, "Import a Dauntless Revived 1.4.4 server's accounts")
                    .arg(
                        "database",
                        true,
                        "The 1.4.4 metagame's SQLite database (absolute path); never written",
                    )
                    .arg("target", true, target)
                    .arg("options", false, "--node <id>, --dry-run"),
            )
            .command(
                CommandSpec::new(
                    commands::OFFLINE_SAVE,
                    "Import a player's Dauntless Revived 1.4.4 offline save",
                )
                .arg(
                    "save",
                    true,
                    "The .drsave file or the bare offline database (absolute path)",
                )
                .arg("target", true, target)
                .arg("options", false, "--node <id>, --dry-run"),
            )
            .command(
                CommandSpec::new(
                    commands::REFORGED,
                    "Upgrade a Reforged Revival Node-kit install to Dainty Lass",
                )
                .arg("checkout", true, "The old install's folder (absolute path)")
                .arg(
                    "options",
                    false,
                    "--to/--env-file, --fresh-start, --db-tool <program>, --dry-run",
                ),
            );
        Ok(())
    }

    fn start(&self, _ctx: ModuleContext, _config: ModuleConfig) -> BoxFuture<'_, Result<(), ModuleError>> {
        Box::pin(async move {
            self.set(Phase::Ready);
            Ok(())
        })
    }

    fn stop(&self, _ctx: ModuleContext) -> BoxFuture<'_, Result<(), ModuleError>> {
        Box::pin(async move {
            self.set(Phase::Stopped);
            Ok(())
        })
    }

    fn status(&self) -> ModuleStatus {
        ModuleStatus::new(*self.phase.lock().unwrap_or_else(std::sync::PoisonError::into_inner))
    }

    fn command(
        &self,
        ctx: ModuleContext,
        request: CommandRequest,
    ) -> BoxFuture<'_, Result<CommandOutput, ModuleError>> {
        Box::pin(async move {
            let Some(kind) = kind_of(&request.name) else {
                return Err(ModuleError::unsupported(format!("command {}", request.name)));
            };
            if !matches!(request.actor.actor, Actor::Console) {
                return Err(ModuleError::new(
                    ErrorKind::Invalid,
                    "an import reads the host's files: run it from the host's console",
                ));
            }
            let parsed = cli::parse(kind, &request.args).map_err(error)?;
            let report = cli::run(&parsed, &ctx.node().node).await.map_err(error)?;
            Ok(CommandOutput {
                text: report.text(),
                data: serde_json::to_value(&report).ok(),
            })
        })
    }
}
