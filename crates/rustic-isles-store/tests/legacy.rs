// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Old saves imported on demand: one old account into one account, and claim codes that bind one.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use common::{account, system, targets};
use rustic_isles_store::repo::legacy::{self, CLAIM_KEEP_MS, ImportRecord, Redeem};
use rustic_isles_store::{AccountId, IslesStore, RepoResult};

const T: i64 = 1_795_000_000_000;
const RR: &str = "ramsgate-reborn";

async fn import(store: &IslesStore, account: &AccountId, old: &str) -> RepoResult<()> {
    let (a, old) = (account.clone(), old.to_owned());
    store
        .write(&system(), move |tx, _| {
            legacy::record_import(
                tx,
                &ImportRecord {
                    source: RR,
                    source_user: &old,
                    account: &a,
                    matched_by: "discord",
                    imported_at: T,
                    report: "{}",
                },
            )
        })
        .await
}

async fn redeem(store: &IslesStore, account: &AccountId, hash: &str, now: i64) -> Redeem {
    let (a, h) = (account.clone(), hash.to_owned());
    store
        .write(&system(), move |tx, _| legacy::redeem_claim(tx, &h, &a, now))
        .await
        .unwrap()
}

async fn claim(store: &IslesStore, hash: &str, old: &str) {
    let (h, old) = (hash.to_owned(), old.to_owned());
    store
        .write(&system(), move |tx, _| {
            legacy::create_claim(tx, &h, RR, &old, "admin", T)
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn one_old_account_fills_one_account() {
    for t in targets() {
        let store = &t.store;
        let (a, b) = (account(store, "ua").await, account(store, "ub").await);
        import(store, &a, "rr-1").await.unwrap();
        assert!(import(store, &b, "rr-1").await.is_err(), "the save is a's");
        assert!(import(store, &a, "rr-2").await.is_err(), "a already has one");
        import(store, &b, "rr-2").await.unwrap();
        let w = a.clone();
        let (into, by) = store
            .read(move |conn, _| {
                Ok((
                    legacy::imported_into(conn, RR, &w)?,
                    legacy::taken_by(conn, RR, "rr-2")?,
                ))
            })
            .await
            .unwrap();
        assert_eq!(into.as_deref(), Some("rr-1"));
        assert_eq!(by, Some(b));
    }
}

#[tokio::test]
async fn a_claim_code_binds_once_and_runs_out() {
    for t in targets() {
        let store = &t.store;
        let (a, b, c) = (
            account(store, "ua").await,
            account(store, "ub").await,
            account(store, "uc").await,
        );
        claim(store, "h1", "rr-1").await;
        claim(store, "h2", "rr-2").await;
        claim(store, "h3", "rr-3").await;
        claim(store, "h4", "rr-1").await;

        assert_eq!(redeem(store, &a, "nope", T).await, Redeem::Unknown);
        assert_eq!(redeem(store, &a, "h1", T + 1).await, Redeem::Done("rr-1".into()));
        assert_eq!(redeem(store, &b, "h1", T + 1).await, Redeem::Used);
        assert_eq!(redeem(store, &a, "h2", T + 1).await, Redeem::AccountHasOne);
        assert_eq!(redeem(store, &b, "h3", T + CLAIM_KEEP_MS).await, Redeem::Unknown);
        import(store, &a, "rr-1").await.unwrap();
        assert_eq!(redeem(store, &c, "h4", T + 1).await, Redeem::SaveTaken);
        let w = a.clone();
        let named = store
            .read(move |conn, _| legacy::redeemed_by(conn, RR, &w))
            .await
            .unwrap();
        assert_eq!(named.as_deref(), Some("rr-1"));

        let pruned = store
            .write(&system(), |tx, _| legacy::prune_claims(tx, T + CLAIM_KEEP_MS))
            .await
            .unwrap();
        assert_eq!(pruned, 3, "h2, h3 and h4 ran out unredeemed; h1 stays");
    }
}
