//! `realms/realmsRepositoryContract.ts` and `realmPresenceRepositoryContract.ts`, on both engines. Node's racing cases
//! fire their calls at once (`Promise.all`); here they are spawned tasks against one database.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{actor, t, targets};
use rifts_realms::realm::{
    AccountAt, HostingPatch, HubState, JoinPolicy, MemberRole, Patch, RealmBan, RealmHosting, RealmInviteRecord,
    RealmMember, RealmPresenceRecord, RealmRecord, apply_heartbeat,
};
use rifts_realms::repo::realms::{
    AddBanResult, AddEntryResult, AddMemberResult, InsertInviteResult, InsertRealmResult, RealmPresenceRepository,
    RealmsRepository,
};

const NOW: &str = "2026-10-13T12:00:00.000Z";
const LATER: &str = "2026-10-13T13:00:00.000Z";
const FAR: &str = "2026-10-20T12:00:00.000Z";

fn make_realm(name: &str, host: &str) -> RealmRecord {
    RealmRecord {
        realm_id: format!("id-{name}"),
        name: name.to_owned(),
        host_account_id: host.to_owned(),
        host_public_key: "KEY-0123456789abcdef".to_owned(),
        join_policy: JoinPolicy::InviteOnly,
        listed: false,
        members: vec![member_at(host, MemberRole::Host, NOW)],
        bans: Vec::new(),
        created_at: t(NOW),
        last_online_at: None,
        online_until: None,
        version: None,
        hosting: None,
        hub_room_ids: Vec::new(),
        offline_notified_for: None,
        join_requests: Vec::new(),
        player_invites: Vec::new(),
    }
}

fn member_at(account: &str, role: MemberRole, at: &str) -> RealmMember {
    RealmMember {
        account_id: account.to_owned(),
        role,
        joined_at: t(at),
    }
}

fn member(account: &str) -> RealmMember {
    member_at(account, MemberRole::Member, NOW)
}

fn ban(account: &str, at: &str) -> RealmBan {
    RealmBan {
        account_id: account.to_owned(),
        reason: None,
        at: t(at),
    }
}

fn entry(account: &str, at: &str) -> AccountAt {
    AccountAt {
        account_id: account.to_owned(),
        at: t(at),
    }
}

fn make_invite(code: &str, realm_id: &str) -> RealmInviteRecord {
    RealmInviteRecord {
        code: code.to_owned(),
        realm_id: realm_id.to_owned(),
        created_by: "host".to_owned(),
        created_at: t(NOW),
        expires_at: t(FAR),
        max_uses: 2,
        uses: 0,
        revoked_at: None,
    }
}

fn heartbeat(at: &str) -> rifts_realms::realm::HeartbeatUpdate {
    apply_heartbeat(t(at), None, None)
}

fn names(realms: &[RealmRecord]) -> Vec<&str> {
    realms.iter().map(|realm| realm.name.as_str()).collect()
}

fn accounts(members: &[RealmMember]) -> Vec<&str> {
    members.iter().map(|member| member.account_id.as_str()).collect()
}

macro_rules! each_engine {
    (|$repo:ident| $body:block) => {
        for target in targets() {
            let $repo = RealmsRepository::new(target.open());
            eprintln!("engine: {}", target.name());
            $body
        }
    };
}

#[tokio::test(flavor = "multi_thread")]
async fn a_realm_round_trips_exactly_with_its_optional_fields_absent_until_written() {
    each_engine!(|repo| {
        let realm = make_realm("friends", "host-1");
        assert_eq!(
            repo.insert_realm(&actor(), &realm).await.unwrap(),
            InsertRealmResult::Inserted
        );
        assert_eq!(repo.find_by_id("id-friends").await.unwrap(), Some(realm.clone()));
        assert_eq!(repo.find_by_name("friends").await.unwrap(), Some(realm.clone()));
        assert_eq!(repo.find_by_host("host-1").await.unwrap(), Some(realm));
        assert_eq!(repo.find_by_id("nope").await.unwrap(), None);
        assert_eq!(repo.find_by_name("nope").await.unwrap(), None);
        assert_eq!(repo.find_by_host("nobody").await.unwrap(), None);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_name_and_a_host_account_are_each_unique() {
    each_engine!(|repo| {
        let a = actor();
        assert_eq!(
            repo.insert_realm(&a, &make_realm("friends", "host-1")).await.unwrap(),
            InsertRealmResult::Inserted
        );
        let mut other = make_realm("friends", "host-2");
        other.realm_id = "other-id".to_owned();
        assert_eq!(
            repo.insert_realm(&a, &other).await.unwrap(),
            InsertRealmResult::NameTaken
        );
        let mut second = make_realm("second", "host-1");
        second.realm_id = "second-id".to_owned();
        assert_eq!(
            repo.insert_realm(&a, &second).await.unwrap(),
            InsertRealmResult::HostHasRealm
        );
        assert_eq!(repo.find_by_name("second").await.unwrap(), None);
        second.host_account_id = "host-2".to_owned();
        second.members = vec![member_at("host-2", MemberRole::Host, NOW)];
        assert_eq!(
            repo.insert_realm(&a, &second).await.unwrap(),
            InsertRealmResult::Inserted
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn racing_claims_for_one_name_produce_exactly_one_realm() {
    each_engine!(|repo| {
        let mut tasks = tokio::task::JoinSet::new();
        for index in 0..8 {
            let repo = repo.clone();
            tasks.spawn(async move {
                let mut realm = make_realm("friends", &format!("host-{index}"));
                realm.realm_id = format!("race-{index}");
                repo.insert_realm(&actor(), &realm).await.unwrap()
            });
        }
        let results = tasks.join_all().await;
        assert_eq!(results.iter().filter(|r| **r == InsertRealmResult::Inserted).count(), 1);
        assert_eq!(
            results.iter().filter(|r| **r == InsertRealmResult::NameTaken).count(),
            7
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_and_heartbeats_change_only_the_hosts_own_realm() {
    each_engine!(|repo| {
        let a = actor();
        repo.insert_realm(&a, &make_realm("friends", "host-1")).await.unwrap();
        assert!(
            !repo
                .set_settings(&a, "id-friends", "intruder", None, Some(true))
                .await
                .unwrap()
        );
        assert!(!repo.find_by_id("id-friends").await.unwrap().unwrap().listed);
        assert!(
            repo.set_settings(&a, "id-friends", "host-1", Some(JoinPolicy::Public), Some(true))
                .await
                .unwrap()
        );
        assert!(
            !repo
                .set_settings(&a, "missing", "host-1", None, Some(true))
                .await
                .unwrap()
        );
        let after = repo.find_by_id("id-friends").await.unwrap().unwrap();
        assert_eq!(after.join_policy, JoinPolicy::Public);
        assert!(after.listed);

        assert!(
            !repo
                .record_heartbeat(&a, "id-friends", "intruder", &heartbeat(NOW))
                .await
                .unwrap()
        );
        assert_eq!(
            repo.find_by_id("id-friends").await.unwrap().unwrap().last_online_at,
            None
        );
        let with_version = apply_heartbeat(t(NOW), None, Some("0.2.128".to_owned()));
        assert!(
            repo.record_heartbeat(&a, "id-friends", "host-1", &with_version)
                .await
                .unwrap()
        );
        let realm = repo.find_by_id("id-friends").await.unwrap().unwrap();
        assert_eq!(realm.last_online_at, Some(t(NOW)));
        assert_eq!(realm.online_until, Some(t("2026-10-13T12:01:30.000Z")));
        assert_eq!(realm.version.as_deref(), Some("0.2.128"));
        // A heartbeat without a version keeps the one on record
        repo.record_heartbeat(&a, "id-friends", "host-1", &heartbeat(LATER))
            .await
            .unwrap();
        let realm = repo.find_by_id("id-friends").await.unwrap().unwrap();
        assert_eq!(realm.last_online_at, Some(t(LATER)));
        assert_eq!(realm.version.as_deref(), Some("0.2.128"));
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn members_are_added_once_never_past_the_ceiling_never_when_banned() {
    each_engine!(|repo| {
        let a = actor();
        repo.insert_realm(&a, &make_realm("friends", "host-1")).await.unwrap();
        assert_eq!(
            repo.add_member(&a, "id-friends", &member("a"), 3).await.unwrap(),
            AddMemberResult::Added
        );
        assert_eq!(
            repo.add_member(&a, "id-friends", &member("a"), 3).await.unwrap(),
            AddMemberResult::AlreadyMember
        );
        assert_eq!(
            repo.add_member(&a, "id-friends", &member("host-1"), 3).await.unwrap(),
            AddMemberResult::AlreadyMember
        );
        assert_eq!(
            repo.add_member(&a, "id-friends", &member("b"), 3).await.unwrap(),
            AddMemberResult::Added
        );
        assert_eq!(
            repo.add_member(&a, "id-friends", &member("c"), 3).await.unwrap(),
            AddMemberResult::Full
        );
        assert_eq!(
            repo.add_member(&a, "missing", &member("c"), 3).await.unwrap(),
            AddMemberResult::NotFound
        );
        let realm = repo.find_by_id("id-friends").await.unwrap().unwrap();
        assert_eq!(accounts(&realm.members), ["host-1", "a", "b"]);

        assert_eq!(
            repo.add_ban(&a, "id-friends", &ban("banned", NOW), 10).await.unwrap(),
            AddBanResult::Banned
        );
        assert_eq!(
            repo.add_member(&a, "id-friends", &member("banned"), 10).await.unwrap(),
            AddMemberResult::Banned
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn racing_joins_never_fill_a_realm_past_its_ceiling() {
    each_engine!(|repo| {
        repo.insert_realm(&actor(), &make_realm("friends", "host-1"))
            .await
            .unwrap();
        let mut tasks = tokio::task::JoinSet::new();
        for index in 0..12 {
            let repo = repo.clone();
            tasks.spawn(async move {
                repo.add_member(&actor(), "id-friends", &member(&format!("joiner-{index}")), 5)
                    .await
                    .unwrap()
            });
        }
        let results = tasks.join_all().await;
        assert_eq!(results.iter().filter(|r| **r == AddMemberResult::Added).count(), 4);
        assert_eq!(results.iter().filter(|r| **r == AddMemberResult::Full).count(), 8);
        assert_eq!(repo.find_by_id("id-friends").await.unwrap().unwrap().members.len(), 5);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_double_join_by_the_same_account_adds_it_once() {
    each_engine!(|repo| {
        repo.insert_realm(&actor(), &make_realm("friends", "host-1"))
            .await
            .unwrap();
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..6 {
            let repo = repo.clone();
            tasks.spawn(async move {
                repo.add_member(&actor(), "id-friends", &member("same"), 50)
                    .await
                    .unwrap()
            });
        }
        let results = tasks.join_all().await;
        assert_eq!(results.iter().filter(|r| **r == AddMemberResult::Added).count(), 1);
        let realm = repo.find_by_id("id-friends").await.unwrap().unwrap();
        assert_eq!(realm.members.iter().filter(|m| m.account_id == "same").count(), 1);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_a_member_never_removes_the_host() {
    each_engine!(|repo| {
        let a = actor();
        repo.insert_realm(&a, &make_realm("friends", "host-1")).await.unwrap();
        repo.add_member(&a, "id-friends", &member("a"), 10).await.unwrap();
        assert!(!repo.remove_member(&a, "id-friends", "host-1").await.unwrap());
        assert!(!repo.remove_member(&a, "id-friends", "stranger").await.unwrap());
        assert!(!repo.remove_member(&a, "missing", "a").await.unwrap());
        assert!(repo.remove_member(&a, "id-friends", "a").await.unwrap());
        assert!(!repo.remove_member(&a, "id-friends", "a").await.unwrap());
        assert_eq!(
            accounts(&repo.find_by_id("id-friends").await.unwrap().unwrap().members),
            ["host-1"]
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_ban_removes_the_member_is_recorded_once_has_a_ceiling_and_can_be_lifted() {
    each_engine!(|repo| {
        let a = actor();
        repo.insert_realm(&a, &make_realm("friends", "host-1")).await.unwrap();
        repo.add_member(&a, "id-friends", &member("a"), 10).await.unwrap();
        let griefing = RealmBan {
            reason: Some("griefing".to_owned()),
            ..ban("a", NOW)
        };
        assert_eq!(
            repo.add_ban(&a, "id-friends", &griefing, 2).await.unwrap(),
            AddBanResult::Banned
        );
        assert_eq!(
            repo.add_ban(&a, "id-friends", &ban("a", LATER), 2).await.unwrap(),
            AddBanResult::AlreadyBanned
        );
        assert_eq!(
            repo.add_ban(&a, "id-friends", &ban("b", NOW), 2).await.unwrap(),
            AddBanResult::Banned
        );
        assert_eq!(
            repo.add_ban(&a, "id-friends", &ban("c", NOW), 2).await.unwrap(),
            AddBanResult::Full
        );
        assert_eq!(
            repo.add_ban(&a, "missing", &ban("c", NOW), 2).await.unwrap(),
            AddBanResult::NotFound
        );
        // Banning the host's own id leaves the host entry (the service refuses it earlier; this is the backstop)
        assert_eq!(
            repo.add_ban(&a, "id-friends", &ban("host-1", NOW), 10).await.unwrap(),
            AddBanResult::Banned
        );
        let realm = repo.find_by_id("id-friends").await.unwrap().unwrap();
        assert_eq!(accounts(&realm.members), ["host-1"]);
        assert_eq!(realm.bans[0], griefing);
        assert_eq!(realm.bans[1].reason, None);

        assert!(repo.remove_ban(&a, "id-friends", "a").await.unwrap());
        assert!(!repo.remove_ban(&a, "id-friends", "a").await.unwrap());
        assert!(!repo.remove_ban(&a, "missing", "a").await.unwrap());
        assert_eq!(
            repo.add_member(&a, "id-friends", &member("a"), 10).await.unwrap(),
            AddMemberResult::Added
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn find_by_member_and_the_listing() {
    each_engine!(|repo| {
        let a = actor();
        for (name, host, listed) in [
            ("alpha", "host-1", true),
            ("bravo", "host-2", true),
            ("charlie", "host-3", false),
            ("delta", "host-4", true),
        ] {
            let mut realm = make_realm(name, host);
            realm.listed = listed;
            repo.insert_realm(&a, &realm).await.unwrap();
        }
        repo.record_heartbeat(&a, "id-delta", "host-4", &heartbeat(NOW))
            .await
            .unwrap();
        repo.add_member(&a, "id-alpha", &member("pal"), 10).await.unwrap();
        repo.add_member(&a, "id-charlie", &member("pal"), 10).await.unwrap();

        assert_eq!(
            names(&repo.find_by_member("pal", 10).await.unwrap()),
            ["alpha", "charlie"]
        );
        assert_eq!(names(&repo.find_by_member("host-2", 10).await.unwrap()), ["bravo"]);
        assert!(repo.find_by_member("nobody", 10).await.unwrap().is_empty());
        assert_eq!(repo.find_by_member("pal", 1).await.unwrap().len(), 1);

        assert_eq!(
            names(&repo.list_listed(10, 0).await.unwrap()),
            ["delta", "alpha", "bravo"]
        );
        assert_eq!(names(&repo.list_listed(1, 1).await.unwrap()), ["alpha"]);
        assert!(repo.list_listed(10, 3).await.unwrap().is_empty());
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn name_release_deletes_only_a_realm_not_online_since_it_was_read_with_its_invites() {
    each_engine!(|repo| {
        let a = actor();
        repo.insert_realm(&a, &make_realm("never", "host-1")).await.unwrap();
        repo.insert_invite(&a, &make_invite("ABCDEFGHJK", "id-never"))
            .await
            .unwrap();
        let mut seen = make_realm("seen", "host-2");
        seen.last_online_at = Some(t(NOW));
        repo.insert_realm(&a, &seen).await.unwrap();

        // The realm came back online after the claim check read no last_online_at
        repo.record_heartbeat(&a, "id-never", "host-1", &heartbeat(LATER))
            .await
            .unwrap();
        assert!(!repo.delete_realm_if_last_online(&a, "id-never", None).await.unwrap());
        assert!(repo.find_by_id("id-never").await.unwrap().is_some());
        assert!(
            !repo
                .delete_realm_if_last_online(&a, "id-seen", Some(t(LATER)))
                .await
                .unwrap()
        );
        assert!(
            repo.delete_realm_if_last_online(&a, "id-seen", Some(t(NOW)))
                .await
                .unwrap()
        );
        assert_eq!(repo.find_by_id("id-seen").await.unwrap(), None);
        assert!(
            repo.delete_realm_if_last_online(&a, "id-never", Some(t(LATER)))
                .await
                .unwrap()
        );
        assert_eq!(
            repo.find_invite("ABCDEFGHJK").await.unwrap(),
            None,
            "invites go with the realm"
        );

        repo.insert_realm(&a, &make_realm("fresh", "host-3")).await.unwrap();
        assert!(repo.delete_realm_if_last_online(&a, "id-fresh", None).await.unwrap());
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn the_host_releases_its_own_realm_with_its_invites_nobody_else_can() {
    each_engine!(|repo| {
        let a = actor();
        repo.insert_realm(&a, &make_realm("friends", "host-1")).await.unwrap();
        repo.insert_realm(&a, &make_realm("rivals", "host-2")).await.unwrap();
        repo.insert_invite(&a, &make_invite("ABCDEFGHJK", "id-friends"))
            .await
            .unwrap();
        repo.insert_invite(&a, &make_invite("ABCDEFGHJL", "id-rivals"))
            .await
            .unwrap();
        assert!(!repo.delete_realm_of_host(&a, "id-friends", "host-2").await.unwrap());
        assert!(repo.find_by_id("id-friends").await.unwrap().is_some());
        assert!(repo.delete_realm_of_host(&a, "id-friends", "host-1").await.unwrap());
        assert!(!repo.delete_realm_of_host(&a, "id-friends", "host-1").await.unwrap());
        assert_eq!(repo.find_invite("ABCDEFGHJK").await.unwrap(), None);
        assert!(
            repo.find_invite("ABCDEFGHJL").await.unwrap().is_some(),
            "another realm's invites stay"
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn an_invite_round_trips_its_code_is_unique_and_the_list_is_newest_first() {
    each_engine!(|repo| {
        let a = actor();
        repo.insert_realm(&a, &make_realm("friends", "host-1")).await.unwrap();
        let first = make_invite("ABCDEFGHJK", "id-friends");
        let second = RealmInviteRecord {
            created_at: t(LATER),
            ..make_invite("ABCDEFGHJL", "id-friends")
        };
        assert_eq!(
            repo.insert_invite(&a, &first).await.unwrap(),
            InsertInviteResult::Inserted
        );
        assert_eq!(
            repo.insert_invite(&a, &second).await.unwrap(),
            InsertInviteResult::Inserted
        );
        assert_eq!(
            repo.insert_invite(&a, &make_invite("ABCDEFGHJK", "id-friends"))
                .await
                .unwrap(),
            InsertInviteResult::Duplicate
        );
        assert_eq!(repo.find_invite("ABCDEFGHJK").await.unwrap(), Some(first));
        assert_eq!(repo.find_invite("ZZZZZZZZZZ").await.unwrap(), None);
        let codes: Vec<String> = repo
            .list_invites("id-friends", 10)
            .await
            .unwrap()
            .into_iter()
            .map(|i| i.code)
            .collect();
        assert_eq!(codes, ["ABCDEFGHJL", "ABCDEFGHJK"]);
        assert!(repo.list_invites("other", 10).await.unwrap().is_empty());
        assert_eq!(repo.list_invites("id-friends", 1).await.unwrap().len(), 1);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn an_invite_is_consumed_until_used_up_expired_or_revoked() {
    each_engine!(|repo| {
        let a = actor();
        repo.insert_realm(&a, &make_realm("friends", "host-1")).await.unwrap();
        let invite = RealmInviteRecord {
            expires_at: t(LATER),
            ..make_invite("ABCDEFGHJK", "id-friends")
        };
        repo.insert_invite(&a, &invite).await.unwrap();
        assert_eq!(
            repo.consume_invite(&a, "ABCDEFGHJK", t(NOW))
                .await
                .unwrap()
                .unwrap()
                .uses,
            1
        );
        assert_eq!(
            repo.consume_invite(&a, "ABCDEFGHJK", t(NOW))
                .await
                .unwrap()
                .unwrap()
                .uses,
            2
        );
        assert_eq!(
            repo.consume_invite(&a, "ABCDEFGHJK", t(NOW)).await.unwrap(),
            None,
            "used up"
        );
        repo.refund_invite(&a, "ABCDEFGHJK").await.unwrap();
        assert_eq!(repo.find_invite("ABCDEFGHJK").await.unwrap().unwrap().uses, 1);
        assert_eq!(
            repo.consume_invite(&a, "ABCDEFGHJK", t(NOW))
                .await
                .unwrap()
                .unwrap()
                .uses,
            2
        );
        for _ in 0..3 {
            repo.refund_invite(&a, "ABCDEFGHJK").await.unwrap();
        }
        assert_eq!(
            repo.find_invite("ABCDEFGHJK").await.unwrap().unwrap().uses,
            0,
            "never below zero"
        );
        assert_eq!(
            repo.consume_invite(&a, "ABCDEFGHJK", t(LATER)).await.unwrap(),
            None,
            "expired at expiresAt"
        );
        assert_eq!(repo.consume_invite(&a, "ZZZZZZZZZZ", t(NOW)).await.unwrap(), None);
        repo.revoke_invite(&a, "id-friends", "ABCDEFGHJK", t(NOW))
            .await
            .unwrap();
        assert_eq!(
            repo.consume_invite(&a, "ABCDEFGHJK", t(NOW)).await.unwrap(),
            None,
            "revoked"
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn racing_joins_never_use_an_invite_more_than_its_use_count() {
    each_engine!(|repo| {
        let a = actor();
        repo.insert_realm(&a, &make_realm("friends", "host-1")).await.unwrap();
        let invite = RealmInviteRecord {
            max_uses: 3,
            ..make_invite("ABCDEFGHJK", "id-friends")
        };
        repo.insert_invite(&a, &invite).await.unwrap();
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..10 {
            let repo = repo.clone();
            tasks.spawn(async move { repo.consume_invite(&actor(), "ABCDEFGHJK", t(NOW)).await.unwrap() });
        }
        assert_eq!(tasks.join_all().await.iter().filter(|r| r.is_some()).count(), 3);
        assert_eq!(repo.find_invite("ABCDEFGHJK").await.unwrap().unwrap().uses, 3);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn revoking_marks_an_invite_once_keeps_the_first_time_and_only_for_its_realm() {
    each_engine!(|repo| {
        let a = actor();
        repo.insert_realm(&a, &make_realm("friends", "host-1")).await.unwrap();
        repo.insert_invite(&a, &make_invite("ABCDEFGHJK", "id-friends"))
            .await
            .unwrap();
        assert_eq!(
            repo.revoke_invite(&a, "other-realm", "ABCDEFGHJK", t(NOW))
                .await
                .unwrap(),
            None
        );
        assert_eq!(repo.find_invite("ABCDEFGHJK").await.unwrap().unwrap().revoked_at, None);
        let revoked = repo
            .revoke_invite(&a, "id-friends", "ABCDEFGHJK", t(NOW))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(revoked.revoked_at, Some(t(NOW)));
        let again = repo
            .revoke_invite(&a, "id-friends", "ABCDEFGHJK", t(LATER))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(again.revoked_at, Some(t(NOW)));
        assert_eq!(
            repo.revoke_invite(&a, "id-friends", "ZZZZZZZZZZ", t(NOW))
                .await
                .unwrap(),
            None
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn open_invites_are_counted_at_the_given_time() {
    each_engine!(|repo| {
        let a = actor();
        repo.insert_realm(&a, &make_realm("friends", "host-1")).await.unwrap();
        let invites = [
            make_invite("ABCDEFGHJK", "id-friends"),
            RealmInviteRecord {
                expires_at: t(LATER),
                ..make_invite("ABCDEFGHJL", "id-friends")
            },
            RealmInviteRecord {
                max_uses: 1,
                uses: 1,
                ..make_invite("ABCDEFGHJM", "id-friends")
            },
            RealmInviteRecord {
                revoked_at: Some(t(NOW)),
                ..make_invite("ABCDEFGHJN", "id-friends")
            },
            make_invite("ABCDEFGHJP", "other-realm"),
        ];
        for invite in &invites {
            repo.insert_invite(&a, invite).await.unwrap();
        }
        assert_eq!(repo.count_active_invites("id-friends", t(NOW)).await.unwrap(), 2);
        assert_eq!(repo.count_active_invites("id-friends", t(LATER)).await.unwrap(), 1);
        assert_eq!(repo.count_active_invites("nothing", t(NOW)).await.unwrap(), 0);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn patch_hosting_creates_the_record_with_defaults_merges_removes_and_leaves_the_rest() {
    each_engine!(|repo| {
        let a = actor();
        repo.insert_realm(&a, &make_realm("friends", "host-1")).await.unwrap();
        let running = HostingPatch {
            hub_state: Some(HubState::Running),
            ..HostingPatch::default()
        };
        assert!(!repo.patch_hosting(&a, "missing", &running, t(NOW)).await.unwrap());
        assert_eq!(repo.find_by_id("id-friends").await.unwrap().unwrap().hosting, None);

        let starting = HostingPatch {
            hub_state: Some(HubState::Starting),
            hub_session_id: Patch::Set("sess-1".to_owned()),
            ..HostingPatch::default()
        };
        assert!(repo.patch_hosting(&a, "id-friends", &starting, t(NOW)).await.unwrap());
        let expected = RealmHosting {
            max_servers: 1,
            max_players: 16,
            afk_seconds: None,
            hub_session_id: Some("sess-1".to_owned()),
            hub_state: HubState::Starting,
            training_state: HubState::None,
            active_servers: 0,
            updated_at: t(NOW),
        };
        assert_eq!(
            repo.find_by_id("id-friends").await.unwrap().unwrap().hosting,
            Some(expected.clone())
        );
        // The host's own fields, written separately, never undo the hub fields (and the other way round)
        let host_fields = HostingPatch {
            max_servers: Some(4),
            max_players: Some(32),
            afk_seconds: Patch::Set(600),
            ..HostingPatch::default()
        };
        let hub_fields = HostingPatch {
            hub_state: Some(HubState::Running),
            active_servers: Some(2),
            ..HostingPatch::default()
        };
        assert!(
            repo.patch_hosting(&a, "id-friends", &host_fields, t(LATER))
                .await
                .unwrap()
        );
        assert!(
            repo.patch_hosting(&a, "id-friends", &hub_fields, t(LATER))
                .await
                .unwrap()
        );
        assert_eq!(
            repo.find_by_id("id-friends").await.unwrap().unwrap().hosting,
            Some(RealmHosting {
                max_servers: 4,
                max_players: 32,
                afk_seconds: Some(600),
                hub_state: HubState::Running,
                active_servers: 2,
                updated_at: t(LATER),
                ..expected
            })
        );
        // Remove drops an optional field; Keep changes nothing (Node's unknown keys have no Rust spelling)
        let removal = HostingPatch {
            afk_seconds: Patch::Remove,
            hub_session_id: Patch::Remove,
            ..HostingPatch::default()
        };
        assert!(repo.patch_hosting(&a, "id-friends", &removal, t(FAR)).await.unwrap());
        let hosting = repo.find_by_id("id-friends").await.unwrap().unwrap().hosting.unwrap();
        assert_eq!(hosting.afk_seconds, None);
        assert_eq!(hosting.hub_session_id, None);
        assert_eq!(hosting.training_state, HubState::None);
        assert_eq!(hosting.updated_at, t(FAR));
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn racing_patches_on_disjoint_hosting_fields_all_land() {
    each_engine!(|repo| {
        repo.insert_realm(&actor(), &make_realm("friends", "host-1"))
            .await
            .unwrap();
        let patches = [
            HostingPatch {
                max_servers: Some(3),
                ..HostingPatch::default()
            },
            HostingPatch {
                max_players: Some(20),
                ..HostingPatch::default()
            },
            HostingPatch {
                hub_state: Some(HubState::Running),
                ..HostingPatch::default()
            },
            HostingPatch {
                training_state: Some(HubState::Starting),
                ..HostingPatch::default()
            },
            HostingPatch {
                active_servers: Some(2),
                ..HostingPatch::default()
            },
        ];
        let mut tasks = tokio::task::JoinSet::new();
        for patch in patches {
            let repo = repo.clone();
            tasks.spawn(async move {
                repo.patch_hosting(&actor(), "id-friends", &patch, t(NOW))
                    .await
                    .unwrap()
            });
        }
        tasks.join_all().await;
        let hosting = repo.find_by_id("id-friends").await.unwrap().unwrap().hosting.unwrap();
        assert_eq!(
            (
                hosting.max_servers,
                hosting.max_players,
                hosting.hub_state,
                hosting.training_state,
                hosting.active_servers
            ),
            (3, 20, HubState::Running, HubState::Starting, 2)
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn mark_offline_ends_a_lasting_online_state_once_and_only_for_an_online_realm() {
    each_engine!(|repo| {
        let a = actor();
        repo.insert_realm(&a, &make_realm("friends", "host-1")).await.unwrap();
        assert!(
            !repo.mark_offline(&a, "id-friends", t(NOW)).await.unwrap(),
            "never online"
        );
        assert!(!repo.mark_offline(&a, "missing", t(NOW)).await.unwrap());
        repo.record_heartbeat(&a, "id-friends", "host-1", &heartbeat(NOW))
            .await
            .unwrap();
        assert!(
            repo.mark_offline(&a, "id-friends", t("2026-10-13T12:00:10.000Z"))
                .await
                .unwrap()
        );
        let realm = repo.find_by_id("id-friends").await.unwrap().unwrap();
        assert_eq!(realm.online_until, Some(t("2026-10-13T12:00:10.000Z")));
        assert_eq!(
            realm.last_online_at,
            Some(t(NOW)),
            "lastOnlineAt is the last real heartbeat"
        );
        assert!(
            !repo
                .mark_offline(&a, "id-friends", t("2026-10-13T12:00:20.000Z"))
                .await
                .unwrap()
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn claim_lapsed_hosts_returns_each_recent_lapse_once() {
    each_engine!(|repo| {
        let a = actor();
        for (name, host) in [
            ("never", "host-0"),
            ("fresh", "host-1"),
            ("stale", "host-2"),
            ("live", "host-3"),
        ] {
            repo.insert_realm(&a, &make_realm(name, host)).await.unwrap();
        }
        repo.record_heartbeat(&a, "id-stale", "host-2", &heartbeat("2026-10-13T10:00:00.000Z"))
            .await
            .unwrap();
        repo.record_heartbeat(&a, "id-fresh", "host-1", &heartbeat("2026-10-13T11:58:00.000Z"))
            .await
            .unwrap();
        repo.record_heartbeat(&a, "id-live", "host-3", &heartbeat("2026-10-13T12:00:00.000Z"))
            .await
            .unwrap();

        let (now, since) = (t("2026-10-13T12:00:00.000Z"), t("2026-10-13T11:50:00.000Z"));
        let first = repo.claim_lapsed_hosts(&a, now, since, 10).await.unwrap();
        assert_eq!(names(&first), ["fresh"]);
        assert_eq!(first[0].offline_notified_for, Some(t("2026-10-13T11:59:30.000Z")));
        assert!(
            repo.claim_lapsed_hosts(&a, now, since, 10).await.unwrap().is_empty(),
            "a lapse is returned once"
        );

        // Two backends' sweepers racing: each lapse goes to exactly one
        repo.record_heartbeat(&a, "id-live", "host-3", &heartbeat("2026-10-13T11:00:00.000Z"))
            .await
            .unwrap();
        let mut tasks = tokio::task::JoinSet::new();
        for _ in 0..6 {
            let repo = repo.clone();
            tasks.spawn(async move {
                repo.claim_lapsed_hosts(&actor(), now, t("2026-10-13T11:00:00.000Z"), 10)
                    .await
                    .unwrap()
            });
        }
        let racers = tasks.join_all().await;
        assert_eq!(racers.iter().flatten().filter(|realm| realm.name == "live").count(), 1);

        // A new heartbeat and a new lapse is a new notice
        repo.record_heartbeat(&a, "id-fresh", "host-1", &heartbeat("2026-10-13T12:10:00.000Z"))
            .await
            .unwrap();
        let since = t("2026-10-13T12:01:00.000Z");
        assert!(
            repo.claim_lapsed_hosts(&a, t("2026-10-13T12:11:00.000Z"), since, 10)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            names(
                &repo
                    .claim_lapsed_hosts(&a, t("2026-10-13T12:12:00.000Z"), since, 10)
                    .await
                    .unwrap()
            ),
            ["fresh"]
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn claim_lapsed_hosts_honours_its_limit() {
    each_engine!(|repo| {
        let a = actor();
        for index in 1..=3 {
            repo.insert_realm(&a, &make_realm(&format!("realm-{index}"), &format!("host-{index}")))
                .await
                .unwrap();
            repo.record_heartbeat(
                &a,
                &format!("id-realm-{index}"),
                &format!("host-{index}"),
                &heartbeat("2026-10-13T11:58:00.000Z"),
            )
            .await
            .unwrap();
        }
        let (now, since) = (t("2026-10-13T12:00:00.000Z"), t("2026-10-13T11:50:00.000Z"));
        assert_eq!(repo.claim_lapsed_hosts(&a, now, since, 2).await.unwrap().len(), 2);
        assert_eq!(repo.claim_lapsed_hosts(&a, now, since, 2).await.unwrap().len(), 1);
        assert_eq!(repo.claim_lapsed_hosts(&a, now, since, 2).await.unwrap().len(), 0);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_realm_is_found_by_either_hub_room_id_by_exact_match_only() {
    each_engine!(|repo| {
        let a = actor();
        let rooms = |x: &[&str]| x.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        let friends_rooms = rooms(&[
            "11111111-1111-5111-8111-111111111111",
            "22222222-2222-5222-8222-222222222222",
        ]);
        let mut friends = make_realm("friends", "host-1");
        friends.hub_room_ids = friends_rooms.clone();
        let mut rivals = make_realm("rivals", "host-2");
        rivals.hub_room_ids = rooms(&[
            "33333333-3333-5333-8333-333333333333",
            "44444444-4444-5444-8444-444444444444",
        ]);
        repo.insert_realm(&a, &friends).await.unwrap();
        repo.insert_realm(&a, &rivals).await.unwrap();
        repo.insert_realm(&a, &make_realm("old", "host-3")).await.unwrap();
        let found = |room: &'static str| {
            let repo = repo.clone();
            async move { repo.find_by_hub_room_id(room).await.unwrap().map(|realm| realm.name) }
        };
        assert_eq!(
            found("11111111-1111-5111-8111-111111111111").await.as_deref(),
            Some("friends")
        );
        assert_eq!(
            found("22222222-2222-5222-8222-222222222222").await.as_deref(),
            Some("friends")
        );
        assert_eq!(
            found("44444444-4444-5444-8444-444444444444").await.as_deref(),
            Some("rivals")
        );
        assert_eq!(found("55555555-5555-5555-8555-555555555555").await, None);
        assert_eq!(found("").await, None);
        let realm = repo.find_by_hub_room_id(&friends_rooms[0]).await.unwrap().unwrap();
        assert_eq!(realm.hub_room_ids, friends_rooms);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn join_requests_are_recorded_once_and_refused_in_order() {
    each_engine!(|repo| {
        let a = actor();
        let mut realm = make_realm("friends", "host-1");
        realm.members.push(member("friend"));
        realm.bans.push(ban("banned", NOW));
        repo.insert_realm(&a, &realm).await.unwrap();
        let add = |account: &'static str, max| {
            let repo = repo.clone();
            async move {
                repo.add_join_request(&actor(), "id-friends", &entry(account, NOW), max)
                    .await
                    .unwrap()
            }
        };
        assert_eq!(add("stranger-1", 2).await, AddEntryResult::Added);
        assert_eq!(
            repo.add_join_request(&a, "id-friends", &entry("stranger-1", LATER), 2)
                .await
                .unwrap(),
            AddEntryResult::Already
        );
        assert_eq!(add("banned", 2).await, AddEntryResult::Banned);
        assert_eq!(add("friend", 2).await, AddEntryResult::AlreadyMember);
        assert_eq!(add("host-1", 2).await, AddEntryResult::AlreadyMember);
        assert_eq!(add("stranger-2", 2).await, AddEntryResult::Added);
        assert_eq!(add("stranger-3", 2).await, AddEntryResult::Full);
        assert_eq!(
            repo.add_join_request(&a, "id-nope", &entry("stranger-3", NOW), 2)
                .await
                .unwrap(),
            AddEntryResult::NotFound
        );
        assert_eq!(
            repo.find_by_id("id-friends").await.unwrap().unwrap().join_requests,
            [entry("stranger-1", NOW), entry("stranger-2", NOW)]
        );
        assert!(repo.remove_join_request(&a, "id-friends", "stranger-1").await.unwrap());
        assert!(!repo.remove_join_request(&a, "id-friends", "stranger-1").await.unwrap());
        assert_eq!(
            repo.find_by_id("id-friends").await.unwrap().unwrap().join_requests,
            [entry("stranger-2", NOW)]
        );
        assert_eq!(
            add("stranger-3", 2).await,
            AddEntryResult::Added,
            "a withdrawn request frees its place"
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_ban_withdraws_the_banned_accounts_join_request_and_leaves_the_others() {
    each_engine!(|repo| {
        let a = actor();
        repo.insert_realm(&a, &make_realm("friends", "host-1")).await.unwrap();
        repo.add_join_request(&a, "id-friends", &entry("stranger", NOW), 10)
            .await
            .unwrap();
        repo.add_join_request(&a, "id-friends", &entry("other", NOW), 10)
            .await
            .unwrap();
        assert_eq!(
            repo.add_ban(&a, "id-friends", &ban("stranger", LATER), 10)
                .await
                .unwrap(),
            AddBanResult::Banned
        );
        assert_eq!(
            repo.find_by_id("id-friends").await.unwrap().unwrap().join_requests,
            [entry("other", NOW)]
        );
        assert!(!repo.remove_join_request(&a, "id-friends", "stranger").await.unwrap());
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn racing_join_requests_never_exceed_the_bound() {
    each_engine!(|repo| {
        repo.insert_realm(&actor(), &make_realm("friends", "host-1"))
            .await
            .unwrap();
        let mut tasks = tokio::task::JoinSet::new();
        for index in 0..6 {
            let repo = repo.clone();
            tasks.spawn(async move {
                let request = entry(&format!("stranger-{index}"), NOW);
                repo.add_join_request(&actor(), "id-friends", &request, 3)
                    .await
                    .unwrap()
            });
        }
        let results = tasks.join_all().await;
        assert_eq!(results.iter().filter(|r| **r == AddEntryResult::Added).count(), 3);
        assert_eq!(results.iter().filter(|r| **r == AddEntryResult::Full).count(), 3);
        assert_eq!(
            repo.find_by_id("id-friends")
                .await
                .unwrap()
                .unwrap()
                .join_requests
                .len(),
            3
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn player_invites_are_recorded_once_and_refused_in_order() {
    each_engine!(|repo| {
        let a = actor();
        let mut realm = make_realm("friends", "host-1");
        realm.members.push(member("friend"));
        realm.bans.push(ban("banned", NOW));
        repo.insert_realm(&a, &realm).await.unwrap();
        let add = |account: &'static str| {
            let repo = repo.clone();
            async move {
                repo.add_player_invite(&actor(), "id-friends", &entry(account, NOW), 2)
                    .await
                    .unwrap()
            }
        };
        assert_eq!(add("guest-1").await, AddEntryResult::Added);
        assert_eq!(
            repo.add_player_invite(&a, "id-friends", &entry("guest-1", LATER), 2)
                .await
                .unwrap(),
            AddEntryResult::Already
        );
        assert_eq!(add("banned").await, AddEntryResult::Banned);
        assert_eq!(add("friend").await, AddEntryResult::AlreadyMember);
        assert_eq!(add("guest-2").await, AddEntryResult::Added);
        assert_eq!(add("guest-3").await, AddEntryResult::Full);
        assert_eq!(
            repo.add_player_invite(&a, "id-nope", &entry("guest-3", NOW), 2)
                .await
                .unwrap(),
            AddEntryResult::NotFound
        );
        assert_eq!(
            repo.find_by_id("id-friends").await.unwrap().unwrap().player_invites,
            [entry("guest-1", NOW), entry("guest-2", NOW)]
        );
        assert!(repo.remove_player_invite(&a, "id-friends", "guest-1").await.unwrap());
        assert!(!repo.remove_player_invite(&a, "id-friends", "guest-1").await.unwrap());
        assert_eq!(
            add("guest-3").await,
            AddEntryResult::Added,
            "a withdrawn invite frees its place"
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_membership_and_a_ban_each_remove_the_player_invite() {
    each_engine!(|repo| {
        let a = actor();
        repo.insert_realm(&a, &make_realm("friends", "host-1")).await.unwrap();
        repo.insert_realm(&a, &make_realm("embers", "host-2")).await.unwrap();
        repo.add_player_invite(&a, "id-friends", &entry("guest", NOW), 10)
            .await
            .unwrap();
        repo.add_player_invite(&a, "id-embers", &entry("guest", NOW), 10)
            .await
            .unwrap();
        repo.add_player_invite(&a, "id-friends", &entry("other", NOW), 10)
            .await
            .unwrap();
        assert_eq!(
            names(&repo.find_by_player_invite("guest", 10).await.unwrap()),
            ["embers", "friends"]
        );

        let joined = member_at("guest", MemberRole::Member, LATER);
        assert_eq!(
            repo.add_member(&a, "id-friends", &joined, 10).await.unwrap(),
            AddMemberResult::Added
        );
        assert_eq!(
            repo.find_by_id("id-friends").await.unwrap().unwrap().player_invites,
            [entry("other", NOW)]
        );
        assert_eq!(
            repo.add_ban(&a, "id-embers", &ban("guest", LATER), 10).await.unwrap(),
            AddBanResult::Banned
        );
        assert!(
            repo.find_by_id("id-embers")
                .await
                .unwrap()
                .unwrap()
                .player_invites
                .is_empty()
        );
        assert!(repo.find_by_player_invite("guest", 10).await.unwrap().is_empty());
    });
}

// ------------------------------------------------------------------ realmPresenceRepositoryContract.ts

const T0: &str = "2026-10-13T12:00:00.000Z";
const T60: &str = "2026-10-13T12:01:00.000Z";
const T120: &str = "2026-10-13T12:02:00.000Z";
const T180: &str = "2026-10-13T12:03:00.000Z";

fn row(account: &str, realm: &str) -> RealmPresenceRecord {
    RealmPresenceRecord {
        account_id: account.to_owned(),
        realm_id: realm.to_owned(),
        bound_at: t(T0),
        last_seen_at: t(T0),
        expires_at: t(T120),
    }
}

macro_rules! each_presence {
    (|$repo:ident| $body:block) => {
        for target in targets() {
            let $repo = RealmPresenceRepository::new(target.open());
            eprintln!("engine: {}", target.name());
            $body
        }
    };
}

#[tokio::test(flavor = "multi_thread")]
async fn a_presence_row_round_trips_and_a_bind_replaces_the_earlier_row() {
    each_presence!(|repo| {
        let a = actor();
        assert_eq!(repo.find_active("a", t(T0)).await.unwrap(), None);
        repo.bind(&a, &row("a", "realm-1")).await.unwrap();
        assert_eq!(repo.find_active("a", t(T0)).await.unwrap(), Some(row("a", "realm-1")));
        let later = RealmPresenceRecord {
            bound_at: t(T60),
            last_seen_at: t(T60),
            expires_at: t(T180),
            ..row("a", "realm-2")
        };
        repo.bind(&a, &later).await.unwrap();
        assert_eq!(repo.find_active("a", t(T60)).await.unwrap(), Some(later));
        assert_eq!(repo.find_active("b", t(T0)).await.unwrap(), None);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_presence_row_is_active_only_strictly_before_its_expiry() {
    each_presence!(|repo| {
        repo.bind(&actor(), &row("a", "realm-1")).await.unwrap();
        assert!(
            repo.find_active("a", t("2026-10-13T12:01:59.999Z"))
                .await
                .unwrap()
                .is_some()
        );
        assert_eq!(repo.find_active("a", t(T120)).await.unwrap(), None);
        assert_eq!(repo.find_active("a", t(T180)).await.unwrap(), None);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_touch_extends_an_active_row_and_never_brings_one_back() {
    each_presence!(|repo| {
        let a = actor();
        repo.bind(&a, &row("a", "realm-1")).await.unwrap();
        let touched = repo.touch(&a, "a", t(T60), t(T180)).await.unwrap();
        let expected = RealmPresenceRecord {
            last_seen_at: t(T60),
            expires_at: t(T180),
            ..row("a", "realm-1")
        };
        assert_eq!(touched, Some(expected.clone()));
        assert_eq!(repo.find_active("a", t(T120)).await.unwrap(), Some(expected));
        assert_eq!(
            repo.touch(&a, "a", t(T180), t("2026-10-13T12:05:00.000Z"))
                .await
                .unwrap(),
            None
        );
        assert_eq!(repo.find_active("a", t(T180)).await.unwrap(), None);
        assert_eq!(repo.touch(&a, "nobody", t(T0), t(T120)).await.unwrap(), None);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn remove_deletes_the_row_and_with_a_realm_only_that_realms() {
    each_presence!(|repo| {
        let a = actor();
        repo.bind(&a, &row("a", "realm-1")).await.unwrap();
        repo.bind(&a, &row("b", "realm-2")).await.unwrap();
        assert!(
            !repo.remove(&a, "a", Some("realm-2")).await.unwrap(),
            "bound to another realm"
        );
        assert!(repo.find_active("a", t(T0)).await.unwrap().is_some());
        assert!(repo.remove(&a, "a", Some("realm-1")).await.unwrap());
        assert!(!repo.remove(&a, "a", None).await.unwrap());
        assert!(repo.remove(&a, "b", None).await.unwrap());
        assert!(!repo.remove(&a, "nobody", None).await.unwrap());
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn remove_for_realm_deletes_every_row_of_that_realm_and_no_other() {
    each_presence!(|repo| {
        let a = actor();
        for (account, realm) in [("a", "realm-1"), ("b", "realm-1"), ("c", "realm-2")] {
            repo.bind(&a, &row(account, realm)).await.unwrap();
        }
        assert_eq!(repo.remove_for_realm(&a, "realm-1").await.unwrap(), 2);
        assert_eq!(repo.remove_for_realm(&a, "realm-1").await.unwrap(), 0);
        assert_eq!(repo.find_active("a", t(T0)).await.unwrap(), None);
        assert!(repo.find_active("c", t(T0)).await.unwrap().is_some());
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn the_active_rows_of_each_realm_are_counted_at_the_given_time() {
    each_presence!(|repo| {
        let a = actor();
        repo.bind(&a, &row("a", "realm-1")).await.unwrap();
        let short = RealmPresenceRecord {
            expires_at: t(T60),
            ..row("b", "realm-1")
        };
        repo.bind(&a, &short).await.unwrap();
        repo.bind(&a, &row("c", "realm-2")).await.unwrap();
        repo.bind(&a, &row("d", "realm-3")).await.unwrap();
        let ids = |x: &[&str]| x.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        let counts = repo
            .count_active_by_realm(&ids(&["realm-1", "realm-2", "none"]), t(T0))
            .await
            .unwrap();
        assert_eq!(
            counts.into_iter().collect::<Vec<_>>(),
            [("realm-1".to_owned(), 2), ("realm-2".to_owned(), 1)]
        );
        let counts = repo.count_active_by_realm(&ids(&["realm-1"]), t(T60)).await.unwrap();
        assert_eq!(
            counts.into_iter().collect::<Vec<_>>(),
            [("realm-1".to_owned(), 1)],
            "b expired at T60"
        );
        assert!(repo.count_active_by_realm(&[], t(T0)).await.unwrap().is_empty());
        assert!(
            repo.count_active_by_realm(&ids(&["realm-1"]), t(T120))
                .await
                .unwrap()
                .is_empty()
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn racing_binds_for_one_account_leave_exactly_one_row() {
    each_presence!(|repo| {
        let mut tasks = tokio::task::JoinSet::new();
        for index in 0..8 {
            let repo = repo.clone();
            tasks.spawn(async move { repo.bind(&actor(), &row("a", &format!("realm-{index}"))).await.unwrap() });
        }
        tasks.join_all().await;
        assert!(repo.find_active("a", t(T0)).await.unwrap().is_some());
        let ids: Vec<String> = (0..8).map(|index| format!("realm-{index}")).collect();
        assert_eq!(repo.count_active_by_realm(&ids, t(T0)).await.unwrap().len(), 1);
    });
}
