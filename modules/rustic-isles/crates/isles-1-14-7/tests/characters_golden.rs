//! The characters-loadouts routes of 1.14.7 against the Node reference: the scenario in isles-core's
//! `tests/golden/characters` replayed on SQLite (and on Postgres when `REFORGED_TEST_POSTGRES_URL` names a server),
//! compared with what the Node 1.14.7 backend answered (`node-1.14.7.golden.json`, recorded by `record.sh`).
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use isles_core::areas::characters::golden::{Fakes, TestDb, differences, open_store, replay};
use serde_json::Value;

const SCENARIO: &str = include_str!("../../../crates/isles-core/tests/golden/characters/scenario.json");
const GOLDEN: &str = include_str!("../../../crates/isles-core/tests/golden/characters/node-1.14.7.golden.json");

async fn check(db: TestDb) {
    let scenario: Value = serde_json::from_str(SCENARIO).unwrap();
    let golden: Value = serde_json::from_str(GOLDEN).unwrap();
    let store = open_store(&db, &Fakes::default());
    let replies = replay(isles_1_14_7::version(), store, &scenario).await;
    let found = differences(&golden, &replies);
    assert!(
        found.is_empty(),
        "{} answers differ from Node:
{}",
        found.len(),
        found.join(
            "
"
        )
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn sqlite_answers_as_node_does() {
    check(TestDb::sqlite()).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn postgres_answers_as_node_does() {
    match TestDb::postgres() {
        Some(db) => check(db).await,
        None => eprintln!("REFORGED_TEST_POSTGRES_URL is not set: the Postgres half is skipped"),
    }
}
