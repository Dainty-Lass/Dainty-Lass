//! `p2p/persistence/realmHubRepositoryContract.ts`, on both engines.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{actor, t, targets};
use rifts_realms::hub::PresenceState::{self, Admitted, Expired, Left, Lost, Pending, Removed, Ticketed};
use rifts_realms::repo::hub::{
    EntryAt, EntryOutcome, HubEntryBatch, HubEntryRequest, HubPresenceRecord, RealmHubRepository,
};
use rifts_seam::UnixMillis;

const HUB: &str = "hub-session-1";
const REALM: &str = "realm-1";
const LIVE: &[PresenceState] = &[Pending, Ticketed, Admitted];

fn at(offset: i64) -> UnixMillis {
    UnixMillis(t("2026-10-14T12:00:00.000Z").0 + offset)
}

fn where_(session: &str, realm: &str, now: i64, expires: i64) -> EntryAt {
    EntryAt {
        session_id: session.to_owned(),
        generation: 1,
        realm_id: realm.to_owned(),
        now: at(now),
        expires_at: at(expires),
    }
}

fn hub_at(now: i64, expires: i64) -> EntryAt {
    where_(HUB, REALM, now, expires)
}

fn request(account: &str, candidate: &str) -> HubEntryRequest {
    HubEntryRequest {
        account_id: account.to_owned(),
        peer_id: format!("peer-{account}"),
        candidate_id: candidate.to_owned(),
    }
}

fn first(account: &str) -> HubEntryRequest {
    request(account, &format!("cand-{account}-1"))
}

fn batch(accounts: &[&str]) -> Vec<HubEntryRequest> {
    accounts.iter().map(|a| first(a)).collect()
}

fn ids(rows: &[HubPresenceRecord]) -> Vec<&str> {
    rows.iter().map(|row| row.account_id.as_str()).collect()
}

fn sorted(rows: &[HubPresenceRecord]) -> Vec<String> {
    let mut out: Vec<String> = rows.iter().map(|row| row.account_id.clone()).collect();
    out.sort();
    out
}

const WEEK: i64 = 604_800_000;
const DAY: i64 = 86_400_000;

macro_rules! each_engine {
    (|$repo:ident| $body:block) => {
        for target in targets() {
            let $repo = RealmHubRepository::new(target.open());
            eprintln!("engine: {}", target.name());
            $body
        }
    };
}

async fn admit(
    repo: &RealmHubRepository,
    session: &str,
    account: &str,
    admission: &str,
    now: i64,
) -> Option<HubPresenceRecord> {
    repo.mark_admitted(&actor(), session, 1, account, admission, at(now), at(now), None)
        .await
        .unwrap()
}

async fn host(repo: &RealmHubRepository, port: i64, now: i64) -> HubPresenceRecord {
    repo.admit_host(&actor(), HUB, 1, REALM, "host", "peer-host", port, at(now))
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_first_entry_is_pending_with_entry_epoch_1_and_round_trips() {
    each_engine!(|repo| {
        let (record, outcome) = repo
            .request_entry(&actor(), &hub_at(0, 90_000), &first("a"))
            .await
            .unwrap();
        assert_eq!(outcome, EntryOutcome::Created);
        assert_eq!(
            record,
            HubPresenceRecord {
                session_id: HUB.to_owned(),
                generation: 1,
                realm_id: REALM.to_owned(),
                account_id: "a".to_owned(),
                peer_id: "peer-a".to_owned(),
                state: Pending,
                candidate_id: "cand-a-1".to_owned(),
                entry_epoch: 1,
                local_bridge_port: None,
                local_bridge_ready_at: None,
                admission_id: None,
                route: None,
                requested_at: at(0),
                ticketed_at: None,
                admitted_at: None,
                lost_at: None,
                left_at: None,
                left_reason: None,
                expires_at: at(90_000),
                purge_at: None,
            }
        );
        assert_eq!(repo.find_presence(HUB, "a").await.unwrap(), Some(record));
        assert_eq!(repo.find_presence(HUB, "nobody").await.unwrap(), None);
        assert_eq!(repo.find_presence("other-session", "a").await.unwrap(), None);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_repeated_request_of_a_waiting_entry_reuses_it_and_only_extends_its_expiry() {
    each_engine!(|repo| {
        let a = actor();
        repo.request_entry(&a, &hub_at(0, 90_000), &first("a")).await.unwrap();
        let (again, outcome) = repo
            .request_entry(&a, &hub_at(10_000, 100_000), &request("a", "cand-a-2"))
            .await
            .unwrap();
        assert_eq!(outcome, EntryOutcome::Reused);
        assert_eq!(
            again.candidate_id, "cand-a-1",
            "the candidate of the attempt does not churn"
        );
        assert_eq!(again.entry_epoch, 1);
        assert_eq!(again.expires_at, at(100_000));
        let (shorter, _) = repo
            .request_entry(&a, &hub_at(20_000, 50_000), &request("a", "cand-a-3"))
            .await
            .unwrap();
        assert_eq!(shorter.expires_at, at(100_000), "an expiry is never shortened");
        repo.mark_ticketed(&a, HUB, 1, "a", at(21_000), at(200_000))
            .await
            .unwrap();
        let (ticketed, outcome) = repo
            .request_entry(&a, &hub_at(22_000, 120_000), &request("a", "cand-a-4"))
            .await
            .unwrap();
        assert_eq!(outcome, EntryOutcome::Reused);
        assert_eq!(ticketed.state, Ticketed, "a ticket in flight stays valid");
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_request_after_the_wait_ran_out_or_from_any_non_waiting_state_starts_a_new_entry() {
    each_engine!(|repo| {
        let a = actor();
        repo.request_entry(&a, &hub_at(0, 90_000), &first("a")).await.unwrap();
        let (expired, outcome) = repo
            .request_entry(&a, &hub_at(95_000, 185_000), &request("a", "cand-a-2"))
            .await
            .unwrap();
        assert_eq!(outcome, EntryOutcome::Reentered);
        assert_eq!(expired.entry_epoch, 2);
        assert_eq!(expired.candidate_id, "cand-a-2");
        assert_eq!(expired.state, Pending);
        assert_eq!(expired.requested_at, at(95_000));

        repo.mark_admitted(&a, HUB, 1, "a", "adm-1", at(96_000), at(96_000), Some("relay"))
            .await
            .unwrap();
        repo.mark_route_ready(&a, HUB, 1, "a", 55001, at(97_000), at(97_000))
            .await
            .unwrap();
        let (reentry, outcome) = repo
            .request_entry(&a, &hub_at(100_000, 190_000), &request("a", "cand-a-3"))
            .await
            .unwrap();
        assert_eq!(outcome, EntryOutcome::Reentered);
        assert_eq!(reentry.entry_epoch, 3);
        assert_eq!(reentry.state, Pending);
        assert_eq!(
            (
                &reentry.admission_id,
                reentry.admitted_at,
                &reentry.route,
                reentry.local_bridge_port
            ),
            (&None, None, &None, None),
            "a new entry clears the connection"
        );
        assert_eq!(
            (
                reentry.local_bridge_ready_at,
                reentry.ticketed_at,
                reentry.lost_at,
                reentry.left_at,
                reentry.purge_at
            ),
            (None, None, None, None, None)
        );

        for lost in ["lost", "left", "removed", "expired"] {
            let id = format!("acc-{lost}");
            repo.request_entry(&a, &hub_at(0, 90_000), &first(&id)).await.unwrap();
            admit(&repo, HUB, &id, &format!("adm-{lost}"), 1_000).await;
            match lost {
                "lost" | "expired" => {
                    repo.mark_lost(&a, HUB, 1, &id, None, at(2_000), at(DAY)).await.unwrap();
                }
                "left" => {
                    repo.mark_left(&a, HUB, 1, &id, "TRAVEL", at(2_000), at(DAY))
                        .await
                        .unwrap();
                }
                _ => {
                    repo.remove_account_from_realm(&a, REALM, &id, at(2_000), at(DAY))
                        .await
                        .unwrap();
                }
            }
            let (next, outcome) = repo
                .request_entry(&a, &hub_at(3_000, 93_000), &request(&id, &format!("c-{lost}")))
                .await
                .unwrap();
            assert_eq!(outcome, EntryOutcome::Reentered, "{lost}");
            assert_eq!(next.state, Pending, "{lost}");
            assert_eq!(next.entry_epoch, 2, "{lost}");
            assert_eq!(next.purge_at, None, "{lost}: a live row is not purged");
        }
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn racing_requests_for_one_entry_create_exactly_one_row() {
    each_engine!(|repo| {
        let mut tasks = tokio::task::JoinSet::new();
        for n in 1..=5 {
            let repo = repo.clone();
            tasks.spawn(async move {
                repo.request_entry(&actor(), &hub_at(0, 90_000), &request("race", &format!("cand-{n}")))
                    .await
                    .unwrap()
            });
        }
        let results = tasks.join_all().await;
        assert_eq!(results.iter().filter(|(_, o)| *o == EntryOutcome::Created).count(), 1);
        let candidates: std::collections::BTreeSet<&str> =
            results.iter().map(|(r, _)| r.candidate_id.as_str()).collect();
        assert_eq!(candidates.len(), 1, "every caller sees the same candidate");
        assert_eq!(repo.count_by_state(HUB, 1, &[Pending]).await.unwrap(), 1);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn the_hosts_row_is_created_admitted_and_replaced_by_a_later_call() {
    each_engine!(|repo| {
        let row = host(&repo, 55000, 0).await;
        assert_eq!(row.state, Admitted);
        assert_eq!(row.local_bridge_port, Some(55000));
        assert_eq!(row.entry_epoch, 1);
        assert_eq!(
            ids(&repo.list_by_state(HUB, 1, &[Admitted], 10).await.unwrap()),
            ["host"]
        );
        let again = host(&repo, 55010, 5_000).await;
        assert_eq!(again.local_bridge_port, Some(55010));
        assert_eq!(again.entry_epoch, 2);
        assert_eq!(
            repo.find_presence(HUB, "host")
                .await
                .unwrap()
                .unwrap()
                .local_bridge_port,
            Some(55010)
        );
        assert_eq!(repo.count_by_state(HUB, 1, &[Admitted]).await.unwrap(), 1);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_ticket_is_issued_for_a_waiting_unexpired_entry_and_keeps_the_row_alive() {
    each_engine!(|repo| {
        let a = actor();
        repo.request_entry(&a, &hub_at(0, 90_000), &first("a")).await.unwrap();
        assert_eq!(
            repo.mark_ticketed(&a, HUB, 2, "a", at(1_000), at(181_000))
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            repo.mark_ticketed(&a, HUB, 1, "nobody", at(1_000), at(181_000))
                .await
                .unwrap(),
            None
        );
        let ticketed = repo
            .mark_ticketed(&a, HUB, 1, "a", at(80_000), at(260_000))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ticketed.state, Ticketed);
        assert_eq!(ticketed.ticketed_at, Some(at(80_000)));
        assert_eq!(
            ticketed.expires_at,
            at(260_000),
            "the row outlives the entry's 90 s for the ticket"
        );
        let second = repo
            .mark_ticketed(&a, HUB, 1, "a", at(81_000), at(100_000))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second.state, Ticketed);
        assert_eq!(second.expires_at, at(260_000), "an expiry is never shortened");
        repo.request_entry(&a, &hub_at(0, 90_000), &first("b")).await.unwrap();
        assert_eq!(
            repo.mark_ticketed(&a, HUB, 1, "b", at(90_000), at(270_000))
                .await
                .unwrap(),
            None
        );
        admit(&repo, HUB, "a", "adm", 82_000).await;
        assert_eq!(
            repo.mark_ticketed(&a, HUB, 1, "a", at(83_000), at(263_000))
                .await
                .unwrap(),
            None
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn admission_from_pending_ticketed_or_again_admitted_never_from_lost_or_expired() {
    each_engine!(|repo| {
        let a = actor();
        for account in ["a", "b", "c"] {
            repo.request_entry(&a, &hub_at(0, 90_000), &first(account))
                .await
                .unwrap();
        }
        repo.mark_ticketed(&a, HUB, 1, "b", at(1_000), at(181_000))
            .await
            .unwrap();
        let admitted = repo
            .mark_admitted(&a, HUB, 1, "a", "adm-a", at(2_000), at(2_000), Some("direct"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(admitted.state, Admitted);
        assert_eq!(admitted.admission_id.as_deref(), Some("adm-a"));
        assert_eq!(admitted.route.as_deref(), Some("direct"));
        assert_eq!(admitted.admitted_at, Some(at(2_000)));
        assert_eq!(admit(&repo, HUB, "b", "adm-b", 2_000).await.unwrap().state, Admitted);
        let again = admit(&repo, HUB, "a", "adm-a", 3_000).await.unwrap();
        assert_eq!(again.state, Admitted, "re-consuming the same ticket is idempotent");
        assert_eq!(again.admission_id.as_deref(), Some("adm-a"));
        assert_eq!(admit(&repo, HUB, "c", "adm-c", 95_000).await, None, "the wait ran out");
        assert_eq!(
            repo.mark_admitted(&a, HUB, 2, "a", "x", at(4_000), at(4_000), None)
                .await
                .unwrap(),
            None
        );
        assert_eq!(admit(&repo, HUB, "nobody", "x", 4_000).await, None);
        repo.mark_lost(&a, HUB, 1, "a", None, at(5_000), at(DAY)).await.unwrap();
        assert_eq!(
            admit(&repo, HUB, "a", "adm-a2", 6_000).await,
            None,
            "a lost link needs a fresh entry"
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_members_loopback_port_is_recorded_for_a_waiting_or_connected_row_only() {
    each_engine!(|repo| {
        let a = actor();
        repo.request_entry(&a, &hub_at(0, 90_000), &first("a")).await.unwrap();
        let waiting = repo
            .mark_route_ready(&a, HUB, 1, "a", 56001, at(1_000), at(1_000))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(waiting.local_bridge_port, Some(56001));
        assert_eq!(waiting.local_bridge_ready_at, Some(at(1_000)));
        assert_eq!(waiting.state, Pending, "the route does not admit");
        admit(&repo, HUB, "a", "adm", 2_000).await;
        let connected = repo
            .mark_route_ready(&a, HUB, 1, "a", 56002, at(3_000), at(3_000))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(connected.local_bridge_port, Some(56002));
        assert_eq!(
            repo.mark_route_ready(&a, HUB, 2, "a", 1, at(1), at(1)).await.unwrap(),
            None
        );
        repo.request_entry(&a, &hub_at(0, 90_000), &first("late"))
            .await
            .unwrap();
        assert_eq!(
            repo.mark_route_ready(&a, HUB, 1, "late", 56003, at(95_000), at(95_000))
                .await
                .unwrap(),
            None
        );
        repo.mark_lost(&a, HUB, 1, "a", None, at(4_000), at(DAY)).await.unwrap();
        assert_eq!(
            repo.mark_route_ready(&a, HUB, 1, "a", 56004, at(5_000), at(5_000))
                .await
                .unwrap(),
            None
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_lost_link_ends_the_current_connection_only_and_its_row_is_dated_for_purging() {
    each_engine!(|repo| {
        let a = actor();
        repo.request_entry(&a, &hub_at(0, 90_000), &first("a")).await.unwrap();
        assert_eq!(
            repo.mark_lost(&a, HUB, 1, "a", None, at(1_000), at(WEEK))
                .await
                .unwrap(),
            None
        );
        admit(&repo, HUB, "a", "adm-new", 2_000).await;
        assert_eq!(
            repo.mark_lost(&a, HUB, 1, "a", Some("adm-old"), at(3_000), at(WEEK))
                .await
                .unwrap(),
            None,
            "a late report of an older connection does not end the newer one"
        );
        assert_eq!(repo.find_presence(HUB, "a").await.unwrap().unwrap().state, Admitted);
        let lost = repo
            .mark_lost(&a, HUB, 1, "a", Some("adm-new"), at(4_000), at(WEEK))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(lost.state, Lost);
        assert_eq!(lost.lost_at, Some(at(4_000)));
        assert_eq!(lost.purge_at, Some(at(WEEK)));
        assert_eq!(
            repo.mark_lost(&a, HUB, 1, "a", None, at(5_000), at(WEEK))
                .await
                .unwrap(),
            None,
            "already lost"
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn travelling_into_a_hunt_leaves_the_hub() {
    each_engine!(|repo| {
        let a = actor();
        repo.request_entry(&a, &hub_at(0, 90_000), &first("a")).await.unwrap();
        assert_eq!(
            repo.mark_left(&a, HUB, 1, "a", "TRAVEL", at(1_000), at(WEEK))
                .await
                .unwrap(),
            None
        );
        admit(&repo, HUB, "a", "adm", 2_000).await;
        let left = repo
            .mark_left(&a, HUB, 1, "a", "TRAVEL", at(3_000), at(WEEK))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(left.state, Left);
        assert_eq!(left.left_reason.as_deref(), Some("TRAVEL"));
        assert_eq!(left.left_at, Some(at(3_000)));
        assert_eq!(
            repo.mark_left(&a, HUB, 1, "a", "TRAVEL", at(4_000), at(WEEK))
                .await
                .unwrap(),
            None
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn listing_and_counting_by_state_oldest_first_bounded_scoped_to_the_generation() {
    each_engine!(|repo| {
        let a = actor();
        repo.request_entry(&a, &hub_at(3_000, 93_000), &first("c"))
            .await
            .unwrap();
        repo.request_entry(&a, &hub_at(1_000, 91_000), &first("a"))
            .await
            .unwrap();
        repo.request_entry(&a, &hub_at(2_000, 92_000), &first("b"))
            .await
            .unwrap();
        repo.request_entry(&a, &where_("other-hub", REALM, 500, 90_500), &first("elsewhere"))
            .await
            .unwrap();
        admit(&repo, HUB, "b", "adm", 4_000).await;
        assert_eq!(
            ids(&repo.list_by_state(HUB, 1, &[Pending, Admitted], 10).await.unwrap()),
            ["a", "b", "c"]
        );
        assert_eq!(
            ids(&repo.list_by_state(HUB, 1, &[Pending, Admitted], 2).await.unwrap()),
            ["a", "b"]
        );
        assert_eq!(ids(&repo.list_by_state(HUB, 1, &[Admitted], 10).await.unwrap()), ["b"]);
        assert!(
            repo.list_by_state(HUB, 2, &[Pending, Admitted], 10)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(repo.count_by_state(HUB, 1, LIVE).await.unwrap(), 3);
        assert_eq!(repo.count_by_state(HUB, 1, &[Lost]).await.unwrap(), 0);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn the_newest_live_entry_of_an_account_is_found_across_hubs() {
    each_engine!(|repo| {
        let a = actor();
        repo.request_entry(&a, &where_("hub-old", REALM, 1_000, 91_000), &request("a", "old"))
            .await
            .unwrap();
        repo.request_entry(&a, &where_("hub-new", REALM, 5_000, 95_000), &request("a", "new"))
            .await
            .unwrap();
        assert_eq!(
            repo.find_for_account("a", None).await.unwrap().unwrap().candidate_id,
            "new"
        );
        admit(&repo, "hub-old", "a", "adm", 6_000).await;
        assert_eq!(
            repo.find_for_account("a", Some(&[Admitted]))
                .await
                .unwrap()
                .unwrap()
                .session_id,
            "hub-old"
        );
        assert_eq!(repo.find_for_account("a", Some(&[Lost])).await.unwrap(), None);
        assert_eq!(repo.find_for_account("nobody", None).await.unwrap(), None);
        repo.mark_lost(&a, "hub-old", 1, "a", None, at(7_000), at(DAY))
            .await
            .unwrap();
        assert_eq!(
            repo.find_for_account("a", None).await.unwrap().unwrap().session_id,
            "hub-new",
            "a lost row is not live"
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn kick_or_ban_removes_the_accounts_live_rows_from_the_realm_and_no_other() {
    each_engine!(|repo| {
        let a = actor();
        repo.request_entry(&a, &where_("hub-1", REALM, 0, 90_000), &first("a"))
            .await
            .unwrap();
        repo.request_entry(&a, &where_("hub-2", REALM, 0, 90_000), &first("a"))
            .await
            .unwrap();
        admit(&repo, "hub-2", "a", "adm", 1_000).await;
        repo.request_entry(&a, &where_("hub-3", "realm-2", 0, 90_000), &first("a"))
            .await
            .unwrap();
        repo.request_entry(&a, &where_("hub-1", REALM, 0, 90_000), &first("b"))
            .await
            .unwrap();
        repo.request_entry(&a, &where_("hub-1", REALM, 0, 90_000), &first("lost"))
            .await
            .unwrap();
        let before = repo
            .remove_account_from_realm(&a, REALM, "a", at(2_000), at(WEEK))
            .await
            .unwrap();
        let mut seen: Vec<(String, PresenceState)> = before.iter().map(|r| (r.session_id.clone(), r.state)).collect();
        seen.sort_by(|x, y| x.0.cmp(&y.0));
        assert_eq!(
            seen,
            [("hub-1".to_owned(), Pending), ("hub-2".to_owned(), Admitted)],
            "the rows as they were"
        );
        let removed = repo.find_presence("hub-2", "a").await.unwrap().unwrap();
        assert_eq!(removed.state, Removed);
        assert_eq!(removed.purge_at, Some(at(WEEK)));
        assert_eq!(
            repo.find_presence("hub-3", "a").await.unwrap().unwrap().state,
            Pending,
            "another realm's row"
        );
        assert_eq!(
            repo.find_presence("hub-1", "b").await.unwrap().unwrap().state,
            Pending,
            "another account's row"
        );
        assert!(
            repo.remove_account_from_realm(&a, REALM, "a", at(3_000), at(WEEK))
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            repo.find_presence("hub-2", "a").await.unwrap().unwrap().purge_at,
            Some(at(WEEK))
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn releasing_a_realm_removes_every_live_row_of_it() {
    each_engine!(|repo| {
        let a = actor();
        repo.request_entry(&a, &where_("hub-1", REALM, 0, 90_000), &first("a"))
            .await
            .unwrap();
        repo.request_entry(&a, &where_("hub-2", REALM, 0, 90_000), &first("b"))
            .await
            .unwrap();
        admit(&repo, "hub-2", "b", "adm", 1_000).await;
        repo.request_entry(&a, &where_("hub-3", "realm-2", 0, 90_000), &first("c"))
            .await
            .unwrap();
        let rows = repo.remove_realm(&a, REALM, at(2_000), at(WEEK)).await.unwrap();
        assert_eq!(sorted(&rows), ["a", "b"]);
        assert_eq!(repo.find_presence("hub-1", "a").await.unwrap().unwrap().state, Removed);
        assert_eq!(repo.find_presence("hub-2", "b").await.unwrap().unwrap().state, Removed);
        assert_eq!(repo.find_presence("hub-3", "c").await.unwrap().unwrap().state, Pending);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_waiting_entry_past_its_expiry_becomes_expired_connected_and_not_yet_due_rows_stay() {
    each_engine!(|repo| {
        let a = actor();
        repo.request_entry(&a, &hub_at(0, 30_000), &first("due")).await.unwrap();
        repo.request_entry(&a, &hub_at(0, 60_000), &first("later"))
            .await
            .unwrap();
        repo.request_entry(&a, &hub_at(0, 30_000), &first("ticketed"))
            .await
            .unwrap();
        repo.mark_ticketed(&a, HUB, 1, "ticketed", at(1_000), at(31_000))
            .await
            .unwrap();
        repo.request_entry(&a, &hub_at(0, 30_000), &first("connected"))
            .await
            .unwrap();
        admit(&repo, HUB, "connected", "adm", 1_000).await;
        repo.request_entry(&a, &where_("other-hub", REALM, 0, 30_000), &first("other"))
            .await
            .unwrap();
        let expired = repo.expire_stale(&a, Some(HUB), at(45_000), at(WEEK)).await.unwrap();
        assert_eq!(sorted(&expired), ["due", "ticketed"]);
        assert_eq!(repo.find_presence(HUB, "due").await.unwrap().unwrap().state, Expired);
        assert_eq!(repo.find_presence(HUB, "later").await.unwrap().unwrap().state, Pending);
        assert_eq!(
            repo.find_presence(HUB, "connected").await.unwrap().unwrap().state,
            Admitted
        );
        assert_eq!(
            repo.find_presence("other-hub", "other").await.unwrap().unwrap().state,
            Pending,
            "scoped to the hub"
        );
        assert!(
            repo.expire_stale(&a, Some(HUB), at(45_000), at(WEEK))
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            ids(&repo.expire_stale(&a, None, at(45_000), at(WEEK)).await.unwrap()),
            ["other"],
            "every hub's"
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn the_end_of_a_hub_turns_the_generations_live_rows_into_lost() {
    each_engine!(|repo| {
        let a = actor();
        host(&repo, 55000, 0).await;
        repo.request_entry(&a, &hub_at(0, 90_000), &first("waiting"))
            .await
            .unwrap();
        repo.request_entry(&a, &hub_at(0, 90_000), &first("connected"))
            .await
            .unwrap();
        admit(&repo, HUB, "connected", "adm", 1_000).await;
        repo.request_entry(&a, &hub_at(0, 90_000), &first("gone"))
            .await
            .unwrap();
        admit(&repo, HUB, "gone", "adm2", 1_000).await;
        repo.mark_left(&a, HUB, 1, "gone", "TRAVEL", at(2_000), at(WEEK))
            .await
            .unwrap();
        repo.request_entry(&a, &where_("other-hub", REALM, 0, 90_000), &first("elsewhere"))
            .await
            .unwrap();
        assert!(
            repo.end_session(&a, HUB, 2, at(3_000), at(WEEK))
                .await
                .unwrap()
                .is_empty(),
            "another generation"
        );
        let rows = repo.end_session(&a, HUB, 1, at(3_000), at(WEEK)).await.unwrap();
        assert_eq!(sorted(&rows), ["connected", "host", "waiting"]);
        for id in ["connected", "host", "waiting"] {
            let row = repo.find_presence(HUB, id).await.unwrap().unwrap();
            assert_eq!(
                (row.state, row.lost_at, row.purge_at),
                (Lost, Some(at(3_000)), Some(at(WEEK))),
                "{id}"
            );
        }
        assert_eq!(repo.find_presence(HUB, "gone").await.unwrap().unwrap().state, Left);
        assert_eq!(
            repo.find_presence("other-hub", "elsewhere")
                .await
                .unwrap()
                .unwrap()
                .state,
            Pending
        );
    });
}

fn placed(result: &HubEntryBatch) -> bool {
    matches!(result, HubEntryBatch::Placed(_))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_batch_takes_its_seats_in_one_step_or_nobody_is_written() {
    each_engine!(|repo| {
        let a = actor();
        host(&repo, 55000, 0).await;
        assert!(
            placed(
                &repo
                    .request_entries(&a, &hub_at(0, 90_000), 3, &batch(&["a"]))
                    .await
                    .unwrap()
            ),
            "2 of 3"
        );
        assert_eq!(
            repo.request_entries(&a, &hub_at(0, 90_000), 3, &batch(&["b", "c"]))
                .await
                .unwrap(),
            HubEntryBatch::Full { live: 2, max_peers: 3 }
        );
        assert_eq!(
            repo.find_presence(HUB, "b").await.unwrap(),
            None,
            "a refused batch writes no row"
        );
        assert_eq!(repo.find_presence(HUB, "c").await.unwrap(), None);
        assert!(placed(
            &repo
                .request_entries(&a, &hub_at(0, 90_000), 3, &batch(&["c"]))
                .await
                .unwrap()
        ));
        assert_eq!(repo.count_by_state(HUB, 1, LIVE).await.unwrap(), 3);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_placed_batch_reports_each_members_entry_as_request_entry_would() {
    each_engine!(|repo| {
        let a = actor();
        let HubEntryBatch::Placed(entries) = repo
            .request_entries(&a, &hub_at(0, 90_000), 4, &batch(&["a", "b"]))
            .await
            .unwrap()
        else {
            panic!("not placed");
        };
        let summary: Vec<(&str, EntryOutcome, i64)> = entries
            .iter()
            .map(|(id, row, outcome)| (id.as_str(), *outcome, row.entry_epoch))
            .collect();
        assert_eq!(
            summary,
            [("a", EntryOutcome::Created, 1), ("b", EntryOutcome::Created, 1)]
        );
        let HubEntryBatch::Placed(again) = repo
            .request_entries(&a, &hub_at(1_000, 91_000), 4, &batch(&["a"]))
            .await
            .unwrap()
        else {
            panic!("not placed");
        };
        assert_eq!(again[0].2, EntryOutcome::Reused, "a waiting entry is reused");
        assert_eq!(
            repo.find_presence(HUB, "a").await.unwrap().unwrap().expires_at,
            at(91_000)
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn racing_entries_never_take_more_seats_than_max_peers() {
    each_engine!(|repo| {
        host(&repo, 55000, 0).await;
        let mut tasks = tokio::task::JoinSet::new();
        let mut groups: Vec<Vec<String>> = (0..12).map(|n| vec![format!("single-{n}")]).collect();
        groups.push(vec!["pair-1".to_owned(), "pair-2".to_owned()]);
        groups.push(vec!["pair-3".to_owned(), "pair-4".to_owned()]);
        for group in groups {
            let repo = repo.clone();
            tasks.spawn(async move {
                let accounts: Vec<&str> = group.iter().map(String::as_str).collect();
                repo.request_entries(&actor(), &hub_at(0, 90_000), 5, &batch(&accounts))
                    .await
                    .unwrap()
            });
        }
        let results = tasks.join_all().await;
        let seated: Vec<String> = results
            .iter()
            .flat_map(|result| match result {
                HubEntryBatch::Placed(entries) => entries.iter().map(|(id, _, _)| id.clone()).collect(),
                HubEntryBatch::Full { .. } => Vec::new(),
            })
            .collect();
        assert_eq!(seated.len(), 4, "the host and four members fill the five seats");
        assert_eq!(seated.iter().collect::<std::collections::BTreeSet<_>>().len(), 4);
        assert_eq!(repo.count_by_state(HUB, 1, LIVE).await.unwrap(), 5);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_member_already_present_is_never_refused_and_takes_no_seat() {
    each_engine!(|repo| {
        let a = actor();
        host(&repo, 55000, 0).await;
        assert!(
            placed(
                &repo
                    .request_entries(&a, &hub_at(0, 90_000), 3, &batch(&["a", "b"]))
                    .await
                    .unwrap()
            ),
            "full now"
        );
        assert!(!placed(
            &repo
                .request_entries(&a, &hub_at(0, 90_000), 3, &batch(&["c"]))
                .await
                .unwrap()
        ));
        let HubEntryBatch::Placed(waiting) = repo
            .request_entries(&a, &hub_at(0, 90_000), 3, &batch(&["a"]))
            .await
            .unwrap()
        else {
            panic!("a waiting member asking again is placed");
        };
        assert_eq!(waiting[0].2, EntryOutcome::Reused);
        admit(&repo, HUB, "a", "adm-a", 1_000).await;
        let HubEntryBatch::Placed(reentry) = repo
            .request_entries(&a, &hub_at(2_000, 90_000), 3, &batch(&["a"]))
            .await
            .unwrap()
        else {
            panic!("an admitted member asking again re-enters");
        };
        assert_eq!(reentry[0].2, EntryOutcome::Reentered);
        assert_eq!(
            repo.request_entries(&a, &hub_at(2_000, 90_000), 3, &batch(&["c"]))
                .await
                .unwrap(),
            HubEntryBatch::Full { live: 3, max_peers: 3 },
            "still full for a newcomer"
        );
        assert!(
            placed(
                &repo
                    .request_entries(&a, &hub_at(2_000, 90_000), 1, &batch(&["a", "b"]))
                    .await
                    .unwrap()
            ),
            "members already present are placed even when maxPeers is lower than the hub holds"
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn an_expired_waiting_entry_takes_no_seat() {
    each_engine!(|repo| {
        let a = actor();
        host(&repo, 55000, 0).await;
        assert!(
            placed(
                &repo
                    .request_entries(&a, &hub_at(0, 1_000), 2, &batch(&["a"]))
                    .await
                    .unwrap()
            ),
            "full"
        );
        assert_eq!(
            repo.request_entries(&a, &hub_at(500, 90_000), 2, &batch(&["b"]))
                .await
                .unwrap(),
            HubEntryBatch::Full { live: 2, max_peers: 2 },
            "a is still waiting"
        );
        assert!(
            placed(
                &repo
                    .request_entries(&a, &hub_at(5_000, 90_000), 2, &batch(&["b"]))
                    .await
                    .unwrap()
            ),
            "a expired at 1 s: its seat is free"
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_seat_is_counted_per_hub_and_an_account_listed_twice_takes_one() {
    each_engine!(|repo| {
        let a = actor();
        let other = where_("other-hub", REALM, 0, 90_000);
        assert!(placed(
            &repo
                .request_entries(&a, &other, 3, &batch(&["a", "b", "c"]))
                .await
                .unwrap()
        ));
        assert!(
            placed(
                &repo
                    .request_entries(&a, &hub_at(0, 90_000), 1, &batch(&["d"]))
                    .await
                    .unwrap()
            ),
            "per hub"
        );
        let HubEntryBatch::Placed(twice) = repo
            .request_entries(&a, &hub_at(0, 90_000), 2, &batch(&["e", "e"]))
            .await
            .unwrap()
        else {
            panic!("not placed");
        };
        assert_eq!(twice.len(), 1, "one entry for the account");
        assert_eq!(repo.count_by_state(HUB, 1, &[Pending]).await.unwrap(), 2);
    });
}
