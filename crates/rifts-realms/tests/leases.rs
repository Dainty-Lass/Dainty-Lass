// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Host leases and the lifecycle monitor, on SQLite and (when configured) Postgres.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use common::kit::{T0, code};
use common::{Target, actor, admitted, hunt, participant, targets};
use rifts_realms::RealmsError;
use rifts_realms::capabilities::HostCapabilities;
use rifts_realms::hunt::{HostCapabilityRecord, HuntSession, HuntState};
use rifts_realms::leases::{Hosting, HostingParts, HostingSettings, SweepStep};
use rifts_realms::lifecycle::{RealmsRecovery, Recovery};
use rifts_realms::ports::{Clock, DefaultOperations, ManualClock, NoGuardSessions, RealmsLog};
use rifts_realms::repo::hunts::HuntRepository;
use rifts_seam::{BoxFuture, CapabilityScope, NoCore, Secret, UnixMillis};
use serde_json::{Map, Value, json};

const WIRE: &str = "0190a1b2-c3d4-4e5f-8a6b-7c8d9e0f1a2b";

#[derive(Default)]
struct Prepared {
    calls: Mutex<Vec<(String, String, bool)>>,
}

impl Recovery for Prepared {
    fn prepare(&self, id: String, _g: i64, reason: String, before: bool) -> BoxFuture<'_, Result<(), RealmsError>> {
        self.calls.lock().unwrap().push((id, reason, before));
        Box::pin(std::future::ready(Ok(())))
    }

    fn advance(&self, _session: HuntSession) -> BoxFuture<'_, Result<(), RealmsError>> {
        Box::pin(std::future::ready(Ok(())))
    }
}

#[derive(Default)]
struct Lines(Mutex<Vec<String>>);

impl RealmsLog for Lines {
    fn info(&self, _line: &str) {}

    fn warn(&self, line: &str) {
        self.0.lock().unwrap().push(line.to_owned());
    }
}

fn settings() -> HostingSettings {
    let pairs: BTreeMap<String, String> = [("P2P_HUNTS_ENABLED", "true"), ("RUSTIC_RIFTS_REALMS_HOSTING", "on")]
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    HostingSettings::from_settings(&pairs)
}

fn hosting(target: &Target, clock: Arc<ManualClock>, recovery: Arc<dyn Recovery>, log: Arc<Lines>) -> Hosting {
    Hosting::new(HostingParts {
        db: target.db.clone(),
        settings: settings(),
        clock,
        operations: Arc::new(DefaultOperations::default()),
        guard: Arc::new(NoGuardSessions),
        recovery,
        expected_artifacts: Arc::new(|| Err("no feed".to_owned())),
        log,
    })
}

/// A running hunt whose lease lapses at T0 + 10 s, and its host's capability.
async fn running(target: &Target) -> (HostCapabilityRecord, Secret) {
    let mut session = hunt(WIRE);
    session.state = HuntState::Running;
    session.active_party_key = Some("party-1".into());
    session.lease_expires_at = Some(UnixMillis(T0 + 10_000));
    session.roster = vec![
        admitted(participant("host", true)),
        admitted(participant("member", true)),
    ];
    HuntRepository::new(target.db.clone())
        .create_hunt(&actor(), &session)
        .await
        .unwrap();
    let caps = HostCapabilities::new(target.db.clone());
    let (secret, record) = caps
        .mint(
            &session,
            "bridge",
            vec![CapabilityScope::LeaseHeartbeat],
            UnixMillis(T0),
            UnixMillis(T0 + 3_600_000),
        )
        .await
        .unwrap();
    (record, secret)
}

fn heartbeat(sequence: i64, alive: bool) -> Map<String, Value> {
    match json!({
        "leaseSequence": sequence,
        "bridgeAlive": alive,
        "serverReady": true,
        "admittedPeerCount": 1,
        "queuePressure": 0,
    }) {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

fn steps(steps: Vec<(String, SweepStep)>) -> Vec<SweepStep> {
    steps.into_iter().map(|(_, step)| step).collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_lapsed_lease_goes_suspect_recovers_and_is_handed_to_recovery() {
    for target in targets() {
        let clock = Arc::new(ManualClock::at(UnixMillis(T0)));
        let recovery = Arc::new(Prepared::default());
        let h = hosting(&target, clock.clone(), recovery.clone(), Arc::default());
        let (capability, _) = running(&target).await;
        clock.set(UnixMillis(T0 + 10_000));
        let suspect = SweepStep::Suspect {
            reason: "HOST_LEASE_MISSED".into(),
        };
        assert_eq!(steps(h.sweep().await.unwrap()), std::slice::from_ref(&suspect));
        clock.set(UnixMillis(T0 + 12_000));
        let renewed = h.renew(&capability, &heartbeat(1, true)).await.unwrap();
        assert_eq!(renewed.state, "SUSPECT");
        assert_eq!(renewed.lease_expires_at, "2026-10-13T12:00:22.000Z");
        let stale = h.renew(&capability, &heartbeat(1, true)).await;
        assert_eq!(code(&stale), "P2P_GENERATION_FENCED");
        assert_eq!(steps(h.sweep().await.unwrap()), [SweepStep::Recovered]);
        clock.set(UnixMillis(T0 + 22_000));
        assert_eq!(steps(h.sweep().await.unwrap()), [suspect]);
        clock.set(UnixMillis(T0 + 30_000));
        assert!(h.sweep().await.unwrap().is_empty());
        clock.set(UnixMillis(T0 + 37_000));
        assert_eq!(
            steps(h.sweep().await.unwrap()),
            [SweepStep::Recovery {
                reason: "HOST_LEASE_MISSED".into(),
                before_active: false,
            }]
        );
        assert_eq!(recovery.calls.lock().unwrap().len(), 1);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn realms_recovery_ends_the_hunt_aborted() {
    for target in targets() {
        let clock = Arc::new(ManualClock::at(UnixMillis(T0)));
        let log = Arc::new(Lines::default());
        let clock_dyn: Arc<dyn Clock> = clock.clone();
        let recovery = Arc::new(RealmsRecovery::new(
            target.db.clone(),
            Arc::new(NoCore),
            clock_dyn,
            log.clone(),
        ));
        let h = hosting(&target, clock.clone(), recovery, log.clone());
        let (_, secret) = running(&target).await;
        clock.set(UnixMillis(T0 + 10_000));
        h.sweep().await.unwrap();
        clock.set(UnixMillis(T0 + 25_000));
        h.sweep().await.unwrap();
        let hunts = HuntRepository::new(target.db.clone());
        let ended = hunts.find_hunt(WIRE).await.unwrap().unwrap();
        assert_eq!(ended.state, HuntState::Aborted);
        assert_eq!(ended.failure_reason.as_deref(), Some("HOST_LEASE_MISSED"));
        assert_eq!((ended.lease_expires_at, ended.active_party_key), (None, None));
        let caps = HostCapabilities::new(target.db.clone());
        let check = caps
            .check(
                Some(secret.expose()),
                WIRE,
                CapabilityScope::LeaseHeartbeat,
                UnixMillis(T0 + 25_000),
            )
            .await;
        assert_eq!(code(&check), "P2P_CAPABILITY_INVALID");
        let fences = hunts
            .list_control_messages("member", 0, UnixMillis(T0 + 25_000), 32, false)
            .await
            .unwrap();
        assert!(fences.iter().any(|m| m.message_type == "FENCE_NOW"));
        let warned = log.0.lock().unwrap().clone();
        assert!(
            warned
                .iter()
                .any(|l| l.contains(WIRE) && l.contains("HOST_LEASE_MISSED")),
            "{warned:?}"
        );
        assert!(h.sweep().await.unwrap().is_empty());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn heartbeats_are_checked() {
    for target in targets() {
        let clock = Arc::new(ManualClock::at(UnixMillis(T0)));
        let recovery = Arc::new(Prepared::default());
        let h = hosting(&target, clock.clone(), recovery.clone(), Arc::default());
        let (capability, _) = running(&target).await;
        let mut too_many = heartbeat(1, true);
        too_many.insert("admittedPeerCount".into(), json!(5));
        assert_eq!(code(&h.renew(&capability, &too_many).await), "P2P_TICKET_INVALID");
        let mut zero = heartbeat(0, true);
        zero.remove("queuePressure");
        assert_eq!(code(&h.renew(&capability, &zero).await), "P2P_TICKET_INVALID");
        // A host process that reports itself down is handed to recovery.
        let down = h.renew(&capability, &heartbeat(1, false)).await;
        assert_eq!(code(&down), "P2P_GENERATION_FENCED");
        assert_eq!(
            recovery.calls.lock().unwrap().clone(),
            [(WIRE.to_owned(), "HOST_PROCESS_UNHEALTHY".to_owned(), false)]
        );
        let ok = h.renew(&capability, &heartbeat(2, true)).await.unwrap();
        assert!(ok.renewed);
        assert_eq!(serde_json::to_string(&ok).unwrap().split(',').count(), 5);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn observations_need_the_roster_generation() {
    for target in targets() {
        let clock = Arc::new(ManualClock::at(UnixMillis(T0)));
        let h = hosting(&target, clock, Arc::new(Prepared::default()), Arc::default());
        running(&target).await;
        let body = |generation: i64| match json!({
            "generation": generation,
            "observedAt": "2026-10-13T12:00:00.000Z",
            "hostReachable": false,
            "localBridgeAlive": true,
            "metricsAvailable": false,
            "routeKind": "UNKNOWN",
        }) {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        let reply = h.observe("member", WIRE, &body(1)).await.unwrap();
        assert_eq!(
            serde_json::to_string(&reply).unwrap(),
            format!(r#"{{"accepted":true,"sessionId":"{WIRE}","generation":1,"state":"RUNNING"}}"#)
        );
        assert_eq!(
            code(&h.observe("member", WIRE, &body(2)).await),
            "P2P_GENERATION_FENCED"
        );
        assert_eq!(
            code(&h.observe("stranger", WIRE, &body(1)).await),
            "P2P_GENERATION_FENCED"
        );
        let mut metrics = body(1);
        metrics.insert("metricsAvailable".into(), json!(true));
        assert_eq!(code(&h.observe("member", WIRE, &metrics).await), "P2P_TICKET_INVALID");
    }
}
