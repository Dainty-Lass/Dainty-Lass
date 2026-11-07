// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `RealmsHuntSessions` and `SqlGuardStore` over one `RealmsDb`: Guard's view of realms' hunts and hubs, and Guard
//! itself running on both. SQLite, and Postgres when configured.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use std::sync::Arc;

use common::{actor, admitted, hunt, node, participant, targets};
use dainty_store::DatabaseOptions;
use rustic_rifts_guard::{
    CompletionMode, Guard, GuardConfig, GuardParts, GuardStore, HostWriteRecord, HuntSessions, MemoryLog,
    MemoryOperations, RosterEntry, SqlGuardStore,
};
use rustic_rifts_realms::RealmsHuntSessions;
use rustic_rifts_realms::db::RealmsDb;
use rustic_rifts_realms::hunt::{HostCapabilityRecord, HuntState};
use rustic_rifts_realms::ids::{seam_realm_id, seam_session_id};
use rustic_rifts_realms::ports::ManualClock;
use rustic_rifts_realms::repo::hub::{EntryAt, HubEntryRequest, RealmHubRepository};
use rustic_rifts_realms::repo::hunts::HuntRepository;
use rustic_rifts_seam::{
    AccountId, Actor, CapabilityScope, GuardHooks, HOST_CAPABILITY_HEADER, HostWrite, HostWriteKind, HostWriteOptions,
    HubKind, InventoryDecision, InventoryTransaction, Json, NoCore, RequestInfo, RuntimeAuth, SessionId, UnixMillis,
};
use serde_json::json;

const HUNT: &str = "0190a1b2-c3d4-4e5f-8a6b-7c8d9e0f1a2b";
const HUB: &str = "0190a1b2-c3d4-4e5f-8a6b-7c8d9e0f1a2c";
const REALM: &str = "0190a1b2-c3d4-4e5f-8a6b-7c8d9e0f1a2d";
const NOW: i64 = 1_791_892_800_000;
/// A capability token's shape: 43 characters of base64url.
const TOKEN: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJ0123-_x";

fn seam(wire: &str) -> SessionId {
    seam_session_id(wire, &node()).unwrap()
}

fn sessions(db: &RealmsDb, now: i64) -> (RealmsHuntSessions, Arc<ManualClock>) {
    let clock = Arc::new(ManualClock::at(rustic_rifts_seam::UnixMillis(now)));
    (RealmsHuntSessions::new(db.clone(), clock.clone()), clock)
}

async fn running_hunt(db: &RealmsDb) {
    let mut h = hunt(HUNT);
    h.state = HuntState::Suspect;
    h.pre_suspect_state = Some(HuntState::Running);
    h.realm_id = Some(REALM.into());
    h.guard_completion_mode = Some("NO_COMPLETION".into());
    h.roster = vec![
        admitted(participant("host", true)),
        admitted(participant("member", true)),
        participant("pending", false),
    ];
    h.extra.insert(
        "frozenLoadouts".into(),
        json!([
            {"accountId": "member", "characterId": "char-m"},
            {"account_id": "host", "character_id": " char-h "},
            {"accountId": "pending", "characterId": "  "},
            {"characterId": "orphan"}
        ]),
    );
    HuntRepository::new(db.clone()).create_hunt(&actor(), &h).await.unwrap();
}

async fn hub_session(db: &RealmsDb, host: &str) {
    let mut h = hunt(HUB);
    h.state = HuntState::Running;
    h.host_account_id = host.into();
    h.hub_kind = Some(HubKind::Ramsgate);
    h.realm_id = Some(REALM.into());
    h.active_party_key = Some(format!("realmhub:{REALM}:ramsgate"));
    HuntRepository::new(db.clone()).create_hunt(&actor(), &h).await.unwrap();
}

async fn present(db: &RealmsDb, account: &str) {
    let hub = RealmHubRepository::new(db.clone());
    let at = EntryAt {
        session_id: HUB.into(),
        generation: 1,
        realm_id: REALM.into(),
        now: UnixMillis(NOW),
        expires_at: UnixMillis(NOW + 90_000),
    };
    let request = HubEntryRequest {
        account_id: account.into(),
        peer_id: "1".into(),
        candidate_id: "c".into(),
    };
    hub.request_entry(&actor(), &at, &request).await.unwrap();
    hub.mark_admitted(&actor(), HUB, 1, account, "adm", UnixMillis(NOW), UnixMillis(NOW), None)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_hunt_maps_to_guards_view() {
    for target in targets() {
        running_hunt(&target.db).await;
        let (hunts, _) = sessions(&target.db, NOW);
        let view = hunts.hunt(&seam(HUNT)).unwrap().unwrap();
        assert_eq!(view.session, seam(HUNT));
        assert_eq!((view.generation, view.state.as_str()), (1, "SUSPECT"));
        assert_eq!(view.pre_suspect_state.as_deref(), Some("RUNNING"));
        assert_eq!(view.host, Some(AccountId("host".into())));
        assert_eq!(view.realm, seam_realm_id(REALM, &node()));
        assert_eq!(view.hub, None);
        assert_eq!(
            view.roster,
            ["host", "member", "pending"]
                .iter()
                .map(|a| RosterEntry {
                    account: AccountId((*a).into()),
                    admitted: *a != "pending",
                })
                .collect::<Vec<_>>()
        );
        assert_eq!(
            view.frozen_characters,
            [
                (AccountId("member".into()), "char-m".to_owned()),
                (AccountId("host".into()), "char-h".to_owned())
            ]
        );
        assert_eq!(view.completion_mode, CompletionMode::parse("NO_COMPLETION"));
        assert_eq!(hunts.hunt(&seam(HUB)).unwrap(), None);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_hub_without_a_host_and_its_present_accounts_through_the_cache() {
    for target in targets() {
        hub_session(&target.db, "").await;
        let (hunts, clock) = sessions(&target.db, NOW);
        let view = hunts.hunt(&seam(HUB)).unwrap().unwrap();
        assert_eq!((&view.host, view.hub), (&None, Some(HubKind::Ramsgate)));
        assert_eq!(view.session_kind(), Some("REALM_HUB"));

        present(&target.db, "m1").await;
        assert_eq!(
            hunts.hub_present_accounts(&seam(HUB), 1).unwrap(),
            [AccountId("m1".into())]
        );
        present(&target.db, "m2").await;
        // Cached for a second.
        assert_eq!(hunts.hub_present_accounts(&seam(HUB), 1).unwrap().len(), 1);
        clock.advance(1000);
        assert_eq!(hunts.hub_present_accounts(&seam(HUB), 1).unwrap().len(), 2);
        RealmHubRepository::new(target.db.clone())
            .mark_lost(&actor(), HUB, 1, "m1", None, UnixMillis(NOW), UnixMillis(NOW + 1))
            .await
            .unwrap();
        assert_eq!(hunts.hub_present_accounts(&seam(HUB), 1).unwrap().len(), 2);
        hunts.invalidate_hub_presence(&seam(HUB));
        assert_eq!(
            hunts.hub_present_accounts(&seam(HUB), 1).unwrap(),
            [AccountId("m2".into())]
        );
        assert!(hunts.hub_present_accounts(&seam(HUB), u64::MAX).unwrap().is_empty());
        assert!(hunts.hub_present_accounts(&seam(HUB), 2).unwrap().is_empty());
    }
}

fn capability(id: &str, digest: &str, revoked_at: Option<i64>) -> HostCapabilityRecord {
    HostCapabilityRecord {
        capability_id: id.into(),
        secret_digest: digest.into(),
        session_id: HUNT.into(),
        generation: 1,
        host_account_id: "host".into(),
        host_steam_id: common::HOST_STEAM.into(),
        bridge_instance_id: "bridge".into(),
        scopes: vec![CapabilityScope::GameServerRuntime, CapabilityScope::EventsWrite],
        issued_at: UnixMillis(NOW),
        expires_at: UnixMillis(NOW + 60_000),
        revoked_at: revoked_at.map(UnixMillis),
        last_sequence: 0,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn capabilities_by_digest_answer_active_then_revoked_views() {
    for target in targets() {
        let repo = HuntRepository::new(target.db.clone());
        let digest = rustic_rifts_guard::sha256_hex(TOKEN.as_bytes());
        repo.create_host_capability(&actor(), &capability("cap-1", &digest, None))
            .await
            .unwrap();
        let (hunts, _) = sessions(&target.db, NOW);
        let view = hunts.capability_by_digest(&digest).unwrap().unwrap();
        assert_eq!((view.capability_id.as_str(), view.revoked), ("cap-1", false));
        assert_eq!(view.expires_at, rustic_rifts_seam::UnixMillis(NOW + 60_000));
        let auth = hunts
            .resolve_capability(
                &digest,
                HUNT,
                CapabilityScope::EventsWrite,
                rustic_rifts_seam::UnixMillis(NOW),
            )
            .unwrap()
            .unwrap();
        assert_eq!(auth.session, seam(HUNT));
        assert_eq!(auth.wire_session.as_deref(), Some(HUNT));
        assert!(auth.roster.is_empty());
        let by_seam = seam(HUNT).to_string();
        assert!(
            hunts
                .resolve_capability(
                    &digest,
                    &by_seam,
                    CapabilityScope::EventsWrite,
                    rustic_rifts_seam::UnixMillis(NOW)
                )
                .unwrap()
                .is_some()
        );
        assert!(
            hunts
                .resolve_capability(
                    &digest,
                    HUNT,
                    CapabilityScope::Abort,
                    rustic_rifts_seam::UnixMillis(NOW)
                )
                .unwrap()
                .is_none()
        );
        assert!(
            hunts
                .resolve_capability(
                    &digest,
                    HUB,
                    CapabilityScope::EventsWrite,
                    rustic_rifts_seam::UnixMillis(NOW)
                )
                .unwrap()
                .is_none()
        );

        repo.revoke_generation(&actor(), HUNT, 1, UnixMillis(NOW + 1))
            .await
            .unwrap();
        assert!(hunts.capability_by_digest(&digest).unwrap().unwrap().revoked);
        assert_eq!(hunts.capability_by_digest("0".repeat(64).as_str()).unwrap(), None);
    }
}

fn guard(db: &RealmsDb) -> (Guard, SqlGuardStore) {
    let clock = Arc::new(rustic_rifts_guard::ManualClock::at(rustic_rifts_seam::UnixMillis(NOW)));
    let store = db.guard_store();
    let guard = Guard::new(GuardParts {
        config: GuardConfig::default(),
        store: Arc::new(store.clone()),
        hunts: Arc::new(RealmsHuntSessions::with_guard_clock(db.clone(), clock.clone())),
        operations: Arc::new(MemoryOperations::default()),
        core: Arc::new(NoCore),
        clock,
        log: Arc::new(MemoryLog::default()),
    });
    (guard, store)
}

fn runtime() -> RuntimeAuth {
    RuntimeAuth {
        capability_id: "cap-1".into(),
        session: seam(HUNT),
        generation: 1,
        host_account: AccountId("host".into()),
        scopes: vec![CapabilityScope::GameServerRuntime],
        roster: Vec::new(),
        host_peer: None,
        bridge_instance: None,
        wire_session: Some(HUNT.into()),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn guard_over_realms_authorises_and_records_a_host_write() {
    for target in targets() {
        let mut h = hunt(HUNT);
        h.state = HuntState::Running;
        h.roster = vec![
            admitted(participant("host", true)),
            admitted(participant("member", true)),
        ];
        h.extra.insert(
            "frozenLoadouts".into(),
            json!([{"accountId": "member", "characterId": "char-m"}]),
        );
        HuntRepository::new(target.db.clone())
            .create_hunt(&actor(), &h)
            .await
            .unwrap();
        let (guard, store) = guard(&target.db);

        let member = AccountId("member".into());
        let refused = guard
            .authorize_host_write(
                runtime(),
                HostWriteKind::Xp,
                member.clone(),
                HostWriteOptions::default(),
            )
            .await
            .unwrap();
        assert_eq!(refused, None, "{}", target.engine);
        let write = HostWrite {
            kind: HostWriteKind::Xp,
            target: member.clone(),
            summary: Json(r#"{"xp":5}"#.into()),
        };
        guard.record_host_write(runtime(), write).await.unwrap();
        let writes = store.host_writes().unwrap();
        assert_eq!(writes.len(), 1);
        assert_eq!(
            (writes[0].target.clone(), writes[0].session.clone()),
            (member.clone(), seam(HUNT))
        );

        let inventory = |user: &str| InventoryTransaction {
            user: AccountId(user.into()),
            character_id: "char-m".into(),
            transaction_id: "tx".into(),
            body: None,
        };
        let decision = guard.authorize_inventory(runtime(), inventory("member")).await.unwrap();
        assert!(matches!(decision, InventoryDecision::Authorized(_)));
        let decision = guard
            .authorize_inventory(runtime(), inventory("stranger"))
            .await
            .unwrap();
        assert!(matches!(decision, InventoryDecision::Refused(_)));

        let stranger = guard
            .authorize_host_write(
                runtime(),
                HostWriteKind::Xp,
                AccountId("stranger".into()),
                HostWriteOptions::default(),
            )
            .await
            .unwrap();
        assert!(stranger.is_some());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn guard_resolves_a_rate_limit_bucket_from_the_header() {
    for target in targets() {
        let digest = rustic_rifts_guard::sha256_hex(TOKEN.as_bytes());
        HuntRepository::new(target.db.clone())
            .create_host_capability(&actor(), &capability("cap-9", &digest, None))
            .await
            .unwrap();
        let (guard, _) = guard(&target.db);
        let request = |value: &str| RequestInfo {
            headers: vec![(HOST_CAPABILITY_HEADER.to_owned(), value.to_owned())],
            ..RequestInfo::default()
        };
        assert_eq!(
            guard.rate_limit_bucket(request(TOKEN)).await.unwrap().as_deref(),
            Some("host:cap-9")
        );
        assert_eq!(guard.rate_limit_bucket(request("short")).await.unwrap(), None);
        assert_eq!(guard.rate_limit_bucket(RequestInfo::default()).await.unwrap(), None);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn guard_first_then_realms_then_both_again_keeps_the_data() {
    for target in targets() {
        // A fresh database of the same engine, migrated by Guard alone first.
        let fresh = common::empty(target.engine);
        let config = fresh.config.clone();
        let guard_only = SqlGuardStore::open(&config, &DatabaseOptions::default()).unwrap();
        let record = HostWriteRecord {
            kind: HostWriteKind::Mastery,
            target: AccountId("m".into()),
            host: AccountId("host".into()),
            session: seam(HUNT),
            generation: 1,
            realm: None,
            session_kind: None,
            hub_kind: None,
            summary: Json("{}".into()),
            at: rustic_rifts_seam::UnixMillis(NOW),
        };
        let actor = Actor::System("test");
        guard_only.record_host_write(&actor, &record).unwrap();
        drop(guard_only);

        let db = RealmsDb::open(&config, &DatabaseOptions::default(), node()).unwrap();
        HuntRepository::new(db.clone())
            .create_hunt(&actor, &hunt(HUNT))
            .await
            .unwrap();
        assert_eq!(db.guard_store().host_writes().unwrap().len(), 1);
        drop(db);

        let again = RealmsDb::open(&config, &DatabaseOptions::default(), node()).unwrap();
        let guard_again = SqlGuardStore::open(&config, &DatabaseOptions::default()).unwrap();
        assert_eq!(guard_again.host_writes().unwrap(), [record]);
        assert!(HuntRepository::new(again).find_hunt(HUNT).await.unwrap().is_some());
        drop(fresh);
    }
}
