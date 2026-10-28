//! Integration tests for [`RealmsHuntSessions`] (`rifts_guard::ports::HuntSessions`) and [`SqlGuardStore`]
//! (`rifts_guard::GuardStore`) over [`RealmsDb`] on SQLite and Postgres.
#![allow(clippy::unwrap_used)]

mod common;

use std::sync::Arc;

use common::{actor, hunt_session, participant, targets};
use rifts_guard::ports::{CapabilityView, HuntSessions, HuntView, RosterEntry};
use rifts_guard::{
    Guard, GuardConfig, GuardParts, GuardStore, HostWriteRecord, ManualClock as GuardClock, MemoryLog, MemoryOperations,
};
use rifts_realms::RealmsHuntSessions;
use rifts_realms::capabilities::{HostCapabilities, hash_capability_secret};
use rifts_realms::hunt::{AdmissionState, HuntState};
use rifts_realms::ids::{seam_realm_id, seam_session_id};
use rifts_realms::ports::ManualClock;
use rifts_realms::repo::hub::{EntryAt, HubEntryRequest, RealmHubRepository};
use rifts_realms::repo::hunts::HuntRepository;
use rifts_seam::{
    AccountId, CapabilityScope, GuardHooks, HOST_CAPABILITY_HEADER, HostWrite, HostWriteKind, HostWriteOptions,
    HubKind, InventoryDecision, InventoryTransaction, Json, NoCore, RequestInfo, RuntimeAuth, UnixMillis,
};

const SESSION_WIRE: &str = "0d6f2b9c-5b1e-4c3a-9e57-2f1a8b7c4d10";
const HUB_SESSION_WIRE: &str = "1a2b3c4d-5e6f-4a7b-8c9d-0e1f2a3b4c5d";
const REALM_WIRE: &str = "2b3c4d5e-6f7a-4b8c-9d0e-1f2a3b4c5d6e";
const NOW: UnixMillis = UnixMillis(1_791_892_800_000);
const HOUR: i64 = 3_600_000;

async fn reserve_pending(hub: &RealmHubRepository, session_id: &str, generation: i64, account: &str) {
    hub.request_entries(
        &actor(),
        &EntryAt {
            session_id: session_id.to_owned(),
            generation,
            realm_id: REALM_WIRE.to_owned(),
            now: NOW,
            expires_at: UnixMillis(NOW.0 + HOUR),
        },
        64,
        &[HubEntryRequest {
            account_id: account.to_owned(),
            peer_id: format!("peer-{account}"),
            candidate_id: format!("cand-{account}"),
        }],
    )
    .await
    .unwrap();
}

async fn admit_player(hub: &RealmHubRepository, session_id: &str, generation: i64, account: &str) {
    reserve_pending(hub, session_id, generation, account).await;
    hub.mark_admitted(
        &actor(),
        session_id,
        generation,
        account,
        &format!("adm-{account}"),
        NOW,
        NOW,
        None,
    )
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn hunt_maps_hunt_and_hub_sessions_to_hunt_view() {
    for target in targets() {
        let db = target.open();
        let hunts = HuntRepository::new(db.clone());
        let clock = Arc::new(ManualClock::at(NOW));
        let adapter = RealmsHuntSessions::new(db.clone(), clock);

        let seam_sess = seam_session_id(SESSION_WIRE, db.node()).unwrap();
        let seam_hub = seam_session_id(HUB_SESSION_WIRE, db.node()).unwrap();
        let seam_realm = seam_realm_id(REALM_WIRE, db.node()).unwrap();

        // Missing session returns None
        assert_eq!(adapter.hunt(&seam_sess).unwrap(), None);

        // Regular hunt with roster, suspect state, realm, and frozenLoadouts
        let mut hunt = hunt_session(SESSION_WIRE, HuntState::Suspect);
        hunt.generation = 3;
        hunt.host_account_id = "host-1".to_owned();
        hunt.pre_suspect_state = Some(HuntState::Running);
        hunt.realm_id = Some(REALM_WIRE.to_owned());
        hunt.roster = vec![
            participant("host-1", AdmissionState::Admitted, true),
            participant("friend-1", AdmissionState::Admitted, true),
            participant("pending-1", AdmissionState::Pending, false),
        ];
        hunt.extra.insert(
            "frozenLoadouts".to_owned(),
            serde_json::json!([
                { "accountId": "host-1", "characterId": "char-host" },
                { "accountId": "friend-1", "characterId": "char-friend" }
            ]),
        );
        hunts.create_hunt(&actor(), &hunt).await.unwrap();

        let view = adapter.hunt(&seam_sess).unwrap().unwrap();
        assert_eq!(
            view,
            HuntView {
                session: seam_sess.clone(),
                generation: 3,
                state: "SUSPECT".to_owned(),
                pre_suspect_state: Some("RUNNING".to_owned()),
                host: Some(AccountId("host-1".to_owned())),
                realm: Some(seam_realm.clone()),
                hub: None,
                roster: vec![
                    RosterEntry {
                        account: AccountId("host-1".to_owned()),
                        admitted: true,
                    },
                    RosterEntry {
                        account: AccountId("friend-1".to_owned()),
                        admitted: true,
                    },
                    RosterEntry {
                        account: AccountId("pending-1".to_owned()),
                        admitted: false,
                    },
                ],
                frozen_characters: vec![
                    (AccountId("host-1".to_owned()), "char-host".to_owned()),
                    (AccountId("friend-1".to_owned()), "char-friend".to_owned()),
                ],
            }
        );

        // Realm hub session maps hub_kind and empty host when host_account_id is empty
        let mut hub_hunt = hunt_session(HUB_SESSION_WIRE, HuntState::Running);
        hub_hunt.generation = 2;
        hub_hunt.host_account_id = String::new();
        hub_hunt.realm_id = Some(REALM_WIRE.to_owned());
        hub_hunt.hub_kind = Some(HubKind::Ramsgate);
        hub_hunt.roster.clear();
        hunts.create_hunt(&actor(), &hub_hunt).await.unwrap();

        let hub_view = adapter.hunt(&seam_hub).unwrap().unwrap();
        assert_eq!(hub_view.session, seam_hub);
        assert_eq!(hub_view.generation, 2);
        assert_eq!(hub_view.state, "RUNNING");
        assert_eq!(hub_view.host, None);
        assert_eq!(hub_view.realm, Some(seam_realm));
        assert_eq!(hub_view.hub, Some(HubKind::Ramsgate));
        assert!(hub_view.is_hub());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn hub_present_accounts_filters_admitted_and_caches_for_one_second() {
    for target in targets() {
        let db = target.open();
        let hub = RealmHubRepository::new(db.clone());
        let clock = Arc::new(ManualClock::at(NOW));
        let caps = HostCapabilities::new(db.clone());
        let adapter = caps.hunt_sessions(clock.clone());
        let seam_hub = seam_session_id(HUB_SESSION_WIRE, db.node()).unwrap();

        admit_player(&hub, HUB_SESSION_WIRE, 1, "p-admitted").await;
        reserve_pending(&hub, HUB_SESSION_WIRE, 1, "p-pending").await;

        let present = adapter.hub_present_accounts(&seam_hub, 1).unwrap();
        assert_eq!(present, vec![AccountId("p-admitted".to_owned())]);

        // Add a second admitted player; within 1s (500ms later) the cache still answers the old list
        admit_player(&hub, HUB_SESSION_WIRE, 1, "p-second").await;
        clock.advance(500);
        assert_eq!(
            adapter.hub_present_accounts(&seam_hub, 1).unwrap(),
            vec![AccountId("p-admitted".to_owned())]
        );

        // Past the 1-second TTL (500ms + 500ms = 1000ms), the cache expires and reloads from the DB
        clock.advance(500);
        let refreshed = adapter.hub_present_accounts(&seam_hub, 1).unwrap();
        assert_eq!(
            refreshed,
            vec![AccountId("p-admitted".to_owned()), AccountId("p-second".to_owned()),]
        );

        // Explicit invalidation evicts the cached entry immediately before the 1s TTL expires
        admit_player(&hub, HUB_SESSION_WIRE, 1, "p-third").await;
        clock.advance(100);
        assert_eq!(adapter.hub_present_accounts(&seam_hub, 1).unwrap().len(), 2);
        adapter.invalidate_hub_presence(&seam_hub);
        assert_eq!(adapter.hub_present_accounts(&seam_hub, 1).unwrap().len(), 3);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn capability_by_digest_returns_active_and_revoked_views() {
    for target in targets() {
        let db = target.open();
        let hunts = HuntRepository::new(db.clone());
        let caps = HostCapabilities::new(db.clone());
        let clock = Arc::new(ManualClock::at(NOW));
        let adapter = caps.hunt_sessions(clock);

        assert_eq!(adapter.capability_by_digest("00".repeat(32).as_str()).unwrap(), None);

        let hunt = hunt_session(SESSION_WIRE, HuntState::Running);
        hunts.create_hunt(&actor(), &hunt).await.unwrap();
        let (secret, record) = caps
            .mint(
                &hunt,
                "bridge-1",
                vec![CapabilityScope::GameServerRuntime, CapabilityScope::EventsWrite],
                NOW,
                UnixMillis(NOW.0 + HOUR),
            )
            .await
            .unwrap();
        let digest = hash_capability_secret(secret.expose()).unwrap();

        assert_eq!(
            adapter.capability_by_digest(&digest).unwrap(),
            Some(CapabilityView {
                capability_id: record.capability_id.clone(),
                expires_at: UnixMillis(NOW.0 + HOUR),
                revoked: false,
            })
        );

        caps.revoke(SESSION_WIRE, hunt.generation, UnixMillis(NOW.0 + 1_000))
            .await
            .unwrap();
        assert_eq!(
            adapter.capability_by_digest(&digest).unwrap(),
            Some(CapabilityView {
                capability_id: record.capability_id,
                expires_at: UnixMillis(NOW.0 + HOUR),
                revoked: true,
            })
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn guard_integrates_with_realms_hunt_sessions_and_sql_guard_store() {
    for target in targets() {
        let db = target.open();
        let hunts = HuntRepository::new(db.clone());
        let caps = HostCapabilities::new(db.clone());
        let guard_clock = Arc::new(GuardClock::at(NOW));
        let sessions = Arc::new(RealmsHuntSessions::with_guard_clock(db.clone(), guard_clock.clone()));
        let sql_store = Arc::new(db.guard_store());

        let seam_sess = seam_session_id(SESSION_WIRE, db.node()).unwrap();
        let seam_realm = seam_realm_id(REALM_WIRE, db.node()).unwrap();

        let mut hunt = hunt_session(SESSION_WIRE, HuntState::Running);
        hunt.generation = 1;
        hunt.host_account_id = "host-1".to_owned();
        hunt.realm_id = Some(REALM_WIRE.to_owned());
        hunt.roster = vec![
            participant("host-1", AdmissionState::Admitted, true),
            participant("friend-1", AdmissionState::Admitted, true),
        ];
        hunt.extra.insert(
            "frozenLoadouts".to_owned(),
            serde_json::json!([
                { "accountId": "friend-1", "characterId": "char-friend" }
            ]),
        );
        hunts.create_hunt(&actor(), &hunt).await.unwrap();

        let (secret, cap_record) = caps
            .mint(
                &hunt,
                "bridge-1",
                vec![CapabilityScope::GameServerRuntime, CapabilityScope::EventsWrite],
                NOW,
                UnixMillis(NOW.0 + HOUR),
            )
            .await
            .unwrap();

        let guard = Guard::new(GuardParts {
            config: GuardConfig::default(),
            store: sql_store.clone(),
            hunts: sessions,
            operations: Arc::new(MemoryOperations::default()),
            core: Arc::new(NoCore),
            clock: guard_clock,
            log: Arc::new(MemoryLog::default()),
        });

        let runtime = RuntimeAuth {
            capability_id: cap_record.capability_id.clone(),
            session: seam_sess.clone(),
            generation: 1,
            host_account: AccountId("host-1".to_owned()),
            scopes: vec![CapabilityScope::GameServerRuntime],
            roster: vec![AccountId("host-1".to_owned()), AccountId("friend-1".to_owned())],
            host_peer: None,
            bridge_instance: Some("bridge-1".to_owned()),
            wire_session: Some(SESSION_WIRE.to_owned()),
        };

        // 1. Authorize and record a host write for admitted participant friend-1
        let refusal = guard
            .authorize_host_write(
                runtime.clone(),
                HostWriteKind::Xp,
                AccountId("friend-1".to_owned()),
                HostWriteOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(refusal, None);

        guard
            .record_host_write(
                runtime.clone(),
                HostWrite {
                    kind: HostWriteKind::Xp,
                    target: AccountId("friend-1".to_owned()),
                    summary: Json(r#"{"xp":500}"#.to_owned()),
                },
            )
            .await
            .unwrap();

        let writes = sql_store.host_writes().unwrap();
        assert_eq!(
            writes,
            vec![HostWriteRecord {
                kind: HostWriteKind::Xp,
                target: AccountId("friend-1".to_owned()),
                host: AccountId("host-1".to_owned()),
                session: seam_sess,
                generation: 1,
                realm: Some(seam_realm),
                session_kind: None,
                hub_kind: None,
                summary: Json(r#"{"xp":500}"#.to_owned()),
                at: NOW,
            }]
        );

        // 2. Authorize an inventory transaction matching frozenLoadouts characterId
        let inv_ok = guard
            .authorize_inventory(
                runtime.clone(),
                InventoryTransaction {
                    user: AccountId("friend-1".to_owned()),
                    character_id: "char-friend".to_owned(),
                    transaction_id: "tx-1".to_owned(),
                    body: None,
                },
            )
            .await
            .unwrap();
        assert!(matches!(inv_ok, InventoryDecision::Authorized(_)));

        // Non-member is refused
        let inv_stranger = guard
            .authorize_inventory(
                runtime,
                InventoryTransaction {
                    user: AccountId("stranger".to_owned()),
                    character_id: "char-stranger".to_owned(),
                    transaction_id: "tx-2".to_owned(),
                    body: None,
                },
            )
            .await
            .unwrap();
        assert!(matches!(inv_stranger, InventoryDecision::Refused(_)));

        // 3. Capability lookup via Guard rate_limit_bucket resolves active capability through RealmsHuntSessions
        let mut req = RequestInfo::default();
        req.headers
            .push((HOST_CAPABILITY_HEADER.to_owned(), secret.expose().to_owned()));
        let bucket = guard.rate_limit_bucket(req).await.unwrap();
        assert_eq!(bucket, Some(format!("host:{}", cap_record.capability_id)));
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn realms_and_guard_schemas_migrate_fresh_and_existing_databases() {
    for target in targets() {
        let config = target.config();
        let options = store::DatabaseOptions::default();

        // 1. Fresh database migrated by rifts-guard first (`2026-10-26-000002_rifts_guard`).
        let guard_store = rifts_guard::SqlGuardStore::open(&config, &options).unwrap();
        let seam_sess = seam_session_id(SESSION_WIRE, &module_api::NodeId::new("local").unwrap()).unwrap();
        guard_store
            .record_host_write(
                &actor(),
                &HostWriteRecord {
                    kind: HostWriteKind::Xp,
                    target: AccountId("friend-1".to_owned()),
                    host: AccountId("host-1".to_owned()),
                    session: seam_sess.clone(),
                    generation: 1,
                    realm: None,
                    session_kind: None,
                    hub_kind: None,
                    summary: Json(r#"{"xp":100}"#.to_owned()),
                    at: NOW,
                },
            )
            .unwrap();

        // 2. Existing database that already applied `2026-10-26-000002_rifts_guard` is then migrated by `rifts-realms`.
        let realms_db =
            rifts_realms::db::RealmsDb::open(&config, &options, module_api::NodeId::new("local").unwrap()).unwrap();
        let hunts = HuntRepository::new(realms_db.clone());
        let hunt = hunt_session(SESSION_WIRE, HuntState::Running);
        hunts.create_hunt(&actor(), &hunt).await.unwrap();

        // 3. Reopening both `rifts-guard` and `rifts-realms` on the already-migrated database succeeds and preserves data.
        let guard_reopened = rifts_guard::SqlGuardStore::open(&config, &options).unwrap();
        assert_eq!(guard_reopened.host_writes().unwrap().len(), 1);
        let realms_reopened =
            rifts_realms::db::RealmsDb::open(&config, &options, module_api::NodeId::new("local").unwrap()).unwrap();
        let hunts_reopened = HuntRepository::new(realms_reopened);
        assert!(hunts_reopened.find_hunt(SESSION_WIRE).await.unwrap().is_some());
    }
}
