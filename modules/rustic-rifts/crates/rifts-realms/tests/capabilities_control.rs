//! `p2p/hostCapabilities.test.ts` and `p2p/p2pControl.test.ts` on both engines: minting, checking and resolving a
//! host capability, the runtime identity rules, the bearer parser, and the control channel's cursor.
#![allow(clippy::unwrap_used)]

mod common;

use std::sync::Arc;

use common::{actor, hunt_session, participant, targets};
use rifts_realms::RealmsError;
use rifts_realms::capabilities::{
    HostCapabilities, SignedUser, generate_capability_secret, hash_capability_secret, is_opaque_capability,
    parse_optional_bearer, request_identity, runtime_identity,
};
use rifts_realms::control::ControlChannel;
use rifts_realms::hunt::{AdmissionState, HuntState};
use rifts_realms::ports::ManualClock;
use rifts_realms::repo::hunts::{HuntRepository, NewControlMessage};
use rifts_seam::{CapabilityScope, UnixMillis};

const SESSION: &str = "0d6f2b9c-5b1e-4c3a-9e57-2f1a8b7c4d10";
const NOW: UnixMillis = UnixMillis(1_791_892_800_000);
const HOUR: i64 = 3_600_000;

fn refusal<T: std::fmt::Debug>(result: Result<T, RealmsError>) -> String {
    match result {
        Err(RealmsError::Refused(r)) => r.code,
        other => panic!("{other:?}"),
    }
}

fn runtime_scopes() -> Vec<CapabilityScope> {
    vec![CapabilityScope::GameServerRuntime, CapabilityScope::EventsWrite]
}

#[test]
fn secrets_are_43_character_base64url_and_hash_to_a_stable_digest() {
    let (secret, digest) = generate_capability_secret();
    assert!(is_opaque_capability(secret.expose()));
    assert_eq!(
        hash_capability_secret(secret.expose()).as_deref(),
        Some(digest.as_str())
    );
    assert_eq!(digest.len(), 64);
    assert_ne!(generate_capability_secret().1, digest);
    assert!(hash_capability_secret("short").is_none());
    assert!(hash_capability_secret(&"a".repeat(44)).is_none());
    assert!(hash_capability_secret(&format!("{}!", "a".repeat(42))).is_none());
}

#[test]
fn the_bearer_parser_accepts_one_token_and_refuses_anything_malformed() {
    assert_eq!(parse_optional_bearer(None), Ok(None));
    assert_eq!(parse_optional_bearer(Some("Bearer abc")), Ok(Some("abc")));
    assert_eq!(parse_optional_bearer(Some("bearer abc")), Ok(Some("abc")));
    for bad in ["abc", "Bearer", "Bearer ", "Basic abc", "Bearer a b", "Bearer  abc"] {
        assert!(parse_optional_bearer(Some(bad)).is_err(), "{bad:?}");
    }
}

#[test]
fn a_per_player_route_names_its_account_from_params_first_then_three_bodies() {
    let params = |k: &str, v: &str| vec![(k.to_owned(), v.to_owned())];
    assert_eq!(
        request_identity(&params("userId", " acct "), "/x", None).as_deref(),
        Some("acct")
    );
    assert_eq!(
        request_identity(&params("phxAccountId", "p"), "/x", None).as_deref(),
        Some("p")
    );
    assert_eq!(request_identity(&params("userId", "  "), "/x", None), None);
    let body = serde_json::json!({ "accountId": " body " });
    assert_eq!(
        request_identity(&[], "/Inventory", Some(&body)).as_deref(),
        Some("body")
    );
    assert_eq!(
        request_identity(&[], "/character", Some(&body)).as_deref(),
        Some("body")
    );
    assert_eq!(
        request_identity(&[], "/inventory/instanceditem", Some(&body)).as_deref(),
        Some("body")
    );
    assert_eq!(request_identity(&[], "/other", Some(&body)), None);
    assert_eq!(request_identity(&[], "/inventory", Some(&serde_json::json!([1]))), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_minted_capability_checks_only_for_its_session_scope_and_lifetime() {
    for target in targets() {
        let db = target.open();
        let hunts = HuntRepository::new(db.clone());
        let caps = HostCapabilities::new(db);
        let hunt = hunt_session(SESSION, HuntState::Starting);
        hunts.create_hunt(&actor(), &hunt).await.unwrap();
        let (secret, record) = caps
            .mint(&hunt, "bridge", runtime_scopes(), NOW, UnixMillis(NOW.0 + HOUR))
            .await
            .unwrap();
        let header = Some(secret.expose());

        let ok = caps
            .check(header, SESSION, CapabilityScope::EventsWrite, NOW)
            .await
            .unwrap();
        assert_eq!(ok.capability_id, record.capability_id);
        assert_eq!(
            refusal(caps.check(header, SESSION, CapabilityScope::Abort, NOW).await),
            "P2P_CAPABILITY_INVALID"
        );
        assert_eq!(
            refusal(caps.check(header, "other", CapabilityScope::EventsWrite, NOW).await),
            "P2P_CAPABILITY_INVALID"
        );
        assert_eq!(
            refusal(caps.check(None, SESSION, CapabilityScope::EventsWrite, NOW).await),
            "P2P_CAPABILITY_INVALID"
        );
        assert_eq!(
            refusal(
                caps.check(Some("junk"), SESSION, CapabilityScope::EventsWrite, NOW)
                    .await
            ),
            "P2P_CAPABILITY_INVALID"
        );
        let late = UnixMillis(NOW.0 + HOUR);
        assert_eq!(
            refusal(caps.check(header, SESSION, CapabilityScope::EventsWrite, late).await),
            "P2P_CAPABILITY_INVALID"
        );

        caps.revoke(SESSION, 1, NOW).await.unwrap();
        assert_eq!(
            refusal(caps.check(header, SESSION, CapabilityScope::EventsWrite, NOW).await),
            "P2P_CAPABILITY_INVALID"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_runtime_capability_resolves_only_while_its_exact_generation_is_live() {
    for target in targets() {
        let db = target.open();
        let hunts = HuntRepository::new(db.clone());
        let caps = HostCapabilities::new(db);
        let mut hunt = hunt_session(SESSION, HuntState::Running);
        hunt.roster = vec![
            participant("host", AdmissionState::Admitted, true),
            participant("pal", AdmissionState::Admitted, true),
        ];
        hunts.create_hunt(&actor(), &hunt).await.unwrap();
        let (secret, _) = caps
            .mint(&hunt, "bridge", runtime_scopes(), NOW, UnixMillis(NOW.0 + HOUR))
            .await
            .unwrap();
        let header = Some(secret.expose());

        let context = caps.resolve_runtime(header, NOW).await.unwrap().unwrap();
        assert_eq!(context.present, None);
        let identity = runtime_identity(&context, SignedUser::Absent, None).unwrap();
        assert_eq!(identity.user_id, "host");
        assert_eq!(identity.roster, ["host", "pal"]);
        assert_eq!(
            runtime_identity(&context, SignedUser::Absent, Some(" pal "))
                .unwrap()
                .user_id,
            "pal"
        );
        assert!(runtime_identity(&context, SignedUser::Absent, Some("stranger")).is_none());
        assert!(runtime_identity(&context, SignedUser::Missing, None).is_none());
        assert_eq!(
            runtime_identity(&context, SignedUser::User("pal"), Some("host"))
                .unwrap()
                .user_id,
            "pal"
        );
        let auth = caps.runtime_auth(&context, &identity).unwrap();
        assert_eq!(auth.generation, 1);
        assert_eq!(auth.roster.len(), 2);

        // No digest, a junk one, an expired one or one without the runtime scope resolves to nothing
        assert!(caps.resolve_runtime(None, NOW).await.unwrap().is_none());
        assert!(caps.resolve_runtime(Some("junk"), NOW).await.unwrap().is_none());
        assert!(
            caps.resolve_runtime(header, UnixMillis(NOW.0 + HOUR))
                .await
                .unwrap()
                .is_none()
        );
        let (narrow, _) = caps
            .mint(
                &hunt,
                "bridge",
                vec![CapabilityScope::Abort],
                NOW,
                UnixMillis(NOW.0 + HOUR),
            )
            .await
            .unwrap();
        assert!(
            caps.resolve_runtime(Some(narrow.expose()), NOW)
                .await
                .unwrap()
                .is_none()
        );

        // A drained hunt keeps serving; a completed one does not
        let draining = hunts
            .compare_and_set_hunt(&actor(), SESSION, 1, &[HuntState::Running], 1, |h| {
                h.state = HuntState::Draining
            })
            .await
            .unwrap();
        assert_eq!(draining.unwrap().state_version, 2);
        assert!(caps.resolve_runtime(header, NOW).await.unwrap().is_some());
        let done = hunts
            .compare_and_set_hunt(&actor(), SESSION, 1, &[HuntState::Draining], 2, |h| {
                h.state = HuntState::Completed
            })
            .await
            .unwrap();
        assert!(done.is_some());
        assert!(caps.resolve_runtime(header, NOW).await.unwrap().is_none());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hunt_runtime_resolves_an_unadmitted_player_to_nothing_but_the_host_always() {
    for target in targets() {
        let db = target.open();
        let hunts = HuntRepository::new(db.clone());
        let caps = HostCapabilities::new(db);
        let mut hunt = hunt_session(SESSION, HuntState::Admitting);
        hunt.roster = vec![
            participant("host", AdmissionState::Pending, false),
            participant("pal", AdmissionState::Pending, false),
        ];
        hunts.create_hunt(&actor(), &hunt).await.unwrap();
        let (secret, _) = caps
            .mint(&hunt, "bridge", runtime_scopes(), NOW, UnixMillis(NOW.0 + HOUR))
            .await
            .unwrap();
        let context = caps.resolve_runtime(Some(secret.expose()), NOW).await.unwrap().unwrap();
        assert!(runtime_identity(&context, SignedUser::Absent, Some("pal")).is_none());
        assert!(runtime_identity(&context, SignedUser::Absent, Some("host")).is_some());
    }
}

fn message(account: &str, kind: &str, created: i64, expires: i64) -> NewControlMessage {
    NewControlMessage {
        account_id: account.to_owned(),
        message_type: kind.to_owned(),
        session_id: None,
        generation: None,
        payload: serde_json::json!({ "n": kind }),
        created_at: UnixMillis(created),
        expires_at: UnixMillis(expires),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn control_messages_poll_in_sequence_acknowledge_and_expire() {
    for target in targets() {
        let channel = ControlChannel::new(target.open(), Arc::new(ManualClock::at(NOW)));
        let live = NOW.0 + HOUR;
        for kind in ["one", "two", "three"] {
            channel
                .send(&actor(), &message("acct", kind, NOW.0, live))
                .await
                .unwrap();
        }
        channel
            .send(&actor(), &message("acct", "gone", NOW.0, NOW.0 - 1))
            .await
            .unwrap();
        channel
            .send(&actor(), &message("other", "theirs", NOW.0, live))
            .await
            .unwrap();

        let first = channel.poll("acct", None, Some("0")).await.unwrap();
        let kinds: Vec<&str> = first.messages.iter().map(|m| m.message_type.as_str()).collect();
        assert_eq!(
            kinds,
            ["one", "two", "three"],
            "expired and foreign messages are not delivered"
        );
        assert_eq!(first.stale_cursor, None);
        let second = first.messages[1].sequence;
        let after = channel
            .poll("acct", Some(&second.to_string()), Some("0"))
            .await
            .unwrap();
        assert_eq!(after.messages.len(), 1);
        assert_eq!(after.messages[0].message_type, "three");

        let id = first.messages[0].message_id.clone();
        assert_eq!(channel.ack("acct", &id).await.unwrap().message_id, id);
        assert_eq!(refusal(channel.ack("other", &id).await), "P2P_SESSION_NOT_FOUND");
        assert_eq!(refusal(channel.ack("acct", "nope").await), "P2P_TICKET_INVALID");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_cursor_beyond_the_accounts_last_message_is_answered_from_the_start_and_flagged() {
    for target in targets() {
        let channel = ControlChannel::new(target.open(), Arc::new(ManualClock::at(NOW)));
        let live = NOW.0 + HOUR;
        for kind in ["one", "two"] {
            channel
                .send(&actor(), &message("acct", kind, NOW.0, live))
                .await
                .unwrap();
        }
        let stale = channel.poll("acct", Some("999999"), Some("0")).await.unwrap();
        assert_eq!(stale.stale_cursor, Some(true));
        assert_eq!(stale.messages.len(), 2);
        let id = stale.messages[0].message_id.clone();
        channel.ack("acct", &id).await.unwrap();
        let again = channel.poll("acct", Some("999999"), Some("0")).await.unwrap();
        assert_eq!(again.messages.len(), 1, "a stale answer lists only the unacknowledged");
        let empty = channel.poll("nobody", Some("5"), Some("0")).await.unwrap();
        assert_eq!((empty.messages.len(), empty.stale_cursor), (0, None));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_malformed_cursor_is_refused_and_a_negative_wait_is_zero() {
    for target in targets() {
        let channel = ControlChannel::new(target.open(), Arc::new(ManualClock::at(NOW)));
        for bad in ["-1", "1.5", "abc", "NaN"] {
            assert_eq!(
                refusal(channel.poll("acct", Some(bad), Some("0")).await),
                "P2P_TICKET_INVALID",
                "{bad}"
            );
        }
        assert_eq!(
            refusal(channel.poll("acct", None, Some("abc")).await),
            "P2P_TICKET_INVALID"
        );
        let started = std::time::Instant::now();
        channel.poll("acct", None, Some("-50")).await.unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }
}
