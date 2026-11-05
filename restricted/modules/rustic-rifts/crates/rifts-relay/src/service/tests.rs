use std::collections::HashMap;
use std::sync::Mutex;

use rifts_seam::{PeerId, Secret, poll_once};

use super::*;
use crate::RelayConfig;
use crate::credentials::tests::{FakeHttp, NOW};
use crate::rules::tests::{HOST, JOIN_A, JOIN_B, account, hunt};
use crate::rules::{HuntState, PresenceState, SessionKind};

#[derive(Default)]
struct FakeHunts {
    hunt: Mutex<Option<HuntSnapshot>>,
    presence: Vec<PresenceRow>,
    gate: Option<P2PGate>,
    queued: Mutex<Vec<ControlMessage>>,
}

impl IceHunts for FakeHunts {
    fn gate(&self) -> Result<P2PGate, RelayError> {
        Ok(self.gate.unwrap_or(P2PGate {
            enabled: true,
            emergency_stop: false,
        }))
    }
    fn find_hunt(&self, _: &SessionId) -> Result<Option<HuntSnapshot>, RelayError> {
        Ok(self.hunt.lock().unwrap().clone())
    }
    fn hub_presence(&self, _: &HuntSnapshot, limit: usize) -> Result<Vec<PresenceRow>, RelayError> {
        Ok(self.presence.iter().take(limit).cloned().collect())
    }
    fn enqueue(&self, message: ControlMessage) -> Result<(), RelayError> {
        self.queued.lock().unwrap().push(message);
        Ok(())
    }
}

struct FakePolicy;

impl OperationsPolicy for FakePolicy {
    fn set_coop_transport(
        &self,
        _: &str,
        transport: CoopTransport,
        expected_version: u64,
        _: &str,
        _: &str,
    ) -> Result<CoopTransportOutcome, RelayError> {
        let policy = Json(format!(
            r#"{{"coopTransport":"{}","version":{}}}"#,
            transport.as_str(),
            expected_version + 1
        ));
        Ok(if expected_version == 4 {
            CoopTransportOutcome::Updated(policy)
        } else {
            CoopTransportOutcome::Conflict(Json(r#"{"coopTransport":"ICE","version":4}"#.to_owned()))
        })
    }
}

fn session() -> SessionId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV@local".parse().unwrap()
}

fn turn_rest_config(extra: &[(&str, &str)]) -> RelayConfig {
    let key_secret = "s".repeat(40);
    let mut pairs: HashMap<String, String> = [
        ("P2P_TURN_PROVIDER", "turn-rest"),
        ("P2P_TURN_REST_URLS", "turn:relay.example.org:3478?transport=udp"),
        ("P2P_TURN_REST_SECRET", "a-shared-secret-of-32-characters"),
        ("P2P_TRANSPORT_KEY_SECRET", key_secret.as_str()),
    ]
    .iter()
    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
    .collect();
    pairs.extend(extra.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())));
    RelayConfig::from_settings(&pairs)
}

fn ice(hunts: Arc<FakeHunts>, config: RelayConfig) -> Ice {
    let grants = RelayGrants {
        config: Arc::new(config),
        http: FakeHttp::answering(vec![]),
        usage: Arc::default(),
    };
    Ice::new(hunts, grants, Arc::new(FakePolicy), Arc::new(|| NOW))
}

fn with_hunt(snapshot: HuntSnapshot) -> Arc<FakeHunts> {
    Arc::new(FakeHunts {
        hunt: Mutex::new(Some(snapshot)),
        ..FakeHunts::default()
    })
}

fn code(error: RelayError) -> String {
    match error {
        RelayError::Refused(refusal) => refusal.code,
        other => format!("{other:?}"),
    }
}

#[test]
fn a_joiner_gets_stun_turn_rest_and_its_pair_with_the_host() {
    let service = ice(with_hunt(hunt()), turn_rest_config(&[]));
    let reply = poll_once(service.transport_config(account(JOIN_A), session(), Some(3)))
        .unwrap()
        .unwrap();
    let json: Value = serde_json::from_str(&reply.0).unwrap();
    // serde_json sorts keys, so the text shows Node's order
    assert!(reply.0.starts_with(concat!(
        r#"{"sessionId":"session-1","generation":3,"role":"JOIN","localAccountId":"join-aaaa-account","#,
        r#""localPeerId":"1002","hostAccountId":"host-0000-account","hostPeerId":"1001","forceRelay":false,"#,
        r#""iceServers":[{"urls":["stun:stun.cloudflare.com:3478"]},"#,
        r#"{"urls":["turn:relay.example.org:3478?transport=udp"],"username":"#
    )));
    assert_eq!(json.as_object().unwrap().len(), 11);
    // A hunt expiring at 12:00 seen at 08:00 is capped at the 3-hour default
    assert_eq!(
        json["relay"],
        serde_json::json!({ "allowed": true, "ttlSeconds": 10_800, "expiresAt": "2026-10-13T11:00:00.000Z" })
    );
    let username = json["iceServers"][1]["username"].as_str().unwrap();
    assert!(username.starts_with(&format!("{}:", NOW.0 / 1000 + 10_800)));
    let expected_key = crate::derive_pair_key(
        &Secret::new("s".repeat(40)),
        "session-1",
        3,
        &account(HOST),
        &account(JOIN_A),
    );
    assert_eq!(
        json["peers"],
        serde_json::json!([{ "accountId": HOST, "role": "HOST", "transportKey": expected_key.expose(), "peerId": "1001" }])
    );
    assert!(reply.0.ends_with(r#""peerId":"1001"}]}"#));
}

#[test]
fn the_host_gets_every_active_joiner() {
    let service = ice(with_hunt(hunt()), turn_rest_config(&[("P2P_FORCE_SDR", "true")]));
    let reply = poll_once(service.transport_config(account(HOST), session(), Some(3)))
        .unwrap()
        .unwrap();
    let json: Value = serde_json::from_str(&reply.0).unwrap();
    assert_eq!(json["role"], "HOST");
    assert_eq!(json["forceRelay"], true);
    let peers: Vec<(&str, &str)> = json["peers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|peer| (peer["accountId"].as_str().unwrap(), peer["peerId"].as_str().unwrap()))
        .collect();
    assert_eq!(peers, [(JOIN_A, "1002"), (JOIN_B, "1003")]);
}

#[test]
fn transport_config_refusals_are_nodes() {
    let refused = |hunts: Arc<FakeHunts>, config: RelayConfig, who: &str, generation: Option<i64>| {
        let service = ice(hunts, config);
        code(
            poll_once(service.transport_config(account(who), session(), generation))
                .unwrap()
                .unwrap_err(),
        )
    };
    let config = || turn_rest_config(&[]);
    let gated = |enabled: bool, emergency_stop: bool| {
        Arc::new(FakeHunts {
            gate: Some(P2PGate {
                enabled,
                emergency_stop,
            }),
            ..FakeHunts::default()
        })
    };
    assert_eq!(refused(gated(false, false), config(), JOIN_A, Some(3)), "P2P_DISABLED");
    assert_eq!(
        refused(gated(true, true), config(), JOIN_A, Some(3)),
        "P2P_EMERGENCY_STOP"
    );
    let no_secret = RelayConfig {
        transport_key_secret: None,
        ..config()
    };
    assert_eq!(
        refused(with_hunt(hunt()), no_secret, JOIN_A, Some(3)),
        "P2P_ICE_UNAVAILABLE"
    );
    assert_eq!(refused(with_hunt(hunt()), config(), JOIN_A, None), "P2P_TICKET_INVALID");
    assert_eq!(
        refused(Arc::default(), config(), JOIN_A, Some(3)),
        "P2P_SESSION_NOT_FOUND"
    );
    let steam = HuntSnapshot {
        coop_transport: CoopTransport::SteamExperimental,
        ..hunt()
    };
    assert_eq!(
        refused(with_hunt(steam), config(), JOIN_A, Some(3)),
        "P2P_TRANSPORT_MISMATCH"
    );
    assert_eq!(
        refused(with_hunt(hunt()), config(), JOIN_A, Some(2)),
        "P2P_GENERATION_FENCED"
    );
    let over = HuntSnapshot {
        state: HuntState::Completed,
        ..hunt()
    };
    assert_eq!(
        refused(with_hunt(over), config(), JOIN_A, Some(3)),
        "P2P_SESSION_NOT_READY"
    );
    assert_eq!(
        refused(with_hunt(hunt()), config(), "join-home-account", Some(3)),
        "P2P_NOT_ROSTERED"
    );
}

#[test]
fn configs_are_rate_limited_per_account() {
    let service = ice(
        with_hunt(hunt()),
        turn_rest_config(&[("P2P_ICE_CONFIGS_PER_HOUR", "2")]),
    );
    let request = |who: &str| poll_once(service.transport_config(account(who), session(), Some(3))).unwrap();
    assert!(request(JOIN_A).is_ok());
    assert!(request(JOIN_A).is_ok());
    assert_eq!(code(request(JOIN_A).unwrap_err()), "P2P_RELAY_RATE_LIMITED");
    assert!(request(HOST).is_ok());
}

#[test]
fn a_hub_uses_presence_rows_and_its_own_credential_lifetime() {
    let hub = HuntSnapshot {
        kind: SessionKind::RealmHub,
        realm: Some("realm-1".to_owned()),
        roster: Vec::new(),
        expires_at: UnixMillis(NOW.0 + 30 * 3_600_000),
        ..hunt()
    };
    let hunts = Arc::new(FakeHunts {
        hunt: Mutex::new(Some(hub)),
        presence: vec![PresenceRow {
            account: account(JOIN_A),
            peer: PeerId(2002),
            state: PresenceState::Ticketed,
        }],
        ..FakeHunts::default()
    });
    let service = ice(hunts, turn_rest_config(&[]));
    let reply = poll_once(service.transport_config(account(HOST), session(), Some(3)))
        .unwrap()
        .unwrap();
    let json: Value = serde_json::from_str(&reply.0).unwrap();
    assert_eq!(json["relay"]["ttlSeconds"], 86_400);
    assert_eq!(json["peers"][0]["peerId"], "2002");
    let member = poll_once(service.transport_config(account(JOIN_A), session(), Some(3)))
        .unwrap()
        .unwrap();
    assert!(member.0.contains(r#""localPeerId":"2002""#));
}

#[test]
fn signals_are_queued_to_the_other_side() {
    let hunts = with_hunt(hunt());
    let service = ice(hunts.clone(), turn_rest_config(&[]));
    let body = serde_json::json!({ "generation": 3, "toAccountId": HOST, "kind": "CANDIDATE",
        "data": "a=candidate:1 1 UDP 1 203.0.113.5 50000 typ srflx" });
    let reply = poll_once(service.submit_signal(account(JOIN_A), session(), body))
        .unwrap()
        .unwrap();
    let json: Value = serde_json::from_str(&reply.0).unwrap();
    assert_eq!(json["delivered"], true);
    let queued = hunts.queued.lock().unwrap();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].account, account(HOST));
    assert_eq!(queued[0].message_type, "ICE_SIGNAL");
    assert_eq!(queued[0].expires_at, UnixMillis(NOW.0 + 120_000));
    let payload = &queued[0].payload.0;
    assert!(payload.starts_with(r#"{"sessionId":"session-1","generation":3,"signalId":""#));
    assert!(payload.ends_with(concat!(
        r#","fromAccountId":"join-aaaa-account","fromRole":"JOIN","kind":"CANDIDATE","#,
        r#""data":"a=candidate:1 1 UDP 1 203.0.113.5 50000 typ srflx"}"#
    )));
    assert!(payload.contains(json["signalId"].as_str().unwrap()));
}

#[test]
fn relay_only_filters_signals_and_refusals_are_nodes() {
    let hunts = with_hunt(hunt());
    let service = ice(hunts.clone(), turn_rest_config(&[("P2P_FORCE_SDR", "true")]));
    let host_candidate = serde_json::json!({ "generation": 3, "toAccountId": HOST, "kind": "CANDIDATE",
        "data": "a=candidate:1 1 UDP 2122317823 192.168.1.20 50000 typ host" });
    let reply = poll_once(service.submit_signal(account(JOIN_A), session(), host_candidate))
        .unwrap()
        .unwrap();
    assert_eq!(reply.0, r#"{"delivered":false,"reason":"FILTERED_RELAY_ONLY"}"#);
    assert!(hunts.queued.lock().unwrap().is_empty());

    let submit = |from: &str, body: Value| {
        code(
            poll_once(service.submit_signal(account(from), session(), body))
                .unwrap()
                .unwrap_err(),
        )
    };
    assert_eq!(
        submit(JOIN_A, serde_json::json!({ "generation": 3, "kind": "OFFER" })),
        "P2P_SIGNAL_INVALID"
    );
    let to_joiner = serde_json::json!({ "generation": 3, "toAccountId": JOIN_B, "kind": "GATHERING_DONE" });
    assert_eq!(submit(JOIN_A, to_joiner), "P2P_NOT_ROSTERED");
    let stale = serde_json::json!({ "generation": 2, "toAccountId": HOST, "kind": "GATHERING_DONE" });
    assert_eq!(submit(JOIN_A, stale), "P2P_GENERATION_FENCED");
}

#[test]
fn signals_are_limited_per_sender_and_generation() {
    let service = ice(with_hunt(hunt()), turn_rest_config(&[]));
    let body = serde_json::json!({ "generation": 3, "toAccountId": HOST, "kind": "GATHERING_DONE" });
    for _ in 0..300 {
        assert!(
            poll_once(service.submit_signal(account(JOIN_A), session(), body.clone()))
                .unwrap()
                .is_ok()
        );
    }
    let refused = poll_once(service.submit_signal(account(JOIN_A), session(), body))
        .unwrap()
        .unwrap_err();
    assert_eq!(code(refused), "P2P_SIGNAL_LIMIT");
}

#[test]
fn coop_transport_updates_validate_and_report_conflicts() {
    let service = ice(Arc::default(), turn_rest_config(&[]));
    let update = |body: Value| {
        poll_once(service.set_coop_transport("admin-1".to_owned(), body))
            .unwrap()
            .unwrap()
    };
    let bad = update(serde_json::json!({ "coopTransport": "UDP", "expectedVersion": 4, "reason": "switching over" }));
    assert_eq!(bad.status, 400);
    assert!(bad.body.0.contains("ADMIN_VALIDATION"));
    let short = update(serde_json::json!({ "coopTransport": "ICE", "expectedVersion": 4, "reason": " short  " }));
    assert_eq!(short.status, 400);
    let missing = update(serde_json::json!({ "coopTransport": "ICE", "reason": "switching over" }));
    assert_eq!(missing.status, 400);
    let ok = update(
        serde_json::json!({ "coopTransport": "STEAM_EXPERIMENTAL", "expectedVersion": "4",
        "reason": "back to steam" }),
    );
    assert_eq!(ok.status, 200);
    assert!(
        ok.body
            .0
            .starts_with(r#"{"policy":{"coopTransport":"STEAM_EXPERIMENTAL","version":5},"requestId":""#)
    );
    let conflict =
        update(serde_json::json!({ "coopTransport": "ICE", "expectedVersion": 3, "reason": "switching over" }));
    assert_eq!(conflict.status, 409);
    assert_eq!(
        conflict.body.0,
        concat!(
            r#"{"error":{"code":"ADMIN_POLICY_CONFLICT","message":"The operations policy changed. "#,
            r#"Refresh it before trying again."},"policy":{"coopTransport":"ICE","version":4}}"#
        )
    );
}
