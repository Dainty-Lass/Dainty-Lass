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
use rifts_realms::hunt::{AdmissionState, HuntSession, HuntState};
use rifts_realms::leases::{Hosting, HostingParts, HostingSettings, SweepStep};
use rifts_realms::lifecycle::{RealmsRecovery, Recovery};
use rifts_realms::ports::{DefaultOperations, ManualClock, NoGuardSessions, QuietLog, RealmsLog};
use rifts_realms::repo::hunts::HuntRepository;
use rifts_seam::{BoxFuture, CapabilityScope, NoCore, UnixMillis};
use serde_json::json;

#[derive(Debug, Default)]
struct RecordedRecovery(std::sync::Mutex<Vec<(String, i64, String, bool)>>);

impl Recovery for RecordedRecovery {
    fn prepare(
        &self,
        session_id: String,
        generation: i64,
        reason: String,
        before_active: bool,
    ) -> BoxFuture<'_, Result<(), RealmsError>> {
        self.0
            .lock()
            .unwrap()
            .push((session_id, generation, reason, before_active));
        Box::pin(async { Ok(()) })
    }

    fn advance(&self, _: HuntSession) -> BoxFuture<'_, Result<(), RealmsError>> {
        Box::pin(async { Ok(()) })
    }
}

#[derive(Default)]
struct CapturedWarnings(std::sync::Mutex<Vec<String>>);

impl RealmsLog for CapturedWarnings {
    fn info(&self, _: &str) {}
    fn warn(&self, line: &str) {
        self.0.lock().unwrap().push(line.to_owned());
    }
}

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

#[tokio::test(flavor = "multi_thread")]
async fn a_hunt_whose_host_disappears_ends_aborted_and_logs_warn_via_realms_recovery() {
    for target in targets() {
        let db = target.open();
        let clock = Arc::new(ManualClock::at(NOW));
        let warnings = Arc::new(CapturedWarnings::default());
        let recovery = Arc::new(RealmsRecovery::new(
            db.clone(),
            Arc::new(NoCore),
            clock.clone(),
            warnings.clone(),
        ));
        let hosting = Hosting::new(HostingParts {
            db: db.clone(),
            settings: HostingSettings::from_settings(&BTreeMap::<String, String>::new()),
            clock: clock.clone(),
            operations: Arc::new(DefaultOperations::default()),
            guard: Arc::new(NoGuardSessions),
            recovery,
            expected_artifacts: Arc::new(|| Err("no feed".to_owned())),
            log: warnings.clone(),
        });
        let hunts = HuntRepository::new(db.clone());
        let caps = HostCapabilities::new(db);

        let mut hunt = hunt_session(SESSION, HuntState::Running);
        hunt.lease_expires_at = Some(UnixMillis(NOW.0 + 10_000));
        hunt.roster = vec![
            participant("host", AdmissionState::Admitted, true),
            participant("pal", AdmissionState::Admitted, true),
        ];
        hunts.create_hunt(&actor(), &hunt).await.unwrap();
        let (_, capability) = caps
            .mint(
                &hunt,
                "bridge",
                vec![CapabilityScope::LeaseHeartbeat],
                NOW,
                UnixMillis(NOW.0 + 3_600_000),
            )
            .await
            .unwrap();

        // Host lease expires -> SUSPECT, then suspect deadline passes -> RealmsRecovery ends the hunt ABORTED
        clock.set(UnixMillis(NOW.0 + 10_000));
        assert_eq!(
            hosting.sweep().await.unwrap(),
            [(
                SESSION.to_owned(),
                SweepStep::Suspect {
                    reason: "HOST_LEASE_MISSED".to_owned()
                }
            )]
        );
        clock.set(UnixMillis(NOW.0 + 25_000));
        let steps = hosting.sweep().await.unwrap();
        assert_eq!(
            steps,
            [(
                SESSION.to_owned(),
                SweepStep::Recovery {
                    reason: "HOST_LEASE_MISSED".to_owned(),
                    before_active: false,
                }
            )]
        );

        let stored = hunts.find_hunt(SESSION).await.unwrap().unwrap();
        assert_eq!(stored.state, HuntState::Aborted);
        assert_eq!(stored.failure_reason.as_deref(), Some("HOST_LEASE_MISSED"));
        assert_eq!(stored.lease_expires_at, None);
        assert_eq!(stored.active_party_key, None);
        let stored_cap = hunts
            .find_host_capability_by_digest(&capability.secret_digest)
            .await
            .unwrap()
            .unwrap();
        assert!(stored_cap.revoked_at.is_some());
        let messages = hunts
            .list_control_messages("pal", 0, UnixMillis(NOW.0 + 25_000), 10, false)
            .await
            .unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].message_type, "FENCE_NOW");
        let logged = warnings.0.lock().unwrap().clone();
        assert!(
            logged
                .iter()
                .any(|line| line.contains(SESSION) && line.contains("HOST_LEASE_MISSED")),
            "{logged:?}"
        );
        assert!(
            hosting.sweep().await.unwrap().is_empty(),
            "aborted hunt does not stay live"
        );
    }
}
