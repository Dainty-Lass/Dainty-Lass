// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The manager's history and revert against a workspace's admin database (spec-tauri-shell §7, §12.2): an
//! owner's revocation of two moderators' keys is listed, planned and reverted, and the revert is logged as the
//! server GUI's.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use dainty_admin::keys::{Keys, Role};
use dainty_host::paths::Paths;
use dainty_launcher_kit::bridge::checks::HistoryFilter;
use dainty_launcher_kit::bridge::types::ErrorCode;
use dainty_manager_server::history::{apply, dry_run, query};
use dainty_store::{Actor, DatabaseConfig};

const KEY: &str = "0123456789abcdef0123456789abcdef";

fn slashed(path: &std::path::Path) -> String {
    format!("\"{}\"", path.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/"))
}

/// A workspace whose admin.env names a SQLite game database; the admin database is beside it.
async fn workspace() -> (tempfile::TempDir, Paths) {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path());
    std::fs::create_dir_all(&paths.config).unwrap();
    let file = paths.config.join("dainty-lass.db");
    let env = format!(
        "DAINTY_ADMIN_API_KEY={KEY}\nDAINTY_ADMIN_KEY={KEY}\nRUSTIC_ISLES_MANAGER_OPERATOR_SECRET={KEY}\n\
         DAINTY_ADMIN_API_TIMEOUTS_FILE={}\nDAINTY_DB_ENGINE=sqlite\nDAINTY_DB_FILE={}\n",
        slashed(&paths.config.join("timeouts.json")),
        slashed(&file)
    );
    std::fs::write(paths.config.join("admin.env"), env).unwrap();

    let node = DatabaseConfig::Sqlite { file };
    let keys = Keys::open(&Keys::database_for(&node), KEY).unwrap();
    let mut ids = Vec::new();
    for name in ["Mod One", "Mod Two"] {
        let (info, _) = keys
            .issue(name, Role::Moderator, Actor::Host("console".into()), 1_000)
            .await
            .unwrap();
        ids.push(info.id);
    }
    for id in &ids {
        keys.revoke(id, Actor::Admin("owner".into()), 2_000).await.unwrap();
    }
    (dir, paths)
}

fn by(actor: &str) -> HistoryFilter {
    HistoryFilter {
        actor: Some(actor.into()),
        ..HistoryFilter::default()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_owners_revocations_are_listed_planned_and_reverted_once() {
    let (_dir, paths) = workspace().await;
    let workspace = paths.repo.clone();

    let page = query(&workspace, &by("admin:owner")).await.unwrap();
    assert_eq!(page.entries.len(), 2);
    for entry in &page.entries {
        assert_eq!((entry.database.as_str(), entry.table.as_str()), ("admin", "admin_keys"));
        for row in [&entry.before, &entry.after].into_iter().flatten() {
            let row: serde_json::Value = serde_json::from_str(row).unwrap();
            let hash = row.get("key_hash").and_then(serde_json::Value::as_str);
            assert!(hash.is_none_or(|h| h == "<hidden>"), "a key hash is never shown: {row}");
        }
    }

    let plan = dry_run(&workspace, &by("admin:owner")).await.unwrap();
    assert_eq!((plan.action_count, plan.conflict_count), (2, 0));
    assert!(
        plan.actions
            .iter()
            .all(|a| a.kind == "update" && a.table == "admin_keys")
    );
    assert!(plan.conflicts.is_empty() && !plan.truncated);

    let unknown = "0".repeat(64);
    assert_eq!(
        apply(&workspace, &unknown).await.unwrap_err().code,
        ErrorCode::HistoryPlanChanged
    );
    let done = apply(&workspace, &plan.id).await.unwrap();
    assert_eq!(done.rows_changed, 2);
    assert_eq!(done.databases.len(), 1);
    assert_eq!(
        (done.databases[0].database.as_str(), done.databases[0].rows),
        ("admin", 2)
    );
    assert!(done.at.is_some());
    assert_eq!(
        apply(&workspace, &plan.id).await.unwrap_err().code,
        ErrorCode::HistoryPlanChanged,
        "a plan applies once"
    );

    let reverted = query(&workspace, &by("host:server-gui")).await.unwrap();
    assert_eq!(reverted.entries.len(), 2);
    assert!(reverted.entries.iter().all(|e| e.actor == "host:server-gui"));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_actor_is_a_bad_filter() {
    let (_dir, paths) = workspace().await;
    let workspace = paths.repo.clone();
    assert_eq!(
        query(&workspace, &by("robot:x")).await.unwrap_err().code,
        ErrorCode::HistoryBadFilter
    );
    assert_eq!(
        dry_run(&workspace, &by("robot:x")).await.unwrap_err().code,
        ErrorCode::HistoryBadFilter
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_workspace_without_admin_env_has_no_history() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(
        query(dir.path(), &HistoryFilter::default()).await.unwrap_err().code,
        ErrorCode::HistoryUnavailable
    );
    assert_eq!(
        dry_run(dir.path(), &HistoryFilter::default()).await.unwrap_err().code,
        ErrorCode::HistoryUnavailable
    );
}
