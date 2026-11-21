// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The engines the tests run on: SQLite always, and a throwaway Postgres database when
//! `DAINTY_TEST_POSTGRES_URL` names a server the tests may create databases on (as `store`'s tests do).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    dead_code,
    reason = "shared by several test binaries, each using only some; a panic is how a test fails"
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use dainty_module_api::network::StandaloneHome;
use dainty_module_api::{Actor, ActorContext, NodeIdentity};
use dainty_store::{DatabaseConfig, DatabaseOptions, PgConnection, PostgresConfig, portable};
use diesel::Connection;
use diesel::connection::SimpleConnection;
use rustic_isles_store::repo::auth::{AccountRecord, LauncherAccountRecord, OperationalStatus};
use rustic_isles_store::{AccountId, IslesStore, UserId};

pub struct PgDatabase {
    server: PostgresConfig,
    pub config: PostgresConfig,
}

impl Drop for PgDatabase {
    fn drop(&mut self) {
        if let Ok(mut admin) = PgConnection::establish(&self.server.conninfo()) {
            let _ = admin.batch_execute(&format!(
                "DROP DATABASE IF EXISTS {} WITH (FORCE)",
                portable::quote_ident(&self.config.database)
            ));
        }
    }
}

static NEXT: AtomicU32 = AtomicU32::new(0);

fn new_postgres() -> Option<PgDatabase> {
    let url = std::env::var("DAINTY_TEST_POSTGRES_URL").ok()?;
    let url = url::Url::parse(&url).expect("DAINTY_TEST_POSTGRES_URL is a postgres:// URL");
    let server = PostgresConfig {
        host: url.host_str().unwrap_or("127.0.0.1").to_owned(),
        port: url.port().unwrap_or(5432),
        user: url.username().to_owned(),
        database: url.path().trim_start_matches('/').to_owned(),
        password: url.password().unwrap_or_default().to_owned(),
        allow_remote: false,
    };
    let name = format!(
        "isles_store_test_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    PgConnection::establish(&server.conninfo())
        .expect("DAINTY_TEST_POSTGRES_URL is reachable")
        .batch_execute(&format!("CREATE DATABASE {}", portable::quote_ident(&name)))
        .unwrap();
    let config = PostgresConfig {
        database: name,
        ..server.clone()
    };
    Some(PgDatabase { server, config })
}

/// An open store and what keeps its database alive.
pub struct Target {
    pub store: IslesStore,
    pub engine: &'static str,
    _sqlite: Option<tempfile::TempDir>,
    _postgres: Option<PgDatabase>,
}

fn open(config: &DatabaseConfig, identity: &NodeIdentity) -> IslesStore {
    let home = Arc::new(StandaloneHome {
        identity: identity.clone(),
    });
    IslesStore::open(config, &DatabaseOptions::default(), home, identity).unwrap()
}

/// SQLite, and Postgres when it is configured, both as a standalone node.
pub fn targets() -> Vec<Target> {
    let identity = NodeIdentity::standalone();
    let dir = tempfile::tempdir().unwrap();
    let config = DatabaseConfig::Sqlite {
        file: dir.path().join("isles.db"),
    };
    let mut out = vec![Target {
        store: open(&config, &identity),
        engine: "sqlite",
        _sqlite: Some(dir),
        _postgres: None,
    }];
    match new_postgres() {
        Some(pg) => out.push(Target {
            store: open(&DatabaseConfig::Postgres(pg.config.clone()), &identity),
            engine: "postgres",
            _sqlite: None,
            _postgres: Some(pg),
        }),
        None => eprintln!("DAINTY_TEST_POSTGRES_URL is not set: the Postgres half is skipped"),
    }
    out
}

pub fn system() -> ActorContext {
    ActorContext::new(Actor::System)
}

pub fn admin() -> ActorContext {
    ActorContext::new(Actor::Admin { key: "owner".into() })
}

pub fn player(account: &AccountId) -> ActorContext {
    ActorContext::new(Actor::Player {
        account: account.global().clone(),
    })
}

/// A new game account on this node, `user_id` as given.
pub async fn account(store: &IslesStore, user_id: &str) -> AccountId {
    let id = AccountId::new(store.node());
    let record = AccountRecord {
        id: id.clone(),
        user_id: UserId::new(user_id),
        name: user_id.to_owned(),
        notes: 0,
    };
    store
        .write(&system(), move |tx, repos| repos.accounts.create(tx, &record))
        .await
        .unwrap();
    id
}

pub fn launcher(
    id: &AccountId,
    user_id: &str,
    email: Option<&str>,
    name: &str,
    created: &str,
) -> LauncherAccountRecord {
    LauncherAccountRecord {
        account: id.clone(),
        user_id: UserId::new(user_id),
        email: email.map(str::to_owned),
        display_name_normalized: name.to_lowercase(),
        display_name: name.to_owned(),
        password_hash: Some("stored-password-digest".into()),
        status: OperationalStatus::Active,
        approval_status: None,
        approval_updated_at: None,
        approval_updated_by: None,
        approval_reason: None,
        roles: vec!["player".into()],
        roles_updated_at: None,
        roles_updated_by: None,
        created_at: created.to_owned(),
        last_login_at: None,
        username_set: Some(true),
        consent: None,
        deletion: None,
        email_verified_at: None,
        email_verification_pending_since: None,
        timeout: None,
    }
}

/// `(actor, table, op, row_key)` of every change-log entry after the migrations.
pub async fn log(store: &IslesStore) -> Vec<(String, String, String, String)> {
    store
        .database()
        .changes(0, 100_000)
        .await
        .unwrap()
        .into_iter()
        .filter(|e| e.op != "migrate")
        .map(|e| (e.actor, e.table_name, e.op, e.row_key))
        .collect()
}
