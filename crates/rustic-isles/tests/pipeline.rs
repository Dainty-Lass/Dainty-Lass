// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The pipeline black-box through `app()` over a bare composition (core spec §13.2).

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::thread::ThreadId;

use axum::body::Body;
use axum::extract::{ConnectInfo, Request};
use axum::response::IntoResponse;
use dainty_service_kit::serve::ConnectionInfo;
use http::StatusCode;
use http_body_util::BodyExt;
use rustic_isles::handlers::auth_handlers;
use rustic_isles::testing::sample_version;
use rustic_isles::{Handlers, Method, MountError, Service, Settings, Shared, SharedParts, Side, When, routes};
use rustic_rifts_seam::{
    BoxFuture, Component, DedicatedOnly, GuardHooks, HostRouteSample, HostWrite, HostWriteKind, HostWriteOptions,
    InventoryDecision, InventoryTransaction, Json, Lifecycle, Reply, RequestInfo, RiftsParts, RuntimeAuth, SeamError,
    StubRoutes, TableSpec,
};
use tower::ServiceExt;

struct Answer {
    status: StatusCode,
    headers: http::HeaderMap,
    body: String,
}

impl Answer {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }
}

struct Harness {
    app: axum::Router,
    shared: Arc<Shared>,
}

#[derive(Default)]
struct Options {
    settings: Vec<(&'static str, &'static str)>,
    admin_api: bool,
    rifts: bool,
    handlers: Handlers,
    admin: Option<Arc<dyn rustic_isles::AdminAuthenticator>>,
}

fn ok_handler(handlers: &mut Handlers, method: Method, path: &str) {
    handlers.on(Service::Backend, method, path, |_| async { "ok".into_response() });
}

impl Harness {
    fn new(options: Options) -> Self {
        let mut parts = SharedParts::bare(sample_version());
        parts.settings = Settings::from_pairs(options.settings.iter().copied());
        parts.admin_api = options.admin_api;
        parts.admin_authenticator = options.admin;
        if options.rifts {
            parts.rifts = rifts();
        }
        let mut handlers = auth_handlers().unwrap();
        handlers.extend(options.handlers);
        parts.handlers = handlers.clone();
        let shared = Shared::new(parts).unwrap();
        Self {
            app: rustic_isles::app(shared.clone(), Service::Backend, &handlers).unwrap(),
            shared,
        }
    }

    fn bare() -> Self {
        Self::new(Options::default())
    }

    async fn send(&self, method: &str, uri: &str, headers: &[(&str, &str)], peer: &str) -> Answer {
        let mut builder = Request::builder().method(method).uri(uri).header("host", "localhost");
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let mut request = builder.body(Body::empty()).unwrap();
        let peer: SocketAddr = peer.parse().unwrap();
        request.extensions_mut().insert(ConnectInfo(ConnectionInfo {
            peer,
            local: "127.0.0.1:61000".parse().unwrap(),
        }));
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        Answer {
            status,
            headers,
            body: String::from_utf8_lossy(&body).into_owned(),
        }
    }

    async fn get(&self, uri: &str) -> Answer {
        self.send("GET", uri, &[], "127.0.0.1:5000").await
    }
}

fn message(answer: &Answer) -> String {
    let value: serde_json::Value = serde_json::from_str(&answer.body).unwrap();
    value["error"]["message"].as_str().unwrap().to_owned()
}

/// A guard that names `host:<capability>` for `cap-*`, fails for `cap-err`, and has no bucket otherwise.
struct Guard;

impl Lifecycle for Guard {
    fn start(&self, role: &str) -> Result<(), SeamError> {
        DedicatedOnly.start(role)
    }
    fn stop(&self) -> BoxFuture<'_, Result<(), SeamError>> {
        Box::pin(async { Ok(()) })
    }
    fn startup_summary(&self, role: &str) -> Result<Option<Json>, SeamError> {
        DedicatedOnly.startup_summary(role)
    }
    fn runtime_safety_issues(&self, settings: &dyn rustic_rifts_seam::Settings) -> Result<Vec<String>, SeamError> {
        DedicatedOnly.runtime_safety_issues(settings)
    }
    fn tables(&self) -> &'static [TableSpec] {
        &[]
    }
    fn account_deletion_steps(&self) -> Vec<rustic_rifts_seam::DeletionStep> {
        vec![]
    }
}

impl GuardHooks for Guard {
    fn guard_manifest_roles(&self) -> &'static [&'static str] {
        &[]
    }
    fn service_roles(&self) -> &'static [&'static str] {
        &[]
    }
    fn host_policy_refuses(&self) -> Result<bool, SeamError> {
        Ok(true)
    }
    fn note_would_refuse(&self, _: &str, _: &str) -> Result<(), SeamError> {
        Ok(())
    }
    fn census(&self, _: &HostRouteSample) -> Result<(), SeamError> {
        Ok(())
    }
    fn rate_limit_bucket(&self, request: RequestInfo) -> BoxFuture<'_, Result<Option<String>, SeamError>> {
        let capability = request.header("x-rustic-host-capability").unwrap_or("").to_owned();
        Box::pin(async move {
            match capability.as_str() {
                "cap-err" => Err(SeamError::unimplemented(Component::Guard, "bucket")),
                c if c.starts_with("cap-") => Ok(Some(format!("host:{c}"))),
                _ => Ok(None),
            }
        })
    }
    fn authorize_host_write(
        &self,
        _: RuntimeAuth,
        _: HostWriteKind,
        _: rustic_rifts_seam::AccountId,
        _: HostWriteOptions,
    ) -> BoxFuture<'_, Result<Option<Reply>, SeamError>> {
        Box::pin(async { Ok(None) })
    }
    fn record_host_write(&self, _: RuntimeAuth, _: HostWrite) -> BoxFuture<'_, Result<(), SeamError>> {
        Box::pin(async { Ok(()) })
    }
    fn authorize_inventory(
        &self,
        _: RuntimeAuth,
        _: InventoryTransaction,
    ) -> BoxFuture<'_, Result<InventoryDecision, SeamError>> {
        Box::pin(async {
            Ok(InventoryDecision::Refused(Reply {
                status: 403,
                body: Json::empty_object(),
            }))
        })
    }
}

/// Rifts loaded, with a stub answering every `rifts` row "not implemented".
fn rifts() -> rustic_rifts_seam::Rifts {
    let routes: Vec<rustic_rifts_seam::Route> = routes::all()
        .iter()
        .filter(|r| r.when == When::Rifts)
        .map(|r| rustic_rifts_seam::Route {
            method: match r.method {
                Method::Get => rustic_rifts_seam::Method::Get,
                Method::Patch => rustic_rifts_seam::Method::Patch,
                _ => rustic_rifts_seam::Method::Post,
            },
            path: r.path,
            mount: rustic_rifts_seam::Mount::Normal,
            handler: "stub.route",
        })
        .collect();
    let routes: &'static [rustic_rifts_seam::Route] = Box::leak(routes.into_boxed_slice());
    let dedicated = Arc::new(DedicatedOnly);
    rustic_rifts_seam::Rifts::new(RiftsParts {
        realms: dedicated.clone(),
        entry: dedicated,
        guard: Arc::new(Guard),
        routes: vec![Arc::new(StubRoutes {
            component: Component::Guard,
            routes,
        })],
    })
}

#[test]
fn the_table() {
    let all = routes::all();
    assert_eq!(all.len(), 295);
    let backend: Vec<_> = all.iter().filter(|r| r.service == Service::Backend).collect();
    assert_eq!(backend.len(), 290);
    assert_eq!(backend.iter().filter(|r| r.side == Side::Isles).count(), 223);
    assert_eq!(backend.iter().filter(|r| r.side == Side::Rifts).count(), 67);
    let director: Vec<_> = all.iter().filter(|r| r.service == Service::Director).collect();
    assert_eq!(director.len(), 5);
    assert_eq!(director.iter().filter(|r| r.side == Side::Isles).count(), 4);
    assert_eq!(rustic_isles::RouteSet::V1147.routes(Service::Backend).len(), 290);
    let admin: Vec<_> = all.iter().filter(|r| r.when == When::Admin).collect();
    assert_eq!(admin.len(), 31);
    assert!(
        admin
            .iter()
            .all(|r| r.path.starts_with("/admin/") || r.path == "/health/release-gates")
    );
    assert!(
        all.iter()
            .filter(|r| r.path.starts_with("/admin/"))
            .all(|r| r.when != When::Always)
    );
    let gauntlet: Vec<_> = all.iter().filter(|r| r.when == When::Gauntlet).collect();
    assert_eq!(gauntlet.len(), 5);
    assert!(gauntlet.iter().any(|r| r.path == "/config"));
    assert_eq!(all.iter().filter(|r| r.when == When::Rifts).count(), 60);
    assert!(
        all.iter()
            .filter(|r| r.side == Side::Isles)
            .all(|r| r.when != When::Rifts)
    );
    let kept: Vec<_> = all
        .iter()
        .filter(|r| r.side == Side::Rifts && r.when != When::Rifts)
        .collect();
    assert_eq!(kept.len(), 8);
    assert!(kept.iter().all(|r| r.path.contains("guard")));
    for row in all {
        rustic_isles::Pattern::parse(row.path).unwrap();
    }
}

#[tokio::test]
async fn rows_without_handlers_answer_501() {
    let t = Harness::bare();
    let answer = t.get("/character").await;
    assert_eq!(answer.status, StatusCode::NOT_IMPLEMENTED);
    assert_eq!(answer.header("content-type"), Some("application/json; charset=utf-8"));
    let id = answer.header("x-rustic-isles-request-id").unwrap();
    assert_eq!(
        answer.body,
        format!(
            r#"{{"error":{{"code":"NOT_IMPLEMENTED","message":"isles: GET /character is not implemented yet","requestId":"{id}"}}}}"#
        )
    );
    let head = t.send("HEAD", "/CHARACTER/", &[], "127.0.0.1:5000").await;
    assert_eq!((head.status, head.body.as_str()), (StatusCode::NOT_IMPLEMENTED, ""));
    let none = t.get("/nothing/here").await;
    assert_eq!((none.status, none.body.as_str()), (StatusCode::NOT_FOUND, ""));
    assert!(none.header("content-type").is_none());
    assert!(none.header("x-rustic-isles-request-id").is_some());
    let options = t.send("OPTIONS", "/character", &[], "127.0.0.1:5000").await;
    assert_eq!(options.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_order_clients_see() {
    let t = Harness::bare();
    assert_eq!(
        message(&t.get("/progression/a/b").await),
        "isles: GET /progression/:userId/:progressionId is not implemented yet"
    );
    assert_eq!(
        message(&t.get("/__origin/gauntlet-prod.steelyard.ca/progression/a/b").await),
        r"isles: GET /^\/progression\/([^/]*)\/([^/]+)$/i is not implemented yet"
    );
    assert_eq!(
        message(&t.get("/progression/objectives/u").await),
        "isles: GET /progression/objectives/:userId is not implemented yet"
    );
    let inventory = t
        .send("POST", "/inventory//dauntlessrel-1.14.7:647472", &[], "127.0.0.1:5000")
        .await;
    assert_eq!(
        message(&inventory),
        r"isles: POST /^\/inventory\/([^/]*)\/(.+)$/ is not implemented yet"
    );
    let evil = t.get("/__origin/evil.example/character").await;
    assert_eq!((evil.status, evil.body.as_str()), (StatusCode::BAD_REQUEST, ""));
    assert_eq!(
        message(&t.get("/__origin/Gauntlet-Prod.steelyard.ca/config?x=1").await),
        "isles: GET /config is not implemented yet"
    );
    assert_eq!(t.get("/config").await.status, StatusCode::NOT_FOUND);
    assert_eq!(t.get("/x/%E2%28").await.status, StatusCode::NOT_FOUND);
    assert_eq!(t.get("/bounty/%E2%28").await.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn the_admin_gate() {
    let off = Harness::bare();
    for path in ["/admin/v1/me", "/Admin", "/health/release-gates"] {
        let answer = off.get(path).await;
        assert_eq!(
            (answer.status, answer.body.as_str()),
            (StatusCode::NOT_FOUND, ""),
            "{path}"
        );
    }
    let on = Harness::new(Options {
        admin_api: true,
        ..Options::default()
    });
    assert_eq!(on.get("/admin/v1/me").await.status, StatusCode::NOT_IMPLEMENTED);
    assert_eq!(
        on.get("/health/release-gates").await.status,
        StatusCode::NOT_IMPLEMENTED
    );
    assert_eq!(on.get("/Admin").await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn handlers_replace_the_501() {
    let mut handlers = Handlers::new();
    ok_handler(&mut handlers, Method::Get, "/");
    let t = Harness::new(Options {
        handlers,
        ..Options::default()
    });
    let answer = t.get("/").await;
    assert_eq!((answer.status, answer.body.as_str()), (StatusCode::OK, "ok"));

    let mut typo = Handlers::new();
    ok_handler(&mut typo, Method::Get, "/charactr");
    let shared = Shared::bare(sample_version()).unwrap();
    let error = rustic_isles::app(shared, Service::Backend, &typo).unwrap_err();
    assert!(matches!(error, MountError::UnknownRoute { .. }));
}

#[tokio::test]
async fn request_ids() {
    let t = Harness::bare();
    let t = &t;
    let with = |id: &'static str| async move {
        t.send(
            "GET",
            "/character",
            &[("x-rustic-isles-request-id", id)],
            "127.0.0.1:5000",
        )
        .await
    };
    let kept = with("abcd-1234").await;
    assert_eq!(kept.header("x-rustic-isles-request-id"), Some("abcd-1234"));
    assert!(kept.body.contains(r#""requestId":"abcd-1234""#));
    let short = with("short").await;
    assert_eq!(short.header("x-rustic-isles-request-id").unwrap().len(), 36);
    let dash = with("-abcdefgh").await;
    assert_eq!(dash.header("x-rustic-isles-request-id").unwrap().len(), 36);
    let legacy = t
        .send(
            "GET",
            "/character",
            &[("x-request-id", "from-the-gateway")],
            "127.0.0.1:5000",
        )
        .await;
    assert_eq!(legacy.header("x-rustic-isles-request-id"), Some("from-the-gateway"));
}

const PINNED: (&str, &str) = ("x-isles-replay-now-ms", "1800000000000");

fn budget_settings(extra: &[(&'static str, &'static str)]) -> Vec<(&'static str, &'static str)> {
    let mut settings = vec![
        ("RUSTIC_ISLES_REPLAY_HEADERS", "on"),
        ("RUSTIC_ISLES_RATE_LIMIT_PER_MINUTE", "2"),
        ("RUSTIC_ISLES_GAMESERVER_RATE_LIMIT_PER_MINUTE", "3"),
        ("RUSTIC_ISLES_HOST_RATE_LIMIT_PER_MINUTE", "2"),
    ];
    settings.extend_from_slice(extra);
    settings
}

#[tokio::test]
async fn budget_buckets() {
    let t = Harness::new(Options {
        settings: budget_settings(&[]),
        rifts: true,
        ..Options::default()
    });
    let hit = |extra: &'static [(&'static str, &'static str)], path: &'static str| {
        let mut headers = vec![PINNED, ("x-forwarded-for", "198.51.100.10")];
        headers.extend_from_slice(extra);
        let t = &t;
        async move { t.send("GET", path, &headers, "127.0.0.1:5000").await }
    };
    let statuses = |answers: &[Answer]| answers.iter().map(|a| a.status.as_u16()).collect::<Vec<_>>();
    let client = [
        hit(&[], "/isbanned").await,
        hit(&[], "/isbanned").await,
        hit(&[], "/isbanned").await,
    ];
    assert_eq!(statuses(&client), [200, 200, 429]);
    assert_eq!(client[2].body, r#"{"error":"Too many requests. Please retry later."}"#);
    assert_eq!(client[2].header("retry-after"), Some("60"));

    let key: &[(&str, &str)] = &[("x-rustic-isles-gameserver-apikey", "unverified-key")];
    assert_eq!(hit(key, "/isbanned").await.status, StatusCode::OK);
    let info = hit(key, "/accountinfo").await;
    assert_eq!(info.header("ratelimit"), Some(r#""3-in-1min"; r=1; t=60"#));
    assert_eq!(hit(key, "/isbanned").await.status, StatusCode::OK);
    assert_eq!(hit(key, "/isbanned").await.status, StatusCode::TOO_MANY_REQUESTS);

    let host: &[(&str, &str)] = &[("x-rustic-host-capability", " cap-123 ")];
    let hosts = [
        hit(host, "/isbanned").await,
        hit(host, "/isbanned").await,
        hit(host, "/isbanned").await,
    ];
    assert_eq!(statuses(&hosts), [200, 200, 429]);
    assert_eq!(
        hit(&[("x-rustic-host-capability", "cap-456")], "/isbanned")
            .await
            .status,
        StatusCode::OK
    );
    for capability in ["nope", "cap-err", "   "] {
        let headers: &'static [(&str, &str)] = Box::leak(Box::new([("x-rustic-host-capability", capability)]));
        let answer = hit(headers, "/isbanned").await;
        assert_eq!(answer.status, StatusCode::TOO_MANY_REQUESTS, "{capability}");
        assert_eq!(answer.header("ratelimit"), Some(r#""2-in-1min"; r=0; t=60"#));
    }
    let empty_key = t
        .send(
            "GET",
            "/isbanned",
            &[PINNED, ("x-rustic-isles-gameserver-apikey", "")],
            "198.51.100.20:5000",
        )
        .await;
    assert_eq!(empty_key.header("ratelimit"), Some(r#""3-in-1min"; r=2; t=60"#));
}

#[tokio::test]
async fn the_budget_switch() {
    for off in ["0", "off", "OFF", " false ", "No"] {
        let settings: Vec<(&'static str, &'static str)> = vec![
            ("RUSTIC_ISLES_RATE_LIMIT", off),
            ("RUSTIC_ISLES_RATE_LIMIT_PER_MINUTE", "1"),
        ];
        let t = Harness::new(Options {
            settings,
            ..Options::default()
        });
        for _ in 0..4 {
            let answer = t.get("/isbanned").await;
            assert_eq!(answer.status, StatusCode::OK);
            assert!(answer.header("ratelimit").is_none() && answer.header("ratelimit-policy").is_none());
        }
    }
    for bad in ["0", "-10", "not-a-number"] {
        let settings: Vec<(&'static str, &'static str)> = vec![
            ("RUSTIC_ISLES_REPLAY_HEADERS", "on"),
            ("RUSTIC_ISLES_RATE_LIMIT_PER_MINUTE", bad),
            ("RUSTIC_ISLES_GAMESERVER_RATE_LIMIT_PER_MINUTE", bad),
            ("RUSTIC_ISLES_HOST_RATE_LIMIT_PER_MINUTE", bad),
        ];
        let t = Harness::new(Options {
            settings,
            rifts: true,
            ..Options::default()
        });
        let client = t.send("GET", "/isbanned", &[PINNED], "127.0.0.1:5000").await;
        assert_eq!(client.header("ratelimit"), Some(r#""600-in-1min"; r=599; t=60"#));
        let server = t
            .send(
                "GET",
                "/isbanned",
                &[PINNED, ("x-rustic-isles-gameserver-apikey", "k")],
                "127.0.0.1:5000",
            )
            .await;
        assert_eq!(server.header("ratelimit"), Some(r#""6000-in-1min"; r=5999; t=60"#));
        let host = t
            .send(
                "GET",
                "/isbanned",
                &[PINNED, ("x-rustic-host-capability", "cap-1")],
                "127.0.0.1:5000",
            )
            .await;
        assert_eq!(host.header("ratelimit"), Some(r#""6000-in-1min"; r=5999; t=60"#));
    }
}

#[tokio::test]
async fn budget_placement() {
    let mut handlers = Handlers::new();
    for path in ["/", "/health/live", "/health/ready", "/health/release-gates"] {
        ok_handler(&mut handlers, Method::Get, path);
    }
    ok_handler(&mut handlers, Method::Get, "/rustic/v1/hunts/:sessionId/status");
    let t = Harness::new(Options {
        settings: budget_settings(&[]),
        admin_api: true,
        rifts: true,
        handlers,
        admin: None,
    });
    let pinned = |path: &'static str| {
        let t = &t;
        async move { t.send("GET", path, &[PINNED], "127.0.0.1:5000").await }
    };
    let probes = ["/", "/health/live", "/health/ready", "/health/release-gates"];
    for _ in 0..5 {
        for probe in probes {
            let answer = pinned(probe).await;
            assert_eq!(answer.status, StatusCode::OK, "{probe}");
            assert!(answer.header("ratelimit").is_none(), "{probe}");
        }
    }
    let status = pinned("/rustic/v1/hunts/s1/status").await;
    assert_eq!(status.status, StatusCode::OK);
    assert_eq!(status.header("ratelimit"), Some(r#""2-in-1min"; r=1; t=60"#));
    let tail = pinned("/rustic/v1/hunts/unknown-tail").await;
    assert_eq!(tail.status, StatusCode::NOT_FOUND);
    assert_eq!(tail.header("ratelimit"), Some(r#""2-in-1min"; r=0; t=60"#));
    assert_eq!(
        pinned("/rustic/v1/hunts/s1/status").await.status,
        StatusCode::TOO_MANY_REQUESTS
    );
    for probe in probes {
        let answer = pinned(probe).await;
        assert_eq!(answer.status, StatusCode::OK);
        assert!(answer.header("ratelimit").is_none());
    }
}

#[tokio::test]
async fn client_addresses() {
    let t = Harness::new(Options {
        settings: budget_settings(&[("RUSTIC_ISLES_RATE_LIMIT_PER_MINUTE", "1")]),
        ..Options::default()
    });
    let from = |xff: &'static str, peer: &'static str| {
        let t = &t;
        async move {
            t.send("GET", "/isbanned", &[PINNED, ("x-forwarded-for", xff)], peer)
                .await
                .status
                .as_u16()
        }
    };
    assert_eq!(from("2001:db8:abcd:ef12:1:2:3:4", "127.0.0.1:1").await, 200);
    assert_eq!(from("2001:db8:abcd:ef99:9:8:7:6", "127.0.0.1:1").await, 429);
    assert_eq!(from("2001:db8:abcd:ee00::1", "127.0.0.1:1").await, 200);
    assert_eq!(from("203.0.113.1", "198.51.100.77:1").await, 200);
    assert_eq!(from("203.0.113.2", "198.51.100.77:1").await, 429);
}

#[tokio::test]
async fn replay_headers_are_ignored_unless_switched_on() {
    let settings = vec![("RUSTIC_ISLES_RATE_LIMIT_PER_MINUTE", "2")];
    let t = Harness::new(Options {
        settings,
        ..Options::default()
    });
    let answer = t.send("GET", "/isbanned", &[PINNED], "127.0.0.1:5000").await;
    let window = answer.header("ratelimit").unwrap();
    assert!(window.starts_with(r#""2-in-1min"; r=1; t="#));
    // A real clock: a hit pinned in 2027 would open a window a year from now and still say t=60
    let pinned_far = t
        .send("GET", "/isbanned", &[("x-isles-replay-now-ms", "0")], "127.0.0.1:5000")
        .await;
    assert_eq!(pinned_far.header("ratelimit").map(|h| h.contains("r=0")), Some(true));
}

#[tokio::test]
async fn rifts_rows_relay() {
    let t = Harness::new(Options {
        rifts: true,
        ..Options::default()
    });
    let launcher = t.send("GET", "/launcher/v1/realms/mine", &[], "127.0.0.1:5000").await;
    assert_eq!(launcher.status, StatusCode::UNAUTHORIZED);
    assert!(launcher.body.contains(r#""code":"AUTH_UNAUTHORIZED""#));
    let host = t
        .send(
            "POST",
            "/rustic/v1/hunts/s1/host-ready",
            &[("x-rustic-isles-request-id", "relay-12345")],
            "127.0.0.1:5000",
        )
        .await;
    assert_eq!(host.status, StatusCode::NOT_IMPLEMENTED);
    assert_eq!(
        host.body,
        r#"{"error":{"code":"NOT_IMPLEMENTED","message":"rifts-guard: stub.route is not implemented yet","requestId":"relay-12345"}}"#
    );
    assert_eq!(host.header("content-type"), Some("application/json; charset=utf-8"));
    // A player's own hunt reads stay behind launcher auth under the D3 name, and the old name is no alias
    let status = t.get("/rustic/v1/hunts/s1/status").await;
    assert_eq!(status.status, StatusCode::UNAUTHORIZED);
    let old = t.get("/mystic/v1/hunts/s1/status").await;
    assert_eq!((old.status, old.body.as_str()), (StatusCode::NOT_FOUND, ""));
    let bare = Harness::bare();
    assert_eq!(bare.get("/launcher/v1/realms/mine").await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn launcher_cors() {
    let t = Harness::bare();
    let preflight = t
        .send(
            "OPTIONS",
            "/launcher/v1/me",
            &[("origin", "https://tauri.localhost")],
            "127.0.0.1:5000",
        )
        .await;
    assert_eq!(preflight.status, StatusCode::NO_CONTENT);
    assert_eq!(
        preflight.header("access-control-allow-origin"),
        Some("https://tauri.localhost")
    );
    assert!(preflight.header("ratelimit").is_some());
    let refused = t
        .send(
            "GET",
            "/launcher/v1/me",
            &[("origin", "https://evil.example")],
            "127.0.0.1:5000",
        )
        .await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    let unrouted = t
        .send(
            "GET",
            "/launcher/v1/nothing",
            &[("origin", "http://localhost:1420")],
            "127.0.0.1:5000",
        )
        .await;
    assert_eq!(unrouted.status, StatusCode::NOT_FOUND);
    assert_eq!(unrouted.header("vary"), Some("Origin"));
    let game = t
        .send(
            "GET",
            "/character",
            &[("origin", "https://evil.example")],
            "127.0.0.1:5000",
        )
        .await;
    assert_eq!(game.status, StatusCode::NOT_IMPLEMENTED);
}

struct Keys;

impl rustic_isles::AdminAuthenticator for Keys {
    fn authenticate<'a>(
        &'a self,
        request: rustic_isles::AdminRequestHead<'a>,
    ) -> BoxFuture<'a, Result<rustic_isles::AdminCaller, axum::response::Response>> {
        let known = request.headers.get("x-admin-key").is_some_and(|v| v == "k1")
            && request.method == "PATCH"
            && request.target == "/admin/v1/operations/p2p-emergency-stop";
        Box::pin(async move {
            match known {
                true => Ok(rustic_isles::AdminCaller { key_id: "k1".into() }),
                false => Err(StatusCode::FORBIDDEN.into_response()),
            }
        })
    }
}

#[tokio::test]
async fn admin_rifts_rows_ask_the_authenticator() {
    let path = "/admin/v1/operations/p2p-emergency-stop";
    let empty = Harness::new(Options {
        admin_api: true,
        rifts: true,
        ..Options::default()
    });
    let refused = empty.send("PATCH", path, &[], "127.0.0.1:5000").await;
    assert_eq!(refused.status, StatusCode::UNAUTHORIZED);
    assert!(refused.body.contains(r#""code":"AUTH_UNAUTHORIZED""#));
    let slot = Harness::new(Options {
        admin_api: true,
        rifts: true,
        admin: Some(Arc::new(Keys)),
        ..Options::default()
    });
    assert_eq!(
        slot.send("PATCH", path, &[], "127.0.0.1:5000").await.status,
        StatusCode::FORBIDDEN
    );
    let relayed = slot
        .send("PATCH", path, &[("x-admin-key", "k1")], "127.0.0.1:5000")
        .await;
    assert_eq!(relayed.status, StatusCode::NOT_IMPLEMENTED);
}

#[tokio::test]
async fn the_budget_counts_before_launcher_cors() {
    let t = Harness::new(Options {
        settings: budget_settings(&[("RUSTIC_ISLES_RATE_LIMIT_PER_MINUTE", "1")]),
        ..Options::default()
    });
    let lines = capture_logs();
    let rejected = || {
        String::from_utf8_lossy(&lines.lock().unwrap())
            .matches("[CORS] Rejected")
            .count()
    };
    let evil = [PINNED, ("origin", "https://evil.example")];
    let first = t.send("GET", "/launcher/v1/me", &evil, "127.0.0.1:5000").await;
    assert_eq!(first.status, StatusCode::FORBIDDEN);
    assert_eq!(first.header("ratelimit"), Some(r#""1-in-1min"; r=0; t=60"#));
    // Refused by the budget before CORS could refuse the origin, routed or not
    for path in ["/launcher/v1/me", "/launcher/v1/nothing"] {
        let answer = t.send("GET", path, &evil, "127.0.0.1:5000").await;
        assert_eq!(answer.status, StatusCode::TOO_MANY_REQUESTS, "{path}");
        assert!(answer.header("access-control-allow-origin").is_none());
    }
    assert_eq!(rejected(), 1, "CORS was asked for a request the budget refused");
}

type Sink = Arc<std::sync::Mutex<Vec<u8>>>;

/// The log lines written on this test's thread from now on.
// One global subscriber: a per-test `set_default` raced the other tests' threads over tracing's callsite
// interest cache, which now and then cached the warning as disabled and the capture read 0.
fn capture_logs() -> Sink {
    static SINKS: std::sync::OnceLock<std::sync::Mutex<HashMap<ThreadId, Sink>>> = std::sync::OnceLock::new();
    let sinks = SINKS.get_or_init(Default::default);
    let sink = Sink::default();
    sinks
        .lock()
        .unwrap()
        .insert(std::thread::current().id(), Arc::clone(&sink));
    static INSTALL: std::sync::Once = std::sync::Once::new();
    INSTALL.call_once(|| {
        let subscriber = tracing_subscriber::fmt()
            .with_writer(|| {
                Capture(
                    SINKS
                        .get()
                        .and_then(|s| s.lock().unwrap().get(&std::thread::current().id()).cloned()),
                )
            })
            .with_ansi(false)
            .finish();
        tracing::subscriber::set_global_default(subscriber).unwrap();
    });
    // A callsite another thread was registering while the subscriber went in may have cached the old answer
    tracing::callsite::rebuild_interest_cache();
    sink
}

/// Writes into the thread's sink, or nowhere for a thread no test is capturing.
struct Capture(Option<Sink>);

impl std::io::Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if let Some(sink) = &self.0 {
            sink.lock().unwrap().extend_from_slice(bytes);
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn a_game_server_may_not_write_for_a_kicked_player() {
    let mut handlers = Handlers::new();
    ok_handler(&mut handlers, Method::Post, "/progression/:userId");
    ok_handler(&mut handlers, Method::Get, "/progression/:userId");
    let t = Harness::new(Options {
        handlers,
        ..Options::default()
    });
    t.shared.auth().kick("kicked-user", 60_000);
    let key: &[(&str, &str)] = &[("x-rustic-isles-gameserver-apikey", "k")];
    let peer = "127.0.0.1:5000";
    let refused = t.send("POST", "/progression/kicked-user", key, peer).await;
    assert_eq!(refused.status, StatusCode::FORBIDDEN);
    // Reads, other players and calls without the server's key go on to the route
    assert_eq!(
        t.send("GET", "/progression/kicked-user", key, peer).await.status,
        StatusCode::OK
    );
    assert_eq!(
        t.send("POST", "/progression/someone-else", key, peer).await.status,
        StatusCode::OK
    );
    assert_eq!(
        t.send("POST", "/progression/kicked-user", &[], peer).await.status,
        StatusCode::OK
    );
}

#[test]
fn the_players_a_body_names() {
    use serde_json::json;
    assert_eq!(rustic_isles::pipeline::body_user_ids(&json!({"userId": "a"})), ["a"]);
    assert_eq!(
        rustic_isles::pipeline::body_user_ids(&json!([{"UserId": "b"}, {"other": 1}])),
        ["b"]
    );
    assert!(rustic_isles::pipeline::body_user_ids(&json!({"userId": 3})).is_empty());
}
