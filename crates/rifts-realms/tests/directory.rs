// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The realm directory's launcher operations, on SQLite and (when configured) Postgres.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;
use std::sync::atomic::Ordering;

use common::kit::{FailingNames, FailingSink, KeyLimiter, Kit, Options, T0, code, kit, kit_with};
use common::targets;
use rifts_realms::RealmsError;
use rifts_realms::directory::EntryAuthorization;
use rifts_realms::input::{
    CreateRealmInput, EntryRealmInput, HeartbeatInput, HostingInput, InviteInput, JoinInput, SettingsInput,
};
use rifts_realms::realm::{JoinPolicy, RealmEvent, RemovalReason};
use serde_json::Value;

const KEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAA";

fn create(name: Option<&str>, policy: JoinPolicy) -> CreateRealmInput {
    CreateRealmInput {
        name: name.map(str::to_owned),
        host_public_key: KEY.to_owned(),
        join_policy: policy,
        listed: true,
        version: Some("0.1.70".into()),
    }
}

fn join_name(name: &str) -> JoinInput {
    JoinInput {
        name: Some(name.to_owned()),
        invite_code: None,
        known_host_key: None,
    }
}

fn join_code(code: &str) -> JoinInput {
    JoinInput {
        name: None,
        invite_code: Some(code.to_owned()),
        known_host_key: None,
    }
}

fn message<T>(result: &Result<T, RealmsError>) -> String {
    match result {
        Err(RealmsError::Refused(r)) => r.message.clone(),
        _ => String::new(),
    }
}

async fn hosted(k: &Kit, policy: JoinPolicy) -> String {
    let reply = k.directory.create_realm("host-1", &create(None, policy)).await.unwrap();
    reply.realm.info.realm_id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn create_realm_names_it_after_the_host() {
    for target in targets() {
        let k = kit(target);
        let reply = k
            .directory
            .create_realm("host-1", &create(None, JoinPolicy::Public))
            .await
            .unwrap();
        assert_eq!(reply.realm.info.name, "hosty");
        assert_eq!(reply.realm.info.realm_id, "realm-1");
        assert_eq!(reply.realm.info.membership, Some("HOST"));
        assert_eq!(reply.realm.members.len(), 1);
        let again = k
            .directory
            .create_realm("host-1", &create(Some("other"), JoinPolicy::Public))
            .await;
        assert_eq!(code(&again), "REALM_ALREADY_EXISTS");
        assert_eq!(
            message(&again),
            "You already host a realm. Release it before claiming another name."
        );
        let nameless = k
            .directory
            .create_realm("stranger", &create(None, JoinPolicy::Public))
            .await;
        assert_eq!(code(&nameless), "REALM_USERNAME_REQUIRED");
        let squat = k
            .directory
            .create_realm("host-2", &create(Some("pal"), JoinPolicy::Public))
            .await;
        assert_eq!(code(&squat), "REALM_NAME_IS_USERNAME");
        let held = k
            .directory
            .create_realm("host-3", &create(Some("realmy"), JoinPolicy::Public))
            .await
            .unwrap();
        assert_eq!(held.realm.info.realm_id, "realm-2");
        let taken = k
            .directory
            .create_realm("host-2", &create(Some("realmy"), JoinPolicy::Public))
            .await;
        assert_eq!(code(&taken), "REALM_NAME_TAKEN");
        // A name never online is free three days after creation.
        k.advance(259_200_000);
        let claimed = k
            .directory
            .create_realm("host-2", &create(Some("realmy"), JoinPolicy::Public))
            .await
            .unwrap();
        assert_eq!(claimed.realm.info.name, "realmy");
        assert!(k.directory.get_realm_by_host("host-3").await.unwrap().is_none());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_host_view_is_written_in_wire_order() {
    for target in targets() {
        let k = kit(target);
        let reply = k
            .directory
            .create_realm("host-1", &create(None, JoinPolicy::Public))
            .await
            .unwrap();
        let text = serde_json::to_string(&reply).unwrap();
        let keys: Vec<String> = match serde_json::from_str::<Value>(&text).unwrap()["realm"].clone() {
            Value::Object(map) => map.keys().cloned().collect(),
            _ => Vec::new(),
        };
        assert_eq!(
            keys,
            [
                "realmId",
                "name",
                "host",
                "hostPublicKey",
                "joinPolicy",
                "listed",
                "online",
                "serving",
                "hubReady",
                "playerCount",
                "version",
                "memberCount",
                "membership",
                "requested",
                "invited",
                "createdAt",
                "lastOnlineAt",
                "onlineUntil",
                "nameReleaseAt",
                "members",
                "bans",
                "joinRequestCount",
                "playerInvites",
            ]
        );
        assert!(text.contains(r#""host":{"displayName":"Hosty"}"#), "{text}");
        assert!(text.contains(r#""createdAt":"2026-10-13T12:00:00.000Z""#), "{text}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn public_and_invite_joins() {
    for target in targets() {
        let k = kit(target);
        hosted(&k, JoinPolicy::Public).await;
        let joined = k.directory.join("friend-1", &join_name("hosty")).await.unwrap();
        assert!(joined.joined);
        assert_eq!(joined.realm.membership, Some("MEMBER"));
        let again = k.directory.join("friend-1", &join_name("hosty")).await.unwrap();
        assert!(!again.joined);
        let settings = SettingsInput {
            join_policy: Some(JoinPolicy::InviteOnly),
            listed: None,
        };
        k.directory.update_settings("host-1", &settings).await.unwrap();
        let invite = k
            .directory
            .create_invite(
                "host-1",
                &InviteInput {
                    max_uses: 1,
                    ttl_hours: 1,
                },
            )
            .await
            .unwrap()
            .invite;
        assert_eq!((invite.code.as_str(), invite.state), ("ABCDEFGHJK", "USABLE"));
        let preview = k.directory.preview_invite("friend-2", &invite.code).await.unwrap();
        assert_eq!(preview.realm.name, "hosty");
        assert!(
            k.directory
                .join("friend-2", &join_code(&invite.code))
                .await
                .unwrap()
                .joined
        );
        let used = k.directory.join("stranger", &join_code(&invite.code)).await;
        assert_eq!(code(&used), "REALM_INVITE_EXPIRED");
        let unknown = k.directory.join("stranger", &join_code("ZZZZZZZZZZ")).await;
        assert_eq!(message(&unknown), "That invite code is not valid.");
        let owner = JoinInput {
            known_host_key: Some("B".repeat(24)),
            ..join_name("hosty")
        };
        assert_eq!(code(&k.directory.join("friend-1", &owner).await), "REALM_OWNER_CHANGED");
        assert_eq!(
            code(&k.directory.join("x", &join_name("nope")).await),
            "REALM_NOT_FOUND"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invite_only_asks_the_host() {
    for target in targets() {
        let k = kit(target);
        hosted(&k, JoinPolicy::InviteOnly).await;
        let asked = k.directory.join("friend-1", &join_name("hosty")).await;
        assert_eq!(code(&asked), "REALM_JOIN_REQUESTED");
        assert_eq!(
            message(&asked),
            "Request sent to the host. Wait for their approval, then press Play again."
        );
        let requests = k.directory.list_join_requests("host-1").await.unwrap().requests;
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].display_name.as_deref(), Some("Friendly"));
        let missing = k.directory.approve_join_request("host-1", "nobody").await;
        assert_eq!(code(&missing), "REALM_REQUEST_NOT_FOUND");
        let approved = k.directory.approve_join_request("host-1", "friend-1").await.unwrap();
        assert_eq!(approved.status, "APPROVED");
        assert!(!k.directory.join("friend-1", &join_name("hosty")).await.unwrap().joined);
        assert_eq!(
            code(&k.directory.join("friend-2", &join_name("hosty")).await),
            "REALM_JOIN_REQUESTED"
        );
        assert_eq!(
            k.directory
                .deny_join_request("host-1", "friend-2")
                .await
                .unwrap()
                .status,
            "DENIED"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kick_ban_and_leave_emit_events() {
    for target in targets() {
        let k = kit(target);
        let realm = hosted(&k, JoinPolicy::Public).await;
        k.directory.join("friend-1", &join_name("hosty")).await.unwrap();
        k.directory.join("friend-2", &join_name("hosty")).await.unwrap();
        assert_eq!(
            code(&k.directory.kick("host-1", "host-1").await),
            "REALM_INVALID_TARGET"
        );
        assert_eq!(k.directory.kick("host-1", "friend-1").await.unwrap().status, "KICKED");
        assert_eq!(code(&k.directory.kick("host-1", "friend-1").await), "REALM_NOT_MEMBER");
        let banned = k
            .directory
            .ban("host-1", "friend-2", Some("rude".into()))
            .await
            .unwrap();
        assert!(banned.changed);
        assert!(!k.directory.ban("host-1", "friend-2", None).await.unwrap().changed);
        assert_eq!(
            code(&k.directory.join("friend-2", &join_name("hosty")).await),
            "REALM_BANNED"
        );
        assert!(k.directory.unban("host-1", "friend-2").await.unwrap().changed);
        assert!(!k.directory.unban("host-1", "friend-2").await.unwrap().changed);
        k.directory.join("friend-1", &join_name("hosty")).await.unwrap();
        assert_eq!(
            code(&k.directory.leave("host-1", "hosty").await),
            "REALM_HOST_CANNOT_LEAVE"
        );
        assert!(k.directory.leave("friend-1", "hosty").await.unwrap().left);
        assert_eq!(code(&k.directory.leave("friend-1", "hosty").await), "REALM_NOT_MEMBER");
        let reasons: Vec<RemovalReason> = k
            .events
            .take()
            .into_iter()
            .filter_map(|e| match e {
                RealmEvent::MemberRemoved { realm_id, reason, .. } if realm_id == realm => Some(reason),
                _ => None,
            })
            .collect();
        assert_eq!(reasons, [RemovalReason::Kick, RemovalReason::Ban, RemovalReason::Leave]);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn player_invites() {
    for target in targets() {
        let k = kit(target);
        hosted(&k, JoinPolicy::InviteOnly).await;
        assert_eq!(
            code(&k.directory.invite_player_by_username("host-1", "nobody").await),
            "REALM_PLAYER_NOT_FOUND"
        );
        assert_eq!(
            code(&k.directory.invite_player_by_username("host-1", "hosty").await),
            "REALM_INVALID_TARGET"
        );
        let invited = k.directory.invite_player_by_username("host-1", " Pal ").await.unwrap();
        assert_eq!((invited.account_id.as_str(), invited.status), ("friend-2", "INVITED"));
        assert_eq!(
            k.directory
                .invite_player_by_username("host-1", "pal")
                .await
                .unwrap()
                .status,
            "INVITED"
        );
        let mine = k.directory.list_my_player_invites("friend-2").await.unwrap();
        assert_eq!(mine.invites.len(), 1);
        assert!(mine.invites[0].realm.invited);
        let accepted = k.directory.accept_player_invite("friend-2", "hosty").await.unwrap();
        assert!(accepted.joined);
        assert_eq!(
            code(&k.directory.decline_player_invite("friend-2", "hosty").await),
            "REALM_PLAYER_INVITE_NOT_FOUND"
        );
        // One who had asked to join is let in by the invite.
        assert_eq!(
            code(&k.directory.join("friend-1", &join_name("hosty")).await),
            "REALM_JOIN_REQUESTED"
        );
        assert_eq!(
            k.directory
                .invite_player_by_username("host-1", "friendly")
                .await
                .unwrap()
                .status,
            "MEMBER"
        );
        let added = k.directory.add_member_by_username("host-1", "hosty").await.unwrap();
        assert!(!added.added);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invites_have_a_limit_and_revoke_once() {
    for target in targets() {
        let k = kit(target);
        hosted(&k, JoinPolicy::InviteOnly).await;
        let one = InviteInput {
            max_uses: 1,
            ttl_hours: 1,
        };
        for _ in 0..25 {
            k.directory.create_invite("host-1", &one).await.unwrap();
        }
        let full = k.directory.create_invite("host-1", &one).await;
        assert_eq!(
            message(&full),
            "You can have at most 25 open invites; revoke one first."
        );
        let first = k.directory.revoke_invite("host-1", "ABCDEFGHJK").await.unwrap().invite;
        k.advance(5);
        let second = k.directory.revoke_invite("host-1", "ABCDEFGHJK").await.unwrap().invite;
        assert_eq!(first.revoked_at, second.revoked_at);
        assert_eq!(second.state, "REVOKED");
        assert_eq!(k.directory.list_invites("host-1").await.unwrap().invites.len(), 25);
        assert_eq!(code(&k.directory.list_invites("friend-1").await), "REALM_NOT_FOUND");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn heartbeat_listing_and_release() {
    for target in targets() {
        let k = kit(target);
        let realm = hosted(&k, JoinPolicy::Public).await;
        let beat = HeartbeatInput {
            online: None,
            version: None,
            hosting: Some(HostingInput {
                max_servers: 2,
                max_players: 8,
                afk_seconds: Some(300),
            }),
        };
        let reply = k.directory.heartbeat("host-1", &beat).await.unwrap();
        assert!(reply.online);
        assert_eq!(reply.ttl_seconds, 90);
        assert_eq!(reply.version.as_deref(), Some("0.1.70"));
        assert_eq!(reply.online_until, "2026-10-13T12:01:30.000Z");
        let listed = k.directory.list_listed("friend-1", 25, 0).await.unwrap();
        assert_eq!(listed.realms.len(), 1);
        assert!(listed.realms[0].online && !listed.realms[0].serving);
        k.serve("host-1", &realm).await;
        assert!(
            k.directory
                .get_by_name("friend-1", "hosty", None)
                .await
                .unwrap()
                .realm
                .serving
        );
        let mine = k.directory.get_mine("host-1").await.unwrap();
        assert!(mine.hosted.is_some() && mine.member_of.is_empty());
        assert!(k.directory.release("host-1").await.unwrap().released);
        assert_eq!(code(&k.directory.release("host-1").await), "REALM_NOT_FOUND");
        assert!(k.events.all().contains(&RealmEvent::RealmReleased {
            realm_id: realm,
            host_account_id: "host-1".into(),
        }));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn entry_authorization_for_play() {
    for target in targets() {
        let k = kit(target);
        let realm = hosted(&k, JoinPolicy::Public).await;
        let entry = |name: &str| EntryRealmInput {
            name: name.to_owned(),
            invite_code: None,
            known_host_key: None,
        };
        assert_eq!(
            k.directory.authorize_entry("host-1", &entry("hosty")).await.unwrap(),
            EntryAuthorization::Ok {
                realm_id: realm.clone(),
                is_host: true,
            }
        );
        // Offline: no member is added.
        assert_eq!(
            k.directory.authorize_entry("friend-1", &entry("hosty")).await.unwrap(),
            EntryAuthorization::Refused("REALM_OFFLINE")
        );
        assert!(!k.directory.is_member(&realm, "friend-1").await.unwrap());
        k.serve("host-1", &realm).await;
        assert_eq!(
            k.directory.authorize_entry("friend-1", &entry("hosty")).await.unwrap(),
            EntryAuthorization::Ok {
                realm_id: realm.clone(),
                is_host: false,
            }
        );
        assert!(k.directory.is_member(&realm, "friend-1").await.unwrap());
        let off = k
            .directory
            .authorize_game_session_realm("friend-1", Some(&serde_json::json!({ "name": "hosty" })), false)
            .await;
        assert_eq!(code(&off), "REALMS_DISABLED");
        let none = k
            .directory
            .authorize_game_session_realm("friend-1", None, false)
            .await
            .unwrap();
        assert_eq!(none, None);
        let missing = k
            .directory
            .authorize_game_session_realm("friend-1", Some(&serde_json::json!({ "name": "nope" })), true)
            .await;
        assert_eq!(message(&missing), "There is no realm with that name.");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sweeping_lapsed_hosts_reports_each_lapse_once() {
    for target in targets() {
        let k = kit(target);
        let realm = hosted(&k, JoinPolicy::Public).await;
        k.serve("host-1", &realm).await;
        k.events.take();
        assert_eq!(k.directory.sweep_lapsed_hosts().await.unwrap(), 0);
        k.advance(90_000);
        assert_eq!(k.directory.sweep_lapsed_hosts().await.unwrap(), 1);
        assert_eq!(k.directory.sweep_lapsed_hosts().await.unwrap(), 0);
        assert_eq!(
            k.events.take(),
            [RealmEvent::HostOffline {
                realm_id: realm.clone(),
                host_account_id: "host-1".into(),
            }]
        );
        // One closed by `mark_offline` is never announced.
        k.serve("host-1", &realm).await;
        assert!(k.directory.mark_offline(&realm).await.unwrap());
        k.advance(90_000);
        assert_eq!(k.directory.sweep_lapsed_hosts().await.unwrap(), 0);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn budgets_names_and_sinks_fail_softly() {
    for target in targets() {
        let limiter = Arc::new(KeyLimiter::default());
        limiter.limited.lock().unwrap().push("realms:read:friend-1".into());
        let sink = Arc::new(FailingSink::default());
        let k = kit_with(
            target,
            Options {
                limiter: limiter.clone(),
                events: Some(sink.clone()),
                ..Options::default()
            },
        );
        let realm = hosted(&k, JoinPolicy::Public).await;
        assert_eq!(code(&k.directory.get_mine("friend-1").await), "AUTH_RATE_LIMITED");
        assert!(
            limiter
                .keys
                .lock()
                .unwrap()
                .contains(&"realms:create:host-1".to_owned())
        );
        k.directory.join("friend-1", &join_name("hosty")).await.unwrap();
        assert!(k.directory.leave("friend-1", "hosty").await.unwrap().left);
        assert!(sink.calls.load(Ordering::SeqCst));
        assert!(k.directory.get_realm(&realm).await.unwrap().is_some());
        // The core's calls are never throttled.
        assert!(k.directory.find_by_hub_room_id("not-a-room").await.unwrap().is_none());
    }
    for target in targets() {
        let k = kit_with(
            target,
            Options {
                accounts: Arc::new(FailingNames),
                ..Options::default()
            },
        );
        let made = k
            .directory
            .create_realm("host-1", &create(Some("named"), JoinPolicy::Public))
            .await
            .unwrap();
        assert_eq!(made.realm.info.host.display_name, None);
        assert_eq!(made.realm.created_at, "2026-10-13T12:00:00.000Z");
        assert_eq!(T0, 1_791_892_800_000);
    }
}
