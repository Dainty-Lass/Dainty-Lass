// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! migrate and verify against the mock server (spec §16.2 D5 to D10, D13 to D15), on SQLite.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::BTreeSet;

use rustic_isles_db::fuzzing::{read_target, rows};
use rustic_isles_db::ids::{derived_account_id, is_instance_id};
use rustic_isles_db::mongo::{MockMongoServer, MongoClient, MongoUri};
use rustic_isles_db::{Area, DbError, run_migrate, run_verify, verify_report};
use rustic_isles_store::repo::auth::{ApprovalStatus, EmailCodePurpose};
use rustic_isles_store::repo::characters::slots::MAX_TOTAL_LOADOUT_SLOTS;
use rustic_isles_store::{AccountId, Repositories, UserId};
use serde_json::{Value, json};

fn server() -> MockMongoServer {
    // One document a batch: every collection is paged through getMore
    let server = MockMongoServer::start(1).unwrap();
    common::seed(&server);
    server
}

fn s(v: &Value) -> &str {
    v.as_str().unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn dry_run_counts_without_creating_the_target() {
    let server = server();
    let dir = tempfile::tempdir().unwrap();
    let file = common::sqlite(dir.path());
    let mut opts = common::migrate_options(&server, file.to_str().unwrap());
    opts.dry_run = true;
    let dry = run_migrate(&opts).await.unwrap();
    assert!(dry.dry_run);
    assert_eq!(dry.rb11_launcher_approvals_backfilled, 1);
    assert!(!file.exists());
    assert_eq!(dry.counts.len(), 37);
    opts.dry_run = false;
    let real = run_migrate(&opts).await.unwrap();
    assert_eq!(real.counts, dry.counts);
}

#[tokio::test(flavor = "multi_thread")]
async fn migrate_counts_and_logs_under_system_migrate() {
    let server = server();
    let dir = tempfile::tempdir().unwrap();
    let file = common::sqlite(dir.path());
    let report = run_migrate(&common::migrate_options(&server, file.to_str().unwrap()))
        .await
        .unwrap();
    let c = |t: &str| report.counts[t];
    assert_eq!(
        (
            c("accounts"),
            c("launcher_accounts"),
            c("characters"),
            c("loadouts"),
            c("inventories")
        ),
        (2, 2, 1, 1, 1)
    );
    assert_eq!((c("parties"), c("guard_findings"), c("launcher_notices")), (1, 1, 1));
    assert_eq!(c("entitlements"), 2);
    assert_eq!(c("economy_rollup_counters"), 2);
    assert_eq!(report.rb11_inventory_arrays_transformed, 1);
    assert_eq!(report.rb11_starter_instances_repaired, 1);
    assert_eq!(report.rb11_loadout_slots_reconciled, 1);
    assert_eq!(report.skipped_invalid, 0);

    let db = common::open(&file);
    let changes = db.changes(0, 100_000).await.unwrap();
    let written: Vec<_> = changes.iter().filter(|e| e.op != "migrate").collect();
    assert!(!written.is_empty());
    assert!(written.iter().all(|e| e.actor == "system:migrate"));
    assert!(db.verify(None).await.unwrap().broken.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn rows_read_back_through_the_repositories() {
    let server = server();
    let dir = tempfile::tempdir().unwrap();
    let file = common::sqlite(dir.path());
    run_migrate(&common::migrate_options(&server, file.to_str().unwrap()))
        .await
        .unwrap();
    let db = common::open(&file);
    let repos = Repositories::on_store();
    let u1: AccountId = derived_account_id("local", "u1").parse().unwrap();
    let u2: AccountId = derived_account_id("local", "u2").parse().unwrap();
    let (a1, l1, l2, identity, session, code, gs, uk) = db
        .read(move |conn| {
            Ok::<_, rustic_isles_store::RepoError>((
                repos.accounts.find_by_user_id(conn, &UserId::new("u1"))?,
                repos.launcher_accounts.find_by_account(conn, &u1)?,
                repos.launcher_accounts.find_by_account(conn, &u2)?,
                repos.auth_identities.find_by_provider_subject(conn, "d-1")?,
                repos.refresh_sessions.find_by_token_hash(conn, "token-hash-1")?,
                repos
                    .email_codes
                    .peek_live(conn, &u1, EmailCodePurpose::Verify, "2026-06-01T00:00:00.000Z", 5)?,
                repos.api_keys.find_all_game_server_key_hashes(conn)?,
                repos.api_keys.find_all_user_key_hashes(conn)?,
            ))
        })
        .await
        .unwrap();
    let a1 = a1.unwrap();
    assert_eq!((a1.name.as_str(), a1.notes), ("Alice", 3));
    let l1 = l1.unwrap();
    assert_eq!(l1.approval_status, Some(ApprovalStatus::Approved));
    assert_eq!(l1.approval_updated_by.as_deref(), Some("migration:0.1.73"));
    assert_eq!(l1.approval_updated_at.as_deref(), Some("2026-02-01T00:00:00.000Z"));
    assert_eq!(l1.roles, vec!["user".to_owned(), "admin".to_owned()]);
    let l2 = l2.unwrap();
    assert_eq!(l2.approval_status, Some(ApprovalStatus::Pending));
    assert_eq!(l2.approval_updated_at.as_deref(), Some("2026-02-25T06:13:20.000Z"));
    assert_eq!(l2.created_at, "2026-02-02T00:00:00.000Z");
    assert_eq!(identity.unwrap().provider_username, "alice#1");
    assert_eq!(session.unwrap().device_name, "Launcher");
    assert_eq!(code.unwrap().attempts, 2);
    assert!(gs.iter().any(|k| k.key_hash.as_deref() == Some("gs-key-1")));
    assert!(uk.iter().any(|k| k.key_hash == "user-key-1"));
}

#[tokio::test(flavor = "multi_thread")]
async fn rb11_repairs_slots_starter_ids_and_storage() {
    let server = server();
    let dir = tempfile::tempdir().unwrap();
    let file = common::sqlite(dir.path());
    let data = dir.path().join("game");
    std::fs::create_dir_all(data.join("data")).unwrap();
    std::fs::write(data.join("inventory_storage_policy.json"), "not json").unwrap();
    std::fs::write(
        data.join("data").join("inventory_storage_policy.json"),
        r#"{"schema":1,"entries":{"STACK_ME":{"storage":"stacked","maxQuantity":99}}}"#,
    )
    .unwrap();
    let mut opts = common::migrate_options(&server, file.to_str().unwrap());
    opts.game_data_dir = Some(data);
    run_migrate(&opts).await.unwrap();
    let state = read_target(file.to_str().unwrap()).await.unwrap();

    let loadout = rows(&state, "loadouts")[0].clone();
    assert_eq!(loadout["unlocked_total_slots"], 3);
    assert_eq!(loadout["active_index"], 2);
    let slots: Value = serde_json::from_str(s(&loadout["slots_json"])).unwrap();
    assert_eq!(slots.as_array().unwrap().len(), 3);
    assert_eq!(slots[2]["slot_index"], 2);
    assert_eq!(slots[2]["custom_name"], "");

    let inventory = rows(&state, "inventories")[0].clone();
    let instanced: Value = serde_json::from_str(s(&inventory["instanced_items"])).unwrap();
    let weapon = instanced
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["catalogId"] == "WEAPON_STARTER")
        .unwrap();
    assert!(is_instance_id(s(&weapon["instanceId"])));
    assert_eq!(slots[0]["weapon"]["instance_id"], weapon["instanceId"]);
    // The banner and glider are seeded and the loadout points at them
    let banner = instanced
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["catalogId"] == "BANNER_STARTER")
        .unwrap();
    let persistent: Value = serde_json::from_str(s(&loadout["persistent"])).unwrap();
    assert_eq!(persistent["banner"]["fabric_id"], banner["instanceId"]);
    assert_eq!(persistent["glider"]["PrimaryDyeId"], "DYE_DEFAULT");
    // The policy says stacked: it moved
    assert!(
        !instanced
            .as_array()
            .unwrap()
            .iter()
            .any(|i| i["catalogId"] == "STACK_ME")
    );
    let stacked: Value = serde_json::from_str(s(&inventory["stacked_items"])).unwrap();
    assert!(stacked.as_array().unwrap().iter().any(|i| i["catalogId"] == "STACK_ME"));
}

#[tokio::test(flavor = "multi_thread")]
async fn every_area_reads_back() {
    let server = server();
    let dir = tempfile::tempdir().unwrap();
    let file = common::sqlite(dir.path());
    run_migrate(&common::migrate_options(&server, file.to_str().unwrap()))
        .await
        .unwrap();
    let state = read_target(file.to_str().unwrap()).await.unwrap();
    let one = |t: &str| {
        let r = rows(&state, t);
        assert_eq!(r.len(), 1, "{t}");
        r[0].clone()
    };
    let u1 = derived_account_id("local", "u1");
    assert_eq!(one("bounty_states")["update_version"], 2);
    assert_eq!(one("cooldown_states")["entries"], r#"{"X":1}"#);
    let entitlements = rows(&state, "entitlements");
    assert_eq!(entitlements[0]["duration"], json!(30.0));
    assert_eq!(entitlements[1]["duration"], "inf");
    let wallet = one("wallets");
    // Ordinals keep the skipped entry's gap
    assert_eq!(wallet["balances"], json!([["CUR_A", 100.0, 0], ["CUR_B", 2.5, 2]]));
    let counters = rows(&state, "economy_rollup_counters");
    assert_eq!(counters[0]["path"], "earned.bounties");
    assert_eq!(counters[1]["value"], json!(3.0));
    assert_eq!(counters[1]["ttl_at"], 1_772_400_000_000_i64);
    let track = one("progression_tracks");
    assert_eq!(
        (track["progress"].clone(), track["prestige_claimed"].clone()),
        (json!(120), json!(1))
    );
    assert_eq!(one("progression_objectives")["value"], 7);
    assert_eq!(one("progression_objective_events")["raw_body"], "{ \"a\": 1 }");
    let grant: Value = serde_json::from_str(s(&one("progression_grants")["result"])).unwrap();
    assert_eq!(grant["userId"], u1.as_str());
    assert_eq!(one("escalation_progress")["talents_progress"], "[1,2]");
    assert_eq!(one("player_journey")["update_version"], 4);
    assert_eq!(one("breadcrumbs")["items"], r#"["b"]"#);
    assert_eq!(one("encountered_content")["content"], r#"["x"]"#);
    let party = one("parties");
    let members = party["members"].as_array().unwrap();
    assert_eq!(members.len(), 2);
    assert_eq!(members[1][1], 2);
    assert_eq!(members[1][2], "2026-02-05T00:00:00.000Z");
    assert_eq!(one("party_invites")["party"], party["id"]);
    assert_eq!(one("party_outbox")["party"], "p1");
    let friend = one("friend_edges");
    assert_eq!(
        (friend["favorite"].clone(), friend["other_user_id"].clone()),
        (json!(true), json!("u2"))
    );
    assert_eq!(one("reforged_invites")["uses_remaining"], 3);
    assert_eq!(one("admin_sessions")["ttl_at"], 1_772_359_200_000_i64);
    assert_eq!(one("admin_audit")["old_state"], r#"{"x":1}"#);
    assert_eq!(one("launcher_notices")["version"], 4);
    assert_eq!(one("operations_policy")["guard_enforcement"], "OBSERVE");
    let guard = one("launcher_guard_sessions");
    assert_eq!(
        (guard["role"].clone(), guard["risk_score"].clone()),
        (json!("host"), json!(0.25))
    );
    assert_eq!(
        guard["detail"],
        r#"{"launcherVersion":"","lastSignals":null,"reasons":["r"],"history":[]}"#
    );
    let finding = one("guard_findings");
    assert_eq!(finding["account_ids"], json!([u1]).to_string());
    assert_eq!(finding["account_count"], 1);
    let transaction = one("inventory_transactions");
    assert_eq!(transaction["extra"], r#"{"__v":0,"updatedAt":"2026-02-04T00:00:00Z"}"#);
    assert_eq!(one("email_codes")["purpose"], "verify");
    assert!(i64::from(MAX_TOTAL_LOADOUT_SLOTS) >= one("loadouts")["unlocked_total_slots"].as_i64().unwrap());
}

#[tokio::test(flavor = "multi_thread")]
async fn verify_passes_twice_and_catches_a_change() {
    let server = server();
    let dir = tempfile::tempdir().unwrap();
    let file = common::sqlite(dir.path());
    let opts = common::migrate_options(&server, file.to_str().unwrap());
    let first = run_migrate(&opts).await.unwrap();
    let report = run_verify(&opts).await.unwrap();
    assert!(report.ok);
    assert_eq!(report.collections.len(), 37);
    let again = run_migrate(&opts).await.unwrap();
    assert_eq!(again.counts, first.counts);
    assert!(run_verify(&opts).await.unwrap().ok);

    let db = common::open(&file);
    db.write(store::Actor::Host("test".into()), |tx| {
        use diesel::RunQueryDsl as _;
        diesel::sql_query("UPDATE characters SET name = 'Changed'").execute(tx.conn())?;
        Ok::<_, store::Error>(())
    })
    .await
    .unwrap();
    assert!(matches!(run_verify(&opts).await, Err(DbError::VerificationFailed(1))));
    let report = verify_report(&opts).await.unwrap();
    let characters = report.collections.iter().find(|c| c.name == "characters").unwrap();
    assert!(!characters.ok);
    assert!(characters.mismatches[0].starts_with("content hash mismatch for c1: expected "));
}

#[tokio::test(flavor = "multi_thread")]
async fn existing_ids_and_names_survive_later_runs() {
    let server = server();
    let dir = tempfile::tempdir().unwrap();
    let file = common::sqlite(dir.path());
    // An account the target already has keeps its id (D5)
    let existing = AccountId::new(&"local".parse().unwrap());
    let db = common::open(&file);
    let record = rustic_isles_store::repo::auth::AccountRecord {
        id: existing.clone(),
        user_id: UserId::new("u1"),
        name: "Before".into(),
        notes: 0,
    };
    db.write(store::Actor::Host("test".into()), move |tx| {
        Repositories::on_store().accounts.create(tx, &record)
    })
    .await
    .unwrap();
    drop(db);
    let mut opts = common::migrate_options(&server, file.to_str().unwrap());
    run_migrate(&opts).await.unwrap();
    let state = read_target(file.to_str().unwrap()).await.unwrap();
    let accounts = rows(&state, "accounts");
    let alice = accounts.iter().find(|a| a["user_id"] == "u1").unwrap();
    assert_eq!(alice["id"], existing.to_string());
    assert_eq!(alice["name"], "Alice");
    // A run without auth leaves names alone: placeholders are insert-only (coordinator item 3)
    opts.areas = [Area::Characters].into_iter().collect::<BTreeSet<_>>();
    run_migrate(&opts).await.unwrap();
    let state = read_target(file.to_str().unwrap()).await.unwrap();
    let alice = rows(&state, "accounts")
        .into_iter()
        .find(|a| a["user_id"] == "u1")
        .unwrap()
        .clone();
    assert_eq!(
        (alice["name"].clone(), alice["notes"].clone()),
        (json!("Alice"), json!(3))
    );
    assert!(run_verify(&opts).await.unwrap().ok);
}

#[tokio::test(flavor = "multi_thread")]
async fn invalid_documents_stop_the_run_unless_skipped() {
    let server = server();
    server.seed("characters", vec![json!({"characterId": "c9", "name": "No owner"})]);
    let dir = tempfile::tempdir().unwrap();
    let file = common::sqlite(dir.path());
    let mut opts = common::migrate_options(&server, file.to_str().unwrap());
    let err = run_migrate(&opts).await.unwrap_err();
    assert_eq!(err.to_string(), "invalid document in characters: missing userId");
    opts.skip_invalid = true;
    let report = run_migrate(&opts).await.unwrap();
    assert_eq!(report.skipped_invalid, 1);
    assert_eq!(report.counts["characters"], 1);
    // An empty user id is fatal whatever the flag (P2)
    server.seed("bountyStates", vec![json!({"userId": "  "})]);
    assert_eq!(
        run_migrate(&opts).await.unwrap_err().to_string(),
        "invalid argument: empty userId"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_sqlite_files_are_refused_by_verify_and_export() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("typo.db");
    // A closed port: nothing may be contacted before the refusal
    let mut opts = common::migrate_options(&MockMongoServer::start(1).unwrap(), missing.to_str().unwrap());
    opts.from_uri = "mongodb://127.0.0.1:1/dauntless".into();
    let err = run_verify(&opts).await.unwrap_err().to_string();
    assert!(err.contains("does not exist"), "{err}");
    let export = rustic_isles_db::ExportOptions {
        from_source: missing.to_str().unwrap().into(),
        to_uri: "mongodb://127.0.0.1:1/dauntless".into(),
        areas: BTreeSet::new(),
        dry_run: false,
        allow_remote_postgres: false,
    };
    let err = rustic_isles_db::run_export_mongo(&export)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.starts_with("invalid argument: the database "), "{err}");
    assert!(!missing.exists());
}

#[test]
fn probe_answers_whether_a_collection_has_documents() {
    let server = MockMongoServer::start(10).unwrap();
    server.seed("full", vec![json!({"a": 1}), json!({"a": 2})]);
    server.seed("empty", Vec::new());
    let mut client = MongoClient::connect(&MongoUri::parse(&server.uri("dauntless")).unwrap()).unwrap();
    assert_eq!(client.database(), "dauntless");
    assert!(client.collection_has_documents("full").unwrap());
    assert!(!client.collection_has_documents("empty").unwrap());
    assert!(!client.collection_has_documents("missing").unwrap());
    assert_eq!(client.find_all("full").unwrap().len(), 2);
}

#[test]
fn credentials_fail_against_the_mock() {
    let server = MockMongoServer::start(10).unwrap();
    let uri = server.uri("dauntless").replace("mongodb://", "mongodb://u:p@");
    let err = MongoClient::connect(&MongoUri::parse(&uri).unwrap())
        .err()
        .unwrap()
        .to_string();
    assert_eq!(err, "MongoDB command error: saslStart: unsupported command");
}
