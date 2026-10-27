//! The characters-loadouts routes of 1.12.0 against the Node reference: the scenario in isles-core's
//! `tests/golden/characters` replayed on SQLite (and on Postgres when `REFORGED_TEST_POSTGRES_URL` names a server),
//! compared with what the Node 1.12.0 backend answered (`node-1.12.0.golden.json`, recorded by `record.sh`).
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use isles_core::areas::characters::golden::{Fakes, TestDb, differences_unified, open_store, replay};
use serde_json::Value;

const SCENARIO: &str = include_str!("../../../crates/isles-core/tests/golden/characters/scenario.json");
const GOLDEN: &str = include_str!("../../../crates/isles-core/tests/golden/characters/node-1.12.0.golden.json");
const GOLDEN_1_14_7: &str = include_str!("../../../crates/isles-core/tests/golden/characters/node-1.14.7.golden.json");

/// Steps 1.12.0's Node answered otherwise than RU3c has the port answer on both versions, compared with the 1.14.7
/// answers instead: the unlock is a delta read from the Slayer's Path (its messages and slot counts differ), and
/// `/loadout/.../active/:index` is served (1.12.0's Node answered 404; its client calls it).
const UNIFIED: &[&str] = &[
    "unlock without a journey",
    "unlock with a delta of zero",
    "unlock with a delta of seven",
    "unlock with a delta that is not a number",
    "unlock for the no-player sentinel",
    "select slot 2",
    "select slot 2 again",
    "select a locked slot",
    "select slot 6",
    "select slot -1",
    "select slot 1.5",
    "select slot of another character",
    "gameserver selects slot 0 for the player",
    "gameserver selects for the sentinel",
];

/// Steps where 1.12.0's Node threw a TypeError (500) on a missing character record and the port answers the 409
/// the 1.12 client recognises (RU3c: "missing record now 409 not 500 (same)").
const PORT_ANSWERS: &[(&str, u16)] = &[
    ("update of a character that is not theirs", 409),
    ("gameserver without a character id", 409),
];

async fn check(db: TestDb) {
    let scenario: Value = serde_json::from_str(SCENARIO).unwrap();
    let golden: Value = serde_json::from_str(GOLDEN).unwrap();
    let store = open_store(&db, &Fakes::default());
    let replies = replay(isles_1_12_0::version(), store, &scenario).await;
    let golden_1_14_7: Value = serde_json::from_str(GOLDEN_1_14_7).unwrap();
    let found = differences_unified(&golden, &golden_1_14_7, UNIFIED, PORT_ANSWERS, &replies);
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
