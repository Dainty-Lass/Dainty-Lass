// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Integration tests for `Store`, SQLite compile options, online backup, and read-only connections.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use diesel::connection::{Connection as _, SimpleConnection};
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Text};
use diesel_migrations::{EmbeddedMigrations, embed_migrations};
use store::{Connection, Error, Options, Schema, Store, open_read_only, schema_version};

const MIGRATIONS: EmbeddedMigrations = embed_migrations!("tests/migrations");
const SCHEMA: Schema = Schema {
    migrations: || MIGRATIONS,
    drizzle: None,
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

fn temp_store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.db");
    let store = Store::open(&path, &Options::default(), &SCHEMA).unwrap();
    (dir, store)
}

#[tokio::test(flavor = "multi_thread")]
async fn writes_commit_and_errors_roll_back() {
    let (_dir, store) = temp_store();

    store
        .write(|conn| {
            conn.batch_execute("INSERT INTO items (id, name) VALUES (1, 'sword');")?;
            Ok::<_, Error>(())
        })
        .await
        .unwrap();

    let failed: Result<(), Error> = store
        .write(|conn| {
            conn.batch_execute("INSERT INTO items (id, name) VALUES (2, 'shield');")?;
            Err(Error::Migration("abort".into()))
        })
        .await;
    assert!(failed.is_err());

    let count = store
        .read(|conn| {
            let row = diesel::sql_query("SELECT count(*) AS n FROM items").get_result::<CountRow>(conn)?;
            Ok::<_, Error>(row.n)
        })
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn reads_cannot_write() {
    let (_dir, store) = temp_store();

    let res: Result<(), Error> = store
        .read(|conn| {
            conn.batch_execute("INSERT INTO items (id, name) VALUES (1, 'illegal');")?;
            Ok(())
        })
        .await;
    assert!(res.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_database_is_in_wal_mode() {
    let (_dir, store) = temp_store();

    let mode = store
        .read(|conn| {
            let row = diesel::sql_query("PRAGMA journal_mode")
                .get_result::<TextRow>(conn)
                .or_else(|_| {
                    #[derive(QueryableByName)]
                    struct Mode {
                        #[diesel(sql_type = Text)]
                        journal_mode: String,
                    }
                    diesel::sql_query("PRAGMA journal_mode")
                        .get_result::<Mode>(conn)
                        .map(|m| TextRow { val: m.journal_mode })
                })?;
            Ok::<_, Error>(row.val)
        })
        .await
        .unwrap();
    assert!(mode.eq_ignore_ascii_case("wal"), "got {mode}");
    store.checkpoint().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_behaves_like_better_sqlite3() {
    let (_dir, store) = temp_store();

    // 1. Double-quoted string literals are rejected (SQLITE_DQS=0)
    let dqs: Result<(), Error> = store
        .read(|conn| {
            conn.batch_execute("SELECT \"not_a_column\";")?;
            Ok(())
        })
        .await;
    assert!(dqs.is_err(), "double-quoted string literals should be rejected");

    // 2. Foreign keys are enforced
    let fk: Result<(), Error> = store
        .write(|conn| {
            conn.batch_execute("INSERT INTO tags (item, tag) VALUES (999, 'orphan');")?;
            Ok(())
        })
        .await;
    assert!(fk.is_err(), "foreign key constraint should be enforced");

    // 3. Math functions exist (e.g. sqrt)
    let math: i64 = store
        .read(|conn| {
            let row = diesel::sql_query("SELECT CAST(sqrt(16) AS INTEGER) AS n").get_result::<CountRow>(conn)?;
            Ok::<_, Error>(row.n)
        })
        .await
        .unwrap();
    assert_eq!(math, 4);

    // 4. LIKE does not match BLOBs (SQLITE_LIKE_DOESNT_MATCH_BLOBS)
    let like_blob: i64 = store
        .read(|conn| {
            let row =
                diesel::sql_query("SELECT CAST(X'616263' LIKE '%b%' AS INTEGER) AS n").get_result::<CountRow>(conn)?;
            Ok::<_, Error>(row.n)
        })
        .await
        .unwrap();
    assert_eq!(like_blob, 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dropped_request_still_finishes_its_transaction() {
    let (_dir, store) = temp_store();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel::<()>();
    let (proceed_tx, proceed_rx) = std::sync::mpsc::channel::<()>();

    let store_clone = store.clone();
    let handle = tokio::spawn(async move {
        let _ = store_clone
            .write(move |conn| {
                conn.batch_execute("INSERT INTO items (id, name) VALUES (1, 'committed');")?;
                let _ = started_tx.send(());
                let _ = proceed_rx.recv();
                conn.batch_execute("INSERT INTO items (id, name) VALUES (2, 'also_committed');")?;
                Ok::<_, Error>(())
            })
            .await;
    });

    started_rx.await.unwrap();
    handle.abort();
    let _ = handle.await;
    let _ = proceed_tx.send(());

    let count = store
        .write(|conn| {
            let row = diesel::sql_query("SELECT count(*) AS n FROM items").get_result::<CountRow>(conn)?;
            Ok::<_, Error>(row.n)
        })
        .await
        .unwrap();
    assert_eq!(count, 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_panicking_closure_rolls_back_and_the_store_keeps_working() {
    let (_dir, store) = temp_store();

    let panicked: Result<(), Error> = store
        .write(|conn| {
            conn.batch_execute("INSERT INTO items (id, name) VALUES (1, 'before_panic');")?;
            panic!("boom");
        })
        .await;
    assert!(matches!(panicked, Err(Error::Panicked)));

    store
        .write(|conn| {
            conn.batch_execute("INSERT INTO items (id, name) VALUES (2, 'after_panic');")?;
            Ok::<_, Error>(())
        })
        .await
        .unwrap();

    let count = store
        .read(|conn| {
            let row = diesel::sql_query("SELECT count(*) AS n FROM items").get_result::<CountRow>(conn)?;
            Ok::<_, Error>(row.n)
        })
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn a_database_with_unknown_tables_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("unknown.db");
    {
        let mut conn = Connection::establish(&path.to_string_lossy()).unwrap();
        conn.batch_execute("CREATE TABLE mystery (id INTEGER PRIMARY KEY);")
            .unwrap();
    }
    let res = Store::open(&path, &Options::default(), &SCHEMA);
    assert!(matches!(res, Err(Error::Adoption(_))));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_backup_under_constant_writes_is_one_consistent_snapshot() {
    let (dir, store) = temp_store();
    let stop = Arc::new(AtomicBool::new(false));
    let writer_stop = stop.clone();
    let writer_store = store.clone();

    let writer = tokio::spawn(async move {
        let mut i = 1i64;
        while !writer_stop.load(Ordering::Relaxed) {
            let id = i;
            let _ = writer_store
                .write(move |conn| {
                    conn.batch_execute(&format!(
                        "INSERT INTO items (id, name) VALUES ({id}, 'item-{id}');
                         INSERT INTO tags (item, tag) VALUES ({id}, 'tag-{id}');"
                    ))?;
                    Ok::<_, Error>(())
                })
                .await;
            i += 1;
        }
    });

    // Wait until some writes have landed
    loop {
        let n = store
            .read(|conn| {
                let row = diesel::sql_query("SELECT count(*) AS n FROM items").get_result::<CountRow>(conn)?;
                Ok::<_, Error>(row.n)
            })
            .await
            .unwrap();
        if n >= 5 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    let backup_path = dir.path().join("backup.db");
    store.backup(&backup_path, Duration::from_secs(10)).await.unwrap();

    stop.store(true, Ordering::Relaxed);
    let _ = writer.await;

    let mut conn = open_read_only(&backup_path, Duration::from_secs(5)).unwrap();
    let items_count = diesel::sql_query("SELECT count(*) AS n FROM items")
        .get_result::<CountRow>(&mut conn)
        .unwrap()
        .n;
    let tags_count = diesel::sql_query("SELECT count(*) AS n FROM tags")
        .get_result::<CountRow>(&mut conn)
        .unwrap()
        .n;
    assert!(items_count >= 5);
    assert_eq!(items_count, tags_count);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_read_only_connection_reads_but_refuses_writes() {
    let (dir, store) = temp_store();
    store
        .write(|conn| {
            conn.batch_execute("INSERT INTO items (id, name) VALUES (1, 'hello');")?;
            Ok::<_, Error>(())
        })
        .await
        .unwrap();

    let path = dir.path().join("test.db");
    let mut ro = open_read_only(&path, Duration::from_secs(5)).unwrap();
    let count = diesel::sql_query("SELECT count(*) AS n FROM items")
        .get_result::<CountRow>(&mut ro)
        .unwrap()
        .n;
    assert_eq!(count, 1);

    let err = ro.batch_execute("INSERT INTO items (id, name) VALUES (2, 'nope');");
    assert!(err.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_schema_version_is_readable_while_a_write_is_held() {
    let (dir, store) = temp_store();
    let path = dir.path().join("test.db");

    let (held_tx, held_rx) = tokio::sync::oneshot::channel::<()>();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();

    let store_clone = store.clone();
    let writer = tokio::spawn(async move {
        store_clone
            .write(move |conn| {
                conn.batch_execute("INSERT INTO items (id, name) VALUES (1, 'holding');")?;
                let _ = held_tx.send(());
                let _ = release_rx.recv();
                Ok::<_, Error>(())
            })
            .await
            .unwrap();
    });

    held_rx.await.unwrap();

    let mut ro = open_read_only(&path, Duration::from_secs(5)).unwrap();
    let version = schema_version(&mut ro).unwrap();
    assert_eq!(version.as_deref(), Some("0001"));

    let _ = release_tx.send(());
    writer.await.unwrap();
}
