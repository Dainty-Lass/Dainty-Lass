//! Every repository test runs on SQLite, and on Postgres when `REFORGED_TEST_POSTGRES_URL` names a server the tests may
//! create databases on (`postgres://user:pw@127.0.0.1:5432/postgres`), as `store`'s own tests do (DD15).
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

pub mod kit;

use std::sync::atomic::{AtomicU32, Ordering};

use diesel::connection::SimpleConnection;
use diesel::{Connection, PgConnection};
use module_api::NodeId;
use rifts_realms::db::RealmsDb;
use rifts_seam::{Actor, UnixMillis};
use store::{DatabaseConfig, DatabaseOptions, PostgresConfig, portable};

pub struct PgDatabase {
    server: PostgresConfig,
    config: PostgresConfig,
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
    let url = std::env::var("REFORGED_TEST_POSTGRES_URL").ok()?;
    let url = url::Url::parse(&url).expect("REFORGED_TEST_POSTGRES_URL is a postgres:// URL");
    let server = PostgresConfig {
        host: url.host_str().unwrap_or("127.0.0.1").to_owned(),
        port: url.port().unwrap_or(5432),
        user: url.username().to_owned(),
        database: url.path().trim_start_matches('/').to_owned(),
        password: url.password().unwrap_or_default().to_owned(),
        allow_remote: false,
    };
    let name = format!(
        "rifts_test_{}_{}_{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos()
    );
    // Parallel tests can hold every slot of a small server for a moment: wait for one
    let mut admin = None;
    for _ in 0..100 {
        match PgConnection::establish(&server.conninfo()) {
            Ok(conn) => {
                admin = Some(conn);
                break;
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(100)),
        }
    }
    admin
        .expect("REFORGED_TEST_POSTGRES_URL is reachable")
        .batch_execute(&format!("CREATE DATABASE {}", portable::quote_ident(&name)))
        .unwrap();
    let config = PostgresConfig {
        database: name,
        ..server.clone()
    };
    Some(PgDatabase { server, config })
}

pub enum Target {
    Sqlite(tempfile::TempDir),
    Postgres(PgDatabase),
}

impl Target {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Sqlite(_) => "sqlite",
            Self::Postgres(_) => "postgres",
        }
    }

    pub fn config(&self) -> DatabaseConfig {
        match self {
            Self::Sqlite(dir) => DatabaseConfig::Sqlite {
                file: dir.path().join("realms.db"),
            },
            Self::Postgres(pg) => DatabaseConfig::Postgres(pg.config.clone()),
        }
    }

    pub fn open(&self) -> RealmsDb {
        let config = self.config();
        RealmsDb::open(&config, &DatabaseOptions::default(), NodeId::new("local").unwrap()).unwrap()
    }
}

/// A fresh database per engine: SQLite always, Postgres when configured.
pub fn targets() -> Vec<Target> {
    let mut out = vec![Target::Sqlite(tempfile::tempdir().unwrap())];
    match new_postgres() {
        Some(pg) => out.push(Target::Postgres(pg)),
        None => eprintln!("REFORGED_TEST_POSTGRES_URL is not set: the Postgres half is skipped"),
    }
    out
}

/// A Node ISO time.
pub fn t(text: &str) -> UnixMillis {
    rifts_realms::time::parse_iso(text).expect(text)
}

pub fn actor() -> Actor {
    Actor::System("test")
}

/// A session with Node's test defaults; tests change what they need.
pub fn hunt_session(session_id: &str, state: rifts_realms::hunt::HuntState) -> rifts_realms::hunt::HuntSession {
    rifts_realms::hunt::HuntSession {
        session_id: session_id.to_owned(),
        generation: 1,
        state,
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
        host_steam_id: "76561198000000101".to_owned(),
        bridge_instance_id: "bridge".to_owned(),
        roster: Vec::new(),
        lobby_id: None,
        coop_transport: None,
        lease_expires_at: None,
        created_at: UnixMillis(0),
        expires_at: UnixMillis(i64::MAX / 2),
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

/// A roster entry: admitted or not, with or without a published loopback route.
pub fn participant(
    account: &str,
    state: rifts_realms::hunt::AdmissionState,
    route_ready: bool,
) -> rifts_realms::hunt::Participant {
    let mut p = rifts_realms::hunt::Participant::pending(
        account,
        if account == "host" {
            "76561198000000101"
        } else {
            "76561198000000102"
        },
        &format!("{account}-candidate"),
    );
    p.admission_state = state;
    if route_ready {
        p.local_bridge_instance_id = Some(format!("{account}-bridge"));
        p.local_bridge_port = Some(55_000);
        p.local_bridge_ready_at = Some(UnixMillis(1_787_587_208_000));
    }
    p
}
