// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The server GUI's module panels over the running server's registry (spec-tauri-shell §6, DU3, DF3). Nothing here
//! takes a path or a program from the page (DD5): the registry only knows what `settings.toml` listed. The page's
//! input is checked by the launcher kit's `bridge::modules` functions.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dainty_controller::{ModuleInfo, ModuleRegistry, ModuleState, RegistryError};
use dainty_host::hostconfig::{HostConfig, ModulesConfig, load_host_config, save_host_config};
use dainty_host::paths::Paths;
use dainty_launcher_kit::bridge::modules::{
    DETAIL_MAX, Listed, ModulePanel, OUTPUT_DATA_MAX, Settings, command_args, merge_settings, module_id, output_text,
    plain, refusal, view,
};
use dainty_launcher_kit::bridge::types::{
    AdminActionKind, AdminOutcome, AdminPlayerView, AdminResultView, ErrorCode, LauncherError, ModuleCommandOutcome,
    ModuleListOutcome, ModuleOrigin, ModuleOutcome, ModuleRunState, ModuleView,
};
use dainty_module_api::{
    Actor, ActorContext, AdminCall, AdminRequest, AdminResponse, CommandRequest, ModuleError, ModuleId, ModuleKind,
    PlayerInfo,
};

use crate::{Boxed, lock};

/// How long a module may take to answer the page; past it the module keeps running and its answer is dropped.
pub const MODULE_TIMEOUT: Duration = Duration::from_secs(120);
/// How much of a player's name or input a message or log line repeats.
const PLAYER_MAX: usize = 64;

#[derive(Default)]
struct Slots {
    registry: Mutex<Option<ModuleRegistry>>,
    workspace: Mutex<Option<PathBuf>>,
}

/// The module panel: the registry and the workspace of the running server, set by its start and cleared on stop.
#[derive(Clone, Default)]
pub struct ServerModules {
    slots: Arc<Slots>,
}

type Refused<T> = Result<T, LauncherError>;

fn stopped() -> LauncherError {
    refusal(ErrorCode::ServerStopped, None)
}

fn not_loaded() -> LauncherError {
    refusal(ErrorCode::ModuleBadState, "not_loaded".to_owned())
}

fn settings_toml_error(why: impl std::fmt::Display) -> LauncherError {
    tracing::warn!(service = "host", "module settings: {why}");
    refusal(ErrorCode::ModuleFailed, "settings_toml".to_owned())
}

fn registry_error(error: RegistryError) -> LauncherError {
    match error {
        RegistryError::NotListed(id) => refusal(ErrorCode::ModuleUnknown, id.to_string()),
        RegistryError::AlreadyLoaded(_) => refusal(ErrorCode::ModuleBadState, "loaded".to_owned()),
        RegistryError::NotLoaded(_) => not_loaded(),
        other => refusal(ErrorCode::ModuleFailed, plain(&other.to_string(), DETAIL_MAX)),
    }
}

fn module_error(error: &ModuleError) -> LauncherError {
    refusal(ErrorCode::ModuleFailed, plain(&error.message, DETAIL_MAX))
}

fn timed_out() -> LauncherError {
    refusal(ErrorCode::ModuleFailed, "timeout".to_owned())
}

fn console() -> ActorContext {
    // Whoever has this window has the server (DH1)
    ActorContext::new(Actor::Console)
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
        state: match info.state {
            ModuleState::NotLoaded => ModuleRunState::NotLoaded,
            ModuleState::Starting => ModuleRunState::Starting,
            ModuleState::Ready => ModuleRunState::Ready,
            ModuleState::Failed => ModuleRunState::Failed,
        },
        detail: info.detail,
        manifest: info.manifest,
        ports: info.ports,
        commands: info.commands,
        config: info.config,
    }
}

fn load_config(workspace: &std::path::Path) -> Refused<HostConfig> {
    match load_host_config(&Paths::new(workspace)) {
        Ok(Some(config)) => Ok(config),
        Ok(None) => Err(settings_toml_error("there is no settings.toml")),
        Err(why) => Err(settings_toml_error(why)),
    }
}

fn saved_in(config: &HostConfig, id: &ModuleId) -> Settings {
    config
        .modules
        .as_ref()
        .and_then(|modules| modules.settings.get(id).cloned())
        .unwrap_or_default()
}

fn cut(text: &str) -> String {
    text.chars().take(PLAYER_MAX).collect()
}

fn one_hour_from_now() -> String {
    (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn player_view(info: &PlayerInfo, input: &str) -> AdminPlayerView {
    AdminPlayerView {
        global_id: info
            .account
            .as_ref()
            .map_or_else(|| input.to_owned(), ToString::to_string),
        display_name: cut(&info.name),
        module_user_id: info
            .account
            .as_ref()
            .map_or_else(|| input.to_owned(), |account| account.ulid().to_string()),
        online: info.online,
        banned: false,
        ban_reason: None,
        timeout_until: None,
        created_at: None,
        last_login_at: None,
    }
}

// T-4: English made in Rust, as the restricted build did; the page does not call `admin_action` yet
fn done_message(action: AdminActionKind, name: &str) -> String {
    match action {
        AdminActionKind::Lookup => format!("Found {name}."),
        AdminActionKind::Kick => format!("{name} was kicked."),
        AdminActionKind::Timeout => format!("{name} is timed out."),
        AdminActionKind::Ban => format!("{name} is banned."),
        AdminActionKind::Unban => format!("{name} is no longer banned."),
    }
}

impl ServerModules {
    /// A panel with no server behind it.
    pub fn new() -> Self {
        Self::default()
    }

    /// The running server's registry, or none to refuse every call.
    pub fn attach_registry(&self, registry: Option<ModuleRegistry>) {
        *lock(&self.slots.registry) = registry;
    }

    /// Forgets the registry of a server that stopped.
    pub fn detach_registry(&self) {
        *lock(&self.slots.registry) = None;
    }

    /// The workspace whose settings.toml holds the modules' settings.
    pub fn set_workspace(&self, workspace: Option<PathBuf>) {
        *lock(&self.slots.workspace) = workspace;
    }

    fn registry(&self) -> Refused<ModuleRegistry> {
        lock(&self.slots.registry).clone().ok_or_else(stopped)
    }

    fn workspace(&self) -> Option<PathBuf> {
        lock(&self.slots.workspace).clone()
    }

    /// The page's view of a module, with what settings.toml sets for it.
    async fn view(&self, info: ModuleInfo) -> ModuleView {
        let workspace = self.workspace();
        let id = info.id.clone();
        let saved = tokio::task::spawn_blocking(move || {
            workspace
                .and_then(|ws| load_host_config(&Paths::new(ws)).ok().flatten())
                .map(|config| saved_in(&config, &id))
                .unwrap_or_default()
        })
        .await
        .unwrap_or_default();
        view(&listed(info), &saved)
    }

    async fn list_modules(&self) -> Refused<Vec<ModuleView>> {
        let registry = self.registry()?;
        let mut views = Vec::new();
        for info in registry.list() {
            views.push(self.view(info).await);
        }
        Ok(views)
    }

    async fn get_module(&self, module: &str) -> Refused<ModuleView> {
        let id = module_id(module)?;
        let info = self.registry()?.info(&id).map_err(registry_error)?;
        Ok(self.view(info).await)
    }

    async fn load_module(&self, module: &str) -> Refused<ModuleView> {
        let id = module_id(module)?;
        let registry = self.registry()?;
        let info = registry.load(&id).await.map_err(registry_error)?;
        tracing::info!(service = "host", "module {id} loaded from the server GUI");
        Ok(self.view(info).await)
    }

    async fn unload_module(&self, module: &str) -> Refused<ModuleView> {
        let id = module_id(module)?;
        let registry = self.registry()?;
        registry.unload(&id).await.map_err(registry_error)?;
        tracing::info!(service = "host", "module {id} unloaded from the server GUI");
        let info = registry.info(&id).map_err(registry_error)?;
        Ok(self.view(info).await)
    }

    async fn run_command(&self, module: &str, command: &str, input: &str) -> Refused<(String, Option<String>)> {
        let id = module_id(module)?;
        let registry = self.registry()?;
        let info = registry.info(&id).map_err(registry_error)?;
        if info.state != ModuleState::Ready {
            return Err(not_loaded());
        }
        let spec = info
            .commands
            .iter()
            .find(|spec| spec.name == command)
            .ok_or_else(|| refusal(ErrorCode::ModuleInputInvalid, "command".to_owned()))?;
        let args = command_args(spec, input)?;
        tracing::info!(
            service = "host",
            "module {id} command {} run from the server GUI",
            spec.name
        );
        let request = CommandRequest {
            actor: console(),
            name: spec.name.clone(),
            args,
        };
        let output = tokio::time::timeout(MODULE_TIMEOUT, registry.command(&id, request))
            .await
            .map_err(|_| timed_out())?
            .map_err(|error| module_error(&error))?;
        let data = output
            .data
            .map(|data| data.to_string())
            .filter(|data| data.len() <= OUTPUT_DATA_MAX);
        Ok((output_text(&output.text), data))
    }

    async fn save_settings(&self, module: &str, values: String) -> Refused<ModuleView> {
        let id = module_id(module)?;
        let registry = self.registry()?;
        let workspace = self.workspace().ok_or_else(stopped)?;
        let info = registry.info(&id).map_err(registry_error)?;
        let schema = info.config;
        let key = id.clone();
        let merged = tokio::task::spawn_blocking(move || -> Refused<Settings> {
            let mut config = load_config(&workspace)?;
            let merged = merge_settings(&schema, &saved_in(&config, &key), &values)?;
            config
                .modules
                .get_or_insert_with(ModulesConfig::default)
                .settings
                .insert(key, merged.clone());
            save_host_config(&Paths::new(&workspace), &config).map_err(settings_toml_error)?;
            Ok(merged)
        })
        .await
        .map_err(settings_toml_error)??;
        // The next load takes them; a running module keeps what it started with
        registry.update_settings(&id, merged).map_err(registry_error)?;
        tracing::info!(service = "host", "module {id} settings saved from the server GUI");
        let info = registry.info(&id).map_err(registry_error)?;
        Ok(self.view(info).await)
    }

    /// The module an admin action goes to: the one named, else the first ready game module, else any ready one.
    fn admin_target(registry: &ModuleRegistry, module: Option<&str>) -> Refused<ModuleId> {
        if let Some(module) = module.map(str::trim).filter(|m| !m.is_empty()) {
            let id = module_id(module)?;
            let info = registry.info(&id).map_err(registry_error)?;
            return if info.state == ModuleState::Ready {
                Ok(id)
            } else {
                Err(not_loaded())
            };
        }
        let ready: Vec<ModuleInfo> = registry
            .list()
            .into_iter()
            .filter(|info| info.state == ModuleState::Ready)
            .collect();
        ready
            .iter()
            .find(|info| info.manifest.as_ref().is_none_or(|m| m.kind == ModuleKind::Game))
            .or_else(|| ready.first())
            .map(|info| info.id.clone())
            .ok_or_else(not_loaded)
    }

    async fn ask(registry: &ModuleRegistry, id: &ModuleId, request: AdminRequest) -> Refused<AdminResponse> {
        let call = AdminCall {
            actor: console(),
            request,
        };
        tokio::time::timeout(MODULE_TIMEOUT, registry.admin_for(id, call))
            .await
            .map_err(|_| timed_out())?
            .map_err(|error| module_error(&error))
    }

    async fn admin_action(
        &self,
        action: AdminActionKind,
        player: String,
        reason: Option<String>,
        until: Option<String>,
        module: Option<String>,
    ) -> Refused<AdminResultView> {
        let registry = self.registry()?;
        let id = Self::admin_target(&registry, module.as_deref())?;
        let until = match action {
            AdminActionKind::Timeout => Some(
                until
                    .filter(|until| !until.trim().is_empty())
                    .unwrap_or_else(one_hour_from_now),
            ),
            _ => None,
        };
        let request = match action {
            AdminActionKind::Lookup => AdminRequest::Lookup { player: player.clone() },
            AdminActionKind::Kick => AdminRequest::Kick {
                player: player.clone(),
                reason: reason.clone(),
            },
            AdminActionKind::Timeout => AdminRequest::Ban {
                player: player.clone(),
                reason: reason.clone(),
                until: until.clone(),
            },
            AdminActionKind::Ban => AdminRequest::Ban {
                player: player.clone(),
                reason: reason.clone(),
                until: None,
            },
            AdminActionKind::Unban => AdminRequest::Unban {
                player: player.clone(),
                reason: reason.clone(),
            },
        };
        tracing::info!(
            service = "host",
            "admin {action:?} on {} in {id} from the server GUI",
            cut(&player)
        );
        let shown = cut(&player);
        let result = |ok, message: String, player| AdminResultView {
            action,
            ok,
            message: Some(message),
            player,
        };
        match Self::ask(&registry, &id, request).await? {
            AdminResponse::NotFound => Ok(result(false, format!("No player matched {shown}."), None)),
            AdminResponse::Player(found) => {
                let view = player_view(&found, &player);
                let message = format!("Found {}.", view.display_name);
                Ok(result(true, message, Some(view)))
            }
            AdminResponse::Done => {
                let lookup = Self::ask(&registry, &id, AdminRequest::Lookup { player: player.clone() }).await;
                let view = match lookup {
                    Ok(AdminResponse::Player(found)) => Some(AdminPlayerView {
                        banned: action == AdminActionKind::Ban,
                        ban_reason: matches!(action, AdminActionKind::Ban | AdminActionKind::Timeout)
                            .then(|| reason.clone())
                            .flatten(),
                        timeout_until: until,
                        ..player_view(&found, &player)
                    }),
                    _ => None,
                };
                let name = view.as_ref().map_or(shown, |v| v.display_name.clone());
                Ok(result(true, done_message(action, &name), view))
            }
        }
    }
}

fn outcome(result: Refused<ModuleView>) -> ModuleOutcome {
    match result {
        Ok(view) => ModuleOutcome::Ok(Box::new(view)),
        Err(error) => ModuleOutcome::Err(error),
    }
}

impl ModulePanel for ServerModules {
    fn list(&self) -> Boxed<ModuleListOutcome> {
        let this = self.clone();
        Box::pin(async move {
            match this.list_modules().await {
                Ok(views) => ModuleListOutcome::Ok(views),
                Err(error) => ModuleListOutcome::Err(error),
            }
        })
    }

    fn get(&self, module: String) -> Boxed<ModuleOutcome> {
        let this = self.clone();
        Box::pin(async move { outcome(this.get_module(&module).await) })
    }

    fn run(&self, module: String, command: String, input: String) -> Boxed<ModuleCommandOutcome> {
        let this = self.clone();
        Box::pin(async move {
            match this.run_command(&module, &command, &input).await {
                Ok((text, data)) => ModuleCommandOutcome::Ok { text, data },
                Err(error) => ModuleCommandOutcome::Err(error),
            }
        })
    }

    fn set_settings(&self, module: String, values: String) -> Boxed<ModuleOutcome> {
        let this = self.clone();
        Box::pin(async move { outcome(this.save_settings(&module, values).await) })
    }

    fn load(&self, module: String) -> Boxed<ModuleOutcome> {
        let this = self.clone();
        Box::pin(async move { outcome(this.load_module(&module).await) })
    }

    fn unload(&self, module: String) -> Boxed<ModuleOutcome> {
        let this = self.clone();
        Box::pin(async move { outcome(this.unload_module(&module).await) })
    }

    fn admin(
        &self,
        action: AdminActionKind,
        player: String,
        reason: Option<String>,
        until: Option<String>,
        module: Option<String>,
    ) -> Boxed<AdminOutcome> {
        let this = self.clone();
        Box::pin(async move {
            match this.admin_action(action, player, reason, until, module).await {
                Ok(result) => AdminOutcome::Ok(result),
                Err(error) => AdminOutcome::Err(error),
            }
        })
    }
}
