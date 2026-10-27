//! "Rustic Isles 1.14.7" loads into DaintyLass and answers 501 (RU4's verify): compiled in through module-api's
//! `InProcessModule`, and out of process through `ChildModule::spawn` on its built executable, with every route of
//! both services requested.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use isles_core::routes::RouteSet;
use isles_core::testing::{Mounts, drive};
use module_api::host::file_sha256;
use module_api::stub::recording_host;
use module_api::{ChildModule, ChildOptions, ExternalModule, GameModule, InProcessModule, NodeIdentity};

const MOUNTS: Mounts = Mounts {
    set: RouteSet::V1147,
    rifts: cfg!(feature = "realms"),
    admin: true,
};

/// The admin API on, so its routes are mounted and swept too.
fn env_file() -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    writeln!(file, "MYSTICPARADOX_ADMIN_API=on").unwrap();
    file
}

#[test]
fn the_manifest_is_ddfours() {
    let manifest = isles_1_14_7::module().manifest();
    manifest.validate().unwrap();
    assert_eq!(manifest.id.as_str(), "isles-1-14-7");
    assert_eq!(manifest.display_name, "Rustic Isles 1.14.7");
    assert_eq!(manifest.game_version.to_string(), "1.14.7");
    assert!(manifest.answers_invite("reforged") && manifest.answers_invite("1.14.7"));
    assert_eq!(manifest.capabilities.realms, cfg!(feature = "realms"));
    assert!(manifest.capabilities.network);
    assert!(!manifest.unverified);
}

#[tokio::test(flavor = "multi_thread")]
async fn it_answers_501_compiled_in() {
    let host = recording_host();
    let module = InProcessModule::new(
        Arc::new(isles_1_14_7::module()),
        NodeIdentity::standalone(),
        host.clone(),
    )
    .unwrap();
    let env = env_file();
    let failures = drive(&module, &host, env.path(), MOUNTS).await;
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[tokio::test(flavor = "multi_thread")]
async fn it_answers_501_out_of_process() {
    let path = PathBuf::from(env!("CARGO_BIN_EXE_isles-1-14-7"));
    let external = ExternalModule {
        sha256: file_sha256(&path).await.unwrap(),
        path,
        args: vec![],
        expected: Some("isles-1-14-7".parse().unwrap()),
    };
    let host = recording_host();
    let module = ChildModule::spawn(
        &external,
        NodeIdentity::standalone(),
        host.clone(),
        ChildOptions::default(),
    )
    .await
    .unwrap();
    let env = env_file();
    let failures = drive(&module, &host, env.path(), MOUNTS).await;
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn director_args_are_one_point_fourteens() {
    let args = isles_1_14_7::DIRECTOR.launch_args(&isles_core::Settings::default());
    assert_eq!(
        args.join(" "),
        "-EpicPortal -server -nullrhi -warp -nosound -RepDriverEnable \
         -LogCmds=\"LogPhoenixCharacter VeryVerbose, LogOnline Verbose, LogLoad Off\""
    );
}

/// RU5 game-data: the Rust answers equal the reference Node handlers' (`capture.mjs`, Node 22).
#[tokio::test(flavor = "multi_thread")]
async fn game_data_answers_equal_nodes() {
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/isles-core/tests/golden");
    let failures = isles_core::game_data::golden::check(isles_1_14_7::version(), "1.14.7", &fixtures).await;
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// RU5 game-data: the auth gate, the missing game data file and the bodies Express refuses.
#[tokio::test(flavor = "multi_thread")]
async fn game_data_edges() {
    let failures = isles_core::game_data::golden::check_edges(isles_1_14_7::version()).await;
    assert!(
        failures.is_empty(),
        "{}",
        failures.join(
            "
"
        )
    );
}
