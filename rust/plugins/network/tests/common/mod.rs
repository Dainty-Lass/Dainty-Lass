//! The engines the network's tests run on: SQLite always, and Postgres too when `REFORGED_TEST_POSTGRES_URL` names
//! a server the tests may create databases on, as `store`'s tests do.
#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)] // test helpers

use std::sync::atomic::{AtomicU32, Ordering};

use diesel::connection::SimpleConnection;
use diesel::prelude::*;
use network::directory::SCHEMA;
use store::{Database, DatabaseConfig, DatabaseOptions, PostgresConfig, portable};

/// A throwaway Postgres database, dropped with the guard.
pub struct PgDatabase {
    pub server: PostgresConfig,
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

pub fn postgres_server() -> Option<PostgresConfig> {
    let url = std::env::var("REFORGED_TEST_POSTGRES_URL").ok()?;
    let url = url::Url::parse(&url).expect("REFORGED_TEST_POSTGRES_URL is a postgres:// URL");
    Some(PostgresConfig {
        host: url.host_str().unwrap_or("127.0.0.1").to_owned(),
        port: url.port().unwrap_or(5432),
        user: url.username().to_owned(),
        database: url.path().trim_start_matches('/').to_owned(),
        password: url.password().unwrap_or_default().to_owned(),
        allow_remote: false,
    })
}

static NEXT: AtomicU32 = AtomicU32::new(0);

pub fn new_postgres(server: PostgresConfig) -> PgDatabase {
    let name = format!(
        "network_test_{}_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    );
    PgConnection::establish(&server.conninfo())
        .expect("REFORGED_TEST_POSTGRES_URL is reachable")
        .batch_execute(&format!("CREATE DATABASE {}", portable::quote_ident(&name)))
        .unwrap();
    let config = PostgresConfig {
        database: name,
        ..server.clone()
    };
    PgDatabase { server, config }
}

#[derive(Debug, Clone, Copy)]
pub enum Engine {
    Sqlite,
    Postgres,
}

/// SQLite, and Postgres when it is configured.
pub fn engines() -> Vec<Engine> {
    if postgres_server().is_some() {
        vec![Engine::Sqlite, Engine::Postgres]
    } else {
        eprintln!("REFORGED_TEST_POSTGRES_URL is not set: the Postgres half is skipped");
        vec![Engine::Sqlite]
    }
}

pub enum Target {
    Sqlite(tempfile::TempDir),
    Postgres(PgDatabase),
}

impl Target {
    pub fn new(engine: Engine) -> Self {
        match engine {
            Engine::Sqlite => Self::Sqlite(tempfile::tempdir().unwrap()),
            Engine::Postgres => Self::Postgres(new_postgres(postgres_server().unwrap())),
        }
    }

    pub fn open(&self) -> Database {
        let config = match self {
            Self::Sqlite(dir) => DatabaseConfig::Sqlite {
                file: dir.path().join("node.db"),
            },
            Self::Postgres(pg) => DatabaseConfig::Postgres(pg.config.clone()),
        };
        Database::open(&config, &DatabaseOptions::default(), &SCHEMA).unwrap()
    }
}
