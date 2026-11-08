//! dainty-launcher-kit's game installs and stored settings view (DE5, DD22, spec §2.4.11), as the shell's
//! `controller/{install,stored}.rs` tested them.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use dainty_epic::download::DownloadError;
use dainty_epic::manifest::{Guid, InstallPath};
use dainty_launcher_kit::bridge::types::{ErrorCode, GraphicsPreset, LauncherError, ServerGame};
use dainty_launcher_kit::controller::install::launcher_error;
use dainty_launcher_kit::controller::stored::{Stored, StoredServer, edit};
use dainty_launcher_kit::core::i18n::Language;
use dainty_launcher_kit::core::pins::{EXE_RELATIVE_PATH, PINNED_EXE_SHA256};
use dainty_launcher_kit::net::versions::{KnownProfile, builtin};
use dainty_launcher_kit::platform::settings::{Os, defaults, sanitize};
use dainty_module_api::GameVersion;
use dainty_protocol::invite::ServerMode;
use serde_json::{Value, json};

/// The shell's `known_profile`: the compiled-in profile that names an Epic build for `version`.
fn known_profile(version: &str) -> Option<KnownProfile> {
    let version = version.parse::<GameVersion>().ok()?;
    builtin::profiles()
        .into_iter()
        .find(|profile| profile.game_version == version && profile.client.epic.is_some())
}

#[test]
fn the_compiled_in_profiles_name_what_an_install_needs() {
    let version = "1.14.7";
    let profile = known_profile(version).unwrap();
    let epic = profile.client.epic.as_ref().unwrap();
    assert!(!epic.manifest_sources.is_empty());
    assert!(InstallPath::new(&profile.client.executable).is_ok());
    assert!(InstallPath::new(version).is_ok(), "a version names a folder");
    // The executable the install checks is the one the launcher checks before PLAY
    let current = known_profile("1.14.7").unwrap();
    assert_eq!(
        current.client.epic.unwrap().executable_sha256.to_string(),
        PINNED_EXE_SHA256
    );
    assert_eq!(current.client.executable, EXE_RELATIVE_PATH);
    // 1.4.4 has no archived manifest (DD22), and anything else is no version
    assert!(known_profile("1.4.4").is_none());
    assert!(known_profile("9.9.9").is_none());
    assert!(known_profile("../evil").is_none());
}

#[test]
fn install_errors_map_to_codes_the_page_knows() {
    let code = |e: DownloadError| launcher_error(&e).code;
    assert_eq!(code(DownloadError::NotOwned), ErrorCode::EpicNotOwned);
    assert_eq!(code(DownloadError::SignedOut), ErrorCode::EpicSignedOut);
    assert_eq!(code(DownloadError::Library), ErrorCode::EpicLibraryFailed);
    assert_eq!(
        launcher_error(&DownloadError::NoSpace {
            needed: 5,
            available: 1
        }),
        LauncherError::with_detail(ErrorCode::DiskSpace, "5")
    );
    let chunk = |deleted| DownloadError::ChunkUnavailable {
        guid: Guid([0; 4]),
        deleted,
        reasons: Vec::new(),
    };
    assert_eq!(code(chunk(true)), ErrorCode::GameVersionUnavailable);
    assert_eq!(code(chunk(false)), ErrorCode::DownloadFailed);
    assert_eq!(
        code(DownloadError::ExecutableHash { name: "x".into() }),
        ErrorCode::GameWrongVersion
    );
    assert_eq!(code(DownloadError::Cancelled), ErrorCode::Cancelled);
    assert_eq!(
        code(DownloadError::LinkInFolder { name: "a".into() }),
        ErrorCode::FolderInvalid
    );
}

#[test]
fn reads_every_checked_settings_file() {
    let fresh = Stored(defaults(Language::PtBr));
    assert!(fresh.server().is_none());
    assert_eq!(fresh.language(), Language::PtBr);
    assert_eq!(fresh.graphics(), GraphicsPreset::default());

    let mut map = defaults(Language::En);
    let server = StoredServer {
        game: ServerGame::Reforged,
        mode: ServerMode::Private,
        host: "100.64.0.1".into(),
        port: 61000,
        name: "Friends".into(),
        share: None,
        fp: None,
        code: Some("abcd".into()),
    };
    edit::set_server(&mut map, Some(&server));
    edit::keyed_value(&mut map, "usernames", &server.id(), Some(json!("Sam")));
    edit::keyed_value(&mut map, "backupOffered", &server.id(), Some(json!(true)));
    let map = sanitize(&Value::Object(map), Language::En, Os::current());
    let stored = Stored(map);
    assert_eq!(stored.server().as_ref(), Some(&server));
    assert_eq!(stored.saved_servers(), vec![server.clone()]);
    assert_eq!(stored.username(&server.id()), Some("Sam"));
    assert!(stored.backup_offered(&server.id()));
}
