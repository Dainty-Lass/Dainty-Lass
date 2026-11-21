// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! What the end-to-end tests share: a free band of UDP ports, the two-hunt tables, a Director served on
//! 127.0.0.1:0 over the real launcher and the fake game server, and a small HTTP client.

#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "shared by several test binaries, each using only some; a panic is how a test fails"
)]

use std::net::{SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;

use dainty_module_api::PortRange;
use rustic_isles::testing::sample_version;
use rustic_isles::{GameDataRoot, Settings, Shared, SharedParts};
use rustic_isles_director::{Config, Director, DirectorConfig, GameserverArgs, MemoryAdmission, OsLauncher, hunt};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub const DIRECTOR_1_14_7: DirectorConfig = DirectorConfig::new(
    647_472,
    GameserverArgs::new(
        &[
            "-EpicPortal",
            "-server",
            "-nullrhi",
            "-warp",
            "-nosound",
            "-RepDriverEnable",
        ],
        "LogPhoenixCharacter VeryVerbose, LogOnline Verbose, LogLoad Off",
    ),
);
pub const MY_IP: &str = "100.101.102.103";
pub const FAKE: &str = env!("CARGO_BIN_EXE_rustic-isles-fake-gameserver");

/// Eight consecutive UDP ports that bind now, spread by the process id, below the dynamic range.
pub fn port_band() -> PortRange {
    static NEXT: AtomicU16 = AtomicU16::new(0);
    let spread = (std::process::id() % 300) as u16;
    loop {
        let band = NEXT.fetch_add(1, Ordering::SeqCst);
        let first = 20_000 + ((spread + band * 7) % 1_500) * 16;
        let free = (first..first + 8).all(|p| UdpSocket::bind(("127.0.0.1", p)).is_ok());
        if free {
            return PortRange { first, last: first + 7 };
        }
        assert!(band < 500, "no free band of UDP ports");
    }
}

fn table(name: &str, rows: Value) -> Value {
    json!([{ "Name": name, "Type": "DataTable", "Rows": rows }])
}

fn map_row(map: &str, behemoth: &str) -> Value {
    json!({
        "HuntTags": [],
        "MapList": [{ "MapName": "m", "MapAssetName": map, "Biome": "EHuntBiome::Moss", "Weighting": 1 }],
        "SpecificBehemoth": { "BehemothAsset": { "AssetPathName": behemoth } },
        "MaxPlayers": 4
    })
}

fn handle(row: &str) -> Value {
    json!({ "DataTable": { "ObjectName": "DataTable'matchmaker_hunts_table'", "ObjectPath": "x" }, "RowName": row })
}

/// `Plain_Hunt` → `MM_Plain`, `Boss_Hunt` → `MM_Boss`, the other five tables empty.
pub fn write_tables(dir: &Path) {
    let player = json!({
        "Plain_Hunt": { "MatchmakerHuntIDs": [handle("MM_Plain")] },
        "Boss_Hunt": { "MatchmakerHuntIDs": [handle("MM_Boss")] },
    });
    let matchmaker = json!({
        "MM_Plain": map_row("/Game/Maps/test/plain.plain", "None"),
        "MM_Boss": map_row("/Game/Maps/test/boss.boss", "/Game/Monsters/test/boss_bp.boss_bp_C"),
    });
    std::fs::create_dir_all(dir).unwrap();
    for (n, file) in hunt::table_files(true).iter().enumerate() {
        let name = file.trim_end_matches(".json");
        let rows = match n {
            0 => player.clone(),
            1 => matchmaker.clone(),
            _ => json!({}),
        };
        std::fs::write(dir.join(file), table(name, rows).to_string()).unwrap();
    }
}

pub struct Rig {
    pub dir: tempfile::TempDir,
    pub range: PortRange,
    pub director: Arc<Director>,
    pub address: SocketAddr,
    pub shared: Arc<Shared>,
}

impl Rig {
    pub fn logs(&self) -> PathBuf {
        self.dir.path().join("logs")
    }

    pub fn control(&self) -> PathBuf {
        self.dir.path().join("control")
    }

    pub fn log(&self, port: u16, label: &str) -> String {
        std::fs::read_to_string(self.logs().join(format!("port{port}_{label}.log"))).unwrap_or_default()
    }

    /// The fake's `FAKE_ARGS` report in a log.
    pub fn fake_args(&self, port: u16, label: &str) -> Value {
        let log = self.log(port, label);
        let line = log
            .lines()
            .find_map(|l| l.strip_prefix("FAKE_ARGS "))
            .expect("no FAKE_ARGS line");
        serde_json::from_str(line).unwrap()
    }

    pub fn stop_server(&self, port: u16) {
        std::fs::write(self.control().join(format!("stop-{port}")), "").unwrap();
    }

    pub async fn get(&self, path: &str) -> Reply {
        send(self.address, "GET", path, &[], None).await
    }

    pub async fn matchmake(&self, body: Value) -> Reply {
        send(
            self.address,
            "POST",
            "/api/matchmaker/handle-matchmaking-for-player",
            &[("content-type", "application/json")],
            Some(body.to_string().as_bytes()),
        )
        .await
    }

    pub fn listed_ports(&self) -> Vec<u16> {
        self.director.gameservers().iter().map(|s| s.port).collect()
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.director.kill_all();
    }
}

/// The settings every end-to-end test starts from, with `extra` over them.
pub fn settings(dir: &Path, range: PortRange, extra: &[(&str, &str)]) -> Settings {
    let logs = dir.join("logs").display().to_string();
    let control = dir.join("control").display().to_string();
    std::fs::create_dir_all(&control).unwrap();
    let parent = std::process::id().to_string();
    let (first, last) = (range.first.to_string(), range.last.to_string());
    let mut pairs: Vec<(String, String)> = [
        ("PORT_RANGE_BEGIN", first.as_str()),
        ("PORT_RANGE_END", last.as_str()),
        ("GAMESERVER_BINARY_PATH", FAKE),
        ("METAGAME_API_KEY", "gameserver-key"),
        ("MY_IP", MY_IP),
        ("SECONDS_TO_WAIT_BETWEEN_GAMESERVER_STARTUP", "0"),
        ("GAMESERVER_READY_TIMEOUT_MS", "15000"),
        ("MAX_CONCURRENT_HUNT_SERVERS", "2"),
        ("DAINTY_SERVER_MAX_FPS", "30"),
        ("GAMESERVER_LOG_DIR", logs.as_str()),
        ("FAKE_GS_CONTROL", control.as_str()),
        ("FAKE_GS_PARENT_PID", parent.as_str()),
        ("FAKE_GS_MAX_SECS", "300"),
    ]
    .iter()
    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
    .collect();
    for (k, v) in extra {
        pairs.retain(|(name, _)| name != k);
        pairs.push(((*k).to_owned(), (*v).to_owned()));
    }
    Settings::new(pairs.into_iter().collect(), Default::default())
}

pub fn shared(settings: Settings, director_dir: Option<PathBuf>) -> Arc<Shared> {
    let mut parts = SharedParts::bare(sample_version());
    parts.settings = settings;
    parts.game_data = GameDataRoot::new(None, director_dir);
    Shared::new(parts).unwrap()
}

/// A Director served on 127.0.0.1:0, not booted; `tweak` adjusts its config first.
pub async fn rig(extra: &[(&str, &str)], tweak: impl FnOnce(&mut Config)) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let range = port_band();
    let settings = settings(dir.path(), range, extra);
    let tables_dir = dir.path().join("tables");
    write_tables(&tables_dir);
    let mut config = Config::from_settings(&settings, &DIRECTOR_1_14_7, range, std::env::vars()).unwrap();
    tweak(&mut config);
    let memory = MemoryAdmission::new(config.memory);
    let tables = hunt::load_tables(&tables_dir).unwrap();
    let director = Director::new(config, tables, memory, Arc::new(OsLauncher));
    let shared = shared(settings, Some(tables_dir));
    let router = rustic_isles_director::router(Arc::clone(&shared), &director).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    Rig {
        dir,
        range,
        director,
        address,
        shared,
    }
}

/// Polls until `check` holds, at most `secs`.
pub async fn wait_for(secs: u64, mut check: impl FnMut() -> bool) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    while tokio::time::Instant::now() < deadline {
        if check() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    check()
}

pub fn udp_free(port: u16) -> bool {
    UdpSocket::bind(("127.0.0.1", port)).is_ok()
}

#[derive(Debug)]
pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl Reply {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }

    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
}

/// One HTTP/1.1 request on a new connection.
pub async fn send(
    address: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&[u8]>,
) -> Reply {
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let body = body.unwrap_or_default();
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(body).await.unwrap();
    let mut raw = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(60), stream.read_to_end(&mut raw)).await;
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("no end of headers");
    let head = String::from_utf8_lossy(&raw[..split]).into_owned();
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|l| l.split(' ').nth(1))
        .and_then(|c| c.parse().ok())
        .unwrap();
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect();
    Reply {
        status,
        headers,
        body: String::from_utf8_lossy(&raw[split + 4..]).into_owned(),
    }
}
