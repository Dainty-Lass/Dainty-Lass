// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The Host page (RD27 spec §2.5): the host kit's workspace, its requirements, the builds and game data, and
//! the server a host runs from the launcher, all through the workspace's `dainty-host`. With an embedded
//! server (the manager) the server and its host kit run in process (`manager`, §4.3).

use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use serde_json::json;

use super::Controller;
use super::platform::{FolderPurpose, Seams};
use super::stored::edit;
use crate::bridge::types::{
    ActionResult, ErrorCode, GameDataState, GameProblem, HostAction, HostFolder, HostInvite, HostJob, HostJobKind,
    HostJobState, HostKit, HostMultipliers, HostServerMode, HostServerState, HostServerView, HostSetupRequest,
    HostUpdateState, HostUpdateView, LauncherError, NoticeCode, Requirement,
};
use crate::platform::hostkit::{
    self, BACKEND_GAME_DATA, DIRECTOR_GAME_DATA, EXPORT_FLAGS, HOST_BINARY, Toolchain, WorkspacePaths, copy_tree,
    detect_game_data, detect_host_kit, exporter_build_args, find_dumps, find_toolchain, invite_line, is_workspace,
    last_json, msbuild_args, pack_written, platform_toolset, published_version, read_server_config, run_logged,
    run_logged_env, update_check, update_done, update_failure,
};
use crate::platform::hostserver::{CommandOutput, EmbeddedServer, HostServer, HostTool, Probe};
use crate::platform::launch::{LaunchPlan, STRIPPED_PREFIXES};
use crate::platform::loader_session::SessionKind;
use crate::platform::log;
use crate::platform::requirements::{Inputs, check_requirements_here};

#[cfg(feature = "server-gui")]
mod manager;

/// How long a requirements check is reused for one workspace.
pub const REQUIREMENTS_TTL: Duration = Duration::from_secs(15);
/// The workspace folder beside the game folder.
pub const PROJECT_FOLDER: &str = "DaintyLass";
/// The embedded server's default data folder in `userData`.
pub const SERVER_FOLDER: &str = "Server";
/// CatalogExporter's flags file, in Win64.
pub const EXPORT_FLAGS_FILE: &str = "export_flags.txt";
/// Dumper-7 needs the game without EAC and offline from Epic.
pub const DUMPER_ARGS: [&str; 3] = ["-NoEAC", "-EpicPortal", "-DisableEOS"];
/// Where the game's CatalogExporter writes, in Win64.
pub const GAME_EXPORTS: &str = "Items_Analysis";

/// A job's answer: done with a message, or failed with one (none: the job kind's name).
type Outcome = Result<Option<String>, Option<String>>;

struct Cached {
    at: Instant,
    workspace: Option<PathBuf>,
    list: Vec<Requirement>,
}

/// The Host page's state (§2.5.2), outside the controller's mutex.
pub(crate) struct HostState {
    server: HostServer,
    /// The workspace's backend port, which the default probe knocks on
    backend_port: Arc<std::sync::atomic::AtomicU16>,
    job: Mutex<Option<HostJob>>,
    update: Mutex<Option<HostUpdateView>>,
    requirements: Mutex<Option<Cached>>,
    dump_root: PathBuf,
    tool: Option<HostTool>,
}

impl HostState {
    pub(crate) fn new(seams: &Seams) -> Self {
        let backend_port = Arc::new(std::sync::atomic::AtomicU16::new(
            crate::platform::hostserver::BACKEND_PORT,
        ));
        let server = match &seams.host_server_probe {
            Some(probe) => HostServer::new(Arc::clone(probe) as Probe, seams.host_stop_timeout),
            None => {
                let port = Arc::clone(&backend_port);
                let probe: Probe = Arc::new(move || {
                    Box::pin(crate::platform::hostserver::port_answers(
                        port.load(std::sync::atomic::Ordering::Relaxed),
                    ))
                });
                HostServer::new(probe, seams.host_stop_timeout)
            }
        };
        Self {
            server,
            backend_port,
            job: Mutex::new(None),
            update: Mutex::new(None),
            requirements: Mutex::new(None),
            dump_root: seams.dumper_root.clone(),
            tool: seams.host_tool.clone(),
        }
    }
}

fn hold<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn failure(detail: &str) -> LauncherError {
    LauncherError::with_detail(ErrorCode::HostFailed, detail)
}

fn kind_name(kind: HostJobKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|arg| (*arg).to_owned()).collect()
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

impl Controller {
    fn host(&self) -> &HostState {
        &self.shared.host
    }

    /// Marks "server up" and emits `host-server` whenever the server's view changes.
    pub(crate) fn attach_host_server(&self) {
        let me = self.weak();
        self.host().server.set_listener(Arc::new(move |view: HostServerView| {
            if let Some(me) = Controller::from_weak(&me) {
                me.lock().host.server_up = matches!(
                    view.state,
                    HostServerState::Starting
                        | HostServerState::Running
                        | HostServerState::Stopping
                        | HostServerState::External
                );
                me.platform().emit_host_server(&view);
            }
        }));
    }

    /// The manager's in-process server; the Host page's server is then that one.
    pub fn set_embedded_server(&self, server: Arc<dyn EmbeddedServer>) {
        self.host().server.set_embedded(server);
    }

    fn embedded(&self) -> bool {
        self.host().server.has_embedded()
    }

    /// Exit: a server this launcher started is stopped.
    pub(crate) async fn host_shutdown(&self) {
        if self.host().server.running() {
            self.host().server.stop().await;
            log::info("host: the server was stopped on exit");
        }
    }

    fn refuse_with(&self, detail: &str) -> ActionResult {
        ActionResult::Err(self.refuse(failure(detail)))
    }

    fn refuse_code(&self, code: ErrorCode) -> ActionResult {
        ActionResult::Err(self.refuse(LauncherError::new(code)))
    }

    // --- The workspace (§2.5.3) ---

    fn workspace_valid(&self, dir: &Path) -> bool {
        if self.embedded() {
            dir.is_dir()
        } else {
            is_workspace(dir)
        }
    }

    fn valid_workspace(&self) -> Option<PathBuf> {
        let dir = self
            .stored()
            .host_workspace()
            .map(PathBuf::from)
            .filter(|dir| self.workspace_valid(dir))?;
        // [ports] can move the backend the probe looks for
        let port = crate::platform::hostserver::workspace_backend_port(&dir);
        self.host()
            .backend_port
            .store(port, std::sync::atomic::Ordering::Relaxed);
        Some(dir)
    }

    fn workspace_failure(&self) -> LauncherError {
        failure(if self.embedded() { "workspace_mgr" } else { "workspace" })
    }

    fn need_workspace(&self) -> Result<PathBuf, ActionResult> {
        self.valid_workspace()
            .ok_or_else(|| ActionResult::Err(self.refuse(self.workspace_failure())))
    }

    fn project_target(&self) -> Option<PathBuf> {
        let state = self.lock();
        if matches!(state.game.problem, Some(GameProblem::NotSet | GameProblem::NotFound)) {
            return None;
        }
        let root = state.game.root.clone()?;
        Path::new(&root).parent().map(|parent| parent.join(PROJECT_FOLDER))
    }

    fn clear_requirements(&self) {
        *hold(&self.host().requirements) = None;
    }

    async fn store_workspace(&self, dir: &Path) {
        let text = path_text(dir);
        self.edit_settings(|map| edit::field(map, "hostWorkspace", json!(text)))
            .await;
        self.clear_requirements();
        log::info(format!("host: workspace {}", dir.display()));
        self.emit_kit().await;
        self.publish();
    }

    /// Makes sure a Host workspace is chosen; `quiet` never fails for a missing one.
    pub async fn ensure_host_workspace(&self, quiet: bool) -> ActionResult {
        if self.embedded() {
            if !quiet {
                return self.refuse_code(ErrorCode::NotInThisBuild);
            }
            if self.valid_workspace().is_none() {
                let dir = self.user_data().join(SERVER_FOLDER);
                match std::fs::create_dir_all(&dir) {
                    Ok(()) => self.store_workspace(&dir).await,
                    Err(error) => log::warn(format!(
                        "host: could not make the server's folder: {}",
                        log::describe_error(&error)
                    )),
                }
            }
            return ActionResult::Ok;
        }
        if quiet && !self.stored().host_tools() {
            return ActionResult::Ok;
        }
        if self.valid_workspace().is_some() {
            return ActionResult::Ok;
        }
        if self.job_running() {
            return if quiet {
                ActionResult::err(ErrorCode::Busy)
            } else {
                self.refuse_code(ErrorCode::Busy)
            };
        }
        let refused = |detail| {
            if quiet {
                ActionResult::Ok
            } else {
                self.refuse_with(detail)
            }
        };
        let Some(target) = self.project_target() else {
            return refused("project_no_game");
        };
        if !is_workspace(&target) {
            return refused(if target.exists() {
                "project_folder_taken"
            } else {
                "project_missing"
            });
        }
        self.store_workspace(&target).await;
        ActionResult::Ok
    }

    // --- Jobs (§2.5.3) ---

    fn job_running(&self) -> bool {
        hold(&self.host().job)
            .as_ref()
            .is_some_and(|job| job.state == HostJobState::Running)
    }

    fn busy_job(&self) -> Result<(), ActionResult> {
        if self.job_running() {
            return Err(self.refuse_code(ErrorCode::Busy));
        }
        Ok(())
    }

    async fn begin_job(&self, kind: HostJobKind, message: Option<String>, log_file: Option<&Path>) -> bool {
        {
            let mut job = hold(&self.host().job);
            if job.as_ref().is_some_and(|job| job.state == HostJobState::Running) {
                return false;
            }
            *job = Some(HostJob {
                kind,
                state: HostJobState::Running,
                message,
                log_file: log_file.map(path_text),
            });
        }
        self.emit_kit().await;
        true
    }

    async fn end_job(&self, kind: HostJobKind, outcome: Outcome) -> ActionResult {
        let (state, message) = match &outcome {
            Ok(message) => (HostJobState::Done, message.clone()),
            Err(message) => (HostJobState::Failed, message.clone()),
        };
        if let Some(job) = hold(&self.host().job).as_mut() {
            job.state = state;
            if message.is_some() {
                job.message = message;
            }
        }
        self.emit_kit().await;
        match outcome {
            Ok(_) => {
                self.lock().notice = Some(if kind == HostJobKind::ImportSdk {
                    NoticeCode::SdkImported
                } else {
                    NoticeCode::HostDone
                });
                self.publish();
                ActionResult::Ok
            }
            Err(message) => {
                let message = message.unwrap_or_else(|| kind_name(kind));
                log::warn(format!("host: {} failed: {message}", kind_name(kind)));
                ActionResult::Err(self.refuse(failure(&message)))
            }
        }
    }

    async fn run_job(
        &self,
        kind: HostJobKind,
        message: Option<String>,
        log_file: Option<PathBuf>,
        work: impl Future<Output = Outcome>,
    ) -> ActionResult {
        if !self.begin_job(kind, message, log_file.as_deref()).await {
            return self.refuse_code(ErrorCode::Busy);
        }
        let outcome = work.await;
        self.end_job(kind, outcome).await
    }

    // --- Running the host kit ---

    fn host_tool(&self, workspace: &Path) -> Option<HostTool> {
        self.host().tool.clone().or_else(|| HostTool::find(workspace))
    }

    /// One host kit command; the failure is a detail for the page.
    async fn run_kit(&self, workspace: &Path, args: Vec<String>, log_name: &str) -> Result<CommandOutput, String> {
        if self.embedded() {
            #[cfg(feature = "server-gui")]
            return Ok(self.run_kit_in_process(workspace, args, log_name).await);
            #[cfg(not(feature = "server-gui"))]
            return Err("not_in_this_build".to_owned());
        }
        let tool = self
            .host_tool(workspace)
            .ok_or(crate::platform::hostserver::HostKitError::Missing);
        let result = match tool {
            Ok(tool) => self.host().server.command(&tool, workspace, &args, log_name).await,
            Err(missing) => Err(missing),
        };
        result.map_err(|error| {
            let detail = error.detail();
            log::warn(format!("host: the host kit did not run: {detail}"));
            detail
        })
    }

    /// A host kit command that must succeed, as a job step.
    async fn kit_step(
        &self,
        workspace: &Path,
        args: Vec<String>,
        log_name: &str,
    ) -> Result<CommandOutput, Option<String>> {
        let output = self.run_kit(workspace, args, log_name).await.map_err(Some)?;
        if output.code != 0 {
            return Err(output.failure.clone());
        }
        Ok(output)
    }

    /// Runs `work` with a server this launcher started stopped, and starts it again after.
    async fn while_stopped<T>(&self, workspace: &Path, work: impl Future<Output = T>) -> T {
        let server = &self.host().server;
        let was_running = server.running();
        if was_running {
            server.stop().await;
        }
        let result = work.await;
        if was_running {
            let restarted = match self.server_tool(workspace) {
                Ok(tool) => server.start(&tool, workspace).await.map_err(|error| error.detail()),
                Err(detail) => Err(detail),
            };
            if let Err(detail) = restarted {
                log::warn(format!("host: the server did not start again: {detail}"));
            }
        }
        result
    }

    fn server_tool(&self, workspace: &Path) -> Result<HostTool, String> {
        if self.embedded() {
            // The embedded server never runs it
            return Ok(HostTool {
                program: workspace.join(HOST_BINARY),
                prefix: Vec::new(),
            });
        }
        self.host_tool(workspace)
            .ok_or_else(|| crate::platform::hostserver::HostKitError::Missing.detail())
    }

    fn server_state(&self) -> HostServerState {
        self.host().server.state()
    }

    fn server_runs(&self) -> bool {
        self.host().server.running() || self.server_state() == HostServerState::External
    }

    // --- The kit (§2.5.4) ---

    async fn requirements(&self, toolchain: &Toolchain, workspace: Option<&Path>) -> Vec<Requirement> {
        if let Some(cached) = hold(&self.host().requirements).as_ref()
            && cached.at.elapsed() < REQUIREMENTS_TTL
            && cached.workspace.as_deref() == workspace
        {
            return cached.list.clone();
        }
        let game = {
            let state = self.lock();
            (state.game.problem.is_none(), state.game.version_text.clone())
        };
        let dumper = self.stored().dumper_dll().map(PathBuf::from);
        let inputs = Inputs {
            game,
            toolchain,
            dumper_dll: dumper.as_deref(),
            tailscale: self.platform().find_tailscale().is_some(),
            missing_vc_runtime: self.platform().missing_files().visual_cpp.len(),
        };
        let list = check_requirements_here(&inputs).await;
        *hold(&self.host().requirements) = Some(Cached {
            at: Instant::now(),
            workspace: workspace.map(Path::to_path_buf),
            list: list.clone(),
        });
        list
    }

    fn game_exports(&self) -> Option<PathBuf> {
        self.win64()
            .map(|win64| win64.join(GAME_EXPORTS))
            .filter(|dir| dir.is_dir())
    }

    /// `host_get`.
    pub async fn host_kit(&self) -> HostKit {
        let toolchain = find_toolchain().await;
        let stored = self.stored();
        let workspace = stored.host_workspace().map(PathBuf::from);
        let dump_root = self.host().dump_root.clone();
        let detect = (workspace.clone(), toolchain.clone());
        let mut kit = tokio::task::spawn_blocking(move || detect_host_kit(detect.0.as_deref(), &detect.1, &dump_root))
            .await
            .unwrap_or_else(|_| detect_host_kit(None, &Toolchain::default(), Path::new("")));
        kit.workspace_ok = workspace.as_deref().is_some_and(|dir| self.workspace_valid(dir));
        kit.requirements = self.requirements(&toolchain, workspace.as_deref()).await;
        kit.exports.in_game = self.game_exports().is_some();
        kit.dumper_dll = stored.dumper_dll().map(str::to_owned);
        kit.job = hold(&self.host().job).clone();
        kit.update = hold(&self.host().update).clone();
        kit.project_target = self.project_target().as_deref().map(path_text);
        kit
    }

    async fn emit_kit(&self) {
        let kit = self.host_kit().await;
        self.platform().emit_host_kit(&kit);
    }

    /// `host_choose_workspace`.
    pub async fn host_choose_workspace(&self) -> ActionResult {
        let embedded = self.embedded();
        let start = if embedded {
            Some(self.user_data().join(SERVER_FOLDER))
        } else {
            self.stored().host_workspace().map(PathBuf::from)
        };
        let picked = self
            .platform()
            .choose_folder(self.language(), FolderPurpose::Workspace, start)
            .await;
        let Some(picked) = picked else {
            return ActionResult::err(ErrorCode::Cancelled);
        };
        if embedded
            && !picked.exists()
            && let Err(error) = std::fs::create_dir_all(&picked)
        {
            log::warn(format!(
                "host: could not make {}: {}",
                picked.display(),
                log::describe_error(&error)
            ));
        }
        if !self.workspace_valid(&picked) {
            return ActionResult::Err(self.refuse(self.workspace_failure()));
        }
        self.store_workspace(&picked).await;
        ActionResult::Ok
    }

    /// `host_choose_dumper`.
    pub async fn host_choose_dumper(&self) -> ActionResult {
        let Some(dll) = self.platform().choose_dumper_dll(self.language()).await else {
            return ActionResult::err(ErrorCode::Cancelled);
        };
        let is_dll = dll
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("dll"));
        if !is_dll || !dll.is_file() {
            return self.refuse_with("dumper");
        }
        let text = path_text(&dll);
        self.edit_settings(|map| edit::field(map, "dumperDll", json!(text)))
            .await;
        self.clear_requirements();
        self.emit_kit().await;
        ActionResult::Ok
    }

    /// `host_import_gamedata`.
    pub async fn host_import_gamedata(&self) -> ActionResult {
        if let Err(refused) = self.busy_job() {
            return refused;
        }
        let workspace = match self.need_workspace() {
            Ok(workspace) => workspace,
            Err(refused) => return refused,
        };
        let Some(pack) = self.platform().choose_gamedata_pack(self.language()).await else {
            return ActionResult::err(ErrorCode::Cancelled);
        };
        let name = pack
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let log_file = WorkspacePaths::new(&workspace).launcher_log("gamedata-import");
        let work = async {
            self.kit_step(
                &workspace,
                vec!["gamedata".into(), "--import".into(), path_text(&pack)],
                "gamedata-import",
            )
            .await
            .map(|_| Some(name.clone()))
        };
        self.run_job(HostJobKind::ImportGamedata, Some(name.clone()), Some(log_file), work)
            .await
    }

    /// `host_import_sdk_folder`.
    pub async fn host_import_sdk_folder(&self) -> ActionResult {
        if let Err(refused) = self.busy_job() {
            return refused;
        }
        let workspace = match self.need_workspace() {
            Ok(workspace) => workspace,
            Err(refused) => return refused,
        };
        let start = Some(self.host().dump_root.clone());
        let Some(picked) = self
            .platform()
            .choose_folder(self.language(), FolderPurpose::Sdk, start)
            .await
        else {
            return ActionResult::err(ErrorCode::Cancelled);
        };
        let sdk = if picked.join("SDK.hpp").is_file() {
            picked
        } else if picked.join("CppSDK").join("SDK.hpp").is_file() {
            picked.join("CppSDK")
        } else {
            return self.refuse_with("sdk_folder");
        };
        if let Err(refused) = self.ensure_runtime_source(&workspace).await {
            return refused;
        }
        let name = sdk
            .parent()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.import_sdk(&workspace, sdk, name).await
    }

    /// `host_download_project`.
    pub async fn host_download_project(&self) -> ActionResult {
        self.ensure_host_workspace(false).await
    }

    /// The runtime's C++ source, fetched by the host kit when it is not there yet.
    async fn ensure_runtime_source(&self, workspace: &Path) -> Result<(), ActionResult> {
        let paths = WorkspacePaths::new(workspace);
        let present = || paths.runtime_source.join("dllmain.cpp").is_file();
        if present() {
            return Ok(());
        }
        let work = async {
            self.kit_step(workspace, args(&["upstream"]), "upstream")
                .await
                .map(|_| None)
        };
        let answer = self
            .run_job(HostJobKind::Upstream, None, Some(paths.launcher_log("upstream")), work)
            .await;
        if answer != ActionResult::Ok {
            return Err(answer);
        }
        if !present() {
            return Err(self.refuse_with("upstream_old"));
        }
        log::info("host: the runtime source was fetched");
        Ok(())
    }

    async fn import_sdk(&self, workspace: &Path, dir: PathBuf, name: String) -> ActionResult {
        let runtime = WorkspacePaths::new(workspace).runtime_source;
        let work = async {
            let from = dir.clone();
            match tokio::task::spawn_blocking(move || hostkit::import_sdk(&from, &runtime)).await {
                Ok(Ok(hostkit::SdkImport {
                    function_files: files,
                    repaired_lookups,
                })) => {
                    if repaired_lookups > 0 {
                        log::info(format!(
                            "host: SDK {name}: repaired {repaired_lookups} function lookups that named the class                              instead of the function (Dumper-7 42f37f6)"
                        ));
                    }
                    log::info(format!("host: SDK {name} imported ({files} files)"));
                    Ok(Some(format!("{files} files")))
                }
                Ok(Err(error)) => Err(Some(error.code().to_owned())),
                Err(_) => Err(Some(hostkit::SdkError::Io.code().to_owned())),
            }
        };
        self.run_job(HostJobKind::ImportSdk, Some(name.clone()), None, work)
            .await
    }

    /// `host_action`.
    pub async fn host_action(&self, action: HostAction) -> ActionResult {
        if let Err(refused) = self.busy_job() {
            return refused;
        }
        match action {
            HostAction::InjectDumper => return self.inject_dumper().await,
            HostAction::InjectExporter => return self.inject_exporter().await,
            _ => {}
        }
        let workspace = match self.need_workspace() {
            Ok(workspace) => workspace,
            Err(refused) => return refused,
        };
        let needs_source = matches!(action, HostAction::ImportSdk | HostAction::BuildLoader)
            || (!self.embedded() && matches!(action, HostAction::SyntheticData | HostAction::GenerateGamedata));
        if needs_source && let Err(refused) = self.ensure_runtime_source(&workspace).await {
            return refused;
        }
        let paths = WorkspacePaths::new(&workspace);
        match action {
            HostAction::ImportSdk => {
                let Some(newest) = find_dumps(&self.host().dump_root).into_iter().next() else {
                    return self.refuse_with("no_dump");
                };
                self.import_sdk(&workspace, PathBuf::from(newest.dir), newest.name)
                    .await
            }
            HostAction::BuildRuntime => self.build_runtime(&paths).await,
            HostAction::BuildExporter => self.build_exporter(&paths).await,
            HostAction::BuildLoader => {
                let toolchain = find_toolchain().await;
                let Some(cargo) = toolchain.cargo else {
                    return self.refuse_with("cargo");
                };
                let log_file = paths.logs.join("build_loader.log");
                let work = async {
                    let ran = run_logged(&cargo, &args(&["build", "--release"]), &paths.loader_dir, &log_file).await;
                    if ran.code == 0 {
                        Ok(ran.last_line)
                    } else {
                        Err(ran.last_line)
                    }
                };
                self.run_job(HostJobKind::BuildLoader, None, Some(log_file.clone()), work)
                    .await
            }
            HostAction::CopyExports => {
                let Some(source) = self.game_exports() else {
                    return self.refuse_with("no_exports");
                };
                let target = paths.exports.clone();
                let work = async move {
                    match tokio::task::spawn_blocking(move || copy_tree(&source, &target)).await {
                        Ok(Ok(files)) => Ok(Some(format!("{files} files"))),
                        Ok(Err(error)) => Err(Some(log::describe_error(&error))),
                        Err(_) => Err(None),
                    }
                };
                self.run_job(HostJobKind::CopyExports, None, None, work).await
            }
            HostAction::SyntheticData => {
                let work = async {
                    self.kit_step(&workspace, args(&["gamedata", "--examples"]), "synthetic-data")
                        .await
                        .map(|_| None)
                };
                let log_file = paths.launcher_log("synthetic-data");
                self.run_job(HostJobKind::SyntheticData, None, Some(log_file), work)
                    .await
            }
            HostAction::PublishRuntime => {
                let work = async {
                    let output = self
                        .kit_step(&workspace, args(&["publish-runtime"]), "publish-runtime")
                        .await?;
                    let version = published_version(&output.lines);
                    log::info(format!("host: runtime {} published", version.as_deref().unwrap_or("?")));
                    Ok(version)
                };
                let log_file = paths.launcher_log("publish-runtime");
                self.run_job(HostJobKind::PublishRuntime, None, Some(log_file), work)
                    .await
            }
            HostAction::GenerateGamedata => {
                let work = async {
                    if let Some(source) = self.game_exports() {
                        let target = paths.exports.clone();
                        let copied = tokio::task::spawn_blocking(move || copy_tree(&source, &target)).await;
                        match copied {
                            Ok(Ok(files)) => self
                                .host()
                                .server
                                .note(&format!("Copied {files} exported files from the game.")),
                            Ok(Err(error)) => return Err(Some(log::describe_error(&error))),
                            Err(_) => return Err(None),
                        }
                    } else if !paths.exports.is_dir() {
                        return Err(Some("no_exports".to_owned()));
                    }
                    self.kit_step(&workspace, args(&["gamedata"]), "gamedata").await?;
                    Ok(self.server_runs().then(|| "restart".to_owned()))
                };
                let log_file = paths.launcher_log("gamedata");
                self.run_job(HostJobKind::GenerateGamedata, None, Some(log_file), work)
                    .await
            }
            HostAction::ExportGamedata => {
                let work = async {
                    let output = self
                        .kit_step(&workspace, args(&["gamedata", "--export"]), "gamedata-export")
                        .await?;
                    Ok(pack_written(&output.lines))
                };
                let log_file = paths.launcher_log("gamedata-export");
                self.run_job(HostJobKind::ExportGamedata, None, Some(log_file), work)
                    .await
            }
            HostAction::InjectDumper | HostAction::InjectExporter => ActionResult::err(ErrorCode::Unknown),
        }
    }

    /// The CatalogExporter: the MIT crate, built against the workspace's SDK dump (spec-catalog-exporter 9). No
    /// export root is compiled in, so it writes beside the game and reads the flags the export session writes there.
    async fn build_exporter(&self, paths: &WorkspacePaths) -> ActionResult {
        let toolchain = find_toolchain().await;
        let Some(cargo) = toolchain.cargo else {
            return self.refuse_with("cargo");
        };
        // cargo links the DLL with the MSVC linker that comes with the C++ tools
        if toolchain.msbuild.is_none() {
            return self.refuse_with("msbuild");
        }
        let log_file = paths.logs.join("build_exporter.log");
        let env = [
            ("DAINTY_SDK_DIR", Some(paths.runtime_source.as_path())),
            ("RUSTIC_ISLES_EXPORT_ROOT", None),
        ];
        let work = async {
            let ran = run_logged_env(&cargo, &exporter_build_args(), &paths.exporter_crate, &log_file, &env).await;
            if ran.code == 0 {
                Ok(ran.last_line)
            } else {
                Err(ran.last_line)
            }
        };
        self.run_job(HostJobKind::BuildExporter, None, Some(log_file.clone()), work)
            .await
    }

    async fn build_runtime(&self, paths: &WorkspacePaths) -> ActionResult {
        let toolchain = find_toolchain().await;
        let Some(msbuild) = toolchain.msbuild.clone() else {
            return self.refuse_with("msbuild");
        };
        let (kind, project, log_file) = (
            HostJobKind::BuildRuntime,
            paths.runtime_solution.clone(),
            paths.logs.join("build_runtime.log"),
        );
        let build_args = msbuild_args(&project, platform_toolset(toolchain.major_version));
        let folder = project.parent().map_or_else(|| paths.root.clone(), Path::to_path_buf);
        let work = async {
            let ran = run_logged(&msbuild, &build_args, &folder, &log_file).await;
            if ran.code == 0 {
                Ok(ran.last_line)
            } else {
                Err(ran.last_line)
            }
        };
        self.run_job(kind, None, Some(log_file.clone()), work).await
    }

    // --- Loader sessions ---

    /// What Dumper-7 and CatalogExporter both need: Windows, an idle launcher and the right game.
    fn session_game(&self) -> Result<PathBuf, ActionResult> {
        if !cfg!(windows) {
            return Err(self.refuse_with("windows_only"));
        }
        let (occupied, problem, root, version_ok) = {
            let state = self.lock();
            (
                state.busy || state.shutting_down,
                state.game.problem,
                state.game.root.clone(),
                state.game.version_ok,
            )
        };
        if occupied {
            return Err(self.refuse_code(ErrorCode::Busy));
        }
        if self.shared.game.is_running() {
            return Err(self.refuse_with("game_running"));
        }
        let win64 = self.win64();
        let Some(win64) =
            win64.filter(|_| root.is_some() && !matches!(problem, Some(GameProblem::NotSet | GameProblem::NotFound)))
        else {
            return Err(self.refuse_code(ErrorCode::GameFolderNotFound));
        };
        if problem == Some(GameProblem::WrongVersion) || !version_ok {
            return Err(self.refuse_code(ErrorCode::GameWrongVersion));
        }
        Ok(win64)
    }

    async fn inject_dumper(&self) -> ActionResult {
        let win64 = match self.session_game() {
            Ok(win64) => win64,
            Err(refused) => return refused,
        };
        let Some(dll) = self
            .stored()
            .dumper_dll()
            .map(PathBuf::from)
            .filter(|dll| dll.is_file())
        else {
            return self.refuse_with("dumper");
        };
        if !self.platform().running_clients().await.is_empty() {
            return self.refuse_code(ErrorCode::AlreadyRunning);
        }
        let session = match self.begin_loader_session(SessionKind::Dump, &win64, &dll) {
            Ok(session) => session,
            Err(refused) => return ActionResult::Err(refused),
        };
        if !self
            .begin_job(HostJobKind::Inject, Some("Dumper-7".to_owned()), None)
            .await
        {
            self.end_loader_session(session).await;
            return self.refuse_code(ErrorCode::Busy);
        }
        {
            let mut state = self.lock();
            state.host.dumping = true;
            state.host.loader_session = Some(session);
        }
        // Dumper-7 reads the game as shipped, so no runtime setting reaches it
        let env = self
            .base_environment()
            .into_iter()
            .filter(|(name, _)| {
                let upper = name.to_ascii_uppercase();
                !STRIPPED_PREFIXES.iter().any(|prefix| upper.starts_with(prefix))
            })
            .collect();
        let plan = LaunchPlan::new(&win64, None, &args(&DUMPER_ARGS), env);
        log::info(format!("host: starting {} for Dumper-7", plan.describe()));
        let me = self.clone();
        let outcome = match self.shared.game.start(&plan, move |event| me.on_game_event(event)) {
            Ok(_) => Ok(Some("Dumper-7".to_owned())),
            Err(error) => {
                log::warn(format!("host: the game did not start for Dumper-7: {error:?}"));
                self.lock().host.dumping = false;
                self.end_current_loader_session().await;
                Err(None)
            }
        };
        self.end_job(HostJobKind::Inject, outcome).await
    }

    async fn inject_exporter(&self) -> ActionResult {
        let win64 = match self.session_game() {
            Ok(win64) => win64,
            Err(refused) => return refused,
        };
        let workspace = match self.need_workspace() {
            Ok(workspace) => workspace,
            Err(refused) => return refused,
        };
        let dll = WorkspacePaths::new(&workspace).exporter_dll;
        if !dll.is_file() {
            return self.refuse_with("exporter");
        }
        if let Err(error) = std::fs::write(win64.join(EXPORT_FLAGS_FILE), EXPORT_FLAGS) {
            return self.refuse_with(&format!("export_flags:{}", log::describe_error(&error)));
        }
        if !self
            .begin_job(HostJobKind::Inject, Some("CatalogExporter".to_owned()), None)
            .await
        {
            return self.refuse_code(ErrorCode::Busy);
        }
        let answer = self.play_with(Some(&dll)).await;
        if answer == ActionResult::Ok {
            log::info("host: the game started with CatalogExporter");
            return self
                .end_job(HostJobKind::Inject, Ok(Some("CatalogExporter".to_owned())))
                .await;
        }
        *hold(&self.host().job) = None;
        self.emit_kit().await;
        answer
    }

    // --- Folders ---

    /// `host_open`.
    pub fn host_open(&self, folder: HostFolder) -> ActionResult {
        let dir = if folder == HostFolder::Dumps {
            self.host().dump_root.clone()
        } else {
            let workspace = match self.need_workspace() {
                Ok(workspace) => workspace,
                Err(refused) => return refused,
            };
            let paths = WorkspacePaths::new(&workspace);
            match folder {
                HostFolder::RuntimeSource => paths.runtime_source,
                HostFolder::GameData => paths
                    .backend_game_data
                    .parent()
                    .map_or(paths.backend_game_data.clone(), Path::to_path_buf),
                HostFolder::RuntimePublish => paths.publish,
                HostFolder::HostLogs => paths.logs,
                HostFolder::GamedataPacks => paths.gamedata_packs,
                HostFolder::Workspace | HostFolder::Dumps => workspace,
            }
        };
        if !dir.is_dir() {
            return self.refuse_with("folder");
        }
        match self.platform().open_folder(&dir) {
            Ok(true) => ActionResult::Ok,
            Ok(false) => self.refuse_with("folder"),
            Err(error) => {
                log::warn(format!(
                    "host: could not open a folder: {}",
                    log::describe_error(&error)
                ));
                self.refuse_with("folder")
            }
        }
    }

    // --- The server ---

    /// `host_server_get`: the view now, while whether an outside server answers is looked at again.
    #[must_use]
    pub fn host_server_view(&self) -> HostServerView {
        if tokio::runtime::Handle::try_current().is_ok() {
            let server = self.host().server.clone();
            self.spawn(async move { server.refresh().await });
        }
        self.host().server.view()
    }

    /// `host_server_start`.
    pub async fn host_server_start(&self) -> ActionResult {
        let workspace = match self.need_workspace() {
            Ok(workspace) => workspace,
            Err(refused) => return refused,
        };
        if let Err(refused) = self.busy_job() {
            return refused;
        }
        let tool = match self.server_tool(&workspace) {
            Ok(tool) => tool,
            Err(detail) => return self.refuse_with(&detail),
        };
        if !read_server_config(&workspace).configured {
            return self.refuse_with("not_configured");
        }
        let paths = WorkspacePaths::new(&workspace);
        let missing = detect_game_data(&paths.backend_game_data, &BACKEND_GAME_DATA) == GameDataState::Missing
            || detect_game_data(&paths.director_game_data, &DIRECTOR_GAME_DATA) == GameDataState::Missing;
        if missing {
            return self.refuse_with("gamedata_missing");
        }
        if let Err(error) = self.host().server.start(&tool, &workspace).await {
            let detail = error.detail();
            log::warn(format!("host: the server did not start: {detail}"));
            return self.refuse_with(&detail);
        }
        log::info("host: server started");
        ActionResult::Ok
    }

    /// `host_server_restart`.
    pub async fn host_server_restart(&self, name: &str) -> ActionResult {
        match self.host().server.restart(name).await {
            Ok(()) => {
                log::info(format!("host: {name} restarted"));
                ActionResult::Ok
            }
            Err(why) => self.refuse_with(&why),
        }
    }

    /// `host_server_stop`.
    pub async fn host_server_stop(&self) -> ActionResult {
        self.host().server.stop().await;
        log::info("host: server stopped");
        ActionResult::Ok
    }

    /// `host_setup`.
    pub async fn host_setup(&self, request: HostSetupRequest) -> ActionResult {
        let workspace = match self.need_workspace() {
            Ok(workspace) => workspace,
            Err(refused) => return refused,
        };
        if let Err(refused) = self.busy_job() {
            return refused;
        }
        if self.server_runs() {
            return self.refuse_with("server_running");
        }
        let public = request.mode == HostServerMode::Public;
        let mut setup = vec![
            "setup".to_owned(),
            "--mode".to_owned(),
            (if public { "public" } else { "private" }).to_owned(),
            "--name".to_owned(),
            request.name.clone(),
        ];
        if public && let Some(host) = &request.public_host {
            setup.extend(["--host".to_owned(), host.clone()]);
        }
        if let Some(root) = self.lock().game.root.clone() {
            setup.extend(["--game".to_owned(), root]);
        }
        let log_file = WorkspacePaths::new(&workspace).launcher_log("setup");
        let work = async {
            let ran = self.kit_step(&workspace, setup, "setup").await;
            self.clear_requirements();
            ran.map(|_| Some(request.name.clone()))
        };
        self.run_job(HostJobKind::Setup, None, Some(log_file), work).await
    }

    /// `host_create_invite`; the link is a secret, so only the server view carries it.
    pub async fn host_create_invite(&self, for_name: Option<String>) -> ActionResult {
        let workspace = match self.need_workspace() {
            Ok(workspace) => workspace,
            Err(refused) => return refused,
        };
        if !matches!(
            self.server_state(),
            HostServerState::Running | HostServerState::External
        ) {
            return self.refuse_with("server_not_running");
        }
        let mut invite = args(&["invite", "--json"]);
        if let Some(name) = &for_name {
            invite.extend(["--for".to_owned(), name.clone()]);
        }
        let output = match self.run_kit(&workspace, invite, "invite").await {
            Ok(output) => output,
            Err(detail) => return self.refuse_with(&detail),
        };
        if output.code != 0 {
            return self.refuse_with(output.failure.as_deref().unwrap_or("invite"));
        }
        let Some(line) = invite_line(&output.lines) else {
            return self.refuse_with("invite");
        };
        self.host().server.set_invite(Some(HostInvite {
            link: line.link,
            for_name: line.for_name,
        }));
        log::info(if for_name.is_some() {
            "host: invite created with a note"
        } else {
            "host: invite created"
        });
        ActionResult::Ok
    }

    fn set_update_view(&self, view: HostUpdateView) {
        *hold(&self.host().update) = Some(view);
    }

    /// `host_update`.
    pub async fn host_update(&self) -> ActionResult {
        let workspace = match self.need_workspace() {
            Ok(workspace) => workspace,
            Err(refused) => return refused,
        };
        if let Err(refused) = self.busy_job() {
            return refused;
        }
        if self.server_state() == HostServerState::External {
            return self.refuse_with("server_external");
        }
        let configured = read_server_config(&workspace).configured;
        let log_file = WorkspacePaths::new(&workspace).launcher_log("update");
        let work = async {
            let check = self
                .run_kit(&workspace, args(&["update", "--check", "--json"]), "update")
                .await
                .map_err(Some)?;
            let json = last_json(&check.lines);
            let Some(behind) = json.as_ref().and_then(update_check).filter(|_| check.code == 0) else {
                return Err(Some(update_failure(json.as_ref(), check.failure.as_deref())));
            };
            if behind.behind == 0 {
                self.set_update_view(HostUpdateView {
                    state: HostUpdateState::UpToDate,
                    head: behind.head.clone(),
                    files: 0,
                    runtime_changed: false,
                });
                return Ok(Some(behind.head));
            }
            self.while_stopped(&workspace, async {
                let ran = self
                    .run_kit(&workspace, args(&["update", "--json", "--no-setup"]), "update")
                    .await
                    .map_err(Some)?;
                let json = last_json(&ran.lines);
                let Some(done) = json.as_ref().and_then(update_done).filter(|_| ran.code == 0) else {
                    return Err(Some(update_failure(json.as_ref(), ran.failure.as_deref())));
                };
                log::info(format!("host: updated to {} ({} files)", done.to, done.files));
                self.set_update_view(HostUpdateView {
                    state: HostUpdateState::Updated,
                    head: done.to.clone(),
                    files: done.files,
                    runtime_changed: done.runtime_changed,
                });
                if configured {
                    self.kit_step(&workspace, args(&["setup"]), "setup").await?;
                }
                Ok(Some(done.to))
            })
            .await
        };
        self.run_job(HostJobKind::Update, None, Some(log_file), work).await
    }

    /// `host_set_multipliers`; none turns them off.
    pub async fn host_set_multipliers(&self, multipliers: Option<HostMultipliers>) -> ActionResult {
        let workspace = match self.need_workspace() {
            Ok(workspace) => workspace,
            Err(refused) => return refused,
        };
        if let Err(refused) = self.busy_job() {
            return refused;
        }
        if self.server_state() == HostServerState::External {
            return self.refuse_with("server_external");
        }
        let config = read_server_config(&workspace);
        if !config.configured {
            return self.refuse_with("not_configured");
        }
        // Public servers keep the game's own rates
        if config.mode == Some(HostServerMode::Public) {
            return self.refuse_with("multipliers_public");
        }
        let change = match multipliers {
            Some(m) => vec![
                "multipliers".to_owned(),
                "--xp".to_owned(),
                m.xp.to_string(),
                "--rewards".to_owned(),
                m.rewards.to_string(),
            ],
            None => args(&["multipliers", "--off"]),
        };
        let log_file = WorkspacePaths::new(&workspace).launcher_log("multipliers");
        let work = self.while_stopped(&workspace, async {
            self.kit_step(&workspace, change, "multipliers").await?;
            log::info(match multipliers {
                Some(m) => format!("host: multipliers set to {} XP, {} rewards", m.xp, m.rewards),
                None => "host: multipliers off".to_owned(),
            });
            Ok(None)
        });
        self.run_job(HostJobKind::Multipliers, None, Some(log_file), work).await
    }
}
