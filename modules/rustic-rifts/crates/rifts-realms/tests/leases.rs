//! `p2p/hostLease.test.ts` and `p2p/p2pLifecycleMonitor.test.ts` in miniature, on both engines: a lease renewed through
//! the store, a stale sequence fenced, a missed lease marked suspect, recovered by a heartbeat or handed to recovery,
//! a lapsed hub aborted, and an operator stop. The rule tables themselves are in `lifecycle_rules.rs`.
#![allow(clippy::unwrap_used)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use common::{actor, hunt_session, participant, targets};
use rifts_realms::RealmsError;
use rifts_realms::capabilities::HostCapabilities;
use rifts_realms::hunt::{AdmissionState, HuntState};
use rifts_realms::leases::{Hosting, HostingParts, HostingSettings, SweepStep};
use rifts_realms::lifecycle::RecordedRecovery;
use rifts_realms::ports::{DefaultOperations, ManualClock, NoGuardSessions, QuietLog};
use rifts_realms::repo::hunts::HuntRepository;
use rifts_seam::{CapabilityScope, UnixMillis};
use serde_json::json;

const SESSION: &str = "0d6f2b9c-5b1e-4c3a-9e57-2f1a8b7c4d10";
const NOW: UnixMillis = UnixMillis(1_791_892_800_000);

struct Rig {
    hosting: Hosting,
    hunts: HuntRepository,
    caps: HostCapabilities,
    clock: Arc<ManualClock>,
    recovery: Arc<RecordedRecovery>,
    operations: Arc<DefaultOperations>,
}

fn rig(db: rifts_realms::db::RealmsDb) -> Rig {
    let clock = Arc::new(ManualClock::at(NOW));
    let recovery = Arc::new(RecordedRecovery::default());
    let operations = Arc::new(DefaultOperations::default());
    let settings = HostingSettings::from_settings(&BTreeMap::<String, String>::new());
    Rig {
        hosting: Hosting::new(HostingParts {
            db: db.clone(),
            settings,
            clock: clock.clone(),
            operations: operations.clone(),
            guard: Arc::new(NoGuardSessions),
            recovery: recovery.clone(),
            expected_artifacts: Arc::new(|| Err("no feed".to_owned())),
            log: Arc::new(QuietLog),
        }),
        hunts: HuntRepository::new(db.clone()),
        caps: HostCapabilities::new(db),
        clock,
        recovery,
        operations,
    }
}

fn beat(sequence: i64) -> serde_json::Value {
    json!({ "leaseSequence": sequence, "bridgeAlive": true, "serverReady": true, "admittedPeerCount": 1, "queuePressure": 0 })
}

fn code<T: std::fmt::Debug>(result: Result<T, RealmsError>) -> String {
    match result {
        Err(RealmsError::Refused(r)) => r.code,
        other => panic!("{other:?}"),
    }
}

/// A RUNNING hunt with a capability that may heartbeat.
async fn running(rig: &Rig) -> rifts_realms::hunt::HostCapabilityRecord {
    let mut hunt = hunt_session(SESSION, HuntState::Running);
    hunt.lease_expires_at = Some(UnixMillis(NOW.0 + 10_000));
    // Someone is still in the hunt, so losing the host is a failure and not a clean end
    hunt.roster = vec![
        participant("host", AdmissionState::Admitted, true),
        participant("pal", AdmissionState::Admitted, true),
    ];
    rig.hunts.create_hunt(&actor(), &hunt).await.unwrap();
    let scopes = vec![CapabilityScope::LeaseHeartbeat, CapabilityScope::GameServerRuntime];
    rig.caps
        .mint(&hunt, "bridge", scopes, NOW, UnixMillis(NOW.0 + 3_600_000))
        .await
        .unwrap()
        .1
}

#[tokio::test(flavor = "multi_thread")]
async fn a_heartbeat_extends_the_lease_and_a_repeated_sequence_is_fenced() {
    for target in targets() {
        let rig = rig(target.open());
        let capability = running(&rig).await;
        rig.clock.set(UnixMillis(NOW.0 + 3_000));
        let reply = rig.hosting.renew(&capability, &beat(1)).await.unwrap();
        assert_eq!((reply.renewed, reply.generation, reply.state), (true, 1, "RUNNING"));
        let stored = rig.hunts.find_hunt(SESSION).await.unwrap().unwrap();
        assert_eq!(stored.lease_expires_at, Some(UnixMillis(NOW.0 + 3_000 + 10_000)));
        assert_eq!(stored.last_lease_sequence, Some(1));

        assert_eq!(
            code(rig.hosting.renew(&capability, &beat(1)).await),
            "P2P_GENERATION_FENCED"
        );
        assert_eq!(
            code(rig.hosting.renew(&capability, &beat(0)).await),
            "P2P_TICKET_INVALID"
        );
        assert!(rig.hosting.renew(&capability, &beat(2)).await.is_ok());
        let missing = json!({ "leaseSequence": 3, "bridgeAlive": true });
        assert_eq!(
            code(rig.hosting.renew(&capability, &missing).await),
            "P2P_TICKET_INVALID"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_emergency_stop_refuses_the_heartbeat_and_a_dead_process_ends_the_session_to_recovery() {
    for target in targets() {
        let rig = rig(target.open());
        let capability = running(&rig).await;
        rig.operations.stopped.store(true, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(
            code(rig.hosting.renew(&capability, &beat(1)).await),
            "P2P_EMERGENCY_STOP"
        );
        rig.operations
            .stopped
            .store(false, std::sync::atomic::Ordering::Relaxed);

        let dead = json!({ "leaseSequence": 1, "bridgeAlive": false, "serverReady": true, "admittedPeerCount": 0, "queuePressure": 0 });
        assert_eq!(
            code(rig.hosting.renew(&capability, &dead).await),
            "P2P_GENERATION_FENCED"
        );
        let handed = rig.recovery.0.lock().unwrap().clone();
        assert_eq!(
            handed,
            [(SESSION.to_owned(), 1, "HOST_PROCESS_UNHEALTHY".to_owned(), false)]
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missed_lease_marks_the_session_suspect_a_heartbeat_recovers_it_and_a_second_miss_goes_to_recovery() {
    for target in targets() {
        let rig = rig(target.open());
        let capability = running(&rig).await;
        assert!(rig.hosting.sweep().await.unwrap().is_empty(), "the lease is still good");

        rig.clock.set(UnixMillis(NOW.0 + 10_000));
        let steps = rig.hosting.sweep().await.unwrap();
        assert_eq!(
            steps,
            [(
                SESSION.to_owned(),
                SweepStep::Suspect {
                    reason: "HOST_LEASE_MISSED".to_owned()
                }
            )]
        );
        let suspect = rig.hunts.find_hunt(SESSION).await.unwrap().unwrap();
        assert_eq!(
            (suspect.state, suspect.pre_suspect_state),
            (HuntState::Suspect, Some(HuntState::Running))
        );
        let transitions = rig.hunts.list_transitions(SESSION).await.unwrap();
        assert_eq!(transitions.last().unwrap().to_state, HuntState::Suspect);

        // The host comes back inside the suspect window
        rig.clock.set(UnixMillis(NOW.0 + 12_000));
        rig.hosting.renew(&capability, &beat(1)).await.unwrap();
        let steps = rig.hosting.sweep().await.unwrap();
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].1, SweepStep::Recovered);
        assert_eq!(
            rig.hunts.find_hunt(SESSION).await.unwrap().unwrap().state,
            HuntState::Running
        );

        // And goes away for good: suspect, then past the suspect window it is isles' recovery's
        rig.clock.set(UnixMillis(NOW.0 + 12_000 + 10_000));
        rig.hosting.sweep().await.unwrap();
        rig.clock.set(UnixMillis(NOW.0 + 12_000 + 10_000 + 15_000));
        let steps = rig.hosting.sweep().await.unwrap();
        assert!(
            matches!(
                steps[0].1,
                SweepStep::Recovery {
                    before_active: false,
                    ..
                }
            ),
            "{steps:?}"
        );
        assert_eq!(rig.recovery.0.lock().unwrap().len(), 1);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_operator_stop_hands_a_running_session_to_recovery_on_the_next_sweep() {
    for target in targets() {
        let rig = rig(target.open());
        running(&rig).await;
        rig.operations.stopped.store(true, std::sync::atomic::Ordering::Relaxed);
        let steps = rig.hosting.sweep().await.unwrap();
        assert!(
            matches!(&steps[0].1, SweepStep::Recovery { reason, .. } if reason == "OPERATOR_P2P_STOP"),
            "{steps:?}"
        );
    }
}
