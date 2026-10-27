//! RU5 bounties-store golden test on 1.12.0: every step of `tests/golden/bounties_store/1.12.0.json`, recorded from the
//! reference Node routers (`record.cjs`, `run.sh`), replayed against the Rust handlers and compared on status, content
//! type and body text. The clock is the recording's. Not a gameplay claim.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use isles_core::areas::bounties_store::harness::{
    DatabaseConfig, make_account, pin_golden_clock, replay_golden, start,
};

const GOLDEN: &str = include_str!("../../../crates/isles-core/tests/golden/bounties_store/1.12.0.json");
const GAME_DATA: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../crates/isles-core/tests/golden/bounties_store/game-data"
);

#[tokio::test(flavor = "multi_thread")]
async fn rust_answers_what_node_answered() {
    pin_golden_clock();
    let dir = tempfile::tempdir().unwrap();
    let target = DatabaseConfig::Sqlite {
        file: dir.path().join("isles.db"),
    };
    let server = start(
        isles_1_12_0::version(),
        &target,
        Some(std::path::Path::new(GAME_DATA)),
        &[],
    )
    .await;
    make_account(&server.store, "p1").await;
    make_account(&server.store, "p2").await;
    let diffs = replay_golden(&server, GOLDEN).await;
    assert!(diffs.is_empty(), "{} steps differ:\n{}", diffs.len(), diffs.join("\n"));
}
