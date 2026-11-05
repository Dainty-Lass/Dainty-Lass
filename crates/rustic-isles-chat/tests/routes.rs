// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The chat area's routes black-box through core's `app()` (spec §11.2, routes table).

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use axum::body::Body;
use axum::extract::Request;
use http::StatusCode;
use http_body_util::BodyExt;
use rustic_isles::testing::sample_version;
use rustic_isles::{Handlers, Service, Shared, SharedParts, Slots};
use rustic_isles_auth::AuthData;
use rustic_isles_chat::{Clock, register_with};
use rustic_isles_store::IslesStore;
use serde_json::{Value, json};
use tower::ServiceExt;

struct Answer {
    status: StatusCode,
    content_type: Option<String>,
    body: String,
}

impl Answer {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap()
    }
}

/// The caller a stand-in auth layer puts in the extensions: none, or a userId (`None` inside is no userId).
#[derive(Clone, Copy)]
enum As<'a> {
    Nobody,
    Empty,
    User(&'a str),
}

struct Harness {
    app: axum::Router,
}

fn counting_clock() -> Clock {
    let ticks = Arc::new(AtomicU32::new(0));
    Arc::new(move || format!("2026-10-20T12:00:{:02}.000Z", ticks.fetch_add(1, Ordering::SeqCst)))
}

fn tick(n: u32) -> String {
    format!("2026-10-20T12:00:{n:02}.000Z")
}

impl Harness {
    fn new(store: Option<IslesStore>) -> Self {
        let mut parts = SharedParts::bare(sample_version());
        let mut handlers = Handlers::new();
        register_with(&mut handlers, counting_clock());
        handlers.check().unwrap();
        parts.handlers = handlers.clone();
        if let Some(store) = store {
            parts.slots = Slots::default().with(Arc::new(store));
        }
        let shared = Shared::new(parts).unwrap();
        Self {
            app: rustic_isles::app(shared, Service::Backend, &handlers).unwrap(),
        }
    }

    async fn send(&self, method: &str, uri: &str, caller: As<'_>, body: Option<(&str, String)>) -> Answer {
        let mut builder = Request::builder().method(method).uri(uri).header("host", "localhost");
        let body = match body {
            Some((content_type, text)) => {
                builder = builder
                    .header("content-type", content_type)
                    .header("content-length", text.len().to_string());
                Body::from(text)
            }
            None => Body::empty(),
        };
        let mut request = builder.body(body).unwrap();
        let user_id = match caller {
            As::Nobody => None,
            As::Empty => Some(None),
            As::User(user) => Some(Some(user.to_owned())),
        };
        if let Some(user_id) = user_id {
            request.extensions_mut().insert(AuthData {
                user_id,
                ..AuthData::default()
            });
        }
        let response = self.app.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let content_type = response
            .headers()
            .get("content-type")
            .map(|v| v.to_str().unwrap().to_owned());
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        Answer {
            status,
            content_type,
            body: String::from_utf8(bytes.to_vec()).unwrap(),
        }
    }

    async fn get(&self, uri: &str, caller: As<'_>) -> Answer {
        self.send("GET", uri, caller, None).await
    }
}

fn assert_empty(answer: &Answer, status: StatusCode) {
    assert_eq!(answer.status, status, "{}", answer.body);
    assert_eq!(answer.body, "");
    assert_eq!(answer.content_type, None);
}

fn edge(account: &str, status: &str, direction: &str, created: &str) -> Value {
    json!({"accountId": account, "status": status, "direction": direction, "created": created, "favorite": false})
}

const FRIENDS: &str = "/friends/api/public/friends";

#[tokio::test(flavor = "multi_thread")]
async fn friends_invite_accept_remove() {
    let temp = rustic_isles_store::test_support::TempStore::open().unwrap();
    for user in ["alice", "bob", "carol"] {
        common::account(&temp.store, user).await;
    }
    let h = Harness::new(Some(temp.store.clone()));
    let alice = As::User("alice");
    let bob = As::User("bob");
    let list = |user: &'static str, pending: bool| {
        let query = if pending { "?includePending=true" } else { "" };
        format!("{FRIENDS}/{user}{query}")
    };

    // 1-3
    let answer = h.get(&list("alice", false), alice).await;
    assert_eq!((answer.status, answer.body.as_str()), (StatusCode::OK, "[]"));
    assert_eq!(answer.content_type.as_deref(), Some("application/json; charset=utf-8"));
    assert_empty(
        &h.get(&list("alice", false), As::Nobody).await,
        StatusCode::UNAUTHORIZED,
    );
    assert_eq!(h.get(&format!("{FRIENDS}/zz"), As::Empty).await.body, "[]");

    // 4-8: the second invite rewrites both edges at tick 1
    for _ in 0..2 {
        assert_empty(
            &h.send("POST", &format!("{FRIENDS}/alice/bob"), alice, None).await,
            StatusCode::NO_CONTENT,
        );
    }
    assert_empty(
        &h.send("POST", &format!("{FRIENDS}/alice/alice"), alice, None).await,
        StatusCode::BAD_REQUEST,
    );
    assert_eq!(h.get(&list("alice", false), alice).await.body, "[]");
    // The path's account is ignored: the owner is the caller
    assert_eq!(
        h.get(&list("bob", true), alice).await.json(),
        json!([edge("bob", "PENDING", "OUTBOUND", &tick(1))])
    );
    let answer = h.get(&list("bob", true), bob).await;
    assert_eq!(
        answer.body,
        r#"[{"accountId":"alice","status":"PENDING","direction":"INBOUND","created":"2026-10-20T12:00:01.000Z","favorite":false}]"#
    );

    // 9-11
    h.send("POST", &format!("{FRIENDS}/alice/carol"), alice, None).await;
    assert_empty(
        &h.send("POST", &format!("{FRIENDS}/bob/alice"), bob, None).await,
        StatusCode::NO_CONTENT,
    );
    assert_eq!(
        h.get(&list("alice", true), alice).await.json(),
        json!([
            edge("bob", "ACCEPTED", "OUTBOUND", &tick(3)),
            edge("carol", "PENDING", "OUTBOUND", &tick(2))
        ])
    );
    assert_eq!(
        h.get(&list("bob", false), bob).await.json(),
        json!([edge("alice", "ACCEPTED", "OUTBOUND", &tick(1))])
    );
    // An accepted edge takes no new invite (the clock is still read once)
    h.send("POST", &format!("{FRIENDS}/alice/bob"), alice, None).await;
    assert_eq!(
        h.get(&list("bob", false), bob).await.json(),
        json!([edge("alice", "ACCEPTED", "OUTBOUND", &tick(1))])
    );

    // 12-13
    assert_empty(
        &h.send("DELETE", &format!("{FRIENDS}/alice/bob"), alice, None).await,
        StatusCode::NO_CONTENT,
    );
    assert_eq!(h.get(&list("bob", true), bob).await.body, "[]");
    assert_empty(
        &h.send("DELETE", &format!("{FRIENDS}/alice/nobody"), alice, None).await,
        StatusCode::NO_CONTENT,
    );
    assert_empty(
        &h.send("DELETE", &format!("{FRIENDS}/carol/alice"), As::User("carol"), None)
            .await,
        StatusCode::NO_CONTENT,
    );
    assert_eq!(h.get(&list("alice", true), alice).await.body, "[]");
    // DELETE oneself and with no caller: 204, nothing else
    assert_empty(
        &h.send("DELETE", &format!("{FRIENDS}/alice/alice"), alice, None).await,
        StatusCode::NO_CONTENT,
    );
    assert_empty(
        &h.send("DELETE", &format!("{FRIENDS}/alice/bob"), As::Empty, None).await,
        StatusCode::NO_CONTENT,
    );

    // 14: no edge for a stranger
    assert_empty(
        &h.send("POST", &format!("{FRIENDS}/alice/ghost"), alice, None).await,
        StatusCode::NO_CONTENT,
    );
    assert_eq!(h.get(&list("alice", true), alice).await.body, "[]");
    assert_empty(
        &h.send("POST", &format!("{FRIENDS}/alice/bob"), As::Empty, None).await,
        StatusCode::NO_CONTENT,
    );
    assert_eq!(h.get(&list("bob", true), bob).await.body, "[]");
}

#[tokio::test(flavor = "multi_thread")]
async fn stubs() {
    let h = Harness::new(None);
    // 15: no auth at all
    for (path, body) in [
        ("/friends/api/public/blocklist/x", "[]"),
        ("/friends/api/public/list/fortnite/x/recentPlayers", "[]"),
        ("/friends/api/public/list/any/x/recentPlayers", "[]"),
        (
            "/friends/api/v1/x/settings",
            r#"{"acceptInvites":"public","mutualPrivacy":"ALL"}"#,
        ),
    ] {
        let answer = h.get(path, As::Nobody).await;
        assert_eq!((answer.status, answer.body.as_str()), (StatusCode::OK, body), "{path}");
    }
    // 16-17
    let alice = As::User("alice");
    let answer = h.get("/guild/invite/player", alice).await;
    assert_eq!(
        (answer.status, answer.body.as_str()),
        (
            StatusCode::OK,
            r#"{"code":null,"message":"OK","payload":{"invites":[]}}"#
        )
    );
    assert_empty(&h.get("/guild", alice).await, StatusCode::NO_CONTENT);
    let answer = h.get("/slayerlink/status_good", alice).await;
    assert_eq!(
        answer.body,
        r#"{"code":null,"message":"OK","payload":{"config":{"invite_expiry_hours":48,"link_duration_hours":168},"invites":[],"links":[]}}"#
    );
    assert_empty(&h.get("/vivox/login", alice).await, StatusCode::NOT_FOUND);
    for path in [
        "/guild/invite/player",
        "/guild",
        "/slayerlink/status_good",
        "/vivox/login",
    ] {
        assert_empty(&h.get(path, As::Nobody).await, StatusCode::UNAUTHORIZED);
    }
}

fn availability(answer: &Answer) -> Value {
    assert_eq!(answer.status, StatusCode::OK, "{}", answer.body);
    let body = answer.json();
    assert_eq!((&body["code"], &body["message"]), (&Value::Null, &json!("OK")));
    body["payload"]["availability"].clone()
}

#[tokio::test(flavor = "multi_thread")]
async fn slayer_link_availability() {
    let h = Harness::new(None);
    let alice = As::User("alice");
    let post = |body: Option<(&'static str, String)>, caller| h.send("POST", "/slayerlink/availability", caller, body);
    let json_body = |v: &Value| Some(("application/json", v.to_string()));

    // 18
    let answer = post(json_body(&json!({"account_ids": ["AB", "", 7, "CD"]})), alice).await;
    assert_eq!(
        availability(&answer),
        json!([{"account_id": "AB", "available": false}, {"account_id": "CD", "available": false}])
    );
    // 19
    for body in [
        json_body(&json!({"account_ids": "AB"})),
        None,
        json_body(&json!(["a"])),
        Some(("text/plain", r#"{"account_ids":["AB"]}"#.to_owned())),
        Some(("application/json", String::new())),
    ] {
        assert_eq!(availability(&post(body, alice).await), json!([]));
    }
    // A form body reads as Express's urlencoded parser does
    let form = Some((
        "application/x-www-form-urlencoded",
        "account_ids=A&account_ids=B".to_owned(),
    ));
    assert_eq!(availability(&post(form, alice).await).as_array().unwrap().len(), 2);
    // 20
    let ids: Vec<String> = (0..300).map(|i| format!("ID{i}")).collect();
    let listed = availability(&post(json_body(&json!({ "account_ids": ids })), alice).await);
    let listed = listed.as_array().unwrap();
    assert_eq!((listed.len(), &listed[255]["account_id"]), (256, &json!("ID255")));
    // 21
    let emoji = "\u{1F600}";
    let body = json!({"account_ids": [emoji.repeat(64), emoji.repeat(65), "x".repeat(129)]});
    assert_eq!(
        availability(&post(json_body(&body), alice).await),
        json!([{"account_id": emoji.repeat(64), "available": false}])
    );
    // 22: JSON.stringify's escapes
    let odd = "a\"b\\c\nd\u{1}e\u{7f}f/g\u{2028}h";
    let answer = post(json_body(&json!({ "account_ids": [odd] })), alice).await;
    assert_eq!(
        answer.body,
        "{\"code\":null,\"message\":\"OK\",\"payload\":{\"availability\":[{\"account_id\":\
         \"a\\\"b\\\\c\\nd\\u0001e\u{7f}f/g\u{2028}h\",\"available\":false}]}}"
    );
    // 23: the body before auth, bare statuses
    let answer = post(Some(("application/json", "{nope".to_owned())), As::Nobody).await;
    assert_empty(&answer, StatusCode::BAD_REQUEST);
    assert_empty(
        &post(Some(("application/json", "7".to_owned())), alice).await,
        StatusCode::BAD_REQUEST,
    );
    let big = format!("[\"{}\"]", "x".repeat(101 * 1024));
    assert_empty(
        &post(Some(("application/json", big)), As::Nobody).await,
        StatusCode::PAYLOAD_TOO_LARGE,
    );
    // The coordinator's parity: a BOM is kept (malformed), a charset is not checked
    let bom = Some(("application/json", "\u{feff}{}".to_owned()));
    assert_empty(&post(bom, alice).await, StatusCode::BAD_REQUEST);
    let latin = Some((
        "application/json; charset=latin1",
        r#"{"account_ids":["A"]}"#.to_owned(),
    ));
    assert_eq!(availability(&post(latin, alice).await).as_array().unwrap().len(), 1);
    // A good body still needs auth
    assert_empty(&post(json_body(&json!({})), As::Nobody).await, StatusCode::UNAUTHORIZED);
}

#[tokio::test(flavor = "multi_thread")]
async fn no_store_is_501_after_auth() {
    // 24
    let h = Harness::new(None);
    for (method, path) in [
        ("GET", "/friends/api/public/friends/alice"),
        ("POST", "/friends/api/public/friends/alice/bob"),
        ("DELETE", "/friends/api/public/friends/alice/bob"),
    ] {
        assert_empty(&h.send(method, path, As::Nobody, None).await, StatusCode::UNAUTHORIZED);
        let answer = h.send(method, path, As::User("alice"), None).await;
        assert_eq!(answer.status, StatusCode::NOT_IMPLEMENTED);
        let row = if method == "GET" {
            "/friends/api/public/friends/:accountId"
        } else {
            "/friends/api/public/friends/:accountId/:friendId"
        };
        assert_eq!(
            answer.json()["error"]["message"],
            json!(format!("isles: {method} {row} is not implemented yet"))
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_friend_homed_elsewhere_is_503() {
    // 25
    let split = common::split_store();
    common::account(&split.store, "alice").await;
    let bob = common::account(&split.store, "bob").await;
    split.home.send_away(&bob);
    let h = Harness::new(Some(split.store.clone()));
    let answer = h
        .send("POST", &format!("{FRIENDS}/alice/bob"), As::User("alice"), None)
        .await;
    assert_empty(&answer, StatusCode::SERVICE_UNAVAILABLE);
    // The caller's edge was its own write and stays (§12's one-sided edge)
    assert_eq!(
        h.get(&format!("{FRIENDS}/alice?includePending=true"), As::User("alice"))
            .await
            .json(),
        json!([edge("bob", "PENDING", "OUTBOUND", &tick(0))])
    );
}
