// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Save resets: a reset leaves a new account's save and keeps identity, the 24-hour limit, a staff restore round
//! trip and the 30-day prune.
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use common::{account, launcher, system, targets};
use diesel::QueryableByName;
use diesel::RunQueryDsl;
use diesel::sql_types::BigInt;
use rustic_isles_store::repo::characters::CharacterRecord;
use rustic_isles_store::repo::chat::{FriendEdgeRecord, FriendshipStatus};
use rustic_isles_store::repo::resets::{self, PLAYER_RESET_EVERY_MS, PlayerReset, SNAPSHOT_KEEP_MS};
use rustic_isles_store::{AccountId, IslesStore, UserId};

const T: i64 = 1_795_000_000_000;

#[derive(QueryableByName)]
struct N {
    #[diesel(sql_type = BigInt)]
    n: i64,
}

fn character(id: &str, account: &AccountId) -> CharacterRecord {
    CharacterRecord {
        character_id: id.into(),
        account: account.clone(),
        created_date: "d".into(),
        last_modified_date: "d".into(),
        name: "n".into(),
        update_version: 0,
        data: "{}".into(),
    }
}

/// A save: a character, an account-less inventory of it, and a track.
async fn seed(store: &IslesStore, a: &AccountId, character_id: &str) {
    let (w, c) = (a.clone(), character_id.to_owned());
    store
        .write(&system(), move |tx, r| {
            r.characters.create(tx, &character(&c, &w))?;
            r.progression_tracks.increment(tx, &w, "xp", 5)?;
            diesel::sql_query(format!(
                "INSERT INTO inventories (character_id, account_id, instanced_items, stacked_items) \
                 VALUES ('{c}', NULL, '[]', '[]')"
            ))
            .execute(tx.conn())
            .map_err(|e| rustic_isles_store::RepoError::Data(e.to_string()))?;
            Ok(())
        })
        .await
        .unwrap();
}

/// `(characters, xp, inventories of those characters)` the account has now.
async fn save_of(store: &IslesStore, a: &AccountId) -> (Vec<String>, Option<i64>, i64) {
    let w = a.clone();
    store
        .read(move |conn, r| {
            let characters: Vec<String> = r
                .characters
                .find_many_by_account(conn, &w)?
                .into_iter()
                .map(|c| c.character_id)
                .collect();
            let xp = r.progression_tracks.get(conn, &w, "xp")?.map(|t| t.progress);
            let mut inventories = 0;
            for c in &characters {
                inventories += diesel::sql_query(format!(
                    "SELECT COUNT(*) AS n FROM inventories WHERE character_id = '{c}'"
                ))
                .get_result::<N>(conn)
                .unwrap()
                .n;
            }
            Ok((characters, xp, inventories))
        })
        .await
        .unwrap()
}

async fn reset(store: &IslesStore, a: &AccountId, id: &str, now: i64) -> PlayerReset {
    let (w, id) = (a.clone(), id.to_owned());
    store
        .account_write(a, &system(), move |tx, _| resets::player_reset(tx, &w, &id, now))
        .await
        .unwrap()
}

#[tokio::test]
async fn reset_leaves_a_new_save_keeps_identity_and_waits_a_day() {
    for t in targets() {
        let store = &t.store;
        let a = account(store, "ua").await;
        let friend = account(store, "friend").await;
        let (w, f) = (a.clone(), friend.clone());
        store
            .write(&system(), move |tx, r| {
                r.launcher_accounts
                    .create(tx, &launcher(&w, "ua", None, "Ash", "2026-10-01T00:00:00.000Z"))?;
                r.friendships.upsert(
                    tx,
                    &FriendEdgeRecord {
                        owner: w.clone(),
                        other: f.clone(),
                        other_user_id: UserId::new("friend"),
                        status: FriendshipStatus::Accepted,
                        direction: None,
                        created: "c".into(),
                        favorite: false,
                    },
                )
            })
            .await
            .unwrap();
        seed(store, &a, "c1").await;
        seed(store, &friend, "f1").await;
        assert_eq!(save_of(store, &a).await, (vec!["c1".to_owned()], Some(5), 1));

        assert_eq!(reset(store, &a, "s1", T).await, PlayerReset::Done(2), "{}", t.engine);
        // A new account's save: nothing, for every area to bootstrap on first read
        assert_eq!(save_of(store, &a).await, (vec![], None, 0));
        // Identity and other players stay
        let w = a.clone();
        let (game, launch, friends) = store
            .read(move |conn, r| {
                Ok((
                    r.accounts.find(conn, &w)?,
                    r.launcher_accounts.find_by_account(conn, &w)?,
                    r.friendships.list_for_owner(conn, &w)?,
                ))
            })
            .await
            .unwrap();
        assert_eq!(game.unwrap().name, "ua");
        assert_eq!(launch.unwrap().display_name, "Ash");
        assert_eq!(friends.len(), 1);
        assert_eq!(save_of(store, &friend).await, (vec!["f1".to_owned()], Some(5), 1));

        // Once a day
        let next = T + PLAYER_RESET_EVERY_MS;
        assert_eq!(reset(store, &a, "s2", next - 1).await, PlayerReset::TooSoon(next));
        let w = a.clone();
        let asked = store
            .read(move |conn, _| resets::next_player_reset(conn, &w, next - 1))
            .await
            .unwrap();
        assert_eq!(asked, Some(next));
        assert_eq!(reset(store, &a, "s2", next).await, PlayerReset::Done(0));
        let w = a.clone();
        let listed = store.read(move |conn, _| resets::list(conn, &w)).await.unwrap();
        let ids: Vec<(&str, u64)> = listed.iter().map(|s| (s.id.as_str(), s.rows)).collect();
        assert_eq!(ids, [("s2", 0), ("s1", 2)]);
        assert_eq!(listed[1].expires_at, T + SNAPSHOT_KEEP_MS);
    }
}

#[tokio::test]
async fn restore_round_trips_and_can_itself_be_undone() {
    for t in targets() {
        let store = &t.store;
        let a = account(store, "ua").await;
        seed(store, &a, "c1").await;
        assert_eq!(reset(store, &a, "s1", T).await, PlayerReset::Done(2));
        // The fresh save the player made after
        let w = a.clone();
        store
            .write(&system(), move |tx, r| {
                r.characters.create(tx, &character("c2", &w)).map(|_| ())
            })
            .await
            .unwrap();

        let w = a.clone();
        let moved = store
            .account_write(&a, &system(), move |tx, _| {
                resets::restore(tx, &w, "s1", "r1", "staff", T + 1)
            })
            .await
            .unwrap();
        assert_eq!(moved, 2, "{}", t.engine);
        assert_eq!(save_of(store, &a).await, (vec!["c1".to_owned()], Some(5), 1));

        let w = a.clone();
        let listed = store.read(move |conn, _| resets::list(conn, &w)).await.unwrap();
        let rows: Vec<(&str, &str, u64, bool)> = listed
            .iter()
            .map(|s| (s.id.as_str(), s.source.as_str(), s.rows, s.restored_at.is_some()))
            .collect();
        assert_eq!(rows, [("r1", "restore", 1, false), ("s1", "player", 0, true)]);
        assert_eq!(listed[0].actor_user_id.as_deref(), Some("staff"));

        // Restored once only; the replaced save goes back the same way
        let w = a.clone();
        let again = store
            .account_write(&a, &system(), move |tx, _| {
                resets::restore(tx, &w, "s1", "r2", "staff", T + 2)
            })
            .await;
        assert!(again.is_err());
        let w = a.clone();
        store
            .account_write(&a, &system(), move |tx, _| {
                resets::restore(tx, &w, "r1", "r3", "staff", T + 3)
            })
            .await
            .unwrap();
        assert_eq!(save_of(store, &a).await.0, ["c2"]);
        // A staff restore doesn't use up the player's daily reset
        let w = a.clone();
        let last = store
            .read(move |conn, _| resets::last_player_reset(conn, &w))
            .await
            .unwrap();
        assert_eq!(last, Some(T));
    }
}

#[tokio::test]
async fn snapshots_are_pruned_with_their_rows_after_thirty_days() {
    for t in targets() {
        let store = &t.store;
        let a = account(store, "ua").await;
        seed(store, &a, "c1").await;
        assert_eq!(reset(store, &a, "s1", T).await, PlayerReset::Done(2));
        let held = |store: &IslesStore| {
            let store = store.clone();
            async move {
                store
                    .read(|conn, _| {
                        let mut n = |q: &str| diesel::sql_query(q).get_result::<N>(conn).unwrap().n;
                        Ok((
                            n("SELECT COUNT(*) AS n FROM save_snapshots"),
                            n("SELECT COUNT(*) AS n FROM characters WHERE character_id = 'c1'"),
                            n("SELECT COUNT(*) AS n FROM inventories WHERE character_id = 'c1'"),
                            n("SELECT COUNT(*) AS n FROM progression_tracks"),
                        ))
                    })
                    .await
                    .unwrap()
            }
        };
        let expiry = T + SNAPSHOT_KEEP_MS;
        let pruned = store
            .write(&system(), move |tx, _| resets::prune(tx, expiry - 1))
            .await
            .unwrap();
        assert_eq!((pruned, held(store).await), (0, (1, 1, 1, 1)), "{}", t.engine);
        let pruned = store
            .write(&system(), move |tx, _| resets::prune(tx, expiry))
            .await
            .unwrap();
        assert_eq!((pruned, held(store).await), (1, (0, 0, 0, 0)));
        // The account itself stays, with its new save
        let w = a.clone();
        let found = store.read(move |conn, r| r.accounts.find(conn, &w)).await.unwrap();
        assert!(found.is_some());
    }
}
