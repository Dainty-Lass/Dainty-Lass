// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The `dainty-legacy` executable as a real process: the `legacy` plugin out of process (spec §10, L-2) and the
//! command line's exit codes and streams (§3).

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::*;
use dainty_legacy::USAGE;
use dainty_module_api::host::file_sha256;
use dainty_module_api::stub::{console, recording_host};
use dainty_module_api::{
    Actor, ActorContext, ChildModule, ChildOptions, CommandRequest, ErrorKind, ExternalModule, ModuleConfig,
    ModuleDriver, ModuleDriverExt, ModuleKind, NodeIdentity, Phase,
};
use rustic_isles_db::ids::derived_account_id;

fn exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_dainty-legacy"))
}

/// A 1.4.4 server database at the last prefix with two users, and a 1.14.7 target beside it.
fn fixtures(dir: &Path) -> (PathBuf, PathBuf) {
    let db = dir.join("metagame.db");
    let mut conn = revived(&db, 31, false);
    user(&mut conn, "U1", "Alice");
    user(&mut conn, "U2", "Bob");
    drop(conn);
    (db, target(dir))
}

fn accounts(target: &Path) -> i64 {
    count(target, "SELECT count(*) AS n FROM accounts")
}

fn path(p: &Path) -> String {
    p.to_str().unwrap().to_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_plugin_runs_out_of_process() {
    let dir = tempfile::tempdir().unwrap();
    let (db, target) = fixtures(dir.path());
    let module = ExternalModule {
        path: exe(),
        sha256: file_sha256(&exe()).await.unwrap(),
        args: Vec::new(),
        expected: Some("legacy".parse().unwrap()),
    };
    let child = ChildModule::spawn(
        &module,
        NodeIdentity::standalone(),
        recording_host(),
        ChildOptions::default(),
    )
    .await
    .unwrap();
    assert!(child.is_running());

    let manifest = child.manifest().clone();
    manifest.validate().unwrap();
    assert_eq!(manifest.id.as_str(), "legacy");
    assert_eq!(manifest.kind, ModuleKind::Plugin);
    assert_eq!(manifest.game_version.to_string(), "1.14.7");
    assert!(manifest.ports.is_empty());

    let registration = child.register().await.unwrap();
    let names: Vec<&str> = registration.commands.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["import-revived", "import-offline-save", "import-reforged"]);

    assert_eq!(child.status().await.unwrap().phase, Phase::Loaded);
    child
        .start(ModuleConfig::defaults(&manifest, &registration))
        .await
        .unwrap();
    assert_eq!(child.status().await.unwrap().phase, Phase::Ready);

    let request = |actor: ActorContext| CommandRequest {
        actor,
        name: "import-revived".into(),
        args: vec![path(&db), "--to".into(), path(&target)],
    };
    // Refused before any argument is read: nothing reaches the target
    let err = child
        .command(request(ActorContext::new(Actor::System)))
        .await
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Invalid);
    assert_eq!(
        err.message,
        "an import reads the host's files: run it from the host's console"
    );
    assert_eq!(accounts(&target), 0);

    let output = child.command(request(console())).await.unwrap();
    assert!(
        output.text.starts_with("revived import:\n  imported accounts: 2"),
        "{}",
        output.text
    );
    let data = output.data.unwrap();
    assert_eq!(data["source"], "revived");
    assert_eq!(data["dry_run"], false);
    assert_eq!(data["imported"]["accounts"], 2);
    assert_eq!(accounts(&target), 2);
    // The running node names the accounts' home
    assert!(launcher(&target, &derived_account_id("local", "U1")).is_some());

    child.stop().await.unwrap();
    assert_eq!(child.status().await.unwrap().phase, Phase::Stopped);
    child.unload().await.unwrap();
    assert!(!child.is_running());
}

fn run(args: &[&str]) -> Output {
    Command::new(exe()).args(args).output().unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

#[test]
fn an_unknown_command_or_kind_prints_the_usage_and_exits_2() {
    for args in [&["import"][..], &["import", "mongo"], &["export", "revived"]] {
        let out = run(args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(out.stdout.is_empty(), "{args:?}");
        assert_eq!(text(&out.stderr).trim_end(), USAGE, "{args:?}");
    }
}

#[test]
fn an_argument_error_is_a_failure_with_the_usage() {
    let dir = tempfile::tempdir().unwrap();
    let target = path(&dir.path().join("isles.db"));
    let out = run(&["import", "revived", "metagame.db", "--to", &target]);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert_eq!(
        text(&out.stderr).trim_end(),
        format!("dainty-legacy: the source must be an absolute path\n{USAGE}")
    );
}

#[test]
fn a_dry_run_prints_the_report_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let (db, target) = fixtures(dir.path());
    let out = run(&["import", "revived", &path(&db), "--to", &path(&target), "--dry-run"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
    let stdout = text(&out.stdout);
    assert!(
        stdout.starts_with("revived import (dry run: nothing written):\n  imported accounts: 2"),
        "{stdout}"
    );
    assert_eq!(accounts(&target), 0);
}

#[test]
fn a_failed_import_prints_the_error_and_exits_1() {
    let dir = tempfile::tempdir().unwrap();
    let (db, _) = fixtures(dir.path());
    let missing = dir.path().join("nowhere.db");
    let out = run(&["import", "revived", &path(&db), "--to", &path(&missing)]);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert_eq!(
        text(&out.stderr).trim_end(),
        format!(
            "dainty-legacy: target database: {}: there is no database here; start the 1.14.7 server once so it \
             makes one",
            missing.display()
        )
    );
}
