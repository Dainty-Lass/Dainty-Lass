//! The module panel over the embedded server's registry (DU3, DF3): the page lists, loads, unloads and runs
//! the commands of the modules `host.toml` lists, by id, and edits their settings. Nothing here takes a path
//! or an executable from the page (DD5): the registry only knows what `host.toml` listed. The checks that need
//! no server are `bridge::modules`'s.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use module_api::{Actor, ActorContext, CommandRequest, ModuleId};
use server_controller::{ModuleInfo, ModuleRegistry, ModuleState, RegistryError};

use super::{Slot, lock};
use crate::bridge::modules::{
    self, DETAIL_MAX, Listed, ModulePanel, OUTPUT_DATA_MAX, Settings, command_args, merge_settings, module_id,
    output_text, plain, refusal,
};
use crate::bridge::types::{
    ErrorCode, LauncherError, ModuleCommandOutcome, ModuleListOutcome, ModuleOrigin, ModuleOutcome, ModuleRunState,
    ModuleView,
};
use crate::platform::log;

/// A command that does not answer by now is given up on (the module keeps running; its answer is dropped).
const COMMAND_TIMEOUT: Duration = Duration::from_secs(120);

/// The registry and the checkout of the server inside the launcher, set when it starts and cleared when it
/// stops (the same slots `EmbeddedServerState` fills).
#[derive(Clone, Default)]
pub struct Panel {
    pub registry: Slot<ModuleRegistry>,
    pub workspace: Slot<PathBuf>,
}

fn run_state(state: ModuleState) -> ModuleRunState {
    match state {
        ModuleState::NotLoaded => ModuleRunState::NotLoaded,
        ModuleState::Starting => ModuleRunState::Starting,
        ModuleState::Ready => ModuleRunState::Ready,
        ModuleState::Failed => ModuleRunState::Failed,
    }
}

fn listed(info: ModuleInfo) -> Listed {
    Listed {
        id: info.id,
        origin: if info.source == "external" {
            ModuleOrigin::External
        } else {
            ModuleOrigin::CompiledIn
        },
        autoload: info.autoload,
        state: run_state(info.state),
        detail: info.detail,
        manifest: info.manifest,
        ports: info.ports,
        commands: info.commands,
        config: info.config,
    }
}

fn registry_error(error: &RegistryError) -> LauncherError {
    match error {
        RegistryError::NotListed(id) => refusal(ErrorCode::ModuleUnknown, id.to_string()),
        RegistryError::AlreadyLoaded(_) => refusal(ErrorCode::ModuleBadState, "loaded".to_owned()),
        RegistryError::NotLoaded(_) => refusal(ErrorCode::ModuleBadState, "not_loaded".to_owned()),
        other => refusal(ErrorCode::ModuleFailed, plain(&other.to_string(), DETAIL_MAX)),
    }
}

fn host_toml_error(why: impl std::fmt::Display) -> LauncherError {
    log::warn(&format!("host: module settings: {why}"));
    refusal(ErrorCode::ModuleFailed, "host_toml".to_owned())
}

impl Panel {
    fn registry(&self) -> Result<ModuleRegistry, LauncherError> {
        lock(&self.registry)
            .clone()
            .ok_or_else(|| LauncherError::new(ErrorCode::ServerStopped))
    }

    fn paths(&self) -> Option<host::paths::Paths> {
        lock(&self.workspace).clone().map(host::paths::Paths::new)
    }

    /// What host.toml sets, by module; empty when the server has no host.toml yet.
    async fn saved(&self) -> Result<BTreeMap<ModuleId, Settings>, LauncherError> {
        let Some(paths) = self.paths() else {
            return Ok(BTreeMap::new());
        };
        let read = tauri::async_runtime::spawn_blocking(move || host::hostconfig::load_host_config(&paths))
            .await
            .map_err(host_toml_error)?
            .map_err(host_toml_error)?;
        Ok(read
            .and_then(|config| config.modules)
            .map(|modules| modules.settings)
            .unwrap_or_default())
    }

    async fn view_of(&self, id: &ModuleId) -> Result<ModuleView, LauncherError> {
        let info = self.registry()?.info(id).map_err(|e| registry_error(&e))?;
        let saved = self.saved().await?;
        Ok(modules::view(&listed(info), saved.get(id).unwrap_or(&Settings::new())))
    }

    async fn list(&self) -> Result<Vec<ModuleView>, LauncherError> {
        let infos = self.registry()?.list();
        let saved = self.saved().await?;
        let none = Settings::new();
        Ok(infos
            .into_iter()
            .map(|info| {
                let settings = saved.get(&info.id).unwrap_or(&none);
                modules::view(&listed(info), settings)
            })
            .collect())
    }

    async fn run(&self, module: &str, command: &str, input: &str) -> Result<(String, Option<String>), LauncherError> {
        let id = module_id(module)?;
        let registry = self.registry()?;
        let info = registry.info(&id).map_err(|e| registry_error(&e))?;
        if info.state != ModuleState::Ready {
            return Err(refusal(ErrorCode::ModuleBadState, "not_loaded".to_owned()));
        }
        let spec = info
            .commands
            .iter()
            .find(|spec| spec.name == command)
            .ok_or_else(|| refusal(ErrorCode::ModuleInputInvalid, "command".to_owned()))?;
        let args = command_args(spec, input)?;
        // Whoever has this window has the server, as with the console (DH1)
        let request = CommandRequest {
            actor: ActorContext::new(Actor::Console),
            name: spec.name.clone(),
            args,
        };
        log::info(&format!(
            "host: module {id} command {} run from the server GUI",
            spec.name
        ));
        let output = tokio::time::timeout(COMMAND_TIMEOUT, registry.command(&id, request))
            .await
            .map_err(|_| refusal(ErrorCode::ModuleFailed, "timeout".to_owned()))?
            .map_err(|error| refusal(ErrorCode::ModuleFailed, plain(&error.message, DETAIL_MAX)))?;
        let data = output
            .data
            .and_then(|data| serde_json::to_string(&data).ok())
            .filter(|data| data.len() <= OUTPUT_DATA_MAX);
        Ok((output_text(&output.text), data))
    }

    async fn set_settings(&self, module: &str, values: &str) -> Result<ModuleView, LauncherError> {
        let id = module_id(module)?;
        let registry = self.registry()?;
        let info = registry.info(&id).map_err(|e| registry_error(&e))?;
        let paths = self
            .paths()
            .ok_or_else(|| LauncherError::new(ErrorCode::ServerStopped))?;
        let (schema, values) = (info.config, values.to_owned());
        let key = id.clone();
        // The file is read and written in one step, so two saves can't undo each other
        let merged = tauri::async_runtime::spawn_blocking(move || -> Result<Settings, LauncherError> {
            let mut config = host::hostconfig::load_host_config(&paths)
                .map_err(host_toml_error)?
                .ok_or_else(|| host_toml_error("the server has no host.toml yet"))?;
            let modules = config.modules.get_or_insert_with(Default::default);
            let merged = merge_settings(&schema, modules.settings.get(&key).unwrap_or(&Settings::new()), &values)?;
            modules.settings.insert(key, merged.clone());
            host::hostconfig::save_host_config(&paths, &config).map_err(host_toml_error)?;
            Ok(merged)
        })
        .await
        .map_err(host_toml_error)??;
        // The next load takes them; one that is running keeps what it started with
        registry.update_settings(&id, merged).map_err(|e| registry_error(&e))?;
        log::info(&format!("host: module {id} settings saved from the server GUI"));
        self.view_of(&id).await
    }

    async fn load(&self, module: &str) -> Result<ModuleView, LauncherError> {
        let id = module_id(module)?;
        self.registry()?.load(&id).await.map_err(|e| registry_error(&e))?;
        log::info(&format!("host: module {id} loaded from the server GUI"));
        self.view_of(&id).await
    }

    async fn unload(&self, module: &str) -> Result<ModuleView, LauncherError> {
        let id = module_id(module)?;
        self.registry()?.unload(&id).await.map_err(|e| registry_error(&e))?;
        log::info(&format!("host: module {id} unloaded from the server GUI"));
        self.view_of(&id).await
    }
}

fn outcome(result: Result<ModuleView, LauncherError>) -> ModuleOutcome {
    match result {
        Ok(view) => ModuleOutcome::Ok(Box::new(view)),
        Err(error) => ModuleOutcome::Err(error),
    }
}

impl ModulePanel for Panel {
    fn list(&self) -> super::Boxed<ModuleListOutcome> {
        let panel = self.clone();
        Box::pin(async move {
            match panel.list().await {
                Ok(views) => ModuleListOutcome::Ok(views),
                Err(error) => ModuleListOutcome::Err(error),
            }
        })
    }

    fn get(&self, module: String) -> super::Boxed<ModuleOutcome> {
        let panel = self.clone();
        Box::pin(async move {
            outcome(match module_id(&module) {
                Ok(id) => panel.view_of(&id).await,
                Err(error) => Err(error),
            })
        })
    }

    fn run(&self, module: String, command: String, input: String) -> super::Boxed<ModuleCommandOutcome> {
        let panel = self.clone();
        Box::pin(async move {
            match panel.run(&module, &command, &input).await {
                Ok((text, data)) => ModuleCommandOutcome::Ok { text, data },
                Err(error) => ModuleCommandOutcome::Err(error),
            }
        })
    }

    fn set_settings(&self, module: String, values: String) -> super::Boxed<ModuleOutcome> {
        let panel = self.clone();
        Box::pin(async move { outcome(panel.set_settings(&module, &values).await) })
    }

    fn load(&self, module: String) -> super::Boxed<ModuleOutcome> {
        let panel = self.clone();
        Box::pin(async move { outcome(panel.load(&module).await) })
    }

    fn unload(&self, module: String) -> super::Boxed<ModuleOutcome> {
        let panel = self.clone();
        Box::pin(async move { outcome(panel.unload(&module).await) })
    }
}

/// The panel as the page's commands hold it.
pub fn shared(panel: &Panel) -> Arc<dyn ModulePanel> {
    Arc::new(panel.clone())
}

#[cfg(test)]
mod tests;
