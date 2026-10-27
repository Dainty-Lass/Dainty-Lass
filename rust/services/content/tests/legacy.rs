#![cfg(feature = "legacy")]
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

//! A legacy content server (`CONTENT_GAME=legacy`) over HTTP: DR's 1.4.4 manifest and game files,
//! with HTTP ranges as DR serves them, and the key checked at the metagame (names translated as
//! legacy-revived's `services/metagame` has them: `x-reforged-user-api-key`, `/reforged/api/GetUserInfo`).

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use content::app::{App, Deps};
use content::auth::{AccountInfo, AuthCache, Lookup, LookupResult};
use content::branding::Branding;
use content::config::Config;
use content::legacy::LegacyGame;
use content::limits::StreamLimiter;
use content::news::News;
use content::runtime::{RuntimeStore, sha256_hex};
use futures_util::FutureExt;
use serde_json::json;
use service_kit::env::Env;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const KEY: &str = "RRK_cccccccccccccccccccccccccccccccccccccccccccccccc";

/// A game folder with two files and the manifest that lists them.
fn game(dir: &Path) -> (Vec<u8>, std::path::PathBuf) {
    let exe: Vec<u8> = (0..=255u8).cycle().take(1000).collect();
    let version = b"1.4.4".to_vec();
    std::fs::create_dir_all(dir.join("Archon/Binaries/Win64")).unwrap();
    std::fs::write(dir.join("Archon/Binaries/Win64/Dauntless-Win64-Shipping.exe"), &exe).unwrap();
    std::fs::write(dir.join("Version.txt"), &version).unwrap();
    let manifest = json!({
        "build": "dauntless_rel-1.4.4_Shipping_2020-10-28_20-11-15_239827",
        "totalBytes": exe.len() + version.len() + 7,
        "files": [
            {"path": "Archon/Binaries/Win64/Dauntless-Win64-Shipping.exe", "size": exe.len(), "sha256": sha256_hex(&exe)},
            {"path": "Missing.txt", "size": 7, "sha256": "0".repeat(64)},
            {"path": "Version.txt", "size": version.len(), "sha256": sha256_hex(&version)},
        ],
    });
    let file = dir.join("manifest.json");
    std::fs::write(&file, manifest.to_string()).unwrap();
    (exe, file)
}

async fn start(dir: &Path, manifest: &Path) -> u16 {
    let lookup: Lookup = Arc::new(|key: String| {
        async move {
            if key == KEY {
                LookupResult::Ok(AccountInfo {
                    user_id: "uid-1".into(),
                    username: "Aurora".into(),
                    is_admin: false,
                })
            } else {
                LookupResult::Invalid
            }
        }
        .boxed()
    });
    let clock: content::auth::Clock = Arc::new(content::now_ms);
    let game = LegacyGame::prepare(dir, Some(manifest)).unwrap();
    let app = App::legacy(
        Deps {
            runtime: Arc::new(RuntimeStore::new(None, clock.clone())),
            auth: AuthCache::new(lookup, Duration::from_secs(300)),
            limiter: StreamLimiter::new(4, 48),
            branding: Branding::new(None, clock.clone()),
            news: Arc::new(News::new(None, None, clock)),
        },
        game,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (stop, stopping) = tokio::sync::watch::channel(false);
    std::mem::forget(stop);
    tokio::spawn(content::server::serve(listener, app, stopping));
    port
}

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

async fn request(port: u16, method: &str, path: &str, headers: &[(&str, &str)]) -> Reply {
    let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let extra: String = headers.iter().map(|(k, v)| format!("{k}: {v}\r\n")).collect();
    socket
        .write_all(format!("{method} {path} HTTP/1.1\r\nhost: x\r\n{extra}connection: close\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut reply = Vec::new();
    socket.read_to_end(&mut reply).await.unwrap();
    let end = reply.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8_lossy(&reply[..end]).into_owned();
    let mut lines = head.split("\r\n");
    let status = lines.next().unwrap()[9..12].parse().unwrap();
    let headers = lines
        .map(|l| {
            let (k, v) = l.split_once(':').unwrap();
            (k.trim().to_lowercase(), v.trim().to_owned())
        })
        .collect();
    Reply {
        status,
        headers,
        body: reply[end + 4..].to_vec(),
    }
}

const EXE: &str = "/content/v1/files/Archon/Binaries/Win64/Dauntless-Win64-Shipping.exe";

#[tokio::test(flavor = "multi_thread")]
async fn serves_the_game_manifest_without_a_key_and_no_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let (exe, manifest) = game(dir.path());
    let port = start(dir.path(), &manifest).await;
    let reply = request(port, "GET", "/content/v1/manifest", &[]).await;
    assert_eq!(reply.status, 200);
    let text = String::from_utf8(reply.body.clone()).unwrap();
    assert!(text.starts_with(r#"{"build":"dauntless_rel-1.4.4_Shipping_2020-10-28_20-11-15_239827","totalBytes":1012,"files":[{"path":"Archon/"#));
    assert!(text.contains(&sha256_hex(&exe)));
    let etag = reply.header("etag").unwrap().to_owned();
    assert_eq!(etag, format!("\"{}\"", sha256_hex(&reply.body)));
    assert_eq!(reply.header("cache-control"), Some("no-cache"));
    assert_eq!(reply.header("access-control-allow-origin"), None);
    let again = request(port, "GET", "/content/v1/manifest", &[("if-none-match", &etag)]).await;
    assert_eq!((again.status, again.header("etag")), (304, Some(etag.as_str())));
    for path in ["/content/v1/runtime/manifest", "/content/v1/runtime/files/winmm.dll"] {
        assert_eq!(request(port, "GET", path, &[]).await.status, 404, "{path}");
    }
    assert_eq!(request(port, "GET", "/content/v1/news", &[]).await.status, 200);
    assert_eq!(request(port, "POST", "/content/v1/manifest", &[]).await.status, 405);
}

#[tokio::test(flavor = "multi_thread")]
async fn game_files_need_a_key_and_serve_ranges_as_dr_did() {
    let dir = tempfile::tempdir().unwrap();
    let (exe, manifest) = game(dir.path());
    let port = start(dir.path(), &manifest).await;
    let key = ("x-reforged-user-api-key", KEY);

    assert_eq!(request(port, "GET", EXE, &[]).await.status, 401);
    assert_eq!(
        request(port, "GET", EXE, &[("x-reforged-user-api-key", "RRK_wrong")])
            .await
            .status,
        401
    );
    // DR's untranslated header means nothing here
    assert_eq!(
        request(port, "GET", EXE, &[("x-undaunted-user-api-key", KEY)])
            .await
            .status,
        401
    );

    let whole = request(port, "GET", EXE, &[key]).await;
    assert_eq!(whole.status, 200);
    assert_eq!(whole.body, exe);
    let etag = format!("\"{}\"", sha256_hex(&exe));
    assert_eq!(whole.header("etag"), Some(etag.as_str()));
    assert_eq!(whole.header("accept-ranges"), Some("bytes"));
    assert_eq!(whole.header("cache-control"), Some("private, no-transform"));

    let part = request(port, "GET", EXE, &[key, ("range", "bytes=10-19")]).await;
    assert_eq!(part.status, 206);
    assert_eq!(part.body, exe[10..20]);
    assert_eq!(part.header("content-range"), Some("bytes 10-19/1000"));
    let tail = request(port, "GET", EXE, &[key, ("range", "bytes=-5")]).await;
    assert_eq!((tail.status, tail.body.as_slice()), (206, &exe[995..]));
    // A stale If-Range sends it all
    let stale = request(
        port,
        "GET",
        EXE,
        &[key, ("range", "bytes=10-19"), ("if-range", "\"other\"")],
    )
    .await;
    assert_eq!((stale.status, stale.body.len()), (200, 1000));
    let bad = request(port, "GET", EXE, &[key, ("range", "bytes=1000-")]).await;
    assert_eq!(bad.status, 416);
    assert_eq!(bad.header("content-range"), Some("bytes */1000"));
    let head = request(port, "HEAD", EXE, &[key, ("range", "bytes=0-9")]).await;
    assert_eq!((head.status, head.header("content-length")), (206, Some("10")));
    assert!(head.body.is_empty());
    let cached = request(port, "GET", EXE, &[key, ("if-none-match", &etag)]).await;
    assert_eq!(cached.status, 304);
}

#[tokio::test(flavor = "multi_thread")]
async fn only_files_the_manifest_lists_and_still_has() {
    let dir = tempfile::tempdir().unwrap();
    let (_, manifest) = game(dir.path());
    let port = start(dir.path(), &manifest).await;
    let key = ("x-reforged-user-api-key", KEY);
    for (path, status) in [
        ("/content/v1/files/manifest.json", 404),
        ("/content/v1/files/..%2fmanifest.json", 400),
        ("/content/v1/files/Archon/../manifest.json", 400),
        ("/content/v1/files/version.txt", 404),
        ("/content/v1/files/Missing.txt", 503),
        ("/content/v1/files/Version.txt", 200),
    ] {
        assert_eq!(request(port, "GET", path, &[key]).await.status, status, "{path}");
    }
    // Replaced after the start-up check: refused until a restart
    std::fs::write(dir.path().join("Version.txt"), b"1.4.5!").unwrap();
    let changed = request(port, "GET", "/content/v1/files/Version.txt", &[key]).await;
    assert_eq!(changed.status, 503);
    assert_eq!(changed.header("etag"), None);
    std::fs::write(dir.path().join("Version.txt"), b"1.4.4").unwrap();
    assert_eq!(
        request(port, "GET", "/content/v1/files/Version.txt", &[key])
            .await
            .status,
        503
    );
}

#[test]
fn a_game_folder_without_any_listed_file_stops_the_start() {
    let dir = tempfile::tempdir().unwrap();
    let (_, manifest) = game(dir.path());
    let empty = tempfile::tempdir().unwrap();
    let error = LegacyGame::prepare(empty.path(), Some(&manifest)).err().unwrap();
    assert!(error.contains("CONTENT_GAME_DIR"), "{error}");
    assert!(LegacyGame::prepare(dir.path(), Some(&dir.path().join("nope.json"))).is_err());
    let game_dir = dir.path().to_string_lossy().into_owned();
    let manifest = manifest.to_string_lossy().into_owned();
    let config = Config::from_env(&Env::from_pairs([
        ("CONTENT_GAME", "legacy"),
        ("CONTENT_GAME_DIR", game_dir.as_str()),
        ("CONTENT_MANIFEST", manifest.as_str()),
    ]))
    .unwrap();
    assert!(content::server::prepare_game(&config).is_ok());
}
