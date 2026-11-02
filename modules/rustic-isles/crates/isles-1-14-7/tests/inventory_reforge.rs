//! The inventory and reforge routes against what Rustic Isles' Node backend answered to the same requests
//! (`crates/isles-core/tests/golden/inventory/record.cjs`): every status, content type and body, byte for byte, with
//! timestamps, UUIDs and minted ids masked. SQLite always, Postgres when `REFORGED_TEST_POSTGRES_URL` is set. Then the
//! parts the recording can't reach: a rolled-back transaction, and a realm host's runtime through rustic-rifts' hooks.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use isles_core::Settings;
use isles_core::areas::inventory::testkit::{
    DatabaseConfig, Harness, SCENARIOS, postgres_config, recorded, replay, sqlite_config,
};
use isles_core::repo::inventory::LedgerKey;
use serde_json::{Value, json};

fn seed() -> Value {
    serde_json::from_str::<Value>(SCENARIOS).unwrap()["seed"].clone()
}

/// The recording ran with GAMESERVER_NO_PLAYER_USER_ID=u1 and the reforge on, as a default 1.14.7 host.
fn settings() -> Settings {
    Settings::from_pairs([("GAMESERVER_NO_PLAYER_USER_ID", "u1")])
}

async fn golden(name: &str, config: DatabaseConfig) {
    let harness = Harness::start_on(&config, isles_1_14_7::version(), settings(), &seed()).await;
    let have = replay(&harness).await;
    let want = recorded();
    assert_eq!(have.len(), want.len());
    assert!(want.len() > 70, "the recording is missing");
    let mut differences = Vec::new();
    for ((name, have), (want_name, want)) in have.iter().zip(&want) {
        assert_eq!(name, want_name);
        if have != want {
            differences.push(format!("{name}\n  node: {want:?}\n  rust: {have:?}"));
        }
    }
    assert!(
        differences.is_empty(),
        "{name}: {} differ:\n{}",
        differences.len(),
        differences.join("\n")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn every_route_answers_as_node_does_on_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    golden("sqlite", sqlite_config(dir.path())).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn every_route_answers_as_node_does_on_postgres() {
    // The seed's user ids are fixed, so this is the only test that uses the Postgres database for them
    if let Some(config) = postgres_config() {
        golden("postgres", config).await;
    }
}

fn gameserver(user: &str) -> Vec<(&'static str, String)> {
    vec![
        ("x-test-auth", json!({"IsGameserver": true, "userId": user}).to_string()),
        ("content-type", "application/json".into()),
    ]
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_transaction_leaves_no_ledger_row_and_the_id_can_be_used_again() {
    let dir = tempfile::tempdir().unwrap();
    let harness = Harness::start(dir.path(), isles_1_14_7::version(), settings(), &seed()).await;
    let refused = json!({"characterId": "c1", "transactionId": "tx-rb",
        "removeStackedItems": [{"catalogId": "CORE", "quantity": 5}], "addStackedItems": [{"catalogId": "MAT_A", "quantity": 2}]});
    let reply = harness
        .send("POST", "/inventory", &gameserver("u1"), Some(&refused.to_string()))
        .await;
    assert_eq!(reply.status, 409);
    let account = harness.accounts["u1"].clone();
    let ledger = |harness: &Harness| {
        let (repos, store, account) = (harness.repos.clone(), harness.store.clone(), account.clone());
        async move {
            store
                .read(move |conn, _| {
                    repos.inventory_transactions.find_record(
                        conn,
                        LedgerKey {
                            account: &account,
                            character_id: "c1",
                            transaction_id: "tx-rb",
                        },
                    )
                })
                .await
                .unwrap()
        }
    };
    assert_eq!(ledger(&harness).await, None);
    // Not the "reused with another body" refusal: the first attempt never happened
    let fixed = json!({"characterId": "c1", "transactionId": "tx-rb", "addStackedItems": [{"catalogId": "MAT_A", "quantity": 2}]});
    let reply = harness
        .send("POST", "/inventory", &gameserver("u1"), Some(&fixed.to_string()))
        .await;
    assert_eq!(reply.status, 200, "{reply:?}");
    assert!(
        reply.body.contains(r#"{"catalogId":"MAT_A","quantity":6}"#),
        "{}",
        reply.body
    );
    assert!(ledger(&harness).await.is_some());
    // and the stored inventory moved once
    let read = harness.send("GET", "/inventory/u1/c1", &gameserver("u1"), None).await;
    assert!(
        read.body.contains(r#"{"catalogId":"MAT_A","quantity":6}"#),
        "{}",
        read.body
    );
}

fn host(user: Option<&str>) -> Vec<(&'static str, String)> {
    let mut auth = json!({"host": true});
    if let Some(user) = user {
        auth["userId"] = json!(user);
    }
    vec![
        ("x-test-auth", auth.to_string()),
        ("content-type", "application/json".into()),
    ]
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hosts_runtime_is_refused_without_the_module() {
    let dir = tempfile::tempdir().unwrap();
    // The dedicated-only hooks of Node's DEDICATED_ONLY
    let harness = Harness::start(dir.path(), isles_1_14_7::version(), settings(), &seed()).await;
    let transaction = json!({"characterId": "c1", "transactionId": "h-1", "addStackedItems": [{"catalogId": "MAT_A", "quantity": 1}]});
    let reply = harness
        .send("POST", "/inventory", &host(Some("u1")), Some(&transaction.to_string()))
        .await;
    assert_eq!(
        (reply.status, reply.body.as_str()),
        (
            403,
            r#"{"code":"forbidden","message":"This runtime cannot mutate inventory.","payload":null}"#
        )
    );
    let item = json!({"characterId": "c1", "instanceId": "I-AXE", "catalogId": "WP_AX_BEGINNER", "updateVersion": 50});
    let reply = harness
        .send(
            "POST",
            "/inventory/instanceditem",
            &host(Some("u1")),
            Some(&item.to_string()),
        )
        .await;
    assert_eq!(
        (reply.status, reply.body.as_str()),
        (
            403,
            r#"{"code":"forbidden","message":"This runtime cannot write player state.","payload":null}"#
        )
    );
    // no player to act for
    let reply = harness
        .send("POST", "/inventory/instanceditem", &host(None), Some(&item.to_string()))
        .await;
    assert_eq!(
        (reply.status, reply.body.as_str()),
        (
            403,
            r#"{"code":"forbidden","message":"Player-host runtime cannot mutate inventory.","payload":null}"#
        )
    );
    let reply = harness
        .send(
            "DELETE",
            "/progression/u1/ExperienceTrack_Weapon_Axe",
            &host(Some("u1")),
            None,
        )
        .await;
    assert_eq!(
        (reply.status, reply.body.as_str()),
        (
            403,
            r#"{"code":"forbidden","message":"This runtime cannot write player state.","payload":null}"#
        )
    );
    let reply = harness
        .send(
            "POST",
            "/progression/purchase/u1/ExperienceTrack_Weapon_Axe/3500",
            &host(Some("u1")),
            None,
        )
        .await;
    assert_eq!(reply.status, 403);
}
