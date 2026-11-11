// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Server and offline-save imports into a 1.14.7 target (spec §4 to §7, cases of §13).

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use common::*;
use dainty_legacy::{ImportError, Kind};
use rustic_isles_db::ids::derived_account_id;

const DAY: i64 = 86_400_000;

fn refusal(reason: &str) -> String {
    format!("not a Dauntless Revived 1.4.4 database: {reason}")
}

#[test]
fn the_soft_delete_list_is_rustic_isles() {
    // The importer declares it rather than linking the store crate; this keeps the two in step
    assert_eq!(dainty_legacy::SOFT_DELETE, rustic_isles_store::SOFT_DELETE);
}

#[tokio::test(flavor = "multi_thread")]
async fn every_prefix_boundary() {
    let soon = now_ms() + DAY;
    for prefix in [1, 6, 7, 8, 9, 17, 18, 23, 24, 29, 30, 31] {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("metagame.db");
        let mut conn = revived(&db, prefix, false);
        user(&mut conn, "U1", "Alice");
        user(&mut conn, "U2", "Bob");
        if prefix > 8 {
            exec(&mut conn, "UPDATE users SET \"isAdmin\" = 1 WHERE \"userId\" = 'U1'");
        }
        if prefix > 7 {
            exec(&mut conn, "INSERT INTO invitecodes VALUES ('CODE1', 3, 0)");
        }
        if prefix > 17 {
            exec(
                &mut conn,
                "INSERT INTO discordlinks VALUES ('D1', 'U1', '2026-01-02T03:04:05+01:00')",
            );
        }
        if prefix > 29 {
            exec(
                &mut conn,
                &format!(
                    "INSERT INTO account_bans (account_id, reason, active, expires_at) VALUES ('U2', 'rude', 1, {soon})"
                ),
            );
        } else if prefix > 23 {
            exec(
                &mut conn,
                "INSERT INTO account_bans (account_id, reason, active) VALUES ('U2', 'rude', 1)",
            );
        }
        drop(conn);
        let target = target(dir.path());
        let report = import(args(Kind::Revived, &db, &target))
            .await
            .unwrap_or_else(|e| panic!("prefix {prefix}: {e}"));
        let at = |what| (prefix, what, n(&report, what));
        assert_eq!(at("accounts"), (prefix, "accounts", 2));
        assert_eq!(at("invites"), (prefix, "invites", u64::from(prefix > 7)));
        assert_eq!(
            at("1.4.4 admins imported as players"),
            (prefix, "1.4.4 admins imported as players", u64::from(prefix > 8))
        );
        assert_eq!(at("discord links"), (prefix, "discord links", u64::from(prefix > 17)));
        assert_eq!(at("timeouts"), (prefix, "timeouts", u64::from(prefix > 29)));
        assert_eq!(at("bans"), (prefix, "bans", u64::from(prefix > 23 && prefix <= 29)));
        assert_eq!(n(&report, "temporary bans made permanent"), 0);
        let links = if prefix > 17 { 1 } else { 2 };
        assert!(
            report
                .notes
                .contains(&format!("{links} 1.4.4 account(s) had no Discord link, so their players have no way to sign in yet: link one from the admin UI")),
            "prefix {prefix}: {:?}",
            report.notes
        );
        if prefix > 17 {
            let linked_at = count(
                &target,
                "SELECT count(*) AS n FROM auth_identities WHERE provider = 'discord' AND provider_subject = 'D1' \
                 AND provider_username = 'D1' AND linked_at = '2026-01-02T02:04:05.000Z'",
            );
            assert_eq!(linked_at, 1);
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_diesel_history_and_a_wrong_one() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("rust.db");
    let mut conn = revived(&db, 31, true);
    user(&mut conn, "U1", "Alice");
    drop(conn);
    let target = target(dir.path());
    let report = import(args(Kind::Revived, &db, &target)).await.unwrap();
    assert_eq!(n(&report, "accounts"), 1);

    exec(
        &mut open(&db),
        "INSERT INTO __diesel_schema_migrations (version) VALUES ('0002')",
    );
    let err = import(args(Kind::Revived, &db, &target)).await.unwrap_err();
    assert_eq!(
        err.to_string(),
        refusal("its Diesel history is not one the 1.4.4 metagame made")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn drizzle_histories_that_are_not_the_metagames() {
    let data = dainty_legacy::data::revived().unwrap();
    let wrong = refusal("its Drizzle history is not one the Node metagame made");
    let cases: [(&str, String); 4] = [
        (
            "hash",
            String::from("UPDATE \"__drizzle_migrations\" SET hash = 'abc' WHERE rowid = 2"),
        ),
        (
            "time",
            String::from("UPDATE \"__drizzle_migrations\" SET created_at = created_at + 1 WHERE rowid = 1"),
        ),
        (
            "too long",
            format!(
                "INSERT INTO \"__drizzle_migrations\" (hash, created_at) VALUES ('{}', 1792200000003)",
                data.drizzle[0].sha256_lf
            ),
        ),
        ("empty", String::from("DELETE FROM \"__drizzle_migrations\"")),
    ];
    for (name, sql) in cases {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("node.db");
        let mut conn = revived(&db, 31, false);
        exec(&mut conn, &sql);
        drop(conn);
        let target = target(dir.path());
        let err = import(args(Kind::Revived, &db, &target)).await.unwrap_err();
        assert_eq!(err.to_string(), wrong, "{name}");
    }

    // A CRLF checkout's hash is the metagame's too
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("crlf.db");
    let mut conn = revived(&db, 3, false);
    exec(
        &mut conn,
        &format!(
            "UPDATE \"__drizzle_migrations\" SET hash = '{}' WHERE rowid = 2",
            data.drizzle[1].sha256_crlf
        ),
    );
    drop(conn);
    let target = target(dir.path());
    import(args(Kind::Revived, &db, &target)).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn objects_columns_and_bookkeeping_it_never_made() {
    let cases = [
        (
            "CREATE TABLE extra (a)",
            "it holds objects the 1.4.4 metagame never made",
        ),
        (
            "CREATE TRIGGER sneaky AFTER INSERT ON users BEGIN SELECT 1; END",
            "it holds objects the 1.4.4 metagame never made",
        ),
        (
            "CREATE VIEW v AS SELECT 1",
            "it holds objects the 1.4.4 metagame never made",
        ),
        (
            "ALTER TABLE users DROP COLUMN \"notes\"; ALTER TABLE users RENAME COLUMN \"name\" TO \"nom\"",
            "its schema is not the one its migration history makes",
        ),
        (
            "DROP TABLE \"__drizzle_migrations\"; CREATE TABLE \"__drizzle_migrations\" (hash text, created_at numeric)",
            "a migration table is not the one its tool makes",
        ),
        (
            "DROP TABLE \"__drizzle_migrations\"",
            "it has neither Drizzle's nor Diesel's migration history",
        ),
    ];
    for (sql, reason) in cases {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("odd.db");
        let mut conn = revived(&db, 9, false);
        exec(&mut conn, sql);
        drop(conn);
        let target = target(dir.path());
        let err = import(args(Kind::Revived, &db, &target)).await.unwrap_err();
        assert_eq!(err.to_string(), refusal(reason), "{sql}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_store_change_log_is_dropped_and_other_tables_counted() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("run-by-dainty.db");
    let mut conn = revived(&db, 31, false);
    user(&mut conn, "U1", "Alice");
    exec(
        &mut conn,
        "CREATE TABLE __store_change_log (id INTEGER PRIMARY KEY, x TEXT);
         CREATE TRIGGER __store_users_insert AFTER INSERT ON users BEGIN INSERT INTO __store_change_log (x) VALUES ('i'); END;
         CREATE TABLE characters (id INTEGER PRIMARY KEY);
         INSERT INTO characters (id) VALUES (1), (2);
         CREATE TABLE friendships (id INTEGER PRIMARY KEY);
         CREATE TABLE onetimejobs (id INTEGER PRIMARY KEY);
         INSERT INTO onetimejobs (id) VALUES (1);
         INSERT INTO userapikeys VALUES ('U1', 'h');",
    );
    drop(conn);
    let before = std::fs::read(&db).unwrap();
    let target = target(dir.path());
    let report = import(args(Kind::Revived, &db, &target)).await.unwrap();
    assert_eq!(
        report.dropped.into_iter().collect::<Vec<_>>(),
        [("characters".to_owned(), 2)]
    );
    // The host's file is never written
    assert_eq!(std::fs::read(&db).unwrap(), before);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_running_server_in_wal_mode_is_copied_whole() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("live.db");
    let mut conn = revived(&db, 31, false);
    exec(&mut conn, "PRAGMA journal_mode = WAL; PRAGMA wal_autocheckpoint = 0;");
    user(&mut conn, "U1", "Alice");
    user(&mut conn, "U2", "Bob");
    let mut wal = db.as_os_str().to_owned();
    wal.push("-wal");
    assert!(std::path::Path::new(&wal).exists());
    let target = target(dir.path());
    let report = import(args(Kind::Revived, &db, &target)).await.unwrap();
    assert_eq!(n(&report, "accounts"), 2);
    drop(conn);
}

#[tokio::test(flavor = "multi_thread")]
async fn every_skip_and_count() {
    let dir = tempfile::tempdir().unwrap();
    let target = target(dir.path());

    // Another node's import leaves U3, the name "carol" and Discord D9 taken
    let seed = dir.path().join("seed.db");
    let mut conn = revived(&seed, 31, false);
    user(&mut conn, "U3", "Seed");
    user(&mut conn, "S2", "Carol");
    exec(&mut conn, "INSERT INTO discordlinks VALUES ('D9', 'U3', 'not a time')");
    drop(conn);
    let mut seeded = args(Kind::Revived, &seed, &target);
    seeded.node = Some("other".parse().unwrap());
    let report = import(seeded).await.unwrap();
    assert_eq!((n(&report, "accounts"), n(&report, "discord links")), (2, 1));

    let db = dir.path().join("metagame.db");
    let mut conn = revived(&db, 31, false);
    let past = now_ms() - DAY;
    let soon = now_ms() + DAY;
    for (id, name) in [
        ("U1", " Alice "),
        ("U3", "Clash"),
        ("U4", " CAROL "),
        ("U5", "   "),
        ("U6", "Dave"),
        ("U7", "Eve"),
        ("U8", "Finn"),
        ("U9", "Gus"),
    ] {
        user(&mut conn, id, name);
    }
    exec(
        &mut conn,
        &format!(
            "UPDATE users SET \"isAdmin\" = 1, \"notes\" = 5 WHERE \"userId\" = 'U7';
             INSERT INTO discordlinks VALUES ('D9', 'U6', '2026-01-01T00:00:00Z');
             INSERT INTO account_bans (account_id, reason, active, expires_at) VALUES ('U7', 'cheat', 1, NULL);
             INSERT INTO account_bans (account_id, reason, active, expires_at) VALUES ('U8', 'old', 1, {past});
             INSERT INTO account_bans (account_id, reason, active, expires_at) VALUES ('U9', 'lifted', 0, NULL);
             INSERT INTO account_bans (account_id, reason, active, expires_at) VALUES ('U1', 'spam', 1, {soon});
             INSERT INTO invitecodes VALUES ('A', 0, 0), ('B', -2, 1), ('C', 5, 0);"
        ),
    );
    drop(conn);

    let report = import(args(Kind::Revived, &db, &target)).await.unwrap();
    let skipped: Vec<(String, String, String)> = report
        .skipped
        .iter()
        .map(|s| (s.kind.clone(), s.id.clone(), s.reason.clone()))
        .collect();
    let s = |k: &str, id: &str, r: &str| (k.to_owned(), id.to_owned(), r.to_owned());
    assert_eq!(
        skipped,
        [
            s("account", "U3", "a 1.14.7 account already has this userId"),
            s("account", "U4", "the name \"CAROL\" is taken on this server"),
            s("account", "U5", "it has no name"),
            s(
                "discord_link",
                "D9",
                "that Discord account is linked to another 1.14.7 account"
            ),
            s("invite", "A", "it has no uses left"),
        ]
    );
    assert_eq!(n(&report, "accounts"), 5);
    assert_eq!(n(&report, "timeouts"), 1);
    assert_eq!(n(&report, "bans"), 1);
    assert_eq!(n(&report, "1.4.4 admins imported as players"), 1);
    assert_eq!(n(&report, "invites"), 2);
    assert_eq!(n(&report, "discord links"), 0);
    assert!(report.notes[0].starts_with("1.4.4 account keys are not carried"));
    assert!(report.notes.contains(&String::from(
        "1.4.4 admins were imported as players: give them their roles in the admin UI"
    )));

    let alice = launcher(&target, &derived_account_id("local", "U1")).unwrap();
    assert_eq!(alice.display_name, "Alice");
    assert_eq!(alice.status, "active");
    assert_eq!(alice.approval_status.as_deref(), Some("approved"));
    assert_eq!(alice.approval_updated_by.as_deref(), Some("legacy-import:revived"));
    assert_eq!(alice.roles, "[\"user\"]");
    assert_eq!(alice.timeout_reason.as_deref(), Some("spam"));
    assert_eq!(alice.timeout_until.unwrap().len(), 24);
    let eve = launcher(&target, &derived_account_id("local", "U7")).unwrap();
    assert_eq!((eve.status.as_str(), eve.timeout_until), ("banned", None));
    for active in ["U8", "U9"] {
        assert_eq!(
            launcher(&target, &derived_account_id("local", active)).unwrap().status,
            "active"
        );
    }
    assert_eq!(
        count(
            &target,
            "SELECT count(*) AS n FROM accounts WHERE user_id = 'U7' AND notes = 5"
        ),
        1
    );
    assert_eq!(
        count(
            &target,
            "SELECT count(*) AS n FROM reforged_invites WHERE code = 'B' AND uses_remaining = 0 AND infinite_uses \
             AND note = 'imported from revived'"
        ),
        1
    );
    // The change log names the import, so it can be reverted
    assert!(
        count(
            &target,
            "SELECT count(*) AS n FROM __store_change_log WHERE actor = 'host:legacy-import'"
        ) > 0
    );

    // Again: what is there is unchanged, the rest skipped as before
    let again = import(args(Kind::Revived, &db, &target)).await.unwrap();
    assert_eq!(same(&again, "accounts"), 5);
    assert_eq!(same(&again, "invites"), 2);
    assert!(again.imported.is_empty(), "{:?}", again.imported);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_target_without_timeouts_bans_for_good() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("metagame.db");
    let mut conn = revived(&db, 31, false);
    user(&mut conn, "U1", "Alice");
    let soon = now_ms() + DAY;
    exec(
        &mut conn,
        &format!("INSERT INTO account_bans (account_id, reason, active, expires_at) VALUES ('U1', 'spam', 1, {soon})"),
    );
    drop(conn);
    let target = target(dir.path());
    exec(
        &mut open(&target),
        "DELETE FROM __diesel_schema_migrations WHERE version = '0610'",
    );
    let report = import(args(Kind::Revived, &db, &target)).await.unwrap();
    assert_eq!(n(&report, "bans"), 1);
    assert_eq!(n(&report, "temporary bans made permanent"), 1);
    assert_eq!(n(&report, "timeouts"), 0);
    assert!(
        report
            .notes
            .iter()
            .any(|n| n.starts_with("this 1.14.7 database keeps no timeouts yet"))
    );
    let alice = launcher(&target, &derived_account_id("local", "U1")).unwrap();
    assert_eq!((alice.status.as_str(), alice.timeout_until), ("banned", None));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dry_run_writes_nothing_and_counts_the_same() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("metagame.db");
    let mut conn = revived(&db, 31, false);
    user(&mut conn, "U1", "Alice");
    exec(&mut conn, "INSERT INTO invitecodes VALUES ('C', 5, 0)");
    drop(conn);
    let target = target(dir.path());
    let log = "SELECT count(*) AS n FROM __store_change_log";
    let before = count(&target, log);
    let mut dry = args(Kind::Revived, &db, &target);
    dry.dry_run = true;
    let dry = import(dry).await.unwrap();
    assert!(dry.dry_run);
    assert!(dry.text().starts_with("revived import (dry run: nothing written):"));
    assert_eq!(count(&target, "SELECT count(*) AS n FROM accounts"), 0);
    assert_eq!(count(&target, log), before);
    let real = import(args(Kind::Revived, &db, &target)).await.unwrap();
    assert_eq!((dry.imported, dry.skipped), (real.imported, real.skipped));
}

#[tokio::test(flavor = "multi_thread")]
async fn targets_it_refuses() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("metagame.db");
    drop(revived(&db, 31, false));
    let missing = dir.path().join("none.db");
    let err = import(args(Kind::Revived, &db, &missing)).await.unwrap_err();
    assert_eq!(
        err.to_string(),
        format!(
            "target database: {}: there is no database here; start the 1.14.7 server once so it makes one",
            missing.display()
        )
    );
    let plain = dir.path().join("plain.db");
    exec(&mut open(&plain), "CREATE TABLE t (a)");
    let err = import(args(Kind::Revived, &db, &plain)).await.unwrap_err().to_string();
    assert!(
        err.starts_with("target database: not a database rustic-isles made ("),
        "{err}"
    );
    let old = target(dir.path());
    exec(
        &mut open(&old),
        "DELETE FROM __diesel_schema_migrations WHERE version IN ('0020', '0200')",
    );
    let err = import(args(Kind::Revived, &db, &old)).await.unwrap_err();
    assert_eq!(
        err.to_string(),
        "target database: not a 1.14.7 database rustic-isles made, or one too old: migrations 0020, 0200 are missing"
    );
    let gone = dir.path().join("gone.db");
    let err = import(args(Kind::Revived, &gone, &old)).await.unwrap_err();
    assert_eq!(
        err,
        ImportError::Io(format!("{}: there is no file here", gone.display()))
    );
}

fn save_db(dir: &std::path::Path, prefix: usize) -> std::path::PathBuf {
    let db = dir.join("offline.db");
    let mut conn = revived(&db, prefix, false);
    user(&mut conn, "UID-1", "Sam");
    if prefix > 6 {
        api_key(&mut conn, "UID-1", KEY);
    }
    if prefix > 7 {
        exec(&mut conn, "INSERT INTO invitecodes VALUES ('C', 5, 0)");
    }
    db
}

fn write_save(dir: &std::path::Path, db: &std::path::Path, username: &str, key: &str) -> std::path::PathBuf {
    let bytes = std::fs::read(db).unwrap();
    let save = dir.join("player.drsave");
    std::fs::write(
        &save,
        dainty_legacy::drsave::build(&bytes, "UID-1", username, key, "2026-10-21T22:00:00.000Z"),
    )
    .unwrap();
    save
}

#[tokio::test(flavor = "multi_thread")]
async fn offline_saves() {
    let no_player = "not a Dauntless Revived 1.4.4 offline save: it doesn't hold one player";
    let dir = tempfile::tempdir().unwrap();
    let target = target(dir.path());
    let db = save_db(dir.path(), 31);
    let save = write_save(dir.path(), &db, "Sam", KEY);
    let report = import(args(Kind::OfflineSave, &save, &target)).await.unwrap();
    assert_eq!(report.source, "offline-save");
    assert_eq!(n(&report, "accounts"), 1);
    assert_eq!(n(&report, "invites"), 0);
    assert_eq!(
        report.notes.last().unwrap(),
        "the save's account had no Discord link, so its player has no way to sign in yet: link one from the admin UI"
    );
    assert_eq!(
        launcher(&target, &derived_account_id("local", "UID-1"))
            .unwrap()
            .approval_updated_by
            .as_deref(),
        Some("legacy-import:offline-save")
    );

    // A key that matches no row, a username that differs, a cleared table with rows, a prefix without keys
    let other = "UUK_ffffffffffffffffffffffffffffffffffffffffffffffff";
    for (name, save) in [
        ("key", write_save(dir.path(), &db, "Sam", other)),
        ("username", write_save(dir.path(), &db, "Sammy", KEY)),
    ] {
        let err = import(args(Kind::OfflineSave, &save, &target)).await.unwrap_err();
        assert_eq!(err.to_string(), no_player, "{name}");
    }
    let cleared = tempfile::tempdir().unwrap();
    let db2 = save_db(cleared.path(), 31);
    exec(
        &mut open(&db2),
        "CREATE TABLE gameserverapikeys (k TEXT); INSERT INTO gameserverapikeys VALUES ('x')",
    );
    let save = write_save(cleared.path(), &db2, "Sam", KEY);
    let err = import(args(Kind::OfflineSave, &save, &target)).await.unwrap_err();
    assert_eq!(err.to_string(), no_player);
    let early = tempfile::tempdir().unwrap();
    let db3 = save_db(early.path(), 5);
    let save = write_save(early.path(), &db3, "Sam", KEY);
    let err = import(args(Kind::OfflineSave, &save, &target)).await.unwrap_err();
    assert_eq!(err.to_string(), no_player);

    // A damaged trailer
    let mut bytes = std::fs::read(write_save(dir.path(), &db, "Sam", KEY)).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    let damaged = dir.path().join("damaged.drsave");
    std::fs::write(&damaged, bytes).unwrap();
    let err = import(args(Kind::OfflineSave, &damaged, &target)).await.unwrap_err();
    assert_eq!(
        err.to_string(),
        "not a Dauntless Revived offline save (.drsave), or a damaged one"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn bare_offline_databases() {
    let dir = tempfile::tempdir().unwrap();
    let target = target(dir.path());
    let db = save_db(dir.path(), 31);
    let report = import(args(Kind::OfflineSave, &db, &target)).await.unwrap();
    assert_eq!(n(&report, "accounts"), 1);

    user(&mut open(&db), "UID-2", "Two");
    let err = import(args(Kind::OfflineSave, &db, &target)).await.unwrap_err();
    assert_eq!(
        err.to_string(),
        "not a Dauntless Revived 1.4.4 offline save: it doesn't hold one player"
    );

    let journal = tempfile::tempdir().unwrap();
    let db = save_db(journal.path(), 31);
    let mut beside = db.as_os_str().to_owned();
    beside.push("-journal");
    std::fs::write(&beside, b"").unwrap();
    let err = import(args(Kind::OfflineSave, &db, &target)).await.unwrap_err();
    assert_eq!(
        err.to_string(),
        refusal("it has journal files beside it, so it was not stopped cleanly")
    );

    let wal = tempfile::tempdir().unwrap();
    let db = save_db(wal.path(), 31);
    exec(&mut open(&db), "PRAGMA journal_mode = WAL");
    let err = import(args(Kind::OfflineSave, &db, &target)).await.unwrap_err();
    assert_eq!(
        err.to_string(),
        refusal("an offline save is a stopped database in rollback-journal mode")
    );
}

#[test]
fn the_command_line() {
    let exe = env!("CARGO_BIN_EXE_dainty-legacy");
    let out = std::process::Command::new(exe).args(["export", "x"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).starts_with("usage:\n"));
    let out = std::process::Command::new(exe)
        .args(["import", "revived", "relative.db"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).starts_with("dainty-legacy: the source must be an absolute path\nusage:")
    );

    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("metagame.db");
    let mut conn = revived(&db, 31, false);
    user(&mut conn, "U1", "Alice");
    drop(conn);
    let target = target(dir.path());
    let out = std::process::Command::new(exe)
        .args(["import", "revived"])
        .arg(&db)
        .arg("--to")
        .arg(&target)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.starts_with("revived import:\n  imported accounts: 1\n"),
        "{stdout}"
    );
}
