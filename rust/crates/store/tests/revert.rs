//! History and revert (DD23, DH3a) on both engines. Postgres when `REFORGED_TEST_POSTGRES_URL` is set, as in
//! `database.rs`.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use std::sync::atomic::{AtomicU32, Ordering};

use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Text};
use diesel_migrations::{EmbeddedMigrations, embed_migrations};
use store::revert::{Conflict, RevertKind};
use store::{
    AccountColumn, Actor, AnyConnection, Cursor, Database, DatabaseConfig, DatabaseOptions, DatabaseSchema, Error,
    Filter, KeyValue, PostgresConfig, portable, transfer,
};

const SQLITE: EmbeddedMigrations = embed_migrations!("tests/db/sqlite");
const POSTGRES: EmbeddedMigrations = embed_migrations!("tests/db/postgres");
const SCHEMA: DatabaseSchema = DatabaseSchema {
    sqlite: || SQLITE,
    postgres: || POSTGRES,
    soft_delete: &["accounts"],
};
const ACCOUNTS: &[AccountColumn] = &[
    AccountColumn {
        table: "accounts",
        column: "id",
    },
    AccountColumn {
        table: "items",
        column: "account_id",
    },
];

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

diesel::table! {
    notes (id) {
        id -> BigInt,
        body -> Text,
    }
}

// ------------------------------------------------------------------ the engines

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

static NEXT: AtomicU32 = AtomicU32::new(0);

fn new_postgres() -> Option<PgDatabase> {
    let url = std::env::var("REFORGED_TEST_POSTGRES_URL").ok()?;
    let url = url::Url::parse(&url).expect("REFORGED_TEST_POSTGRES_URL is a postgres:// URL");
    let server = PostgresConfig {
        host: url.host_str().unwrap_or("127.0.0.1").to_owned(),
        port: url.port().unwrap_or(5432),
        user: url.username().to_owned(),
        database: url.path().trim_start_matches('/').to_owned(),
        password: url.password().unwrap_or_default().to_owned(),
        allow_remote: false,
    };
    let name = format!(
        "store_revert_{}_{}_{}",
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
        Database::open(&self.config(), &DatabaseOptions::default(), &SCHEMA)
            .unwrap()
            .with_account_columns(ACCOUNTS)
    }
}

fn targets() -> Vec<Target> {
    let mut out = vec![Target::Sqlite(tempfile::tempdir().unwrap())];
    match new_postgres() {
        Some(pg) => out.push(Target::Postgres(pg)),
        None => eprintln!("REFORGED_TEST_POSTGRES_URL is not set: the Postgres half is skipped"),
    }
    out
}

// ------------------------------------------------------------------ a repository

type Account = (i64, String, bool, Option<f64>, Option<Vec<u8>>, Option<i64>);

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

async fn all_accounts(db: &Database) -> Vec<Account> {
    db.read(|c| Ok::<_, Error>(accounts::table.order(accounts::id).load::<Account>(c)?))
        .await
        .unwrap()
}

async fn all_items(db: &Database) -> Vec<(i64, i32, String)> {
    db.read(|c| {
        Ok::<_, Error>(
            items::table
                .order((items::account_id, items::slot))
                .load::<(i64, i32, String)>(c)?,
        )
    })
    .await
    .unwrap()
}

async fn all_notes(db: &Database) -> Vec<(i64, String)> {
    db.read(|c| Ok::<_, Error>(notes::table.order(notes::id).load::<(i64, String)>(c)?))
        .await
        .unwrap()
}

fn owner() -> Actor {
    Actor::Admin("owner".into())
}

fn partner() -> Actor {
    Actor::Admin("partner".into())
}

fn by(actor: Actor) -> Filter {
    Filter {
        actor: Some(actor),
        ..Filter::default()
    }
}

/// Two accounts with items and two notes, written by the owner. Returns the account ids.
async fn seed(db: &Database) -> (i64, i64) {
    db.write(owner(), |tx| {
        let a = create_account(tx.conn(), "alex")?;
        let b = create_account(tx.conn(), "blair")?;
        put_item(tx.conn(), a, 1, "Sword")?;
        put_item(tx.conn(), a, 2, "Axe")?;
        put_item(tx.conn(), b, 1, "Bow")?;
        tx.conn()
            .batch_execute("INSERT INTO notes (body) VALUES ('one'); INSERT INTO notes (body) VALUES ('two')")?;
        Ok::<_, Error>((a, b))
    })
    .await
    .unwrap()
}

// ------------------------------------------------------------------ the tests

// DH3's Verify: a partner's mass delete is reverted in one call; a row changed since stays and is reported.
#[tokio::test(flavor = "multi_thread")]
async fn a_partners_mass_delete_is_reverted_in_one_call() {
    for target in targets() {
        let db = target.open();
        let (a, b) = seed(&db).await;
        let (accounts_before, items_before, notes_before) =
            (all_accounts(&db).await, all_items(&db).await, all_notes(&db).await);

        // The partner wipes everything: a's items and a itself for good, b soft-deleted, b's item, the notes.
        db.write(partner(), move |tx| {
            tx.conn().batch_execute(&format!(
                "DELETE FROM items WHERE account_id = {a}; DELETE FROM accounts WHERE id = {a};"
            ))?;
            tx.soft_delete("accounts", &[("id", KeyValue::Int(b))])?;
            tx.conn().batch_execute("DELETE FROM items; DELETE FROM notes;")?;
            Ok::<_, Error>(())
        })
        .await
        .unwrap();
        assert!(all_items(&db).await.is_empty());
        // Before reverting, the owner brings b back with a new score and gives it a new item: changes since.
        db.write(owner(), move |tx| {
            tx.restore("accounts", &[("id", KeyValue::Int(b))])?;
            diesel::update(accounts::table.find(b))
                .set(accounts::score.eq(Some(2.0)))
                .execute(tx.conn())?;
            put_item(tx.conn(), b, 1, "Owner's bow")?;
            Ok::<_, Error>(())
        })
        .await
        .unwrap();

        let plan = db.revert_dry_run(by(partner())).await.unwrap();
        assert_eq!(plan.id.len(), 64);
        assert_eq!(plan.entries, 7, "{plan:#?}");
        let kinds: Vec<(&str, RevertKind)> = plan.actions.iter().map(|x| (x.table.as_str(), x.kind)).collect();
        assert_eq!(
            kinds,
            [
                ("accounts", RevertKind::Insert),
                ("notes", RevertKind::Insert),
                ("notes", RevertKind::Insert),
                ("items", RevertKind::Insert),
                ("items", RevertKind::Insert),
            ],
            "parents first: a before its items"
        );
        let mut conflicts: Vec<(&str, &Conflict)> =
            plan.conflicts.iter().map(|c| (c.table.as_str(), &c.conflict)).collect();
        conflicts.sort_by_key(|c| c.0);
        assert_eq!(conflicts.len(), 2, "{conflicts:?}");
        for (table, conflict) in &conflicts {
            assert!(
                matches!(conflict, Conflict::ChangedSince { actor, .. } if actor == "admin:owner"),
                "{table}: {conflict:?}"
            );
        }
        assert!(plan.unchanged.is_empty());

        // Nothing written by the dry run.
        assert!(all_notes(&db).await.is_empty());

        let outcome = db
            .revert_apply(Actor::Host("console".into()), plan.clone())
            .await
            .unwrap();
        assert_eq!(outcome.plan, plan, "the plan applied is the one shown");

        let mut expected_accounts = accounts_before.clone();
        expected_accounts[1].3 = Some(2.0); // b as the owner left it
        assert_eq!(
            all_accounts(&db).await,
            expected_accounts,
            "a back as it was, blob and double too"
        );
        assert_eq!(
            all_items(&db).await,
            [
                (a, 1, "Sword".to_owned()),
                (a, 2, "Axe".to_owned()),
                (b, 1, "Owner's bow".to_owned())
            ],
            "the owner's newer item is left as it is"
        );
        assert_eq!(all_notes(&db).await, notes_before);
        assert_ne!(items_before, all_items(&db).await);

        // The revert is logged under its actor and time, and nothing is left to do.
        let page = db.history(by(Actor::Host("console".into())), None, 100).await.unwrap();
        assert_eq!(page.entries.len(), 5);
        assert!(page.entries.iter().all(|e| e.at == outcome.at && e.op == "insert"));
        let again = db.revert_dry_run(by(partner())).await.unwrap();
        assert!(again.actions.is_empty(), "{again:#?}");
        assert_eq!(again.unchanged.len(), 5);
        assert_eq!(again.conflicts.len(), 2);

        // And the revert can itself be reverted.
        let undo = db.revert_dry_run(by(Actor::Host("console".into()))).await.unwrap();
        let undo_kinds: Vec<RevertKind> = undo.actions.iter().map(|x| x.kind).collect();
        assert_eq!(
            undo_kinds,
            [
                RevertKind::SoftDelete,
                RevertKind::Delete,
                RevertKind::Delete,
                RevertKind::Delete,
                RevertKind::Delete
            ],
            "children out before parents"
        );
        db.revert_apply(owner(), undo).await.unwrap();
        assert!(all_notes(&db).await.is_empty());
        assert_eq!(all_items(&db).await, [(b, 1, "Owner's bow".to_owned())]);
        let live: Vec<i64> = all_accounts(&db)
            .await
            .iter()
            .filter(|x| x.5.is_none())
            .map(|x| x.0)
            .collect();
        assert_eq!(
            live,
            [b],
            "a, back by the revert, soft-deleted by its undo (accounts' policy)"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn inserts_and_updates_are_undone_by_each_tables_policy() {
    for target in targets() {
        let db = target.open();
        let (a, _) = seed(&db).await;
        let new = db
            .write(partner(), move |tx| {
                let new = create_account(tx.conn(), "sockpuppet")?;
                put_item(tx.conn(), new, 1, "Loot")?;
                put_item(tx.conn(), a, 1, "Stick")?;
                diesel::update(accounts::table.find(a))
                    .set((accounts::name.eq("mallory"), accounts::is_admin.eq(false)))
                    .execute(tx.conn())?;
                diesel::update(accounts::table.find(a))
                    .set(accounts::score.eq(None::<f64>))
                    .execute(tx.conn())?;
                tx.conn().batch_execute("INSERT INTO notes (body) VALUES ('spam')")?;
                Ok::<_, Error>(new)
            })
            .await
            .unwrap();
        let plan = db.revert_dry_run(by(partner())).await.unwrap();
        let mut kinds: Vec<(&str, RevertKind)> = plan.actions.iter().map(|x| (x.table.as_str(), x.kind)).collect();
        kinds.sort_by_key(|k| k.0);
        assert_eq!(
            kinds,
            [
                ("accounts", RevertKind::Update),
                ("accounts", RevertKind::SoftDelete),
                ("items", RevertKind::Update),
                ("items", RevertKind::Delete),
                ("notes", RevertKind::Delete),
            ]
        );
        let changed_a = plan
            .actions
            .iter()
            .find(|x| x.kind == RevertKind::Update && x.table == "accounts");
        assert_eq!(changed_a.unwrap().entries.len(), 2, "two updates, one row");
        db.revert_apply(owner(), plan).await.unwrap();

        let rows = all_accounts(&db).await;
        assert_eq!(rows[0].1, "alex");
        assert!(rows[0].2);
        assert_eq!(rows[0].3, Some(1.0 / 3.0));
        let puppet = rows.iter().find(|r| r.0 == new).unwrap();
        assert!(
            puppet.5.is_some(),
            "a soft-delete table's insert is soft-deleted, not removed"
        );
        assert_eq!(
            all_items(&db).await.iter().find(|i| i.0 == a && i.1 == 1).unwrap().2,
            "Sword"
        );
        assert!(!all_items(&db).await.iter().any(|i| i.0 == new));
        assert_eq!(all_notes(&db).await.len(), 2);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_plan_that_no_longer_holds_is_refused_and_nothing_is_written() {
    for target in targets() {
        let db = target.open();
        let (a, b) = seed(&db).await;
        db.write(partner(), move |tx| {
            tx.soft_delete("accounts", &[("id", KeyValue::Int(a))])?;
            tx.soft_delete("accounts", &[("id", KeyValue::Int(b))])?;
            Ok::<_, Error>(())
        })
        .await
        .unwrap();
        let plan = db.revert_dry_run(by(partner())).await.unwrap();
        assert_eq!(plan.actions.len(), 2);

        // The owner restores b meanwhile: the dry run's plan would now do something else.
        db.write(owner(), move |tx| {
            tx.restore("accounts", &[("id", KeyValue::Int(b))])?;
            Ok::<_, Error>(())
        })
        .await
        .unwrap();
        let log = db
            .history(
                Filter {
                    since: Some(0),
                    ..Filter::default()
                },
                None,
                1000,
            )
            .await
            .unwrap();
        let refused = db.revert_apply(owner(), plan.clone()).await;
        assert!(matches!(refused, Err(Error::PlanChanged)), "{refused:?}");
        let after = db
            .history(
                Filter {
                    since: Some(0),
                    ..Filter::default()
                },
                None,
                1000,
            )
            .await
            .unwrap();
        assert_eq!(log, after, "nothing written");

        // A plan whose selection was edited after the dry run is refused the same way.
        let mut forged = db.revert_dry_run(by(partner())).await.unwrap();
        assert_eq!(forged.actions.len(), 1);
        forged.selection.since = Some(1);
        assert!(matches!(
            db.revert_apply(owner(), forged).await,
            Err(Error::PlanChanged)
        ));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn history_by_account_actor_and_time_pages_newest_first() {
    for target in targets() {
        let db = target.open();
        let (a, b) = seed(&db).await;
        db.write(partner(), move |tx| {
            put_item(tx.conn(), b, 2, "Shield")?;
            tx.soft_delete("accounts", &[("id", KeyValue::Int(a))])?;
            Ok::<_, Error>(())
        })
        .await
        .unwrap();

        let of_a = db
            .history(
                Filter {
                    account: Some(a.to_string()),
                    ..Filter::default()
                },
                None,
                100,
            )
            .await
            .unwrap();
        let shape: Vec<(&str, &str)> = of_a
            .entries
            .iter()
            .map(|e| (e.table_name.as_str(), e.op.as_str()))
            .collect();
        assert_eq!(
            shape,
            [
                ("accounts", "soft_delete"),
                ("items", "insert"),
                ("items", "insert"),
                ("accounts", "insert")
            ],
            "a's rows only, newest first"
        );
        assert_eq!(of_a.next, None);

        let partner_on_b = db
            .history(
                Filter {
                    account: Some(b.to_string()),
                    actor: Some(partner()),
                    ..Filter::default()
                },
                None,
                100,
            )
            .await
            .unwrap();
        assert_eq!(partner_on_b.entries.len(), 1);
        assert_eq!(partner_on_b.entries[0].op, "insert");

        // Pages of two through everything, by cursor text as a caller would carry it.
        let everything = Filter {
            since: Some(0),
            ..Filter::default()
        };
        let mut seen = Vec::new();
        let mut cursor: Option<Cursor> = None;
        loop {
            let page = db.history(everything.clone(), cursor, 2).await.unwrap();
            seen.extend(page.entries.iter().map(|e| e.id));
            match page.next {
                Some(next) => cursor = Some(next.to_string().parse().unwrap()),
                None => break,
            }
        }
        let all = db.changes(0, 1000).await.unwrap();
        let mut expected: Vec<i64> = all.iter().map(|e| e.id).collect();
        expected.reverse();
        assert_eq!(seen, expected);

        let at = all.last().unwrap().at;
        let window = db
            .history(
                Filter {
                    since: Some(at),
                    until: Some(at + 1),
                    ..Filter::default()
                },
                None,
                100,
            )
            .await
            .unwrap();
        assert!(window.entries.iter().all(|e| e.at == at));
        assert!(window.entries.len() >= 2);
        assert!("x1".parse::<Cursor>().is_err());
        assert!("-1".parse::<Cursor>().is_err());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_revert_needs_a_selection_and_accounts_need_their_columns() {
    let target = Target::Sqlite(tempfile::tempdir().unwrap());
    let db = target.open();
    let everything = db.revert_dry_run(Filter::default()).await;
    assert!(matches!(everything, Err(Error::Revert(_))), "{everything:?}");
    let until_only = db
        .revert_dry_run(Filter {
            until: Some(1),
            ..Filter::default()
        })
        .await;
    assert!(matches!(until_only, Err(Error::Revert(_))));

    let Target::Sqlite(dir) = &target else { return };
    let bare = Database::open(
        &DatabaseConfig::Sqlite {
            file: dir.path().join("test.db"),
        },
        &DatabaseOptions::default(),
        &SCHEMA,
    )
    .unwrap();
    let by_account = Filter {
        account: Some("1".into()),
        ..Filter::default()
    };
    assert!(matches!(
        bare.history(by_account.clone(), None, 10).await,
        Err(Error::Schema(_))
    ));
    let wrong = bare.with_account_columns(&[AccountColumn {
        table: "items",
        column: "owner",
    }]);
    assert!(matches!(
        wrong.history(by_account, None, 10).await,
        Err(Error::Schema(_))
    ));
}

// A log written on SQLite names rows as `{"id":1}` and booleans as 0 and 1; after `db move`, Postgres's
// entries say `{"id": 1}` and `true`. One row's history is still one row, and the old rows go back in.
#[tokio::test(flavor = "multi_thread")]
async fn a_delete_logged_on_sqlite_is_reverted_after_moving_to_postgres() {
    let Some(pg) = new_postgres() else {
        eprintln!("REFORGED_TEST_POSTGRES_URL is not set: the move is skipped");
        return;
    };
    let original = Target::Sqlite(tempfile::tempdir().unwrap());
    let db = original.open();
    let (a, _) = seed(&db).await;
    let before = all_accounts(&db).await;
    db.write(partner(), move |tx| {
        tx.conn().batch_execute(&format!(
            "DELETE FROM items WHERE account_id = {a}; DELETE FROM accounts WHERE id = {a};"
        ))?;
        Ok::<_, Error>(())
    })
    .await
    .unwrap();
    drop(db);
    let moved = Target::Postgres(pg);
    transfer::move_database(
        &original.config(),
        &moved.config(),
        &SCHEMA,
        &DatabaseOptions::default(),
    )
    .unwrap();
    let db = moved.open();
    let plan = db.revert_dry_run(by(partner())).await.unwrap();
    assert_eq!(plan.actions.len(), 3, "{plan:#?}");
    db.revert_apply(owner(), plan).await.unwrap();
    assert_eq!(
        all_accounts(&db).await,
        before,
        "the boolean, the double and the blob as they were"
    );
    assert_eq!(all_items(&db).await.iter().filter(|i| i.0 == a).count(), 2);
    let again = db.revert_dry_run(by(partner())).await.unwrap();
    assert!(again.actions.is_empty() && again.conflicts.is_empty(), "{again:#?}");
    assert_eq!(
        again.unchanged.len(),
        3,
        "rows named in both engines' key text are one row each"
    );
}
