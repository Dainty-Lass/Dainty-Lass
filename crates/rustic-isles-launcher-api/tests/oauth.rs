// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The OAuth sign-in routes through core's `app()`, the provider answered by a fake (`docs/oauth.md`).

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use std::sync::{Arc, Mutex};

use common::{Request1, Rig, options, settings};
use dainty_module_api::BoxFuture;
use rustic_isles_auth::oauth::{OAuthApi, OAuthFailure, OAuthProvider, OAuthUser};
use serde_json::{Value, json};

/// Answers every code with `user`, or fails; keeps what it was asked.
struct FakeProvider {
    user: Mutex<Option<OAuthUser>>,
    asked: Mutex<Vec<(String, String, String)>>,
}

impl FakeProvider {
    fn new(user: Option<OAuthUser>) -> Arc<Self> {
        Arc::new(Self {
            user: Mutex::new(user),
            asked: Mutex::new(Vec::new()),
        })
    }
}

impl OAuthApi for FakeProvider {
    fn sign_in<'a>(
        &'a self,
        provider: &'a OAuthProvider,
        code: &'a str,
        verifier: &'a str,
    ) -> BoxFuture<'a, Result<OAuthUser, OAuthFailure>> {
        self.asked
            .lock()
            .unwrap()
            .push((provider.id.clone(), code.to_owned(), verifier.to_owned()));
        let user = self.user.lock().unwrap().clone();
        Box::pin(async move { user.ok_or(OAuthFailure { status: Some(400) }) })
    }
}

fn fox(guilds: Option<Vec<&str>>) -> OAuthUser {
    OAuthUser {
        subject: "80351110224678912".into(),
        username: "fox.tail".into(),
        avatar: Some("a1b2".into()),
        guilds: guilds.map(|g| g.into_iter().map(str::to_owned).collect()),
    }
}

const DISCORD: [(&str, &str); 3] = [
    ("DAINTY_OAUTH_PRESET", "discord"),
    ("DAINTY_OAUTH_CLIENT_ID", "123456789"),
    ("DAINTY_OAUTH_CLIENT_SECRET", "client-secret"),
];

async fn make(extra: &[(&'static str, &str)], user: Option<OAuthUser>) -> (Rig, Arc<FakeProvider>) {
    let mut pairs = DISCORD.to_vec();
    pairs.extend_from_slice(extra);
    let rig = Rig::new(common::Options {
        settings: settings(&pairs),
        ..options(true)
    })
    .await;
    let fake = FakeProvider::new(user);
    rig.state.set_oauth_api(fake.clone());
    (rig, fake)
}

async fn start(rig: &Rig) -> Value {
    let answer = rig
        .send(Request1::new("POST", "/dainty/api/OAuthStart").json(&json!({})))
        .await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    answer.json()
}

async fn finish(rig: &Rig, started: &Value, extra: Value) -> common::Answer {
    let mut body = json!({"State": started["state"], "Code": "the-code", "Ticket": started["ticket"]});
    for (k, v) in extra.as_object().unwrap() {
        body[k] = v.clone();
    }
    rig.send(Request1::new("POST", "/dainty/api/OAuthFinish").json(&body))
        .await
}

async fn holder(rig: &Rig, key: &str) -> u16 {
    rig.send(Request1::new("GET", "/dainty/api/GetUserInfo").header("x-dainty-user-api-key", key))
        .await
        .status
}

#[tokio::test]
async fn a_discord_sign_up_then_a_sign_in_on_another_pc() {
    let (rig, fake) = make(&[("REGISTRATION_MODE", "OPEN")], Some(fox(None))).await;
    let status = rig.get("/dainty/api/ServerStatus").await.json();
    assert_eq!(
        status["signIn"],
        json!({"provider": "discord", "name": "Discord", "required": false})
    );

    let started = start(&rig).await;
    let url = started["authorizeUrl"].as_str().unwrap();
    assert!(
        url.starts_with("https://discord.com/oauth2/authorize?client_id=123456789&redirect_uri="),
        "{url}"
    );
    assert!(
        url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A61190%2Foauth%2Fcallback"),
        "{url}"
    );
    assert!(!started.to_string().contains("client-secret"));
    assert_eq!(started["ticket"].as_str().unwrap().len(), 64);

    let wrong = finish(&rig, &started, json!({"Ticket": "0".repeat(64)})).await;
    assert_eq!((wrong.status, wrong.code()), (401, "oauth_expired".to_owned()));
    assert!(
        fake.asked.lock().unwrap().is_empty(),
        "a wrong ticket never reaches the provider"
    );

    let made = finish(&rig, &started, json!({})).await;
    assert_eq!(made.status, 200, "{}", made.body);
    let body = made.json();
    assert_eq!(
        (body["Username"].as_str(), body["Created"].as_bool()),
        (Some("foxtail"), Some(true))
    );
    let first_key = body["UUK"].as_str().unwrap().to_owned();
    assert_eq!(holder(&rig, &first_key).await, 200);
    let (provider, code, verifier) = fake.asked.lock().unwrap()[0].clone();
    assert_eq!((provider.as_str(), code.as_str()), ("discord", "the-code"));
    assert_eq!(verifier.len(), 43, "32 random bytes, base64url");

    let again = finish(&rig, &started, json!({})).await;
    assert_eq!(
        (again.status, again.code()),
        (401, "oauth_expired".to_owned()),
        "a state is spent once"
    );

    let second = start(&rig).await;
    let back = finish(&rig, &second, json!({"Username": "Ignored_Name"})).await;
    let body = back.json();
    assert_eq!(
        (body["Username"].as_str(), body["Created"].as_bool()),
        (Some("foxtail"), Some(false))
    );
    let second_key = body["UUK"].as_str().unwrap();
    assert_ne!(second_key, first_key);
    assert_eq!(holder(&rig, second_key).await, 200);
    assert_eq!(
        holder(&rig, &first_key).await,
        401,
        "one key per account: the older one stops"
    );
}

#[tokio::test]
async fn sign_up_follows_the_registration_mode_and_the_name_rules() {
    let (rig, _) = make(&[], Some(fox(None))).await;
    let no_invite = finish(&rig, &start(&rig).await, json!({})).await;
    assert_eq!((no_invite.status, no_invite.code()), (401, "invite_invalid".to_owned()));
    let bad_name = finish(&rig, &start(&rig).await, json!({"Username": "no spaces"})).await;
    assert_eq!((bad_name.status, bad_name.code()), (400, "username_invalid".to_owned()));

    let (closed, _) = make(&[("REGISTRATION_MODE", "NONE")], Some(fox(None))).await;
    let refused = finish(&closed, &start(&closed).await, json!({})).await;
    assert_eq!(
        (refused.status, refused.code()),
        (400, "registration_closed".to_owned())
    );

    let mut odd = fox(None);
    odd.username = "é.".into();
    let (open, _) = make(&[("REGISTRATION_MODE", "OPEN")], Some(odd)).await;
    let needed = finish(&open, &start(&open).await, json!({})).await;
    assert_eq!((needed.status, needed.code()), (400, "username_needed".to_owned()));
    let named = finish(&open, &start(&open).await, json!({"Username": "Moonwick"})).await;
    assert_eq!(named.json()["Username"], "Moonwick");

    let (failing, _) = make(&[("REGISTRATION_MODE", "OPEN")], None).await;
    let failed = finish(&failing, &start(&failing).await, json!({})).await;
    assert_eq!((failed.status, failed.code()), (502, "oauth_failed".to_owned()));
}

#[tokio::test]
async fn guilds_and_required() {
    let gated = [
        ("REGISTRATION_MODE", "OPEN"),
        ("DAINTY_OAUTH_GUILD_IDS", "111,222"),
        ("DAINTY_OAUTH_REQUIRED", "true"),
    ];
    let (outsider, _) = make(&gated, Some(fox(Some(vec!["999"])))).await;
    let started = start(&outsider).await;
    assert!(
        started["authorizeUrl"]
            .as_str()
            .unwrap()
            .contains("scope=identify+guilds")
    );
    let refused = finish(&outsider, &started, json!({})).await;
    assert_eq!((refused.status, refused.code()), (403, "oauth_not_member".to_owned()));
    let register = outsider
        .send(Request1::new("POST", "/dainty/api/Register").json(&json!({"Username": "Fox_9"})))
        .await;
    assert_eq!((register.status, register.code()), (403, "oauth_required".to_owned()));
    assert_eq!(
        outsider.get("/dainty/api/ServerStatus").await.json()["signIn"]["required"],
        true
    );

    let (member, _) = make(&gated, Some(fox(Some(vec!["222"])))).await;
    assert_eq!(finish(&member, &start(&member).await, json!({})).await.status, 200);
}

#[tokio::test]
async fn off_or_set_up_wrong_is_not_found() {
    let off = Rig::new(common::Options {
        settings: settings(&[("REGISTRATION_MODE", "OPEN")]),
        ..options(true)
    })
    .await;
    let answer = off
        .send(Request1::new("POST", "/dainty/api/OAuthStart").json(&json!({})))
        .await;
    assert_eq!((answer.status, answer.code()), (404, "oauth_off".to_owned()));
    assert!(off.get("/dainty/api/ServerStatus").await.json().get("signIn").is_none());
    let (wrong, _) = make(&[("DAINTY_OAUTH_GUILD_IDS", "my-server")], None).await;
    let answer = wrong
        .send(Request1::new("POST", "/dainty/api/OAuthStart").json(&json!({})))
        .await;
    assert_eq!(answer.status, 404);
    let bad = wrong
        .send(Request1::new("POST", "/dainty/api/OAuthFinish").json(&json!({"State": 1})))
        .await;
    assert_eq!(bad.status, 404);
}

async fn link(rig: &Rig, key: &str) -> common::Answer {
    let started = start(rig).await;
    let body = json!({"State": started["state"], "Code": "the-code", "Ticket": started["ticket"]});
    rig.send(
        Request1::new("POST", "/dainty/api/OAuthFinish")
            .header("x-dainty-user-api-key", key)
            .json(&body),
    )
    .await
}

async fn register(rig: &Rig, name: &str) -> String {
    let made = rig
        .send(Request1::new("POST", "/dainty/api/Register").json(&json!({"Username": name})))
        .await;
    made.json()["UUK"].as_str().unwrap().to_owned()
}

async fn linked(rig: &Rig, key: &str) -> Value {
    rig.send(Request1::new("GET", "/dainty/api/GetUserInfo").header("x-dainty-user-api-key", key))
        .await
        .json()["OAuthLinked"]
        .clone()
}

#[tokio::test]
async fn an_existing_account_links_and_then_signs_in_with_it() {
    let (rig, fake) = make(&[("REGISTRATION_MODE", "OPEN")], Some(fox(None))).await;
    let key = register(&rig, "Moonwick").await;
    assert_eq!(linked(&rig, &key).await, json!(false));

    let unknown = link(&rig, &format!("DLK_{}", "0".repeat(48))).await;
    assert_eq!((unknown.status, unknown.code()), (401, "unauthorized".to_owned()));
    assert!(
        fake.asked.lock().unwrap().is_empty(),
        "an unknown key never reaches the provider"
    );

    let done = link(&rig, &key).await;
    assert_eq!(done.status, 200, "{}", done.body);
    assert_eq!(done.json(), json!({"Linked": true, "Username": "Moonwick"}));
    assert_eq!(holder(&rig, &key).await, 200, "linking keeps the key");
    assert_eq!(linked(&rig, &key).await, json!(true));
    assert_eq!(link(&rig, &key).await.status, 200, "linking again is a no-op");

    let other = register(&rig, "Tamsin_").await;
    let taken = link(&rig, &other).await;
    assert_eq!((taken.status, taken.code()), (409, "oauth_linked_elsewhere".to_owned()));

    let back = finish(&rig, &start(&rig).await, json!({})).await.json();
    assert_eq!(
        (back["Username"].as_str(), back["Created"].as_bool()),
        (Some("Moonwick"), Some(false))
    );

    *fake.user.lock().unwrap() = Some(OAuthUser {
        subject: "1".into(),
        ..fox(None)
    });
    let second = link(&rig, back["UUK"].as_str().unwrap()).await;
    assert_eq!(
        (second.status, second.code()),
        (409, "oauth_account_has_link".to_owned())
    );
}

#[tokio::test]
async fn user_info_says_nothing_of_links_without_sign_in() {
    let rig = Rig::new(common::Options {
        settings: settings(&[("REGISTRATION_MODE", "OPEN")]),
        ..options(true)
    })
    .await;
    let key = register(&rig, "Moonwick").await;
    let info = rig
        .send(Request1::new("GET", "/dainty/api/GetUserInfo").header("x-dainty-user-api-key", &key))
        .await
        .json();
    assert!(info.get("OAuthLinked").is_none());
}
