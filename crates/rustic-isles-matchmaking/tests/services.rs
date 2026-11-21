// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The outbox, the QoS listener, the Director over HTTP against an in-test fake, the rifts core and the module hooks
//! (spec §17.2).

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use dainty_module_api::chat::Headline;
use dainty_module_api::{BoxFuture, GlobalId, IdKind, NodeId};
use rustic_isles::{DedicatedServers, Launch, LaunchRequest, Settings, Startup, Unconfigured};
use rustic_isles_matchmaking::test_support::{FakeDirector, TestModule};
use rustic_isles_matchmaking::{
    HeadlineSink, HttpDirector, MatchmakingCore, MatchmakingState, Parties, QosListener, outbox_pass,
    place_hunt_players, player_offline, record_ring_instance,
};
use rustic_isles_store::repo::parties::{PartyEvent, PartyEventType};
use rustic_isles_store::test_support::TempStore;
use rustic_isles_store::{IslesStore, UserId};
use rustic_rifts_seam::{AccountId as RealmAccount, Candidate, IslesCore, Json, NoCore, PartyId, SeamError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

// -------------------------------------------------------------------- the outbox

#[derive(Default)]
struct Sink {
    sent: Mutex<Vec<Headline>>,
    fail: bool,
}

impl HeadlineSink for Sink {
    fn emit<'a>(&'a self, headline: &'a Headline) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            if self.fail {
                return Err("the bus is down".into());
            }
            self.sent.lock().unwrap().push(headline.clone());
            Ok(())
        })
    }
}

async fn event(store: &IslesStore, party: &str, recipients: &[&str], at: &str) {
    let event = PartyEvent {
        party_id: party.into(),
        party_revision: 3,
        event_type: PartyEventType::PartyInviteCreated,
        recipients: recipients.iter().map(|r| UserId::new(*r)).collect(),
        payload: serde_json::json!({"accountId": "bob"}),
        now: at.into(),
    };
    store
        .write(&common::system(), move |tx, repos| {
            repos.parties.append_event(tx, &event)
        })
        .await
        .unwrap();
}

const T0: i64 = 1_793_000_000_000;

#[tokio::test(flavor = "multi_thread")]
async fn the_outbox_publishes_in_order_once() {
    let temp = TempStore::open().unwrap();
    let alice = common::account(&temp.store, "alice").await;
    let at = rustic_isles::season::to_iso_string(T0);
    event(&temp.store, "p1", &["alice", "ghost"], &at).await;
    event(&temp.store, "p2", &["alice"], &at).await;
    let sink = Sink::default();
    assert_eq!(outbox_pass(&temp.store, &sink, "owner-1", T0).await.unwrap(), 2);
    assert_eq!(outbox_pass(&temp.store, &sink, "owner-1", T0).await.unwrap(), 0);
    let sent = sink.sent.lock().unwrap().clone();
    let parties: Vec<String> = sent
        .iter()
        .map(|h| {
            serde_json::from_str::<serde_json::Value>(&h.body).unwrap()["partyId"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(parties, ["p1", "p2"]);
    // A recipient without an account is left out
    assert_eq!(sent[0].recipients.len(), 1);
    assert_eq!(
        (&sent[0].recipients[0].account, sent[0].recipients[0].user_id.as_str()),
        (alice.global(), "alice")
    );
    let expected = rustic_isles_chat::headline::public_event(
        &sent[0].id,
        "PARTY_INVITE_CREATED",
        &at,
        "p1",
        3,
        &serde_json::json!({"accountId": "bob"}),
    );
    assert_eq!(sent[0].body, dainty_service_kit::jsjson::stringify(&expected));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_publish_is_released_and_stops_the_pass() {
    let temp = TempStore::open().unwrap();
    common::account(&temp.store, "alice").await;
    let at = rustic_isles::season::to_iso_string(T0);
    event(&temp.store, "p1", &["alice"], &at).await;
    event(&temp.store, "p2", &["alice"], &at).await;
    let failing = Sink {
        fail: true,
        ..Sink::default()
    };
    assert_eq!(outbox_pass(&temp.store, &failing, "owner-1", T0).await.unwrap(), 0);
    let sink = Sink::default();
    assert_eq!(outbox_pass(&temp.store, &sink, "owner-2", T0).await.unwrap(), 2);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_stale_claim_is_taken_over() {
    let temp = TempStore::open().unwrap();
    common::account(&temp.store, "alice").await;
    let at = rustic_isles::season::to_iso_string(T0);
    event(&temp.store, "p1", &["alice"], &at).await;
    let stale = rustic_isles::season::to_iso_string(T0 - 30_000);
    let claimed = temp
        .store
        .write(&common::system(), move |tx, repos| {
            repos.parties.claim_next_outbox(tx, "crashed", &at, &stale)
        })
        .await
        .unwrap();
    assert!(claimed.is_some());
    let sink = Sink::default();
    assert_eq!(outbox_pass(&temp.store, &sink, "other", T0 + 10_000).await.unwrap(), 0);
    assert_eq!(outbox_pass(&temp.store, &sink, "other", T0 + 30_001).await.unwrap(), 1);
}

// -------------------------------------------------------------------- the QoS listener

#[tokio::test(flavor = "multi_thread")]
async fn the_qos_listener_answers_every_path() {
    let module = TestModule::new(None, Settings::from_pairs([("QOS_PORT", "8081")]));
    let started = QosListener { bind_port: 0 }.start(&module.shared).await.unwrap();
    let listener = started.listeners.into_iter().next().unwrap();
    assert_eq!(listener.addresses[0].0, "qos");
    assert_eq!(listener.addresses[1], ("qos_public_port".to_owned(), "8081".to_owned()));
    let address: SocketAddr = listener.addresses[0].1.parse().unwrap();
    assert!(address.ip().is_loopback());
    for (method, path) in [("GET", "/"), ("POST", "/anything/at/all"), ("GET", "/QoS")] {
        let reply = rustic_isles::testing::request(address, method, path).await.unwrap();
        assert_eq!(reply.status, 200);
        assert_eq!(reply.header("content-type"), Some("text/html"));
        assert_eq!(reply.body, "<!DOCTYPE html><html><body>pong</body></html>");
    }
    (listener.stop)();
    let _ = listener.task.await;
    let off = TestModule::new(None, Settings::from_pairs([("QOS_PORT", "off")]));
    assert!(
        QosListener { bind_port: 0 }
            .start(&off.shared)
            .await
            .unwrap()
            .listeners
            .is_empty()
    );
}

// -------------------------------------------------------------------- the Director over HTTP

/// Answers one connection with `answer` and hands back what it read.
async fn fake_director(answer: &'static [u8]) -> (SocketAddr, tokio::task::JoinHandle<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut read = Vec::new();
        let mut buffer = [0u8; 4096];
        loop {
            let n = stream.read(&mut buffer).await.unwrap();
            read.extend_from_slice(&buffer[..n]);
            let text = String::from_utf8_lossy(&read).into_owned();
            if let Some(split) = text.find("\r\n\r\n") {
                let length = text[..split]
                    .lines()
                    .find_map(|l| l.strip_prefix("Content-Length: "))
                    .map_or(0, |v| v.trim().parse::<usize>().unwrap());
                if read.len() >= split + 4 + length {
                    break;
                }
            }
            if n == 0 {
                break;
            }
        }
        stream.write_all(answer).await.unwrap();
        String::from_utf8(read).unwrap()
    });
    (address, task)
}

#[tokio::test]
async fn the_director_launches_over_http() {
    let (address, seen) = fake_director(
        b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"host\":\"203.0.113.10\",\"port\":7777}",
    )
    .await;
    let director = HttpDirector::new(&address.to_string());
    let request = LaunchRequest {
        game_mode: Some("ISLAND".into()),
        game_args: None,
        hunt_id: "ShatteredIsles_IslandA".into(),
        expected_players: vec!["a".into(), "b".into()],
        gauntlet_level: None,
        request_id: Some("trace-0042-alpha".into()),
    };
    assert_eq!(
        director.launch(request).await,
        Ok(Launch::Online {
            host: "203.0.113.10".into(),
            port: 7777
        })
    );
    let seen = seen.await.unwrap();
    let body = r#"{"GameMode":"ISLAND","HuntId":"ShatteredIsles_IslandA","ExpectedPlayers":["a","b"]}"#;
    assert_eq!(
        seen,
        format!(
            "POST /api/matchmaker/handle-matchmaking-for-player HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Type: application/json\r\nx-rustic-isles-request-id: trace-0042-alpha\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
    );
}

#[tokio::test]
async fn director_refusals_and_failures() {
    let launch = || LaunchRequest {
        hunt_id: "h".into(),
        ..LaunchRequest::default()
    };
    let (address, _) = fake_director(b"HTTP/1.1 503 Busy\r\n\r\n").await;
    assert_eq!(
        HttpDirector::new(&address.to_string()).launch(launch()).await,
        Ok(Launch::Refused { reason: None })
    );
    let (address, _) = fake_director(b"HTTP/1.1 200 OK\r\n\r\nnot json").await;
    let error = HttpDirector::new(&address.to_string())
        .launch(launch())
        .await
        .unwrap_err();
    assert!(error.starts_with("invalid DeployServer JSON: "), "{error}");
    let closed = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap()
        .local_addr()
        .unwrap();
    let error = HttpDirector::new(&closed.to_string())
        .launch(launch())
        .await
        .unwrap_err();
    assert!(error.starts_with("fetch failed: "), "{error}");
}

#[tokio::test]
async fn the_director_reports_its_hubs() {
    let (address, seen) = fake_director(
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2d\r\n{\"ramsgate\":{\"ready\":true,\"port\":8791},\"x\":1}\r\n0\r\n\r\n",
    )
    .await;
    let status = HttpDirector::new(&address.to_string()).hub_status().await.unwrap();
    let ramsgate = status.ramsgate.unwrap();
    assert_eq!(
        (ramsgate.ready, ramsgate.host.as_str(), ramsgate.port),
        (true, "", 8791)
    );
    assert!(status.training.is_none());
    assert!(seen.await.unwrap().starts_with("GET /api/hub-status HTTP/1.1\r\n"));
    let (address, _) = fake_director(b"HTTP/1.1 500 No\r\n\r\n").await;
    assert!(HttpDirector::new(&address.to_string()).hub_status().await.is_none());
}

// -------------------------------------------------------------------- the rifts core

fn node() -> NodeId {
    NodeId::new("local").unwrap()
}

fn refusal_code(error: SeamError) -> String {
    match error {
        SeamError::Refused(refusal) => refusal.code,
        other => panic!("not a refusal: {other}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_rifts_core() {
    let state = Arc::new(MatchmakingState::new());
    let bare = MatchmakingCore::new(None, node(), Arc::clone(&state), Arc::new(NoCore));
    let unknown: PartyId = format!("{}@local", GlobalId::new(IdKind::Party, &node()).ulid())
        .parse()
        .unwrap();
    assert_eq!(
        refusal_code(bare.party(unknown.clone()).await.unwrap_err()),
        "NO_DATABASE"
    );
    let user = || RealmAccount("alice".into());
    let bad = bare.set_candidate(user(), Candidate(Json("{".into()))).unwrap_err();
    assert_eq!(refusal_code(bad), "INVALID_CANDIDATE");
    let partial = bare
        .set_candidate(user(), Candidate(Json(r#"{"HuntId":"h"}"#.into())))
        .unwrap_err();
    assert_eq!(refusal_code(partial), "INVALID_CANDIDATE");
    let good = r#"{"Ready":false,"HuntId":"h","CandidateId":"c","GameSessionId":"c"}"#;
    bare.set_candidate(user(), Candidate(Json(good.into()))).unwrap();
    assert_eq!(state.candidate("alice").unwrap().candidate_id, "c");
    let accounts = [user(), RealmAccount("bob".into()), user()];
    assert_eq!(bare.cancel_travel("h", &accounts, "REALM_FULL"), Ok(true));
    assert!(state.candidate("bob").unwrap().canceled);
    bare.touch_game_session(user()).await.unwrap();
    assert!(state.presence().is_recently_active("alice", 1000, state.now_ms()));
    assert!(
        matches!(bare.display_name(user()).await, Err(SeamError::Unimplemented(m)) if m.operation == "display_name")
    );
    assert!(
        matches!(bare.stacked_items("c".into()).await, Err(SeamError::Unimplemented(m)) if m.operation == "stacked_items")
    );
    assert!(bare.host_grant_catalog().is_none());

    // The Director slot is the bound module's
    let module = TestModule::new(None, Settings::default());
    let core = MatchmakingCore::new(None, node(), Arc::clone(&module.state), Arc::new(NoCore));
    assert!(!core.dedicated_servers_configured());
    module.shared.set_dedicated_servers(FakeDirector::online("h", 1));
    assert!(core.dedicated_servers_configured());
    module.shared.set_dedicated_servers(Arc::new(Unconfigured));
    assert!(!core.dedicated_servers_configured());

    let temp = TempStore::open().unwrap();
    common::account(&temp.store, "alice").await;
    let stored = MatchmakingCore::new(Some(temp.store.clone()), node(), state, Arc::new(NoCore));
    assert_eq!(stored.party(unknown).await, Ok(None));
    let party = stored.get_or_create_party(user()).await.unwrap();
    assert_eq!(
        (party.leader.0.as_str(), party.members.len(), party.revision),
        ("alice", 1, 1)
    );
    assert_eq!(stored.party(party.id.clone()).await.unwrap(), Some(party));
    assert_eq!(
        refusal_code(
            stored
                .get_or_create_party(RealmAccount("ghost".into()))
                .await
                .unwrap_err()
        ),
        "PARTY_WRITE_FAILED"
    );
}

// -------------------------------------------------------------------- the module hooks

#[tokio::test(flavor = "multi_thread")]
async fn the_director_invocation_hooks() {
    let temp = TempStore::open().unwrap();
    let alice = common::account(&temp.store, "alice").await;
    let module = TestModule::new(Some(temp.store.clone()), Settings::default());
    let foreign = GlobalId::new(IdKind::Account, &NodeId::new("germany-1").unwrap());
    let unknown = GlobalId::new(IdKind::Account, &node());
    let players = [alice.global().clone(), foreign.clone(), unknown.clone()];
    let users = place_hunt_players(&module.shared, &players).await;
    assert_eq!(users, ["alice".to_owned(), foreign.to_string(), unknown.to_string()]);
    let hunt = GlobalId::new(IdKind::Hunt, &node());
    let hub = GlobalId::new(IdKind::HubInstance, &node());
    record_ring_instance(&module.shared, &hunt, &users);
    record_ring_instance(&module.shared, &hub, &[]);
    assert_eq!(module.state.ring_instance_for_player("alice"), Some(hunt));
    assert_eq!(module.state.ring_instance_for_player("someone"), Some(hub));

    // A player the chat plugin saw go leaves their party
    let parties = Parties {
        store: Some(&temp.store),
        node: &node(),
        request_id: None,
        now: module.state.now_ms(),
    };
    parties.get_or_create("alice", "").await.unwrap();
    assert!(parties.for_player("alice").await.unwrap().is_some());
    player_offline(&module.shared, "alice").await;
    assert!(parties.for_player("alice").await.unwrap().is_none());
}
