// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Host capabilities and the launcher's control channel, on SQLite and (when configured) Postgres.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;

use common::kit::{T0, code};
use common::{actor, admitted, hunt, participant, targets};
use rifts_realms::capabilities::{
    HostCapabilities, SignedUser, generate_capability_secret, hash_capability_secret, is_opaque_capability,
    parse_optional_bearer, request_identity, runtime_identity,
};
use rifts_realms::control::ControlChannel;
use rifts_realms::hunt::HuntState;
use rifts_realms::ports::ManualClock;
use rifts_realms::repo::hunts::{HuntRepository, NewControlMessage};
use rifts_seam::{CapabilityScope, UnixMillis};
use serde_json::json;

const WIRE: &str = "0190a1b2-c3d4-4e5f-8a6b-7c8d9e0f1a2b";

#[test]
fn secrets_and_their_digest() {
    assert_eq!(
        hash_capability_secret(&"a".repeat(43)).as_deref(),
        Some("66d34fba71f8f450f7e45598853e53bfc23bbd129027cbb131a2f4ffd7878cd0")
    );
    assert_eq!(hash_capability_secret(&"a".repeat(42)), None);
    assert!(!is_opaque_capability(&format!("{}+", "a".repeat(42))));
    let (secret, digest) = generate_capability_secret();
    assert!(is_opaque_capability(secret.expose()));
    assert_eq!(hash_capability_secret(secret.expose()), Some(digest));
    assert!(!format!("{secret:?}").contains(secret.expose()));
}

#[test]
fn bearer_tokens_and_identity() {
    assert_eq!(parse_optional_bearer(None), Ok(None));
    assert_eq!(parse_optional_bearer(Some("Bearer abc")), Ok(Some("abc")));
    assert_eq!(parse_optional_bearer(Some("bearer abc")), Ok(Some("abc")));
    for bad in ["abc", "Bearer", "Bearer ", "Basic abc", "Bearer a b", "Bearer  abc"] {
        assert_eq!(
            parse_optional_bearer(Some(bad)),
            Err("malformed Bearer authorization"),
            "{bad}"
        );
    }
    let params = vec![
        ("accountId".to_owned(), "  ".to_owned()),
        ("phxAccountId".to_owned(), " p ".to_owned()),
    ];
    assert_eq!(request_identity(&params, "/x", None), Some("p".into()));
    let body = json!({ "accountId": " a1 " });
    assert_eq!(request_identity(&[], "/Inventory", Some(&body)), Some("a1".into()));
    assert_eq!(request_identity(&[], "/other", Some(&body)), None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mint_check_and_revoke() {
    for target in targets() {
        let mut session = hunt(WIRE);
        session.state = HuntState::Running;
        session.roster = vec![admitted(participant("host", true)), participant("member", false)];
        HuntRepository::new(target.db.clone())
            .create_hunt(&actor(), &session)
            .await
            .unwrap();
        let caps = HostCapabilities::new(target.db.clone());
        let scopes = vec![CapabilityScope::LeaseHeartbeat, CapabilityScope::GameServerRuntime];
        let (secret, record) = caps
            .mint(&session, "bridge", scopes, UnixMillis(T0), UnixMillis(T0 + 60_000))
            .await
            .unwrap();
        let header = Some(secret.expose());
        let now = UnixMillis(T0 + 1);
        let found = caps
            .check(header, WIRE, CapabilityScope::LeaseHeartbeat, now)
            .await
            .unwrap();
        assert_eq!(found.capability_id, record.capability_id);
        // The route's session id is taken after its first `:`.
        let prefixed = format!("hunt:{WIRE}");
        assert!(
            caps.check(header, &prefixed, CapabilityScope::LeaseHeartbeat, now)
                .await
                .is_ok()
        );
        let wrong_scope = caps.check(header, WIRE, CapabilityScope::Abort, now).await;
        assert_eq!(code(&wrong_scope), "P2P_CAPABILITY_INVALID");
        let missing = caps.check(None, WIRE, CapabilityScope::Abort, now).await;
        assert!(format!("{missing:?}").contains("A valid hunt host capability is required."));
        let expired = caps
            .check(header, WIRE, CapabilityScope::LeaseHeartbeat, UnixMillis(T0 + 60_000))
            .await;
        assert_eq!(code(&expired), "P2P_CAPABILITY_INVALID");
        let context = caps.resolve_runtime(header, now).await.unwrap().unwrap();
        let identity = runtime_identity(&context, SignedUser::Absent, None).unwrap();
        assert_eq!(identity.user_id, "host");
        assert!(runtime_identity(&context, SignedUser::Missing, None).is_none());
        assert!(runtime_identity(&context, SignedUser::User("member"), None).is_none());
        assert!(runtime_identity(&context, SignedUser::User("stranger"), None).is_none());
        let auth = caps.runtime_auth(&context, &identity).unwrap();
        assert_eq!(auth.wire_session.as_deref(), Some(WIRE));
        assert_eq!(auth.roster.len(), 2);
        caps.revoke(WIRE, 1, now).await.unwrap();
        assert_eq!(
            code(&caps.check(header, WIRE, CapabilityScope::LeaseHeartbeat, now).await),
            "P2P_CAPABILITY_INVALID"
        );
        assert!(caps.resolve_runtime(header, now).await.unwrap().is_none());
    }
}

fn message(account: &str, at: i64) -> NewControlMessage {
    NewControlMessage {
        account_id: account.to_owned(),
        message_type: "HOST_OFFERED".to_owned(),
        session_id: Some(WIRE.to_owned()),
        generation: Some(1),
        payload: json!({ "offerId": "o" }),
        created_at: UnixMillis(at),
        expires_at: UnixMillis(at + 600_000),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_control_poll_and_ack() {
    for target in targets() {
        let clock = Arc::new(ManualClock::at(UnixMillis(T0)));
        let control = ControlChannel::new(target.db.clone(), clock.clone());
        let empty = control.poll("a", None, Some("0")).await.unwrap();
        assert_eq!(serde_json::to_string(&empty).unwrap(), r#"{"messages":[]}"#);
        for bad in ["-1", "1.5", "abc", "NaN"] {
            assert_eq!(
                code(&control.poll("a", Some(bad), Some("0")).await),
                "P2P_TICKET_INVALID",
                "{bad}"
            );
        }
        assert_eq!(code(&control.poll("a", None, Some("abc")).await), "P2P_TICKET_INVALID");
        // A negative wait waits nothing.
        assert!(control.poll("a", None, Some("-50")).await.unwrap().messages.is_empty());
        let first = control.send(&actor(), &message("a", T0)).await.unwrap();
        let second = control.send(&actor(), &message("a", T0)).await.unwrap();
        assert_eq!((first, second), (1, 2));
        let polled = control.poll("a", None, Some("0")).await.unwrap();
        assert_eq!(polled.messages.len(), 2);
        let text = serde_json::to_string(&polled.messages[0]).unwrap();
        assert!(text.starts_with(r#"{"_id":""#), "{text}");
        assert!(text.contains(r#""createdAt":"2026-10-13T12:00:00.000Z""#), "{text}");
        assert!(!text.contains("acknowledgedAt"));
        let after = control.poll("a", Some("1"), Some("0")).await.unwrap();
        assert_eq!(after.messages.len(), 1);
        let id = after.messages[0].message_id.clone();
        assert_eq!(code(&control.ack("b", &id).await), "P2P_SESSION_NOT_FOUND");
        assert_eq!(code(&control.ack("a", "short").await), "P2P_TICKET_INVALID");
        assert!(control.ack("a", &id).await.unwrap().acknowledged);
        // Another account's cursor is stale: answered from the start, unacknowledged only.
        let stale = control.poll("a", Some("9"), Some("0")).await.unwrap();
        assert_eq!(stale.stale_cursor, Some(true));
        assert_eq!(stale.messages.len(), 1);
        clock.advance(600_000);
        assert!(control.poll("a", None, Some("0")).await.unwrap().messages.is_empty());
    }
}
