// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The launcher API black-box, from spec §12.2 and §12.3.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use axum::http::{Method, StatusCode};
use common::{Harness, bearer, h};
use rustic_isles_auth::{Approval, AuthStore, Status};
use serde_json::{Value, json};

const MSG_LENGTH: &str = "Username must be 3\u{2013}16 characters.";
const MSG_SYMBOLS: &str = "Username can only contain letters and numbers (no spaces or symbols).";
const MSG_RESERVED: &str = "That username is reserved. Choose another.";
const MSG_NOT_ALLOWED: &str = "That username isn't allowed. Choose another.";

#[tokio::test]
async fn username_availability() {
    let t = Harness::new().await;
    for (query, reason) in [
        ("?name=ab", MSG_LENGTH),
        ("?name=Slay%20er", MSG_SYMBOLS),
        ("?name=AdminBob", MSG_RESERVED),
        ("?name=Dick69", MSG_NOT_ALLOWED),
        ("", MSG_LENGTH),
        ("?name=Slayer1&name=Slayer2", MSG_SYMBOLS),
    ] {
        let answer = t.get(&format!("/launcher/v1/username{query}"), &[]).await;
        assert_eq!(answer.status, StatusCode::OK);
        assert_eq!(answer.json(), json!({"available": false, "reason": reason}), "{query}");
    }
    assert_eq!(
        t.get("/launcher/v1/username?name=Slayer1", &[]).await.json(),
        json!({"available": true})
    );
    t.register(Some("Slayer1"), "s1@example.test").await;
    assert_eq!(
        t.get("/launcher/v1/username?name=SLAYER1", &[]).await.json(),
        json!({"available": false, "reason": "That username is already taken."})
    );
}

#[tokio::test]
async fn registration() {
    let t = Harness::new().await;
    for (body, message) in [
        (
            json!({"email": "nope", "password": "correct horse"}),
            "Enter a valid email address.",
        ),
        (
            json!({"email": "a@b.test", "password": "short"}),
            "Password must be at least 8 characters.",
        ),
        (
            json!({"email": "a@b.test", "password": "correct horse", "displayName": "ab"}),
            MSG_LENGTH,
        ),
        (
            json!({"email": "a@b.test", "password": "correct horse", "displayName": "AdminBob"}),
            MSG_RESERVED,
        ),
        (
            json!({"email": "a@b.test", "password": "correct horse", "displayName": "Dick"}),
            MSG_NOT_ALLOWED,
        ),
    ] {
        let answer = t.post("/launcher/v1/auth/register", &[], body).await;
        assert_eq!(answer.status, StatusCode::BAD_REQUEST);
        assert_eq!(
            (answer.code(), answer.message()),
            ("AUTH_VALIDATION_FAILED".into(), message.into())
        );
    }
    let none = t.raw(Method::POST, "/launcher/v1/auth/register", &[], None).await;
    assert_eq!(
        (none.status, none.message()),
        (StatusCode::BAD_REQUEST, "Enter a valid email address.".into())
    );

    let answer = t.register(Some("Slayer1"), "Slayer1@Example.Test").await;
    assert_eq!(answer.status, StatusCode::ACCEPTED);
    let mut json = answer.json();
    let user_id = json["account"]["userId"].as_str().unwrap().to_owned();
    assert_eq!(user_id.len(), 36);
    json["account"]["userId"] = json!("<uuid>");
    assert_eq!(
        serde_json::to_string(&json).unwrap(),
        r#"{"approvalRequired":false,"account":{"userId":"<uuid>","displayName":"Slayer1","email":"slayer1@example.test","discordLinked":false,"discord":{"linked":false},"emailVerified":false,"status":"active","approvalStatus":"approved","needsUsername":false}}"#
    );
    assert!(
        answer
            .header("content-type")
            .unwrap()
            .starts_with("application/json; charset=utf-8")
    );
    let taken = t.register(Some("Other1"), "slayer1@example.test").await;
    assert_eq!(
        (taken.status, taken.code()),
        (StatusCode::CONFLICT, "AUTH_EMAIL_TAKEN".into())
    );
    let taken = t.register(Some("SLAYER1"), "other@example.test").await;
    assert_eq!(
        (taken.status, taken.code()),
        (StatusCode::CONFLICT, "AUTH_DISPLAY_NAME_TAKEN".into())
    );
    for (name, email) in [(None, "anon@example.test"), (Some("  "), "blank@example.test")] {
        let answer = t.register(name, email).await;
        assert_eq!(answer.status, StatusCode::ACCEPTED);
        let account = &answer.json()["account"];
        assert_eq!(account["displayName"], account["userId"]);
        assert_eq!(account["needsUsername"], json!(true));
    }
    let game = t.store().game_account_by_user_id(&user_id).await.unwrap().unwrap();
    assert_eq!(game.name, "Slayer1");
}

#[tokio::test]
async fn login() {
    let t = Harness::new().await;
    t.register(Some("Slayer1"), "slayer1@example.test").await;
    for body in [
        json!({"email": "slayer1@example.test", "password": "wrong horse"}),
        json!({"email": "nobody@example.test", "password": "correct horse"}),
        json!({}),
    ] {
        let answer = t.post("/launcher/v1/auth/login", &[], body).await;
        assert_eq!(
            (answer.status, answer.code()),
            (StatusCode::UNAUTHORIZED, "AUTH_INVALID_CREDENTIALS".into())
        );
        assert_eq!(answer.message(), "The email or password is incorrect.");
    }
    let none = t.raw(Method::POST, "/launcher/v1/auth/login", &[], None).await;
    assert_eq!(none.status, StatusCode::UNAUTHORIZED);
    let numeric = t
        .post("/launcher/v1/auth/login", &[], json!({"email": 5, "password": "x"}))
        .await;
    assert_eq!(
        (numeric.status, numeric.code()),
        (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL".into())
    );
    let spaced = t.login(" SLAYER1@example.test ", "correct horse").await;
    assert_eq!(spaced.status, StatusCode::OK);
    let json = spaced.json();
    assert_eq!(json["refreshToken"].as_str().unwrap().len(), 64);
    assert!(json["accessToken"].as_str().unwrap().starts_with("eyJ"));
    let form = t
        .raw(
            Method::POST,
            "/launcher/v1/auth/login",
            &[],
            Some((
                "application/x-www-form-urlencoded",
                "email=slayer1%40example.test&password=correct+horse".into(),
            )),
        )
        .await;
    assert_eq!(form.status, StatusCode::OK);
    for raw in ["{\"email\":", "\"text\""] {
        let bad = t
            .raw(
                Method::POST,
                "/launcher/v1/auth/login",
                &[],
                Some(("application/json", raw.into())),
            )
            .await;
        assert_eq!(bad.status, StatusCode::BAD_REQUEST);
        assert!(bad.text.contains("<pre>Bad Request</pre>"));
    }
}

#[tokio::test]
async fn login_limits() {
    let t = Harness::new().await;
    for i in 0..10 {
        let answer = t.login("victim@example.test", &format!("guess {i}")).await;
        assert_eq!(answer.status, StatusCode::UNAUTHORIZED);
    }
    let limited = t.login("victim@example.test", "guess").await;
    assert_eq!(
        (limited.status, limited.message()),
        (
            StatusCode::TOO_MANY_REQUESTS,
            "Too many login attempts. Try again later.".into()
        )
    );
}

#[tokio::test]
async fn me_and_sign_in_required() {
    let t = Harness::new().await;
    let (access, _, _) = t.account("Slayer1", "slayer1@example.test").await;
    for (headers, message) in [
        (vec![], "Sign in required."),
        (
            vec![("authorization", "Bearer not.a.token")],
            "Your session has expired. Please sign in again.",
        ),
        (vec![("authorization", "Basic abc")], "Sign in required."),
    ] {
        let answer = t.get("/launcher/v1/me", &headers).await;
        assert_eq!(
            (answer.status, answer.message()),
            (StatusCode::UNAUTHORIZED, message.into())
        );
        assert_eq!(answer.json()["error"]["requestId"].as_str().unwrap().len(), 36);
    }
    let me = t.get("/launcher/v1/me", &[("authorization", &bearer(&access))]).await;
    assert_eq!(me.json()["displayName"], json!("Slayer1"));
}

#[tokio::test]
async fn account_keys_sign_in() {
    let t = Harness::new().await;
    let (_, _, user_id) = t.account("Slayer1", "slayer1@example.test").await;
    let account = t.store().launcher_account_by_user_id(&user_id).await.unwrap().unwrap();
    let hash = rustic_isles_auth::hashes::user_key_hash(common::SECRET, "RRK_test");
    t.store().insert_user_key(&account.account_id, &hash);
    let me = t
        .get("/launcher/v1/me", &[("x-reforged-user-api-key", " RRK_test ")])
        .await;
    assert_eq!(me.status, StatusCode::OK);
    let wrong = t
        .get("/launcher/v1/me", &[("x-reforged-user-api-key", "RRK_other")])
        .await;
    assert_eq!(wrong.message(), "Sign in required.");
    // Logout with a key matches no family
    let out = t
        .post(
            "/launcher/v1/auth/logout",
            &[("x-reforged-user-api-key", "RRK_test")],
            json!({}),
        )
        .await;
    assert_eq!((out.status, out.text.as_str()), (StatusCode::OK, "{}"));
}

#[tokio::test]
async fn usernames() {
    let t = Harness::new().await;
    t.register(Some("Slayer1"), "slayer1@example.test").await;
    t.register(None, "anon@example.test").await;
    let access = t.login("anon@example.test", "correct horse").await.json()["accessToken"]
        .as_str()
        .unwrap()
        .to_owned();
    let auth = bearer(&access);
    for (name, status, message) in [
        (json!("x"), 400, MSG_LENGTH),
        (json!("Moderator"), 400, MSG_RESERVED),
        (json!("slayer1"), 409, "That username is already taken."),
        (json!(5), 400, MSG_LENGTH),
    ] {
        let answer = t
            .post(
                "/launcher/v1/username",
                &[("authorization", &auth)],
                json!({"username": name}),
            )
            .await;
        assert_eq!((answer.status.as_u16(), answer.message()), (status, message.into()));
    }
    for _ in 0..2 {
        let answer = t
            .post(
                "/launcher/v1/username",
                &[("authorization", &auth)],
                json!({"username": "Slayer2"}),
            )
            .await;
        assert_eq!(answer.status, StatusCode::OK);
        assert_eq!(answer.json()["displayName"], json!("Slayer2"));
        assert_eq!(answer.json()["needsUsername"], json!(false));
    }
    let none = t
        .post("/launcher/v1/username", &[], json!({"username": "Slayer3"}))
        .await;
    assert_eq!(none.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn refresh_rotates_and_reuse_revokes_the_family() {
    let t = Harness::new().await;
    let (_, refresh, _) = t.account("Slayer1", "slayer1@example.test").await;
    let path = "/launcher/v1/auth/refresh";
    for body in [json!({}), json!({"refreshToken": h('f')})] {
        let answer = t.post(path, &[], body).await;
        assert_eq!(
            (answer.status, answer.code()),
            (StatusCode::UNAUTHORIZED, "AUTH_REFRESH_INVALID".into())
        );
    }
    let first = t.post(path, &[], json!({"refreshToken": refresh})).await;
    assert_eq!(first.status, StatusCode::OK);
    let second_access = first.json()["accessToken"].as_str().unwrap().to_owned();
    let second_refresh = first.json()["refreshToken"].as_str().unwrap().to_owned();
    let reuse = t.post(path, &[], json!({"refreshToken": refresh})).await;
    assert_eq!(reuse.status, StatusCode::UNAUTHORIZED);
    let me = t
        .get("/launcher/v1/me", &[("authorization", &bearer(&second_access))])
        .await;
    assert_eq!(me.message(), "Your session has expired. Please sign in again.");
    let again = t.post(path, &[], json!({"refreshToken": second_refresh})).await;
    assert_eq!(again.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn refresh_refuses_expired_and_unadmitted_sessions() {
    let t = Harness::new().await;
    let (_, refresh, user_id) = t.account("Slayer1", "slayer1@example.test").await;
    t.store().update_account(&user_id, |a| a.status = Status::Banned);
    let banned = t
        .post("/launcher/v1/auth/refresh", &[], json!({"refreshToken": refresh}))
        .await;
    assert_eq!(
        (banned.status, banned.code()),
        (StatusCode::FORBIDDEN, "AUTH_ACCOUNT_BANNED".into())
    );
    assert!(t.store().sessions().iter().all(|s| s.revoked_at.is_some()));

    let (_, refresh, _) = t.account("Slayer2", "slayer2@example.test").await;
    t.advance(31 * 24 * 3600 * 1000);
    let expired = t
        .post("/launcher/v1/auth/refresh", &[], json!({"refreshToken": refresh}))
        .await;
    assert_eq!(expired.code(), "AUTH_REFRESH_INVALID");
}

#[tokio::test]
async fn refresh_notes_the_sign_in() {
    let t = Harness::new().await;
    let (_, refresh, user_id) = t.account("Slayer1", "slayer1@example.test").await;
    let signed = t
        .store()
        .launcher_account_by_user_id(&user_id)
        .await
        .unwrap()
        .unwrap()
        .last_login_at;
    t.advance(60_000);
    let refreshed = t
        .post("/launcher/v1/auth/refresh", &[], json!({"refreshToken": refresh}))
        .await;
    let within = t
        .store()
        .launcher_account_by_user_id(&user_id)
        .await
        .unwrap()
        .unwrap()
        .last_login_at;
    assert_eq!(signed, within, "within five minutes nothing is written");
    t.advance(10 * 60_000);
    let next = refreshed.json()["refreshToken"].as_str().unwrap().to_owned();
    t.post("/launcher/v1/auth/refresh", &[], json!({"refreshToken": next}))
        .await;
    let later = t
        .store()
        .launcher_account_by_user_id(&user_id)
        .await
        .unwrap()
        .unwrap()
        .last_login_at;
    assert_eq!(later, Some(rustic_isles_auth::iso(t.now())));
}

#[tokio::test]
async fn logout_and_logout_all() {
    let t = Harness::new().await;
    let (access, _, _) = t.account("Slayer1", "slayer1@example.test").await;
    let out = t
        .post(
            "/launcher/v1/auth/logout",
            &[("authorization", &bearer(&access))],
            json!({}),
        )
        .await;
    assert_eq!((out.status, out.text.as_str()), (StatusCode::OK, "{}"));
    assert_eq!(
        t.get("/launcher/v1/me", &[("authorization", &bearer(&access))])
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    let one = t.login("slayer1@example.test", "correct horse").await.json()["accessToken"]
        .as_str()
        .unwrap()
        .to_owned();
    let two = t.login("slayer1@example.test", "correct horse").await.json()["accessToken"]
        .as_str()
        .unwrap()
        .to_owned();
    let all = t
        .post(
            "/launcher/v1/auth/logout-all",
            &[("authorization", &bearer(&one))],
            json!({}),
        )
        .await;
    assert_eq!(all.status, StatusCode::OK);
    assert_eq!(
        t.get("/launcher/v1/me", &[("authorization", &bearer(&two))])
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn policy_legal_status_notices() {
    let t = Harness::new().await;
    let (access, _, _) = t.account("Slayer1", "slayer1@example.test").await;
    let policy = t
        .get("/launcher/v1/policy", &[("authorization", &bearer(&access))])
        .await;
    let mut json = policy.json();
    assert!(json["policyVersion"].as_str().unwrap().ends_with(":ops-0"));
    json["policyVersion"] = json!("<iso>:ops-0");
    assert_eq!(
        serde_json::to_string(&json).unwrap(),
        r#"{"policyVersion":"<iso>:ops-0","roles":["player"],"channel":"stable","managedFeatureIds":[],"logUpload":{"auto":false},"guardEnforcement":"ENFORCE","diagnosticsProfile":"PRODUCTION"}"#
    );
    assert_eq!(
        t.get("/launcher/v1/legal", &[]).await.text,
        r#"{"termsVersion":"2026-10-14","privacyVersion":"2026-10-17","minimumAge":16}"#
    );
    assert_eq!(
        t.get("/launcher/v1/status", &[]).await.text,
        r#"{"online":true,"supportedBuildChangelist":647472,"officialPlay":{"available":true}}"#
    );
    t.settings.set("RUSTIC_ISLES_OFFICIAL_PLAY", "off");
    assert_eq!(
        t.get("/launcher/v1/status", &[]).await.json()["officialPlay"],
        json!({"available": false, "message": "The official server is currently unavailable. Join or host a realm."})
    );
    let notices = t.get("/launcher/v1/notices", &[]).await;
    assert_eq!(notices.header("cache-control"), Some("public, max-age=30"));
    assert_eq!(notices.json()["announcements"], json!([]));
}

#[tokio::test]
async fn consent() {
    let t = Harness::new().await;
    let (access, _, _) = t.account("Slayer1", "slayer1@example.test").await;
    let auth = bearer(&access);
    let path = "/launcher/v1/account/consent";
    let state = t.get(path, &[("authorization", &auth)]).await.json();
    assert_eq!(state["accepted"], Value::Null);
    assert_eq!(state["upToDate"], json!(false));
    let age = t
        .post(path, &[("authorization", &auth)], json!({"termsVersion": "2026-10-14"}))
        .await;
    assert_eq!(
        (age.status, age.code()),
        (StatusCode::BAD_REQUEST, "CONSENT_AGE_REQUIRED".into())
    );
    let missing = t
        .post(path, &[("authorization", &auth)], json!({"ageConfirmed": true}))
        .await;
    assert_eq!(
        (missing.status, missing.code()),
        (StatusCode::BAD_REQUEST, "AUTH_VALIDATION_FAILED".into())
    );
    let old = t
        .post(
            path,
            &[("authorization", &auth)],
            json!({"ageConfirmed": true, "termsVersion": "2020-01-01", "privacyVersion": "2026-10-17"}),
        )
        .await;
    assert_eq!(old.status, StatusCode::CONFLICT);
    let error = &old.json()["error"];
    let keys: Vec<&str> = error.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(keys, ["termsVersion", "privacyVersion", "code", "message", "requestId"]);
    assert_eq!(error["code"], json!("LEGAL_VERSION_OUTDATED"));
    let ok = t
        .post(
            path,
            &[("authorization", &auth)],
            json!({"ageConfirmed": true, "termsVersion": "2026-10-14", "privacyVersion": "2026-10-16"}),
        )
        .await;
    assert_eq!(ok.status, StatusCode::OK);
    assert_eq!(ok.json()["accepted"]["privacyVersion"], json!("2026-10-16"));
    assert_eq!(ok.json()["upToDate"], json!(true));
}

#[tokio::test]
async fn deletion_hold() {
    let t = Harness::new().await;
    let (access, _, _) = t.account("Slayer1", "slayer1@example.test").await;
    let auth = bearer(&access);
    let path = "/launcher/v1/account/deletion";
    assert_eq!(
        t.get(path, &[("authorization", &auth)]).await.text,
        r#"{"request":null,"holdDays":30}"#
    );
    for body in [json!({"confirmUsername": "nope"}), json!({})] {
        let answer = t.post(path, &[("authorization", &auth)], body).await;
        assert_eq!(
            (answer.status, answer.code()),
            (StatusCode::BAD_REQUEST, "DELETION_CONFIRMATION_MISMATCH".into())
        );
    }
    let first = t
        .post(
            path,
            &[("authorization", &auth)],
            json!({"confirmUsername": " slayer1 "}),
        )
        .await;
    assert_eq!(first.status, StatusCode::OK);
    t.advance(1000);
    let again = t
        .post(path, &[("authorization", &auth)], json!({"confirmUsername": "Slayer1"}))
        .await;
    assert_eq!(first.text, again.text);
    let launch = t
        .post(
            "/launcher/v1/game-sessions",
            &[("authorization", &auth)],
            Harness::launch_body(),
        )
        .await;
    assert_eq!(
        (launch.status, launch.code()),
        (StatusCode::FORBIDDEN, "ACCOUNT_DEACTIVATED".into())
    );
    let keys: Vec<String> = launch.json()["error"].as_object().unwrap().keys().cloned().collect();
    assert_eq!(keys[0], "scheduledFor");
    for _ in 0..2 {
        let answer = t.raw(Method::DELETE, path, &[("authorization", &auth)], None).await;
        assert_eq!(
            (answer.status, answer.text.as_str()),
            (StatusCode::OK, r#"{"request":null,"holdDays":30}"#)
        );
    }
    let user = t.get("/launcher/v1/me", &[("authorization", &auth)]).await.json()["userId"]
        .as_str()
        .unwrap()
        .to_owned();
    t.store().update_account(&user, |a| {
        a.deletion = Some(rustic_isles_auth::Deletion {
            requested_at: "2026-01-01T00:00:00.000Z".into(),
            scheduled_for: "2026-01-31T00:00:00.000Z".into(),
        });
    });
    let late = t.raw(Method::DELETE, path, &[("authorization", &auth)], None).await;
    assert_eq!(
        (late.status, late.code()),
        (StatusCode::CONFLICT, "DELETION_HOLD_EXPIRED".into())
    );
}

#[tokio::test]
async fn game_sessions() {
    let t = Harness::new().await;
    let (access, _, _) = t.account("Slayer1", "slayer1@example.test").await;
    let auth = bearer(&access);
    let path = "/launcher/v1/game-sessions";
    let none = t.post(path, &[], Harness::launch_body()).await;
    assert_eq!(none.status, StatusCode::UNAUTHORIZED);
    let with = |change: &dyn Fn(&mut Value)| {
        let mut body = Harness::launch_body();
        change(&mut body);
        body
    };
    let cases: Vec<(Value, u16, &str)> = vec![
        (
            with(&|b| b["runtimeChannel"] = json!("preview")),
            400,
            "runtimeChannel must be stable, beta, or dev.",
        ),
        (
            with(&|b| {
                b.as_object_mut().unwrap().remove("buildChangelist");
            }),
            400,
            "buildChangelist is required.",
        ),
        (
            with(&|b| {
                b.as_object_mut().unwrap().remove("executableSha256");
            }),
            400,
            "executableSha256 is required.",
        ),
        (
            with(&|b| b["runtimeChannel"] = json!("beta")),
            409,
            "Your account currently requires the stable runtime channel.",
        ),
        (
            with(&|b| b["buildChangelist"] = json!(1)),
            400,
            "This Dauntless build isn't supported. Verify or repair your installation.",
        ),
        (
            with(&|b| b["executableSha256"] = json!(h('d'))),
            400,
            "This Dauntless installation isn't recognized. Verify or repair your installation.",
        ),
        (
            with(&|b| b["runtimeManifestVersion"] = json!("0.1.77")),
            409,
            "The installed stable runtime set is not the approved release.",
        ),
        (
            with(&|b| b["runtimeArtifacts"][0]["sha256"] = json!(h('e'))),
            409,
            "The installed stable runtime set is not the approved release.",
        ),
        (
            with(&|b| {
                b["runtimeArtifacts"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"name": "extra.dll", "sha256": h('e')}))
            }),
            400,
            "The complete runtime artifact set is required.",
        ),
        (
            with(&|b| b["realm"] = json!({"id": "r1"})),
            501,
            "Realms are not available on this server.",
        ),
    ];
    for (body, status, message) in cases {
        let answer = t.post(path, &[("authorization", &auth)], body).await;
        assert_eq!((answer.status.as_u16(), answer.message()), (status, message.into()));
    }
    let ok = t.post(path, &[("authorization", &auth)], Harness::launch_body()).await;
    assert_eq!(ok.status, StatusCode::OK);
    let json = ok.json();
    assert_eq!(json["exchangeCode"].as_str().unwrap().len(), 64);
    assert_eq!(json["expiresInSeconds"], json!(60));
}

#[tokio::test]
async fn game_sessions_are_limited_and_need_a_name_and_official_play() {
    let t = Harness::new().await;
    t.register(None, "anon@example.test").await;
    let access = t.login("anon@example.test", "correct horse").await.json()["accessToken"]
        .as_str()
        .unwrap()
        .to_owned();
    let path = "/launcher/v1/game-sessions";
    let auth = bearer(&access);
    let nameless = t.post(path, &[("authorization", &auth)], Harness::launch_body()).await;
    assert_eq!(
        (nameless.status, nameless.code()),
        (StatusCode::FORBIDDEN, "AUTH_USERNAME_REQUIRED".into())
    );
    let (access, _, user_id) = t.account("Slayer1", "slayer1@example.test").await;
    let auth = bearer(&access);
    t.settings.set("RUSTIC_ISLES_OFFICIAL_PLAY", "off");
    let off = t.post(path, &[("authorization", &auth)], Harness::launch_body()).await;
    assert_eq!(
        (off.status, off.code()),
        (StatusCode::SERVICE_UNAVAILABLE, "OFFICIAL_PLAY_UNAVAILABLE".into())
    );
    t.settings.set("RUSTIC_ISLES_OFFICIAL_PLAY", "on");
    t.settings.set("RUSTIC_ISLES_CONSENT_ENFORCE", "on");
    let consent = t.post(path, &[("authorization", &auth)], Harness::launch_body()).await;
    assert_eq!(
        (consent.status, consent.code()),
        (StatusCode::FORBIDDEN, "CONSENT_REQUIRED".into())
    );
    t.settings.unset("RUSTIC_ISLES_CONSENT_ENFORCE");
    t.store().update_account(&user_id, |a| {
        a.timeout = Some(rustic_isles_auth::Timeout {
            until: "2999-01-01T00:00:00.000Z".into(),
            since: "2000-01-01T00:00:00.000Z".into(),
            reason: None,
        });
    });
    let timed_out = t.post(path, &[("authorization", &auth)], Harness::launch_body()).await;
    assert_eq!(
        (timed_out.status, timed_out.code()),
        (StatusCode::INTERNAL_SERVER_ERROR, "AUTH_ACCOUNT_TIMED_OUT".into())
    );
    t.store().update_account(&user_id, |a| a.timeout = None);
    for _ in 0..10 {
        t.post(path, &[("authorization", &auth)], Harness::launch_body()).await;
    }
    let limited = t.post(path, &[("authorization", &auth)], Harness::launch_body()).await;
    assert_eq!(
        (limited.status, limited.message()),
        (
            StatusCode::TOO_MANY_REQUESTS,
            "Too many launch attempts. Try again in a few minutes.".into()
        )
    );
}

#[tokio::test]
async fn approval_required() {
    let t = Harness::with(&[("RUSTIC_ISLES_REGISTRATION_APPROVAL", "requried")], false).await;
    let answer = t.register(Some("Slayer1"), "slayer1@example.test").await;
    assert_eq!(answer.json()["approvalRequired"], json!(true));
    assert_eq!(answer.json()["account"]["approvalStatus"], json!("pending"));
    let login = t.login("slayer1@example.test", "correct horse").await;
    assert_eq!(
        (login.status, login.code()),
        (StatusCode::FORBIDDEN, "AUTH_APPROVAL_PENDING".into())
    );
    t.settings.set("RUSTIC_ISLES_REGISTRATION_APPROVAL", " OFF ");
    let next = t.register(Some("Slayer2"), "slayer2@example.test").await;
    assert_eq!(next.json()["account"]["approvalStatus"], json!("approved"));
    let user = answer.json()["account"]["userId"].as_str().unwrap().to_owned();
    t.store()
        .update_account(&user, |a| a.approval = Some(Approval::Rejected));
    let rejected = t.login("slayer1@example.test", "correct horse").await;
    assert_eq!(rejected.code(), "AUTH_APPROVAL_REJECTED");
}

#[tokio::test]
async fn registration_consent_follows_the_engine() {
    let postgres = Harness::with(&[], true).await;
    let refused = postgres.register(Some("Slayer1"), "s1@example.test").await;
    assert_eq!(
        (refused.status, refused.code()),
        (StatusCode::BAD_REQUEST, "CONSENT_REQUIRED".into())
    );
    let body = json!({"email": "s1@example.test", "password": "correct horse", "displayName": "Slayer1",
        "consent": {"ageConfirmed": true, "termsVersion": "2026-10-14", "privacyVersion": "2026-10-15"}});
    let ok = postgres.post("/launcher/v1/auth/register", &[], body).await;
    assert_eq!(ok.status, StatusCode::ACCEPTED);
    let user = ok.json()["account"]["userId"].as_str().unwrap().to_owned();
    let consent = postgres
        .store()
        .launcher_account_by_user_id(&user)
        .await
        .unwrap()
        .unwrap()
        .consent
        .unwrap();
    assert_eq!(consent.privacy_version, "2026-10-15");
    let sqlite = Harness::new().await;
    assert_eq!(
        sqlite.register(Some("Slayer1"), "s1@example.test").await.status,
        StatusCode::ACCEPTED
    );
}

#[tokio::test]
async fn cors() {
    let t = Harness::new().await;
    let allowed = t
        .raw(
            Method::OPTIONS,
            "/launcher/v1/auth/login",
            &[("origin", "http://localhost:1420")],
            None,
        )
        .await;
    assert_eq!(allowed.status, StatusCode::NO_CONTENT);
    assert_eq!(
        allowed.header("access-control-allow-origin"),
        Some("http://localhost:1420")
    );
    assert_eq!(allowed.header("vary"), Some("Origin"));
    assert_eq!(
        allowed.header("access-control-allow-methods"),
        Some("GET, POST, DELETE, OPTIONS")
    );
    assert_eq!(
        allowed.header("access-control-allow-headers"),
        Some("Content-Type, Authorization")
    );
    let evil = t
        .raw(
            Method::OPTIONS,
            "/launcher/v1/auth/login",
            &[("origin", "https://evil.example")],
            None,
        )
        .await;
    assert_eq!((evil.status, evil.text.as_str()), (StatusCode::FORBIDDEN, ""));
    let bare = t.raw(Method::OPTIONS, "/launcher/v1/auth/login", &[], None).await;
    assert_eq!(
        (bare.status, bare.header("access-control-allow-origin")),
        (StatusCode::NO_CONTENT, None)
    );
    let unrouted = t
        .raw(
            Method::OPTIONS,
            "/launcher/v1/nothing",
            &[("origin", "https://tauri.localhost")],
            None,
        )
        .await;
    assert_eq!(
        (unrouted.status, unrouted.header("access-control-allow-origin")),
        (StatusCode::NO_CONTENT, Some("https://tauri.localhost"))
    );
    let legal = t
        .get("/launcher/v1/legal", &[("origin", "http://localhost:1420")])
        .await;
    assert_eq!((legal.status, legal.header("vary")), (StatusCode::OK, Some("Origin")));
    assert_eq!(
        t.get("/launcher/v1/legal", &[("origin", "https://evil.example")])
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    let game = t.get("/isbanned", &[("origin", "https://evil.example")]).await;
    assert_eq!(
        (game.status, game.header("access-control-allow-origin")),
        (StatusCode::OK, None)
    );
    t.settings.set("LAUNCHER_ALLOWED_ORIGINS", " https://app.example , ");
    assert_eq!(
        t.get("/launcher/v1/legal", &[("origin", "https://app.example")])
            .await
            .status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn codes_need_mail() {
    let t = Harness::new().await;
    let (access, _, _) = t.account("Slayer1", "slayer1@example.test").await;
    for path in ["/launcher/v1/auth/email/resend", "/launcher/v1/auth/password/forgot"] {
        let answer = t.post(path, &[], json!({"email": "slayer1@example.test"})).await;
        assert_eq!(
            (answer.status, answer.code()),
            (StatusCode::SERVICE_UNAVAILABLE, "AUTH_EMAIL_UNAVAILABLE".into())
        );
    }
    // Unlink checks the link first: no link is 403 before the mail check
    let unlink = t
        .post(
            "/launcher/v1/account/discord/unlink/start",
            &[("authorization", &bearer(&access))],
            json!({}),
        )
        .await;
    assert_eq!(unlink.code(), "AUTH_DISCORD_NOT_LINKED");
}

#[tokio::test]
async fn verification_flow() {
    let t = Harness::new().await;
    t.smtp_on();
    t.settings.set("RUSTIC_ISLES_EMAIL_VERIFICATION", "required");
    let register = t.register(Some("Slayer4"), "slayer4@example.test").await;
    assert_eq!(register.status, StatusCode::ACCEPTED);
    let json = register.json();
    assert_eq!(
        (json["verificationRequired"].clone(), json["email"].clone()),
        (json!(true), json!("slayer4@example.test"))
    );
    let keys: Vec<&str> = json.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(keys, ["approvalRequired", "verificationRequired", "email", "account"]);
    for _ in 0..2 {
        let login = t.login("slayer4@example.test", "correct horse").await;
        assert_eq!(
            (login.status, login.code()),
            (StatusCode::FORBIDDEN, "AUTH_EMAIL_UNVERIFIED".into())
        );
        assert!(login.message().contains("s***4@example.test"));
    }
    assert_eq!(
        t.login("slayer4@example.test", "wrong horse").await.status,
        StatusCode::UNAUTHORIZED
    );
    let code = t.mail.last_code("slayer4@example.test").unwrap();
    let only_its_hash = t.store().email_codes();
    assert!(only_its_hash.iter().all(|c| !c.code_hash.contains(&code)));
    let path = "/launcher/v1/auth/email/verify";
    let missing = t
        .post(path, &[], json!({"email": "slayer4@example.test", "code": code}))
        .await;
    assert_eq!(missing.status, StatusCode::UNAUTHORIZED);
    let wrong = t
        .post(
            path,
            &[],
            json!({"email": "slayer4@example.test", "password": "wrong horse", "code": code}),
        )
        .await;
    assert_eq!(wrong.status, StatusCode::UNAUTHORIZED);
    let shape = t
        .post(
            path,
            &[],
            json!({"email": "slayer4@example.test", "password": "correct horse", "code": "12ab"}),
        )
        .await;
    assert_eq!(
        (shape.status, shape.code()),
        (StatusCode::BAD_REQUEST, "AUTH_CODE_INVALID".into())
    );
    let ok = t
        .post(
            path,
            &[],
            json!({"email": "slayer4@example.test", "password": "correct horse", "code": code}),
        )
        .await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.text);
    assert_eq!(ok.json()["account"]["emailVerified"], json!(true));
    let again = t
        .post(
            path,
            &[],
            json!({"email": "slayer4@example.test", "password": "correct horse", "code": code}),
        )
        .await;
    assert_eq!(again.status, StatusCode::BAD_REQUEST);
    let access = ok.json()["accessToken"].as_str().unwrap().to_owned();
    assert_eq!(
        t.get("/launcher/v1/me", &[("authorization", &bearer(&access))])
            .await
            .status,
        StatusCode::OK
    );
}

#[tokio::test]
async fn resend_forgot_and_reset() {
    let t = Harness::new().await;
    t.smtp_on();
    let (old_access, _, _) = t.account("Slayer1", "slayer1@example.test").await;
    let resend = "/launcher/v1/auth/email/resend";
    let bad = t.post(resend, &[], json!({"email": "nope"})).await;
    assert_eq!(bad.status, StatusCode::BAD_REQUEST);
    for email in ["slayer1@example.test", "nobody@example.test"] {
        let answer = t.post(resend, &[], json!({"email": email})).await;
        assert_eq!(
            (answer.status, answer.text.as_str()),
            (StatusCode::OK, r#"{"sent":true}"#)
        );
    }
    assert!(t.mail.wait_for(1).await, "the unverified account gets a code");
    let forgot = t
        .post(
            "/launcher/v1/auth/password/forgot",
            &[],
            json!({"email": "slayer1@example.test"}),
        )
        .await;
    assert_eq!(forgot.text, r#"{"sent":true}"#);
    let blank = t
        .post("/launcher/v1/auth/password/forgot", &[], json!({"email": ""}))
        .await;
    assert_eq!(blank.status, StatusCode::BAD_REQUEST);
    assert!(t.mail.wait_for(2).await);
    let code = t.mail.last_code("slayer1@example.test").unwrap();
    assert!(
        t.mail
            .messages()
            .last()
            .unwrap()
            .subject
            .starts_with("Your Rustic Isles password reset code")
    );
    let reset = "/launcher/v1/auth/password/reset";
    let short = t
        .post(
            reset,
            &[],
            json!({"email": "slayer1@example.test", "code": code, "newPassword": "short"}),
        )
        .await;
    assert_eq!(short.message(), "Password must be at least 8 characters.");
    let wrong_code = if code == "000000" { "111111" } else { "000000" };
    let wrong = t
        .post(
            reset,
            &[],
            json!({"email": "slayer1@example.test", "code": wrong_code, "newPassword": "new horse 1"}),
        )
        .await;
    assert_eq!(wrong.code(), "AUTH_CODE_INVALID");
    let nobody = t
        .post(
            reset,
            &[],
            json!({"email": "nobody@example.test", "code": code, "newPassword": "new horse 1"}),
        )
        .await;
    assert_eq!(nobody.code(), "AUTH_CODE_INVALID");
    let ok = t
        .post(
            reset,
            &[],
            json!({"email": "slayer1@example.test", "code": code, "newPassword": "new horse 1"}),
        )
        .await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.text);
    assert_eq!(
        t.get("/launcher/v1/me", &[("authorization", &bearer(&old_access))])
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        t.login("slayer1@example.test", "new horse 1").await.status,
        StatusCode::OK
    );
    assert_eq!(
        t.login("slayer1@example.test", "correct horse").await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn discord_unconfigured_and_callback_refusals() {
    let t = Harness::new().await;
    let (access, _, _) = t.account("Slayer1", "slayer1@example.test").await;
    let start = t.post("/launcher/v1/auth/discord/start", &[], json!({})).await;
    assert_eq!(
        (start.status, start.code()),
        (StatusCode::SERVICE_UNAVAILABLE, "AUTH_DISCORD_NOT_CONFIGURED".into())
    );
    let link = t
        .post(
            "/launcher/v1/account/discord/link/start",
            &[("authorization", &bearer(&access))],
            json!({}),
        )
        .await;
    assert_eq!(link.code(), "AUTH_DISCORD_NOT_CONFIGURED");
    let error = "rusticisles://auth/error?code=AUTH_DISCORD_CANCELLED";
    for query in ["", "?code=c&state=unknown", "?code=a&code=b&state=s"] {
        let answer = t.get(&format!("/launcher/v1/auth/discord/callback{query}"), &[]).await;
        assert_eq!(answer.status, StatusCode::FOUND);
        assert_eq!(answer.header("location"), Some(error));
        assert_eq!(answer.text, format!("Found. Redirecting to {error}"));
        assert_eq!(answer.header("vary"), Some("Accept"));
    }
    let html = t
        .get(
            "/launcher/v1/auth/discord/callback",
            &[("accept", "text/html,application/xhtml+xml,*/*;q=0.8")],
        )
        .await;
    assert_eq!(html.text, format!("<p>Found. Redirecting to {error}</p>"));
    assert_eq!(html.header("content-type"), Some("text/html; charset=utf-8"));
}

fn state_of(url: &str) -> String {
    url.split("state=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn discord_link_sign_in_and_unlink() {
    let t = Harness::new().await;
    t.discord_on();
    let (access, _, _) = t.account("Slayer1", "slayer1@example.test").await;
    let (access_b, _, _) = t.account("Slayer2", "slayer2@example.test").await;
    let auth = bearer(&access);
    let callback = |code: &str, state: &str| format!("/launcher/v1/auth/discord/callback?code={code}&state={state}");

    let start = t.post("/launcher/v1/auth/discord/start", &[], json!({})).await;
    let url = start.json()["authorizeUrl"].as_str().unwrap().to_owned();
    assert!(
        url.starts_with("https://discord.com/api/oauth2/authorize?client_id=client&redirect_uri=https%3A%2F%2Fisles")
    );
    assert!(url.contains("&response_type=code&scope=identify&state=") && url.ends_with("&code_challenge_method=S256"));
    assert_eq!(state_of(&url).len(), 48);
    let unlinked = t.get(&callback("code-1230001", &state_of(&url)), &[]).await;
    assert_eq!(
        unlinked.header("location"),
        Some("rusticisles://auth/error?code=AUTH_DISCORD_NOT_LINKED")
    );

    let link = t
        .post(
            "/launcher/v1/account/discord/link/start",
            &[("authorization", &auth)],
            json!({}),
        )
        .await;
    let state = state_of(link.json()["authorizeUrl"].as_str().unwrap());
    let failed = t.get(&callback("bad", &state), &[]).await;
    assert_eq!(
        failed.header("location"),
        Some("rusticisles://link/error?code=AUTH_DISCORD_CANCELLED")
    );
    let link = t
        .post(
            "/launcher/v1/account/discord/link/start",
            &[("authorization", &auth)],
            json!({}),
        )
        .await;
    let state = state_of(link.json()["authorizeUrl"].as_str().unwrap());
    let done = t.get(&callback("code-1230001", &state), &[]).await;
    assert_eq!(done.header("location"), Some("rusticisles://link/done"));
    let replayed = t.get(&callback("code-1230001", &state), &[]).await;
    assert_eq!(
        replayed.header("location"),
        Some("rusticisles://auth/error?code=AUTH_DISCORD_CANCELLED")
    );
    let me = t.get("/launcher/v1/me", &[("authorization", &auth)]).await.json();
    assert_eq!(me["discord"], json!({"linked": true, "username": "user-0001"}));
    assert_eq!(me["discordLinked"], json!(true));
    let twice = t
        .post(
            "/launcher/v1/account/discord/link/start",
            &[("authorization", &auth)],
            json!({}),
        )
        .await;
    assert_eq!(
        (twice.status, twice.code()),
        (StatusCode::CONFLICT, "AUTH_DISCORD_ACCOUNT_HAS_LINK".into())
    );
    let link_b = t
        .post(
            "/launcher/v1/account/discord/link/start",
            &[("authorization", &bearer(&access_b))],
            json!({}),
        )
        .await;
    let state_b = state_of(link_b.json()["authorizeUrl"].as_str().unwrap());
    let taken = t.get(&callback("code-1230001", &state_b), &[]).await;
    assert_eq!(
        taken.header("location"),
        Some("rusticisles://link/error?code=AUTH_DISCORD_ALREADY_LINKED")
    );

    let start = t.post("/launcher/v1/auth/discord/start", &[], json!({})).await;
    let state = state_of(start.json()["authorizeUrl"].as_str().unwrap());
    let signed = t.get(&callback("code-1230001", &state), &[]).await;
    let location = signed.header("location").unwrap().to_owned();
    let completion = location
        .strip_prefix("rusticisles://auth/complete?code=")
        .unwrap()
        .to_owned();
    assert_eq!(completion.len(), 64);
    let complete = t
        .post("/launcher/v1/auth/discord/complete", &[], json!({"code": completion}))
        .await;
    assert_eq!(complete.status, StatusCode::OK);
    assert_eq!(complete.json()["account"]["displayName"], json!("Slayer1"));
    let again = t
        .post("/launcher/v1/auth/discord/complete", &[], json!({"code": completion}))
        .await;
    assert_eq!(again.message(), "That sign-in link has expired. Please try again.");
    let empty = t.post("/launcher/v1/auth/discord/complete", &[], json!({})).await;
    assert_eq!(empty.message(), "Discord sign-in didn't complete. Please try again.");

    let unavailable = t
        .post(
            "/launcher/v1/account/discord/unlink/start",
            &[("authorization", &auth)],
            json!({}),
        )
        .await;
    assert_eq!(unavailable.code(), "AUTH_EMAIL_UNAVAILABLE");
    t.smtp_on();
    let start = t
        .post(
            "/launcher/v1/account/discord/unlink/start",
            &[("authorization", &auth)],
            json!({}),
        )
        .await;
    assert_eq!(start.text, r#"{"sent":true,"email":"s***1@example.test"}"#);
    let code = t.mail.last_code("slayer1@example.test").unwrap();
    let wrong_code = if code == "000000" { "111111" } else { "000000" };
    let wrong = t
        .post(
            "/launcher/v1/account/discord/unlink",
            &[("authorization", &auth)],
            json!({"code": wrong_code}),
        )
        .await;
    assert_eq!(wrong.code(), "AUTH_CODE_INVALID");
    let done = t
        .post(
            "/launcher/v1/account/discord/unlink",
            &[("authorization", &auth)],
            json!({"code": code}),
        )
        .await;
    assert_eq!(done.status, StatusCode::OK);
    assert_eq!(done.json()["account"]["discordLinked"], json!(false));
    let gone = t
        .post(
            "/launcher/v1/account/discord/unlink/start",
            &[("authorization", &auth)],
            json!({}),
        )
        .await;
    assert_eq!(
        (gone.status, gone.code()),
        (StatusCode::FORBIDDEN, "AUTH_DISCORD_NOT_LINKED".into())
    );
}
