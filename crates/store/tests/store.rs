// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use diesel::connection::{Connection as _, SimpleConnection};
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Text};
use diesel_migrations::{EmbeddedMigration, EmbeddedMigrations, EmbeddedName, TomlMetadataWrapper, embed_migrations};
use store::{
    Connection, DrizzleBaseline, Error, Options, Schema, Store, backup, drizzle, migrate, open_read_only,
    schema_snapshot, schema_version,
};
use tokio::sync::Notify;

const MIGRATIONS_V1: EmbeddedMigrations = embed_migrations!("tests/migrations");

const UP_V1: &str = include_str!("migrations/0001_items/up.sql");
const DOWN_V1: &str = include_str!("migrations/0001_items/down.sql");
const UP_V2: &str = "ALTER TABLE items ADD COLUMN note TEXT NOT NULL DEFAULT '';";
const DOWN_V2: &str = "ALTER TABLE items DROP COLUMN note;";

const MIGRATIONS_V2: EmbeddedMigrations = EmbeddedMigrations::new(&[
    EmbeddedMigration::new(
        UP_V1,
        Some(DOWN_V1),
        EmbeddedName::new("0001_items"),
        TomlMetadataWrapper::new(true),
    ),
    EmbeddedMigration::new(
        UP_V2,
        Some(DOWN_V2),
        EmbeddedName::new("0002_items_note"),
        TomlMetadataWrapper::new(true),
    ),
]);

const BASELINE_SNAPSHOT: &str = "\
CREATE TABLE items (id INTEGER PRIMARY KEY NOT NULL, name TEXT NOT NULL);
CREATE TABLE tags (item INTEGER NOT NULL REFERENCES items (id), tag TEXT NOT NULL);
";

const SCHEMA_V1: Schema = Schema {
    migrations: || MIGRATIONS_V1,
    drizzle: None,
};

const SCHEMA_WITH_DRIZZLE_V2: Schema = Schema {
    migrations: || MIGRATIONS_V2,
    drizzle: Some(DrizzleBaseline {
        created_at: 1_700_000_000_000,
        sql: UP_V1,
        snapshot: BASELINE_SNAPSHOT,
    }),
};

#[derive(QueryableByName)]
struct CountRow {
    #[diesel(sql_type = BigInt)]
    n: i64,
}

#[derive(QueryableByName)]
struct TextRow {
    #[diesel(sql_type = Text)]
    val: String,
}

#[derive(QueryableByName)]
struct JournalModeRow {
    #[diesel(sql_type = Text)]
    journal_mode: String,
}

fn item_count(conn: &mut Connection) -> Result<i64, diesel::result::Error> {
    Ok(diesel::sql_query("SELECT count(*) AS n FROM items")
        .get_result::<CountRow>(conn)?
        .n)
}

#[tokio::test]
async fn commits_writes_rolls_back_errors_and_rejects_writes_in_read() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("store.db");
    let store = Store::open(&db_path, &Options::default(), &SCHEMA_V1).unwrap();

    // Commit a write
    store
        .write::<_, Error, _>(|conn| {
            conn.batch_execute("INSERT INTO items (id, name) VALUES (1, 'sword');")?;
            Ok(())
        })
        .await
        .unwrap();

    let count = store.read::<_, Error, _>(|conn| Ok(item_count(conn)?)).await.unwrap();
    assert_eq!(count, 1);

    // Roll back on Err
    let err = store
        .write::<(), Error, _>(|conn| {
            conn.batch_execute("INSERT INTO items (id, name) VALUES (2, 'shield');")?;
            Err(Error::Config("abort".into()))
        })
        .await;
    assert!(err.is_err());

    let count = store.read::<_, Error, _>(|conn| Ok(item_count(conn)?)).await.unwrap();
    assert_eq!(count, 1);

    // Reject write inside Store::read
    let read_write = store
        .read::<(), Error, _>(|conn| {
            conn.batch_execute("INSERT INTO items (id, name) VALUES (3, 'potion');")?;
            Ok(())
        })
        .await;
    assert!(read_write.is_err());
}

#[tokio::test]
async fn enforces_wal_foreign_keys_and_busy_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("wal.db");
    let options = Options {
        read_connections: 2,
        busy_timeout: Duration::from_secs(2),
    };
    let store = Store::open(&db_path, &options, &SCHEMA_V1).unwrap();

    // Check WAL journal mode
    let mode = store
        .read::<_, Error, _>(|conn| {
            Ok(diesel::sql_query("PRAGMA journal_mode")
                .get_result::<JournalModeRow>(conn)
                .map(|r| r.journal_mode)?)
        })
        .await
        .unwrap();
    assert!(mode.eq_ignore_ascii_case("wal"));

    // Foreign key enforcement: inserting a tag for missing item 99 must fail
    let fk_err = store
        .write::<(), Error, _>(|conn| {
            conn.batch_execute("INSERT INTO tags (item, tag) VALUES (99, 'rare');")?;
            Ok(())
        })
        .await;
    assert!(fk_err.is_err());

    // Busy timeout waits for a concurrent lock to release
    let path_clone = db_path.clone();
    let locked = Arc::new(Notify::new());
    let locked_clone = locked.clone();
    let holder = std::thread::spawn(move || {
        let mut ext = Connection::establish(&path_clone.to_string_lossy()).unwrap();
        ext.batch_execute("BEGIN IMMEDIATE;").unwrap();
        locked_clone.notify_one();
        std::thread::sleep(Duration::from_millis(150));
        ext.batch_execute("COMMIT;").unwrap();
    });

    locked.notified().await;
    store
        .write::<_, Error, _>(|conn| {
            conn.batch_execute("INSERT INTO items (id, name) VALUES (10, 'bow');")?;
            Ok(())
        })
        .await
        .unwrap();
    holder.join().unwrap();

    store.checkpoint().await.unwrap();
}

#[tokio::test]
async fn completes_in_flight_transaction_when_caller_future_is_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("drop.db");
    let store = Store::open(&db_path, &Options::default(), &SCHEMA_V1).unwrap();

    let entered = Arc::new(Notify::new());
    let entered_clone = entered.clone();

    let store_clone = store.clone();
    let mut write_fut = Box::pin(store_clone.write::<_, Error, _>(move |conn| {
        entered_clone.notify_one();
        std::thread::sleep(Duration::from_millis(80));
        conn.batch_execute("INSERT INTO items (id, name) VALUES (7, 'axe');")?;
        Ok(())
    }));

    tokio::select! {
        _ = &mut write_fut => panic!("write finished before caller dropped it"),
        _ = entered.notified() => {}
    }
    drop(write_fut);

    let count = store.write::<_, Error, _>(|conn| Ok(item_count(conn)?)).await.unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn recovers_from_panicking_write_without_losing_store_usability() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("panic.db");
    let store = Store::open(&db_path, &Options::default(), &SCHEMA_V1).unwrap();

    let res = store
        .write::<(), Error, _>(|conn| {
            conn.batch_execute("INSERT INTO items (id, name) VALUES (1, 'cursed');")?;
            panic!("write closure panicked");
        })
        .await;
    assert!(matches!(res, Err(Error::Panicked)));

    // Rolled back and still usable
    let count = store.read::<_, Error, _>(|conn| Ok(item_count(conn)?)).await.unwrap();
    assert_eq!(count, 0);

    store
        .write::<_, Error, _>(|conn| {
            conn.batch_execute("INSERT INTO items (id, name) VALUES (2, 'blessed');")?;
            Ok(())
        })
        .await
        .unwrap();

    let count = store.read::<_, Error, _>(|conn| Ok(item_count(conn)?)).await.unwrap();
    assert_eq!(count, 1);
}

#[test]
fn adopts_drizzle_database_and_runs_subsequent_diesel_migrations() {
    let dir = tempfile::tempdir().unwrap();
    let drizzle_dir = dir.path().join("drizzle");
    std::fs::create_dir_all(drizzle_dir.join("meta")).unwrap();
    std::fs::write(
        drizzle_dir.join("meta").join("_journal.json"),
        r#"{"entries":[{"tag":"0000_init","when":1700000000000}]}"#,
    )
    .unwrap();
    std::fs::write(
        drizzle_dir.join("0000_init.sql"),
        "CREATE TABLE items (id INTEGER PRIMARY KEY NOT NULL, name TEXT NOT NULL);\n--> statement-breakpoint\nCREATE TABLE tags (item INTEGER NOT NULL REFERENCES items (id), tag TEXT NOT NULL);\n",
    )
    .unwrap();

    let migrations = drizzle::read_folder(&drizzle_dir).unwrap();
    assert_eq!(migrations.len(), 1);
    assert!(drizzle::hash_matches(&drizzle::hash("SELECT 1;\n"), "SELECT 1;\r\n"));

    let db_path = dir.path().join("adopt.db");
    let mut conn = Connection::establish(&db_path.to_string_lossy()).unwrap();

    // Replay the Drizzle migration with the exact baseline SQL
    drizzle::replay(
        &mut conn,
        &[drizzle::Migration {
            tag: "0000_init".into(),
            created_at: 1_700_000_000_000,
            sql: UP_V1.to_owned(),
        }],
    )
    .unwrap();
    conn.batch_execute("INSERT INTO items (id, name) VALUES (1, 'preserved');")
        .unwrap();
    assert_eq!(schema_snapshot(&mut conn).unwrap(), BASELINE_SNAPSHOT);
    drop(conn);

    // Open via Store with SCHEMA_WITH_DRIZZLE_V2: adopts 0001_items and runs 0002_items_note
    let _store = Store::open(&db_path, &Options::default(), &SCHEMA_WITH_DRIZZLE_V2).unwrap();

    let mut check = Connection::establish(&db_path.to_string_lossy()).unwrap();
    assert_eq!(schema_version(&mut check).unwrap().as_deref(), Some("0002"));
    let note = diesel::sql_query("SELECT note AS val FROM items WHERE id = 1")
        .get_result::<TextRow>(&mut check)
        .unwrap();
    assert_eq!(note.val, "");
}

#[test]
fn refuses_invalid_adoption_cases() {
    let dir = tempfile::tempdir().unwrap();

    // 1. Unknown user tables without migration tracking
    let unknown_path = dir.path().join("unknown.db");
    let mut conn = Connection::establish(&unknown_path.to_string_lossy()).unwrap();
    conn.batch_execute("CREATE TABLE mystery (id INTEGER PRIMARY KEY);")
        .unwrap();
    assert!(matches!(
        migrate(&mut conn, &SCHEMA_WITH_DRIZZLE_V2),
        Err(Error::Adoption(_))
    ));

    // 2. Drizzle table present when schema.drizzle is None
    let no_baseline_path = dir.path().join("no_baseline.db");
    let mut conn = Connection::establish(&no_baseline_path.to_string_lossy()).unwrap();
    drizzle::replay(&mut conn, &[]).unwrap();
    assert!(matches!(migrate(&mut conn, &SCHEMA_V1), Err(Error::Adoption(_))));

    // 3. Empty __drizzle_migrations table
    assert!(matches!(
        migrate(&mut conn, &SCHEMA_WITH_DRIZZLE_V2),
        Err(Error::Adoption(_))
    ));

    // 4. Wrong Drizzle timestamp
    let wrong_ts_path = dir.path().join("wrong_ts.db");
    let mut conn = Connection::establish(&wrong_ts_path.to_string_lossy()).unwrap();
    drizzle::replay(
        &mut conn,
        &[drizzle::Migration {
            tag: "0000_init".into(),
            created_at: 42,
            sql: UP_V1.to_owned(),
        }],
    )
    .unwrap();
    assert!(matches!(
        migrate(&mut conn, &SCHEMA_WITH_DRIZZLE_V2),
        Err(Error::Adoption(_))
    ));

    // 5. Wrong Drizzle SQL hash
    let wrong_hash_path = dir.path().join("wrong_hash.db");
    let mut conn = Connection::establish(&wrong_hash_path.to_string_lossy()).unwrap();
    drizzle::replay(
        &mut conn,
        &[drizzle::Migration {
            tag: "0000_init".into(),
            created_at: 1_700_000_000_000,
            sql: "CREATE TABLE items (id INTEGER PRIMARY KEY NOT NULL);".into(),
        }],
    )
    .unwrap();
    assert!(matches!(
        migrate(&mut conn, &SCHEMA_WITH_DRIZZLE_V2),
        Err(Error::Adoption(_))
    ));

    // 6. Modified live schema after Drizzle migration
    let drifted_path = dir.path().join("drifted.db");
    let mut conn = Connection::establish(&drifted_path.to_string_lossy()).unwrap();
    drizzle::replay(
        &mut conn,
        &[drizzle::Migration {
            tag: "0000_init".into(),
            created_at: 1_700_000_000_000,
            sql: UP_V1.to_owned(),
        }],
    )
    .unwrap();
    conn.batch_execute("CREATE INDEX idx_extra ON items (name);").unwrap();
    assert!(matches!(
        migrate(&mut conn, &SCHEMA_WITH_DRIZZLE_V2),
        Err(Error::Adoption(_))
    ));
    assert_eq!(schema_version(&mut conn).unwrap(), None);
}

#[tokio::test]
async fn backup_produces_consistent_single_file_and_honours_deadline() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("live.db");
    let store = Store::open(&db_path, &Options::default(), &SCHEMA_V1).unwrap();

    for i in 1..=25 {
        store
            .write::<_, Error, _>(move |conn| {
                conn.batch_execute(&format!("INSERT INTO items (id, name) VALUES ({i}, 'item-{i}');"))?;
                Ok(())
            })
            .await
            .unwrap();
    }

    let writer_store = store.clone();
    let writer = tokio::spawn(async move {
        for i in 26..=50 {
            let _ = writer_store
                .write::<_, Error, _>(move |conn| {
                    conn.batch_execute(&format!("INSERT INTO items (id, name) VALUES ({i}, 'item-{i}');"))?;
                    Ok(())
                })
                .await;
        }
    });

    let backup_path = dir.path().join("backup.db");
    store.backup(&backup_path, Duration::from_secs(10)).await.unwrap();
    writer.await.unwrap();

    assert!(backup_path.is_file());
    assert!(!dir.path().join("backup.db-wal").exists());
    assert!(!dir.path().join("backup.db-shm").exists());

    let mut ro = open_read_only(&backup_path, Duration::from_secs(1)).unwrap();
    let mode = diesel::sql_query("PRAGMA journal_mode")
        .get_result::<JournalModeRow>(&mut ro)
        .unwrap();
    assert!(mode.journal_mode.eq_ignore_ascii_case("delete"));
    let count = item_count(&mut ro).unwrap();
    assert!(count >= 25);

    // Refuses overwriting existing destination
    assert!(matches!(
        backup::backup_file(&db_path, &backup_path, Duration::from_secs(1), Duration::from_secs(5),),
        Err(Error::Backup(_))
    ));

    // Zero deadline fails and removes partial file
    let expired_path = dir.path().join("expired.db");
    assert!(matches!(
        backup::backup_file(&db_path, &expired_path, Duration::from_secs(1), Duration::ZERO,),
        Err(Error::Backup(_))
    ));
    assert!(!expired_path.exists());
}

#[test]
fn read_only_connections_and_schema_version_inspection() {
    let dir = tempfile::tempdir().unwrap();

    // Missing file is refused and not created
    let missing = dir.path().join("missing.db");
    assert!(matches!(
        open_read_only(&missing, Duration::from_secs(1)),
        Err(Error::Connection(_))
    ));
    assert!(!missing.exists());

    // Special characters in path (` `, `#`, `?` on platforms that allow them; `#` and spaces on Windows)
    let special_name = if cfg!(windows) {
        "space and #hash.db"
    } else {
        "space #hash ?query.db"
    };
    let special_path = dir.path().join(special_name);
    let _store = Store::open(&special_path, &Options::default(), &SCHEMA_V1).unwrap();

    let mut ro = open_read_only(&special_path, Duration::from_secs(1)).unwrap();
    assert!(
        ro.batch_execute("INSERT INTO items (id, name) VALUES (1, 'nope');")
            .is_err()
    );

    // Read schema_version while a write transaction is open
    let mut writer = Connection::establish(&special_path.to_string_lossy()).unwrap();
    writer.batch_execute("BEGIN IMMEDIATE;").unwrap();
    assert_eq!(schema_version(&mut ro).unwrap().as_deref(), Some("0001"));
    writer.batch_execute("ROLLBACK;").unwrap();
}
