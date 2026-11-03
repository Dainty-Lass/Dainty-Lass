// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Realm presence, its two-second cache and the touch rule, through the directory.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::kit::{T0, code, kit};
use common::targets;
use rifts_realms::input::CreateRealmInput;
use rifts_realms::realm::JoinPolicy;
use rifts_seam::UnixMillis;

async fn realm(k: &common::kit::Kit) -> String {
    let input = CreateRealmInput {
        name: None,
        host_public_key: "A".repeat(24),
        join_policy: JoinPolicy::Public,
        listed: false,
        version: None,
    };
    k.directory
        .create_realm("host-1", &input)
        .await
        .unwrap()
        .realm
        .info
        .realm_id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn presence_is_cached_for_two_seconds() {
    for target in targets() {
        let k = kit(target);
        assert_eq!(k.directory.presence_of("friend-1").await.unwrap(), None);
        assert_eq!(k.directory.presence_of("friend-1").await.unwrap(), None);
        assert_eq!(k.directory.presence_io(), (1, 0));
        k.advance(2_000);
        k.directory.presence_of("friend-1").await.unwrap();
        assert_eq!(k.directory.presence_io(), (2, 0));
        let realm = realm(&k).await;
        k.directory.bind_presence("friend-1", Some(&realm)).await.unwrap();
        // A bind invalidates the cached "none".
        let row = k.directory.presence_of("friend-1").await.unwrap().unwrap();
        assert_eq!(
            (row.realm_id.as_str(), row.expires_at),
            (realm.as_str(), UnixMillis(T0 + 2_000 + 120_000))
        );
        assert_eq!(k.directory.presence_io(), (3, 0));
        k.directory.presence_of("friend-1").await.unwrap();
        assert_eq!(k.directory.presence_io(), (3, 0));
        k.directory.bind_presence("friend-1", None).await.unwrap();
        assert_eq!(k.directory.presence_of("friend-1").await.unwrap(), None);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn touches_write_once_per_forty_seconds_and_never_revive() {
    for target in targets() {
        let k = kit(target);
        let realm = realm(&k).await;
        k.directory.bind_presence("friend-1", Some(&realm)).await.unwrap();
        for _ in 0..12 {
            k.advance(10_000);
            assert!(k.directory.touch_presence("friend-1").await.unwrap().is_some());
        }
        // 120 s of touches every 10 s: one write per 40 s.
        assert_eq!(k.directory.presence_io().1, 3);
        let row = k.directory.presence_of("friend-1").await.unwrap().unwrap();
        assert!(row.expires_at > UnixMillis(k.clock.as_ref().now_ms() + 60_000));
        k.advance(200_000);
        assert_eq!(k.directory.touch_presence("friend-1").await.unwrap(), None);
        assert_eq!(k.directory.presence_of("friend-1").await.unwrap(), None);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_presence_routes() {
    for target in targets() {
        let k = kit(target);
        let empty = k.directory.presence_status("friend-1").await.unwrap();
        assert_eq!(
            serde_json::to_string(&empty).unwrap(),
            r#"{"realm":null,"expiresAt":null}"#
        );
        let realm = realm(&k).await;
        k.directory
            .bind_presence_at_login("friend-1", Some(&realm), true)
            .await
            .unwrap();
        let view = k.directory.presence_heartbeat("friend-1").await.unwrap();
        assert_eq!(
            serde_json::to_string(&view).unwrap(),
            r#"{"realm":{"name":"hosty","serving":false,"hubReady":false,"hostOnline":false},"expiresAt":"2026-10-13T12:02:00.000Z"}"#
        );
        assert!(k.directory.presence_leave("friend-1").await.unwrap().left);
        assert!(k.directory.presence_leave("friend-1").await.unwrap().left);
        assert_eq!(k.directory.presence_status("friend-1").await.unwrap().realm, None);
        // With the module off nothing is bound.
        k.directory
            .bind_presence_at_login("friend-1", Some(&realm), false)
            .await
            .unwrap();
        assert_eq!(k.directory.presence_of("friend-1").await.unwrap(), None);
        // A realm released takes its presence with it.
        k.directory.bind_presence("friend-2", Some(&realm)).await.unwrap();
        k.directory.release("host-1").await.unwrap();
        assert_eq!(k.directory.presence_of("friend-2").await.unwrap(), None);
        assert_eq!(code(&k.directory.presence_status("friend-2").await), "ok");
    }
}

trait NowMs {
    fn now_ms(&self) -> i64;
}

impl NowMs for rifts_realms::ports::ManualClock {
    fn now_ms(&self) -> i64 {
        rifts_realms::ports::Clock::now(self).0
    }
}
