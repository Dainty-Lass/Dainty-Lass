//! The admin API end to end: the bot's HTTP requests against a fake backend and Director that answer
//! as the Node services do (`routes/reforgedApi.ts`, `controllers/admin.ts`, the Director's
//! `/api/instances`) and check the manager-operator signature and the admin key as they do.

#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use std::net::SocketAddr;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use admin::config::{Config, ServerMode};
use admin::modules::AdminPlugin;
use admin::schema::Admin;
use admin::upstream::{Operator, operator_signature};
use axum::Router;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use serde_json::{Value, json};
use tokio::net::TcpListener;

const API_KEY: &str = "api-key-0123456789abcdef0123456789";
const ADMIN_KEY: &str = "admin-key-0123456789abcdef01234567";
const SECRET: &str = "operator-secret-0123456789abcdef01";
const START: i64 = 1_800_000_000_000;

// ------------------------------------------------------------------ the fake Node services

#[derive(Default)]
struct Backend {
    accounts: Mutex<Vec<Value>>,
    online: Mutex<Vec<String>>,
    kicked: Mutex<Vec<String>>,
    secret: String,
}

fn reply(status: StatusCode, body: Value) -> Response {
    (status, [("content-type", "application/json")], body.to_string()).into_response()
}

fn account(user_id: &str, name: &str, roles: &[&str]) -> Value {
    json!({
        "userId": user_id, "displayName": name, "email": "", "approvalStatus": "approved", "status": "active",
        "roles": roles, "usernameSet": true, "createdAt": "2026-10-01T10:00:00.000Z", "lastLoginAt": "2026-10-24T09:00:00.000Z"
    })
}

async fn backend(State(state): State<Arc<Backend>>, request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let body = body.collect().await.map(|b| b.to_bytes()).unwrap_or_default();
    let target = parts
        .uri
        .path_and_query()
        .map(|p| p.as_str().to_owned())
        .unwrap_or_default();
    let header = |name: &str| parts.headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_owned);
    let path = parts.uri.path().to_owned();
    let query: Vec<(String, String)> = url::form_urlencoded::parse(parts.uri.query().unwrap_or("").as_bytes())
        .into_owned()
        .collect();
    let param = |name: &str| query.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());

    if path == "/reforged/api/ServerStatus" {
        return reply(
            StatusCode::OK,
            json!({
                "name": "Friends' server", "online": true, "version": "0.1.0", "commit": "8f2c97f", "sourceUrl": null,
                "registration": "INVITECODE", "uptimeSeconds": 3600, "contentPort": null,
                "game": {"build": "rel-1.14.7-Archon", "changelist": 647472, "executableSha256": null},
                "gameData": "real", "chat": true, "playersOnline": 0, "players": [], "instances": [], "limited": true
            }),
        );
    }
    if path == "/reforged/api/Kick" {
        // HasAdminKey: the key, from this machine, not through the gateway
        if header("x-reforged-admin-key").as_deref() != Some(ADMIN_KEY) || header("x-reforged-gateway").is_some() {
            return reply(StatusCode::FORBIDDEN, json!({"error": "forbidden"}));
        }
        let given: Value = serde_json::from_slice(&body).unwrap_or_default();
        let name = given["Username"].as_str().unwrap_or("").to_lowercase();
        let accounts = state.accounts.lock().unwrap();
        let Some(found) = accounts
            .iter()
            .find(|a| a["displayName"].as_str().unwrap().to_lowercase() == name)
        else {
            return reply(StatusCode::NOT_FOUND, json!({"error": "not_found", "message": "x"}));
        };
        state
            .kicked
            .lock()
            .unwrap()
            .push(found["displayName"].as_str().unwrap().to_owned());
        return reply(
            StatusCode::OK,
            json!({"UserId": found["userId"], "Username": found["displayName"], "Closed": 2}),
        );
    }
    if !path.starts_with("/admin/v1/") {
        return reply(StatusCode::NOT_FOUND, json!({}));
    }

    // HasAdminAuth with the manager's credential
    let (Some(id), Some(ts), Some(sig)) = (
        header("x-paradox-operator-id"),
        header("x-paradox-operator-timestamp"),
        header("x-paradox-operator-signature"),
    ) else {
        return reply(
            StatusCode::UNAUTHORIZED,
            json!({"error": {"code": "ADMIN_UNAUTHORIZED"}}),
        );
    };
    let operator = Operator::parse(&id).unwrap();
    if operator_signature(&state.secret, parts.method.as_str(), &target, &ts, &operator, &body) != sig {
        return reply(
            StatusCode::UNAUTHORIZED,
            json!({"error": {"code": "ADMIN_OPERATOR_SIGNATURE"}}),
        );
    }
    if !body.is_empty() && header("content-type").as_deref() != Some("application/json") {
        return reply(
            StatusCode::BAD_REQUEST,
            json!({"error": {"code": "ADMIN_OPERATOR_BODY"}}),
        );
    }

    let mut accounts = state.accounts.lock().unwrap();
    if path == "/admin/v1/online-players" {
        let online = state.online.lock().unwrap();
        let players: Vec<Value> = accounts
            .iter()
            .filter(|a| online.iter().any(|id| a["userId"] == *id))
            .map(|a| json!({"userId": a["userId"], "displayName": a["displayName"]}))
            .collect();
        return reply(
            StatusCode::OK,
            json!({"count": players.len(), "players": players, "observedAt": "x", "source": "xmpp+presence"}),
        );
    }
    if path == "/admin/v1/players" {
        if let Some(ids) = param("ids") {
            let found: Vec<Value> = accounts
                .iter()
                .filter(|a| ids.split(',').any(|id| a["userId"] == id))
                .cloned()
                .collect();
            return reply(
                StatusCode::OK,
                json!({"accounts": found, "total": found.len(), "page": 1, "limit": 100}),
            );
        }
        let search = param("search").unwrap_or_default().to_lowercase();
        let page: usize = param("page").and_then(|p| p.parse().ok()).unwrap_or(1);
        let limit: usize = param("limit").and_then(|p| p.parse().ok()).unwrap_or(25);
        let matching: Vec<Value> = accounts
            .iter()
            .filter(|a| {
                a["displayName"].as_str().unwrap().to_lowercase().contains(&search)
                    || a["userId"].as_str().unwrap().contains(&search)
            })
            .cloned()
            .collect();
        let shown: Vec<Value> = matching.iter().skip((page - 1) * limit).take(limit).cloned().collect();
        return reply(
            StatusCode::OK,
            json!({"accounts": shown, "total": matching.len(), "page": page, "limit": limit}),
        );
    }
    if let Some(user_id) = path
        .strip_prefix("/admin/v1/players/")
        .and_then(|p| p.strip_suffix("/access"))
    {
        let given: Value = serde_json::from_slice(&body).unwrap_or_default();
        let Some(found) = accounts.iter_mut().find(|a| a["userId"] == user_id) else {
            return reply(StatusCode::NOT_FOUND, json!({"error": {"code": "NOT_FOUND"}}));
        };
        found["status"] = given["status"].clone();
        found["approvalReason"] = given["reason"].clone();
        found["approvalUpdatedBy"] = json!(format!("manager:{id}"));
        found["approvalUpdatedAt"] = json!("2026-10-24T12:00:00.000Z");
        return reply(StatusCode::OK, json!({"account": found.clone(), "requestId": "r"}));
    }
    reply(StatusCode::NOT_FOUND, json!({}))
}

async fn director(State(instances): State<Arc<Mutex<Value>>>) -> Response {
    reply(StatusCode::OK, instances.lock().unwrap().clone())
}

async fn listen(app: Router) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(service_kit::serve::serve(
        listener,
        app,
        std::future::pending(),
        Duration::from_secs(1),
    ));
    address
}

struct World {
    url: String,
    backend: Arc<Backend>,
    instances: Arc<Mutex<Value>>,
    admin: Arc<Admin>,
    clock: Arc<AtomicI64>,
    timeouts_file: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

struct Options {
    director_up: bool,
    secret: &'static str,
    modules: Arc<AdminPlugin>,
}

async fn world_with(options: Options) -> World {
    let state = Arc::new(Backend {
        secret: SECRET.into(),
        ..Backend::default()
    });
    *state.accounts.lock().unwrap() = vec![
        account("u-ada", "Ada", &["player"]),
        account("u-bo", "Bo_Slayer", &["player"]),
        account("u-cy", "Cyra", &["player", "admin"]),
    ];
    *state.online.lock().unwrap() = vec!["u-ada".into(), "u-cy".into()];
    let backend_address = listen(Router::new().fallback(backend).with_state(state.clone())).await;
    let instances = Arc::new(Mutex::new(json!([
        {"id": "city-8777", "kind": "city", "map": "/Game/Maps/ramsgate/ramsgate_01_persistent", "behemoth": null,
         "players": 1, "maxPlayers": 0, "playerIds": ["u-cy"], "startedAt": "2026-10-24T08:00:00.000Z"},
        {"id": "hunt-8770", "kind": "hunt", "map": "/Game/Maps/islands/a/IslandA_persistent",
         "behemoth": "/Game/Behemoths/BP_Skarn.BP_Skarn_C", "players": 2, "maxPlayers": 4,
         "playerIds": ["u-ada", "u-gone"], "startedAt": "2026-10-24T09:30:00.000Z"}
    ])));
    let director_url = if options.director_up {
        let address = listen(
            Router::new()
                .route("/api/instances", axum::routing::get(director))
                .with_state(instances.clone()),
        )
        .await;
        format!("http://{address}")
    } else {
        // Nothing listens there
        let unused = TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap();
        format!("http://{unused}")
    };
    let dir = tempfile::tempdir().unwrap();
    let timeouts_file = dir.path().join("admin").join("timeouts.json");
    let config = Config {
        port: 0,
        bind_hosts: vec!["127.0.0.1".into()],
        backend_url: format!("http://{backend_address}"),
        director_url,
        api_key: API_KEY.into(),
        admin_key: ADMIN_KEY.into(),
        operator_secret: options.secret.into(),
        timeouts_file: timeouts_file.clone(),
        max_hunt_servers: 3,
        server_mode: ServerMode::Public,
        database: store::DatabaseConfig::Sqlite {
            file: timeouts_file.with_file_name("dainty-lass.db"),
        },
    };
    let clock = Arc::new(AtomicI64::new(START));
    let reading = clock.clone();
    let (app, admin) = admin::server::prepare_with_modules(
        &config,
        Arc::new(move || reading.load(Ordering::SeqCst)),
        options.modules,
    )
    .unwrap();
    let address = listen(admin::app::router(app)).await;
    World {
        url: format!("http://{address}"),
        backend: state,
        instances,
        admin,
        clock,
        timeouts_file,
        _dir: dir,
    }
}

async fn world() -> World {
    world_with(Options {
        director_up: true,
        secret: SECRET,
        modules: Arc::new(AdminPlugin::none()),
    })
    .await
}

// ------------------------------------------------------------------ the bot's side

async fn send(url: &str, method: &str, headers: &[(&str, &str)], body: &str) -> (u16, Value) {
    let client: Client<HttpConnector, Full<Bytes>> = Client::builder(TokioExecutor::new()).build(HttpConnector::new());
    let mut request = hyper::Request::builder().method(method).uri(url);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = client
        .request(request.body(Full::new(Bytes::from(body.to_owned()))).unwrap())
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

impl World {
    /// A GraphQL request as the bot sends it, for Discord user 42.
    async fn gql(&self, query: &str, variables: Value) -> Value {
        let bearer = format!("Bearer {API_KEY}");
        let (status, body) = send(
            &format!("{}/graphql", self.url),
            "POST",
            &[
                ("authorization", &bearer),
                ("content-type", "application/json"),
                ("x-reforged-operator-id", "42"),
            ],
            &json!({"query": query, "variables": variables}).to_string(),
        )
        .await;
        assert_eq!(status, 200, "{body}");
        body
    }

    fn status_of(&self, user_id: &str) -> String {
        let accounts = self.backend.accounts.lock().unwrap();
        let found = accounts.iter().find(|a| a["userId"] == user_id).unwrap();
        found["status"].as_str().unwrap().to_owned()
    }
}

fn code(body: &Value) -> &str {
    body["errors"][0]["extensions"]["code"].as_str().unwrap_or("")
}

const PLAYER: &str = "query($name: String!) { player(username: $name) { userId username status online admin
  restriction { kind reason by until } location { kind instanceId title } } }";

// ------------------------------------------------------------------ tests

#[tokio::test]
async fn refuses_requests_without_the_key_or_through_a_proxy() {
    let w = world().await;
    let url = format!("{}/graphql", w.url);
    let query = r#"{"query":"{ serverStatus { name } }"}"#;
    let bearer = format!("Bearer {API_KEY}");
    let json_type = ("content-type", "application/json");
    for (headers, status, code_text) in [
        (vec![json_type], 401, "UNAUTHENTICATED"),
        (
            vec![json_type, ("authorization", "Bearer wrong")],
            401,
            "UNAUTHENTICATED",
        ),
        (
            vec![
                json_type,
                ("authorization", bearer.as_str()),
                ("x-forwarded-for", "203.0.113.9"),
            ],
            403,
            "FORWARDED",
        ),
        (
            vec![
                json_type,
                ("authorization", bearer.as_str()),
                ("x-reforged-gateway", "s"),
            ],
            403,
            "FORWARDED",
        ),
        (
            vec![("authorization", bearer.as_str()), ("content-type", "text/plain")],
            415,
            "NOT_JSON",
        ),
        (
            vec![
                json_type,
                ("authorization", bearer.as_str()),
                ("x-reforged-operator-id", "me"),
            ],
            400,
            "BAD_OPERATOR",
        ),
    ] {
        let (got, body) = send(&url, "POST", &headers, query).await;
        assert_eq!((got, code(&body)), (status, code_text), "{headers:?}");
    }
    let (got, body) = send(&url, "POST", &[json_type, ("authorization", bearer.as_str())], "[1,2]").await;
    assert_eq!((got, code(&body)), (400, "BAD_REQUEST"));
    // No query by GET, and nothing else is served
    assert_eq!(
        send(&url, "GET", &[("authorization", bearer.as_str())], "").await.0,
        405
    );
    assert_eq!(send(&format!("{}/", w.url), "GET", &[], "").await.0, 404);
    let (got, body) = send(&format!("{}/health", w.url), "GET", &[], "").await;
    assert_eq!((got, body), (200, json!({"ok": true})));
}

#[tokio::test]
async fn looks_up_a_player_with_where_they_are() {
    let w = world().await;
    let body = w.gql(PLAYER, json!({"name": "ada"})).await;
    assert_eq!(
        body["data"]["player"],
        json!({"userId": "u-ada", "username": "Ada", "status": "ACTIVE", "online": true, "admin": false,
               "restriction": null, "location": {"kind": "HUNT", "instanceId": "hunt-8770", "title": "Skarn"}})
    );
    let body = w.gql(PLAYER, json!({"name": "Bo_Slayer"})).await;
    assert_eq!(body["data"]["player"]["online"], false);
    assert_eq!(body["data"]["player"]["location"], Value::Null);
    let body = w.gql(PLAYER, json!({"name": "Cyra"})).await;
    assert_eq!(body["data"]["player"]["location"]["kind"], "HUB");
    assert_eq!(body["data"]["player"]["admin"], true);
    // A part of a name is not a match
    assert_eq!(
        w.gql(PLAYER, json!({"name": "Bo_"})).await["data"]["player"],
        Value::Null
    );
    assert_eq!(code(&w.gql(PLAYER, json!({"name": "no spaces"})).await), "BAD_INPUT");
}

#[tokio::test]
async fn reports_the_server_and_its_hunts() {
    let w = world().await;
    let body = w
        .gql(
            "{ serverStatus { name mode registration version commit uptimeSeconds onlineCount chat gameData } }",
            json!({}),
        )
        .await;
    assert_eq!(
        body["data"]["serverStatus"],
        json!({"name": "Friends' server", "mode": "PUBLIC", "registration": "INVITECODE", "version": "0.1.0",
               "commit": "8f2c97f", "uptimeSeconds": 3600, "onlineCount": 2, "chat": true, "gameData": "real"})
    );
    let body = w
        .gql(
            "{ huntStatus { maxHuntServers running free hunts { id title map behemoth playerCount maxPlayers startedAt players { userId username } } } }",
            json!({}),
        )
        .await;
    assert_eq!(
        body["data"]["huntStatus"],
        json!({"maxHuntServers": 3, "running": 1, "free": 2, "hunts": [{
            "id": "hunt-8770", "title": "Skarn", "map": "IslandA_persistent", "behemoth": "BP_Skarn",
            "playerCount": 2, "maxPlayers": 4, "startedAt": "2026-10-24T09:30:00Z",
            "players": [{"userId": "u-ada", "username": "Ada"}, {"userId": "u-gone", "username": null}]
        }]})
    );
    *w.instances.lock().unwrap() = json!([]);
    let body = w.gql("{ huntStatus { running free hunts { id } } }", json!({})).await;
    assert_eq!(
        body["data"]["huntStatus"],
        json!({"running": 0, "free": 3, "hunts": []})
    );
}

#[tokio::test]
async fn kicks_through_the_backend_but_never_an_admin() {
    let w = world().await;
    let kick = "mutation($name: String!) { kick(username: $name) { userId username connectionsClosed } }";
    let body = w.gql(kick, json!({"name": "ADA"})).await;
    assert_eq!(
        body["data"]["kick"],
        json!({"userId": "u-ada", "username": "Ada", "connectionsClosed": 2})
    );
    assert_eq!(code(&w.gql(kick, json!({"name": "Cyra"})).await), "PROTECTED_ACCOUNT");
    assert_eq!(code(&w.gql(kick, json!({"name": "Nobody"})).await), "NOT_FOUND");
    assert_eq!(*w.backend.kicked.lock().unwrap(), ["Ada"]);
    assert_eq!(w.status_of("u-ada"), "active");
}

#[tokio::test]
async fn bans_and_unbans_in_the_operators_name() {
    let w = world().await;
    let ban = "mutation($name: String!, $reason: String!) { ban(username: $name, reason: $reason) { status restriction { kind reason by until } } }";
    let body = w
        .gql(ban, json!({"name": "Bo_Slayer", "reason": "griefing\nhunts"}))
        .await;
    assert_eq!(
        body["data"]["ban"],
        json!({"status": "BANNED", "restriction": {"kind": "BAN", "reason": "griefing hunts", "by": "manager:42", "until": null}})
    );
    assert_eq!(w.status_of("u-bo"), "banned");
    assert_eq!(
        code(&w.gql(ban, json!({"name": "Bo_Slayer", "reason": "x"})).await),
        "BAD_INPUT"
    );
    assert_eq!(
        code(&w.gql(ban, json!({"name": "Cyra", "reason": "testing"})).await),
        "PROTECTED_ACCOUNT"
    );

    let unban = "mutation($name: String!) { unban(username: $name) { status restriction { kind } } }";
    let body = w.gql(unban, json!({"name": "bo_slayer"})).await;
    assert_eq!(body["data"]["unban"], json!({"status": "ACTIVE", "restriction": null}));
    assert_eq!(code(&w.gql(unban, json!({"name": "bo_slayer"})).await), "NOT_BANNED");
}

const TIMEOUT: &str = "mutation($name: String!, $minutes: Int, $until: String, $reason: String!) {
  timeout(username: $name, minutes: $minutes, until: $until, reason: $reason) { status restriction { kind reason until } } }";

#[tokio::test]
async fn a_timeout_is_lifted_when_it_runs_out_even_after_a_restart() {
    let w = world().await;
    let body = w
        .gql(
            TIMEOUT,
            json!({"name": "Ada", "minutes": 30, "reason": "spamming chat"}),
        )
        .await;
    assert_eq!(
        body["data"]["timeout"],
        json!({"status": "BANNED", "restriction": {"kind": "TIMEOUT",
               "reason": "Timed out until 2027-01-15T08:30:00Z: spamming chat", "until": "2027-01-15T08:30:00Z"}})
    );
    assert!(std::fs::read_to_string(&w.timeouts_file).unwrap().contains("u-ada"));

    // A restart reads the file back
    let again = admin::timeouts::Timeouts::load(&w.timeouts_file).unwrap();
    assert_eq!(again.due(START + 30 * 60_000).await.len(), 1);

    w.admin.sweep().await;
    assert_eq!(w.status_of("u-ada"), "banned", "not due yet");
    w.clock.store(START + 30 * 60_000, Ordering::SeqCst);
    w.admin.sweep().await;
    assert_eq!(w.status_of("u-ada"), "active");
    assert!(!std::fs::read_to_string(&w.timeouts_file).unwrap().contains("u-ada"));
    let body = w.gql(PLAYER, json!({"name": "Ada"})).await;
    assert_eq!(body["data"]["player"]["restriction"], Value::Null);
}

#[tokio::test]
async fn a_timeout_never_lifts_a_ban_made_since_or_one_it_did_not_make() {
    let w = world().await;
    // Already banned for good: a timeout would shorten it
    let ban = "mutation { ban(username: \"Bo_Slayer\", reason: \"cheating\") { status } }";
    w.gql(ban, json!({})).await;
    assert_eq!(
        code(
            &w.gql(TIMEOUT, json!({"name": "Bo_Slayer", "minutes": 5, "reason": "spam"}))
                .await
        ),
        "ALREADY_BANNED"
    );

    // Timed out, then banned for good from the backend's own dashboard
    w.gql(
        TIMEOUT,
        json!({"name": "Ada", "until": "2027-01-15T09:00:00Z", "reason": "spam"}),
    )
    .await;
    for a in w.backend.accounts.lock().unwrap().iter_mut() {
        if a["userId"] == "u-ada" {
            a["approvalReason"] = json!("cheating, for good");
        }
    }
    assert_eq!(
        w.gql(PLAYER, json!({"name": "Ada"})).await["data"]["player"]["restriction"]["kind"],
        "BAN"
    );
    w.clock.store(START + 2 * 60 * 60_000, Ordering::SeqCst);
    w.admin.sweep().await;
    assert_eq!(w.status_of("u-ada"), "banned");
    assert!(w.admin.timeouts.due(i64::MAX).await.is_empty());

    // A bad end, and lifting what is not a timeout
    for (minutes, until) in [
        (json!(0), Value::Null),
        (Value::Null, json!("tomorrow")),
        (json!(5), json!("2027-01-16T00:00:00Z")),
    ] {
        let body = w
            .gql(
                TIMEOUT,
                json!({"name": "Cyra", "minutes": minutes, "until": until, "reason": "spam"}),
            )
            .await;
        assert_eq!(code(&body), "BAD_INPUT", "{minutes} {until}");
    }
    let lift = "mutation { liftTimeout(username: \"Bo_Slayer\") { status } }";
    assert_eq!(code(&w.gql(lift, json!({})).await), "NOT_TIMED_OUT");
}

#[tokio::test]
async fn lifting_a_timeout_early_or_banning_over_it() {
    let w = world().await;
    w.gql(TIMEOUT, json!({"name": "Ada", "minutes": 60, "reason": "spam"}))
        .await;
    let body = w
        .gql(
            "mutation { liftTimeout(username: \"Ada\") { status restriction { kind } } }",
            json!({}),
        )
        .await;
    assert_eq!(
        body["data"]["liftTimeout"],
        json!({"status": "ACTIVE", "restriction": null})
    );
    assert!(w.admin.timeouts.get("u-ada").await.is_none());

    w.gql(TIMEOUT, json!({"name": "Ada", "minutes": 60, "reason": "spam"}))
        .await;
    let body = w
        .gql(
            "mutation { ban(username: \"Ada\", reason: \"for good\") { restriction { kind until } } }",
            json!({}),
        )
        .await;
    assert_eq!(
        body["data"]["ban"]["restriction"],
        json!({"kind": "BAN", "until": null})
    );
    assert!(w.admin.timeouts.get("u-ada").await.is_none());
}

#[tokio::test]
async fn says_what_is_wrong_when_a_service_is_down_or_the_secret_differs() {
    let w = world_with(Options {
        director_up: false,
        secret: SECRET,
        modules: Arc::new(AdminPlugin::none()),
    })
    .await;
    // The lookup still answers, without a location
    let body = w.gql(PLAYER, json!({"name": "Ada"})).await;
    assert_eq!(body["data"]["player"]["online"], true);
    assert_eq!(body["data"]["player"]["location"], Value::Null);
    assert_eq!(
        code(&w.gql("{ huntStatus { running } }", json!({})).await),
        "BACKEND_UNAVAILABLE"
    );

    let w = world_with(Options {
        director_up: true,
        secret: "another-secret-0123456789abcdef0123",
        modules: Arc::new(AdminPlugin::none()),
    })
    .await;
    let body = w.gql(PLAYER, json!({"name": "Ada"})).await;
    assert_eq!(code(&body), "BACKEND_REFUSED");
    assert!(
        body["errors"][0]["message"]
            .as_str()
            .unwrap()
            .contains("reforged-host setup")
    );
}

#[tokio::test]
async fn refuses_queries_too_deep_or_too_big() {
    let w = world().await;
    let body = w
        .gql(
            "{ __schema { types { fields { type { ofType { ofType { name } } } } } } }",
            json!({}),
        )
        .await;
    assert!(!body["errors"].as_array().unwrap().is_empty());
}

#[test]
fn schema_graphql_is_current() {
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("schema.graphql");
    let current = admin::schema::sdl();
    if std::env::var("RR_UPDATE_GENERATED").as_deref() == Ok("1") {
        std::fs::write(&file, &current).unwrap();
    }
    let written = std::fs::read_to_string(&file).unwrap_or_default().replace("\r\n", "\n");
    assert_eq!(
        written, current,
        "schema.graphql is stale: RR_UPDATE_GENERATED=1 cargo test -p admin"
    );
}

// ------------------------------------------------------------------ the same cases on game modules (DF4c)

mod games {
    use std::collections::BTreeMap;
    use std::sync::atomic::AtomicBool;

    use admin::modules::AdminCalls;
    use module_api::module::PlayerInfo;
    use module_api::registrar::AdminAction;
    use module_api::stub::recording_host;
    use module_api::{
        API_VERSION, Actor, AdminCall, AdminRequest, AdminResponse, BoxFuture, Capabilities, CorePlugin, ErrorKind,
        GameModule, GlobalId, IdKind, InProcessModule, ModuleConfig, ModuleContext, ModuleDriver, ModuleDriverExt,
        ModuleError, ModuleId, ModuleKind, ModuleManifest, ModuleStatus, NodeIdentity, Phase, Registrar,
    };

    use super::*;

    struct StubPlayer {
        name: &'static str,
        account: GlobalId,
        online: bool,
        admin: bool,
    }

    fn stub_player(name: &'static str, online: bool, admin: bool) -> StubPlayer {
        StubPlayer {
            name,
            account: GlobalId::new(IdKind::Account, &NodeIdentity::standalone().node),
            online,
            admin,
        }
    }

    /// A game module that keeps a few players and every admin call it was sent.
    struct StubGame {
        id: &'static str,
        version: &'static str,
        kind: ModuleKind,
        actions: Vec<AdminAction>,
        players: Mutex<Vec<StubPlayer>>,
        calls: Mutex<Vec<AdminCall>>,
        down: AtomicBool,
    }

    impl StubGame {
        fn with(id: &'static str, version: &'static str, players: Vec<StubPlayer>) -> Self {
            Self {
                id,
                version,
                kind: ModuleKind::Game,
                actions: vec![AdminAction::Kick, AdminAction::Ban, AdminAction::Lookup],
                players: Mutex::new(players),
                calls: Mutex::new(Vec::new()),
                down: AtomicBool::new(false),
            }
        }

        fn account_of(&self, name: &str) -> String {
            let players = self.players.lock().unwrap();
            players.iter().find(|p| p.name == name).unwrap().account.to_string()
        }

        fn calls(&self) -> Vec<AdminCall> {
            self.calls.lock().unwrap().clone()
        }

        /// The calls other than lookups.
        fn actions(&self) -> Vec<AdminRequest> {
            self.calls()
                .into_iter()
                .map(|c| c.request)
                .filter(|r| r.action() != AdminAction::Lookup)
                .collect()
        }
    }

    impl GameModule for StubGame {
        fn manifest(&self) -> ModuleManifest {
            ModuleManifest {
                id: self.id.parse().unwrap(),
                kind: self.kind,
                display_name: format!("Stub {}", self.version),
                game_version: self.version.parse().unwrap(),
                invite_aliases: vec![],
                api: API_VERSION,
                ports: vec![],
                capabilities: Capabilities::default(),
                unverified: false,
                client: None,
            }
        }

        fn register(&self, registrar: &mut Registrar) -> Result<(), ModuleError> {
            for action in &self.actions {
                registrar.admin_action(*action);
            }
            Ok(())
        }

        fn start(&self, _ctx: ModuleContext, _config: ModuleConfig) -> BoxFuture<'_, Result<(), ModuleError>> {
            Box::pin(async { Ok(()) })
        }

        fn stop(&self, _ctx: ModuleContext) -> BoxFuture<'_, Result<(), ModuleError>> {
            Box::pin(async { Ok(()) })
        }

        fn status(&self) -> ModuleStatus {
            ModuleStatus::new(Phase::Ready)
        }

        fn admin(&self, _ctx: ModuleContext, call: AdminCall) -> BoxFuture<'_, Result<AdminResponse, ModuleError>> {
            Box::pin(async move {
                if self.down.load(Ordering::SeqCst) {
                    return Err(ModuleError::new(ErrorKind::Unavailable, "stopped"));
                }
                self.calls.lock().unwrap().push(call.clone());
                let players = self.players.lock().unwrap();
                let (AdminRequest::Kick { player, .. }
                | AdminRequest::Ban { player, .. }
                | AdminRequest::Lookup { player }) = &call.request;
                let Some(found) = players.iter().find(|p| p.name.eq_ignore_ascii_case(player)) else {
                    return Ok(AdminResponse::NotFound);
                };
                Ok(match call.request {
                    AdminRequest::Lookup { .. } => AdminResponse::Player(PlayerInfo {
                        account: Some(found.account.clone()),
                        name: found.name.to_owned(),
                        online: found.online,
                    }),
                    _ if found.admin => return Err(ModuleError::failed("this player is a server admin")),
                    AdminRequest::Kick { .. } | AdminRequest::Ban { .. } => AdminResponse::Done,
                })
            })
        }
    }

    /// What DF3's controller does for the admin plugin: one call to one loaded module's driver.
    struct Drivers(BTreeMap<ModuleId, InProcessModule>);

    impl AdminCalls for Drivers {
        fn admin<'a>(
            &'a self,
            module: &'a ModuleId,
            call: AdminCall,
        ) -> BoxFuture<'a, Result<AdminResponse, ModuleError>> {
            Box::pin(async move {
                match self.0.get(module) {
                    Some(driver) => driver.admin(call).await,
                    None => Err(ModuleError::new(ErrorKind::Unavailable, "not loaded")),
                }
            })
        }
    }

    /// Loads the games and attaches them, as the controller would.
    async fn load(games: &[Arc<StubGame>]) -> Arc<AdminPlugin> {
        let mut drivers = BTreeMap::new();
        let mut registered = Vec::new();
        for game in games {
            let module: Arc<dyn GameModule> = game.clone();
            let driver = InProcessModule::new(module, NodeIdentity::standalone(), recording_host()).unwrap();
            registered.push((driver.manifest().clone(), driver.register().await.unwrap()));
            drivers.insert(driver.manifest().id.clone(), driver);
        }
        let plugin = Arc::new(AdminPlugin::new(Arc::new(Drivers(drivers))));
        for (manifest, registration) in registered {
            plugin.attach(&manifest, &registration).unwrap();
        }
        plugin
    }

    async fn world_on(games: &[Arc<StubGame>]) -> World {
        world_with(Options {
            director_up: true,
            secret: SECRET,
            modules: load(games).await,
        })
        .await
    }

    /// The 1.14.7 module takes over from the Node backend, so it holds the backend's names.
    fn isles() -> Arc<StubGame> {
        Arc::new(StubGame::with(
            "isles-stub",
            "1.14.7",
            vec![
                stub_player("Ada", true, false),
                stub_player("Bo_Slayer", false, false),
                stub_player("Cyra", true, true),
            ],
        ))
    }

    /// The 1.4.4 module runs beside the Node backend, with names of its own.
    fn revived_with(extra: Vec<StubPlayer>) -> Arc<StubGame> {
        let mut players = vec![
            stub_player("Wren", true, false),
            stub_player("Moss_Eater", false, false),
            stub_player("Kestrel", true, true),
        ];
        players.extend(extra);
        Arc::new(StubGame::with("revived-stub", "1.4.4", players))
    }

    /// Who RD25's cases act on in one game: someone online, someone offline, an admin.
    struct Cast {
        game: &'static str,
        online: &'static str,
        offline: &'static str,
        admin: &'static str,
        /// What refusing an admin answers: the backend's own code, or a module's refusal, since module-api
        /// has no error kind for it
        protected: &'static str,
    }

    const NODE_CAST: Cast = Cast {
        game: "1.14.7",
        online: "Ada",
        offline: "Bo_Slayer",
        admin: "Cyra",
        protected: "PROTECTED_ACCOUNT",
    };

    const KICK: &str = "mutation($name: String!) { kick(username: $name) { userId username connectionsClosed game } }";
    const BAN: &str =
        "mutation($name: String!, $reason: String!) { ban(username: $name, reason: $reason) { username game } }";
    const LOOKUP: &str = "query($name: String!) { player(username: $name) { userId username online game } }";

    /// RD25's lookup, kick and ban cases, as every game answers them. Returns the player's user id.
    async fn rd25_cases(w: &World, cast: &Cast) -> String {
        let found = w.gql(LOOKUP, json!({"name": cast.online.to_lowercase()})).await;
        assert_eq!(found["data"]["player"]["username"], cast.online, "{found}");
        assert_eq!(found["data"]["player"]["online"], true);
        assert_eq!(found["data"]["player"]["game"], cast.game);
        let offline = w.gql(LOOKUP, json!({"name": cast.offline})).await;
        assert_eq!(offline["data"]["player"]["online"], false);
        assert_eq!(
            w.gql(LOOKUP, json!({"name": "Nobody"})).await["data"]["player"],
            Value::Null
        );
        let part = &cast.offline[..3];
        assert_eq!(
            w.gql(LOOKUP, json!({"name": part})).await["data"]["player"],
            Value::Null
        );
        assert_eq!(code(&w.gql(LOOKUP, json!({"name": "no spaces"})).await), "BAD_INPUT");

        let kicked = w.gql(KICK, json!({"name": cast.online.to_uppercase()})).await;
        assert_eq!(kicked["data"]["kick"]["username"], cast.online, "{kicked}");
        assert_eq!(kicked["data"]["kick"]["userId"], found["data"]["player"]["userId"]);
        assert_eq!(kicked["data"]["kick"]["game"], cast.game);
        assert_eq!(code(&w.gql(KICK, json!({"name": cast.admin})).await), cast.protected);
        assert_eq!(code(&w.gql(KICK, json!({"name": "Nobody"})).await), "NOT_FOUND");
        assert_eq!(code(&w.gql(KICK, json!({"name": "x"})).await), "BAD_INPUT");

        let banned = w
            .gql(BAN, json!({"name": cast.offline, "reason": "griefing\nhunts"}))
            .await;
        assert_eq!(banned["data"]["ban"]["username"], cast.offline, "{banned}");
        assert_eq!(banned["data"]["ban"]["game"], cast.game);
        assert_eq!(
            code(&w.gql(BAN, json!({"name": cast.offline, "reason": "x"})).await),
            "BAD_INPUT"
        );
        assert_eq!(
            code(&w.gql(BAN, json!({"name": cast.admin, "reason": "testing"})).await),
            cast.protected
        );
        assert_eq!(
            code(&w.gql(BAN, json!({"name": "Nobody", "reason": "testing"})).await),
            "NOT_FOUND"
        );
        found["data"]["player"]["userId"].as_str().unwrap().to_owned()
    }

    fn sent_as_owner(game: &StubGame) {
        let calls = game.calls();
        assert!(!calls.is_empty());
        for call in calls {
            assert_eq!(call.actor.actor, Actor::Admin { key: "owner".into() });
            assert!(call.actor.request.as_deref().unwrap().starts_with("admin-"));
            assert!(call.actor.is_valid());
        }
    }

    /// What the cases sent: the admin's refusals are the module's to make, so they reach it too.
    fn sent(game: &StubGame, cast: &Cast) {
        let kick = |player: &str| AdminRequest::Kick {
            player: player.into(),
            reason: None,
        };
        let ban = |player: &str, reason: &str| AdminRequest::Ban {
            player: player.into(),
            reason: Some(reason.into()),
            until: None,
        };
        assert_eq!(
            game.actions(),
            [
                kick(cast.online),
                kick(cast.admin),
                ban(cast.offline, "griefing hunts"),
                ban(cast.admin, "testing"),
            ]
        );
    }

    #[tokio::test]
    async fn rd25_cases_on_the_node_backend() {
        let w = world().await;
        assert_eq!(rd25_cases(&w, &NODE_CAST).await, "u-ada");
        assert_eq!(*w.backend.kicked.lock().unwrap(), ["Ada"]);
        assert_eq!(w.status_of("u-bo"), "banned");
    }

    #[tokio::test]
    async fn rd25_cases_on_a_1_14_7_module_which_takes_over_from_the_node_backend() {
        let isles = isles();
        let w = world_on(std::slice::from_ref(&isles)).await;
        let cast = Cast {
            protected: "BACKEND_REFUSED",
            ..NODE_CAST
        };
        assert_eq!(rd25_cases(&w, &cast).await, isles.account_of("Ada"));
        sent(&isles, &cast);
        sent_as_owner(&isles);
        // The Node backend was never asked
        assert!(w.backend.kicked.lock().unwrap().is_empty());
        assert_eq!(w.status_of("u-bo"), "active");
    }

    #[tokio::test]
    async fn rd25_cases_on_a_1_4_4_module_beside_the_node_backend() {
        let revived = revived_with(vec![]);
        let w = world_on(std::slice::from_ref(&revived)).await;
        let cast = Cast {
            game: "1.4.4",
            online: "Wren",
            offline: "Moss_Eater",
            admin: "Kestrel",
            protected: "BACKEND_REFUSED",
        };
        assert_eq!(rd25_cases(&w, &cast).await, revived.account_of("Wren"));
        sent(&revived, &cast);
        sent_as_owner(&revived);
        // The Node backend's players are still its own
        assert_eq!(rd25_cases(&w, &NODE_CAST).await, "u-ada");
        assert_eq!(*w.backend.kicked.lock().unwrap(), ["Ada"]);
    }

    #[tokio::test]
    async fn a_name_in_two_games_needs_the_game() {
        let revived = revived_with(vec![stub_player("Ada", false, false)]);
        let w = world_on(std::slice::from_ref(&revived)).await;
        let lookup = "query($name: String!, $game: String) { player(username: $name, game: $game) { userId game } }";
        let body = w.gql(lookup, json!({"name": "ada"})).await;
        assert_eq!(code(&body), "AMBIGUOUS_PLAYER");
        assert!(body["errors"][0]["message"].as_str().unwrap().contains("1.14.7, 1.4.4"));
        assert_eq!(code(&w.gql(KICK, json!({"name": "Ada"})).await), "AMBIGUOUS_PLAYER");
        let body = w.gql(lookup, json!({"name": "ada", "game": "1.4.4"})).await;
        assert_eq!(
            body["data"]["player"],
            json!({"userId": revived.account_of("Ada"), "game": "1.4.4"})
        );
        let body = w.gql(lookup, json!({"name": "ada", "game": " 1.14.7 "})).await;
        assert_eq!(body["data"]["player"], json!({"userId": "u-ada", "game": "1.14.7"}));
        assert_eq!(
            code(&w.gql(lookup, json!({"name": "ada", "game": "9.9"})).await),
            "UNKNOWN_GAME"
        );
        assert_eq!(
            code(&w.gql(lookup, json!({"name": "ada", "game": "reforged"})).await),
            "BAD_INPUT"
        );
        let kick = "mutation { kick(username: \"Ada\", game: \"1.4.4\") { game } }";
        assert_eq!(w.gql(kick, json!({})).await["data"]["kick"]["game"], "1.4.4");
        assert!(w.backend.kicked.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_game_that_does_not_answer_stops_an_action_but_not_a_lookup() {
        let revived = revived_with(vec![]);
        let w = world_on(std::slice::from_ref(&revived)).await;
        revived.down.store(true, Ordering::SeqCst);
        // Ada might also be in 1.4.4, which can't say
        let body = w.gql(LOOKUP, json!({"name": "Ada"})).await;
        assert_eq!(body["data"]["player"]["userId"], "u-ada");
        let body = w.gql(KICK, json!({"name": "Ada"})).await;
        assert_eq!(code(&body), "BACKEND_UNAVAILABLE");
        assert!(body["errors"][0]["message"].as_str().unwrap().contains("Stub 1.4.4"));
        assert!(w.backend.kicked.lock().unwrap().is_empty());
        let kick = "mutation { kick(username: \"Ada\", game: \"1.14.7\") { username } }";
        assert_eq!(w.gql(kick, json!({})).await["data"]["kick"]["username"], "Ada");
        // Found nowhere that answered is not "no such player" while a game is silent
        assert_eq!(
            code(&w.gql(LOOKUP, json!({"name": "Wren"})).await),
            "BACKEND_UNAVAILABLE"
        );
    }

    #[tokio::test]
    async fn a_module_keeps_its_own_timeouts_and_lifts_nothing_yet() {
        let revived = revived_with(vec![]);
        let w = world_on(std::slice::from_ref(&revived)).await;
        let body = w
            .gql(TIMEOUT, json!({"name": "Wren", "minutes": 30, "reason": "spam"}))
            .await;
        assert_eq!(body["data"]["timeout"]["status"], "UNKNOWN", "{body}");
        assert_eq!(body["data"]["timeout"]["restriction"], Value::Null);
        assert_eq!(
            revived.actions(),
            [AdminRequest::Ban {
                player: "Wren".into(),
                reason: Some("Timed out until 2027-01-15T08:30:00Z: spam".into()),
                until: Some("2027-01-15T08:30:00Z".into()),
            }]
        );
        assert!(w.admin.timeouts.all().await.is_empty());
        for lift in [
            "mutation { unban(username: \"Wren\") { status } }",
            "mutation { liftTimeout(username: \"Wren\") { status } }",
        ] {
            assert_eq!(code(&w.gql(lift, json!({})).await), "NOT_SUPPORTED", "{lift}");
        }
    }

    #[test]
    fn the_plugin_takes_game_modules_that_answer_lookups_one_per_version() {
        let plugin = AdminPlugin::none();
        let attach = |game: StubGame| {
            let mut registrar = Registrar::new(game.manifest());
            game.register(&mut registrar).unwrap();
            plugin.attach(&game.manifest(), &registrar.finish().unwrap())
        };
        attach(StubGame::with("revived-stub", "1.4.4", vec![])).unwrap();
        assert!(plugin.has_game("1.4.4") && !plugin.has_game("1.14.7"));
        assert!(
            attach(StubGame::with("revived-twin", "1.4.4", vec![])).is_err(),
            "a second module for 1.4.4"
        );
        let no_lookup = StubGame {
            actions: vec![AdminAction::Kick],
            ..StubGame::with("no-lookup", "1.12.0", vec![])
        };
        assert!(attach(no_lookup).is_err(), "kick without lookup");
        let a_plugin = StubGame {
            kind: ModuleKind::Plugin,
            ..StubGame::with("a-plugin", "1.14.7", vec![])
        };
        assert!(attach(a_plugin).is_err(), "a plugin has no players");
        let quiet = StubGame {
            actions: vec![],
            ..StubGame::with("quiet", "1.12.0", vec![])
        };
        attach(quiet).unwrap();
        assert!(!plugin.has_game("1.12.0"), "no admin actions, nothing to route");
        plugin.detach(&"revived-stub".parse().unwrap());
        assert!(plugin.modules().is_empty());
    }
}
