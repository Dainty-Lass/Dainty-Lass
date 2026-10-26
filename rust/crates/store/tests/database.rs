//! `Database` on both engines (DD15) and its change log (DD23, DH1). SQLite always; Postgres when
//! `REFORGED_TEST_POSTGRES_URL` names a server the tests may create databases on (`postgres://user:pw@
//! 127.0.0.1:5432/postgres`), and skipped otherwise, like the nftables test.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Text};
use diesel_migrations::{EmbeddedMigrations, embed_migrations};
use serde_json::Value;
use store::{
    Actor, AnyConnection, ChangeEntry, Database, DatabaseConfig, DatabaseOptions, DatabaseSchema, Engine, Error,
    KeyValue, PostgresConfig, portable, transfer,
};

const SQLITE: EmbeddedMigrations = embed_migrations!("tests/db/sqlite");
const POSTGRES: EmbeddedMigrations = embed_migrations!("tests/db/postgres");
const SCHEMA: DatabaseSchema = DatabaseSchema {
    sqlite: || SQLITE,
    postgres: || POSTGRES,
    soft_delete: &["accounts"],
};

diesel::table! {
    accounts (id) {
        id -> BigInt,
        name -> Text,
        is_admin -> Bool,
        score -> Nullable<Double>,
        avatar -> Nullable<Binary>,
        deleted_at -> Nullable<BigInt>,
    }
}

diesel::table! {
    items (account_id, slot) {
        account_id -> BigInt,
        slot -> Integer,
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
