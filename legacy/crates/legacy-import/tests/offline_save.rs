//! A player's 1.4.4 offline save into a 1.14.7 server: as DR's launcher packed it (`.drsave`), or the bare database.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use diesel::connection::SimpleConnection;
use legacy_import::ImportError;
use legacy_import::revived::drsave::{PortableIdentity, write_save};
use legacy_import::revived::{self, Options};
use legacy_import::target::Target;
use legacy_import::testing::{KEY, NOW_MS, Player, count, diesel_database, insert_player, isles_database};
use store::DatabaseConfig;

const WHEN: &str = "2026-10-21T22:00:00.000Z";

fn options() -> Options {
    Options {
        dry_run: false,
        now_ms: NOW_MS,
    }
}

struct Fixture {
    dir: tempfile::TempDir,
    isles: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let isles = dir.path().join("isles.db");
        isles_database(&isles);
        Self { dir, isles }
    }

    fn target(&self) -> Target {
        Target::open(
            &DatabaseConfig::Sqlite {
                file: self.isles.clone(),
            },
            "local".parse().unwrap(),
        )
        .unwrap()
    }

    /// An offline database holding Sam, whose key is the save's, and whatever `more` adds.
    fn offline_db(&self, name: &str, more: impl FnOnce(&mut store::Connection)) -> PathBuf {
        let file = self.dir.path().join(name);
        let mut conn = diesel_database(&file);
        let mut sam = Player::new("UID-sam", "Sam");
        sam.key = Some(KEY);
        sam.discord = Some("100000000000000001");
        sam.characters = 4;
        insert_player(&mut conn, &sam);
        more(&mut conn);
        file
    }

    fn save(&self, database: &Path, user_id: &str, key: &str) -> PathBuf {
        let file = database.with_extension("drsave");
        let identity = PortableIdentity {
            user_id: user_id.into(),
            username: "Sam".into(),
            key: key.into(),
        };
        write_save(&file, database, &identity, WHEN).unwrap();
        file
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_drsave_brings_its_one_player() {
    let f = Fixture::new();
    let db = f.offline_db("offline.db", |_| {});
    let save = f.save(&db, "UID-sam", KEY);
    let report = revived::import_offline_save(&save, &f.target(), options())
        .await
        .unwrap();
    assert_eq!(report.source, "offline-save");
    assert_eq!(report.imported("accounts"), 1, "{}", report.text());
    assert_eq!(report.imported("discord links"), 1);
    assert_eq!(report.dropped.get("characters"), Some(&4));
    assert_eq!(count(&f.isles, "accounts", "user_id = 'UID-sam'"), 1);
    assert!(!report.text().contains(KEY));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bare_offline_database_is_imported_too() {
    let f = Fixture::new();
    let db = f.offline_db("offline.db", |_| {});
    let report = revived::import_offline_save(&db, &f.target(), options()).await.unwrap();
    assert_eq!(report.imported("accounts"), 1, "{}", report.text());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_save_that_isn_t_its_player_s_is_refused() {
    let f = Fixture::new();
    let other_key = KEY.replace("0123", "4567");
    let cases = [
        // The key in the metadata isn't the database's
        f.save(&f.offline_db("a.db", |_| {}), "UID-sam", &other_key),
        // The metadata names another player
        f.save(&f.offline_db("b.db", |_| {}), "UID-other", KEY),
        // A credential the export clears is still there
        f.save(
            &f.offline_db("c.db", |conn| {
                conn.batch_execute("INSERT INTO gameserverapikeys (keyHash) VALUES ('x');")
                    .unwrap()
            }),
            "UID-sam",
            KEY,
        ),
        // A bare database with two players
        f.offline_db("d.db", |conn| insert_player(conn, &Player::new("UID-two", "Two"))),
        // A bare database in WAL mode, and one with a journal beside it
        f.offline_db("e.db", |conn| conn.batch_execute("PRAGMA journal_mode = WAL;").unwrap()),
        {
            let db = f.offline_db("f.db", |_| {});
            std::fs::write(f.dir.path().join("f.db-journal"), b"x").unwrap();
            db
        },
    ];
    for source in &cases {
        let result = revived::import_offline_save(source, &f.target(), options()).await;
        assert!(
            matches!(result, Err(ImportError::Invalid(_))),
            "{}: {:?}",
            source.display(),
            result.map(|r| r.text())
        );
    }
    assert_eq!(count(&f.isles, "accounts", "1"), 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_damaged_drsave_is_refused() {
    let f = Fixture::new();
    let save = f.save(&f.offline_db("offline.db", |_| {}), "UID-sam", KEY);
    let mut bytes = std::fs::read(&save).unwrap();
    let middle = bytes.len() / 2;
    bytes[middle] ^= 1;
    std::fs::write(&save, bytes).unwrap();
    let result = revived::import_offline_save(&save, &f.target(), options()).await;
    assert!(matches!(result, Err(ImportError::Invalid(_))));
}
