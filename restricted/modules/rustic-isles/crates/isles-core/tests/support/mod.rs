//! A database per test on SQLite, and on Postgres when `REFORGED_TEST_POSTGRES_URL` names a server the tests may create
//! databases on (`postgres://user:pw@127.0.0.1:5432/postgres`); each test's Postgres database is dropped after it.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)] // test helpers

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use diesel::Connection;
use diesel::connection::SimpleConnection;
use isles_core::db::IslesStore;
use isles_core::repo::auth::AccountRecord;
use isles_core::{AccountId, UserId};
use module_api::network::StandaloneHome;
use module_api::{Actor, ActorContext, NodeIdentity};
use store::PgConnection;
use store::portable::quote_ident;
use store::{DatabaseConfig, DatabaseOptions, PostgresConfig};

pub struct Target {
    pub store: IslesStore,
    pub node: NodeIdentity,
    pub engine: &'static str,
    cleanup: Option<PostgresConfig>,
    server: Option<PostgresConfig>,
    _dir: Option<tempfile::TempDir>,
}

impl Drop for Target {
    fn drop(&mut self) {
        if let (Some(server), Some(config)) = (&self.server, &self.cleanup)
            && let Ok(mut admin) = PgConnection::establish(&server.conninfo())
        {
            _ = admin.batch_execute(&format!(
                "DROP DATABASE IF EXISTS {} WITH (FORCE)",
                quote_ident(&config.database)
            ));
        }
    }
}

fn server() -> Option<PostgresConfig> {
    let url = std::env::var("REFORGED_TEST_POSTGRES_URL").ok()?;
    let rest = url
        .strip_prefix("postgres://")
        .expect("REFORGED_TEST_POSTGRES_URL is a postgres:// URL");
    let (login, location) = rest.split_once('@').expect("a user and a host");
    let (user, password) = login.split_once(':').unwrap_or((login, ""));
    let (hostport, database) = location.split_once('/').unwrap_or((location, "postgres"));
    let (host, port) = hostport.split_once(':').unwrap_or((hostport, "5432"));
    Some(PostgresConfig {
        host: host.to_owned(),
        port: port.parse().unwrap(),
        user: user.to_owned(),
        database: database.to_owned(),
        password: password.to_owned(),
        allow_remote: false,
    })
}

static NEXT: AtomicU32 = AtomicU32::new(0);

fn open(config: &DatabaseConfig, node: &NodeIdentity) -> IslesStore {
    let home = Arc::new(StandaloneHome { identity: node.clone() });
    IslesStore::open(config, &DatabaseOptions::default(), home, node).unwrap()
}

/// SQLite, and Postgres when it is configured.
pub fn targets() -> Vec<Target> {
    let node = NodeIdentity::standalone();
    let dir = tempfile::tempdir().unwrap();
    let config = DatabaseConfig::Sqlite {
        file: dir.path().join("isles.db"),
    };
    let mut out = vec![Target {
        store: open(&config, &node),
        node: node.clone(),
        engine: "sqlite",
        cleanup: None,
        server: None,
        _dir: Some(dir),
    }];
    match server() {
        Some(server) => {
            let name = format!(
                "isles_test_{}_{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            PgConnection::establish(&server.conninfo())
                .expect("REFORGED_TEST_POSTGRES_URL is reachable")
                .batch_execute(&format!("CREATE DATABASE {}", quote_ident(&name)))
                .unwrap();
            let config = PostgresConfig {
                database: name,
                ..server.clone()
            };
            out.push(Target {
                store: open(&DatabaseConfig::Postgres(config.clone()), &node),
                node,
                engine: "postgres",
                cleanup: Some(config),
                server: Some(server),
                _dir: None,
            });
        }
        None => eprintln!("REFORGED_TEST_POSTGRES_URL is not set: the Postgres half is skipped"),
    }
    out
}

impl Target {
    /// A game account whose `userId` and name are `name`.
    pub async fn account(&self, name: &str) -> AccountId {
        let record = AccountRecord {
            id: AccountId::new(&self.node.node),
            user_id: UserId(name.to_owned()),
            name: name.to_owned(),
            notes: 0,
        };
        let id = record.id.clone();
        self.store
            .account_write(&id, &ActorContext::new(Actor::System), move |tx, repos| {
                repos.accounts.create(tx, &record)
            })
            .await
            .unwrap();
        id
    }
}
