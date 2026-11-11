// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The Reforged Node-kit upgrade on a checkout made here (spec §11).

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::path::{Path, PathBuf};

use dainty_legacy::{ImportArgs, ImportError, Kind, Report, Target};

fn checkout() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join(".workspace").join("config");
    std::fs::create_dir_all(&config).unwrap();
    (dir, config)
}

fn args(root: &Path) -> ImportArgs {
    ImportArgs {
        kind: Kind::Reforged,
        source: root.to_path_buf(),
        target: None,
        node: None,
        dry_run: false,
        fresh_start: false,
        db_tool: None,
    }
}

async fn upgrade(args: ImportArgs) -> Result<Report, ImportError> {
    dainty_legacy::run(args, "local").await
}

fn touch(path: &Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, b"x").unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn not_a_reforged_install() {
    let dir = tempfile::tempdir().unwrap();
    let err = upgrade(args(dir.path())).await.unwrap_err();
    assert_eq!(
        err.to_string(),
        format!(
            "{}: no .workspace/config/host.json or host.toml, so not a Reforged install",
            dir.path().display()
        )
    );
    let (dir, config) = checkout();
    std::fs::write(config.join("host.json"), vec![b' '; 262_145]).unwrap();
    let err = upgrade(args(dir.path())).await.unwrap_err().to_string();
    assert!(err.ends_with(": not a host.json of at most 262144 bytes"), "{err}");
    std::fs::write(config.join("host.json"), "[1]").unwrap();
    let err = upgrade(args(dir.path())).await.unwrap_err();
    assert_eq!(err.to_string(), "host.json must hold a JSON object");
}

#[tokio::test(flavor = "multi_thread")]
async fn host_json_becomes_host_toml() {
    let (dir, config) = checkout();
    std::fs::write(
        config.join("host.json"),
        r#"{"mode": "public", "edition": "legacy", "legacyPublicPort": 4443, "gameDir": null}"#,
    )
    .unwrap();
    std::fs::write(config.join("host.json.bak"), "older").unwrap();
    std::fs::write(
        config.join("secrets.env"),
        "GATEWAY_SECRET=a\nexport REFORGED_ADMIN_KEY=\"b\"\nALLOWLIST_SECRET=\n",
    )
    .unwrap();

    let mut dry = args(dir.path());
    dry.dry_run = true;
    let report = upgrade(dry).await.unwrap();
    assert_eq!(report.imported.get("host.toml"), Some(&1));
    assert!(config.join("host.json").exists() && !config.join("host.toml").exists());

    let report = upgrade(args(dir.path())).await.unwrap();
    let toml = std::fs::read_to_string(config.join("host.toml")).unwrap();
    assert!(toml.starts_with("# This server's settings (dainty-host)"), "{toml}");
    assert!(toml.ends_with("mode = \"public\"\n"), "{toml}");
    assert!(!config.join("host.json").exists());
    assert!(!config.join("host.toml.partial").exists());
    assert!(
        std::fs::read_to_string(config.join("host.json.bak"))
            .unwrap()
            .starts_with("{\"mode\"")
    );
    assert_eq!(report.unchanged.get("secrets.env"), Some(&1));
    assert_eq!(
        report.notes,
        [
            "secrets.env keeps the old names REFORGED_ADMIN_KEY; dainty-host setup moves each to its new name, same value",
            "secrets.env is kept; dainty-host setup adds what it lacks: ALLOWLIST_SECRET, GAMESERVER_API_KEY, \
             API_KEY_HASH_SECRET, RUSTIC_ISLES_METRICS_TOKEN, AUTH_SIGNING_PRIVKEY_B64, AUTH_SIGNING_PUBKEY_B64, \
             DAINTY_ADMIN_API_KEY, RUSTIC_ISLES_MANAGER_OPERATOR_SECRET, DAINTY_DB_PASSWORD",
            "host.json is now host.toml; the old file is kept as host.json.bak",
            "left out of host.toml (1.4.4, 1.12 or unknown): edition, legacyPublicPort",
            "this server ran only Dauntless 1.4.4: set game_dir in host.toml to a 1.14.7 install before setup",
            "next: run dainty-host setup, which checks host.toml and writes the services' settings",
        ]
    );
    // No secret's value reaches the report
    let text = report.text();
    assert!(!text.contains("=\"b\"") && !text.contains("GATEWAY_SECRET=a"));

    // Again: host.toml is there
    std::fs::write(config.join("host.json"), "{}").unwrap();
    let report = upgrade(args(dir.path())).await.unwrap();
    assert_eq!(report.unchanged.get("host.toml"), Some(&1));
    assert!(report.notes.contains(&String::from(
        "host.toml is already there, so host.json was left as it is: delete it once you've checked"
    )));
}

#[tokio::test(flavor = "multi_thread")]
async fn folders_move_and_nothing_is_overwritten() {
    let (dir, config) = checkout();
    let root = dir.path();
    std::fs::write(config.join("host.toml"), "").unwrap();
    touch(&root.join("native/runtime/SDK.hpp"));
    touch(&root.join("native/runtime/x64/Release/a.dll"));
    touch(&root.join(".workspace/Mystic-Paradox/MysticParadox/x64/b.dll"));
    touch(&root.join("paradox/Items_Analysis/items.json"));

    let mut dry = args(root);
    dry.dry_run = true;
    let dry = upgrade(dry).await.unwrap();
    assert!(root.join("native/runtime/SDK.hpp").exists());

    let report = upgrade(args(root)).await.unwrap();
    assert_eq!(dry.imported, report.imported);
    assert_eq!(dry.skipped, report.skipped);
    assert_eq!(report.imported.get("folders moved"), Some(&3));
    assert!(root.join("modules/rustic-isles/native/runtime/SDK.hpp").exists());
    assert!(
        root.join("modules/rustic-isles/native/runtime/x64/Release/a.dll")
            .exists()
    );
    assert!(
        root.join("modules/rustic-isles/reference/1.14.7/Items_Analysis/items.json")
            .exists()
    );
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(report.skipped[0].id, ".workspace/Mystic-Paradox/MysticParadox/x64");
    assert_eq!(
        report.skipped[0].reason,
        "modules/rustic-isles/native/runtime/x64 is already there"
    );
    assert!(root.join(".workspace/Mystic-Paradox/MysticParadox/x64/b.dll").exists());
    for note in [
        "moved native/runtime/SDK.hpp -> modules/rustic-isles/native/runtime/SDK.hpp",
        ".workspace/Mystic-Paradox is no longer used: delete it once you've checked what moved",
        "native/runtime is no longer used: delete it once you've checked what moved",
        "there is no secrets.env: dainty-host setup makes a new one, which logs every player out",
    ] {
        assert!(report.notes.iter().any(|n| n == note), "{note}: {:?}", report.notes);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn mongodb_is_moved_or_left_on_purpose() {
    let (dir, config) = checkout();
    let root = dir.path();
    std::fs::write(config.join("host.toml"), "").unwrap();
    let mongo = root.join(".workspace/mongodb");
    touch(&mongo.join("mongod.lock"));
    // Only the lock: nothing to move
    let report = upgrade(args(root)).await.unwrap();
    assert!(!report.notes.iter().any(|n| n.contains("MongoDB")));

    touch(&mongo.join("collection-0.wt"));
    std::fs::write(
        config.join("backend.env"),
        "MONGODB_URI=mongodb://admin:hunter2@127.0.0.1:27017\nMONGODB_DB=isles\n",
    )
    .unwrap();
    let err = upgrade(args(root)).await.unwrap_err();
    let text = err.to_string();
    assert!(matches!(err, ImportError::Usage(_)));
    assert!(
        text.starts_with(
            "this install has MongoDB data (mongodb://admin:***@127.0.0.1:27017/isles): give the 1.14.7 database"
        ),
        "{text}"
    );
    assert!(!text.contains("hunter2"));

    let mut dry = args(root);
    dry.dry_run = true;
    dry.target = Some(Target::To(root.join("isles.db")));
    let report = upgrade(dry).await.unwrap();
    assert!(report.notes.contains(&String::from(
        "MongoDB (mongodb://admin:***@127.0.0.1:27017/isles) would be moved with dainty-host db migrate and verify"
    )));

    let mut missing_tool = args(root);
    missing_tool.target = Some(Target::To(root.join("isles.db")));
    missing_tool.db_tool = Some(root.join("no-such-tool"));
    let err = upgrade(missing_tool).await.unwrap_err();
    assert!(matches!(err, ImportError::Io(_)), "{err}");
    assert!(!config.join(".mongo-migrated").exists());

    let mut fresh = args(root);
    fresh.fresh_start = true;
    let report = upgrade(fresh).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(config.join(".mongo-migrated")).unwrap(),
        "fresh-start\n"
    );
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.ends_with("was not moved (--fresh-start); its files are kept"))
    );
    let report = upgrade(args(root)).await.unwrap();
    assert_eq!(report.unchanged.get("mongodb"), Some(&1));
}
