//! Guard on in-memory parts: the stub module's answers, the evidence chain through the routes, the hooks and the
//! admin operations.

use std::sync::Arc;

use rifts_seam::{
    Actor, Caller, Component, GuardHooks, HostWrite, HostWriteKind, HostWriteOptions, HubKind, InventoryDecision,
    InventoryFence, InventoryTransaction, Json, Lifecycle, Method, Refusal, Reply, RequestInfo, Route, RouteCall,
    RouteHandler, SeamError, Secret, StoreError, TransactionPhase, TransactionReads, Unimplemented, UnixMillis,
    poll_once,
};
use serde_json::{Value, json};

use crate::evidence::tests::batch_body;
use crate::ports::lock;
use crate::testkit::{Kit, kit};
use crate::writes::tests::{account, hub, hunt, runtime, session};
use crate::*;

fn refused(code: &str) -> impl Fn(&GuardError) -> bool + '_ {
    move |error| error.code() == Some(code)
}

fn route(handler: &str) -> Route {
    *ROUTES.iter().find(|route| route.handler == handler).unwrap()
}

fn call(handler: &str, caller: Caller, body: &Value) -> RouteCall {
    RouteCall {
        route: route(handler),
        params: vec![("sessionId".to_owned(), session().to_string())],
        query: String::new(),
        request: RequestInfo {
            path: String::new(),
            headers: vec![("content-type".to_owned(), "application/json".to_owned())],
            remote: Some("203.0.113.9".parse().unwrap()),
            request_id: "req-1".to_owned(),
        },
        caller,
        body: serde_json::to_vec(body).unwrap(),
    }
}

fn answer(kit: &Kit, call: RouteCall) -> Result<Reply, SeamError> {
    poll_once(GuardRoutes::new(kit.guard.clone()).handle(call)).unwrap()
}

fn refusal_code(result: Result<Reply, SeamError>) -> String {
    match result {
        Err(SeamError::Refused(Refusal { code, .. })) => code,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

fn reply_json(result: Result<Reply, SeamError>) -> Value {
    let reply = result.unwrap();
    assert_eq!(reply.status, 200);
    serde_json::from_str(&reply.body.0).unwrap()
}

const SUBMIT: &str = "antiCheat.ts:hunts.submitEvidence";
const COMPLETE: &str = "antiCheat.ts:hunts.complete";
const SHADOW: &str = "antiCheat.ts:hunts.recordShadowComparison";
const SELF_REWARD: &str = "p2pHuntStatus.ts:GetSelfHuntReward";
const ROLLOUT: &str = "p2pAdminOperations.ts:UpdateRewardRollout";
const ACK: &str = "p2pAdminOperations.ts:AcknowledgeOperationsAlert";

fn host() -> Caller {
    Caller::HostRuntime(runtime())
}

// antiCheat.test.ts: "without the anti-cheat module hunts collect no evidence and earn no rewards"
#[test]
fn without_evidence_hunts_collect_none_and_earn_no_rewards() {
    let kit = kit(&[]);
    let guard = &kit.guard;
    assert_eq!(guard.service_roles(), &[] as &[&str]);
    assert_eq!(
        guard.runtime_safety_issues(&std::collections::BTreeMap::new()),
        Ok(Vec::new())
    );
    assert_eq!(
        guard.reward_policy_status("hunt", Some("v1")),
        RewardPolicyStatus::NotReady {
            reason: "ANTICHEAT_MODULE_ABSENT"
        }
    );
    let not_collected = refused("HUNT_EVIDENCE_NOT_COLLECTED");
    assert!(not_collected(
        &poll_once(guard.submit_evidence(runtime(), Json::empty_object()))
            .unwrap()
            .unwrap_err()
    ));
    assert!(not_collected(
        &poll_once(guard.complete(runtime(), Json::empty_object()))
            .unwrap()
            .unwrap_err()
    ));
    let key = Some(Secret::new("key".to_owned()));
    assert!(not_collected(
        &poll_once(guard.record_shadow_comparison(key, Json::empty_object()))
            .unwrap()
            .unwrap_err()
    ));
    assert_eq!(poll_once(guard.verdict(session(), 1)).unwrap(), Ok(None));
    assert_eq!(
        poll_once(guard.reward_receipt(session(), 1, account("a"))).unwrap(),
        Ok(None)
    );
    assert_eq!(status_of("HUNT_EVIDENCE_NOT_COLLECTED"), 404);
}

#[test]
fn the_routes_answer_as_node_without_the_module() {
    let kit = kit(&[]);
    kit.hunts.put(hunt());
    let batch = batch_body("b-1", 1, GENESIS_HASH, &["SESSION_STARTED"]);
    assert_eq!(
        refusal_code(answer(&kit, call(SUBMIT, host(), &batch))),
        "HUNT_EVIDENCE_NOT_COLLECTED"
    );
    assert_eq!(
        refusal_code(answer(&kit, call(COMPLETE, host(), &json!({})))),
        "HUNT_EVIDENCE_NOT_COLLECTED"
    );
    assert_eq!(
        refusal_code(answer(&kit, call(SHADOW, Caller::Anonymous, &json!({})))),
        "HUNT_EVIDENCE_NOT_COLLECTED"
    );
    // GetSelfHuntReward: not a member is not found; a member hears that no rewards are granted
    let get = |name: &str| RouteCall {
        body: Vec::new(),
        ..call(SELF_REWARD, Caller::Launcher(account(name)), &json!({}))
    };
    assert_eq!(refusal_code(answer(&kit, get("stranger"))), "P2P_SESSION_NOT_FOUND");
    assert_eq!(refusal_code(answer(&kit, get("late"))), "HUNT_REWARDS_UNAVAILABLE");
    let mut unknown = get("friend");
    unknown.params = vec![(
        "sessionId".to_owned(),
        "11111111-1111-4111-8111-111111111111".to_owned(),
    )];
    assert_eq!(refusal_code(answer(&kit, unknown)), "P2P_SESSION_NOT_FOUND");
    // Each refusal still went through the Guard budget and traffic windows, and released the budget
    assert_eq!(kit.guard.request_budget().snapshot(), (0, 0));
}

#[test]
fn p2p_bodies_are_bounded_and_strict() {
    let kit = kit(&[("MYSTICPARADOX_HUNT_EVIDENCE", "on")]);
    kit.hunts.put(hunt());
    let mut big = call(SUBMIT, host(), &json!({}));
    big.body = vec![b' '; json::P2P_MAX_HTTP_BODY_BYTES + 1];
    assert_eq!(refusal_code(answer(&kit, big)), "GUARD_BATCH_TOO_LARGE");
    assert_eq!(status_of("GUARD_BATCH_TOO_LARGE"), 413);
    let mut broken = call(SUBMIT, host(), &json!({}));
    broken.body = b"{\"schema\":".to_vec();
    assert_eq!(refusal_code(answer(&kit, broken)), "GUARD_BATCH_INVALID");
    let mut scalar = call(SHADOW, Caller::Anonymous, &json!({}));
    scalar.body = b"42".to_vec();
    assert_eq!(
        refusal_code(answer(&kit, scalar)),
        "GUARD_BATCH_INVALID",
        "strict: objects and arrays only"
    );
    assert_eq!(
        refusal_code(answer(&kit, call(SUBMIT, Caller::Anonymous, &json!({})))),
        "P2P_CAPABILITY_INVALID"
    );
}

#[test]
fn the_evidence_chain_through_the_routes() {
    let kit = kit(&[("MYSTICPARADOX_HUNT_EVIDENCE", "on")]);
    kit.hunts.put(hunt());
    kit.guard.open_chain(session(), 2, Some("FIRST_DEFEAT")).unwrap();

    let first = batch_body("b-1", 1, GENESIS_HASH, &["SESSION_STARTED", "WORLD_READY"]);
    let reply = reply_json(answer(&kit, call(SUBMIT, host(), &first)));
    assert_eq!(reply["accepted"], json!(true));
    assert_eq!(reply["lastSequence"], json!(2));
    let head = reply["batchHash"].as_str().unwrap().to_owned();

    // The transport resends a batch it got no answer for: taken again, same hash
    let again = reply_json(answer(&kit, call(SUBMIT, host(), &first)));
    assert_eq!(
        (again["batchHash"].as_str(), again["duplicate"].as_bool()),
        (Some(head.as_str()), Some(true))
    );
    // The same id with other evidence is a replay
    let other = batch_body("b-1", 3, &head, &["DAMAGE"]);
    assert_eq!(
        refusal_code(answer(&kit, call(SUBMIT, host(), &other))),
        "GUARD_BATCH_REPLAY"
    );
    // A batch that skips the head conflicts and leaves a finding
    let gap = batch_body("b-2", 5, &head, &["DAMAGE"]);
    assert_eq!(
        refusal_code(answer(&kit, call(SUBMIT, host(), &gap))),
        "GUARD_CHAIN_CONFLICT"
    );
    assert_eq!(kit.store.findings(&session(), 2).unwrap()[0].code, "CHAIN_CONFLICT");
    assert!(
        kit.log
            .lines()
            .iter()
            .any(|line| line.starts_with("warn [HuntGuardObserve] would refuse CHAIN_CONFLICT"))
    );

    let last = batch_body("b-3", 3, &head, &["BEHEMOTH_DEFEATED", "COMPLETION_REQUESTED"]);
    let reply = reply_json(answer(&kit, call(SUBMIT, host(), &last)));
    assert_eq!(reply["encounterState"], json!("COMPLETION_REQUESTED"));
    assert_eq!(
        reply["disposition"],
        json!("OBSERVE_FLAGGED"),
        "the conflict's finding flags the hunt"
    );
    let final_hash = reply["batchHash"].as_str().unwrap().to_owned();
    assert_eq!(
        poll_once(kit.guard.verdict(session(), 2)).unwrap(),
        Ok(None),
        "no verdict before completion"
    );

    let completion = json!({"finalSequence": 4, "finalHash": final_hash});
    let done = reply_json(answer(&kit, call(COMPLETE, host(), &completion)));
    assert_eq!(
        (done["accepted"].as_bool(), done["progressionApplied"].as_bool()),
        (Some(true), Some(false))
    );
    assert_eq!(
        poll_once(kit.guard.verdict(session(), 2)).unwrap(),
        Ok(Some(HuntVerdict::OBSERVED))
    );
    let status = kit.guard.chain_status(&session(), 2).unwrap().unwrap();
    assert_eq!(
        (status.batch_count, status.event_count, status.finding_count),
        (2, 4, 1)
    );
    // Closed: more evidence is refused, the same completion is taken again
    let late = batch_body("b-4", 5, &final_hash, &["DAMAGE"]);
    assert_eq!(
        refusal_code(answer(&kit, call(SUBMIT, host(), &late))),
        "GUARD_CHAIN_FROZEN"
    );
    reply_json(answer(&kit, call(COMPLETE, host(), &completion)));
}

#[test]
fn evidence_from_another_generation_or_a_hub_is_refused() {
    let kit = kit(&[("MYSTICPARADOX_HUNT_EVIDENCE", "on")]);
    let batch = batch_body("b-1", 1, GENESIS_HASH, &["SESSION_STARTED"]);
    assert_eq!(
        refusal_code(answer(&kit, call(SUBMIT, host(), &batch))),
        "P2P_GENERATION_FENCED"
    );
    kit.hunts.put(HuntView {
        generation: 3,
        ..hunt()
    });
    assert_eq!(
        refusal_code(answer(&kit, call(SUBMIT, host(), &batch))),
        "P2P_GENERATION_FENCED"
    );
    kit.hunts.put(hub(HubKind::Ramsgate));
    assert_eq!(
        refusal_code(answer(&kit, call(SUBMIT, host(), &batch))),
        "HUNT_EVIDENCE_NOT_COLLECTED"
    );
}

// p2pGuardTraffic.test.ts: "local development fallback checks host and session independently"
#[test]
fn the_local_windows_bound_host_and_session() {
    let kit = kit(&[]);
    let (host, session) = (account("host"), session());
    for _ in 0..HUNT_GUARD_SESSION_TRAFFIC_LIMIT.max_requests {
        assert_eq!(kit.guard.charge_now(&host, &session, 10), Ok(TrafficDecision::Accepted));
    }
    assert_eq!(
        kit.guard.charge_now(&host, &session, 10),
        Ok(TrafficDecision::RateLimited)
    );
    let other: rifts_seam::SessionId = "01ARZ3NDEKTSV4RRFFQ69G5FAX@local".parse().unwrap();
    assert_eq!(
        kit.guard.charge_now(&host, &other, 10),
        Ok(TrafficDecision::Accepted),
        "the host window has room"
    );
    kit.clock.advance(60_000);
    assert_eq!(
        kit.guard.charge_now(&host, &session, 10),
        Ok(TrafficDecision::Accepted),
        "a new window"
    );
}

// p2pGuardTraffic.test.ts: "distributed traffic rejection preserves the bounded rate-limit error" and "distributed
// storage failure maps to bounded unavailability without local fallback"
#[test]
fn the_distributed_windows_refuse_and_fail_closed() {
    let kit = kit(&[("P2P_DISTRIBUTED_TRAFFIC_LIMITER", "true")]);
    let (host, session) = (account("host"), session());
    for _ in 0..HUNT_GUARD_SESSION_TRAFFIC_LIMIT.max_requests {
        assert_eq!(kit.guard.charge_now(&host, &session, 1), Ok(TrafficDecision::Accepted));
    }
    assert_eq!(
        kit.guard.charge_now(&host, &session, 1),
        Ok(TrafficDecision::RateLimited)
    );
    assert!(
        kit.store
            .windows()
            .iter()
            .all(|window| !window.window_id.contains("guard-host"))
    );

    let stubbed = Guard::new(GuardParts {
        config: GuardConfig::from_settings(&std::collections::BTreeMap::from([(
            "P2P_DISTRIBUTED_TRAFFIC_LIMITER".to_owned(),
            "true".to_owned(),
        )])),
        store: Arc::new(StubGuardStore),
        hunts: kit.hunts.clone(),
        operations: kit.operations.clone(),
        core: Arc::new(rifts_seam::NoCore),
        clock: kit.clock.clone(),
        log: kit.log.clone(),
    });
    assert_eq!(stubbed.charge_now(&host, &session, 1), Ok(TrafficDecision::Unavailable));
    assert_eq!(status_of("GUARD_TRAFFIC_UNAVAILABLE"), 503);
}

#[test]
fn traffic_refusals_reach_the_route() {
    let kit = kit(&[("MYSTICPARADOX_HUNT_EVIDENCE", "on")]);
    kit.hunts.put(hunt());
    for _ in 0..HUNT_GUARD_SESSION_TRAFFIC_LIMIT.max_requests {
        kit.guard.charge_now(&account("host"), &session(), 1).unwrap();
    }
    let batch = batch_body("b-1", 1, GENESIS_HASH, &["SESSION_STARTED"]);
    assert_eq!(
        refusal_code(answer(&kit, call(SUBMIT, host(), &batch))),
        "GUARD_TRAFFIC_RATE_LIMITED"
    );
}

// hostCapabilities.test.ts: "only a live, validated host capability gets its own request-budget bucket"
#[test]
fn only_a_live_capability_gets_its_own_bucket() {
    let kit = kit(&[]);
    let live = "A".repeat(43);
    let revoked = "B".repeat(43);
    let now = kit.clock.now();
    let capability = |id: &str, revoked: bool| CapabilityView {
        capability_id: id.to_owned(),
        expires_at: UnixMillis(now.0 + 60_000),
        revoked,
    };
    lock(&kit.hunts.capabilities).insert(sha256_hex(live.as_bytes()), capability("cap-live", false));
    lock(&kit.hunts.capabilities).insert(sha256_hex(revoked.as_bytes()), capability("cap-revoked", true));
    let bucket = |raw: Option<&str>| {
        let request = RequestInfo {
            headers: raw
                .map(|raw| vec![("x-mystic-host-capability".to_owned(), raw.to_owned())])
                .unwrap_or_default(),
            ..RequestInfo::default()
        };
        poll_once(kit.guard.rate_limit_bucket(request)).unwrap().unwrap()
    };
    assert_eq!(bucket(Some(&live)).as_deref(), Some("host:cap-live"));
    assert_eq!(bucket(Some(&live)).as_deref(), Some("host:cap-live"));
    assert_eq!(
        *lock(&kit.hunts.capability_lookups),
        1,
        "the second request is served from the short cache"
    );
    assert_eq!(bucket(Some(&revoked)), None);
    assert_eq!(bucket(Some(&"C".repeat(43))), None);
    assert_eq!(bucket(Some("not-a-capability")), None);
    assert_eq!(bucket(None), None);
    *lock(&kit.hunts.failing) = true;
    assert_eq!(bucket(Some(&"D".repeat(43))), None);
    assert!(
        kit.log.lines().iter().all(|line| !line.contains(&live)),
        "a capability is never logged"
    );
}

fn write(kit: &Kit, kind: HostWriteKind, target: &str, options: HostWriteOptions) -> Option<Reply> {
    poll_once(
        kit.guard
            .authorize_host_write(runtime(), kind, account(target), options),
    )
    .unwrap()
    .unwrap()
}

fn present_friend(kit: &Kit, kind: HubKind) {
    kit.hunts.put(hub(kind));
    *lock(&kit.hunts.present) = vec![account("friend")];
}

// hostPolicyObserve.gamedata.test.ts: "observe: a Ramsgate hub's XP write for a present member is authorized and
// logged; an absent player is still refused"
#[test]
fn observe_lets_a_write_kind_through_and_logs_it() {
    let kit = kit(&[]);
    present_friend(&kit, HubKind::Ramsgate);
    assert_eq!(
        write(&kit, HostWriteKind::Xp, "friend", HostWriteOptions::default()),
        None
    );
    let lines = kit.log.lines();
    assert!(lines.iter().any(
        |line| line.starts_with("warn [HostPolicyObserve] would refuse write kind=XP")
            && line.contains("reason=WRITE_KIND_NOT_ALLOWED_IN_HUB")
            && line.contains("hub=RAMSGATE")
    ));
    assert!(!lines.iter().any(|line| line.contains("[HostWrite] refused")));
    let refusal = write(&kit, HostWriteKind::Xp, "stranger", HostWriteOptions::default()).unwrap();
    assert_eq!(refusal.status, 403);
    assert!(refusal.body.0.contains("P2P_HOST_WRITE_DENIED"));
    assert!(
        kit.log
            .lines()
            .iter()
            .any(|line| line.starts_with("warn [HostWrite] refused kind=XP")
                && line.contains("reason=MEMBER_NOT_PRESENT"))
    );
    assert!(kit.store.host_writes().is_empty(), "authorizing writes no tag");
}

// "observe: a probe stays strict and quiet"
#[test]
fn a_probe_stays_strict_and_quiet() {
    let kit = kit(&[]);
    present_friend(&kit, HubKind::Ramsgate);
    let probe = HostWriteOptions {
        probe: true,
        parity: false,
    };
    assert_eq!(write(&kit, HostWriteKind::Xp, "friend", probe).unwrap().status, 403);
    assert!(kit.log.lines().is_empty());
    assert_eq!(write(&kit, HostWriteKind::HuntPassXp, "friend", probe), None);
}

// "observe: a Training hub's reforge write is authorized and logged"
#[test]
fn a_training_hub_reforge_is_let_through_in_observe() {
    let kit = kit(&[]);
    present_friend(&kit, HubKind::Training);
    assert_eq!(
        write(&kit, HostWriteKind::Reforge, "friend", HostWriteOptions::default()),
        None
    );
    assert!(
        kit.log
            .lines()
            .iter()
            .any(|line| line.contains("would refuse write kind=REFORGE") && line.contains("hub=TRAINING"))
    );
}

// "parity: a realm hub's dedicated-server work for a present member is allowed quietly, whatever the write kind" and
// "parity: enforce keeps the dedicated-only paths for dedicated servers"
#[test]
fn parity_is_quiet_in_observe_and_refused_in_enforce() {
    let parity = HostWriteOptions {
        probe: false,
        parity: true,
    };
    let kit = kit(&[]);
    present_friend(&kit, HubKind::Training);
    assert_eq!(write(&kit, HostWriteKind::Reforge, "friend", parity), None);
    assert_eq!(
        write(&kit, HostWriteKind::Reforge, "stranger", parity).unwrap().status,
        403
    );
    assert!(kit.log.lines().is_empty());

    let enforce = crate::testkit::kit(&[("MYSTICPARADOX_HOST_POLICY", "enforce")]);
    present_friend(&enforce, HubKind::Ramsgate);
    assert_eq!(
        write(&enforce, HostWriteKind::Reforge, "friend", parity)
            .unwrap()
            .status,
        403
    );
}

// "enforce: the same Ramsgate XP write is refused as before"; hostWrites.test.ts hub tests under enforce
#[test]
fn enforce_refuses_write_kinds() {
    let kit = kit(&[("MYSTICPARADOX_HOST_POLICY", "enforce")]);
    present_friend(&kit, HubKind::Ramsgate);
    assert_eq!(
        write(&kit, HostWriteKind::Xp, "friend", HostWriteOptions::default())
            .unwrap()
            .status,
        403
    );
    assert_eq!(
        write(&kit, HostWriteKind::Bounty, "friend", HostWriteOptions::default()),
        None
    );
    assert_eq!(
        write(&kit, HostWriteKind::ItemSave, "friend", HostWriteOptions::default()),
        None
    );
    assert_eq!(
        write(&kit, HostWriteKind::ItemSave, "stranger", HostWriteOptions::default())
            .unwrap()
            .status,
        403
    );
    present_friend(&kit, HubKind::Training);
    assert_eq!(
        write(&kit, HostWriteKind::Bounty, "friend", HostWriteOptions::default()),
        None
    );
    assert_eq!(
        write(&kit, HostWriteKind::Reforge, "friend", HostWriteOptions::default())
            .unwrap()
            .status,
        403
    );
    assert!(kit.guard.host_policy_refuses().unwrap());
}

// hostWrites.test.ts: "P2P_HOST_WRITES=off refuses a Ramsgate hub's writes as well"
#[test]
fn the_kill_switch_refuses_every_host_write() {
    let kit = kit(&[("P2P_HOST_WRITES", "off")]);
    present_friend(&kit, HubKind::Ramsgate);
    let refusal = write(&kit, HostWriteKind::Bounty, "friend", HostWriteOptions::default()).unwrap();
    assert_eq!(
        refusal.body.0,
        r#"{"code":"P2P_HOST_WRITE_DENIED","message":"Host writes are switched off.","payload":null}"#
    );
    assert!(kit.log.lines()[0].ends_with("reason=HOST_WRITES_OFF"));
}

// hostWrites.test.ts: "a Ramsgate hub's host write for a present member is accepted and tagged with the realm and the
// hub", "an official hunt's tag keeps today's row shape: no realm fields"
#[test]
fn accepted_writes_are_tagged_with_their_session() {
    let kit = kit(&[]);
    present_friend(&kit, HubKind::Ramsgate);
    assert_eq!(
        write(&kit, HostWriteKind::Bounty, "friend", HostWriteOptions::default()),
        None
    );
    let tag = HostWrite {
        kind: HostWriteKind::Bounty,
        target: account("friend"),
        summary: Json(r#"{"bountyId":"b-1"}"#.to_owned()),
    };
    poll_once(kit.guard.record_host_write(runtime(), tag.clone()))
        .unwrap()
        .unwrap();
    let rows = kit.store.host_writes();
    assert_eq!(rows.len(), 1);
    assert_eq!(
        (
            rows[0].session_kind.as_deref(),
            rows[0].hub_kind.as_deref(),
            rows[0].realm.is_some()
        ),
        (Some("REALM_HUB"), Some("RAMSGATE"), true)
    );
    assert_eq!(rows[0].host, account("host"));
    assert!(kit.log.lines().iter().any(
        |line| line.starts_with("info [HostWrite] kind=BOUNTY target=friend host=host")
            && line.contains(" hub=RAMSGATE summary={\"bountyId\":\"b-1\"}")
    ));

    kit.hunts.put(hunt());
    poll_once(kit.guard.record_host_write(runtime(), tag.clone()))
        .unwrap()
        .unwrap();
    let official = &kit.store.host_writes()[1];
    assert_eq!(
        (
            official.session_kind.as_ref(),
            official.hub_kind.as_ref(),
            official.realm.as_ref()
        ),
        (None, None, None)
    );

    // A write for a session that is gone is not tagged (Node's record without authorize)
    kit.hunts.put(HuntView {
        generation: 9,
        ..hunt()
    });
    poll_once(kit.guard.record_host_write(runtime(), tag)).unwrap().unwrap();
    assert_eq!(kit.store.host_writes().len(), 2);
    assert!(
        kit.log
            .lines()
            .iter()
            .any(|line| line.starts_with("warn [HostWrite] record without authorize kind=BOUNTY"))
    );
}

// p2pInventoryAuthorization.test.ts: "a member who left the hub is refused at the start and inside the transaction"
#[test]
fn inventory_is_fenced_again_inside_the_transaction() {
    let kit = kit(&[]);
    present_friend(&kit, HubKind::Ramsgate);
    let transaction = InventoryTransaction {
        user: account("friend"),
        character_id: "char-1".to_owned(),
        transaction_id: "tx-1".to_owned(),
        body: None,
    };
    let decision = poll_once(kit.guard.authorize_inventory(runtime(), transaction.clone()))
        .unwrap()
        .unwrap();
    let InventoryDecision::Authorized(fence) = decision else {
        panic!("a present member's loot is authorized");
    };
    fence.report(TransactionPhase::Applying);
    assert_eq!(fence.on_grant(&Json::empty_object()), Ok(()));
    lock(&kit.hunts.present).clear();
    let refusal = fence.on_grant(&Json::empty_object()).unwrap_err();
    assert_eq!(
        refusal.body.0,
        concat!(
            r#"{"code":"P2P_LOOT_GRANT_PARTICIPANT_NOT_ADMITTED","#,
            r#""message":"Player-hosted hunt loot grant is not authorized.","payload":null}"#
        )
    );
    fence.report(TransactionPhase::Failed);
    let lines = kit.log.lines();
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("info [P2PInventory] applying account=friend character=char-1"))
    );
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("error [P2PInventory] transaction failed"))
    );
    let InventoryDecision::Refused(reply) = poll_once(kit.guard.authorize_inventory(runtime(), transaction))
        .unwrap()
        .unwrap()
    else {
        panic!("a member who left is refused at the start");
    };
    assert!(reply.body.0.contains("P2P_LOOT_GRANT_DENIED"));
}

const DRAFT: &str = "TOKEN_DAILY_CHALLENGE_DRAFT_PREMIUM";

fn grant(kit: &Kit, user: &str, character: &str, body: Value) -> InventoryDecision {
    let transaction = InventoryTransaction {
        user: account(user),
        character_id: character.to_owned(),
        transaction_id: "tx-grant".to_owned(),
        body: Some(Json(body.to_string())),
    };
    poll_once(kit.guard.authorize_inventory(runtime(), transaction))
        .unwrap()
        .unwrap()
}

fn refused_with(decision: InventoryDecision) -> Reply {
    match decision {
        InventoryDecision::Refused(reply) => reply,
        InventoryDecision::Authorized(_) => panic!("expected a refusal"),
    }
}

fn fence_of(decision: InventoryDecision) -> Box<dyn InventoryFence> {
    match decision {
        InventoryDecision::Authorized(fence) => fence,
        InventoryDecision::Refused(reply) => panic!("expected an authorization, got {}", reply.body.0),
    }
}

fn holding(kit: &Kit, tokens: u32) {
    *lock(&kit.core.stacked_items) = Ok(Some(format!(r#"[{{"catalogId":"{DRAFT}","quantity":{tokens}}}]"#)));
}

fn lines_starting(kit: &Kit, prefix: &str) -> Vec<String> {
    kit.log
        .lines()
        .into_iter()
        .filter(|line| line.starts_with(prefix))
        .collect()
}

/// `onGrant`'s reads inside the transaction.
struct Reads(Option<String>);

impl TransactionReads for Reads {
    fn stacked_items(&self, character_id: &str) -> Result<Option<Json>, SeamError> {
        assert_eq!(character_id, "char-1");
        Ok(self.0.clone().map(Json))
    }
}

fn reads_holding(tokens: u32) -> Reads {
    Reads(Some(format!(r#"[{{"catalogId":"{DRAFT}","quantity":{tokens}}}]"#)))
}

fn draft_token() -> Value {
    json!({"addStackedItems": [{"catalogId": DRAFT, "quantity": 1}]})
}

// hostPolicyObserve.gamedata.test.ts: "observe: a Ramsgate hub's Platinum, Hunt Pass token and cosmetic grant is
// authorized and logged with its classes"; p2pInventoryAuthorization.test.ts: "Platinum is refused in a Ramsgate hub
// by the grant rule, as in a hunt"
#[test]
fn a_host_grant_class_is_refused_in_enforce_and_only_logged_in_observe() {
    let body = json!({
        "addStackedItems": [{"catalogId": "CURRENCY_PLATINUM", "quantity": 100}, {"catalogId": "TOKEN_HUNTPASS_X", "quantity": 1},
            {"catalogId": "CURRENCY_NOTES", "quantity": 5}],
        "addInstancedItems": [{"catalogId": "DYE_BABYBLUE_DURABLE", "instanceId": "d"}]
    });
    let observe = kit(&[]);
    present_friend(&observe, HubKind::Ramsgate);
    let fence = fence_of(grant(&observe, "friend", "char-1", body.clone()));
    assert_eq!(fence.on_grant(&Json(body.to_string())), Ok(()));
    let observed = lines_starting(&observe, "warn [HostPolicyObserve] would refuse grant");
    assert_eq!(observed.len(), 1, "{:?}", observe.log.lines());
    assert_eq!(
        observed[0],
        format!(
            "warn [HostPolicyObserve] would refuse grant account=friend session={} generation=2 transaction=tx-grant \
             classes=PREMIUM_CURRENCY,HUNT_PASS,COSMETIC denied=CURRENCY_PLATINUM(PREMIUM_CURRENCY,addStackedItems),\
             TOKEN_HUNTPASS_X(HUNT_PASS,addStackedItems),DYE_BABYBLUE_DURABLE(COSMETIC,addInstancedItems) hub=RAMSGATE",
            session()
        )
    );
    assert!(lines_starting(&observe, "warn [HostGrantDeny]").is_empty());

    let enforce = kit(&[("MYSTICPARADOX_HOST_POLICY", "enforce")]);
    present_friend(&enforce, HubKind::Ramsgate);
    let reply = refused_with(grant(&enforce, "friend", "char-1", body));
    assert_eq!(reply.status, 403);
    assert_eq!(
        reply.body.0,
        concat!(
            r#"{"code":"P2P_HOST_GRANT_DENIED","message":"A player host may not grant this item class.","#,
            r#""payload":{"classes":["PREMIUM_CURRENCY","HUNT_PASS","COSMETIC"]}}"#
        )
    );
    let denied = lines_starting(&enforce, "warn [HostGrantDeny] account=friend");
    assert_eq!(denied.len(), 1);
    assert!(denied[0].ends_with(" hub=RAMSGATE"));
    assert!(lines_starting(&enforce, "warn [HostPolicyObserve]").is_empty());

    // Loot alone passes in both, quietly
    let loot = json!({"addStackedItems": [{"catalogId": "CURRENCY_NOTES", "quantity": 5}]});
    fence_of(grant(&enforce, "friend", "char-1", loot));
    assert_eq!(lines_starting(&enforce, "warn ").len(), 1);
}

// hostPolicyObserve.gamedata.test.ts: "observe: a hosted Training hunt's Platinum spend (a no-reward session) is
// authorized and logged"
#[test]
fn a_no_reward_session_is_refused_in_enforce_and_only_logged_in_observe() {
    let spend = json!({"removeStackedItems": [{"catalogId": "CURRENCY_PLATINUM", "quantity": 40}]});
    let training = HuntView {
        completion_mode: Some(CompletionMode::NoCompletion),
        ..hunt()
    };
    let observe = kit(&[]);
    observe.hunts.put(training.clone());
    fence_of(grant(&observe, "friend", "char-friend", spend.clone()));
    let observed = lines_starting(&observe, "warn [HostPolicyObserve] would refuse grant");
    assert_eq!(observed.len(), 1);
    assert!(observed[0].contains("classes=NO_REWARD_SESSION denied=any grant or spend"));

    let enforce = kit(&[("MYSTICPARADOX_HOST_POLICY", "enforce")]);
    enforce.hunts.put(training);
    let reply = refused_with(grant(&enforce, "friend", "char-friend", spend.clone()));
    assert!(
        reply
            .body
            .0
            .ends_with(r#""payload":{"classes":["NO_REWARD_SESSION"]}}"#)
    );
    // The same spend in a hunt that completes is a Platinum spend, not a grant
    enforce.hunts.put(hunt());
    fence_of(grant(&enforce, "friend", "char-friend", spend));
    // A realm hub carries NO_COMPLETION too, but grants as a hunt does
    present_friend(&enforce, HubKind::Training);
    fence_of(grant(
        &enforce,
        "friend",
        "char-1",
        json!({"addStackedItems": [{"catalogId": "CURRENCY_NOTES", "quantity": 100}]}),
    ));
}

// hostPolicyObserve.gamedata.test.ts: "observe: a draft premium token over the cap is granted, and the cap is logged
// before and inside the transaction"; hostDraftPremium.gamedata.test.ts: "a member who already holds an unspent token
// is refused before the transaction"
#[test]
fn a_draft_token_over_the_cap_is_refused_in_enforce_and_only_logged_in_observe() {
    let observe = kit(&[]);
    present_friend(&observe, HubKind::Ramsgate);
    holding(&observe, 1);
    let fence = fence_of(grant(&observe, "friend", "char-1", draft_token()));
    let early = lines_starting(&observe, "warn [HostPolicyObserve] would refuse grant");
    assert_eq!(early.len(), 1);
    assert!(
        early[0].contains("classes=HUNT_PASS denied=TOKEN_DAILY_CHALLENGE_DRAFT_PREMIUM(HUNT_PASS,addStackedItems)")
    );
    assert!(
        early[0].ends_with(" draftPremium=OVER_BALANCE_CAP hub=RAMSGATE"),
        "{}",
        early[0]
    );
    assert_eq!(
        fence.on_grant_in(&Json(draft_token().to_string()), &reads_holding(1)),
        Ok(())
    );
    let inside = lines_starting(&observe, "warn [HostPolicyObserve] would refuse draft premium");
    assert_eq!(inside.len(), 1);
    assert!(
        inside[0].ends_with(&format!(
            "transaction=tx-grant realm={} hub=RAMSGATE item={DRAFT} reason=OVER_BALANCE_CAP at=transaction \
             balance=1->2 cap=1",
            hub(HubKind::Ramsgate).realm.unwrap()
        )),
        "{}",
        inside[0]
    );
    fence.report(TransactionPhase::Accepted);
    assert!(lines_starting(&observe, "info [HostGrantAllow]").is_empty());

    let enforce = kit(&[("MYSTICPARADOX_HOST_POLICY", "enforce")]);
    present_friend(&enforce, HubKind::Ramsgate);
    holding(&enforce, 1);
    let reply = refused_with(grant(&enforce, "friend", "char-1", draft_token()));
    assert!(reply.body.0.contains(r#""code":"P2P_HOST_GRANT_DENIED""#));
    let denied = lines_starting(&enforce, "warn [HostGrantDeny]");
    assert_eq!(denied.len(), 1);
    assert!(denied[0].contains(" draftPremium=OVER_BALANCE_CAP"));
}

// hostDraftPremium.gamedata.test.ts: "a realm hub grants the daily premium token to a member who ... has none
// unspent; the accepted grant is logged" and "the balance is read again inside the transaction: a token that arrived
// after the early check is refused there, with 403"
#[test]
fn the_draft_token_is_decided_again_from_the_transactions_reads() {
    let enforce = kit(&[("MYSTICPARADOX_HOST_POLICY", "enforce")]);
    present_friend(&enforce, HubKind::Ramsgate);
    let fence = fence_of(grant(&enforce, "friend", "char-1", draft_token()));
    let token = Json(draft_token().to_string());
    let refusal = fence.on_grant_in(&token, &reads_holding(1)).unwrap_err();
    assert_eq!(refusal.status, 403);
    assert_eq!(
        refusal.body.0,
        concat!(
            r#"{"code":"P2P_LOOT_GRANT_DRAFT_PREMIUM_OVER_BALANCE_CAP","#,
            r#""message":"Player-hosted hunt loot grant is not authorized.","payload":null}"#
        )
    );
    assert_eq!(
        lines_starting(&enforce, "warn [HostGrantDeny]"),
        [format!(
            "warn [HostGrantDeny] account=friend session={} generation=2 transaction=tx-grant item={DRAFT} quantity=1 \
             reason=OVER_BALANCE_CAP at=transaction",
            session()
        )]
    );
    assert_eq!(
        lines_starting(&enforce, "warn [P2PInventory] transaction denied").len(),
        1
    );

    // Within the cap: granted, and the grant is logged once the transaction is accepted
    assert_eq!(fence.on_grant_in(&token, &Reads(None)), Ok(()));
    fence.report(TransactionPhase::Accepted);
    let day: String = iso_time(enforce.clock.now()).chars().take(10).collect();
    assert_eq!(
        lines_starting(&enforce, "info [HostGrantAllow]"),
        [format!(
            "info [HostGrantAllow] account=friend session={} generation=2 transaction=tx-grant item={DRAFT} quantity=1 \
             reason=WITHIN_BALANCE_CAP day={day} balance=0->1 cap=1",
            session()
        )]
    );

    // Without the transaction's reads the early decision stands
    let fence = fence_of(grant(&enforce, "friend", "char-1", draft_token()));
    assert_eq!(fence.on_grant(&token), Ok(()));
}

// hostDraftPremium.test.ts: "an unreadable balance is refused, never taken as zero"
#[test]
fn an_unreadable_balance_is_refused_in_enforce_and_only_logged_in_observe() {
    for (stored, settings) in [
        (Err(()), &[][..]),
        (Ok(Some("not json".to_owned())), &[][..]),
        (Err(()), &[("MYSTICPARADOX_HOST_POLICY", "enforce")][..]),
    ] {
        let kit = kit(settings);
        present_friend(&kit, HubKind::Ramsgate);
        *lock(&kit.core.stacked_items) = stored;
        let decision = grant(&kit, "friend", "char-1", draft_token());
        let line = if settings.is_empty() {
            fence_of(decision);
            "warn [HostPolicyObserve] would refuse grant"
        } else {
            refused_with(decision);
            "warn [HostGrantDeny]"
        };
        assert!(lines_starting(&kit, line)[0].contains(" draftPremium=BALANCE_UNREADABLE"));
    }
}

// hostDraftPremiumDaily.gamedata.test.ts: "an accepted transaction replayed while the token is still held succeeds,
// grants nothing more, and logs no second grant"
#[test]
fn an_accepted_replay_is_not_decided_again() {
    let kit = kit(&[("MYSTICPARADOX_HOST_POLICY", "enforce")]);
    present_friend(&kit, HubKind::Ramsgate);
    holding(&kit, 1);
    *lock(&kit.core.replay) = true;
    let fence = fence_of(grant(&kit, "friend", "char-1", draft_token()));
    assert_eq!(
        fence.on_grant_in(&Json(draft_token().to_string()), &reads_holding(1)),
        Ok(())
    );
    fence.report(TransactionPhase::Accepted);
    assert!(lines_starting(&kit, "info [HostGrantAllow]").is_empty());
    assert!(lines_starting(&kit, "warn ").is_empty());
    // The replay only exempts its own token: a second token in the body is no replay of one grant
    let two = json!({"addStackedItems": [{"catalogId": DRAFT, "quantity": 1}, {"catalogId": DRAFT, "quantity": 1}]});
    refused_with(grant(&kit, "friend", "char-1", two));
}

// hostDraftPremium.gamedata.test.ts: "the token next to another Hunt Pass item, or as an instanced add, is refused as
// a whole"
#[test]
fn the_draft_token_next_to_another_hunt_pass_item_is_refused_as_a_whole() {
    let kit = kit(&[("MYSTICPARADOX_HOST_POLICY", "enforce")]);
    present_friend(&kit, HubKind::Ramsgate);
    let body = json!({"addStackedItems": [{"catalogId": DRAFT, "quantity": 1}, {"catalogId": "CURRENCY_REWARDCACHE", "quantity": 1}]});
    let reply = refused_with(grant(&kit, "friend", "char-1", body));
    assert!(reply.body.0.ends_with(r#""payload":{"classes":["HUNT_PASS"]}}"#));
    assert!(
        lines_starting(&kit, "warn [HostGrantDeny]")[0]
            .contains("denied=CURRENCY_REWARDCACHE(HUNT_PASS,addStackedItems)")
    );
    let instanced = json!({"addInstancedItems": [{"catalogId": DRAFT, "instanceId": "i"}]});
    refused_with(grant(&kit, "friend", "char-1", instanced));
}

fn admin(handler: &str, body: Value, alert: &str) -> RouteCall {
    let mut call = call(
        handler,
        Caller::Admin {
            key_id: "admin-1".to_owned(),
        },
        &body,
    );
    call.params = vec![("alertId".to_owned(), alert.to_owned())];
    call
}

fn rollout(version: u64, tier1: bool, expected: u64) -> Value {
    json!({"rewardRollout": {"version": version, "tier0ShadowEnabled": true, "tier1GrantsEnabled": tier1},
        "expectedVersion": expected, "reason": "operator change for the test"})
}

#[test]
fn the_reward_rollout_route() {
    let kit = kit(&[]);
    let status = |result: Result<Reply, SeamError>| {
        let reply = result.unwrap();
        (reply.status, serde_json::from_str::<Value>(&reply.body.0).unwrap())
    };
    let (code, body) = status(answer(&kit, admin(ROLLOUT, json!({"reason": "x"}), "")));
    assert_eq!((code, body["error"]["code"].as_str()), (400, Some("ADMIN_VALIDATION")));
    let (code, body) = status(answer(&kit, admin(ROLLOUT, rollout(1, false, 5), "")));
    assert_eq!(
        (code, body["error"]["code"].as_str()),
        (409, Some("ADMIN_POLICY_CONFLICT"))
    );
    assert_eq!(body["policy"]["version"], json!(0));
    let (code, body) = status(answer(&kit, admin(ROLLOUT, rollout(0, false, 0), "")));
    assert_eq!(
        (code, body["error"]["code"].as_str()),
        (409, Some("ADMIN_REWARD_ROLLOUT_CONFLICT"))
    );
    let (code, body) = status(answer(&kit, admin(ROLLOUT, rollout(1, true, 0), "")));
    assert_eq!(
        (code, body["error"]["code"].as_str()),
        (403, Some("ADMIN_POLICY_STEP_UP_REQUIRED"))
    );
    assert_eq!(body["weakeningCodes"], json!(["TIER1_REWARD_GRANTS_ENABLE"]));
    assert_eq!(kit.operations.audit()[0].action, "operations.policy.weakening_rejected");
    let mut stepped = rollout(1, true, 0);
    stepped["breakGlassTotp"] = json!("123456");
    let (code, body) = status(answer(&kit, admin(ROLLOUT, stepped, "")));
    assert_eq!(code, 200);
    assert_eq!(body["policy"]["rewardRollout"]["tier1GrantsEnabled"], json!(true));
    assert_eq!(body["policy"]["version"], json!(1));
    let audit = kit.operations.audit();
    assert_eq!(audit[1].action, "operations.reward_rollout.update");
    assert_eq!(audit[1].ip, "203.0.113.9");
    assert!(
        audit[1]
            .new_state
            .0
            .ends_with(r#","weakeningCodes":["TIER1_REWARD_GRANTS_ENABLE"]}"#)
    );
    assert_eq!(body["requestId"].as_str(), Some(audit[1].request_id.as_str()));
}

#[test]
fn the_alert_acknowledgement_route() {
    let kit = kit(&[]);
    kit.store
        .insert_alert(
            &Actor::System("test"),
            &AlertRecord {
                alert_id: "alert-0001".to_owned(),
                code: "REWARD_INVARIANT".to_owned(),
                severity: "critical".to_owned(),
                message: "m".to_owned(),
                details: Json::empty_object(),
                created_at: UnixMillis(1),
                acknowledged_at: None,
                acknowledged_by: None,
            },
        )
        .unwrap();
    let reason = json!({"reason": "looked into it"});
    let bad = answer(&kit, admin(ACK, reason.clone(), "bad")).unwrap();
    assert_eq!(bad.status, 400);
    let done = answer(&kit, admin(ACK, reason.clone(), "alert-0001")).unwrap();
    assert_eq!(done.status, 200);
    let body: Value = serde_json::from_str(&done.body.0).unwrap();
    assert_eq!(body["status"], json!("ACKNOWLEDGED"));
    assert_eq!(body["acknowledgedAt"], json!(iso_time(kit.clock.now())));
    assert_eq!(
        kit.store.alert("alert-0001").unwrap().acknowledged_by.as_deref(),
        Some("admin-1")
    );
    assert_eq!(kit.operations.audit()[0].action, "operations.alert.acknowledge");
    let again = answer(&kit, admin(ACK, reason, "alert-0001")).unwrap();
    assert_eq!(again.status, 404);
    assert!(again.body.0.contains("ADMIN_ALERT_NOT_OPEN"));
    let anonymous = RouteCall {
        caller: Caller::Anonymous,
        ..admin(ACK, json!({}), "alert-0001")
    };
    assert_eq!(refusal_code(answer(&kit, anonymous)), "AUTH_UNAUTHORIZED");
}

#[test]
fn the_census_logs_one_line_per_window() {
    let kit = kit(&[]);
    assert_eq!(
        kit.guard.census(&HostRouteSample {
            method: "POST".to_owned(),
            route: "/inventory".to_owned(),
            status_code: 200,
            session_kind: None,
            hub_kind: None,
        }),
        Ok(())
    );
    kit.guard.flush_census();
    kit.guard.flush_census();
    let lines = kit.log.lines();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].starts_with(r#"info [HostRoute] summary {"windowSeconds":60,"total":1,"#));
}

#[test]
fn lifecycle() {
    let kit = kit(&[("MYSTICPARADOX_HOST_POLICY", "enforce")]);
    assert_eq!(kit.guard.tables(), TABLES);
    assert_eq!(kit.guard.guard_manifest_roles(), GUARD_MANIFEST_ROLES);
    let steps: Vec<(&str, &str)> = kit
        .guard
        .account_deletion_steps()
        .iter()
        .map(|step| (step.table, step.account_column))
        .collect();
    assert_eq!(
        steps,
        [
            ("rifts_host_writes", "target"),
            ("rifts_host_writes", "host"),
            ("rifts_hunt_xp_summaries", "member")
        ]
    );
    assert_eq!(
        kit.guard.startup_summary("backend").unwrap().unwrap().0,
        r#"{"antiCheatLoaded":false,"hostPolicy":"enforce","huntEvidence":false,"trustTier":0}"#
    );
    kit.guard.start("backend").unwrap();
    let lines = kit.log.lines();
    assert!(lines[0].starts_with("info [Startup] MYSTICPARADOX_HOST_POLICY=enforce"));
    assert!(lines[1].starts_with("info [Startup] MYSTICPARADOX_HUNT_EVIDENCE not set"));
    // Every host policy refusal enforces, so nothing warns that one does not
    assert_eq!(lines.len(), 2, "{lines:?}");
    poll_once(kit.guard.stop()).unwrap().unwrap();
}

#[test]
fn hooks_on_a_stubbed_store_say_so() {
    let kit = kit(&[("MYSTICPARADOX_HUNT_EVIDENCE", "on")]);
    kit.hunts.put(hunt());
    let stubbed = Guard::new(GuardParts {
        config: kit.guard.config().clone(),
        store: Arc::new(StubGuardStore),
        hunts: kit.hunts.clone(),
        operations: kit.operations.clone(),
        core: Arc::new(rifts_seam::NoCore),
        clock: kit.clock.clone(),
        log: kit.log.clone(),
    });
    let batch = batch_body("b-1", 1, GENESIS_HASH, &["SESSION_STARTED"]);
    let error = stubbed.submit_parsed(&runtime(), Some(&batch)).unwrap_err();
    assert_eq!(
        error,
        GuardError::from(StoreError::Unimplemented(Unimplemented::new(
            Component::Guard,
            "store.chain"
        )))
    );
    assert_eq!(Method::Post.as_str(), "POST");
}

// RF2's stub checks, kept
#[test]
fn the_stub_answers_unimplemented() {
    fn refuse<T>(operation: &'static str) -> Result<T, SeamError> {
        Err(SeamError::unimplemented(Component::Guard, operation))
    }
    fn guard<T>(operation: &'static str) -> Result<T, GuardError> {
        Err(GuardError::from(Unimplemented::new(Component::Guard, operation)))
    }
    assert_eq!(Stub.host_policy_refuses(), refuse("host_policy_refuses"));
    assert_eq!(Stub.census(&HostRouteSample::default()), refuse("census"));
    assert_eq!(
        poll_once(Stub.rate_limit_bucket(RequestInfo::default())).unwrap(),
        refuse("rate_limit_bucket")
    );
    assert_eq!(Stub.guard_manifest_roles(), GUARD_MANIFEST_ROLES);
    assert_eq!(poll_once(Stub.verdict(session(), 1)).unwrap(), guard("verdict"));
    assert_eq!(
        poll_once(Stub.charge(account("host"), session(), 10)).unwrap(),
        guard("charge")
    );
    assert_eq!(
        poll_once(Stub.acknowledge_alert("key".to_owned(), "alert".to_owned(), None, String::new())).unwrap(),
        guard("acknowledge_alert")
    );
}
