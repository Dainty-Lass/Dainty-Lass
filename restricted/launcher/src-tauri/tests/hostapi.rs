//! dainty-launcher-kit's host API (spec §2.6.3), as the shell's `net/hostapi/tests.rs` tested it: DR's recorded table
//! (`test/cases/hostapi.json`), the answers' parsers, and Reforged's `pinned.test.ts` against a fake server.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bytes::Bytes;
use common::{acceptor, certificate};
use dainty_launcher_kit::bridge::types::{ErrorCode, LauncherError};
use dainty_launcher_kit::net::hostapi::parse as kit;
use dainty_launcher_kit::net::hostapi::{
    KEY_HEADER, REFORGED, StatusOutcome, fetch_runtime_manifest, fetch_status, fetch_user_info, key_headers, register,
};
use dainty_launcher_kit::net::http::{Endpoint, FailureKind};
use dainty_launcher_kit::net::pinned::certificate_fingerprint;
use dainty_launcher_kit::net::runtime_files::RuntimeManifestResult;
use dainty_protocol::account::RegisterRequest;
use dainty_protocol::manifest::{DEFAULT_CHANGELIST, RuntimeFile};
use http_body_util::{BodyExt, Full};
use hyper::service::service_fn;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use serde::Deserialize;
use serde_json::Value;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

// Built from pieces, so the repository's secret check never sees a whole key
const KEY: &str = concat!("DLK_", "eeeeeeeeeeeeeeee", "eeeeeeeeeeeeeeee", "eeeeeeeeeeeeeeee");

fn bytes(body: &Value) -> Vec<u8> {
    if body.is_null() {
        Vec::new()
    } else {
        body.to_string().into_bytes()
    }
}

fn info(status: u16, body: Value) -> Result<kit::UserInfo, LauncherError> {
    kit::user_info_reply(status, &bytes(&body))
}

fn request(username: &str, invite_code: &str) -> RegisterRequest {
    RegisterRequest {
        username: username.to_owned(),
        invite_code: invite_code.to_owned(),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cases {
    news: Vec<Case>,
    branding: Vec<Case>,
    images: Vec<Image>,
    news_body: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    input: Value,
    expected: Value,
}

#[derive(Deserialize)]
struct Image {
    bytes: Vec<u8>,
    #[serde(rename = "type")]
    kind: Option<String>,
}

/// DR's recorded table (`test/cases/hostapi.json`) against dainty-launcher-kit's parsers.
#[test]
fn the_shared_table() {
    let cases: Cases = serde_json::from_value(common::cases("hostapi.json")).unwrap();
    for case in cases.news {
        assert_eq!(
            serde_json::to_value(kit::parse_news(&case.input).unwrap_or_default()).unwrap(),
            case.expected,
            "{}",
            case.input
        );
    }
    for case in cases.branding {
        let branding = kit::parse_branding(&case.input);
        let backgrounds: Vec<Value> = branding
            .backgrounds
            .iter()
            .map(|background| serde_json::json!({ "path": background.url, "credit": background.credit }))
            .collect();
        assert_eq!(
            serde_json::json!({ "backgrounds": backgrounds, "accent": branding.accent }),
            case.expected,
            "{}",
            case.input
        );
    }
    for case in cases.images {
        assert_eq!(
            kit::sniff_image(&case.bytes).map(kit::ImageType::content_type),
            case.kind.as_deref(),
            "{:?}",
            case.bytes
        );
    }
    for case in cases.news_body {
        let cleaned = case.input.as_str().and_then(kit::clean_news_body);
        assert_eq!(serde_json::to_value(cleaned).unwrap(), case.expected, "{}", case.input);
    }
}

#[test]
fn branding_paths_with_other_characters_are_refused_without_panicking() {
    assert!(kit::is_branding_path("/content/v1/branding/a.jpg"));
    assert!(kit::is_branding_path("/CONTENT/v1/branding/A-b_c.d.WEBP"));
    assert!(!kit::is_branding_path("/content/v1/branding/.jpg"));
    assert!(!kit::is_branding_path("/content/v1/branding/a/../b.jpg"));
    assert!(!kit::is_branding_path("/content/v1/branding/é.jpg"));
    assert!(!kit::is_branding_path("/content/v1/branding/aé.png"));
    assert!(!kit::is_branding_path("/content/v1/brandinǵ/a.png"));
    assert!(!kit::is_branding_path(&format!(
        "/content/v1/branding/{}.png",
        "a".repeat(102)
    )));
    assert!(kit::is_branding_path(&format!(
        "/content/v1/branding/{}.png",
        "a".repeat(101)
    )));
}

// ---------------------------------------------------------------- the runtime manifest

fn hash(c: char) -> String {
    c.to_string().repeat(64)
}

fn manifest() -> Value {
    serde_json::json!({
        "version": "20261014-abcdef12",
        "build": { "name": "rel-1.14.7-Archon", "changelist": 647472 },
        "files": [
            { "name": "winmm.dll", "size": 40001, "sha256": hash('b') },
            { "name": "Rustic-Isles.dll", "size": 200001, "sha256": hash('a') },
        ],
    })
}

#[test]
fn runtime_manifests_name_the_two_files_only() {
    let parsed = kit::parse_runtime_manifest(&manifest()).unwrap();
    assert_eq!(parsed.version, "20261014-abcdef12");
    assert_eq!(parsed.build.changelist, 647472);
    assert_eq!(
        parsed.files,
        [
            RuntimeFile {
                name: "Rustic-Isles.dll".into(),
                size: 200001,
                sha256: hash('a')
            },
            RuntimeFile {
                name: "winmm.dll".into(),
                size: 40001,
                sha256: hash('b')
            },
        ]
    );

    let with = |edit: &dyn Fn(&mut Value)| {
        let mut raw = manifest();
        edit(&mut raw);
        kit::parse_runtime_manifest(&raw)
    };
    // An R21 server's set: the MIT runtime alone, or beside the loader (spec-runtime-r21 7.2); the pair above is a
    // pre-R21 server's, still played for one launcher release
    let mit = with(&|raw| raw["files"][1]["name"] = "rustic-isles-runtime.dll".into()).unwrap();
    assert_eq!(mit.files[0].name, "rustic-isles-runtime.dll");
    assert_eq!(
        with(&|raw| {
            raw["files"] = serde_json::json!([{ "name": "rustic-isles-runtime.dll", "size": 1, "sha256": hash('a') }]);
        })
        .unwrap()
        .files
        .len(),
        1
    );
    // A host that still publishes the runtime under its name from before the rename (DX4)
    assert_eq!(
        with(&|raw| raw["files"][1]["name"] = "MysticParadox.dll".into())
            .unwrap()
            .files[0]
            .name,
        "MysticParadox.dll"
    );
    assert_eq!(
        with(
            &|raw| raw["files"][0] = serde_json::json!({ "name": "MysticParadox.dll", "size": 1, "sha256": hash('a') })
        ),
        None
    );
    // Kept, with the build read loosely
    // The shell's "none" is dainty-launcher-kit's default: protocol's manifest always carries a changelist
    let changelist = |edit: &dyn Fn(&mut Value)| with(edit).unwrap().build.changelist;
    assert_eq!(changelist(&|raw| raw["build"] = Value::Null), DEFAULT_CHANGELIST);
    assert_eq!(
        changelist(&|raw| raw["build"]["changelist"] = 1.5.into()),
        DEFAULT_CHANGELIST
    );
    assert_eq!(
        changelist(&|raw| raw["build"]["changelist"] = 9_007_199_254_740_992u64.into()),
        DEFAULT_CHANGELIST
    );
    assert_eq!(
        with(&|raw| raw["files"][0]["size"] = 40001.0.into()).unwrap().files[1].size,
        40001
    );
    assert_eq!(
        with(&|raw| raw["version"] = " a\u{7} b ".into()).unwrap().version,
        "a b"
    );
    // Refused whole
    let refused: [&dyn Fn(&mut Value); 14] = [
        &|raw| raw["files"][0]["name"] = "evil.dll".into(),
        &|raw| raw["files"][0]["name"] = "..\\winmm.dll".into(),
        &|raw| raw["files"][0]["name"] = "Rustic-Isles.dll".into(),
        &|raw| raw["files"][0]["sha256"] = hash('B').into(),
        &|raw| raw["files"][0]["sha256"] = "ab".into(),
        &|raw| raw["files"][0]["size"] = 0.into(),
        &|raw| raw["files"][0]["size"] = (64 * 1024 * 1024 + 1).into(),
        &|raw| raw["files"][0]["size"] = "40001".into(),
        &|raw| raw["files"][0]["size"] = 1.5.into(),
        &|raw| raw["files"].as_array_mut().unwrap().truncate(1),
        &|raw| {
            let first = raw["files"][0].clone();
            raw["files"].as_array_mut().unwrap().push(first);
        },
        &|raw| raw["files"] = serde_json::json!({}),
        &|raw| raw["version"] = " ".into(),
        &|raw| raw["version"] = 5.into(),
    ];
    for (i, edit) in refused.iter().enumerate() {
        assert_eq!(with(*edit), None, "case {i}");
    }
    assert_eq!(kit::parse_runtime_manifest(&Value::Null), None);
}

/// The shell kept a negative changelist; dainty-launcher-kit replaces it with the default, as L8 decided (no server
/// writes one: protocol's changelist is a u64).
#[test]
#[ignore = "accepted difference (L8): dainty-launcher-kit replaces a negative changelist with the default"]
fn a_negative_changelist_is_kept() {
    let mut raw = manifest();
    raw["build"]["changelist"] = (-3).into();
    let parsed = kit::parse_runtime_manifest(&raw).unwrap();
    assert_eq!(
        serde_json::to_value(parsed.build.changelist).unwrap(),
        serde_json::json!(-3)
    );
}

// ---------------------------------------------------------------- answers, without the network

#[test]
fn register_answers() {
    let ok = |code| Err(LauncherError::new(code));
    let cases = [
        (200, serde_json::json!({ "UUK": KEY }), Ok(KEY.to_owned())),
        (200, serde_json::json!({ "UUK": "bad key" }), ok(ErrorCode::ServerError)),
        (200, serde_json::json!({ "UUK": 5 }), ok(ErrorCode::ServerError)),
        (200, Value::Null, ok(ErrorCode::ServerError)),
        (
            400,
            serde_json::json!({ "error": "registration_closed", "message": " closed\u{7} " }),
            Err(LauncherError::with_detail(ErrorCode::RegistrationClosed, "closed")),
        ),
        (
            400,
            serde_json::json!({ "error": "username_invalid" }),
            ok(ErrorCode::UsernameInvalid),
        ),
        (400, serde_json::json!({ "error": "other" }), ok(ErrorCode::BadRequest)),
        (400, Value::Null, ok(ErrorCode::BadRequest)),
        (401, Value::Null, ok(ErrorCode::InviteInvalid)),
        (409, Value::Null, ok(ErrorCode::UsernameTaken)),
        (413, Value::Null, ok(ErrorCode::RateLimited)),
        (429, Value::Null, ok(ErrorCode::RateLimited)),
        (500, Value::Null, ok(ErrorCode::ServerError)),
    ];
    for (status, body, expected) in cases {
        assert_eq!(kit::register_reply(status, &bytes(&body)), expected, "{status} {body}");
    }
}

#[test]
fn user_info_answers() {
    let ok = serde_json::json!({ "UserId": "uid-1_A", "Username": " Slayer ", "IsAdmin": true });
    assert_eq!(
        info(200, ok.clone()),
        Ok(kit::UserInfo {
            user_id: "uid-1_A".into(),
            username: "Slayer".into(),
            is_admin: true,
            oauth_linked: None
        })
    );
    let not_admin = serde_json::json!({ "UserId": "u", "Username": "S", "IsAdmin": "true" });
    assert!(!info(200, not_admin.clone()).unwrap().is_admin);
    let failed = |code| Err(LauncherError::new(code));
    // The id goes on the game's command line
    for id in ["a b", "a;b", "-AUTH=x y", "é", ""] {
        let body = serde_json::json!({ "UserId": id, "Username": "S" });
        assert_eq!(info(200, body.clone()), failed(ErrorCode::ServerError), "{id:?}");
    }
    let long = serde_json::json!({ "UserId": "a".repeat(129), "Username": "S" });
    assert_eq!(info(200, long.clone()).unwrap().user_id.len(), 128);
    assert_eq!(info(200, Value::Null), failed(ErrorCode::ServerError));

    let banned = serde_json::json!({ "error": "account_banned", "reason": "cheating\u{202e}" });
    assert_eq!(
        info(403, banned.clone()),
        Err(LauncherError::with_detail(ErrorCode::AccountBanned, "cheating"))
    );
    // Disabled reads as banned; no reason, no default text
    let disabled = serde_json::json!({ "error": "account_disabled" });
    assert_eq!(info(403, disabled.clone()), failed(ErrorCode::AccountBanned));
    let other = serde_json::json!({ "error": "forbidden" });
    assert_eq!(info(403, other.clone()), failed(ErrorCode::KeyRejected));
    assert_eq!(info(401, Value::Null), failed(ErrorCode::KeyRejected));
    assert_eq!(info(429, Value::Null), failed(ErrorCode::RateLimited));
    assert_eq!(info(502, Value::Null), failed(ErrorCode::ServerError));
}

#[test]
fn the_key_is_never_sent_outside_the_tailnet_without_a_pin() {
    let public_plain = Endpoint {
        host: "203.0.113.9".into(),
        port: 80,
        pin: None,
    };
    assert!(key_headers(&public_plain, Some(KEY)).is_empty());
    let pinned = Endpoint {
        pin: Some("3f".repeat(32)),
        ..public_plain
    };
    assert_eq!(key_headers(&pinned, Some(KEY)), vec![(KEY_HEADER, KEY.to_owned())]);
    assert!(key_headers(&pinned, Some("short")).is_empty());
    assert!(key_headers(&pinned, Some("DLK_ with space")).is_empty());
    assert!(key_headers(&pinned, None).is_empty());
    let tailnet = Endpoint {
        host: "100.64.0.1".into(),
        port: 61000,
        pin: None,
    };
    assert_eq!(key_headers(&tailnet, Some(KEY)).len(), 1);
}

// ---------------------------------------------------------------- a fake Reforged server

/// Reforged's `test/fakes.ts` `FakeServer`, the parts these calls reach: the launcher API and the runtime
/// manifest on one address, plain HTTP or TLS. Records every request it sees, with the key header.
#[derive(Default)]
struct Fake {
    seen: Vec<(String, Option<String>)>,
    users: HashMap<String, (String, String)>,
    codes: HashSet<String>,
    registrations: usize,
    status_missing: bool,
    runtime: Option<Value>,
}

impl Fake {
    fn new() -> Self {
        let mut fake = Self {
            runtime: Some(manifest()),
            ..Self::default()
        };
        fake.codes.insert("GOOD-CODE".into());
        fake.users.insert(KEY.into(), ("aurora".into(), "uid-aurora".into()));
        fake
    }

    fn answer(&mut self, method: &hyper::Method, path: &str, key: Option<String>, body: &[u8]) -> (u16, Value) {
        self.seen.push((path.to_owned(), key.clone()));
        let user = key.as_ref().and_then(|key| self.users.get(key)).cloned();
        match path {
            "/dauntless-status" => (200, serde_json::json!({ "show-status": true, "name": "Test Server" })),
            "/dainty/api/ServerStatus" if self.status_missing => (404, serde_json::json!({})),
            "/dainty/api/ServerStatus" => {
                let full = user.is_some();
                let players = if full {
                    serde_json::json!([{ "name": "Aurora", "where": "city", "instance": "city-8777" }])
                } else {
                    serde_json::json!([])
                };
                (
                    200,
                    serde_json::json!({
                        "name": "Test Server", "online": true, "version": "0.1.0", "commit": "abc1234",
                        "registration": "INVITECODE", "playersOnline": u8::from(full), "players": players,
                        "game": { "build": "rel-1.14.7-Archon", "changelist": 647472, "executableSha256": null },
                        "gameData": "real", "chat": true, "limited": !full,
                    }),
                )
            }
            "/dainty/api/GetUserInfo" => match user {
                Some((username, id)) => (
                    200,
                    serde_json::json!({ "UserId": id, "Username": username, "IsAdmin": false }),
                ),
                None => (401, serde_json::json!({ "error": "unauthorized" })),
            },
            "/dainty/api/Register" if method == hyper::Method::POST => {
                let Ok(parsed) = serde_json::from_slice::<Value>(body) else {
                    return (400, serde_json::json!({ "error": "bad_request" }));
                };
                let username = parsed["Username"].as_str().unwrap_or_default().to_owned();
                if !dainty_protocol::account::is_valid_username(&username) {
                    return (400, serde_json::json!({ "error": "username_invalid" }));
                }
                if self
                    .users
                    .values()
                    .any(|(name, _)| name.eq_ignore_ascii_case(&username))
                {
                    return (409, serde_json::json!({ "error": "username_taken" }));
                }
                if !parsed["InviteCode"]
                    .as_str()
                    .is_some_and(|code| self.codes.remove(code))
                {
                    return (401, serde_json::json!({ "error": "invite_invalid" }));
                }
                self.registrations += 1;
                let key = format!("DLK_{:048x}", self.registrations);
                self.users
                    .insert(key.clone(), (username, format!("uid-{}", self.registrations)));
                (200, serde_json::json!({ "UUK": key }))
            }
            "/content/v1/runtime/manifest" => match &self.runtime {
                Some(manifest) => (200, manifest.clone()),
                None => (404, serde_json::json!({ "error": "no_runtime" })),
            },
            _ => (404, Value::Null),
        }
    }
}

type Shared = Arc<Mutex<Fake>>;

async fn serve(fake: Shared, tls: Option<TlsAcceptor>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else { return };
            let (fake, tls) = (fake.clone(), tls.clone());
            tokio::spawn(async move {
                let service = service_fn(move |request: Request<hyper::body::Incoming>| {
                    let fake = fake.clone();
                    async move {
                        let key = request
                            .headers()
                            .get(KEY_HEADER)
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_owned);
                        let (method, path) = (request.method().clone(), request.uri().path().to_owned());
                        let body = request
                            .into_body()
                            .collect()
                            .await
                            .map(|b| b.to_bytes())
                            .unwrap_or_default();
                        let (status, body) = fake.lock().unwrap().answer(&method, &path, key, &body);
                        let text = if body.is_null() {
                            String::new()
                        } else {
                            body.to_string()
                        };
                        let response = Response::builder()
                            .status(status)
                            .header("content-type", "application/json")
                            .body(Full::new(Bytes::from(text)))
                            .unwrap();
                        Ok::<_, Infallible>(response)
                    }
                });
                let http = hyper::server::conn::http1::Builder::new();
                match tls {
                    Some(tls) => {
                        if let Ok(stream) = tls.accept(tcp).await {
                            let _ = http.serve_connection(TokioIo::new(stream), service).await;
                        }
                    }
                    None => {
                        let _ = http.serve_connection(TokioIo::new(tcp), service).await;
                    }
                }
            });
        }
    });
    port
}

/// A fake server over TLS with its certificate's fingerprint, and the fingerprint of another certificate.
async fn pinned_server() -> (Shared, Endpoint, String) {
    let (cert, key) = certificate();
    let (other, _) = certificate();
    let fingerprint = certificate_fingerprint(&cert);
    let fake = Arc::new(Mutex::new(Fake::new()));
    let port = serve(fake.clone(), Some(acceptor(cert, key))).await;
    let endpoint = Endpoint {
        host: "127.0.0.1".into(),
        port,
        pin: Some(fingerprint),
    };
    (fake, endpoint, certificate_fingerprint(&other))
}

fn seen(fake: &Shared) -> Vec<(String, Option<String>)> {
    fake.lock().unwrap().seen.clone()
}

// Reforged's pinned.test.ts, one test per case

#[tokio::test]
async fn status_over_the_pin_the_full_list_with_the_key_the_limited_view_without() {
    let (fake, endpoint, other) = pinned_server().await;
    match fetch_status(&endpoint, None, None).await {
        StatusOutcome::Status(status) => assert!(status.limited && status.players.is_empty()),
        answer => panic!("{answer:?}"),
    }
    match fetch_status(&endpoint, Some(KEY), None).await {
        StatusOutcome::Status(status) => {
            assert!(!status.limited);
            let names: Vec<&str> = status.players.iter().map(|p| p.name.as_str()).collect();
            assert_eq!(names, ["Aurora"]);
            let game = status.game.unwrap();
            assert_eq!(
                (game.build.as_str(), game.changelist),
                ("rel-1.14.7-Archon", Some(647472))
            );
            assert_eq!(serde_json::to_value(status.game_data).unwrap(), "real");
        }
        answer => panic!("{answer:?}"),
    }
    assert_eq!(
        seen(&fake),
        [
            ("/dainty/api/ServerStatus".to_owned(), None),
            ("/dainty/api/ServerStatus".to_owned(), Some(KEY.to_owned())),
        ]
    );

    let before = seen(&fake).len();
    let mismatched = Endpoint {
        pin: Some(other),
        ..endpoint.clone()
    };
    assert!(matches!(
        fetch_status(&mismatched, Some(KEY), None).await,
        StatusOutcome::CertMismatch { .. }
    ));
    assert_eq!(seen(&fake).len(), before, "a mismatching server never sees a request");

    // A public address over plain HTTP is refused before connecting
    let public_plain = Endpoint {
        host: "203.0.113.10".into(),
        port: endpoint.port,
        pin: None,
    };
    assert_eq!(
        fetch_status(&public_plain, Some(KEY), Some(Duration::from_millis(2000))).await,
        StatusOutcome::Unreachable(FailureKind::Other)
    );
}

#[tokio::test]
async fn an_older_server_is_recognised_through_dauntless_status_which_never_gets_the_key() {
    let fake = Arc::new(Mutex::new(Fake {
        status_missing: true,
        ..Fake::new()
    }));
    let port = serve(fake.clone(), None).await;
    let endpoint = Endpoint {
        host: "127.0.0.1".into(),
        port,
        pin: None,
    };
    assert_eq!(
        fetch_status(&endpoint, Some(KEY), None).await,
        StatusOutcome::Unsupported
    );
    assert_eq!(
        seen(&fake),
        [
            ("/dainty/api/ServerStatus".to_owned(), Some(KEY.to_owned())),
            ("/dauntless-status".to_owned(), None),
        ]
    );

    // Nobody listening
    // Refused on Linux; Windows retries a refused connect past the timeout
    let closed = Endpoint { port: 1, ..endpoint };
    assert!(matches!(
        fetch_status(&closed, None, Some(Duration::from_millis(500))).await,
        StatusOutcome::Unreachable(FailureKind::Refused | FailureKind::Timeout)
    ));
}

#[tokio::test]
async fn register_and_user_info_over_the_pin_and_nothing_sent_to_a_mismatching_server() {
    let (fake, endpoint, other) = pinned_server().await;
    let mismatched = Endpoint {
        pin: Some(other),
        ..endpoint.clone()
    };
    assert_eq!(
        register(&mismatched, &request("Borealis", "GOOD-CODE")).await,
        Err(LauncherError::new(ErrorCode::CertMismatch))
    );
    assert!(seen(&fake).is_empty());

    let key = register(&endpoint, &request("Borealis", "GOOD-CODE")).await.unwrap();
    assert!(key.starts_with("DLK_") && key.len() == 52 && key[4..].bytes().all(|b| b.is_ascii_hexdigit()));
    let info = fetch_user_info(&endpoint, &key).await.unwrap();
    assert_eq!(info.username, "Borealis");
    assert_eq!(
        seen(&fake).last().unwrap(),
        &("/dainty/api/GetUserInfo".to_owned(), Some(key.clone()))
    );
    assert_eq!(
        fetch_user_info(&mismatched, &key).await,
        Err(LauncherError::new(ErrorCode::CertMismatch))
    );

    let failed = |code| Err(LauncherError::new(code));
    // The same code twice: used up
    assert_eq!(
        register(&endpoint, &request("Cirrus", "GOOD-CODE")).await,
        failed(ErrorCode::InviteInvalid)
    );
    assert_eq!(
        register(&endpoint, &request("borealis", "X")).await,
        failed(ErrorCode::UsernameTaken)
    );
    assert_eq!(
        register(&endpoint, &request("no", "X")).await,
        failed(ErrorCode::UsernameInvalid)
    );
    assert_eq!(fake.lock().unwrap().registrations, 1);
    // Register never carries a key
    assert!(
        seen(&fake)
            .iter()
            .filter(|(path, _)| path == REFORGED.register_path)
            .all(|(_, key)| key.is_none())
    );
}

#[tokio::test]
async fn the_runtime_manifest_over_the_pin() {
    let (fake, endpoint, other) = pinned_server().await;
    match fetch_runtime_manifest(&endpoint).await {
        RuntimeManifestResult::Ok(manifest) => assert_eq!(manifest.build.changelist, 647472),
        answer => panic!("{answer:?}"),
    }
    let mismatched = Endpoint {
        pin: Some(other),
        ..endpoint.clone()
    };
    assert_eq!(
        fetch_runtime_manifest(&mismatched).await,
        RuntimeManifestResult::CertMismatch
    );
    fake.lock().unwrap().runtime = None;
    assert_eq!(fetch_runtime_manifest(&endpoint).await, RuntimeManifestResult::None);
    fake.lock().unwrap().runtime = Some(serde_json::json!({ "version": "1", "files": [] }));
    assert_eq!(
        fetch_runtime_manifest(&endpoint).await,
        RuntimeManifestResult::BadAnswer
    );
    // The manifest is public: no key goes with it
    assert!(seen(&fake).iter().all(|(_, key)| key.is_none()));
}

#[tokio::test]
async fn user_info_sends_no_key_where_the_rule_refuses_it() {
    // Plain HTTP to a public address is refused by request() before connecting; the header is never built
    let public_plain = Endpoint {
        host: "203.0.113.10".into(),
        port: 80,
        pin: None,
    };
    assert_eq!(
        fetch_user_info(&public_plain, KEY).await,
        Err(LauncherError::new(ErrorCode::ServerUnreachable))
    );
    assert_eq!(
        fetch_user_info(&public_plain, "bad key").await,
        Err(LauncherError::new(ErrorCode::KeyRejected))
    );
}
