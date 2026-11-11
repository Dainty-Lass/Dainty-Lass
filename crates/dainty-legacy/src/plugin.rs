// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The `legacy` plugin (spec §10): serves nothing, and runs the imports as console commands. An import reads
//! the host's files, so only the console may run one.

use std::sync::{Mutex, PoisonError};

use dainty_module_api::actor::Actor;
use dainty_module_api::manifest::{Capabilities, ModuleKind, ModuleManifest};
use dainty_module_api::registrar::{CommandSpec, Registrar};
use dainty_module_api::{
    API_VERSION, BoxFuture, CommandOutput, CommandRequest, ErrorKind, GameModule, ModuleConfig, ModuleContext,
    ModuleError, ModuleStatus, Phase,
};

use crate::args::{self, Kind};

const TARGET: &str = "--to <1.14.7 database> or --env-file <the 1.14.7 backend's env file>";

/// The plugin.
#[derive(Debug, Default)]
pub struct LegacyPlugin {
    phase: Mutex<Option<Phase>>,
}

impl LegacyPlugin {
    /// A plugin not yet started.
    pub fn new() -> Self {
        Self::default()
    }

    fn set(&self, phase: Phase) {
        *self.phase.lock().unwrap_or_else(PoisonError::into_inner) = Some(phase);
    }
}

fn command_kind(name: &str) -> Option<Kind> {
    name.strip_prefix("import-").and_then(Kind::parse)
}

impl GameModule for LegacyPlugin {
    fn manifest(&self) -> ModuleManifest {
        let (Ok(id), Ok(game_version)) = ("legacy".parse(), "1.14.7".parse()) else {
            unreachable!("the plugin's constants parse")
        };
        ModuleManifest {
            id,
            kind: ModuleKind::Plugin,
            display_name: String::from("Legacy import"),
            game_version,
            invite_aliases: Vec::new(),
            api: API_VERSION,
            ports: Vec::new(),
            capabilities: Capabilities::default(),
            unverified: false,
            client: None,
        }
    }

    fn register(&self, registrar: &mut Registrar) -> Result<(), ModuleError> {
        registrar
            .command(
                CommandSpec::new("import-revived", "Import a Dauntless Revived 1.4.4 server's accounts")
                    .arg(
                        "database",
                        true,
                        "The 1.4.4 metagame's SQLite database (absolute path); never written",
                    )
                    .arg("target", true, TARGET)
                    .arg("options", false, "--node <id>, --dry-run"),
            )
            .command(
                CommandSpec::new(
                    "import-offline-save",
                    "Import a player's Dauntless Revived 1.4.4 offline save",
                )
                .arg(
                    "save",
                    true,
                    "The .drsave file or the bare offline database (absolute path)",
                )
                .arg("target", true, TARGET)
                .arg("options", false, "--node <id>, --dry-run"),
            )
            .command(
                CommandSpec::new(
                    "import-reforged",
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
        ModuleStatus::new(
            self.phase
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .unwrap_or(Phase::Loaded),
        )
    }

    fn command(
        &self,
        ctx: ModuleContext,
        request: CommandRequest,
    ) -> BoxFuture<'_, Result<CommandOutput, ModuleError>> {
        Box::pin(async move {
            let Some(kind) = command_kind(&request.name) else {
                return Err(ModuleError::unsupported(format!("command {}", request.name)));
            };
            if request.actor.actor != Actor::Console {
                return Err(ModuleError::new(
                    ErrorKind::Invalid,
                    "an import reads the host's files: run it from the host's console",
                ));
            }
            let parsed = args::parse(kind, &request.args)?;
            let node = ctx.node().node.as_str().to_owned();
            let handle = tokio::runtime::Handle::current();
            // Off the async threads: a copy may take minutes, and the reads block
            let report = tokio::task::spawn_blocking(move || handle.block_on(crate::run(parsed, &node)))
                .await
                .map_err(|_| ModuleError::failed("the import stopped"))??;
            let data = serde_json::to_value(&report).map_err(|e| ModuleError::failed(e.to_string()))?;
            Ok(CommandOutput {
                text: report.text(),
                data: Some(data),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use dainty_module_api::stub::RecordingHost;
    use dainty_module_api::{ActorContext, NodeIdentity};

    use super::*;

    fn ctx() -> ModuleContext {
        ModuleContext::new(
            "legacy".parse().unwrap(),
            NodeIdentity::standalone(),
            Arc::new(RecordingHost::new(NodeIdentity::standalone())),
        )
    }

    fn request(name: &str, actor: Actor, args: &[&str]) -> CommandRequest {
        CommandRequest {
            actor: ActorContext::new(actor),
            name: name.into(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
        }
    }

    #[test]
    fn manifest_validates() {
        let manifest = LegacyPlugin::new().manifest();
        manifest.validate().unwrap();
        assert_eq!(manifest.id.as_str(), "legacy");
        assert_eq!(manifest.kind, ModuleKind::Plugin);
    }

    #[tokio::test]
    async fn only_the_console_runs_an_import() {
        let plugin = LegacyPlugin::new();
        assert_eq!(plugin.status().phase, Phase::Loaded);
        let err = plugin
            .command(ctx(), request("import-nothing", Actor::Console, &[]))
            .await
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Unsupported);
        let err = plugin
            .command(ctx(), request("import-revived", Actor::System, &[]))
            .await
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Invalid);
        assert_eq!(
            err.message,
            "an import reads the host's files: run it from the host's console"
        );
        let err = plugin
            .command(ctx(), request("import-revived", Actor::Console, &["--what"]))
            .await
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Invalid);
        assert!(
            err.message.starts_with("unknown option --what\nusage:"),
            "{}",
            err.message
        );
    }
}
