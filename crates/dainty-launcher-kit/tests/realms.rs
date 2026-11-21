// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Realm hosting and joining tests (`realms` feature, RF7): drives `Controller::realm_host`,
//! `realm_join`, `realm_stop`, and `realm_get` against a scripted [`TransportAbi`] (the MIT `rifts-host` links its
//! transport in, RD27, so there is no C ABI to fake), checks RD9 (no injection or loader `.ini` written by realms,
//! and leftover loader files refused by both `realm_host` and `play`), and verifies that the account key in
//! `KeyStore` never reaches `rifts-host`, `p2p-sessions/`, or the dedicated server launch environment.

#![cfg(feature = "realms")]
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use dainty_protocol::invite::{Invite, ServerMode, dainty_version, format_invite};
use dainty_protocol::manifest::{RUNTIME_FILE_NAMES, RuntimeFile, RuntimeManifest};
use futures_util::future::BoxFuture;
use rustic_rifts_host::{
    ADMISSION_PUBLIC_KEY_FILE, DefaultHostBackend, Handle, HostBackend, HostError, HostOpen, JoinOpen,
    NoopServerSpawner, ReadyState, SESSION_FILES, ServerSpawner, SessionResult, Signal, TransportAbi, TransportLoader,
};
use rustic_rifts_seam::{ADMISSION_KEY_PATH, AccountId, PeerId, Secret};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use dainty_launcher_kit::bridge::types::{
    ActionResult, ErrorCode, LauncherError, RealmKind, RealmOutcome, RealmState, Snapshot,
};
use dainty_launcher_kit::controller::realms::RealmsSeams;
use dainty_launcher_kit::controller::{
    Controller, ControllerConfig, FileFilter, FolderPurpose, Platform, PreparedGame, RelayOwner, Seams,
};
use dainty_launcher_kit::core::i18n::Language;
use dainty_launcher_kit::core::pins::sha256_hex;
use dainty_launcher_kit::platform::game_folder::StoreFolders;
use dainty_launcher_kit::platform::keystore::os::{Encryptor, SealError, open_rr1, seal_rr1};
use dainty_launcher_kit::platform::launch::MissingFiles;
use dainty_launcher_kit::platform::loader_session::{EXPORTER_DLL, LOADER_INI, RECORD_FILE, leftovers};
use dainty_launcher_kit::platform::paths::Paths;
use dainty_launcher_kit::platform::settings::Os;

const VERSION_TEXT: &str = "dauntless_rel-1.14.7_Shipping_2024-04-01_14-59-21_647472";
const TEST_HUB_SESSION_UUID: &str = "018f0000-0000-7000-8000-000000000001";

// --- A server with accounts, a runtime and the realm routes ---

#[derive(Default)]
struct Host {
    name: &'static str,
    /// Account key → (username, user id).
    users: HashMap<String, (String, String)>,
    runtime: Option<(RuntimeManifest, BTreeMap<String, Vec<u8>>)>,
    /// What `ADMISSION_KEY_PATH` answers a signed-in launcher; 404 when `None`.
    admission_key: Option<String>,
    seen: Vec<String>,
}

type Shared = Arc<Mutex<Host>>;

struct Reply {
    status: u16,
    kind: &'static str,
    body: Vec<u8>,
}

fn reply(status: u16, body: Value) -> Reply {
    Reply {
        status,
        kind: "application/json",
        body: body.to_string().into_bytes(),
    }
}

fn answer(host: &Shared, method: &str, path: &str, key: Option<&str>) -> Reply {
    let h = host.lock().unwrap();
    match (method, path) {
        ("GET", "/dainty/api/ServerStatus") => {
            return reply(
                200,
                json!({ "name": h.name, "online": true, "version": "1.0", "commit": "abc" }),
            );
        }
        ("GET", "/dainty/api/GetUserInfo") => {
            return match key.and_then(|k| h.users.get(k)) {
                Some((name, id)) => reply(200, json!({ "UserId": id, "Username": name, "IsAdmin": false })),
                None => reply(401, json!({ "error": "key" })),
            };
        }
        ("GET", "/content/v1/runtime/manifest") => {
            return match &h.runtime {
                Some((manifest, _)) => Reply {
                    status: 200,
                    kind: "application/json",
                    body: manifest.to_json().into_bytes(),
                },
                None => reply(404, json!({ "error": "not_found" })),
            };
        }
        _ => {}
    }
    if let Some(name) = path.strip_prefix("/content/v1/runtime/files/") {
        return match h.runtime.as_ref().and_then(|(_, files)| files.get(name)) {
            Some(bytes) => Reply {
                status: 200,
                kind: "application/octet-stream",
                body: bytes.clone(),
            },
            None => reply(404, json!({})),
        };
    }
    let user = key.and_then(|k| h.users.get(k).cloned());
    if path == ADMISSION_KEY_PATH && user.is_some() {
        return match &h.admission_key {
            Some(body) => Reply {
                status: 200,
                kind: "application/json",
                body: body.clone().into_bytes(),
            },
            None => reply(404, json!({ "error": "not_found" })),
        };
    }
    drop(h);
    handle_realms_route(method, path, user).unwrap_or_else(|| reply(404, json!({ "error": "not_found" })))
}

fn handle_realms_route(method: &str, path: &str, user: Option<(String, String)>) -> Option<Reply> {
    if !path.starts_with("/launcher/v1/") && !path.starts_with("/rustic/v1/") {
        return None;
    }
    if path.starts_with("/launcher/v1/") && user.is_none() {
        return Some(reply(
            401,
            json!({ "error": { "code": "AUTH_UNAUTHORIZED", "message": "Sign in required." } }),
        ));
    }
    let (_, user_id) = user.unwrap_or_else(|| ("Host".to_owned(), "user-slayer-1".to_owned()));

    match (method, path) {
        ("GET", "/launcher/v1/realms/mine") => Some(reply(200, json!({ "realm": null }))),
        ("POST", "/launcher/v1/realms") => Some(reply(
            201,
            json!({
                "realm": {
                    "id": "realm-1",
                    "name": "slayerrealm",
                    "joinPolicy": "PUBLIC",
                    "listed": true
                }
            }),
        )),
        ("POST", "/launcher/v1/realms/mine/heartbeat") => Some(reply(200, json!({ "ok": true }))),
        ("POST", "/launcher/v1/p2p/host-presence") => Some(reply(200, json!({ "eligible": true }))),
        ("POST", "/launcher/v1/realm-hosting/hubs/start") => Some(reply(
            200,
            json!({
                "keyId": "local-key-1",
                "sessionId": TEST_HUB_SESSION_UUID,
                "partyId": "realm-hub:realm-1:ramsgate",
                "partyRevision": 1,
                "huntDefinitionId": "ShatteredIsles_ReturnToRamsgate",
                "generation": 1,
                "build": 647472,
                "bridgeProtocol": 1,
                "virtualPort": 1,
                "hostPeerId": "100001",
                "bridgeInstanceId": "bridge-hub-0001",
                "metadataNonce": "0123456789abcdef0123456789abcdef",
                "forceRelay": false,
                "guardCompletionMode": "NO_COMPLETION",
                "capability": "cap-hub-1",
                "hostTicket": "MPT2.hostticket",
                "serverUdpPort": 8791,
                "maximumPeers": 16,
                "hostLocalClient": true,
                "huntEvidence": false,
                "matchmakerHuntId": "ShatteredIsles_ReturnToRamsgate",
                "mapPath": "/Game/Maps/ramsgate/ramsgate_01_persistent",
                "behemothPath": "NO_BEHEMOTH",
                "expectedPlayers": "1",
                "runtimeProfile": "production",
                "maxPlayers": 16,
                "hubAfkSeconds": 600
            }),
        )),
        ("GET", "/launcher/v1/realms/listed") => Some(reply(
            200,
            json!({
                "realms": [{ "id": "realm-1", "name": "slayerrealm" }]
            }),
        )),
        ("POST", "/launcher/v1/realms/join") => Some(reply(200, json!({ "joined": true }))),
        ("POST", "/launcher/v1/realms/presence/heartbeat") => Some(reply(200, json!({ "ok": true }))),
        ("POST", "/launcher/v1/realm-hosting/hubs/peers") => Some(reply(200, json!({ "peers": [] }))),
        ("POST", "/launcher/v1/realm-hosting/hubs/stop") => Some(reply(
            200,
            json!({
                "stopped": true,
                "hubKind": "RAMSGATE",
                "sessionId": TEST_HUB_SESSION_UUID
            }),
        )),
        ("GET", "/launcher/v1/p2p/control") => Some(reply(
            200,
            json!({
                "messages": [{ "id": "ctrl-msg-1", "type": "REALM_HUB_STOPPED" }]
            }),
        )),
        _ => {
            if let Some(rest) = path.strip_prefix("/rustic/v1/hunts/") {
                if rest.ends_with("/host-ready") {
                    return Some(reply(
                        200,
                        json!({
                            "ready": true,
                            "sessionId": TEST_HUB_SESSION_UUID,
                            "generation": 1,
                            "state": "HOST_READY"
                        }),
                    ));
                }
                if rest.ends_with("/tickets/consume") {
                    return Some(reply(200, json!({ "admitted": true })));
                }
            }
            if let Some(rest) = path.strip_prefix("/launcher/v1/p2p/hunts/") {
                if rest.ends_with("/transport-config") {
                    let is_host = user_id == "user-slayer-1";
                    let peers = if is_host {
                        json!([])
                    } else {
                        json!([{
                            "accountId": "user-slayer-1",
                            "peerId": "100001",
                            "transportKey": "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef_"
                        }])
                    };
                    return Some(reply(
                        200,
                        json!({
                            "sessionId": TEST_HUB_SESSION_UUID,
                            "generation": 1,
                            "localAccountId": user_id,
                            "localPeerId": if is_host { "100001" } else { "100002" },
                            "hostAccountId": "user-slayer-1",
                            "hostPeerId": "100001",
                            "iceServers": [
                                { "urls": ["stun:127.0.0.1:3478"] }
                            ],
                            "peers": peers
                        }),
                    ));
                }
                if rest.ends_with("/route-ready") {
                    return Some(reply(200, json!({ "ready": true })));
                }
                if rest.ends_with("/join-bootstrap") {
                    return Some(reply(
                        200,
                        json!({
                            "keyId": "local-key-1",
                            "sessionId": TEST_HUB_SESSION_UUID,
                            "sessionRole": "HUB",
                            "partyId": "realm-hub:realm-1:ramsgate",
                            "partyRevision": 1,
                            "huntDefinitionId": "ShatteredIsles_ReturnToRamsgate",
                            "generation": 1,
                            "build": 647472,
                            "bridgeProtocol": 1,
                            "virtualPort": 1,
                            "hostPeerId": "100001",
                            "bridgeInstanceId": "bridge-hub-0001",
                            "metadataNonce": "0123456789abcdef0123456789abcdef",
                            "forceRelay": false,
                            "guardCompletionMode": "NO_COMPLETION",
                            "ticket": "MPT2.jointicket"
                        }),
                    ));
                }
                if rest.ends_with("/signal") {
                    return Some(reply(200, json!({ "accepted": true })));
                }
                if rest.ends_with("/peers") {
                    return Some(reply(200, json!({ "generation": 1, "peers": [] })));
                }
            }
            if path.starts_with("/launcher/v1/p2p/control/") && path.ends_with("/ack") {
                return Some(reply(200, json!({ "acknowledged": true })));
            }
            Some(reply(
                404,
                json!({ "error": { "code": "P2P_SESSION_NOT_FOUND", "message": "Session not found." } }),
            ))
        }
    }
}

async fn serve(mut stream: TcpStream, host: Shared) {
    loop {
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            if stream.read(&mut byte).await.unwrap_or(0) == 0 {
                return;
            }
            head.push(byte[0]);
        }
        let head = String::from_utf8_lossy(&head).into_owned();
        let header = |wanted: &str| {
            head.lines().skip(1).find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.trim()
                    .eq_ignore_ascii_case(wanted)
                    .then(|| value.trim().to_owned())
            })
        };
        let length = header("content-length")
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or(0);
        let mut body = vec![0u8; length];
        if stream.read_exact(&mut body).await.is_err() {
            return;
        }
        let mut first = head.lines().next().unwrap_or("").split(' ');
        let method = first.next().unwrap_or("").to_owned();
        let target = first.next().unwrap_or("");
        let path = target.split('?').next().unwrap_or("").to_owned();
        let key = header("x-dainty-user-api-key");
        let out = answer(&host, &method, &path, key.as_deref());
        host.lock().unwrap().seen.push(path);
        let mut bytes = format!(
            "HTTP/1.1 {} X\r\ncontent-type: {}\r\ncontent-length: {}\r\n\r\n",
            out.status,
            out.kind,
            out.body.len()
        )
        .into_bytes();
        bytes.extend_from_slice(&out.body);
        if stream.write_all(&bytes).await.is_err() {
            return;
        }
    }
}

async fn lan(host: Host) -> (Shared, u16) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let shared: Shared = Arc::new(Mutex::new(host));
    let served = Arc::clone(&shared);
    tokio::spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            tokio::spawn(serve(tcp, Arc::clone(&served)));
        }
    });
    (shared, port)
}

fn make_runtime(build: u32) -> (RuntimeManifest, BTreeMap<String, Vec<u8>>) {
    let mut files = BTreeMap::new();
    let mut listed = Vec::new();
    for name in RUNTIME_FILE_NAMES {
        let bytes = format!("{name} build {build}").into_bytes();
        listed.push(RuntimeFile {
            name: name.to_owned(),
            size: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
        });
        files.insert(name.to_owned(), bytes);
    }
    (RuntimeManifest::new(format!("1.0.{build}"), 647_472, listed), files)
}

fn invite(port: u16, name: &str) -> String {
    format_invite(&Invite {
        version: dainty_version(),
        mode: ServerMode::Private,
        host: "127.0.0.1".to_owned(),
        port,
        code: "INVITE-1234".to_owned(),
        name: name.to_owned(),
        share: None,
        fp: None,
    })
    .unwrap()
}

// --- One launcher on a temporary PC ---

struct NoDialogs;

impl Platform for NoDialogs {
    fn choose_folder(&self, _: Language, _: FolderPurpose, _: Option<PathBuf>) -> BoxFuture<'_, Option<PathBuf>> {
        Box::pin(async { None })
    }

    fn choose_save_file(&self, _: Language, _: String, _: FileFilter) -> BoxFuture<'_, Option<PathBuf>> {
        Box::pin(async { None })
    }

    fn choose_open_file(&self, _: Language, _: FileFilter) -> BoxFuture<'_, Option<PathBuf>> {
        Box::pin(async { None })
    }

    fn emit_snapshot(&self, _: &Snapshot) {}

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
}

struct FakeSealer;

impl Encryptor for FakeSealer {
    fn available(&self) -> bool {
        true
    }

    fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, SealError> {
        seal_rr1(&[3; 32], plain)
    }

    fn open(&self, file: &[u8]) -> Result<Vec<u8>, SealError> {
        open_rr1(&[3; 32], file)
    }
}

struct Harness {
    _dir: tempfile::TempDir,
    root: PathBuf,
    user: PathBuf,
    c: Controller,
}

impl Harness {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("Dauntless");
        let win64 = root.join("Archon/Binaries/Win64");
        std::fs::create_dir_all(&win64).unwrap();
        let exe = b"a stand-in for the 1.14.7 executable";
        std::fs::write(win64.join("Dauntless-Win64-Shipping.exe"), exe).unwrap();
        std::fs::write(root.join("Version.txt"), VERSION_TEXT).unwrap();
        let user = dir.path().join("User Data");
        let paths = Paths {
            user_data: user.clone(),
            documents: dir.path().join("Documents"),
            home: user.clone(),
        };
        let mut config = ControllerConfig::new(paths, Arc::new(FakeSealer));
        let relay = TcpListener::bind("127.0.0.1:0").await.unwrap();
        config.relay_port = relay.local_addr().unwrap().port();
        drop(relay);
        config.exe_pin = sha256_hex(exe);
        config.game_config_dir = dir.path().join("Config");
        config.os = Os::current();
        let seams = Seams {
            store_folders: Some(StoreFolders {
                legendary: Vec::new(),
                heroic: None,
                epic_manifests: None,
            }),
            qos_port: None,
            relay_owner: RelayOwner::Anyone,
            ..Seams::default()
        };
        let c = Controller::new(config, Arc::new(NoDialogs), seams);
        Self {
            _dir: dir,
            root,
            user,
            c,
        }
    }

    fn win64(&self) -> PathBuf {
        self.root.join("Archon").join("Binaries").join("Win64")
    }
}

// --- A transport whose sessions are ready at once ---

/// A host answers `HOST_READY`, a join `ROUTE_READY` and one offer to the host's account.
#[derive(Default)]
struct FakeTransport {
    next_handle: AtomicI32,
    is_join: AtomicBool,
    signal_pending: AtomicBool,
}

fn known(handle: Handle) -> Result<(), HostError> {
    if handle.0 > 0 {
        Ok(())
    } else {
        Err(HostError::NotFound(format!("unknown session handle {}", handle.0)))
    }
}

impl FakeTransport {
    fn open(&self) -> Handle {
        Handle(self.next_handle.fetch_add(1, Ordering::SeqCst) + 1)
    }
}

impl TransportAbi for FakeTransport {
    fn abi_version(&self) -> Result<u32, HostError> {
        Ok(3)
    }
    fn initialize(&self) -> Result<(), HostError> {
        Ok(())
    }
    fn networking_ready(&self) -> Result<bool, HostError> {
        Ok(true)
    }
    fn peer_id(&self) -> Result<PeerId, HostError> {
        Ok(PeerId(0xCAFE_1234_5678))
    }
    fn shutdown(&self) -> Result<(), HostError> {
        Ok(())
    }
    fn host_open(&self, _: &HostOpen) -> Result<Handle, HostError> {
        self.is_join.store(false, Ordering::SeqCst);
        Ok(self.open())
    }
    fn join_open(&self, _: &JoinOpen) -> Result<Handle, HostError> {
        self.is_join.store(true, Ordering::SeqCst);
        self.signal_pending.store(true, Ordering::SeqCst);
        Ok(self.open())
    }
    fn wait_ready(&self, handle: Handle, _: u32) -> Result<ReadyState, HostError> {
        known(handle)?;
        let line = if self.is_join.load(Ordering::SeqCst) {
            "ROUTE_READY localHost=127.0.0.1 localPort=29100 transport=ice routePolicy=AUTOMATIC route=direct"
        } else {
            "HOST_READY localHost=127.0.0.1 localPort=8791 transport=ice routePolicy=AUTOMATIC"
        };
        Ok(ReadyState::Ready(line.to_owned()))
    }
    fn session_running(&self, handle: Handle) -> Result<bool, HostError> {
        Ok(handle.0 > 0)
    }
    fn last_session_result(&self, handle: Handle) -> Result<SessionResult, HostError> {
        known(handle).map(|()| SessionResult::Clean)
    }
    fn stop_session(&self, handle: Handle) -> Result<(), HostError> {
        known(handle)
    }
    fn read_peer_event(&self, _: Handle) -> Result<Option<String>, HostError> {
        Ok(None)
    }
    fn read_diagnostic(&self, _: Handle) -> Result<Option<String>, HostError> {
        Ok(None)
    }
    fn read_signal(&self, handle: Handle) -> Result<Option<Signal>, HostError> {
        if handle.0 > 0 && self.signal_pending.swap(false, Ordering::SeqCst) {
            return Ok(Some(Signal {
                to_account: AccountId("user-slayer-1".to_owned()),
                kind: "DESCRIPTION".to_owned(),
                data: "v=0\r\na=ice-ufrag:daintylass\r\n".to_owned(),
            }));
        }
        Ok(None)
    }
    fn submit_signal(&self, handle: Handle, _: &AccountId, _: &str, _: &str) -> Result<(), HostError> {
        known(handle)
    }
    fn add_peer(&self, handle: Handle, _: &AccountId, _: PeerId, _: &Secret) -> Result<(), HostError> {
        known(handle)
    }
    fn remove_peer(&self, handle: Handle, _: &AccountId, _: &str) -> Result<(), HostError> {
        known(handle)
    }
    fn update_relay_credential(&self, handle: Handle, _: &str, _: &Secret) -> Result<(), HostError> {
        known(handle)
    }
}

struct FakeLoader;

impl TransportLoader for FakeLoader {
    fn load(&self) -> Result<Box<dyn TransportAbi>, HostError> {
        Ok(Box::new(FakeTransport::default()))
    }
}

/// Nothing ships a transport file (D1): a game folder with only the runtime's admission key is ready for realms, with
/// the launcher's own loader and no seams (spec-paradox-transport C1).
#[tokio::test]
async fn realms_need_no_transport_file_beside_the_game() {
    let hx = Harness::new().await;
    let win64 = hx.win64();
    assert_eq!(hx.c.use_game_path(hx.root.to_str().unwrap()).await, ActionResult::Ok);
    let RealmOutcome::Ok(without_key) = hx.c.realm_get().await else {
        panic!("expected RealmOutcome::Ok from realm_get");
    };
    assert!(!without_key.available);

    std::fs::write(win64.join(ADMISSION_PUBLIC_KEY_FILE), [0x11u8; 64]).unwrap();
    let transport_files: Vec<_> = std::fs::read_dir(&win64)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_ascii_lowercase())
        .filter(|name| name.contains("transport"))
        .collect();
    assert!(transport_files.is_empty(), "{transport_files:?}");
    let RealmOutcome::Ok(view) = hx.c.realm_get().await else {
        panic!("expected RealmOutcome::Ok from realm_get");
    };
    assert!(view.available);
    hx.c.shutdown().await;
}

/// A signed-in launcher on a server with this admission key answer, its game folder chosen.
async fn on_key_server(hx: &Harness, answer: String) {
    let account_key = "DLK_admission_key_test_account_0001";
    let mut host = Host {
        name: "KeySrv",
        admission_key: Some(answer),
        ..Host::default()
    };
    host.users
        .insert(account_key.into(), ("Slayer".into(), "user-slayer-1".into()));
    let (_shared, port) = lan(host).await;
    assert_eq!(
        hx.c.submit_invite(&invite(port, "KeySrv"), false, false).await,
        ActionResult::Ok
    );
    assert_eq!(hx.c.use_existing_key(account_key).await, ActionResult::Ok);
    assert_eq!(hx.c.use_game_path(hx.root.to_str().unwrap()).await, ActionResult::Ok);
}

/// Nothing ships the admission key, so the server answers it: `realm_get` writes it beside the game, and hosting
/// replaces one left by another server.
#[tokio::test]
async fn the_admission_key_comes_from_the_server() {
    use base64::Engine as _;
    let hx = Harness::new().await;
    let xy = [0x5Au8; 64];
    let answer = json!({ "keyId": "k1", "publicXy": base64::engine::general_purpose::STANDARD.encode(xy) });
    on_key_server(&hx, answer.to_string()).await;
    let key_file = hx.win64().join(ADMISSION_PUBLIC_KEY_FILE);

    let RealmOutcome::Ok(view) = hx.c.realm_get().await else {
        panic!("expected RealmOutcome::Ok from realm_get");
    };
    assert!(view.available);
    assert_eq!(std::fs::read(&key_file).unwrap(), xy);

    std::fs::write(&key_file, [0x11u8; 64]).unwrap();
    assert_eq!(hx.c.verify_game().await, ActionResult::Ok);
    hx.c.set_realms_seams(RealmsSeams {
        loader: Arc::new(FakeLoader) as Arc<dyn TransportLoader>,
        backend: Arc::new(DefaultHostBackend::new()) as Arc<dyn HostBackend>,
        spawner: Arc::new(NoopServerSpawner::new()) as Arc<dyn ServerSpawner>,
    });
    assert!(matches!(hx.c.realm_host(RealmKind::Hub).await, RealmOutcome::Ok(_)));
    assert_eq!(std::fs::read(&key_file).unwrap(), xy);
    hx.c.shutdown().await;
}

/// An answer without a 64-byte key writes nothing, so realms stay unavailable.
#[tokio::test]
async fn a_short_admission_key_is_not_written() {
    let hx = Harness::new().await;
    on_key_server(&hx, json!({ "keyId": "k1", "publicXy": "AAAA" }).to_string()).await;
    let RealmOutcome::Ok(view) = hx.c.realm_get().await else {
        panic!("expected RealmOutcome::Ok from realm_get");
    };
    assert!(!view.available);
    let written: Vec<_> = std::fs::read_dir(hx.win64())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(ADMISSION_PUBLIC_KEY_FILE))
        .collect();
    assert!(written.is_empty(), "{written:?}");
    hx.c.shutdown().await;
}

#[tokio::test]
async fn realm_commands_host_join_stop_keep_rd9_and_never_expose_account_key() {
    let hx = Harness::new().await;
    let secret_key = "DLK_super_secret_account_key_never_in_realms_998877";
    let mut host = Host {
        name: "RealmsSrv",
        runtime: Some(make_runtime(1)),
        ..Host::default()
    };
    host.users
        .insert(secret_key.into(), ("Slayer".into(), "user-slayer-1".into()));
    let (_shared, port) = lan(host).await;

    assert_eq!(
        hx.c.submit_invite(&invite(port, "RealmsSrv"), false, false).await,
        ActionResult::Ok
    );
    assert_eq!(hx.c.use_existing_key(secret_key).await, ActionResult::Ok);

    let win64 = hx.win64();
    std::fs::write(win64.join(ADMISSION_PUBLIC_KEY_FILE), [0x11u8; 64]).unwrap();

    assert_eq!(hx.c.use_game_path(hx.root.to_str().unwrap()).await, ActionResult::Ok);
    assert_eq!(hx.c.verify_game().await, ActionResult::Ok);

    let spawner = Arc::new(NoopServerSpawner::new());
    hx.c.set_realms_seams(RealmsSeams {
        loader: Arc::new(FakeLoader) as Arc<dyn TransportLoader>,
        backend: Arc::new(DefaultHostBackend::new()) as Arc<dyn HostBackend>,
        spawner: spawner.clone() as Arc<dyn ServerSpawner>,
    });

    // Initial realm_get reports available with no sessions.
    let RealmOutcome::Ok(initial) = hx.c.realm_get().await else {
        panic!("expected RealmOutcome::Ok from realm_get");
    };
    assert!(initial.available);
    assert!(initial.active.is_none());
    assert!(initial.sessions.is_empty());

    // Host a hub.
    let RealmOutcome::Ok(after_hub) = hx.c.realm_host(RealmKind::Hub).await else {
        panic!("expected RealmOutcome::Ok from realm_host(Hub)");
    };
    assert_eq!(after_hub.sessions.len(), 1);
    assert_eq!(after_hub.active.as_ref().map(|s| s.kind), Some(RealmKind::Hub));
    assert_eq!(after_hub.active.as_ref().map(|s| s.state), Some(RealmState::Running));

    // Host a hunt alongside the hub.
    let RealmOutcome::Ok(after_hunt) = hx.c.realm_host(RealmKind::Hunt).await else {
        panic!("expected RealmOutcome::Ok from realm_host(Hunt)");
    };
    assert_eq!(after_hunt.sessions.len(), 2);
    assert_eq!(after_hunt.active.as_ref().map(|s| s.kind), Some(RealmKind::Hunt));

    // Join a hunt.
    let RealmOutcome::Ok(after_join) = hx.c.realm_join("01ARZ3NDEKTSV4RRFFQ69G5FAV@local").await else {
        panic!("expected RealmOutcome::Ok from realm_join");
    };
    assert_eq!(after_join.sessions.len(), 3);
    let joined = after_join.active.as_ref().unwrap();
    assert!(!joined.hosting);
    assert_eq!(joined.local_port, Some(29_100));

    // RD9: realm hosting and joining never write mystic_loader.ini or any DLL beside the loader.
    assert!(!win64.join(LOADER_INI).exists());
    assert!(leftovers(&win64, &hx.user.join(RECORD_FILE), &[EXPORTER_DLL]).is_empty());

    // Account key isolation: neither the session files under userData/p2p-sessions/ nor the server
    // launch environment may contain the player's account key.
    let sessions_root = hx.user.join("p2p-sessions");
    assert!(sessions_root.is_dir());
    let mut checked_dirs = 0usize;
    for entry in std::fs::read_dir(&sessions_root).unwrap() {
        let dir = entry.unwrap().path();
        if !dir.is_dir() {
            continue;
        }
        checked_dirs += 1;
        for file_name in SESSION_FILES {
            let content = std::fs::read_to_string(dir.join(file_name)).unwrap();
            assert!(
                !content.contains(secret_key),
                "session file {file_name} must never contain the account key"
            );
        }
    }
    assert_eq!(checked_dirs, 3);
    let env = spawner.last_environment().unwrap();
    for (k, v) in &env {
        assert!(
            !v.contains(secret_key),
            "launch variable {k} must never contain the account key"
        );
    }

    // Stop all realm sessions and verify session directories are cleaned up.
    let RealmOutcome::Ok(after_stop) = hx.c.realm_stop().await else {
        panic!("expected RealmOutcome::Ok from realm_stop");
    };
    assert!(after_stop.sessions.is_empty());
    assert!(after_stop.active.is_none());
    let remaining_dirs = std::fs::read_dir(&sessions_root)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .count();
    assert_eq!(
        remaining_dirs, 0,
        "all session directories must be deleted on realm_stop"
    );

    // RD9: if a leftover mystic_loader.ini sits in Win64, both realm_host and play refuse with LoaderLeftovers.
    std::fs::write(win64.join(LOADER_INI), "[loader]\ndll=Dumper-7.dll\n").unwrap();
    assert_eq!(
        hx.c.realm_host(RealmKind::Hub).await,
        RealmOutcome::Err(LauncherError {
            code: ErrorCode::LoaderLeftovers,
            detail: Some(LOADER_INI.to_owned()),
        })
    );
    assert_eq!(
        hx.c.play().await,
        ActionResult::Err(LauncherError {
            code: ErrorCode::LoaderLeftovers,
            detail: Some(LOADER_INI.to_owned()),
        })
    );
    std::fs::remove_file(win64.join(LOADER_INI)).unwrap();

    // No SHA-256 mismatch case: the linked transport has no file to check (spec-rd27-rifts D1)
    hx.c.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn server_host_backend_drives_realm_hub_and_join_across_two_launchers() {
    let host_hx = Harness::new().await;
    let join_hx = Harness::new().await;
    let host_key = "DLK_host_account_key_for_server_realms_1122334455";
    let join_key = "DLK_join_account_key_for_server_realms_6677889900";

    let host_win64 = host_hx.win64();
    std::fs::write(host_win64.join(ADMISSION_PUBLIC_KEY_FILE), [0x22u8; 64]).unwrap();

    let join_win64 = join_hx.win64();
    std::fs::write(join_win64.join(ADMISSION_PUBLIC_KEY_FILE), [0x22u8; 64]).unwrap();

    let mut srv = Host {
        name: "LiveRealmsSrv",
        runtime: Some(make_runtime(1)),
        ..Host::default()
    };
    srv.users
        .insert(host_key.into(), ("HostSlayer".into(), "user-slayer-1".into()));
    srv.users
        .insert(join_key.into(), ("JoinSlayer".into(), "user-slayer-2".into()));
    let (shared, port) = lan(srv).await;

    let inv = invite(port, "LiveRealmsSrv");
    for (hx, key) in [(&host_hx, host_key), (&join_hx, join_key)] {
        assert_eq!(hx.c.submit_invite(&inv, false, false).await, ActionResult::Ok);
        assert_eq!(hx.c.use_existing_key(key).await, ActionResult::Ok);
        assert_eq!(hx.c.use_game_path(hx.root.to_str().unwrap()).await, ActionResult::Ok);
        assert_eq!(hx.c.verify_game().await, ActionResult::Ok);
        assert_eq!(hx.c.trust_runtime().await, ActionResult::Ok);
    }

    // The live ServerHostBackend against the server; only the transport is scripted (the linked one needs a peer)
    for hx in [&host_hx, &join_hx] {
        hx.c.set_realms_seams(RealmsSeams {
            loader: Arc::new(FakeLoader) as Arc<dyn TransportLoader>,
            backend: hx.c.live_host_backend(),
            spawner: Arc::new(NoopServerSpawner::new()) as Arc<dyn ServerSpawner>,
        });
    }

    // Host launcher starts a realm hub through the default ServerHostBackend.
    let outcome = host_hx.c.realm_host(RealmKind::Hub).await;
    let RealmOutcome::Ok(hosted_view) = outcome else {
        panic!("expected RealmOutcome::Ok from host_hx.c.realm_host(Hub), got {outcome:?}");
    };
    assert_eq!(hosted_view.sessions.len(), 1);
    let hub_session = hosted_view.active.as_ref().unwrap();
    assert!(hub_session.hosting);
    assert_eq!(hub_session.kind, RealmKind::Hub);
    assert_eq!(hub_session.state, RealmState::Running);

    // Join launcher joins the hosted hub using the UUID or session identifier through ServerHostBackend.
    let join_outcome = join_hx.c.realm_join(TEST_HUB_SESSION_UUID).await;
    let RealmOutcome::Ok(joined_view) = join_outcome else {
        panic!("expected RealmOutcome::Ok from join_hx.c.realm_join, got {join_outcome:?}");
    };
    assert_eq!(joined_view.sessions.len(), 1);
    let joined_session = joined_view.active.as_ref().unwrap();
    assert!(!joined_session.hosting);
    assert_eq!(joined_session.state, RealmState::Running);
    assert_eq!(joined_session.local_port, Some(29_100));
    assert_eq!(joined_session.session, hub_session.session);

    // Stop both launchers cleanly and verify all server routes were exercised.
    assert!(matches!(join_hx.c.realm_stop().await, RealmOutcome::Ok(_)));
    assert!(matches!(host_hx.c.realm_stop().await, RealmOutcome::Ok(_)));

    let seen = shared.lock().unwrap().seen.clone();
    for expected in [
        "/launcher/v1/realms/mine",
        "/launcher/v1/realms",
        "/launcher/v1/realms/mine/heartbeat",
        "/launcher/v1/p2p/host-presence",
        "/launcher/v1/realm-hosting/hubs/start",
        &format!("/rustic/v1/hunts/{TEST_HUB_SESSION_UUID}/host-ready"),
        &format!("/rustic/v1/hunts/{TEST_HUB_SESSION_UUID}/tickets/consume"),
        &format!("/launcher/v1/p2p/hunts/{TEST_HUB_SESSION_UUID}/transport-config"),
        &format!("/launcher/v1/p2p/hunts/{TEST_HUB_SESSION_UUID}/route-ready"),
        "/launcher/v1/realms/listed",
        "/launcher/v1/realms/join",
        "/launcher/v1/realms/presence/heartbeat",
        &format!("/launcher/v1/p2p/hunts/{TEST_HUB_SESSION_UUID}/join-bootstrap"),
        &format!("/launcher/v1/p2p/hunts/{TEST_HUB_SESSION_UUID}/signal"),
        "/launcher/v1/realm-hosting/hubs/stop",
        "/launcher/v1/p2p/control",
        "/launcher/v1/p2p/control/ctrl-msg-1/ack",
    ] {
        assert!(
            seen.iter().any(|p| p == expected),
            "expected server to receive {expected}, seen: {seen:?}"
        );
    }

    join_hx.c.shutdown().await;
    host_hx.c.shutdown().await;
}

// --- A hosted server that is ready only when the test says so ---

/// `wait_ready` blocks until opened, as the player host's ready pipe does for up to `READY_TIMEOUT`.
#[derive(Default)]
struct GatedSpawner {
    waiting: AtomicBool,
    open: Mutex<bool>,
    opened: std::sync::Condvar,
    stopped: Mutex<Vec<u32>>,
}

impl GatedSpawner {
    const PID: u32 = 99;

    async fn until_waiting(&self) {
        while !self.waiting.load(Ordering::SeqCst) {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    fn release(&self) {
        *self.open.lock().unwrap() = true;
        self.opened.notify_all();
    }

    async fn until_stopped(&self) -> bool {
        for _ in 0..1000 {
            if self.stopped.lock().unwrap().contains(&Self::PID) {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        false
    }
}

impl ServerSpawner for GatedSpawner {
    fn spawn(&self, _: &Path, _: &[String], _: &Path, _: &BTreeMap<String, String>) -> Result<u32, HostError> {
        Ok(Self::PID)
    }

    fn wait_ready(&self, _: u32) -> Result<(), HostError> {
        self.waiting.store(true, Ordering::SeqCst);
        let open = self.open.lock().unwrap();
        let (open, _) = self
            .opened
            .wait_timeout_while(open, std::time::Duration::from_secs(5), |open| !*open)
            .unwrap();
        if *open {
            Ok(())
        } else {
            Err(HostError::Transport("the gate was never opened".to_owned()))
        }
    }

    fn stop(&self, pid: u32) {
        self.stopped.lock().unwrap().push(pid);
    }
}

async fn gated_launcher() -> (Harness, Arc<GatedSpawner>) {
    let hx = Harness::new().await;
    use base64::Engine as _;
    let xy = base64::engine::general_purpose::STANDARD.encode([0x5Au8; 64]);
    let answer = json!({ "keyId": "k1", "publicXy": xy });
    on_key_server(&hx, answer.to_string()).await;
    assert_eq!(hx.c.verify_game().await, ActionResult::Ok);
    let spawner = Arc::new(GatedSpawner::default());
    hx.c.set_realms_seams(RealmsSeams {
        loader: Arc::new(FakeLoader) as Arc<dyn TransportLoader>,
        backend: Arc::new(DefaultHostBackend::new()) as Arc<dyn HostBackend>,
        spawner: spawner.clone() as Arc<dyn ServerSpawner>,
    });
    (hx, spawner)
}

/// On a one-thread runtime the gate opens only if the start waiting for the server leaves the thread free.
#[tokio::test(flavor = "current_thread")]
async fn a_start_waiting_for_its_server_does_not_hold_the_runtime() {
    let (hx, spawner) = gated_launcher().await;
    let opener = async {
        spawner.until_waiting().await;
        spawner.release();
    };
    let (outcome, ()) = tokio::join!(hx.c.realm_host(RealmKind::Hub), opener);
    let RealmOutcome::Ok(view) = outcome else {
        panic!("expected RealmOutcome::Ok from realm_host(Hub), got {outcome:?}");
    };
    assert_eq!(view.sessions.len(), 1);
    assert!(spawner.stopped.lock().unwrap().is_empty());
    hx.c.shutdown().await;
}

/// A start dropped while its server starts still ends that server once the wait is over, and records nothing.
#[tokio::test(flavor = "current_thread")]
async fn a_dropped_start_ends_its_server() {
    let (hx, spawner) = gated_launcher().await;
    {
        let start = hx.c.realm_host(RealmKind::Hub);
        tokio::pin!(start);
        tokio::select! {
            outcome = &mut start => panic!("the start finished before its server was ready: {outcome:?}"),
            () = spawner.until_waiting() => {}
        }
    }
    spawner.release();
    assert!(spawner.until_stopped().await, "the abandoned server was never stopped");
    let RealmOutcome::Ok(view) = hx.c.realm_get().await else {
        panic!("expected RealmOutcome::Ok from realm_get");
    };
    assert!(view.sessions.is_empty());
    hx.c.shutdown().await;
}
