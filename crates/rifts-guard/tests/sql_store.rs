// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `SqlGuardStore` through its public API, on SQLite and (with `REFORGED_TEST_POSTGRES_URL`, a server the test may
//! create a database on: `postgres://user:pw@127.0.0.1:5432/postgres`) on Postgres. Each engine runs from sync
//! code, from a multi-thread runtime and from a current-thread one, since `block_on_store` serves all three.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;

use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use diesel::sql_types::Text;
use diesel::{Connection, PgConnection};
use rifts_guard::{
    AlertRecord, BatchRecord, ChainRecord, ChainUpdate, CompletionRecord, Disposition, EncounterState, FindingRecord,
    GuardStore, HostWriteRecord, SqlGuardStore, TABLES, TrafficReservation, TrafficScope, XP_SUMMARY_TTL_MS,
    XpSummaryRecord, block_on_store,
};
use rifts_seam::{AccountId, Actor, HostWriteKind, Json, RealmId, SessionId, StoreError, UnixMillis};
use store::{AnyConnection, DatabaseConfig, DatabaseOptions, Engine, PostgresConfig, portable};

const SESSION: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV@eu-1";
const REALM: &str = "01BX5ZZKBKACTAV9WEVGEMMVS0@eu-1";

fn session() -> SessionId {
    SESSION.parse().unwrap()
}

fn host_actor() -> Actor {
    Actor::HostRuntime {
        session: session(),
        host: AccountId("host-1".into()),
    }
}

#[derive(QueryableByName)]
struct Name {
    #[diesel(sql_type = Text)]
    name: String,
}

fn guard_tables(store: &SqlGuardStore) -> Vec<String> {
    let db = store.database().clone();
    block_on_store(async move {
        db.read(|conn: &mut AnyConnection| {
            let sql = match conn.engine() {
                Engine::Sqlite => {
                    "SELECT name FROM sqlite_schema WHERE type = 'table' AND name LIKE 'rifts!_%' ESCAPE '!' ORDER BY rowid"
                }
                Engine::Postgres => {
                    "SELECT relname::text AS name FROM pg_class WHERE relkind = 'r' AND relname LIKE 'rifts!_%' ESCAPE '!' \
                     AND relnamespace = current_schema()::regnamespace ORDER BY oid"
                }
            };
            Ok::<_, store::Error>(diesel::sql_query(sql).load::<Name>(conn)?)
        })
        .await
        .map_err(|e| StoreError::Database(e.to_string()))
    })
    .unwrap()
    .into_iter()
    .map(|n| n.name)
    .collect()
}

fn write(kind: HostWriteKind, realm: Option<&str>, at: i64) -> HostWriteRecord {
    HostWriteRecord {
        kind,
        target: AccountId("member-1".into()),
        host: AccountId("host-1".into()),
        session: session(),
        generation: 3,
        realm: realm.map(|r| r.parse::<RealmId>().unwrap()),
        session_kind: realm.map(|_| "REALM_HUB".to_owned()),
        hub_kind: realm.map(|_| "RAMSGATE".to_owned()),
        summary: Json(r#"{"xp":120,"note":"ü"}"#.into()),
        at: UnixMillis(at),
    }
}

fn reservation(scope: TrafficScope, id: &str, max_requests: u64) -> TrafficReservation {
    TrafficReservation {
        window_id: id.into(),
        scope,
        window_start: UnixMillis(60_000),
        expires_at: UnixMillis(180_000),
        bytes: 100,
        max_requests,
        max_bytes: 1000,
    }
}

fn chain_at(revision: u64) -> ChainRecord {
    let mut chain = ChainRecord::new(session(), 2, None);
    chain.revision = revision;
    chain
}

fn batch(id: &str, hash: &str) -> BatchRecord {
    BatchRecord {
        session: session(),
        generation: 2,
        batch_id: id.into(),
        previous_hash: "0".repeat(64),
        batch_hash: hash.into(),
        first_sequence: 1,
        last_sequence: 2,
        event_count: 2,
        event_bytes: 140,
        event_types: vec!["DAMAGE".into(), "WORLD_READY".into()],
        received_at: UnixMillis(9),
    }
}

fn finding(code: &str, sequence: Option<u64>) -> FindingRecord {
    FindingRecord {
        session: session(),
        generation: 2,
        sequence,
        code: code.into(),
        detail: format!("detail of {code}"),
        at: UnixMillis(9),
    }
}

fn summary(member: &str, minutes: u32, created_at: i64) -> XpSummaryRecord {
    XpSummaryRecord {
        session: session(),
        member: AccountId(member.into()),
        host: AccountId("host-1".into()),
        realm: Some(REALM.parse().unwrap()),
        minutes,
        grants: Json(r#"[{"track":"WEAPON","xp":40}]"#.into()),
        flagged: minutes > 30,
        created_at: UnixMillis(created_at),
    }
}

fn alert(id: &str, message: &str) -> AlertRecord {
    AlertRecord {
        alert_id: id.into(),
        code: "GUARD_TRAFFIC".into(),
        severity: "HIGH".into(),
        message: message.into(),
        details: Json(r#"{"hosts":3}"#.into()),
        created_at: UnixMillis(5),
        acknowledged_at: None,
        acknowledged_by: None,
    }
}

/// Every contract of the store, on a fresh database; returns what `purge_expired` removed.
fn exercise(store: &SqlGuardStore) -> u64 {
    let expected: Vec<&str> = TABLES.iter().map(|t| t.name).collect();
    assert_eq!(guard_tables(store), expected);

    // host writes come back as written, oldest first
    let rows = [
        write(HostWriteKind::SlayersPath, Some(REALM), 10),
        write(HostWriteKind::Xp, None, 11),
    ];
    for row in &rows {
        store.record_host_write(&host_actor(), row).unwrap();
    }
    assert_eq!(store.host_writes().unwrap(), rows);

    // the traffic pair: both windows or neither
    let system = Actor::System("guard");
    let host = reservation(TrafficScope::Host, "host:60000:a", 5);
    let tight = reservation(TrafficScope::Session, "session:60000:b", 1);
    assert!(store.reserve_traffic(&system, &host, &tight).unwrap());
    assert!(!store.reserve_traffic(&system, &host, &tight).unwrap());
    let windows = store.windows().unwrap();
    assert_eq!(windows.len(), 2);
    assert_eq!(
        windows
            .iter()
            .map(|w| (w.window_id.as_str(), w.scope, w.requests, w.bytes))
            .collect::<Vec<_>>(),
        [
            ("host:60000:a", TrafficScope::Host, 1, 100),
            ("session:60000:b", TrafficScope::Session, 1, 100)
        ]
    );
    // a later window purges the expired ones first
    let later = |r: TrafficReservation| TrafficReservation {
        window_start: UnixMillis(180_000),
        expires_at: UnixMillis(300_000),
        window_id: r.window_id.replace("60000", "180000"),
        ..r
    };
    assert!(
        store
            .reserve_traffic(&system, &later(host.clone()), &later(tight.clone()))
            .unwrap()
    );
    assert_eq!(store.windows().unwrap().len(), 2);
    assert!(
        store
            .windows()
            .unwrap()
            .iter()
            .all(|w| w.window_start == UnixMillis(180_000))
    );
    // the same id for both sees its own first charge
    let shared = TrafficReservation {
        window_start: UnixMillis(180_000),
        expires_at: UnixMillis(300_000),
        ..reservation(TrafficScope::Host, "shared", 2)
    };
    assert!(store.reserve_traffic(&system, &shared, &shared).unwrap());
    assert!(!store.reserve_traffic(&system, &shared, &shared).unwrap());

    // chains: the revision guards every write
    assert_eq!(store.chain(&session(), 2).unwrap(), None);
    let mut first = chain_at(1);
    first.encounter_state = EncounterState::WorldReady;
    first.last_sequence = 2;
    first.head_hash = "a".repeat(64);
    first.event_types = BTreeMap::from([("DAMAGE".to_owned(), 1), ("WORLD_READY".to_owned(), 1)]);
    first.has_unattributed_damage = true;
    first.last_monotonic_ms = Some(2500);
    first.last_received_at = Some(UnixMillis(9));
    first.finding_count = 2;
    first.disposition = Disposition::ObserveFlagged;
    first.batch_count = 1;
    first.event_count = 2;
    first.event_bytes = 140;
    let update = ChainUpdate {
        chain: first.clone(),
        batch: Some(batch("batch-1", &"a".repeat(64))),
        findings: vec![
            finding("ACTOR_NOT_ROSTERED", Some(1)),
            finding("UNATTRIBUTED_DAMAGE", Some(2)),
        ],
    };
    assert!(!store.apply_chain(&host_actor(), &update, 1).unwrap());
    assert!(store.apply_chain(&host_actor(), &update, 0).unwrap());
    assert_eq!(store.chain(&session(), 2).unwrap(), Some(first.clone()));
    assert!(!store.apply_chain(&host_actor(), &update, 0).unwrap());
    assert_eq!(store.batch(&session(), 2, "batch-1").unwrap(), update.batch.clone());
    assert_eq!(store.batch(&session(), 2, "batch-2").unwrap(), None);

    // a taken batch id writes nothing, even at the right revision
    let mut second = first.clone();
    second.revision = 2;
    second.last_sequence = 4;
    let replay = ChainUpdate {
        chain: second.clone(),
        batch: Some(batch("batch-1", &"b".repeat(64))),
        findings: vec![finding("EVIDENCE_LOSS", Some(3))],
    };
    assert!(!store.apply_chain(&host_actor(), &replay, 1).unwrap());
    assert_eq!(store.chain(&session(), 2).unwrap(), Some(first.clone()));
    assert_eq!(store.findings(&session(), 2).unwrap().len(), 2);

    // a finding with no batch, then the completion
    let flagged = ChainUpdate {
        chain: second.clone(),
        batch: None,
        findings: vec![finding("CHAIN_CONFLICT", None)],
    };
    assert!(store.apply_chain(&host_actor(), &flagged, 1).unwrap());
    let codes: Vec<(String, Option<u64>)> = store
        .findings(&session(), 2)
        .unwrap()
        .into_iter()
        .map(|f| (f.code, f.sequence))
        .collect();
    assert_eq!(
        codes,
        [
            ("ACTOR_NOT_ROSTERED".to_owned(), Some(1)),
            ("UNATTRIBUTED_DAMAGE".to_owned(), Some(2)),
            ("CHAIN_CONFLICT".to_owned(), None)
        ]
    );
    let mut done = second.clone();
    done.revision = 3;
    done.completion = Some(CompletionRecord {
        final_sequence: 4,
        final_hash: "c".repeat(64),
        at: UnixMillis(20),
    });
    let completion = ChainUpdate {
        chain: done.clone(),
        batch: None,
        findings: Vec::new(),
    };
    assert!(store.apply_chain(&host_actor(), &completion, 2).unwrap());
    assert_eq!(store.chain(&session(), 2).unwrap(), Some(done));
    assert_eq!(store.chain(&session(), 3).unwrap(), None);

    // XP summaries upsert by (session, member)
    store
        .put_xp_summary(&host_actor(), &summary("member-1", 10, 100))
        .unwrap();
    store
        .put_xp_summary(&host_actor(), &summary("member-1", 45, 200))
        .unwrap();
    store
        .put_xp_summary(&host_actor(), &summary("member-2", 5, 300))
        .unwrap();
    let member = AccountId("member-1".into());
    assert_eq!(
        store.xp_summary(&session(), &member).unwrap(),
        Some(summary("member-1", 45, 200))
    );
    assert_eq!(store.xp_summary(&session(), &AccountId("nobody".into())).unwrap(), None);

    // alerts: a retried insert keeps the first row, and one acknowledgement
    store.insert_alert(&system, &alert("alert-0001", "first")).unwrap();
    store.insert_alert(&system, &alert("alert-0001", "retried")).unwrap();
    assert_eq!(store.alert("alert-0001").unwrap().unwrap().message, "first");
    let admin = Actor::Admin {
        key_id: "ops-key".into(),
    };
    assert!(store.acknowledge_alert(&admin, "alert-0001", UnixMillis(30)).unwrap());
    assert!(!store.acknowledge_alert(&admin, "alert-0001", UnixMillis(31)).unwrap());
    assert!(
        !store
            .acknowledge_alert(&admin, "alert-missing", UnixMillis(31))
            .unwrap()
    );
    let acknowledged = store.alert("alert-0001").unwrap().unwrap();
    assert_eq!(
        (acknowledged.acknowledged_at, acknowledged.acknowledged_by.as_deref()),
        (Some(UnixMillis(30)), Some("ops-key"))
    );

    // purge: the windows of the 180 000 window (3 ids) are not expired yet at 299 999
    assert_eq!(store.purge_expired(UnixMillis(299_999)).unwrap(), 0);
    let removed = store.purge_expired(UnixMillis(300 + XP_SUMMARY_TTL_MS as i64)).unwrap();
    assert!(store.windows().unwrap().is_empty());
    assert_eq!(store.xp_summary(&session(), &member).unwrap(), None);
    removed
}

/// What a reopened database must still hold.
fn check_kept(store: &SqlGuardStore) {
    assert_eq!(store.host_writes().unwrap().len(), 2);
    assert!(store.chain(&session(), 2).unwrap().unwrap().completion.is_some());
    assert_eq!(store.findings(&session(), 2).unwrap().len(), 3);
    assert!(store.alert("alert-0001").unwrap().is_some());
}

fn sqlite(dir: &tempfile::TempDir) -> DatabaseConfig {
    DatabaseConfig::Sqlite {
        file: dir.path().join("guard.db"),
    }
}

#[test]
fn sqlite_from_sync_code() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqlGuardStore::open(&sqlite(&dir), &DatabaseOptions::default()).unwrap();
    assert_eq!(exercise(&store), 5);
    drop(store);
    let reopened = SqlGuardStore::open(&sqlite(&dir), &DatabaseOptions::default()).unwrap();
    check_kept(&reopened);
    let clone = reopened.clone();
    assert_eq!(clone.host_writes().unwrap(), reopened.host_writes().unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_from_a_multi_thread_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqlGuardStore::open(&sqlite(&dir), &DatabaseOptions::default()).unwrap();
    // three windows and two summaries
    assert_eq!(exercise(&store), 5);
    // two of each lapse together
    store.put_xp_summary(&host_actor(), &summary("member-3", 1, 0)).unwrap();
    store.put_xp_summary(&host_actor(), &summary("member-4", 1, 0)).unwrap();
    let system = Actor::System("guard");
    store
        .reserve_traffic(
            &system,
            &reservation(TrafficScope::Host, "h", 5),
            &reservation(TrafficScope::Session, "s", 5),
        )
        .unwrap();
    assert_eq!(store.purge_expired(UnixMillis(XP_SUMMARY_TTL_MS as i64)).unwrap(), 4);
}

#[tokio::test(flavor = "current_thread")]
async fn sqlite_from_a_current_thread_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqlGuardStore::open(&sqlite(&dir), &DatabaseOptions::default()).unwrap();
    assert_eq!(exercise(&store), 5);
}

#[test]
fn rows_that_do_not_map_back_are_failures() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqlGuardStore::open(&sqlite(&dir), &DatabaseOptions::default()).unwrap();
    store
        .record_host_write(&host_actor(), &write(HostWriteKind::Xp, None, 1))
        .unwrap();
    let db = store.database().clone();
    block_on_store(async move {
        db.write(store::Actor::System("test".into()), |tx| {
            tx.conn()
                .batch_execute("UPDATE rifts_host_writes SET kind = 'TELEPORT'")
                .map_err(store::Error::Query)
        })
        .await
        .map_err(|e| StoreError::Database(e.to_string()))
    })
    .unwrap();
    assert!(matches!(store.host_writes(), Err(StoreError::Database(text)) if text.contains("TELEPORT")));
}

struct PgDatabase {
    server: PostgresConfig,
    config: PostgresConfig,
}

impl Drop for PgDatabase {
    fn drop(&mut self) {
        if let Ok(mut conn) = PgConnection::establish(&self.server.conninfo()) {
            let _ = conn.batch_execute(&format!(
                "DROP DATABASE IF EXISTS {} WITH (FORCE)",
                portable::quote_ident(&self.config.database)
            ));
        }
    }
}

fn new_postgres(suffix: &str) -> Option<PgDatabase> {
    let url = url::Url::parse(&std::env::var("REFORGED_TEST_POSTGRES_URL").ok()?).expect("a postgres:// URL");
    let server = PostgresConfig {
        host: url.host_str().unwrap_or("127.0.0.1").to_owned(),
        port: url.port().unwrap_or(5432),
        user: url.username().to_owned(),
        database: url.path().trim_start_matches('/').to_owned(),
        password: url.password().unwrap_or_default().to_owned(),
        allow_remote: false,
    };
    let name = format!("rifts_guard_test_{}_{suffix}", std::process::id());
    PgConnection::establish(&server.conninfo())
        .expect("REFORGED_TEST_POSTGRES_URL is reachable")
        .batch_execute(&format!("CREATE DATABASE {}", portable::quote_ident(&name)))
        .unwrap();
    let config = PostgresConfig {
        database: name,
        ..server.clone()
    };
    Some(PgDatabase { server, config })
}

#[test]
fn postgres_from_sync_code() {
    let Some(pg) = new_postgres("sync") else {
        eprintln!("REFORGED_TEST_POSTGRES_URL is not set: skipping the Postgres half");
        return;
    };
    let config = DatabaseConfig::Postgres(pg.config.clone());
    let store = SqlGuardStore::open(&config, &DatabaseOptions::default()).unwrap();
    assert_eq!(exercise(&store), 5);
    drop(store);
    check_kept(&SqlGuardStore::open(&config, &DatabaseOptions::default()).unwrap());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn postgres_from_a_multi_thread_runtime() {
    let Some(pg) = new_postgres("async") else {
        eprintln!("REFORGED_TEST_POSTGRES_URL is not set: skipping the Postgres half");
        return;
    };
    let store = SqlGuardStore::open(
        &DatabaseConfig::Postgres(pg.config.clone()),
        &DatabaseOptions::default(),
    )
    .unwrap();
    assert_eq!(exercise(&store), 5);
}
