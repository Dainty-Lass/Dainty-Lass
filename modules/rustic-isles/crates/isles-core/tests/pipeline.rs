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
