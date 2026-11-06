// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The realms and realm presence repositories' contract, on SQLite and (when configured) Postgres.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use common::{Target, actor, t, targets};
use rifts_realms::realm::{
    AccountAt, HeartbeatUpdate, HostingPatch, HubState, JoinPolicy, MemberRole, Patch, RealmBan, RealmHosting,
    RealmInviteRecord, RealmMember, RealmPresenceRecord, RealmRecord, default_hosting, hub_room_ids_of,
};
use rifts_realms::repo::realms::{
    AddBanResult, AddEntryResult, AddMemberResult, InsertInviteResult, InsertRealmResult, RealmPresenceRepository,
    RealmsRepository,
};
use rifts_seam::UnixMillis;

const T0: i64 = 1_791_892_800_000;

/// Each engine with a realms repository over it.
fn engines() -> Vec<(Target, RealmsRepository)> {
    let pair = |target: Target| {
        let repo = RealmsRepository::new(target.db.clone());
        (target, repo)
    };
    targets().into_iter().map(pair).collect()
}

/// Each engine with a presence repository over it.
fn presence_engines() -> Vec<(Target, RealmPresenceRepository)> {
    let with_repo = |target: Target| (RealmPresenceRepository::new(target.db.clone()), target);
    targets()
        .into_iter()
        .map(with_repo)
        .map(|(repo, target)| (target, repo))
        .collect()
}

fn at(millis: i64) -> UnixMillis {
    UnixMillis(millis)
}

/// Invite-only and unlisted, the host its only member, never online.
fn realm(id: &str, name: &str, host: &str) -> RealmRecord {
    let founder = RealmMember {
        role: MemberRole::Host,
        joined_at: at(T0),
        account_id: host.into(),
    };
    RealmRecord {
        hub_room_ids: hub_room_ids_of(id),
        host_public_key: format!("key-{host}"),
        members: vec![founder],
        created_at: at(T0),
        join_policy: JoinPolicy::InviteOnly,
        listed: false,
        realm_id: id.into(),
        name: name.into(),
        host_account_id: host.into(),
        player_invites: Vec::new(),
        join_requests: Vec::new(),
        bans: Vec::new(),
        offline_notified_for: None,
        hosting: None,
        version: None,
        online_until: None,
        last_online_at: None,
    }
}

fn member(account: &str) -> RealmMember {
    RealmMember {
        joined_at: at(T0 + 1),
        role: MemberRole::Member,
        account_id: account.into(),
    }
}

fn entry(account: &str, when: i64) -> AccountAt {
    AccountAt {
        at: at(when),
        account_id: account.into(),
    }
}

/// Made by `host-1`, unused, an hour long.
fn invite(code: &str, realm_id: &str, created_at: i64, max_uses: i64) -> RealmInviteRecord {
    RealmInviteRecord {
        expires_at: at(created_at + 3_600_000),
        created_at: at(created_at),
        revoked_at: None,
        uses: 0,
        max_uses,
        created_by: "host-1".into(),
        realm_id: realm_id.into(),
        code: code.into(),
    }
}

fn presence(account: &str, realm_id: &str, expires_at: i64) -> RealmPresenceRecord {
    RealmPresenceRecord {
        expires_at: at(expires_at),
        last_seen_at: at(T0),
        bound_at: at(T0),
        realm_id: realm_id.into(),
        account_id: account.into(),
    }
}

/// `alpha` (`realm-1`), hosted by `host-1`.
async fn seeded(repo: &RealmsRepository) -> RealmRecord {
    let alpha = realm("realm-1", "alpha", "host-1");
    let inserted = repo.insert_realm(&actor(), &alpha).await.unwrap();
    assert_eq!(inserted, InsertRealmResult::Inserted);
    alpha
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn round_trip_keeps_optional_fields_absent_until_written() {
    for (target, repo) in engines() {
        let record = seeded(&repo).await;
        assert_eq!(repo.find_by_id("realm-1").await.unwrap(), Some(record.clone()));
        assert_eq!(repo.find_by_name("alpha").await.unwrap(), Some(record.clone()));
        assert_eq!(repo.find_by_host("host-1").await.unwrap(), Some(record.clone()));
        assert_eq!(repo.find_by_name("ALPHA").await.unwrap(), None, "{}", target.engine);
        assert_eq!(repo.find_by_id("realm-2").await.unwrap(), None);

        let full = RealmRecord {
            last_online_at: Some(UnixMillis(T0)),
            online_until: Some(UnixMillis(T0 + 90_000)),
            version: Some("0.1.80".into()),
            offline_notified_for: Some(UnixMillis(T0 - 1)),
            hosting: Some(RealmHosting {
                afk_seconds: Some(600),
                hub_session_id: Some("hub-1".into()),
                hub_state: HubState::Running,
                ..default_hosting(UnixMillis(T0))
            }),
            bans: vec![RealmBan {
                account_id: "bad".into(),
                reason: Some("griefing".into()),
                at: UnixMillis(T0),
            }],
            join_requests: vec![entry("asker", T0)],
            player_invites: vec![entry("guest", T0)],
            ..realm("realm-2", "beta", "host-2")
        };
        assert_eq!(
            repo.insert_realm(&actor(), &full).await.unwrap(),
            InsertRealmResult::Inserted
        );
        assert_eq!(repo.find_by_id("realm-2").await.unwrap(), Some(full));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn name_and_host_are_each_unique_name_first() {
    for (_engine, repo) in engines() {
        seeded(&repo).await;
        let insert = |r: RealmRecord| {
            let repo = repo.clone();
            async move { repo.insert_realm(&actor(), &r).await.unwrap() }
        };
        assert_eq!(
            insert(realm("realm-2", "alpha", "host-1")).await,
            InsertRealmResult::NameTaken
        );
        assert_eq!(
            insert(realm("realm-2", "alpha", "host-2")).await,
            InsertRealmResult::NameTaken
        );
        assert_eq!(
            insert(realm("realm-2", "beta", "host-1")).await,
            InsertRealmResult::HostHasRealm
        );
        assert_eq!(
            insert(realm("realm-2", "beta", "host-2")).await,
            InsertRealmResult::Inserted
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn racing_name_claims_make_one_realm() {
    for (_engine, repo) in engines() {
        let tasks: Vec<_> = (0..8)
            .map(|i| {
                let repo = repo.clone();
                tokio::spawn(async move {
                    repo.insert_realm(&actor(), &realm(&format!("realm-{i}"), "same", &format!("host-{i}")))
                        .await
                        .unwrap()
                })
            })
            .collect();
        let mut outcomes = Vec::new();
        for task in tasks {
            outcomes.push(task.await.unwrap());
        }
        assert_eq!(
            outcomes.iter().filter(|o| **o == InsertRealmResult::Inserted).count(),
            1
        );
        assert!(
            outcomes
                .iter()
                .all(|o| matches!(o, InsertRealmResult::Inserted | InsertRealmResult::NameTaken))
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn settings_and_heartbeats_only_on_the_hosts_own_realm() {
    for (_engine, repo) in engines() {
        seeded(&repo).await;
        assert!(
            repo.set_settings(&actor(), "missing", "nobody", None, None)
                .await
                .unwrap()
        );
        assert!(
            !repo
                .set_settings(&actor(), "realm-1", "host-2", Some(JoinPolicy::Public), None)
                .await
                .unwrap()
        );
        assert!(
            repo.set_settings(&actor(), "realm-1", "host-1", Some(JoinPolicy::Public), Some(true))
                .await
                .unwrap()
        );
        let found = repo.find_by_id("realm-1").await.unwrap().unwrap();
        assert_eq!((found.join_policy, found.listed), (JoinPolicy::Public, true));

        let beat = |version: Option<&str>, at: i64| HeartbeatUpdate {
            last_online_at: UnixMillis(at),
            online_until: UnixMillis(at + 90_000),
            version: version.map(str::to_owned),
        };
        assert!(
            !repo
                .record_heartbeat(&actor(), "realm-1", "host-2", &beat(Some("x"), T0))
                .await
                .unwrap()
        );
        assert!(
            repo.record_heartbeat(&actor(), "realm-1", "host-1", &beat(Some("0.1.9"), T0))
                .await
                .unwrap()
        );
        assert!(
            repo.record_heartbeat(&actor(), "realm-1", "host-1", &beat(None, T0 + 5))
                .await
                .unwrap()
        );
        let found = repo.find_by_id("realm-1").await.unwrap().unwrap();
        assert_eq!(found.last_online_at, Some(UnixMillis(T0 + 5)));
        assert_eq!(found.online_until, Some(UnixMillis(T0 + 90_005)));
        assert_eq!(found.version.as_deref(), Some("0.1.9"));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn members_are_added_once_never_past_the_ceiling_never_when_banned() {
    for (_engine, repo) in engines() {
        seeded(&repo).await;
        let add = |account: &'static str, max: u32| {
            let repo = repo.clone();
            async move {
                repo.add_member(&actor(), "realm-1", &member(account), max)
                    .await
                    .unwrap()
            }
        };
        assert_eq!(add("friend-1", 3).await, AddMemberResult::Added);
        assert_eq!(add("friend-1", 3).await, AddMemberResult::AlreadyMember);
        assert_eq!(add("host-1", 3).await, AddMemberResult::AlreadyMember);
        assert_eq!(add("friend-2", 3).await, AddMemberResult::Added);
        assert_eq!(add("friend-3", 3).await, AddMemberResult::Full);
        let ban = RealmBan {
            account_id: "friend-4".into(),
            reason: None,
            at: UnixMillis(T0),
        };
        assert_eq!(
            repo.add_ban(&actor(), "realm-1", &ban, 10).await.unwrap(),
            AddBanResult::Banned
        );
        assert_eq!(add("friend-4", 10).await, AddMemberResult::Banned);
        assert_eq!(
            repo.add_member(&actor(), "missing", &member("x"), 10).await.unwrap(),
            AddMemberResult::NotFound
        );
        let members: Vec<String> = repo
            .find_by_id("realm-1")
            .await
            .unwrap()
            .unwrap()
            .members
            .into_iter()
            .map(|m| m.account_id)
            .collect();
        assert_eq!(members, ["host-1", "friend-1", "friend-2"]);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn racing_joins_never_overfill_and_a_double_join_adds_once() {
    for (_engine, repo) in engines() {
        seeded(&repo).await;
        let mut tasks = Vec::new();
        for i in 0..10 {
            let repo = repo.clone();
            tasks.push(tokio::spawn(async move {
                repo.add_member(&actor(), "realm-1", &member(&format!("p-{i}")), 5)
                    .await
                    .unwrap()
            }));
        }
        for _ in 0..2 {
            let repo = repo.clone();
            tasks.push(tokio::spawn(async move {
                repo.add_member(&actor(), "realm-1", &member("twice"), 100)
                    .await
                    .unwrap()
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        let members = repo.find_by_id("realm-1").await.unwrap().unwrap().members;
        assert!(members.len() <= 6, "{} members", members.len());
        assert!(members.iter().filter(|m| m.account_id == "twice").count() <= 1);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn removing_a_member_never_removes_the_host() {
    for (_engine, repo) in engines() {
        seeded(&repo).await;
        repo.add_member(&actor(), "realm-1", &member("friend-1"), 10)
            .await
            .unwrap();
        assert!(!repo.remove_member(&actor(), "realm-1", "host-1").await.unwrap());
        assert!(repo.remove_member(&actor(), "realm-1", "friend-1").await.unwrap());
        assert!(!repo.remove_member(&actor(), "realm-1", "friend-1").await.unwrap());
        assert_eq!(repo.find_by_id("realm-1").await.unwrap().unwrap().members.len(), 1);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_ban_removes_the_member_once_has_a_ceiling_and_can_be_lifted() {
    for (_engine, repo) in engines() {
        seeded(&repo).await;
        repo.add_member(&actor(), "realm-1", &member("friend-1"), 10)
            .await
            .unwrap();
        let ban = |account: &str| RealmBan {
            account_id: account.to_owned(),
            reason: Some("rude".into()),
            at: UnixMillis(T0),
        };
        assert_eq!(
            repo.add_ban(&actor(), "realm-1", &ban("friend-1"), 2).await.unwrap(),
            AddBanResult::Banned
        );
        assert_eq!(
            repo.add_ban(&actor(), "realm-1", &ban("friend-1"), 2).await.unwrap(),
            AddBanResult::AlreadyBanned
        );
        assert_eq!(
            repo.add_ban(&actor(), "realm-1", &ban("b"), 2).await.unwrap(),
            AddBanResult::Banned
        );
        assert_eq!(
            repo.add_ban(&actor(), "realm-1", &ban("c"), 2).await.unwrap(),
            AddBanResult::Full
        );
        assert_eq!(
            repo.add_ban(&actor(), "missing", &ban("c"), 2).await.unwrap(),
            AddBanResult::NotFound
        );
        // A host's own row stays even when banned.
        assert_eq!(
            repo.add_ban(&actor(), "realm-1", &ban("host-1"), 9).await.unwrap(),
            AddBanResult::Banned
        );
        let found = repo.find_by_id("realm-1").await.unwrap().unwrap();
        assert_eq!(
            found.members.iter().map(|m| m.account_id.as_str()).collect::<Vec<_>>(),
            ["host-1"]
        );
        assert_eq!(found.bans.len(), 3);
        assert!(repo.remove_ban(&actor(), "realm-1", "friend-1").await.unwrap());
        assert!(!repo.remove_ban(&actor(), "realm-1", "friend-1").await.unwrap());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn find_by_member_and_the_listing_order() {
    for (_engine, repo) in engines() {
        for (id, name, host) in [
            ("r-a", "charlie", "h-a"),
            ("r-b", "alpha", "h-b"),
            ("r-c", "bravo", "h-c"),
        ] {
            repo.insert_realm(
                &actor(),
                &RealmRecord {
                    listed: true,
                    ..realm(id, name, host)
                },
            )
            .await
            .unwrap();
            repo.add_member(&actor(), id, &member("friend"), 10).await.unwrap();
        }
        repo.insert_realm(&actor(), &realm("r-d", "delta", "h-d"))
            .await
            .unwrap();
        let names = |list: Vec<RealmRecord>| list.into_iter().map(|r| r.name).collect::<Vec<_>>();
        assert_eq!(
            names(repo.find_by_member("friend", 10).await.unwrap()),
            ["alpha", "bravo", "charlie"]
        );
        assert_eq!(
            names(repo.find_by_member("friend", 2).await.unwrap()),
            ["alpha", "bravo"]
        );
        assert_eq!(names(repo.find_by_member("h-d", 10).await.unwrap()), ["delta"]);

        let beat = |at: i64| HeartbeatUpdate {
            last_online_at: UnixMillis(at),
            online_until: UnixMillis(at + 90_000),
            version: None,
        };
        repo.record_heartbeat(&actor(), "r-a", "h-a", &beat(T0)).await.unwrap();
        repo.record_heartbeat(&actor(), "r-c", "h-c", &beat(T0 + 10))
            .await
            .unwrap();
        assert_eq!(
            names(repo.list_listed(10, 0).await.unwrap()),
            ["bravo", "charlie", "alpha"]
        );
        assert_eq!(names(repo.list_listed(1, 1).await.unwrap()), ["charlie"]);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn name_release_only_when_not_online_since_read_with_its_invites() {
    for (_engine, repo) in engines() {
        seeded(&repo).await;
        repo.insert_invite(&actor(), &invite("ABCDEFGHJK", "realm-1", T0, 5))
            .await
            .unwrap();
        let beat = HeartbeatUpdate {
            last_online_at: UnixMillis(T0),
            online_until: UnixMillis(T0 + 90_000),
            version: None,
        };
        repo.record_heartbeat(&actor(), "realm-1", "host-1", &beat)
            .await
            .unwrap();
        assert!(
            !repo
                .delete_realm_if_last_online(&actor(), "realm-1", None)
                .await
                .unwrap()
        );
        assert!(
            !repo
                .delete_realm_if_last_online(&actor(), "realm-1", Some(UnixMillis(T0 - 1)))
                .await
                .unwrap()
        );
        assert!(
            repo.delete_realm_if_last_online(&actor(), "realm-1", Some(UnixMillis(T0)))
                .await
                .unwrap()
        );
        assert_eq!(repo.find_by_name("alpha").await.unwrap(), None);
        assert_eq!(repo.find_invite("ABCDEFGHJK").await.unwrap(), None);
        assert!(
            !repo
                .delete_realm_if_last_online(&actor(), "realm-1", Some(UnixMillis(T0)))
                .await
                .unwrap()
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_host_releases_its_own_realm_and_nobody_else_can() {
    for (_engine, repo) in engines() {
        seeded(&repo).await;
        repo.add_member(&actor(), "realm-1", &member("friend-1"), 10)
            .await
            .unwrap();
        repo.insert_invite(&actor(), &invite("ABCDEFGHJK", "realm-1", T0, 5))
            .await
            .unwrap();
        assert!(
            !repo
                .delete_realm_of_host(&actor(), "realm-1", "friend-1")
                .await
                .unwrap()
        );
        assert!(repo.delete_realm_of_host(&actor(), "realm-1", "host-1").await.unwrap());
        assert_eq!(repo.find_by_id("realm-1").await.unwrap(), None);
        assert_eq!(repo.find_invite("ABCDEFGHJK").await.unwrap(), None);
        // The name and host are free again.
        assert_eq!(
            repo.insert_realm(&actor(), &realm("realm-9", "alpha", "host-1"))
                .await
                .unwrap(),
            InsertRealmResult::Inserted
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invites_round_trip_with_a_unique_code_newest_first() {
    for (_engine, repo) in engines() {
        seeded(&repo).await;
        let first = invite("AAAAAAAAAA", "realm-1", T0, 5);
        assert_eq!(
            repo.insert_invite(&actor(), &first).await.unwrap(),
            InsertInviteResult::Inserted
        );
        assert_eq!(
            repo.insert_invite(&actor(), &first).await.unwrap(),
            InsertInviteResult::Duplicate
        );
        assert_eq!(repo.find_invite("AAAAAAAAAA").await.unwrap(), Some(first));
        repo.insert_invite(&actor(), &invite("BBBBBBBBBB", "realm-1", T0 + 5, 5))
            .await
            .unwrap();
        repo.insert_invite(&actor(), &invite("CCCCCCCCCC", "realm-1", T0 + 5, 5))
            .await
            .unwrap();
        repo.insert_invite(&actor(), &invite("DDDDDDDDDD", "realm-2", T0 + 9, 5))
            .await
            .unwrap();
        let codes: Vec<String> = repo
            .list_invites("realm-1", 10)
            .await
            .unwrap()
            .into_iter()
            .map(|i| i.code)
            .collect();
        assert_eq!(codes, ["CCCCCCCCCC", "BBBBBBBBBB", "AAAAAAAAAA"]);
        assert_eq!(repo.list_invites("realm-1", 1).await.unwrap().len(), 1);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn consumption_until_used_up_expired_or_revoked_and_refunds() {
    for (_engine, repo) in engines() {
        seeded(&repo).await;
        repo.insert_invite(&actor(), &invite("AAAAAAAAAA", "realm-1", T0, 2))
            .await
            .unwrap();
        let consume = |at: i64| {
            let repo = repo.clone();
            async move {
                repo.consume_invite(&actor(), "AAAAAAAAAA", UnixMillis(at))
                    .await
                    .unwrap()
            }
        };
        assert_eq!(consume(T0).await.map(|i| i.uses), Some(1));
        assert_eq!(consume(T0).await.map(|i| i.uses), Some(2));
        assert_eq!(consume(T0).await, None);
        repo.refund_invite(&actor(), "AAAAAAAAAA").await.unwrap();
        assert_eq!(consume(T0 + 3_600_000).await, None, "expired");
        assert_eq!(consume(T0).await.map(|i| i.uses), Some(2));
        for _ in 0..3 {
            repo.refund_invite(&actor(), "AAAAAAAAAA").await.unwrap();
        }
        assert_eq!(repo.find_invite("AAAAAAAAAA").await.unwrap().unwrap().uses, 0);
        repo.refund_invite(&actor(), "NOPENOPENO").await.unwrap();
        repo.revoke_invite(&actor(), "realm-1", "AAAAAAAAAA", UnixMillis(T0))
            .await
            .unwrap();
        assert_eq!(consume(T0).await, None, "revoked");
        assert_eq!(
            repo.consume_invite(&actor(), "NOPENOPENO", UnixMillis(T0))
                .await
                .unwrap(),
            None
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn racing_joins_never_over_use_an_invite() {
    for (_engine, repo) in engines() {
        seeded(&repo).await;
        repo.insert_invite(&actor(), &invite("AAAAAAAAAA", "realm-1", T0, 3))
            .await
            .unwrap();
        let tasks: Vec<_> = (0..10)
            .map(|_| {
                let repo = repo.clone();
                tokio::spawn(async move {
                    repo.consume_invite(&actor(), "AAAAAAAAAA", UnixMillis(T0))
                        .await
                        .unwrap()
                })
            })
            .collect();
        let mut used = 0;
        for task in tasks {
            used += usize::from(task.await.unwrap().is_some());
        }
        assert_eq!(used, 3);
        assert_eq!(repo.find_invite("AAAAAAAAAA").await.unwrap().unwrap().uses, 3);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revoke_once_first_time_kept_only_for_its_realm() {
    for (_engine, repo) in engines() {
        seeded(&repo).await;
        repo.insert_invite(&actor(), &invite("AAAAAAAAAA", "realm-1", T0, 3))
            .await
            .unwrap();
        assert_eq!(
            repo.revoke_invite(&actor(), "realm-2", "AAAAAAAAAA", UnixMillis(T0))
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            repo.revoke_invite(&actor(), "realm-1", "ZZZZZZZZZZ", UnixMillis(T0))
                .await
                .unwrap(),
            None
        );
        let first = repo
            .revoke_invite(&actor(), "realm-1", "AAAAAAAAAA", UnixMillis(T0 + 1))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.revoked_at, Some(UnixMillis(T0 + 1)));
        let again = repo
            .revoke_invite(&actor(), "realm-1", "AAAAAAAAAA", UnixMillis(T0 + 9))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(again.revoked_at, Some(UnixMillis(T0 + 1)));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn open_invites_are_counted_at_a_time() {
    for (_engine, repo) in engines() {
        seeded(&repo).await;
        repo.insert_invite(&actor(), &invite("AAAAAAAAAA", "realm-1", T0, 1))
            .await
            .unwrap();
        repo.insert_invite(&actor(), &invite("BBBBBBBBBB", "realm-1", T0 + 1000, 5))
            .await
            .unwrap();
        repo.insert_invite(&actor(), &invite("CCCCCCCCCC", "realm-1", T0, 5))
            .await
            .unwrap();
        repo.insert_invite(&actor(), &invite("DDDDDDDDDD", "realm-2", T0, 5))
            .await
            .unwrap();
        repo.consume_invite(&actor(), "AAAAAAAAAA", UnixMillis(T0))
            .await
            .unwrap();
        repo.revoke_invite(&actor(), "realm-1", "CCCCCCCCCC", UnixMillis(T0))
            .await
            .unwrap();
        assert_eq!(repo.count_active_invites("realm-1", UnixMillis(T0)).await.unwrap(), 1);
        assert_eq!(
            repo.count_active_invites("realm-1", UnixMillis(T0 + 3_600_999))
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            repo.count_active_invites("realm-1", UnixMillis(T0 + 3_601_000))
                .await
                .unwrap(),
            0
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn patch_hosting_creates_with_defaults_merges_removes_and_leaves_the_rest() {
    for (_engine, repo) in engines() {
        seeded(&repo).await;
        let patch = HostingPatch {
            max_players: Some(8),
            afk_seconds: Patch::Set(300),
            ..HostingPatch::default()
        };
        assert!(
            !repo
                .patch_hosting(&actor(), "missing", &patch, UnixMillis(T0))
                .await
                .unwrap()
        );
        assert!(
            repo.patch_hosting(&actor(), "realm-1", &patch, UnixMillis(T0))
                .await
                .unwrap()
        );
        let hosting = repo.find_by_id("realm-1").await.unwrap().unwrap().hosting.unwrap();
        assert_eq!(
            hosting,
            RealmHosting {
                max_players: 8,
                afk_seconds: Some(300),
                ..default_hosting(UnixMillis(T0))
            }
        );
        let next = HostingPatch {
            afk_seconds: Patch::Remove,
            hub_session_id: Patch::Set("hub-1".into()),
            training_state: Some(HubState::Starting),
            ..HostingPatch::default()
        };
        repo.patch_hosting(&actor(), "realm-1", &next, UnixMillis(T0 + 5))
            .await
            .unwrap();
        let hosting = repo.find_by_id("realm-1").await.unwrap().unwrap().hosting.unwrap();
        assert_eq!(
            hosting,
            RealmHosting {
                max_players: 8,
                hub_session_id: Some("hub-1".into()),
                training_state: HubState::Starting,
                ..default_hosting(UnixMillis(T0 + 5))
            }
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn racing_disjoint_patches_all_land() {
    for (_engine, repo) in engines() {
        seeded(&repo).await;
        let patches = [
            HostingPatch {
                max_servers: Some(3),
                ..HostingPatch::default()
            },
            HostingPatch {
                max_players: Some(32),
                ..HostingPatch::default()
            },
            HostingPatch {
                hub_state: Some(HubState::Running),
                ..HostingPatch::default()
            },
            HostingPatch {
                active_servers: Some(2),
                ..HostingPatch::default()
            },
        ];
        let tasks: Vec<_> = patches
            .into_iter()
            .map(|patch| {
                let repo = repo.clone();
                tokio::spawn(async move {
                    repo.patch_hosting(&actor(), "realm-1", &patch, UnixMillis(T0))
                        .await
                        .unwrap()
                })
            })
            .collect();
        for task in tasks {
            assert!(task.await.unwrap());
        }
        let h = repo.find_by_id("realm-1").await.unwrap().unwrap().hosting.unwrap();
        assert_eq!(
            (h.max_servers, h.max_players, h.hub_state, h.active_servers),
            (3, 32, HubState::Running, 2)
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mark_offline_once_and_only_when_online() {
    for (_engine, repo) in engines() {
        seeded(&repo).await;
        assert!(
            !repo.mark_offline(&actor(), "realm-1", UnixMillis(T0)).await.unwrap(),
            "never online"
        );
        let beat = HeartbeatUpdate {
            last_online_at: UnixMillis(T0),
            online_until: UnixMillis(T0 + 90_000),
            version: None,
        };
        repo.record_heartbeat(&actor(), "realm-1", "host-1", &beat)
            .await
            .unwrap();
        assert!(
            repo.mark_offline(&actor(), "realm-1", UnixMillis(T0 + 10))
                .await
                .unwrap()
        );
        assert!(
            !repo
                .mark_offline(&actor(), "realm-1", UnixMillis(T0 + 10))
                .await
                .unwrap()
        );
        let found = repo.find_by_id("realm-1").await.unwrap().unwrap();
        assert_eq!(
            (found.online_until, found.offline_notified_for),
            (Some(UnixMillis(T0 + 10)), Some(UnixMillis(T0 + 10)))
        );
        // The sweeper never announces it.
        assert!(
            repo.claim_lapsed_hosts(&actor(), UnixMillis(T0 + 100_000), UnixMillis(0), 10)
                .await
                .unwrap()
                .is_empty()
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn claim_lapsed_hosts_returns_each_recent_lapse_once_within_its_limit() {
    for (_engine, repo) in engines() {
        for (i, until) in [(1, T0 + 10), (2, T0 + 20), (3, T0 + 30), (4, T0 + 500), (5, T0 - 5)] {
            let id = format!("realm-{i}");
            let host = format!("host-{i}");
            repo.insert_realm(&actor(), &realm(&id, &format!("name-{i}"), &host))
                .await
                .unwrap();
            let beat = HeartbeatUpdate {
                last_online_at: UnixMillis(until - 90_000),
                online_until: UnixMillis(until),
                version: None,
            };
            repo.record_heartbeat(&actor(), &id, &host, &beat).await.unwrap();
        }
        let ids = |list: Vec<RealmRecord>| list.into_iter().map(|r| r.realm_id).collect::<Vec<_>>();
        let first = repo
            .claim_lapsed_hosts(&actor(), UnixMillis(T0 + 100), UnixMillis(T0), 2)
            .await
            .unwrap();
        assert_eq!(
            first.iter().map(|r| r.offline_notified_for).collect::<Vec<_>>(),
            [Some(UnixMillis(T0 + 10)), Some(UnixMillis(T0 + 20))]
        );
        assert_eq!(ids(first), ["realm-1", "realm-2"]);
        assert_eq!(
            ids(repo
                .claim_lapsed_hosts(&actor(), UnixMillis(T0 + 100), UnixMillis(T0), 10)
                .await
                .unwrap()),
            ["realm-3"]
        );
        assert!(
            repo.claim_lapsed_hosts(&actor(), UnixMillis(T0 + 100), UnixMillis(T0), 10)
                .await
                .unwrap()
                .is_empty()
        );
        // A new lapse is a new claim.
        let beat = HeartbeatUpdate {
            last_online_at: UnixMillis(T0),
            online_until: UnixMillis(T0 + 60),
            version: None,
        };
        repo.record_heartbeat(&actor(), "realm-1", "host-1", &beat)
            .await
            .unwrap();
        assert_eq!(
            ids(repo
                .claim_lapsed_hosts(&actor(), UnixMillis(T0 + 100), UnixMillis(T0), 10)
                .await
                .unwrap()),
            ["realm-1"]
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn find_by_either_hub_room_id_exact_only() {
    for (_engine, repo) in engines() {
        let record = seeded(&repo).await;
        for room in &record.hub_room_ids {
            assert_eq!(
                repo.find_by_hub_room_id(room).await.unwrap().map(|r| r.realm_id),
                Some("realm-1".into())
            );
            assert_eq!(repo.find_by_hub_room_id(&room.to_uppercase()).await.unwrap(), None);
            assert_eq!(repo.find_by_hub_room_id(&room[..35]).await.unwrap(), None);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn join_requests_once_refused_in_order_and_bounded() {
    for (_engine, repo) in engines() {
        seeded(&repo).await;
        repo.add_member(&actor(), "realm-1", &member("friend-1"), 10)
            .await
            .unwrap();
        repo.add_ban(
            &actor(),
            "realm-1",
            &RealmBan {
                account_id: "bad".into(),
                reason: None,
                at: UnixMillis(T0),
            },
            10,
        )
        .await
        .unwrap();
        let ask = |account: &'static str, max: u32| {
            let repo = repo.clone();
            async move {
                repo.add_join_request(&actor(), "realm-1", &entry(account, T0), max)
                    .await
                    .unwrap()
            }
        };
        assert_eq!(ask("asker", 2).await, AddEntryResult::Added);
        assert_eq!(ask("asker", 2).await, AddEntryResult::Already);
        assert_eq!(ask("bad", 2).await, AddEntryResult::Banned);
        assert_eq!(ask("friend-1", 2).await, AddEntryResult::AlreadyMember);
        assert_eq!(ask("second", 2).await, AddEntryResult::Added);
        assert_eq!(ask("third", 2).await, AddEntryResult::Full);
        assert_eq!(
            repo.add_join_request(&actor(), "missing", &entry("x", T0), 2)
                .await
                .unwrap(),
            AddEntryResult::NotFound
        );
        assert!(repo.remove_join_request(&actor(), "realm-1", "asker").await.unwrap());
        assert!(!repo.remove_join_request(&actor(), "realm-1", "asker").await.unwrap());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_ban_withdraws_only_the_banned_accounts_request() {
    for (_engine, repo) in engines() {
        seeded(&repo).await;
        for account in ["a", "b"] {
            repo.add_join_request(&actor(), "realm-1", &entry(account, T0), 10)
                .await
                .unwrap();
        }
        repo.add_ban(
            &actor(),
            "realm-1",
            &RealmBan {
                account_id: "a".into(),
                reason: None,
                at: UnixMillis(T0),
            },
            10,
        )
        .await
        .unwrap();
        let requests = repo.find_by_id("realm-1").await.unwrap().unwrap().join_requests;
        assert_eq!(requests, [entry("b", T0)]);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn racing_requests_never_exceed_the_bound() {
    for (_engine, repo) in engines() {
        seeded(&repo).await;
        let tasks: Vec<_> = (0..10)
            .map(|i| {
                let repo = repo.clone();
                tokio::spawn(async move {
                    repo.add_join_request(&actor(), "realm-1", &entry(&format!("a-{i}"), T0), 4)
                        .await
                        .unwrap()
                })
            })
            .collect();
        for task in tasks {
            task.await.unwrap();
        }
        assert_eq!(
            repo.find_by_id("realm-1").await.unwrap().unwrap().join_requests.len(),
            4
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn player_invites_once_refused_in_order_and_withdrawn_by_membership_or_ban() {
    for (_engine, repo) in engines() {
        seeded(&repo).await;
        let invite_player = |account: &'static str| {
            let repo = repo.clone();
            async move {
                repo.add_player_invite(&actor(), "realm-1", &entry(account, T0), 3)
                    .await
                    .unwrap()
            }
        };
        assert_eq!(invite_player("guest").await, AddEntryResult::Added);
        assert_eq!(invite_player("guest").await, AddEntryResult::Already);
        assert_eq!(invite_player("host-1").await, AddEntryResult::AlreadyMember);
        assert_eq!(invite_player("other").await, AddEntryResult::Added);
        assert_eq!(invite_player("third").await, AddEntryResult::Added);
        assert_eq!(invite_player("fourth").await, AddEntryResult::Full);
        let names = |list: Vec<RealmRecord>| list.into_iter().map(|r| r.name).collect::<Vec<_>>();
        assert_eq!(names(repo.find_by_player_invite("guest", 10).await.unwrap()), ["alpha"]);

        repo.add_member(&actor(), "realm-1", &member("guest"), 10)
            .await
            .unwrap();
        repo.add_ban(
            &actor(),
            "realm-1",
            &RealmBan {
                account_id: "other".into(),
                reason: None,
                at: UnixMillis(T0),
            },
            10,
        )
        .await
        .unwrap();
        assert_eq!(invite_player("other").await, AddEntryResult::Banned);
        let invites = repo.find_by_id("realm-1").await.unwrap().unwrap().player_invites;
        assert_eq!(invites, [entry("third", T0)]);
        assert!(repo.find_by_player_invite("guest", 10).await.unwrap().is_empty());
        assert!(repo.remove_player_invite(&actor(), "realm-1", "third").await.unwrap());
        assert!(!repo.remove_player_invite(&actor(), "realm-1", "third").await.unwrap());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn purge_drops_invites_a_week_past_expiry_and_expired_presence() {
    for (target, repo) in engines() {
        let presence_repo = RealmPresenceRepository::new(target.db.clone());
        seeded(&repo).await;
        repo.insert_invite(&actor(), &invite("AAAAAAAAAA", "realm-1", T0, 3))
            .await
            .unwrap();
        presence_repo
            .bind(&actor(), &presence("p", "realm-1", T0 + 10))
            .await
            .unwrap();
        let purge_at = T0 + 3_600_000 + 604_800_000;
        assert_eq!(repo.purge_expired(UnixMillis(purge_at - 1)).await.unwrap(), 1);
        assert_eq!(repo.purge_expired(UnixMillis(purge_at)).await.unwrap(), 1);
        assert_eq!(repo.find_invite("AAAAAAAAAA").await.unwrap(), None);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn presence_round_trip_replacing_bind_active_strictly_before_expiry() {
    for (_engine, repo) in presence_engines() {
        let row = presence("p", "realm-1", T0 + 120_000);
        repo.bind(&actor(), &row).await.unwrap();
        assert_eq!(repo.find_active("p", UnixMillis(T0)).await.unwrap(), Some(row.clone()));
        assert_eq!(
            repo.find_active("p", UnixMillis(T0 + 119_999)).await.unwrap(),
            Some(row)
        );
        assert_eq!(repo.find_active("p", UnixMillis(T0 + 120_000)).await.unwrap(), None);
        let moved = RealmPresenceRecord {
            bound_at: UnixMillis(T0 + 5),
            ..presence("p", "realm-2", T0 + 200_000)
        };
        repo.bind(&actor(), &moved).await.unwrap();
        assert_eq!(repo.find_active("p", UnixMillis(T0)).await.unwrap(), Some(moved));
        assert_eq!(repo.find_active("nobody", UnixMillis(T0)).await.unwrap(), None);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn touch_extends_and_never_revives() {
    for (_engine, repo) in presence_engines() {
        repo.bind(&actor(), &presence("p", "realm-1", T0 + 100)).await.unwrap();
        let touched = repo
            .touch(&actor(), "p", UnixMillis(T0 + 50), UnixMillis(T0 + 500))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (touched.last_seen_at, touched.expires_at),
            (UnixMillis(T0 + 50), UnixMillis(T0 + 500))
        );
        assert_eq!(
            repo.touch(&actor(), "p", UnixMillis(T0 + 500), UnixMillis(T0 + 900))
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            repo.find_active("p", UnixMillis(T0 + 499))
                .await
                .unwrap()
                .map(|r| r.expires_at),
            Some(UnixMillis(T0 + 500))
        );
        assert_eq!(
            repo.touch(&actor(), "nobody", UnixMillis(T0), UnixMillis(T0 + 9))
                .await
                .unwrap(),
            None
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remove_with_and_without_a_realm_and_for_a_realm() {
    for (_engine, repo) in presence_engines() {
        for (account, realm_id) in [("a", "realm-1"), ("b", "realm-1"), ("c", "realm-2")] {
            repo.bind(&actor(), &presence(account, realm_id, T0 + 100))
                .await
                .unwrap();
        }
        assert!(!repo.remove(&actor(), "c", Some("realm-1")).await.unwrap());
        assert!(repo.remove(&actor(), "c", Some("realm-2")).await.unwrap());
        assert!(repo.remove(&actor(), "a", None).await.unwrap());
        assert!(!repo.remove(&actor(), "a", None).await.unwrap());
        repo.bind(&actor(), &presence("d", "realm-1", T0 + 100)).await.unwrap();
        assert_eq!(repo.remove_for_realm(&actor(), "realm-1").await.unwrap(), 2);
        assert_eq!(repo.remove_for_realm(&actor(), "realm-1").await.unwrap(), 0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn active_counts_per_realm_at_a_time() {
    for (_engine, repo) in presence_engines() {
        for (account, realm_id, expires) in [
            ("a", "realm-1", T0 + 100),
            ("b", "realm-1", T0 + 50),
            ("c", "realm-2", T0 + 100),
            ("d", "realm-3", T0 + 100),
        ] {
            repo.bind(&actor(), &presence(account, realm_id, expires))
                .await
                .unwrap();
        }
        let ids = |list: &[&str]| list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        let counts = repo
            .count_active_by_realm(&ids(&["realm-1", "realm-2", "realm-9"]), UnixMillis(T0))
            .await
            .unwrap();
        assert_eq!(counts, [("realm-1".to_owned(), 2), ("realm-2".to_owned(), 1)].into());
        let later = repo
            .count_active_by_realm(&ids(&["realm-1"]), UnixMillis(T0 + 50))
            .await
            .unwrap();
        assert_eq!(later, [("realm-1".to_owned(), 1)].into());
        assert!(
            repo.count_active_by_realm(&[], UnixMillis(T0))
                .await
                .unwrap()
                .is_empty()
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn racing_binds_leave_one_row() {
    for (_engine, repo) in presence_engines() {
        let tasks: Vec<_> = (0..8)
            .map(|i| {
                let repo = repo.clone();
                tokio::spawn(async move {
                    repo.bind(&actor(), &presence("p", &format!("realm-{i}"), T0 + 100))
                        .await
                        .unwrap()
                })
            })
            .collect();
        for task in tasks {
            task.await.unwrap();
        }
        let ids: Vec<String> = (0..8).map(|i| format!("realm-{i}")).collect();
        let counts = repo.count_active_by_realm(&ids, UnixMillis(T0)).await.unwrap();
        assert_eq!(counts.values().sum::<i64>(), 1);
        assert_eq!(t("2026-10-13T12:00:00.000Z"), T0);
    }
}
