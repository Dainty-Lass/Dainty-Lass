//! dainty-launcher-kit's game folder rules (spec §2.7.7, rows L30, RA9), as the shell's `platform/game_folder.rs` tested
//! them: Reforged's `game.test.ts` cases for the folder input, the layouts, `Version.txt` and the store manifests.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::path::{Path, PathBuf};

use dainty_launcher_kit::bridge::types::{DetectedGame, GameSource};
use dainty_launcher_kit::platform::game_folder::{
    LocateError, MAX_ROOT_UNITS, SUPPORTED_VERSION, StoreFolders, StoreInstall, candidates, detect_in,
    is_supported_version, locate, normalize, parse_epic_item, parse_legendary_installed, read_version_text,
};
use serde_json::json;

const EXE_NAME: &str = "Dauntless-Win64-Shipping.exe";

/// `game.test.ts`'s `fakeGame`: a game root with the exe and, unless None, a `Version.txt` with a byte order mark
/// and a line break.
fn fake_game(dir: &Path, version: Option<&str>) -> PathBuf {
    let root = dir.join("Dauntless");
    let win64 = root.join("Archon").join("Binaries").join("Win64");
    std::fs::create_dir_all(&win64).unwrap();
    std::fs::write(win64.join(EXE_NAME), b"MZ fake").unwrap();
    if let Some(version) = version {
        std::fs::write(root.join("Version.txt"), format!("\u{feff}{version}\r\n")).unwrap();
    }
    root
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// `game.test.ts`: "cleanGameFolderInput: drive-letter paths only on Windows, quotes from Copy as path removed"
#[test]
fn clean_game_folder_input_takes_drive_letter_paths_only_on_windows() {
    let win = |input| normalize(input, true);
    assert_eq!(win("\"C:\\Games\\Dauntless\"").as_deref(), Some("C:\\Games\\Dauntless"));
    assert_eq!(win("C:/Games/Dauntless").as_deref(), Some("C:\\Games\\Dauntless"));
    assert_eq!(win("\\\\server\\share\\Dauntless"), None, "UNC paths are never touched");
    assert_eq!(win("\\\\?\\C:\\Games"), None);
    assert_eq!(win("\\Games"), None);
    assert_eq!(win("Games\\Dauntless"), None);
    assert_eq!(win(""), None);
    assert_eq!(win("C:\\a\0b"), None);
    assert_eq!(
        normalize("/home/me/Dauntless", false).as_deref(),
        Some("/home/me/Dauntless")
    );
    assert_eq!(normalize("relative/path", false), None);
}

/// `game.test.ts`: "gameRootCandidates: the root, a Dauntless subfolder, Archon's parent, three up from Win64 or
/// the exe"
#[test]
fn game_root_candidates_cover_the_known_layouts() {
    let windows = |folder| candidates(folder, true).unwrap();
    assert_eq!(
        windows("C:\\Games\\Dauntless"),
        ["C:\\Games\\Dauntless", "C:\\Games\\Dauntless\\Dauntless"]
    );
    for folder in [
        "C:\\Games\\Dauntless\\Archon",
        "C:\\Games\\Dauntless\\Archon\\Binaries\\Win64",
        "C:\\Games\\Dauntless\\Archon\\Binaries\\Win64\\Dauntless-Win64-Shipping.exe",
        "C:\\Games\\Dauntless\\Archon\\Binaries\\Win64\\DAUNTLESS-WIN64-SHIPPING.EXE",
    ] {
        assert!(windows(folder).contains(&"C:\\Games\\Dauntless".to_owned()), "{folder}");
    }
    assert_eq!(candidates("/", false).unwrap(), ["/", "/Dauntless"], "no duplicates");
}

/// `game.test.ts`: "locateExistingGame finds the game from any of the layouts"
#[test]
fn locate_existing_game_finds_the_game_from_any_layout() {
    let dir = tempfile::tempdir().unwrap();
    let root = fake_game(dir.path(), Some(SUPPORTED_VERSION));
    let win64 = root.join("Archon").join("Binaries").join("Win64");
    for input in [
        root.clone(),
        dir.path().to_path_buf(),
        root.join("Archon"),
        win64.clone(),
        win64.join(EXE_NAME),
    ] {
        assert_eq!(locate(&text(&input)), Ok(root.clone()), "{input:?}");
    }
    assert_eq!(locate(&format!("\"{}\"", text(&root))), Ok(root.clone()));
    assert_eq!(locate(&text(&root.join("nothing-here"))), Err(LocateError::NotFound));
    assert_eq!(locate("not absolute"), Err(LocateError::Invalid));
}

#[test]
fn a_root_past_the_limit_is_too_long() {
    let dir = tempfile::tempdir().unwrap();
    let mut deep = dir.path().to_path_buf();
    while text(&deep).len() <= MAX_ROOT_UNITS {
        deep = deep.join("long");
    }
    let root = fake_game(&deep, None);
    assert_eq!(locate(&text(&root)), Err(LocateError::TooLong));
}

/// `game.test.ts`: "Version.txt: 1.14.7 (CL 647472) is accepted, other builds are not"
#[test]
fn version_txt_accepts_1_14_7_only() {
    assert!(is_supported_version(SUPPORTED_VERSION));
    assert!(is_supported_version(
        "dauntless_rel-1.14.7_Shipping_2024-04-02_10-00-00_647472"
    ));
    assert!(is_supported_version("DAUNTLESS_REL-1.14.7 647472"));
    assert!(!is_supported_version(
        "dauntless_rel-1.4.4_Shipping_2020-10-28_20-11-15_239827"
    ));
    assert!(!is_supported_version(
        "dauntless_rel-1.14.7_Shipping_2024-04-01_14-59-21_6474720"
    ));
    assert!(!is_supported_version("dauntless_rel-1.14.6_Shipping_2024-03-01_647472"));
    // The shell's VersionInfo: the text when readable, and whether it is the supported build
    let version = |content: Option<&str>| {
        let dir = tempfile::tempdir().unwrap();
        let text = read_version_text(&fake_game(dir.path(), content));
        let ok = text.as_deref().is_some_and(is_supported_version);
        (text, ok)
    };
    assert_eq!(version(Some(SUPPORTED_VERSION)), (Some(SUPPORTED_VERSION.into()), true));
    assert_eq!(
        version(Some("dauntless_rel-1.12.0_Shipping_x_600000")),
        (Some("dauntless_rel-1.12.0_Shipping_x_600000".into()), false)
    );
    assert_eq!(version(None), (None, false));
    assert_eq!(version(Some("bad\u{7}text")), (None, false));
    assert_eq!(version(Some(&"x".repeat(129))), (None, false));
}

fn install(path: &str, version: Option<&str>) -> StoreInstall {
    StoreInstall {
        path: path.into(),
        version: version.map(str::to_owned),
    }
}

/// `game.test.ts`: "store manifests: Legendary's installed.json and the Epic Games Launcher's .item files"
#[test]
fn store_manifests() {
    let bytes = |value: serde_json::Value| value.to_string().into_bytes();
    let installed = json!({
        "Jackal": { "app_name": "Jackal", "install_path": "D:\\Games\\Dauntless", "version": "1.14.7" },
        "Other": { "app_name": "Fortnite", "install_path": "D:\\Games\\Fortnite" },
        "Broken": { "app_name": "Jackal", "install_path": 7 },
    });
    assert_eq!(
        parse_legendary_installed(&bytes(installed)),
        [install("D:\\Games\\Dauntless", Some("1.14.7"))]
    );
    assert!(parse_legendary_installed(b"null").is_empty());
    assert_eq!(
        parse_legendary_installed(&bytes(
            json!({ "Jackal": { "install_path": "D:\\D", "version": "v".repeat(70) } })
        ))[0]
            .version
            .as_deref(),
        Some("v".repeat(64).as_str()),
        "the key names the app when app_name is missing; the version is cut to 64"
    );
    assert_eq!(
        parse_epic_item(&bytes(json!({
            "AppName": "Jackal",
            "InstallLocation": "C:\\Program Files\\Epic Games\\Dauntless",
            "AppVersionString": "1.14.7"
        }))),
        Some(install("C:\\Program Files\\Epic Games\\Dauntless", Some("1.14.7")))
    );
    assert_eq!(
        parse_epic_item(&bytes(json!({ "AppName": "Fortnite", "InstallLocation": "C:\\x" }))),
        None
    );
    assert_eq!(parse_epic_item(&bytes(json!({ "AppName": "Jackal" }))), None);
    assert_eq!(
        parse_epic_item(&bytes(
            json!({ "DisplayName": "Dauntless", "InstallLocation": "C:\\x" })
        ))
        .map(|game| game.version),
        Some(None)
    );
}

#[test]
fn store_detection_keeps_installs_that_are_there_once() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path();
    let root = fake_game(&base.join("Games"), Some(SUPPORTED_VERSION));
    let root_text = text(&root);
    let legendary = base.join("profile").join(".config").join("legendary");
    std::fs::create_dir_all(&legendary).unwrap();
    std::fs::write(
        legendary.join("installed.json"),
        format!(
            "\u{feff}{}",
            json!({ "Jackal": { "app_name": "Jackal", "install_path": root_text, "version": "1.14.7" } })
        ),
    )
    .unwrap();
    let heroic = base
        .join("appdata")
        .join("heroic")
        .join("legendaryConfig")
        .join("legendary");
    std::fs::create_dir_all(&heroic).unwrap();
    // The same folder in another case, and one that is gone
    std::fs::write(
        heroic.join("installed.json"),
        json!({
            "Jackal": { "install_path": root_text.to_uppercase() },
            "Gone": { "app_name": "Jackal", "install_path": text(&base.join("gone")) },
        })
        .to_string(),
    )
    .unwrap();
    let manifests = base
        .join("programdata")
        .join("Epic")
        .join("EpicGamesLauncher")
        .join("Data")
        .join("Manifests");
    std::fs::create_dir_all(&manifests).unwrap();
    let epic_root = fake_game(&base.join("Epic Games"), None);
    std::fs::write(
        manifests.join("A1.ITEM"),
        json!({ "AppName": "Jackal", "InstallLocation": text(&epic_root) }).to_string(),
    )
    .unwrap();
    std::fs::write(manifests.join("big.item"), " ".repeat(1024 * 1024 + 1)).unwrap();
    std::fs::write(manifests.join("other.txt"), "{}").unwrap();
    // The shell's StoreEnv: ProgramData, USERPROFILE and APPDATA
    let vars = |name: &str| match name {
        "ProgramData" => Some(text(&base.join("programdata"))),
        "USERPROFILE" => Some(text(&base.join("profile"))),
        "APPDATA" => Some(text(&base.join("appdata"))),
        _ => None,
    };
    let found = detect_in(&StoreFolders::from_vars(vars));
    if !cfg!(windows) {
        return;
    }
    assert_eq!(
        found,
        [
            DetectedGame {
                root: root_text,
                source: GameSource::Legendary,
                version: Some("1.14.7".into())
            },
            DetectedGame {
                root: text(&epic_root),
                source: GameSource::Epic,
                version: None
            },
        ]
    );
    assert!(detect_in(&StoreFolders::default()).is_empty());
}
