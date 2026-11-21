// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Guard as isles drives it: the evidence chain, the routes, traffic, host writes, inventory, operations.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use rifts_seam::{
    AccountId, Caller, CapabilityScope, Component, GuardHooks, HOST_CAPABILITY_HEADER, HostWrite, HostWriteKind,
    HostWriteOptions, HubKind, InventoryDecision, InventoryFence, InventoryTransaction, Json, Lifecycle, Method, Mount,
    Refusal, Reply, RequestInfo, RouteCall, RouteHandler, RuntimeAuth, SeamError, SessionId, TransactionPhase,
    TransactionReads, UnixMillis, poll_once,
};
use serde_json::{Value, json};

use crate::evidence::{
    BatchProblem, MAX_BATCH_EVENTS, apply_batch, apply_completion, finding_update, parse_batch, parse_completion,
};
use crate::grants::HOST_DRAFT_PREMIUM_ID;
use crate::json::stringify;
use crate::ports::{CapabilityView, MemoryOperations};
use crate::store::{ChainRecord, EncounterState, GENESIS_HASH, GuardStore, StubGuardStore};
use crate::testkit::{HUB_SESSION, Kit, NOW, REALM, SESSION, TOKEN, account, hub, hunt, kit, kit_on, runtime, session};
use crate::{
    EvidenceIntake, GuardError, GuardOperations, GuardRoutes, HuntVerdict, ROUTES, RewardPolicyStatus, Stub,
    TrafficDecision, TrafficLimits, sha256_hex, stub_routes,
};

const EVIDENCE_ON: (&str, &str) = ("RUSTIC_RIFTS_HUNT_EVIDENCE", "on");

/// The transport's batch as it writes it: keys in its order, the first event with no actor.
fn transport_body(first: u64, previous: &str, batch_id: &str, events: &[(&str, Option<&str>)]) -> Value {
    let events: Vec<Value> = events
        .iter()
        .enumerate()
        .map(|(i, (kind, actor))| {
            let mut event = serde_json::Map::new();
            event.insert("sequence".into(), json!(first + i as u64));
            event.insert("monotonicMs".into(), json!(1000 + 250 * (first - 1 + i as u64)));
            event.insert("type".into(), json!(kind));
            if let Some(actor) = actor {
                event.insert("actorAccountId".into(), json!(actor));
            }
            event.insert("payload".into(), json!({}));
            Value::Object(event)
        })
        .collect();
    json!({
        "schema": 1,
        "batchId": batch_id,
        "firstSequence": first,
        "lastSequence": first + events.len() as u64 - 1,
        "previousHash": previous,
        "events": events,
    })
}

fn refusal_code<T: std::fmt::Debug>(result: Result<T, GuardError>) -> String {
    match result {
        Err(GuardError::Refused(refusal)) => refusal.code,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

// ---- the batch body and hash ----

#[test]
fn known_body_hashes_to_the_pinned_value() {
    // spec-rd27-rifts-guard §9 risk 1: a chain mid-hunt continues across the rewrite only if this holds
    let text = format!(
        r#"{{ "schema": 1, "batchId": "batch-0001", "firstSequence": 1, "lastSequence": 2,
            "previousHash": "{GENESIS_HASH}",
            "events": [
              {{"sequence": 1, "monotonicMs": 1500, "type": "SESSION_STARTED", "payload": {{}}}},
              {{"sequence": 2, "monotonicMs": 2750, "type": "DAMAGE", "actorAccountId": "member",
                "payload": {{"amount": 12.5, "target": "behemoth"}}}}
            ] }}"#
    );
    let body: Value = serde_json::from_str(&text).unwrap();
    let batch = parse_batch(Some(&body)).unwrap();
    assert_eq!(
        batch.batch_hash,
        "5781c8e2f3426ae61cc86d0114d32b033f0360e42610f378a84932dc9bc238fb"
    );
    assert_eq!(stringify(&body).len(), 357);
    assert_eq!(batch.events[0].actor, None);
    assert_eq!(batch.events[1].actor.as_deref(), Some("member"));
    assert_eq!((batch.first_sequence, batch.last_sequence), (1, 2));
    // whitespace is not, but key order is part of the hash
    let mut moved = body.as_object().unwrap().clone();
    let events = moved.shift_remove("events").unwrap();
    let mut front = serde_json::Map::new();
    front.insert("events".into(), events);
    front.extend(moved);
    let other = parse_batch(Some(&Value::Object(front))).unwrap();
    assert_ne!(other.batch_hash, batch.batch_hash);
}

#[test]
fn transport_body_parses_and_hashes_deterministically() {
    let body = transport_body(
        1,
        GENESIS_HASH,
        "b-1",
        &[("SESSION_STARTED", None), ("WORLD_READY", Some("host"))],
    );
    let a = parse_batch(Some(&body)).unwrap();
    let b = parse_batch(Some(&body.clone())).unwrap();
    assert_eq!(a.batch_hash, b.batch_hash);
    assert!(crate::digest::is_hex64(&a.batch_hash));
    assert_eq!(a.events.len(), 2);
    let expected_bytes: usize = body["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| stringify(e).len())
        .sum();
    assert_eq!(a.event_bytes, expected_bytes as u64);
}

#[test]
fn every_malformation_is_refused() {
    let good = transport_body(
        1,
        GENESIS_HASH,
        "b-1",
        &[("SESSION_STARTED", None), ("DAMAGE", Some("member"))],
    );
    let invalid = |edit: &dyn Fn(&mut Value)| {
        let mut body = good.clone();
        edit(&mut body);
        parse_batch(Some(&body))
    };
    type Edit = Box<dyn Fn(&mut Value)>;
    let cases: Vec<(&str, Edit)> = vec![
        ("extra key", Box::new(|b| b["extra"] = json!(1))),
        ("schema 2", Box::new(|b| b["schema"] = json!(2))),
        ("schema text", Box::new(|b| b["schema"] = json!("1"))),
        ("empty id", Box::new(|b| b["batchId"] = json!(""))),
        ("id 65", Box::new(|b| b["batchId"] = json!("a".repeat(65)))),
        ("id char", Box::new(|b| b["batchId"] = json!("a_b"))),
        ("upper hash", Box::new(|b| b["previousHash"] = json!("A".repeat(64)))),
        ("short hash", Box::new(|b| b["previousHash"] = json!("0".repeat(63)))),
        ("first 0", Box::new(|b| b["firstSequence"] = json!(0))),
        ("first fraction", Box::new(|b| b["firstSequence"] = json!(1.5))),
        ("last negative", Box::new(|b| b["lastSequence"] = json!(-1))),
        ("last wrong", Box::new(|b| b["lastSequence"] = json!(3))),
        ("events object", Box::new(|b| b["events"] = json!({}))),
        ("no events", Box::new(|b| b["events"] = json!([]))),
        ("event key", Box::new(|b| b["events"][0]["x"] = json!(1))),
        ("gap", Box::new(|b| b["events"][1]["sequence"] = json!(3))),
        ("regressed", Box::new(|b| b["events"][1]["monotonicMs"] = json!(1))),
        ("type char", Box::new(|b| b["events"][0]["type"] = json!("A-B"))),
        (
            "type long",
            Box::new(|b| b["events"][0]["type"] = json!("A".repeat(65))),
        ),
        (
            "actor null",
            Box::new(|b| b["events"][0]["actorAccountId"] = Value::Null),
        ),
        (
            "actor char",
            Box::new(|b| b["events"][0]["actorAccountId"] = json!("a b")),
        ),
        (
            "actor long",
            Box::new(|b| b["events"][0]["actorAccountId"] = json!("a".repeat(97))),
        ),
        ("payload array", Box::new(|b| b["events"][0]["payload"] = json!([]))),
        (
            "no payload",
            Box::new(|b| {
                b["events"][0].as_object_mut().unwrap().remove("payload");
            }),
        ),
        (
            "payload big",
            Box::new(|b| b["events"][0]["payload"] = json!({"x": "y".repeat(8200)})),
        ),
        (
            "terminal early",
            Box::new(|b| b["events"][0]["type"] = json!("HUNT_ABORTED")),
        ),
    ];
    for (name, edit) in &cases {
        assert!(
            matches!(invalid(edit.as_ref()), Err(BatchProblem::Invalid(_))),
            "{name}"
        );
    }
    assert!(parse_batch(None).is_err());
    assert!(parse_batch(Some(&json!([1]))).is_err());
    let actor_ok = invalid(&|b| b["events"][0]["actorAccountId"] = json!("acct.1:x-y_z"));
    assert!(actor_ok.is_ok());
    let terminal_last = invalid(&|b| b["events"][1]["type"] = json!("COMPLETION_REQUESTED"));
    assert!(terminal_last.is_ok());

    let many: Vec<(&str, Option<&str>)> = vec![("TICK", None); MAX_BATCH_EVENTS + 1];
    let too_many = transport_body(1, GENESIS_HASH, "b", &many);
    assert_eq!(parse_batch(Some(&too_many)), Err(BatchProblem::TooLarge));
    let mut heavy = transport_body(1, GENESIS_HASH, "b", &vec![("TICK", None); 9]);
    for event in heavy["events"].as_array_mut().unwrap() {
        event["payload"] = json!({"x": "y".repeat(8000)});
    }
    assert_eq!(parse_batch(Some(&heavy)), Err(BatchProblem::TooLarge));
    assert_eq!(
        BatchProblem::TooLarge.into_error(),
        GuardError::refused(
            "GUARD_BATCH_TOO_LARGE",
            "The Guard request body exceeds its bounded limit."
        )
    );
    assert_eq!(
        BatchProblem::Invalid("x").into_error(),
        GuardError::refused("GUARD_BATCH_INVALID", "The Guard event batch is invalid.")
    );
}

// ---- applying batches and completions ----

fn roster() -> Vec<AccountId> {
    ["host", "member"].map(account).to_vec()
}

#[test]
fn chains_by_hash_and_sequence() {
    let chain = ChainRecord::new(session(), 1, None);
    let first = parse_batch(Some(&transport_body(
        1,
        GENESIS_HASH,
        "a",
        &[("SESSION_STARTED", None)],
    )))
    .unwrap();
    let update = apply_batch(&chain, &first, &roster(), UnixMillis(5)).unwrap();
    let next = update.chain;
    assert_eq!((next.revision, next.last_sequence, next.batch_count), (1, 1, 1));
    assert_eq!(next.head_hash, first.batch_hash);
    assert_eq!(next.encounter_state, EncounterState::SessionStarted);
    assert_eq!(next.last_received_at, Some(UnixMillis(5)));
    let second = parse_batch(Some(&transport_body(
        2,
        &first.batch_hash,
        "b",
        &[("WORLD_READY", None)],
    )))
    .unwrap();
    let after = apply_batch(&next, &second, &roster(), UnixMillis(6)).unwrap().chain;
    assert_eq!(
        (after.last_sequence, after.encounter_state),
        (2, EncounterState::WorldReady)
    );
    // a stale batch, written against the old head
    let stale = parse_batch(Some(&transport_body(2, GENESIS_HASH, "c", &[("WORLD_READY", None)]))).unwrap();
    let refusal = apply_batch(&after, &stale, &roster(), UnixMillis(7)).unwrap_err();
    assert_eq!(refusal.error.code(), Some("GUARD_CHAIN_CONFLICT"));
    let (code, detail) = refusal.finding.unwrap();
    assert_eq!(code, "CHAIN_CONFLICT");
    assert_eq!(detail, format!("expected=3:{} got=2:{GENESIS_HASH}", after.head_hash));
    let flagged = finding_update(&after, &code, &detail, UnixMillis(7));
    assert_eq!(flagged.chain.revision, after.revision + 1);
    assert_eq!(flagged.findings[0].sequence, None);
    assert_eq!(flagged.chain.disposition.as_str(), "OBSERVE_FLAGGED");
}

#[test]
fn findings_in_order_and_batch_types_sorted() {
    let chain = ChainRecord::new(session(), 1, None);
    let batch = parse_batch(Some(&transport_body(
        1,
        GENESIS_HASH,
        "a",
        &[
            ("DAMAGE", Some("stranger")),
            ("DAMAGE", None),
            ("EVIDENCE_LOSS", None),
            ("DAMAGE", None),
        ],
    )))
    .unwrap();
    let update = apply_batch(&chain, &batch, &roster(), UnixMillis(1)).unwrap();
    let codes: Vec<&str> = update.findings.iter().map(|f| f.code.as_str()).collect();
    assert_eq!(codes, ["ACTOR_NOT_ROSTERED", "UNATTRIBUTED_DAMAGE", "EVIDENCE_LOSS"]);
    assert_eq!(update.findings[0].detail, "actor=stranger");
    assert_eq!(update.findings[1].sequence, Some(2));
    assert_eq!(update.batch.unwrap().event_types, ["DAMAGE", "EVIDENCE_LOSS"]);
    assert_eq!(update.chain.event_types.get("DAMAGE"), Some(&3));
    assert_eq!(update.chain.finding_count, 3);
    assert!(update.chain.has_unattributed_damage);
    assert_eq!(update.chain.encounter_state, EncounterState::EncounterActive);
}

#[test]
fn monotonic_regression_and_limits() {
    let mut chain = ChainRecord::new(session(), 1, None);
    chain.last_monotonic_ms = Some(5000);
    let batch = parse_batch(Some(&transport_body(1, GENESIS_HASH, "a", &[("TICK", None)]))).unwrap();
    let update = apply_batch(&chain, &batch, &roster(), UnixMillis(1)).unwrap();
    assert_eq!(update.findings[0].code, "MONOTONIC_REGRESSION");
    assert_eq!(update.findings[0].detail, "from=5000 to=1000");
    chain.event_count = 262_144;
    let refusal = apply_batch(&chain, &batch, &roster(), UnixMillis(1)).unwrap_err();
    assert_eq!(refusal.error.code(), Some("GUARD_EVIDENCE_LIMIT"));
    assert_eq!(refusal.finding, Some(("EVIDENCE_LIMIT".into(), "events=262144".into())));
}

#[test]
fn closed_chains_and_completions() {
    let chain = ChainRecord::new(session(), 1, None);
    let body = transport_body(
        1,
        GENESIS_HASH,
        "a",
        &[("BEHEMOTH_DEFEATED", None), ("COMPLETION_REQUESTED", None)],
    );
    let batch = parse_batch(Some(&body)).unwrap();
    let request =
        |seq: u64, hash: &str| parse_completion(Some(&json!({"finalSequence": seq, "finalHash": hash}))).unwrap();
    let early = apply_completion(&chain, &request(1, GENESIS_HASH), UnixMillis(1));
    assert_eq!(
        early,
        Err(GuardError::refused(
            "GUARD_COMPLETION_INVALID",
            "The Guard chain has not requested completion."
        ))
    );
    let requested = apply_batch(&chain, &batch, &roster(), UnixMillis(1)).unwrap().chain;
    assert_eq!(requested.encounter_state, EncounterState::CompletionRequested);
    let late = parse_batch(Some(&transport_body(3, &requested.head_hash, "b", &[("TICK", None)]))).unwrap();
    assert_eq!(
        apply_batch(&requested, &late, &roster(), UnixMillis(2))
            .unwrap_err()
            .error
            .code(),
        Some("GUARD_CHAIN_FROZEN")
    );
    assert_eq!(
        refusal_code(apply_completion(&requested, &request(2, GENESIS_HASH), UnixMillis(2))),
        "GUARD_COMPLETION_CONFLICT"
    );
    let done = apply_completion(&requested, &request(2, &requested.head_hash), UnixMillis(3))
        .unwrap()
        .unwrap()
        .chain;
    assert_eq!(done.completion.as_ref().unwrap().at, UnixMillis(3));
    assert_eq!(
        apply_completion(&done, &request(2, &requested.head_hash), UnixMillis(4)),
        Ok(None)
    );
    assert_eq!(
        refusal_code(apply_completion(
            &done,
            &request(1, &requested.head_hash),
            UnixMillis(4)
        )),
        "GUARD_COMPLETION_CONFLICT"
    );
    let mut none = requested.clone();
    none.completion_mode = Some(crate::CompletionMode::NoCompletion);
    assert_eq!(
        apply_completion(&none, &request(2, &requested.head_hash), UnixMillis(3)),
        Err(GuardError::refused(
            "GUARD_COMPLETION_INVALID",
            "This hunt generation has no Guard completion."
        ))
    );
    let mut aborted = requested;
    aborted.encounter_state = EncounterState::Aborted;
    assert_eq!(
        refusal_code(apply_completion(
            &aborted,
            &request(2, &aborted.head_hash),
            UnixMillis(3)
        )),
        "GUARD_COMPLETION_INVALID"
    );
}

#[test]
fn completion_body_is_strict() {
    let hash = "a".repeat(64);
    assert!(parse_completion(Some(&json!({"finalSequence": 1, "finalHash": hash}))).is_ok());
    for bad in [
        json!({"finalSequence": 0, "finalHash": hash}),
        json!({"finalSequence": "1", "finalHash": hash}),
        json!({"finalSequence": 1, "finalHash": hash.to_uppercase()}),
        json!({"finalSequence": 1, "finalHash": hash, "extra": true}),
        json!({"finalSequence": 1}),
        json!([]),
    ] {
        assert_eq!(
            parse_completion(Some(&bad)),
            Err(GuardError::refused(
                "GUARD_COMPLETION_INVALID",
                "The Guard completion request is invalid."
            )),
            "{bad}"
        );
    }
    assert!(parse_completion(None).is_err());
}

// ---- the routes ----

fn call(route: usize, caller: Caller, body: &[u8], headers: &[(&str, &str)]) -> RouteCall {
    let mut headers: Vec<(String, String)> = headers
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    headers.push(("content-type".into(), "application/json".into()));
    RouteCall {
        route: ROUTES[route],
        params: vec![
            ("sessionId".into(), SESSION.into()),
            ("alertId".into(), "alert-0001".into()),
        ],
        query: String::new(),
        request: RequestInfo {
            path: ROUTES[route].path.into(),
            headers,
            remote: Some(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 7))),
            request_id: "req-1".into(),
        },
        caller,
        body: body.to_vec(),
    }
}

fn handle(routes: &GuardRoutes, call: RouteCall) -> Result<Reply, SeamError> {
    poll_once(routes.handle(call)).expect("ready")
}

fn post(kit: &Kit, route: usize, body: &Value) -> Result<Reply, SeamError> {
    let routes = GuardRoutes::new(kit.guard.clone());
    handle(
        &routes,
        call(route, Caller::HostRuntime(runtime()), stringify(body).as_bytes(), &[]),
    )
}

fn seam_code(result: Result<Reply, SeamError>) -> String {
    match result {
        Err(SeamError::Refused(refusal)) => refusal.code,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn reply_json(result: Result<Reply, SeamError>) -> Value {
    let reply = result.unwrap();
    assert_eq!(reply.status, 200);
    serde_json::from_str(&reply.body.0).unwrap()
}

#[test]
fn route_list_in_mount_order() {
    let table: Vec<(Method, &str, Mount, &str)> =
        ROUTES.iter().map(|r| (r.method, r.path, r.mount, r.handler)).collect();
    assert_eq!(
        table,
        [
            (
                Method::Post,
                "/rustic/v1/hunts/:sessionId/event-batches",
                Mount::Early,
                "guard.event_batches"
            ),
            (
                Method::Post,
                "/rustic/v1/hunts/:sessionId/completion",
                Mount::Early,
                "guard.completion"
            ),
            (
                Method::Get,
                "/rustic/v1/hunts/:sessionId/reward/self",
                Mount::Early,
                "guard.reward_self"
            ),
            (
                Method::Post,
                "/internal/v1/hunt-guard/shadow-comparisons",
                Mount::Early,
                "guard.shadow_comparisons"
            ),
            (
                Method::Patch,
                "/admin/v1/operations/reward-rollout",
                Mount::Admin,
                "guard.reward_rollout"
            ),
            (
                Method::Patch,
                "/admin/v1/operations/alerts/:alertId/ack",
                Mount::Admin,
                "guard.alert_ack"
            ),
        ]
    );
    assert_eq!(stub_routes().component, Component::Guard);
    assert_eq!(stub_routes().routes, ROUTES);
}

#[test]
fn stub_and_store_less_routes_answer_unimplemented() {
    let kit = kit(&[EVIDENCE_ON]);
    let storeless = GuardRoutes::with_store(kit.guard.clone(), false);
    for (i, route) in ROUTES.iter().enumerate() {
        let expected = Err(SeamError::unimplemented(Component::Guard, route.handler));
        assert_eq!(handle(&storeless, call(i, Caller::Anonymous, b"{}", &[])), expected);
        assert_eq!(
            poll_once(stub_routes().handle(call(i, Caller::Anonymous, b"{}", &[]))).unwrap(),
            expected
        );
    }
}

#[test]
fn evidence_off_collects_nothing_and_returns_the_budget() {
    let kit = kit(&[]);
    let body = transport_body(1, GENESIS_HASH, "a", &[("SESSION_STARTED", None)]);
    assert_eq!(seam_code(post(&kit, 0, &body)), "HUNT_EVIDENCE_NOT_COLLECTED");
    assert_eq!(kit.guard.request_budget().snapshot(), (0, 0));
    assert_eq!(kit.store.chain(&session(), 1).unwrap(), None);
}

#[test]
fn bodies_are_bounded_and_strict_before_authentication() {
    let kit = kit(&[EVIDENCE_ON]);
    let routes = GuardRoutes::new(kit.guard.clone());
    let big = vec![b' '; 73_729];
    assert_eq!(
        seam_code(handle(&routes, call(0, Caller::Anonymous, &big, &[]))),
        "GUARD_BATCH_TOO_LARGE"
    );
    let broken = handle(&routes, call(0, Caller::Anonymous, b"{oops", &[]));
    assert_eq!(
        broken,
        Err(SeamError::Refused(Refusal {
            code: "GUARD_BATCH_INVALID".into(),
            message: "The Guard request body is not valid JSON.".into()
        }))
    );
    assert_eq!(
        seam_code(handle(&routes, call(0, Caller::Anonymous, b"\"x\"", &[]))),
        "GUARD_BATCH_INVALID"
    );
    assert_eq!(
        seam_code(handle(&routes, call(1, Caller::Anonymous, b"{}", &[]))),
        "P2P_CAPABILITY_INVALID"
    );
    let launcher = Caller::Launcher(account("member"));
    assert_eq!(
        seam_code(handle(&routes, call(0, launcher, b"{}", &[]))),
        "P2P_CAPABILITY_INVALID"
    );
    // a malformed or unknown header leaves the caller anonymous
    assert_eq!(
        seam_code(handle(
            &routes,
            call(0, Caller::Anonymous, b"{}", &[(HOST_CAPABILITY_HEADER, "short")])
        )),
        "P2P_CAPABILITY_INVALID"
    );
    assert_eq!(
        seam_code(handle(
            &routes,
            call(0, Caller::Anonymous, b"{}", &[(HOST_CAPABILITY_HEADER, TOKEN)])
        )),
        "P2P_CAPABILITY_INVALID"
    );
}

#[test]
fn full_chain_flow_through_the_routes() {
    let kit = kit(&[EVIDENCE_ON]);
    let routes = GuardRoutes::new(kit.guard.clone());
    kit.hunts
        .runtimes
        .lock()
        .unwrap()
        .insert(sha256_hex(TOKEN.as_bytes()), runtime());
    let with_token = |route: usize, body: &Value| {
        handle(
            &routes,
            call(
                route,
                Caller::Anonymous,
                stringify(body).as_bytes(),
                &[(HOST_CAPABILITY_HEADER, TOKEN)],
            ),
        )
    };

    let first = transport_body(
        1,
        GENESIS_HASH,
        "batch-1",
        &[("SESSION_STARTED", None), ("WORLD_READY", Some("host"))],
    );
    let reply = with_token(0, &first).unwrap();
    assert!(
        reply.body.0.starts_with(r#"{"accepted":true,"batchHash":""#),
        "{}",
        reply.body.0
    );
    let answer: Value = serde_json::from_str(&reply.body.0).unwrap();
    let head = answer["batchHash"].as_str().unwrap().to_owned();
    assert_eq!(answer["duplicate"], json!(false));
    assert_eq!(answer["sessionId"], json!(SESSION));
    assert_eq!(answer["lastSequence"], json!(2));
    assert_eq!(answer["encounterState"], json!("WORLD_READY"));
    assert_eq!(answer["observeOnly"], json!(true));

    // a resend is answered from the stored batch
    let again = reply_json(with_token(0, &first));
    assert_eq!(
        (again["duplicate"].clone(), again["batchHash"].clone()),
        (json!(true), json!(head))
    );
    // the same id with other evidence
    let other = transport_body(1, GENESIS_HASH, "batch-1", &[("SESSION_STARTED", None)]);
    assert_eq!(seam_code(with_token(0, &other)), "GUARD_BATCH_REPLAY");
    // a gap leaves a finding and a log line
    let gap = transport_body(5, &head, "batch-gap", &[("TICK", None)]);
    assert_eq!(seam_code(with_token(0, &gap)), "GUARD_CHAIN_CONFLICT");
    let line = format!(
        "warn [HuntGuardObserve] would refuse CHAIN_CONFLICT session={SESSION} generation=1 sequence=- expected=3:{head} got=5:{head}"
    );
    assert!(kit.lines().contains(&line), "{:?}", kit.lines());

    let terminal = transport_body(
        3,
        &head,
        "batch-2",
        &[("DAMAGE", Some("member")), ("COMPLETION_REQUESTED", None)],
    );
    let closing = reply_json(with_token(0, &terminal));
    assert_eq!(closing["encounterState"], json!("COMPLETION_REQUESTED"));
    assert_eq!(closing["disposition"], json!("OBSERVE_FLAGGED"));
    let final_hash = closing["batchHash"].as_str().unwrap().to_owned();

    let completion = json!({"finalSequence": 4, "finalHash": final_hash});
    let done = with_token(1, &completion).unwrap();
    assert!(done.body.0.starts_with(r#"{"accepted":true,"sessionId":"#));
    let done: Value = serde_json::from_str(&done.body.0).unwrap();
    assert_eq!(done["progressionApplied"], json!(false));
    assert_eq!(done["findingCount"], json!(1));
    assert_eq!(
        poll_once(kit.guard.verdict(session(), 1)).unwrap(),
        Ok(Some(HuntVerdict::OBSERVED))
    );
    assert_eq!(
        HuntVerdict::OBSERVED.to_json().0,
        r#"{"progressionApplied":false,"rewardsApplied":false}"#
    );
    let chain = kit.guard.chain_status(&session(), 1).unwrap().unwrap();
    assert_eq!((chain.batch_count, chain.event_count, chain.finding_count), (2, 4, 1));
    assert!(
        kit.lines().contains(&format!(
            "info [HuntGuardObserve] completion session={SESSION} generation=1 lastSequence=4 findings=1 disposition=OBSERVE_FLAGGED"
        ))
    );
    let late = transport_body(5, &final_hash, "batch-3", &[("TICK", None)]);
    assert_eq!(seam_code(with_token(0, &late)), "GUARD_CHAIN_FROZEN");
    assert_eq!(with_token(1, &completion).unwrap().status, 200);
    assert_eq!(kit.guard.request_budget().snapshot(), (0, 0));
}

#[test]
fn fenced_and_hub_hunts() {
    let kit = kit(&[EVIDENCE_ON]);
    let body = transport_body(1, GENESIS_HASH, "a", &[("SESSION_STARTED", None)]);
    let routes = GuardRoutes::new(kit.guard.clone());
    let as_runtime = |rt: RuntimeAuth| {
        seam_code(handle(
            &routes,
            call(0, Caller::HostRuntime(rt), stringify(&body).as_bytes(), &[]),
        ))
    };
    assert_eq!(
        as_runtime(RuntimeAuth {
            generation: 2,
            ..runtime()
        }),
        "P2P_GENERATION_FENCED"
    );
    assert_eq!(
        as_runtime(RuntimeAuth {
            host_account: account("member"),
            ..runtime()
        }),
        "P2P_GENERATION_FENCED"
    );
    let missing: SessionId = HUB_SESSION.parse().unwrap();
    assert_eq!(
        as_runtime(RuntimeAuth {
            session: missing,
            ..runtime()
        }),
        "P2P_GENERATION_FENCED"
    );
    kit.hunts.put(hub(HubKind::Ramsgate));
    assert_eq!(as_runtime(runtime()), "HUNT_EVIDENCE_NOT_COLLECTED");
}

#[test]
fn a_stub_store_surfaces_as_unimplemented() {
    let kit = kit_on(&[EVIDENCE_ON], Some(Arc::new(StubGuardStore)));
    let body = transport_body(1, GENESIS_HASH, "a", &[("SESSION_STARTED", None)]);
    assert_eq!(
        post(&kit, 0, &body),
        Err(SeamError::unimplemented(Component::Guard, "store.chain"))
    );
}

#[test]
fn self_reward_and_shadow() {
    let kit = kit(&[EVIDENCE_ON]);
    let routes = GuardRoutes::new(kit.guard.clone());
    let reward = |caller: Caller| seam_code(handle(&routes, call(2, caller, b"", &[])));
    assert_eq!(reward(Caller::Launcher(account("pending"))), "HUNT_REWARDS_UNAVAILABLE");
    assert_eq!(reward(Caller::Launcher(account("stranger"))), "P2P_SESSION_NOT_FOUND");
    assert_eq!(reward(Caller::Anonymous), "AUTH_UNAUTHORIZED");
    assert_eq!(
        refusal_code(kit.guard.self_reward(&account("member"), "not-a-session")),
        "P2P_SESSION_NOT_FOUND"
    );
    assert_eq!(
        refusal_code(kit.guard.self_reward(&account("member"), HUB_SESSION)),
        "P2P_SESSION_NOT_FOUND"
    );
    assert_eq!(
        seam_code(handle(&routes, call(3, Caller::Anonymous, b"{}", &[]))),
        "HUNT_EVIDENCE_NOT_COLLECTED"
    );
    assert_eq!(kit.guard.request_budget().snapshot(), (0, 0));
    let shadow = poll_once(kit.guard.record_shadow_comparison(None, Json::empty_object())).unwrap();
    assert_eq!(refusal_code(shadow), "HUNT_EVIDENCE_NOT_COLLECTED");
    assert_eq!(
        poll_once(kit.guard.reward_receipt(session(), 1, account("member"))).unwrap(),
        Ok(None)
    );
    assert_eq!(
        kit.guard.reward_policy_status("hunt", None),
        RewardPolicyStatus::NotReady {
            reason: "ANTICHEAT_MODULE_ABSENT"
        }
    );
}

#[test]
fn intake_parses_its_text() {
    let kit = kit(&[EVIDENCE_ON]);
    let submitted = poll_once(kit.guard.submit_evidence(runtime(), Json("not json".into()))).unwrap();
    assert_eq!(refusal_code(submitted), "GUARD_BATCH_INVALID");
    let body = stringify(&transport_body(1, GENESIS_HASH, "a", &[("SESSION_STARTED", None)]));
    assert!(
        poll_once(kit.guard.submit_evidence(runtime(), Json(body)))
            .unwrap()
            .is_ok()
    );
    let completion = poll_once(kit.guard.complete(runtime(), Json("{}".into()))).unwrap();
    assert_eq!(refusal_code(completion), "GUARD_COMPLETION_INVALID");
    assert_eq!(poll_once(kit.guard.verdict(session(), 1)).unwrap(), Ok(None));
}

#[test]
fn open_chain_writes_once() {
    let kit = kit(&[]);
    kit.guard.open_chain(session(), 4, Some("NO_COMPLETION")).unwrap();
    let chain = kit.guard.chain_status(&session(), 4).unwrap().unwrap();
    assert_eq!(
        (chain.revision, chain.completion_mode),
        (1, Some(crate::CompletionMode::NoCompletion))
    );
    kit.guard.open_chain(session(), 4, Some("FIRST_DEFEAT")).unwrap();
    assert_eq!(
        kit.guard.chain_status(&session(), 4).unwrap().unwrap().completion_mode,
        chain.completion_mode
    );
    kit.guard.open_chain(session(), 5, Some("whatever")).unwrap();
    assert_eq!(
        kit.guard.chain_status(&session(), 5).unwrap().unwrap().completion_mode,
        None
    );
}

// ---- traffic ----

#[test]
fn local_windows_per_host_and_session() {
    let kit = kit(&[]);
    let charge = |host: &str, session: &SessionId| {
        poll_once(kit.guard.charge(account(host), session.clone(), 10))
            .unwrap()
            .unwrap()
    };
    let other: SessionId = HUB_SESSION.parse().unwrap();
    for _ in 0..120 {
        assert_eq!(charge("host", &session()), TrafficDecision::Accepted);
    }
    assert_eq!(charge("host", &session()), TrafficDecision::RateLimited);
    // the session refusal still charged the host: 121 of 180
    for _ in 0..59 {
        assert_eq!(charge("host", &other), TrafficDecision::Accepted);
    }
    assert_eq!(charge("host", &other), TrafficDecision::RateLimited);
    kit.clock.advance(60_000);
    assert_eq!(charge("host", &session()), TrafficDecision::Accepted);
}

#[test]
fn distributed_limit_then_unavailable() {
    let kit = kit(&[("P2P_DISTRIBUTED_TRAFFIC_LIMITER", "true")]);
    for _ in 0..120 {
        assert_eq!(
            kit.guard.charge_now(&account("host"), &session(), 10),
            TrafficDecision::Accepted
        );
    }
    assert_eq!(
        kit.guard.charge_now(&account("host"), &session(), 10),
        TrafficDecision::RateLimited
    );
    let windows = kit.store.windows();
    assert_eq!(windows.len(), 2);
    assert!(
        windows
            .iter()
            .all(|w| !w.window_id.contains("host") || w.window_id.starts_with("host:"))
    );
    assert!(
        windows
            .iter()
            .all(|w| !w.window_id.contains(SESSION) && !w.window_id.contains("guard-"))
    );
    assert_eq!(
        kit.guard.charge_now(&account("host"), &session(), 4_194_305),
        TrafficDecision::RateLimited
    );
    let stubbed = kit_on(
        &[("P2P_DISTRIBUTED_TRAFFIC_LIMITER", "true")],
        Some(Arc::new(StubGuardStore)),
    );
    assert_eq!(
        stubbed.guard.charge_now(&account("host"), &session(), 10),
        TrafficDecision::Unavailable
    );
}

#[test]
fn route_level_rate_limit() {
    let kit = kit(&[EVIDENCE_ON]);
    let body = json!({"schema": 2});
    for _ in 0..120 {
        assert_eq!(seam_code(post(&kit, 0, &body)), "GUARD_BATCH_INVALID");
    }
    let limited = post(&kit, 0, &body);
    assert_eq!(seam_code(limited), "GUARD_TRAFFIC_RATE_LIMITED");
    assert_eq!(crate::status_of("GUARD_TRAFFIC_RATE_LIMITED"), 429);
}

#[test]
fn rate_limit_buckets_are_cached_and_never_logged() {
    let kit = kit(&[]);
    let digest = sha256_hex(TOKEN.as_bytes());
    kit.hunts.capabilities.lock().unwrap().insert(
        digest.clone(),
        CapabilityView {
            capability_id: "cap-9".into(),
            expires_at: UnixMillis(NOW + 60_000),
            revoked: false,
        },
    );
    let request = |token: Option<&str>| RequestInfo {
        headers: token
            .map(|t| vec![(HOST_CAPABILITY_HEADER.to_owned(), t.to_owned())])
            .unwrap_or_default(),
        ..RequestInfo::default()
    };
    let bucket = |token: Option<&str>| poll_once(kit.guard.rate_limit_bucket(request(token))).unwrap().unwrap();
    assert_eq!(bucket(Some(TOKEN)), Some("host:cap-9".to_owned()));
    assert_eq!(bucket(Some(TOKEN)), Some("host:cap-9".to_owned()));
    assert_eq!(kit.hunts.lookups.load(Ordering::SeqCst), 1);
    kit.clock.advance(10_000);
    assert_eq!(bucket(Some(TOKEN)), Some("host:cap-9".to_owned()));
    assert_eq!(kit.hunts.lookups.load(Ordering::SeqCst), 2);
    assert_eq!(bucket(None), None);
    assert_eq!(bucket(Some("short")), None);
    assert_eq!(kit.hunts.lookups.load(Ordering::SeqCst), 2);
    let unknown = "zbcdefghijklmnopqrstuvwxyzABCDEFGHIJ0123-_x";
    assert_eq!(bucket(Some(unknown)), None);
    kit.hunts.capabilities.lock().unwrap().get_mut(&digest).unwrap().revoked = true;
    kit.clock.advance(10_000);
    assert_eq!(bucket(Some(TOKEN)), None);
    kit.hunts.failing.store(true, Ordering::SeqCst);
    kit.clock.advance(10_000);
    assert_eq!(bucket(Some(TOKEN)), None);
    assert!(
        kit.lines()
            .iter()
            .all(|line| !line.contains(TOKEN) && !line.contains(&digest))
    );
}

// ---- host writes ----

fn authorize(kit: &Kit, kind: HostWriteKind, target: &str, options: HostWriteOptions) -> Option<Reply> {
    poll_once(
        kit.guard
            .authorize_host_write(runtime(), kind, account(target), options),
    )
    .unwrap()
    .unwrap()
}

const PLAIN: HostWriteOptions = HostWriteOptions {
    probe: false,
    parity: false,
};
const PROBE: HostWriteOptions = HostWriteOptions {
    probe: true,
    parity: false,
};
const PARITY: HostWriteOptions = HostWriteOptions {
    probe: false,
    parity: true,
};

#[test]
fn observe_logs_a_would_refuse_and_lets_hub_xp_through() {
    let kit = kit(&[]);
    kit.hunts.put(hub(HubKind::Ramsgate));
    kit.hunts.set_presence(&["guest"]);
    assert_eq!(authorize(&kit, HostWriteKind::Xp, "guest", PLAIN), None);
    assert_eq!(
        kit.lines(),
        [format!(
            "warn [HostPolicyObserve] would refuse write kind=XP target=guest session={SESSION} reason=WRITE_KIND_NOT_ALLOWED_IN_HUB hub=RAMSGATE"
        )]
    );
    // the identity check still refuses a stranger, with its own reason
    let refused = authorize(&kit, HostWriteKind::Xp, "stranger", PLAIN).unwrap();
    assert_eq!(refused.status, 403);
    assert!(
        kit.lines()
            .last()
            .unwrap()
            .ends_with("reason=MEMBER_NOT_PRESENT hub=RAMSGATE")
    );
}

#[test]
fn probes_are_strict_and_quiet() {
    let kit = kit(&[]);
    kit.hunts.put(hub(HubKind::Ramsgate));
    kit.hunts.set_presence(&["guest"]);
    assert!(authorize(&kit, HostWriteKind::Xp, "guest", PROBE).is_some());
    assert!(authorize(&kit, HostWriteKind::Mastery, "stranger", PROBE).is_some());
    assert!(authorize(&kit, HostWriteKind::Mastery, "guest", PROBE).is_none());
    assert!(kit.lines().is_empty());
}

#[test]
fn training_reforge_is_observed() {
    let kit = kit(&[]);
    kit.hunts.put(hub(HubKind::Training));
    kit.hunts.set_presence(&["guest"]);
    assert_eq!(authorize(&kit, HostWriteKind::Reforge, "guest", PLAIN), None);
    assert!(kit.lines()[0].contains("kind=REFORGE") && kit.lines()[0].ends_with("hub=TRAINING"));
}

#[test]
fn parity_is_quiet_in_observe_and_refused_in_enforce() {
    let kit = kit(&[]);
    assert_eq!(authorize(&kit, HostWriteKind::Reforge, "member", PARITY), None);
    let refused = authorize(&kit, HostWriteKind::Xp, "stranger", PARITY).unwrap();
    assert!(refused.body.0.contains("This host may not write this player's state."));
    assert!(kit.lines().is_empty());
    let enforcing = crate::testkit::kit(&[("RUSTIC_RIFTS_HOST_POLICY", "enforce")]);
    let reply = authorize(&enforcing, HostWriteKind::Xp, "member", PARITY).unwrap();
    assert_eq!(
        reply.body.0,
        r#"{"code":"P2P_HOST_WRITE_DENIED","message":"This host may not do a dedicated server's work here.","payload":null}"#
    );
    assert!(enforcing.lines().is_empty());
}

#[test]
fn enforce_refuses_kinds() {
    let kit = kit(&[("RUSTIC_RIFTS_HOST_POLICY", "enforce")]);
    assert!(authorize(&kit, HostWriteKind::Reforge, "member", PLAIN).is_some());
    assert_eq!(
        kit.lines(),
        [format!(
            "warn [HostWrite] refused kind=REFORGE target=member session={SESSION} reason=WRITE_KIND_NOT_ALLOWED"
        )]
    );
    assert!(authorize(&kit, HostWriteKind::Xp, "member", PLAIN).is_none());
    assert_eq!(poll_once(async { kit.guard.host_policy_refuses() }).unwrap(), Ok(true));
}

#[test]
fn the_kill_switch() {
    let kit = kit(&[("P2P_HOST_WRITES", "off")]);
    let reply = authorize(&kit, HostWriteKind::Xp, "member", PLAIN).unwrap();
    assert_eq!(
        reply.body.0,
        r#"{"code":"P2P_HOST_WRITE_DENIED","message":"Host writes are switched off.","payload":null}"#
    );
    assert_eq!(
        kit.lines(),
        ["warn [HostWrite] refused kind=XP target=member reason=HOST_WRITES_OFF"]
    );
    assert!(authorize(&kit, HostWriteKind::Xp, "member", PROBE).is_some());
    assert_eq!(kit.lines().len(), 1);
}

#[test]
fn recorded_writes_carry_the_stored_hunts_tags() {
    let kit = kit(&[]);
    let write = |kind| HostWrite {
        kind,
        target: account("guest"),
        summary: Json(r#"{"xp":5}"#.into()),
    };
    kit.hunts.put(hub(HubKind::Ramsgate));
    poll_once(kit.guard.record_host_write(runtime(), write(HostWriteKind::Mastery)))
        .unwrap()
        .unwrap();
    kit.hunts.put(hunt());
    poll_once(kit.guard.record_host_write(runtime(), write(HostWriteKind::Xp)))
        .unwrap()
        .unwrap();
    let gone = RuntimeAuth {
        session: HUB_SESSION.parse().unwrap(),
        ..runtime()
    };
    poll_once(kit.guard.record_host_write(gone, write(HostWriteKind::Xp)))
        .unwrap()
        .unwrap();
    let rows = kit.store.host_writes();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].realm.as_ref().map(ToString::to_string).as_deref(), Some(REALM));
    assert_eq!(
        (rows[0].session_kind.as_deref(), rows[0].hub_kind.as_deref()),
        (Some("REALM_HUB"), Some("RAMSGATE"))
    );
    assert_eq!(
        (
            rows[1].realm.clone(),
            rows[1].session_kind.clone(),
            rows[1].hub_kind.clone()
        ),
        (None, None, None)
    );
    assert_eq!(rows[1].host, account("host"));
    assert_eq!(
        kit.lines(),
        [
            format!(
                "info [HostWrite] kind=MASTERY target=guest host=host session={SESSION} generation=1 realm={REALM} hub=RAMSGATE summary={{\"xp\":5}}"
            ),
            format!(
                "info [HostWrite] kind=XP target=guest host=host session={SESSION} generation=1 summary={{\"xp\":5}}"
            ),
            "warn [HostWrite] record without authorize kind=XP target=guest".to_owned(),
        ]
    );
}

// ---- inventory ----

fn transaction(user: &str, character: &str, body: Option<Value>) -> InventoryTransaction {
    InventoryTransaction {
        user: account(user),
        character_id: character.into(),
        transaction_id: "tx-1".into(),
        body: body.map(|b| Json(stringify(&b))),
    }
}

fn authorize_inventory(kit: &Kit, tx: InventoryTransaction) -> InventoryDecision {
    poll_once(kit.guard.authorize_inventory(runtime(), tx))
        .unwrap()
        .unwrap()
}

fn fence(decision: InventoryDecision) -> Box<dyn InventoryFence> {
    match decision {
        InventoryDecision::Authorized(fence) => fence,
        InventoryDecision::Refused(reply) => panic!("refused: {}", reply.body.0),
    }
}

fn refused(decision: InventoryDecision) -> Reply {
    match decision {
        InventoryDecision::Refused(reply) => reply,
        InventoryDecision::Authorized(_) => panic!("authorized"),
    }
}

struct Reads(Result<Option<Json>, SeamError>);

impl TransactionReads for Reads {
    fn stacked_items(&self, _character_id: &str) -> Result<Option<Json>, SeamError> {
        self.0.clone()
    }
}

fn draft_body() -> Value {
    json!({"addStackedItems": [{"catalogId": HOST_DRAFT_PREMIUM_ID, "quantity": 1}]})
}

#[test]
fn fence_rechecks_presence() {
    let kit = kit(&[]);
    kit.hunts.put(hub(HubKind::Ramsgate));
    kit.hunts.set_presence(&["guest"]);
    let fence = fence(authorize_inventory(&kit, transaction("guest", "c", None)));
    assert!(fence.on_grant(&Json("{}".into())).is_ok());
    kit.hunts.set_presence(&[]);
    let reply = fence.on_grant(&Json("{}".into())).unwrap_err();
    assert_eq!(
        reply.body.0,
        r#"{"code":"P2P_LOOT_GRANT_PARTICIPANT_NOT_ADMITTED","message":"Player-hosted hunt loot grant is not authorized.","payload":null}"#
    );
    assert!(kit.lines().last().unwrap().starts_with(
        "warn [P2PInventory] transaction denied code=P2P_LOOT_GRANT_PARTICIPANT_NOT_ADMITTED account=guest character=c"
    ));
    kit.hunts.failing.store(true, Ordering::SeqCst);
    assert!(
        fence
            .on_grant(&Json("{}".into()))
            .unwrap_err()
            .body
            .0
            .contains("P2P_LOOT_GRANT_DENIED")
    );
}

#[test]
fn inventory_refusal_logs_and_answers_denied() {
    let kit = kit(&[]);
    let reply = refused(authorize_inventory(&kit, transaction("stranger", "c", None)));
    assert!(reply.body.0.contains(r#""code":"P2P_LOOT_GRANT_DENIED""#));
    assert_eq!(
        kit.lines(),
        [format!(
            "warn [P2PInventory] rejected code=PARTICIPANT_NOT_ADMITTED account=stranger session={SESSION} generation=1"
        )]
    );
}

#[test]
fn grant_classes_observed_or_enforced() {
    let body = json!({"addStackedItems": [{"catalogId": "CURRENCY_PLATINUM", "quantity": 5}]});
    let observing = kit(&[]);
    let fence = fence(authorize_inventory(
        &observing,
        transaction("member", "char-m", Some(body.clone())),
    ));
    fence.report(TransactionPhase::Applying);
    fence.report(TransactionPhase::Accepted);
    fence.report(TransactionPhase::Failed);
    let tags = format!("account=member character=char-m session={SESSION} generation=1 transaction=tx-1");
    assert_eq!(
        observing.lines(),
        [
            format!(
                "warn [HostPolicyObserve] would refuse grant account=member session={SESSION} generation=1 transaction=tx-1 classes=PREMIUM_CURRENCY denied=CURRENCY_PLATINUM(PREMIUM_CURRENCY,addStackedItems)"
            ),
            format!("info [P2PInventory] applying {tags}"),
            format!("info [P2PInventory] transaction accepted {tags}"),
            format!("error [P2PInventory] transaction failed {tags}"),
        ]
    );
    let enforcing = crate::testkit::kit(&[("RUSTIC_RIFTS_HOST_POLICY", "enforce")]);
    let reply = refused(authorize_inventory(
        &enforcing,
        transaction("member", "char-m", Some(body)),
    ));
    assert_eq!(reply.status, 403);
    assert_eq!(
        reply.body.0,
        r#"{"code":"P2P_HOST_GRANT_DENIED","message":"A player host may not grant this item class.","payload":{"classes":["PREMIUM_CURRENCY"]}}"#
    );
    assert!(enforcing.lines()[0].starts_with("warn [HostGrantDeny] account=member"));
}

#[test]
fn no_reward_sessions_refuse_any_grant() {
    let kit = kit(&[("RUSTIC_RIFTS_HOST_POLICY", "enforce")]);
    let mut no_reward = hunt();
    no_reward.completion_mode = Some(crate::CompletionMode::NoCompletion);
    kit.hunts.put(no_reward);
    let body = json!({"addStackedItems": [{"catalogId": "WEAPON_SWORD", "quantity": 1}]});
    let reply = refused(authorize_inventory(&kit, transaction("member", "char-m", Some(body))));
    assert!(reply.body.0.contains(r#""classes":["NO_REWARD_SESSION"]"#));
    assert!(kit.lines()[0].contains("classes=NO_REWARD_SESSION denied=any grant or spend"));
}

#[test]
fn draft_over_the_cap_early_and_inside_the_transaction() {
    let kit = kit(&[("RUSTIC_RIFTS_HOST_POLICY", "enforce")]);
    let held = format!(r#"[{{"catalogId":"{HOST_DRAFT_PREMIUM_ID}","quantity":1}}]"#);
    *kit.core.stacked.lock().unwrap() = Ok(Some(Json(held.clone())));
    let reply = refused(authorize_inventory(
        &kit,
        transaction("member", "char-m", Some(draft_body())),
    ));
    assert!(reply.body.0.contains("P2P_HOST_GRANT_DENIED"));
    assert!(kit.lines()[0].contains(" draftPremium=OVER_BALANCE_CAP"));

    // allowed early, decided again from the transaction's own reads
    let kit = crate::testkit::kit(&[("RUSTIC_RIFTS_HOST_POLICY", "enforce")]);
    let fence = fence(authorize_inventory(
        &kit,
        transaction("member", "char-m", Some(draft_body())),
    ));
    let reply = fence
        .on_grant_in(&Json("{}".into()), &Reads(Ok(Some(Json(held)))))
        .unwrap_err();
    assert!(
        reply
            .body
            .0
            .contains(r#""code":"P2P_LOOT_GRANT_DRAFT_PREMIUM_OVER_BALANCE_CAP""#)
    );
    let lines = kit.lines();
    assert_eq!(
        lines[lines.len() - 1],
        format!(
            "warn [HostGrantDeny] account=member session={SESSION} generation=1 transaction=tx-1 item={HOST_DRAFT_PREMIUM_ID} quantity=1 reason=OVER_BALANCE_CAP at=transaction"
        )
    );
    assert!(
        lines[lines.len() - 2]
            .starts_with("warn [P2PInventory] transaction denied code=P2P_LOOT_GRANT_DRAFT_PREMIUM_OVER_BALANCE_CAP")
    );
}

#[test]
fn draft_allowed_inside_logs_the_allow_once() {
    let kit = kit(&[("RUSTIC_RIFTS_HOST_POLICY", "enforce")]);
    let fence = fence(authorize_inventory(
        &kit,
        transaction("member", "char-m", Some(draft_body())),
    ));
    fence.on_grant_in(&Json("{}".into()), &Reads(Ok(None))).unwrap();
    fence.report(TransactionPhase::Accepted);
    fence.report(TransactionPhase::Accepted);
    let allows: Vec<String> = kit
        .lines()
        .into_iter()
        .filter(|l| l.contains("[HostGrantAllow]"))
        .collect();
    assert_eq!(
        allows,
        [format!(
            "info [HostGrantAllow] account=member session={SESSION} generation=1 transaction=tx-1 item={HOST_DRAFT_PREMIUM_ID} quantity=1 reason=WITHIN_BALANCE_CAP day=2026-10-14 balance=0->1 cap=1"
        )]
    );
}

#[test]
fn unreadable_balance_is_observed() {
    let kit = kit(&[]);
    *kit.core.stacked.lock().unwrap() = Err(SeamError::unimplemented(Component::Isles, "stacked_items"));
    let fence = fence(authorize_inventory(
        &kit,
        transaction("member", "char-m", Some(draft_body())),
    ));
    assert!(kit.lines()[0].contains(" draftPremium=BALANCE_UNREADABLE"));
    fence.on_grant(&Json("{}".into())).unwrap();
    assert!(
        kit.lines()
            .last()
            .unwrap()
            .starts_with("warn [HostPolicyObserve] would refuse draft premium account=member")
    );
    assert!(
        kit.lines()
            .last()
            .unwrap()
            .ends_with("reason=BALANCE_UNREADABLE at=transaction")
    );
    let reads = Reads(Ok(Some(Json("{}".into()))));
    fence.on_grant_in(&Json("{}".into()), &reads).unwrap();
}

#[test]
fn accepted_replay_skips_the_draft_decision() {
    let kit = kit(&[("RUSTIC_RIFTS_HOST_POLICY", "enforce")]);
    kit.core.replay.store(true, Ordering::SeqCst);
    *kit.core.stacked.lock().unwrap() = Ok(Some(Json("not json".into())));
    let fence = fence(authorize_inventory(
        &kit,
        transaction("member", "char-m", Some(draft_body())),
    ));
    fence
        .on_grant_in(&Json("{}".into()), &Reads(Ok(Some(Json("not json".into())))))
        .unwrap();
    assert!(kit.lines().iter().all(|l| !l.contains("draft")));
}

#[test]
fn token_beside_another_hunt_pass_item() {
    let kit = kit(&[]);
    let body = json!({"addStackedItems": [
        {"catalogId": HOST_DRAFT_PREMIUM_ID, "quantity": 1},
        {"catalogId": "TOKEN_HUNTPASS_SKIP", "quantity": 1}
    ]});
    fence(authorize_inventory(&kit, transaction("member", "char-m", Some(body))));
    assert!(kit.lines()[0].ends_with("classes=HUNT_PASS denied=TOKEN_HUNTPASS_SKIP(HUNT_PASS,addStackedItems)"));
}

// ---- operations ----

fn admin(kit: &Kit, route: usize, body: &Value) -> Reply {
    let routes = GuardRoutes::new(kit.guard.clone());
    let caller = Caller::Admin {
        key_id: "ops-key".into(),
    };
    handle(&routes, call(route, caller, stringify(body).as_bytes(), &[])).unwrap()
}

fn rollout(version: u64, grants: bool) -> Value {
    json!({"version": version, "tier0ShadowEnabled": true, "tier1GrantsEnabled": grants})
}

#[test]
fn reward_rollout_replies() {
    let kit = kit(&[]);
    let invalid = admin(
        &kit,
        4,
        &json!({"rewardRollout": rollout(1, false), "expectedVersion": 0, "reason": "short"}),
    );
    assert_eq!(
        (invalid.status, invalid.body.0.as_str()),
        (
            400,
            r#"{"error":{"code":"ADMIN_VALIDATION","message":"A complete reward rollout, expectedVersion, and audit reason of at least 8 characters are required."}}"#
        )
    );
    let stale = admin(
        &kit,
        4,
        &json!({"rewardRollout": rollout(1, false), "expectedVersion": "4", "reason": "rolling out"}),
    );
    assert_eq!(stale.status, 409);
    assert!(stale.body.0.starts_with(r#"{"error":{"code":"ADMIN_POLICY_CONFLICT","message":"The operations policy changed. Refresh it before trying again."},"policy":{"version":0,"#));
    let behind = admin(
        &kit,
        4,
        &json!({"rewardRollout": rollout(0, false), "expectedVersion": null, "reason": "rolling out"}),
    );
    assert_eq!(behind.status, 409);
    assert!(behind.body.0.contains("ADMIN_REWARD_ROLLOUT_CONFLICT"));
    let weakening = admin(
        &kit,
        4,
        &json!({"rewardRollout": rollout(1, true), "expectedVersion": [0], "reason": "rolling out"}),
    );
    assert_eq!(weakening.status, 403);
    assert!(
        weakening
            .body
            .0
            .ends_with(r#""weakeningCodes":["TIER1_REWARD_GRANTS_ENABLE"]}"#)
    );
    let audit = kit.operations.audit();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].action, "operations.policy.weakening_rejected");
    assert_eq!(audit[0].ip, "10.0.0.7");
    assert_eq!(
        audit[0].new_state.0,
        r#"{"rewardRollout":{"version":1,"tier0ShadowEnabled":true,"tier1GrantsEnabled":true},"weakeningCodes":["TIER1_REWARD_GRANTS_ENABLE"]}"#
    );

    kit.core.step_up.store(true, Ordering::SeqCst);
    let ok = admin(
        &kit,
        4,
        &json!({"rewardRollout": rollout(1, true), "expectedVersion": 0, "reason": "rolling out", "breakGlassTotp": "123456"}),
    );
    assert_eq!(ok.status, 200);
    let answer: Value = serde_json::from_str(&ok.body.0).unwrap();
    assert_eq!(answer["policy"]["version"], json!(1));
    let audit = kit.operations.audit();
    let update = audit.last().unwrap();
    assert_eq!(update.action, "operations.reward_rollout.update");
    assert_eq!(answer["requestId"], json!(update.request_id));
    assert_eq!(update.actor, "ops-key");
    assert_eq!(update.reason, "rolling out");
    assert!(
        update
            .new_state
            .0
            .ends_with(r#""coopTransport":"ICE","weakeningCodes":["TIER1_REWARD_GRANTS_ENABLE"]}"#)
    );
    assert_eq!(update.created_at, "2026-10-14T17:46:40.000Z");
}

#[test]
fn core_policy_reads_isles_and_writes_to_memory() {
    let core = Arc::new(crate::testkit::FakeCore::default());
    *core.policy.lock().unwrap() = Ok(Json(
        r#"{"version":7,"rewardRollout":{"version":2,"tier0ShadowEnabled":false,"tier1GrantsEnabled":false}}"#.into(),
    ));
    let policy = crate::CoreOperationsPolicy::new(core.clone());
    let snapshot = poll_once(crate::OperationsPolicyPort::policy(&policy))
        .unwrap()
        .unwrap();
    assert_eq!((snapshot.version, snapshot.reward_rollout.version), (7, 2));
    *core.policy.lock().unwrap() = Ok(Json(r#"{"version":-1}"#.into()));
    let odd = poll_once(crate::OperationsPolicyPort::policy(&policy))
        .unwrap()
        .unwrap();
    assert_eq!((odd.version, odd.reward_rollout), (0, crate::DEFAULT_REWARD_ROLLOUT));
    *core.policy.lock().unwrap() = Err(SeamError::unimplemented(Component::Isles, "operations_policy"));
    let fallback = poll_once(crate::OperationsPolicyPort::policy(&policy))
        .unwrap()
        .unwrap();
    assert_eq!(
        fallback,
        poll_once(crate::OperationsPolicyPort::policy(&MemoryOperations::default()))
            .unwrap()
            .unwrap()
    );
}

#[test]
fn alert_acknowledgement() {
    let kit = kit(&[]);
    let alert = crate::AlertRecord {
        alert_id: "alert-0001".into(),
        code: "TRAFFIC".into(),
        severity: "HIGH".into(),
        message: "m".into(),
        details: Json::empty_object(),
        created_at: UnixMillis(1),
        acknowledged_at: None,
        acknowledged_by: None,
    };
    kit.store
        .insert_alert(&rifts_seam::Actor::System("test"), &alert)
        .unwrap();
    let invalid = admin(&kit, 5, &json!({"reason": "x"}));
    assert_eq!(invalid.status, 400);
    assert!(
        invalid
            .body
            .0
            .contains("A valid alert ID and an audit reason of at least 8 characters are required.")
    );
    let ok = admin(&kit, 5, &json!({"reason": "handled it"}));
    assert_eq!(ok.status, 200);
    let answer: Value = serde_json::from_str(&ok.body.0).unwrap();
    let keys: Vec<&String> = answer.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["alertId", "status", "acknowledgedAt", "requestId"]);
    assert_eq!(answer["acknowledgedAt"], json!("2026-10-14T17:46:40.000Z"));
    let audit = kit.operations.audit();
    assert_eq!(audit[0].action, "operations.alert.acknowledge");
    assert_eq!(audit[0].old_state.0, r#"{"alertId":"alert-0001","status":"OPEN"}"#);
    assert_eq!(
        audit[0].new_state.0,
        r#"{"alertId":"alert-0001","status":"ACKNOWLEDGED","acknowledgedAt":"2026-10-14T17:46:40.000Z"}"#
    );
    assert_eq!(
        kit.store.alert("alert-0001").unwrap().acknowledged_by.as_deref(),
        Some("ops-key")
    );
    let again = admin(&kit, 5, &json!({"reason": "handled it"}));
    assert_eq!(
        (again.status, again.body.0.as_str()),
        (
            404,
            r#"{"error":{"code":"ADMIN_ALERT_NOT_OPEN","message":"The alert is missing or has already been acknowledged."}}"#
        )
    );
    let routes = GuardRoutes::new(kit.guard.clone());
    assert_eq!(
        seam_code(handle(&routes, call(5, Caller::Anonymous, b"{}", &[]))),
        "AUTH_UNAUTHORIZED"
    );
    let direct = poll_once(
        kit.guard
            .acknowledge_alert("k".into(), " short ".into(), None, "ip".into()),
    )
    .unwrap();
    assert_eq!(direct.unwrap().status, 400);
}

#[test]
fn internal_errors_are_logged_and_hidden() {
    let kit = kit(&[EVIDENCE_ON]);
    kit.hunts.failing.store(true, Ordering::SeqCst);
    let body = transport_body(1, GENESIS_HASH, "a", &[("SESSION_STARTED", None)]);
    assert_eq!(
        post(&kit, 0, &body),
        Err(SeamError::Refused(Refusal {
            code: "INTERNAL".into(),
            message: "The player-hosted hunt request failed.".into()
        }))
    );
    assert_eq!(
        kit.lines(),
        ["error Unhandled P2P route error requestId=req-1: hunts are down"]
    );
}

// ---- lifecycle and the stub ----

#[test]
fn lifecycle() {
    let kit = kit(&[("RUSTIC_RIFTS_HOST_POLICY", "enforce"), ("P2P_TRUST_TIER", "1")]);
    let guard = kit.guard.as_ref();
    assert_eq!(
        guard.startup_summary("all").unwrap().unwrap().0,
        r#"{"antiCheatLoaded":false,"hostPolicy":"enforce","huntEvidence":true,"trustTier":1}"#
    );
    guard.start("all").unwrap();
    assert_eq!(kit.lines().len(), 2);
    let steps: Vec<(&str, &str)> = guard
        .account_deletion_steps()
        .iter()
        .map(|s| (s.table, s.account_column))
        .collect();
    assert_eq!(
        steps,
        [
            ("rifts_host_writes", "target"),
            ("rifts_host_writes", "host"),
            ("rifts_hunt_xp_summaries", "member")
        ]
    );
    assert_eq!(guard.tables().len(), 7);
    assert!(
        guard
            .runtime_safety_issues(&crate::testkit::settings(&[]))
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        guard.guard_manifest_roles(),
        ["TRANSPORT_MODULE", "TRANSPORT_RUNTIME", "ADMISSION_KEY"]
    );
    assert!(guard.service_roles().is_empty());
    guard
        .census(&rifts_seam::HostRouteSample {
            method: "GET".into(),
            route: "/x".into(),
            status_code: 200,
            ..Default::default()
        })
        .unwrap();
    poll_once(guard.stop()).unwrap().unwrap();
    assert!(
        kit.lines()
            .last()
            .unwrap()
            .starts_with("info [HostRoute] summary {\"windowSeconds\":60,\"total\":1")
    );
    guard.note_would_refuse("xp", "account=a").unwrap();
    assert_eq!(
        kit.lines().last().unwrap(),
        "warn [HostPolicyObserve] would refuse xp account=a"
    );
    let hooks: Arc<dyn GuardHooks> = kit.guard.clone();
    assert!(!hooks.tables().is_empty());
}

#[test]
fn the_stub_answers_unimplemented_by_method() {
    let stub = Stub;
    let name = |error: SeamError| match error {
        SeamError::Unimplemented(missing) => missing.operation,
        other => panic!("{other:?}"),
    };
    let guard_name = |error: GuardError| match error {
        GuardError::Unimplemented(missing) => missing.operation,
        other => panic!("{other:?}"),
    };
    let settings = crate::testkit::settings(&[]);
    let mut names = vec![
        name(stub.runtime_safety_issues(&settings).unwrap_err()),
        name(stub.startup_summary("r").unwrap_err()),
        name(stub.start("r").unwrap_err()),
        name(poll_once(stub.stop()).unwrap().unwrap_err()),
        name(stub.census(&rifts_seam::HostRouteSample::default()).unwrap_err()),
        name(
            poll_once(stub.rate_limit_bucket(RequestInfo::default()))
                .unwrap()
                .unwrap_err(),
        ),
    ];
    match poll_once(stub.authorize_inventory(runtime(), transaction("a", "c", None))).unwrap() {
        Err(error) => names.push(name(error)),
        Ok(_) => panic!("authorized"),
    }
    names.extend([
        name(
            poll_once(stub.authorize_host_write(runtime(), HostWriteKind::Xp, account("a"), PLAIN))
                .unwrap()
                .unwrap_err(),
        ),
        name(
            poll_once(stub.record_host_write(
                runtime(),
                HostWrite {
                    kind: HostWriteKind::Xp,
                    target: account("a"),
                    summary: Json::empty_object(),
                },
            ))
            .unwrap()
            .unwrap_err(),
        ),
        name(stub.host_policy_refuses().unwrap_err()),
        name(stub.note_would_refuse("a", "b").unwrap_err()),
        guard_name(
            poll_once(stub.submit_evidence(runtime(), Json::empty_object()))
                .unwrap()
                .unwrap_err(),
        ),
        guard_name(
            poll_once(stub.complete(runtime(), Json::empty_object()))
                .unwrap()
                .unwrap_err(),
        ),
        guard_name(poll_once(stub.verdict(session(), 1)).unwrap().unwrap_err()),
        guard_name(
            poll_once(stub.reward_receipt(session(), 1, account("a")))
                .unwrap()
                .unwrap_err(),
        ),
        guard_name(
            poll_once(stub.record_shadow_comparison(None, Json::empty_object()))
                .unwrap()
                .unwrap_err(),
        ),
        guard_name(poll_once(stub.charge(account("a"), session(), 1)).unwrap().unwrap_err()),
        guard_name(
            poll_once(stub.set_reward_rollout("k".into(), None, "ip".into()))
                .unwrap()
                .unwrap_err(),
        ),
        guard_name(
            poll_once(stub.acknowledge_alert("k".into(), "a".into(), None, "ip".into()))
                .unwrap()
                .unwrap_err(),
        ),
    ]);
    assert_eq!(
        names,
        [
            "runtime_safety_issues",
            "startup_summary",
            "start",
            "stop",
            "census",
            "rate_limit_bucket",
            "authorize_inventory",
            "authorize_host_write",
            "record_host_write",
            "host_policy_refuses",
            "note_would_refuse",
            "submit_evidence",
            "complete",
            "verdict",
            "reward_receipt",
            "record_shadow_comparison",
            "charge",
            "set_reward_rollout",
            "acknowledge_alert",
        ]
    );
    assert_eq!(stub.tables().len(), 7);
    assert!(stub.account_deletion_steps().is_empty());
    assert!(stub.service_roles().is_empty());
    assert_eq!(stub.guard_manifest_roles().len(), 3);
    let _ = CapabilityScope::Abort;
}
