//! The inventory repositories on `store`: SQLite always, Postgres when `REFORGED_TEST_POSTGRES_URL` names a throwaway
//! database on 127.0.0.1. Every test uses ids of its own, so the engines can share a database across tests.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use isles_core::AccountId;
use isles_core::areas::inventory::testkit::{open_store, postgres_config, sqlite_config};
use isles_core::db::IslesStore;
use isles_core::repo::inventory::{InventoryRecord, LedgerKey, LedgerStatus, TryBegin};
use module_api::{Actor, ActorContext, NodeIdentity};
use serde_json::json;
use store::{DatabaseConfig, KeyValue};

/// One test at a time: each opens the database, which prepares it (triggers, migrations) under locks that a write of
/// another open store can deadlock with. A server opens its database once.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Each engine to test, with the directory a SQLite file lives in.
fn engines() -> Vec<(&'static str, DatabaseConfig, Option<tempfile::TempDir>)> {
    let dir = tempfile::tempdir().unwrap();
    let mut engines = vec![("sqlite", sqlite_config(dir.path()), Some(dir))];
    if let Some(config) = postgres_config() {
        engines.push(("postgres", config, None));
    }
    engines
}

fn account() -> AccountId {
    AccountId::new(&NodeIdentity::standalone().node)
}

fn actor(account: &AccountId) -> ActorContext {
    ActorContext::new(Actor::Player {
        account: account.global().clone(),
    })
}

fn record(character: &str, account: &AccountId) -> InventoryRecord {
    InventoryRecord {
        character_id: character.to_owned(),
        account: Some(account.clone()),
        instanced_items: r#"[{"catalogId":"A","instanceId":"i"}]"#.into(),
        stacked_items: r#"[{"catalogId":"B","quantity":2}]"#.into(),
        revision: Some(0),
        bootstrap_version: Some("starter".into()),
    }
}

async fn create(store: &IslesStore, account: &AccountId, inventory: InventoryRecord) {
    let repos = store.repos().clone();
    store
        .account_write(account, &actor(account), move |tx, _| {
            repos.inventories.create(tx, &inventory)
        })
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn an_inventory_is_written_by_revision_and_logged() {
    let _serial = SERIAL.lock().await;
    for (name, config, _dir) in engines() {
        let store = open_store(&config);
        let account = account();
        let character = format!("char-{}", uuid::Uuid::new_v4());
        create(&store, &account, record(&character, &account)).await;

        let find = |store: &IslesStore, character: String| {
            let repos = store.repos().clone();
            let store = store.clone();
            async move {
                store
                    .read(move |conn, _| repos.inventories.find_by_character_id(conn, &character))
                    .await
                    .unwrap()
            }
        };
        assert_eq!(
            find(&store, character.clone()).await,
            Some(record(&character, &account)),
            "{name}"
        );

        let update = |revision: i64| {
            let (repos, character, store, account) =
                (store.repos().clone(), character.clone(), store.clone(), account.clone());
            async move {
                store
                    .account_write(&account, &actor(&account), move |tx, _| {
                        repos.inventories.update_both_if_revision_matches(
                            tx,
                            &character,
                            "[]",
                            r#"[{"catalogId":"B","quantity":9}]"#,
                            revision,
                        )
                    })
                    .await
                    .unwrap()
            }
        };
        // Only the revision that was read can write; the new row is one revision on
        assert_eq!(update(1).await, None, "{name}: a stale revision");
        let written = update(0).await.unwrap();
        assert_eq!(
            (written.revision, written.instanced_items.as_str()),
            (Some(1), "[]"),
            "{name}"
        );
        assert_eq!(update(0).await, None, "{name}: the same revision twice");
        assert_eq!(update(1).await.unwrap().revision, Some(2), "{name}");

        // The change log names the player and the character
        let log = store.database().changes(0, 10_000).await.unwrap();
        let mine: Vec<_> = log
            .iter()
            .filter(|e| e.table_name == "inventories" && e.row_key.contains(&character))
            .collect();
        assert_eq!(mine.len(), 3, "{name}");
        assert_eq!(mine[0].op, "insert");
        assert_eq!(mine[0].actor, format!("player:{account}"));
        assert!(mine[1..].iter().all(|e| e.op == "update"));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_deleted_inventory_is_hidden_and_comes_back() {
    let _serial = SERIAL.lock().await;
    for (name, config, _dir) in engines() {
        let store = open_store(&config);
        let account = account();
        let character = format!("char-{}", uuid::Uuid::new_v4());
        create(&store, &account, record(&character, &account)).await;
        let key = character.clone();
        store
            .account_write(&account, &actor(&account), move |tx, _| {
                Ok(tx.soft_delete("inventories", &[("character_id", KeyValue::Text(key))])?)
            })
            .await
            .unwrap();
        let (repos, c) = (store.repos().clone(), character.clone());
        let gone = store
            .read(move |conn, _| repos.inventories.find_by_character_id(conn, &c))
            .await
            .unwrap();
        assert_eq!(gone, None, "{name}");
        let key = character.clone();
        store
            .account_write(&account, &actor(&account), move |tx, _| {
                Ok(tx.restore("inventories", &[("character_id", KeyValue::Text(key))])?)
            })
            .await
            .unwrap();
        let (repos, c) = (store.repos().clone(), character.clone());
        let back = store
            .read(move |conn, _| repos.inventories.find_by_character_id(conn, &c))
            .await
            .unwrap();
        assert!(back.is_some(), "{name}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_ledger_claims_replays_and_lists() {
    let _serial = SERIAL.lock().await;
    for (name, config, _dir) in engines() {
        let store = open_store(&config);
        let (mine, theirs) = (account(), account());
        let character = format!("char-{}", uuid::Uuid::new_v4());
        let run = |account: &AccountId, id: &'static str, hash: &'static str, complete: bool| {
            let (repos, store, account, character) =
                (store.repos().clone(), store.clone(), account.clone(), character.clone());
            async move {
                let actor = actor(&account);
                let owner = account.clone();
                store
                    .account_write(&owner, &actor, move |tx, _| {
                        let key = LedgerKey {
                            account: &account,
                            character_id: &character,
                            transaction_id: id,
                        };
                        let begun = repos.inventory_transactions.try_begin(
                            tx,
                            key,
                            hash,
                            &format!("2026-10-27T00:00:0{}.000Z", id.len()),
                        )?;
                        if complete && begun == TryBegin::Begun {
                            repos.inventory_transactions.complete(
                                tx,
                                key,
                                &json!({"createdInstancedItems": []}),
                                Some(&json!({"economySource": "INVENTORY_GAMESERVER", "annotated": {"sku": 1}})),
                                "2026-10-27T00:01:00.000Z",
                            )?;
                        }
                        Ok(begun)
                    })
                    .await
                    .unwrap()
            }
        };
        assert_eq!(run(&mine, "t", "h1", false).await, TryBegin::Begun, "{name}");
        // Pending: the same key is reported as it is, with its hash
        let TryBegin::Existing(pending) = run(&mine, "t", "h2", false).await else {
            panic!("{name}: a second claim began");
        };
        assert_eq!(
            (pending.status, pending.request_hash.as_deref()),
            (LedgerStatus::Pending, Some("h1")),
            "{name}"
        );
        // Another account's id is its own
        assert_eq!(run(&theirs, "t", "h1", false).await, TryBegin::Begun, "{name}");
        assert_eq!(run(&mine, "tt", "h3", true).await, TryBegin::Begun, "{name}");
        let TryBegin::Existing(done) = run(&mine, "tt", "h3", true).await else {
            panic!("{name}: a replay began");
        };
        assert_eq!(done.status, LedgerStatus::Completed, "{name}");
        assert_eq!(done.result, Some(json!({"createdInstancedItems": []})), "{name}");
        assert_eq!(done.economy_source.as_deref(), Some("INVENTORY_GAMESERVER"), "{name}");
        assert_eq!(done.extra, Some(json!({"annotated": {"sku": 1}})), "{name}");

        let (repos, c, m) = (store.repos().clone(), character.clone(), mine.clone());
        let found = store
            .read(move |conn, _| {
                repos.inventory_transactions.find_record(
                    conn,
                    LedgerKey {
                        account: &m,
                        character_id: &c,
                        transaction_id: "tt",
                    },
                )
            })
            .await
            .unwrap();
        assert_eq!(
            found.unwrap().completed_at.as_deref(),
            Some("2026-10-27T00:01:00.000Z"),
            "{name}"
        );

        let (repos, m) = (store.repos().clone(), mine.clone());
        let recent = store
            .read(move |conn, _| repos.inventory_transactions.list_recent_for_account(conn, &m, 10))
            .await
            .unwrap();
        // newest first, no results, only the fields an admin timeline shows
        assert_eq!(
            recent,
            vec![
                json!({"transactionId": "tt", "characterId": character, "status": "completed",
                    "createdAt": "2026-10-27T00:00:02.000Z", "economySource": "INVENTORY_GAMESERVER",
                    "completedAt": "2026-10-27T00:01:00.000Z"}),
                json!({"transactionId": "t", "characterId": character, "status": "pending",
                    "createdAt": "2026-10-27T00:00:01.000Z"}),
            ],
            "{name}"
        );
    }
}
