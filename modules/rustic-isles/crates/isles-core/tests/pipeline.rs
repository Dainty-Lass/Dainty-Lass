//! The pipeline as `app.ts` orders it: match order, mount conditions, the `/admin` gate, the origin strip, the 501
//! envelope, and an area's handler taking a route over from the stub.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use std::net::SocketAddr;
use std::sync::Arc;

use axum::response::IntoResponse;
use isles_core::http::{Handlers, MountError, Shared, app};
use isles_core::routes::{Method, RouteSet, Service};
use isles_core::tables::{StarterManifest, VersionTables};
use isles_core::testing::request;
use isles_core::{IslesVersion, VersionHooks};

struct NoHooks;

impl VersionHooks for NoHooks {
    fn no_player_policy(&self) -> isles_core::hooks::NoPlayerPolicy {
        isles_core::hooks::NoPlayerPolicy::Off
    }
    fn game_session_payload(&self, _: &str) -> serde_json::Value {
        serde_json::Value::Null
    }
    fn network_encryption_keys(&self) -> bool {
        false
    }
    fn weapons_at_base_level(&self, _: &isles_core::Settings) -> bool {
        false
    }
    fn reforge_rules(&self) -> bool {
        false
    }
    fn mastery_persistence(&self) -> bool {
        false
    }
    fn huntpass_payload(&self, _: &str) -> serde_json::Value {
        serde_json::Value::Null
    }
    fn objective_row(&self, _: &str, _: &isles_core::hooks::ObjectiveRow) -> serde_json::Value {
        serde_json::Value::Null
    }
    fn progression_config(&self) -> isles_core::hooks::ProgressionConfigRule {
        isles_core::hooks::ProgressionConfigRule::AsIs
    }
    fn survey_config(&self) -> isles_core::hooks::StubReply {
        isles_core::hooks::StubReply {
            status: 400,
            body: None,
        }
    }
    fn entitlements_payload(&self, _: &[isles_core::hooks::Entitlement]) -> serde_json::Value {
        serde_json::Value::Null
    }
    fn candidate_destination(&self, hunt_id: &str, _: bool) -> isles_core::hooks::Destination {
        isles_core::hooks::Destination {
            game_mode: isles_core::hooks::GameMode::Island,
            hunt_id: Some(hunt_id.to_owned()),
        }
    }
}

const TABLES: VersionTables = VersionTables {
    target_changelist: 1,
    game_version: "0.0.1",
    active_huntpass_id: "",
    recent_huntpass_seasons: &[],
    patchnotes_release_version: "",
    bounty_item_grant_data: &[],
    store_sections: &[],
    seasonal_coin: None,
    masked_journey_nodes: &[],
    starter_manifest: StarterManifest {
        bootstrap_version: "",
        stacked: &[],
        quick_emojis: None,
    },
    switch_defaults: &[],
};

fn shared(routes: RouteSet, admin_api: bool) -> Arc<Shared> {
    let mut shared = Shared::bare(IslesVersion {
        hooks: Arc::new(NoHooks),
        tables: TABLES,
        routes,
    });
    shared.admin_api = admin_api;
    Arc::new(shared)
}

async fn serve(shared: Arc<Shared>, handlers: &Handlers) -> SocketAddr {
    let router = app(shared, Service::Backend, handlers).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await });
    address
}

fn message(body: &str) -> String {
    let json: serde_json::Value = serde_json::from_str(body).unwrap();
    json["error"]["message"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn every_route_is_a_501_in_the_launcher_envelope() {
    let address = serve(shared(RouteSet::V1147, false), &Handlers::default()).await;
    let (status, headers, body) = request(address, "GET", "/character").await.unwrap();
    assert_eq!(status, 501);
    assert!(
        headers
            .iter()
            .any(|(n, v)| n == "content-type" && v == "application/json; charset=utf-8")
    );
    let id = headers
        .iter()
        .find(|(n, _)| n == "x-mysticparadox-request-id")
        .map(|(_, v)| v.clone())
        .unwrap();
    assert_eq!(
        body,
        format!(
            r#"{{"error":{{"code":"NOT_IMPLEMENTED","message":"isles: GET /character is not implemented yet","requestId":"{id}"}}}}"#
        )
    );
    let (status, _, body) = request(address, "HEAD", "/CHARACTER/").await.unwrap();
    assert_eq!((status, body.as_str()), (501, ""));
    let (status, _, body) = request(address, "GET", "/nothing/here").await.unwrap();
    assert_eq!((status, body.as_str()), (404, ""));
}

#[tokio::test]
async fn routes_match_in_express_order() {
    let address = serve(shared(RouteSet::V1147, false), &Handlers::default()).await;
    // The Gauntlet regex comes first but answers only for its origin
    let (_, _, body) = request(address, "GET", "/progression/a/b").await.unwrap();
    assert_eq!(
        message(&body),
        "isles: GET /progression/:userId/:progressionId is not implemented yet"
    );
    let (_, _, body) = request(address, "GET", "/__origin/gauntlet-prod.steelyard.ca/progression/a/b")
        .await
        .unwrap();
    assert_eq!(
        message(&body),
        r"isles: GET /^\/progression\/([^/]*)\/([^/]+)$/i is not implemented yet"
    );
    // The literal route is registered before the :param ones that also match it
    let (_, _, body) = request(address, "GET", "/progression/objectives/u").await.unwrap();
    assert_eq!(
        message(&body),
        "isles: GET /progression/objectives/:userId is not implemented yet"
    );
    let (_, _, body) = request(address, "POST", "/inventory//dauntlessrel-1.14.7:647472")
        .await
        .unwrap();
    assert_eq!(
        message(&body),
        r"isles: POST /^\/inventory\/([^/]*)\/(.+)$/ is not implemented yet"
    );
    let (status, _, _) = request(address, "GET", "/__origin/evil.example/character")
        .await
        .unwrap();
    assert_eq!(status, 400);
}

#[tokio::test]
async fn the_112_set_leaves_114_only_routes_to_the_404() {
    let address = serve(shared(RouteSet::V1120, false), &Handlers::default()).await;
    assert_eq!(request(address, "PUT", "/gamesession/epiceos").await.unwrap().0, 404);
    assert_eq!(request(address, "PUT", "/gamesession/epic").await.unwrap().0, 501);
    assert_eq!(request(address, "GET", "/progression/a/b").await.unwrap().0, 404);
    // 1.12 keeps the named route the 1.14.7 regex shadows
    let (_, _, body) = request(address, "POST", "/inventory/c/cl").await.unwrap();
    assert_eq!(
        message(&body),
        "isles: POST /inventory/:characterId/:changeList is not implemented yet"
    );
    // Node-scope routes are served whichever version is loaded (RU3c, seam 7)
    assert_eq!(
        request(address, "GET", "/reforged/api/ServerStatus").await.unwrap().0,
        501
    );
    // Realms are 1.14.7's, and only with rustic-rifts loaded
    assert_eq!(
        request(address, "GET", "/launcher/v1/realms/mine").await.unwrap().0,
        404
    );
}

#[tokio::test]
async fn the_admin_gate_answers_404_with_the_admin_api_off() {
    let off = serve(shared(RouteSet::V1147, false), &Handlers::default()).await;
    assert_eq!(request(off, "GET", "/admin/v1/me").await.unwrap().0, 404);
    assert_eq!(request(off, "GET", "/Admin").await.unwrap().0, 404);
    assert_eq!(request(off, "GET", "/health/release-gates").await.unwrap().0, 404);
    let on = serve(shared(RouteSet::V1147, true), &Handlers::default()).await;
    assert_eq!(request(on, "GET", "/admin/v1/me").await.unwrap().0, 501);
    assert_eq!(request(on, "GET", "/health/release-gates").await.unwrap().0, 501);
}

#[tokio::test]
async fn an_areas_handler_replaces_the_stub() {
    let mut handlers = Handlers::default();
    handlers.on(Service::Backend, Method::Get, "/", |_call| {
        Box::pin(async { (axum::http::StatusCode::OK, "ok").into_response() })
    });
    let address = serve(shared(RouteSet::V1147, false), &handlers).await;
    let (status, _, body) = request(address, "GET", "/").await.unwrap();
    assert_eq!((status, body.as_str()), (200, "ok"));

    let mut typo = Handlers::default();
    typo.on(Service::Backend, Method::Get, "/charactr", |_call| {
        Box::pin(async { axum::http::StatusCode::OK.into_response() })
    });
    assert!(matches!(
        app(shared(RouteSet::V1147, false), Service::Backend, &typo),
        Err(MountError::UnknownRoute(_))
    ));
}

// ------------------------------------------------------------------ request budget (CreateRequestRateLimiter)

struct TestGuardHooks;

impl rifts_seam::Lifecycle for TestGuardHooks {
    fn runtime_safety_issues(&self, _: &dyn rifts_seam::Settings) -> Result<Vec<String>, rifts_seam::SeamError> {
        Ok(Vec::new())
    }
    fn startup_summary(&self, _: &str) -> Result<Option<rifts_seam::Json>, rifts_seam::SeamError> {
        Ok(None)
    }
    fn start(&self, _: &str) -> Result<(), rifts_seam::SeamError> {
        Ok(())
    }
    fn stop(&self) -> module_api::BoxFuture<'_, Result<(), rifts_seam::SeamError>> {
        Box::pin(std::future::ready(Ok(())))
    }
    fn tables(&self) -> &'static [rifts_seam::TableSpec] {
        &[]
    }
    fn account_deletion_steps(&self) -> Vec<rifts_seam::DeletionStep> {
        Vec::new()
    }
}

impl rifts_seam::GuardHooks for TestGuardHooks {
    fn service_roles(&self) -> &'static [&'static str] {
        &[]
    }
    fn guard_manifest_roles(&self) -> &'static [&'static str] {
        &[]
    }
    fn census(&self, _: &rifts_seam::HostRouteSample) -> Result<(), rifts_seam::SeamError> {
        Ok(())
    }
    fn rate_limit_bucket(
        &self,
        request: rifts_seam::RequestInfo,
    ) -> module_api::BoxFuture<'_, Result<Option<String>, rifts_seam::SeamError>> {
        let out = match request.header("x-mystic-host-capability") {
            Some("valid-cap") => Ok(Some("host:cap-123".to_owned())),
            Some("valid-cap-2") => Ok(Some("host:cap-456".to_owned())),
            Some("resolver-error") => Err(rifts_seam::SeamError::unimplemented(
                rifts_seam::Component::Guard,
                "resolver-error",
            )),
            _ => Ok(None),
        };
        Box::pin(std::future::ready(out))
    }
    fn authorize_inventory(
        &self,
        _: rifts_seam::RuntimeAuth,
        _: rifts_seam::InventoryTransaction,
    ) -> module_api::BoxFuture<'_, Result<rifts_seam::InventoryDecision, rifts_seam::SeamError>> {
        Box::pin(std::future::ready(Ok(rifts_seam::InventoryDecision::Refused(
            rifts_seam::Reply {
                status: 403,
                body: rifts_seam::Json::empty_object(),
            },
        ))))
    }
    fn authorize_host_write(
        &self,
        _: rifts_seam::RuntimeAuth,
        _: rifts_seam::HostWriteKind,
        _: rifts_seam::AccountId,
        _: rifts_seam::HostWriteOptions,
    ) -> module_api::BoxFuture<'_, Result<Option<rifts_seam::Reply>, rifts_seam::SeamError>> {
        Box::pin(std::future::ready(Ok(None)))
    }
    fn record_host_write(
        &self,
        _: rifts_seam::RuntimeAuth,
        _: rifts_seam::HostWrite,
    ) -> module_api::BoxFuture<'_, Result<(), rifts_seam::SeamError>> {
        Box::pin(std::future::ready(Ok(())))
    }
    fn host_policy_refuses(&self) -> Result<bool, rifts_seam::SeamError> {
        Ok(false)
    }
    fn note_would_refuse(&self, _: &str, _: &str) -> Result<(), rifts_seam::SeamError> {
        Ok(())
    }
}

struct StubRiftsRoute(&'static [rifts_seam::Route]);

impl rifts_seam::RouteHandler for StubRiftsRoute {
    fn routes(&self) -> &'static [rifts_seam::Route] {
        self.0
    }
    fn handle(
        &self,
        _call: rifts_seam::RouteCall,
    ) -> module_api::BoxFuture<'_, Result<rifts_seam::Reply, rifts_seam::SeamError>> {
        Box::pin(std::future::ready(Ok(rifts_seam::Reply {
            status: 200,
            body: rifts_seam::Json(r#"{"huntOk":true}"#.to_owned()),
        })))
    }
}

fn rifts_with_test_guard(settings: &isles_core::Settings) -> isles_core::http::Rifts {
    isles_core::http::Rifts::load(settings, || {
        let mut route_defs = Vec::new();
        for def in isles_core::routes::all() {
            if def.when == isles_core::routes::When::Rifts {
                let method = match def.method {
                    Method::Get => rifts_seam::Method::Get,
                    Method::Post => rifts_seam::Method::Post,
                    Method::Patch => rifts_seam::Method::Patch,
                    _ => rifts_seam::Method::All,
                };
                route_defs.push(rifts_seam::Route {
                    method,
                    path: def.path,
                    mount: rifts_seam::Mount::Normal,
                    handler: "stub",
                });
            }
        }
        let leaked: &'static [rifts_seam::Route] = Box::leak(route_defs.into_boxed_slice());
        rifts_seam::RiftsParts {
            realms: Arc::new(rifts_seam::DedicatedOnly),
            entry: Arc::new(rifts_seam::DedicatedOnly),
            guard: Arc::new(TestGuardHooks),
            routes: vec![Arc::new(StubRiftsRoute(leaked))],
        }
    })
}

async fn serve_with_peer(shared: Arc<Shared>, handlers: &Handlers, peer_override: Option<SocketAddr>) -> SocketAddr {
    let mut router = app(shared, Service::Backend, handlers).unwrap();
    if let Some(peer) = peer_override {
        router = router.layer(axum::Extension(axum::extract::ConnectInfo(
            service_kit::serve::ConnectionInfo { peer, local: peer },
        )));
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        if peer_override.is_some() {
            let _ = axum::serve(listener, router).await;
        } else {
            let _ = service_kit::serve::serve(
                listener,
                router,
                std::future::pending(),
                std::time::Duration::from_secs(1),
            )
            .await;
        }
    });
    address
}

async fn request_with_headers(
    address: SocketAddr,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
) -> (u16, Vec<(String, String)>, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n");
    for (name, value) in headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("Content-Length: 0\r\n\r\n");
    stream.write_all(head.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((text.as_str(), ""));
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    let parsed_headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    (status, parsed_headers, body.to_owned())
}

fn find_header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
}

struct LocalHome;

impl module_api::HomeApi for LocalHome {
    fn resolve_home<'a>(
        &'a self,
        _: &'a module_api::GlobalId,
    ) -> module_api::BoxFuture<'a, Result<module_api::NodeIdentity, module_api::network::HomeError>> {
        Box::pin(std::future::ready(Ok(module_api::NodeIdentity::standalone())))
    }

    fn session_ticket<'a>(
        &'a self,
        _: &'a module_api::GlobalId,
        _: &'a module_api::NodeId,
    ) -> module_api::BoxFuture<'a, Result<module_api::network::SessionTicket, module_api::network::HomeError>> {
        Box::pin(std::future::ready(Err(module_api::network::HomeError::Unreachable(
            "none".into(),
        ))))
    }
}

fn test_handlers() -> Handlers {
    let mut handlers = Handlers::default();
    for path in [
        "/",
        "/health/live",
        "/health/ready",
        "/health/release-gates",
        "/character",
        "/mystic/v1/hunts/:sessionId/status",
    ] {
        handlers.on(Service::Backend, Method::Get, path, |_call| {
            Box::pin(async {
                (
                    axum::http::StatusCode::OK,
                    service_kit::json::Json(serde_json::json!({"ok": true})),
                )
                    .into_response()
            })
        });
    }
    handlers.on(Service::Backend, Method::Get, "/accountinfo", |mut call| {
        Box::pin(async move {
            match isles_core::areas::auth::middleware::game_auth(&mut call).await {
                Ok(_) => (
                    axum::http::StatusCode::OK,
                    service_kit::json::Json(serde_json::json!({"authed": true})),
                )
                    .into_response(),
                Err(refusal) => refusal,
            }
        })
    });
    handlers
}

fn shared_with_settings(routes: RouteSet, pairs: &[(&str, &str)], admin_api: bool) -> Arc<Shared> {
    let mut all_pairs = vec![
        ("JWT_SECRET", "0123456789abcdef0123456789abcdef"),
        ("API_KEY_HASH_SECRET", "0123456789abcdef0123456789abcdef"),
    ];
    all_pairs.extend_from_slice(pairs);
    let settings = isles_core::Settings::from_pairs(all_pairs);
    let mut s = Shared::bare(IslesVersion {
        hooks: Arc::new(NoHooks),
        tables: TABLES,
        routes,
    });
    s.rifts = rifts_with_test_guard(&settings);
    s.settings = settings;
    s.admin_api = admin_api;
    Arc::new(s)
}

#[tokio::test]
async fn request_budget_matches_express_rate_limit_golden_byte_for_byte() {
    let golden_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/request_budget.json");
    let golden: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(golden_path).unwrap()).unwrap();
    let shared = shared_with_settings(
        RouteSet::V1147,
        &[
            ("MYSTICPARADOX_RATE_LIMIT_PER_MINUTE", "2"),
            ("MYSTICPARADOX_GAMESERVER_RATE_LIMIT_PER_MINUTE", "4"),
            ("MYSTICPARADOX_HOST_RATE_LIMIT_PER_MINUTE", "5"),
        ],
        false,
    );
    let address = serve_with_peer(shared, &test_handlers(), None).await;

    for case in golden["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let now_str = case["nowMs"].as_i64().unwrap().to_string();
        let mut req_headers = vec![("x-isles-replay-now-ms", now_str.as_str())];
        let extra: Vec<(String, String)> = case["headers"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned()))
            .collect();
        for (k, v) in &extra {
            req_headers.push((k.as_str(), v.as_str()));
        }
        let (status, res_headers, body) = request_with_headers(address, "GET", "/character", &req_headers).await;
        let expected = &case["response"];
        assert_eq!(status, expected["status"].as_u64().unwrap() as u16, "case {name}");
        assert_eq!(body, expected["body"].as_str().unwrap(), "case {name}");
        for hdr in ["content-type", "ratelimit", "ratelimit-policy", "retry-after"] {
            let want = expected["headers"][hdr].as_str();
            let got = find_header(&res_headers, hdr);
            assert_eq!(got, want, "case {name} header {hdr}");
        }
    }
}

#[tokio::test]
async fn buckets_are_independent_and_gameserver_and_host_headers_select_their_buckets() {
    let db_path = std::env::temp_dir().join(format!("isles-pipeline-budget-{}.sqlite", uuid::Uuid::new_v4()));
    let store = isles_core::db::IslesStore::open(
        &store::DatabaseConfig::Sqlite { file: db_path.clone() },
        &store::DatabaseOptions::default(),
        Arc::new(LocalHome),
        &module_api::NodeIdentity::standalone(),
    )
    .unwrap();
    let mut shared = shared_with_settings(
        RouteSet::V1147,
        &[
            ("MYSTICPARADOX_RATE_LIMIT_PER_MINUTE", "2"),
            ("MYSTICPARADOX_GAMESERVER_RATE_LIMIT_PER_MINUTE", "3"),
            ("MYSTICPARADOX_HOST_RATE_LIMIT_PER_MINUTE", "2"),
        ],
        false,
    );
    Arc::get_mut(&mut shared).unwrap().store = Some(store);
    let address = serve_with_peer(shared, &test_handlers(), None).await;
    let now = ("x-isles-replay-now-ms", "1700000000000");

    // Exhaust client:198.51.100.10 (limit 2)
    let client_hdrs = [now, ("x-forwarded-for", "198.51.100.10")];
    assert_eq!(
        request_with_headers(address, "GET", "/character", &client_hdrs).await.0,
        200
    );
    assert_eq!(
        request_with_headers(address, "GET", "/character", &client_hdrs).await.0,
        200
    );
    assert_eq!(
        request_with_headers(address, "GET", "/character", &client_hdrs).await.0,
        429
    );

    // Gameserver header on the same IP uses gameserver:198.51.100.10 (limit 3), independent of client bucket
    let gs_hdrs = [
        now,
        ("x-forwarded-for", "198.51.100.10"),
        ("x-mysticparadox-gameserver-apikey", "unverified-key"),
    ];
    // On an unauthenticated route, 3 hits succeed then the 4th is 429
    assert_eq!(
        request_with_headers(address, "GET", "/character", &gs_hdrs).await.0,
        200
    );
    // On an authenticated game route, auth still checks and rejects the invalid key with 401 while counting the hit
    let (auth_status, auth_res_hdrs, _) = request_with_headers(address, "GET", "/accountinfo", &gs_hdrs).await;
    assert_eq!(auth_status, 401);
    assert_eq!(
        find_header(&auth_res_hdrs, "ratelimit"),
        Some("\"3-in-1min\"; r=1; t=60")
    );
    assert_eq!(
        request_with_headers(address, "GET", "/character", &gs_hdrs).await.0,
        200
    );
    assert_eq!(
        request_with_headers(address, "GET", "/character", &gs_hdrs).await.0,
        429
    );

    // Valid host capability gets host:cap-123 (limit 2), independent of client and gameserver buckets
    let host1_hdrs = [
        now,
        ("x-forwarded-for", "198.51.100.10"),
        ("x-mystic-host-capability", "valid-cap"),
    ];
    assert_eq!(
        request_with_headers(address, "GET", "/character", &host1_hdrs).await.0,
        200
    );
    assert_eq!(
        request_with_headers(address, "GET", "/character", &host1_hdrs).await.0,
        200
    );
    assert_eq!(
        request_with_headers(address, "GET", "/character", &host1_hdrs).await.0,
        429
    );

    // A second valid host capability gets its own host:cap-456 bucket
    let host2_hdrs = [
        now,
        ("x-forwarded-for", "198.51.100.10"),
        ("x-mystic-host-capability", "valid-cap-2"),
    ];
    assert_eq!(
        request_with_headers(address, "GET", "/character", &host2_hdrs).await.0,
        200
    );

    // Invalid capability or resolver error falls back to client:198.51.100.10 (already exhausted -> 429)
    for bad_cap in ["invalid-cap", "resolver-error", "   "] {
        let bad_hdrs = [
            now,
            ("x-forwarded-for", "198.51.100.10"),
            ("x-mystic-host-capability", bad_cap),
        ];
        let (status, res_hdrs, _) = request_with_headers(address, "GET", "/character", &bad_hdrs).await;
        assert_eq!(
            status, 429,
            "bad capability {bad_cap:?} must fall back to client bucket"
        );
        assert_eq!(find_header(&res_hdrs, "ratelimit"), Some("\"2-in-1min\"; r=0; t=60"));
    }
    let _ = std::fs::remove_file(&db_path);
}

#[tokio::test]
async fn switch_disables_budget_and_invalid_limits_fall_back_to_defaults() {
    for off_value in ["0", "off", "OFF", " false ", "No"] {
        let shared = shared_with_settings(
            RouteSet::V1147,
            &[
                ("MYSTICPARADOX_RATE_LIMIT", off_value),
                ("MYSTICPARADOX_RATE_LIMIT_PER_MINUTE", "1"),
            ],
            false,
        );
        let address = serve_with_peer(shared, &test_handlers(), None).await;
        for _ in 0..4 {
            let (status, res_hdrs, _) = request_with_headers(address, "GET", "/character", &[]).await;
            assert_eq!(status, 200);
            assert!(find_header(&res_hdrs, "ratelimit").is_none());
            assert!(find_header(&res_hdrs, "ratelimit-policy").is_none());
        }
    }

    // Invalid limits ("0", "-10", "abc", "") fall back to defaults (600, 6000, 6000)
    let shared = shared_with_settings(
        RouteSet::V1147,
        &[
            ("MYSTICPARADOX_RATE_LIMIT", "on"),
            ("MYSTICPARADOX_RATE_LIMIT_PER_MINUTE", "0"),
            ("MYSTICPARADOX_GAMESERVER_RATE_LIMIT_PER_MINUTE", "-10"),
            ("MYSTICPARADOX_HOST_RATE_LIMIT_PER_MINUTE", "not-a-number"),
        ],
        false,
    );
    let address = serve_with_peer(shared, &test_handlers(), None).await;
    let now = ("x-isles-replay-now-ms", "1700000000000");

    let (_, client_hdrs, _) = request_with_headers(address, "GET", "/character", &[now]).await;
    assert_eq!(
        find_header(&client_hdrs, "ratelimit"),
        Some("\"600-in-1min\"; r=599; t=60")
    );

    let (_, gs_hdrs, _) = request_with_headers(
        address,
        "GET",
        "/character",
        &[now, ("x-mysticparadox-gameserver-apikey", "k")],
    )
    .await;
    assert_eq!(
        find_header(&gs_hdrs, "ratelimit"),
        Some("\"6000-in-1min\"; r=5999; t=60")
    );

    let (_, host_hdrs, _) = request_with_headers(
        address,
        "GET",
        "/character",
        &[now, ("x-mystic-host-capability", "valid-cap")],
    )
    .await;
    assert_eq!(
        find_header(&host_hdrs, "ratelimit"),
        Some("\"6000-in-1min\"; r=5999; t=60")
    );
}

#[tokio::test]
async fn health_probes_are_not_counted_and_hunts_mount_counts_only_once() {
    let shared = shared_with_settings(RouteSet::V1147, &[("MYSTICPARADOX_RATE_LIMIT_PER_MINUTE", "2")], true);
    let address = serve_with_peer(shared, &test_handlers(), None).await;
    let now = [("x-isles-replay-now-ms", "1700000000000")];

    // Probes above line 245 in app.ts are not counted and emit no RateLimit headers
    for probe in ["/", "/health/live", "/health/ready", "/health/release-gates"] {
        for _ in 0..5 {
            let (status, hdrs, _) = request_with_headers(address, "GET", probe, &now).await;
            assert_eq!(status, 200, "probe {probe}");
            assert!(
                find_header(&hdrs, "ratelimit").is_none(),
                "probe {probe} must not emit RateLimit"
            );
        }
    }

    // Request through `/mystic/v1/hunts/...` passes both the early mount (line 196) and the main mount (line 245)
    // and must be counted exactly once (r goes from 1 -> 0 across two requests, not 1 -> 429 on the first).
    let (status1, hdrs1, _) = request_with_headers(address, "GET", "/mystic/v1/hunts/s1/status", &now).await;
    assert_eq!(status1, 200);
    assert_eq!(find_header(&hdrs1, "ratelimit"), Some("\"2-in-1min\"; r=1; t=60"));
    // An unmounted path under `/mystic/v1/hunts` also passes both mounts and counts once before answering 404
    let (status2, hdrs2, _) = request_with_headers(address, "GET", "/mystic/v1/hunts/unknown-tail", &now).await;
    assert_eq!(status2, 404);
    assert_eq!(find_header(&hdrs2, "ratelimit"), Some("\"2-in-1min\"; r=0; t=60"));
    // Third counted request is refused with 429
    let (status3, _, _) = request_with_headers(address, "GET", "/mystic/v1/hunts/s1/status", &now).await;
    assert_eq!(status3, 429);

    // Even while the bucket is exhausted, health probes continue to answer 200
    for probe in ["/", "/health/live", "/health/ready", "/health/release-gates"] {
        let (status, hdrs, _) = request_with_headers(address, "GET", probe, &now).await;
        assert_eq!(status, 200, "probe {probe} while bucket is exhausted");
        assert!(find_header(&hdrs, "ratelimit").is_none());
    }
}

#[tokio::test]
async fn x_forwarded_for_honoured_only_from_loopback_and_ipv6_grouped_by_56_subnet() {
    let handlers = test_handlers();
    let now = ("x-isles-replay-now-ms", "1700000000000");

    // 1. Loopback peer (trust proxy: loopback): X-Forwarded-For separates clients and groups IPv6 by /56
    let shared_loopback = shared_with_settings(RouteSet::V1147, &[("MYSTICPARADOX_RATE_LIMIT_PER_MINUTE", "1")], false);
    let loopback_addr = serve_with_peer(shared_loopback, &handlers, None).await;
    // Two different IPv6 addresses in the same /56 subnet (2001:db8:abcd:ef00::/56) share one bucket
    let v6_a = [now, ("x-forwarded-for", "2001:db8:abcd:ef12:1:2:3:4")];
    let v6_b = [now, ("x-forwarded-for", "2001:db8:abcd:ef99:9:8:7:6")];
    let v6_other = [now, ("x-forwarded-for", "2001:db8:abcd:ee00::1")];
    assert_eq!(
        request_with_headers(loopback_addr, "GET", "/character", &v6_a).await.0,
        200
    );
    assert_eq!(
        request_with_headers(loopback_addr, "GET", "/character", &v6_b).await.0,
        429
    );
    assert_eq!(
        request_with_headers(loopback_addr, "GET", "/character", &v6_other)
            .await
            .0,
        200
    );

    // 2. Non-loopback peer (198.51.100.77): X-Forwarded-For is ignored; spoofed headers share the peer's own bucket
    let shared_direct = shared_with_settings(RouteSet::V1147, &[("MYSTICPARADOX_RATE_LIMIT_PER_MINUTE", "1")], false);
    let direct_peer: SocketAddr = "198.51.100.77:54321".parse().unwrap();
    let direct_addr = serve_with_peer(shared_direct, &handlers, Some(direct_peer)).await;
    let spoof1 = [now, ("x-forwarded-for", "203.0.113.1")];
    let spoof2 = [now, ("x-forwarded-for", "203.0.113.2")];
    assert_eq!(
        request_with_headers(direct_addr, "GET", "/character", &spoof1).await.0,
        200
    );
    assert_eq!(
        request_with_headers(direct_addr, "GET", "/character", &spoof2).await.0,
        429
    );
}

#[tokio::test]
async fn version_1_12_0_is_not_rate_limited() {
    let shared = shared_with_settings(
        RouteSet::V1120,
        &[
            ("MYSTICPARADOX_RATE_LIMIT", "on"),
            ("MYSTICPARADOX_RATE_LIMIT_PER_MINUTE", "1"),
        ],
        false,
    );
    let address = serve_with_peer(shared, &test_handlers(), None).await;
    let now = [("x-isles-replay-now-ms", "1700000000000")];
    for _ in 0..5 {
        let (status, hdrs, _) = request_with_headers(address, "GET", "/character", &now).await;
        assert_eq!(status, 200);
        assert!(find_header(&hdrs, "ratelimit").is_none());
        assert!(find_header(&hdrs, "ratelimit-policy").is_none());
    }
}
