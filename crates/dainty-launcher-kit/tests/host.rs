// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The Host page (RD27 spec §2.5, §2.10) through the fake host kit: setup, the server's start, invite and stop,
//! the jobs' failures, the workspace's refusals, multipliers, the SDK import, a Dumper-7 session, and the
//! player-only build's refusals. Its own test binary, because it logs to the process-wide log.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]
#![cfg_attr(
    not(feature = "host"),
    allow(
        dead_code,
        unused_imports,
        reason = "the player-only build's test needs only the harness"
    )
)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use futures_util::future::BoxFuture;
use tokio::sync::Notify;

use dainty_launcher_kit::bridge::types::{
    ActionResult, ErrorCode, HostAction, HostFolder, HostJobKind, HostJobState, HostKit, HostMultipliers,
    HostServerMode, HostServerState, HostServerView, HostSetupRequest, NoticeCode, SettingsPatch, Snapshot,
};
use dainty_launcher_kit::controller::{
    Controller, ControllerConfig, FileFilter, FolderPurpose, Platform, PreparedGame, RelayOwner, Seams, ServerProbe,
};
use dainty_launcher_kit::core::i18n::Language;
use dainty_launcher_kit::core::pins::sha256_hex;
use dainty_launcher_kit::platform::game_folder::StoreFolders;
use dainty_launcher_kit::platform::hostkit::{BACKEND_GAME_DATA, DIRECTOR_GAME_DATA, HOST_BINARY, WorkspacePaths};
use dainty_launcher_kit::platform::hostserver::HostTool;
use dainty_launcher_kit::platform::keystore::os::{Encryptor, SealError, open_rr1, seal_rr1};
use dainty_launcher_kit::platform::launch::{GameChild, LaunchPlan, MissingFiles, Spawner};
use dainty_launcher_kit::platform::log;
use dainty_launcher_kit::platform::paths::Paths;
use dainty_launcher_kit::platform::settings::Os;

const FAKE: &str = env!("CARGO_BIN_EXE_fake-host-kit");
const INVITE_LINK: &str = "dauntless://fake-host-kit-invite-secret";
const VERSION_TEXT: &str = "dauntless_rel-1.14.7_Shipping_2024-04-01_14-59-21_647472";
const WAIT: Duration = Duration::from_secs(15);

fn log_dir() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("launcher-kit-host-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        log::set_folder(&dir);
        dir
    })
}

// --- Fakes for the app ---

#[derive(Default)]
struct FakePlatform {
    folder: Mutex<Option<PathBuf>>,
    dumper: Mutex<Option<PathBuf>>,
    kits: Mutex<Vec<HostKit>>,
    servers: Mutex<Vec<HostServerView>>,
    opened: Mutex<Vec<PathBuf>>,
}

impl FakePlatform {
    fn last_kit(&self) -> HostKit {
        self.kits.lock().unwrap().last().cloned().expect("a host-kit event")
    }
}

impl Platform for FakePlatform {
    fn choose_folder(&self, _: Language, _: FolderPurpose, _: Option<PathBuf>) -> BoxFuture<'_, Option<PathBuf>> {
        let picked = self.folder.lock().unwrap().clone();
        Box::pin(async move { picked })
    }

    fn choose_save_file(&self, _: Language, _: String, _: FileFilter) -> BoxFuture<'_, Option<PathBuf>> {
        Box::pin(async { None })
    }

    fn choose_open_file(&self, _: Language, _: FileFilter) -> BoxFuture<'_, Option<PathBuf>> {
        Box::pin(async { None })
    }

    fn choose_dumper_dll(&self, _: Language) -> BoxFuture<'_, Option<PathBuf>> {
        let picked = self.dumper.lock().unwrap().clone();
        Box::pin(async move { picked })
    }

    fn open_url(&self, _: &url::Url) -> std::io::Result<()> {
        Ok(())
    }

    fn open_folder(&self, dir: &Path) -> std::io::Result<bool> {
        self.opened.lock().unwrap().push(dir.to_path_buf());
        Ok(true)
    }

    fn emit_snapshot(&self, _: &Snapshot) {}

    fn emit_host_kit(&self, kit: &HostKit) {
        self.kits.lock().unwrap().push(kit.clone());
    }

    fn emit_host_server(&self, view: &HostServerView) {
        self.servers.lock().unwrap().push(view.clone());
    }

    fn find_tailscale(&self) -> Option<PathBuf> {
        None
    }

    fn running_clients(&self) -> BoxFuture<'_, Vec<u32>> {
        Box::pin(async { Vec::new() })
    }

    fn prepare_launch<'a>(&'a self, _: &'a Path, _: bool) -> BoxFuture<'a, PreparedGame> {
        Box::pin(async { PreparedGame::Native })
    }

    fn missing_files(&self) -> MissingFiles {
        MissingFiles::default()
    }
}

struct FakeSealer;

impl Encryptor for FakeSealer {
    fn available(&self) -> bool {
        true
    }

    fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, SealError> {
        seal_rr1(&[7; 32], plain)
    }

    fn open(&self, file: &[u8]) -> Result<Vec<u8>, SealError> {
        open_rr1(&[7; 32], file)
    }
}

/// A game that runs until the test lets it exit.
#[derive(Default)]
struct FakeSpawner {
    plans: Mutex<Vec<LaunchPlan>>,
    exit: Arc<Notify>,
}

struct FakeChild(Arc<Notify>);

impl GameChild for FakeChild {
    fn id(&self) -> Option<u32> {
        Some(5151)
    }

    fn wait(self: Box<Self>) -> BoxFuture<'static, std::io::Result<Option<i32>>> {
        Box::pin(async move {
            self.0.notified().await;
            Ok(Some(0))
        })
    }
}

impl Spawner for FakeSpawner {
    fn spawn(&self, plan: &LaunchPlan) -> std::io::Result<Box<dyn GameChild>> {
        self.plans.lock().unwrap().push(plan.clone());
        Ok(Box::new(FakeChild(Arc::clone(&self.exit))))
    }
}

// --- One host's PC ---

struct Pc {
    dir: tempfile::TempDir,
    root: PathBuf,
    win64: PathBuf,
    project: PathBuf,
    dumps: PathBuf,
    pin: String,
}

fn pc() -> Pc {
    log_dir();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Dauntless");
    let win64 = root.join("Archon/Binaries/Win64");
    std::fs::create_dir_all(&win64).unwrap();
    let exe = b"a stand-in for the 1.14.7 executable";
    std::fs::write(win64.join("Dauntless-Win64-Shipping.exe"), exe).unwrap();
    std::fs::write(root.join("Version.txt"), VERSION_TEXT).unwrap();
    Pc {
        project: dir.path().join("Reforged"),
        dumps: dir.path().join("Dumper-7"),
        root,
        win64,
        pin: sha256_hex(exe),
        dir,
    }
}

/// A host kit checkout beside the game, as `is_workspace` wants it.
fn make_workspace(dir: &Path) {
    std::fs::create_dir_all(dir.join("modules/rustic-isles")).unwrap();
    std::fs::copy(FAKE, dir.join(HOST_BINARY)).unwrap();
}

fn launcher(pc: &Pc, platform: &Arc<FakePlatform>, spawner: &Arc<FakeSpawner>, mode: Option<&str>) -> Controller {
    let paths = Paths {
        user_data: pc.dir.path().join("User Data"),
        documents: pc.dir.path().join("Documents"),
        home: pc.dir.path().join("User Data"),
    };
    let mut config = ControllerConfig::new(paths, Arc::new(FakeSealer));
    config.exe_pin = pc.pin.clone();
    config.game_config_dir = pc.dir.path().join("Config");
    config.os = Os::current();
    let probe: ServerProbe = Arc::new(|| Box::pin(async { false }));
    let seams = Seams {
        spawner: Some(Arc::clone(spawner) as Arc<dyn Spawner>),
        game_env: Some(BTreeMap::from([
            ("PATH".to_owned(), "/usr/bin".to_owned()),
            ("DAINTY_BACKEND_URL".to_owned(), "http://127.0.0.1:1".to_owned()),
            ("rustic_isles_runtime_profile".to_owned(), "development".to_owned()),
        ])),
        store_folders: Some(StoreFolders {
            legendary: Vec::new(),
            heroic: None,
            epic_manifests: None,
        }),
        qos_port: None,
        relay_owner: RelayOwner::Anyone,
        host_server_probe: Some(probe),
        dumper_root: pc.dumps.clone(),
        host_tool: Some(HostTool {
            program: PathBuf::from(FAKE),
            prefix: mode.map(OsString::from).into_iter().collect(),
        }),
        host_stop_timeout: Duration::from_secs(2),
    };
    Controller::new(config, Arc::clone(platform) as Arc<dyn Platform>, seams)
}

async fn until(what: &str, mut check: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + WAIT;
    while !check() {
        assert!(tokio::time::Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn ok(result: &ActionResult) {
    assert_eq!(*result, ActionResult::Ok);
}

fn refused(result: &ActionResult) -> (ErrorCode, Option<String>) {
    match result {
        ActionResult::Err(error) => (error.code, error.detail.clone()),
        ActionResult::Ok => panic!("expected a refusal"),
    }
}

fn host_failed(result: &ActionResult) -> String {
    let (code, detail) = refused(result);
    assert_eq!(code, ErrorCode::HostFailed, "{result:?}");
    detail.unwrap_or_default()
}

fn private_setup() -> HostSetupRequest {
    HostSetupRequest {
        name: "Test Hunts".to_owned(),
        mode: HostServerMode::Private,
        public_host: None,
    }
}

fn host_toml(workspace: &Path, mode: &str) {
    let file = WorkspacePaths::new(workspace).host_toml;
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, format!("[server]\nname = \"Test Hunts\"\nmode = \"{mode}\"\n")).unwrap();
}

fn game_data(workspace: &Path) {
    let paths = WorkspacePaths::new(workspace);
    for (dir, files) in [
        (&paths.backend_game_data, &BACKEND_GAME_DATA),
        (&paths.director_game_data, &DIRECTOR_GAME_DATA),
    ] {
        std::fs::create_dir_all(dir).unwrap();
        for name in files {
            std::fs::write(dir.join(name), b"{\"real\":true}").unwrap();
        }
    }
}

/// A launcher whose game is set and whose workspace is the project folder beside it.
async fn host_ready(pc: &Pc, platform: &Arc<FakePlatform>, mode: Option<&str>) -> Controller {
    let spawner = Arc::new(FakeSpawner::default());
    make_workspace(&pc.project);
    let c = launcher(pc, platform, &spawner, mode);
    c.init().await;
    ok(&c.use_game_path(&pc.root.to_string_lossy()).await);
    c.set_settings(SettingsPatch {
        host_tools: Some(true),
        ..SettingsPatch::default()
    })
    .await;
    c
}

fn assert_send<T: Send>(_: &T) {}

#[cfg(feature = "host")]
mod host {
    use super::*;

    #[tokio::test]
    async fn setup_start_invite_and_stop_through_the_fake() {
        let pc = pc();
        let platform = Arc::new(FakePlatform::default());
        let c = host_ready(&pc, &platform, None).await;

        // Turning the Host tools on took the project folder beside the game.
        let kit = c.host_kit().await;
        assert!(kit.workspace_ok);
        assert_eq!(kit.workspace.as_deref(), Some(&*pc.project.to_string_lossy()));
        assert_eq!(kit.project_target.as_deref(), Some(&*pc.project.to_string_lossy()));
        assert_eq!(kit.requirements.len(), 6);

        // The fake has no `setup`: the job fails with its `Error:` line.
        let detail = host_failed(&c.host_setup(private_setup()).await);
        assert!(detail.contains("no `setup"), "{detail}");
        let job = platform.last_kit().job.unwrap();
        assert_eq!((job.kind, job.state), (HostJobKind::Setup, HostJobState::Failed));
        assert!(job.log_file.unwrap().ends_with("launcher-setup.log"));

        // Start needs a configured server with game data.
        assert_eq!(host_failed(&c.host_server_start().await), "not_configured");
        host_toml(&pc.project, "private");
        assert_eq!(host_failed(&c.host_server_start().await), "gamedata_missing");
        game_data(&pc.project);

        // No invite before the server runs.
        assert_eq!(host_failed(&c.host_create_invite(None).await), "server_not_running");

        let start = c.host_server_start();
        assert_send(&start);
        ok(&start.await);
        until("the server to be ready", || {
            c.host_server_view().state == HostServerState::Running
        })
        .await;
        assert!(
            platform
                .servers
                .lock()
                .unwrap()
                .iter()
                .any(|v| v.state == HostServerState::Starting)
        );

        // Setup refuses while the server runs.
        assert_eq!(host_failed(&c.host_setup(private_setup()).await), "server_running");

        ok(&c.host_create_invite(Some("Alice".to_owned())).await);
        let invite = c.host_server_view().invite.unwrap();
        assert_eq!(invite.link, INVITE_LINK);
        assert_eq!(invite.for_name.as_deref(), Some("Alice"));
        assert!(!c.host_server_view().lines.iter().any(|line| line.contains(INVITE_LINK)));

        ok(&c.host_server_stop().await);
        until("the server to stop", || {
            c.host_server_view().state == HostServerState::Stopped
        })
        .await;
        assert!(c.host_server_view().lines.iter().any(|line| line.contains("Stopped.")));

        // The invite is a secret: never in the launcher's log, nor in the host kit's logs.
        let launcher_log = std::fs::read_to_string(log_dir().join(log::LOG_FILE)).unwrap_or_default();
        assert!(launcher_log.contains("host: invite created with a note"));
        assert!(!launcher_log.contains(INVITE_LINK));
        let invite_log = WorkspacePaths::new(&pc.project).launcher_log("invite");
        assert!(
            !std::fs::read_to_string(invite_log)
                .unwrap_or_default()
                .contains(INVITE_LINK)
        );
        c.shutdown().await;
    }

    #[tokio::test]
    async fn a_failing_start_and_a_deaf_server() {
        let pc = pc();
        let platform = Arc::new(FakePlatform::default());
        let c = host_ready(&pc, &platform, Some("fail")).await;
        host_toml(&pc.project, "private");
        game_data(&pc.project);
        ok(&c.host_server_start().await);
        until("the start to fail", || {
            c.host_server_view().state == HostServerState::Failed
        })
        .await;
        assert_eq!(
            c.host_server_view().message.as_deref(),
            Some("the backend did not start")
        );

        let pc = self::pc();
        let platform = Arc::new(FakePlatform::default());
        let c = host_ready(&pc, &platform, Some("deaf")).await;
        host_toml(&pc.project, "private");
        game_data(&pc.project);
        ok(&c.host_server_start().await);
        until("the deaf server to be ready", || {
            c.host_server_view().state == HostServerState::Running
        })
        .await;
        ok(&c.host_server_stop().await);
        until("the deaf server to be ended", || {
            c.host_server_view().state == HostServerState::Stopped
        })
        .await;
        assert!(
            c.host_server_view()
                .lines
                .iter()
                .any(|line| line.contains("did not stop in time"))
        );
        assert!(!c.host_server_view().lines.iter().any(|line| line.contains("Stopped.")));
    }

    #[tokio::test]
    async fn the_jobs_failures() {
        let pc = pc();
        let platform = Arc::new(FakePlatform::default());
        let c = host_ready(&pc, &platform, None).await;

        // Without the runtime source the host kit fetches it first; the fake can't.
        let detail = host_failed(&c.host_action(HostAction::SyntheticData).await);
        assert!(detail.contains("no `upstream"), "{detail}");
        let job = platform.last_kit().job.unwrap();
        assert_eq!((job.kind, job.state), (HostJobKind::Upstream, HostJobState::Failed));

        // An update whose check fails says why.
        let detail = host_failed(&c.host_update().await);
        assert!(detail.contains("no `update --check --json"), "{detail}");

        // The game has no exports yet.
        assert_eq!(host_failed(&c.host_action(HostAction::CopyExports).await), "no_exports");
        let exports = pc.win64.join("Items_Analysis");
        std::fs::create_dir_all(exports.join("Hunts")).unwrap();
        std::fs::write(exports.join("catalog.json"), b"{}").unwrap();
        std::fs::write(exports.join("Hunts/hunts.json"), b"{}").unwrap();
        assert!(c.host_kit().await.exports.in_game);
        ok(&c.host_action(HostAction::CopyExports).await);
        let job = platform.last_kit().job.unwrap();
        assert_eq!((job.kind, job.state), (HostJobKind::CopyExports, HostJobState::Done));
        assert_eq!(job.message.as_deref(), Some("2 files"));
        assert!(
            WorkspacePaths::new(&pc.project)
                .exports
                .join("Hunts/hunts.json")
                .is_file()
        );
        assert_eq!(c.snapshot().notice, Some(NoticeCode::HostDone));

        // A folder that is not there is refused; one that is opens.
        assert_eq!(host_failed(&c.host_open(HostFolder::GamedataPacks)), "folder");
        ok(&c.host_open(HostFolder::Workspace));
        assert_eq!(platform.opened.lock().unwrap().last(), Some(&pc.project));
    }

    #[tokio::test]
    async fn the_workspaces_refusals() {
        let pc = pc();
        let platform = Arc::new(FakePlatform::default());
        let spawner = Arc::new(FakeSpawner::default());
        let c = launcher(&pc, &platform, &spawner, None);
        c.init().await;

        // No game, no project target; quiet never refuses.
        assert_eq!(host_failed(&c.host_download_project().await), "project_no_game");
        ok(&c.ensure_host_workspace(true).await);
        assert_eq!(
            host_failed(&c.host_action(HostAction::SyntheticData).await),
            "workspace"
        );
        assert_eq!(host_failed(&c.host_server_start().await), "workspace");

        ok(&c.use_game_path(&pc.root.to_string_lossy()).await);
        assert_eq!(host_failed(&c.host_download_project().await), "project_missing");
        std::fs::create_dir_all(&pc.project).unwrap();
        assert_eq!(host_failed(&c.host_download_project().await), "project_folder_taken");
        ok(&c.ensure_host_workspace(true).await);

        // The picker: cancelled, then a folder that is no host kit.
        assert_eq!(refused(&c.host_choose_workspace().await).0, ErrorCode::Cancelled);
        *platform.folder.lock().unwrap() = Some(pc.project.clone());
        assert_eq!(host_failed(&c.host_choose_workspace().await), "workspace");
        make_workspace(&pc.project);
        ok(&c.host_choose_workspace().await);
        assert!(platform.last_kit().workspace_ok);
        ok(&c.host_download_project().await);

        // Dumper-7's DLL must be a DLL.
        assert_eq!(refused(&c.host_choose_dumper().await).0, ErrorCode::Cancelled);
        let text = pc.dir.path().join("Dumper-7.txt");
        std::fs::write(&text, b"not a dll").unwrap();
        *platform.dumper.lock().unwrap() = Some(text);
        assert_eq!(host_failed(&c.host_choose_dumper().await), "dumper");
        let dll = pc.dir.path().join("Dumper-7.dll");
        std::fs::write(&dll, b"MZ").unwrap();
        *platform.dumper.lock().unwrap() = Some(dll.clone());
        ok(&c.host_choose_dumper().await);
        assert_eq!(platform.last_kit().dumper_dll.as_deref(), Some(&*dll.to_string_lossy()));
    }

    #[tokio::test]
    async fn multipliers_are_for_private_servers() {
        let pc = pc();
        let platform = Arc::new(FakePlatform::default());
        let c = host_ready(&pc, &platform, None).await;
        let rates = Some(HostMultipliers { xp: 2.5, rewards: 3.0 });

        assert_eq!(host_failed(&c.host_set_multipliers(rates).await), "not_configured");
        host_toml(&pc.project, "public");
        assert_eq!(host_failed(&c.host_set_multipliers(rates).await), "multipliers_public");
        assert_eq!(host_failed(&c.host_set_multipliers(None).await), "multipliers_public");

        // A private server reaches the host kit, which the fake refuses with the arguments it got.
        host_toml(&pc.project, "private");
        let detail = host_failed(&c.host_set_multipliers(rates).await);
        assert!(detail.contains("multipliers --xp 2.5 --rewards 3"), "{detail}");
        let job = platform.last_kit().job.unwrap();
        assert_eq!((job.kind, job.state), (HostJobKind::Multipliers, HostJobState::Failed));
        let detail = host_failed(&c.host_set_multipliers(None).await);
        assert!(detail.contains("multipliers --off"), "{detail}");
    }

    fn dump(root: &Path, name: &str, functions: &[&str], age: Duration) {
        let sdk = root.join(name).join("CppSDK");
        std::fs::create_dir_all(sdk.join("SDK")).unwrap();
        for function in functions {
            std::fs::write(sdk.join("SDK").join(format!("{function}_functions.cpp")), b"// fn").unwrap();
        }
        let header = sdk.join("SDK.hpp");
        std::fs::write(&header, b"// header").unwrap();
        let file = std::fs::File::options().write(true).open(&header).unwrap();
        file.set_modified(SystemTime::now() - age).unwrap();
    }

    #[tokio::test]
    async fn import_sdk_takes_the_newest_dump() {
        let pc = pc();
        let platform = Arc::new(FakePlatform::default());
        let c = host_ready(&pc, &platform, None).await;
        let runtime = WorkspacePaths::new(&pc.project).runtime_source;
        std::fs::create_dir_all(&runtime).unwrap();
        std::fs::write(runtime.join("dllmain.cpp"), b"// main").unwrap();

        assert_eq!(host_failed(&c.host_action(HostAction::ImportSdk).await), "no_dump");

        dump(&pc.dumps, "4.27-Older", &["Old"], Duration::from_secs(3600));
        dump(&pc.dumps, "4.27-Newer", &["New", "Other"], Duration::from_secs(60));
        dump(&pc.dumps, "4.27-Oldest", &["Ancient"], Duration::from_secs(7200));
        ok(&c.host_action(HostAction::ImportSdk).await);
        let job = platform.last_kit().job.unwrap();
        assert_eq!((job.kind, job.state), (HostJobKind::ImportSdk, HostJobState::Done));
        assert_eq!(job.message.as_deref(), Some("2 files"));
        assert!(runtime.join("SDK/New_functions.cpp").is_file());
        assert!(!runtime.join("SDK/Old_functions.cpp").exists());
        assert_eq!(c.snapshot().notice, Some(NoticeCode::SdkImported));
        assert_eq!(c.host_kit().await.sdk.function_files, 2);

        // The picked folder: a dump's own, its `CppSDK` found inside.
        *platform.folder.lock().unwrap() = Some(pc.dumps.join("4.27-Older"));
        ok(&c.host_import_sdk_folder().await);
        assert!(runtime.join("SDK/Old_functions.cpp").is_file());
        assert_eq!(platform.last_kit().job.unwrap().message.as_deref(), Some("1 files"));
        *platform.folder.lock().unwrap() = Some(pc.dumps.clone());
        assert_eq!(host_failed(&c.host_import_sdk_folder().await), "sdk_folder");
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn a_dumper_7_session_runs_the_game_as_shipped() {
        let pc = pc();
        let platform = Arc::new(FakePlatform::default());
        let spawner = Arc::new(FakeSpawner::default());
        make_workspace(&pc.project);
        let c = launcher(&pc, &platform, &spawner, None);
        c.init().await;
        ok(&c.use_game_path(&pc.root.to_string_lossy()).await);
        std::fs::write(pc.win64.join("winmm.dll"), b"loader").unwrap();
        std::fs::write(pc.win64.join("Rustic-Isles.dll"), b"runtime").unwrap();

        assert_eq!(host_failed(&c.host_action(HostAction::InjectDumper).await), "dumper");
        let dll = pc.dir.path().join("Dumper-7.dll");
        std::fs::write(&dll, b"MZ dumper").unwrap();
        *platform.dumper.lock().unwrap() = Some(dll);
        ok(&c.host_choose_dumper().await);

        ok(&c.host_action(HostAction::InjectDumper).await);
        let plan = spawner.plans.lock().unwrap().last().cloned().unwrap();
        assert_eq!(plan.args, ["-NoEAC", "-EpicPortal", "-DisableEOS"].map(OsString::from));
        assert_eq!(plan.env.keys().collect::<Vec<_>>(), ["PATH"]);
        assert!(pc.win64.join("mystic_loader.ini").is_file());
        assert!(pc.win64.join("Dumper-7.dll").is_file());
        assert!(!pc.win64.join("Rustic-Isles.dll").exists());
        let job = platform.last_kit().job.unwrap();
        assert_eq!((job.kind, job.state), (HostJobKind::Inject, HostJobState::Done));
        assert_eq!(
            host_failed(&c.host_action(HostAction::InjectDumper).await),
            "game_running"
        );

        // The game's exit ends the session: the runtime is back and nothing is left beside the loader.
        c.dismiss_error();
        spawner.exit.notify_one();
        until("the session to end", || !pc.win64.join("mystic_loader.ini").exists()).await;
        until("the runtime to come back", || {
            pc.win64.join("Rustic-Isles.dll").is_file()
        })
        .await;
        assert!(!pc.win64.join("Dumper-7.dll").exists());
        assert!(c.snapshot().last_error.is_none());
    }
}

#[cfg(not(feature = "host"))]
mod player_only {
    use super::*;

    #[tokio::test]
    async fn every_host_command_is_refused() {
        let pc = pc();
        let platform = Arc::new(FakePlatform::default());
        let c = host_ready(&pc, &platform, None).await;
        assert!(!c.snapshot().app.host_tools_available);
        assert!(!c.snapshot().settings.host_tools);
        ok(&c.ensure_host_workspace(false).await);

        let kit = c.host_kit().await;
        assert!(kit.workspace.is_none() && !kit.workspace_ok && kit.requirements.is_empty());
        assert_eq!(c.host_server_view().state, HostServerState::Stopped);

        let answers = [
            c.host_choose_workspace().await,
            c.host_choose_dumper().await,
            c.host_import_gamedata().await,
            c.host_import_sdk_folder().await,
            c.host_download_project().await,
            c.host_action(HostAction::ImportSdk).await,
            c.host_open(HostFolder::Workspace),
            c.host_server_start().await,
            c.host_server_stop().await,
            c.host_server_restart("backend").await,
            c.host_setup(private_setup()).await,
            c.host_create_invite(None).await,
            c.host_update().await,
            c.host_set_multipliers(None).await,
        ];
        for answer in &answers {
            assert_eq!(host_failed(answer), "host_tools_unavailable");
        }
        assert!(platform.kits.lock().unwrap().is_empty());
        assert_eq!(
            c.snapshot().last_error.and_then(|error| error.detail).as_deref(),
            Some("host_tools_unavailable")
        );
    }
}
