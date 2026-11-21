// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The Trials results table on each engine: a run saved and read back, a board's order and what it leaves out.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use common::{account, system, targets};
use rustic_isles_store::AccountId;
use rustic_isles_store::repo::trials::{self, TrialMode, TrialRun};

const TRIAL: &str = "CR19_PlayerHunt_Arena_Elite";

fn run(account: AccountId, mode: TrialMode, weapon: i64, time: f64) -> TrialRun {
    TrialRun {
        account,
        trial_id: TRIAL.into(),
        difficulty: 1,
        mode,
        weapon,
        completion_time: time,
        objectives_completed: 3,
        player_role_id: "role".into(),
        platform: "WIN".into(),
        party_key: "k".into(),
        party: "[]".into(),
        runs: 1,
    }
}

#[tokio::test]
async fn runs_round_trip_and_rank() {
    for t in targets() {
        let store = &t.store;
        let a = account(store, "u-a").await;
        let b = account(store, "u-b").await;
        let (ra, rb) = (a.clone(), b.clone());
        store
            .write(&system(), move |tx, _| {
                let at = "2026-11-19T20:00:00.000Z";
                trials::save(tx, &run(ra.clone(), TrialMode::Solo, 1, 400.0), at)?;
                let mut faster = run(ra.clone(), TrialMode::Solo, 1, 300.25);
                faster.runs = 2;
                trials::save(tx, &faster, at)?;
                trials::save(tx, &run(ra.clone(), TrialMode::Solo, 3, 350.0), at)?;
                trials::save(tx, &run(rb.clone(), TrialMode::Solo, 1, 320.0), at)?;
                trials::save(tx, &run(rb, TrialMode::Group, 0, 200.0), at)?;
                let mut other = run(ra, TrialMode::Solo, 1, 1.0);
                other.difficulty = 2;
                trials::save(tx, &other, at)
            })
            .await
            .unwrap();
        let ra = a.clone();
        let (found, solo, group) = store
            .read(move |conn, _| {
                Ok((
                    trials::find(conn, &ra, TRIAL, 1, TrialMode::Solo, 1)?,
                    trials::runs(conn, TRIAL, 1, TrialMode::Solo)?,
                    trials::runs(conn, TRIAL, 1, TrialMode::Group)?,
                ))
            })
            .await
            .unwrap();
        let found = found.unwrap();
        assert_eq!((found.completion_time, found.runs), (300.25, 2), "{}", t.engine);
        let lines: Vec<(&str, i64, f64)> = solo
            .iter()
            .map(|e| (e.user_id.as_str(), e.run.weapon, e.run.completion_time))
            .collect();
        assert_eq!(
            lines,
            [("u-a", 1, 300.25), ("u-b", 1, 320.0), ("u-a", 3, 350.0)],
            "{}",
            t.engine
        );
        assert_eq!(group.len(), 1, "{}", t.engine);
        assert_eq!(
            (group[0].name.as_str(), group[0].run.mode),
            ("u-b", TrialMode::Group),
            "{}",
            t.engine
        );
    }
}
