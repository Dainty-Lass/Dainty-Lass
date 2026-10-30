//! The Host page (Reforged's `controller.ts` 1765–2351): the host kit's workspace, requirements, builds,
//! game data and the server a host runs from the launcher, for the planned layout only (RD23), on
//! `platform::hostkit`, `platform::hostserver`, `platform::requirements` and
//! `platform::loader_session`, driving `reforged-host`.
//!
//! Dumper-7 and CatalogExporter are loaded by the runtime's own loader through its `mystic_loader.ini`
//! (RD9): a session places the DLL and the ini, starts the game, and `mod.rs` ends the session when that
//! game exits. Nothing here touches another process.
//!
//! Not ported (RD23): the Node kit's paths (`npm run upstream`, `npm install`), git's clone and fast
//! forward. The project folder is a server release (or a checkout with `reforged-host` built); when the
//! one beside the game is not there, the page says so.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::{Controller, State, lock};
use crate::bridge::types::{
    ActionResult, ErrorCode, GameDataState, GameProblem, HostAction, HostFolder, HostInvite, HostJob, HostJobKind,
    HostJobState, HostKit, HostMultipliers, HostServerMode, HostServerState, HostServerView, HostSetupRequest,
    HostUpdateState, HostUpdateView, NoticeCode, Requirement,
};
use crate::platform::dialogs::FolderPurpose;
use crate::platform::hostkit::parse::{
    invite_line, last_json, pack_written, platform_toolset, published_version, update_check, update_done,
    update_failure,
};
use crate::platform::hostkit::{
    DUMPER_OUTPUT_ROOT, EXPORT_FLAGS, WorkspacePaths, copy_tree, detect_host_kit, find_dumps, find_toolchain,
    import_sdk, is_workspace, msbuild_args, read_server_config, run_logged,
};
pub use crate::platform::hostserver::Probe;
use crate::platform::hostserver::{CommandOutput, HostKitError, HostServer, HostTool, STOP_TIMEOUT};
use crate::platform::loader_session::{self, SessionKind};
use crate::platform::log;
use crate::platform::requirements::{Inputs, check_requirements_here};
use crate::platform::settings::Platform as HostPlatform;

/// Where a downloaded project goes beside the game folder (Reforged's `PROJECT_FOLDER_NAME`).
const PROJECT_FOLDER_NAME: &str = "Reforged";
/// The requirements are looked at again after this long (vswhere and friends are slow).
const REQUIREMENTS_TTL: Duration = Duration::from_secs(15);
/// What the Dumper-7 game starts with: no anti-cheat, no Epic or EOS sign-in.
const DUMP_ARGS: [&str; 3] = ["-NoEAC", "-EpicPortal", "-DisableEOS"];

struct CachedRequirements {
    at: Instant,
    workspace: Option<String>,
    list: Vec<Requirement>,
}

/// The Host page's own state, beside the controller's (`State` keeps what `mod.rs` acts on:
/// `host_server_up`, `dumping` and `loader_session`).
pub struct HostState {
    server: HostServer,
    job: Mutex<Option<HostJob>>,
    update: Mutex<Option<HostUpdateView>>,
    requirements: Mutex<Option<CachedRequirements>>,
    /// Where Dumper-7 writes its SDKs (`C:\Dumper-7`; tests: a folder of their own)
    dump_root: PathBuf,
}

impl Default for HostState {
    fn default() -> Self {
        Self {
            server: HostServer::default(),
            job: Mutex::new(None),
            update: Mutex::new(None),
            requirements: Mutex::new(None),
            dump_root: PathBuf::from(DUMPER_OUTPUT_ROOT),
        }
    }
}

impl HostState {
    /// The tests' seams: whether a server answers on port 61000, and where the dumps are.
    pub fn with(probe: Option<Probe>, dump_root: Option<PathBuf>) -> Self {
        let mut state = Self::default();
        if let Some(probe) = probe {
            state.server = HostServer::new(probe, STOP_TIMEOUT);
        }
        if let Some(root) = dump_root {
            state.dump_root = root;
        }
        state
    }
}

/// The steps that work in Mystic Paradox's source (`NEEDS_UPSTREAM`).
fn needs_upstream(action: HostAction, embedded: bool) -> bool {
    if embedded {
        matches!(action, HostAction::ImportSdk | HostAction::BuildLoader)
    } else {
        matches!(
            action,
            HostAction::ImportSdk | HostAction::BuildLoader | HostAction::SyntheticData | HostAction::GenerateGamedata
        )
    }
}

fn kind_name(kind: HostJobKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn is_file(path: &Path) -> bool {
    path.metadata().is_ok_and(|m| m.is_file())
}

fn is_dir(path: &Path) -> bool {
    path.metadata().is_ok_and(|m| m.is_dir())
}

fn busy() -> ActionResult {
    ActionResult::err(ErrorCode::Busy)
}

impl Controller {
    // ------------------------------------------------------------------ seams for mod.rs

    /// Wires the Host page's server to the page and to `State::host_server_up`.
    pub(super) fn host_attach(&self) {
        let me = self.me.clone();
        self.host.server.set_listener(Arc::new(move |view: HostServerView| {
            let Some(this) = me.upgrade() else { return };
            this.st().host_server_up = matches!(
                view.state,
                HostServerState::Starting
                    | HostServerState::Running
                    | HostServerState::Stopping
                    | HostServerState::External
            );
            this.p.emit_host_server(&view);
        }));
    }

    /// The server the Host page started stops with the launcher (the page says so).
    pub(super) async fn host_shutdown(&self) {
        if self.host.server.running() {
            self.host.server.stop().await;
            log::info("host: server stopped with the launcher");
        }
    }

    /// `ensureHostWorkspace`: the project folder a host's server runs from (or, in DaintyLass Manager,
    /// the server's data folder under app data). With Host tools on and none chosen yet, the one beside
    /// the game is taken when it is there. `auto` never shows an error.
    pub(super) async fn ensure_host_workspace(&self, auto: bool) -> ActionResult {
        if self.host.server.has_embedded() {
            if !auto {
                return ActionResult::err(ErrorCode::NotInThisBuild);
            }
            if self.workspace().is_some() {
                return ActionResult::Ok;
            }
            let target = self.default_server_data_dir();
            if let Err(error) = std::fs::create_dir_all(&target) {
                log::warn(&format!(
                    "host: could not create default server data folder {}: {}",
                    target.display(),
                    log::describe_error(&error)
                ));
                return ActionResult::Ok;
            }
            let text = path_text(&target);
            self.update_settings(|map| super::stored::set_text(map, "hostWorkspace", Some(&text)))
                .await;
            log::info(&format!("host: server data folder {text}"));
            self.emit_host_kit();
            self.publish();
            return ActionResult::Ok;
        }
        if auto && !self.s().host_tools {
            return ActionResult::Ok;
        }
        if self.workspace().is_some() {
            return ActionResult::Ok;
        }
        if self.job_running() {
            return busy();
        }
        let quiet = |detail: &str| {
            if auto { ActionResult::Ok } else { self.host_fail(detail) }
        };
        let Some(target) = self.project_target() else {
            return quiet("project_no_game");
        };
        if !self.valid_workspace(&target) {
            return quiet(if target.exists() {
                "project_folder_taken"
            } else {
                "project_missing"
            });
        }
        let text = path_text(&target);
        self.update_settings(|map| super::stored::set_text(map, "hostWorkspace", Some(&text)))
            .await;
        log::info(&format!("host: project folder {text}"));
        self.emit_host_kit();
        self.publish();
        ActionResult::Ok
    }

    // ------------------------------------------------------------------ helpers

    pub(super) fn default_server_data_dir(&self) -> PathBuf {
        self.c.user_data.join("Server")
    }

    pub(super) fn valid_workspace(&self, ws: &Path) -> bool {
        if self.host.server.has_embedded() {
            is_dir(ws)
        } else {
            is_workspace(ws)
        }
    }

    fn workspace_detail(&self) -> &'static str {
        if self.host.server.has_embedded() {
            "workspace_mgr"
        } else {
            "workspace"
        }
    }

    fn workspace_fail(&self) -> ActionResult {
        self.host_fail(self.workspace_detail())
    }

    fn host_fail(&self, detail: &str) -> ActionResult {
        ActionResult::Err(self.fail(ErrorCode::HostFailed, Some(detail.to_owned())))
    }

    fn host_kit_failure(&self, error: &HostKitError) -> ActionResult {
        log::error(&format!("host kit: {}", error.detail()));
        self.host_fail(&error.detail())
    }

    /// The workspace the server GUI hosts, for the history commands (DH3b).
    #[cfg_attr(
        not(feature = "server-gui"),
        allow(dead_code, reason = "the history commands read it")
    )]
    pub fn host_workspace(&self) -> Option<PathBuf> {
        self.workspace()
    }

    /// The workspace in the settings when it still is one.
    pub(super) fn workspace(&self) -> Option<PathBuf> {
        self.s()
            .host_workspace
            .map(PathBuf::from)
            .filter(|ws| self.valid_workspace(ws))
    }

    fn job_running(&self) -> bool {
        lock(&self.host.job)
            .as_ref()
            .is_some_and(|job| job.state == HostJobState::Running)
    }

    fn set_job(&self, job: Option<HostJob>) {
        *lock(&self.host.job) = job;
        self.emit_host_kit();
    }

    fn start_job(&self, kind: HostJobKind, message: Option<String>, log_file: Option<&Path>) {
        self.set_job(Some(HostJob {
            kind,
            state: HostJobState::Running,
            message,
            log_file: log_file.map(path_text),
        }));
    }

    fn finish_job(
        &self,
        kind: HostJobKind,
        ok: bool,
        message: Option<String>,
        log_file: Option<&Path>,
    ) -> ActionResult {
        self.set_job(Some(HostJob {
            kind,
            state: if ok { HostJobState::Done } else { HostJobState::Failed },
            message: message.clone(),
            log_file: log_file.map(path_text),
        }));
        if ok {
            self.st().notice = Some(if kind == HostJobKind::ImportSdk {
                NoticeCode::SdkImported
            } else {
                NoticeCode::HostDone
            });
            self.publish();
            return ActionResult::Ok;
        }
        let detail = message.unwrap_or_else(|| kind_name(kind));
        ActionResult::Err(self.fail(ErrorCode::HostFailed, Some(detail)))
    }

    /// The page hears of the kit after a change (in the background: the toolchain check is slow).
    fn emit_host_kit(&self) {
        let Some(this) = self.me.upgrade() else { return };
        self.rt.spawn(async move {
            let kit = this.host_kit().await;
            this.p.emit_host_kit(&kit);
        });
    }

    /// Where the project folder goes: beside the game's folder (`D:\Games\Dauntless` ->
    /// `D:\Games\Reforged`). None without a game folder.
    fn project_target(&self) -> Option<PathBuf> {
        let st = self.st();
        if matches!(st.game.problem, Some(GameProblem::NotFound | GameProblem::NotSet)) {
            return None;
        }
        let root = PathBuf::from(st.game.root.as_deref()?);
        root.parent().map(|parent| parent.join(PROJECT_FOLDER_NAME))
    }

    fn game_ok(st: &State) -> bool {
        st.game.root.is_some()
            && !matches!(
                st.game.problem,
                Some(GameProblem::NotFound | GameProblem::NotSet | GameProblem::WrongVersion)
            )
    }

    fn tool(&self, ws: &Path) -> Result<HostTool, ActionResult> {
        if self.host.server.has_embedded() {
            return Ok(HostTool::find(ws).unwrap_or_else(|| HostTool {
                program: PathBuf::new(),
                prefix: Vec::new(),
            }));
        }
        HostTool::find(ws).ok_or_else(|| self.host_kit_failure(&HostKitError::Missing))
    }

    /// A one-shot `reforged-host` command in the workspace (in-process when the server GUI runs embedded).
    async fn run_host(&self, ws: &Path, args: &[&str], log_name: &str) -> Result<CommandOutput, ActionResult> {
        #[cfg(feature = "server-gui")]
        if self.host.server.has_embedded() {
            return Ok(self.run_host_in_process(ws, args, log_name).await);
        }
        let tool = self.tool(ws)?;
        let args: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
        self.host
            .server
            .command(&tool, ws, &args, log_name)
            .await
            .map_err(|error| self.host_kit_failure(&error))
    }

    /// A server this launcher runs is stopped for a change and started again after it; one started in a
    /// terminal (`external`) cannot be stopped from here, so the callers refuse it first.
    async fn while_stopped<T>(&self, ws: &Path, work: impl Future<Output = T>) -> T {
        let was_running = self.host.server.running();
        if was_running {
            self.host.server.stop().await;
        }
        let out = work.await;
        if was_running {
            match self.tool(ws) {
                Ok(tool) => {
                    if let Err(error) = self.host.server.start(&tool, ws).await {
                        log::warn(&format!("host: server not started again: {}", error.detail()));
                    }
                }
                Err(_) => log::warn("host: server not started again: no reforged-host"),
            }
        }
        out
    }

    // ------------------------------------------------------------------ the page's commands

    /// `hostGet`: the requirements are looked at afresh after 15 s.
    pub async fn host_kit(&self) -> HostKit {
        let toolchain = find_toolchain().await;
        let s = self.s();
        let workspace = s.host_workspace.clone();
        let mut kit = detect_host_kit(workspace.as_deref().map(Path::new), &toolchain, &self.host.dump_root);
        if let Some(ws) = workspace.as_deref()
            && self.valid_workspace(Path::new(ws))
        {
            kit.workspace_ok = true;
        }
        let cached = {
            let cache = lock(&self.host.requirements);
            cache
                .as_ref()
                .filter(|c| c.workspace == workspace && c.at.elapsed() < REQUIREMENTS_TTL)
                .map(|c| c.list.clone())
        };
        kit.requirements = match cached {
            Some(list) => list,
            None => {
                let (game_ok, version) = {
                    let st = self.st();
                    (Self::game_ok(&st), st.game.version_text.clone())
                };
                let dumper = s.dumper_dll.as_deref().map(PathBuf::from);
                let inputs = Inputs {
                    game: (game_ok, version),
                    toolchain: &toolchain,
                    dumper_dll: dumper.as_deref(),
                    tailscale: self.p.find_tailscale(),
                    missing_vc_runtime: self.p.missing_runtimes().0.len(),
                };
                let list = check_requirements_here(&inputs).await;
                *lock(&self.host.requirements) = Some(CachedRequirements {
                    at: Instant::now(),
                    workspace,
                    list: list.clone(),
                });
                list
            }
        };
        let win64 = Self::win64_dir_of(&self.st());
        kit.exports.in_game = win64.is_some_and(|dir| is_dir(&dir.join("Items_Analysis")));
        kit.dumper_dll = s.dumper_dll;
        kit.job = lock(&self.host.job).clone();
        kit.update = lock(&self.host.update).clone();
        kit.project_target = self.project_target().map(|p| path_text(&p));
        kit
    }

    pub async fn host_choose_workspace(&self) -> ActionResult {
        let start = self.s().host_workspace.map(PathBuf::from).unwrap_or_else(|| {
            if self.host.server.has_embedded() {
                self.default_server_data_dir()
            } else {
                PathBuf::new()
            }
        });
        let Some(picked) = self
            .p
            .choose_folder(self.language(), FolderPurpose::Workspace, start)
            .await
        else {
            return ActionResult::err(ErrorCode::Cancelled);
        };
        if self.host.server.has_embedded() && !picked.exists() {
            let _ = std::fs::create_dir_all(&picked);
        }
        if !self.valid_workspace(&picked) {
            return self.workspace_fail();
        }
        let text = path_text(&picked);
        self.update_settings(|map| super::stored::set_text(map, "hostWorkspace", Some(&text)))
            .await;
        *lock(&self.host.requirements) = None;
        self.emit_host_kit();
        self.publish();
        ActionResult::Ok
    }

    pub async fn host_choose_dumper(&self) -> ActionResult {
        let Some(picked) = self.p.choose_dll().await else {
            return ActionResult::err(ErrorCode::Cancelled);
        };
        let dll = picked.extension().is_some_and(|e| e.eq_ignore_ascii_case("dll"));
        if !dll || !is_file(&picked) {
            return self.host_fail("dumper");
        }
        let text = path_text(&picked);
        self.update_settings(|map| super::stored::set_text(map, "dumperDll", Some(&text)))
            .await;
        *lock(&self.host.requirements) = None;
        self.emit_host_kit();
        ActionResult::Ok
    }

    /// A game data pack the host exported earlier (`reforged-host gamedata --import`).
    pub async fn host_import_gamedata(&self) -> ActionResult {
        if self.job_running() {
            return busy();
        }
        let Some(ws) = self.workspace() else {
            return self.workspace_fail();
        };
        let Some(file) = self.p.choose_pack().await else {
            return ActionResult::err(ErrorCode::Cancelled);
        };
        let name = file_name(&file);
        let log_file = WorkspacePaths::new(&ws).launcher_log("gamedata-import");
        self.start_job(HostJobKind::ImportGamedata, Some(name.clone()), Some(&log_file));
        let file_arg = path_text(&file);
        let r = match self
            .run_host(&ws, &["gamedata", "--import", &file_arg], "gamedata-import")
            .await
        {
            Ok(r) => r,
            Err(failed) => {
                self.set_job(None);
                return failed;
            }
        };
        if r.code == 0 {
            log::info(&format!("host: game data pack {name} imported"));
        }
        let message = if r.code == 0 { Some(name) } else { r.failure };
        self.finish_job(HostJobKind::ImportGamedata, r.code == 0, message, Some(&log_file))
    }

    /// A Dumper-7 output that is not under `C:\Dumper-7`: the CppSDK folder itself, or the folder that
    /// holds it.
    pub async fn host_import_sdk_folder(&self) -> ActionResult {
        if self.job_running() {
            return busy();
        }
        let Some(ws) = self.workspace() else {
            return self.workspace_fail();
        };
        let Some(picked) = self
            .p
            .choose_folder(self.language(), FolderPurpose::Sdk, self.host.dump_root.clone())
            .await
        else {
            return ActionResult::err(ErrorCode::Cancelled);
        };
        let dir = if is_file(&picked.join("SDK.hpp")) {
            picked
        } else if is_file(&picked.join("CppSDK").join("SDK.hpp")) {
            picked.join("CppSDK")
        } else {
            return self.host_fail("sdk_folder");
        };
        if let Err(failed) = self.ensure_upstream(&ws).await {
            return failed;
        }
        let name = dir
            .parent()
            .map(file_name)
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| file_name(&dir));
        self.import_sdk_from(&ws, dir, name).await
    }

    /// `ensureHostWorkspace` from the page's button: errors are shown.
    pub async fn host_download_project(&self) -> ActionResult {
        self.ensure_host_workspace(false).await
    }

    pub async fn host_action(&self, action: HostAction) -> ActionResult {
        if self.job_running() {
            return busy();
        }
        match action {
            HostAction::InjectDumper => return self.start_dump().await,
            HostAction::InjectExporter => return self.start_export().await,
            _ => {}
        }
        let Some(ws) = self.workspace() else {
            return self.workspace_fail();
        };
        let w = WorkspacePaths::new(&ws);
        if needs_upstream(action, self.host.server.has_embedded())
            && let Err(failed) = self.ensure_upstream(&ws).await
        {
            return failed;
        }
        match action {
            HostAction::InjectDumper | HostAction::InjectExporter => unreachable!("handled above"),
            HostAction::ImportSdk => {
                let Some(dump) = find_dumps(&self.host.dump_root).into_iter().next() else {
                    return self.host_fail("no_dump");
                };
                self.import_sdk_from(&ws, PathBuf::from(dump.dir), dump.name).await
            }
            HostAction::BuildRuntime | HostAction::BuildExporter => {
                let tools = find_toolchain().await;
                let Some(msbuild) = tools.msbuild else {
                    return self.host_fail("msbuild");
                };
                let (kind, project, name) = if action == HostAction::BuildRuntime {
                    (HostJobKind::BuildRuntime, w.runtime_solution.clone(), "build_runtime")
                } else {
                    (HostJobKind::BuildExporter, w.exporter_project.clone(), "build_exporter")
                };
                let log_file = w.logs.join(format!("{name}.log"));
                self.start_job(kind, None, Some(&log_file));
                let cwd = project.parent().map_or_else(|| ws.clone(), Path::to_path_buf);
                let args = msbuild_args(&project, platform_toolset(tools.major_version));
                let r = run_logged(&msbuild, &args, &cwd, &log_file).await;
                self.finish_job(kind, r.code == 0, r.last_line, Some(&log_file))
            }
            HostAction::BuildLoader => {
                let Some(cargo) = find_toolchain().await.cargo else {
                    return self.host_fail("cargo");
                };
                let log_file = w.logs.join("build_loader.log");
                self.start_job(HostJobKind::BuildLoader, None, Some(&log_file));
                let args = ["build".to_owned(), "--release".to_owned()];
                let r = run_logged(&cargo, &args, &w.loader_dir, &log_file).await;
                self.finish_job(HostJobKind::BuildLoader, r.code == 0, r.last_line, Some(&log_file))
            }
            HostAction::CopyExports => {
                let Some(from) = self.game_exports() else {
                    return self.host_fail("no_exports");
                };
                self.start_job(HostJobKind::CopyExports, None, None);
                let to = w.exports.clone();
                match tokio::task::spawn_blocking(move || copy_tree(&from, &to)).await {
                    Ok(Ok(n)) => self.finish_job(HostJobKind::CopyExports, true, Some(format!("{n} files")), None),
                    Ok(Err(error)) => {
                        self.finish_job(HostJobKind::CopyExports, false, Some(log::describe_error(&error)), None)
                    }
                    Err(error) => self.finish_job(HostJobKind::CopyExports, false, Some(error.to_string()), None),
                }
            }
            HostAction::SyntheticData => {
                let log_file = w.launcher_log("gamedata");
                self.start_job(HostJobKind::SyntheticData, None, Some(&log_file));
                self.one_shot(
                    HostJobKind::SyntheticData,
                    &ws,
                    &["gamedata", "--examples"],
                    "gamedata",
                    None,
                )
                .await
            }
            HostAction::PublishRuntime => {
                let log_file = w.launcher_log("publish-runtime");
                self.start_job(HostJobKind::PublishRuntime, None, Some(&log_file));
                let r = match self.run_host(&ws, &["publish-runtime"], "publish-runtime").await {
                    Ok(r) => r,
                    Err(failed) => {
                        self.set_job(None);
                        return failed;
                    }
                };
                let version = published_version(&r.lines);
                if r.code == 0 {
                    log::info(&format!(
                        "host: runtime {} published to the content server",
                        version.as_deref().unwrap_or("?")
                    ));
                }
                let message = if r.code == 0 { version } else { r.failure };
                self.finish_job(HostJobKind::PublishRuntime, r.code == 0, message, Some(&log_file))
            }
            HostAction::GenerateGamedata => self.generate_game_data(&ws).await,
            HostAction::ExportGamedata => {
                let log_file = w.launcher_log("gamedata-export");
                self.start_job(HostJobKind::ExportGamedata, None, Some(&log_file));
                let r = match self.run_host(&ws, &["gamedata", "--export"], "gamedata-export").await {
                    Ok(r) => r,
                    Err(failed) => {
                        self.set_job(None);
                        return failed;
                    }
                };
                let written = pack_written(&r.lines).map(|file| file_name(Path::new(&file)));
                let message = if r.code == 0 { written } else { r.failure };
                self.finish_job(HostJobKind::ExportGamedata, r.code == 0, message, Some(&log_file))
            }
        }
    }

    pub fn host_open(&self, folder: HostFolder) -> ActionResult {
        let ws = self.s().host_workspace.map(PathBuf::from);
        let w = ws.as_deref().map(WorkspacePaths::new);
        let dir = match folder {
            HostFolder::Workspace => ws.clone(),
            HostFolder::RuntimeSource => w.as_ref().map(|w| w.runtime_source.clone()),
            HostFolder::Dumps => Some(self.host.dump_root.clone()),
            HostFolder::GameData => w
                .as_ref()
                .and_then(|w| w.backend_game_data.parent().map(Path::to_path_buf)),
            HostFolder::RuntimePublish => w.as_ref().map(|w| w.publish.clone()),
            HostFolder::HostLogs => w.as_ref().map(|w| w.logs.clone()),
            HostFolder::GamedataPacks => w.as_ref().map(|w| w.gamedata_packs.clone()),
        };
        match dir.filter(|dir| is_dir(dir)) {
            Some(dir) => {
                self.p.open_folder(&dir);
                ActionResult::Ok
            }
            None => self.host_fail("folder"),
        }
    }

    /// The server's view now; whether one started elsewhere answers is looked at in the background and
    /// reaches the page as a `host-server` event.
    pub fn host_server_view(&self) -> HostServerView {
        let server = self.host.server.clone();
        self.rt.spawn(async move { server.refresh().await });
        self.host.server.view()
    }

    pub async fn host_server_start(&self) -> ActionResult {
        let Some(ws) = self.workspace() else {
            return self.workspace_fail();
        };
        if self.job_running() {
            return busy();
        }
        let tool = match self.tool(&ws) {
            Ok(tool) => tool,
            Err(failed) => return failed,
        };
        let toolchain = crate::platform::hostkit::Toolchain::default();
        let kit = detect_host_kit(Some(&ws), &toolchain, &self.host.dump_root);
        if !kit.server_config.configured {
            return self.host_fail("not_configured");
        }
        if kit.game_data.backend == GameDataState::Missing || kit.game_data.director == GameDataState::Missing {
            return self.host_fail("gamedata_missing");
        }
        match self.host.server.start(&tool, &ws).await {
            Ok(()) => {
                log::info("host: server start requested from the Host page");
                ActionResult::Ok
            }
            Err(error) => self.host_kit_failure(&error),
        }
    }

    /// One service of the server inside the launcher, stopped and started again.
    pub async fn host_server_restart(&self, name: &str) -> ActionResult {
        match self.host.server.restart(name).await {
            Ok(()) => {
                log::info(&format!("host: {name} restarted from the Host page"));
                ActionResult::Ok
            }
            Err(why) => self.host_fail(&why),
        }
    }

    /// The server GUI's in-process server, driven in place of `reforged-host start`.
    pub fn set_embedded_server(&self, server: Arc<dyn crate::platform::hostserver::EmbeddedServer>) {
        self.host.server.set_embedded(server);
    }

    pub async fn host_server_stop(&self) -> ActionResult {
        self.host.server.stop().await;
        log::info("host: server stopped from the Host page");
        ActionResult::Ok
    }

    /// `reforged-host setup` from the page's form, already checked (`commands::host_setup_request`). The
    /// game folder is the one this launcher plays from.
    pub async fn host_setup(&self, request: HostSetupRequest) -> ActionResult {
        let Some(ws) = self.workspace() else {
            return self.workspace_fail();
        };
        if self.job_running() {
            return busy();
        }
        if self.host.server.running() || self.host.server.state() == HostServerState::External {
            return self.host_fail("server_running");
        }
        let mode = match request.mode {
            HostServerMode::Private => "private",
            HostServerMode::Public => "public",
        };
        let mut args = vec!["setup", "--mode", mode, "--name", request.name.as_str()];
        if request.mode == HostServerMode::Public
            && let Some(host) = request.public_host.as_deref()
        {
            args.extend(["--host", host]);
        }
        let root = self.st().game.root.clone();
        if let Some(root) = root.as_deref() {
            args.extend(["--game", root]);
        }
        let log_file = WorkspacePaths::new(&ws).launcher_log("setup");
        self.start_job(HostJobKind::Setup, None, Some(&log_file));
        let r = match self.run_host(&ws, &args, "setup").await {
            Ok(r) => r,
            Err(failed) => {
                self.set_job(None);
                return failed;
            }
        };
        *lock(&self.host.requirements) = None;
        let message = if r.code == 0 { Some(request.name) } else { r.failure };
        self.finish_job(HostJobKind::Setup, r.code == 0, message, Some(&log_file))
    }

    /// Who the invite is for, already trimmed and checked; None for nobody in particular. The link is a
    /// secret: it goes to the page's view only, never to the log.
    pub async fn host_create_invite(&self, for_name: Option<String>) -> ActionResult {
        let Some(ws) = self.workspace() else {
            return self.workspace_fail();
        };
        if !matches!(
            self.host.server.state(),
            HostServerState::Running | HostServerState::External
        ) {
            return self.host_fail("server_not_running");
        }
        let mut args = vec!["invite", "--json"];
        if let Some(name) = for_name.as_deref() {
            args.extend(["--for", name]);
        }
        let r = match self.run_host(&ws, &args, "invite").await {
            Ok(r) => r,
            Err(failed) => return failed,
        };
        let invite = if r.code == 0 { invite_line(&r.lines) } else { None };
        let Some(invite) = invite else {
            return self.host_fail(r.failure.as_deref().unwrap_or("invite"));
        };
        self.host.server.set_invite(Some(HostInvite {
            link: invite.link,
            for_name: invite.for_name,
        }));
        log::info(&format!(
            "host: invite created{}",
            if for_name.is_some() { " (with a note)" } else { "" }
        ));
        ActionResult::Ok
    }

    /// `reforged-host update`: compares first (a server stays up when there is nothing to update), then
    /// updates with the server stopped and runs setup again with this server's settings.
    pub async fn host_update(&self) -> ActionResult {
        let Some(ws) = self.workspace() else {
            return self.workspace_fail();
        };
        if self.job_running() {
            return busy();
        }
        if self.host.server.state() == HostServerState::External {
            return self.host_fail("server_external");
        }
        let log_file = WorkspacePaths::new(&ws).launcher_log("update");
        self.start_job(HostJobKind::Update, None, Some(&log_file));
        let check = match self.run_host(&ws, &["update", "--check", "--json"], "update").await {
            Ok(r) => r,
            Err(failed) => {
                self.set_job(None);
                return failed;
            }
        };
        let checked_json = last_json(&check.lines);
        let checked = checked_json.as_ref().and_then(update_check);
        let Some(checked) = checked.filter(|_| check.code == 0) else {
            let why = update_failure(checked_json.as_ref(), check.failure.as_deref());
            return self.finish_job(HostJobKind::Update, false, Some(why), Some(&log_file));
        };
        if checked.behind == 0 {
            *lock(&self.host.update) = Some(HostUpdateView {
                state: HostUpdateState::UpToDate,
                head: checked.head,
                files: 0,
                runtime_changed: false,
            });
            return self.finish_job(HostJobKind::Update, true, None, Some(&log_file));
        }
        let configured = read_server_config(&ws).configured;
        self.while_stopped(&ws, async {
            let r = match self.run_host(&ws, &["update", "--json", "--no-setup"], "update").await {
                Ok(r) => r,
                Err(failed) => {
                    self.set_job(None);
                    return failed;
                }
            };
            let done_json = last_json(&r.lines);
            let Some(done) = done_json.as_ref().and_then(update_done).filter(|_| r.code == 0) else {
                let why = update_failure(done_json.as_ref(), r.failure.as_deref());
                return self.finish_job(HostJobKind::Update, false, Some(why), Some(&log_file));
            };
            log::info(&format!("host: project updated to {} ({} files)", done.to, done.files));
            *lock(&self.host.update) = Some(HostUpdateView {
                state: HostUpdateState::Updated,
                head: done.to,
                files: done.files,
                runtime_changed: done.runtime_changed,
            });
            if configured {
                let s = match self.run_host(&ws, &["setup"], "setup").await {
                    Ok(s) => s,
                    Err(failed) => {
                        self.set_job(None);
                        return failed;
                    }
                };
                *lock(&self.host.requirements) = None;
                if s.code != 0 {
                    let why = s.failure.unwrap_or_else(|| "setup".into());
                    return self.finish_job(HostJobKind::Update, false, Some(why), Some(&log_file));
                }
            }
            self.finish_job(HostJobKind::Update, true, None, Some(&log_file))
        })
        .await
    }

    /// `reforged-host multipliers` (None turns them off). The backend reads them when it starts.
    pub async fn host_set_multipliers(&self, multipliers: Option<HostMultipliers>) -> ActionResult {
        let Some(ws) = self.workspace() else {
            return self.workspace_fail();
        };
        if self.job_running() {
            return busy();
        }
        if self.host.server.state() == HostServerState::External {
            return self.host_fail("server_external");
        }
        let config = read_server_config(&ws);
        if !config.configured {
            return self.host_fail("not_configured");
        }
        if config.mode != Some(HostServerMode::Private) {
            return self.host_fail("multipliers_public");
        }
        let (xp, rewards) = multipliers.map_or((String::new(), String::new()), |m| {
            (m.xp.to_string(), m.rewards.to_string())
        });
        let args: Vec<&str> = if multipliers.is_some() {
            vec!["multipliers", "--xp", &xp, "--rewards", &rewards]
        } else {
            vec!["multipliers", "--off"]
        };
        let log_file = WorkspacePaths::new(&ws).launcher_log("multipliers");
        self.start_job(HostJobKind::Multipliers, None, Some(&log_file));
        let r = self.while_stopped(&ws, self.run_host(&ws, &args, "multipliers")).await;
        let r = match r {
            Ok(r) => r,
            Err(failed) => {
                self.set_job(None);
                return failed;
            }
        };
        log::info(&format!(
            "host: test multipliers {}",
            multipliers.map_or_else(|| "off".to_owned(), |m| format!("XP x{}, rewards x{}", m.xp, m.rewards))
        ));
        let message = if r.code == 0 { None } else { r.failure };
        self.finish_job(HostJobKind::Multipliers, r.code == 0, message, Some(&log_file))
    }

    // ------------------------------------------------------------------ steps

    /// A host kit command as a job: done or failed on its exit code.
    async fn one_shot(
        &self,
        kind: HostJobKind,
        ws: &Path,
        args: &[&str],
        log_name: &str,
        ok_message: Option<String>,
    ) -> ActionResult {
        let log_file = WorkspacePaths::new(ws).launcher_log(log_name);
        let r = match self.run_host(ws, args, log_name).await {
            Ok(r) => r,
            Err(failed) => {
                self.set_job(None);
                return failed;
            }
        };
        let message = if r.code == 0 { ok_message } else { r.failure };
        self.finish_job(kind, r.code == 0, message, Some(&log_file))
    }

    /// Mystic Paradox's source is part of the checkout (RD1); a release without it gets it from
    /// `reforged-host upstream`, so the SDK, the loader and the game data steps do not wait for setup.
    async fn ensure_upstream(&self, ws: &Path) -> Result<(), ActionResult> {
        let w = WorkspacePaths::new(ws);
        if is_file(&w.runtime_source.join("dllmain.cpp")) {
            return Ok(());
        }
        let log_file = w.launcher_log("upstream");
        self.start_job(HostJobKind::Upstream, None, Some(&log_file));
        let r = match self.run_host(ws, &["upstream"], "upstream").await {
            Ok(r) => r,
            Err(failed) => {
                self.set_job(None);
                return Err(failed);
            }
        };
        if r.code != 0 {
            let why = r.failure.unwrap_or_else(|| "upstream".into());
            return Err(self.finish_job(HostJobKind::Upstream, false, Some(why), Some(&log_file)));
        }
        if !is_file(&w.runtime_source.join("dllmain.cpp")) {
            self.set_job(None);
            return Err(self.host_fail("upstream_old"));
        }
        log::info("host: Mystic Paradox downloaded for the Build tab");
        Ok(())
    }

    async fn import_sdk_from(&self, ws: &Path, dir: PathBuf, name: String) -> ActionResult {
        self.start_job(HostJobKind::ImportSdk, Some(name.clone()), None);
        let source = WorkspacePaths::new(ws).runtime_source;
        match tokio::task::spawn_blocking(move || import_sdk(&dir, &source)).await {
            Ok(Ok(n)) => {
                log::info(&format!("host: imported {n} SDK files from {name}"));
                self.finish_job(HostJobKind::ImportSdk, true, Some(format!("{n} files")), None)
            }
            Ok(Err(error)) => self.finish_job(HostJobKind::ImportSdk, false, Some(error.code().into()), None),
            Err(error) => self.finish_job(HostJobKind::ImportSdk, false, Some(error.to_string()), None),
        }
    }

    /// The game's `Items_Analysis` (CatalogExporter's output), when it is there.
    fn game_exports(&self) -> Option<PathBuf> {
        let dir = Self::win64_dir_of(&self.st())?.join("Items_Analysis");
        is_dir(&dir).then_some(dir)
    }

    /// The game's export into the workspace when there is one, then `reforged-host gamedata`. A running
    /// server reads the new data once it is started again.
    async fn generate_game_data(&self, ws: &Path) -> ActionResult {
        let w = WorkspacePaths::new(ws);
        let log_file = w.launcher_log("gamedata");
        self.start_job(HostJobKind::GenerateGamedata, None, Some(&log_file));
        if let Some(from) = self.game_exports() {
            let to = w.exports.clone();
            match tokio::task::spawn_blocking(move || copy_tree(&from, &to)).await {
                Ok(Ok(n)) => self.host.server.note(&format!(
                    "Copied {n} files of the game's CatalogExporter export into the workspace."
                )),
                Ok(Err(error)) => {
                    let why = log::describe_error(&error);
                    return self.finish_job(HostJobKind::GenerateGamedata, false, Some(why), Some(&log_file));
                }
                Err(error) => {
                    return self.finish_job(
                        HostJobKind::GenerateGamedata,
                        false,
                        Some(error.to_string()),
                        Some(&log_file),
                    );
                }
            }
        } else if !is_dir(&w.exports) {
            self.set_job(None);
            return self.host_fail("no_exports");
        }
        let restart = self.host.server.running().then(|| "restart".to_owned());
        self.one_shot(HostJobKind::GenerateGamedata, ws, &["gamedata"], "gamedata", restart)
            .await
    }

    /// Where the Dumper-7 or CatalogExporter session runs: the game's Win64 folder, after the checks
    /// both sessions share.
    fn session_folder(&self) -> Result<PathBuf, ActionResult> {
        if self.c.host != HostPlatform::Windows {
            // Under Proton the loader works the same way; RA12a's spike decides (RD4)
            return Err(self.host_fail("windows_only"));
        }
        {
            let st = self.st();
            if st.is_busy() || st.shutting_down {
                return Err(busy());
            }
        }
        if self.game.running() {
            return Err(self.host_fail("game_running"));
        }
        let (dir, problem) = {
            let st = self.st();
            (Self::win64_dir_of(&st), st.game.problem)
        };
        match (dir, problem) {
            (None, _) | (_, Some(GameProblem::NotFound | GameProblem::NotSet)) => {
                Err(ActionResult::Err(self.fail(ErrorCode::GameFolderNotFound, None)))
            }
            // The SDK and the export must come from the exact build the runtime targets
            (_, Some(GameProblem::WrongVersion)) => {
                Err(ActionResult::Err(self.fail(ErrorCode::GameWrongVersion, None)))
            }
            (Some(dir), _) => Ok(dir),
        }
    }

    /// The Dumper-7 session: the game starts with the loader and Dumper-7 on its list, without the
    /// runtime. The SDK appears under `C:\Dumper-7` a minute or two later; "Import SDK" copies it into the
    /// workspace. Closing the game ends the session (`mod.rs`).
    async fn start_dump(&self) -> ActionResult {
        let dir = match self.session_folder() {
            Ok(dir) => dir,
            Err(failed) => return failed,
        };
        let Some(dll) = self.s().dumper_dll.map(PathBuf::from).filter(|dll| is_file(dll)) else {
            return self.host_fail("dumper");
        };
        if !self.p.find_running_clients().await.is_empty() {
            return ActionResult::Err(self.fail(ErrorCode::AlreadyRunning, None));
        }
        let session = match self.begin_loader_session(SessionKind::Dump, &dir, &dll) {
            Ok(session) => session,
            Err(error) => return ActionResult::Err(error),
        };
        self.start_job(HostJobKind::Inject, Some("Dumper-7".into()), None);
        {
            let mut st = self.st();
            st.dumping = true;
            st.loader_session = Some(session);
        }
        // The game's own environment, without anything that would point a runtime somewhere
        let base = self.seams.env.clone().unwrap_or_else(|| std::env::vars_os().collect());
        let env: Vec<_> = base
            .into_iter()
            .filter(|(name, _)| {
                let name = name.to_string_lossy().to_ascii_uppercase();
                !name.starts_with("REFORGED_") && !name.starts_with("MYSTICPARADOX_")
            })
            .collect();
        let args: Vec<String> = DUMP_ARGS.iter().map(|a| (*a).to_owned()).collect();
        log::info(&format!(
            "host: starting the game for Dumper-7 ({} through {})",
            file_name(&dll),
            loader_session::LOADER_INI
        ));
        if let Err(error) = self.game.start(&dir, &args, None, &env) {
            let session = {
                let mut st = self.st();
                st.dumping = false;
                st.loader_session.take()
            };
            if let Some(session) = session {
                self.end_loader_session(session).await;
            }
            return self.finish_job(HostJobKind::Inject, false, Some(log::describe_error(&error)), None);
        }
        self.finish_job(HostJobKind::Inject, true, Some("Dumper-7".into()), None)
    }

    /// The CatalogExporter session: `export_flags.txt` in the game's working folder, then the game
    /// starts as PLAY starts it (with the runtime, through the relay) and the loader loads CatalogExporter
    /// after the runtime. It writes `Items_Analysis` there; closing the game ends the session.
    async fn start_export(&self) -> ActionResult {
        let Some(ws) = self.workspace() else {
            return self.workspace_fail();
        };
        let dir = match self.session_folder() {
            Ok(dir) => dir,
            Err(failed) => return failed,
        };
        let exporter = WorkspacePaths::new(&ws).exporter_dll;
        if !is_file(&exporter) {
            return self.host_fail("exporter");
        }
        if let Err(error) = tokio::fs::write(dir.join("export_flags.txt"), EXPORT_FLAGS).await {
            return self.host_fail(&format!("export_flags:{}", log::describe_error(&error)));
        }
        self.start_job(HostJobKind::Inject, Some("CatalogExporter".into()), None);
        match self.play_with(Some(exporter)).await {
            ActionResult::Ok => {
                log::info("host: the game started with CatalogExporter on the loader's list");
                self.finish_job(HostJobKind::Inject, true, Some("CatalogExporter".into()), None)
            }
            failed => {
                self.set_job(None);
                failed
            }
        }
    }

    // ------------------------------------------------------------------ server GUI in-process host kit (B3)

    #[cfg(feature = "server-gui")]
    pub(super) async fn run_host_in_process(&self, ws: &Path, args: &[&str], log_name: &str) -> CommandOutput {
        let cmd_name = args.first().copied().unwrap_or("").to_owned();
        let secret_json = cmd_name == "invite";
        self.host.server.note(&format!("> {cmd_name}"));
        let ws_buf = ws.to_path_buf();
        let arg_vec: Vec<String> = args.iter().map(|s| (*s).to_owned()).collect();
        let log_file = WorkspacePaths::new(ws).launcher_log(log_name);
        let server = self.host.server.clone();

        tokio::task::spawn_blocking(move || {
            let mut log = log_file.parent().and_then(|dir| {
                std::fs::create_dir_all(dir).ok()?;
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&log_file)
                    .ok()
            });
            if let Some(f) = log.as_mut() {
                use std::io::Write as _;
                let _ = writeln!(
                    f,
                    "\n> embedded-host {}  (in {}, {})",
                    arg_vec.join(" "),
                    ws_buf.display(),
                    chrono::Utc::now().to_rfc3339()
                );
            }
            let mut lines = Vec::<String>::new();
            let mut emit = |raw: &str| {
                for part in raw.lines() {
                    let clean = crate::platform::hostkit::parse::clean_line(part);
                    if clean.is_empty() {
                        continue;
                    }
                    lines.push(clean.clone());
                    let hide = secret_json && clean.trim_start().starts_with('{');
                    if !hide {
                        if let Some(f) = log.as_mut() {
                            use std::io::Write as _;
                            let _ = writeln!(f, "{clean}");
                        }
                        server.note(&clean);
                    }
                }
            };

            let paths = host::paths::Paths::new(&ws_buf);
            let outcome: host::Outcome = match cmd_name.as_str() {
                "setup" => {
                    let mut setup_args = host::cli::SetupArgs::default();
                    let mut i = 1;
                    while i < arg_vec.len() {
                        match arg_vec[i].as_str() {
                            "--mode" if i + 1 < arg_vec.len() => {
                                setup_args.mode = Some(arg_vec[i + 1].clone());
                                i += 2;
                            }
                            "--name" if i + 1 < arg_vec.len() => {
                                setup_args.name = Some(arg_vec[i + 1].clone());
                                i += 2;
                            }
                            "--host" if i + 1 < arg_vec.len() => {
                                setup_args.host = Some(arg_vec[i + 1].clone());
                                i += 2;
                            }
                            "--game" if i + 1 < arg_vec.len() => {
                                setup_args.game = Some(arg_vec[i + 1].clone());
                                i += 2;
                            }
                            "--fresh-start" => {
                                setup_args.fresh_start = true;
                                i += 1;
                            }
                            _ => i += 1,
                        }
                    }
                    host::commands::setup::run_with(&paths, &setup_args, &InProcessSetupEnv, &mut emit)
                }
                "invite" => {
                    let mut invite_args = host::cli::InviteArgs {
                        uses: "1".to_owned(),
                        ..Default::default()
                    };
                    let mut i = 1;
                    while i < arg_vec.len() {
                        match arg_vec[i].as_str() {
                            "--json" => {
                                invite_args.json = true;
                                i += 1;
                            }
                            "--for" if i + 1 < arg_vec.len() => {
                                invite_args.for_whom = Some(arg_vec[i + 1].clone());
                                i += 2;
                            }
                            _ => i += 1,
                        }
                    }
                    host::commands::invite::run_with(&paths, &invite_args, host::paths::PORTS.backend, &mut emit)
                }
                "multipliers" => {
                    let mut mult_args = host::cli::MultipliersArgs::default();
                    let mut i = 1;
                    while i < arg_vec.len() {
                        match arg_vec[i].as_str() {
                            "--off" => {
                                mult_args.off = true;
                                i += 1;
                            }
                            "--xp" if i + 1 < arg_vec.len() => {
                                mult_args.xp = Some(arg_vec[i + 1].clone());
                                i += 2;
                            }
                            "--rewards" if i + 1 < arg_vec.len() => {
                                mult_args.rewards = Some(arg_vec[i + 1].clone());
                                i += 2;
                            }
                            _ => i += 1,
                        }
                    }
                    let commit = |_repo: &Path| -> Result<String, host::HostError> { Ok(String::new()) };
                    host::commands::multipliers::run_with(
                        &paths,
                        &mult_args,
                        host::hostconfig::Platform::current(),
                        &commit,
                        &mut emit,
                    )
                }
                "gamedata" => {
                    let mut gd_args = host::cli::GamedataArgs::default();
                    let mut i = 1;
                    while i < arg_vec.len() {
                        match arg_vec[i].as_str() {
                            "--examples" => {
                                gd_args.examples = true;
                                i += 1;
                            }
                            "--export" => {
                                if i + 1 < arg_vec.len() && !arg_vec[i + 1].starts_with("--") {
                                    gd_args.export = Some(Some(arg_vec[i + 1].clone()));
                                    i += 2;
                                } else {
                                    gd_args.export = Some(None);
                                    i += 1;
                                }
                            }
                            "--import" if i + 1 < arg_vec.len() => {
                                gd_args.import = Some(Some(arg_vec[i + 1].clone()));
                                i += 2;
                            }
                            _ => i += 1,
                        }
                    }
                    if !paths.backend_dir.exists() && !paths.host_toml.exists() && !paths.host_json.exists() {
                        let _ = host::gamedata::seed_embedded_examples(&paths);
                    }
                    host::commands::gamedata::run_with(&paths, &gd_args, &mut emit)
                }
                "publish-runtime" => {
                    let pr_args = host::cli::PublishRuntimeArgs::default();
                    host::commands::publish_runtime::run_with(&paths, &pr_args, &mut emit)
                }
                "update" => {
                    emit(r#"{"ok":true,"behind":0,"head":"embedded"}"#);
                    Ok(std::process::ExitCode::SUCCESS)
                }
                other => Err(host::HostError::message(format!("unsupported command: {other}"))),
            };

            let code = match outcome {
                Ok(exit) => {
                    if exit == std::process::ExitCode::SUCCESS {
                        0
                    } else {
                        1
                    }
                }
                Err(error) => {
                    emit(&format!("Error: {error}"));
                    1
                }
            };
            if let Some(f) = log.as_mut() {
                use std::io::Write as _;
                let _ = writeln!(f, "(exit code {code})");
            }
            let failure = if code == 0 {
                None
            } else {
                crate::platform::hostkit::parse::failure_line(&lines)
            };
            CommandOutput { code, lines, failure }
        })
        .await
        .unwrap_or_else(|e| CommandOutput {
            code: 1,
            lines: vec![e.to_string()],
            failure: Some(e.to_string()),
        })
    }

    #[cfg(feature = "server-gui")]
    pub async fn server_doctor(&self) -> crate::bridge::types::ServerDoctorOutcome {
        use crate::bridge::types::{DoctorCheckView, DoctorLevel, DoctorReportView, ServerDoctorOutcome};
        let Some(ws) = self.workspace() else {
            return ServerDoctorOutcome::Err(self.fail(ErrorCode::HostFailed, Some(self.workspace_detail().into())));
        };
        let res = tokio::task::spawn_blocking(move || {
            let paths = host::paths::Paths::new(ws);
            host::doctor::run_doctor(&paths, &host::doctor::DoctorOptions { hash_exe: false })
        })
        .await;
        let checks = match res {
            Ok(Ok(checks)) => checks,
            Ok(Err(error)) => {
                return ServerDoctorOutcome::Err(self.fail(ErrorCode::HostFailed, Some(error.to_string())));
            }
            Err(error) => {
                return ServerDoctorOutcome::Err(self.fail(ErrorCode::HostFailed, Some(error.to_string())));
            }
        };
        let mut warnings = 0u32;
        let mut problems = 0u32;
        let mut views = Vec::with_capacity(checks.len());
        for c in checks {
            let level = match c.level {
                host::doctor::Level::Ok | host::doctor::Level::Info => DoctorLevel::Ok,
                host::doctor::Level::Warn => {
                    warnings += 1;
                    DoctorLevel::Warning
                }
                host::doctor::Level::Fail => {
                    problems += 1;
                    DoctorLevel::Problem
                }
            };
            let detail = match c.fix {
                Some(fix) => format!("{} — {}", c.text, fix),
                None => c.text,
            };
            views.push(DoctorCheckView {
                level,
                label: c.area.to_owned(),
                detail,
            });
        }
        ServerDoctorOutcome::Ok(DoctorReportView {
            checks: views,
            warnings,
            problems,
        })
    }

    #[cfg(feature = "server-gui")]
    pub async fn server_backup(&self) -> ActionResult {
        if self.job_running() {
            return busy();
        }
        let Some(ws) = self.workspace() else {
            return self.workspace_fail();
        };
        let log_file = WorkspacePaths::new(&ws).launcher_log("backup");
        self.start_job(HostJobKind::Backup, None, Some(&log_file));
        let server = self.host.server.clone();
        let log_clone = log_file.clone();
        let res = tokio::task::spawn_blocking(move || {
            let mut log = log_clone.parent().and_then(|dir| {
                std::fs::create_dir_all(dir).ok()?;
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&log_clone)
                    .ok()
            });
            let mut lines = Vec::<String>::new();
            let mut emit = |raw: &str| {
                for part in raw.lines() {
                    let clean = crate::platform::hostkit::parse::clean_line(part);
                    if clean.is_empty() {
                        continue;
                    }
                    if let Some(f) = log.as_mut() {
                        use std::io::Write as _;
                        let _ = writeln!(f, "{clean}");
                    }
                    server.note(&clean);
                    lines.push(clean);
                }
            };
            let paths = host::paths::Paths::new(ws);
            match host::commands::backup::run_with(&paths, &host::commands::backup::System, &mut emit) {
                Ok(code) if code == std::process::ExitCode::SUCCESS => Ok(lines.last().cloned()),
                Ok(_) => Err(lines.last().cloned().unwrap_or_else(|| "backup".into())),
                Err(error) => {
                    emit(&format!("Error: {error}"));
                    Err(error.to_string())
                }
            }
        })
        .await;
        match res {
            Ok(Ok(msg)) => self.finish_job(HostJobKind::Backup, true, msg, Some(&log_file)),
            Ok(Err(why)) => self.finish_job(HostJobKind::Backup, false, Some(why), Some(&log_file)),
            Err(error) => self.finish_job(HostJobKind::Backup, false, Some(error.to_string()), Some(&log_file)),
        }
    }

    #[cfg(feature = "server-gui")]
    pub async fn server_config_get(&self) -> crate::bridge::types::ServerConfigOutcome {
        use crate::bridge::types::ServerConfigOutcome;
        let Some(ws) = self.workspace() else {
            return ServerConfigOutcome::Err(self.fail(ErrorCode::HostFailed, Some(self.workspace_detail().into())));
        };
        match tokio::task::spawn_blocking(move || read_server_config_view(&ws)).await {
            Ok(Ok(view)) => ServerConfigOutcome::Ok(view),
            Ok(Err(why)) => ServerConfigOutcome::Err(self.fail(ErrorCode::HostFailed, Some(why))),
            Err(error) => ServerConfigOutcome::Err(self.fail(ErrorCode::HostFailed, Some(error.to_string()))),
        }
    }

    #[cfg(feature = "server-gui")]
    pub async fn server_config_save(
        &self,
        registration: String,
        database: String,
        max_hubs: u32,
        max_hunts: u32,
        fresh_start: bool,
    ) -> crate::bridge::types::ServerConfigOutcome {
        use crate::bridge::types::ServerConfigOutcome;
        let Some(ws) = self.workspace() else {
            return ServerConfigOutcome::Err(self.fail(ErrorCode::HostFailed, Some(self.workspace_detail().into())));
        };
        let res = tokio::task::spawn_blocking(move || -> Result<crate::bridge::types::ServerConfigView, String> {
            let paths = host::paths::Paths::new(&ws);
            let mut config = host::hostconfig::load_host_config(&paths)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "not_configured".to_owned())?;
            let reg_norm = if registration == "INVITE" {
                "INVITECODE"
            } else {
                registration.as_str()
            };
            config.registration =
                host::hostconfig::Registration::parse(reg_norm).ok_or_else(|| "registration".to_owned())?;
            config.max_hunt_servers = max_hunts.clamp(1, 128);
            let db = config.database.get_or_insert_with(Default::default);
            db.engine = Some(database);
            db.fresh_start = if fresh_start { Some(true) } else { None };
            let _ = max_hubs;
            host::hostconfig::save_host_config(&paths, &config).map_err(|e| e.to_string())?;
            if fresh_start {
                host::commands::update::confirm_fresh_start(&paths).map_err(|e| e.to_string())?;
            } else {
                let marker = paths.config.join(host::commands::update::MONGO_MIGRATED_MARKER);
                if host::commands::update::is_fresh_start_marker(&marker) {
                    let _ = std::fs::remove_file(&marker);
                }
            }
            if let Ok((secrets, _)) =
                host::hostconfig::load_or_create_secrets(&paths.config, host::commands::setup::secrets::new_secrets)
            {
                let envs = host::hostconfig::build_env_files(
                    &config,
                    &secrets,
                    &paths,
                    "",
                    host::hostconfig::Platform::current(),
                );
                let _ = host::hostconfig::write_env_files(&paths, &envs);
            }
            read_server_config_view(&ws)
        })
        .await;
        self.emit_host_kit();
        match res {
            Ok(Ok(view)) => ServerConfigOutcome::Ok(view),
            Ok(Err(why)) => ServerConfigOutcome::Err(self.fail(ErrorCode::HostFailed, Some(why))),
            Err(error) => ServerConfigOutcome::Err(self.fail(ErrorCode::HostFailed, Some(error.to_string()))),
        }
    }

    #[cfg(feature = "server-gui")]
    pub async fn server_migrate_mongo(&self) -> ActionResult {
        if self.job_running() {
            return busy();
        }
        if self.host.server.running() || self.host.server.state() == HostServerState::External {
            return self.host_fail("server_running");
        }
        let Some(ws) = self.workspace() else {
            return self.workspace_fail();
        };
        let log_file = WorkspacePaths::new(&ws).launcher_log("migrate-mongo");
        self.start_job(HostJobKind::MigrateMongo, None, Some(&log_file));
        let server = self.host.server.clone();
        let log_clone = log_file.clone();
        let res = tokio::task::spawn_blocking(move || {
            let mut log = log_clone.parent().and_then(|dir| {
                std::fs::create_dir_all(dir).ok()?;
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&log_clone)
                    .ok()
            });
            let mut lines = Vec::<String>::new();
            let mut emit = |raw: &str| {
                for part in raw.lines() {
                    let clean = crate::platform::hostkit::parse::clean_line(part);
                    if clean.is_empty() {
                        continue;
                    }
                    if let Some(f) = log.as_mut() {
                        use std::io::Write as _;
                        let _ = writeln!(f, "{clean}");
                    }
                    server.note(&clean);
                    lines.push(clean);
                }
            };
            let paths = host::paths::Paths::new(ws);
            match host::commands::update::migrate_existing_mongo_data(&paths, &mut emit) {
                Ok(()) => Ok(lines.last().cloned()),
                Err(error) => {
                    emit(&format!("Error: {error}"));
                    Err(error.to_string())
                }
            }
        })
        .await;
        match res {
            Ok(Ok(msg)) => self.finish_job(HostJobKind::MigrateMongo, true, msg, Some(&log_file)),
            Ok(Err(why)) => self.finish_job(HostJobKind::MigrateMongo, false, Some(why), Some(&log_file)),
            Err(error) => self.finish_job(
                HostJobKind::MigrateMongo,
                false,
                Some(error.to_string()),
                Some(&log_file),
            ),
        }
    }

    #[cfg(feature = "server-gui")]
    pub async fn server_confirm_fresh_start(&self) -> crate::bridge::types::ServerConfigOutcome {
        use crate::bridge::types::ServerConfigOutcome;
        let Some(ws) = self.workspace() else {
            return ServerConfigOutcome::Err(self.fail(ErrorCode::HostFailed, Some(self.workspace_detail().into())));
        };
        let res = tokio::task::spawn_blocking(move || -> Result<crate::bridge::types::ServerConfigView, String> {
            let paths = host::paths::Paths::new(&ws);
            host::commands::update::confirm_fresh_start(&paths).map_err(|e| e.to_string())?;
            read_server_config_view(&ws)
        })
        .await;
        self.emit_host_kit();
        match res {
            Ok(Ok(view)) => ServerConfigOutcome::Ok(view),
            Ok(Err(why)) => ServerConfigOutcome::Err(self.fail(ErrorCode::HostFailed, Some(why))),
            Err(error) => ServerConfigOutcome::Err(self.fail(ErrorCode::HostFailed, Some(error.to_string()))),
        }
    }

    #[cfg(feature = "server-gui")]
    pub async fn server_invite_list(&self) -> crate::bridge::types::ServerInviteListOutcome {
        use crate::bridge::types::{ServerInviteEntry, ServerInviteListOutcome};
        let Some(ws) = self.workspace() else {
            return ServerInviteListOutcome::Err(
                self.fail(ErrorCode::HostFailed, Some(self.workspace_detail().into())),
            );
        };
        if !matches!(
            self.host.server.state(),
            HostServerState::Running | HostServerState::External
        ) {
            return ServerInviteListOutcome::Err(self.fail(ErrorCode::ServerStopped, None));
        }
        let res = tokio::task::spawn_blocking(move || -> Result<Vec<ServerInviteEntry>, String> {
            let paths = host::paths::Paths::new(&ws);
            let config = host::hostconfig::load_host_config(&paths)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| "not_configured".to_owned())?;
            let (secrets, _) =
                host::hostconfig::load_or_create_secrets(&paths.config, host::commands::setup::secrets::new_secrets)
                    .map_err(|e| e.to_string())?;
            let fp = match config.mode {
                host::hostconfig::ServerMode::Public => {
                    Some(host::invite::certificate_fingerprint(&paths.cert_file).map_err(|e| e.to_string())?)
                }
                host::hostconfig::ServerMode::Private => None,
            };
            let raw = host::invite::list_invite_codes(&secrets.admin_key, host::paths::PORTS.backend)
                .map_err(|e| e.to_string())?;
            let items = raw
                .get("InviteCodes")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                let Some(code) = item.get("code").and_then(|v| v.as_str()) else {
                    continue;
                };
                let link = host::invite::invite_for(&config, code, fp.as_deref()).unwrap_or_else(|_| code.to_owned());
                let for_name = item
                    .get("note")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
                let uses_remaining = item.get("usesRemaining").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let infinite_uses = item.get("infiniteUses").and_then(|v| v.as_bool()).unwrap_or(false);
                let created_at = item
                    .get("createdAt")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_owned();
                out.push(ServerInviteEntry {
                    code: code.to_owned(),
                    link,
                    for_name,
                    uses_remaining,
                    infinite_uses,
                    created_at,
                });
            }
            Ok(out)
        })
        .await;
        match res {
            Ok(Ok(invites)) => ServerInviteListOutcome::Ok(invites),
            Ok(Err(why)) => ServerInviteListOutcome::Err(self.fail(ErrorCode::HostFailed, Some(why))),
            Err(error) => ServerInviteListOutcome::Err(self.fail(ErrorCode::HostFailed, Some(error.to_string()))),
        }
    }
}

#[cfg(feature = "server-gui")]
fn read_server_config_view(ws: &Path) -> Result<crate::bridge::types::ServerConfigView, String> {
    use crate::bridge::types::{MongoMigrationState, MongoMigrationView, ServerConfigView};
    use host::commands::update::MongoMigrationStatus;

    let paths = host::paths::Paths::new(ws);
    let config = host::hostconfig::load_host_config(&paths).map_err(|e| e.to_string())?;
    let mongo_migration = match host::commands::update::inspect_mongo_migration(&paths) {
        MongoMigrationStatus::None => MongoMigrationView {
            state: MongoMigrationState::None,
            source: None,
            detail: None,
        },
        MongoMigrationStatus::Migrated => MongoMigrationView {
            state: MongoMigrationState::Migrated,
            source: None,
            detail: None,
        },
        MongoMigrationStatus::Skipped => MongoMigrationView {
            state: MongoMigrationState::Skipped,
            source: None,
            detail: None,
        },
        MongoMigrationStatus::Unmigrated { source } => MongoMigrationView {
            state: MongoMigrationState::Unmigrated,
            source: Some(source),
            detail: None,
        },
        MongoMigrationStatus::Unreachable { uri, detail } => MongoMigrationView {
            state: MongoMigrationState::Unreachable,
            source: Some(uri),
            detail: Some(detail),
        },
    };
    let config_file = if paths.host_toml.is_file() {
        Some(path_text(&paths.host_toml))
    } else if paths.host_json.is_file() {
        Some(path_text(&paths.host_json))
    } else {
        None
    };
    let Some(c) = config else {
        return Ok(ServerConfigView {
            configured: false,
            name: String::new(),
            mode: HostServerMode::Private,
            public_host: None,
            registration: "INVITECODE".into(),
            database: "sqlite".into(),
            max_hubs: 1,
            max_hunts: 3,
            fresh_start: false,
            mongo_migration,
            config_file,
        });
    };
    let mode = match c.mode {
        host::hostconfig::ServerMode::Private => HostServerMode::Private,
        host::hostconfig::ServerMode::Public => HostServerMode::Public,
    };
    let public_host = (c.mode == host::hostconfig::ServerMode::Public).then(|| c.host.clone());
    let database = c
        .database
        .as_ref()
        .and_then(|d| d.engine.clone())
        .unwrap_or_else(|| "sqlite".into());
    let fresh_start = c.database.as_ref().and_then(|d| d.fresh_start).unwrap_or(false)
        || matches!(mongo_migration.state, MongoMigrationState::Skipped);
    Ok(ServerConfigView {
        configured: true,
        name: c.name,
        mode,
        public_host,
        registration: c.registration.as_str().to_owned(),
        database,
        max_hubs: 1,
        max_hunts: c.max_hunt_servers,
        fresh_start,
        mongo_migration,
        config_file,
    })
}

#[cfg(feature = "server-gui")]
struct InProcessSetupEnv;

#[cfg(feature = "server-gui")]
impl host::commands::setup::SetupEnv for InProcessSetupEnv {
    fn platform(&self) -> host::hostconfig::Platform {
        host::hostconfig::Platform::current()
    }

    fn is_root(&self) -> Result<bool, host::HostError> {
        host::linux::is_root()
    }

    fn tailscale_ipv4(&self) -> Result<Option<String>, host::HostError> {
        Ok(host::doctor::tailscale_ipv4()?.or_else(|| Some("100.64.0.1".to_owned())))
    }

    fn find_mongod(&self) -> Result<Option<String>, host::HostError> {
        Ok(None)
    }

    fn find_on_path(&self, name: &str) -> Result<Option<String>, host::HostError> {
        host::wine::find_on_path(&[name])
    }

    fn source_commit(&self, repo: &Path) -> Result<String, host::HostError> {
        if let Some(release) = host::selfupdate::read_manifest(repo)? {
            return Ok(release.short_commit().to_owned());
        }
        Ok(String::new())
    }

    fn apply_preset(&self, paths: &host::paths::Paths) -> Result<Option<host::preset::PresetApplied>, host::HostError> {
        host::preset::apply_preset(paths, false)
    }

    fn install_services(
        &self,
        _layout: &host::commands::setup::paradox::Layout,
        _log_file: &Path,
        _out: &mut dyn FnMut(&str),
    ) -> Result<(), host::HostError> {
        Ok(())
    }

    fn make_certificate(
        &self,
        _layout: &host::commands::setup::paradox::Layout,
        hosts: &[String],
        tls: &Path,
        _log_file: &Path,
    ) -> Result<(), host::HostError> {
        let mut args = vec!["make-cert".to_owned()];
        for host in hosts {
            args.extend(["--host".to_owned(), host.clone()]);
        }
        args.extend([
            "--out".to_owned(),
            tls.to_string_lossy().into_owned(),
            "--json".to_owned(),
        ]);
        let code = host::service_main("gateway", &args);
        if code != std::process::ExitCode::SUCCESS {
            return Err(host::HostError::message("could not make the gateway certificate"));
        }
        Ok(())
    }

    fn gateway_program(&self) -> String {
        std::env::current_exe().map_or_else(|_| "dainty-lass-server-gui".to_owned(), |p| p.display().to_string())
    }

    fn require_checkout_sources(&self) -> bool {
        false
    }
}
