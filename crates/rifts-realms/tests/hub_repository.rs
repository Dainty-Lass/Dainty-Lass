// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The realm hub presence repository's contract, on SQLite and (when configured) Postgres.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use common::{actor, targets};
use rifts_realms::hub::PresenceState;
use rifts_realms::repo::hub::{
    EntryAt, EntryOutcome, HubEntryBatch, HubEntryRequest, HubPresenceRecord, RealmHubRepository,
};
use rifts_seam::UnixMillis;

const NOW: i64 = 1_791_892_800_000;
const HUB: &str = "hub-1";
const REALM: &str = "realm-1";
const PURGE: i64 = NOW + 604_800_000;

fn at(now: i64, generation: i64) -> EntryAt {
    EntryAt {
        session_id: HUB.to_owned(),
        generation,
        realm_id: REALM.to_owned(),
        now: UnixMillis(now),
        expires_at: UnixMillis(now + 90_000),
    }
}

fn request(account: &str) -> HubEntryRequest {
    HubEntryRequest {
        account_id: account.to_owned(),
        peer_id: format!("{account}-peer"),
        candidate_id: format!("{account}-candidate"),
    }
}

async fn enter(repo: &RealmHubRepository, account: &str, now: i64) -> (HubPresenceRecord, EntryOutcome) {
    repo.request_entry(&actor(), &at(now, 1), &request(account))
        .await
        .unwrap()
}

async fn admit(repo: &RealmHubRepository, account: &str, now: i64) -> HubPresenceRecord {
    enter(repo, account, now).await;
    repo.mark_admitted(
        &actor(),
        HUB,
        1,
        account,
        &format!("adm-{account}"),
        UnixMillis(now),
        UnixMillis(now),
        Some("relay"),
    )
    .await
    .unwrap()
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn first_entry_is_pending_epoch_one_and_round_trips() {
    for target in targets() {
        let repo = RealmHubRepository::new(target.db.clone());
        let (row, outcome) = enter(&repo, "m", NOW).await;
        assert_eq!(outcome, EntryOutcome::Created);
        assert_eq!(
            row,
            HubPresenceRecord {
                session_id: HUB.into(),
                generation: 1,
                realm_id: REALM.into(),
                account_id: "m".into(),
                peer_id: "m-peer".into(),
                state: PresenceState::Pending,
                candidate_id: "m-candidate".into(),
                entry_epoch: 1,
                local_bridge_port: None,
                local_bridge_ready_at: None,
                admission_id: None,
                route: None,
                requested_at: UnixMillis(NOW),
                ticketed_at: None,
                admitted_at: None,
                lost_at: None,
                left_at: None,
                left_reason: None,
                expires_at: UnixMillis(NOW + 90_000),
                purge_at: None,
            }
        );
        assert_eq!(repo.find_presence(HUB, "m").await.unwrap(), Some(row));
        assert_eq!(repo.find_presence(HUB, "other").await.unwrap(), None);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_repeated_waiting_request_reuses_and_only_extends_expiry() {
    for target in targets() {
        let repo = RealmHubRepository::new(target.db.clone());
        enter(&repo, "m", NOW).await;
        let (later, outcome) = enter(&repo, "m", NOW + 10).await;
        assert_eq!(outcome, EntryOutcome::Reused);
        assert_eq!(
            (later.requested_at, later.expires_at, later.entry_epoch),
            (UnixMillis(NOW), UnixMillis(NOW + 90_010), 1)
        );
        let earlier = EntryAt {
            expires_at: UnixMillis(NOW + 5),
            ..at(NOW + 1, 1)
        };
        let (kept, outcome) = repo.request_entry(&actor(), &earlier, &request("m")).await.unwrap();
        assert_eq!(outcome, EntryOutcome::Reused);
        assert_eq!(kept.expires_at, UnixMillis(NOW + 90_010));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn after_expiry_or_any_non_waiting_state_a_new_entry_starts() {
    for target in targets() {
        let repo = RealmHubRepository::new(target.db.clone());
        enter(&repo, "m", NOW).await;
        let (row, outcome) = enter(&repo, "m", NOW + 90_000).await;
        assert_eq!(
            (outcome, row.entry_epoch, row.requested_at),
            (EntryOutcome::Reentered, 2, UnixMillis(NOW + 90_000))
        );

        let admitted = admit(&repo, "n", NOW).await;
        assert_eq!(admitted.route.as_deref(), Some("relay"));
        let (row, outcome) = repo
            .request_entry(&actor(), &at(NOW + 5, 2), &request("n"))
            .await
            .unwrap();
        assert_eq!(outcome, EntryOutcome::Reentered);
        assert_eq!(
            (row.state, row.generation, row.entry_epoch),
            (PresenceState::Pending, 2, 2)
        );
        assert_eq!((row.admission_id, row.route, row.admitted_at), (None, None, None));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn racing_requests_for_one_entry_make_one_row() {
    for target in targets() {
        let repo = RealmHubRepository::new(target.db.clone());
        let tasks: Vec<_> = (0..8)
            .map(|i| {
                let repo = repo.clone();
                tokio::spawn(async move {
                    repo.request_entry(&actor(), &at(NOW + i, 1), &request("m"))
                        .await
                        .unwrap()
                })
            })
            .collect();
        let mut created = 0;
        for task in tasks {
            created += usize::from(task.await.unwrap().1 == EntryOutcome::Created);
        }
        assert_eq!(created, 1);
        assert_eq!(repo.count_by_state(HUB, 1, &PresenceState::ALL).await.unwrap(), 1);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_host_row_is_admitted_and_replaced_by_a_later_call() {
    for target in targets() {
        let repo = RealmHubRepository::new(target.db.clone());
        let row = repo
            .admit_host(&actor(), HUB, 1, REALM, "host", "host-peer", 7777, UnixMillis(NOW))
            .await
            .unwrap();
        assert_eq!(row.state, PresenceState::Admitted);
        assert_eq!(row.candidate_id, format!("host:{HUB}"));
        assert_eq!(
            (row.entry_epoch, row.local_bridge_port, row.admitted_at),
            (1, Some(7777), Some(UnixMillis(NOW)))
        );
        let again = repo
            .admit_host(&actor(), HUB, 2, REALM, "host", "peer-2", 8888, UnixMillis(NOW + 5))
            .await
            .unwrap();
        assert_eq!(
            (again.generation, again.entry_epoch, again.peer_id.as_str()),
            (2, 2, "peer-2")
        );
        assert_eq!(
            (again.requested_at, again.expires_at),
            (UnixMillis(NOW), UnixMillis(NOW))
        );
        assert_eq!(repo.find_presence(HUB, "host").await.unwrap(), Some(again));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_ticket_only_for_a_waiting_unexpired_entry_keeps_the_row_alive() {
    for target in targets() {
        let repo = RealmHubRepository::new(target.db.clone());
        enter(&repo, "m", NOW).await;
        let ticketed = repo
            .mark_ticketed(&actor(), HUB, 1, "m", UnixMillis(NOW + 5), UnixMillis(NOW + 300_000))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (ticketed.state, ticketed.ticketed_at),
            (PresenceState::Ticketed, Some(UnixMillis(NOW + 5)))
        );
        assert_eq!(ticketed.expires_at, UnixMillis(NOW + 300_000));
        let shorter = repo
            .mark_ticketed(&actor(), HUB, 1, "m", UnixMillis(NOW + 6), UnixMillis(NOW + 1))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(shorter.expires_at, UnixMillis(NOW + 300_000));
        assert_eq!(
            repo.mark_ticketed(&actor(), HUB, 2, "m", UnixMillis(NOW), UnixMillis(NOW + 9))
                .await
                .unwrap(),
            None,
            "generation"
        );
        assert_eq!(
            repo.mark_ticketed(&actor(), HUB, 1, "m", UnixMillis(NOW + 300_000), UnixMillis(NOW + 9))
                .await
                .unwrap(),
            None
        );
        admit(&repo, "n", NOW).await;
        assert_eq!(
            repo.mark_ticketed(&actor(), HUB, 1, "n", UnixMillis(NOW), UnixMillis(NOW + 9))
                .await
                .unwrap(),
            None,
            "admitted"
        );
        assert_eq!(
            repo.mark_ticketed(&actor(), HUB, 1, "nobody", UnixMillis(NOW), UnixMillis(NOW + 9))
                .await
                .unwrap(),
            None
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn admission_from_pending_ticketed_or_admitted_never_lost_or_expired() {
    for target in targets() {
        let repo = RealmHubRepository::new(target.db.clone());
        let admit_at = |account: &'static str, now: i64| {
            let repo = repo.clone();
            async move {
                repo.mark_admitted(&actor(), HUB, 1, account, "adm", UnixMillis(now), UnixMillis(now), None)
                    .await
                    .unwrap()
            }
        };
        enter(&repo, "pending", NOW).await;
        assert_eq!(
            admit_at("pending", NOW).await.map(|r| r.state),
            Some(PresenceState::Admitted)
        );
        enter(&repo, "ticketed", NOW).await;
        repo.mark_ticketed(&actor(), HUB, 1, "ticketed", UnixMillis(NOW), UnixMillis(NOW + 9))
            .await
            .unwrap();
        assert!(admit_at("ticketed", NOW).await.is_some());
        assert!(admit_at("ticketed", NOW + 1).await.is_some(), "admitted again");

        admit(&repo, "lost", NOW).await;
        repo.mark_lost(&actor(), HUB, 1, "lost", None, UnixMillis(NOW), UnixMillis(PURGE))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(admit_at("lost", NOW).await, None);

        enter(&repo, "late", NOW).await;
        assert_eq!(admit_at("late", NOW + 90_000).await, None, "expired at now");
        repo.expire_stale(&actor(), Some(HUB), UnixMillis(NOW + 90_000), UnixMillis(PURGE))
            .await
            .unwrap();
        assert_eq!(admit_at("late", NOW).await, None, "EXPIRED");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_loopback_port_is_recorded_for_waiting_or_connected_rows_only() {
    for target in targets() {
        let repo = RealmHubRepository::new(target.db.clone());
        enter(&repo, "m", NOW).await;
        let ready = repo
            .mark_route_ready(&actor(), HUB, 1, "m", 55_000, UnixMillis(NOW + 1), UnixMillis(NOW + 1))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (ready.state, ready.local_bridge_port),
            (PresenceState::Pending, Some(55_000))
        );
        admit(&repo, "n", NOW).await;
        let ready = repo
            .mark_route_ready(&actor(), HUB, 1, "n", 1, UnixMillis(NOW), UnixMillis(NOW + 999_999))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (ready.state, ready.local_bridge_ready_at),
            (PresenceState::Admitted, Some(UnixMillis(NOW)))
        );
        assert_eq!(
            repo.mark_route_ready(&actor(), HUB, 1, "m", 2, UnixMillis(NOW), UnixMillis(NOW + 90_000))
                .await
                .unwrap(),
            None
        );
        repo.mark_lost(&actor(), HUB, 1, "n", None, UnixMillis(NOW), UnixMillis(PURGE))
            .await
            .unwrap();
        assert_eq!(
            repo.mark_route_ready(&actor(), HUB, 1, "n", 2, UnixMillis(NOW), UnixMillis(NOW))
                .await
                .unwrap(),
            None
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_lost_link_ends_only_the_current_connection_and_dates_the_row() {
    for target in targets() {
        let repo = RealmHubRepository::new(target.db.clone());
        admit(&repo, "m", NOW).await;
        assert_eq!(
            repo.mark_lost(&actor(), HUB, 1, "m", Some("old"), UnixMillis(NOW), UnixMillis(PURGE))
                .await
                .unwrap(),
            None
        );
        let lost = repo
            .mark_lost(
                &actor(),
                HUB,
                1,
                "m",
                Some("adm-m"),
                UnixMillis(NOW + 7),
                UnixMillis(PURGE),
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (lost.state, lost.lost_at, lost.purge_at),
            (PresenceState::Lost, Some(UnixMillis(NOW + 7)), Some(UnixMillis(PURGE)))
        );
        assert_eq!(
            repo.mark_lost(&actor(), HUB, 1, "m", None, UnixMillis(NOW), UnixMillis(PURGE))
                .await
                .unwrap(),
            None
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn travelling_leaves_the_hub() {
    for target in targets() {
        let repo = RealmHubRepository::new(target.db.clone());
        enter(&repo, "waiting", NOW).await;
        assert_eq!(
            repo.mark_left(
                &actor(),
                HUB,
                1,
                "waiting",
                "TRAVEL",
                UnixMillis(NOW),
                UnixMillis(PURGE)
            )
            .await
            .unwrap(),
            None
        );
        admit(&repo, "m", NOW).await;
        let left = repo
            .mark_left(&actor(), HUB, 1, "m", "TRAVEL", UnixMillis(NOW + 3), UnixMillis(PURGE))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(left.state, PresenceState::Left);
        assert_eq!(
            (left.left_at, left.left_reason.as_deref()),
            (Some(UnixMillis(NOW + 3)), Some("TRAVEL"))
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn list_and_count_by_state_oldest_first_bounded_per_generation() {
    for target in targets() {
        let repo = RealmHubRepository::new(target.db.clone());
        enter(&repo, "b", NOW + 2).await;
        enter(&repo, "a", NOW + 2).await;
        enter(&repo, "c", NOW + 1).await;
        repo.request_entry(&actor(), &at(NOW, 2), &request("d")).await.unwrap();
        let waiting = [PresenceState::Pending, PresenceState::Ticketed];
        let accounts = |rows: Vec<HubPresenceRecord>| rows.into_iter().map(|r| r.account_id).collect::<Vec<_>>();
        assert_eq!(
            accounts(repo.list_by_state(HUB, 1, &waiting, 10).await.unwrap()),
            ["c", "a", "b"]
        );
        assert_eq!(
            accounts(repo.list_by_state(HUB, 1, &waiting, 2).await.unwrap()),
            ["c", "a"]
        );
        assert_eq!(accounts(repo.list_by_state(HUB, 2, &waiting, 10).await.unwrap()), ["d"]);
        assert_eq!(repo.count_by_state(HUB, 1, &waiting).await.unwrap(), 3);
        assert_eq!(
            repo.count_by_state(HUB, 1, &[PresenceState::Admitted]).await.unwrap(),
            0
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_newest_live_entry_across_hubs() {
    for target in targets() {
        let repo = RealmHubRepository::new(target.db.clone());
        enter(&repo, "m", NOW).await;
        let other = EntryAt {
            session_id: "hub-2".into(),
            ..at(NOW + 5, 1)
        };
        repo.request_entry(&actor(), &other, &request("m")).await.unwrap();
        assert_eq!(
            repo.find_for_account("m", None).await.unwrap().unwrap().session_id,
            "hub-2"
        );
        repo.mark_admitted(&actor(), "hub-2", 1, "m", "adm", UnixMillis(NOW), UnixMillis(NOW), None)
            .await
            .unwrap();
        repo.mark_lost(&actor(), "hub-2", 1, "m", None, UnixMillis(NOW), UnixMillis(PURGE))
            .await
            .unwrap();
        assert_eq!(repo.find_for_account("m", None).await.unwrap().unwrap().session_id, HUB);
        let lost = repo
            .find_for_account("m", Some(&[PresenceState::Lost]))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(lost.session_id, "hub-2");
        assert_eq!(repo.find_for_account("nobody", None).await.unwrap(), None);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_kick_or_ban_removes_only_that_accounts_live_rows_in_the_realm() {
    for target in targets() {
        let repo = RealmHubRepository::new(target.db.clone());
        admit(&repo, "m", NOW).await;
        enter(&repo, "keep", NOW).await;
        let elsewhere = EntryAt {
            session_id: "hub-other".into(),
            realm_id: "realm-2".into(),
            ..at(NOW, 1)
        };
        repo.request_entry(&actor(), &elsewhere, &request("m")).await.unwrap();
        let removed = repo
            .remove_account_from_realm(&actor(), REALM, "m", UnixMillis(NOW + 1), UnixMillis(PURGE))
            .await
            .unwrap();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].state, PresenceState::Admitted, "the row as it was");
        let now_row = repo.find_presence(HUB, "m").await.unwrap().unwrap();
        assert_eq!(now_row.state, PresenceState::Removed);
        assert_eq!(
            (now_row.left_reason.as_deref(), now_row.left_at),
            (Some("REMOVED"), Some(UnixMillis(NOW + 1)))
        );
        assert_eq!(
            repo.find_presence(HUB, "keep").await.unwrap().unwrap().state,
            PresenceState::Pending
        );
        assert_eq!(
            repo.find_presence("hub-other", "m").await.unwrap().unwrap().state,
            PresenceState::Pending
        );
        assert!(
            repo.remove_account_from_realm(&actor(), REALM, "m", UnixMillis(NOW), UnixMillis(PURGE))
                .await
                .unwrap()
                .is_empty()
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn release_removes_every_live_row() {
    for target in targets() {
        let repo = RealmHubRepository::new(target.db.clone());
        admit(&repo, "a", NOW).await;
        enter(&repo, "b", NOW).await;
        admit(&repo, "c", NOW).await;
        repo.mark_lost(&actor(), HUB, 1, "c", None, UnixMillis(NOW), UnixMillis(PURGE))
            .await
            .unwrap();
        let removed = repo
            .remove_realm(&actor(), REALM, UnixMillis(NOW + 1), UnixMillis(PURGE))
            .await
            .unwrap();
        assert_eq!(
            removed.iter().map(|r| r.account_id.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );
        let a = repo.find_presence(HUB, "a").await.unwrap().unwrap();
        assert_eq!(
            (a.state, a.left_reason.as_deref()),
            (PresenceState::Removed, Some("REALM_RELEASED"))
        );
        assert_eq!(
            repo.find_presence(HUB, "c").await.unwrap().unwrap().state,
            PresenceState::Lost
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn expired_waiting_rows_expire_connected_and_not_yet_due_rows_stay() {
    for target in targets() {
        let repo = RealmHubRepository::new(target.db.clone());
        enter(&repo, "old", NOW).await;
        enter(&repo, "fresh", NOW + 50_000).await;
        admit(&repo, "connected", NOW).await;
        let expired = repo
            .expire_stale(&actor(), None, UnixMillis(NOW + 90_000), UnixMillis(PURGE))
            .await
            .unwrap();
        assert_eq!(
            expired.iter().map(|r| r.account_id.as_str()).collect::<Vec<_>>(),
            ["old"]
        );
        let old = repo.find_presence(HUB, "old").await.unwrap().unwrap();
        assert_eq!(
            (old.state, old.purge_at),
            (PresenceState::Expired, Some(UnixMillis(PURGE)))
        );
        assert_eq!(
            repo.find_presence(HUB, "fresh").await.unwrap().unwrap().state,
            PresenceState::Pending
        );
        assert_eq!(
            repo.find_presence(HUB, "connected").await.unwrap().unwrap().state,
            PresenceState::Admitted
        );
        assert!(
            repo.expire_stale(&actor(), Some("hub-x"), UnixMillis(NOW + 999_999), UnixMillis(PURGE))
                .await
                .unwrap()
                .is_empty()
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_hub_end_turns_live_rows_lost_and_purge_deletes_them_later() {
    for target in targets() {
        let repo = RealmHubRepository::new(target.db.clone());
        admit(&repo, "b", NOW).await;
        enter(&repo, "a", NOW).await;
        repo.request_entry(&actor(), &at(NOW, 2), &request("next"))
            .await
            .unwrap();
        let ended = repo
            .end_session(&actor(), HUB, 1, UnixMillis(NOW + 1), UnixMillis(PURGE))
            .await
            .unwrap();
        assert_eq!(
            ended.iter().map(|r| r.account_id.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );
        let b = repo.find_presence(HUB, "b").await.unwrap().unwrap();
        assert_eq!((b.state, b.lost_at), (PresenceState::Lost, Some(UnixMillis(NOW + 1))));
        assert_eq!(
            repo.find_presence(HUB, "next").await.unwrap().unwrap().state,
            PresenceState::Pending
        );
        assert_eq!(repo.purge(UnixMillis(PURGE - 1)).await.unwrap(), 0);
        assert_eq!(repo.purge(UnixMillis(PURGE)).await.unwrap(), 2);
        assert_eq!(repo.find_presence(HUB, "b").await.unwrap(), None);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_batch_takes_its_seats_in_one_step_or_nobody_is_written() {
    for target in targets() {
        let repo = RealmHubRepository::new(target.db.clone());
        admit(&repo, "seated", NOW).await;
        let batch = [request("a"), request("b"), request("c")];
        let full = repo.request_entries(&actor(), &at(NOW, 1), 3, &batch).await.unwrap();
        assert_eq!(full, HubEntryBatch::Full { live: 1, max_peers: 3 });
        assert_eq!(repo.find_presence(HUB, "a").await.unwrap(), None);
        let HubEntryBatch::Placed(placed) = repo.request_entries(&actor(), &at(NOW, 1), 4, &batch).await.unwrap()
        else {
            panic!("expected Placed");
        };
        assert_eq!(
            placed.iter().map(|(a, _, _)| a.as_str()).collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
        // A placed batch reports as request_entry would.
        for (account, row, outcome) in placed {
            assert_eq!(outcome, EntryOutcome::Created);
            assert_eq!(repo.find_presence(HUB, &account).await.unwrap(), Some(row));
        }
        let HubEntryBatch::Placed(again) = repo
            .request_entries(&actor(), &at(NOW + 1, 1), 4, &batch)
            .await
            .unwrap()
        else {
            panic!("expected Placed");
        };
        assert!(again.iter().all(|(_, _, outcome)| *outcome == EntryOutcome::Reused));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn racing_batches_never_exceed_max_peers() {
    for target in targets() {
        let repo = RealmHubRepository::new(target.db.clone());
        let tasks: Vec<_> = (0..6)
            .map(|i| {
                let repo = repo.clone();
                tokio::spawn(async move {
                    let batch = [request(&format!("x{i}")), request(&format!("y{i}"))];
                    repo.request_entries(&actor(), &at(NOW, 1), 5, &batch).await.unwrap()
                })
            })
            .collect();
        for task in tasks {
            task.await.unwrap();
        }
        let live = repo.count_by_state(HUB, 1, &PresenceState::LIVE).await.unwrap();
        assert!(live <= 5, "{live} live rows");
        assert_eq!(live % 2, 0, "batches land whole");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_present_member_is_never_refused_and_an_expired_waiting_row_takes_no_seat() {
    for target in targets() {
        let repo = RealmHubRepository::new(target.db.clone());
        admit(&repo, "a", NOW).await;
        admit(&repo, "b", NOW).await;
        let placed = repo
            .request_entries(&actor(), &at(NOW, 1), 2, &[request("a")])
            .await
            .unwrap();
        assert!(matches!(placed, HubEntryBatch::Placed(ref rows) if rows[0].2 == EntryOutcome::Reentered));
        enter(&repo, "stale", NOW - 90_000).await;
        repo.mark_lost(&actor(), HUB, 1, "b", None, UnixMillis(NOW), UnixMillis(PURGE))
            .await
            .unwrap();
        // "a" re-entered as PENDING (live); "stale" expired; "b" lost: one seat taken of two.
        let placed = repo
            .request_entries(&actor(), &at(NOW, 1), 2, &[request("new")])
            .await
            .unwrap();
        assert!(matches!(placed, HubEntryBatch::Placed(_)));
        let full = repo
            .request_entries(&actor(), &at(NOW, 1), 2, &[request("one-more")])
            .await
            .unwrap();
        assert_eq!(full, HubEntryBatch::Full { live: 2, max_peers: 2 });
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn seats_are_per_hub_and_a_twice_listed_account_takes_one() {
    for target in targets() {
        let repo = RealmHubRepository::new(target.db.clone());
        admit(&repo, "a", NOW).await;
        let other = EntryAt {
            session_id: "hub-2".into(),
            ..at(NOW, 1)
        };
        let twice = [
            request("m"),
            HubEntryRequest {
                candidate_id: "second".into(),
                ..request("m")
            },
        ];
        let HubEntryBatch::Placed(placed) = repo.request_entries(&actor(), &other, 1, &twice).await.unwrap() else {
            panic!("expected Placed");
        };
        assert_eq!(placed.len(), 1);
        assert_eq!(placed[0].1.candidate_id, "second", "its last value");
        let HubEntryBatch::Placed(_) = repo.request_entries(&actor(), &at(NOW, 1), 2, &twice).await.unwrap() else {
            panic!("one seat each, per hub");
        };
    }
}
