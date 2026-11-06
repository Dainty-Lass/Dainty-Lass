// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! export-mongo's round trip (spec §11.3, §16.2 D11) and the opt-in rehearsal on real servers (D12).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use diesel::Connection as _;
use diesel::connection::SimpleConnection as _;
use rustic_isles_db::mongo::{MockMongoServer, MongoUri, MongoWriter};
use rustic_isles_db::{Area, run_export_mongo, run_migrate, run_verify};
use rustic_isles_store::repo::auth::{
    AccountRecord, ApprovalStatus, CreateOutcome, LauncherAccountRecord, OperationalStatus,
};
use rustic_isles_store::{AccountId, Repositories, UserId};
use serde_json::{Value, json};

/// A player registered through the launcher after the migration, written by the store's own repositories.
async fn register(file: &std::path::Path) {
    let db = common::open(file);
    let account = AccountId::new(&"local".parse().unwrap());
    let record = AccountRecord {
        id: account.clone(),
        user_id: UserId::new("u3"),
        name: "Carol".into(),
        notes: 0,
    };
    let launcher = LauncherAccountRecord {
        account,
        user_id: UserId::new("u3"),
        email: Some("carol@example.test".into()),
        display_name_normalized: "carol".into(),
        display_name: "Carol".into(),
        password_hash: Some("$argon2id$y".into()),
        status: OperationalStatus::Active,
        approval_status: Some(ApprovalStatus::Pending),
        approval_updated_at: None,
        approval_updated_by: None,
        approval_reason: None,
        roles: vec!["user".into()],
        roles_updated_at: None,
        roles_updated_by: None,
        created_at: "2026-11-05T00:00:00.000Z".into(),
        last_login_at: None,
        username_set: Some(false),
        consent: None,
        deletion: None,
        email_verified_at: None,
        email_verification_pending_since: Some("2026-11-05T00:00:00.000Z".into()),
        timeout: None,
    };
    let outcome = db
        .write(store::Actor::Host("test".into()), move |tx| {
            let repos = Repositories::on_store();
            repos.accounts.create(tx, &record)?;
            repos.launcher_accounts.create(tx, &launcher)
        })
        .await
        .unwrap();
    assert_eq!(outcome, CreateOutcome::Created);
}

fn ids(snapshot: &std::collections::BTreeMap<String, Vec<serde_json::Map<String, Value>>>, c: &str) -> Vec<Value> {
    snapshot
        .get(c)
        .into_iter()
        .flatten()
        .map(|d| d.get("_id").cloned().unwrap_or(Value::Null))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn export_round_trips_through_a_fresh_store() {
    let source = MockMongoServer::start(2).unwrap();
    common::seed(&source);
    let dir = tempfile::tempdir().unwrap();
    let first = common::sqlite(dir.path());
    run_migrate(&common::migrate_options(&source, first.to_str().unwrap()))
        .await
        .unwrap();
    register(&first).await;

    let destination = MockMongoServer::start(3).unwrap();
    let mut opts = common::export_options(&destination, first.to_str().unwrap(), &[]);
    opts.dry_run = true;
    let dry = run_export_mongo(&opts).await.unwrap();
    assert_eq!(dry.counts["accounts"], 3);
    assert_eq!(dry.counts["characters"], 1);
    assert_eq!(dry.counts["operationsPolicy"], 1);
    assert!(destination.snapshot().is_empty());

    opts.dry_run = false;
    let report = run_export_mongo(&opts).await.unwrap();
    assert_eq!(report.counts, dry.counts);
    let snapshot = destination.snapshot();
    let mut users = ids(&snapshot, "accounts");
    users.sort_by_key(ToString::to_string);
    assert_eq!(users, vec![json!("u1"), json!("u2"), json!("u3")]);
    assert_eq!(ids(&snapshot, "loadouts"), vec![json!("c1")]);
    assert_eq!(ids(&snapshot, "launcherNotices"), vec![json!("current")]);
    assert_eq!(ids(&snapshot, "operationsPolicy"), vec![json!("global")]);
    assert_eq!(ids(&snapshot, "guardFindings"), vec![Value::Null]);
    assert_eq!(ids(&snapshot, "economyDaily"), vec![json!("u1:2026-03-01:CUR_A")]);
    let daily = &snapshot["economyDaily"][0];
    assert_eq!(daily["earned"], json!({"bounties": 2.0, "hunts": 3.0}));
    let carol = snapshot["accounts"].iter().find(|d| d["_id"] == "u3").unwrap();
    assert_eq!(carol["approvalStatus"], "pending");
    assert_eq!(carol["usernameSet"], 0);
    assert!(carol.get("approvalReason").is_none());
    let invite = &snapshot["partyInvites"][0];
    assert_eq!(invite["_id"], "u2\u{0}u1");
    assert_eq!(invite["partyId"], "p1");

    // Back into a fresh store, then that store against the export: every table matches
    let second = dir.path().join("second.db");
    let back = common::migrate_options(&destination, second.to_str().unwrap());
    run_migrate(&back).await.unwrap();
    let verified = run_verify(&back).await.unwrap();
    assert_eq!(verified.collections.len(), 37);
    assert!(verified.ok);

    // An area alone replaces only its collections
    let only = MockMongoServer::start(3).unwrap();
    only.seed("characters", vec![json!({"_id": "stale"})]);
    only.seed("bountyStates", vec![json!({"_id": "kept"})]);
    run_export_mongo(&common::export_options(
        &only,
        first.to_str().unwrap(),
        &[Area::Characters],
    ))
    .await
    .unwrap();
    let snapshot = only.snapshot();
    assert_eq!(ids(&snapshot, "characters"), vec![json!("c1")]);
    assert_eq!(ids(&snapshot, "bountyStates"), vec![json!("kept")]);
}

struct PgDatabase {
    server: store::PostgresConfig,
    url: String,
}

impl Drop for PgDatabase {
    fn drop(&mut self) {
        if let Ok(mut admin) = store::PgConnection::establish(&self.server.conninfo()) {
            let name = self.url.rsplit('/').next().unwrap_or_default();
            let _ = admin.batch_execute(&format!("DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)"));
        }
    }
}

fn new_postgres() -> Option<PgDatabase> {
    let url = std::env::var("REFORGED_TEST_POSTGRES_URL").ok()?;
    let server = rustic_isles_db::parse_postgres_url(&url, false).expect("a postgres:// URL");
    let name = format!("isles_db_test_{}", std::process::id());
    store::PgConnection::establish(&server.conninfo())
        .expect("REFORGED_TEST_POSTGRES_URL is reachable")
        .batch_execute(&format!("CREATE DATABASE \"{name}\""))
        .unwrap();
    let url = format!(
        "postgres://{}:{}@{}:{}/{name}",
        server.user, server.password, server.host, server.port
    );
    Some(PgDatabase { server, url })
}

/// D12: one source into SQLite and Postgres. `REFORGED_TEST_MONGO_URI` uses a real MongoDB as the source
/// (seeded first only with `REFORGED_TEST_SEED_MONGO=1`, since that writes to it).
#[tokio::test(flavor = "multi_thread")]
async fn rehearsal_on_both_engines() {
    let Some(pg) = new_postgres() else {
        eprintln!("REFORGED_TEST_POSTGRES_URL is not set: skipping the rehearsal");
        return;
    };
    let mock = MockMongoServer::start(500).unwrap();
    let source = match std::env::var("REFORGED_TEST_MONGO_URI") {
        Ok(uri) => {
            if std::env::var("REFORGED_TEST_SEED_MONGO").as_deref() == Ok("1") {
                common::seed(&mock);
                let parsed = MongoUri::parse(&uri).unwrap();
                tokio::task::spawn_blocking(move || {
                    let mut writer = MongoWriter::connect(&parsed).unwrap();
                    for (name, docs) in mock.snapshot() {
                        writer.replace_collection(&name, &docs).unwrap();
                    }
                })
                .await
                .unwrap();
            }
            uri
        }
        Err(_) => {
            eprintln!("REFORGED_TEST_MONGO_URI is not set: the rehearsal reads the mock server");
            common::seed(&mock);
            mock.uri("dauntless")
        }
    };
    let dir = tempfile::tempdir().unwrap();
    let file = common::sqlite(dir.path());
    let mut on_sqlite = common::migrate_options(&MockMongoServer::start(1).unwrap(), file.to_str().unwrap());
    on_sqlite.from_uri = source.clone();
    let mut on_postgres = on_sqlite.clone();
    on_postgres.to_target = pg.url.clone();
    let a = run_migrate(&on_sqlite).await.unwrap();
    let b = run_migrate(&on_postgres).await.unwrap();
    assert_eq!(a.counts, b.counts);
    assert!(run_verify(&on_sqlite).await.unwrap().ok);
    assert!(run_verify(&on_postgres).await.unwrap().ok);
}
