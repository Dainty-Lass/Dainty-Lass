// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The Gauntlet progress table on each engine: a run saved and read back, the season's leaderboard order.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use common::{account, system, targets};
use rustic_isles_store::repo::gauntlet::{self, GauntletProgress};

#[tokio::test]
async fn runs_round_trip_and_rank() {
    for t in targets() {
        let store = &t.store;
        let a = account(store, "u-a").await;
        let b = account(store, "u-b").await;
        let (ra, rb) = (a.clone(), b.clone());
        store
            .write(&system(), move |tx, _| {
                let mut first = GauntletProgress::new(ra, "ESC_SEASON_1");
                first.record(1, 526);
                gauntlet::save(tx, &first, "2026-11-19T19:25:19.619Z")?;
                first.record(2, 100);
                gauntlet::save(tx, &first, "2026-11-19T19:35:00.000Z")?;
                let mut second = GauntletProgress::new(rb.clone(), "ESC_SEASON_1");
                second.record(2, 300);
                gauntlet::save(tx, &second, "2026-11-19T19:40:00.000Z")?;
                let mut other = GauntletProgress::new(rb, "ESC_SEASON_2");
                other.record(9, 1);
                gauntlet::save(tx, &other, "2026-11-19T19:41:00.000Z")
            })
            .await
            .unwrap();
        let ra = a.clone();
        let (found, board, none) = store
            .read(move |conn, _| {
                Ok((
                    gauntlet::find(conn, &ra, "ESC_SEASON_1")?,
                    gauntlet::leaderboard(conn, "ESC_SEASON_1", 100)?,
                    gauntlet::find(conn, &ra, "ESC_SEASON_2")?,
                ))
            })
            .await
            .unwrap();
        let found = found.unwrap();
        assert_eq!(
            (found.best_level, found.best_remaining_sec, found.runs),
            (2, 100, 2),
            "{}",
            t.engine
        );
        assert!(none.is_none(), "{}", t.engine);
        let ranks: Vec<(i64, i64)> = board.iter().map(|e| (e.level, e.remaining_sec)).collect();
        assert_eq!(ranks, [(2, 300), (2, 100)], "{}", t.engine);
    }
}
