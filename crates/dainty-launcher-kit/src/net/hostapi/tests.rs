// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The requests against a fake server on loopback, in clear and over pinned TLS with a throwaway certificate.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::sync::{Arc, Mutex};

use dainty_protocol::account::RegisterRequest;
use serde_json::json;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;

use super::*;
use crate::net::pinned::certificate_fingerprint;
use crate::net::pinned::tests::{acceptor, certificate};

const KEY: &str = concat!("RRK_", "0123456789abcdef", "0123456789abcdef", "0123456789abcdef");
const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

type Seen = Arc<Mutex<Vec<String>>>;

/// A server answering each path from `routes` (404 otherwise) and keeping every request it read.
struct FakeServer {
    port: u16,
    pin: Option<String>,
    seen: Seen,
}

impl FakeServer {
    async fn start(tls: bool, routes: Vec<(&'static str, u16, Vec<u8>)>) -> Self {
        let (acceptor, pin) = if tls {
            let (cert, key) = certificate();
            let pin = certificate_fingerprint(cert.as_ref());
            (Some(acceptor(cert, key)), Some(pin))
        } else {
            (None, None)
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen: Seen = Arc::default();
        let routes = Arc::new(routes);
        let log = seen.clone();
        tokio::spawn(async move {
            while let Ok((tcp, _)) = listener.accept().await {
                let (routes, log, acceptor) = (routes.clone(), log.clone(), acceptor.clone());
                tokio::spawn(async move {
                    match acceptor {
                        Some(tls) => {
                            if let Ok(stream) = tls.accept(tcp).await {
                                answer(stream, &routes, &log).await;
                            }
                        }
                        None => answer(tcp, &routes, &log).await,
                    }
                });
            }
        });
        Self { port, pin, seen }
    }

    fn endpoint(&self) -> Endpoint {
        Endpoint {
            host: "127.0.0.1".to_owned(),
            port: self.port,
            pin: self.pin.clone(),
        }
    }

    fn wrong_pin(&self) -> Endpoint {
        Endpoint {
            pin: Some("0".repeat(64)),
            ..self.endpoint()
        }
    }

    fn requests(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
}

async fn read_request<S: AsyncRead + Unpin>(stream: &mut S) -> String {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).await.unwrap_or(0) == 0 {
            break;
        }
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    let length = head
        .lines()
        .find_map(|line| line.strip_prefix("content-length: "))
        .map_or(0, |len| len.trim().parse::<usize>().unwrap());
    let mut body = vec![0u8; length];
    let _ = stream.read_exact(&mut body).await;
    head + &String::from_utf8_lossy(&body)
}

async fn answer<S: AsyncRead + AsyncWrite + Unpin>(mut stream: S, routes: &[(&str, u16, Vec<u8>)], seen: &Seen) {
    let request = read_request(&mut stream).await;
    let path = request.split(' ').nth(1).unwrap_or_default().to_owned();
    seen.lock().unwrap().push(request);
    let (status, body) = routes
        .iter()
        .find(|(route, _, _)| *route == path)
        .map_or((404, Vec::new()), |(_, status, body)| (*status, body.clone()));
    let head = format!(
        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(&body).await;
    let _ = stream.flush().await;
}

fn json_body(value: &serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}

fn status_route() -> (&'static str, u16, Vec<u8>) {
    (
        STATUS_PATH,
        200,
        json_body(&json!({"name": "Fake Isle", "online": true})),
    )
}

fn has_key(request: &str) -> bool {
    request.contains(KEY_HEADER) || request.contains(KEY)
}

fn closed_endpoint() -> Endpoint {
    // Port 9 is never listened on here
    Endpoint {
        host: "127.0.0.1".to_owned(),
        port: 9,
        pin: None,
    }
}

#[tokio::test]
async fn status_carries_the_key_in_clear_only_to_loopback() {
    let server = FakeServer::start(false, vec![status_route()]).await;
    let outcome = fetch_status(&server.endpoint(), Some(KEY), None).await;
    let StatusOutcome::Status(status) = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(status.name, "Fake Isle");
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].starts_with(&format!("GET {STATUS_PATH} HTTP/1.1")));
    assert!(requests[0].contains(&format!("{KEY_HEADER}: {KEY}")), "{}", requests[0]);
}

#[tokio::test]
async fn a_missing_status_route_asks_the_probe_without_the_key() {
    let probe = (PROBE_PATH, 200, json_body(&json!({"show-status": true})));
    let server = FakeServer::start(false, vec![probe]).await;
    assert_eq!(
        fetch_status(&server.endpoint(), Some(KEY), None).await,
        StatusOutcome::Unsupported
    );
    let requests = server.requests();
    assert_eq!(requests.len(), 2);
    assert!(has_key(&requests[0]));
    assert!(requests[1].starts_with(&format!("GET {PROBE_PATH} ")));
    assert!(!has_key(&requests[1]), "{}", requests[1]);

    let server = FakeServer::start(false, vec![(PROBE_PATH, 200, json_body(&json!({})))]).await;
    assert_eq!(
        fetch_status(&server.endpoint(), None, None).await,
        StatusOutcome::BadAnswer
    );
}

#[tokio::test]
async fn status_over_pinned_tls_carries_the_key() {
    let server = FakeServer::start(true, vec![status_route()]).await;
    let outcome = fetch_status(&server.endpoint(), Some(KEY), None).await;
    assert!(matches!(outcome, StatusOutcome::Status(_)), "{outcome:?}");
    let requests = server.requests();
    assert!(requests[0].contains(&format!("{KEY_HEADER}: {KEY}")));
}

#[tokio::test]
async fn a_pin_mismatch_is_reported_and_no_route_sees_the_key() {
    let server = FakeServer::start(true, vec![status_route()]).await;
    let actual = server.pin.clone().unwrap();
    let wrong = server.wrong_pin();
    assert_eq!(
        fetch_status(&wrong, Some(KEY), None).await,
        StatusOutcome::CertMismatch { actual: actual.clone() }
    );
    assert_eq!(
        fetch_user_info(&wrong, KEY).await,
        Err(LauncherError::new(ErrorCode::CertMismatch))
    );
    let body = RegisterRequest {
        username: "spud".to_owned(),
        invite_code: "CODE-1234".to_owned(),
    };
    assert_eq!(
        register(&wrong, &body).await,
        Err(LauncherError::new(ErrorCode::CertMismatch))
    );
    assert_eq!(
        fetch_runtime_manifest(&wrong).await,
        RuntimeManifestResult::CertMismatch
    );
    let fallback = [NewsItem {
        date: None,
        title: "Old".to_owned(),
        body: "Kept".to_owned(),
    }];
    assert_eq!(fetch_news(&wrong, &fallback).await, fallback);
    assert_eq!(fetch_branding(&wrong).await, Branding::default());
    assert_eq!(fetch_branding_image(&wrong, "/content/v1/branding/a.png").await, None);
    // The handshake fails before a request is written
    assert!(server.requests().is_empty(), "{:?}", server.requests());
}

#[tokio::test]
async fn user_info_over_pinned_tls() {
    let info = json!({"UserId": "u-1", "Username": "Spud", "IsAdmin": true});
    let server = FakeServer::start(true, vec![(USER_INFO_PATH, 200, json_body(&info))]).await;
    assert_eq!(
        fetch_user_info(&server.endpoint(), KEY).await,
        Ok(UserInfo {
            user_id: "u-1".to_owned(),
            username: "Spud".to_owned(),
            is_admin: true,
        })
    );
    let requests = server.requests();
    assert!(requests[0].starts_with(&format!("GET {USER_INFO_PATH} ")));
    assert!(requests[0].contains(&format!("{KEY_HEADER}: {KEY}")));
}

#[tokio::test]
async fn an_implausible_key_is_rejected_without_asking() {
    assert_eq!(
        fetch_user_info(&closed_endpoint(), "not a key").await,
        Err(LauncherError::new(ErrorCode::KeyRejected))
    );
}

#[tokio::test]
async fn register_posts_the_request_without_a_key() {
    let server = FakeServer::start(true, vec![(REGISTER_PATH, 200, json_body(&json!({"UUK": KEY})))]).await;
    let body = RegisterRequest {
        username: "spud".to_owned(),
        invite_code: "CODE-1234".to_owned(),
    };
    assert_eq!(register(&server.endpoint(), &body).await, Ok(KEY.to_owned()));
    let request = &server.requests()[0];
    assert!(request.starts_with(&format!("POST {REGISTER_PATH} HTTP/1.1")));
    assert!(request.contains("content-type: application/json"));
    assert!(
        request.ends_with(r#"{"Username":"spud","InviteCode":"CODE-1234"}"#),
        "{request}"
    );
    assert!(!request.contains(KEY_HEADER));

    let taken = FakeServer::start(false, vec![(REGISTER_PATH, 409, Vec::new())]).await;
    assert_eq!(
        register(&taken.endpoint(), &body).await,
        Err(LauncherError::new(ErrorCode::UsernameTaken))
    );
}

#[tokio::test]
async fn no_answer_is_unreachable() {
    let endpoint = closed_endpoint();
    assert_eq!(
        fetch_status(&endpoint, Some(KEY), None).await,
        StatusOutcome::Unreachable
    );
    assert_eq!(
        fetch_user_info(&endpoint, KEY).await,
        Err(LauncherError::new(ErrorCode::ServerUnreachable))
    );
    assert_eq!(
        fetch_runtime_manifest(&endpoint).await,
        RuntimeManifestResult::Unreachable
    );
}

#[tokio::test]
async fn content_routes_never_carry_the_key() {
    let manifest = json!({
        "version": "1.0",
        "files": [
            {"name": "winmm.dll", "size": 2, "sha256": SHA_B},
            {"name": "Rustic-Isles.dll", "size": 1, "sha256": SHA_A},
        ],
    });
    let news = json!({"items": [{"date": "2026-10-31", "title": "Hello", "body": "World"}]});
    let branding = json!({"backgrounds": [{"url": "/content/v1/branding/a.png", "credit": "Me"}], "accent": "#ABCDEF"});
    let png = b"\x89PNG\r\n\x1a\nrest".to_vec();
    let server = FakeServer::start(
        true,
        vec![
            (RUNTIME_MANIFEST_PATH, 200, json_body(&manifest)),
            (NEWS_PATH, 200, json_body(&news)),
            (BRANDING_PATH, 200, json_body(&branding)),
            ("/content/v1/branding/a.png", 200, png.clone()),
        ],
    )
    .await;
    let endpoint = server.endpoint();

    let RuntimeManifestResult::Ok(manifest) = fetch_runtime_manifest(&endpoint).await else {
        panic!("no manifest");
    };
    assert_eq!(manifest.files[0].name, "Rustic-Isles.dll");
    let items = fetch_news(&endpoint, &[]).await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].title, "Hello");
    let look = fetch_branding(&endpoint).await;
    assert_eq!(look.accent.as_deref(), Some("#abcdef"));
    assert_eq!(look.backgrounds[0].url, "/content/v1/branding/a.png");
    let image = fetch_branding_image(&endpoint, &look.backgrounds[0].url).await.unwrap();
    assert_eq!((image.kind, image.bytes.to_vec()), (ImageType::Png, png));

    let requests = server.requests();
    assert_eq!(requests.len(), 4);
    assert!(requests.iter().all(|r| !has_key(r)), "{requests:?}");
}

#[tokio::test]
async fn a_path_outside_branding_is_never_asked() {
    let server = FakeServer::start(false, vec![("/secret.png", 200, b"\x89PNG\r\n\x1a\n".to_vec())]).await;
    assert_eq!(fetch_branding_image(&server.endpoint(), "/secret.png").await, None);
    assert!(server.requests().is_empty());
}

#[tokio::test]
async fn news_keeps_the_old_list_on_a_bad_answer() {
    let server = FakeServer::start(false, vec![(NEWS_PATH, 500, json_body(&json!({"items": []})))]).await;
    let old = vec![NewsItem {
        date: None,
        title: "Old".to_owned(),
        body: "Kept".to_owned(),
    }];
    assert_eq!(fetch_news(&server.endpoint(), &old).await, old);
}

#[test]
fn the_flavor_names_the_protocol_routes() {
    assert_eq!(REFORGED.status_path, dainty_protocol::status::STATUS_PATH);
    assert_eq!(REFORGED.register_path, dainty_protocol::account::REGISTER_PATH);
    assert_eq!(
        REFORGED.runtime_manifest_path,
        dainty_protocol::manifest::RUNTIME_MANIFEST_PATH
    );
    assert_eq!(REFORGED.key_header, KEY_HEADER);
}
