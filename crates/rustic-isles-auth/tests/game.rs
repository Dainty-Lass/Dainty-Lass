// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The game client's routes and game auth black-box, from spec §12.2 and §12.3.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use axum::extract::Request;
use axum::http::{Method, StatusCode};
use common::{GS_KEY, Harness, bearer};
use rustic_isles_auth::{AuthData, AuthStore, Status};
use serde_json::{Value, json};

const KEY: &str = "x-rustic-isles-gameserver-apikey";

#[tokio::test]
async fn stubs() {
    let t = Harness::new().await;
    assert_eq!(
        t.get("/features/platform/win", &[]).await.text,
        r#"{"code":null,"message":"OK","payload":{"crossplay":true,"crossprogression":true}}"#
    );
    assert_eq!(
        t.get("/account/link/epic/anyone", &[]).await.text,
        r#"{"code":null,"message":"OK","payload":{"isLinked":true}}"#
    );
    assert_eq!(t.get("/isbanned", &[]).await.text, r#"{"isBanned":false}"#);
    assert_eq!(
        t.raw(Method::DELETE, "/account/api/oauth/sessions/kill/abc", &[], None)
            .await
            .text,
        "{}"
    );
}

#[tokio::test]
async fn oauth_token_and_verify() {
    let t = Harness::new().await;
    let path = "/account/api/oauth/token";
    let none = t.raw(Method::POST, path, &[], None).await;
    assert_eq!(none.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(none.text.contains("<pre>Internal Server Error</pre>"));
    let no_code = t.post(path, &[], json!({"grant_type": "exchange_code"})).await;
    assert_eq!((no_code.status, no_code.text.as_str()), (StatusCode::BAD_REQUEST, ""));
    for body in [
        Some(("application/json", r#"{"grant_type":"client_credentials"}"#.to_owned())),
        Some((
            "application/x-www-form-urlencoded",
            "grant_type=client_credentials".to_owned(),
        )),
    ] {
        let answer = t.raw(Method::POST, path, &[], body).await;
        assert_eq!(answer.status, StatusCode::OK);
        let keys: Vec<String> = answer.json().as_object().unwrap().keys().cloned().collect();
        assert_eq!(
            keys,
            [
                "access_token",
                "token_type",
                "expires_at",
                "features",
                "organization_id",
                "product_id",
                "sandbox_id",
                "deployment_id",
                "expires_in"
            ]
        );
    }
    let unknown = t.post(path, &[], json!({"exchange_code": "nope"})).await;
    assert_eq!((unknown.status, unknown.text.as_str()), (StatusCode::UNAUTHORIZED, ""));

    let (access, _, user_id) = t.account("Slayer1", "slayer1@example.test").await;
    let launch = t
        .post(
            "/launcher/v1/game-sessions",
            &[("authorization", &bearer(&access))],
            Harness::launch_body(),
        )
        .await;
    let code = launch.json()["exchangeCode"].as_str().unwrap().to_owned();
    let login = t.post(path, &[], json!({"exchange_code": code})).await;
    assert_eq!(login.status, StatusCode::OK);
    let json = login.json();
    assert_eq!(json["account_id"], json!(user_id));
    assert_eq!(json["refresh_token"], json!("refresh.token.lol"));
    assert_eq!(json["expires_at"], json!("2085-09-09T01:01:01.703Z"));
    let again = t.post(path, &[], json!({"exchange_code": code})).await;
    assert_eq!(again.status, StatusCode::UNAUTHORIZED);

    let token = json["access_token"].as_str().unwrap().to_owned();
    let verify = t
        .get("/account/api/oauth/verify", &[("authorization", &bearer(&token))])
        .await;
    assert_eq!(
        verify.text,
        format!(
            r#"{{"active":true,"scope":"basic_profile friends_list presence","token_type":"bearer","expires_in":86400,"expires_at":"2085-09-09T01:01:01.703Z","account_id":"{user_id}","client_id":"xyza7891lhxMVYGCON7LgnKZZ8HQGD5H","application_id":"fghi4567O03HROxEjwbn7kgXpBhnhWwv"}}"#
        )
    );
    for headers in [
        vec![("authorization", bearer(&access))],
        vec![],
        vec![("authorization", "Bearer a b".to_owned())],
    ] {
        let headers: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        assert_eq!(
            t.get("/account/api/oauth/verify", &headers).await.status,
            StatusCode::UNAUTHORIZED
        );
    }
}

#[tokio::test]
async fn exchange_rechecks_the_account_and_runtime() {
    let t = Harness::new().await;
    let (access, _, user_id) = t.account("Slayer1", "slayer1@example.test").await;
    async fn launch(t: &Harness, access: &str) -> String {
        let auth = bearer(access);
        let answer = t
            .post(
                "/launcher/v1/game-sessions",
                &[("authorization", &auth)],
                Harness::launch_body(),
            )
            .await;
        answer.json()["exchangeCode"].as_str().unwrap().to_owned()
    }
    let code = launch(&t, &access).await;
    t.store().update_account(&user_id, |a| a.status = Status::Banned);
    let banned = t
        .post("/account/api/oauth/token", &[], json!({"exchange_code": code}))
        .await;
    assert_eq!(banned.status, StatusCode::FORBIDDEN);
    t.store().update_account(&user_id, |a| a.status = Status::Active);
    let code = launch(&t, &access).await;
    t.advance(61_000);
    let expired = t
        .post("/account/api/oauth/token", &[], json!({"exchange_code": code}))
        .await;
    assert_eq!(expired.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn public_accounts() {
    let t = Harness::new().await;
    let (access, _, user_a) = t.account("Slayer1", "slayer1@example.test").await;
    let (_, _, user_b) = t.account("Slayer2", "slayer2@example.test").await;
    let token = t.game_token(&access).await;
    let auth = bearer(&token);
    let a = t.get(&format!("/account/api/public/account/{user_a}"), &[]).await;
    assert_eq!(
        a.text,
        format!(r#"{{"id":"{user_a}","displayName":"Slayer1","externalAuths":{{}}}}"#)
    );
    assert_eq!(
        t.get(
            &format!("/account/api/public/account/{user_b}"),
            &[("authorization", &auth)]
        )
        .await
        .status,
        StatusCode::OK
    );
    assert_eq!(
        t.get("/account/api/public/account/nobody", &[]).await.status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        t.get("/account/api/public/account/dev-user", &[("authorization", &auth)])
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        t.get(&format!("/account/api/public/account/{user_a}/externalAuths"), &[])
            .await
            .text,
        "[]"
    );
    assert_eq!(
        t.get("/account/api/public/account/nobody/externalAuths", &[])
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    let by_name = t
        .get("/account/api/public/account/displayName/%20SLAYER1%20", &[])
        .await;
    assert_eq!(by_name.json()["id"], json!(user_a));
    assert_eq!(
        t.get("/account/api/public/account/displayName/Nobody", &[])
            .await
            .status,
        StatusCode::NOT_FOUND
    );

    assert_eq!(
        t.get("/account/api/public/account", &[]).await.status,
        StatusCode::UNAUTHORIZED
    );
    let bulk = t
        .get(
            &format!("/account/api/public/account?accountId={user_b}&accountId={user_a}&accountId=nobody"),
            &[("authorization", &auth)],
        )
        .await;
    let ids: Vec<Value> = bulk
        .json()
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["id"].clone())
        .collect();
    assert_eq!(ids, [json!(user_b), json!(user_a)]);
    let me = t.get("/account/api/public/account", &[("authorization", &auth)]).await;
    assert_eq!(me.json()[0]["id"], json!(user_a));
    assert_eq!(
        t.get(
            "/account/api/public/account?accountId=dev-user",
            &[("authorization", &auth)]
        )
        .await
        .text,
        "[]"
    );
}

#[tokio::test]
async fn game_login_and_auth_matrix() {
    let t = Harness::new().await;
    let (access, _, user_id) = t.account("Slayer1", "slayer1@example.test").await;
    let token = t.game_token(&access).await;
    let own = json!({"email": user_id});
    type Case<'a> = (Vec<(&'a str, String)>, Option<Value>, u16);
    let cases: Vec<Case> = vec![
        (vec![("authorization", bearer(&token))], Some(own.clone()), 200),
        (
            vec![("authorization", bearer(&token))],
            Some(json!({"email": "someone-else"})),
            400,
        ),
        (vec![("authorization", bearer(&token))], None, 500),
        (vec![], Some(own.clone()), 401),
        (vec![("authorization", "Bearer garbage".into())], Some(own.clone()), 401),
        (vec![("authorization", bearer(&access))], Some(own.clone()), 401),
        (
            vec![("authorization", format!("bearer {token}"))],
            Some(own.clone()),
            200,
        ),
        (
            vec![("authorization", format!("BeArEr {token}"))],
            Some(own.clone()),
            401,
        ),
        (vec![(KEY, GS_KEY.into())], Some(own.clone()), 400),
        (vec![(KEY, "wrong".into())], Some(own.clone()), 401),
        (
            vec![(KEY, GS_KEY.into()), ("authorization", bearer(&token))],
            Some(own.clone()),
            200,
        ),
        (
            vec![(KEY, GS_KEY.into()), ("authorization", "Bearer garbage".into())],
            Some(own.clone()),
            500,
        ),
    ];
    for (i, (headers, body, status)) in cases.into_iter().enumerate() {
        let headers: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let body = body.map(|b| ("application/json", b.to_string()));
        let answer = t.raw(Method::POST, "/login", &headers, body).await;
        assert_eq!(answer.status.as_u16(), status, "case {i}: {}", answer.text);
        if status == 200 {
            assert_eq!(
                answer.text,
                r#"{"error_code":"TicketRateOk","message":"","state":"OPEN","timeout":8000,"title":""}"#
            );
        }
    }
}

#[tokio::test]
async fn account_reads() {
    let t = Harness::new().await;
    let (access, _, user_id) = t.account("Slayer1", "slayer1@example.test").await;
    let (_, _, user_b) = t.account("Slayer2", "slayer2@example.test").await;
    let token = t.game_token(&access).await;
    let auth = bearer(&token);
    let info = t.get("/accountinfo", &[("authorization", &auth)]).await;
    assert_eq!(
        info.text,
        format!(
            r#"{{"accountId":"{user_id}","creationDate":"2000-01-01 00:00:00","email":null,"preferredLanguage":null,"username":"Slayer1","verified":true}}"#
        )
    );
    assert_eq!(
        t.get("/accountinfo", &[(KEY, GS_KEY)]).await.status,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        t.get("/tags", &[("authorization", &auth)]).await.text,
        format!(r#"{{"accountId":"{user_id}","tags":[]}}"#)
    );
    assert_eq!(t.get("/tags", &[(KEY, GS_KEY)]).await.text, r#"{"tags":[]}"#);
    for path in ["/gamesession/epic", "/gamesession/epiceos"] {
        let session = t.raw(Method::PUT, path, &[("authorization", &auth)], None).await;
        assert_eq!(
            session.text,
            format!(
                r#"{{"code":null,"message":"OK","payload":{{"error_code":null,"sessionid":"{token}","sessiontoken":"{token}","sessionToken":"{token}"}}}}"#
            )
        );
    }
    assert_eq!(
        t.raw(Method::PUT, "/gamesession/epic", &[(KEY, GS_KEY)], None)
            .await
            .status,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    let public = |body: Option<Value>, headers: Vec<(&'static str, String)>| {
        let t = &t;
        async move {
            let headers: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
            t.raw(
                Method::POST,
                "/accountinfo/public",
                &headers,
                body.map(|b| ("application/json", b.to_string())),
            )
            .await
        }
    };
    let b = public(
        Some(json!({"accountId": user_b})),
        vec![("authorization", auth.clone())],
    )
    .await;
    assert_eq!(
        b.text,
        format!(
            r#"{{"accountId":"{user_b}","isSubscribed":true,"language":null,"linkedAccounts":[{{"accountId":"{user_b}","accountType":"epic"}}],"username":"Slayer2"}}"#
        )
    );
    let nobody = public(
        Some(json!({"accountId": "nobody"})),
        vec![("authorization", auth.clone())],
    )
    .await;
    assert_eq!(nobody.json()["username"], json!("nobody"));
    let alias = public(
        Some(json!({"accountId": "mystpax"})),
        vec![("authorization", auth.clone())],
    )
    .await;
    assert_eq!(
        (alias.json()["accountId"].clone(), alias.json()["username"].clone()),
        (json!("mystpax"), json!("Slayer1"))
    );
    assert_eq!(
        public(Some(json!({})), vec![("authorization", auth.clone())])
            .await
            .status,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        public(None, vec![("authorization", auth.clone())]).await.status,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(public(Some(json!({})), vec![]).await.status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn encryption_keys() {
    let t = Harness::new().await;
    let (access, _, _) = t.account("Slayer1", "slayer1@example.test").await;
    let token = t.game_token(&access).await;
    let auth = bearer(&token);
    let generated = t
        .post(
            "/key/generate",
            &[("authorization", &auth)],
            json!({"candidateId": "cand-1"}),
        )
        .await;
    let json = generated.json();
    assert_eq!(json["key"].as_str().unwrap().len(), 64);
    assert_eq!(json["nonce"].as_str().unwrap().len(), 43);
    assert_eq!(json["token"], json!(auth));
    assert_eq!(
        t.post("/key/generate", &[("authorization", &auth)], json!({}))
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        t.post("/key/generate", &[], json!({})).await.status,
        StatusCode::UNAUTHORIZED
    );
    let consume = |body: Option<Value>, query: &str| {
        let t = &t;
        let path = format!("/key/consume{query}");
        async move {
            t.raw(
                Method::POST,
                &path,
                &[(KEY, GS_KEY)],
                body.map(|b| ("application/json", b.to_string())),
            )
            .await
        }
    };
    let by_candidate = consume(Some(json!({"candidateId": "cand-1"})), "").await;
    assert_eq!(by_candidate.json()["key"], json["key"]);
    let by_token = consume(Some(json!({"token": auth})), "").await;
    assert_eq!(by_token.status, StatusCode::OK);
    let by_query = consume(None, &format!("?token={token}")).await;
    assert_eq!(by_query.status, StatusCode::OK);
    assert_eq!(
        consume(Some(json!({"token": "unknown"})), "").await.status,
        StatusCode::OK
    );
    assert_eq!(consume(None, "").await.status, StatusCode::OK);
}

#[tokio::test]
async fn no_key_answers_404() {
    let t = Harness::new().await;
    let answer = t.raw(Method::POST, "/key/consume", &[(KEY, GS_KEY)], None).await;
    assert_eq!(
        (answer.status, answer.text.as_str()),
        (StatusCode::NOT_FOUND, r#"{"error":"no issued key"}"#)
    );
}

fn get(uri: &str, headers: &[(&str, &str)]) -> Request {
    let mut builder = axum::http::Request::builder().method(Method::GET).uri(uri);
    for (k, v) in headers {
        builder = builder.header(*k, *v);
    }
    builder.body(axum::body::Body::empty()).unwrap()
}

#[tokio::test]
async fn game_middleware() {
    let t = Harness::new().await;
    let mut request = get("/loadout/INVALID//all?x=1", &[(KEY, GS_KEY)]);
    let data = t.auth.game_auth(&mut request).await.unwrap();
    assert_eq!(request.uri().to_string(), "/loadout/mystpax//all?x=1");
    assert!(data.is_gameserver && data.user_id.is_none());
    assert_eq!(request.extensions().get::<AuthData>(), Some(&data));
    t.settings.set("GAMESERVER_NO_PLAYER_USER_ID", " some user ");
    let mut request = get("/loadout/INVALID//all?x=1", &[(KEY, GS_KEY)]);
    let data = t.auth.game_auth(&mut request).await.unwrap();
    assert_eq!(request.uri().to_string(), "/loadout/some%20user//all?x=1");
    assert_eq!(data.user_id.as_deref(), Some("some user"));
    let mut post = axum::http::Request::builder()
        .method(Method::POST)
        .uri("/loadout/INVALID")
        .header(KEY, GS_KEY)
        .body(axum::body::Body::empty())
        .unwrap();
    t.auth.game_auth(&mut post).await.unwrap();
    assert_eq!(post.uri().path(), "/loadout/INVALID");
    t.settings.unset("GAMESERVER_NO_PLAYER_USER_ID");
    t.settings.set("DEV_USER_ID", "dev-account");
    let mut request = get("/x", &[(KEY, GS_KEY)]);
    assert_eq!(
        t.auth.game_auth(&mut request).await.unwrap().user_id.as_deref(),
        Some("dev-account")
    );
    let mut wrong = get("/x", &[(KEY, "nope")]);
    assert_eq!(
        t.auth.game_auth(&mut wrong).await.unwrap_err().status(),
        StatusCode::UNAUTHORIZED
    );

    let (access, _, user_id) = t.account("Slayer1", "slayer1@example.test").await;
    let token = t.game_token(&access).await;
    t.store().update_account(&user_id, |a| a.status = Status::Banned);
    let mut banned = get("/x", &[("authorization", &bearer(&token))]);
    assert_eq!(
        t.auth.game_auth(&mut banned).await.unwrap_err().status(),
        StatusCode::FORBIDDEN
    );
    let service = t.auth.sign_game_token("__launcher_client_credentials__").unwrap();
    let mut ok = get("/x", &[("authorization", &bearer(&service))]);
    assert!(t.auth.game_auth(&mut ok).await.is_ok());
    let game = t.store().game_account_by_user_id(&user_id).await.unwrap();
    assert!(game.is_some());
}

#[tokio::test]
async fn apikey_mode() {
    let t = Harness::with(&[("AUTH_MODE", "APIKEY")], false).await;
    let (_, _, user_id) = t.account("Slayer1", "slayer1@example.test").await;
    let account = t.store().launcher_account_by_user_id(&user_id).await.unwrap().unwrap();
    let hash = rustic_isles_auth::hashes::user_key_hash(common::SECRET, "DLK_x");
    t.store().insert_user_key(&account.account_id, &hash);
    let login = t
        .post("/account/api/oauth/token", &[], json!({"exchange_code": "DLK_x"}))
        .await;
    assert_eq!(login.json()["account_id"], json!(user_id));
    let wrong = t
        .post("/account/api/oauth/token", &[], json!({"exchange_code": "DLK_y"}))
        .await;
    assert_eq!(wrong.status, StatusCode::BAD_REQUEST);
    let service = t
        .post(
            "/account/api/oauth/token",
            &[],
            json!({"grant_type": "client_credentials"}),
        )
        .await;
    assert_eq!(service.status, StatusCode::OK);
    t.store().update_account(&user_id, |a| a.status = Status::Disabled);
    let disabled = t
        .post("/account/api/oauth/token", &[], json!({"exchange_code": "DLK_x"}))
        .await;
    assert_eq!(disabled.status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn unknown_auth_mode_and_dev_mode() {
    let t = Harness::with(&[("AUTH_MODE", "NONE")], false).await;
    let refused = t.post("/account/api/oauth/token", &[], json!({})).await;
    assert_eq!(
        (refused.status, refused.text.as_str()),
        (StatusCode::INTERNAL_SERVER_ERROR, "")
    );
    t.settings.set("NODE_ENV", "development");
    t.settings.set("ALLOW_NO_AUTH_DEV_MODE", "true");
    let dev = t
        .post("/account/api/oauth/token", &[], json!({"exchange_code": "INVALID"}))
        .await;
    assert_eq!(dev.json()["account_id"], json!("dev-user"));
    let game = t.store().game_account_by_user_id("dev-user").await.unwrap().unwrap();
    assert_eq!(game.name, "Dev Slayer");
}
