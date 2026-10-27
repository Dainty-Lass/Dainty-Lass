//! `realms/realmsService.test.ts` on both engines: claiming, lookups, joining, moderation, settings, invites,
//! heartbeat and release, join requests, usernames and player invites. Node's in-memory repository double is the real
//! database here, so its internals are read through the repositories.
#![allow(clippy::unwrap_used)]

mod common;

use std::sync::{Arc, Mutex};

use common::kit::*;
use common::targets;
use rifts_realms::input::{HeartbeatInput, InviteInput, JoinInput, SettingsInput};
use rifts_realms::realm::{JoinPolicy, RealmLimits};
use rifts_realms::time::iso;

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

fn limits(
    max_members: u32,
    max_bans: u32,
    max_active_invites: u32,
    max_join_requests: u32,
    max_player_invites: u32,
) -> RealmLimits {
    RealmLimits {
        max_members,
        max_bans,
        max_active_invites,
        max_join_requests,
        max_player_invites,
    }
}

fn join_name(name: &str) -> JoinInput {
    JoinInput {
        name: Some(name.to_owned()),
        ..JoinInput::default()
    }
}

fn join_code(code: &str) -> JoinInput {
    JoinInput {
        invite_code: Some(code.to_owned()),
        ..JoinInput::default()
    }
}

fn invite(max_uses: i64, ttl_hours: i64) -> InviteInput {
    InviteInput { max_uses, ttl_hours }
}

fn heartbeat() -> HeartbeatInput {
    HeartbeatInput::default()
}

// ------------------------------------------------------------------ claiming a name

#[tokio::test(flavor = "multi_thread")]
async fn a_new_realm_is_invite_only_and_unlisted_with_its_host_as_first_member_offline() {
    each!(|k| {
        let realm = k.create("host-1", "friends").await;
        assert_eq!(realm.info.name, "friends");
        assert_eq!(realm.info.join_policy, "INVITE_ONLY");
        assert!(!realm.info.listed && !realm.info.online);
        assert_eq!(realm.info.member_count, 1);
        assert_eq!(realm.info.membership, Some("HOST"));
        assert_eq!(realm.info.host.display_name.as_deref(), Some("Hosty"));
        assert_eq!(realm.info.host_public_key, KEY_A);
        assert_eq!(realm.members.len(), 1);
        let host = &realm.members[0];
        assert_eq!(
            (
                host.account_id.as_str(),
                host.display_name.as_deref(),
                host.role,
                host.joined_at.as_str()
            ),
            ("host-1", Some("Hosty"), "HOST", "2026-10-13T12:00:00.000Z")
        );
        assert_eq!(realm.name_release_at.as_deref(), Some(iso(at(3 * DAY)).as_str()));
        let mine = k.dir.get_mine("host-1").await.unwrap();
        assert_eq!(mine.hosted.unwrap().info.realm_id, realm.info.realm_id);
        assert!(mine.member_of.is_empty());
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_taken_name_and_a_second_realm_for_the_same_host_are_refused() {
    each!(|k| {
        k.create("host-1", "friends").await;
        rejects(k.try_create("host-2", "friends", |_| {}).await, "REALM_NAME_TAKEN");
        rejects(
            k.try_create("host-1", "other-name", |_| {}).await,
            "REALM_ALREADY_EXISTS",
        );
        assert!(k.repo.find_by_name("other-name").await.unwrap().is_none());
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_name_never_online_is_released_3_days_after_the_claim_and_the_new_claim_replaces_the_old_realm() {
    each!(|k| {
        let first = k.create("host-1", "friends").await;
        let invite = k.dir.create_invite("host-1", &invite(5, 24)).await.unwrap().invite;
        k.dir.join("friend-1", &join_code(&invite.code)).await.unwrap();

        k.set_now(3 * DAY - 1);
        rejects(
            k.try_create("host-2", "friends", |i| i.host_public_key = KEY_B.to_owned())
                .await,
            "REALM_NAME_TAKEN",
        );
        assert!(k.repo.find_by_id(&first.info.realm_id).await.unwrap().is_some());

        k.set_now(3 * DAY);
        let second = k
            .create_with("host-2", "friends", |i| i.host_public_key = KEY_B.to_owned())
            .await;
        assert_ne!(second.info.realm_id, first.info.realm_id);
        assert_eq!(second.info.host_public_key, KEY_B);
        assert!(k.repo.find_by_id(&first.info.realm_id).await.unwrap().is_none());
        assert!(
            k.repo.find_invite(&invite.code).await.unwrap().is_none(),
            "the old realm's invites go with it"
        );
        // The old host is free to claim again, and the old member is not in the new realm
        assert_eq!(k.dir.get_mine("friend-1").await.unwrap().member_of.len(), 0);
        k.create("host-1", "my-new-name").await;
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_realm_that_was_online_keeps_its_name_for_30_days_after_the_last_heartbeat() {
    each!(|k| {
        k.create("host-1", "friends").await;
        k.set_now(2 * DAY);
        k.dir.heartbeat("host-1", &heartbeat()).await.unwrap();
        for offset in [2 * DAY + 29 * DAY, 2 * DAY + 30 * DAY - 1] {
            k.set_now(offset);
            rejects(k.try_create("host-2", "friends", |_| {}).await, "REALM_NAME_TAKEN");
        }
        k.set_now(2 * DAY + 30 * DAY);
        k.create("host-2", "friends").await;
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_realms_name_is_held_while_it_is_online_however_old_the_claim() {
    each!(|k| {
        k.create("host-1", "friends").await;
        for day in (0..40).step_by(10) {
            k.set_now(day * DAY);
            k.dir.heartbeat("host-1", &heartbeat()).await.unwrap();
        }
        k.set_now(41 * DAY);
        rejects(k.try_create("host-2", "friends", |_| {}).await, "REALM_NAME_TAKEN");
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_who_already_has_a_realm_cannot_take_a_dormant_name() {
    each!(|k| {
        let dormant = k.create("host-1", "friends").await;
        k.create("host-3", "mine-already").await;
        k.set_now(10 * DAY);
        rejects(k.try_create("host-3", "friends", |_| {}).await, "REALM_ALREADY_EXISTS");
        assert!(
            k.repo.find_by_id(&dormant.info.realm_id).await.unwrap().is_some(),
            "nothing was released"
        );
        assert!(k.repo.find_by_name("mine-already").await.unwrap().is_some());
    });
}

// ------------------------------------------------------------------ directory lookups

#[tokio::test(flavor = "multi_thread")]
async fn anyone_signed_in_can_look_a_realm_up_by_name_and_gets_public_information_only() {
    each!(|k| {
        k.create_with("host-1", "friends", |i| {
            i.join_policy = JoinPolicy::Public;
            i.listed = true;
            i.version = Some("0.2.128".to_owned());
        })
        .await;
        let realm = k.dir.get_by_name("stranger", "friends", None).await.unwrap().realm;
        let json = serde_json::to_value(&realm).unwrap();
        let mut keys: Vec<&str> = json.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "host",
                "hostPublicKey",
                "hubReady",
                "invited",
                "joinPolicy",
                "listed",
                "memberCount",
                "membership",
                "name",
                "online",
                "playerCount",
                "realmId",
                "requested",
                "serving",
                "version"
            ]
        );
        assert_eq!(json["host"], serde_json::json!({ "displayName": "Hosty" }));
        assert_eq!(realm.version.as_deref(), Some("0.2.128"));
        assert_eq!(realm.membership, None);
        assert!(!realm.online);
        // No address of any kind: no such field, and no IPv4 text in any value
        let text = json.to_string();
        let ipv4 = text
            .split(|c: char| !(c.is_ascii_digit() || c == '.'))
            .any(|w| w.split('.').count() == 4 && w.split('.').all(|p| (1..=3).contains(&p.len())));
        assert!(!ipv4, "{text}");
        rejects(
            k.dir.get_by_name("stranger", "nothing-here", None).await,
            "REALM_NOT_FOUND",
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn the_ownership_notice_tells_a_launcher_with_the_old_key_that_the_name_has_a_new_host() {
    each!(|k| {
        k.create("host-1", "friends").await;
        let seen = |key: Option<&'static str>| {
            let dir = &k.dir;
            async move {
                dir.get_by_name("friend-1", "friends", key)
                    .await
                    .unwrap()
                    .realm
                    .ownership_changed
            }
        };
        assert_eq!(seen(None).await, None);
        assert_eq!(seen(Some(KEY_A)).await, Some(false));
        assert_eq!(seen(Some(KEY_B)).await, Some(true));
        k.set_now(3 * DAY);
        k.create_with("host-2", "friends", |i| i.host_public_key = KEY_B.to_owned())
            .await;
        assert_eq!(seen(Some(KEY_A)).await, Some(true));
        assert_eq!(seen(Some(KEY_B)).await, Some(false));
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn only_listed_realms_appear_in_the_listing_online_ones_first() {
    each!(|k| {
        for (host, name, listed) in [
            ("host-1", "alpha", true),
            ("host-2", "bravo", true),
            ("host-3", "charlie", false),
            ("host-4", "delta", true),
        ] {
            k.create_with(host, name, |i| i.listed = listed).await;
        }
        k.set_now(1000);
        k.dir.heartbeat("host-4", &heartbeat()).await.unwrap();
        let listing = k.dir.list_listed("friend-1", 10, 0).await.unwrap();
        let names: Vec<&str> = listing.realms.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["delta", "alpha", "bravo"]);
        assert!(listing.realms[0].online);
        let page = k.dir.list_listed("friend-1", 1, 1).await.unwrap();
        assert_eq!(
            page.realms.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["alpha"]
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failing_display_name_lookup_does_not_fail_the_directory() {
    each!(|k, o| o.accounts = Arc::new(FailingNames); {
        let realm = k.create("host-1", "friends").await;
        assert_eq!(realm.info.host.display_name, None);
        let seen = k.dir.get_by_name("friend-1", "friends", None).await.unwrap();
        assert_eq!(seen.realm.host.display_name, None);
    });
}

// ------------------------------------------------------------------ joining

#[tokio::test(flavor = "multi_thread")]
async fn a_public_realm_admits_anyone_once() {
    each!(|k| {
        k.create_with("host-1", "open-house", |i| i.join_policy = JoinPolicy::Public)
            .await;
        let first = k.dir.join("friend-1", &join_name("open-house")).await.unwrap();
        assert!(first.joined);
        assert_eq!((first.realm.membership, first.realm.member_count), (Some("MEMBER"), 2));
        let again = k.dir.join("friend-1", &join_name("open-house")).await.unwrap();
        assert!(!again.joined);
        assert_eq!(again.realm.member_count, 2);
        let mine = k.dir.get_mine("friend-1").await.unwrap();
        assert_eq!(
            mine.member_of.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["open-house"]
        );
        assert!(mine.hosted.is_none());
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn an_invite_only_realm_needs_an_invite_each_join_uses_one() {
    each!(|k| {
        k.create("host-1", "friends").await;
        rejects(
            k.dir.join("friend-1", &join_name("friends")).await,
            "REALM_JOIN_REQUESTED",
        );
        let made = k.dir.create_invite("host-1", &invite(2, 24)).await.unwrap().invite;
        assert_eq!((made.uses, made.state), (0, "USABLE"));

        let joined = k.dir.join("friend-1", &join_code(&made.code)).await.unwrap();
        assert!(joined.joined);
        assert_eq!(joined.realm.name, "friends");
        assert_eq!(k.repo.find_invite(&made.code).await.unwrap().unwrap().uses, 1);
        // Joining again as a member uses nothing
        assert!(!k.dir.join("friend-1", &join_code(&made.code)).await.unwrap().joined);
        assert_eq!(k.repo.find_invite(&made.code).await.unwrap().unwrap().uses, 1);

        let both = JoinInput {
            name: Some("friends".to_owned()),
            invite_code: Some(made.code.clone()),
            known_host_key: None,
        };
        assert!(k.dir.join("friend-2", &both).await.unwrap().joined);
        assert_eq!(k.repo.find_invite(&made.code).await.unwrap().unwrap().uses, 2);
        rejects(
            k.dir.join("friend-3", &join_code(&made.code)).await,
            "REALM_INVITE_EXPIRED",
        );
        assert_eq!(
            k.dir
                .get_by_name("host-1", "friends", None)
                .await
                .unwrap()
                .realm
                .member_count,
            3
        );

        let second = k.dir.create_invite("host-1", &invite(5, 1)).await.unwrap().invite;
        k.set_now(3_600_000);
        rejects(
            k.dir.join("friend-3", &join_code(&second.code)).await,
            "REALM_INVITE_EXPIRED",
        );

        k.set_now(0);
        let third = k.dir.create_invite("host-1", &invite(5, 24)).await.unwrap().invite;
        k.dir.revoke_invite("host-1", &third.code).await.unwrap();
        rejects(
            k.dir.join("friend-3", &join_code(&third.code)).await,
            "REALM_INVITE_EXPIRED",
        );
        rejects(
            k.dir.join("friend-3", &join_code("ZZZZZZZZZZ")).await,
            "REALM_INVITE_INVALID",
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn an_invite_cannot_be_used_for_another_realms_name() {
    each!(|k| {
        k.create("host-1", "friends").await;
        k.create("host-2", "rivals").await;
        let made = k.dir.create_invite("host-1", &invite(5, 24)).await.unwrap().invite;
        let wrong = JoinInput {
            name: Some("rivals".to_owned()),
            invite_code: Some(made.code),
            known_host_key: None,
        };
        rejects(k.dir.join("friend-1", &wrong).await, "REALM_INVITE_INVALID");
        rejects(
            k.dir.join("friend-1", &join_name("no-such-realm")).await,
            "REALM_NOT_FOUND",
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_banned_account_is_refused_before_any_invite_is_used() {
    each!(|k| {
        k.create("host-1", "friends").await;
        let made = k.dir.create_invite("host-1", &invite(5, 24)).await.unwrap().invite;
        k.dir.join("friend-1", &join_code(&made.code)).await.unwrap();
        k.dir.ban("host-1", "friend-1", Some("griefing")).await.unwrap();

        rejects(k.dir.join("friend-1", &join_code(&made.code)).await, "REALM_BANNED");
        rejects(k.dir.preview_invite("friend-1", &made.code).await, "REALM_BANNED");
        assert_eq!(
            k.repo.find_invite(&made.code).await.unwrap().unwrap().uses,
            1,
            "the refused join used nothing"
        );

        let public = SettingsInput {
            join_policy: Some(JoinPolicy::Public),
            listed: None,
        };
        k.dir.update_settings("host-1", &public).await.unwrap();
        rejects(k.dir.join("friend-1", &join_name("friends")).await, "REALM_BANNED");
        assert!(k.dir.join("friend-2", &join_name("friends")).await.unwrap().joined);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_full_realm_refuses_the_join_and_gives_the_invite_use_back() {
    each!(|k, o| o.limits = limits(2, 5, 25, 50, 50); {
        k.create("host-1", "friends").await;
        let made = k.dir.create_invite("host-1", &invite(5, 24)).await.unwrap().invite;
        k.dir.join("friend-1", &join_code(&made.code)).await.unwrap();
        rejects(k.dir.join("friend-2", &join_code(&made.code)).await, "REALM_FULL");
        assert_eq!(k.repo.find_invite(&made.code).await.unwrap().unwrap().uses, 1, "the failed join's use was refunded");
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_launcher_that_remembers_an_old_host_key_is_told_about_a_new_owner() {
    each!(|k| {
        k.create_with("host-1", "open-house", |i| i.join_policy = JoinPolicy::Public)
            .await;
        k.set_now(3 * DAY);
        k.create_with("host-2", "open-house", |i| {
            i.join_policy = JoinPolicy::Public;
            i.host_public_key = KEY_B.to_owned();
        })
        .await;
        let with_key = |key: &str| JoinInput {
            name: Some("open-house".to_owned()),
            invite_code: None,
            known_host_key: Some(key.to_owned()),
        };
        rejects(k.dir.join("friend-1", &with_key(KEY_A)).await, "REALM_OWNER_CHANGED");
        assert!(k.dir.join("friend-1", &with_key(KEY_B)).await.unwrap().joined);
        assert!(k.dir.join("friend-2", &join_name("open-house")).await.unwrap().joined);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn an_invite_can_be_previewed_without_using_it() {
    each!(|k| {
        k.create_with("host-1", "friends", |i| i.version = Some("0.2.128".to_owned()))
            .await;
        let made = k.dir.create_invite("host-1", &invite(1, 24)).await.unwrap().invite;
        let preview = k.dir.preview_invite("friend-1", &made.code).await.unwrap();
        assert_eq!(
            (preview.realm.name.as_str(), preview.realm.membership),
            ("friends", None)
        );
        assert_eq!(k.repo.find_invite(&made.code).await.unwrap().unwrap().uses, 0);
        rejects(
            k.dir.preview_invite("friend-1", "ZZZZZZZZZZ").await,
            "REALM_INVITE_INVALID",
        );
        k.dir.revoke_invite("host-1", &made.code).await.unwrap();
        rejects(
            k.dir.preview_invite("friend-1", &made.code).await,
            "REALM_INVITE_EXPIRED",
        );
    });
}

// ------------------------------------------------------------------ leaving, kicking, banning

#[tokio::test(flavor = "multi_thread")]
async fn a_member_can_leave_the_host_cannot_and_a_stranger_is_not_a_member() {
    each!(|k| {
        k.create_with("host-1", "open-house", |i| i.join_policy = JoinPolicy::Public)
            .await;
        k.dir.join("friend-1", &join_name("open-house")).await.unwrap();
        rejects(k.dir.leave("friend-2", "open-house").await, "REALM_NOT_MEMBER");
        rejects(k.dir.leave("host-1", "open-house").await, "REALM_HOST_CANNOT_LEAVE");
        rejects(k.dir.leave("friend-1", "nothing-here").await, "REALM_NOT_FOUND");
        assert!(k.dir.leave("friend-1", "open-house").await.unwrap().left);
        assert_eq!(
            k.dir
                .get_by_name("host-1", "open-house", None)
                .await
                .unwrap()
                .realm
                .member_count,
            1
        );
        rejects(k.dir.leave("friend-1", "open-house").await, "REALM_NOT_MEMBER");
        // Leaving is not a ban: a public realm takes them back
        assert!(k.dir.join("friend-1", &join_name("open-house")).await.unwrap().joined);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn the_host_can_kick_a_member_who_may_come_back_but_not_themselves_or_a_stranger() {
    each!(|k| {
        k.create_with("host-1", "open-house", |i| i.join_policy = JoinPolicy::Public)
            .await;
        k.dir.join("friend-1", &join_name("open-house")).await.unwrap();
        rejects(k.dir.kick("host-1", "host-1").await, "REALM_INVALID_TARGET");
        rejects(k.dir.kick("host-1", "friend-2").await, "REALM_NOT_MEMBER");
        rejects(k.dir.kick("friend-1", "friend-2").await, "REALM_NOT_FOUND");
        let kicked = k.dir.kick("host-1", "friend-1").await.unwrap();
        assert_eq!((kicked.account_id.as_str(), kicked.status), ("friend-1", "KICKED"));
        assert_eq!(
            k.dir
                .get_by_name("host-1", "open-house", None)
                .await
                .unwrap()
                .realm
                .member_count,
            1
        );
        assert!(k.dir.join("friend-1", &join_name("open-house")).await.unwrap().joined);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_ban_removes_the_member_keeps_the_account_out_lists_with_its_reason_and_can_be_lifted() {
    each!(|k| {
        k.create_with("host-1", "open-house", |i| i.join_policy = JoinPolicy::Public)
            .await;
        k.dir.join("friend-1", &join_name("open-house")).await.unwrap();
        rejects(k.dir.ban("host-1", "host-1", None).await, "REALM_INVALID_TARGET");
        let first = k.dir.ban("host-1", "friend-1", Some("griefing")).await.unwrap();
        assert_eq!((first.status, first.changed), ("BANNED", true));
        assert!(!k.dir.ban("host-1", "friend-1", None).await.unwrap().changed);
        // A stranger can be banned before they ever joined
        assert!(k.dir.ban("host-1", "friend-2", None).await.unwrap().changed);

        let view = k.dir.get_mine("host-1").await.unwrap().hosted.unwrap();
        assert_eq!(view.info.member_count, 1);
        let bans: Vec<(&str, Option<&str>, Option<&str>)> = view
            .bans
            .iter()
            .map(|b| (b.account_id.as_str(), b.display_name.as_deref(), b.reason.as_deref()))
            .collect();
        assert_eq!(
            bans,
            [
                ("friend-1", Some("Friendly"), Some("griefing")),
                ("friend-2", Some("Pal"), None)
            ]
        );
        rejects(k.dir.join("friend-1", &join_name("open-house")).await, "REALM_BANNED");

        let lifted = k.dir.unban("host-1", "friend-1").await.unwrap();
        assert_eq!((lifted.status, lifted.changed), ("UNBANNED", true));
        assert!(!k.dir.unban("host-1", "friend-1").await.unwrap().changed);
        assert!(k.dir.join("friend-1", &join_name("open-house")).await.unwrap().joined);
        rejects(k.dir.join("friend-2", &join_name("open-house")).await, "REALM_BANNED");
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn host_only_actions_answer_not_found_to_an_account_that_hosts_no_realm() {
    each!(|k| {
        k.create("host-1", "friends").await;
        rejects(
            k.dir
                .update_settings(
                    "friend-1",
                    &SettingsInput {
                        join_policy: None,
                        listed: Some(true),
                    },
                )
                .await,
            "REALM_NOT_FOUND",
        );
        rejects(k.dir.create_invite("friend-1", &invite(1, 1)).await, "REALM_NOT_FOUND");
        rejects(k.dir.list_invites("friend-1").await, "REALM_NOT_FOUND");
        rejects(k.dir.revoke_invite("friend-1", "ABCDEFGHJK").await, "REALM_NOT_FOUND");
        rejects(k.dir.kick("friend-1", "host-1").await, "REALM_NOT_FOUND");
        rejects(k.dir.ban("friend-1", "host-1", None).await, "REALM_NOT_FOUND");
        rejects(k.dir.unban("friend-1", "host-1").await, "REALM_NOT_FOUND");
        rejects(k.dir.heartbeat("friend-1", &heartbeat()).await, "REALM_NOT_FOUND");
        rejects(k.dir.release("friend-1").await, "REALM_NOT_FOUND");
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn the_ban_list_has_a_ceiling() {
    each!(|k, o| o.limits = limits(10, 2, 25, 50, 50); {
        k.create("host-1", "friends").await;
        k.dir.ban("host-1", "a1", None).await.unwrap();
        k.dir.ban("host-1", "a2", None).await.unwrap();
        rejects(k.dir.ban("host-1", "a3", None).await, "REALM_FULL");
        assert!(!k.dir.ban("host-1", "a2", None).await.unwrap().changed);
    });
}

// ------------------------------------------------------------------ settings and invites

#[tokio::test(flavor = "multi_thread")]
async fn the_host_changes_the_join_policy_and_listing() {
    each!(|k| {
        k.create("host-1", "friends").await;
        let both = SettingsInput {
            join_policy: Some(JoinPolicy::Public),
            listed: Some(true),
        };
        let realm = k.dir.update_settings("host-1", &both).await.unwrap().realm;
        assert_eq!((realm.info.join_policy, realm.info.listed), ("PUBLIC", true));
        let unlist = SettingsInput {
            join_policy: None,
            listed: Some(false),
        };
        assert_eq!(
            k.dir
                .update_settings("host-1", &unlist)
                .await
                .unwrap()
                .realm
                .info
                .join_policy,
            "PUBLIC"
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn invites_are_created_with_the_requested_use_count_and_lifetime_listed_and_revoked_once() {
    each!(|k| {
        k.create("host-1", "friends").await;
        let made = k.dir.create_invite("host-1", &invite(3, 2)).await.unwrap().invite;
        assert!(
            made.code.len() == 10
                && made
                    .code
                    .bytes()
                    .all(|b| rifts_realms::realm::INVITE_ALPHABET.contains(&b))
        );
        assert_eq!(made.max_uses, 3);
        assert_eq!(made.expires_at, iso(at(2 * 3_600_000)));
        assert_eq!(made.revoked_at, None);
        let listed = k.dir.list_invites("host-1").await.unwrap();
        assert_eq!(
            listed.invites.iter().map(|i| i.code.as_str()).collect::<Vec<_>>(),
            [made.code.as_str()]
        );

        let revoked = k.dir.revoke_invite("host-1", &made.code).await.unwrap().invite;
        assert_eq!(revoked.state, "REVOKED");
        assert_eq!(revoked.revoked_at.as_deref(), Some(iso(T0).as_str()));
        k.set_now(5_000);
        let again = k.dir.revoke_invite("host-1", &made.code).await.unwrap().invite;
        assert_eq!(again.revoked_at, revoked.revoked_at, "the first time stays");
        rejects(
            k.dir.revoke_invite("host-1", "ZZZZZZZZZZ").await,
            "REALM_INVITE_INVALID",
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn another_host_cannot_revoke_an_invite_that_is_not_theirs() {
    each!(|k| {
        k.create("host-1", "friends").await;
        k.create("host-2", "rivals").await;
        let made = k.dir.create_invite("host-1", &invite(3, 2)).await.unwrap().invite;
        rejects(k.dir.revoke_invite("host-2", &made.code).await, "REALM_INVITE_INVALID");
        assert_eq!(k.dir.list_invites("host-1").await.unwrap().invites[0].state, "USABLE");
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_host_has_a_ceiling_of_open_invites_expired_and_revoked_ones_do_not_count() {
    each!(|k, o| o.limits = limits(10, 10, 2, 50, 50); {
        k.create("host-1", "friends").await;
        let first = k.dir.create_invite("host-1", &invite(1, 1)).await.unwrap().invite;
        k.dir.create_invite("host-1", &invite(1, 24)).await.unwrap();
        rejects(k.dir.create_invite("host-1", &invite(1, 24)).await, "REALM_INVITE_LIMIT");
        k.dir.revoke_invite("host-1", &first.code).await.unwrap();
        k.dir.create_invite("host-1", &invite(1, 1)).await.unwrap();
        rejects(k.dir.create_invite("host-1", &invite(1, 24)).await, "REALM_INVITE_LIMIT");
        k.set_now(2 * 3_600_000);
        k.dir.create_invite("host-1", &invite(1, 24)).await.unwrap();
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_clashing_invite_code_is_regenerated() {
    each!(|k, o| o.ids = Arc::new(ListedCodes(Mutex::new(vec!["ABCDEFGHJK", "ABCDEFGHJK", "ABCDEFGHJL"]))); {
        k.create("host-1", "friends").await;
        assert_eq!(k.dir.create_invite("host-1", &invite(1, 1)).await.unwrap().invite.code, "ABCDEFGHJK");
        assert_eq!(k.dir.create_invite("host-1", &invite(1, 1)).await.unwrap().invite.code, "ABCDEFGHJL");
    });
}

// ------------------------------------------------------------------ heartbeat, release, limits

#[tokio::test(flavor = "multi_thread")]
async fn a_heartbeat_puts_the_realm_online_for_90_seconds_and_records_the_version() {
    each!(|k| {
        k.create("host-1", "friends").await;
        k.set_now(1000);
        let with_version = HeartbeatInput {
            version: Some("0.2.128".to_owned()),
            ..heartbeat()
        };
        let beat = k.dir.heartbeat("host-1", &with_version).await.unwrap();
        assert_eq!(
            (
                beat.online,
                beat.last_online_at.as_str(),
                beat.online_until.as_str(),
                beat.ttl_seconds,
                beat.version.as_deref()
            ),
            (
                true,
                iso(at(1000)).as_str(),
                iso(at(91_000)).as_str(),
                90,
                Some("0.2.128")
            )
        );
        let info = k.dir.get_by_name("friend-1", "friends", None).await.unwrap().realm;
        assert!(info.online);
        assert_eq!(info.version.as_deref(), Some("0.2.128"));

        k.set_now(91_000);
        assert!(
            !k.dir
                .get_by_name("friend-1", "friends", None)
                .await
                .unwrap()
                .realm
                .online
        );

        k.set_now(100_000);
        k.dir.heartbeat("host-1", &heartbeat()).await.unwrap();
        let kept = k
            .dir
            .get_by_name("friend-1", "friends", None)
            .await
            .unwrap()
            .realm
            .version;
        assert_eq!(
            kept.as_deref(),
            Some("0.2.128"),
            "no version in the heartbeat keeps the last one"
        );
        let closing = k
            .dir
            .heartbeat(
                "host-1",
                &HeartbeatInput {
                    online: Some(false),
                    ..heartbeat()
                },
            )
            .await
            .unwrap();
        assert!(!closing.online);
        assert!(
            !k.dir
                .get_by_name("friend-1", "friends", None)
                .await
                .unwrap()
                .realm
                .online
        );
        let hosted = k.dir.get_mine("host-1").await.unwrap().hosted.unwrap();
        assert_eq!(hosted.last_online_at.as_deref(), Some(iso(at(100_000)).as_str()));
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn releasing_a_realm_frees_its_name_at_once_and_removes_its_members_and_invites() {
    each!(|k| {
        let realm = k.create("host-1", "friends").await;
        let made = k.dir.create_invite("host-1", &invite(3, 24)).await.unwrap().invite;
        k.dir.join("friend-1", &join_code(&made.code)).await.unwrap();
        assert!(k.dir.release("host-1").await.unwrap().released);
        assert!(k.repo.find_by_id(&realm.info.realm_id).await.unwrap().is_none());
        assert!(k.repo.find_invite(&made.code).await.unwrap().is_none());
        rejects(k.dir.get_by_name("friend-1", "friends", None).await, "REALM_NOT_FOUND");
        rejects(k.dir.release("host-1").await, "REALM_NOT_FOUND");
        k.create("host-2", "friends").await;
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn every_call_counts_against_a_per_account_budget_and_is_refused_when_it_is_spent() {
    let limiter = Arc::new(Limiter {
        keys: Mutex::new(Vec::new()),
        limited: |key| key.starts_with("realms:join:"),
    });
    for target in targets() {
        limiter.keys.lock().unwrap().clear();
        let shared = limiter.clone();
        let k = kit_with(target.open(), move |o| o.limiter = shared);
        k.create("host-1", "friends").await;
        k.dir.get_by_name("friend-1", "friends", None).await.unwrap();
        rejects(k.dir.join("friend-1", &join_name("friends")).await, "AUTH_RATE_LIMITED");
        rejects(
            k.dir.preview_invite("friend-1", "ABCDEFGHJK").await,
            "AUTH_RATE_LIMITED",
        );
        let keys = limiter.keys.lock().unwrap().clone();
        assert_eq!(
            keys,
            [
                "realms:create:host-1",
                "realms:read:friend-1",
                "realms:join:friend-1",
                "realms:join:friend-1"
            ]
        );
    }
}

// ------------------------------------------------------------------ join requests [2026-10-15]

#[tokio::test(flavor = "multi_thread")]
async fn a_join_without_an_invite_is_a_request_recorded_once_shown_newest_first_and_counted() {
    each!(|k| {
        k.create("host-1", "friends").await;
        rejects(
            k.dir.join("friend-1", &join_name("friends")).await,
            "REALM_JOIN_REQUESTED",
        );
        k.set_now(60_000);
        rejects(
            k.dir.join("friend-1", &join_name("friends")).await,
            "REALM_JOIN_REQUESTED",
        );
        k.set_now(120_000);
        rejects(
            k.dir.join("friend-2", &join_name("friends")).await,
            "REALM_JOIN_REQUESTED",
        );
        let requests = k.dir.list_join_requests("host-1").await.unwrap().requests;
        let seen: Vec<(&str, Option<&str>, String)> = requests
            .iter()
            .map(|r| (r.account_id.as_str(), r.display_name.as_deref(), r.at.clone()))
            .collect();
        assert_eq!(
            seen,
            [
                ("friend-2", Some("Pal"), iso(at(120_000))),
                ("friend-1", Some("Friendly"), iso(T0))
            ],
            "asking again keeps the first time"
        );
        assert_eq!(
            k.dir
                .get_mine("host-1")
                .await
                .unwrap()
                .hosted
                .unwrap()
                .join_request_count,
            2
        );
        assert!(
            k.dir.get_mine("friend-1").await.unwrap().member_of.is_empty(),
            "a waiting player is in nothing"
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_banned_account_is_refused_as_banned_and_leaves_no_request() {
    each!(|k| {
        let realm = k.create("host-1", "friends").await;
        k.dir.ban("host-1", "friend-1", Some("griefing")).await.unwrap();
        rejects(k.dir.join("friend-1", &join_name("friends")).await, "REALM_BANNED");
        assert!(
            k.repo
                .find_by_id(&realm.info.realm_id)
                .await
                .unwrap()
                .unwrap()
                .join_requests
                .is_empty()
        );
        assert!(k.dir.list_join_requests("host-1").await.unwrap().requests.is_empty());
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn deny_removes_the_request_and_the_player_may_ask_again() {
    each!(|k| {
        k.create("host-1", "friends").await;
        rejects(
            k.dir.join("friend-1", &join_name("friends")).await,
            "REALM_JOIN_REQUESTED",
        );
        let denied = k.dir.deny_join_request("host-1", "friend-1").await.unwrap();
        assert_eq!((denied.account_id.as_str(), denied.status), ("friend-1", "DENIED"));
        assert!(k.dir.list_join_requests("host-1").await.unwrap().requests.is_empty());
        rejects(
            k.dir.deny_join_request("host-1", "friend-1").await,
            "REALM_REQUEST_NOT_FOUND",
        );
        rejects(
            k.dir.approve_join_request("host-1", "friend-2").await,
            "REALM_REQUEST_NOT_FOUND",
        );
        rejects(
            k.dir.join("friend-1", &join_name("friends")).await,
            "REALM_JOIN_REQUESTED",
        );
        assert_eq!(k.dir.list_join_requests("host-1").await.unwrap().requests.len(), 1);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_full_request_list_refuses_a_new_player_and_keeps_the_requests_it_has() {
    each!(|k, o| o.limits = limits(10, 10, 25, 1, 50); {
        k.create("host-1", "friends").await;
        rejects(k.dir.join("friend-1", &join_name("friends")).await, "REALM_JOIN_REQUESTED");
        rejects(k.dir.join("friend-2", &join_name("friends")).await, "REALM_REQUESTS_FULL");
        let requests = k.dir.list_join_requests("host-1").await.unwrap().requests;
        assert_eq!(requests.iter().map(|r| r.account_id.as_str()).collect::<Vec<_>>(), ["friend-1"]);
        let entry = rifts_realms::input::EntryRealmInput { name: "friends".to_owned(), invite_code: None, known_host_key: None };
        let decision = k.dir.authorize_entry("friend-2", &entry).await.unwrap();
        assert_eq!(decision, rifts_realms::directory::EntryAuthorization::Refused("REALM_REQUESTS_FULL"));
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn approving_into_a_full_realm_is_refused_and_the_request_stays() {
    each!(|k, o| o.limits = limits(2, 10, 25, 50, 50); {
        k.create("host-1", "friends").await;
        rejects(k.dir.join("friend-1", &join_name("friends")).await, "REALM_JOIN_REQUESTED");
        let made = k.dir.create_invite("host-1", &invite(1, 24)).await.unwrap().invite;
        k.dir.join("friend-2", &join_code(&made.code)).await.unwrap();
        rejects(k.dir.approve_join_request("host-1", "friend-1").await, "REALM_FULL");
        let requests = k.dir.list_join_requests("host-1").await.unwrap().requests;
        assert_eq!(requests.iter().map(|r| r.account_id.as_str()).collect::<Vec<_>>(), ["friend-1"]);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn the_host_adds_a_player_by_username_at_once_and_a_pending_request_goes_with_it() {
    each!(|k, o| o.accounts = Arc::new(with_usernames()); {
        k.create("host-1", "friends").await;
        rejects(k.dir.join("friend-1", &join_name("friends")).await, "REALM_JOIN_REQUESTED");

        let added = k.dir.add_member_by_username("host-1", "  Friendly ").await.unwrap();
        assert_eq!((added.account_id.as_str(), added.display_name.as_str(), added.added), ("friend-1", "Friendly", true));
        assert!(k.dir.list_join_requests("host-1").await.unwrap().requests.is_empty());
        assert!(!k.dir.join("friend-1", &join_name("friends")).await.unwrap().joined, "in the realm now");
        let again = k.dir.add_member_by_username("host-1", "FRIENDLY").await.unwrap();
        assert_eq!((again.account_id.as_str(), again.added), ("friend-1", false));

        let pal = k.dir.add_member_by_username("host-1", "PAL").await.unwrap();
        assert_eq!((pal.account_id.as_str(), pal.display_name.as_str(), pal.added), ("friend-2", "Pal", true));
        assert_eq!(k.dir.get_by_name("host-1", "friends", None).await.unwrap().realm.member_count, 3);
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn adding_by_username_an_unknown_name_is_not_found_and_a_banned_account_is_refused() {
    for target in targets() {
        let plain = kit(target.open());
        plain.create("host-1", "friends").await;
        rejects(
            plain.dir.add_member_by_username("host-1", "nobody").await,
            "REALM_PLAYER_NOT_FOUND",
        );
    }
    each!(|k, o| o.accounts = Arc::new(rifts_realms::ports::FixedAccounts {
        usernames: [("pal".to_owned(), "friend-2".to_owned())].into(),
        ..names()
    }); {
        k.create("host-1", "friends").await;
        k.dir.ban("host-1", "friend-2", None).await.unwrap();
        rejects(k.dir.add_member_by_username("host-1", "pal").await, "REALM_BANNED");
        rejects(k.dir.add_member_by_username("host-1", "nobody").await, "REALM_PLAYER_NOT_FOUND");
        assert_eq!(k.dir.get_by_name("host-1", "friends", None).await.unwrap().realm.member_count, 1, "nothing was added");
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn only_the_host_manages_requests_and_adds_players() {
    each!(|k, o| o.accounts = Arc::new(with_usernames()); {
        k.create("host-1", "friends").await;
        rejects(k.dir.list_join_requests("friend-1").await, "REALM_NOT_FOUND");
        rejects(k.dir.approve_join_request("friend-1", "friend-2").await, "REALM_NOT_FOUND");
        rejects(k.dir.deny_join_request("friend-1", "friend-2").await, "REALM_NOT_FOUND");
        rejects(k.dir.add_member_by_username("friend-1", "pal").await, "REALM_NOT_FOUND");
    });
}

// ------------------------------------------------------------------ a realm is named after its host's username

#[tokio::test(flavor = "multi_thread")]
async fn a_claim_without_a_name_takes_the_hosts_username_no_username_no_realm() {
    each!(|k| {
        let realm = k.create_with("host-1", "ignored", |i| i.name = None).await;
        assert_eq!(realm.info.name, "hosty");
        assert_eq!(realm.info.host.display_name.as_deref(), Some("Hosty"));
        assert_eq!(
            k.dir.get_by_name("friend-1", "hosty", None).await.unwrap().realm.name,
            "hosty"
        );
        rejects(
            k.try_create("nobody-yet", "x", |i| i.name = None).await,
            "REALM_USERNAME_REQUIRED",
        );
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reserved_word_is_the_hosts_own_realm_name_when_it_is_their_username() {
    each!(|k, o| o.accounts = Arc::new(rifts_realms::ports::FixedAccounts {
        names: [("host-1".to_owned(), "Mystic".to_owned())].into(),
        usernames: Default::default(),
    }); {
        let realm = k.create_with("host-1", "x", |i| i.name = None).await;
        assert_eq!(realm.info.name, "mystic");
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_name_that_is_another_players_username_is_refused_the_hosts_own_is_fine() {
    each!(|k, o| o.accounts = Arc::new(with_usernames()); {
        rejects(k.try_create("host-1", "pal", |_| {}).await, "REALM_NAME_IS_USERNAME");
        assert_eq!(k.create("host-1", "hosty").await.info.name, "hosty");
        // A name nobody has as a username stays possible through the API
        assert_eq!(k.create("friend-1", "ember-hold").await.info.name, "ember-hold");
    });
}

// ------------------------------------------------------------------ player invites [2026-10-16]

#[tokio::test(flavor = "multi_thread")]
async fn an_invited_player_is_not_a_member_until_they_accept_both_sides_see_the_invite() {
    each!(|k, o| o.accounts = Arc::new(with_usernames()); {
        k.create("host-1", "friends").await;
        let invited = k.dir.invite_player_by_username("host-1", "  Friendly ").await.unwrap();
        assert_eq!((invited.account_id.as_str(), invited.display_name.as_str(), invited.status), ("friend-1", "Friendly", "INVITED"));

        let hosted = k.dir.get_mine("host-1").await.unwrap().hosted.unwrap();
        assert_eq!(hosted.player_invites.len(), 1);
        assert_eq!(
            (hosted.player_invites[0].account_id.as_str(), hosted.player_invites[0].display_name.as_deref(), hosted.player_invites[0].at.clone()),
            ("friend-1", Some("Friendly"), iso(T0))
        );
        assert_eq!(hosted.info.member_count, 1, "an invite is not a membership");
        let seen = k.dir.get_by_name("friend-1", "friends", None).await.unwrap().realm;
        assert!(seen.invited);
        assert_eq!(seen.membership, None);
        assert!(k.dir.get_mine("friend-1").await.unwrap().member_of.is_empty());

        let mine = k.dir.list_my_player_invites("friend-1").await.unwrap();
        assert_eq!(mine.invites.len(), 1);
        assert_eq!((mine.invites[0].realm.name.as_str(), mine.invites[0].realm.invited, mine.invites[0].at.clone()), ("friends", true, iso(T0)));
        assert!(k.dir.list_my_player_invites("friend-2").await.unwrap().invites.is_empty());

        let accepted = k.dir.accept_player_invite("friend-1", "friends").await.unwrap();
        assert!(accepted.joined);
        assert_eq!(accepted.realm.membership, Some("MEMBER"));
        assert!(!accepted.realm.invited);
        assert!(k.dir.list_my_player_invites("friend-1").await.unwrap().invites.is_empty());
        assert!(k.dir.get_mine("host-1").await.unwrap().hosted.unwrap().player_invites.is_empty());
        let member_of = k.dir.get_mine("friend-1").await.unwrap().member_of;
        assert_eq!(member_of.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["friends"]);
        rejects(k.dir.accept_player_invite("friend-1", "friends").await, "REALM_PLAYER_INVITE_NOT_FOUND");
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn declining_removes_the_invite_the_host_can_cancel_a_missing_invite_is_not_found() {
    each!(|k, o| o.accounts = Arc::new(with_usernames()); {
        k.create("host-1", "friends").await;
        k.dir.invite_player_by_username("host-1", "friendly").await.unwrap();
        assert!(k.dir.decline_player_invite("friend-1", "friends").await.unwrap().declined);
        assert!(k.dir.get_mine("host-1").await.unwrap().hosted.unwrap().player_invites.is_empty());
        rejects(k.dir.decline_player_invite("friend-1", "friends").await, "REALM_PLAYER_INVITE_NOT_FOUND");

        k.dir.invite_player_by_username("host-1", "friendly").await.unwrap();
        let again = k.dir.invite_player_by_username("host-1", "FRIENDLY").await.unwrap();
        assert_eq!(again.status, "INVITED", "inviting again changes nothing");
        assert_eq!(k.dir.get_mine("host-1").await.unwrap().hosted.unwrap().player_invites.len(), 1);
        let cancelled = k.dir.cancel_player_invite("host-1", "friend-1").await.unwrap();
        assert_eq!((cancelled.account_id.as_str(), cancelled.status), ("friend-1", "CANCELLED"));
        rejects(k.dir.cancel_player_invite("host-1", "friend-1").await, "REALM_PLAYER_INVITE_NOT_FOUND");
        rejects(k.dir.accept_player_invite("friend-1", "friends").await, "REALM_PLAYER_INVITE_NOT_FOUND");
        rejects(k.dir.accept_player_invite("friend-1", "nothing-here").await, "REALM_NOT_FOUND");
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn joining_or_pressing_play_with_a_pending_invite_accepts_it() {
    each!(|k, o| o.accounts = Arc::new(with_usernames()); {
        let realm = k.create("host-1", "friends").await;
        k.serve("host-1", &realm.info.realm_id).await;
        let entry = |name: &str| rifts_realms::input::EntryRealmInput { name: name.to_owned(), invite_code: None, known_host_key: None };
        let granted = rifts_realms::directory::EntryAuthorization::Ok { realm_id: realm.info.realm_id.clone(), is_host: false };

        k.dir.invite_player_by_username("host-1", "friendly").await.unwrap();
        assert_eq!(k.dir.authorize_entry("friend-1", &entry("friends")).await.unwrap(), granted);
        let seen = k.dir.get_by_name("friend-1", "friends", None).await.unwrap().realm;
        assert_eq!(seen.membership, Some("MEMBER"));
        assert!(k.dir.list_join_requests("host-1").await.unwrap().requests.is_empty(), "no request was recorded");

        k.dir.invite_player_by_username("host-1", "pal").await.unwrap();
        let joined = k.dir.join("friend-2", &join_name("friends")).await.unwrap();
        assert!(joined.joined);
        assert_eq!(joined.realm.membership, Some("MEMBER"));
        assert!(k.dir.get_mine("host-1").await.unwrap().hosted.unwrap().player_invites.is_empty());
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn inviting_a_player_who_asked_lets_them_in_a_member_is_answered_member_the_host_cannot_invite_themselves() {
    each!(|k, o| o.accounts = Arc::new(with_usernames()); {
        k.create("host-1", "friends").await;
        rejects(k.dir.join("friend-1", &join_name("friends")).await, "REALM_JOIN_REQUESTED");
        assert!(k.dir.get_by_name("friend-1", "friends", None).await.unwrap().realm.requested, "the player sees their request waiting");

        let let_in = k.dir.invite_player_by_username("host-1", "friendly").await.unwrap();
        assert_eq!((let_in.account_id.as_str(), let_in.status), ("friend-1", "MEMBER"));
        assert!(k.dir.list_join_requests("host-1").await.unwrap().requests.is_empty());
        let seen = k.dir.get_by_name("friend-1", "friends", None).await.unwrap().realm;
        assert_eq!((seen.membership, seen.requested), (Some("MEMBER"), false));
        assert_eq!(k.dir.invite_player_by_username("host-1", "friendly").await.unwrap().status, "MEMBER");
        rejects(k.dir.invite_player_by_username("host-1", "hosty").await, "REALM_INVALID_TARGET");
        rejects(k.dir.invite_player_by_username("host-1", "nobody").await, "REALM_PLAYER_NOT_FOUND");
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_ban_withdraws_the_invite_and_refuses_a_new_one_accepting_after_a_ban_is_refused() {
    each!(|k, o| o.accounts = Arc::new(with_usernames()); {
        let realm = k.create("host-1", "friends").await;
        k.dir.invite_player_by_username("host-1", "pal").await.unwrap();
        k.dir.ban("host-1", "friend-2", Some("griefing")).await.unwrap();
        assert!(k.repo.find_by_id(&realm.info.realm_id).await.unwrap().unwrap().player_invites.is_empty());
        rejects(k.dir.invite_player_by_username("host-1", "pal").await, "REALM_BANNED");
        rejects(k.dir.accept_player_invite("friend-2", "friends").await, "REALM_PLAYER_INVITE_NOT_FOUND");
        assert!(k.dir.list_my_player_invites("friend-2").await.unwrap().invites.is_empty());
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn the_unanswered_invite_list_has_a_ceiling() {
    each!(|k, o| {
        o.accounts = Arc::new(with_usernames());
        o.limits = limits(10, 10, 25, 50, 1);
    }; {
        k.create("host-1", "friends").await;
        k.dir.invite_player_by_username("host-1", "friendly").await.unwrap();
        rejects(k.dir.invite_player_by_username("host-1", "pal").await, "REALM_PLAYER_INVITES_FULL");
        k.dir.decline_player_invite("friend-1", "friends").await.unwrap();
        let next = k.dir.invite_player_by_username("host-1", "pal").await.unwrap();
        assert_eq!(next.status, "INVITED", "a freed slot takes the next invite");
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn only_the_host_invites_and_cancels() {
    each!(|k, o| o.accounts = Arc::new(with_usernames()); {
        k.create("host-1", "friends").await;
        rejects(k.dir.invite_player_by_username("friend-1", "pal").await, "REALM_NOT_FOUND");
        rejects(k.dir.cancel_player_invite("friend-1", "friend-2").await, "REALM_NOT_FOUND");
    });
}
