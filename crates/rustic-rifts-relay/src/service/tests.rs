// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, Ordering};

use rustic_rifts_seam::{BoxFuture, GlobalId, NodeId, Secret, Ulid, poll_once};
use serde_json::json;

use super::*;
use crate::config::{RelayConfig, TurnProvider, TurnRest};
use crate::credentials::{ProviderAnswer, ProviderHttp, ProviderRequest, ProviderUnreachable, RelayUsageCache};
use crate::rules::{HuntState, PresenceState, RosterEntry, SessionKind, derive_pair_key};
use crate::wire::iso_time;

const NOW: i64 = 1_791_878_400_000;
const WIRE: &str = "01234567-89ab-4def-8123-456789abcdef";

struct FakeHunts {
    gate: Mutex<P2PGate>,
    hunt: Mutex<Option<HuntSnapshot>>,
    presence: Mutex<Vec<PresenceRow>>,
    presence_limits: Mutex<Vec<usize>>,
    queued: Mutex<Vec<ControlMessage>>,
}

impl IceHunts for FakeHunts {
    fn gate(&self) -> Result<P2PGate, RelayError> {
        Ok(*self.gate.lock().unwrap())
    }

    fn find_hunt(&self, session: &SessionId) -> Result<Option<HuntSnapshot>, RelayError> {
        Ok(if session.0.ulid == Ulid(1) {
            self.hunt.lock().unwrap().clone()
        } else {
            None
        })
    }

    fn hub_presence(&self, _: &HuntSnapshot, limit: usize) -> Result<Vec<PresenceRow>, RelayError> {
        self.presence_limits.lock().unwrap().push(limit);
        Ok(self.presence.lock().unwrap().clone())
    }

    fn enqueue(&self, message: ControlMessage) -> Result<(), RelayError> {
        self.queued.lock().unwrap().push(message);
        Ok(())
    }
}

/// Key id, transport, expected version, reason and request id of one call.
type PolicyCall = (String, CoopTransport, u64, String, String);

#[derive(Default)]
struct FakePolicy {
    conflict: bool,
    calls: Mutex<Vec<PolicyCall>>,
}

impl OperationsPolicy for FakePolicy {
    fn set_coop_transport(
        &self,
        admin_key_id: &str,
        transport: CoopTransport,
        expected_version: u64,
        reason: &str,
        request_id: &str,
    ) -> Result<CoopTransportOutcome, RelayError> {
        self.calls.lock().unwrap().push((
            admin_key_id.to_owned(),
            transport,
            expected_version,
            reason.to_owned(),
            request_id.to_owned(),
        ));
        let policy = Json(format!(
            r#"{{"coopTransport":"{}","version":{}}}"#,
            transport.as_str(),
            expected_version + 1
        ));
        Ok(if self.conflict {
            CoopTransportOutcome::Conflict(policy)
        } else {
            CoopTransportOutcome::Updated(policy)
        })
    }
}

struct NoHttp;

impl ProviderHttp for NoHttp {
    fn post(&self, _: ProviderRequest) -> BoxFuture<'_, Result<ProviderAnswer, ProviderUnreachable>> {
        Box::pin(std::future::ready(Err(ProviderUnreachable)))
    }
}

fn id(name: &str) -> AccountId {
    AccountId(name.to_owned())
}

fn session(ulid: u128) -> SessionId {
    SessionId(GlobalId {
        ulid: Ulid(ulid),
        node: NodeId::new("node-a").unwrap(),
    })
}

fn secret() -> Secret {
    Secret::new("transport-key-secret-0123456789abcdef".to_owned())
}

fn hunt() -> HuntSnapshot {
    HuntSnapshot {
        wire_session: WIRE.to_owned(),
        generation: 3,
        state: HuntState::Running,
        kind: SessionKind::Hunt,
        realm: None,
        coop_transport: CoopTransport::Ice,
        host_account: id("host-0001"),
        host_peer: PeerId(11),
        expires_at: UnixMillis(NOW + 3_600_000),
        roster: vec![
            RosterEntry {
                account: id("host-0001"),
                peer: PeerId(11),
                returned_to_hub: false,
            },
            RosterEntry {
                account: id("joiner-01"),
                peer: PeerId(12),
                returned_to_hub: false,
            },
            RosterEntry {
                account: id("joiner-02"),
                peer: PeerId(13),
                returned_to_hub: false,
            },
        ],
    }
}

fn relay_config() -> RelayConfig {
    RelayConfig {
        stun_urls: vec!["stun:stun.example:3478".to_owned()],
        provider: TurnProvider::TurnRest,
        turn_rest: Some(TurnRest {
            urls: vec!["turn:relay.example:3478".to_owned()],
            secret: Secret::new("0123456789abcdef".to_owned()),
        }),
        transport_key_secret: Some(secret()),
        configs_per_hour: 2,
        ..RelayConfig::default()
    }
}

struct Fixture {
    ice: Ice,
    hunts: Arc<FakeHunts>,
    policy: Arc<FakePolicy>,
    clock: Arc<AtomicI64>,
}

fn fixture_with(config: RelayConfig, hunt: HuntSnapshot, conflict: bool) -> Fixture {
    let hunts = Arc::new(FakeHunts {
        gate: Mutex::new(P2PGate {
            enabled: true,
            emergency_stop: false,
        }),
        hunt: Mutex::new(Some(hunt)),
        presence: Mutex::default(),
        presence_limits: Mutex::default(),
        queued: Mutex::default(),
    });
    let policy = Arc::new(FakePolicy {
        conflict,
        calls: Mutex::default(),
    });
    let clock = Arc::new(AtomicI64::new(NOW));
    let grants = RelayGrants {
        config: Arc::new(config),
        http: Arc::new(NoHttp),
        usage: Arc::new(RelayUsageCache::default()),
    };
    let ticking = Arc::clone(&clock);
    let ice = Ice::new(
        Arc::clone(&hunts) as Arc<dyn IceHunts>,
        grants,
        Arc::clone(&policy) as Arc<dyn OperationsPolicy>,
        Arc::new(move || UnixMillis(ticking.load(Ordering::SeqCst))),
    );
    Fixture {
        ice,
        hunts,
        policy,
        clock,
    }
}

fn fixture() -> Fixture {
    fixture_with(relay_config(), hunt(), false)
}

fn config(fixture: &Fixture, account: &str, generation: Option<i64>) -> Result<Json, RelayError> {
    poll_once(fixture.ice.transport_config(id(account), session(1), generation)).unwrap()
}

fn signal(fixture: &Fixture, account: &str, body: Value) -> Result<Json, RelayError> {
    poll_once(fixture.ice.submit_signal(id(account), session(1), body)).unwrap()
}

fn code(result: Result<Json, RelayError>) -> String {
    match result {
        Err(RelayError::Refused(refusal)) => refusal.code,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn pair(joiner: &str) -> String {
    derive_pair_key(&secret(), WIRE, 3, &id("host-0001"), &id(joiner))
        .expose()
        .to_owned()
}

#[test]
fn joiner_gets_stun_turn_and_its_host() {
    let fixture = fixture();
    let body = config(&fixture, "joiner-01", Some(3)).unwrap().0;
    let parsed: Value = serde_json::from_str(&body).unwrap();
    let username = parsed["iceServers"][1]["username"].as_str().unwrap().to_owned();
    let credential = parsed["iceServers"][1]["credential"].as_str().unwrap().to_owned();
    // An hour left plus ten minutes.
    let expires = iso_time(UnixMillis(NOW + 4_200_000));
    let expected = format!(
        concat!(
            r#"{{"sessionId":"{WIRE}","generation":3,"role":"JOIN","localAccountId":"joiner-01","localPeerId":"12","#,
            r#""hostAccountId":"host-0001","hostPeerId":"11","forceRelay":false,"#,
            r#""iceServers":[{{"urls":["stun:stun.example:3478"]}},{{"urls":["turn:relay.example:3478"],"#,
            r#""username":"{username}","credential":"{credential}"}}],"#,
            r#""relay":{{"allowed":true,"ttlSeconds":4200,"expiresAt":"{expires}"}},"#,
            r#""peers":[{{"accountId":"host-0001","role":"HOST","transportKey":"{key}","peerId":"11"}}]}}"#
        ),
        WIRE = WIRE,
        username = username,
        credential = credential,
        expires = expires,
        key = pair("joiner-01"),
    );
    assert_eq!(body, expected);
    assert!(username.starts_with(&format!("{}:", NOW / 1000 + 4200)));
}

#[test]
fn host_gets_every_active_joiner() {
    let fixture = fixture();
    let parsed: Value = serde_json::from_str(&config(&fixture, "host-0001", Some(3)).unwrap().0).unwrap();
    assert_eq!(parsed["role"], "HOST");
    assert_eq!(parsed["localPeerId"], "11");
    assert_eq!(
        parsed["peers"],
        json!([
            {"accountId": "joiner-01", "role": "JOIN", "transportKey": pair("joiner-01"), "peerId": "12"},
            {"accountId": "joiner-02", "role": "JOIN", "transportKey": pair("joiner-02"), "peerId": "13"}
        ])
    );
}

#[test]
fn transport_config_refusals_in_order() {
    let fixture = fixture();
    *fixture.hunts.gate.lock().unwrap() = P2PGate {
        enabled: false,
        emergency_stop: true,
    };
    assert_eq!(code(config(&fixture, "joiner-01", None)), "P2P_DISABLED");
    *fixture.hunts.gate.lock().unwrap() = P2PGate {
        enabled: true,
        emergency_stop: true,
    };
    assert_eq!(code(config(&fixture, "joiner-01", None)), "P2P_EMERGENCY_STOP");
    *fixture.hunts.gate.lock().unwrap() = P2PGate {
        enabled: true,
        emergency_stop: false,
    };

    let keyless = fixture_with(
        RelayConfig {
            transport_key_secret: None,
            ..relay_config()
        },
        hunt(),
        false,
    );
    assert_eq!(code(config(&keyless, "joiner-01", None)), "P2P_ICE_UNAVAILABLE");

    assert_eq!(code(config(&fixture, "joiner-01", None)), "P2P_TICKET_INVALID");
    assert_eq!(
        code(poll_once(fixture.ice.transport_config(id("joiner-01"), session(2), Some(3))).unwrap()),
        "P2P_SESSION_NOT_FOUND"
    );
    let mut steam = hunt();
    steam.coop_transport = CoopTransport::SteamExperimental;
    *fixture.hunts.hunt.lock().unwrap() = Some(steam);
    assert_eq!(code(config(&fixture, "joiner-01", Some(4))), "P2P_TRANSPORT_MISMATCH");
    *fixture.hunts.hunt.lock().unwrap() = Some(hunt());
    assert_eq!(code(config(&fixture, "joiner-01", Some(4))), "P2P_GENERATION_FENCED");
    assert_eq!(code(config(&fixture, "joiner-01", Some(-3))), "P2P_GENERATION_FENCED");
    let mut done = hunt();
    done.state = HuntState::Completed;
    *fixture.hunts.hunt.lock().unwrap() = Some(done);
    assert_eq!(code(config(&fixture, "joiner-01", Some(3))), "P2P_SESSION_NOT_READY");
    *fixture.hunts.hunt.lock().unwrap() = Some(hunt());
    assert_eq!(code(config(&fixture, "stranger", Some(3))), "P2P_NOT_ROSTERED");
    let refused = config(&fixture, "stranger", Some(3)).unwrap_err();
    assert_eq!(
        refused.to_string(),
        "P2P_NOT_ROSTERED: This account is not part of the hunt."
    );
}

#[test]
fn configs_are_limited_per_account() {
    let fixture = fixture();
    assert!(config(&fixture, "joiner-01", Some(3)).is_ok());
    assert!(config(&fixture, "joiner-01", Some(3)).is_ok());
    assert_eq!(code(config(&fixture, "joiner-01", Some(3))), "P2P_RELAY_RATE_LIMITED");
    assert!(config(&fixture, "joiner-02", Some(3)).is_ok());
    fixture.clock.store(NOW + 3_600_000, Ordering::SeqCst);
    assert!(config(&fixture, "joiner-01", Some(3)).is_ok());

    let mut realm = hunt();
    realm.realm = Some("realm-1".to_owned());
    let realm = fixture_with(relay_config(), realm, false);
    for _ in 0..120 {
        assert!(config(&realm, "joiner-01", Some(3)).is_ok());
    }
    assert_eq!(code(config(&realm, "joiner-01", Some(3))), "P2P_RELAY_RATE_LIMITED");
}

fn hub() -> HuntSnapshot {
    HuntSnapshot {
        kind: SessionKind::RealmHub,
        realm: Some("realm-1".to_owned()),
        roster: Vec::new(),
        expires_at: UnixMillis(NOW + 30 * 86_400_000),
        ..hunt()
    }
}

#[test]
fn hubs_use_presence_and_the_hub_ttl() {
    let fixture = fixture_with(relay_config(), hub(), false);
    *fixture.hunts.presence.lock().unwrap() = vec![
        PresenceRow {
            account: id("member-01"),
            peer: PeerId(21),
            state: PresenceState::Admitted,
        },
        PresenceRow {
            account: id("member-02"),
            peer: PeerId(22),
            state: PresenceState::Lost,
        },
    ];
    let parsed: Value = serde_json::from_str(&config(&fixture, "host-0001", Some(3)).unwrap().0).unwrap();
    assert_eq!(parsed["relay"]["ttlSeconds"], 86_400);
    assert_eq!(
        parsed["peers"],
        json!([
            {"accountId": "member-01", "role": "JOIN", "transportKey": pair("member-01"), "peerId": "21"}
        ])
    );
    let member: Value = serde_json::from_str(&config(&fixture, "member-01", Some(3)).unwrap().0).unwrap();
    assert_eq!(member["localPeerId"], "21");
    assert_eq!(code(config(&fixture, "member-02", Some(3))), "P2P_NOT_ROSTERED");
    assert!(
        fixture
            .hunts
            .presence_limits
            .lock()
            .unwrap()
            .iter()
            .all(|limit| *limit == 65)
    );
}

fn candidate_body(to: &str, data: &str) -> Value {
    json!({"generation": 3, "toAccountId": to, "kind": "CANDIDATE", "data": data})
}

const RELAY: &str = "a=candidate:3 1 UDP 1 198.51.100.7 49152 typ relay raddr 0.0.0.0 rport 0";
const HOST: &str = "a=candidate:1 1 UDP 2122 192.168.1.2 5000 typ host";

#[test]
fn signals_are_queued_to_the_other_side() {
    let fixture = fixture();
    let reply = signal(&fixture, "joiner-01", candidate_body("host-0001", HOST))
        .unwrap()
        .0;
    let queued = fixture.hunts.queued.lock().unwrap().clone();
    assert_eq!(queued.len(), 1);
    let message = &queued[0];
    let signal_id = message.payload.0.split("\"signalId\":\"").nth(1).unwrap()[..36].to_owned();
    assert_eq!(reply, format!(r#"{{"delivered":true,"signalId":"{signal_id}"}}"#));
    assert_eq!(message.account, id("host-0001"));
    assert_eq!(message.message_type, "ICE_SIGNAL");
    assert_eq!((message.wire_session.as_str(), message.generation), (WIRE, 3));
    assert_eq!(
        (message.created_at, message.expires_at),
        (UnixMillis(NOW), UnixMillis(NOW + 120_000))
    );
    assert_eq!(
        message.payload.0,
        format!(
            r#"{{"sessionId":"{WIRE}","generation":3,"signalId":"{signal_id}","fromAccountId":"joiner-01","fromRole":"JOIN","kind":"CANDIDATE","data":"{HOST}"}}"#
        )
    );

    let mut ending = hunt();
    ending.expires_at = UnixMillis(NOW + 5000);
    *fixture.hunts.hunt.lock().unwrap() = Some(ending);
    signal(
        &fixture,
        "host-0001",
        json!({"generation": 3, "toAccountId": "joiner-02", "kind": "GATHERING_DONE"}),
    )
    .unwrap();
    let last = fixture.hunts.queued.lock().unwrap().last().cloned().unwrap();
    assert_eq!(last.expires_at, UnixMillis(NOW + 5000));
    assert!(
        last.payload
            .0
            .contains(r#""fromRole":"HOST","kind":"GATHERING_DONE","data":""}"#)
    );
}

#[test]
fn signal_refusals() {
    let fixture = fixture();
    *fixture.hunts.gate.lock().unwrap() = P2PGate {
        enabled: false,
        emergency_stop: false,
    };
    assert_eq!(
        code(signal(&fixture, "joiner-01", candidate_body("host-0001", HOST))),
        "P2P_DISABLED"
    );
    *fixture.hunts.gate.lock().unwrap() = P2PGate {
        enabled: true,
        emergency_stop: false,
    };
    assert_eq!(
        code(signal(&fixture, "joiner-01", json!({"generation": 3}))),
        "P2P_SIGNAL_INVALID"
    );
    let mut stale = candidate_body("host-0001", HOST);
    stale["generation"] = json!(2);
    assert_eq!(code(signal(&fixture, "joiner-01", stale)), "P2P_GENERATION_FENCED");
    let refused = signal(&fixture, "joiner-01", candidate_body("joiner-02", HOST)).unwrap_err();
    assert_eq!(
        refused.to_string(),
        "P2P_NOT_ROSTERED: Signals go only between the host and a joiner of this hunt."
    );
    assert!(fixture.hunts.queued.lock().unwrap().is_empty());
}

#[test]
fn relay_only_filtering() {
    let fixture = fixture_with(
        RelayConfig {
            force_relay: true,
            ..relay_config()
        },
        hunt(),
        false,
    );
    let reply = signal(&fixture, "joiner-01", candidate_body("host-0001", HOST)).unwrap();
    assert_eq!(reply.0, r#"{"delivered":false,"reason":"FILTERED_RELAY_ONLY"}"#);
    assert!(fixture.hunts.queued.lock().unwrap().is_empty());
    signal(
        &fixture,
        "joiner-01",
        candidate_body("host-0001", &format!(" {RELAY}\r\n")),
    )
    .unwrap();
    let queued = fixture.hunts.queued.lock().unwrap().clone();
    assert!(queued[0].payload.0.ends_with(&format!(r#""data":"{RELAY}"}}"#)));
    let config: Value = serde_json::from_str(&config(&fixture, "joiner-01", Some(3)).unwrap().0).unwrap();
    assert_eq!(config["forceRelay"], true);
}

#[test]
fn signals_are_limited_per_sender() {
    let fixture = fixture();
    for _ in 0..300 {
        signal(&fixture, "joiner-01", candidate_body("host-0001", HOST)).unwrap();
    }
    assert_eq!(
        code(signal(&fixture, "joiner-01", candidate_body("host-0001", HOST))),
        "P2P_SIGNAL_LIMIT"
    );
    signal(&fixture, "host-0001", candidate_body("joiner-01", HOST)).unwrap();

    let hub = fixture_with(relay_config(), hub(), false);
    *hub.hunts.presence.lock().unwrap() = vec![PresenceRow {
        account: id("member-01"),
        peer: PeerId(21),
        state: PresenceState::Admitted,
    }];
    for _ in 0..3000 {
        signal(&hub, "host-0001", candidate_body("member-01", HOST)).unwrap();
    }
    assert_eq!(
        code(signal(&hub, "host-0001", candidate_body("member-01", HOST))),
        "P2P_SIGNAL_LIMIT"
    );
    for _ in 0..300 {
        signal(&hub, "member-01", candidate_body("host-0001", HOST)).unwrap();
    }
    assert_eq!(
        code(signal(&hub, "member-01", candidate_body("host-0001", HOST))),
        "P2P_SIGNAL_LIMIT"
    );
}

const VALIDATION: &str = r#"{"error":{"code":"ADMIN_VALIDATION","message":"coopTransport (ICE or STEAM_EXPERIMENTAL), expectedVersion, and an audit reason of at least 8 characters are required."}}"#;

fn coop(fixture: &Fixture, body: Value) -> Reply {
    poll_once(fixture.ice.set_coop_transport("key-1".to_owned(), body))
        .unwrap()
        .unwrap()
}

#[test]
fn coop_transport_validation() {
    let fixture = fixture();
    let good = json!({"coopTransport": "ICE", "expectedVersion": 4, "reason": "rolling back the relay"});
    for (field, value) in [
        ("coopTransport", json!("ice")),
        ("coopTransport", json!(1)),
        ("expectedVersion", json!(-1)),
        ("expectedVersion", json!(1.5)),
        ("expectedVersion", json!("x")),
        ("reason", json!("  short  ")),
        ("reason", json!(12_345_678)),
    ] {
        let mut body = good.clone();
        body[field] = value;
        assert_eq!(
            coop(&fixture, body),
            Reply {
                status: 400,
                body: Json(VALIDATION.to_owned())
            },
            "{field}"
        );
    }
    let mut missing = good.clone();
    missing.as_object_mut().unwrap().remove("expectedVersion");
    assert_eq!(coop(&fixture, missing).status, 400);
    assert_eq!(coop(&fixture, json!([])).status, 400);
    assert!(fixture.policy.calls.lock().unwrap().is_empty());

    let mut null_version = good.clone();
    null_version["expectedVersion"] = Value::Null;
    null_version["reason"] = json!(format!("  {}", "r".repeat(600)));
    assert_eq!(coop(&fixture, null_version).status, 200);
    let mut text_version = good;
    text_version["expectedVersion"] = json!(" 7 ");
    text_version["coopTransport"] = json!("STEAM_EXPERIMENTAL");
    assert_eq!(coop(&fixture, text_version).status, 200);
    let calls = fixture.policy.calls.lock().unwrap().clone();
    assert_eq!((calls[0].2, calls[0].3.len()), (0, 500));
    assert_eq!((calls[1].1, calls[1].2), (CoopTransport::SteamExperimental, 7));
    assert_eq!(calls[1].0, "key-1");
}

#[test]
fn coop_transport_update_and_conflict() {
    let fixture = fixture();
    let body = json!({"coopTransport": "ICE", "expectedVersion": 4, "reason": "rolling back the relay"});
    let reply = coop(&fixture, body.clone());
    let request_id = fixture.policy.calls.lock().unwrap()[0].4.clone();
    assert_eq!(request_id.len(), 36);
    assert_eq!(fixture.policy.calls.lock().unwrap()[0].3, "rolling back the relay");
    assert_eq!(
        reply,
        Reply {
            status: 200,
            body: Json(format!(
                r#"{{"policy":{{"coopTransport":"ICE","version":5}},"requestId":"{request_id}"}}"#
            )),
        }
    );

    let conflict = fixture_with(relay_config(), hunt(), true);
    assert_eq!(coop(&conflict, body), Reply {
        status: 409,
        body: Json(
            r#"{"error":{"code":"ADMIN_POLICY_CONFLICT","message":"The operations policy changed. Refresh it before trying again."},"policy":{"coopTransport":"ICE","version":5}}"#
                .to_owned()
        ),
    });
}

#[test]
fn reason_cut_keeps_whole_characters() {
    assert_eq!(first_utf16_units("abc", 2), "ab");
    assert_eq!(first_utf16_units("a\u{1f600}b", 2), "a");
    assert_eq!(first_utf16_units("a\u{1f600}b", 3), "a\u{1f600}");
    assert_eq!(first_utf16_units("ab", 5), "ab");
}
