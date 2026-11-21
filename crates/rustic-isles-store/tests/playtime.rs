// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The playtime table on each engine: a session saved and read back, the open list and the reset at start.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use common::{account, system, targets};
use rustic_isles_store::repo::playtime::{self, PlaytimeRecord};

#[tokio::test]
async fn sessions_round_trip_and_reset() {
    for t in targets() {
        let store = &t.store;
        let a = account(store, "u-a").await;
        let b = account(store, "u-b").await;
        let (ra, rb) = (a.clone(), b.clone());
        store
            .write(&system(), move |tx, _| {
                let mut first = PlaytimeRecord::new(ra);
                first.open(0);
                first.close(90_000);
                playtime::save(tx, &first)?;
                first.open(100_000);
                playtime::save(tx, &first)?;
                let mut second = PlaytimeRecord::new(rb);
                second.open(5_000);
                playtime::save(tx, &second)
            })
            .await
            .unwrap();
        let (ra, rb) = (a.clone(), b.clone());
        let (found, open) = store
            .read(move |conn, _| Ok((playtime::find(conn, &ra)?, playtime::find_open(conn)?)))
            .await
            .unwrap();
        let found = found.unwrap();
        assert_eq!(
            (found.online_ms, found.last_seen_ms),
            (90_000, Some(100_000)),
            "{}",
            t.engine
        );
        assert_eq!(open.len(), 2, "{}", t.engine);
        let closed = store.write(&system(), |tx, _| playtime::close_all(tx)).await.unwrap();
        assert_eq!(closed, 2, "{}", t.engine);
        let (after, none) = store
            .read(move |conn, _| Ok((playtime::find_open(conn)?, playtime::find(conn, &rb)?)))
            .await
            .unwrap();
        assert!(after.is_empty(), "{}", t.engine);
        assert_eq!(none.unwrap().online_ms, 0, "{}", t.engine);
    }
}
