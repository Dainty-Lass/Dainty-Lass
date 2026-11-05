//! The progression area against the recording of the Node services (RU5 progression): the same requests, the same
//! answers to the byte once dates and minted ids are normalised. `crates/isles-core/tests/golden/progression/record.sh`
//! makes the recording.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use std::path::PathBuf;

use isles_core::areas::progression::testkit::DatabaseConfig;
use isles_core::areas::progression::testkit::golden::replay;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/isles-core/tests/golden/progression")
}

async fn replay_on(database: DatabaseConfig) {
    let recording: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fixture().join("scenarios-1.14.7.json")).unwrap()).unwrap();
    let differences = replay(
        database,
        isles_1_14_7::version(),
        &fixture().join("game-data"),
        &recording,
    )
    .await;
    assert!(
        differences.is_empty(),
        "{} differences:
{}",
        differences.len(),
        differences.join(
            "

"
        )
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_rust_area_answers_as_the_node_services_did() {
    let dir = tempfile::tempdir().unwrap();
    replay_on(DatabaseConfig::Sqlite {
        file: dir.path().join("golden.db"),
    })
    .await;
}

/// The same on Postgres, when `REFORGED_TEST_POSTGRES_URL` names a server.
#[tokio::test(flavor = "multi_thread")]
async fn and_on_postgres() {
    if let Some(database) = isles_core::areas::progression::testkit::Throwaway::create() {
        replay_on(database.config()).await;
    }
}
