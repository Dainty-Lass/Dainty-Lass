// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The repository tests' targets and fixtures. Every test runs on SQLite, and on Postgres too when
//! `DAINTY_TEST_POSTGRES_URL` names a server the tests may create databases on
//! (`postgres://postgres:postgres@127.0.0.1:5432/postgres` in CI).

#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "shared by several test binaries, each using only some; a panic is how a test fails"
)]

pub mod kit;

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use dainty_module_api::NodeId;
use dainty_store::PgConnection;
use dainty_store::{DatabaseConfig, DatabaseOptions, PostgresConfig, portable::quote_ident};
use diesel::{Connection as _, connection::SimpleConnection as _};
use rustic_rifts_realms::{
    db::RealmsDb,
    hunt::{AdmissionState, HuntSession, Participant},
    time::parse_iso,
};
use rustic_rifts_seam::{Actor, UnixMillis};

pub const POSTGRES_URL: &str = "DAINTY_TEST_POSTGRES_URL";

/// A Postgres database of this test's own, dropped with it.
pub struct PgDatabase {
    server: PostgresConfig,
    pub config: PostgresConfig,
}

impl Drop for PgDatabase {
    fn drop(&mut self) {
        let drop_it = format!(
            "DROP DATABASE IF EXISTS {} WITH (FORCE)",
            quote_ident(&self.config.database)
        );
        if let Ok(mut admin) = PgConnection::establish(&self.server.conninfo()) {
            admin.batch_execute(&drop_it).ok();
        }
    }
}

/// The configured server, read from the URL.
fn server() -> Option<PostgresConfig> {
    let raw = std::env::var(POSTGRES_URL).ok()?;
    let parsed = url::Url::parse(&raw).expect("DAINTY_TEST_POSTGRES_URL is a postgres:// URL");
    let database = parsed.path().trim_start_matches('/').to_owned();
    Some(PostgresConfig {
        database,
        user: parsed.username().to_owned(),
        password: parsed.password().unwrap_or_default().to_owned(),
        host: parsed.host_str().unwrap_or("127.0.0.1").to_owned(),
        port: parsed.port().unwrap_or(5432),
        allow_remote: false,
    })
}

/// Parallel tests can fill a small server's connection slots, so the admin connection is retried.
fn admin_connection(server: &PostgresConfig) -> PgConnection {
    let info = server.conninfo();
    let mut failure = None;
    for _attempt in 0..100 {
        match PgConnection::establish(&info) {
            Ok(admin) => return admin,
            Err(error) => failure = Some(error),
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("DAINTY_TEST_POSTGRES_URL is not reachable: {failure:?}");
}

pub fn new_postgres() -> Option<PgDatabase> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let server = server()?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.subsec_nanos());
    let serial = NEXT.fetch_add(1, Ordering::SeqCst);
    let database = format!("rifts_test_{}_{serial}_{stamp}", std::process::id());
    let create = format!("CREATE DATABASE {}", quote_ident(&database));
    admin_connection(&server).batch_execute(&create).unwrap();
    let mut config = server.clone();
    config.database = database;
    Some(PgDatabase { config, server })
}

pub fn node() -> NodeId {
    NodeId::new("local").unwrap()
}

/// One engine's database, kept alive with its folder or Postgres database.
pub struct Target {
    pub engine: &'static str,
    pub config: DatabaseConfig,
    pub db: RealmsDb,
    _dir: Option<tempfile::TempDir>,
    _pg: Option<PgDatabase>,
}

fn opened(
    engine: &'static str,
    config: DatabaseConfig,
    keep: (Option<tempfile::TempDir>, Option<PgDatabase>),
) -> Target {
    let db = RealmsDb::open(&config, &DatabaseOptions::default(), node()).unwrap();
    let (_dir, _pg) = keep;
    Target {
        engine,
        config,
        db,
        _dir,
        _pg,
    }
}

fn sqlite_file(dir: &tempfile::TempDir) -> DatabaseConfig {
    DatabaseConfig::Sqlite {
        file: dir.path().join("realms.db"),
    }
}

pub fn sqlite() -> Target {
    let dir = tempfile::tempdir().unwrap();
    opened("sqlite", sqlite_file(&dir), (Some(dir), None))
}

pub fn postgres() -> Option<Target> {
    let pg = new_postgres()?;
    let config = DatabaseConfig::Postgres(pg.config.clone());
    Some(opened("postgres", config, (None, Some(pg))))
}

/// A database nothing has opened yet, on `engine`, kept alive with its folder or Postgres database.
pub struct Empty {
    pub config: DatabaseConfig,
    _dir: Option<tempfile::TempDir>,
    _pg: Option<PgDatabase>,
}

pub fn empty(engine: &str) -> Empty {
    if engine != "postgres" {
        let dir = tempfile::tempdir().unwrap();
        let config = sqlite_file(&dir);
        return Empty {
            config,
            _dir: Some(dir),
            _pg: None,
        };
    }
    let pg = new_postgres().expect("Postgres is configured");
    Empty {
        config: DatabaseConfig::Postgres(pg.config.clone()),
        _pg: Some(pg),
        _dir: None,
    }
}

/// SQLite always; Postgres when configured.
pub fn targets() -> Vec<Target> {
    let pg = postgres();
    if pg.is_none() {
        eprintln!("DAINTY_TEST_POSTGRES_URL is not set: the Postgres half is skipped");
    }
    std::iter::once(sqlite()).chain(pg).collect()
}

/// A Node ISO time.
pub fn t(iso: &str) -> i64 {
    let Some(at) = parse_iso(iso) else {
        panic!("not an ISO time: {iso}");
    };
    at.0
}

pub fn actor() -> Actor {
    Actor::System("test")
}

pub const HOST_STEAM: &str = "76561198000000101";
pub const MEMBER_STEAM: &str = "76561198000000102";

/// A hunt with Node's test defaults: everything optional absent.
pub fn hunt(session_id: &str) -> HuntSession {
    let detail = serde_json::json!({
        "sessionId": session_id,
        "generation": 1,
        "state": "REQUESTED",
        "stateVersion": 1,
        "partyId": "party-1",
        "partyRevision": 1,
        "huntDefinitionId": "ShatteredIsles_IslandA",
        "build": 647_472,
        "bridgeProtocol": 1,
        "virtualPort": 1,
        "hostAccountId": "host",
        "hostSteamId": HOST_STEAM,
        "bridgeInstanceId": "bridge",
        "createdAt": 0,
        "expiresAt": 4_102_444_800_000_i64,
    });
    serde_json::from_value(detail).unwrap()
}

/// The host's steam id for `host`, another for everyone else; `route_ready` sets the bridge fields.
pub fn participant(account: &str, route_ready: bool) -> Participant {
    let peer = match account {
        "host" => HOST_STEAM,
        _ => MEMBER_STEAM,
    };
    let mut entry = Participant::pending(account, peer, &format!("{account}-candidate"));
    if route_ready {
        entry.local_bridge_ready_at = Some(UnixMillis(1_787_587_208_000));
        entry.local_bridge_port = Some(55_000);
        entry.local_bridge_instance_id = Some(format!("{account}-bridge"));
    }
    entry
}

pub fn admitted(p: Participant) -> Participant {
    Participant {
        admission_state: AdmissionState::Admitted,
        ..p
    }
}
