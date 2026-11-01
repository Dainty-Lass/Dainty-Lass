//! The chat group's routes against what Node's own routers answered (`tests/golden/chat/chat.json`, recorded by
//! `run.sh` from `routes/friends.ts` and `routes/guild.ts` on Node 22 with a fixed clock): status, content type and
//! body text, byte for byte, in the order Node ran them. `/vivox/login` is `systemRouter`'s one-liner and is checked
//! from the source (`res.status(404); res.send()`), since `system.ts` pulls in half the backend.
//!
//! The auth middleware is the auth area's; a layer stands in for it, putting `AuthData` where it will.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use axum::extract::Request;
use axum::middleware::{Next, from_fn};
use axum::response::Response;
use isles_core::areas::chat::{AuthData, register_with};
use isles_core::db::IslesStore;
use isles_core::http::{Handlers, Shared, app};
use isles_core::repo::auth::AccountRecord;
use isles_core::routes::{RouteSet, Service};
use isles_core::tables::{StarterManifest, VersionTables};
use isles_core::{AccountId, IslesVersion, UserId, VersionHooks};
use module_api::network::StandaloneHome;
use module_api::{Actor, ActorContext, NodeIdentity};
use serde_json::Value;
use store::{DatabaseConfig, DatabaseOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

struct NoHooks;

impl VersionHooks for NoHooks {
    fn no_player_policy(&self) -> isles_core::hooks::NoPlayerPolicy {
        isles_core::hooks::NoPlayerPolicy::Off
    }
    fn game_session_payload(&self, _: &str) -> Value {
        Value::Null
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
    fn huntpass_payload(&self, _: &str) -> Value {
        Value::Null
    }
    fn objective_row(&self, _: &str, _: &isles_core::hooks::ObjectiveRow) -> Value {
        Value::Null
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
    fn entitlements_payload(&self, _: &[isles_core::hooks::Entitlement]) -> Value {
        Value::Null
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

/// `x-test-user` is the stand-in for the bearer token: absent is unauthenticated, empty is a game server call that
/// names no player.
async fn fake_auth(mut request: Request, next: Next) -> Response {
    if let Some(user) = request.headers().get("x-test-user") {
        let user = user.to_str().unwrap().to_owned();
        request.extensions_mut().insert(AuthData {
            user_id: (!user.is_empty()).then_some(user),
            is_gameserver: false,
        });
    }
    next.run(request).await
}

async fn account(store: &IslesStore, user: &str) {
    let node = NodeIdentity::standalone();
    let record = AccountRecord {
        id: AccountId::new(&node.node),
        user_id: UserId(user.into()),
        name: user.into(),
        notes: 0,
    };
    let actor = ActorContext::new(Actor::Admin { key: "owner".into() });
    let id = record.id.clone();
    store
        .account_write(&id, &actor, move |tx, repos| repos.accounts.create(tx, &record))
        .await
        .unwrap();
}

async fn serve(routes: RouteSet, dir: &tempfile::TempDir) -> SocketAddr {
    let node = NodeIdentity::standalone();
    let home = Arc::new(StandaloneHome { identity: node.clone() });
    let config = DatabaseConfig::Sqlite {
        file: dir.path().join("isles.db"),
    };
    let store = IslesStore::open(&config, &DatabaseOptions::default(), home, &node).unwrap();
    for user in ["alice", "bob", "carol"] {
        account(&store, user).await;
    }
    let mut shared = Shared::bare(IslesVersion {
        hooks: Arc::new(NoHooks),
        tables: TABLES,
        routes,
    });
    shared.store = Some(store);
    // Node's recording ran `new Date()` as 2026-10-20T12:00:<n>Z, n counting up from 0
    let ticks = Arc::new(AtomicU32::new(0));
    let mut handlers = Handlers::default();
    register_with(
        &mut handlers,
        Arc::new(move || format!("2026-10-20T12:00:{:02}.000Z", ticks.fetch_add(1, Ordering::SeqCst))),
    );
    let router = app(Arc::new(shared), Service::Backend, &handlers)
        .unwrap()
        .layer(from_fn(fake_auth));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await });
    address
}

struct Reply {
    status: u16,
    content_type: Option<String>,
    body: String,
}

async fn send(address: SocketAddr, method: &str, path: &str, user: Option<&str>, body: Option<(&str, &str)>) -> Reply {
    let mut stream = TcpStream::connect(address).await.unwrap();
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n");
    if let Some(user) = user {
        head += &format!("x-test-user: {user}\r\n");
    }
    let payload = body.map_or("", |(_, text)| text);
    if let Some((content_type, _)) = body {
        head += &format!("content-type: {content_type}\r\n");
    }
    head += &format!("content-length: {}\r\n\r\n", payload.len());
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(payload.as_bytes()).await.unwrap();
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let text = String::from_utf8(raw).unwrap();
    let (head, rest) = text.split_once("\r\n\r\n").unwrap();
    let mut lines = head.lines();
    let status = lines.next().unwrap().split(' ').nth(1).unwrap().parse().unwrap();
    let content_type = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.trim().to_owned());
    let chunked = head.to_ascii_lowercase().contains("transfer-encoding: chunked");
    let body = if chunked { dechunk(rest) } else { rest.to_owned() };
    Reply {
        status,
        content_type,
        body,
    }
}

fn dechunk(mut rest: &str) -> String {
    let mut out = String::new();
    while let Some((size, tail)) = rest.split_once("\r\n") {
        let size = usize::from_str_radix(size.trim(), 16).unwrap();
        if size == 0 {
            break;
        }
        out += &tail[..size];
        rest = &tail[size + 2..];
    }
    out
}

fn steps() -> Vec<Value> {
    serde_json::from_str(include_str!("golden/chat/chat.json")).unwrap()
}

/// Replays every recorded step; `slayer` says whether the version serves Slayer link (1.14.7 only).
async fn replay(routes: RouteSet, slayer: bool) {
    let dir = tempfile::tempdir().unwrap();
    let address = serve(routes, &dir).await;
    let steps = steps();
    assert!(steps.len() > 40);
    for step in steps {
        let name = step["name"].as_str().unwrap();
        let (method, path) = (step["method"].as_str().unwrap(), step["path"].as_str().unwrap());
        let user = step["user"].as_str();
        let body: Option<(String, String)> = match &step["body"] {
            Value::Null => None,
            Value::Object(raw) if raw.contains_key("raw") => Some((
                raw["contentType"].as_str().unwrap().to_owned(),
                raw["raw"].as_str().unwrap().to_owned(),
            )),
            other => Some(("application/json".to_owned(), serde_json::to_string(other).unwrap())),
        };
        let got = send(
            address,
            method,
            path,
            user,
            body.as_ref().map(|(c, b)| (c.as_str(), b.as_str())),
        )
        .await;
        if !slayer && path.starts_with("/slayerlink/") {
            // 1.14.7-only routes fall to the shared unstubbed 404
            assert_eq!((got.status, got.body.as_str()), (404, ""), "{name}");
            continue;
        }
        let want = &step["expect"];
        assert_eq!(
            i64::from(got.status),
            want["status"].as_i64().unwrap(),
            "{name}: status"
        );
        assert_eq!(
            got.content_type.as_deref(),
            want["contentType"].as_str(),
            "{name}: content type"
        );
        assert_eq!(got.body, want["body"].as_str().unwrap(), "{name}: body");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn the_114_routes_answer_as_nodes_do() {
    replay(RouteSet::V1147, true).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn vivox_is_a_404_for_a_signed_in_caller() {
    // systemRouter: HasParadoxBackendAuth, then res.status(404); res.send()
    let dir = tempfile::tempdir().unwrap();
    let address = serve(RouteSet::V1147, &dir).await;
    let signed_in = send(address, "GET", "/vivox/login", Some("alice"), None).await;
    assert_eq!((signed_in.status, signed_in.body.as_str()), (404, ""));
    let anonymous = send(address, "GET", "/vivox/login", None, None).await;
    assert_eq!((anonymous.status, anonymous.body.as_str()), (401, ""));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_friend_with_no_account_takes_no_edge() {
    let dir = tempfile::tempdir().unwrap();
    let address = serve(RouteSet::V1147, &dir).await;
    let invite = send(
        address,
        "POST",
        "/friends/api/public/friends/alice/ghost",
        Some("alice"),
        None,
    )
    .await;
    assert_eq!(invite.status, 204);
    let list = send(
        address,
        "GET",
        "/friends/api/public/friends/alice?includePending=true",
        Some("alice"),
        None,
    )
    .await;
    assert_eq!(list.body, "[]");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bad_json_body_is_refused_before_auth_is_asked() {
    let dir = tempfile::tempdir().unwrap();
    let address = serve(RouteSet::V1147, &dir).await;
    let broken = send(
        address,
        "POST",
        "/slayerlink/availability",
        None,
        Some(("application/json", "{nope")),
    )
    .await;
    assert_eq!(broken.status, 400);
    let primitive = send(
        address,
        "POST",
        "/slayerlink/availability",
        Some("alice"),
        Some(("application/json", "7")),
    )
    .await;
    assert_eq!(primitive.status, 400, "strict mode takes only objects and arrays");
    let big = "x".repeat(101 * 1024);
    let huge = send(
        address,
        "POST",
        "/slayerlink/availability",
        Some("alice"),
        Some(("application/json", &big)),
    )
    .await;
    assert_eq!(huge.status, 413);
}
