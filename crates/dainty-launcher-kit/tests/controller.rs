// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The controller's whole flow (RD27 spec §2.10) against fake servers on loopback: plain HTTP for private mode
//! and a TLS gateway with a throwaway certificate for public mode. Its own test binary, because it logs to the
//! process-wide log and `log`'s own test reads its file exactly.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use dainty_protocol::invite::{Invite, ServerMode, dainty_version, format_invite};
use dainty_protocol::manifest::{RUNTIME_FILE_NAMES, RuntimeFile, RuntimeManifest};
use futures_util::future::BoxFuture;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use serde_json::json;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Notify;
use tokio_rustls::TlsAcceptor;

use dainty_launcher_kit::bridge::types::{
    ActionResult, ConnectProblem, ErrorCode, GameProblem, InstallOutcome, InstallState, InviteCheck, NoticeCode, Phase,
    RegisterOutcome, RuntimeState, SettingsPatch, Snapshot, TaskProgress,
};
use dainty_launcher_kit::controller::{
    ConsoleSink, Controller, ControllerConfig, FileFilter, FolderPurpose, Platform, PreparedGame, RelayOwner,
    STATUS_POLL_INTERVAL, Seams,
};
use dainty_launcher_kit::core::i18n::Language;
use dainty_launcher_kit::core::pins::sha256_hex;
use dainty_launcher_kit::platform::game_folder::StoreFolders;
use dainty_launcher_kit::platform::keystore::os::{Encryptor, SealError, open_rr1, seal_rr1};
use dainty_launcher_kit::platform::launch::{GameChild, LaunchPlan, MissingFiles, Spawner};
use dainty_launcher_kit::platform::log;
use dainty_launcher_kit::platform::paths::Paths;
use dainty_launcher_kit::platform::settings::Os;

const KEY: &str = "DLK_flowtestaccountkey0123456789abcdef";
/// The key a server that forgot [`KEY`] hands out to the next registration.
const NEW_KEY: &str = "DLK_flowtestreplacementkey0123456789abc";
const USER_ID: &str = "user-0001";
const VERSION_TEXT: &str = "dauntless_rel-1.14.7_Shipping_2024-04-01_14-59-21_647472";
const WAIT: Duration = Duration::from_secs(10);

// --- The process-wide log, for the key check ---

fn log_dir() -> &'static Path {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("launcher-kit-controller-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        log::set_folder(&dir);
        dir
    })
}

fn assert_key_never_logged() {
    let text = std::fs::read_to_string(log_dir().join(log::LOG_FILE)).unwrap_or_default();
    assert!(!text.contains(KEY), "the account key reached the log:\n{text}");
    assert!(!text.contains(&KEY[4..]), "the account key reached the log:\n{text}");
}

// --- A fake server ---

#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    head: String,
    body: String,
}

struct FakeServer {
    port: u16,
    seen: Arc<Mutex<Vec<Seen>>>,
}

#[derive(Clone)]
struct Answers {
    name: &'static str,
    manifest: Option<RuntimeManifest>,
    files: BTreeMap<String, Vec<u8>>,
    /// Set when the server's account store no longer knows [`KEY`].
    forgot: Arc<AtomicBool>,
}

fn response(status: u16, kind: &str, body: &[u8]) -> Vec<u8> {
    let mut out = format!(
        "HTTP/1.1 {status} X\r\ncontent-type: {kind}\r\ncontent-length: {}\r\n\r\n",
        body.len()
    )
    .into_bytes();
    out.extend_from_slice(body);
    out
}

fn json_response(status: u16, body: &serde_json::Value) -> Vec<u8> {
    response(status, "application/json", body.to_string().as_bytes())
}

fn answer(answers: &Answers, seen: &Seen) -> Vec<u8> {
    let path = seen.path.split('?').next().unwrap_or("");
    let head = seen.head.to_ascii_lowercase();
    let carries = |key: &str| head.contains(&format!("x-dainty-user-api-key: {}", key.to_ascii_lowercase()));
    let forgot = answers.forgot.load(Ordering::SeqCst);
    let (keyed, rekeyed) = (carries(KEY) && !forgot, carries(NEW_KEY) && forgot);
    // INVITECODE registration refuses a missing code before anything is spent.
    let has_code = seen.body.contains("\"INVITE-1234\"");
    match (seen.method.as_str(), path) {
        ("GET", "/dainty/api/ServerStatus") => json_response(
            200,
            &json!({ "name": answers.name, "online": true, "version": "1.0", "commit": "abc" }),
        ),
        ("POST", "/dainty/api/Register") if !has_code => json_response(401, &json!({ "error": "InviteInvalid" })),
        ("POST", "/dainty/api/Register") => json_response(200, &json!({ "UUK": if forgot { NEW_KEY } else { KEY } })),
        ("GET", "/dainty/api/GetUserInfo") if keyed => json_response(
            200,
            &json!({ "UserId": USER_ID, "Username": "Player_1", "IsAdmin": false }),
        ),
        ("GET", "/dainty/api/GetUserInfo") if rekeyed => json_response(
            200,
            &json!({ "UserId": "user-0002", "Username": "Player_2", "IsAdmin": false }),
        ),
        ("GET", "/dainty/api/GetUserInfo") => json_response(401, &json!({ "error": "key" })),
        ("GET", "/content/v1/runtime/manifest") => match &answers.manifest {
            Some(manifest) => response(200, "application/json", manifest.to_json().as_bytes()),
            None => json_response(404, &json!({ "error": "not_found" })),
        },
        ("GET", file) if file.starts_with("/content/v1/runtime/files/") => {
            let name = file.trim_start_matches("/content/v1/runtime/files/");
            match answers.files.get(name) {
                Some(bytes) => response(200, "application/octet-stream", bytes),
                None => json_response(404, &json!({})),
            }
        }
        ("GET", "/game/hello") => response(200, "text/plain", b"hello from the server"),
        _ => json_response(404, &json!({ "error": "not_found" })),
    }
}

async fn read_request<S: AsyncRead + Unpin>(stream: &mut S) -> Option<Seen> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).await.ok()? == 0 {
            return None;
        }
        buf.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&buf).into_owned();
    let length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0);
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body).await.ok()?;
    let mut first = head.lines().next().unwrap_or("").split(' ');
    Some(Seen {
        method: first.next().unwrap_or("").to_owned(),
        path: first.next().unwrap_or("").to_owned(),
        head,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

async fn serve<S: AsyncRead + AsyncWrite + Unpin>(mut stream: S, answers: Answers, seen: Arc<Mutex<Vec<Seen>>>) {
    while let Some(request) = read_request(&mut stream).await {
        let reply = answer(&answers, &request);
        seen.lock().unwrap().push(request);
        if stream.write_all(&reply).await.is_err() || stream.flush().await.is_err() {
            return;
        }
    }
}

async fn fake_server(answers: Answers, tls: Option<TlsAcceptor>) -> FakeServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else { return };
            let (answers, log, tls) = (answers.clone(), Arc::clone(&log), tls.clone());
            tokio::spawn(async move {
                match tls {
                    Some(tls) => {
                        if let Ok(stream) = tls.accept(tcp).await {
                            serve(stream, answers, log).await;
                        }
                    }
                    None => serve(tcp, answers, log).await,
                }
            });
        }
    });
    FakeServer { port, seen }
}

impl FakeServer {
    fn saw(&self, path: &str) -> Vec<Seen> {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.path == path)
            .cloned()
            .collect()
    }
}

/// A throwaway gateway certificate and its fingerprint.
fn gateway_tls() -> (TlsAcceptor, String) {
    let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_owned(), "127.0.0.1".to_owned()]).unwrap();
    let cert: CertificateDer<'static> = certified.cert.der().clone();
    let fingerprint = sha256_hex(cert.as_ref());
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(certified.signing_key.serialize_der()));
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .unwrap();
    (TlsAcceptor::from(Arc::new(config)), fingerprint)
}

// --- Fakes for the app ---

#[derive(Default)]
struct FakePlatform {
    snapshots: Mutex<Vec<Snapshot>>,
    progress: AtomicUsize,
    folder: Mutex<Option<PathBuf>>,
    save_file: Mutex<Option<PathBuf>>,
    console: Mutex<Option<(u32, bool, ConsoleSink)>>,
    game_log: Mutex<Vec<String>>,
}

impl Platform for FakePlatform {
    fn choose_folder(&self, _: Language, _: FolderPurpose, _: Option<PathBuf>) -> BoxFuture<'_, Option<PathBuf>> {
        let picked = self.folder.lock().unwrap().clone();
        Box::pin(async move { picked })
    }

    fn choose_save_file(&self, _: Language, _: String, _: FileFilter) -> BoxFuture<'_, Option<PathBuf>> {
        let picked = self.save_file.lock().unwrap().clone();
        Box::pin(async move { picked })
    }

    fn choose_open_file(&self, _: Language, _: FileFilter) -> BoxFuture<'_, Option<PathBuf>> {
        let picked = self.save_file.lock().unwrap().clone();
        Box::pin(async move { picked })
    }

    fn open_url(&self, _: &url::Url) -> std::io::Result<()> {
        Ok(())
    }

    fn open_folder(&self, _: &Path) -> std::io::Result<bool> {
        Ok(true)
    }

    fn emit_snapshot(&self, snapshot: &Snapshot) {
        self.snapshots.lock().unwrap().push(snapshot.clone());
    }

    fn emit_progress(&self, _: &TaskProgress) {
        self.progress.fetch_add(1, Ordering::SeqCst);
    }

    fn find_tailscale(&self) -> Option<PathBuf> {
        None
    }

    fn running_clients(&self) -> BoxFuture<'_, Vec<u32>> {
        Box::pin(async { Vec::new() })
    }

    fn prepare_launch<'a>(&'a self, _: &'a Path, _: bool) -> BoxFuture<'a, PreparedGame> {
        Box::pin(async { PreparedGame::Native })
    }

    fn missing_files(&self) -> MissingFiles {
        MissingFiles::default()
    }

    fn watch_game_console(&self, pid: u32, hide: bool, sink: ConsoleSink) {
        *self.console.lock().unwrap() = Some((pid, hide, sink));
    }

    fn emit_game_log(&self, lines: &[String]) {
        self.game_log.lock().unwrap().extend_from_slice(lines);
    }
}

struct FakeSealer;

impl Encryptor for FakeSealer {
    fn available(&self) -> bool {
        true
    }

    fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, SealError> {
        seal_rr1(&[9; 32], plain)
    }

    fn open(&self, file: &[u8]) -> Result<Vec<u8>, SealError> {
        open_rr1(&[9; 32], file)
    }
}

/// A game that runs until the test lets it exit.
#[derive(Default)]
struct FakeSpawner {
    plans: Mutex<Vec<LaunchPlan>>,
    exit: Arc<Notify>,
}

struct FakeChild(Arc<Notify>);

impl GameChild for FakeChild {
    fn id(&self) -> Option<u32> {
        Some(4242)
    }

    fn wait(self: Box<Self>) -> BoxFuture<'static, std::io::Result<Option<i32>>> {
        Box::pin(async move {
            self.0.notified().await;
            Ok(Some(0))
        })
    }
}

impl Spawner for FakeSpawner {
    fn spawn(&self, plan: &LaunchPlan) -> std::io::Result<Box<dyn GameChild>> {
        self.plans.lock().unwrap().push(plan.clone());
        Ok(Box::new(FakeChild(Arc::clone(&self.exit))))
    }
}

// --- One launcher on a temporary PC ---

struct Pc {
    _dir: tempfile::TempDir,
    root: PathBuf,
    win64: PathBuf,
    config_dir: PathBuf,
    user_data: PathBuf,
    documents: PathBuf,
    pin: String,
    relay_port: u16,
}

async fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    listener.local_addr().unwrap().port()
}

async fn pc() -> Pc {
    log_dir();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Dauntless");
    let win64 = root.join("Archon/Binaries/Win64");
    std::fs::create_dir_all(&win64).unwrap();
    let exe = b"a stand-in for the 1.14.7 executable";
    std::fs::write(win64.join("Dauntless-Win64-Shipping.exe"), exe).unwrap();
    std::fs::write(root.join("Version.txt"), VERSION_TEXT).unwrap();
    Pc {
        root,
        win64,
        config_dir: dir.path().join("Config"),
        user_data: dir.path().join("User Data"),
        documents: dir.path().join("Documents"),
        pin: sha256_hex(exe),
        relay_port: free_port().await,
        _dir: dir,
    }
}

fn launcher(pc: &Pc, platform: &Arc<FakePlatform>, spawner: &Arc<FakeSpawner>) -> Controller {
    launcher_polling_every(pc, platform, spawner, STATUS_POLL_INTERVAL)
}

fn launcher_polling_every(
    pc: &Pc,
    platform: &Arc<FakePlatform>,
    spawner: &Arc<FakeSpawner>,
    every: Duration,
) -> Controller {
    let paths = Paths {
        user_data: pc.user_data.clone(),
        documents: pc.documents.clone(),
        home: pc.user_data.clone(),
    };
    let mut config = ControllerConfig::new(paths, Arc::new(FakeSealer));
    config.relay_port = pc.relay_port;
    config.exe_pin = pc.pin.clone();
    config.game_config_dir = pc.config_dir.clone();
    config.os = Os::current();
    config.poll_interval = every;
    let env = BTreeMap::from([
        ("PATH".to_owned(), "/usr/bin".to_owned()),
        ("DAINTY_STALE_SETTING".to_owned(), "leak".to_owned()),
        ("rustic_isles_old".to_owned(), "leak".to_owned()),
    ]);
    let seams = Seams {
        spawner: Some(Arc::clone(spawner) as Arc<dyn Spawner>),
        game_env: Some(env),
        store_folders: Some(StoreFolders {
            legendary: Vec::new(),
            heroic: None,
            epic_manifests: None,
        }),
        qos_port: None,
        relay_owner: RelayOwner::Anyone,
        ..Seams::default()
    };
    Controller::new(config, Arc::clone(platform) as Arc<dyn Platform>, seams)
}

fn invite(mode: ServerMode, port: u16, fp: Option<String>, name: &str) -> String {
    format_invite(&Invite {
        version: dainty_version(),
        mode,
        host: "127.0.0.1".to_owned(),
        port,
        code: "INVITE-1234".to_owned(),
        name: name.to_owned(),
        share: None,
        fp,
    })
    .unwrap()
}

fn runtime(version: &str) -> (RuntimeManifest, BTreeMap<String, Vec<u8>>) {
    let mut files = BTreeMap::new();
    let mut listed = Vec::new();
    for name in RUNTIME_FILE_NAMES {
        let bytes = format!("{name} built for {version}").into_bytes();
        listed.push(RuntimeFile {
            name: name.to_owned(),
            size: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
        });
        files.insert(name.to_owned(), bytes);
    }
    (RuntimeManifest::new(version.to_owned(), 647_472, listed), files)
}

async fn until(what: &str, mut check: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + WAIT;
    while !check() {
        assert!(tokio::time::Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn relay_get(port: u16, path: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let request = format!("GET {path} HTTP/1.1\r\nhost: 127.0.0.1:{port}\r\nconnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut reply = Vec::new();
    let _ = tokio::time::timeout(WAIT, stream.read_to_end(&mut reply)).await;
    String::from_utf8_lossy(&reply).into_owned()
}

fn assert_key_not_in(snapshot: &Snapshot) {
    let text = serde_json::to_string(snapshot).unwrap();
    assert!(!text.contains(KEY), "the account key reached a snapshot");
}

fn ok(result: &ActionResult) {
    assert_eq!(*result, ActionResult::Ok);
}

fn refused(result: &ActionResult) -> ErrorCode {
    match result {
        ActionResult::Err(error) => error.code,
        ActionResult::Ok => panic!("expected a refusal"),
    }
}

#[tokio::test]
async fn private_flow_from_invite_to_the_games_exit() {
    let pc = pc().await;
    let (manifest, files) = runtime("2.0.0");
    let server = fake_server(
        Answers {
            name: "Flow Server",
            manifest: Some(manifest),
            files,
            forgot: Arc::default(),
        },
        None,
    )
    .await;
    let platform = Arc::new(FakePlatform::default());
    let spawner = Arc::new(FakeSpawner::default());
    let controller = launcher(&pc, &platform, &spawner);
    controller.init().await;
    assert_eq!(controller.snapshot().phase, Phase::Join);
    assert_eq!(controller.snapshot().game.problem, Some(GameProblem::NotSet));

    // Invite, then status.
    let text = invite(ServerMode::Private, server.port, None, "Flow Server");
    assert_eq!(
        controller.check_invite(&text),
        InviteCheck::Ok {
            certificate_changed: false,
            previous_fingerprint: None,
            moved_from: None
        }
    );
    ok(&controller.submit_invite(&text, false, false).await);
    let snapshot = controller.snapshot();
    assert_eq!(snapshot.phase, Phase::Register);
    assert_eq!(snapshot.status.as_ref().map(|s| s.name.as_str()), Some("Flow Server"));
    assert!(snapshot.server.as_ref().unwrap().has_pending_invite);
    assert!(snapshot.server.as_ref().unwrap().loopback);
    assert!(snapshot.connect.last_checked_at.is_some());

    // Register: the server takes the invite's code, and the key is stored.
    assert_eq!(
        controller.register("Player_1").await,
        RegisterOutcome::Ok {
            username: "Player_1".to_owned()
        }
    );
    let register = server.saw("/dainty/api/Register");
    assert_eq!(register.len(), 1);
    let snapshot = controller.snapshot();
    assert!(snapshot.account.has_key);
    assert!(snapshot.account.offer_backup);
    assert_eq!(snapshot.account.username.as_deref(), Some("Player_1"));
    assert!(!snapshot.server.as_ref().unwrap().has_pending_invite);
    assert_eq!(snapshot.phase, Phase::Locate);
    assert!(
        server
            .saw("/dainty/api/ServerStatus")
            .last()
            .unwrap()
            .head
            .contains("x-dainty-user-api-key"),
        "the status carries the key once one is stored"
    );

    // The key backup goes where the player says, with the key in it.
    let backup = pc.documents.join("backup.txt");
    *platform.save_file.lock().unwrap() = Some(backup.clone());
    ok(&controller.save_key_backup().await);
    assert!(
        std::fs::read_to_string(&backup)
            .unwrap()
            .contains(&format!("Key: {KEY}"))
    );
    assert_eq!(controller.snapshot().notice, Some(NoticeCode::BackupSaved));
    assert!(!controller.snapshot().account.offer_backup);

    // The game: a pasted path is verified against the pin at once.
    ok(&controller.use_game_path(&pc.root.to_string_lossy()).await);
    let snapshot = controller.snapshot();
    assert_eq!(snapshot.game.problem, None);
    assert!(snapshot.game.exe_verified);
    assert_eq!(snapshot.notice, Some(NoticeCode::GameVerified));
    assert!(platform.progress.load(Ordering::SeqCst) > 0);

    // The runtime: offered, untrusted, then trusted and installed, then checked.
    assert_eq!(snapshot.runtime.state, RuntimeState::Untrusted);
    assert_eq!(snapshot.runtime.version.as_deref(), Some("2.0.0"));
    assert_eq!(snapshot.phase, Phase::Runtime);
    assert_eq!(refused(&controller.play().await), ErrorCode::RuntimeUntrusted);
    ok(&controller.trust_runtime().await);
    let snapshot = controller.snapshot();
    assert_eq!(snapshot.runtime.state, RuntimeState::Ok);
    assert_eq!(snapshot.notice, Some(NoticeCode::RuntimeInstalled));
    assert_eq!(snapshot.phase, Phase::Ready);
    for name in RUNTIME_FILE_NAMES {
        assert!(pc.win64.join(name).is_file(), "{name} installed");
    }
    let downloads = server
        .seen
        .lock()
        .unwrap()
        .iter()
        .filter(|s| s.path.starts_with("/content/v1/runtime/files/"))
        .count();
    assert_eq!(downloads, RUNTIME_FILE_NAMES.len());

    // PLAY: the relay, Engine.ini, the arguments and the environment.
    ok(&controller.play().await);
    let snapshot = controller.snapshot();
    assert_eq!(snapshot.phase, Phase::Running);
    assert!(snapshot.running.game);
    assert_eq!(snapshot.running.relay_port, Some(pc.relay_port));
    assert_key_not_in(&snapshot);
    assert_eq!(
        controller.keep_open(),
        Some(dainty_launcher_kit::controller::KeepOpen::Relay)
    );

    let engine = std::fs::read_to_string(pc.config_dir.join("Engine.ini")).unwrap();
    assert!(engine.contains("ServerAddr=\"ws://127.0.0.1\""), "{engine}");
    assert!(engine.contains(&format!("ServerPort={}", pc.relay_port)), "{engine}");

    let plan = spawner.plans.lock().unwrap().last().cloned().unwrap();
    let args: Vec<String> = plan.args.iter().map(|a| a.to_string_lossy().into_owned()).collect();
    assert!(args.contains(&format!("-AUTH_PASSWORD={KEY}")));
    assert!(args.contains(&format!("-AUTH_LOGIN={USER_ID}")));
    assert!(args.contains(&"-epicusername=Player_1".to_owned()));
    assert!(args.contains(&"-epiclocale=en-US".to_owned()));
    assert_eq!(plan.working_dir, pc.win64);
    assert_eq!(
        plan.env.get("DAINTY_BACKEND_URL").map(String::as_str),
        Some(format!("http://127.0.0.1:{}", pc.relay_port).as_str())
    );
    assert_eq!(
        plan.env.get("RUSTIC_ISLES_RUNTIME_PROFILE").map(String::as_str),
        Some("production")
    );
    assert!(!plan.env.contains_key("DAINTY_STALE_SETTING"));
    assert!(!plan.env.contains_key("rustic_isles_old"));
    assert_eq!(plan.env.get("PATH").map(String::as_str), Some("/usr/bin"));

    let logged = std::fs::read_to_string(log_dir().join(log::LOG_FILE)).unwrap();
    assert!(
        logged.contains("-AUTH_PASSWORD=<hidden>"),
        "the launch line is logged, masked"
    );
    assert!(
        logged.contains("relay: relay listening on 127.0.0.1"),
        "relay lines carry their prefix"
    );

    // The game reaches the server through the relay, which adds the key on the way.
    let reply = relay_get(pc.relay_port, "/game/hello").await;
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    assert!(reply.contains("hello from the server"));
    assert_eq!(server.saw("/game/hello").len(), 1);

    // The Game log: console lines kept and emitted redacted.
    let (pid, hide, sink) = platform.console.lock().unwrap().take().unwrap();
    assert_eq!(pid, 4242);
    assert!(hide, "the console is hidden unless showGameConsole");
    sink(vec![
        "LogInit: started".to_owned(),
        format!("LogInit: Command Line: -AUTH_PASSWORD={KEY} -epicapp=Archon"),
    ]);
    let kept = controller.game_log();
    assert_eq!(kept.len(), 2);
    assert_eq!(kept[0], "LogInit: started");
    assert!(!kept[1].contains(KEY), "{}", kept[1]);
    assert_eq!(*platform.game_log.lock().unwrap(), kept);

    // Busy while the game runs.
    assert_eq!(refused(&controller.play().await), ErrorCode::AlreadyRunning);
    assert_eq!(refused(&controller.logout().await), ErrorCode::Busy);

    // The game's exit stops the relay.
    spawner.exit.notify_one();
    until("the relay to stop", || {
        controller.snapshot().running.relay_port.is_none()
    })
    .await;
    until("the game to stop", || !controller.snapshot().running.game).await;
    assert!(TcpStream::connect(("127.0.0.1", pc.relay_port)).await.is_err());
    assert_eq!(controller.keep_open(), None);
    assert_eq!(controller.snapshot().phase, Phase::Ready);
    assert_eq!(controller.snapshot().last_error, None);

    // Logout forgets the key.
    ok(&controller.logout().await);
    let snapshot = controller.snapshot();
    assert!(!snapshot.account.has_key);
    assert_eq!(snapshot.phase, Phase::Register);

    controller.shutdown().await;
    for snapshot in platform.snapshots.lock().unwrap().iter() {
        assert_key_not_in(snapshot);
    }
    assert_key_never_logged();
}

#[tokio::test]
async fn public_flow_through_a_tls_gateway() {
    let pc = pc().await;
    let (tls, fingerprint) = gateway_tls();
    let server = fake_server(
        Answers {
            name: "Public Server",
            manifest: None,
            files: BTreeMap::new(),
            forgot: Arc::default(),
        },
        Some(tls),
    )
    .await;
    let platform = Arc::new(FakePlatform::default());
    let spawner = Arc::new(FakeSpawner::default());
    let controller = launcher(&pc, &platform, &spawner);
    controller.init().await;

    let text = invite(
        ServerMode::Public,
        server.port,
        Some(fingerprint.clone()),
        "Public Server",
    );
    ok(&controller.submit_invite(&text, false, false).await);
    let snapshot = controller.snapshot();
    assert_eq!(snapshot.phase, Phase::Register, "{:?}", snapshot.connect);
    assert_eq!(
        snapshot.server.as_ref().unwrap().fingerprint.as_deref(),
        Some(fingerprint.as_str())
    );
    assert!(matches!(
        controller.register("Player_1").await,
        RegisterOutcome::Ok { .. }
    ));
    assert!(controller.snapshot().account.has_key);

    // Another certificate for the same address: flagged, and refused without consent.
    let other = "ab".repeat(32);
    let moved = invite(ServerMode::Public, server.port, Some(other.clone()), "Public Server");
    assert_eq!(
        controller.check_invite(&moved),
        InviteCheck::Ok {
            certificate_changed: true,
            previous_fingerprint: Some(fingerprint.clone()),
            moved_from: None
        }
    );
    assert_eq!(
        refused(&controller.submit_invite(&moved, false, false).await),
        ErrorCode::CertChanged
    );
    assert_eq!(
        controller.snapshot().server.unwrap().fingerprint.as_deref(),
        Some(fingerprint.as_str()),
        "a refused change changes nothing"
    );

    // A runtime already in place and none offered: PLAY may go, once nothing is left beside the loader (RD9).
    ok(&controller.use_game_path(&pc.root.to_string_lossy()).await);
    for name in RUNTIME_FILE_NAMES {
        std::fs::write(pc.win64.join(name), name.as_bytes()).unwrap();
    }
    std::fs::write(pc.win64.join("mystic_loader.ini"), "Dumper-7.dll\n").unwrap();
    assert_eq!(refused(&controller.play().await), ErrorCode::LoaderLeftovers);
    std::fs::remove_file(pc.win64.join("mystic_loader.ini")).unwrap();

    // Over TLS through the relay, pinned to the invite's certificate.
    ok(&controller.play().await);
    let reply = relay_get(pc.relay_port, "/game/hello").await;
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    let forwarded = server.saw("/game/hello");
    assert_eq!(forwarded.len(), 1);
    assert!(
        forwarded[0].head.contains(&format!("host: 127.0.0.1:{}", server.port)),
        "{}",
        forwarded[0].head
    );

    spawner.exit.notify_one();
    until("the relay to stop", || {
        controller.snapshot().running.relay_port.is_none()
    })
    .await;
    controller.shutdown().await;
    assert_key_not_in(&controller.snapshot());
    assert_key_never_logged();
}

#[tokio::test]
async fn an_unreachable_server_and_a_mismatched_certificate() {
    let pc = pc().await;
    let (tls, _) = gateway_tls();
    let server = fake_server(
        Answers {
            name: "Pinned",
            manifest: None,
            files: BTreeMap::new(),
            forgot: Arc::default(),
        },
        Some(tls),
    )
    .await;
    let platform = Arc::new(FakePlatform::default());
    let spawner = Arc::new(FakeSpawner::default());
    let controller = launcher(&pc, &platform, &spawner);
    controller.init().await;

    let wrong = invite(ServerMode::Public, server.port, Some("cd".repeat(32)), "Pinned");
    ok(&controller.submit_invite(&wrong, false, false).await);
    let snapshot = controller.snapshot();
    assert_eq!(snapshot.phase, Phase::Connect);
    assert_eq!(snapshot.connect.problem, Some(ConnectProblem::CertMismatch));

    let nobody = free_port().await;
    let gone = invite(ServerMode::Private, nobody, None, "Gone");
    ok(&controller.submit_invite(&gone, false, false).await);
    assert_eq!(controller.snapshot().connect.problem, Some(ConnectProblem::Unreachable));
    assert_eq!(controller.snapshot().saved_servers.len(), 2);
    assert_eq!(refused(&controller.play().await), ErrorCode::KeyInvalid);
    controller.shutdown().await;
}

#[tokio::test]
async fn an_unreachable_saved_server_is_checked_less_and_less_often() {
    let pc = pc().await;
    // Accepts and hangs up: never a status, so every check fails without waiting on a timeout
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let accepted = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&accepted);
    let refuser = tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            count.fetch_add(1, Ordering::SeqCst);
            drop(stream);
        }
    });
    let platform = Arc::new(FakePlatform::default());
    let spawner = Arc::new(FakeSpawner::default());
    let controller = launcher_polling_every(&pc, &platform, &spawner, Duration::from_millis(40));
    controller.init().await;
    ok(&controller
        .submit_invite(&invite(ServerMode::Private, port, None, "Moved"), false, false)
        .await);
    assert_ne!(controller.snapshot().connect.problem, None);
    let after_submit = accepted.load(Ordering::SeqCst);

    controller.set_status_polling(true);
    // 25 ticks: one check per tick would be 25; backing off checks on ticks 0, 1, 3, 7 and 15
    tokio::time::sleep(Duration::from_millis(40 * 25)).await;
    controller.set_status_polling(false);
    let checks = accepted.load(Ordering::SeqCst) - after_submit;
    assert!((3..=10).contains(&checks), "{checks} checks");
    controller.shutdown().await;
    refuser.abort();
}

#[tokio::test]
async fn settings_the_tray_flags_and_the_game_log() {
    let pc = pc().await;
    let platform = Arc::new(FakePlatform::default());
    let spawner = Arc::new(FakeSpawner::default());
    let controller = launcher(&pc, &platform, &spawner);
    controller.init().await;
    assert_eq!(controller.tray(), (false, false));

    let patch: SettingsPatch = serde_json::from_value(json!({
        "graphics": 3,
        "windowed": true,
        "language": "pt-BR",
        "gameLanguage": "de-DE",
        "minimizeToTray": true,
        "closeToTray": true,
        "showGameConsole": true,
        "gameEdition": "legacy",
    }))
    .unwrap();
    let settings = controller.set_settings(patch).await;
    assert_eq!(settings.graphics.get(), 3);
    assert!(settings.windowed && settings.minimize_to_tray && settings.close_to_tray && settings.show_game_console);
    assert_eq!(settings.language, Language::PtBr);
    assert_eq!(settings.game_edition, dainty_launcher_kit::bridge::types::Game::Reborn);
    assert_eq!(controller.tray(), (true, true));
    assert_eq!(controller.language(), Language::PtBr);
    assert_eq!(controller.snapshot().settings, settings);

    let file = std::fs::read_to_string(pc.user_data.join("settings.json")).unwrap();
    let written: serde_json::Value = serde_json::from_str(&file).unwrap();
    assert_eq!(written["minimizeToTray"], json!(true));
    assert_eq!(written["closeToTray"], json!(true));
    assert_eq!(written["gameLanguage"], json!("de-DE"));

    let off: SettingsPatch = serde_json::from_value(json!({ "minimizeToTray": false })).unwrap();
    controller.set_settings(off).await;
    assert_eq!(controller.tray(), (false, true));
    let file = std::fs::read_to_string(pc.user_data.join("settings.json")).unwrap();
    assert!(!file.contains("minimizeToTray"), "a false flag is not written");

    assert!(controller.game_log().is_empty());
    controller.shutdown().await;
}

#[tokio::test]
async fn saved_1_4_4_servers_are_shown_disabled() {
    let pc = pc().await;
    std::fs::create_dir_all(&pc.user_data).unwrap();
    let old = json!({
        "version": 1,
        "server": null,
        "savedServers": [
            { "game": "revived", "mode": "private", "host": "100.64.0.9", "port": 61000, "name": "Old Times" },
            { "game": "reforged", "mode": "private", "host": "100.64.0.10", "port": 61000, "name": "New Times" }
        ]
    });
    std::fs::write(pc.user_data.join("settings.json"), old.to_string()).unwrap();
    let platform = Arc::new(FakePlatform::default());
    let spawner = Arc::new(FakeSpawner::default());
    let controller = launcher(&pc, &platform, &spawner);
    controller.init().await;

    let saved = controller.snapshot().saved_servers;
    assert_eq!(saved.len(), 2);
    assert!(saved[0].unsupported);
    assert!(!saved[1].unsupported);
    assert!(saved.iter().all(|s| !s.current));

    match controller.switch_server(&saved[0].id).await {
        ActionResult::Err(error) => {
            assert_eq!(error.code, ErrorCode::InviteInvalidFormat);
            assert_eq!(error.detail.as_deref(), Some("revived"));
        }
        ActionResult::Ok => panic!("a 1.4.4 server cannot be joined"),
    }
    assert_eq!(
        refused(&controller.switch_server("000000000000000000000000").await),
        ErrorCode::InviteInvalidFormat
    );
    ok(&controller.remove_saved_server(&saved[0].id).await);
    assert_eq!(controller.snapshot().saved_servers.len(), 1);
    controller.shutdown().await;
}

#[tokio::test]
async fn the_moved_modules_answer_without_a_server() {
    let pc = pc().await;
    let platform = Arc::new(FakePlatform::default());
    let spawner = Arc::new(FakeSpawner::default());
    let controller = launcher(&pc, &platform, &spawner);
    controller.init().await;
    let error = |value: serde_json::Value, code: &str, detail: Option<&str>| {
        assert_eq!(value["ok"], json!(false), "{value}");
        assert_eq!(value["error"]["code"], json!(code), "{value}");
        assert_eq!(value["error"].get("detail").and_then(|d| d.as_str()), detail, "{value}");
    };
    let realms = serde_json::to_value(controller.realm_get().await).unwrap();
    if cfg!(feature = "realms") {
        assert_eq!(
            realms,
            json!({ "ok": true, "realm": { "available": false, "active": null, "sessions": [] } })
        );
    } else {
        error(realms, "not_in_this_build", Some("realm_get"));
    }
    error(
        serde_json::to_value(controller.region_ping().await).unwrap(),
        "server_unreachable",
        None,
    );
    match controller.game_install_get("1.14.7") {
        InstallOutcome::Ok(info) => assert_eq!(info.state, InstallState::NotInstalled),
        InstallOutcome::Err(error) => panic!("{error:?}"),
    }
    error(
        serde_json::to_value(controller.game_install_get("9.9.9")).unwrap(),
        "game_version_unknown",
        Some("9.9.9"),
    );
    assert_eq!(refused(&controller.game_install_pause("1.14.7")), ErrorCode::Unknown);
    assert_eq!(refused(&controller.revived_choose_folder().await), ErrorCode::Unknown);
    let guarded = serde_json::to_value(controller.guarded_list().await).unwrap();
    if cfg!(feature = "server-gui") {
        error(guarded, "host_failed", Some("workspace"));
    } else {
        error(guarded, "not_in_this_build", Some("guarded_list"));
    }
    let host = serde_json::to_value(controller.host_server_start().await).unwrap();
    assert!(!controller.host_kit().await.workspace_ok);
    let ensured = serde_json::to_value(controller.ensure_host_workspace(false).await).unwrap();
    if cfg!(feature = "host") {
        error(host, "host_failed", Some("workspace"));
        error(ensured, "host_failed", Some("project_no_game"));
    } else {
        error(host, "host_failed", Some("host_tools_unavailable"));
        assert_eq!(ensured, json!({ "ok": true }));
    }
    controller.shutdown().await;
}

#[tokio::test]
async fn a_fresh_invite_replaces_a_key_the_server_forgot() {
    let pc = pc().await;
    let forgot = Arc::new(AtomicBool::new(false));
    let server = fake_server(
        Answers {
            name: "Forgetful",
            manifest: None,
            files: BTreeMap::new(),
            forgot: Arc::clone(&forgot),
        },
        None,
    )
    .await;
    let platform = Arc::new(FakePlatform::default());
    let spawner = Arc::new(FakeSpawner::default());
    let controller = launcher(&pc, &platform, &spawner);
    controller.init().await;
    let text = invite(ServerMode::Private, server.port, None, "Forgetful");
    ok(&controller.submit_invite(&text, false, false).await);
    assert!(matches!(
        controller.register("Player_1").await,
        RegisterOutcome::Ok { .. }
    ));
    assert!(!controller.snapshot().server.unwrap().has_pending_invite);

    // The server's account store loses the key: the launcher asks for an account again.
    forgot.store(true, Ordering::SeqCst);
    controller.refresh_status().await;
    let snapshot = controller.snapshot();
    assert_eq!(snapshot.phase, Phase::Register);
    assert_eq!(
        snapshot.last_error.map(|error| error.code),
        Some(ErrorCode::KeyRejected)
    );

    // A fresh invite for the same server keeps its code although a (dead) key is stored.
    ok(&controller.submit_invite(&text, false, false).await);
    let snapshot = controller.snapshot();
    assert_eq!(snapshot.phase, Phase::Register);
    assert!(snapshot.server.unwrap().has_pending_invite);
    assert_eq!(
        controller.register("Player_2").await,
        RegisterOutcome::Ok {
            username: "Player_2".to_owned()
        }
    );
    let register = server.saw("/dainty/api/Register");
    assert_eq!(register.len(), 2);
    assert!(
        register[1].body.contains(r#""InviteCode":"INVITE-1234""#),
        "{}",
        register[1].body
    );

    // The new key replaced the dead one, and the server takes it.
    let snapshot = controller.snapshot();
    assert_ne!(snapshot.phase, Phase::Register);
    assert!(snapshot.account.has_key);
    assert_eq!(snapshot.account.username.as_deref(), Some("Player_2"));
    assert!(!snapshot.server.unwrap().has_pending_invite);
    controller.refresh_status().await;
    let asked = server.saw("/dainty/api/GetUserInfo");
    let last = asked.last().unwrap().head.to_ascii_lowercase();
    assert!(last.contains(&NEW_KEY.to_ascii_lowercase()));
    assert_eq!(controller.snapshot().phase, snapshot.phase);

    // Rejoining while the key works leaves no code behind.
    ok(&controller.submit_invite(&text, false, false).await);
    assert!(!controller.snapshot().server.unwrap().has_pending_invite);
    controller.shutdown().await;
}
