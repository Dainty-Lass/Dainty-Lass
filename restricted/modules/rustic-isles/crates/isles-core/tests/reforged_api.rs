//! RU5 reforged-api: the probes and the Reforged API (`/reforged/api/*`), over a real socket. The expectations are
//! what `app.ts`, `routes/reforgedApi.ts`, `routes/metrics.ts`, `routes/releaseGates.ts` and `routes/loginQueue.ts`
//! send (the golden table in `tests/golden/reforged_api.json` is captured from those files, see
//! `tools/golden/README.md`), for both versions where the area exists.
#![allow(clippy::unwrap_used, clippy::expect_used)] // tests

mod common;

use std::sync::Arc;

use common::{Fakes, get, post_json, send, serve, shared, store};
use isles_core::areas::reforged_api::presence::{self, ChatSessions};
use isles_core::routes::RouteSet;
use serde_json::{Value, json};

const ADMIN_KEY: &str = "0123456789abcdef0123456789abcdef0123";
const HASH_SECRET: &str = "a-test-secret-of-at-least-32-characters";

fn settings() -> Vec<(&'static str, &'static str)> {
    vec![
        ("REGISTRATION_MODE", "OPEN"),
        ("API_KEY_HASH_SECRET", HASH_SECRET),
        ("REFORGED_ADMIN_KEY", ADMIN_KEY),
        ("REFORGED_SERVER_NAME", "  Fox\u{7}hold  "),
        ("REFORGED_SERVER_VERSION", "1.2.3"),
        ("REFORGED_SOURCE_URL", "https://Example.COM/src"),
        ("RUSTIC_ISLES_ADMIN_API", "on"),
    ]
}

struct Server {
    address: std::net::SocketAddr,
    fakes: Fakes,
    _dir: tempfile::TempDir,
}

async fn server(routes: RouteSet, extra: &[(&str, &str)]) -> Server {
    let dir = tempfile::tempdir().unwrap();
    let fakes = Fakes::default();
    let mut pairs = settings();
    pairs.retain(|(name, _)| !extra.iter().any(|(n, _)| n == name));
    pairs.extend_from_slice(extra);
    let address = serve(shared(routes, &pairs, Some(store(&dir, &fakes)))).await;
    Server {
        address,
        fakes,
        _dir: dir,
    }
}

async fn register(server: &Server, name: &str, code: Option<&str>) -> common::Answer {
    let body = match code {
        Some(code) => json!({"Username": name, "InviteCode": code}),
        None => json!({"Username": name}),
    };
    post_json(server.address, "/reforged/api/Register", &[], &body.to_string()).await
}

fn host_headers() -> Vec<(&'static str, &'static str)> {
    vec![("x-reforged-admin-key", ADMIN_KEY)]
}

#[tokio::test(flavor = "multi_thread")]
async fn registration_and_the_key_it_makes() {
    let s = server(RouteSet::V1147, &[]).await;
    let reg = get(s.address, "/reforged/api/RegistrationStatus", &[]).await;
    assert_eq!((reg.status, reg.text().as_str()), (200, r#"{"mode":"OPEN"}"#));
    assert_eq!(reg.header("content-type"), Some("application/json; charset=utf-8"));

    let bad = post_json(s.address, "/reforged/api/Register", &[], "{\"Username\":5}").await;
    assert_eq!(bad.status, 400);
    assert_eq!(bad.json()["error"], "bad_request");
    let bad_code = post_json(
        s.address,
        "/reforged/api/Register",
        &[],
        "{\"Username\":\"Fox_9\",\"InviteCode\":null}",
    )
    .await;
    assert_eq!(bad_code.json()["error"], "bad_request");
    let short = register(&s, "ab", None).await;
    assert_eq!(short.status, 400);
    assert_eq!(
        short.text(),
        r#"{"error":"username_invalid","message":"A username is 3 to 16 characters: letters, digits and underscore."}"#
    );

    let ok = register(&s, " Fox_9 ", None).await;
    assert_eq!(ok.status, 200);
    let key = ok.json()["UUK"].as_str().unwrap().to_owned();
    assert!(key.starts_with("RRK_") && key.len() == 52, "{key}");
    assert_eq!(ok.json().as_object().unwrap().len(), 1);

    let taken = register(&s, "fox_9", None).await;
    assert_eq!(taken.status, 409);
    assert_eq!(taken.json()["error"], "username_taken");

    let me = get(
        s.address,
        "/reforged/api/GetUserInfo",
        &[("x-reforged-user-api-key", &key)],
    )
    .await;
    assert_eq!(me.status, 200);
    let body = me.json();
    assert_eq!(body["Username"], "Fox_9");
    assert_eq!(body["IsAdmin"], false);
    assert_eq!(
        me.text(),
        format!(
            r#"{{"UserId":"{}","Username":"Fox_9","IsAdmin":false}}"#,
            body["UserId"].as_str().unwrap()
        )
    );

    let none = get(s.address, "/reforged/api/GetUserInfo", &[]).await;
    assert_eq!(
        (none.status, none.text().as_str()),
        (401, r#"{"error":"unauthorized"}"#)
    );
    let wrong = get(
        s.address,
        "/reforged/api/GetUserInfo",
        &[("x-reforged-user-api-key", "RRK_00000000000000000000")],
    )
    .await;
    assert_eq!(wrong.status, 401);
    let odd = get(
        s.address,
        "/reforged/api/GetUserInfo",
        &[("x-reforged-user-api-key", "no good")],
    )
    .await;
    assert_eq!(odd.status, 401);
}

#[tokio::test(flavor = "multi_thread")]
async fn registration_can_be_closed_or_need_an_invite() {
    let closed = server(RouteSet::V1147, &[("REGISTRATION_MODE", "none")]).await;
    let answer = register(&closed, "Fox_9", None).await;
    assert_eq!(answer.status, 400);
    assert_eq!(answer.json()["error"], "registration_closed");

    let s = server(RouteSet::V1147, &[("REGISTRATION_MODE", "")]).await;
    assert_eq!(
        get(s.address, "/reforged/api/RegistrationStatus", &[]).await.text(),
        r#"{"mode":"INVITECODE"}"#
    );
    let refused = register(&s, "Fox_9", Some("nope")).await;
    assert_eq!(refused.status, 401);
    assert_eq!(refused.json()["error"], "invite_invalid");
    assert_eq!(register(&s, "Fox_9", None).await.status, 401);

    // A one-use code: the second registration finds it spent
    let made = post_json(
        s.address,
        "/reforged/api/CreateInvite",
        &host_headers(),
        r#"{"name":"for\u0000Alex"}"#,
    )
    .await;
    assert_eq!(made.status, 200);
    let code = made.json()["code"].as_str().unwrap().to_owned();
    assert_eq!(code.len(), 14);
    assert_eq!(register(&s, "Alex_1", Some(&format!(" {code} "))).await.status, 200);
    assert_eq!(register(&s, "Alex_2", Some(&code)).await.status, 401);

    // A taken name costs the code no use
    let two = post_json(
        s.address,
        "/reforged/api/CreateInvite",
        &host_headers(),
        r#"{"uses":2}"#,
    )
    .await;
    let code2 = two.json()["code"].as_str().unwrap().to_owned();
    assert_eq!(register(&s, "alex_1", Some(&code2)).await.status, 409);
    let list = get(s.address, "/reforged/api/InviteCodes", &host_headers())
        .await
        .json();
    let two_row = list["InviteCodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["code"] == code2.as_str())
        .unwrap();
    assert_eq!(two_row["usesRemaining"], 2);
    assert_eq!(two_row["note"], Value::Null);
    let spent = list["InviteCodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["code"] == code.as_str())
        .unwrap();
    assert_eq!(
        (spent["usesRemaining"].clone(), spent["note"].clone()),
        (json!(0), json!("for Alex"))
    );
    assert_eq!(
        spent.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["code", "usesRemaining", "infiniteUses", "note", "createdAt"]
    );

    // An unlimited code is never spent
    let unlimited = post_json(
        s.address,
        "/reforged/api/CreateInvite",
        &host_headers(),
        r#"{"infinite":true}"#,
    )
    .await;
    let code3 = unlimited.json()["code"].as_str().unwrap().to_owned();
    assert_eq!(register(&s, "Bob_1", Some(&code3)).await.status, 200);
    assert_eq!(register(&s, "Bob_2", Some(&code3)).await.status, 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_host_routes_answer_only_the_host() {
    let s = server(RouteSet::V1147, &[]).await;
    for (method, path) in [
        ("POST", "/reforged/api/CreateInvite"),
        ("GET", "/reforged/api/InviteCodes"),
        ("DELETE", "/reforged/api/InviteCode/ABCD-EFGH-JKMN"),
        ("POST", "/reforged/api/SetAccountStatus"),
        ("POST", "/reforged/api/Kick"),
    ] {
        for headers in [
            vec![],
            vec![("x-reforged-admin-key", "wrong")],
            vec![("x-reforged-admin-key", ADMIN_KEY), ("x-reforged-gateway", "1")],
            vec![("x-reforged-admin-key", ADMIN_KEY), ("x-forwarded-for", "203.0.113.9")],
        ] {
            let answer = send(s.address, method, path, &headers, None).await;
            assert_eq!(
                (answer.status, answer.text().as_str()),
                (403, r#"{"error":"forbidden"}"#),
                "{method} {path}"
            );
        }
    }
    // No admin key configured, or too short a one, locks the routes for everybody
    let locked = server(RouteSet::V1147, &[("REFORGED_ADMIN_KEY", "short")]).await;
    assert_eq!(
        get(
            locked.address,
            "/reforged/api/InviteCodes",
            &[("x-reforged-admin-key", "short")]
        )
        .await
        .status,
        403
    );

    let bad = post_json(
        s.address,
        "/reforged/api/CreateInvite",
        &host_headers(),
        r#"{"uses":0}"#,
    )
    .await;
    assert_eq!(
        (bad.status, bad.text()),
        (
            400,
            r#"{"error":"bad_request","message":"uses is a whole number from 1 to 1000; name is text"}"#.to_owned()
        )
    );
    for body in [
        r#"{"uses":1001}"#,
        r#"{"uses":1.5}"#,
        r#"{"uses":"2"}"#,
        r#"{"name":5}"#,
    ] {
        assert_eq!(
            post_json(s.address, "/reforged/api/CreateInvite", &host_headers(), body)
                .await
                .status,
            400,
            "{body}"
        );
    }
    assert_eq!(
        post_json(
            s.address,
            "/reforged/api/CreateInvite",
            &host_headers(),
            r#"{"uses":null}"#
        )
        .await
        .status,
        200
    );

    let code = post_json(s.address, "/reforged/api/CreateInvite", &host_headers(), "{}")
        .await
        .json()["code"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        send(
            s.address,
            "DELETE",
            "/reforged/api/InviteCode/a%20b",
            &host_headers(),
            None
        )
        .await
        .status,
        400
    );
    assert_eq!(
        send(
            s.address,
            "DELETE",
            &format!("/reforged/api/InviteCode/{code}"),
            &host_headers(),
            None
        )
        .await
        .status,
        200
    );
    let gone = send(
        s.address,
        "DELETE",
        &format!("/reforged/api/InviteCode/{code}"),
        &host_headers(),
        None,
    )
    .await;
    assert_eq!((gone.status, gone.body.len()), (404, 0));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_banned_player_is_refused_and_the_host_can_change_that() {
    let s = server(RouteSet::V1147, &[]).await;
    let key = register(&s, "Fox_9", None).await.json()["UUK"]
        .as_str()
        .unwrap()
        .to_owned();
    let keyed = [("x-reforged-user-api-key", key.as_str())];
    let set = |body: &'static str| async move {
        post_json(s.address, "/reforged/api/SetAccountStatus", &host_headers(), body).await
    };

    assert_eq!(set(r#"{"Username":"Fox_9"}"#).await.status, 400);
    assert_eq!(set(r#"{"Username":"Fox_9","Status":"gone"}"#).await.status, 400);
    assert_eq!(
        set(r#"{"Username":"x","Status":"banned"}"#).await.text(),
        r#"{"error":"bad_request","message":"Username and Status (active, banned or disabled) are required."}"#
    );
    let missing = set(r#"{"Username":"Nobody","Status":"banned"}"#).await;
    assert_eq!(missing.status, 404);
    assert_eq!(missing.json()["error"], "not_found");

    let banned = set(r#"{"Username":"fox_9","Status":"banned","Reason":"cheating"}"#).await;
    assert_eq!(banned.status, 200);
    let id = banned.json()["UserId"].as_str().unwrap().to_owned();
    assert_eq!(
        banned.text(),
        format!(r#"{{"UserId":"{id}","Username":"Fox_9","Status":"banned"}}"#)
    );
    let refused = get(s.address, "/reforged/api/GetUserInfo", &keyed).await;
    assert_eq!(
        (refused.status, refused.text()),
        (403, r#"{"error":"account_banned","reason":"cheating"}"#.to_owned())
    );
    assert_eq!(set(r#"{"Username":"Fox_9","Status":"disabled"}"#).await.status, 200);
    let disabled = get(s.address, "/reforged/api/GetUserInfo", &keyed).await;
    assert_eq!(disabled.json()["error"], "account_disabled");
    assert_eq!(disabled.json()["reason"], "cheating");
    assert_eq!(set(r#"{"Username":"Fox_9","Status":"active"}"#).await.status, 200);
    assert_eq!(get(s.address, "/reforged/api/GetUserInfo", &keyed).await.status, 200);

    // Without a reason the launcher still gets a sentence
    let other = register(&s, "Rex_2", None).await.json()["UUK"]
        .as_str()
        .unwrap()
        .to_owned();
    post_json(
        s.address,
        "/reforged/api/SetAccountStatus",
        &host_headers(),
        r#"{"Username":"Rex_2","Status":"banned"}"#,
    )
    .await;
    let refused = get(
        s.address,
        "/reforged/api/GetUserInfo",
        &[("x-reforged-user-api-key", other.as_str())],
    )
    .await;
    assert_eq!(refused.json()["reason"], "Ask the server's host.");
}

struct FakeChat(std::sync::Mutex<Vec<(String, u16, String)>>);

impl ChatSessions for FakeChat {
    fn online_user_ids(&self) -> Vec<String> {
        vec![]
    }
    fn close_connections(&self, user_id: &str, code: u16, reason: &str) -> usize {
        self.0
            .lock()
            .unwrap()
            .push((user_id.to_owned(), code, reason.to_owned()));
        2
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_kick_revokes_tokens_and_closes_chat() {
    let s = server(RouteSet::V1147, &[]).await;
    register(&s, "Fox_9", None).await;
    let chat = Arc::new(FakeChat(std::sync::Mutex::new(vec![])));
    presence::set_chat_sessions(Some(chat.clone()));
    let kick =
        |body: &'static str| async move { post_json(s.address, "/reforged/api/Kick", &host_headers(), body).await };
    assert_eq!(
        kick("{}").await.text(),
        r#"{"error":"bad_request","message":"Username is required."}"#
    );
    assert_eq!(kick(r#"{"Username":"Nobody"}"#).await.status, 404);
    let answer = kick(r#"{"Username":"FOX_9"}"#).await;
    presence::set_chat_sessions(None);
    assert_eq!(answer.status, 200);
    let id = answer.json()["UserId"].as_str().unwrap().to_owned();
    assert_eq!(
        answer.text(),
        format!(r#"{{"UserId":"{id}","Username":"Fox_9","Closed":2}}"#)
    );
    assert_eq!(
        chat.0.lock().unwrap().as_slice(),
        [(id, 1008, "kicked by the host".to_owned())]
    );
    let world = s.fakes.world();
    assert_eq!((world.refresh_revoked.len(), world.codes_revoked.len()), (1, 1));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_status_has_a_full_and_a_limited_view_per_changelist() {
    let s = server(RouteSet::V1147, &[("APPROVED_EXECUTABLE_SHA256", &"AB".repeat(32))]).await;
    let key = register(&s, "Fox_9", None).await.json()["UUK"]
        .as_str()
        .unwrap()
        .to_owned();
    let limited = get(s.address, "/reforged/api/ServerStatus", &[]).await;
    assert_eq!(limited.status, 200);
    assert_eq!(limited.header("cache-control"), Some("no-store"));
    assert_eq!(limited.header("vary"), Some("x-reforged-user-api-key"));
    let body = limited.json();
    assert_eq!(
        body.as_object().unwrap().keys().collect::<Vec<_>>(),
        [
            "name",
            "online",
            "version",
            "commit",
            "sourceUrl",
            "registration",
            "uptimeSeconds",
            "contentPort",
            "game",
            "gameData",
            "chat",
            "playersOnline",
            "players",
            "instances",
            "limited"
        ]
    );
    assert_eq!(body["name"], "Fox hold");
    assert_eq!(body["version"], "1.2.3");
    assert_eq!(body["sourceUrl"], "https://example.com/src");
    assert_eq!(body["registration"], "OPEN");
    assert_eq!(body["gameData"], "missing");
    assert_eq!(body["chat"], false);
    assert_eq!(body["limited"], true);
    assert_eq!(
        body["game"].to_string(),
        format!(
            r#"{{"build":"rel-1.14.7-Archon","changelist":647472,"executableSha256":"{}"}}"#,
            "ab".repeat(32)
        )
    );
    let full = get(
        s.address,
        "/reforged/api/ServerStatus",
        &[("x-reforged-user-api-key", &key)],
    )
    .await
    .json();
    assert_eq!(full["limited"], false);
    assert_eq!(
        (
            full["playersOnline"].clone(),
            full["players"].clone(),
            full["instances"].clone()
        ),
        (json!(0), json!([]), json!([]))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_probes_answer_as_app_ts_does() {
    let s = server(RouteSet::V1147, &[]).await;
    let root = get(s.address, "/", &[]).await;
    assert_eq!((root.status, root.text().as_str()), (200, "ok"));
    assert_eq!(root.header("content-type"), Some("text/html; charset=utf-8"));
    let live = get(s.address, "/health/live", &[]).await;
    assert_eq!(
        live.text(),
        r#"{"ok":true,"service":"paradox-backend","role":"combined"}"#
    );
    let ready = get(s.address, "/health/ready", &[]).await;
    assert_eq!(
        ready.text(),
        r#"{"ok":true,"service":"paradox-backend","role":"combined","reason":"READY","persistence":"ready"}"#
    );
    let api = server(RouteSet::V1147, &[("RUSTIC_ISLES_SERVICE_ROLE", " API ")]).await;
    assert_eq!(get(api.address, "/health/live", &[]).await.json()["role"], "api");

    let maintenance = get(s.address, "/maintenance/status", &[]).await;
    assert_eq!(maintenance.text(), r#"{"show-status":false}"#);
    assert_eq!(
        maintenance.header("content-type"),
        Some("application/json; charset=utf-8")
    );

    // The Reforged route of this path is shadowed by Paradox's, so the client gets the welcome text
    let status = get(s.address, "/dauntless-status", &[]).await;
    let welcome = "Welcome to Rustic Isles!";
    assert_eq!(
        status.text(),
        format!(
            r#"{{"show-status":true,"en":"{welcome}","fr":"{welcome}","it":"{welcome}","es":"{welcome}","de":"{welcome}","pt":"{welcome}","ru":"{welcome}","ja":"{welcome}"}}"#
        )
    );

    // Without a database the node isn't ready, and says so without detail
    let bare = serve(shared(RouteSet::V1147, &settings(), None)).await;
    let down = get(bare, "/health/ready", &[]).await;
    assert_eq!(down.status, 503);
    assert_eq!(
        down.text(),
        r#"{"ok":false,"service":"paradox-backend","role":"combined","reason":"API_BEHIND_PROXY_RUNNING","persistence":"unavailable"}"#
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_release_gates_are_for_the_local_manager() {
    let s = server(RouteSet::V1147, &[]).await;
    let gates = get(s.address, "/health/release-gates", &[]).await;
    assert_eq!(gates.status, 200);
    let body = gates.json();
    assert_eq!(body["ok"], true);
    let names: Vec<_> = body["gates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "no_active_play",
            "guard_mode",
            "registration_approval_mode",
            "tls_proxy_consistency",
            "no_legacy_env_keys"
        ]
    );
    assert_eq!(body["gates"][0]["detail"], "nobody in game");
    for header in ["x-forwarded-for", "x-real-ip", "forwarded"] {
        let refused = get(s.address, "/health/release-gates", &[(header, "203.0.113.7")]).await;
        assert_eq!((refused.status, refused.body.len()), (404, 0), "{header}");
    }
    let held = server(
        RouteSet::V1147,
        &[("MYSTPAX_OLD", "1"), ("RUSTIC_ISLES_GUARD_MODE", "sometimes")],
    )
    .await;
    let answer = get(held.address, "/health/release-gates", &[]).await;
    assert_eq!(answer.status, 503);
    assert_eq!(answer.json()["ok"], false);
    // With the admin API off the route isn't mounted at all
    let off = server(RouteSet::V1147, &[("RUSTIC_ISLES_ADMIN_API", "off")]).await;
    assert_eq!(get(off.address, "/health/release-gates", &[]).await.status, 404);
}

#[tokio::test(flavor = "multi_thread")]
async fn metrics_need_loopback_or_a_token() {
    let plain = server(RouteSet::V1147, &[]).await;
    let ok = get(plain.address, "/internal/v1/metrics", &[]).await;
    assert_eq!(ok.status, 200);
    assert_eq!(
        ok.header("content-type"),
        Some("text/plain; charset=utf-8; version=0.0.4")
    );
    assert_eq!(ok.header("cache-control"), Some("no-store"));
    assert!(ok.text().contains("rustic_isles_service_ready 1\n"));
    assert!(
        ok.text()
            .contains("rustic_isles_http_requests_total{class=\"launcher\",method=\"GET\"}")
            || ok.text().contains("rustic_isles_http_requests_total")
    );

    let production = server(RouteSet::V1147, &[("NODE_ENV", "production")]).await;
    let refused = get(production.address, "/internal/v1/metrics", &[]).await;
    assert_eq!(
        (refused.status, refused.text().as_str()),
        (503, "metrics unavailable\n")
    );
    assert_eq!(refused.header("content-type"), Some("text/plain; charset=utf-8"));

    let token = server(RouteSet::V1147, &[("RUSTIC_ISLES_METRICS_TOKEN", "route-test-token")]).await;
    assert_eq!(get(token.address, "/internal/v1/metrics", &[]).await.status, 401);
    assert_eq!(
        get(
            token.address,
            "/internal/v1/metrics",
            &[("authorization", "Bearer wrong")]
        )
        .await
        .status,
        401
    );
    let bearer = get(
        token.address,
        "/internal/v1/metrics",
        &[("authorization", "Bearer route-test-token")],
    )
    .await;
    assert!(bearer.status == 200 && bearer.text().contains("rustic_isles_service_ready"));
    let header = get(
        token.address,
        "/internal/v1/metrics",
        &[("x-rustic-isles-metrics-token", "route-test-token")],
    )
    .await;
    assert_eq!(header.status, 200);

    // The names from before the Rustic Isles rename still work for one release
    let legacy = server(RouteSet::V1147, &[("MYSTICPARADOX_METRICS_TOKEN", "route-test-token")]).await;
    assert_eq!(get(legacy.address, "/internal/v1/metrics", &[]).await.status, 401);
    let old_header = get(
        legacy.address,
        "/internal/v1/metrics",
        &[("x-mysticparadox-metrics-token", "route-test-token")],
    )
    .await;
    assert_eq!(old_header.status, 200);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_node_without_a_database_answers_the_skeleton_501() {
    let bare = serve(shared(RouteSet::V1147, &settings(), None)).await;
    let answer = post_json(bare, "/reforged/api/Register", &[], r#"{"Username":"Fox_9"}"#).await;
    assert_eq!(answer.status, 501);
    assert_eq!(answer.json()["error"]["code"], "NOT_IMPLEMENTED");
    // The host routes are guarded first
    assert_eq!(get(bare, "/reforged/api/InviteCodes", &[]).await.status, 403);
    assert_eq!(
        get(bare, "/reforged/api/InviteCodes", &host_headers()).await.status,
        501
    );
    assert_eq!(get(bare, "/reforged/api/RegistrationStatus", &[]).await.status, 200);
}

#[test]
fn the_group_s_routes_are_ported_but_the_ones_listed() {
    use isles_core::routes::{Group, Side, all};
    let handlers = isles_core::areas::backend();
    let mut left: Vec<String> = all()
        .iter()
        .filter(|r| {
            matches!(r.group, Group::ReforgedApi | Group::Guard) && r.side == Side::Isles
                || r.group == Group::Guard && !r.path.contains("hunt")
        })
        .filter(|r| handlers.get(r).is_none())
        .map(|r| r.name())
        .collect();
    left.sort();
    assert!(left.is_empty(), "{left:?}");
}
