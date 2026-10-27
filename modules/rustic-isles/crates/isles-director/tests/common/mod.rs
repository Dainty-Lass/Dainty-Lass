//! The Director over HTTP with the fake game server (`isles-fake-gameserver`), for both versions' real
//! configs.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)] // test helpers

use std::net::{SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use isles_core::Settings;
use isles_core::http::Shared;
use isles_core::version::IslesVersion;
use isles_director::gameservers::Director;
use isles_director::memory::MemoryAdmission;
use isles_director::{Config, DirectorConfig, OsLauncher, hunt};
use module_api::PortRange;
use serde_json::{Value, json};

pub const FAKE: &str = env!("CARGO_BIN_EXE_isles-fake-gameserver");

/// One game version's Director, as its module crate configures it.
#[derive(Clone, Copy)]
pub struct Version {
    pub name: &'static str,
    pub config: DirectorConfig,
    pub version: fn() -> IslesVersion,
}

pub const V1147: Version = Version {
    name: "1.14.7",
    config: isles_1_14_7::DIRECTOR,
    version: isles_1_14_7::version,
};

pub const V1120: Version = Version {
    name: "1.12.0",
    config: isles_1_12_0::DIRECTOR,
    version: isles_1_12_0::version,
};

/// `count` consecutive UDP ports on loopback that are free now and that no other test of this binary was given.
/// They start in a band of their own, set by this process's id and well below the system's dynamic ports, so
/// test runs of other worktrees on the same machine (and the system's own sockets) do not take them.
pub fn free_range(count: u16) -> (u16, u16) {
    static NEXT: Mutex<u16> = Mutex::new(0);
    let mut next = NEXT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if *next == 0 {
        *next = 15000 + (std::process::id() % 2500) as u16 * 8;
    }
    for _ in 0..2000 {
        let first = *next;
        *next = if first + count + 8 > 40000 {
            15000
        } else {
            first + count + 3
        };
        if (first..first + count).all(|p| UdpSocket::bind(("127.0.0.1", p)).is_ok()) {
            return (first, first + count - 1);
        }
    }
    panic!("no free range of {count} UDP ports");
}

pub fn udp_free(port: u16) -> bool {
    UdpSocket::bind(("127.0.0.1", port)).is_ok()
}

pub fn write_tables(dir: &Path, trials_new: bool) {
    let table = |rows: Value| json!([{ "Type": "DataTable", "Rows": rows }]).to_string();
    std::fs::write(
        dir.join("player_hunts_table.json"),
        table(json!({
            "Plain_Hunt": { "MatchmakerHuntIDs": [{ "RowName": "MM_Plain" }] },
            "Boss_Hunt": { "MatchmakerHuntIDs": [{ "RowName": "MM_Boss" }] }
        })),
    )
    .unwrap();
    std::fs::write(
        dir.join("matchmaker_hunts_table.json"),
        table(json!({
            "MM_Plain": { "MapList": [{ "MapAssetName": "/Game/Maps/test/plain.plain" }], "SpecificBehemoth": { "BehemothAsset": { "AssetPathName": "None" } } },
            "MM_Boss": { "MapList": [{ "MapAssetName": "/Game/Maps/test/boss.boss" }], "SpecificBehemoth": { "BehemothAsset": { "AssetPathName": "/Game/Monsters/test/boss_bp.boss_bp_C" } } }
        })),
    )
    .unwrap();
    let wanted = if trials_new { 5 } else { 3 };
    for (_, file) in hunt::MATCHMAKER_TABLES[1..].iter().take(wanted) {
        std::fs::write(dir.join(file), table(json!({}))).unwrap();
    }
}

/// The env file a host's `setup` would write for the Director, over the fake game server.
pub fn env_pairs(root: &Path) -> Vec<(String, String)> {
    let path = |name: &str| root.join(name).display().to_string();
    [
        ("MY_IP", "100.101.102.103".to_owned()),
        ("GAMESERVER_BINARY_PATH", FAKE.to_owned()),
        ("METAGAME_API_KEY", "gameserver-key".to_owned()),
        ("SECONDS_TO_WAIT_BETWEEN_GAMESERVER_STARTUP", "0".to_owned()),
        ("GAMESERVER_READY_TIMEOUT_MS", "15000".to_owned()),
        ("MAX_CONCURRENT_HUNT_SERVERS", "2".to_owned()),
        ("GAMESERVER_LOG_DIR", path("logs")),
        ("REFORGED_SERVER_MAX_FPS", "30".to_owned()),
        ("FAKE_GS_CONTROL", path("control")),
        // A fake never outlives this test process, however it ends
        ("FAKE_GS_PARENT_PID", std::process::id().to_string()),
        ("FAKE_GS_MAX_SECS", "300".to_owned()),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect()
}

/// Nothing in a test waits for ever: this long for any one step, and for a whole test.
pub const STEP: Duration = Duration::from_secs(60);

/// A whole test, bounded: a hang fails it (and the harness's `Drop` then ends every game server).
pub async fn bounded<T>(test: impl std::future::Future<Output = T>) -> T {
    match tokio::time::timeout(Duration::from_secs(180), test).await {
        Ok(value) => value,
        Err(_) => panic!("the test ran over 180 s"),
    }
}

/// One step, bounded.
pub async fn within<T>(what: &str, step: impl std::future::Future<Output = T>) -> T {
    match tokio::time::timeout(STEP, step).await {
        Ok(value) => value,
        Err(_) => panic!("{what} took over {STEP:?}"),
    }
}

pub struct Harness {
    pub addr: SocketAddr,
    pub director: Arc<Director>,
    pub control: PathBuf,
    pub logs: PathBuf,
    pub begin: u16,
    pub end: u16,
    pub root: tempfile::TempDir,
    server: tokio::task::JoinHandle<()>,
}

/// However a test ends, its game servers' process trees and its HTTP server end with it.
impl Drop for Harness {
    fn drop(&mut self) {
        self.director.kill_all();
        self.server.abort();
    }
}

/// What a game server needs from the system to start on Windows (the Director clears the environment).
pub fn system_env() -> Vec<(String, String)> {
    ["SystemRoot", "PATH"]
        .iter()
        .filter_map(|name| Some(((*name).to_owned(), std::env::var(name).ok()?)))
        .collect()
}

pub async fn start(version: Version, extra: &[(&str, &str)]) -> Harness {
    start_with(version, extra, |_| {}).await
}

pub async fn start_with(version: Version, extra: &[(&str, &str)], tune: impl FnOnce(&mut Config)) -> Harness {
    let root = tempfile::tempdir().unwrap();
    for d in ["control", "game-data", "logs"] {
        std::fs::create_dir_all(root.path().join(d)).unwrap();
    }
    write_tables(&root.path().join("game-data"), version.config.trials_new_tables);
    let (begin, end) = free_range(5);
    let mut pairs = env_pairs(root.path());
    for (k, v) in extra {
        pairs.retain(|(name, _)| name != k);
        pairs.push(((*k).to_owned(), (*v).to_owned()));
    }
    let settings = Settings::from_pairs(pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    let mut config = Config::from_settings(
        &settings,
        &version.config,
        PortRange {
            first: begin,
            last: end,
        },
        system_env(),
    )
    .unwrap();
    tune(&mut config);
    let tables = hunt::load_tables(&root.path().join("game-data"), config.trials_new_tables).unwrap();
    let memory = MemoryAdmission::new(config.memory.clone());
    let director = Director::new(config, tables, memory, Arc::new(OsLauncher));
    let shared = Arc::new(Shared::bare((version.version)()));
    let app = isles_director::router(shared, &director).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Harness {
        server,
        addr,
        director,
        control: root.path().join("control"),
        logs: root.path().join("logs"),
        begin,
        end,
        root,
    }
}

pub struct Reply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub text: String,
}

impl Reply {
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.text).unwrap_or(Value::Null)
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

/// A minimal HTTP/1.1 client over a TCP socket (no client crate in the tree for tests).
pub async fn send(addr: SocketAddr, method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> Reply {
    within(
        &format!("{method} {path}"),
        send_unbounded(addr, method, path, headers, body),
    )
    .await
}

async fn send_unbounded(addr: SocketAddr, method: &str, path: &str, headers: &[(&str, &str)], body: &str) -> Reply {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let mut request = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n");
    for (k, v) in headers {
        request.push_str(&format!("{k}: {v}\r\n"));
    }
    request.push_str(&format!("Content-Length: {}\r\n\r\n{body}", body.len()));
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let mut lines = head.lines();
    let status = lines.next().unwrap().split(' ').nth(1).unwrap().parse().unwrap();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect();
    let chunked = headers.iter().any(|(k, v)| k == "transfer-encoding" && v == "chunked");
    let body = if chunked { dechunk(body) } else { body.to_owned() };
    Reply {
        status,
        headers,
        text: body,
    }
}

fn dechunk(mut body: &str) -> String {
    let mut out = String::new();
    while let Some((size, rest)) = body.split_once("\r\n") {
        let n = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
        if n == 0 {
            break;
        }
        out.push_str(&rest[..n]);
        body = &rest[n + 2..];
    }
    out
}

impl Harness {
    pub async fn get(&self, path: &str) -> Reply {
        send(self.addr, "GET", path, &[], "").await
    }

    pub async fn post(&self, path: &str, body: Value) -> Reply {
        send(
            self.addr,
            "POST",
            path,
            &[("Content-Type", "application/json")],
            &body.to_string(),
        )
        .await
    }

    pub async fn matchmake(&self, body: Value) -> Reply {
        self.post("/api/matchmaker/handle-matchmaking-for-player", body).await
    }

    /// The `FAKE_ARGS` record a game server wrote to its log (what it was started with).
    pub async fn args(&self, port: u16) -> Value {
        let log = self.log_of(port, "FAKE_ARGS ").await;
        let line = log.lines().find_map(|l| l.strip_prefix("FAKE_ARGS ")).unwrap();
        serde_json::from_str(line).unwrap()
    }

    /// The log file of the game server on `port`, once it holds `needle`.
    pub async fn log_of(&self, port: u16, needle: &str) -> String {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        loop {
            let text = std::fs::read_dir(&self.logs)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|e| e.file_name().to_string_lossy().starts_with(&format!("port{port}_")))
                .filter_map(|e| std::fs::read_to_string(e.path()).ok())
                .collect::<String>();
            if text.contains(needle) {
                return text;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "port {port}'s log never held {needle:?}: {text}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// `Startup`, then ready, bounded.
    pub async fn boot(&self) {
        if tokio::time::timeout(Duration::from_secs(40), self.director.startup())
            .await
            .is_err()
        {
            panic!(
                "the boot took over 40 s
{}",
                self.diagnostics().await
            );
        }
        self.director.set_persistent_hubs_ready(true);
    }

    /// What a failed step should say: the hubs' status as the API gives it and every game server's log.
    pub async fn diagnostics(&self) -> String {
        let mut text = format!(
            "ports {}..={}
hub-status {}
",
            self.begin,
            self.end,
            self.get("/api/hub-status").await.text
        );
        for entry in std::fs::read_dir(&self.logs).into_iter().flatten().flatten() {
            let log = std::fs::read_to_string(entry.path()).unwrap_or_default();
            text.push_str(&format!(
                "--- {:?}
{}
",
                entry.file_name(),
                log.chars().take(2000).collect::<String>()
            ));
        }
        text
    }

    pub async fn shutdown(&self) {
        within("the shutdown", self.director.shutdown()).await;
    }

    pub fn stop(&self, port: u16) {
        std::fs::write(self.control.join(format!("stop-{port}")), "").unwrap();
    }

    pub async fn until(&self, what: &str, check: impl Fn(&Director) -> bool) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        while !check(&self.director) {
            assert!(tokio::time::Instant::now() < deadline, "timed out waiting for {what}");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}
