// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The repository tests' targets and fixtures. Every test runs on SQLite, and on Postgres too when
//! `REFORGED_TEST_POSTGRES_URL` names a server the tests may create databases on
//! (`postgres://postgres:postgres@127.0.0.1:5432/postgres` in CI).

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

pub mod kit;

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use diesel::connection::SimpleConnection;
use diesel::{Connection, PgConnection};
use module_api::NodeId;
use rifts_realms::db::RealmsDb;
use rifts_realms::hunt::{AdmissionState, HuntSession, HuntState, Participant};
use rifts_realms::time::parse_iso;
use rifts_seam::{Actor, UnixMillis};
use store::{DatabaseConfig, DatabaseOptions, PostgresConfig, portable};

pub const POSTGRES_URL: &str = "REFORGED_TEST_POSTGRES_URL";

/// A Postgres database of this test's own, dropped with it.
pub struct PgDatabase {
    server: PostgresConfig,
    pub config: PostgresConfig,
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

fn server() -> Option<PostgresConfig> {
    let text = std::env::var(POSTGRES_URL).ok()?;
    let url = url::Url::parse(&text).expect("REFORGED_TEST_POSTGRES_URL is a postgres:// URL");
    Some(PostgresConfig {
        host: url.host_str().unwrap_or("127.0.0.1").to_owned(),
        port: url.port().unwrap_or(5432),
        user: url.username().to_owned(),
        database: url.path().trim_start_matches('/').to_owned(),
        password: url.password().unwrap_or_default().to_owned(),
        allow_remote: false,
    })
}

/// Parallel tests can fill a small server's connection slots, so the admin connection is retried.
fn admin_connection(server: &PostgresConfig) -> PgConnection {
    let mut last = None;
    for _ in 0..100 {
        match PgConnection::establish(&server.conninfo()) {
            Ok(conn) => return conn,
            Err(error) => last = Some(error),
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    panic!("REFORGED_TEST_POSTGRES_URL is not reachable: {last:?}");
}

pub fn new_postgres() -> Option<PgDatabase> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let server = server()?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let name = format!(
        "rifts_test_{}_{}_{nanos}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    );
    admin_connection(&server)
        .batch_execute(&format!("CREATE DATABASE {}", portable::quote_ident(&name)))
        .unwrap();
    let config = PostgresConfig {
        database: name,
        ..server.clone()
    };
    Some(PgDatabase { server, config })
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

pub fn sqlite() -> Target {
    let dir = tempfile::tempdir().unwrap();
    let config = DatabaseConfig::Sqlite {
        file: dir.path().join("realms.db"),
    };
    let db = RealmsDb::open(&config, &DatabaseOptions::default(), node()).unwrap();
    Target {
        engine: "sqlite",
        config,
        db,
        _dir: Some(dir),
        _pg: None,
    }
}

pub fn postgres() -> Option<Target> {
    let pg = new_postgres()?;
    let config = DatabaseConfig::Postgres(pg.config.clone());
    let db = RealmsDb::open(&config, &DatabaseOptions::default(), node()).unwrap();
    Some(Target {
        engine: "postgres",
        config,
        db,
        _dir: None,
        _pg: Some(pg),
    })
}

/// A database nothing has opened yet, on `engine`, kept alive with its folder or Postgres database.
pub struct Empty {
    pub config: DatabaseConfig,
    _dir: Option<tempfile::TempDir>,
    _pg: Option<PgDatabase>,
}

pub fn empty(engine: &str) -> Empty {
    if engine == "postgres" {
        let pg = new_postgres().expect("Postgres is configured");
        return Empty {
            config: DatabaseConfig::Postgres(pg.config.clone()),
            _dir: None,
            _pg: Some(pg),
        };
    }
    let dir = tempfile::tempdir().unwrap();
    Empty {
        config: DatabaseConfig::Sqlite {
            file: dir.path().join("realms.db"),
        },
        _dir: Some(dir),
        _pg: None,
    }
}

/// SQLite always; Postgres when configured.
pub fn targets() -> Vec<Target> {
    let mut targets = vec![sqlite()];
    match postgres() {
        Some(target) => targets.push(target),
        None => eprintln!("REFORGED_TEST_POSTGRES_URL is not set: the Postgres half is skipped"),
    }
    targets
}

/// A Node ISO time.
pub fn t(iso: &str) -> i64 {
    parse_iso(iso)
        .map(|at| at.0)
        .unwrap_or_else(|| panic!("not an ISO time: {iso}"))
}

pub fn actor() -> Actor {
    Actor::System("test")
}

pub const HOST_STEAM: &str = "76561198000000101";
pub const MEMBER_STEAM: &str = "76561198000000102";

/// A hunt with Node's test defaults: everything optional absent.
pub fn hunt(session_id: &str) -> HuntSession {
    HuntSession {
        session_id: session_id.to_owned(),
        generation: 1,
        state: HuntState::Requested,
        state_version: 1,
        party_id: "party-1".to_owned(),
        active_party_key: None,
        party_revision: 1,
        hunt_definition_id: "ShatteredIsles_IslandA".to_owned(),
        game_mode: None,
        build: 647_472,
        bridge_protocol: 1,
        virtual_port: 1,
        host_account_id: "host".to_owned(),
        host_steam_id: HOST_STEAM.to_owned(),
        bridge_instance_id: "bridge".to_owned(),
        roster: Vec::new(),
        lobby_id: None,
        coop_transport: None,
        lease_expires_at: None,
        created_at: UnixMillis(0),
        expires_at: UnixMillis(4_102_444_800_000),
        metadata_nonce: None,
        hub_kind: None,
        realm_id: None,
        host_participates: None,
        host_local_bridge_port: None,
        server_udp_port: None,
        host_local_bridge_ready_at: None,
        hub_room_id: None,
        max_peers: None,
        guard_completion_mode: None,
        host_offer_id: None,
        offer_expires_at: None,
        startup_deadline: None,
        last_heartbeat_at: None,
        last_lease_sequence: None,
        suspect_since: None,
        suspect_deadline: None,
        pre_suspect_state: None,
        failure_reason: None,
        tried_host_account_ids: Vec::new(),
        terminal_at: None,
        recovery_deadline: None,
        recovery_retry_at: None,
        completion_reason: None,
        drain_deadline: None,
        cancelled_by_account_id: None,
        trust_tier: None,
        guard_encounter_state: None,
        extra: serde_json::Map::new(),
    }
}

/// The host's steam id for `host`, another for everyone else; `route_ready` sets the bridge fields.
pub fn participant(account: &str, route_ready: bool) -> Participant {
    let steam = if account == "host" { HOST_STEAM } else { MEMBER_STEAM };
    let mut p = Participant::pending(account, steam, &format!("{account}-candidate"));
    if route_ready {
        p.local_bridge_instance_id = Some(format!("{account}-bridge"));
        p.local_bridge_port = Some(55_000);
        p.local_bridge_ready_at = Some(UnixMillis(1_787_587_208_000));
    }
    p
}

pub fn admitted(mut p: Participant) -> Participant {
    p.admission_state = AdmissionState::Admitted;
    p
}
