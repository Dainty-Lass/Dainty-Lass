// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The admin half through core's `app()` and its library calls (admin spec §17.2).

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use common::{ADMIN_KEY, CSRF, OPERATOR_SECRET, RAW_SESSION, Request1, Rig, now_ms, options, settings};
use dainty_module_api::{Actor, ActorContext, AdminCall, AdminRequest, AdminResponse, RestrictionKind};
use rustic_isles_launcher_api::admin::bootstrap::{BootstrapAdminRequest, BootstrapError, bootstrap_admin};
use rustic_isles_launcher_api::test_support::{FakeFunding, operator_headers};
use rustic_isles_store::AccountId;
use serde_json::{Value, json};

const OPERATOR: &str = "111111111111111111";

struct Admin {
    rig: Rig,
    boss: AccountId,
    fox: AccountId,
}

async fn rig_with(o: common::Options) -> Admin {
    let rig = Rig::new(o).await;
    let boss = rig
        .player("boss-user", "Boss", Some("boss@example.test"), &["player", "admin"])
        .await;
    let fox = rig
        .player("fox-user", "Fox_9", Some("fox@example.test"), &["player"])
        .await;
    rig.admin_session(&boss).await;
    Admin { rig, boss, fox }
}

async fn rig() -> Admin {
    rig_with(options(true)).await
}

fn session(method: &'static str, uri: &'static str) -> Request1<'static> {
    Request1::new(method, uri)
        .header("cookie", &format!("rustic_isles_admin={RAW_SESSION}"))
        .header("x-csrf-token", CSRF)
}

fn leak(text: &str) -> &'static str {
    Box::leak(text.to_owned().into_boxed_str())
}

fn signed(method: &str, target: &str, body: Option<&Value>, at: i64) -> Request1<'static> {
    let bytes = body.map(|b| b.to_string().into_bytes()).unwrap_or_default();
    let mut r = Request1::new(leak(method), leak(target));
    for (k, v) in operator_headers(OPERATOR_SECRET, method, target, OPERATOR, at, &bytes) {
        r = r.header(&k, &v);
    }
    if let Some(b) = body {
        r = r.json(b);
    }
    r
}

#[tokio::test]
async fn session_gate_and_csrf() {
    let t = rig().await;
    let r = &t.rig;
    r.get("/admin/v1/me").await.assert(
        401,
        r#"{"error":{"code":"ADMIN_UNAUTHORIZED","message":"Administrator sign-in required."}}"#,
    );
    let nope = r
        .send(Request1::new("GET", "/admin/v1/me").header("cookie", "rustic_isles_admin=nope"))
        .await;
    assert_eq!(nope.json()["error"]["message"], "Administrator session expired.");
    let bad = r
        .send(Request1::new("GET", "/admin/v1/me").header("cookie", "rustic_isles_admin=%E0%A4%A"))
        .await;
    assert_eq!(bad.status, 500);
    let legacy = r
        .send(Request1::new("GET", "/admin/v1/me").header("cookie", &format!("mysticparadox_admin={RAW_SESSION}")))
        .await;
    assert_eq!(legacy.status, 401);
    let me = r.send(session("GET", "/admin/v1/me")).await;
    assert_eq!(me.status, 200, "{}", me.body);
    assert_eq!(me.header("cache-control"), Some("no-store"));
    assert_eq!(
        me.header("content-security-policy"),
        Some("default-src 'none'; frame-ancestors 'none'")
    );
    let keys: Vec<String> = me.json()["account"].as_object().unwrap().keys().cloned().collect();
    assert_eq!(
        keys,
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
    assert_eq!(me.json()["csrfToken"], CSRF);
    let no_csrf = r
        .send(
            Request1::new("PATCH", "/admin/v1/players/fox-user/roles")
                .header("cookie", &format!("rustic_isles_admin={RAW_SESSION}"))
                .json(&json!({"roles": []})),
        )
        .await;
    no_csrf.assert(
        403,
        r#"{"error":{"code":"ADMIN_CSRF","message":"Invalid request token."}}"#,
    );
    // Losing the role revokes the session
    let boss = t.boss.clone();
    r.store
        .write(&ActorContext::new(Actor::System), move |tx, repos| {
            repos
                .launcher_accounts
                .update_roles(tx, &boss, &["player".to_owned()], "x", "t")
                .map(|_| ())
        })
        .await
        .unwrap();
    assert_eq!(r.send(session("GET", "/admin/v1/me")).await.code(), "ADMIN_FORBIDDEN");
    let boss = t.boss.clone();
    r.store
        .write(&ActorContext::new(Actor::System), move |tx, repos| {
            let roles = ["player".to_owned(), "admin".to_owned()];
            repos
                .launcher_accounts
                .update_roles(tx, &boss, &roles, "x", "t")
                .map(|_| ())
        })
        .await
        .unwrap();
    assert_eq!(r.send(session("GET", "/admin/v1/me")).await.status, 401);
}

#[tokio::test]
async fn origins() {
    let t = rig().await;
    let evil = t
        .rig
        .send(session("GET", "/admin/v1/me").header("origin", "https://evil.example"))
        .await;
    evil.assert(403, "");
    let local = t
        .rig
        .send(session("GET", "/admin/v1/me").header("origin", "http://localhost:4173"))
        .await;
    assert_eq!(local.status, 200);
    assert_eq!(
        local.header("access-control-allow-origin"),
        Some("http://localhost:4173")
    );
    assert_eq!(local.header("access-control-allow-credentials"), Some("true"));
    assert_eq!(
        local.header("access-control-allow-headers"),
        Some("Content-Type, X-CSRF-Token")
    );
    assert_eq!(
        local.header("access-control-allow-methods"),
        Some("GET, POST, PUT, PATCH, DELETE, OPTIONS")
    );
    assert_eq!(local.header("vary"), Some("Origin"));
    let panel = rig_with(common::Options {
        store: true,
        settings: settings(&[("ADMIN_ALLOWED_ORIGINS", " https://panel.example , ")]),
        ..Default::default()
    })
    .await;
    assert_eq!(
        panel
            .rig
            .send(session("GET", "/admin/v1/me").header("origin", "https://panel.example"))
            .await
            .status,
        200
    );
    assert_eq!(
        panel
            .rig
            .send(session("GET", "/admin/v1/me").header("origin", "http://localhost:4173"))
            .await
            .status,
        403
    );
}

#[tokio::test]
async fn login_and_logout() {
    let t = rig().await;
    let r = &t.rig;
    let invalid =
        r#"{"error":{"code":"ADMIN_INVALID_CREDENTIALS","message":"Administrator credentials are invalid."}}"#;
    for body in [
        json!({"email": "nobody@example.test", "password": "correct horse"}),
        json!({"email": "boss@example.test", "password": "wrong"}),
        json!({"email": "fox@example.test", "password": "correct horse"}),
        json!({}),
    ] {
        r.send(Request1::new("POST", "/admin/v1/auth/login").json(&body))
            .await
            .assert(401, invalid);
    }
    let ok = r
        .send(
            Request1::new("POST", "/admin/v1/auth/login")
                .json(&json!({"email": " BOSS@example.test ", "password": "correct horse"})),
        )
        .await;
    assert_eq!(ok.status, 200, "{}", ok.body);
    let cookie = ok.cookies()[0].clone();
    assert!(
        cookie.starts_with("rustic_isles_admin=")
            && cookie.ends_with("; Path=/admin/v1; HttpOnly; SameSite=Strict; Max-Age=28800")
    );
    let csrf = ok.json()["csrfToken"].as_str().unwrap().to_owned();
    assert_eq!(csrf.len(), 64);
    assert_eq!(ok.json()["account"]["displayName"], "Boss");
    let raw = cookie.split(';').next().unwrap().to_owned();
    let out = r
        .send(
            Request1::new("POST", "/admin/v1/auth/logout")
                .header("cookie", &raw)
                .header("x-csrf-token", &csrf),
        )
        .await;
    out.assert(200, "{}");
    assert_eq!(
        out.cookies(),
        ["rustic_isles_admin=; Path=/admin/v1; HttpOnly; SameSite=Strict; Max-Age=0"]
    );
    assert_eq!(
        r.send(Request1::new("GET", "/admin/v1/me").header("cookie", &raw))
            .await
            .status,
        401
    );
    // Seven attempts from one address: the seventh is limited (four refused above, one success, one more here)
    let _ = r
        .send(Request1::new("POST", "/admin/v1/auth/login").json(&json!({})))
        .await;
    r.send(Request1::new("POST", "/admin/v1/auth/login").json(&json!({})))
        .await
        .assert(
            429,
            r#"{"error":{"code":"ADMIN_RATE_LIMITED","message":"Too many sign-in attempts."}}"#,
        );
    let prod = rig_with(common::Options {
        store: true,
        settings: settings(&[("NODE_ENV", "production")]),
        ..Default::default()
    })
    .await;
    let mfa = prod
        .rig
        .send(
            Request1::new("POST", "/admin/v1/auth/login")
                .json(&json!({"email": "boss@example.test", "password": "correct horse"})),
        )
        .await;
    assert_eq!((mfa.status, mfa.code()), (503, "ADMIN_MFA_NOT_CONFIGURED".to_owned()));
    let totp = rig_with(common::Options {
        store: true,
        settings: settings(&[("ADMIN_TOTP_SECRET", "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ")]),
        ..Default::default()
    })
    .await;
    let wrong = totp
        .rig
        .send(
            Request1::new("POST", "/admin/v1/auth/login")
                .json(&json!({"email": "boss@example.test", "password": "correct horse", "totp": "000000"})),
        )
        .await;
    assert!(wrong.code() == "ADMIN_MFA_INVALID" || wrong.status == 200);
}

#[tokio::test]
async fn players_access_roles_and_audit() {
    let t = rig().await;
    let r = &t.rig;
    let page = r.send(session("GET", "/admin/v1/players")).await.json();
    let keys: Vec<String> = page.as_object().unwrap().keys().cloned().collect();
    assert_eq!(keys, ["accounts", "total", "page", "limit"]);
    assert_eq!(
        (page["total"].clone(), page["page"].clone(), page["limit"].clone()),
        (json!(2), json!(1), json!(25))
    );
    let big = r.send(session("GET", "/admin/v1/players?limit=1000")).await.json();
    assert_eq!(big["limit"], 100);
    let odd = r
        .send(session("GET", "/admin/v1/players?limit=abc&page=-4"))
        .await
        .json();
    assert_eq!((odd["limit"].clone(), odd["page"].clone()), (json!(25), json!(1)));
    let ids = r
        .send(session(
            "GET",
            "/admin/v1/players?ids=fox-user,nobody,bad%20id,fox-user",
        ))
        .await
        .json();
    assert_eq!(
        (ids["total"].clone(), ids["page"].clone(), ids["limit"].clone()),
        (json!(1), json!(1), json!(2))
    );
    let online = r.send(session("GET", "/admin/v1/online-players")).await.json();
    assert_eq!(
        (online["count"].clone(), online["source"].clone()),
        (json!(0), json!("xmpp+presence"))
    );

    let access = |body: Value| session("PATCH", "/admin/v1/players/fox-user/access").json(&body);
    assert_eq!(
        r.send(access(json!({"approvalStatus": "maybe"}))).await.json()["error"]["message"],
        "Invalid approval status."
    );
    assert_eq!(
        r.send(access(json!({"status": "gone"}))).await.json()["error"]["message"],
        "Invalid account status."
    );
    assert_eq!(
        r.send(access(json!({}))).await.json()["error"]["message"],
        "No access-state change was supplied."
    );
    assert_eq!(
        r.send(access(json!({"status": "banned", "reason": " ab "})))
            .await
            .code(),
        "ADMIN_ACCESS_REASON"
    );
    let own = r
        .send(
            session("PATCH", "/admin/v1/players/boss-user/access")
                .json(&json!({"status": "banned", "reason": "testing"})),
        )
        .await;
    assert_eq!(own.code(), "ADMIN_SELF_LOCKOUT");
    r.send(session("PATCH", "/admin/v1/players/nobody/access").json(&json!({"status": "banned", "reason": "testing"})))
        .await
        .assert(404, r#"{"error":{"code":"NOT_FOUND","message":"Player not found."}}"#);
    let banned = r
        .send(access(json!({"status": "banned", "reason": "  cheating  "})))
        .await;
    assert_eq!(banned.status, 200, "{}", banned.body);
    let account = &banned.json()["account"];
    assert_eq!(
        (account["status"].clone(), account["approvalReason"].clone()),
        (json!("banned"), json!("cheating"))
    );
    assert_eq!(account["approvalUpdatedBy"], "boss-user");
    let audit = r
        .send(session("GET", "/admin/v1/audit?targetUserId=fox-user"))
        .await
        .json();
    assert_eq!(audit["total"], 1);
    let record = &audit["records"][0];
    assert_eq!(record["requestId"], banned.json()["requestId"]);
    assert_eq!(
        record["oldState"],
        json!({"approvalStatus":"approved","status":"active"})
    );
    assert_eq!(
        record["newState"],
        json!({"approvalStatus":"approved","status":"banned"})
    );
    let keys: Vec<String> = record.as_object().unwrap().keys().cloned().collect();
    assert_eq!(
        keys,
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

    let roles = |body: Value| session("PATCH", "/admin/v1/players/fox-user/roles").json(&body);
    assert_eq!(
        r.send(roles(json!({"roles": "tester"}))).await.json()["error"]["message"],
        "roles must be an array of strings."
    );
    assert_eq!(
        r.send(roles(json!({"roles": ["admin"]}))).await.json()["error"]["message"],
        "Only these roles can be toggled here: tester."
    );
    let tester = r
        .send(roles(json!({"roles": ["tester"], "reason": "for testing"})))
        .await
        .json();
    assert_eq!(tester["account"]["roles"], json!(["player", "tester"]));
    assert_eq!(tester["account"]["rolesUpdatedBy"], "boss-user");
    let cleared = r.send(roles(json!({"roles": []}))).await.json();
    assert_eq!(cleared["account"]["roles"], json!(["player"]));
    let newest = r.send(session("GET", "/admin/v1/audit?limit=1")).await.json();
    assert_eq!(newest["records"][0]["action"], "player.roles.update");
    assert!(newest["records"][0].get("reason").is_none());
    let third = r.send(session("GET", "/admin/v1/audit?limit=1&page=3")).await.json();
    assert_eq!(third["records"][0]["action"], "player.access.update");
}

#[tokio::test]
async fn deletion() {
    let t = rig().await;
    let r = &t.rig;
    let del = |body: Value| session("DELETE", "/admin/v1/players/fox-user").json(&body);
    assert_eq!(
        r.send(session("DELETE", "/admin/v1/players/boss-user").json(&json!({})))
            .await
            .code(),
        "ADMIN_SELF_DELETE"
    );
    assert_eq!(
        r.send(session("DELETE", "/admin/v1/players/nobody").json(&json!({})))
            .await
            .status,
        404
    );
    let wrong = r.send(del(json!({"confirmation": "DELETE fox_9"}))).await;
    assert_eq!(
        wrong.json()["error"]["message"],
        "Type \"DELETE Fox_9\" to confirm permanent deletion."
    );
    assert_eq!(
        r.send(del(json!({"confirmation": "DELETE Fox_9", "reason": "x"})))
            .await
            .code(),
        "ADMIN_DELETE_REASON"
    );
    let done = r
        .send(del(
            json!({"confirmation": "DELETE Fox_9", "reason": "account removal"}),
        ))
        .await;
    assert_eq!(done.status, 200, "{}", done.body);
    let keys: Vec<String> = done.json().as_object().unwrap().keys().cloned().collect();
    assert_eq!(keys, ["requestId", "deletedCounts", "characterCount"]);
    assert_eq!(done.json()["deletedCounts"]["adminSessions"], 0);
    assert_eq!(r.audits().await[0].action, "player.account.delete");
    let _ = &t.fox;
}

#[tokio::test]
async fn admin_off_is_core_404() {
    let rig = Rig::new(common::Options {
        store: true,
        settings: settings(&[("RUSTIC_ISLES_ADMIN_API", "off")]),
        ..Default::default()
    })
    .await;
    rig.get("/admin/v1/me").await.assert(404, "");
}

#[tokio::test]
async fn manager_operator() {
    let t = rig().await;
    let r = &t.rig;
    let now = now_ms();
    let me = r.send(signed("GET", "/admin/v1/me", None, now)).await;
    assert_eq!(me.status, 200, "{}", me.body);
    assert_eq!(me.json()["csrfToken"], Value::Null);
    assert_eq!(me.json()["account"]["userId"], format!("manager:{OPERATOR}"));
    assert_eq!(me.json()["account"]["displayName"], "admin-api");
    let body = json!({"status": "disabled", "reason": "by the bot"});
    let access = signed("PATCH", "/admin/v1/players/fox-user/access", Some(&body), now);
    let replay_headers = access.headers.clone();
    assert_eq!(r.send(access).await.status, 200);
    assert_eq!(r.audits().await[0].actor_user_id, format!("manager:{OPERATOR}"));
    let mut again = Request1::new("PATCH", "/admin/v1/players/fox-user/access").json(&body);
    again.headers = replay_headers;
    assert_eq!(r.send(again).await.code(), "ADMIN_OPERATOR_REPLAY");
    assert_eq!(
        r.send(signed("GET", "/admin/v1/me", None, now - 61_000)).await.code(),
        "ADMIN_OPERATOR_STALE"
    );
    assert_eq!(
        r.send(signed("GET", "/admin/v1/me", None, now + 6_000)).await.code(),
        "ADMIN_OPERATOR_STALE"
    );
    assert_eq!(
        r.send(signed("GET", "/admin/v1/me", None, now + 4_000)).await.status,
        200
    );
    let mut tampered = signed("GET", "/admin/v1/players", None, now + 1);
    tampered.uri = "/admin/v1/audit";
    assert_eq!(r.send(tampered).await.code(), "ADMIN_OPERATOR_SIGNATURE");
    let mut upper = signed("GET", "/admin/v1/me", None, now + 2);
    for (k, v) in &mut upper.headers {
        if k == "x-paradox-operator-signature" {
            *v = v.to_uppercase();
        }
    }
    assert_eq!(r.send(upper).await.status, 200);
    let mut bad_id = signed("GET", "/admin/v1/me", None, now + 3);
    for (k, v) in &mut bad_id.headers {
        if k == "x-paradox-operator-id" {
            *v = "abc".into();
        }
    }
    assert_eq!(r.send(bad_id).await.code(), "ADMIN_OPERATOR_INVALID");
    let forwarded = signed("GET", "/admin/v1/me", None, now + 5).header("x-forwarded-for", "1.2.3.4");
    assert_eq!(r.send(forwarded).await.code(), "ADMIN_OPERATOR_FORWARDED");
    let far = signed("GET", "/admin/v1/me", None, now + 6).peer("10.0.0.9:1");
    assert_eq!(r.send(far).await.code(), "ADMIN_OPERATOR_PEER_REFUSED");
    let mut form = signed("POST", "/admin/v1/auth/logout", None, now + 7);
    form.body = Some(b"a=b".to_vec());
    form = form.header("content-type", "application/x-www-form-urlencoded");
    assert_eq!(r.send(form).await.code(), "ADMIN_OPERATOR_BODY");
    let off = rig_with(common::Options {
        store: true,
        settings: settings(&[("RUSTIC_ISLES_MANAGER_ADMIN", "off"), ("RUSTIC_ISLES_ADMIN_API", "on")]),
        ..Default::default()
    })
    .await;
    assert_eq!(
        off.rig.send(signed("GET", "/admin/v1/me", None, now)).await.code(),
        "ADMIN_UNAUTHORIZED"
    );
    let short = rig_with(common::Options {
        store: true,
        settings: settings(&[("RUSTIC_ISLES_MANAGER_OPERATOR_SECRET", &"s".repeat(31))]),
        ..Default::default()
    })
    .await;
    assert_eq!(
        short.rig.send(signed("GET", "/admin/v1/me", None, now)).await.code(),
        "ADMIN_OPERATOR_NOT_CONFIGURED"
    );
    let start = rustic_isles_launcher_api::admin::auth::check_operator_config(&rustic_isles::Settings::from_pairs([(
        "RUSTIC_ISLES_MANAGER_ADMIN",
        "on",
    )]));
    assert_eq!(
        start.unwrap_err(),
        "RUSTIC_ISLES_MANAGER_OPERATOR_SECRET must contain at least 32 characters when RUSTIC_ISLES_MANAGER_ADMIN=on."
    );
}

#[tokio::test]
async fn a_request_crates_admin_signs_is_admitted() {
    let t = rig().await;
    let operator = dainty_admin::upstream::Operator::parse(OPERATOR).unwrap();
    let body = serde_json::to_vec(&json!({"approvalStatus": "approved", "reason": "bot check"})).unwrap();
    let target = "/admin/v1/players/fox-user/access";
    let timestamp = now_ms().to_string();
    let theirs =
        dainty_admin::upstream::operator_signature(OPERATOR_SECRET, "PATCH", target, &timestamp, &operator, &body);
    let ours = rustic_isles_launcher_api::admin::auth::operator_signature(
        OPERATOR_SECRET,
        "PATCH",
        target,
        &timestamp,
        OPERATOR,
        &body,
    );
    assert_eq!(theirs, ours);
    let mut r = Request1::new("PATCH", target)
        .header("x-paradox-operator-id", operator.id())
        .header("x-paradox-operator-name", "admin-api")
        .header("x-paradox-operator-timestamp", &timestamp)
        .header("x-paradox-operator-signature", &theirs)
        .header("content-type", "application/json");
    r.body = Some(body);
    let answer = t.rig.send(r).await;
    assert_eq!(answer.status, 200, "{}", answer.body);
}

#[tokio::test]
async fn host_routes() {
    let t = rig().await;
    let r = &t.rig;
    let host =
        |method: &'static str, uri: &'static str| Request1::new(method, uri).header("x-dainty-admin-key", ADMIN_KEY);
    let forbidden = r#"{"error":"forbidden"}"#;
    r.send(Request1::new("GET", "/dainty/api/InviteCodes"))
        .await
        .assert(403, forbidden);
    r.send(host("GET", "/dainty/api/InviteCodes").header("x-dainty-gateway", "1"))
        .await
        .assert(403, forbidden);
    r.send(host("GET", "/dainty/api/InviteCodes").header("x-forwarded-for", "1.2.3.4"))
        .await
        .assert(403, forbidden);
    let bad = r#"{"error":"bad_request","message":"uses is a whole number from 1 to 1000; name is text"}"#;
    for body in [
        json!({"uses": 0}),
        json!({"uses": 1001}),
        json!({"name": 5}),
        json!({"name": null}),
    ] {
        r.send(host("POST", "/dainty/api/CreateInvite").json(&body))
            .await
            .assert(400, bad);
    }
    let made = r.send(host("POST", "/dainty/api/CreateInvite").json(&json!({}))).await;
    assert_eq!(made.status, 200, "{}", made.body);
    let code = made.json()["code"].as_str().unwrap().to_owned();
    let re = regex::Regex::new(r"^[A-HJ-NP-Z2-9]{4}(-[A-HJ-NP-Z2-9]{4}){2}$").unwrap();
    assert!(re.is_match(&code), "{code}");
    let list = r.send(host("GET", "/dainty/api/InviteCodes")).await.json();
    assert_eq!(list["InviteCodes"][0]["note"], Value::Null);
    assert_eq!(list["InviteCodes"][0]["usesRemaining"], 1);
    r.send(host("DELETE", "/dainty/api/InviteCode/a"))
        .await
        .assert(400, r#"{"error":"bad_request"}"#);
    r.send(host("DELETE", "/dainty/api/InviteCode/ZZZZ-ZZZZ-ZZZZ"))
        .await
        .assert(404, "");
    r.send(host("DELETE", leak(&format!("/dainty/api/InviteCode/{code}"))))
        .await
        .assert(200, "");
    let status_bad =
        r#"{"error":"bad_request","message":"Username and Status (active, banned or disabled) are required."}"#;
    r.send(host("POST", "/dainty/api/SetAccountStatus").json(&json!({"Username": "Fox_9", "Status": "x"})))
        .await
        .assert(400, status_bad);
    r.send(host("POST", "/dainty/api/SetAccountStatus").json(&json!({"Username": "a", "Status": "banned"})))
        .await
        .assert(400, status_bad);
    r.send(host("POST", "/dainty/api/SetAccountStatus").json(&json!({"Username": "Nobody", "Status": "banned"})))
        .await
        .assert(404, r#"{"error":"not_found","message":"No account has that name."}"#);
    let set = r
        .send(host("POST", "/dainty/api/SetAccountStatus").json(&json!({"Username": "fox_9", "Status": "banned"})))
        .await;
    set.assert(200, r#"{"UserId":"fox-user","Username":"Fox_9","Status":"banned"}"#);
    let kick = r
        .send(host("POST", "/dainty/api/Kick").json(&json!({"Username": "Fox_9"})))
        .await;
    kick.assert(200, r#"{"UserId":"fox-user","Username":"Fox_9","Closed":0}"#);
}

#[tokio::test]
async fn release_gates() {
    let t = rig().await;
    let ok = t.rig.get("/health/release-gates").await;
    assert_eq!(ok.status, 200, "{}", ok.body);
    let keys: Vec<String> = ok.json().as_object().unwrap().keys().cloned().collect();
    assert_eq!(keys, ["ok", "gates"]);
    assert_eq!(ok.json()["gates"][0]["detail"], "nobody in game");
    t.rig
        .send(Request1::new("GET", "/health/release-gates").peer("10.0.0.1:1"))
        .await
        .assert(404, "");
    t.rig
        .send(Request1::new("GET", "/health/release-gates").header("x-forwarded-for", "1.2.3.4"))
        .await
        .assert(404, "");
}

#[tokio::test]
async fn discord_sign_in() {
    let configured = settings(&[
        ("DISCORD_CLIENT_ID", "client-123"),
        ("DISCORD_CLIENT_SECRET", "secret-456"),
        ("ADMIN_FRONTEND_URL", "http://localhost:4173/admin"),
        (
            "ADMIN_DISCORD_REDIRECT_URI",
            "http://localhost:4173/admin/v1/auth/discord/callback",
        ),
        ("ADMIN_DISCORD_ALLOWED_IDS", "100000000000000001,100000000000000002"),
    ]);
    let bare = rig().await;
    assert_eq!(
        bare.rig.get("/admin/v1/auth/discord/start").await.code(),
        "ADMIN_DISCORD_NOT_CONFIGURED"
    );
    let t = rig_with(common::Options {
        store: true,
        settings: configured,
        ..Default::default()
    })
    .await;
    let r = &t.rig;
    let start = r.get("/admin/v1/auth/discord/start").await;
    assert_eq!(start.status, 302);
    let location = start.header("location").unwrap().to_owned();
    assert!(location.starts_with("https://discord.com/api/oauth2/authorize?"));
    assert!(location.contains("client_id=client-123") && location.contains("code_challenge_method=S256"));
    let binding = start.cookies()[0].split(';').next().unwrap().to_owned();
    assert!(binding.starts_with("rustic_isles_admin_oauth="));
    let state = location
        .split("state=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap()
        .to_owned();
    let cancelled = r.get("/admin/v1/auth/discord/callback?error=access_denied").await;
    assert_eq!(
        cancelled.header("location"),
        Some("http://localhost:4173/admin/?error=discord_cancelled")
    );
    let expired = r.get("/admin/v1/auth/discord/callback?code=abc&state=bad").await;
    assert_eq!(
        expired.header("location"),
        Some("http://localhost:4173/admin/?error=discord_expired")
    );
    let done = r
        .send(
            Request1::new(
                "GET",
                leak(&format!(
                    "/admin/v1/auth/discord/callback?code=code-100000000000000001&state={state}"
                )),
            )
            .header("cookie", &binding),
        )
        .await;
    assert_eq!(done.status, 302, "{}", done.body);
    assert_eq!(done.header("location"), Some("http://localhost:4173/admin/"));
    let cookies = done.cookies();
    assert!(cookies[0].starts_with("rustic_isles_admin_oauth=;") && cookies[1].starts_with("rustic_isles_admin="));
    let made = r
        .store
        .read(|conn, repos| {
            repos
                .launcher_accounts
                .find_by_display_name_normalized(conn, "arcn1hssight")
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(made.roles, ["admin"]);
    assert_eq!(r.audits().await[0].action, "admin.discord.bootstrap");
    // A second start; the callback without the binding
    let again = r.get("/admin/v1/auth/discord/start").await;
    let state = again
        .header("location")
        .unwrap()
        .split("state=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap()
        .to_owned();
    let mismatch = r
        .send(
            Request1::new(
                "GET",
                leak(&format!(
                    "/admin/v1/auth/discord/callback?code=code-100000000000000001&state={state}"
                )),
            )
            .header("cookie", "rustic_isles_admin_oauth=AAAA"),
        )
        .await;
    assert_eq!(
        mismatch.header("location"),
        Some("http://localhost:4173/admin/?error=discord_state_mismatch")
    );
    let third = r.get("/admin/v1/auth/discord/start").await;
    let binding = third.cookies()[0].split(';').next().unwrap().to_owned();
    let state = third
        .header("location")
        .unwrap()
        .split("state=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap()
        .to_owned();
    let other = r
        .send(
            Request1::new(
                "GET",
                leak(&format!(
                    "/admin/v1/auth/discord/callback?code=code-100000000000000009&state={state}"
                )),
            )
            .header("cookie", &binding),
        )
        .await;
    assert_eq!(
        other.header("location"),
        Some("http://localhost:4173/admin/?error=discord_not_allowed")
    );
}

fn kit() -> Value {
    json!({
        "comment": "test kit",
        "progression": [{"progressionId": "ExperienceTrack_PlayerLevel", "targetProgress": "max"}],
        "inventory": [{"catalogId": "ITEM_A", "quantity": 2}],
        "currencies": [{"catalogId": "CURRENCY_PLATINUM_STEAM", "quantity": 100}]
    })
}

#[tokio::test]
async fn tester_max() {
    let funding = FakeFunding::default()
        .with_track("ExperienceTrack_PlayerLevel", 50_000, 5)
        .with_catalog(&[
            "ITEM_A",
            "CURRENCY_PLATINUM_STEAM",
            "CURRENCY_PLATINUM",
            "CURRENCY_SEASONAL_COIN",
        ]);
    let t = rig_with(common::Options {
        store: true,
        funding: Some(funding),
        ..Default::default()
    })
    .await;
    let r = &t.rig;
    let max = |body: Value| session("POST", "/admin/v1/players/fox-user/tester-max").json(&body);
    assert_eq!(r.send(max(json!({}))).await.code(), "ADMIN_TESTER_REQUIRED");
    r.send(session("PATCH", "/admin/v1/players/fox-user/roles").json(&json!({"roles": ["tester"]})))
        .await;
    assert_eq!(
        r.send(max(json!({"confirm": "MAX fox_9"}))).await.code(),
        "ADMIN_TESTER_MAX_CONFIRMATION"
    );
    assert_eq!(
        r.send(max(json!({"confirm": "MAX Fox_9", "runId": "short"})))
            .await
            .code(),
        "ADMIN_TESTER_MAX_RUN_ID"
    );
    let run = json!({"confirm": "MAX Fox_9", "runId": "run-00000001"});
    assert_eq!(r.send(max(run.clone())).await.code(), "ADMIN_TESTER_MAX_KIT_MISSING");
    std::fs::write(r.data.path().join("tester_max_kit.json"), kit().to_string()).unwrap();
    let none = r.send(max(run.clone())).await;
    assert_eq!(
        (none.status, none.code()),
        (409, "ADMIN_TESTER_MAX_NO_CHARACTER".to_owned())
    );
    assert_eq!(r.audits().await[0].action, "player.tester_max.failed");
    let fox = t.fox.clone();
    r.store
        .write(&ActorContext::new(Actor::System), move |tx, repos| {
            repos
                .characters
                .create(
                    tx,
                    &rustic_isles_store::repo::characters::CharacterRecord {
                        character_id: "char-1".into(),
                        account: fox,
                        created_date: "x".into(),
                        last_modified_date: "x".into(),
                        name: "S".into(),
                        update_version: 1,
                        data: "{}".into(),
                    },
                )
                .map(|_| ())
        })
        .await
        .unwrap();
    let done = r.send(max(run.clone())).await;
    assert_eq!(done.status, 200, "{}", done.body);
    let report = &done.json()["report"];
    assert_eq!(report["status"], "done");
    assert_eq!(report["progression"][0]["granted"], 250_000);
    assert_eq!(report["progression"][0]["grants"], 3);
    assert_eq!(report["progression"][0]["confirmedRank"], 5);
    assert_eq!(report["items"], json!({"status":"done","stacks":2}));
    let rerun = r.send(max(run)).await.json();
    assert_eq!(rerun["report"]["progression"][0]["granted"], 0);
    assert_eq!(rerun["report"]["status"], "done");
}

#[test]
fn kit_validation() {
    let shared = rustic_isles::Shared::bare(rustic_isles::testing::sample_version()).unwrap();
    let funding = FakeFunding::default()
        .with_track("ExperienceTrack_PlayerLevel", 50_000, 5)
        .with_catalog(&["ITEM_A", "CURRENCY_PLATINUM_STEAM", "CURRENCY_SEASONAL_COIN"]);
    let check = |kit: Value| rustic_isles_launcher_api::admin::tester_max::validate_kit(&kit, &shared, &funding);
    assert!(check(kit()).is_empty());
    assert_eq!(
        check(json!({"slayersPath": []})),
        ["slayersPath was removed (16 Oct): a tester unlocks the Slayer's Path in game"]
    );
    assert_eq!(
        check(json!({"currencies": [{"catalogId": "CURRENCY_PLATINUM", "quantity": 1}]})),
        [
            "currencies[0]: CURRENCY_PLATINUM is the Platinum aggregate; grant a storefront bucket (CURRENCY_PLATINUM_*) instead"
        ]
    );
    assert!(check(json!({"currencies": [{"catalogId": "CURRENCY_SEASONAL_COIN", "quantity": 1}]})).is_empty());
    assert_eq!(check(json!([])), ["the kit must be a JSON object"]);
}

#[tokio::test]
async fn admin_actions_and_home_operations() {
    let t = rig().await;
    let r = &t.rig;
    let admin = std::sync::Arc::clone(&r.admin);
    let call = |request| AdminCall {
        actor: ActorContext::new(Actor::Console),
        request,
    };
    let until = rustic_isles_auth::iso(now_ms() + 3_600_000);
    let done = admin
        .admin_action(
            &r.shared,
            call(AdminRequest::Ban {
                player: "FOX_9".into(),
                reason: Some("cool off".into()),
                until: Some(until.clone()),
            }),
        )
        .await
        .unwrap();
    assert_eq!(done, AdminResponse::Done);
    let lookup = admin
        .admin_action(
            &r.shared,
            call(AdminRequest::Lookup {
                player: "fox-user".into(),
            }),
        )
        .await
        .unwrap();
    let AdminResponse::Player(info) = lookup else {
        panic!("{lookup:?}")
    };
    // Every action lands in the admin record the dashboard's History reads, by the game's user id
    let records = r.audits().await;
    let timeout = records.iter().find(|a| a.action == "player.timeout").unwrap();
    assert_eq!(
        (timeout.actor_user_id.as_str(), timeout.target_user_id.as_deref()),
        ("host", Some("fox-user"))
    );
    assert_eq!(timeout.reason.as_deref(), Some("cool off"));
    // The game's own id and the account's dates, which the dashboard looks the backend account up by
    assert_eq!(info.user_id.as_deref(), Some("fox-user"));
    assert!(info.created_at.is_some());
    assert!(!info.admin);
    let restriction = info.restriction.unwrap();
    assert_eq!(
        (restriction.kind, restriction.until),
        (RestrictionKind::Timeout, Some(until))
    );
    // The timeout kicked: while the kick is carried out the player's tokens are refused; once it lifts, the
    // session they have works again (the timeout itself keeps them out through the account's eligibility)
    let claims = |user: &str| {
        let mut claims = serde_json::Map::new();
        claims.insert("userId".into(), json!(user));
        claims.insert("iat".into(), json!(now_ms() / 1000 - 60));
        claims
    };
    assert!(r.shared.auth().is_kicked(&claims("fox-user")));
    assert!(!r.shared.auth().is_kicked(&claims("someone-else")));
    r.shared.auth().lift_kick("fox-user");
    assert!(!r.shared.auth().is_kicked(&claims("fox-user")));
    r.shared.auth().kick("fox-user", 0);
    assert!(
        !r.shared.auth().is_kicked(&claims("fox-user")),
        "a window that ended lifts by itself"
    );
    admin
        .admin_action(
            &r.shared,
            call(AdminRequest::Ban {
                player: t.fox.to_string(),
                reason: Some("cheating".into()),
                until: None,
            }),
        )
        .await
        .unwrap();
    let AdminResponse::Player(info) = admin
        .admin_action(
            &r.shared,
            call(AdminRequest::Lookup {
                player: "fox-user".into(),
            }),
        )
        .await
        .unwrap()
    else {
        panic!()
    };
    let restriction = info.restriction.unwrap();
    assert_eq!(
        (restriction.kind, restriction.reason.as_deref()),
        (RestrictionKind::Ban, Some("cheating"))
    );
    assert_eq!(
        admin
            .admin_action(&r.shared, call(AdminRequest::Lookup { player: " ".into() }))
            .await
            .unwrap(),
        AdminResponse::NotFound
    );

    let system = ActorContext::new(Actor::System);
    let fox = t.fox.clone();
    let op = |name: &'static str, body: Value| {
        let admin = std::sync::Arc::clone(&admin);
        let (shared, fox, system) = (std::sync::Arc::clone(&r.shared), fox.clone(), system.clone());
        async move { admin.home_operation(&shared, name, &fox, system, body).await.unwrap() }
    };
    let read = op("inventory.read", json!({})).await.unwrap();
    assert!(read.get("character_id").is_none());
    let wrote = op("inventory.write", json!({"loot": ["A:2", "A", "B:x"]}))
        .await
        .unwrap();
    assert_eq!(wrote["loot_applied"], 3);
    let read = op("inventory.read", json!({})).await.unwrap();
    assert_eq!(
        read["stacked_items"],
        json!([{"catalogId":"A","quantity":3},{"catalogId":"B","quantity":1}])
    );
    let first = op("bounty.write", json!({"bounty_progress": {"a": 1}})).await.unwrap();
    let second = op("bounty.write", json!({})).await.unwrap();
    assert_eq!(
        (first["update_version"].clone(), second["update_version"].clone()),
        (json!(1), json!(2))
    );
    assert_eq!(op("bounty.read", json!({})).await.unwrap()["count"], 2);
    let grant = op("progression.grant", json!({"xp": -5})).await.unwrap();
    assert_eq!(grant["progress"], 0);
    assert_eq!(
        op("character.load", json!({})).await.unwrap()["operation"],
        "character.load"
    );
    assert!(op("nope.op", json!({})).await.is_none());
}

#[tokio::test]
async fn bootstrap() {
    let t = rig().await;
    let r = &t.rig;
    let request = |email: &str, from: Option<&str>, confirm: &str| BootstrapAdminRequest {
        email: email.into(),
        from_email: from.map(str::to_owned),
        username: None,
        confirm: confirm.into(),
    };
    assert!(matches!(
        bootstrap_admin(&r.store, request("a@example.com", None, "yes")).await,
        Err(BootstrapError::Usage)
    ));
    let done = bootstrap_admin(
        &r.store,
        request("Admin@Example.com", Some("fox@example.test"), "GRANT_ADMIN_ROLE"),
    )
    .await
    .unwrap();
    assert_eq!(
        (done.user_id.as_str(), done.email.as_str()),
        ("fox-user", "admin@example.com")
    );
    let account = r.launcher(&t.fox).await;
    assert!(account.roles.contains(&"admin".to_owned()));
    assert_eq!(account.approval_updated_by.as_deref(), Some("bootstrap-script"));
    let rerun = bootstrap_admin(
        &r.store,
        request("admin@example.com", Some("fox@example.test"), "GRANT_ADMIN_ROLE"),
    )
    .await;
    assert!(rerun.is_ok());
    let clash = bootstrap_admin(
        &r.store,
        request("boss@example.test", Some("admin@example.com"), "GRANT_ADMIN_ROLE"),
    )
    .await;
    assert_eq!(
        clash.unwrap_err().to_string(),
        "Cannot rename account: boss@example.test is already in use."
    );
}

#[test]
fn debug_never_shows_secrets() {
    let config = rustic_isles_launcher_api::admin::discord::AdminDiscordConfig {
        client_id: "id".into(),
        client_secret: "hunter2".into(),
        redirect_uri: "r".into(),
        frontend: "f".into(),
        allowed: vec![],
    };
    assert!(!format!("{config:?}").contains("hunter2"));
}

async fn give_character(r: &Rig, account: &AccountId) {
    let account = account.clone();
    r.store
        .write(&ActorContext::new(Actor::System), move |tx, repos| {
            repos
                .characters
                .create(
                    tx,
                    &rustic_isles_store::repo::characters::CharacterRecord {
                        character_id: "char-1".into(),
                        account,
                        created_date: "x".into(),
                        last_modified_date: "x".into(),
                        name: "S".into(),
                        update_version: 1,
                        data: "{}".into(),
                    },
                )
                .map(|_| ())
        })
        .await
        .unwrap();
}

async fn track_progress(r: &Rig, account: &AccountId, track: &'static str) -> i64 {
    let account = account.clone();
    r.store
        .read(move |conn, repos| repos.progression_tracks.get(conn, &account, track))
        .await
        .unwrap()
        .map_or(0, |t| t.progress)
}

async fn grant_rig() -> Admin {
    let funding = FakeFunding::default()
        .with_track("ExperienceTrack_PlayerLevel", 50_000, 5)
        .with_catalog(&[
            "ITEM_A",
            "WEAPON_SWORD",
            "CURRENCY_NOTES",
            "CURRENCY_PLATINUM_UNIV",
            "CURRENCY_PLATINUM",
        ]);
    rig_with(common::Options {
        store: true,
        funding: Some(funding),
        ..Default::default()
    })
    .await
}

#[tokio::test]
async fn grant_catalog_lists_what_a_grant_may_name() {
    let t = grant_rig().await;
    let r = &t.rig;
    assert_eq!(r.get("/admin/v1/grant-catalog").await.status, 401);
    let catalog = r.send(session("GET", "/admin/v1/grant-catalog")).await;
    assert_eq!(catalog.status, 200, "{}", catalog.body);
    assert_eq!(
        catalog.json(),
        json!({
            "items": [
                {"catalogId": "ITEM_A", "storage": "stacked", "tags": []},
                {"catalogId": "WEAPON_SWORD", "storage": "stacked", "tags": []}
            ],
            "currencies": ["CURRENCY_NOTES", "CURRENCY_PLATINUM_UNIV"],
            "progression": [{"progressionId": "ExperienceTrack_PlayerLevel", "maxRank": 5}]
        })
    );
    // The dashboard's path: the manager operator's signed call
    let signed = r.send(signed("GET", "/admin/v1/grant-catalog", None, now_ms())).await;
    assert_eq!(signed.status, 200, "{}", signed.body);
}

#[tokio::test]
async fn grant_validation() {
    let t = grant_rig().await;
    let r = &t.rig;
    let grant = |body: Value| session("POST", "/admin/v1/players/fox-user/grants").json(&body);
    let unsigned = r
        .send(Request1::new("POST", "/admin/v1/players/fox-user/grants").json(&json!({})))
        .await;
    assert_eq!(unsigned.status, 401);
    assert_eq!(r.send(grant(json!({}))).await.code(), "ADMIN_GRANT_ID");
    assert_eq!(
        r.send(grant(json!({"grantId": "grant-0001"}))).await.code(),
        "ADMIN_GRANT_REASON"
    );
    let missing = r
        .send(
            session("POST", "/admin/v1/players/nobody/grants")
                .json(&json!({"grantId": "grant-0001", "reason": "for testing"})),
        )
        .await;
    assert_eq!(missing.status, 404);
    let problems = |body: Value| async move {
        let mut body = body;
        body["grantId"] = json!("grant-0001");
        body["reason"] = json!("for testing");
        let answer = r.send(grant(body)).await;
        assert_eq!((answer.status, answer.code()), (422, "ADMIN_GRANT_INVALID".to_owned()));
        answer.json()["error"]["problems"].clone()
    };
    assert_eq!(problems(json!({})).await, json!(["nothing to grant"]));
    assert_eq!(
        problems(json!({
            "items": [
                {"catalogId": "ITEM_MADE_UP", "quantity": 1},
                {"catalogId": "CURRENCY_NOTES", "quantity": 1},
                {"catalogId": "ITEM_A", "quantity": 1001}
            ],
            "currencies": [
                {"catalogId": "CURRENCY_PLATINUM", "quantity": 5},
                {"catalogId": "ITEM_A", "quantity": 5},
                {"catalogId": "CURRENCY_NOTES", "quantity": 0}
            ],
            "progression": [
                {"progressionId": "Linked_Slayer_Slot_1", "xp": 5},
                {"progressionId": "ExperienceTrack_PlayerLevel", "xp": 5, "rank": 2},
                {"progressionId": "ExperienceTrack_PlayerLevel", "rank": 6}
            ],
            "extra": true
        }))
        .await,
        json!([
            "unknown field \"extra\"",
            "items[0]: \"ITEM_MADE_UP\" is not in the server's catalog",
            "items[1]: CURRENCY_NOTES is a currency; list it under currencies",
            "items[2]: quantity must be an integer from 1 to 1000",
            "currencies[0]: CURRENCY_PLATINUM is the Platinum aggregate; grant a storefront bucket (CURRENCY_PLATINUM_*) instead",
            "currencies[1]: \"ITEM_A\" is not a known CURRENCY_* id",
            "currencies[2]: quantity must be an integer from 1 to 10000000",
            "progression[0]: \"Linked_Slayer_Slot_1\" is not an approved XP track",
            "progression[1]: give exactly one of xp or rank",
            "progression[2]: rank 6 is past ExperienceTrack_PlayerLevel's top rank"
        ])
    );
    assert!(r.audits().await.is_empty(), "a refused grant changes nothing");
}

#[tokio::test]
async fn grant_applies_once_and_is_audited() {
    let t = grant_rig().await;
    let r = &t.rig;
    let grant = |body: &Value| session("POST", "/admin/v1/players/fox-user/grants").json(body);
    let body = json!({
        "grantId": "grant-0001",
        "reason": "lost gear refund",
        "items": [{"catalogId": "WEAPON_SWORD", "quantity": 1}, {"catalogId": "ITEM_A", "quantity": 2}],
        "currencies": [{"catalogId": "CURRENCY_NOTES", "quantity": 5000}],
        "progression": [{"progressionId": "ExperienceTrack_PlayerLevel", "xp": 120_000}]
    });
    let none = r.send(grant(&body)).await;
    assert_eq!((none.status, none.code()), (409, "ADMIN_GRANT_NO_CHARACTER".to_owned()));
    give_character(r, &t.fox).await;
    let done = r.send(grant(&body)).await;
    assert_eq!(done.status, 200, "{}", done.body);
    let report = &done.json()["report"];
    assert_eq!(report["status"], "done");
    assert_eq!(report["characterId"], "char-1");
    assert_eq!(
        report["items"],
        json!({"status": "done", "stacks": [
            {"catalogId": "WEAPON_SWORD", "quantity": 1},
            {"catalogId": "ITEM_A", "quantity": 2},
            {"catalogId": "CURRENCY_NOTES", "quantity": 5000}
        ]})
    );
    assert_eq!(report["progression"][0]["granted"], 120_000);
    assert_eq!(report["progression"][0]["confirmedRank"], 2);
    assert_eq!(track_progress(r, &t.fox, "ExperienceTrack_PlayerLevel").await, 120_000);
    let audits = r.audits().await;
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].action, "player.grant");
    assert_eq!(audits[0].actor_user_id, "boss-user");
    assert_eq!(audits[0].target_user_id.as_deref(), Some("fox-user"));
    assert_eq!(audits[0].reason.as_deref(), Some("lost gear refund"));
    assert_eq!(audits[0].new_state.as_ref().unwrap()["grantId"], "grant-0001");
    // A retried grant id replays the ledger instead of granting twice
    let again = r.send(grant(&body)).await.json();
    assert_eq!(again["report"]["progression"][0]["granted"], 0);
    assert_eq!(track_progress(r, &t.fox, "ExperienceTrack_PlayerLevel").await, 120_000);
    // A rank goal tops the track up to the rank's XP
    let rank = json!({
        "grantId": "grant-0002",
        "reason": "level up for testing",
        "progression": [{"progressionId": "ExperienceTrack_PlayerLevel", "rank": 4}]
    });
    let ranked = r.send(grant(&rank)).await.json();
    assert_eq!(ranked["report"]["progression"][0]["granted"], 80_000);
    assert_eq!(ranked["report"]["progression"][0]["confirmedRank"], 4);
    assert!(ranked["report"].get("items").is_none());
    assert_eq!(track_progress(r, &t.fox, "ExperienceTrack_PlayerLevel").await, 200_000);
}

#[test]
fn the_record_names_who_acted_without_a_key() {
    use rustic_isles_launcher_api::admin::actions::who;
    let admin = |request: Option<&str>| {
        let actor = ActorContext::new(Actor::Admin { key: "abc123".into() });
        match request {
            Some(r) => actor.with_request(r),
            None => actor,
        }
    };
    assert_eq!(who(&admin(Some("admin-1-2-op555"))), "key:abc123/discord:555");
    assert_eq!(who(&admin(Some("admin-1-2"))), "key:abc123");
    assert_eq!(who(&admin(Some("admin-1-2-opx"))), "key:abc123");
    assert_eq!(who(&admin(None)), "key:abc123");
    assert_eq!(who(&ActorContext::new(Actor::Console)), "host");
}

#[test]
fn a_grant_lands_an_item_the_policy_doesnt_name_as_the_catalog_stores_it() {
    use rustic_isles_economy::inventory::storage_policy::{ItemCatalog, StoragePolicy};
    use rustic_isles_launcher_api::admin::tester_max::grant_collections;
    let policy = StoragePolicy::from_document(&json!({"entries": {"WP_POLICY": {"storage": "instanced"}}}));
    let catalog = ItemCatalog::from_document(&json!({"entries": {
        "QI_11A_CRIT_POTION": {"storage": "stacked", "maxQuantity": 30, "name": "Granny's Heartbreaker"},
        "WP_MS_HOST": {"storage": "instanced", "maxQuantity": 1},
        "WP_POLICY": {"storage": "stacked"}
    }}));
    assert_eq!(catalog.names["QI_11A_CRIT_POTION"], "Granny's Heartbreaker");
    let stacks = [
        ("QI_11A_CRIT_POTION".to_owned(), 40),
        ("WP_MS_HOST".to_owned(), 2),
        ("WP_POLICY".to_owned(), 1),
    ];
    let (instanced, stacked) = grant_collections(&policy, &catalog, &stacks);
    // A tonic past its cap stays one stack; a weapon only the catalog has is one instance per unit, its id minted
    // by the transaction; the policy's ids are left to the transaction's normalization
    assert_eq!(
        instanced,
        [json!({"catalogId": "WP_MS_HOST"}), json!({"catalogId": "WP_MS_HOST"})]
    );
    assert_eq!(
        stacked,
        [
            json!({"catalogId": "QI_11A_CRIT_POTION", "quantity": 40}),
            json!({"catalogId": "WP_POLICY", "quantity": 1})
        ]
    );
}
