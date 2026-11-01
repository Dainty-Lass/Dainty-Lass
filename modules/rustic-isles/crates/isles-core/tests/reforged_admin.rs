//! RU5 reforged-api: the admin API's cookie session, its CSRF gate and origin check, the players and their access, the
//! audit and the launcher notices, as `routes/admin.ts`, `controllers/admin.ts`, `HasAdminAuth.ts` and
//! `controllers/launcherNotices.ts` answer them. Sessions are made through the repository: the password and
//! authenticator checks are the auth area's, and a fake stands in for them in the sign-in test.
#![allow(clippy::unwrap_used, clippy::expect_used)] // tests

mod common;

use std::net::SocketAddr;
use std::sync::Arc;

use common::{Fakes, get, send, serve, shared, store};
use isles_core::areas::reforged_api::admin::{AdminCredentials, set_credentials};
use isles_core::areas::reforged_api::util::{iso, now_ms, sha256_hex};
use isles_core::db::IslesStore;
use isles_core::repo::auth::{AccountRecord, ApprovalStatus, LauncherAccountRecord, OperationalStatus};
use isles_core::repo::reforged::AdminSessionRecord;
use isles_core::routes::RouteSet;
use isles_core::{AccountId, UserId};
use module_api::{Actor, ActorContext, NodeIdentity};
use serde_json::{Value, json};

/// The credentials are process-wide, so the tests that set them take turns.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const COOKIE: &str = "mysticparadox_admin=raw-session-token";
const CSRF: &str = "csrf-token-1";

struct Rig {
    address: SocketAddr,
    store: IslesStore,
    fakes: Fakes,
    admin: (AccountId, String),
    player: (AccountId, String),
    _dir: tempfile::TempDir,
}

fn launcher(
    account: &AccountId,
    user_id: &UserId,
    name: &str,
    email: Option<&str>,
    roles: &[&str],
) -> LauncherAccountRecord {
    LauncherAccountRecord {
        account: account.clone(),
        user_id: user_id.clone(),
        email: email.map(str::to_owned),
        display_name_normalized: name.to_lowercase(),
        display_name: name.to_owned(),
        password_hash: Some("hash".into()),
        status: OperationalStatus::Active,
        approval_status: Some(ApprovalStatus::Approved),
        approval_updated_at: None,
        approval_updated_by: None,
        approval_reason: None,
        roles: roles.iter().map(|r| (*r).to_owned()).collect(),
        roles_updated_at: None,
        roles_updated_by: None,
        created_at: "2026-10-01T00:00:00.000Z".into(),
        last_login_at: None,
        username_set: Some(true),
        consent: None,
        deletion: None,
        email_verified_at: None,
        email_verification_pending_since: None,
        timeout: None,
    }
}

async fn rig(routes: RouteSet, extra: &[(&str, &str)]) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let fakes = Fakes::default();
    let opened = store(&dir, &fakes);
    let node = NodeIdentity::standalone().node;
    let make = |name: &str, email: Option<&str>, roles: &[&str]| {
        let id = AccountId::new(&node);
        let user_id = UserId::random();
        let mut world = fakes.world();
        world.accounts.push(AccountRecord {
            id: id.clone(),
            user_id: user_id.clone(),
            name: name.to_owned(),
            notes: 0,
        });
        world.launchers.push(launcher(&id, &user_id, name, email, roles));
        (id, user_id.to_string())
    };
    let admin = make("Boss", Some("boss@example.test"), &["player", "admin"]);
    let player = make("Fox_9", Some("fox@example.test"), &["player"]);
    let session = AdminSessionRecord {
        id: "session-1".into(),
        token_hash: sha256_hex(b"raw-session-token"),
        account: admin.0.clone(),
        csrf_token: CSRF.into(),
        created_at: iso(now_ms()),
        expires_at: iso(now_ms() + 3_600_000),
        ip: "127.0.0.1".into(),
        user_agent: "test".into(),
        revoked_at: None,
    };
    opened
        .write(&ActorContext::new(Actor::System), move |tx, repos| {
            repos.admin.create_session(tx, &session)
        })
        .await
        .unwrap();
    let mut pairs = vec![("MYSTICPARADOX_ADMIN_API", "on")];
    pairs.extend_from_slice(extra);
    let address = serve(shared(routes, &pairs, Some(opened.clone()))).await;
    Rig {
        address,
        store: opened,
        fakes,
        admin,
        player,
        _dir: dir,
    }
}

fn session_headers() -> [(&'static str, &'static str); 2] {
    [("cookie", COOKIE), ("x-csrf-token", CSRF)]
}

async fn patch(r: &Rig, path: &str, body: &str) -> common::Answer {
    let headers = session_headers();
    send(
        r.address,
        "PATCH",
        path,
        &headers,
        Some(("application/json", body.as_bytes())),
    )
    .await
}

#[tokio::test(flavor = "multi_thread")]
async fn the_session_cookie_and_the_csrf_token_gate_every_route() {
    let r = rig(RouteSet::V1147, &[]).await;
    let none = get(r.address, "/admin/v1/me", &[]).await;
    assert_eq!(none.status, 401);
    assert_eq!(
        none.text(),
        r#"{"error":{"code":"ADMIN_UNAUTHORIZED","message":"Administrator sign-in required."}}"#
    );
    let unknown = get(r.address, "/admin/v1/me", &[("cookie", "mysticparadox_admin=nope")]).await;
    assert_eq!(unknown.json()["error"]["message"], "Administrator session expired.");
    let broken = get(r.address, "/admin/v1/me", &[("cookie", "mysticparadox_admin=%E0%A4%A")]).await;
    assert_eq!(broken.status, 500);

    let me = get(r.address, "/admin/v1/me", &[("cookie", COOKIE)]).await;
    assert_eq!(me.status, 200);
    assert_eq!(me.header("cache-control"), Some("no-store"));
    assert_eq!(
        me.header("content-security-policy"),
        Some("default-src 'none'; frame-ancestors 'none'")
    );
    let body = me.json();
    assert_eq!(body["csrfToken"], CSRF);
    assert_eq!(
        body["account"].as_object().unwrap().keys().collect::<Vec<_>>(),
        [
            "userId",
            "displayName",
            "email",
            "approvalStatus",
            "status",
            "roles",
            "usernameSet",
            "createdAt"
        ]
    );
    assert_eq!(body["account"]["userId"], r.admin.1.as_str());
    assert_eq!(body["account"]["email"], "boss@example.test");

    // A change needs the token the session was given
    let no_token = send(
        r.address,
        "PATCH",
        &format!("/admin/v1/players/{}/roles", r.player.1),
        &[("cookie", COOKIE)],
        Some(("application/json", b"{}")),
    )
    .await;
    assert_eq!(
        no_token.text(),
        r#"{"error":{"code":"ADMIN_CSRF","message":"Invalid request token."}}"#
    );
    let wrong = send(
        r.address,
        "PATCH",
        &format!("/admin/v1/players/{}/roles", r.player.1),
        &[("cookie", COOKIE), ("x-csrf-token", "x")],
        Some(("application/json", b"{}")),
    )
    .await;
    assert_eq!(wrong.status, 403);

    // An account that stops being an admin loses its session
    r.fakes
        .world()
        .launchers
        .iter_mut()
        .find(|a| a.account == r.admin.0)
        .unwrap()
        .roles = vec!["player".into()];
    let forbidden = get(r.address, "/admin/v1/me", &[("cookie", COOKIE)]).await;
    assert_eq!(
        forbidden.text(),
        r#"{"error":{"code":"ADMIN_FORBIDDEN","message":"Administrator access denied."}}"#
    );
    r.fakes
        .world()
        .launchers
        .iter_mut()
        .find(|a| a.account == r.admin.0)
        .unwrap()
        .roles = vec!["player".into(), "admin".into()];
    assert_eq!(
        get(r.address, "/admin/v1/me", &[("cookie", COOKIE)]).await.status,
        401,
        "the session was revoked"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn only_a_listed_origin_is_answered() {
    let r = rig(RouteSet::V1147, &[]).await;
    let denied = get(
        r.address,
        "/admin/v1/me",
        &[("cookie", COOKIE), ("origin", "https://evil.example")],
    )
    .await;
    assert_eq!((denied.status, denied.body.len()), (403, 0));
    let allowed = get(
        r.address,
        "/admin/v1/me",
        &[("cookie", COOKIE), ("origin", "http://localhost:4173")],
    )
    .await;
    assert_eq!(allowed.status, 200);
    assert_eq!(
        allowed.header("access-control-allow-origin"),
        Some("http://localhost:4173")
    );
    assert_eq!(allowed.header("access-control-allow-credentials"), Some("true"));
    assert_eq!(
        allowed.header("access-control-allow-headers"),
        Some("Content-Type, X-CSRF-Token")
    );
    assert_eq!(
        allowed.header("access-control-allow-methods"),
        Some("GET, POST, PUT, PATCH, DELETE, OPTIONS")
    );
    assert_eq!(allowed.header("vary"), Some("Origin"));
    let configured = rig(
        RouteSet::V1147,
        &[("ADMIN_ALLOWED_ORIGINS", " https://panel.example , ")],
    )
    .await;
    assert_eq!(
        get(
            configured.address,
            "/admin/v1/me",
            &[("cookie", COOKIE), ("origin", "https://panel.example")]
        )
        .await
        .status,
        200
    );
    assert_eq!(
        get(
            configured.address,
            "/admin/v1/me",
            &[("cookie", COOKIE), ("origin", "http://localhost:4173")]
        )
        .await
        .status,
        403
    );
    let production = rig(RouteSet::V1147, &[("NODE_ENV", "production")]).await;
    assert_eq!(
        get(
            production.address,
            "/admin/v1/me",
            &[("cookie", COOKIE), ("origin", "http://localhost:4173")]
        )
        .await
        .status,
        403
    );
    assert_eq!(
        get(production.address, "/admin/v1/me", &[("cookie", COOKIE)])
            .await
            .status,
        200,
        "no Origin header, no check"
    );
}

struct Credentials;

impl AdminCredentials for Credentials {
    fn verify_password(&self, hash: Option<&str>, password: &str) -> bool {
        hash == Some("hash") && password == "right"
    }
    fn verify_totp(&self, secret: &str, code: Option<&Value>) -> bool {
        code == Some(&json!(secret))
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn signing_in_makes_a_session_and_signing_out_ends_it() {
    let _turn = SERIAL.lock().await;
    let r = rig(RouteSet::V1147, &[]).await;
    // Sign-ins are limited per address, so each part of the test signs in from its own
    let login = |body: &'static str| {
        let address = r.address;
        async move {
            common::post_json(
                address,
                "/admin/v1/auth/login",
                &[("user-agent", "Browser/1"), ("x-forwarded-for", "203.0.113.10")],
                body,
            )
            .await
        }
    };
    set_credentials(None);
    assert_eq!(
        login(r#"{"email":"boss@example.test","password":"right"}"#)
            .await
            .json()["error"]["code"],
        "ADMIN_INVALID_CREDENTIALS"
    );
    set_credentials(Some(Arc::new(Credentials)));
    for body in [
        r#"{"email":"nobody@example.test","password":"right"}"#,
        r#"{"email":"boss@example.test","password":"wrong"}"#,
        r#"{"email":"fox@example.test","password":"right"}"#,
        r#"{}"#,
    ] {
        let refused = login(body).await;
        assert_eq!(refused.status, 401, "{body}");
        assert_eq!(
            refused.text(),
            r#"{"error":{"code":"ADMIN_INVALID_CREDENTIALS","message":"Administrator credentials are invalid."}}"#
        );
    }
    let ok = login(r#"{"email":" BOSS@example.test ","password":"right"}"#).await;
    assert_eq!(ok.status, 200);
    let cookie = ok.header("set-cookie").unwrap().to_owned();
    assert!(cookie.starts_with("mysticparadox_admin="), "{cookie}");
    assert!(
        cookie.ends_with("; Path=/admin/v1; HttpOnly; SameSite=Strict; Max-Age=28800"),
        "{cookie}"
    );
    let csrf = ok.json()["csrfToken"].as_str().unwrap().to_owned();
    assert_eq!(csrf.len(), 64);
    assert_eq!(ok.json()["account"]["displayName"], "Boss");

    let pair = cookie.split(';').next().unwrap().to_owned();
    assert_eq!(
        get(r.address, "/admin/v1/me", &[("cookie", pair.as_str())])
            .await
            .json()["csrfToken"],
        csrf.as_str()
    );
    let out = send(
        r.address,
        "POST",
        "/admin/v1/auth/logout",
        &[("cookie", pair.as_str()), ("x-csrf-token", csrf.as_str())],
        Some(("application/json", b"{}")),
    )
    .await;
    assert_eq!((out.status, out.text().as_str()), (200, "{}"));
    assert_eq!(
        out.header("set-cookie"),
        Some("mysticparadox_admin=; Path=/admin/v1; HttpOnly; SameSite=Strict; Max-Age=0")
    );
    assert_eq!(
        get(r.address, "/admin/v1/me", &[("cookie", pair.as_str())])
            .await
            .status,
        401
    );

    // MFA: with a secret the code must verify; in production a missing secret is a 503
    let mfa = rig(RouteSet::V1147, &[("ADMIN_TOTP_SECRET", "123456")]).await;
    let attempt = |totp: &str| {
        let body = json!({"email": "boss@example.test", "password": "right", "totp": totp}).to_string();
        let address = mfa.address;
        async move {
            common::post_json(
                address,
                "/admin/v1/auth/login",
                &[("x-forwarded-for", "203.0.113.11")],
                &body,
            )
            .await
        }
    };
    assert_eq!(attempt("000000").await.json()["error"]["code"], "ADMIN_MFA_INVALID");
    assert_eq!(attempt("123456").await.status, 200);
    let production = rig(RouteSet::V1147, &[("NODE_ENV", "production")]).await;
    let missing = common::post_json(
        production.address,
        "/admin/v1/auth/login",
        &[("x-forwarded-for", "203.0.113.12")],
        r#"{"email":"boss@example.test","password":"right"}"#,
    )
    .await;
    assert_eq!(missing.status, 503);
    assert_eq!(missing.json()["error"]["code"], "ADMIN_MFA_NOT_CONFIGURED");
    // Six tries in a quarter of an hour, then no more from that address
    for _ in 0..6 {
        common::post_json(
            production.address,
            "/admin/v1/auth/login",
            &[("x-forwarded-for", "203.0.113.13")],
            "{}",
        )
        .await;
    }
    let limited = common::post_json(
        production.address,
        "/admin/v1/auth/login",
        &[("x-forwarded-for", "203.0.113.13")],
        "{}",
    )
    .await;
    assert_eq!(limited.status, 429);
    assert_eq!(
        limited.text(),
        r#"{"error":{"code":"ADMIN_RATE_LIMITED","message":"Too many sign-in attempts."}}"#
    );
    set_credentials(None);
}

#[tokio::test(flavor = "multi_thread")]
async fn players_are_listed_and_their_access_changed_with_a_reason() {
    let r = rig(RouteSet::V1147, &[]).await;
    let cookie = [("cookie", COOKIE)];
    let list = get(r.address, "/admin/v1/players", &cookie).await.json();
    assert_eq!(
        (list["total"].clone(), list["page"].clone(), list["limit"].clone()),
        (json!(2), json!(1), json!(25))
    );
    assert_eq!(list["accounts"].as_array().unwrap().len(), 2);
    assert_eq!(
        list.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["accounts", "total", "page", "limit"]
    );
    let paged = get(r.address, "/admin/v1/players?page=2&limit=1&search=fox", &cookie)
        .await
        .json();
    assert_eq!(
        (
            paged["page"].clone(),
            paged["limit"].clone(),
            paged["accounts"].as_array().unwrap().len()
        ),
        (json!(2), json!(1), 0)
    );
    assert_eq!(
        get(r.address, "/admin/v1/players?limit=1000", &cookie).await.json()["limit"],
        100
    );
    assert_eq!(
        get(r.address, "/admin/v1/players?limit=abc&page=-4", &cookie)
            .await
            .json()["limit"],
        25
    );
    let ids = get(
        r.address,
        &format!("/admin/v1/players?ids={},nobody,bad%20id,{}", r.player.1, r.player.1),
        &cookie,
    )
    .await
    .json();
    assert_eq!(
        (ids["total"].clone(), ids["page"].clone(), ids["limit"].clone()),
        (json!(1), json!(1), json!(2))
    );
    assert_eq!(ids["accounts"][0]["userId"], r.player.1.as_str());
    let online = get(r.address, "/admin/v1/online-players", &cookie).await.json();
    assert_eq!(online["count"], 0);
    assert_eq!(online["source"], "xmpp+presence");

    let path = format!("/admin/v1/players/{}/access", r.player.1);
    let bad = |body: &str, code: &'static str, message: &'static str| {
        let body = body.to_owned();
        let path = path.clone();
        let r = &r;
        async move {
            let answer = patch(r, &path, &body).await;
            assert_eq!(answer.json()["error"]["code"], code, "{body}");
            assert_eq!(answer.json()["error"]["message"], message, "{body}");
        }
    };
    bad(
        r#"{"approvalStatus":"maybe","reason":"because"}"#,
        "ADMIN_VALIDATION",
        "Invalid approval status.",
    )
    .await;
    bad(
        r#"{"status":"gone","reason":"because"}"#,
        "ADMIN_VALIDATION",
        "Invalid account status.",
    )
    .await;
    bad(
        r#"{"reason":"because"}"#,
        "ADMIN_VALIDATION",
        "No access-state change was supplied.",
    )
    .await;
    bad(
        r#"{"status":"banned","reason":" ab "}"#,
        "ADMIN_ACCESS_REASON",
        "Provide a reason of at least 3 characters for every server access change.",
    )
    .await;
    let own = patch(
        &r,
        &format!("/admin/v1/players/{}/access", r.admin.1),
        r#"{"status":"banned","reason":"oops"}"#,
    )
    .await;
    assert_eq!(
        (own.status, own.json()["error"]["code"].clone()),
        (409, json!("ADMIN_SELF_LOCKOUT"))
    );
    let missing = patch(
        &r,
        "/admin/v1/players/nobody/access",
        r#"{"status":"banned","reason":"oops"}"#,
    )
    .await;
    assert_eq!(
        (missing.status, missing.text()),
        (
            404,
            r#"{"error":{"code":"NOT_FOUND","message":"Player not found."}}"#.to_owned()
        )
    );

    let banned = patch(&r, &path, r#"{"status":"banned","reason":"  cheating  "}"#).await;
    assert_eq!(banned.status, 200);
    let body = banned.json();
    assert_eq!(body["account"]["status"], "banned");
    assert_eq!(body["account"]["approvalReason"], "cheating");
    assert_eq!(body["account"]["approvalUpdatedBy"], r.admin.1.as_str());
    let request_id = body["requestId"].as_str().unwrap().to_owned();
    assert_eq!(
        r.fakes.world().refresh_revoked.len(),
        1,
        "a player who can't sign in loses their tokens"
    );
    assert_eq!(r.fakes.world().codes_revoked.len(), 1);

    let audit = get(
        r.address,
        &format!("/admin/v1/audit?targetUserId={}", r.player.1),
        &cookie,
    )
    .await
    .json();
    assert_eq!(audit["total"], 1);
    let record = &audit["records"][0];
    assert_eq!(record["action"], "player.access.update");
    assert_eq!(record["requestId"], request_id.as_str());
    assert_eq!(record["actorUserId"], r.admin.1.as_str());
    assert_eq!(
        record["oldState"],
        json!({"approvalStatus": "approved", "status": "active"})
    );
    assert_eq!(
        record["newState"],
        json!({"approvalStatus": "approved", "status": "banned"})
    );
    assert_eq!(record["reason"], "cheating");
    assert_eq!(
        record.as_object().unwrap().keys().collect::<Vec<_>>(),
        [
            "id",
            "actorUserId",
            "targetUserId",
            "action",
            "oldState",
            "newState",
            "reason",
            "ip",
            "requestId",
            "createdAt"
        ]
    );

    // Roles: only the assignable ones, the rest kept
    let roles = format!("/admin/v1/players/{}/roles", r.player.1);
    assert_eq!(
        patch(&r, &roles, r#"{"roles":"tester"}"#).await.json()["error"]["message"],
        "roles must be an array of strings."
    );
    assert_eq!(
        patch(&r, &roles, r#"{"roles":["admin"]}"#).await.json()["error"]["message"],
        "Only these roles can be toggled here: tester."
    );
    let granted = patch(&r, &roles, r#"{"roles":["tester"],"reason":"testing"}"#)
        .await
        .json();
    assert_eq!(granted["account"]["roles"], json!(["player", "tester"]));
    assert_eq!(granted["account"]["rolesUpdatedBy"], r.admin.1.as_str());
    let revoked = patch(&r, &roles, r#"{"roles":[]}"#).await.json();
    assert_eq!(revoked["account"]["roles"], json!(["player"]));
    assert_eq!(
        r.fakes.world().codes_revoked.len(),
        2,
        "removing Tester revokes the exchange codes"
    );
    let all = get(r.address, "/admin/v1/audit", &cookie).await.json();
    assert_eq!(all["total"], 3);
    assert_eq!(all["records"][0]["action"], "player.roles.update", "newest first");
    assert!(all["records"][0].get("reason").is_none(), "an empty reason is left out");
    assert_eq!(
        get(r.address, "/admin/v1/audit?limit=1&page=3", &cookie).await.json()["records"][0]["action"],
        "player.access.update"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn deleting_a_player_asks_for_the_name_and_a_reason() {
    let r = rig(RouteSet::V1147, &[]).await;
    let path = format!("/admin/v1/players/{}", r.player.1);
    let delete = |body: &'static str| {
        let (address, path) = (r.address, path.clone());
        async move {
            send(
                address,
                "DELETE",
                &path,
                &session_headers(),
                Some(("application/json", body.as_bytes())),
            )
            .await
        }
    };
    let own = send(
        r.address,
        "DELETE",
        &format!("/admin/v1/players/{}", r.admin.1),
        &session_headers(),
        Some(("application/json", b"{}")),
    )
    .await;
    assert_eq!(own.json()["error"]["code"], "ADMIN_SELF_DELETE");
    assert_eq!(
        send(
            r.address,
            "DELETE",
            "/admin/v1/players/nobody",
            &session_headers(),
            Some(("application/json", b"{}"))
        )
        .await
        .status,
        404
    );
    let unconfirmed = delete(r#"{"confirmation":"DELETE fox_9","reason":"cleanup"}"#).await;
    assert_eq!(unconfirmed.json()["error"]["code"], "ADMIN_DELETE_CONFIRMATION");
    assert_eq!(
        unconfirmed.json()["error"]["message"],
        "Type \"DELETE Fox_9\" to confirm permanent deletion."
    );
    assert_eq!(
        delete(r#"{"confirmation":"DELETE Fox_9","reason":"x"}"#).await.json()["error"]["code"],
        "ADMIN_DELETE_REASON"
    );
    // The account the store holds is soft-deleted (DD23), so the real accounts table is needed for the count
    let done = delete(r#"{"confirmation":"DELETE Fox_9","reason":"cleanup"}"#).await;
    assert_eq!(done.status, 200, "{}", done.text());
    let body = done.json();
    assert_eq!(
        body.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["requestId", "deletedCounts", "characterCount"]
    );
    assert_eq!(body["deletedCounts"]["adminSessions"], 0);
    let audit = get(r.address, "/admin/v1/audit", &[("cookie", COOKIE)]).await.json();
    assert_eq!(audit["records"][0]["action"], "player.account.delete");
    assert_eq!(audit["records"][0]["newState"], json!({"deleted": true}));
    assert_eq!(r.fakes.world().refresh_revoked.len(), 0);
    let _ = &r.store;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_launcher_notices_are_saved_by_version() {
    let r = rig(RouteSet::V1147, &[]).await;
    let empty = get(r.address, "/admin/v1/launcher-notices", &[("cookie", COOKIE)])
        .await
        .json();
    assert_eq!(
        empty.as_object().unwrap().keys().collect::<Vec<_>>(),
        [
            "version",
            "updatedAt",
            "updatedBy",
            "announcements",
            "tags",
            "serverTime"
        ]
    );
    assert_eq!(
        (empty["version"].clone(), empty["updatedAt"].clone()),
        (json!(0), Value::Null)
    );
    let put = |body: Value| {
        let (address, text) = (r.address, body.to_string());
        async move {
            send(
                address,
                "PUT",
                "/admin/v1/launcher-notices",
                &session_headers(),
                Some(("application/json", text.as_bytes())),
            )
            .await
        }
    };
    let invalid = put(json!({"announcements": [{"text": "<b>", "tone": "info"}], "tags": [], "reason": "a reason", "expectedVersion": 0})).await;
    assert_eq!(invalid.status, 400);
    assert_eq!(invalid.json()["error"]["code"], "NOTICES_INVALID");
    assert_eq!(
        invalid.json()["error"]["message"],
        "announcements[0].text must be plain text (no < or > characters)."
    );
    assert_eq!(
        put(json!({"announcements": [], "tags": [], "expectedVersion": 0}))
            .await
            .json()["error"]["message"],
        "A change reason of 8 to 200 characters is required."
    );
    assert_eq!(
        put(json!({"announcements": [], "tags": [], "reason": "a reason", "expectedVersion": "0"}))
            .await
            .json()["error"]["message"],
        "expectedVersion must be the version you loaded (a whole number, 0 or more)."
    );

    let saved = put(json!({
        "announcements": [{"text": "Maintenance at {time}", "tone": "warning", "eventAt": "2026-10-30T12:00:00Z"}],
        "tags": [{"id": "t1", "text": "NEW", "tone": "success"}],
        "reason": "announcing it", "expectedVersion": 0
    }))
    .await;
    assert_eq!(saved.status, 200, "{}", saved.text());
    let view = saved.json();
    assert_eq!(
        (view["version"].clone(), view["updatedBy"].clone()),
        (json!(1), json!(r.admin.1))
    );
    assert_eq!(view["announcements"][0]["eventAt"], "2026-10-30T12:00:00.000Z");
    assert_eq!(
        view["announcements"][0].as_object().unwrap().keys().collect::<Vec<_>>(),
        ["text", "tone", "eventAt", "id"]
    );
    assert_eq!(
        get(r.address, "/admin/v1/launcher-notices", &[("cookie", COOKIE)])
            .await
            .json()["tags"][0]["id"],
        "t1"
    );

    // Someone else saved first
    let stale = put(json!({"announcements": [], "tags": [], "reason": "a second one", "expectedVersion": 0})).await;
    assert_eq!(stale.status, 409);
    let error = stale.json()["error"].clone();
    assert_eq!(error["code"], "NOTICES_VERSION_CONFLICT");
    assert_eq!(error["current"]["version"], 1);
    assert_eq!(
        error.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["current", "code", "message", "requestId"]
    );
    let next = put(json!({"announcements": [], "tags": [], "reason": "clearing them", "expectedVersion": 1})).await;
    assert_eq!(next.json()["version"], 2);
    let audit = get(r.address, "/admin/v1/audit", &[("cookie", COOKIE)]).await.json();
    assert_eq!(audit["total"], 2);
    assert_eq!(audit["records"][0]["action"], "launcher_notices.update");
    assert_eq!(audit["records"][0]["reason"], "clearing them");
    assert_eq!(audit["records"][0]["oldState"]["version"], 1);
    assert!(audit["records"][0].get("targetUserId").is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_admin_api_switched_off_is_a_404() {
    let off = rig(RouteSet::V1147, &[("MYSTICPARADOX_ADMIN_API", "off")]).await;
    assert_eq!(
        get(off.address, "/admin/v1/me", &[("cookie", COOKIE)]).await.status,
        404
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_guard_views_leave_out_the_secrets() {
    use isles_core::repo::reforged::{GuardRole, GuardSighting, GuardStatus, LauncherGuardSessionRecord};
    let r = rig(RouteSet::V1147, &[]).await;
    let cookie = [("cookie", COOKIE)];
    let record = LauncherGuardSessionRecord {
        guard_session_id: "11111111-2222-3333-4444-555555555555".into(),
        account: r.player.0.clone(),
        role: GuardRole::Client,
        public_key_base64: "PUBLIC".into(),
        manifest_id: "m".into(),
        manifest_sequence: 3,
        channel: "stable".into(),
        launcher_version: "0.1.70".into(),
        challenge: "SECRET-CHALLENGE".into(),
        last_sequence: 2,
        status: GuardStatus::Degraded,
        risk_score: 65.0,
        created_at: "2026-10-20T00:00:00.000Z".into(),
        expires_at: "2026-10-20T00:05:00.000Z".into(),
        last_heartbeat_at: Some("2026-10-20T00:01:00.000Z".into()),
        last_signals: Some(json!({"gameProcessId": 4})),
        reasons: vec!["DEBUGGER".into()],
        history: vec![json!({"at": "t", "status": "DEGRADED", "riskScore": 65, "reasons": ["DEBUGGER"]})],
    };
    let account = r.player.0.clone();
    let sighting = GuardSighting {
        file_name: "overlay.dll".into(),
        sha256: "a".repeat(64),
        signer: "unsigned".into(),
        size: 9,
        account: r.player.0.clone(),
        seen_at: "2026-10-20T00:01:00.000Z".into(),
        new_in_session: true,
    };
    r.store
        .account_write(&account, &ActorContext::new(Actor::System), move |tx, repos| {
            repos.launcher_guard.create(tx, &record)?;
            repos.launcher_guard.record_finding_sighting(tx, &sighting)
        })
        .await
        .unwrap();
    let list = get(
        r.address,
        "/admin/v1/guard/sessions?status=DEGRADED&minRisk=50",
        &cookie,
    )
    .await;
    assert_eq!(list.status, 200);
    let body = list.json();
    assert_eq!(
        (body["total"].clone(), body["page"].clone(), body["limit"].clone()),
        (json!(1), json!(1), json!(25))
    );
    let session = &body["sessions"][0];
    assert_eq!(
        session.as_object().unwrap().keys().collect::<Vec<_>>(),
        [
            "guardSessionId",
            "accountId",
            "role",
            "status",
            "riskScore",
            "reasons",
            "channel",
            "launcherVersion",
            "manifestId",
            "manifestSequence",
            "lastSequence",
            "createdAt",
            "lastHeartbeatAt",
            "expiresAt",
            "lastSignals"
        ]
    );
    assert_eq!(session["accountId"], r.player.1.as_str());
    assert!(!list.text().contains("SECRET-CHALLENGE") && !list.text().contains("PUBLIC"));
    assert_eq!(
        get(r.address, "/admin/v1/guard/sessions?status=HEALTHY", &cookie)
            .await
            .json()["total"],
        0
    );
    for (query, message) in [
        (
            "status=nope",
            "status must be one of CHALLENGED, HEALTHY, DEGRADED, REVOKED, EXPIRED",
        ),
        ("minRisk=101", "minRisk must be a whole number from 0 to 100"),
        ("minRisk=1.5", "minRisk must be a whole number from 0 to 100"),
        ("minRisk=abc", "minRisk must be a whole number from 0 to 100"),
    ] {
        let bad = get(r.address, &format!("/admin/v1/guard/sessions?{query}"), &cookie).await;
        assert_eq!(
            (
                bad.status,
                bad.json()["error"]["code"].clone(),
                bad.json()["error"]["message"].clone()
            ),
            (400, json!("GUARD_QUERY_INVALID"), json!(message))
        );
    }
    let one = get(
        r.address,
        "/admin/v1/guard/sessions/11111111-2222-3333-4444-555555555555",
        &cookie,
    )
    .await
    .json();
    assert_eq!(one["history"][0]["riskScore"], 65);
    assert_eq!(one["session"]["lastSignals"], json!({"gameProcessId": 4}));
    let missing = get(r.address, "/admin/v1/guard/sessions/nothing", &cookie).await;
    assert_eq!(
        missing.text(),
        r#"{"error":{"code":"GUARD_SESSION_NOT_FOUND","message":"No Guard session has that id."}}"#
    );
    let findings = get(r.address, "/admin/v1/guard/findings?signer=%20UNSIG%20", &cookie)
        .await
        .json();
    assert_eq!(findings["total"], 1);
    assert_eq!(
        findings["findings"][0].as_object().unwrap().keys().collect::<Vec<_>>(),
        [
            "fileName",
            "sha256",
            "signer",
            "size",
            "firstSeenAt",
            "lastSeenAt",
            "sessionCount",
            "accountCount"
        ]
    );
    assert_eq!(
        get(r.address, "/admin/v1/guard/findings?signer=zzz", &cookie)
            .await
            .json()["findings"],
        json!([])
    );
    assert_eq!(get(r.address, "/admin/v1/guard/sessions", &[]).await.status, 401);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_testers_uploads_are_listed_read_and_deleted_with_an_audit() {
    let logs = tempfile::tempdir().unwrap();
    let root = logs.path().display().to_string();
    let r = rig(RouteSet::V1147, &[("LAUNCHER_LOG_STORAGE_DIR", root.as_str())]).await;
    let session = "abcdef12-3456-7890";
    let dir = logs.path().join("user-1").join(session);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("launcher.log"), "log text").unwrap();
    std::fs::write(dir.join("metadata.json"), "{}").unwrap();
    std::fs::write(
        dir.join(".uploaded-at.json"),
        "{\"lastUploadedAt\":\"2026-10-20T10:00:00.123Z\"}\n",
    )
    .unwrap();
    let cookie = [("cookie", COOKIE)];

    let list = get(r.address, "/admin/v1/player-logs", &cookie).await.json();
    assert_eq!(list["nextBefore"], Value::Null);
    assert_eq!(list["sessions"][0]["uploadedAt"], "2026-10-20T10:00:00.123Z");
    assert_eq!(list["sessions"][0]["totalBytes"], 10);
    for (query, message) in [
        ("userId=a/b", "userId must be one account id."),
        ("userId=a&userId=b", "userId must be one account id."),
        ("limit=0", "limit must be a whole number from 1 to 200."),
        ("limit=201", "limit must be a whole number from 1 to 200."),
        ("limit=1.5", "limit must be a whole number from 1 to 200."),
        (
            "before=yesterday",
            "before must be a UTC time such as 2026-10-15T12:00:00.000Z.",
        ),
        (
            "before=2026-10-15",
            "before must be a UTC time such as 2026-10-15T12:00:00.000Z.",
        ),
    ] {
        let bad = get(r.address, &format!("/admin/v1/player-logs?{query}"), &cookie).await;
        assert_eq!(
            (
                bad.status,
                bad.json()["error"]["code"].clone(),
                bad.json()["error"]["message"].clone()
            ),
            (400, json!("PLAYER_LOGS_QUERY"), json!(message)),
            "{query}"
        );
    }
    assert_eq!(
        get(
            r.address,
            "/admin/v1/player-logs?before=2026-10-20T10:00:00.123Z&limit=5",
            &cookie
        )
        .await
        .json()["sessions"],
        json!([])
    );

    let file = get(
        r.address,
        &format!("/admin/v1/player-logs/user-1/{session}/launcher.log?download=1"),
        &cookie,
    )
    .await;
    assert_eq!((file.status, file.text().as_str()), (200, "log text"));
    assert_eq!(file.header("content-type"), Some("text/plain; charset=utf-8"));
    assert_eq!(file.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(
        file.header("content-disposition"),
        Some("attachment; filename=\"launcher.log\"")
    );
    let inline = get(
        r.address,
        &format!("/admin/v1/player-logs/user-1/{session}/metadata.json"),
        &cookie,
    )
    .await;
    assert_eq!(inline.header("content-type"), Some("application/json; charset=utf-8"));
    assert!(inline.header("content-disposition").is_none());
    let bad = get(
        r.address,
        &format!("/admin/v1/player-logs/user-1/{session}/launcher.log?download=2"),
        &cookie,
    )
    .await;
    assert_eq!(bad.json()["error"]["message"], "download must be 1 or 0.");
    let refused = get(
        r.address,
        &format!("/admin/v1/player-logs/user-1/{session}/secrets.txt"),
        &cookie,
    )
    .await;
    assert_eq!(refused.json()["error"]["code"], "PLAYER_LOG_NAME_REFUSED");
    assert_eq!(
        get(
            r.address,
            &format!("/admin/v1/player-logs/user-2/{session}/launcher.log"),
            &cookie
        )
        .await
        .status,
        404
    );

    let delete = |body: &'static str| {
        let path = format!("/admin/v1/player-logs/user-1/{session}");
        let address = r.address;
        async move {
            send(
                address,
                "DELETE",
                &path,
                &session_headers(),
                Some(("application/json", body.as_bytes())),
            )
            .await
        }
    };
    let short = delete(r#"{"reason":"short"}"#).await;
    assert_eq!(
        short.json()["error"]["message"],
        "Give a reason of 8 to 200 characters for the audit log."
    );
    assert!(dir.exists());
    let done = delete(r#"{"reason":"asked to remove them"}"#).await;
    assert_eq!(done.status, 200);
    let body = done.json();
    assert_eq!(
        body.as_object().unwrap().keys().collect::<Vec<_>>(),
        [
            "deleted",
            "requestId",
            "userId",
            "launchSessionId",
            "files",
            "totalBytes"
        ]
    );
    assert_eq!(
        (body["files"].clone(), body["totalBytes"].clone()),
        (json!(2), json!(10))
    );
    assert!(!dir.exists());
    let audit = get(r.address, "/admin/v1/audit?targetUserId=user-1", &cookie)
        .await
        .json();
    assert_eq!(audit["records"][0]["action"], "player_logs.delete");
    assert_eq!(audit["records"][0]["oldState"]["files"][0]["name"], "launcher.log");
    assert_eq!(delete(r#"{"reason":"asked to remove them"}"#).await.status, 404);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_operations_policy_changes_by_version_and_weakening_needs_a_fresh_code() {
    let _turn = SERIAL.lock().await;
    set_credentials(Some(Arc::new(Credentials)));
    let r = rig(RouteSet::V1147, &[("ADMIN_OPERATIONS_TOTP_SECRET", "654321")]).await;
    let cookie = [("cookie", COOKIE)];
    let overview = get(r.address, "/admin/v1/operations/overview", &cookie).await.json();
    assert_eq!(
        overview["policy"],
        json!({"guardEnforcement": "ENFORCE", "diagnosticsProfile": "PRODUCTION", "version": 0})
    );
    assert_eq!(overview["launcherGuard"], json!([]));
    assert_eq!(
        overview.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["observedAt", "policy", "launcherGuard"]
    );
    let patch_policy = |body: Value| {
        let (address, text) = (r.address, body.to_string());
        async move {
            send(
                address,
                "PATCH",
                "/admin/v1/operations/policy",
                &session_headers(),
                Some(("application/json", text.as_bytes())),
            )
            .await
        }
    };
    let invalid = patch_policy(json!({"guardEnforcement": "ENFORCE", "diagnosticsProfile": "PRODUCTION", "expectedVersion": 0, "reason": "short"})).await;
    assert_eq!(invalid.status, 400);
    assert_eq!(invalid.json()["error"]["code"], "ADMIN_VALIDATION");
    for body in [
        json!({"guardEnforcement": "MAYBE", "diagnosticsProfile": "PRODUCTION", "expectedVersion": 0, "reason": "long enough"}),
        json!({"guardEnforcement": "ENFORCE", "diagnosticsProfile": "PRODUCTION", "expectedVersion": -1, "reason": "long enough"}),
        json!({"guardEnforcement": "ENFORCE", "diagnosticsProfile": "PRODUCTION", "expectedVersion": 1.5, "reason": "long enough"}),
        json!({"guardEnforcement": "ENFORCE", "diagnosticsProfile": "PRODUCTION", "reason": "long enough"}),
    ] {
        assert_eq!(patch_policy(body).await.status, 400);
    }

    // A weakening without a code is refused, and the refusal is audited
    let weak = json!({"guardEnforcement": "OBSERVE", "diagnosticsProfile": "DEVELOPMENT", "expectedVersion": 0, "reason": "debugging a report"});
    let refused = patch_policy(weak.clone()).await;
    assert_eq!(refused.status, 403);
    let body = refused.json();
    assert_eq!(body["error"]["code"], "ADMIN_POLICY_STEP_UP_REQUIRED");
    assert_eq!(
        body["weakeningCodes"],
        json!(["GUARD_DOWNGRADE", "DEVELOPMENT_DIAGNOSTICS"])
    );
    assert_eq!(body["policy"]["version"], 0);
    let mut wrong = weak.clone();
    wrong["breakGlassTotp"] = json!("000000");
    assert_eq!(patch_policy(wrong).await.status, 403);
    let mut with_code = weak.clone();
    with_code["breakGlassTotp"] = json!("654321");
    let done = patch_policy(with_code).await;
    assert_eq!(done.status, 200, "{}", done.text());
    let policy = done.json()["policy"].clone();
    assert_eq!(
        (
            policy["guardEnforcement"].clone(),
            policy["diagnosticsProfile"].clone(),
            policy["version"].clone()
        ),
        (json!("OBSERVE"), json!("DEVELOPMENT"), json!(1))
    );
    assert_eq!(policy["updatedBy"], r.admin.1.as_str());
    assert!(policy["diagnosticsExpiresAt"].as_str().is_some());

    // The same version again is a conflict that carries the current policy
    let stale = patch_policy(weak).await;
    assert_eq!(stale.status, 409);
    assert_eq!(stale.json()["error"]["code"], "ADMIN_POLICY_CONFLICT");
    assert_eq!(stale.json()["policy"]["version"], 1);
    // Tightening needs no code
    let tight = patch_policy(json!({"guardEnforcement": "ENFORCE", "diagnosticsProfile": "PRODUCTION", "expectedVersion": 1, "reason": "back to normal"})).await;
    assert_eq!(tight.status, 200);
    assert_eq!(tight.json()["policy"]["version"], 2);
    assert!(tight.json()["policy"].get("diagnosticsExpiresAt").is_none());
    let audit = get(r.address, "/admin/v1/audit", &cookie).await.json();
    let actions: Vec<_> = audit["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["action"].as_str().unwrap())
        .collect();
    assert_eq!(
        actions,
        [
            "operations.policy.update",
            "operations.policy.update",
            "operations.policy.weakening_rejected",
            "operations.policy.weakening_rejected"
        ]
    );
    assert_eq!(
        audit["records"][2]["newState"]["weakeningCodes"],
        json!(["GUARD_DOWNGRADE", "DEVELOPMENT_DIAGNOSTICS"])
    );
    set_credentials(None);
}

#[tokio::test(flavor = "multi_thread")]
async fn activity_overview_player_summary_and_flags_work_and_acknowledge_audits() {
    use isles_core::repo::reforged::{
        ActivityFlagRecord, ActivityHuntRecord, HuntKind, HuntResult, SlayersPathUnlockRecord,
    };

    let r = rig(RouteSet::V1147, &[]).await;
    let cookie = [("cookie", COOKIE)];
    let player_acc = r.player.0.clone();
    let player_uid = r.player.1.clone();
    let today = isles_core::areas::reforged_api::activity::day_of(now_ms());
    let ttl = now_ms() + 86_400_000;
    let now_iso = iso(now_ms());
    let flag_id = format!("UNOFFICIAL_LAUNCHER:{player_uid}:{today}");
    let flag_id_in = flag_id.clone();
    let today_in = today.clone();
    let now_in = now_iso.clone();
    let acc_in = player_acc.clone();
    let uid_in = player_uid.clone();
    r.store
        .account_write(&player_acc, &ActorContext::new(Actor::System), move |tx, repos| {
            repos.activity.add_to_daily(
                tx,
                &acc_in,
                &today_in,
                &[
                    ("playtimeMinutes".into(), 45.0),
                    ("sessions".into(), 2.0),
                    ("huntsStarted.dedicated".into(), 3.0),
                    ("huntsCompleted.dedicated".into(), 2.0),
                    ("xp.PlayerSlayerLevel".into(), 1200.0),
                ],
                ttl,
            )?;
            repos.activity.insert_hunt_if_absent(
                tx,
                &ActivityHuntRecord {
                    id: "dedicated:ref-1:player".into(),
                    account: acc_in.clone(),
                    host: false,
                    kind: HuntKind::Dedicated,
                    hunt_id: "Island_A".into(),
                    hunt_ref: "ref-1".into(),
                    started_at: now_in.clone(),
                    ended_at: Some(now_in.clone()),
                    end_reason: Some("ALL_GONE".into()),
                    result: Some(HuntResult::Completed),
                    kills: 2,
                    ttl_at: ttl,
                },
            )?;
            repos.activity.insert_unlock_if_absent(
                tx,
                &SlayersPathUnlockRecord {
                    id: format!("{uid_in}:Node_1"),
                    account: acc_in.clone(),
                    node_id: "Node_1".into(),
                    unlocked_at: now_in.clone(),
                    source: "gameserver".into(),
                    ttl_at: ttl,
                },
            )?;
            repos.activity.insert_flag_if_absent(
                tx,
                &ActivityFlagRecord {
                    id: flag_id_in,
                    kind: "UNOFFICIAL_LAUNCHER".into(),
                    severity: "high".into(),
                    account: acc_in,
                    day: today_in,
                    detail: "unofficial build".into(),
                    acknowledged: false,
                    created_at: now_in,
                    acknowledged_at: None,
                    acknowledged_by: None,
                    note: None,
                    ttl_at: ttl,
                },
            )?;
            Ok(())
        })
        .await
        .unwrap();

    let overview = get(r.address, "/admin/v1/activity/overview", &cookie).await;
    assert_eq!(overview.status, 200, "{}", overview.text());
    let ov_json = overview.json();
    assert_eq!(ov_json["monitorEnabled"], true);
    assert_eq!(ov_json["truncated"], false);
    assert!(!ov_json["days"].as_array().unwrap().is_empty());

    let summary = get(r.address, &format!("/admin/v1/activity/players/{player_uid}"), &cookie).await;
    assert_eq!(summary.status, 200, "{}", summary.text());
    let sum_json = summary.json();
    assert_eq!(sum_json["userId"], player_uid);
    assert_eq!(sum_json["totals"]["sessions"], 2.0);
    assert_eq!(sum_json["totals"]["huntsCompleted"]["dedicated"], 2.0);
    assert_eq!(sum_json["hunts"].as_array().unwrap().len(), 1);
    assert_eq!(sum_json["unlocks"].as_array().unwrap().len(), 1);
    assert_eq!(sum_json["flags"].as_array().unwrap().len(), 1);

    let flags = get(r.address, "/admin/v1/activity/flags?status=open", &cookie).await;
    assert_eq!(flags.status, 200);
    assert_eq!(flags.json()["total"], 1);

    let encoded_flag = flag_id.replace(':', "%3A");
    let ack = patch(
        &r,
        &format!("/admin/v1/activity/flags/{encoded_flag}"),
        r#"{"acknowledged":true,"note":"reviewed by operator"}"#,
    )
    .await;
    assert_eq!(ack.status, 200, "{}", ack.text());
    let ack_json = ack.json();
    assert_eq!(ack_json["flag"]["status"], "acknowledged");
    assert_eq!(ack_json["flag"]["note"], "reviewed by operator");
    assert_eq!(ack_json["flag"]["acknowledgedBy"], r.admin.1);

    let open_after = get(r.address, "/admin/v1/activity/flags?status=open", &cookie)
        .await
        .json();
    assert_eq!(open_after["total"], 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn discord_admin_auth_start_and_callback_enforce_config_and_pkce_binding() {
    let unconfigured = rig(RouteSet::V1147, &[]).await;
    let start_unconf = get(unconfigured.address, "/admin/v1/auth/discord/start", &[]).await;
    assert_eq!(start_unconf.status, 503);
    assert_eq!(start_unconf.json()["error"]["code"], "ADMIN_DISCORD_NOT_CONFIGURED");

    let configured = rig(
        RouteSet::V1147,
        &[
            ("DISCORD_CLIENT_ID", "client-123"),
            ("DISCORD_CLIENT_SECRET", "secret-456"),
            ("ADMIN_FRONTEND_URL", "http://localhost:4173/admin"),
            (
                "ADMIN_DISCORD_REDIRECT_URI",
                "http://localhost:4173/admin/v1/auth/discord/callback",
            ),
            ("ADMIN_DISCORD_ALLOWED_IDS", "100000000000000001"),
        ],
    )
    .await;
    let start = get(configured.address, "/admin/v1/auth/discord/start", &[]).await;
    assert_eq!(start.status, 302);
    let loc = start.header("location").unwrap();
    assert!(loc.starts_with("https://discord.com/api/oauth2/authorize?"));
    assert!(loc.contains("client_id=client-123"));
    assert!(loc.contains("code_challenge_method=S256"));
    let set_cookie = start.header("set-cookie").unwrap();
    assert!(set_cookie.starts_with("mysticparadox_admin_oauth="));

    let cb_denied = get(
        configured.address,
        "/admin/v1/auth/discord/callback?error=access_denied",
        &[],
    )
    .await;
    assert_eq!(cb_denied.status, 302);
    assert_eq!(
        cb_denied.header("location"),
        Some("http://localhost:4173/admin/?error=discord_cancelled")
    );

    let cb_invalid_state = get(
        configured.address,
        "/admin/v1/auth/discord/callback?code=abc&state=bad",
        &[],
    )
    .await;
    assert_eq!(cb_invalid_state.status, 302);
    assert_eq!(
        cb_invalid_state.header("location"),
        Some("http://localhost:4173/admin/?error=discord_expired")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn tester_max_requires_tester_role_confirmation_and_run_id() {
    let r = rig(RouteSet::V1147, &[]).await;
    let headers = session_headers();
    let player_uid = r.player.1.clone();

    let not_tester = common::post_json(
        r.address,
        &format!("/admin/v1/players/{player_uid}/tester-max"),
        &headers,
        r#"{"confirm":"MAX Fox_9","runId":"run_0001"}"#,
    )
    .await;
    assert_eq!(not_tester.status, 409);
    assert_eq!(not_tester.json()["error"]["code"], "ADMIN_TESTER_REQUIRED");

    r.fakes
        .world()
        .launchers
        .iter_mut()
        .find(|a| a.account == r.player.0)
        .unwrap()
        .roles = vec!["player".into(), "tester".into()];

    let bad_confirm = common::post_json(
        r.address,
        &format!("/admin/v1/players/{player_uid}/tester-max"),
        &headers,
        r#"{"confirm":"MAX fox_9","runId":"run_0001"}"#,
    )
    .await;
    assert_eq!(bad_confirm.status, 400);
    assert_eq!(bad_confirm.json()["error"]["code"], "ADMIN_TESTER_MAX_CONFIRMATION");

    let bad_run_id = common::post_json(
        r.address,
        &format!("/admin/v1/players/{player_uid}/tester-max"),
        &headers,
        r#"{"confirm":"MAX Fox_9","runId":"short"}"#,
    )
    .await;
    assert_eq!(bad_run_id.status, 400);
    assert_eq!(bad_run_id.json()["error"]["code"], "ADMIN_TESTER_MAX_RUN_ID");

    let missing_kit = common::post_json(
        r.address,
        &format!("/admin/v1/players/{player_uid}/tester-max"),
        &headers,
        r#"{"confirm":"MAX Fox_9","runId":"run_0001_valid"}"#,
    )
    .await;
    assert_eq!(missing_kit.status, 503);
    assert_eq!(missing_kit.json()["error"]["code"], "ADMIN_TESTER_MAX_KIT_MISSING");
}
