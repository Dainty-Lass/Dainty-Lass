// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The server against a real listener on 127.0.0.1:0 (spec section 3.3, H and L): keys, streams, a runtime
//! changing under the list, versioned runtimes, the module routes, and the request log.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use content::config::Config;
use content::modules::{ContentModules, RegistryView};
use module_api::CorePlugin as _;
use module_api::manifest::ModuleManifest;
use module_api::registrar::Registration;
use module_api::{ModuleId, ModuleKind};
use serde_json::{Value, json};
use service_kit::env::Env;
use sha2::Digest as _;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, b| {
        _ = write!(out, "{b:02x}");
        out
    })
}

fn sha256(bytes: &[u8]) -> String {
    hex(&sha2::Sha256::digest(bytes))
}

/// The fake backend: any `RRK_` key is an account whose id is a hash of it, named `user-<id>`; anything else is
/// refused.
async fn backend() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut head = Vec::new();
                let mut buf = [0u8; 4096];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match socket.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => head.extend_from_slice(&buf[..n]),
                    }
                }
                let text = String::from_utf8_lossy(&head).to_string();
                let key = text.lines().find_map(|l| {
                    l.to_ascii_lowercase()
                        .starts_with("x-reforged-user-api-key:")
                        .then(|| l.split_once(':').unwrap().1.trim().to_owned())
                });
                let (status, body) = match key.as_deref().filter(|k| k.starts_with("RRK_")) {
                    Some(key) => {
                        let id = sha256(key.as_bytes())[..12].to_owned();
                        (200, json!({"UserId": id, "Username": format!("user-{id}")}).to_string())
                    }
                    None => (401, "{}".to_owned()),
                };
                let reply = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                _ = socket.write_all(reply.as_bytes()).await;
            });
        }
    });
    format!("http://{addr}")
}

struct Running {
    port: u16,
    stop: tokio::sync::watch::Sender<bool>,
}

impl Drop for Running {
    fn drop(&mut self) {
        _ = self.stop.send(true);
    }
}

async fn start(pairs: &[(&str, String)], modules: Arc<ContentModules>) -> Running {
    let mut env: Vec<(&str, String)> = vec![("BACKEND_URL", backend().await)];
    env.extend(pairs.iter().cloned());
    let config = Config::from_env(&Env::from_pairs(env.iter().map(|(k, v)| (*k, v.as_str())))).unwrap();
    let app = content::server::prepare_with_modules(&config, modules);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (stop, stopped) = tokio::sync::watch::channel(false);
    tokio::spawn(content::server::serve(listener, app, stopped));
    Running { port, stop }
}

#[derive(Debug)]
struct Reply {
    status: u16,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

impl Reply {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }
}

async fn send(port: u16, method: &str, path: &str, headers: &[(&str, &str)]) -> Reply {
    let mut request = format!("{method} {path} HTTP/1.1\r\nhost: content\r\n");
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("connection: close\r\n\r\n");
    let mut socket = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    socket.write_all(request.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    tokio::time::timeout(Duration::from_secs(20), socket.read_to_end(&mut raw))
        .await
        .unwrap()
        .unwrap();
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let mut lines = head.split("\r\n");
    let status = lines.next().unwrap().split(' ').nth(1).unwrap().parse().unwrap();
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect();
    Reply {
        status,
        headers,
        body: raw[split + 4..].to_vec(),
    }
}

async fn get(port: u16, path: &str) -> Reply {
    send(port, "GET", path, &[]).await
}

async fn keyed(port: u16, path: &str, key: &str) -> Reply {
    send(port, "GET", path, &[("x-reforged-user-api-key", key)]).await
}

/// A download whose client reads only the status line and then stalls.
async fn stalled(port: u16, path: &str, key: &str) -> (TcpStream, u16) {
    let mut socket = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let request = format!("GET {path} HTTP/1.1\r\nhost: content\r\nx-reforged-user-api-key: {key}\r\n\r\n");
    socket.write_all(request.as_bytes()).await.unwrap();
    let mut first = [0u8; 12];
    socket.read_exact(&mut first).await.unwrap();
    let status = String::from_utf8_lossy(&first[9..12]).parse().unwrap();
    (socket, status)
}

fn publish(dir: &Path, files: &[(&str, &[u8])]) {
    std::fs::create_dir_all(dir).unwrap();
    let listed: Vec<Value> = files
        .iter()
        .map(|(name, bytes)| json!({"name": name, "size": bytes.len(), "sha256": sha256(bytes)}))
        .collect();
    for (name, bytes) in files {
        std::fs::write(dir.join(name), bytes).unwrap();
    }
    std::fs::write(
        dir.join("runtime.json"),
        json!({"version": "v1", "files": listed}).to_string(),
    )
    .unwrap();
}

fn dir_env(name: &'static str, path: &Path) -> (&'static str, String) {
    (name, path.display().to_string())
}

#[tokio::test(flavor = "multi_thread")]
async fn manifest_open_files_keyed_nothing_else() {
    let tmp = tempfile::tempdir().unwrap();
    let runtime = tmp.path().join("runtime");
    std::fs::create_dir_all(&runtime).unwrap();
    let server = start(
        &[dir_env("CONTENT_RUNTIME_DIR", &runtime)],
        Arc::new(ContentModules::none()),
    )
    .await;
    let none = get(server.port, "/content/v1/runtime/manifest").await;
    assert_eq!((none.status, none.text().as_str()), (404, r#"{"error":"no_runtime"}"#));
    drop(server);

    publish(&runtime, &[("Rustic-Isles.dll", b"rustic"), ("winmm.dll", b"winmm")]);
    std::fs::write(runtime.join("dxgi.dll"), b"x").unwrap();
    let server = start(
        &[dir_env("CONTENT_RUNTIME_DIR", &runtime)],
        Arc::new(ContentModules::none()),
    )
    .await;
    let manifest = get(server.port, "/content/v1/runtime/manifest").await;
    assert_eq!(manifest.status, 200);
    assert_eq!(
        manifest.header("etag").unwrap(),
        format!("\"{}\"", sha256(&manifest.body))
    );
    let file = "/content/v1/runtime/files/winmm.dll";
    assert_eq!(get(server.port, file).await.status, 401);
    assert_eq!(keyed(server.port, file, "nope").await.status, 401);
    let served = keyed(server.port, file, "RRK_a").await;
    assert_eq!((served.status, served.body.as_slice()), (200, b"winmm".as_slice()));
    for other in ["runtime.json", "dxgi.dll", "WINMM.DLL", "%2e%2e/runtime.json", ""] {
        let path = format!("/content/v1/runtime/files/{other}");
        assert_eq!(keyed(server.port, &path, "RRK_a").await.status, 404, "{other}");
    }
    assert_eq!(get(server.port, "/content/v1/news").await.text(), r#"{"items":[]}"#);
    assert_eq!(
        get(server.port, "/content/v1/branding").await.text(),
        r#"{"backgrounds":[],"accent":null}"#
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn stream_limits() {
    let tmp = tempfile::tempdir().unwrap();
    let big: Vec<u8> = (0..48u32 << 20).map(|i| (i % 251) as u8).collect();
    publish(tmp.path(), &[("Rustic-Isles.dll", &big), ("winmm.dll", b"w")]);
    let server = start(
        &[
            dir_env("CONTENT_RUNTIME_DIR", tmp.path()),
            ("CONTENT_MAX_STREAMS_PER_ACCOUNT", "2".into()),
            ("CONTENT_MAX_STREAMS_TOTAL", "3".into()),
        ],
        Arc::new(ContentModules::none()),
    )
    .await;
    let file = "/content/v1/runtime/files/Rustic-Isles.dll";
    let (a1, s1) = stalled(server.port, file, "RRK_a").await;
    let (_a2, s2) = stalled(server.port, file, "RRK_a").await;
    assert_eq!((s1, s2), (200, 200));
    let third = keyed(server.port, file, "RRK_a").await;
    assert_eq!((third.status, third.header("retry-after")), (429, Some("2")));
    assert_eq!(third.text(), r#"{"error":"too_many_streams"}"#);
    // HEAD takes no slot
    let head = send(server.port, "HEAD", file, &[("x-reforged-user-api-key", "RRK_a")]).await;
    assert_eq!((head.status, head.header("content-length")), (200, Some("50331648")));
    assert!(head.body.is_empty() && head.header("cache-control").is_none());
    let (_b, sb) = stalled(server.port, file, "RRK_b").await;
    assert_eq!(sb, 200);
    let busy = keyed(server.port, file, "RRK_c").await;
    assert_eq!((busy.status, busy.header("retry-after")), (503, Some("10")));
    assert_eq!(busy.text(), r#"{"error":"server_busy"}"#);
    // Dropping a stalled client frees its slot
    drop(a1);
    let mut freed = false;
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        if send(
            server.port,
            "GET",
            "/content/v1/runtime/files/winmm.dll",
            &[("x-reforged-user-api-key", "RRK_c")],
        )
        .await
        .status
            == 200
        {
            freed = true;
            break;
        }
    }
    assert!(freed, "the dropped client's slot came back");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_changed_under_the_list() {
    let tmp = tempfile::tempdir().unwrap();
    publish(tmp.path(), &[("Rustic-Isles.dll", b"rustic"), ("winmm.dll", b"winmm")]);
    let server = start(
        &[dir_env("CONTENT_RUNTIME_DIR", tmp.path())],
        Arc::new(ContentModules::none()),
    )
    .await;
    let file = "/content/v1/runtime/files/winmm.dll";
    std::fs::write(tmp.path().join("winmm.dll"), b"winmm!").unwrap();
    let changed = keyed(server.port, file, "RRK_a").await;
    assert_eq!((changed.status, changed.header("retry-after")), (503, Some("10")));
    assert_eq!(changed.text(), r#"{"error":"file_unavailable"}"#);
    assert_eq!(get(server.port, "/content/v1/runtime/manifest").await.status, 503);
    assert_eq!(keyed(server.port, file, "RRK_a").await.status, 404);
    drop(server);

    // A new publish is served once the failed check was forgotten
    let tmp = tempfile::tempdir().unwrap();
    publish(tmp.path(), &[("Rustic-Isles.dll", b"rustic"), ("winmm.dll", b"winmm")]);
    let server = start(
        &[dir_env("CONTENT_RUNTIME_DIR", tmp.path())],
        Arc::new(ContentModules::none()),
    )
    .await;
    std::fs::write(tmp.path().join("winmm.dll"), b"winmm!").unwrap();
    assert_eq!(keyed(server.port, file, "RRK_a").await.status, 503);
    publish(tmp.path(), &[("Rustic-Isles.dll", b"rustic"), ("winmm.dll", b"winmm!")]);
    let served = keyed(server.port, file, "RRK_a").await;
    assert_eq!((served.status, served.body.as_slice()), (200, b"winmm!".as_slice()));
}

#[tokio::test(flavor = "multi_thread")]
async fn versioned_runtimes() {
    let tmp = tempfile::tempdir().unwrap();
    publish(tmp.path(), &[("Rustic-Isles.dll", b"own"), ("winmm.dll", b"own-w")]);
    publish(
        &tmp.path().join("revived"),
        &[("Rustic-Isles.dll", b"old"), ("winmm.dll", b"old-w")],
    );
    let server = start(
        &[dir_env("CONTENT_RUNTIME_DIR", tmp.path())],
        Arc::new(ContentModules::none()),
    )
    .await;
    let own = get(server.port, "/content/v1/runtime/manifest").await;
    let old = get(server.port, "/content/v1/runtime/manifest?game=revived").await;
    assert_ne!(own.body, old.body);
    assert!(old.text().contains(&sha256(b"old-w")));
    assert_eq!(
        get(server.port, "/content/v1/runtime/manifest?edition=legacy")
            .await
            .body,
        old.body
    );
    assert_eq!(
        get(server.port, "/content/v1/runtime/manifest?game=nonsense")
            .await
            .body,
        own.body
    );
    assert_eq!(
        get(server.port, "/content/v1/runtime/manifest?game=reforged")
            .await
            .body,
        own.body
    );
    let file = |p: &str| format!("/content/v1/runtime/files/{p}");
    assert_eq!(
        keyed(server.port, &file("revived/winmm.dll"), "RRK_a").await.body,
        b"old-w"
    );
    assert_eq!(
        keyed(server.port, &file("1.4.4/winmm.dll"), "RRK_a").await.body,
        b"old-w"
    );
    assert_eq!(
        keyed(server.port, &file("reforged/winmm.dll"), "RRK_a").await.body,
        b"own-w"
    );
    for bad in ["nonsense/winmm.dll", "revived/runtime.json", "revived/../winmm.dll"] {
        assert_eq!(keyed(server.port, &file(bad), "RRK_a").await.status, 404, "{bad}");
    }
    assert_eq!(get(server.port, &file("revived/winmm.dll")).await.status, 401);
}

#[derive(Default)]
struct Dirs(Mutex<BTreeMap<ModuleId, PathBuf>>);

impl RegistryView for Dirs {
    fn module_dir(&self, module: &ModuleId) -> Option<PathBuf> {
        self.0.lock().unwrap().get(module).cloned()
    }
}

fn module(epic_sha1: &str) -> ModuleManifest {
    serde_json::from_value(json!({
        "id": "isles-1-12",
        "kind": "game",
        "display_name": "Rustic Isles 1.12",
        "game_version": "1.12.0",
        "invite_aliases": ["isles-112"],
        "api": {"major": 1, "minor": 1},
        "unverified": true,
        "client": {
            "executable": "Dauntless/Game.exe",
            "arguments": [],
            "ini_edits": [],
            "epic": {
                "version": "1.12.0",
                "manifest_sha1": epic_sha1,
                "manifest_sources": ["https://example.invalid/m"],
                "executable_sha256": "0".repeat(64),
            },
        },
    }))
    .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn module_routes() {
    let tmp = tempfile::tempdir().unwrap();
    let manifest_bytes = b"epic manifest bytes".to_vec();
    std::fs::create_dir_all(tmp.path().join("data")).unwrap();
    std::fs::write(tmp.path().join("data/game.manifest"), &manifest_bytes).unwrap();
    let sha1 = hex(&sha1::Sha1::digest(&manifest_bytes));
    let dirs = Arc::new(Dirs::default());
    let modules = Arc::new(ContentModules::new(dirs.clone()));
    let server = start(&[], modules.clone()).await;
    assert_eq!(
        get(server.port, "/content/v1/versions").await.text(),
        r#"{"default_version":"1.14.7","versions":[]}"#
    );
    let manifest = module(&sha1);
    assert_eq!(manifest.kind, ModuleKind::Game);
    let registration: Registration = serde_json::from_value(json!({
        "content": [{"path": "epic/manifest", "source": "data/game.manifest", "media_type": "application/octet-stream"}],
    }))
    .unwrap();
    dirs.0
        .lock()
        .unwrap()
        .insert(manifest.id.clone(), tmp.path().to_owned());
    modules.attach(&manifest, &registration).unwrap();
    assert_eq!(
        get(server.port, "/content/v1/versions").await.text(),
        r#"{"default_version":"1.14.7","versions":[{"game_version":"1.12.0","module":"isles-1-12","display_name":"Rustic Isles 1.12","invite_aliases":["isles-112"],"unverified":true,"client":true,"epic_manifest":true}]}"#
    );
    let profile = get(server.port, "/content/v1/versions/1.12.0/client").await;
    assert_eq!(profile.status, 200);
    let etag = profile.header("etag").unwrap().to_owned();
    assert_eq!(etag, format!("\"{}\"", sha256(&profile.body)));
    let by_alias = get(server.port, "/content/v1/versions/isles-112/client").await;
    assert_eq!(by_alias.body, profile.body);
    let cached = send(
        server.port,
        "GET",
        "/content/v1/versions/isles-112/client",
        &[("if-none-match", &etag)],
    )
    .await;
    assert_eq!((cached.status, cached.body.len()), (304, 0));
    for unknown in ["1.14.7", "9.9.9", "nope"] {
        let reply = get(server.port, &format!("/content/v1/versions/{unknown}/client")).await;
        assert_eq!(reply.text(), r#"{"error":"unknown_version"}"#, "{unknown}");
    }
    let epic = "/content/v1/versions/isles-112/epic/manifest";
    assert_eq!(get(server.port, epic).await.status, 401);
    let served = keyed(server.port, epic, "RRK_a").await;
    assert_eq!(served.status, 200);
    assert_eq!(served.body, manifest_bytes);
    assert_eq!(served.header("etag"), Some(format!("\"{sha1}\"").as_str()));
    assert_eq!(served.header("content-type"), Some("application/octet-stream"));
    let head = send(server.port, "HEAD", epic, &[("x-reforged-user-api-key", "RRK_a")]).await;
    assert_eq!(
        head.header("content-length"),
        Some(manifest_bytes.len().to_string().as_str())
    );
    assert!(head.body.is_empty());
    let quoted = format!("\"{sha1}\"");
    let unchanged = send(
        server.port,
        "GET",
        epic,
        &[("x-reforged-user-api-key", "RRK_a"), ("if-none-match", &quoted)],
    )
    .await;
    assert_eq!(unchanged.status, 304);
    // The built-in 1.14.7 has no module: a key is still asked first
    assert_eq!(
        get(server.port, "/content/v1/versions/1.14.7/epic/manifest")
            .await
            .status,
        401
    );
    assert_eq!(
        keyed(server.port, "/content/v1/versions/1.14.7/epic/manifest", "RRK_a")
            .await
            .status,
        404
    );
    std::fs::write(tmp.path().join("data/game.manifest"), b"another manifest, longer").unwrap();
    let changed = keyed(server.port, epic, "RRK_a").await;
    assert_eq!(
        (changed.status, changed.text().as_str()),
        (503, r#"{"error":"epic_manifest_invalid"}"#)
    );
    modules.detach(&manifest.id);
    assert_eq!(
        get(server.port, "/content/v1/versions").await.text(),
        r#"{"default_version":"1.14.7","versions":[]}"#
    );
    assert_eq!(
        get(server.port, "/content/v1/versions/isles-112/client").await.status,
        404
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn l1_the_download_line_and_no_key_anywhere() {
    let lines = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = lines.clone();
    content::log::set_sink(Some(Box::new(move |line, _| {
        seen.lock().unwrap().push(line.to_owned())
    })));
    let tmp = tempfile::tempdir().unwrap();
    publish(tmp.path(), &[("Rustic-Isles.dll", b"rustic"), ("winmm.dll", b"winmm")]);
    let server = start(
        &[dir_env("CONTENT_RUNTIME_DIR", tmp.path())],
        Arc::new(ContentModules::none()),
    )
    .await;
    let key = "RRK_logged-secret";
    let reply = send(
        server.port,
        "GET",
        "/content/v1/runtime/files/winmm.dll",
        &[("x-reforged-user-api-key", key), ("x-forwarded-for", "203.0.113.9")],
    )
    .await;
    assert_eq!(reply.status, 200);
    keyed(
        server.port,
        "/content/v1/runtime/files/Rustic-Isles.dll",
        "RRK_logged secret",
    )
    .await;
    drop(server);
    tokio::time::sleep(Duration::from_millis(200)).await;
    content::log::set_sink(None);
    let lines = lines.lock().unwrap().clone();
    assert!(
        !lines
            .iter()
            .any(|l| l.contains("logged-secret") || l.contains("logged secret"))
    );
    let line = lines
        .iter()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .find(|l| l["msg"] == "download" && l["ip"] == "203.0.113.9")
        .expect("the download's line");
    let keys: Vec<&str> = line.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        [
            "t", "level", "msg", "method", "route", "path", "status", "ms", "ip", "account", "user", "bytes",
            "expected", "complete"
        ]
    );
    assert_eq!(line["ip"], "203.0.113.9");
    assert_eq!(line["path"], "winmm.dll");
    let id = &sha256(key.as_bytes())[..12];
    assert_eq!(
        (line["account"].as_str(), line["user"].as_str()),
        (Some(id), Some(format!("user-{id}").as_str()))
    );
    assert_eq!((line["bytes"].as_u64(), line["expected"].as_u64()), (Some(5), Some(5)));
    assert_eq!(line["complete"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_service_library_refuses_a_bad_config() {
    use service_plugin::Service as _;
    let service = content::service::ContentService::new();
    assert_eq!(service.name(), "content");
    let env = Env::from_pairs([("PORT", "0")]);
    let ctx = service_kit::context::ServiceContext::new(
        "content",
        service_kit::context::StatusReporter::noop("content"),
        tokio::sync::watch::channel(false).1,
    );
    assert_eq!(
        service.run(env, ctx).await.unwrap_err(),
        "PORT must be a whole number from 1 to 65535"
    );
}
