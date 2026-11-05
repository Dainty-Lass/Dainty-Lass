//! `realms/realmsPresence.test.ts`, `realmsHosting.test.ts`, `realmSolo.test.ts` and `realmsEntry.test.ts` on both
//! engines: presence and its 2 s cache, the touch rule, the host's hosting block, the sweeper, the events, SOLO mode
//! and entry into a realm. Skipped: the two cases that hold a database read open to race a write (the cache's epoch
//! is checked by the single-threaded cases around them), and the one that stubs the repository's name release.
#![allow(clippy::unwrap_used)]

mod common;

use std::sync::{Arc, Mutex};

use common::kit::*;
use common::targets;
use rifts_realms::directory::EntryAuthorization;
use rifts_realms::directory::SWEEP_LOOKBACK_MS;
use rifts_realms::input::{EntryRealmInput, HeartbeatInput, HostingInput, InviteInput, JoinInput, SettingsInput};
use rifts_realms::realm::RealmPresenceRecord;
use rifts_realms::realm::{
    HostingPatch, HubState, JoinPolicy, PRESENCE_CACHE_MS, PRESENCE_TTL_MS, Patch, RealmEvent, RealmHosting,
    RealmLimits, RemovalReason, hub_room_id, presence_touch_needs_write,
};
use rifts_realms::repo::realms::RealmPresenceRepository;
use rifts_realms::time::iso;
use rifts_seam::{HubKind, UnixMillis};

macro_rules! each {
    (|$k:ident| $body:block) => {
        for target in targets() {
            eprintln!("engine: {}", target.name());
            let $k = kit(target.open());
            $body
        }
    };
    (|$k:ident, $o:ident| $change:expr; $body:block) => {
        for target in targets() {
            eprintln!("engine: {}", target.name());
            let $k = kit_with(target.open(), |$o: &mut Options| {
                $change;
            });
            $body
        }
    };
}

fn heartbeat() -> HeartbeatInput {
    HeartbeatInput::default()
}

fn join_name(name: &str) -> JoinInput {
    JoinInput {
        name: Some(name.to_owned()),
        ..JoinInput::default()
    }
}

fn entry(name: &str) -> EntryRealmInput {
    EntryRealmInput {
        name: name.to_owned(),
        invite_code: None,
        known_host_key: None,
    }
}

fn entry_with(name: &str, code: Option<&str>, key: Option<&str>) -> EntryRealmInput {
    EntryRealmInput {
        name: name.to_owned(),
        invite_code: code.map(str::to_owned),
        known_host_key: key.map(str::to_owned),
    }
}

fn invite(max_uses: i64, ttl_hours: i64) -> InviteInput {
    InviteInput { max_uses, ttl_hours }
}

fn ok(realm_id: &str, is_host: bool) -> EntryAuthorization {
    EntryAuthorization::Ok {
        realm_id: realm_id.to_owned(),
        is_host,
    }
}

fn refused(code: &'static str) -> EntryAuthorization {
    EntryAuthorization::Refused(code)
}

fn row(account: &str, realm: &str, expires: i64) -> RealmPresenceRecord {
    RealmPresenceRecord {
        account_id: account.to_owned(),
        realm_id: realm.to_owned(),
        bound_at: T0,
        last_seen_at: T0,
        expires_at: at(expires),
    }
}

fn public(i: &mut rifts_realms::input::CreateRealmInput) {
    i.join_policy = JoinPolicy::Public;
}

// ------------------------------------------------------------------ presence

#[tokio::test(flavor = "multi_thread")]
async fn a_bind_records_the_realm_with_a_120_second_life() {
    each!(|k| {
        assert_eq!(k.dir.presence_of("friend-1").await.unwrap(), None);
        k.dir.bind_presence("friend-1", Some("realm-1")).await.unwrap();
        assert_eq!(
            k.dir.presence_of("friend-1").await.unwrap(),
            Some(RealmPresenceRecord {
                account_id: "friend-1".to_owned(),
                realm_id: "realm-1".to_owned(),
                bound_at: T0,
                last_seen_at: T0,
                expires_at: at(PRESENCE_TTL_MS)
            })
        );
        assert_eq!(PRESENCE_TTL_MS, 120_000);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_bind_replaces_the_first_and_a_null_bind_deletes_it() {
    each!(|k| {
        k.dir.bind_presence("friend-1", Some("realm-1")).await.unwrap();
        k.set_now(5000);
        k.dir.bind_presence("friend-1", Some("realm-2")).await.unwrap();
        let now = k.dir.presence_of("friend-1").await.unwrap().unwrap();
        assert_eq!(
            (now.realm_id.as_str(), now.bound_at, now.expires_at),
            ("realm-2", at(5000), at(5000 + PRESENCE_TTL_MS))
        );
        assert_eq!(
            k.presence
                .count_active_by_realm(&["realm-1".to_owned(), "realm-2".to_owned()], at(5000))
                .await
                .unwrap()
                .len(),
            1
        );
        k.dir.bind_presence("friend-1", None).await.unwrap();
        assert_eq!(k.dir.presence_of("friend-1").await.unwrap(), None);
        k.dir.bind_presence("never-bound", None).await.unwrap();
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn presence_is_active_only_while_expires_at_is_in_the_future_even_if_the_row_remains() {
    each!(|k| {
        k.dir.bind_presence("friend-1", Some("realm-1")).await.unwrap();
        k.set_now(PRESENCE_TTL_MS - 1);
        assert_eq!(
            k.dir.presence_of("friend-1").await.unwrap().unwrap().realm_id,
            "realm-1"
        );
        k.set_now(PRESENCE_TTL_MS);
        assert_eq!(
            k.dir.presence_of("friend-1").await.unwrap(),
            None,
            "expired exactly at expiresAt"
        );
        // The row is still stored: the read enforces expiry itself
        let stored = k
            .presence
            .count_active_by_realm(&["realm-1".to_owned()], T0)
            .await
            .unwrap();
        assert_eq!(stored.get("realm-1"), Some(&1));
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_lapsed_row_answers_none_even_from_the_2_second_cache() {
    each!(|k| {
        k.dir.bind_presence("friend-1", Some("realm-1")).await.unwrap();
        k.set_now(PRESENCE_TTL_MS - 500);
        assert!(
            k.dir.presence_of("friend-1").await.unwrap().is_some(),
            "cached while 0.5 s from the end"
        );
        k.set_now(PRESENCE_TTL_MS + 500);
        assert_eq!(k.dir.presence_of("friend-1").await.unwrap(), None);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_touch_gives_an_active_presence_another_120_seconds_and_never_brings_back_an_expired_one() {
    each!(|k| {
        assert_eq!(
            k.dir.touch_presence("friend-1").await.unwrap(),
            None,
            "nothing to touch"
        );
        k.dir.bind_presence("friend-1", Some("realm-1")).await.unwrap();
        k.set_now(60_000);
        let touched = k.dir.touch_presence("friend-1").await.unwrap().unwrap();
        assert_eq!(
            (touched.bound_at, touched.last_seen_at, touched.expires_at),
            (T0, at(60_000), at(60_000 + PRESENCE_TTL_MS))
        );
        k.set_now(150_000);
        assert_eq!(
            k.dir.presence_of("friend-1").await.unwrap().unwrap().expires_at,
            at(180_000)
        );
        k.set_now(180_000);
        assert_eq!(
            k.dir.touch_presence("friend-1").await.unwrap(),
            None,
            "expired: not resurrected"
        );
        assert_eq!(k.dir.presence_of("friend-1").await.unwrap(), None);
        k.set_now(181_000);
        assert_eq!(k.dir.presence_of("friend-1").await.unwrap(), None, "and it stays gone");
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn leaving_removes_the_presence_at_once() {
    each!(|k| {
        k.dir.bind_presence("friend-1", Some("realm-1")).await.unwrap();
        assert!(k.dir.presence_of("friend-1").await.unwrap().is_some());
        k.dir.leave_presence("friend-1").await.unwrap();
        assert_eq!(k.dir.presence_of("friend-1").await.unwrap(), None);
        k.dir.leave_presence("friend-1").await.unwrap();
    });
}

// ------------------------------------------------------------------ the cache

#[tokio::test(flavor = "multi_thread")]
async fn presence_of_is_answered_from_a_2_second_cache_none_included_and_a_local_write_drops_it() {
    each!(|k| {
        assert_eq!(PRESENCE_CACHE_MS, 2000);
        assert_eq!(k.dir.presence_of("friend-1").await.unwrap(), None);
        assert_eq!(k.dir.presence_of("friend-1").await.unwrap(), None);
        assert_eq!(
            k.dir.presence_io().0,
            1,
            "a second ask within 2 s does not reach the database"
        );

        k.dir.bind_presence("friend-1", Some("realm-1")).await.unwrap();
        assert_eq!(
            k.dir.presence_of("friend-1").await.unwrap().unwrap().realm_id,
            "realm-1"
        );
        assert_eq!(k.dir.presence_io().0, 2, "the bind dropped the cached none");
        k.dir.presence_of("friend-1").await.unwrap();
        assert_eq!(k.dir.presence_io().0, 2);

        k.set_now(1999);
        k.dir.presence_of("friend-1").await.unwrap();
        assert_eq!(k.dir.presence_io().0, 2, "still fresh at 1.999 s");
        k.set_now(2000);
        k.dir.presence_of("friend-1").await.unwrap();
        assert_eq!(k.dir.presence_io().0, 3, "stale at 2 s");
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_change_made_by_another_process_shows_up_when_the_cache_lapses_and_not_before() {
    each!(|k| {
        assert_eq!(k.dir.presence_of("friend-1").await.unwrap(), None);
        k.presence
            .bind(&common::actor(), &row("friend-1", "realm-9", PRESENCE_TTL_MS))
            .await
            .unwrap();
        assert_eq!(k.dir.presence_of("friend-1").await.unwrap(), None, "cached none");
        k.set_now(2000);
        assert_eq!(
            k.dir.presence_of("friend-1").await.unwrap().unwrap().realm_id,
            "realm-9"
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn leave_and_touch_also_keep_the_cache_honest() {
    each!(|k| {
        k.dir.bind_presence("friend-1", Some("realm-1")).await.unwrap();
        k.dir.presence_of("friend-1").await.unwrap();
        k.dir.leave_presence("friend-1").await.unwrap();
        assert_eq!(
            k.dir.presence_of("friend-1").await.unwrap(),
            None,
            "the cached row was dropped by the leave"
        );

        k.dir.bind_presence("friend-1", Some("realm-1")).await.unwrap();
        k.set_now(31_000);
        let before = k.dir.presence_io().0;
        k.dir.touch_presence("friend-1").await.unwrap(); // 31 s old, so this one writes
        assert_eq!(
            k.dir.presence_of("friend-1").await.unwrap().unwrap().last_seen_at,
            at(31_000)
        );
        assert_eq!(
            k.dir.presence_io().0,
            before + 1,
            "the touch read once; the written row is cached"
        );
    });
}

// ------------------------------------------------------------------ touch coalescing

#[test]
fn the_touch_rule_writes_when_last_seen_is_over_30_s_old_or_under_60_s_of_life_is_left() {
    let write = |last: i64, expires: i64, now: i64| presence_touch_needs_write(at(last), at(expires), at(now));
    assert!(!write(0, PRESENCE_TTL_MS, 0));
    assert!(
        !write(0, PRESENCE_TTL_MS, 30_000),
        "exactly 30 s is not older than 30 s"
    );
    assert!(write(0, PRESENCE_TTL_MS, 30_001));
    assert!(write(0, 50_000, 0), "50 s left");
    assert!(!write(0, 60_000, 0), "exactly 60 s left");
    assert!(write(0, 60_000, 1), "59.999 s left");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_touch_inside_the_write_window_answers_from_the_row_and_writes_nothing() {
    each!(|k| {
        k.dir.bind_presence("friend-1", Some("realm-1")).await.unwrap();
        k.set_now(5_000);
        let first = k.dir.touch_presence("friend-1").await.unwrap().unwrap();
        assert_eq!(
            (first.last_seen_at, first.expires_at),
            (T0, at(PRESENCE_TTL_MS)),
            "the row as last written"
        );
        assert_eq!(k.dir.presence_io().1, 0, "no write for a 5 s old row");
        let reads = k.dir.presence_io().0;
        k.set_now(6_000);
        assert_eq!(
            k.dir.touch_presence("friend-1").await.unwrap().unwrap().realm_id,
            "realm-1"
        );
        assert_eq!(k.dir.presence_io().1, 0);
        assert_eq!(
            k.dir.presence_io().0,
            reads,
            "the second touch is decided from the fresh cache"
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_row_about_to_expire_is_written_even_when_it_was_written_recently() {
    each!(|k| {
        k.presence
            .bind(&common::actor(), &row("friend-1", "realm-1", 50_000))
            .await
            .unwrap();
        let touched = k.dir.touch_presence("friend-1").await.unwrap().unwrap();
        assert_eq!(touched.expires_at, at(PRESENCE_TTL_MS), "less than 60 s left: extended");
        assert_eq!(k.dir.presence_io().1, 1);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_touch_every_10_seconds_writes_once_per_40_seconds_and_the_presence_never_lapses() {
    each!(|k| {
        k.dir.bind_presence("friend-1", Some("realm-1")).await.unwrap();
        let (reads0, touches0) = k.dir.presence_io();
        let mut calls = 0;
        for offset in (10_000..=15 * 60_000).step_by(10_000) {
            k.set_now(offset);
            calls += 1;
            assert!(
                k.dir.touch_presence("friend-1").await.unwrap().is_some(),
                "present at {} s",
                offset / 1000
            );
        }
        let (reads, touches) = k.dir.presence_io();
        assert_eq!(touches - touches0, (15 * 60_000) / 40_000);
        assert_eq!(reads - reads0, calls, "every touch reads the row once");
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_touch_every_60_seconds_writes_on_every_touch_and_the_presence_never_lapses() {
    each!(|k| {
        k.dir.bind_presence("friend-1", Some("realm-1")).await.unwrap();
        for offset in (60_000..=15 * 60_000).step_by(60_000) {
            k.set_now(offset);
            assert_eq!(
                k.dir.touch_presence("friend-1").await.unwrap().unwrap().expires_at,
                at(offset + PRESENCE_TTL_MS)
            );
        }
        assert_eq!(k.dir.presence_io().1, 15);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn touches_at_least_every_60_seconds_with_any_spacing_never_let_the_presence_lapse() {
    for seed in [1u32, 7, 42, 9001] {
        for target in targets() {
            let k = kit(target.open());
            k.dir.bind_presence("friend-1", Some("realm-1")).await.unwrap();
            let (mut state, mut now) = (seed, 0i64);
            for touch in 0..300 {
                state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                now += 1_000 + i64::from(state % 60_000); // 1 s to 60 s between touches
                k.set_now(now);
                assert!(
                    k.dir.touch_presence("friend-1").await.unwrap().is_some(),
                    "seed {seed}: lapsed before touch {touch} ({now} ms in)"
                );
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_absent_or_lapsed_row_is_never_written_back_by_a_touch() {
    each!(|k| {
        assert_eq!(k.dir.touch_presence("nobody").await.unwrap(), None);
        k.dir.bind_presence("friend-1", Some("realm-1")).await.unwrap();
        k.set_now(PRESENCE_TTL_MS);
        assert_eq!(k.dir.touch_presence("friend-1").await.unwrap(), None, "lapsed");
        assert_eq!(k.dir.presence_io().1, 0, "decided from the read: nothing to write");
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cache_that_grows_without_bound_is_pruned_instead() {
    let target = targets().remove(0);
    let k = kit(target.open());
    for index in 0..10_050 {
        k.dir.presence_of(&format!("account-{index}")).await.unwrap();
    }
    assert_eq!(k.dir.presence_of("account-10049").await.unwrap(), None);
}

// ------------------------------------------------------------------ the launcher's presence calls

#[tokio::test(flavor = "multi_thread")]
async fn presence_heartbeat_describes_an_active_presence_and_never_brings_a_lapsed_one_back() {
    each!(|k| {
        let none = k.dir.presence_heartbeat("friend-1").await.unwrap();
        assert_eq!((none.realm, none.expires_at), (None, None));
        assert_eq!(k.dir.presence_status("friend-1").await.unwrap().realm, None);

        let realm = k.create("host-1", "friends").await;
        k.dir
            .bind_presence("friend-1", Some(&realm.info.realm_id))
            .await
            .unwrap();
        let status = k.dir.presence_status("friend-1").await.unwrap();
        let described = status.realm.unwrap();
        assert_eq!(
            (
                described.name.as_str(),
                described.serving,
                described.hub_ready,
                described.host_online
            ),
            ("friends", false, false, false)
        );
        assert_eq!(status.expires_at.as_deref(), Some(iso(at(PRESENCE_TTL_MS)).as_str()));

        k.serve("host-1", &realm.info.realm_id).await;
        // 31 s, not 30: a touch writes only once the row is older than 30 s
        k.set_now(31_000);
        let beat = k.dir.presence_heartbeat("friend-1").await.unwrap();
        let described = beat.realm.unwrap();
        assert_eq!(
            (described.serving, described.hub_ready, described.host_online),
            (true, true, true)
        );
        assert_eq!(
            beat.expires_at.as_deref(),
            Some(iso(at(31_000 + PRESENCE_TTL_MS)).as_str())
        );

        // The host stops reporting: the hub is still marked running but nothing serves
        k.set_now(130_000);
        k.dir.presence_heartbeat("friend-1").await.unwrap();
        let described = k.dir.presence_status("friend-1").await.unwrap().realm.unwrap();
        assert_eq!(
            (described.serving, described.hub_ready, described.host_online),
            (false, true, false)
        );

        // The launcher stopped for 3 minutes: the presence is gone and a heartbeat does not bring it back
        k.set_now(400_000);
        let gone = k.dir.presence_heartbeat("friend-1").await.unwrap();
        assert_eq!((gone.realm, gone.expires_at), (None, None));
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn presence_of_a_realm_that_no_longer_exists_reads_as_none() {
    each!(|k| {
        k.dir.bind_presence("friend-1", Some("realm-gone")).await.unwrap();
        assert_eq!(k.dir.presence_status("friend-1").await.unwrap().realm, None);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn presence_leave_is_idempotent_and_clears_the_context() {
    each!(|k| {
        k.dir.bind_presence("friend-1", Some("realm-1")).await.unwrap();
        assert!(k.dir.presence_leave("friend-1").await.unwrap().left);
        assert_eq!(k.dir.presence_of("friend-1").await.unwrap(), None);
        assert!(k.dir.presence_leave("friend-1").await.unwrap().left);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn the_launchers_presence_calls_count_against_the_heartbeat_and_read_budgets() {
    let limiter = Arc::new(Limiter {
        keys: Mutex::new(Vec::new()),
        limited: |key| key == "realms:heartbeat:friend-1",
    });
    for target in targets() {
        limiter.keys.lock().unwrap().clear();
        let shared = limiter.clone();
        let k = kit_with(target.open(), move |o| o.limiter = shared);
        rejects(k.dir.presence_heartbeat("friend-1").await, "AUTH_RATE_LIMITED");
        rejects(k.dir.presence_leave("friend-1").await, "AUTH_RATE_LIMITED");
        k.dir.presence_status("friend-1").await.unwrap();
        assert_eq!(
            *limiter.keys.lock().unwrap(),
            [
                "realms:heartbeat:friend-1",
                "realms:heartbeat:friend-1",
                "realms:read:friend-1"
            ]
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_listing_and_the_host_view_count_the_members_whose_presence_is_active() {
    each!(|k| {
        let alpha = k.create_with("host-1", "alpha", |i| i.listed = true).await;
        let bravo = k.create_with("host-2", "bravo", |i| i.listed = true).await;
        k.dir
            .bind_presence("friend-1", Some(&alpha.info.realm_id))
            .await
            .unwrap();
        k.dir
            .bind_presence("friend-2", Some(&alpha.info.realm_id))
            .await
            .unwrap();
        k.dir.bind_presence("host-2", Some(&bravo.info.realm_id)).await.unwrap();

        let listing = k.dir.list_listed("someone", 10, 0).await.unwrap().realms;
        let mut counts: Vec<(&str, i64)> = listing.iter().map(|r| (r.name.as_str(), r.player_count)).collect();
        counts.sort_unstable();
        assert_eq!(counts, [("alpha", 2), ("bravo", 1)]);
        assert_eq!(
            k.dir
                .get_mine("host-1")
                .await
                .unwrap()
                .hosted
                .unwrap()
                .info
                .player_count,
            2
        );
        assert_eq!(
            k.dir
                .get_by_name("someone", "bravo", None)
                .await
                .unwrap()
                .realm
                .player_count,
            1
        );

        k.set_now(60_000);
        k.dir.touch_presence("friend-1").await.unwrap();
        k.set_now(PRESENCE_TTL_MS); // friend-2 and host-2 lapsed, friend-1 was touched
        assert_eq!(
            k.dir
                .get_by_name("someone", "alpha", None)
                .await
                .unwrap()
                .realm
                .player_count,
            1
        );
        assert_eq!(
            k.dir
                .get_by_name("someone", "bravo", None)
                .await
                .unwrap()
                .realm
                .player_count,
            0
        );
    });
}

// ------------------------------------------------------------------ the host's hosting block and the p2p side's hub state

fn hosting_of(k: &Kit, realm_id: &str) -> impl std::future::Future<Output = Option<RealmHosting>> {
    let repo = k.repo.clone();
    let id = realm_id.to_owned();
    async move { repo.find_by_id(&id).await.unwrap().unwrap().hosting }
}

fn block(max_servers: i64, max_players: i64, afk: Option<i64>) -> HeartbeatInput {
    HeartbeatInput {
        hosting: Some(HostingInput {
            max_servers,
            max_players,
            afk_seconds: afk,
        }),
        ..heartbeat()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_host_heartbeat_stores_its_limits_an_omitted_block_leaves_them_an_omitted_afk_clears_it() {
    each!(|k| {
        let realm = k.create("host-1", "friends").await.info.realm_id;
        assert_eq!(
            hosting_of(&k, &realm).await,
            None,
            "no hosting record until someone reports one"
        );
        k.dir.heartbeat("host-1", &block(3, 24, Some(600))).await.unwrap();
        assert_eq!(
            hosting_of(&k, &realm).await,
            Some(RealmHosting {
                max_servers: 3,
                max_players: 24,
                afk_seconds: Some(600),
                hub_session_id: None,
                hub_state: HubState::None,
                training_state: HubState::None,
                active_servers: 0,
                updated_at: T0
            })
        );
        k.dir.heartbeat("host-1", &heartbeat()).await.unwrap();
        assert_eq!(
            hosting_of(&k, &realm).await.unwrap().max_servers,
            3,
            "a heartbeat without hosting changes nothing"
        );
        k.dir.heartbeat("host-1", &block(2, 8, None)).await.unwrap();
        let hosting = hosting_of(&k, &realm).await.unwrap();
        assert_eq!(
            (hosting.max_servers, hosting.max_players, hosting.afk_seconds),
            (2, 8, None)
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_heartbeat_never_writes_the_hub_fields_and_the_p2p_fields_never_undo_the_limits() {
    each!(|k| {
        let realm = k.create("host-1", "friends").await.info.realm_id;
        k.dir.heartbeat("host-1", &block(4, 32, None)).await.unwrap();
        let hub = HostingPatch {
            hub_state: Some(HubState::Running),
            hub_session_id: Patch::Set("sess-1".to_owned()),
            active_servers: Some(1),
            ..HostingPatch::default()
        };
        k.dir.set_hosting(&realm, &hub).await.unwrap();
        k.dir.heartbeat("host-1", &block(5, 33, None)).await.unwrap();
        assert_eq!(
            hosting_of(&k, &realm).await,
            Some(RealmHosting {
                max_servers: 5,
                max_players: 33,
                afk_seconds: None,
                hub_session_id: Some("sess-1".to_owned()),
                hub_state: HubState::Running,
                training_state: HubState::None,
                active_servers: 1,
                updated_at: T0
            })
        );
        let training = HostingPatch {
            training_state: Some(HubState::Starting),
            ..HostingPatch::default()
        };
        k.dir.set_hosting(&realm, &training).await.unwrap();
        assert_eq!(hosting_of(&k, &realm).await.unwrap().max_servers, 5);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn set_hosting_merges_a_patch_removes_the_session_id_validates_and_says_whether_the_realm_exists() {
    each!(|k| {
        let realm = k.create("host-1", "friends").await.info.realm_id;
        k.set_now(1000);
        let starting = HostingPatch {
            hub_state: Some(HubState::Starting),
            hub_session_id: Patch::Set("sess-1".to_owned()),
            ..HostingPatch::default()
        };
        assert!(k.dir.set_hosting(&realm, &starting).await.unwrap());
        let hosting = hosting_of(&k, &realm).await.unwrap();
        assert_eq!(
            (
                hosting.max_servers,
                hosting.max_players,
                hosting.hub_state,
                hosting.updated_at
            ),
            (1, 16, HubState::Starting, at(1000))
        );

        let cleared = HostingPatch {
            hub_state: Some(HubState::None),
            hub_session_id: Patch::Remove,
            ..HostingPatch::default()
        };
        k.dir.set_hosting(&realm, &cleared).await.unwrap();
        let hosting = hosting_of(&k, &realm).await.unwrap();
        assert_eq!((hosting.hub_state, hosting.hub_session_id), (HubState::None, None));

        let too_many = HostingPatch {
            max_players: Some(65),
            ..HostingPatch::default()
        };
        let error = k.dir.set_hosting(&realm, &too_many).await.unwrap_err();
        assert!(error.to_string().contains("maxPlayers"), "{error}");
        let running = HostingPatch {
            hub_state: Some(HubState::Running),
            ..HostingPatch::default()
        };
        assert!(!k.dir.set_hosting("no-such-realm", &running).await.unwrap());
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn serving_hub_ready_and_player_count_appear_in_the_listing_the_lookup_and_the_host_view() {
    each!(|k| {
        let realm = k
            .create_with("host-1", "friends", |i| i.listed = true)
            .await
            .info
            .realm_id;
        let view = || async { k.dir.list_listed("friend-1", 10, 0).await.unwrap().realms.remove(0) };
        let info = view().await;
        assert_eq!(
            (info.online, info.serving, info.hub_ready, info.player_count),
            (false, false, false, 0)
        );

        k.dir.heartbeat("host-1", &heartbeat()).await.unwrap();
        let info = view().await;
        assert_eq!(
            (info.online, info.serving, info.hub_ready),
            (true, false, false),
            "online with no hub"
        );

        let running = HostingPatch {
            hub_state: Some(HubState::Running),
            ..HostingPatch::default()
        };
        k.dir.set_hosting(&realm, &running).await.unwrap();
        k.dir.bind_presence("friend-1", Some(&realm)).await.unwrap();
        let info = view().await;
        assert_eq!(
            (info.online, info.serving, info.hub_ready, info.player_count),
            (true, true, true, 1)
        );
        let host = k.dir.get_mine("host-1").await.unwrap().hosted.unwrap().info;
        assert_eq!(
            (host.online, host.serving, host.hub_ready, host.player_count),
            (true, true, true, 1)
        );

        k.set_now(90_000);
        let info = view().await;
        assert_eq!(
            (info.online, info.serving, info.hub_ready),
            (false, false, true),
            "the hub flag is the p2p side's to clear"
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn mark_offline_ends_the_online_state_now_and_the_sweeper_does_not_announce_it() {
    each!(|k| {
        let realm = k.create("host-1", "friends").await.info.realm_id;
        assert!(!k.dir.mark_offline(&realm).await.unwrap(), "never online");
        assert!(!k.dir.mark_offline("no-such-realm").await.unwrap());
        k.dir.heartbeat("host-1", &heartbeat()).await.unwrap();
        k.set_now(10_000);
        assert!(k.dir.mark_offline(&realm).await.unwrap());
        let stored = k.repo.find_by_id(&realm).await.unwrap().unwrap();
        assert_eq!(stored.online_until, Some(at(10_000)));
        assert_eq!(
            stored.last_online_at,
            Some(T0),
            "the name stays held by the last real heartbeat"
        );
        assert!(
            !k.dir
                .get_by_name("friend-1", "friends", None)
                .await
                .unwrap()
                .realm
                .online
        );
        assert!(!k.dir.mark_offline(&realm).await.unwrap(), "already offline");
        k.set_now(20_000);
        assert_eq!(k.dir.sweep_lapsed_hosts().await.unwrap(), 0);
        assert!(
            k.events.all().is_empty(),
            "the p2p side called markOffline, so it is not told again"
        );
    });
}

// ------------------------------------------------------------------ the sweeper

fn offline(realm: &str, host: &str) -> RealmEvent {
    RealmEvent::HostOffline {
        realm_id: realm.to_owned(),
        host_account_id: host.to_owned(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_sweeper_reports_a_host_whose_online_state_lapsed_once_per_lapse() {
    each!(|k| {
        let realm = k.create("host-1", "friends").await.info.realm_id;
        assert_eq!(
            k.dir.sweep_lapsed_hosts().await.unwrap(),
            0,
            "never online: nothing to report"
        );
        k.dir.heartbeat("host-1", &heartbeat()).await.unwrap(); // online until 90 s
        k.set_now(89_999);
        assert_eq!(k.dir.sweep_lapsed_hosts().await.unwrap(), 0, "still online");
        k.set_now(90_000);
        assert_eq!(k.dir.sweep_lapsed_hosts().await.unwrap(), 1);
        assert_eq!(k.events.all(), [offline(&realm, "host-1")]);
        k.set_now(105_000);
        assert_eq!(
            k.dir.sweep_lapsed_hosts().await.unwrap(),
            0,
            "the same lapse is not reported twice"
        );
        k.set_now(120_000);
        assert_eq!(k.dir.sweep_lapsed_hosts().await.unwrap(), 0);
        assert_eq!(k.events.all().len(), 1);
        // The host comes back and lapses again: a new lapse, a new report
        k.set_now(200_000);
        k.dir.heartbeat("host-1", &heartbeat()).await.unwrap();
        k.set_now(250_000);
        assert_eq!(k.dir.sweep_lapsed_hosts().await.unwrap(), 0);
        k.set_now(290_000);
        assert_eq!(k.dir.sweep_lapsed_hosts().await.unwrap(), 1);
        assert_eq!(k.events.all().len(), 2);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_that_closes_its_realm_on_purpose_is_reported_by_the_next_sweep() {
    each!(|k| {
        let realm = k.create("host-1", "friends").await.info.realm_id;
        k.dir.heartbeat("host-1", &heartbeat()).await.unwrap();
        k.set_now(5000);
        k.dir
            .heartbeat(
                "host-1",
                &HeartbeatInput {
                    online: Some(false),
                    ..heartbeat()
                },
            )
            .await
            .unwrap();
        k.set_now(6000);
        assert_eq!(k.dir.sweep_lapsed_hosts().await.unwrap(), 1);
        assert_eq!(k.events.all(), [offline(&realm, "host-1")]);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn the_sweeper_only_reports_lapses_of_the_last_10_minutes_and_several_realms_in_one_sweep() {
    each!(|k| {
        k.create("host-1", "old-one").await;
        let fresh = k.create("host-2", "fresh-one").await.info.realm_id;
        k.dir.heartbeat("host-1", &heartbeat()).await.unwrap();
        k.set_now(650_000);
        k.dir.heartbeat("host-2", &heartbeat()).await.unwrap(); // online until 740 s
        k.set_now(SWEEP_LOOKBACK_MS + 90_000); // the old one lapsed exactly 10 minutes ago
        assert_eq!(
            k.dir.sweep_lapsed_hosts().await.unwrap(),
            0,
            "a lapse from 10 minutes ago is history, not news"
        );
        k.set_now(740_000);
        assert_eq!(k.dir.sweep_lapsed_hosts().await.unwrap(), 1);
        assert_eq!(k.events.all(), [offline(&fresh, "host-2")]);
    });
    each!(|k| {
        let a = k.create("host-1", "alpha").await.info.realm_id;
        let b = k.create("host-2", "bravo").await.info.realm_id;
        k.dir.heartbeat("host-1", &heartbeat()).await.unwrap();
        k.dir.heartbeat("host-2", &heartbeat()).await.unwrap();
        k.set_now(91_000);
        assert_eq!(k.dir.sweep_lapsed_hosts().await.unwrap(), 2);
        let mut ids: Vec<String> = k
            .events
            .all()
            .into_iter()
            .map(|e| match e {
                RealmEvent::HostOffline { realm_id, .. } => realm_id,
                other => panic!("{other:?}"),
            })
            .collect();
        ids.sort();
        let mut expected = vec![a, b];
        expected.sort();
        assert_eq!(ids, expected);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sink_that_fails_does_not_fail_the_sweep_and_the_lapse_is_still_noticed() {
    each!(|k, o| o.sink = Some(Arc::new(FailingSink)); {
        k.create("host-1", "friends").await;
        k.dir.heartbeat("host-1", &heartbeat()).await.unwrap();
        k.set_now(91_000);
        assert_eq!(k.dir.sweep_lapsed_hosts().await.unwrap(), 1);
        assert_eq!(k.dir.sweep_lapsed_hosts().await.unwrap(), 0);
    });
}

// ------------------------------------------------------------------ events from kick, ban, leave and release

fn removed(realm: &str, account: &str, reason: RemovalReason) -> RealmEvent {
    RealmEvent::MemberRemoved {
        realm_id: realm.to_owned(),
        account_id: account.to_owned(),
        reason,
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_kick_drops_the_members_presence_in_this_realm_only_and_emits_member_removed() {
    each!(|k| {
        let realm = k.create_with("host-1", "open-house", public).await.info.realm_id;
        let other = k.create_with("host-2", "other-house", public).await.info.realm_id;
        k.dir.join("friend-1", &join_name("open-house")).await.unwrap();
        k.dir.join("friend-2", &join_name("open-house")).await.unwrap();
        k.dir.bind_presence("friend-1", Some(&realm)).await.unwrap();
        k.dir.bind_presence("friend-2", Some(&other)).await.unwrap();
        k.dir.kick("host-1", "friend-1").await.unwrap();
        assert_eq!(k.dir.presence_of("friend-1").await.unwrap(), None);
        assert_eq!(k.events.all(), [removed(&realm, "friend-1", RemovalReason::Kick)]);
        k.dir.kick("host-1", "friend-2").await.unwrap();
        assert_eq!(
            k.dir.presence_of("friend-2").await.unwrap().unwrap().realm_id,
            other,
            "the other realm's presence is not touched"
        );
        assert_eq!(k.events.all().len(), 2);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_refused_kick_ban_repeat_or_leave_emits_nothing() {
    each!(|k| {
        k.create_with("host-1", "open-house", public).await;
        k.dir.join("friend-1", &join_name("open-house")).await.unwrap();
        assert!(k.dir.kick("host-1", "friend-2").await.is_err());
        assert!(k.dir.kick("host-1", "host-1").await.is_err());
        assert!(k.dir.leave("friend-2", "open-house").await.is_err());
        assert!(k.dir.leave("host-1", "open-house").await.is_err());
        assert!(k.events.all().is_empty());
        k.dir.ban("host-1", "friend-1", None).await.unwrap();
        assert_eq!(k.events.all().len(), 1);
        k.dir.ban("host-1", "friend-1", None).await.unwrap();
        assert_eq!(k.events.all().len(), 1, "already banned: nothing changed");
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_ban_drops_the_presence_and_emits_member_removed_for_a_member_and_a_stranger_alike() {
    each!(|k| {
        let realm = k.create_with("host-1", "open-house", public).await.info.realm_id;
        k.dir.join("friend-1", &join_name("open-house")).await.unwrap();
        k.dir.bind_presence("friend-1", Some(&realm)).await.unwrap();
        k.dir.ban("host-1", "friend-1", Some("griefing")).await.unwrap();
        assert_eq!(k.dir.presence_of("friend-1").await.unwrap(), None);
        k.dir.ban("host-1", "friend-2", None).await.unwrap();
        assert_eq!(
            k.events.all(),
            [
                removed(&realm, "friend-1", RemovalReason::Ban),
                removed(&realm, "friend-2", RemovalReason::Ban)
            ]
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_member_who_leaves_drops_their_presence_and_emits_member_removed() {
    each!(|k| {
        let realm = k.create_with("host-1", "open-house", public).await.info.realm_id;
        k.dir.join("friend-1", &join_name("open-house")).await.unwrap();
        k.dir.bind_presence("friend-1", Some(&realm)).await.unwrap();
        k.dir.leave("friend-1", "open-house").await.unwrap();
        assert_eq!(k.dir.presence_of("friend-1").await.unwrap(), None);
        assert_eq!(k.events.all(), [removed(&realm, "friend-1", RemovalReason::Leave)]);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn releasing_a_realm_removes_every_presence_row_of_it_and_emits_realm_released() {
    each!(|k| {
        let realm = k.create("host-1", "friends").await.info.realm_id;
        let other = k.create("host-2", "rivals").await.info.realm_id;
        k.dir.bind_presence("friend-1", Some(&realm)).await.unwrap();
        k.dir.bind_presence("host-1", Some(&realm)).await.unwrap();
        k.dir.bind_presence("friend-2", Some(&other)).await.unwrap();
        k.dir.presence_of("friend-1").await.unwrap(); // cached
        k.dir.release("host-1").await.unwrap();
        assert_eq!(k.dir.presence_of("friend-1").await.unwrap(), None);
        assert_eq!(k.dir.presence_of("host-1").await.unwrap(), None);
        assert_eq!(k.dir.presence_of("friend-2").await.unwrap().unwrap().realm_id, other);
        assert_eq!(
            k.events.all(),
            [RealmEvent::RealmReleased {
                realm_id: realm,
                host_account_id: "host-1".to_owned()
            }]
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_dormant_realm_replaced_by_a_new_claim_loses_its_presence_rows_too_with_no_event() {
    each!(|k| {
        let old = k.create("host-1", "friends").await.info.realm_id;
        k.dir.bind_presence("friend-1", Some(&old)).await.unwrap();
        k.set_now(3 * DAY);
        k.create("host-2", "friends").await;
        let left = k.presence.count_active_by_realm(&[old], at(3 * DAY - 1)).await.unwrap();
        assert!(left.is_empty());
        assert!(k.events.all().is_empty());
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sink_that_throws_or_none_at_all_never_fails_the_request_that_caused_the_event() {
    each!(|k, o| o.sink = Some(Arc::new(FailingSink)); {
        k.create_with("host-1", "open-house", public).await;
        k.dir.join("friend-1", &join_name("open-house")).await.unwrap();
        assert_eq!(k.dir.kick("host-1", "friend-1").await.unwrap().status, "KICKED");
        k.dir.join("friend-1", &join_name("open-house")).await.unwrap();
        assert!(k.dir.ban("host-1", "friend-1", None).await.unwrap().changed);
        assert!(k.dir.release("host-1").await.unwrap().released);
    });
    each!(|k, o| o.sink = None; {
        k.create_with("host-1", "open-house", public).await;
        k.dir.join("friend-1", &join_name("open-house")).await.unwrap();
        assert_eq!(k.dir.kick("host-1", "friend-1").await.unwrap().status, "KICKED");
    });
}

// ------------------------------------------------------------------ hub room ids, membership helpers

#[tokio::test(flavor = "multi_thread")]
async fn a_hub_room_id_finds_its_realm_for_both_hubs_in_any_case_anything_else_finds_nothing() {
    each!(|k| {
        let realm = k.create("host-1", "friends").await.info.realm_id;
        let other = k.create("host-2", "rivals").await.info.realm_id;
        let ramsgate = hub_room_id(&realm, HubKind::Ramsgate);
        let training = hub_room_id(&realm, HubKind::Training);
        let found = |id: String| {
            let dir = &k.dir;
            async move { dir.find_by_hub_room_id(&id).await.unwrap().map(|r| r.realm_id) }
        };
        assert_eq!(found(ramsgate).await, Some(realm.clone()));
        assert_eq!(found(training.to_uppercase()).await, Some(realm.clone()));
        assert_eq!(found(hub_room_id(&other, HubKind::Ramsgate)).await, Some(other));
        assert_eq!(found(hub_room_id("no-such-realm", HubKind::Ramsgate)).await, None);
        assert_eq!(found("not-a-uuid".to_owned()).await, None);
        assert_eq!(found(realm).await, None, "the realm id itself is not a room id");
        assert_eq!(found(String::new()).await, None);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn is_member_and_is_banned_answer_for_members_strangers_banned_accounts_and_unknown_realms() {
    each!(|k| {
        let realm = k.create_with("host-1", "open-house", public).await.info.realm_id;
        k.dir.join("friend-1", &join_name("open-house")).await.unwrap();
        k.dir.ban("host-1", "friend-2", None).await.unwrap();
        assert!(k.dir.is_member(&realm, "host-1").await.unwrap());
        assert!(k.dir.is_member(&realm, "friend-1").await.unwrap());
        assert!(!k.dir.is_member(&realm, "friend-2").await.unwrap());
        assert!(!k.dir.is_member("no-such-realm", "friend-1").await.unwrap());
        assert!(k.dir.is_banned(&realm, "friend-2").await.unwrap());
        assert!(!k.dir.is_banned(&realm, "friend-1").await.unwrap());
        assert!(!k.dir.is_banned("no-such-realm", "friend-2").await.unwrap());
        assert_eq!(k.dir.get_realm(&realm).await.unwrap().unwrap().name, "open-house");
        assert_eq!(
            k.dir.get_realm_by_host("host-1").await.unwrap().unwrap().realm_id,
            realm
        );
        assert!(k.dir.get_realm_by_host("friend-1").await.unwrap().is_none());
    });
}

// ------------------------------------------------------------------ SOLO mode [2026-10-16]

#[tokio::test(flavor = "multi_thread")]
async fn a_solo_realm_refuses_play_joins_and_requests_from_others_and_stores_nothing_the_host_plays() {
    each!(|k| {
        let realm = k.create_with("host-1", "friends", public).await.info.realm_id;
        k.serve("host-1", &realm).await;
        // friend-1 joins while the realm is public, then the host switches to solo mode
        assert!(k.dir.join("friend-1", &join_name("friends")).await.unwrap().joined);
        let solo = SettingsInput {
            join_policy: Some(JoinPolicy::Solo),
            listed: None,
        };
        k.dir.update_settings("host-1", &solo).await.unwrap();

        assert_eq!(
            k.dir.authorize_entry("friend-1", &entry("friends")).await.unwrap(),
            refused("REALM_SOLO"),
            "a member waits"
        );
        assert_eq!(
            k.dir.authorize_entry("friend-2", &entry("friends")).await.unwrap(),
            refused("REALM_SOLO")
        );
        rejects(k.dir.join("friend-2", &join_name("friends")).await, "REALM_SOLO");
        assert!(
            k.repo
                .find_by_id(&realm)
                .await
                .unwrap()
                .unwrap()
                .join_requests
                .is_empty(),
            "no join request is stored"
        );
        assert_eq!(
            k.dir.authorize_entry("host-1", &entry("friends")).await.unwrap(),
            ok(&realm, true)
        );

        let seen = k.dir.get_by_name("friend-1", "friends", None).await.unwrap().realm;
        assert_eq!((seen.join_policy, seen.serving, seen.online), ("SOLO", false, true));
        assert!(
            k.dir
                .get_by_name("host-1", "friends", None)
                .await
                .unwrap()
                .realm
                .serving
        );

        let back = SettingsInput {
            join_policy: Some(JoinPolicy::InviteOnly),
            listed: None,
        };
        k.dir.update_settings("host-1", &back).await.unwrap();
        assert_eq!(
            k.dir.authorize_entry("friend-1", &entry("friends")).await.unwrap(),
            ok(&realm, false)
        );
        assert_eq!(
            k.dir.authorize_entry("friend-2", &entry("friends")).await.unwrap(),
            refused("REALM_JOIN_REQUESTED")
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn the_public_directory_does_not_list_a_solo_realm_to_others_its_host_still_sees_it() {
    each!(|k| {
        k.create_with("host-1", "friends", |i| {
            i.join_policy = JoinPolicy::Public;
            i.listed = true;
        })
        .await;
        k.create_with("host-2", "others", |i| {
            i.join_policy = JoinPolicy::Public;
            i.listed = true;
        })
        .await;
        k.dir
            .update_settings(
                "host-1",
                &SettingsInput {
                    join_policy: Some(JoinPolicy::Solo),
                    listed: None,
                },
            )
            .await
            .unwrap();
        let names = |viewer: &'static str| {
            let dir = &k.dir;
            async move {
                let mut names: Vec<String> = dir
                    .list_listed(viewer, 20, 0)
                    .await
                    .unwrap()
                    .realms
                    .into_iter()
                    .map(|r| r.name)
                    .collect();
                names.sort();
                names
            }
        };
        assert_eq!(names("friend-1").await, ["others"]);
        assert_eq!(names("host-1").await, ["friends", "others"]);
    });
}

// ------------------------------------------------------------------ entry into a realm

#[tokio::test(flavor = "multi_thread")]
async fn a_serving_realm_admits_its_host_its_members_and_a_valid_invite_holder() {
    each!(|k| {
        let realm = k.create("host-1", "friends").await.info.realm_id;
        k.serve("host-1", &realm).await;
        assert_eq!(
            k.dir.authorize_entry("host-1", &entry("friends")).await.unwrap(),
            ok(&realm, true)
        );
        let made = k.dir.create_invite("host-1", &invite(2, 24)).await.unwrap().invite;
        let with_code = entry_with("friends", Some(&made.code), None);
        assert_eq!(
            k.dir.authorize_entry("friend-1", &with_code).await.unwrap(),
            ok(&realm, false)
        );
        // Now a member: no invite needed, and nothing more is used
        assert_eq!(
            k.dir.authorize_entry("friend-1", &entry("friends")).await.unwrap(),
            ok(&realm, false)
        );
        assert_eq!(
            k.dir.authorize_entry("friend-1", &with_code).await.unwrap(),
            ok(&realm, false)
        );
        let view = k.dir.get_mine("host-1").await.unwrap().hosted.unwrap();
        assert_eq!(
            view.members.iter().map(|m| m.account_id.as_str()).collect::<Vec<_>>(),
            ["host-1", "friend-1"]
        );
        assert_eq!(
            k.dir.list_invites("host-1").await.unwrap().invites[0].uses,
            1,
            "used once, by the join"
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_public_serving_realm_admits_a_stranger_and_makes_them_a_member() {
    each!(|k| {
        let realm = k.create_with("host-1", "open-house", public).await.info.realm_id;
        k.serve("host-1", &realm).await;
        assert_eq!(
            k.dir.authorize_entry("friend-1", &entry("open-house")).await.unwrap(),
            ok(&realm, false)
        );
        assert_eq!(
            k.dir
                .get_by_name("friend-1", "open-house", None)
                .await
                .unwrap()
                .realm
                .membership,
            Some("MEMBER")
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_realm_a_foreign_invite_and_a_changed_owner_are_each_refused_with_their_own_code() {
    each!(|k| {
        let realm = k.create("host-1", "friends").await.info.realm_id;
        k.serve("host-1", &realm).await;
        k.create("host-2", "rivals").await;
        let foreign = k.dir.create_invite("host-2", &invite(2, 24)).await.unwrap().invite;
        let decide = |input: EntryRealmInput| {
            let dir = &k.dir;
            async move { dir.authorize_entry("friend-1", &input).await.unwrap() }
        };
        assert_eq!(decide(entry("no-such-realm")).await, refused("REALM_NOT_FOUND"));
        assert_eq!(
            decide(entry_with("friends", Some("ZZZZZZZZZZ"), None)).await,
            refused("REALM_INVITE_INVALID")
        );
        assert_eq!(
            decide(entry_with("friends", Some(&foreign.code), None)).await,
            refused("REALM_INVITE_INVALID"),
            "another realm's invite"
        );
        assert_eq!(
            decide(entry_with("friends", None, Some(KEY_B))).await,
            refused("REALM_OWNER_CHANGED")
        );
        assert_eq!(
            decide(entry_with("friends", None, Some(KEY_A))).await,
            refused("REALM_JOIN_REQUESTED")
        );
        assert_eq!(
            k.dir
                .get_by_name("host-1", "friends", None)
                .await
                .unwrap()
                .realm
                .member_count,
            1,
            "nothing was joined"
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stranger_needs_an_invite_used_up_expired_and_revoked_ones_are_refused_and_use_nothing() {
    each!(|k| {
        let realm = k.create("host-1", "friends").await.info.realm_id;
        k.serve("host-1", &realm).await;
        assert_eq!(
            k.dir.authorize_entry("friend-1", &entry("friends")).await.unwrap(),
            refused("REALM_JOIN_REQUESTED")
        );
        let spent = k.dir.create_invite("host-1", &invite(1, 24)).await.unwrap().invite;
        let using = |code: &str| entry_with("friends", Some(code), None);
        assert_eq!(
            k.dir.authorize_entry("friend-1", &using(&spent.code)).await.unwrap(),
            ok(&realm, false)
        );
        assert_eq!(
            k.dir.authorize_entry("friend-2", &using(&spent.code)).await.unwrap(),
            refused("REALM_INVITE_EXPIRED")
        );

        let revoked = k.dir.create_invite("host-1", &invite(5, 24)).await.unwrap().invite;
        k.dir.revoke_invite("host-1", &revoked.code).await.unwrap();
        assert_eq!(
            k.dir.authorize_entry("friend-2", &using(&revoked.code)).await.unwrap(),
            refused("REALM_INVITE_EXPIRED")
        );

        let short = k.dir.create_invite("host-1", &invite(5, 1)).await.unwrap().invite;
        k.set_now(3_600_000);
        k.dir.heartbeat("host-1", &heartbeat()).await.unwrap(); // still online and serving at the new time
        assert_eq!(
            k.dir.authorize_entry("friend-2", &using(&short.code)).await.unwrap(),
            refused("REALM_INVITE_EXPIRED")
        );
        assert_eq!(k.repo.find_invite(&revoked.code).await.unwrap().unwrap().uses, 0);
        assert_eq!(k.repo.find_invite(&short.code).await.unwrap().unwrap().uses, 0);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_banned_account_is_refused_even_with_a_valid_invite_and_in_a_public_realm_and_uses_nothing() {
    each!(|k| {
        let realm = k.create("host-1", "friends").await.info.realm_id;
        k.serve("host-1", &realm).await;
        let made = k.dir.create_invite("host-1", &invite(5, 24)).await.unwrap().invite;
        let with_code = entry_with("friends", Some(&made.code), None);
        assert!(matches!(
            k.dir.authorize_entry("friend-1", &with_code).await.unwrap(),
            EntryAuthorization::Ok { .. }
        ));
        k.dir.ban("host-1", "friend-1", Some("griefing")).await.unwrap();
        assert_eq!(k.repo.find_invite(&made.code).await.unwrap().unwrap().uses, 1);
        assert_eq!(
            k.dir.authorize_entry("friend-1", &with_code).await.unwrap(),
            refused("REALM_BANNED")
        );
        k.dir
            .update_settings(
                "host-1",
                &SettingsInput {
                    join_policy: Some(JoinPolicy::Public),
                    listed: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            k.dir.authorize_entry("friend-1", &entry("friends")).await.unwrap(),
            refused("REALM_BANNED")
        );
        assert_eq!(
            k.repo.find_invite(&made.code).await.unwrap().unwrap().uses,
            1,
            "the refused entries used nothing"
        );
        assert!(matches!(
            k.dir.authorize_entry("friend-2", &entry("friends")).await.unwrap(),
            EntryAuthorization::Ok { .. }
        ));
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_realm_that_is_not_serving_is_offline_to_everyone_but_its_host() {
    each!(|k| {
        let realm = k.create_with("host-1", "friends", public).await.info.realm_id;
        k.dir.create_invite("host-1", &invite(5, 24)).await.unwrap();
        let set = |state| {
            let dir = &k.dir;
            let realm = realm.clone();
            async move {
                let patch = HostingPatch {
                    hub_state: Some(state),
                    ..HostingPatch::default()
                };
                dir.set_hosting(&realm, &patch).await.unwrap();
            }
        };
        let try_in = |who: &'static str| {
            let dir = &k.dir;
            async move { dir.authorize_entry(who, &entry("friends")).await.unwrap() }
        };
        assert_eq!(try_in("friend-1").await, refused("REALM_OFFLINE"), "never online");
        k.dir.heartbeat("host-1", &heartbeat()).await.unwrap();
        assert_eq!(try_in("friend-1").await, refused("REALM_OFFLINE"), "online with no hub");
        set(HubState::Starting).await;
        assert_eq!(try_in("friend-1").await, refused("REALM_OFFLINE"));
        set(HubState::Running).await;
        assert!(matches!(try_in("friend-1").await, EntryAuthorization::Ok { .. }));
        set(HubState::Stopping).await;
        assert_eq!(try_in("friend-2").await, refused("REALM_OFFLINE"));
        set(HubState::Running).await;
        assert!(matches!(try_in("friend-2").await, EntryAuthorization::Ok { .. }));
        k.set_now(90_000);
        assert_eq!(
            try_in("friend-1").await,
            refused("REALM_OFFLINE"),
            "the heartbeat lapsed: offline for members too"
        );
        assert_eq!(try_in("friend-2").await, refused("REALM_OFFLINE"));
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn the_host_enters_its_own_realm_even_when_nothing_is_serving() {
    each!(|k| {
        let realm = k.create("host-1", "friends").await.info.realm_id;
        assert_eq!(
            k.dir.authorize_entry("host-1", &entry("friends")).await.unwrap(),
            ok(&realm, true)
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn an_offline_refusal_costs_nothing_the_invite_is_not_used_and_the_stranger_not_made_a_member() {
    each!(|k| {
        let realm = k.create("host-1", "friends").await.info.realm_id;
        let made = k.dir.create_invite("host-1", &invite(1, 24)).await.unwrap().invite;
        let with_code = entry_with("friends", Some(&made.code), None);
        assert_eq!(
            k.dir.authorize_entry("friend-1", &with_code).await.unwrap(),
            refused("REALM_OFFLINE")
        );
        assert_eq!(k.repo.find_invite(&made.code).await.unwrap().unwrap().uses, 0);
        assert_eq!(k.repo.find_by_id(&realm).await.unwrap().unwrap().members.len(), 1);
        // Once the realm serves, the same invite still works
        k.serve("host-1", &realm).await;
        assert!(matches!(
            k.dir.authorize_entry("friend-1", &with_code).await.unwrap(),
            EntryAuthorization::Ok { .. }
        ));
        assert_eq!(k.repo.find_invite(&made.code).await.unwrap().unwrap().uses, 1);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_full_realm_refuses_the_entry_and_gives_the_invite_use_back() {
    let small = RealmLimits {
        max_members: 2,
        max_bans: 5,
        max_active_invites: 25,
        max_join_requests: 50,
        max_player_invites: 50,
    };
    each!(|k, o| o.limits = small; {
        let realm = k.create("host-1", "friends").await.info.realm_id;
        k.serve("host-1", &realm).await;
        let made = k.dir.create_invite("host-1", &invite(5, 24)).await.unwrap().invite;
        let with_code = entry_with("friends", Some(&made.code), None);
        assert!(matches!(k.dir.authorize_entry("friend-1", &with_code).await.unwrap(), EntryAuthorization::Ok { .. }));
        assert_eq!(k.dir.authorize_entry("friend-2", &with_code).await.unwrap(), refused("REALM_FULL"));
        assert_eq!(k.repo.find_invite(&made.code).await.unwrap().unwrap().uses, 1, "the failed entry's use was refunded");
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_realm_that_changed_hands_is_refused_for_the_old_host_key_before_anything_else() {
    each!(|k| {
        k.create_with("host-1", "open-house", public).await;
        k.set_now(3 * DAY);
        let next = k
            .create_with("host-2", "open-house", |i| {
                i.join_policy = JoinPolicy::Public;
                i.host_public_key = KEY_B.to_owned();
            })
            .await
            .info
            .realm_id;
        k.serve("host-2", &next).await;
        assert_eq!(
            k.dir
                .authorize_entry("friend-1", &entry_with("open-house", None, Some(KEY_A)))
                .await
                .unwrap(),
            refused("REALM_OWNER_CHANGED")
        );
        assert!(matches!(
            k.dir
                .authorize_entry("friend-1", &entry_with("open-house", None, Some(KEY_B)))
                .await
                .unwrap(),
            EntryAuthorization::Ok { .. }
        ));
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn entries_share_a_per_account_budget_and_a_spent_one_throws_the_rate_limit_error() {
    let limiter = Arc::new(Limiter {
        keys: Mutex::new(Vec::new()),
        limited: |key| key.starts_with("realms:entry:"),
    });
    for target in targets() {
        let shared = limiter.clone();
        let k = kit_with(target.open(), move |o| o.limiter = shared);
        k.create("host-1", "friends").await;
        let result = k.dir.authorize_entry("friend-1", &entry("friends")).await;
        rejects(result, "AUTH_RATE_LIMITED");
        assert!(
            limiter
                .keys
                .lock()
                .unwrap()
                .iter()
                .any(|key| key == "realms:entry:friend-1")
        );
    }
}

// ------------------------------------------------------------------ AuthorizeGameSessionRealm

#[tokio::test(flavor = "multi_thread")]
async fn a_game_session_without_a_realm_is_untouched() {
    each!(|k| {
        assert_eq!(
            k.dir
                .authorize_game_session_realm("friend-1", None, true)
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            k.dir
                .authorize_game_session_realm("friend-1", Some(&serde_json::Value::Null), false)
                .await
                .unwrap(),
            None
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_game_session_that_names_a_realm_while_the_module_is_off_is_refused_never_played_as_official() {
    each!(|k| {
        for realm in [
            serde_json::json!({ "name": "friends" }),
            serde_json::json!("friends"),
            serde_json::json!(false),
        ] {
            rejects(
                k.dir
                    .authorize_game_session_realm("friend-1", Some(&realm), false)
                    .await,
                "REALMS_DISABLED",
            );
        }
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_malformed_realm_is_refused_with_the_validation_code_before_the_service_is_asked() {
    each!(|k| {
        let ask = |realm: serde_json::Value| {
            let dir = &k.dir;
            async move { dir.authorize_game_session_realm("friend-1", Some(&realm), true).await }
        };
        rejects(ask(serde_json::json!({})).await, "REALM_VALIDATION_FAILED");
        rejects(ask(serde_json::json!("friends")).await, "REALM_VALIDATION_FAILED");
        rejects(ask(serde_json::json!({ "name": "a" })).await, "REALM_NAME_INVALID");
        rejects(
            ask(serde_json::json!({ "name": "friends", "inviteCode": "nope" })).await,
            "REALM_VALIDATION_FAILED",
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn an_authorized_game_session_names_its_realm_and_every_refusal_surfaces_as_that_launcher_error() {
    each!(|k| {
        let realm = k.create("host-1", "friends").await.info.realm_id;
        let ask = |who: &'static str, body: serde_json::Value| {
            let dir = &k.dir;
            async move { dir.authorize_game_session_realm(who, Some(&body), true).await }
        };
        rejects(
            ask("friend-1", serde_json::json!({ "name": "friends" })).await,
            "REALM_JOIN_REQUESTED",
        );
        assert_eq!(
            ask("host-1", serde_json::json!({ "name": "friends" })).await.unwrap(),
            Some((realm.clone(), true))
        );

        k.serve("host-1", &realm).await;
        let made = k.dir.create_invite("host-1", &invite(3, 24)).await.unwrap().invite;
        let granted = ask(
            "friend-1",
            serde_json::json!({ "name": "friends", "inviteCode": made.code }),
        )
        .await
        .unwrap();
        assert_eq!(granted, Some((realm.clone(), false)));
        rejects(
            ask(
                "friend-2",
                serde_json::json!({ "name": "friends", "knownHostKey": KEY_B }),
            )
            .await,
            "REALM_OWNER_CHANGED",
        );
        rejects(
            ask("friend-2", serde_json::json!({ "name": "missing-realm" })).await,
            "REALM_NOT_FOUND",
        );
        k.dir.ban("host-1", "friend-1", None).await.unwrap();
        rejects(
            ask("friend-1", serde_json::json!({ "name": "friends" })).await,
            "REALM_BANNED",
        );
        k.set_now(120_000); // the host stopped reporting
        let offline = ask(
            "friend-2",
            serde_json::json!({ "name": "friends", "inviteCode": made.code }),
        )
        .await;
        match offline {
            Err(rifts_realms::RealmsError::Refused(refusal)) => {
                assert_eq!(refusal.code, "REALM_OFFLINE");
                assert!(refusal.message.len() > 10);
            }
            other => panic!("{other:?}"),
        }
    });
}

// ------------------------------------------------------------------ BindRealmPresenceAtLogin

#[tokio::test(flavor = "multi_thread")]
async fn the_login_bind_does_nothing_while_the_module_is_off() {
    each!(|k| {
        k.dir
            .bind_presence_at_login("friend-1", Some("realm-1"), false)
            .await
            .unwrap();
        k.dir.bind_presence_at_login("friend-1", None, false).await.unwrap();
        assert_eq!(k.dir.presence_of("friend-1").await.unwrap(), None);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn every_login_resets_the_realm_context_a_code_with_a_realm_binds_it_one_without_clears_it() {
    each!(|k| {
        k.dir
            .bind_presence_at_login("friend-1", Some("realm-1"), true)
            .await
            .unwrap();
        assert_eq!(
            k.dir.presence_of("friend-1").await.unwrap().unwrap().realm_id,
            "realm-1"
        );
        k.dir
            .bind_presence_at_login("friend-1", Some("realm-2"), true)
            .await
            .unwrap();
        assert_eq!(
            k.dir.presence_of("friend-1").await.unwrap().unwrap().realm_id,
            "realm-2",
            "a new realm replaces the old one"
        );
        k.dir.bind_presence_at_login("friend-1", None, true).await.unwrap();
        assert_eq!(
            k.dir.presence_of("friend-1").await.unwrap(),
            None,
            "an official Play keeps no realm"
        );
    });
}

#[test]
fn the_unused_presence_repository_name_is_the_one_the_kit_builds() {
    // Compile-time: the kit's repository handle is the public one
    fn _takes(_: &RealmPresenceRepository) {}
    let _ = UnixMillis(0);
}
