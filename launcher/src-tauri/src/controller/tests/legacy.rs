//! The legacy edition (`legacy` feature): the `gameEdition` setting, a Dauntless Revived server joined
//! from its invite or the saved list, and Dauntless 1.4.4 downloaded into its own folder (Reforged's
//! `revived` cases in `test/controller.test.ts`, on a small manifest and a stand-in release of the 1.4.4
//! DLL and the runtime loader).

use super::*;
use crate::bridge::types::{Game, InstallMode, ServerGame, SettingsPatch};
use crate::controller::legacy::LegacyConfig;
use crate::controller::under;
use crate::core::legacy_pins::{EXE_RELATIVE_PATH as LEGACY_EXE, EXPECTED_BUILD};
use crate::core::legacy_pins::{LOADER_INI, LOADER_INI_TEXT};
use crate::core::manifest::{GameManifest, ManifestFile};
use crate::platform::client_mods::{FetchBytes, Release};

const PAK: &str = "Archon/Content/Paks/pakchunk0-WindowsNoEditor.pak";

fn game_files() -> Vec<(String, Vec<u8>)> {
    vec![
        (PAK.into(), pattern(50_000, 3)),
        (LEGACY_EXE.into(), b"MZ this stands in for Dauntless 1.4.4".to_vec()),
    ]
}

fn manifest(files: &[(String, Vec<u8>)]) -> GameManifest {
    let mut files: Vec<ManifestFile> = files
        .iter()
        .map(|(path, data)| ManifestFile {
            path: path.clone(),
            size: data.len() as u64,
            sha256: sha(data),
        })
        .collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    GameManifest {
        build: EXPECTED_BUILD.into(),
        total_bytes: files.iter().map(|file| file.size).sum(),
        files,
    }
}

const DLLS: [(&str, &[u8]); 2] = [
    ("UndauntedInternalServer.dll", b"MZ the 1.4.4 dll"),
    ("winmm.dll", b"MZ the runtime loader"),
];

/// A stand-in for the maintainer's release, and its source.
fn release() -> Release {
    Release {
        dlls: DLLS
            .iter()
            .map(|(name, data)| ((*name).to_owned(), sha(data), data.len() as u64))
            .collect(),
        source: "https://release.invalid".into(),
    }
}

fn release_source(asked: Arc<Mutex<u32>>) -> FetchBytes {
    Arc::new(move |url, _| {
        *asked.lock().unwrap() += 1;
        let data = DLLS
            .iter()
            .find(|(name, _)| url.ends_with(&format!("/{name}")))
            .map(|(_, data)| data.to_vec());
        Box::pin(async move { data.ok_or_else(|| "404".to_owned()) })
    })
}

struct Legacy {
    hx: Harness,
    /// Where 1.4.4 goes by default (Dauntless Revived's own launcher's folder)
    default_dir: PathBuf,
    dlls_asked: Arc<Mutex<u32>>,
    _dirs: Vec<tempfile::TempDir>,
}

fn legacy_harness(dr_settings: Option<Value>) -> Legacy {
    let (resources, local) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let default_dir = local.path().join("DauntlessRevived").join("Game");
    let dr_file = local.path().join("Dauntless Revived Launcher").join("settings.json");
    if let Some(settings) = dr_settings {
        std::fs::create_dir_all(dr_file.parent().unwrap()).unwrap();
        std::fs::write(&dr_file, settings.to_string()).unwrap();
    }
    let dlls_asked = Arc::new(Mutex::new(0));
    let hx = Harness::with_legacy(
        tempfile::tempdir().unwrap(),
        LegacyConfig {
            resources_dir: resources.path().to_path_buf(),
            default_dir: Some(default_dir.clone()),
            dr_settings: Some(dr_file),
            manifest: Some(manifest(&game_files())),
            dll_fetch: Some(release_source(dlls_asked.clone())),
            release: Some(release()),
            relay_port: Some(free_port()),
        },
    );
    Legacy {
        hx,
        default_dir,
        dlls_asked,
        _dirs: vec![resources, local],
    }
}

fn revived_invite(port: u16, fp: &str, code: &str, name: &str) -> String {
    format_invite(&Invite {
        mode: ServerMode::Public,
        host: "127.0.0.1".into(),
        port,
        code: code.into(),
        name: name.into(),
        share: None,
        fp: Some(fp.into()),
        game: Game::Legacy,
    })
    .unwrap()
}

fn edition(game: Game) -> SettingsPatch {
    SettingsPatch {
        game_edition: Some(game),
        ..SettingsPatch::default()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_revived_invite_switches_the_edition_downloads_1_4_4_and_checks_it_before_play() {
    let mut host = Host::new("Revived Fridays");
    host.valid_codes.insert("ABCD-1234".into());
    host.game_files = Some(game_files());
    let (gw, port, fp) = gateway(host).await;
    let lx = legacy_harness(None);
    let hx = &lx.hx;
    hx.c.init().await;
    let s = hx.last();
    assert!(s.app.legacy_available);
    assert_eq!(s.settings.game_edition, Game::Reborn);

    // The reborn edition offers to switch instead of joining
    let text = revived_invite(port, &fp, "ABCD-1234", "Revived Fridays");
    assert!(matches!(hx.c.check_invite(&text), InviteCheck::Ok { .. }));
    assert_eq!(
        hx.c.submit_invite(&text, false, false).await,
        ActionResult::Err(LauncherError::with_detail(ErrorCode::EditionOther, "legacy"))
    );
    assert!(hx.last().server.is_none());
    assert_eq!(
        hx.c.set_settings(edition(Game::Legacy)).await.game_edition,
        Game::Legacy
    );
    assert_eq!(hx.c.submit_invite(&text, false, false).await, ActionResult::Ok);
    let s = hx.last();
    assert_eq!(s.server.as_ref().map(|sv| sv.game), Some(ServerGame::Revived));
    assert_eq!(s.phase, Phase::Register);
    assert!(s.revived.is_some());

    // DR's API: its routes and its header
    assert_eq!(
        hx.c.register("Borealis").await,
        RegisterOutcome::Ok {
            username: "Borealis".into()
        }
    );
    let seen = gw.lock().unwrap().seen.clone();
    for route in [
        "/undaunted/api/ServerStatus",
        "/undaunted/api/Register",
        "/undaunted/api/GetUserInfo",
    ] {
        assert!(seen.iter().any(|path| path == route), "{route}: {seen:?}");
    }
    assert!(
        !seen
            .iter()
            .any(|path| path.starts_with("/reforged/api/") || path.contains("/runtime/"))
    );

    // Nothing there yet: it goes into Dauntless Revived's own launcher's folder
    let s = hx.last();
    assert_eq!(s.phase, Phase::Locate);
    let info = s.revived.clone().unwrap();
    assert_eq!(info.problem, Some(GameProblem::NotSet));
    assert_eq!(info.root, lx.default_dir.to_string_lossy());
    assert!(!info.chosen);
    assert_eq!(info.total_bytes, 50_000 + 37);

    // The files come down and are checked; a dxgi.dll that is not DR's stops it before the DLLs, since it
    // could load the 1.4.4 DLL a second time
    let win64 = under(&info.root, "Archon/Binaries/Win64");
    std::fs::create_dir_all(&win64).unwrap();
    std::fs::write(win64.join("dxgi.dll"), b"someone else's dxgi.dll").unwrap();
    assert_eq!(
        hx.c.revived_install(InstallMode::Install).await,
        ActionResult::Err(LauncherError::with_detail(ErrorCode::LoaderLeftovers, "dxgi.dll"))
    );
    assert_eq!(*lx.dlls_asked.lock().unwrap(), 0);
    assert_eq!(std::fs::read(under(&info.root, PAK)).unwrap(), pattern(50_000, 3));
    assert!(!under(&info.root, &format!("{PAK}.part")).exists());
    let s = hx.last();
    assert!(s.task.is_none());
    let info = s.revived.unwrap();
    assert_eq!(
        (info.problem, info.missing_files, info.dlls_ok),
        (Some(GameProblem::Unverified), 0, false)
    );
    // Never checked to the end, so never played
    assert_eq!(
        hx.c.play().await,
        ActionResult::Err(LauncherError::with_detail(ErrorCode::GameFilesInvalid, "0"))
    );
    assert!(hx.spawns.lock().unwrap().is_empty());

    // Without it: the DLL and the loader, with the loader's list naming the DLL and nothing else (RD9)
    std::fs::remove_file(win64.join("dxgi.dll")).unwrap();
    assert_eq!(hx.c.revived_install(InstallMode::Install).await, ActionResult::Ok);
    assert_eq!(*lx.dlls_asked.lock().unwrap(), 2);
    for (name, data) in DLLS {
        assert_eq!(std::fs::read(win64.join(name)).unwrap(), data, "{name}");
    }
    assert_eq!(
        std::fs::read_to_string(win64.join(LOADER_INI)).unwrap(),
        LOADER_INI_TEXT
    );
    let s = hx.last();
    assert_eq!((s.phase, s.notice), (Phase::Ready, Some(NoticeCode::InstallDone)));
    assert!(
        s.revived
            .as_ref()
            .is_some_and(|info| info.dlls_ok && info.problem.is_none())
    );

    // PLAY: DR's command line through the relay, none of Reforged's runtime environment
    assert_eq!(hx.c.play().await, ActionResult::Ok);
    let (dir, args, env) = hx.spawns.lock().unwrap().last().cloned().unwrap();
    assert_eq!(dir, win64);
    let relay = hx.last().running.relay_port.unwrap();
    assert_eq!(args[0], format!("127.0.0.1:{relay}"));
    assert!(arg(&args, "-AUTH_PASSWORD=").is_some_and(|key| key.starts_with("RRK_")));
    assert!(args.iter().any(|a| a == "-RamsStore"));
    assert!(env_of(&env, "REFORGED_BACKEND_URL").is_empty());
    assert!(!dir.join("MysticParadox.dll").exists());
    hx.exit(0).await;
    // The server hears that the session ended
    for _ in 0..100 {
        if gw
            .lock()
            .unwrap()
            .seen
            .iter()
            .any(|path| path == "/undaunted/api/PlayingEnded")
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        gw.lock()
            .unwrap()
            .seen
            .iter()
            .any(|path| path == "/undaunted/api/PlayingEnded")
    );

    // Back to reborn: the server stays in the list, a switch away
    assert_eq!(
        hx.c.set_settings(edition(Game::Reborn)).await.game_edition,
        Game::Reborn
    );
    let s = hx.last();
    assert_eq!(s.phase, Phase::Join);
    assert!(s.revived.is_none());
    let saved = s.saved_servers[0].clone();
    assert_eq!((saved.game, saved.unsupported), (ServerGame::Revived, false));
    assert_eq!(
        hx.c.switch_server(&saved.id).await,
        ActionResult::Err(LauncherError::with_detail(ErrorCode::EditionOther, "legacy"))
    );
    let stored: Value = serde_json::from_slice(&std::fs::read(hx.user.path().join("settings.json")).unwrap()).unwrap();
    assert_eq!(stored["gameEdition"], "reborn");
    let root = Value::from(lx.default_dir.to_string_lossy().as_ref());
    assert_eq!((&stored["revivedRoot"], &stored["revivedVerified"]), (&root, &root));
    assert_eq!(stored["savedServers"][0]["game"], "revived");
    hx.c.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_1_4_4_folder_is_found_chosen_or_refused() {
    // Dauntless Revived's own launcher downloaded it: taken the first time
    let found = tempfile::tempdir().unwrap();
    for (path, data) in game_files() {
        let file = under(&found.path().to_string_lossy(), &path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, data).unwrap();
    }
    let lx = legacy_harness(Some(json!({ "installDir": found.path().to_string_lossy() })));
    let hx = &lx.hx;
    let mut host = Host::new("Revived Fridays");
    host.registration = "OPEN";
    let (_gw, port, fp) = gateway(host).await;
    hx.c.init().await;
    hx.c.set_settings(edition(Game::Legacy)).await;
    let text = revived_invite(port, &fp, "OPEN", "Revived Fridays");
    assert_eq!(hx.c.submit_invite(&text, false, false).await, ActionResult::Ok);
    let info = hx.last().revived.unwrap();
    assert_eq!(info.root, found.path().to_string_lossy());
    assert!(info.chosen);
    assert_eq!(info.detected.len(), 1);
    assert_eq!(
        info.problem,
        Some(GameProblem::Unverified),
        "there, but not checked by hash yet"
    );

    // A folder with something else in it, or another Dauntless, is never written into
    let busy = hx.temp();
    std::fs::write(busy.join("notes.txt"), b"mine").unwrap();
    hx.pick("revived", &busy);
    assert_eq!(hx.c.revived_choose_folder().await, error(ErrorCode::FolderNotEmpty));
    hx.pick("revived", &hx.game());
    assert_eq!(hx.c.revived_choose_folder().await, error(ErrorCode::FolderOtherGame));
    // An empty one is used as is
    let empty = hx.temp();
    hx.pick("revived", &empty);
    assert_eq!(hx.c.revived_choose_folder().await, ActionResult::Ok);
    let info = hx.last().revived.unwrap();
    assert_eq!(
        (info.root.as_str(), info.problem),
        (&*empty.to_string_lossy(), Some(GameProblem::NotSet))
    );
    // And back to the one found
    assert_eq!(hx.c.revived_use_detected(0).await, ActionResult::Ok);
    assert_eq!(hx.last().revived.unwrap().root, found.path().to_string_lossy());
    assert_eq!(hx.c.revived_use_detected(5).await, error(ErrorCode::FolderInvalid));
    hx.c.shutdown().await;
}
