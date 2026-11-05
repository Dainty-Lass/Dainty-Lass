//! The plugin as DaintyLass loads it: its manifest and registration, its commands compiled in and out of process,
//! and the executable's command line.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use legacy::commands;
use legacy_import::testing::{Player, count, drizzle_database, insert_player, isles_database};
use module_api::actor::{Actor, ActorContext};
use module_api::host::file_sha256;
use module_api::manifest::plugin_fits;
use module_api::stub::RecordingHost;
use module_api::{
    API_VERSION, ChildModule, ChildOptions, CommandRequest, ErrorKind, ExternalModule, GameModule, InProcessModule,
    ModuleConfig, ModuleDriverExt, ModuleKind, ModuleManifest, NodeIdentity, Phase, Registrar,
};

fn isles_like() -> ModuleManifest {
    ModuleManifest {
        id: "isles-1-14-7".parse().unwrap(),
        kind: ModuleKind::Game,
        display_name: "Rustic Isles 1.14.7".into(),
        game_version: "1.14.7".parse().unwrap(),
        invite_aliases: vec!["reforged".into()],
        api: API_VERSION,
        ports: vec![],
        capabilities: Default::default(),
        unverified: false,
        client: None,
    }
}

#[test]
fn it_is_a_plugin_for_1_14_7() {
    let manifest = legacy::module().manifest();
    manifest.validate().unwrap();
    assert_eq!(manifest.kind, ModuleKind::Plugin);
    assert_eq!(manifest.id.as_str(), "legacy");
    plugin_fits(&isles_like(), &manifest).unwrap();
    let mut registrar = Registrar::new(manifest.clone());
    legacy::module().register(&mut registrar).unwrap();
    let registration = registrar.finish().unwrap();
    registration.check(&manifest).unwrap();
    let names: Vec<&str> = registration.commands.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, [commands::REVIVED, commands::OFFLINE_SAVE, commands::REFORGED]);
}

/// A 1.4.4 server with two players and an empty 1.14.7 one.
fn databases(dir: &Path) -> (PathBuf, PathBuf) {
    let source = dir.join("metagame.db");
    let isles = dir.join("isles.db");
    let mut conn = drizzle_database(&source, 31);
    let mut sam = Player::new("UID-sam", "Sam");
    sam.discord = Some("100000000000000001");
    insert_player(&mut conn, &sam);
    insert_player(&mut conn, &Player::new("UID-kim", "Kim"));
    drop(conn);
    isles_database(&isles);
    (source, isles)
}

fn import(actor: Actor, source: &Path, isles: &Path, extra: &[&str]) -> CommandRequest {
    let mut args = vec![source.display().to_string(), "--to".into(), isles.display().to_string()];
    args.extend(extra.iter().map(|s| (*s).to_owned()));
    CommandRequest {
        actor: ActorContext::new(actor),
        name: commands::REVIVED.into(),
        args,
    }
}

async fn drive(module: &impl ModuleDriverExt, dir: &Path) {
    let (source, isles) = databases(dir);
    module.start(ModuleConfig::default()).await.unwrap();
    assert_eq!(module.status().await.unwrap().phase, Phase::Ready);

    let refused = module
        .command(import(Actor::Admin { key: "ops".into() }, &source, &isles, &[]))
        .await
        .unwrap_err();
    assert_eq!(refused.kind, ErrorKind::Invalid);

    let bad = module
        .command(import(Actor::Console, &source, &isles, &["--frobnicate"]))
        .await
        .unwrap_err();
    assert_eq!(bad.kind, ErrorKind::Invalid);
    assert_eq!(count(&isles, "accounts", "1"), 0);

    let done = module
        .command(import(Actor::Console, &source, &isles, &[]))
        .await
        .unwrap();
    assert!(done.text.contains("imported accounts: 2"), "{}", done.text);
    assert_eq!(done.data.unwrap()["imported"]["accounts"], 2);
    // Imported on the running node's own home (DD18)
    assert_eq!(count(&isles, "accounts", "id LIKE 'account:%@local'"), 2);

    let unknown = module
        .command(CommandRequest {
            actor: ActorContext::new(Actor::Console),
            name: "import-everything".into(),
            args: vec![],
        })
        .await
        .unwrap_err();
    assert_eq!(unknown.kind, ErrorKind::Unsupported);
    module.stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn commands_compiled_in() {
    let dir = tempfile::tempdir().unwrap();
    let host = Arc::new(RecordingHost::new(NodeIdentity::standalone()));
    let module = InProcessModule::new(Arc::new(legacy::module()), NodeIdentity::standalone(), host).unwrap();
    drive(&module, dir.path()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn commands_out_of_process() {
    let dir = tempfile::tempdir().unwrap();
    let path = PathBuf::from(env!("CARGO_BIN_EXE_dainty-legacy"));
    let external = ExternalModule {
        sha256: file_sha256(&path).await.unwrap(),
        path,
        args: vec![],
        expected: Some("legacy".parse().unwrap()),
    };
    let host = Arc::new(RecordingHost::new(NodeIdentity::standalone()));
    let module = ChildModule::spawn(&external, NodeIdentity::standalone(), host, ChildOptions::default())
        .await
        .unwrap();
    drive(&module, dir.path()).await;
}

#[test]
fn the_command_line_runs_an_import() {
    let dir = tempfile::tempdir().unwrap();
    let (source, isles) = databases(dir.path());
    let exe = env!("CARGO_BIN_EXE_dainty-legacy");
    let run = |args: &[&str]| Command::new(exe).args(args).output().unwrap();

    let source = source.display().to_string();
    let isles_arg = isles.display().to_string();
    let dry = run(&["import", "revived", &source, "--to", &isles_arg, "--dry-run"]);
    assert!(dry.status.success(), "{}", String::from_utf8_lossy(&dry.stderr));
    assert!(String::from_utf8_lossy(&dry.stdout).contains("dry run"));
    assert_eq!(count(&isles, "accounts", "1"), 0);

    let done = run(&["import", "revived", &source, "--to", &isles_arg]);
    assert!(done.status.success(), "{}", String::from_utf8_lossy(&done.stderr));
    assert_eq!(count(&isles, "accounts", "1"), 2);

    let usage = run(&["export", "revived"]);
    assert_eq!(usage.status.code(), Some(2));
    let refused = run(&["import", "revived", "relative.db", "--to", &isles_arg]);
    assert!(!refused.status.success());
}
