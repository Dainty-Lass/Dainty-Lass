//! A 1.4.4 server's database into a 1.14.7 one, from each history 1.4.4 left: the Node metagame's Drizzle one, the
//! Rust metagame's Diesel one with DaintyLass's change log on it, and a Drizzle one cut short.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use legacy_import::revived::source::{Origin, prepare};
use legacy_import::revived::{self, Options};
use legacy_import::target::Target;
use legacy_import::testing::{
    NOW_MS, Player, add_change_log, count, diesel_database, drizzle_database, insert_invite, insert_player,
    isles_database,
};
use legacy_import::{ImportError, Report};
use store::DatabaseConfig;

const FROZEN: usize = 31;

fn options(dry_run: bool) -> Options {
    Options {
        dry_run,
        now_ms: NOW_MS,
    }
}

fn target(file: &Path) -> Target {
    Target::open(
        &DatabaseConfig::Sqlite {
            file: file.to_path_buf(),
        },
        "local".parse().unwrap(),
    )
    .unwrap()
}

struct Fixture {
    _dir: tempfile::TempDir,
    source: PathBuf,
    isles: PathBuf,
}

/// A 1.4.4 server with five players, two invites and some characters, made by `make`, and an empty 1.14.7 one.
fn fixture(make: fn(&Path) -> store::Connection) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("metagame.db");
    let isles = dir.path().join("isles.db");
    {
        let mut conn = make(&source);
        let mut sam = Player::new("UID-sam", "Sam");
        sam.discord = Some("100000000000000001");
        sam.characters = 2;
        sam.key = Some("RRK_sam");
        insert_player(&mut conn, &sam);
        let mut ban = Player::new("UID-ban", "Banned");
        ban.ban = Some((true, None));
        insert_player(&mut conn, &ban);
        let mut temp = Player::new("UID-temp", "Timeout");
        temp.ban = Some((true, Some(NOW_MS + 3_600_000)));
        temp.admin = true;
        insert_player(&mut conn, &temp);
        let mut lifted = Player::new("UID-old", "Forgiven");
        lifted.ban = Some((true, Some(NOW_MS - 1)));
        insert_player(&mut conn, &lifted);
        let mut inactive = Player::new("UID-off", "Pardoned");
        inactive.ban = Some((false, None));
        inactive.characters = 1;
        insert_player(&mut conn, &inactive);
        insert_invite(&mut conn, "WELCOME", 5, false);
        insert_invite(&mut conn, "FOREVER", 0, true);
        insert_invite(&mut conn, "USED-UP", 0, false);
    }
    isles_database(&isles);
    Fixture {
        _dir: dir,
        source,
        isles,
    }
}

fn drizzle_full(file: &Path) -> store::Connection {
    drizzle_database(file, FROZEN)
}

fn file_bytes(file: &Path) -> Vec<u8> {
    std::fs::read(file).unwrap()
}

fn check_first_import(report: &Report, isles: &Path) {
    assert_eq!(report.imported("accounts"), 5, "{}", report.text());
    assert_eq!(report.imported("bans"), 2);
    assert_eq!(report.imported("temporary bans made permanent"), 1);
    assert_eq!(report.imported("1.4.4 admins imported as players"), 1);
    assert_eq!(report.imported("discord links"), 1);
    assert_eq!(report.imported("invites"), 2);
    assert_eq!(report.skipped.len(), 1, "the used-up invite: {:?}", report.skipped);
    assert_eq!(report.dropped.get("characters"), Some(&3));

    assert_eq!(count(isles, "accounts", "user_id LIKE 'UID-%'"), 5);
    assert_eq!(count(isles, "launcher_accounts", "status = 'banned'"), 2);
    assert_eq!(
        count(
            isles,
            "launcher_accounts",
            "display_name_normalized = 'forgiven' AND status = 'active'"
        ),
        1
    );
    assert_eq!(
        count(
            isles,
            "auth_identities a JOIN accounts b ON a.account_id = b.id",
            "a.provider = 'discord' AND a.provider_subject = '100000000000000001' AND b.user_id = 'UID-sam'"
        ),
        1
    );
    assert_eq!(
        count(isles, "reforged_invites", "code = 'FOREVER' AND infinite_uses"),
        1
    );
    assert_eq!(count(isles, "reforged_invites", "code = 'USED-UP'"), 0);
    // Every row is in the change log, under the import's actor (DD23)
    assert!(count(isles, "__store_change_log", "actor = 'host:legacy-import'") >= 13);
    // Nothing of a key crosses
    assert_eq!(count(isles, "user_api_keys", "1"), 0);
    assert!(!report.text().contains("RRK_"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_drizzle_server_is_imported_once() {
    let f = fixture(drizzle_full);
    let before = file_bytes(&f.source);
    let target = target(&f.isles);

    let dry = revived::import_server(&f.source, &target, options(true)).await.unwrap();
    assert_eq!(dry.imported("accounts"), 5);
    assert_eq!(count(&f.isles, "accounts", "1"), 0, "a dry run writes nothing");

    let report = revived::import_server(&f.source, &target, options(false))
        .await
        .unwrap();
    check_first_import(&report, &f.isles);
    assert_eq!(file_bytes(&f.source), before, "the source is never written");

    let again = revived::import_server(&f.source, &target, options(false))
        .await
        .unwrap();
    assert!(again.imported.is_empty(), "{}", again.text());
    assert_eq!(again.unchanged.get("accounts"), Some(&5));
    assert_eq!(again.unchanged.get("invites"), Some(&2));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_diesel_server_with_a_change_log_is_imported() {
    let f = fixture(diesel_database);
    add_change_log(&f.source);
    assert!(count(&f.source, "sqlite_schema", "name LIKE '__store_%'") > 0);
    let report = revived::import_server(&f.source, &target(&f.isles), options(false))
        .await
        .unwrap();
    check_first_import(&report, &f.isles);
}

#[test]
fn a_short_drizzle_history_is_brought_forward() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("old.db");
    drop(drizzle_database(&source, 18));
    let prepared = prepare(&source, Origin::Server).unwrap();
    assert!(prepared.path().is_file());
    assert_eq!(
        count(&source, "sqlite_schema", "name = 'account_bans'"),
        0,
        "the source is left at 0017"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn conflicts_are_skipped_and_reported() {
    let f = fixture(drizzle_full);
    {
        // A 1.14.7 server that already has a "sam", the Discord account and a userId of its own
        use diesel::Connection as _;
        use diesel::connection::SimpleConnection;
        let mut conn = store::Connection::establish(&f.isles.to_string_lossy()).unwrap();
        conn.batch_execute(
            "INSERT INTO accounts (id, user_id, name) VALUES ('account:01JA0000000000000000000000@local', 'p1', 'Sam');
             INSERT INTO launcher_accounts (account_id, display_name_normalized, display_name, status, roles, created_at)
               VALUES ('account:01JA0000000000000000000000@local', 'sam', 'Sam', 'active', '[\"user\"]', 'x');
             INSERT INTO accounts (id, user_id, name) VALUES ('account:01JB0000000000000000000000@local', 'UID-ban', 'Other');",
        )
        .unwrap();
    }
    let report = revived::import_server(&f.source, &target(&f.isles), options(false))
        .await
        .unwrap();
    assert_eq!(report.imported("accounts"), 3, "{}", report.text());
    let reasons: Vec<(&str, &str)> = report.skipped.iter().map(|s| (s.kind, s.id.as_str())).collect();
    assert!(reasons.contains(&("account", "UID-sam")), "{reasons:?}");
    assert!(reasons.contains(&("account", "UID-ban")), "{reasons:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tampered_or_foreign_database_is_refused() {
    let isles = |dir: &Path| {
        let file = dir.join("isles.db");
        isles_database(&file);
        target(&file)
    };
    let dir = tempfile::tempdir().unwrap();
    let target = isles(dir.path());

    // A Drizzle history whose SQL isn't the Node metagame's
    let tampered = dir.path().join("tampered.db");
    {
        let mut conn = drizzle_database(&tampered, FROZEN);
        use diesel::connection::SimpleConnection;
        conn.batch_execute("UPDATE __drizzle_migrations SET hash = 'x' WHERE created_at = 1782160527617;")
            .unwrap();
    }
    // The right history with a table of its own
    let extra = dir.path().join("extra.db");
    {
        let mut conn = drizzle_database(&extra, FROZEN);
        use diesel::connection::SimpleConnection;
        conn.batch_execute("CREATE TABLE sneaky (x TEXT);").unwrap();
    }
    // No history at all, and not SQLite at all
    let bare = dir.path().join("bare.db");
    {
        use diesel::Connection as _;
        use diesel::connection::SimpleConnection;
        let mut conn = store::Connection::establish(&bare.to_string_lossy()).unwrap();
        conn.batch_execute("CREATE TABLE users (userId TEXT);").unwrap();
    }
    let junk = dir.path().join("junk.db");
    std::fs::write(&junk, vec![7u8; 4096]).unwrap();

    for source in [&tampered, &extra, &bare, &junk] {
        let result = revived::import_server(source, &target, options(false)).await;
        assert!(
            matches!(result, Err(ImportError::Invalid(_)) | Err(ImportError::Io(_))),
            "{}: {:?}",
            source.display(),
            result.map(|r| r.text())
        );
    }
    assert!(
        revived::import_server(&dir.path().join("missing.db"), &target, options(false))
            .await
            .is_err()
    );
    assert_eq!(count(&dir.path().join("isles.db"), "accounts", "1"), 0);
}

#[test]
fn a_database_isles_did_not_make_is_refused_as_a_target() {
    let f = fixture(drizzle_full);
    let other = f.source.with_file_name("other.db");
    drop(diesel_database(&other));
    let before = file_bytes(&other);
    let result = Target::open(
        &DatabaseConfig::Sqlite { file: other.clone() },
        "local".parse().unwrap(),
    );
    assert!(matches!(result, Err(ImportError::Target(_))));
    assert_eq!(file_bytes(&other), before, "a wrong target is left as it was");
    let missing = Target::open(
        &DatabaseConfig::Sqlite {
            file: f.source.with_file_name("nowhere.db"),
        },
        "local".parse().unwrap(),
    );
    assert!(matches!(missing, Err(ImportError::Target(_))));
}
