#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};

use diesel::Connection;
use diesel::connection::SimpleConnection;
use rifts_guard::{
    AlertRecord, BatchRecord, ChainRecord, ChainUpdate, CompletionMode, CompletionRecord, Disposition, EncounterState,
    FindingRecord, GENESIS_HASH, GuardStore, HostWriteRecord, SqlGuardStore, TABLES, TrafficReservation, TrafficScope,
    XP_SUMMARY_TTL_MS, XpSummaryRecord,
};
use rifts_seam::{AccountId, Actor, HostWriteKind, Json, RealmId, SessionId, UnixMillis};
use store::PgConnection;
use store::{DatabaseConfig, DatabaseOptions, PostgresConfig, portable};
use tempfile::TempDir;

fn session(seed: &str) -> SessionId {
    format!("{seed}@oregon").parse().expect("valid session id")
}

fn realm(seed: &str) -> RealmId {
    format!("{seed}@oregon").parse().expect("valid realm id")
}

fn reservation(
    id: &str,
    scope: TrafficScope,
    window_start: i64,
    expires_at: i64,
    bytes: u64,
    max_requests: u64,
    max_bytes: u64,
) -> TrafficReservation {
    TrafficReservation {
        window_id: id.to_owned(),
        scope,
        window_start: UnixMillis(window_start),
        expires_at: UnixMillis(expires_at),
        bytes,
        max_requests,
        max_bytes,
    }
}

fn open_sqlite() -> (TempDir, SqlGuardStore) {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("guard.sqlite");
    let store = SqlGuardStore::open(&DatabaseConfig::Sqlite { file }, &DatabaseOptions::default())
        .expect("open sqlite guard store");
    (dir, store)
}

#[test]
fn the_migrations_create_exactly_the_listed_tables() {
    for sql in [
        include_str!("../migrations/sqlite/2026-10-26-000002_rifts_guard/up.sql"),
        include_str!("../migrations/postgres/2026-10-26-000002_rifts_guard/up.sql"),
    ] {
        let created: Vec<&str> = sql
            .lines()
            .filter_map(|line| line.strip_prefix("CREATE TABLE "))
            .filter_map(|rest| rest.split_whitespace().next())
            .collect();
        let listed: Vec<&str> = TABLES.iter().map(|table| table.name).collect();
        assert_eq!(created, listed);
    }
}

#[test]
fn sql_guard_store_works_in_synchronous_test() {
    let (_dir, store) = open_sqlite();
    let actor = Actor::System("sync-test");
    let sess = session("01ARZ3NDEKTSV4RRFFQ69G5FAV");
    let rlm = realm("01ARZ3NDEKTSV4RRFFQ69G5FAW");

    // 1. Host writes
    let write = HostWriteRecord {
        kind: HostWriteKind::Xp,
        target: AccountId("player-1".to_owned()),
        host: AccountId("host-1".to_owned()),
        session: sess.clone(),
        generation: 1,
        realm: Some(rlm.clone()),
        session_kind: Some("REALM_HUB".to_owned()),
        hub_kind: Some("RAMSGATE".to_owned()),
        summary: Json(r#"{"xp":120}"#.to_owned()),
        at: UnixMillis(1_000),
    };
    store.record_host_write(&actor, &write).expect("record host write");
    assert_eq!(store.host_writes().expect("host writes"), vec![write]);

    // 2. Traffic reservation and rollback on session rejection
    let host_res = reservation("host:60000:h1", TrafficScope::Host, 60_000, 180_000, 10, 2, 100);
    let sess_a = reservation("session:60000:s1", TrafficScope::Session, 60_000, 180_000, 10, 1, 100);
    let sess_b = reservation("session:60000:s2", TrafficScope::Session, 60_000, 180_000, 10, 1, 100);
    assert!(store.reserve_traffic(&actor, &host_res, &sess_a).expect("first pair"));
    assert!(
        !store
            .reserve_traffic(&actor, &host_res, &sess_a)
            .expect("rejected session")
    );
    assert!(
        store
            .reserve_traffic(&actor, &host_res, &sess_b)
            .expect("second session")
    );
    let windows = store.windows().expect("windows");
    let host_win = windows
        .iter()
        .find(|w| w.scope == TrafficScope::Host)
        .expect("host window");
    assert_eq!((host_win.requests, host_win.bytes), (2, 20));

    // 3. Chain optimistic concurrency, batch dedup, and findings ordering
    assert_eq!(store.chain(&sess, 1).expect("empty chain"), None);
    let mut chain = ChainRecord::new(sess.clone(), 1, Some(CompletionMode::EndOfHunt));
    chain.revision = 1;
    chain.last_sequence = 2;
    chain.head_hash = "hash-1".to_owned();
    chain.encounter_state = EncounterState::EncounterActive;
    chain.disposition = Disposition::ObserveFlagged;
    chain.finding_count = 2;
    chain.batch_count = 1;
    chain.event_count = 2;
    chain.event_bytes = 64;
    chain.last_monotonic_ms = Some(500);
    chain.event_types.insert("DAMAGE".to_owned(), 2);
    chain.has_unattributed_damage = true;
    chain.last_received_at = Some(UnixMillis(2_000));

    let batch = BatchRecord {
        session: sess.clone(),
        generation: 1,
        batch_id: "batch-1".to_owned(),
        previous_hash: GENESIS_HASH.to_owned(),
        batch_hash: "hash-1".to_owned(),
        first_sequence: 1,
        last_sequence: 2,
        event_count: 2,
        event_bytes: 64,
        event_types: vec!["SESSION_START".to_owned(), "DAMAGE".to_owned()],
        received_at: UnixMillis(2_000),
    };
    let finding_1 = FindingRecord {
        session: sess.clone(),
        generation: 1,
        sequence: Some(1),
        code: "UNATTRIBUTED_DAMAGE".to_owned(),
        detail: "first finding".to_owned(),
        at: UnixMillis(2_000),
    };
    let finding_2 = FindingRecord {
        session: sess.clone(),
        generation: 1,
        sequence: None,
        code: "LATE_EVENT".to_owned(),
        detail: "second finding".to_owned(),
        at: UnixMillis(2_001),
    };
    let update = ChainUpdate {
        chain: chain.clone(),
        batch: Some(batch.clone()),
        findings: vec![finding_1.clone(), finding_2.clone()],
    };
    assert!(!store.apply_chain(&actor, &update, 1).expect("wrong initial revision"));
    assert!(store.apply_chain(&actor, &update, 0).expect("apply initial chain"));
    assert_eq!(store.chain(&sess, 1).expect("loaded chain"), Some(chain.clone()));
    assert_eq!(store.batch(&sess, 1, "batch-1").expect("loaded batch"), Some(batch));
    assert_eq!(
        store.findings(&sess, 1).expect("loaded findings"),
        vec![finding_1, finding_2]
    );

    // 4. XP summary upsert and purge
    let summary = XpSummaryRecord {
        session: sess.clone(),
        member: AccountId("player-1".to_owned()),
        host: AccountId("host-1".to_owned()),
        realm: Some(rlm),
        minutes: 15,
        grants: Json(r#"{"weapon":300}"#.to_owned()),
        flagged: false,
        created_at: UnixMillis(5_000),
    };
    store.put_xp_summary(&actor, &summary).expect("put xp summary");
    assert_eq!(
        store
            .xp_summary(&sess, &AccountId("player-1".to_owned()))
            .expect("get xp summary"),
        Some(summary)
    );

    // 5. Alerts insert and acknowledge
    let alert = AlertRecord {
        alert_id: "alert-1".to_owned(),
        code: "REWARD_INVARIANT".to_owned(),
        severity: "critical".to_owned(),
        message: "invariant tripped".to_owned(),
        details: Json(r#"{"count":1}"#.to_owned()),
        created_at: UnixMillis(3_000),
        acknowledged_at: None,
        acknowledged_by: None,
    };
    store.insert_alert(&actor, &alert).expect("insert alert");
    let admin = Actor::Admin {
        key_id: "ops-key".to_owned(),
    };
    assert!(
        store
            .acknowledge_alert(&admin, "alert-1", UnixMillis(4_000))
            .expect("ack alert")
    );
    assert!(
        !store
            .acknowledge_alert(&admin, "alert-1", UnixMillis(4_500))
            .expect("second ack")
    );
    let stored_alert = store.alert("alert-1").expect("load alert").expect("alert exists");
    assert_eq!(stored_alert.acknowledged_at, Some(UnixMillis(4_000)));
    assert_eq!(stored_alert.acknowledged_by.as_deref(), Some("ops-key"));
}

async fn run_suite(store: SqlGuardStore) {
    let actor = Actor::System("async-suite");
    let sess = session("01ARZ3NDEKTSV4RRFFQ69G5FAV");
    let sess_other = session("01ARZ3NDEKTSV4RRFFQ69G5FAZ");
    let rlm = realm("01ARZ3NDEKTSV4RRFFQ69G5FAW");

    // Host writes preserve insertion order and nullable fields
    let w1 = HostWriteRecord {
        kind: HostWriteKind::Mastery,
        target: AccountId("target-a".to_owned()),
        host: AccountId("host-a".to_owned()),
        session: sess.clone(),
        generation: 1,
        realm: None,
        session_kind: None,
        hub_kind: None,
        summary: Json(r#"{"delta":5}"#.to_owned()),
        at: UnixMillis(100),
    };
    let w2 = HostWriteRecord {
        kind: HostWriteKind::Reforge,
        target: AccountId("target-b".to_owned()),
        host: AccountId("host-a".to_owned()),
        session: sess.clone(),
        generation: 2,
        realm: Some(rlm.clone()),
        session_kind: Some("REALM_HUB".to_owned()),
        hub_kind: Some("RAMSGATE".to_owned()),
        summary: Json(r#"{"skillTree":"sword"}"#.to_owned()),
        at: UnixMillis(200),
    };
    store.record_host_write(&actor, &w1).expect("w1");
    store.record_host_write(&actor, &w2).expect("w2");
    assert_eq!(store.host_writes().expect("host_writes"), vec![w1, w2]);

    // Traffic windows: expired windows purged on reserve, rejected session rolls back host
    let expired_host = reservation("host:0:expired", TrafficScope::Host, 0, 10_000, 50, 1, 50);
    let expired_sess = reservation("session:0:expired", TrafficScope::Session, 0, 10_000, 50, 1, 50);
    assert!(
        store
            .reserve_traffic(&actor, &expired_host, &expired_sess)
            .expect("seed expired")
    );

    let host_w = reservation("host:60000:active", TrafficScope::Host, 60_000, 180_000, 30, 2, 60);
    let sess_w1 = reservation("session:60000:s1", TrafficScope::Session, 60_000, 180_000, 30, 1, 30);
    let sess_w2 = reservation("session:60000:s2", TrafficScope::Session, 60_000, 180_000, 30, 1, 30);
    assert!(store.reserve_traffic(&actor, &host_w, &sess_w1).expect("reserve 1"));
    // sess_w1 is now full (1/1 requests, 30/30 bytes); second attempt must fail and leave host_w at 1 request
    assert!(
        !store
            .reserve_traffic(&actor, &host_w, &sess_w1)
            .expect("reserve 1 again")
    );
    // sess_w2 succeeds because host_w still has 1 request and 30 bytes remaining
    assert!(store.reserve_traffic(&actor, &host_w, &sess_w2).expect("reserve 2"));
    // Now host_w is at 2/2 requests; a third session must fail
    let sess_w3 = reservation("session:60000:s3", TrafficScope::Session, 60_000, 180_000, 10, 5, 100);
    assert!(!store.reserve_traffic(&actor, &host_w, &sess_w3).expect("host full"));
    let active_windows = store.windows().expect("active windows");
    assert_eq!(active_windows.len(), 3);
    assert!(!active_windows.iter().any(|w| w.window_id.contains("expired")));

    // Chain optimistic concurrency, batch dedup, and completion update
    let mut c1 = ChainRecord::new(sess.clone(), 3, Some(CompletionMode::FirstDefeat));
    c1.revision = 1;
    c1.last_sequence = 1;
    c1.head_hash = "h1".to_owned();
    c1.encounter_state = EncounterState::SessionStarted;
    c1.batch_count = 1;
    c1.event_count = 1;
    c1.event_bytes = 32;
    c1.event_types = BTreeMap::from([("SESSION_START".to_owned(), 1)]);
    c1.last_received_at = Some(UnixMillis(10_000));

    let b1 = BatchRecord {
        session: sess.clone(),
        generation: 3,
        batch_id: "b-1".to_owned(),
        previous_hash: GENESIS_HASH.to_owned(),
        batch_hash: "h1".to_owned(),
        first_sequence: 1,
        last_sequence: 1,
        event_count: 1,
        event_bytes: 32,
        event_types: vec!["SESSION_START".to_owned()],
        received_at: UnixMillis(10_000),
    };
    let f1 = FindingRecord {
        session: sess.clone(),
        generation: 3,
        sequence: Some(1),
        code: "CODE_A".to_owned(),
        detail: "first".to_owned(),
        at: UnixMillis(10_000),
    };
    assert!(
        store
            .apply_chain(
                &actor,
                &ChainUpdate {
                    chain: c1.clone(),
                    batch: Some(b1.clone()),
                    findings: vec![f1.clone()],
                },
                0,
            )
            .expect("apply c1")
    );

    // Duplicate batch_id at expected_revision = 1 is rejected and does not mutate chain or findings
    let mut c_dup = c1.clone();
    c_dup.revision = 2;
    let f_dup = FindingRecord {
        session: sess.clone(),
        generation: 3,
        sequence: Some(2),
        code: "CODE_DUP".to_owned(),
        detail: "should not be stored".to_owned(),
        at: UnixMillis(10_500),
    };
    assert!(
        !store
            .apply_chain(
                &actor,
                &ChainUpdate {
                    chain: c_dup,
                    batch: Some(b1.clone()),
                    findings: vec![f_dup],
                },
                1,
            )
            .expect("reject duplicate batch_id")
    );
    assert_eq!(store.chain(&sess, 3).expect("chain still rev 1"), Some(c1.clone()));
    assert_eq!(store.findings(&sess, 3).expect("findings unchanged"), vec![f1.clone()]);

    // Completion update at expected_revision = 1 succeeds
    let mut c2 = c1.clone();
    c2.revision = 2;
    c2.encounter_state = EncounterState::CompletionRequested;
    c2.disposition = Disposition::ObserveFlagged;
    c2.finding_count = 2;
    c2.completion = Some(CompletionRecord {
        final_sequence: 1,
        final_hash: "h1".to_owned(),
        at: UnixMillis(11_000),
    });
    let f2 = FindingRecord {
        session: sess.clone(),
        generation: 3,
        sequence: None,
        code: "CODE_B".to_owned(),
        detail: "second".to_owned(),
        at: UnixMillis(11_000),
    };
    assert!(
        store
            .apply_chain(
                &actor,
                &ChainUpdate {
                    chain: c2.clone(),
                    batch: None,
                    findings: vec![f2.clone()],
                },
                1,
            )
            .expect("apply completion")
    );
    assert_eq!(store.chain(&sess, 3).expect("chain rev 2"), Some(c2));
    assert_eq!(store.findings(&sess, 3).expect("both findings"), vec![f1, f2]);
    assert_eq!(store.batch(&sess, 3, "b-1").expect("batch b-1"), Some(b1));
    assert_eq!(store.batch(&sess, 3, "missing").expect("missing batch"), None);

    // XP summary upsert overwrites existing (session, member) and purge_expired cleans up expired rows
    let xp_initial = XpSummaryRecord {
        session: sess.clone(),
        member: AccountId("member-1".to_owned()),
        host: AccountId("host-a".to_owned()),
        realm: Some(rlm.clone()),
        minutes: 10,
        grants: Json(r#"{"xp":100}"#.to_owned()),
        flagged: false,
        created_at: UnixMillis(1_000),
    };
    let xp_updated = XpSummaryRecord {
        minutes: 25,
        grants: Json(r#"{"xp":250}"#.to_owned()),
        flagged: true,
        created_at: UnixMillis(2_000),
        ..xp_initial.clone()
    };
    let xp_other = XpSummaryRecord {
        session: sess_other.clone(),
        member: AccountId("member-2".to_owned()),
        host: AccountId("host-a".to_owned()),
        realm: None,
        minutes: 5,
        grants: Json(r#"{"xp":50}"#.to_owned()),
        flagged: false,
        created_at: UnixMillis(100_000),
    };
    store.put_xp_summary(&actor, &xp_initial).expect("put xp initial");
    store.put_xp_summary(&actor, &xp_updated).expect("put xp updated");
    store.put_xp_summary(&actor, &xp_other).expect("put xp other");
    assert_eq!(
        store
            .xp_summary(&sess, &AccountId("member-1".to_owned()))
            .expect("updated xp"),
        Some(xp_updated)
    );

    // Purge at 2_000 + XP_SUMMARY_TTL_MS purges xp_updated and the 3 traffic windows (expires_at = 180_000),
    // while keeping xp_other (created_at = 100_000)
    let purged = store
        .purge_expired(UnixMillis(2_000 + XP_SUMMARY_TTL_MS))
        .expect("purge expired");
    assert_eq!(purged, 4);
    assert_eq!(
        store
            .xp_summary(&sess, &AccountId("member-1".to_owned()))
            .expect("purged xp"),
        None
    );
    assert_eq!(
        store
            .xp_summary(&sess_other, &AccountId("member-2".to_owned()))
            .expect("kept xp"),
        Some(xp_other)
    );
    assert!(store.windows().expect("windows purged").is_empty());

    // Alert insert $setOnInsert semantics and single acknowledgement
    let alert_1 = AlertRecord {
        alert_id: "alert-100".to_owned(),
        code: "REWARD_INVARIANT".to_owned(),
        severity: "critical".to_owned(),
        message: "original message".to_owned(),
        details: Json(r#"{"v":1}"#.to_owned()),
        created_at: UnixMillis(50_000),
        acknowledged_at: None,
        acknowledged_by: None,
    };
    let alert_retry = AlertRecord {
        message: "overwritten message".to_owned(),
        details: Json(r#"{"v":2}"#.to_owned()),
        ..alert_1.clone()
    };
    store.insert_alert(&actor, &alert_1).expect("insert alert 1");
    store.insert_alert(&actor, &alert_retry).expect("insert alert retry");
    let loaded = store.alert("alert-100").expect("alert").expect("present");
    assert_eq!(loaded.message, "original message");
    assert_eq!(loaded.details, Json(r#"{"v":1}"#.to_owned()));

    let acct_actor = Actor::Account(AccountId("moderator-1".to_owned()));
    assert!(
        store
            .acknowledge_alert(&acct_actor, "alert-100", UnixMillis(60_000))
            .expect("ack once")
    );
    assert!(
        !store
            .acknowledge_alert(&acct_actor, "alert-100", UnixMillis(61_000))
            .expect("ack twice")
    );
    assert!(
        !store
            .acknowledge_alert(&acct_actor, "nonexistent", UnixMillis(61_000))
            .expect("ack missing")
    );
    let acked = store.alert("alert-100").expect("alert").expect("present");
    assert_eq!(acked.acknowledged_at, Some(UnixMillis(60_000)));
    assert_eq!(acked.acknowledged_by.as_deref(), Some("moderator-1"));
}

static NEXT: AtomicU32 = AtomicU32::new(0);

#[tokio::test(flavor = "multi_thread")]
async fn sql_guard_store_suite_runs_on_sqlite_and_postgres() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sqlite_config = DatabaseConfig::Sqlite {
        file: dir.path().join("guard.sqlite"),
    };
    let sqlite_store =
        SqlGuardStore::open(&sqlite_config, &DatabaseOptions::default()).expect("open fresh sqlite guard store");
    run_suite(sqlite_store).await;
    let reopened_sqlite =
        SqlGuardStore::open(&sqlite_config, &DatabaseOptions::default()).expect("reopen existing sqlite guard store");
    assert!(
        reopened_sqlite.alert("alert-100").expect("alert on reopen").is_some(),
        "reopening an already-migrated sqlite database preserves guard rows"
    );

    if let Ok(base_url) = std::env::var("REFORGED_TEST_POSTGRES_URL") {
        let url = url::Url::parse(&base_url).expect("parse REFORGED_TEST_POSTGRES_URL");
        let server = PostgresConfig {
            host: url.host_str().unwrap_or("127.0.0.1").to_owned(),
            port: url.port().unwrap_or(5432),
            user: url.username().to_owned(),
            database: url.path().trim_start_matches('/').to_owned(),
            password: url.password().unwrap_or_default().to_owned(),
            allow_remote: false,
        };
        let name = format!(
            "rifts_guard_test_{}_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.subsec_nanos())
        );
        let mut admin = PgConnection::establish(&server.conninfo()).expect("connect postgres admin");
        admin
            .batch_execute(&format!("CREATE DATABASE {}", portable::quote_ident(&name)))
            .expect("create test database");
        let config = PostgresConfig {
            database: name.clone(),
            ..server.clone()
        };
        let pg_config = DatabaseConfig::Postgres(config);
        let pg_store = SqlGuardStore::open(&pg_config, &DatabaseOptions::default()).expect("open postgres guard store");
        run_suite(pg_store).await;
        let reopened_pg =
            SqlGuardStore::open(&pg_config, &DatabaseOptions::default()).expect("reopen existing postgres guard store");
        assert!(
            reopened_pg.alert("alert-100").expect("alert on reopen").is_some(),
            "reopening an already-migrated postgres database preserves guard rows"
        );
        drop(admin);
        if let Ok(mut cleanup) = PgConnection::establish(&server.conninfo()) {
            let _ = cleanup.batch_execute(&format!(
                "DROP DATABASE IF EXISTS {} WITH (FORCE)",
                portable::quote_ident(&name)
            ));
        }
    }
}
