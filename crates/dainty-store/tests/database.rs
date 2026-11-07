// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `Database` on both engines (DD15) and its change log (DD23, DH1). SQLite always; Postgres when
//! `REFORGED_TEST_POSTGRES_URL` names a server the tests may create databases on (`postgres://user:pw@
//! 127.0.0.1:5432/postgres`), and skipped otherwise, like the nftables test.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use dainty_store::PgConnection;
use dainty_store::{
    Actor, AnyConnection, ChangeEntry, Database, DatabaseConfig, DatabaseOptions, DatabaseSchema, Engine, Error, Head,
    KeyValue, PostgresConfig, history, portable, transfer,
};
use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Text};
use diesel_migrations::{EmbeddedMigrations, embed_migrations};
use serde_json::Value;

const SQLITE: EmbeddedMigrations = embed_migrations!("tests/db/sqlite");
const POSTGRES: EmbeddedMigrations = embed_migrations!("tests/db/postgres");
const SCHEMA: DatabaseSchema = DatabaseSchema {
    sqlite: || SQLITE,
    postgres: || POSTGRES,
    soft_delete: &["accounts"],
};

diesel::table! {
    /// The test schema's soft-deletable accounts.
    accounts (id) {
        /// Primary key.
        id -> BigInt,
        /// Display name.
        name -> Text,
        /// Admin flag.
        is_admin -> Bool,
        /// A nullable float.
        score -> Nullable<Double>,
        /// A nullable blob.
        avatar -> Nullable<Binary>,
        /// Soft-delete time, unix milliseconds.
        deleted_at -> Nullable<BigInt>,
    }
}

diesel::table! {
    /// Items keyed by account and slot, with a renamed column.
    items (account_id, slot) {
        /// Owning account.
        account_id -> BigInt,
        /// Inventory slot.
        slot -> Integer,
        /// Stored as `itemName`.
        #[sql_name = "itemName"]
        item_name -> Text,
    }
}

// ------------------------------------------------------------------ the engines

/// A throwaway Postgres database, dropped with the guard.
struct PgDatabase {
    server: PostgresConfig,
    config: PostgresConfig,
}

impl Drop for PgDatabase {
    fn drop(&mut self) {
        if let Ok(mut admin) = PgConnection::establish(&self.server.conninfo()) {
            let _ = admin.batch_execute(&format!(
                "DROP DATABASE IF EXISTS {} WITH (FORCE)",
                portable::quote_ident(&self.config.database)
            ));
        }
    }
}

fn postgres_server() -> Option<PostgresConfig> {
    let url = std::env::var("REFORGED_TEST_POSTGRES_URL").ok()?;
    let url = url::Url::parse(&url).expect("REFORGED_TEST_POSTGRES_URL is a postgres:// URL");
    Some(PostgresConfig {
        host: url.host_str().unwrap_or("127.0.0.1").to_owned(),
        port: url.port().unwrap_or(5432),
        user: url.username().to_owned(),
        database: url.path().trim_start_matches('/').to_owned(),
        password: url.password().unwrap_or_default().to_owned(),
        allow_remote: false,
    })
}

static NEXT: AtomicU32 = AtomicU32::new(0);

fn new_postgres() -> Option<PgDatabase> {
    let server = postgres_server()?;
    let name = format!(
        "store_test_{}_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    );
    PgConnection::establish(&server.conninfo())
        .expect("REFORGED_TEST_POSTGRES_URL is reachable")
        .batch_execute(&format!("CREATE DATABASE {}", portable::quote_ident(&name)))
        .unwrap();
    let config = PostgresConfig {
        database: name,
        ..server.clone()
    };
    Some(PgDatabase { server, config })
}

enum Target {
    Sqlite(tempfile::TempDir),
    Postgres(PgDatabase),
}

impl Target {
    fn config(&self) -> DatabaseConfig {
        match self {
            Self::Sqlite(dir) => DatabaseConfig::Sqlite {
                file: dir.path().join("test.db"),
            },
            Self::Postgres(pg) => DatabaseConfig::Postgres(pg.config.clone()),
        }
    }

    fn open(&self) -> Database {
        Database::open(&self.config(), &DatabaseOptions::default(), &SCHEMA).unwrap()
    }

    /// A connection of its own, outside `Database`: what a repository writing around `store` would use.
    fn raw(&self) -> AnyConnection {
        match self {
            Self::Sqlite(dir) => AnyConnection::Sqlite(
                SqliteConnection::establish(&dir.path().join("test.db").to_string_lossy()).unwrap(),
            ),
            Self::Postgres(pg) => AnyConnection::Postgres(PgConnection::establish(&pg.config.conninfo()).unwrap()),
        }
    }
}

fn sqlite() -> Target {
    Target::Sqlite(tempfile::tempdir().unwrap())
}

/// SQLite, and Postgres when it is configured.
fn targets() -> Vec<Target> {
    let mut out = vec![sqlite()];
    match new_postgres() {
        Some(pg) => out.push(Target::Postgres(pg)),
        None => eprintln!("REFORGED_TEST_POSTGRES_URL is not set: the Postgres half is skipped"),
    }
    out
}

// ------------------------------------------------------------------ a repository

fn create_account(conn: &mut AnyConnection, name: &str) -> QueryResult<i64> {
    diesel::insert_into(accounts::table)
        .values((
            accounts::name.eq(name),
            accounts::is_admin.eq(true),
            accounts::score.eq(Some(1.0 / 3.0)),
            accounts::avatar.eq(Some(vec![1u8, 2, 255])),
        ))
        .returning(accounts::id)
        .get_result(conn)
}

fn rename(conn: &mut AnyConnection, id: i64, name: &str) -> QueryResult<usize> {
    diesel::update(accounts::table.find(id))
        .set(accounts::name.eq(name))
        .execute(conn)
}

/// An upsert: `on_conflict` doesn't compile through `MultiConnection`, so portable raw SQL.
fn put_item(conn: &mut AnyConnection, account: i64, slot: i32, name: &str) -> QueryResult<usize> {
    diesel::sql_query(portable::upsert(
        "items",
        &["account_id", "slot", "itemName"],
        &["account_id", "slot"],
        &["itemName"],
    ))
    .bind::<BigInt, _>(account)
    .bind::<Integer, _>(slot)
    .bind::<Text, _>(name)
    .execute(conn)
}

fn drop_item(conn: &mut AnyConnection, account: i64, slot: i32) -> QueryResult<usize> {
    diesel::delete(items::table.find((account, slot))).execute(conn)
}

async fn entries(db: &Database) -> Vec<ChangeEntry> {
    db.changes(0, 1000)
        .await
        .unwrap()
        .into_iter()
        .filter(|e| e.op != "migrate")
        .collect()
}

fn json(text: &Option<String>) -> Value {
    text.as_deref()
        .map_or(Value::Null, |t| serde_json::from_str(t).unwrap())
}

fn player() -> Actor {
    Actor::Player("7".into())
}

// ------------------------------------------------------------------ the tests

#[tokio::test(flavor = "multi_thread")]
async fn every_change_is_logged_with_its_actor_before_and_after() {
    for target in targets() {
        let db = target.open();
        let engine = db.engine();
        let id = db
            .write(player(), |tx| {
                let id = create_account(tx.conn(), "alex")?;
                rename(tx.conn(), id, "alexa")?;
                put_item(tx.conn(), id, 1, "Sword")?;
                put_item(tx.conn(), id, 1, "Axe")?;
                drop_item(tx.conn(), id, 1)?;
                Ok::<_, Error>(id)
            })
            .await
            .unwrap();
        db.write(Actor::Admin("owner".into()), move |tx| {
            assert!(tx.soft_delete("accounts", &[("id", KeyValue::Int(id))])?);
            assert!(
                !tx.soft_delete("accounts", &[("id", KeyValue::Int(id))])?,
                "already deleted"
            );
            assert!(tx.restore("accounts", &[("id", KeyValue::Int(id))])?);
            assert!(tx.soft_delete("items", &[("slot", KeyValue::Int(1))]).is_err());
            Ok::<_, Error>(())
        })
        .await
        .unwrap();

        let log = entries(&db).await;
        let ops: Vec<(&str, &str)> = log.iter().map(|e| (e.table_name.as_str(), e.op.as_str())).collect();
        assert_eq!(
            ops,
            [
                ("accounts", "insert"),
                ("accounts", "update"),
                ("items", "insert"),
                ("items", "update"),
                ("items", "delete"),
                ("accounts", "soft_delete"),
                ("accounts", "restore"),
            ],
            "{engine:?}"
        );
        assert!(log[..5].iter().all(|e| e.actor == "player:7"));
        assert!(log[5..].iter().all(|e| e.actor == "admin:owner"));
        assert!(log[..5].iter().all(|e| e.at == log[0].at), "one write, one time");
        assert!(log.windows(2).all(|w| w[0].id < w[1].id));

        let insert = &log[0];
        assert_eq!(json(&Some(insert.row_key.clone())), serde_json::json!({ "id": id }));
        assert_eq!(json(&insert.before), Value::Null);
        let after = json(&insert.after);
        assert_eq!(after["name"], "alex");
        assert_eq!(
            after["avatar"], "\\x0102ff",
            "{engine:?}: blobs as Postgres writes bytea"
        );
        assert!(after["is_admin"] == 1 || after["is_admin"] == true);
        assert_eq!(after["deleted_at"], Value::Null);
        assert_eq!(json(&log[1].before)["name"], "alex");
        assert_eq!(json(&log[1].after)["name"], "alexa");
        assert_eq!(
            json(&Some(log[3].row_key.clone())),
            serde_json::json!({ "account_id": id, "slot": 1 })
        );
        assert_eq!(json(&log[3].before)["itemName"], "Sword");
        assert_eq!(json(&log[3].after)["itemName"], "Axe");
        assert_eq!(json(&log[4].before)["itemName"], "Axe");
        assert_eq!(json(&log[4].after), Value::Null);
        assert!(json(&log[5].after)["deleted_at"].is_i64());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_write_leaves_neither_rows_nor_entries() {
    for target in targets() {
        let db = target.open();
        let failed: Result<(), Error> = db
            .write(player(), |tx| {
                create_account(tx.conn(), "lost")?;
                Err(Error::Schema("refused".into()))
            })
            .await;
        assert!(failed.is_err());
        assert!(entries(&db).await.is_empty());
        let count: i64 = db
            .read(|c| Ok::<_, Error>(accounts::table.count().get_result(c)?))
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
}

// DH1: a repository that writes around `store` fails.
#[tokio::test(flavor = "multi_thread")]
async fn a_write_around_store_is_refused() {
    for target in targets() {
        let db = target.open();
        let mut raw = target.raw();
        let around = raw.batch_execute("INSERT INTO notes (body) VALUES ('sneaky')");
        let message = around.expect_err("a write outside Database::write").to_string();
        assert!(message.contains("outside store's write path"), "{message}");
        assert!(create_account(&mut raw, "sneaky").is_err());
        assert!(entries(&db).await.is_empty());

        let read_write = db
            .read(|c| {
                c.batch_execute("INSERT INTO notes (body) VALUES ('x')")
                    .map_err(Error::from)
            })
            .await;
        assert!(read_write.is_err(), "reads can't write");

        db.write(player(), |tx| {
            tx.conn().batch_execute("INSERT INTO notes (body) VALUES ('fine')")?;
            Ok::<_, Error>(())
        })
        .await
        .unwrap();
        let erase = db
            .write(player(), |tx| {
                tx.conn()
                    .batch_execute("DELETE FROM __store_change_log")
                    .map_err(Error::from)
            })
            .await;
        assert!(erase.is_err(), "the log is append-only");
        let rewrite = db
            .write(player(), |tx| {
                tx.conn()
                    .batch_execute("UPDATE __store_change_log SET actor = 'player:8'")
                    .map_err(Error::from)
            })
            .await;
        assert!(rewrite.is_err(), "the log is append-only");
        assert_eq!(entries(&db).await.len(), 1);
        // A write after the context is gone is refused again.
        assert!(raw.batch_execute("INSERT INTO notes (body) VALUES ('later')").is_err());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn old_soft_deletes_are_purged_and_the_purge_is_logged() {
    for target in targets() {
        let db = target.open();
        let (old, recent) = db
            .write(player(), |tx| {
                let old = create_account(tx.conn(), "old")?;
                let recent = create_account(tx.conn(), "recent")?;
                diesel::update(accounts::table.find(old))
                    .set(accounts::deleted_at.eq(Some(1i64)))
                    .execute(tx.conn())?;
                tx.soft_delete("accounts", &[("id", KeyValue::Int(recent))])?;
                Ok::<_, Error>((old, recent))
            })
            .await
            .unwrap();
        let purged = db.purge_soft_deleted(Duration::from_secs(3600)).await.unwrap();
        assert_eq!(purged, [("accounts".to_owned(), 1)]);
        let left: Vec<i64> = db
            .read(|c| Ok::<_, Error>(accounts::table.select(accounts::id).load(c)?))
            .await
            .unwrap();
        assert_eq!(left, [recent]);
        let last = entries(&db).await.pop().unwrap();
        assert_eq!((last.op.as_str(), last.actor.as_str()), ("delete", "system:retention"));
        assert_eq!(json(&last.before)["id"], old);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn migrations_are_logged_once_and_a_table_without_a_key_is_refused() {
    for target in targets() {
        let db = target.open();
        let migrations = |log: Vec<ChangeEntry>| log.into_iter().filter(|e| e.op == "migrate").count();
        assert_eq!(migrations(db.changes(0, 100).await.unwrap()), 1);
        drop(db);
        let again = target.open();
        assert_eq!(
            migrations(again.changes(0, 100).await.unwrap()),
            1,
            "nothing new to apply"
        );
        drop(again);

        const LOOSE_SQLITE: EmbeddedMigrations = embed_migrations!("tests/nopk/sqlite");
        const LOOSE_POSTGRES: EmbeddedMigrations = embed_migrations!("tests/nopk/postgres");
        let fresh = match target {
            Target::Sqlite(_) => sqlite(),
            Target::Postgres(_) => Target::Postgres(new_postgres().unwrap()),
        };
        let loose = DatabaseSchema {
            sqlite: || LOOSE_SQLITE,
            postgres: || LOOSE_POSTGRES,
            soft_delete: &[],
        };
        let refused = Database::open(&fresh.config(), &DatabaseOptions::default(), &loose);
        assert!(matches!(refused, Err(Error::Schema(ref m)) if m.contains("no primary key")));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn writes_are_serial_and_the_log_keeps_their_order() {
    for target in targets() {
        let db = target.open();
        let writes = (0..16).map(|i| {
            let db = db.clone();
            tokio::spawn(async move {
                db.write(Actor::Player(i.to_string()), move |tx| {
                    tx.conn()
                        .batch_execute(&format!("INSERT INTO notes (body) VALUES ('{i}')"))?;
                    Ok::<_, Error>(())
                })
                .await
            })
        });
        for write in writes.collect::<Vec<_>>() {
            write.await.unwrap().unwrap();
        }
        let log = entries(&db).await;
        assert_eq!(log.len(), 16);
        assert!(
            log.windows(2).all(|w| w[0].at <= w[1].at),
            "times follow the log's order"
        );
    }
}

#[test]
fn a_postgres_server_off_this_machine_is_refused_before_connecting() {
    let far = DatabaseConfig::Postgres(PostgresConfig {
        host: "203.0.113.9".into(),
        port: 5432,
        user: "dainty".into(),
        database: "dainty".into(),
        password: "pw".into(),
        allow_remote: false,
    });
    let refused = Database::open(&far, &DatabaseOptions::default(), &SCHEMA);
    assert!(matches!(refused, Err(Error::Config(ref m)) if m.contains("not encrypted")));
}

#[test]
fn a_refused_postgres_login_says_why() {
    let Some(pg) = new_postgres() else { return };
    let wrong = DatabaseConfig::Postgres(PostgresConfig {
        password: "not the password".into(),
        ..pg.config.clone()
    });
    let refused = Database::open(&wrong, &DatabaseOptions::default(), &SCHEMA)
        .err()
        .unwrap();
    assert!(
        refused.to_string().contains("password authentication failed"),
        "{refused}"
    );
}

// ------------------------------------------------------------------ db move

async fn fill(db: &Database) -> i64 {
    db.write(player(), |tx| {
        let a = create_account(tx.conn(), "alex")?;
        let b = create_account(tx.conn(), "it's \"quoted\" ünïcode")?;
        diesel::update(accounts::table.find(b))
            .set((
                accounts::score.eq(None::<f64>),
                accounts::avatar.eq(Some(Vec::<u8>::new())),
            ))
            .execute(tx.conn())?;
        put_item(tx.conn(), a, 1, "Sword")?;
        put_item(tx.conn(), b, 2, "Axe")?;
        tx.conn()
            .batch_execute("INSERT INTO notes (body) VALUES ('one'); INSERT INTO notes (body) VALUES ('two')")?;
        tx.soft_delete("accounts", &[("id", KeyValue::Int(b))])?;
        Ok::<_, Error>(a)
    })
    .await
    .unwrap()
}

async fn scores(db: &Database) -> Vec<Option<f64>> {
    db.read(|c| Ok::<_, Error>(accounts::table.order(accounts::id).select(accounts::score).load(c)?))
        .await
        .unwrap()
}

/// Moves `from` into `to` and checks both hold the same data.
fn moved(from: &Target, to: &Target) {
    let report = transfer::move_database(&from.config(), &to.config(), &SCHEMA, &DatabaseOptions::default()).unwrap();
    assert!(report.tables.contains(&("accounts".to_owned(), 2)), "{report:?}");
    assert_eq!(
        transfer::dump(&from.config()).unwrap(),
        transfer::dump(&to.config()).unwrap()
    );
}

/// New rows get ids past the copied ones, and the log goes on after the copied entries.
async fn keeps_working(target: &Target) {
    let db = target.open();
    let before = entries(&db).await;
    db.write(player(), |tx| {
        create_account(tx.conn(), "after the move")?;
        tx.conn().batch_execute("INSERT INTO notes (body) VALUES ('three')")?;
        Ok::<_, Error>(())
    })
    .await
    .unwrap();
    let after = entries(&db).await;
    assert_eq!(after.len(), before.len() + 2);
    assert!(after[before.len()].id > before.last().unwrap().id);
    let verified = db.verify(None).await.unwrap();
    assert_eq!(verified.broken, None, "the chain moves with the log and goes on");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_database_moves_between_engines_and_back_unchanged() {
    let original = sqlite();
    fill(&original.open()).await;
    let copy = sqlite();
    moved(&original, &copy);
    let refused = transfer::move_database(&original.config(), &copy.config(), &SCHEMA, &DatabaseOptions::default());
    assert!(matches!(refused, Err(Error::Move(ref m)) if m.contains("already holds data")));
    keeps_working(&copy).await;

    let Some(pg) = new_postgres() else {
        eprintln!("REFORGED_TEST_POSTGRES_URL is not set: the move to Postgres is skipped");
        return;
    };
    let pg = Target::Postgres(pg);
    moved(&original, &pg);
    let back = sqlite();
    moved(&pg, &back);
    assert_eq!(
        transfer::dump(&original.config()).unwrap(),
        transfer::dump(&back.config()).unwrap(),
        "SQLite to Postgres and back"
    );
    assert_eq!(
        scores(&original.open()).await,
        scores(&back.open()).await,
        "doubles exactly"
    );
    assert_eq!(back.open().engine(), Engine::Sqlite);
    keeps_working(&pg).await;
}

// ------------------------------------------------------------------ the hash chain (DH5)

async fn chained(db: &Database) -> Vec<ChangeEntry> {
    db.changes(0, 1000).await.unwrap()
}

/// Five writes of one entry each: an account named `alex...`, renamed four times.
async fn five_writes(db: &Database) -> i64 {
    let name = format!("alex-{}", NEXT.fetch_add(1, Ordering::Relaxed));
    let id = db
        .write(player(), {
            let name = name.clone();
            move |tx| Ok::<_, Error>(create_account(tx.conn(), &name)?)
        })
        .await
        .unwrap();
    for n in 1..5 {
        let renamed = format!("{name}.{n}");
        db.write(player(), move |tx| Ok::<_, Error>(rename(tx.conn(), id, &renamed)?))
            .await
            .unwrap();
    }
    id
}

/// What root on the server can do: the log's guards off, then `sql`.
fn tamper(target: &Target, sql: &str) {
    let mut raw = target.raw();
    match target {
        Target::Sqlite(_) => raw
            .batch_execute(&format!(
                "DROP TRIGGER IF EXISTS \"__store_change_log_no_update\";
                 DROP TRIGGER IF EXISTS \"__store_change_log_no_delete\"; {sql}"
            ))
            .unwrap(),
        Target::Postgres(_) => raw
            .batch_execute(&format!(
                "ALTER TABLE \"__store_change_log\" DISABLE TRIGGER USER; {sql};
                 ALTER TABLE \"__store_change_log\" ENABLE TRIGGER USER;"
            ))
            .unwrap(),
    }
}

/// Rewrites entries from `from` on with `edit` applied to the first, each hash recomputed, as someone who
/// knows the encoding would.
fn rewrite(target: &Target, from: i64, edit: impl Fn(&mut ChangeEntry)) {
    let mut raw = target.raw();
    let log = history::changes(&mut raw, 0, 1000).unwrap();
    let mut prev = log.iter().rfind(|e| e.id < from).and_then(|e| e.hash.clone()).unwrap();
    let mut statements = Vec::new();
    for mut entry in log.into_iter().filter(|e| e.id >= from) {
        if entry.id == from {
            edit(&mut entry);
        }
        let hash = history::entry_hash(&prev, &entry);
        statements.push(format!(
            "UPDATE \"__store_change_log\" SET \"after\" = {}, \"prev_hash\" = {}, \"hash\" = {} WHERE id = {}",
            entry
                .after
                .as_deref()
                .map_or("NULL".to_owned(), portable::quote_literal),
            portable::quote_literal(&prev),
            portable::quote_literal(&hash),
            entry.id
        ));
        prev = hash;
    }
    tamper(target, &statements.join(";\n"));
}

fn edited(entry: &mut ChangeEntry) {
    entry.after = entry.after.as_ref().map(|a| a.replace("alex", "mallory"));
}

fn verify_after(target: &Target, head: &Head) -> history::Verification {
    let mut raw = target.raw();
    history::verify_after(&mut raw, head).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn every_entry_is_chained_from_genesis_and_verifies() {
    for target in targets() {
        let db = target.open();
        five_writes(&db).await;
        let log = chained(&db).await;
        assert_eq!(log.len(), 6, "a migration and five writes");
        assert_eq!(log[0].prev_hash.as_deref(), Some(history::GENESIS));
        for pair in log.windows(2) {
            assert_eq!(pair[1].prev_hash, pair[0].hash);
        }
        for entry in &log {
            let hash = history::entry_hash(entry.prev_hash.as_deref().unwrap(), entry);
            assert_eq!(entry.hash.as_deref(), Some(hash.as_str()));
        }
        let head = db.head().await.unwrap().unwrap();
        assert_eq!((head.id, Some(head.hash.clone())), (log[5].id, log[5].hash.clone()));
        let verified = db.verify(Some(head.clone())).await.unwrap();
        assert_eq!(verified.broken, None, "{:?}", db.engine());
        assert_eq!((verified.entries, verified.head), (6, Some(head.clone())));

        // A shipped head, then more writes: the range after it checks out on its own.
        five_writes(&db).await;
        let range = verify_after(&target, &head);
        assert_eq!((range.entries, range.broken), (5, None));
        assert_eq!(range.head, db.head().await.unwrap());
        // Records read back chain as the log did.
        let mut chain = history::Chain::genesis();
        for entry in chained(&db).await {
            let line = history::record::encode(&entry).unwrap();
            chain.check(&history::record::decode(line.as_bytes()).unwrap()).unwrap();
        }
        assert_eq!(chain.head(), db.head().await.unwrap());
    }
}

// DH5's Verify: an entry edited or removed locally is found and named.
#[tokio::test(flavor = "multi_thread")]
async fn an_entry_edited_or_removed_is_found_and_named() {
    for postgres in targets().iter().map(|t| matches!(t, Target::Postgres(_))) {
        let fresh = || {
            if postgres {
                Target::Postgres(new_postgres().unwrap())
            } else {
                sqlite()
            }
        };
        let log = "\"__store_change_log\"";

        // Edited, its hash left as it was.
        let target = fresh();
        let db = target.open();
        five_writes(&db).await;
        let ids: Vec<i64> = chained(&db).await.iter().map(|e| e.id).collect();
        tamper(
            &target,
            &format!(
                "UPDATE {log} SET \"after\" = replace(\"after\", 'alex', 'mallory') WHERE id = {}",
                ids[3]
            ),
        );
        let found = db.verify(None).await.unwrap();
        assert_eq!(
            found.broken,
            Some(history::Break::Edited { id: ids[3] }),
            "postgres {postgres}"
        );
        assert_eq!((found.entries, found.head.map(|h| h.id)), (3, Some(ids[2])));

        // Removed.
        let target = fresh();
        let db = target.open();
        five_writes(&db).await;
        let ids: Vec<i64> = chained(&db).await.iter().map(|e| e.id).collect();
        tamper(&target, &format!("DELETE FROM {log} WHERE id = {}", ids[3]));
        let found = db.verify(None).await.unwrap().broken.unwrap();
        assert_eq!(
            found,
            history::Break::Unlinked {
                id: ids[4],
                after: Some(ids[2])
            }
        );
        assert!(
            found.to_string().contains(&format!("doesn't follow entry {}", ids[2])),
            "{found}"
        );

        // Edited with its own hash recomputed: the next entry no longer follows it.
        let target = fresh();
        let db = target.open();
        five_writes(&db).await;
        let ids: Vec<i64> = chained(&db).await.iter().map(|e| e.id).collect();
        let mut entry = chained(&db).await.remove(3);
        edited(&mut entry);
        let hash = history::entry_hash(entry.prev_hash.as_deref().unwrap(), &entry);
        tamper(
            &target,
            &format!(
                "UPDATE {log} SET \"after\" = {}, \"hash\" = {} WHERE id = {}",
                portable::quote_literal(entry.after.as_deref().unwrap()),
                portable::quote_literal(&hash),
                ids[3]
            ),
        );
        assert_eq!(
            db.verify(None).await.unwrap().broken,
            Some(history::Break::Unlinked {
                id: ids[4],
                after: Some(ids[3])
            })
        );

        // Rewritten from an entry on, every hash recomputed: only a head kept elsewhere tells.
        let target = fresh();
        let db = target.open();
        five_writes(&db).await;
        let shipped = db.head().await.unwrap().unwrap();
        let ids: Vec<i64> = chained(&db).await.iter().map(|e| e.id).collect();
        rewrite(&target, ids[2], edited);
        assert_eq!(db.verify(None).await.unwrap().broken, None, "consistent with itself");
        assert_eq!(
            db.verify(Some(shipped.clone())).await.unwrap().broken,
            Some(history::Break::HeadMismatch { id: shipped.id })
        );
        assert_eq!(
            verify_after(&target, &shipped).broken,
            Some(history::Break::HeadMismatch { id: shipped.id })
        );

        // Cut short: the last entry gone.
        let target = fresh();
        let db = target.open();
        five_writes(&db).await;
        let shipped = db.head().await.unwrap().unwrap();
        tamper(&target, &format!("DELETE FROM {log} WHERE id = {}", shipped.id));
        assert_eq!(db.verify(None).await.unwrap().broken, None);
        let missing = Some(history::Break::HeadMissing { id: shipped.id });
        assert_eq!(db.verify(Some(shipped.clone())).await.unwrap().broken, missing);
        assert_eq!(verify_after(&target, &shipped).broken, missing);

        // Put in around store, without a hash.
        let target = fresh();
        let db = target.open();
        five_writes(&db).await;
        target
            .raw()
            .batch_execute(&format!(
                "INSERT INTO {log} (\"at\", \"actor\", \"table_name\", \"row_key\", \"op\", \"after\")
                 VALUES (1, 'player:7', 'notes', '{{\"id\":9}}', 'insert', '{{\"id\":9}}')"
            ))
            .unwrap();
        let found = db.verify(None).await.unwrap().broken.unwrap();
        assert!(matches!(found, history::Break::Unsealed { .. }), "{found:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_write_can_seal_nothing_itself() {
    for target in targets() {
        let db = target.open();
        five_writes(&db).await;
        for sql in [
            "UPDATE __store_change_log SET prev_hash = 'x', hash = 'y' WHERE hash IS NULL",
            "UPDATE __store_change_log SET hash = NULL",
            "UPDATE __store_change_log SET hash = 'y' WHERE id = (SELECT max(id) FROM __store_change_log)",
        ] {
            let refused = db
                .write(player(), move |tx| {
                    tx.conn().batch_execute("INSERT INTO notes (body) VALUES ('x')")?;
                    tx.conn().batch_execute(sql)?;
                    Ok::<_, Error>(())
                })
                .await;
            assert!(refused.is_err(), "{sql}");
        }
        assert_eq!(chained(&db).await.len(), 6);
        assert_eq!(db.verify(None).await.unwrap().broken, None);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_log_written_before_the_chain_is_sealed_from_genesis_at_open() {
    for target in targets() {
        let db = target.open();
        five_writes(&db).await;
        drop(db);
        // The log as DH1 left it: no hash columns, and updates refused outright.
        let dh1 = match target {
            Target::Sqlite(_) => {
                "ALTER TABLE __store_change_log DROP COLUMN hash;
                 ALTER TABLE __store_change_log DROP COLUMN prev_hash;
                 CREATE TRIGGER \"__store_change_log_no_update\" BEFORE UPDATE ON __store_change_log
                   BEGIN SELECT RAISE(ABORT, 'store: the change log is append-only'); END;
                 CREATE TRIGGER \"__store_change_log_no_delete\" BEFORE DELETE ON __store_change_log
                   BEGIN SELECT RAISE(ABORT, 'store: the change log is append-only'); END;"
            }
            Target::Postgres(_) => {
                "DROP TRIGGER \"__store_seal_only\" ON __store_change_log;
                 ALTER TABLE __store_change_log DROP COLUMN hash, DROP COLUMN prev_hash;
                 DROP TRIGGER \"__store_append_only\" ON __store_change_log;
                 CREATE TRIGGER \"__store_append_only\" BEFORE UPDATE OR DELETE OR TRUNCATE
                   ON __store_change_log FOR EACH STATEMENT EXECUTE FUNCTION __store_append_only()"
            }
        };
        tamper(&target, dh1);

        let db = target.open();
        let log = chained(&db).await;
        assert_eq!(log.len(), 6);
        assert_eq!(log[0].prev_hash.as_deref(), Some(history::GENESIS));
        let verified = db.verify(None).await.unwrap();
        assert_eq!((verified.entries, verified.broken), (6, None));
        five_writes(&db).await;
        assert_eq!(db.verify(None).await.unwrap().entries, 11);
        let refused = db
            .write(player(), |tx| {
                tx.conn()
                    .batch_execute("UPDATE __store_change_log SET actor = 'player:8'")
                    .map_err(Error::from)
            })
            .await;
        assert!(refused.is_err(), "still append-only");
    }
}
