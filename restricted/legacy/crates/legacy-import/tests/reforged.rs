//! A Reforged Revival Node-kit install upgraded in place: its MongoDB first (a stand-in for dainty-host db here), then the
//! old folders, secrets.env and host.json.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use legacy_import::reforged::layout::Layout;
use legacy_import::reforged::mongo::{MONGO_MIGRATED_MARKER, MongoMigrator, MongoPlan};
use legacy_import::{ImportError, reforged};
use store::DatabaseConfig;

const HOST_JSON: &str = r#"{
  "version": 1,
  "mode": "private",
  "edition": "both",
  "host": "127.0.0.1",
  "gameAddress": "127.0.0.1",
  "name": "Old Kit",
  "legacyGameDir": "C:/Games/Dauntless-1.4.4",
  "mongo": { "uri": "mongodb://kit:hunter2@127.0.0.1:27017", "manage": true }
}"#;

const SECRETS: &str = "GATEWAY_SECRET=gw-s3cret\nAPI_KEY_HASH_SECRET=hash-s3cret\n";

/// An old checkout: host.json, secrets.env, MongoDB's files and the upstream clone's exports.
fn old_install(root: &Path) -> Layout {
    let l = Layout::new(root);
    std::fs::create_dir_all(&l.config).unwrap();
    std::fs::write(&l.host_json, HOST_JSON).unwrap();
    std::fs::write(l.config.join("secrets.env"), SECRETS).unwrap();
    std::fs::create_dir_all(&l.mongo_data).unwrap();
    std::fs::write(l.mongo_data.join("WiredTiger"), b"data").unwrap();
    std::fs::write(l.mongo_data.join("mongod.lock"), b"").unwrap();
    let exports = l.old_upstream.join("Items_Analysis");
    std::fs::create_dir_all(&exports).unwrap();
    std::fs::write(exports.join("items.json"), "[]").unwrap();
    l
}

#[derive(Default)]
struct FakeDbTool {
    differ: Vec<String>,
    calls: RefCell<Vec<String>>,
}

impl MongoMigrator for FakeDbTool {
    fn migrate(&self, uri: &str, _target: &DatabaseConfig) -> legacy_import::Result<()> {
        self.calls.borrow_mut().push(format!("migrate {uri}"));
        Ok(())
    }

    fn verify(&self, uri: &str, _target: &DatabaseConfig) -> legacy_import::Result<Vec<String>> {
        self.calls.borrow_mut().push(format!("verify {uri}"));
        Ok(self.differ.clone())
    }
}

fn target(root: &Path) -> DatabaseConfig {
    DatabaseConfig::Sqlite {
        file: root.join("isles.db"),
    }
}

fn unchanged(l: &Layout) {
    assert!(l.host_json.is_file() && !l.host_toml.exists());
    assert!(!l.config.join(MONGO_MIGRATED_MARKER).exists());
    assert!(l.old_upstream.join("Items_Analysis").join("items.json").is_file());
}

#[test]
fn an_old_kit_is_upgraded() {
    let dir = tempfile::tempdir().unwrap();
    let l = old_install(dir.path());
    let config = target(dir.path());
    let db_tool = FakeDbTool::default();
    let plan = MongoPlan::Migrate {
        target: &config,
        migrator: &db_tool,
    };

    let dry = reforged::upgrade(dir.path(), &plan, true).unwrap();
    assert!(db_tool.calls.borrow().is_empty(), "a dry run moves no MongoDB");
    assert_eq!(dry.imported("host.toml"), 1);
    unchanged(&l);

    let report = reforged::upgrade(dir.path(), &plan, false).unwrap();
    let text = report.text();
    assert_eq!(
        *db_tool.calls.borrow(),
        [
            "migrate mongodb://kit:hunter2@127.0.0.1:27017/reforged",
            "verify mongodb://kit:hunter2@127.0.0.1:27017/reforged"
        ]
    );
    for secret in ["hunter2", "gw-s3cret", "hash-s3cret"] {
        assert!(!text.contains(secret), "{secret} in {text}");
    }
    let marker = std::fs::read_to_string(l.config.join(MONGO_MIGRATED_MARKER)).unwrap();
    assert_eq!(marker, "migrated from mongodb://kit:***@127.0.0.1:27017/reforged\n");
    let toml = std::fs::read_to_string(&l.host_toml).unwrap();
    assert!(toml.contains("name = \"Old Kit\"") && !toml.contains("legacy_game_dir") && !toml.contains("edition"));
    assert!(!l.host_json.exists() && l.config.join("host.json.bak").is_file());
    assert!(l.exports.join("items.json").is_file());
    assert_eq!(
        std::fs::read_to_string(l.config.join("secrets.env")).unwrap(),
        SECRETS,
        "kept as it is"
    );
    assert!(
        text.contains("legacyGameDir") && text.contains("REFORGED_ADMIN_KEY"),
        "{text}"
    );

    // A second run finds everything done
    let again = reforged::upgrade(dir.path(), &plan, false).unwrap();
    assert!(again.imported.is_empty(), "{}", again.text());
    assert_eq!(again.unchanged.get("host.toml"), Some(&1));
    assert_eq!(again.unchanged.get("mongodb"), Some(&1));
    assert_eq!(db_tool.calls.borrow().len(), 2);
}

#[test]
fn nothing_changes_when_mongodb_is_not_moved() {
    let dir = tempfile::tempdir().unwrap();
    let l = old_install(dir.path());

    // No target given: refused before anything moves
    let refused = reforged::upgrade(dir.path(), &MongoPlan::Refuse, false);
    assert!(
        matches!(&refused, Err(ImportError::Usage(m)) if !m.contains("hunter2")),
        "{refused:?}"
    );
    unchanged(&l);

    // dainty-host db verify found a difference
    let config = target(dir.path());
    let db_tool = FakeDbTool {
        differ: vec!["accounts".into()],
        ..FakeDbTool::default()
    };
    let plan = MongoPlan::Migrate {
        target: &config,
        migrator: &db_tool,
    };
    let failed = reforged::upgrade(dir.path(), &plan, false);
    assert!(
        matches!(&failed, Err(ImportError::Invalid(m)) if m.contains("accounts")),
        "{failed:?}"
    );
    unchanged(&l);
}

#[test]
fn a_fresh_start_keeps_mongodb_and_marks_it() {
    let dir = tempfile::tempdir().unwrap();
    let l = old_install(dir.path());
    let report = reforged::upgrade(dir.path(), &MongoPlan::FreshStart, false).unwrap();
    assert_eq!(
        std::fs::read_to_string(l.config.join(MONGO_MIGRATED_MARKER)).unwrap(),
        "fresh-start\n"
    );
    assert!(l.mongo_data.join("WiredTiger").is_file());
    assert!(l.host_toml.is_file(), "{}", report.text());
}

#[test]
fn a_folder_that_is_no_install_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let result = reforged::upgrade(dir.path(), &MongoPlan::Refuse, false);
    assert!(matches!(result, Err(ImportError::Invalid(_))));
    let l = Layout::new(dir.path());
    std::fs::create_dir_all(&l.config).unwrap();
    std::fs::write(&l.host_json, "not json").unwrap();
    let result = reforged::upgrade(dir.path(), &MongoPlan::FreshStart, false);
    assert!(matches!(result, Err(ImportError::Invalid(_))));
    assert!(
        !l.config.join(MONGO_MIGRATED_MARKER).exists(),
        "a bad host.json changes nothing"
    );
    let _: PathBuf = l.host_json;
}
