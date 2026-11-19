// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The public half through core's `app()` (public spec §18.2).

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

mod common;

use common::{PUBLISHER_KEY, Request1, Rig, guard_payload, iso, now_ms, options, settings};
use rustic_isles::{Service, routes};
use serde_json::{Value, json};

#[tokio::test]
async fn probes() {
    let rig = Rig::new(options(true)).await;
    let root = rig.get("/").await;
    assert_eq!((root.status, root.body.as_str()), (200, "ok"));
    assert_eq!(root.header("content-type"), Some("text/html; charset=utf-8"));
    rig.get("/health/live")
        .await
        .assert(200, r#"{"ok":true,"service":"paradox-backend","role":"combined"}"#);
    rig.get("/health/ready").await.assert(
        200,
        r#"{"ok":true,"service":"paradox-backend","role":"combined","reason":"READY","persistence":"ready"}"#,
    );
    rig.get("/maintenance/status")
        .await
        .assert(200, r#"{"show-status":false}"#);
    let status = rig.get("/dauntless-status").await;
    let keys: Vec<String> = status.json().as_object().unwrap().keys().cloned().collect();
    assert_eq!(keys, ["show-status", "en", "fr", "it", "es", "de", "pt", "ru", "ja"]);
    assert_eq!(status.json()["ja"], "Welcome to Rustic Isles!");

    let bare = Rig::new(options(false)).await;
    bare.get("/health/ready").await.assert(
        503,
        r#"{"ok":false,"service":"paradox-backend","role":"combined","reason":"API_BEHIND_PROXY_RUNNING","persistence":"unavailable"}"#,
    );
    let api = Rig::new(common::Options {
        settings: settings(&[("RUSTIC_ISLES_SERVICE_ROLE", " API ")]),
        ..options(false)
    })
    .await;
    assert_eq!(api.get("/health/live").await.json()["role"], "api");
}

#[tokio::test]
async fn metrics_route() {
    let rig = Rig::new(options(false)).await;
    let _ = rig.get("/").await;
    let open = rig.get("/internal/v1/metrics").await;
    assert_eq!(open.status, 200);
    assert_eq!(
        open.header("content-type"),
        Some("text/plain; charset=utf-8; version=0.0.4")
    );
    assert_eq!(open.header("cache-control"), Some("no-store"));
    assert!(open.body.contains("rustic_isles_service_ready 1\n"));
    assert!(
        open.body
            .contains("rustic_isles_http_requests_total{class=\"game\",method=\"GET\"}")
    );
    let prod = Rig::new(common::Options {
        settings: settings(&[("NODE_ENV", "production")]),
        ..options(false)
    })
    .await;
    let refused = prod.get("/internal/v1/metrics").await;
    assert_eq!((refused.status, refused.body.as_str()), (503, "metrics unavailable\n"));
    let token = Rig::new(common::Options {
        settings: settings(&[("RUSTIC_ISLES_METRICS_TOKEN", "t0ken")]),
        ..options(false)
    })
    .await;
    assert_eq!(token.get("/internal/v1/metrics").await.status, 401);
    let send = |h: &'static str, v: &'static str| Request1::new("GET", "/internal/v1/metrics").header(h, v);
    assert_eq!(token.send(send("authorization", "Bearer wrong")).await.status, 401);
    assert_eq!(token.send(send("authorization", "Bearer t0ken")).await.status, 200);
    assert_eq!(
        token.send(send("x-rustic-isles-metrics-token", "t0ken")).await.status,
        200
    );
    assert_eq!(
        token.send(send("x-mysticparadox-metrics-token", "t0ken")).await.status,
        401
    );
}

async fn register(rig: &Rig, body: Value) -> common::Answer {
    rig.send(Request1::new("POST", "/dainty/api/Register").json(&body))
        .await
}

#[tokio::test]
async fn a_key_holder_is_found_by_the_stored_hash_alone() {
    use rustic_isles_launcher_api::launcher_api::holder_of_key;
    let rig = Rig::new(common::Options {
        settings: settings(&[("REGISTRATION_MODE", "OPEN")]),
        ..options(true)
    })
    .await;
    let made = register(&rig, json!({"Username": "Fox_9"})).await;
    let key = made.json()["UUK"].as_str().unwrap().to_owned();
    let account: rustic_isles_store::AccountId = made.header("x-dainty-account-id").unwrap().parse().unwrap();
    // What the ring asks after a restart (N-1): the same HMAC check as a keyed launcher call
    let holder = holder_of_key(&rig.shared, &rig.store, &key).await.unwrap().unwrap();
    assert_eq!(holder.account, account);
    assert!(holder.eligible());
    let last = if key.ends_with('x') { 'y' } else { 'x' };
    let other = format!("{}{last}", &key[..key.len() - 1]);
    assert!(holder_of_key(&rig.shared, &rig.store, &other).await.unwrap().is_none());
    assert!(
        holder_of_key(&rig.shared, &rig.store, "not a key")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn registration() {
    let open = Rig::new(common::Options {
        settings: settings(&[("REGISTRATION_MODE", " open ")]),
        ..options(true)
    })
    .await;
    open.get("/dainty/api/RegistrationStatus")
        .await
        .assert(200, r#"{"mode":"OPEN"}"#);
    let made = register(&open, json!({"Username": "  Fox_9 "})).await;
    assert_eq!(made.status, 200, "{}", made.body);
    let body = made.json();
    let keys: Vec<&String> = body.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["UUK"]);
    let key = body["UUK"].as_str().unwrap().to_owned();
    assert!(key.starts_with("DLK_") && key.len() == 52);
    assert!(made.header("x-dainty-account-id").unwrap().starts_with("account:"));
    assert_eq!(
        register(&open, json!({"Username": "fox_9"})).await.code(),
        "username_taken"
    );
    assert_eq!(register(&open, json!({})).await.code(), "bad_request");
    assert_eq!(
        register(&open, json!({"Username": "abc", "InviteCode": null}))
            .await
            .code(),
        "bad_request"
    );
    assert_eq!(
        register(&open, json!({"Username": "abc", "InviteCode": 5}))
            .await
            .code(),
        "bad_request"
    );
    for bad in ["ab", "a b c", "abcdefghijklmnopq", "née_ab"] {
        assert_eq!(
            register(&open, json!({"Username": bad})).await.code(),
            "username_invalid"
        );
    }
    let info = open
        .send(Request1::new("GET", "/dainty/api/GetUserInfo").header("x-dainty-user-api-key", &key))
        .await;
    assert_eq!(info.status, 200);
    assert_eq!(info.json()["Username"], "Fox_9");
    assert_eq!(info.json()["IsAdmin"], false);

    let none = Rig::new(common::Options {
        settings: settings(&[("REGISTRATION_MODE", "none")]),
        ..options(true)
    })
    .await;
    let closed = none
        .send(Request1::new("POST", "/dainty/api/Register").header("content-type", "application/json"))
        .await;
    assert_eq!((closed.status, closed.code().as_str()), (400, "registration_closed"));

    let invite = Rig::new(options(true)).await;
    invite
        .get("/dainty/api/RegistrationStatus")
        .await
        .assert(200, r#"{"mode":"INVITECODE"}"#);
    let host = |r: Request1<'static>| r.header("x-dainty-admin-key", common::ADMIN_KEY);
    let code = invite
        .send(host(Request1::new("POST", "/dainty/api/CreateInvite").json(&json!({}))))
        .await
        .json()["code"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(register(&invite, json!({"Username": "Nocode"})).await.status, 401);
    assert_eq!(
        register(
            &invite,
            json!({"Username": "Taken_1", "InviteCode": format!(" {code} ")})
        )
        .await
        .status,
        200
    );
    assert_eq!(
        register(&invite, json!({"Username": "Taken_2", "InviteCode": code.clone()}))
            .await
            .status,
        401
    );
    let two = invite
        .send(host(
            Request1::new("POST", "/dainty/api/CreateInvite").json(&json!({"uses": 2})),
        ))
        .await
        .json()["code"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        register(&invite, json!({"Username": "taken_1", "InviteCode": two.clone()}))
            .await
            .status,
        409
    );
    let list = invite
        .send(host(Request1::new("GET", "/dainty/api/InviteCodes")))
        .await
        .json();
    let entry = list["InviteCodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["code"] == two.as_str())
        .unwrap();
    assert_eq!(
        (entry["usesRemaining"].clone(), entry["note"].clone()),
        (json!(2), Value::Null)
    );
}

#[tokio::test]
async fn user_info_and_status() {
    let rig = Rig::new(common::Options {
        settings: settings(&[
            ("REGISTRATION_MODE", "OPEN"),
            ("DAINTY_SERVER_NAME", "Fox hold"),
            ("DAINTY_SERVER_VERSION", "1.2.3"),
            ("DAINTY_SOURCE_URL", "https://example.com/src"),
            ("APPROVED_EXECUTABLE_SHA256", &"AB".repeat(32)),
        ]),
        ..options(true)
    })
    .await;
    rig.get("/dainty/api/GetUserInfo")
        .await
        .assert(401, r#"{"error":"unauthorized"}"#);
    let odd = rig
        .send(Request1::new("GET", "/dainty/api/GetUserInfo").header("x-dainty-user-api-key", "a b c"))
        .await;
    odd.assert(401, r#"{"error":"unauthorized"}"#);
    let made = register(&rig, json!({"Username": "Fox_9"})).await;
    let key = made.json()["UUK"].as_str().unwrap().to_owned();
    let account: rustic_isles_store::AccountId = made.header("x-dainty-account-id").unwrap().parse().unwrap();
    let ban = |status: rustic_isles_store::repo::auth::OperationalStatus, reason: Option<&str>| {
        let a = account.clone();
        let reason = reason.map(str::to_owned);
        let store = rig.store.clone();
        async move {
            store
                .write(
                    &dainty_module_api::ActorContext::new(dainty_module_api::Actor::System),
                    move |tx, repos| {
                        let change = rustic_isles_store::repo::auth::AccessChange {
                            approval_status: None,
                            status: Some(status),
                            approval_updated_at: iso(now_ms()),
                            approval_updated_by: "t".into(),
                            approval_reason: reason,
                        };
                        repos.launcher_accounts.set_access_state(tx, &a, &change).map(|_| ())
                    },
                )
                .await
                .unwrap();
        }
    };
    let info = || Request1::new("GET", "/dainty/api/GetUserInfo").header("x-dainty-user-api-key", &key);
    use rustic_isles_store::repo::auth::OperationalStatus::{Active, Banned, Disabled};
    ban(Banned, Some("cheating")).await;
    rig.send(info())
        .await
        .assert(403, r#"{"error":"account_banned","reason":"cheating"}"#);
    ban(Disabled, None).await;
    rig.send(info())
        .await
        .assert(403, r#"{"error":"account_disabled","reason":"Ask the server's host."}"#);
    ban(Active, None).await;
    assert_eq!(rig.send(info()).await.status, 200);

    let limited = rig.get("/dainty/api/ServerStatus").await;
    assert_eq!(limited.status, 200);
    assert_eq!(limited.header("cache-control"), Some("no-store"));
    assert_eq!(limited.header("vary"), Some("x-dainty-user-api-key"));
    let body = limited.json();
    let keys: Vec<&String> = body.as_object().unwrap().keys().collect();
    assert_eq!(
        keys,
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
    assert_eq!(
        dainty_service_kit::jsjson::stringify(&body["game"]),
        format!(
            r#"{{"build":"rel-1.14.7-Archon","changelist":647472,"executableSha256":"{}"}}"#,
            "ab".repeat(32)
        )
    );
    assert_eq!(body["gameData"], "missing");
    assert_eq!(body["chat"], false);
    assert_eq!(body["limited"], true);
    let full = rig
        .send(Request1::new("GET", "/dainty/api/ServerStatus").header("x-dainty-user-api-key", &key))
        .await
        .json();
    assert_eq!(
        (
            full["limited"].clone(),
            full["playersOnline"].clone(),
            full["players"].clone(),
            full["instances"].clone()
        ),
        (json!(false), json!(0), json!([]), json!([]))
    );
}

// ------------------------------------------------------------------ runtime feed and publishing

fn publish(rig: &Rig, path: &str, body: &[u8], extra: &[(&str, &str)]) -> Request1<'static> {
    let mut r = Request1::new("POST", Box::leak(path.to_owned().into_boxed_str()))
        .bytes(body)
        .header("x-update-api-key", PUBLISHER_KEY)
        .header("x-update-signature", &rig.sign_runtime(body));
    for (k, v) in extra {
        r = r.header(k, v);
    }
    r
}

#[tokio::test]
async fn runtime_feed() {
    let rig = Rig::new(options(true)).await;
    let path = "/launcher/v1/admin/updates/runtime/client/stable/windows-x86_64";
    let meta = [("x-update-version", "1.2.3"), ("x-update-changelist", "647472")];
    let dll = b"runtime bytes";
    let refused = rig.send(publish(&rig, path, dll, &meta).peer("10.0.0.5:1")).await;
    refused.assert(
        403,
        r#"{"error":"Update publishing is not allowed from this address."}"#,
    );
    let mut no_key = publish(&rig, path, dll, &meta);
    no_key.headers.retain(|(k, _)| k != "x-update-api-key");
    let no_key = rig.send(no_key).await;
    assert_eq!(no_key.status, 401, "{}", no_key.body);
    let wrong_changelist = rig
        .send(publish(
            &rig,
            path,
            dll,
            &[("x-update-version", "1.2.3"), ("x-update-changelist", "1")],
        ))
        .await;
    wrong_changelist.assert(400, r#"{"error":"Invalid runtime update metadata."}"#);
    let mut bad_sig = publish(&rig, path, dll, &meta);
    bad_sig.headers.retain(|(k, _)| k != "x-update-signature");
    bad_sig = bad_sig.header("x-update-signature", &rig.sign_runtime(b"other"));
    rig.send(bad_sig)
        .await
        .assert(400, r#"{"error":"Runtime update signature verification failed."}"#);
    let made = rig.send(publish(&rig, path, dll, &meta)).await;
    assert_eq!(made.status, 201, "{}", made.body);
    let keys: Vec<String> = made.json().as_object().unwrap().keys().cloned().collect();
    assert_eq!(keys, ["target", "channel", "version", "sha256"]);
    assert_eq!(rig.send(publish(&rig, path, dll, &meta)).await.status, 409);

    let manifest = rig.get("/launcher/v1/runtime/client/stable/windows-x86_64").await;
    assert_eq!(manifest.status, 200);
    assert_eq!(
        manifest.header("cache-control"),
        Some("no-cache, no-store, must-revalidate")
    );
    let m = manifest.json();
    let keys: Vec<String> = m.as_object().unwrap().keys().cloned().collect();
    assert_eq!(
        keys,
        [
            "schema",
            "component",
            "target",
            "version",
            "channel",
            "targetChangelist",
            "platform",
            "size",
            "sha256",
            "signature",
            "file",
            "publishedAt",
            "url",
            "extraFiles"
        ]
    );
    assert_eq!(m["file"], "ParadoxRuntime-1.2.3-client-647472.dll");
    assert_eq!(
        m["url"],
        "https://updates.example/launcher/v1/runtime/client/stable/windows-x86_64/download"
    );
    assert_eq!(rig.get("/launcher/v1/runtime/stable/windows-x86_64").await.status, 200);
    assert_eq!(
        rig.get("/launcher/v1/runtime/x/stable/windows-x86_64").await.status,
        400
    );
    assert_eq!(rig.get("/launcher/v1/runtime/client/stable/..").await.status, 400);
    let missing = rig.get("/launcher/v1/runtime/server/stable/windows-x86_64").await;
    missing.assert(404, "Not Found");
    let download = rig
        .get("/launcher/v1/runtime/client/stable/windows-x86_64/download")
        .await;
    assert_eq!((download.status, download.body.as_bytes()), (200, &dll[..]));
    assert_eq!(
        download.header("cache-control"),
        Some("public, max-age=31536000, immutable")
    );
    assert_eq!(download.header("content-type"), Some("application/octet-stream"));
    let shadowed = rig.get("/launcher/v1/runtime/stable/windows-x86_64/download").await;
    assert_eq!(
        shadowed.status, 400,
        "the shadowed path answers as B182 with target `stable`"
    );

    let extra_path = "/launcher/v1/admin/updates/runtime/client/stable/windows-x86_64/extra";
    let winmm = b"winmm bytes";
    let early = rig
        .send(publish(
            &rig,
            extra_path,
            winmm,
            &[("x-update-version", "9.9.9"), ("x-update-filename", "winmm.dll")],
        ))
        .await;
    early.assert(
        409,
        r#"{"error":"Publish the main runtime DLL for this version first."}"#,
    );
    let extra = rig
        .send(publish(
            &rig,
            extra_path,
            winmm,
            &[("x-update-version", "1.2.3"), ("x-update-filename", "winmm.dll")],
        ))
        .await;
    assert_eq!(extra.status, 201, "{}", extra.body);
    let listed = rig
        .get("/launcher/v1/runtime/client/stable/windows-x86_64")
        .await
        .json();
    assert_eq!(listed["extraFiles"][0]["name"], "winmm.dll");
    assert!(
        listed["extraFiles"][0]["url"]
            .as_str()
            .unwrap()
            .ends_with("/extra/winmm.dll")
    );
    let served = rig
        .get("/launcher/v1/runtime/client/stable/windows-x86_64/extra/winmm.dll")
        .await;
    assert_eq!((served.status, served.body.as_bytes()), (200, &winmm[..]));
    assert_eq!(
        rig.get("/launcher/v1/runtime/client/stable/windows-x86_64/extra/other.dll")
            .await
            .status,
        404
    );

    let switch = Request1::new("POST", "/launcher/v1/admin/updates/serving")
        .json(&json!({"runtimeEnabled": false}))
        .header("x-update-api-key", PUBLISHER_KEY);
    rig.send(switch).await.assert(200, r#"{"runtimeEnabled":false}"#);
    let paused = rig.get("/launcher/v1/runtime/client/stable/windows-x86_64").await;
    assert_eq!((paused.status, paused.body.as_str()), (204, ""));
    assert_eq!(
        paused.header("cache-control"),
        Some("no-cache, no-store, must-revalidate")
    );
    assert_eq!(rig.get("/launcher/v1/runtime/client/stable/..").await.status, 204);
    assert_eq!(
        rig.get("/launcher/v1/runtime/client/stable/windows-x86_64/download")
            .await
            .status,
        200
    );
    let read = Request1::new("GET", "/launcher/v1/admin/updates/serving").header("x-update-api-key", PUBLISHER_KEY);
    rig.send(read).await.assert(200, r#"{"runtimeEnabled":false}"#);
}

#[tokio::test]
async fn guard_manifests() {
    let rig = Rig::new(options(true)).await;
    let path = "/launcher/v1/admin/guard-manifests/stable/windows-x86_64";
    let post = |body: &Value| {
        Request1::new("POST", path)
            .json(body)
            .header("x-update-api-key", PUBLISHER_KEY)
    };
    rig.send(post(&json!({})))
        .await
        .assert(400, r#"{"error":"Invalid Guard manifest envelope."}"#);
    let mut unknown = rig.envelope(&guard_payload(&"a".repeat(36), 1, "0.1.0"));
    unknown["keyId"] = json!("other");
    rig.send(post(&unknown))
        .await
        .assert(400, r#"{"error":"Unknown Guard manifest signing key."}"#);
    let mut forged = rig.envelope(&guard_payload(&"a".repeat(36), 1, "0.1.0"));
    forged["signature"] = json!("AAAA");
    rig.send(post(&forged))
        .await
        .assert(400, r#"{"error":"Guard manifest signature verification failed."}"#);
    let mut other_channel = guard_payload(&"a".repeat(36), 1, "0.1.0");
    other_channel["channel"] = json!("beta");
    rig.send(post(&rig.envelope(&other_channel))).await.assert(
        400,
        r#"{"error":"Guard manifest route and signed payload do not match."}"#,
    );
    let first = rig.envelope(&guard_payload(&"a".repeat(36), 2, "0.1.0"));
    let made = rig.send(post(&first)).await;
    assert_eq!(made.status, 201, "{}", made.body);
    assert_eq!(made.json()["keyId"], "guard-2026-10");
    let again = rig
        .send(post(&rig.envelope(&guard_payload(&"b".repeat(36), 2, "0.1.0"))))
        .await;
    again.assert(
        409,
        r#"{"error":"Guard manifest sequence must increase monotonically."}"#,
    );
    let served = rig.get("/launcher/v1/guard-manifests/stable/windows-x86_64").await;
    assert_eq!(served.status, 200);
    assert_eq!(served.json(), first);
    assert_eq!(rig.get("/launcher/v1/guard-manifests/stable/linux").await.status, 404);
    assert_eq!(
        rig.get("/launcher/v1/guard-manifests/beta/windows-x86_64").await.status,
        401
    );
}

#[tokio::test]
async fn launcher_feed() {
    let rig = Rig::new(options(true)).await;
    let dir = rig.updates.path().join("launcher").join("windows").join("x86_64");
    std::fs::create_dir_all(dir.join("0.2.0")).unwrap();
    std::fs::write(dir.join("0.2.0").join("Rustic_0.2.0_x64-setup.exe"), b"setup").unwrap();
    let manifest = json!({
        "version": "0.2.0", "notes": "n", "pub_date": "2026-10-20T00:00:00Z",
        "url": "https://updates.example/launcher/v1/updates/windows/x86_64/files/0.2.0/Rustic_0.2.0_x64-setup.exe",
        "signature": "sig", "file": "Rustic_0.2.0_x64-setup.exe"
    });
    std::fs::write(dir.join("latest.json"), manifest.to_string()).unwrap();
    let offered = rig.get("/launcher/v1/updates/windows/x86_64/0.1.0").await;
    assert_eq!(offered.status, 200);
    let keys: Vec<String> = offered.json().as_object().unwrap().keys().cloned().collect();
    assert_eq!(keys, ["version", "notes", "pub_date", "url", "signature"]);
    assert_eq!(rig.get("/launcher/v1/updates/windows/x86_64/0.2.0").await.status, 204);
    assert_eq!(rig.get("/launcher/v1/updates/windows/x86_64/9.9.9").await.status, 204);
    assert_eq!(rig.get("/launcher/v1/updates/windows/x86_64/garbage").await.status, 200);
    assert_eq!(rig.get("/launcher/v1/updates/windows/arm/0.1.0").await.status, 204);
    let download = rig.get("/launcher/v1/updates/windows/x86_64/download").await;
    assert_eq!((download.status, download.body.as_str()), (200, "setup"));
    let file = rig
        .get("/launcher/v1/updates/windows/x86_64/files/0.2.0/Rustic_0.2.0_x64-setup.exe")
        .await;
    assert_eq!(file.status, 200);
    assert_eq!(
        file.header("content-disposition"),
        Some("attachment; filename=\"Rustic_0.2.0_x64-setup.exe\"")
    );
    assert_eq!(
        rig.get("/launcher/v1/updates/windows/x86_64/files/0.2.0/x.exe")
            .await
            .status,
        404
    );

    let remote = json!({
        "version": "0.3.0", "signature": "s",
        "url": "https://github.com/pranav158/Mystic-Paradox/releases/download/v0.3.0/a.exe"
    });
    std::fs::write(dir.join("latest.json"), remote.to_string()).unwrap();
    let redirect = rig.get("/launcher/v1/updates/windows/x86_64/download").await;
    assert_eq!(redirect.status, 302);
    assert_eq!(redirect.header("location"), remote["url"].as_str());
    assert_eq!(
        redirect.body,
        format!("Found. Redirecting to {}", remote["url"].as_str().unwrap())
    );
    let own = json!({"version": "0.3.0", "signature": "s", "url": "https://updates.example/launcher/v1/updates/a.exe"});
    std::fs::write(dir.join("latest.json"), own.to_string()).unwrap();
    assert_eq!(
        rig.get("/launcher/v1/updates/windows/x86_64/download").await.status,
        404
    );
}

// ------------------------------------------------------------------ logs

async fn tester(rig: &Rig, user: &str, roles: &[&str]) -> String {
    let account = rig.player(user, &format!("N{}", &user[..6]), None, roles).await;
    rig.bearer(user, &account).await
}

#[tokio::test]
async fn log_uploads_and_sync() {
    let rig = Rig::new(common::Options {
        settings: settings(&[("LAUNCHER_LOG_SYNC_TOKEN", "sync-1")]),
        ..options(true)
    })
    .await;
    let path = "/launcher/v1/logs/sessions/abcdef12-3456/launcher.log";
    let put = |bearer: Option<&str>, p: &'static str| {
        let r = Request1::new("PUT", p).bytes(b"log lines");
        match bearer {
            Some(b) => r.header("authorization", b),
            None => r,
        }
    };
    assert_eq!(rig.send(put(None, path)).await.status, 401);
    let player = tester(&rig, "aaaaaaaa-0000-4000-8000-000000000001", &["player"]).await;
    let refused = rig.send(put(Some(&player), path)).await;
    assert_eq!((refused.status, refused.code().as_str()), (403, "AUTH_TESTER_REQUIRED"));
    let user = "bbbbbbbb-0000-4000-8000-000000000002";
    let bearer = tester(&rig, user, &["player", "tester"]).await;
    let bad = rig
        .send(put(Some(&bearer), "/launcher/v1/logs/sessions/short/launcher.log"))
        .await;
    assert_eq!((bad.status, bad.code().as_str()), (400, "AUTH_VALIDATION_FAILED"));
    let empty = rig
        .send(Request1::new("PUT", path).bytes(b"").header("authorization", &bearer))
        .await;
    assert_eq!(empty.json()["error"]["message"], "The uploaded file was empty.");
    let ok = rig.send(put(Some(&bearer), path)).await;
    assert_eq!(ok.status, 201, "{}", ok.body);
    assert!(ok.json()["uploadedAt"].is_string());
    let session = rig.logs.path().join(user).join("abcdef12-3456");
    assert!(session.join(".uploaded-at.json").exists());
    assert_eq!(std::fs::read(session.join("launcher.log")).unwrap(), b"log lines");

    let sync = |uri: &'static str, token: Option<&str>| {
        let r = Request1::new("GET", uri);
        match token {
            Some(t) => r.header("x-sync-token", t),
            None => r,
        }
    };
    assert_eq!(rig.send(sync("/admin/v1/launcher-logs", None)).await.status, 401);
    assert_eq!(
        rig.send(sync("/admin/v1/launcher-logs", Some("nope"))).await.status,
        401
    );
    let listed = rig.send(sync("/admin/v1/launcher-logs", Some("sync-1"))).await.json();
    assert_eq!(listed["sessions"][0]["userId"], user);
    assert_eq!(listed["sessions"][0]["files"], json!(["launcher.log"]));
    let file = rig
        .send(sync(
            Box::leak(format!("/admin/v1/launcher-logs/{user}/abcdef12-3456/launcher.log").into_boxed_str()),
            Some("sync-1"),
        ))
        .await;
    assert_eq!((file.status, file.body.as_str()), (200, "log lines"));
    let missing = rig
        .send(sync(
            Box::leak(format!("/admin/v1/launcher-logs/{user}/abcdef12-3456/metadata.json").into_boxed_str()),
            Some("sync-1"),
        ))
        .await;
    assert_eq!(missing.status, 404);
    let traversal = rig
        .send(sync(
            "/admin/v1/launcher-logs/../abcdef12-3456/launcher.log",
            Some("sync-1"),
        ))
        .await;
    assert!(traversal.status == 400 || traversal.status == 404);
}

fn session_dir(rig: &Rig, user: &str, id: &str, at: &str, files: &[(&str, &str)]) {
    let dir = rig.logs.path().join(user).join(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(".uploaded-at.json"),
        format!("{{\"lastUploadedAt\":\"{at}\"}}\n"),
    )
    .unwrap();
    for (name, body) in files {
        std::fs::write(dir.join(name), body).unwrap();
    }
}

async fn operator(rig: &Rig, method: &str, target: &str, body: Option<&Value>) -> common::Answer {
    let bytes = body.map(|b| b.to_string().into_bytes()).unwrap_or_default();
    let headers = rustic_isles_launcher_api::test_support::operator_headers(
        common::OPERATOR_SECRET,
        method,
        target,
        "111111111111111111",
        now_ms(),
        &bytes,
    );
    let mut r = Request1::new(
        Box::leak(method.to_owned().into_boxed_str()),
        Box::leak(target.to_owned().into_boxed_str()),
    );
    for (k, v) in &headers {
        r = r.header(k, v);
    }
    if let Some(b) = body {
        r = r.json(b);
    }
    rig.send(r).await
}

#[tokio::test]
async fn operator_log_view() {
    let rig = Rig::new(options(true)).await;
    session_dir(
        &rig,
        "u1",
        "aaaaaaaa-1",
        "2026-10-03T00:00:00.000Z",
        &[("launcher.log", "four"), ("metadata.json", "{}"), ("evil.exe", "x")],
    );
    session_dir(
        &rig,
        "u2",
        "bbbbbbbb-2",
        "2026-10-03T00:00:00.000Z",
        &[("launcher.log", "x")],
    );
    session_dir(
        &rig,
        "u1",
        "cccccccc-3",
        "2026-10-01T00:00:00.000Z",
        &[("launcher.log", "xy")],
    );
    std::fs::create_dir_all(rig.logs.path().join("bad name").join("dddddddd-4")).unwrap();
    let all = operator(&rig, "GET", "/admin/v1/player-logs", None).await.json();
    let ids: Vec<&str> = all["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["launchSessionId"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["aaaaaaaa-1", "bbbbbbbb-2", "cccccccc-3"]);
    assert_eq!(
        dainty_service_kit::jsjson::stringify(&all["sessions"][0]),
        r#"{"userId":"u1","launchSessionId":"aaaaaaaa-1","uploadedAt":"2026-10-03T00:00:00.000Z","files":[{"name":"launcher.log","size":4},{"name":"metadata.json","size":2}],"totalBytes":6}"#
    );
    let page = operator(&rig, "GET", "/admin/v1/player-logs?limit=1", None)
        .await
        .json();
    assert_eq!(page["sessions"].as_array().unwrap().len(), 2);
    assert_eq!(page["nextBefore"], "2026-10-03T00:00:00.000Z");
    let next = operator(
        &rig,
        "GET",
        "/admin/v1/player-logs?limit=1&before=2026-10-03T00:00:00.000Z",
        None,
    )
    .await
    .json();
    assert_eq!(next["sessions"][0]["launchSessionId"], "cccccccc-3");
    let one = operator(&rig, "GET", "/admin/v1/player-logs?userId=u2", None)
        .await
        .json();
    assert_eq!(one["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(
        operator(&rig, "GET", "/admin/v1/player-logs?limit=0", None)
            .await
            .code(),
        "PLAYER_LOGS_QUERY"
    );
    let text = operator(&rig, "GET", "/admin/v1/player-logs/u1/aaaaaaaa-1/launcher.log", None).await;
    assert_eq!((text.status, text.body.as_str()), (200, "four"));
    assert_eq!(text.header("content-type"), Some("text/plain; charset=utf-8"));
    let meta = operator(
        &rig,
        "GET",
        "/admin/v1/player-logs/u1/aaaaaaaa-1/metadata.json?download=1",
        None,
    )
    .await;
    assert_eq!(meta.header("content-type"), Some("application/json; charset=utf-8"));
    assert_eq!(
        meta.header("content-disposition"),
        Some("attachment; filename=\"metadata.json\"")
    );
    assert_eq!(
        operator(&rig, "GET", "/admin/v1/player-logs/u1/aaaaaaaa-1/other.txt", None)
            .await
            .code(),
        "PLAYER_LOG_NAME_REFUSED"
    );
    assert_eq!(
        operator(&rig, "GET", "/admin/v1/player-logs/u1/short/launcher.log", None)
            .await
            .code(),
        "PLAYER_LOG_PATH"
    );
    assert_eq!(
        operator(&rig, "GET", "/admin/v1/player-logs/u2/aaaaaaaa-1/launcher.log", None)
            .await
            .code(),
        "PLAYER_LOG_NOT_FOUND"
    );
    let big = rig
        .logs
        .path()
        .join("u2")
        .join("bbbbbbbb-2")
        .join("runtime-rustic_isles_dll_port1.log");
    std::fs::write(&big, vec![b'x'; 10 * 1024 * 1024 + 1]).unwrap();
    assert_eq!(
        operator(
            &rig,
            "GET",
            "/admin/v1/player-logs/u2/bbbbbbbb-2/runtime-rustic_isles_dll_port1.log",
            None
        )
        .await
        .code(),
        "PLAYER_LOG_TOO_LARGE"
    );
    let short = operator(
        &rig,
        "DELETE",
        "/admin/v1/player-logs/u1/cccccccc-3",
        Some(&json!({"reason": "x"})),
    )
    .await;
    assert_eq!(short.code(), "PLAYER_LOG_REASON");
    let deleted = operator(
        &rig,
        "DELETE",
        "/admin/v1/player-logs/u1/aaaaaaaa-1",
        Some(&json!({"reason": "clearing an old upload"})),
    )
    .await;
    assert_eq!(deleted.status, 200, "{}", deleted.body);
    assert_eq!(
        (deleted.json()["files"].clone(), deleted.json()["totalBytes"].clone()),
        (json!(2), json!(6))
    );
    assert!(rig.logs.path().join("u1").join("cccccccc-3").exists());
    let again = operator(
        &rig,
        "DELETE",
        "/admin/v1/player-logs/u1/aaaaaaaa-1",
        Some(&json!({"reason": "clearing an old upload"})),
    )
    .await;
    assert_eq!(again.status, 404);
    let audits = rig.audits().await;
    assert!(
        audits
            .iter()
            .any(|a| a.action == "player_logs.delete" && a.actor_user_id == "manager:111111111111111111")
    );
}

// ------------------------------------------------------------------ notices, operations, activity

#[tokio::test]
async fn notices_routes() {
    let rig = Rig::new(options(true)).await;
    let view = operator(&rig, "GET", "/admin/v1/launcher-notices", None).await.json();
    assert_eq!(view["version"], 0);
    let body = json!({
        "announcements": [{"text": "Hello", "tone": "info"}],
        "tags": [],
        "reason": "first notice",
        "expectedVersion": 0
    });
    let saved = operator(&rig, "PUT", "/admin/v1/launcher-notices", Some(&body)).await;
    assert_eq!(saved.status, 200, "{}", saved.body);
    assert_eq!(saved.json()["version"], 1);
    let stale = operator(&rig, "PUT", "/admin/v1/launcher-notices", Some(&body)).await;
    assert_eq!(stale.status, 409);
    let keys: Vec<String> = stale.json()["error"].as_object().unwrap().keys().cloned().collect();
    assert_eq!(keys, ["current", "code", "message", "requestId"]);
    let invalid = operator(&rig, "PUT", "/admin/v1/launcher-notices", Some(&json!({}))).await;
    assert_eq!((invalid.status, invalid.code().as_str()), (400, "NOTICES_INVALID"));
    assert_eq!(
        rig.audits()
            .await
            .iter()
            .filter(|a| a.action == "launcher_notices.update")
            .count(),
        1
    );
    let public = rig.get("/launcher/v1/notices").await;
    assert_eq!(public.json()["announcements"][0]["text"], "Hello");
}

#[tokio::test]
async fn operations_routes() {
    let totp = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
    let rig = Rig::new(common::Options {
        settings: settings(&[("ADMIN_OPERATIONS_TOTP_SECRET", totp)]),
        ..options(true)
    })
    .await;
    let overview = operator(&rig, "GET", "/admin/v1/operations/overview", None)
        .await
        .json();
    assert_eq!(
        dainty_service_kit::jsjson::stringify(&overview["policy"]),
        r#"{"guardEnforcement":"ENFORCE","diagnosticsProfile":"PRODUCTION","version":0}"#
    );
    let change = |guard: &str, version: i64, reason: &str, code: Option<&str>| {
        let mut body = json!({"guardEnforcement": guard, "diagnosticsProfile": "PRODUCTION", "expectedVersion": version, "reason": reason});
        if let Some(c) = code {
            body["breakGlassTotp"] = json!(c);
        }
        body
    };
    let short = operator(
        &rig,
        "PATCH",
        "/admin/v1/operations/policy",
        Some(&change("ENFORCE", 0, "short", None)),
    )
    .await;
    assert_eq!(short.code(), "ADMIN_VALIDATION");
    let stale = operator(
        &rig,
        "PATCH",
        "/admin/v1/operations/policy",
        Some(&change("ENFORCE", 7, "a long enough reason", None)),
    )
    .await;
    assert_eq!((stale.status, stale.code().as_str()), (409, "ADMIN_POLICY_CONFLICT"));
    let weak = operator(
        &rig,
        "PATCH",
        "/admin/v1/operations/policy",
        Some(&change("OBSERVE", 0, "a long enough reason", None)),
    )
    .await;
    assert_eq!(
        (weak.status, weak.code().as_str()),
        (403, "ADMIN_POLICY_STEP_UP_REQUIRED")
    );
    assert_eq!(weak.json()["weakeningCodes"], json!(["GUARD_DOWNGRADE"]));
    assert!(
        rig.audits()
            .await
            .iter()
            .any(|a| a.action == "operations.policy.weakening_rejected")
    );
    let code = totp_now(totp);
    let ok = operator(
        &rig,
        "PATCH",
        "/admin/v1/operations/policy",
        Some(&change("OBSERVE", 0, "a long enough reason", Some(&code))),
    )
    .await;
    assert_eq!(ok.status, 200, "{}", ok.body);
    assert_eq!(ok.json()["policy"]["guardEnforcement"], "OBSERVE");
    assert!(
        rig.audits()
            .await
            .iter()
            .any(|a| a.action == "operations.policy.update")
    );
}

/// The current RFC 6238 code, computed here from the RFC's own definition.
fn totp_now(secret: &str) -> String {
    let key = rustic_isles_auth::totp::decode_base32(secret).unwrap();
    let step = now_ms().div_euclid(30_000);
    let tag = aws_lc_rs::hmac::sign(
        &aws_lc_rs::hmac::Key::new(aws_lc_rs::hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, &key),
        &step.to_be_bytes(),
    );
    let mac = tag.as_ref();
    let offset = usize::from(mac[19] & 0x0f);
    let binary = (u32::from(mac[offset] & 0x7f) << 24)
        | (u32::from(mac[offset + 1]) << 16)
        | (u32::from(mac[offset + 2]) << 8)
        | u32::from(mac[offset + 3]);
    format!("{:06}", binary % 1_000_000)
}

#[tokio::test]
async fn activity_routes() {
    let rig = Rig::new(options(true)).await;
    let account = rig.player("act-user-1", "Actor_1", None, &["player"]).await;
    let too_long = operator(
        &rig,
        "GET",
        "/admin/v1/activity/overview?from=2025-01-01&to=2026-10-30",
        None,
    )
    .await;
    assert_eq!(too_long.code(), "ACTIVITY_RANGE_TOO_LONG");
    let reversed = operator(
        &rig,
        "GET",
        "/admin/v1/activity/overview?from=2026-10-30&to=2026-10-01",
        None,
    )
    .await;
    assert_eq!(reversed.code(), "ACTIVITY_RANGE_INVALID");
    let flag = rustic_isles_store::repo::reforged::ActivityFlagRecord {
        id: "KILLS_PER_HOUR:act-user-1:2026-10-20".into(),
        kind: "KILLS_PER_HOUR".into(),
        severity: "review".into(),
        account: account.clone(),
        day: "2026-10-20".into(),
        detail: json!("kills=91"),
        acknowledged: false,
        created_at: "2026-10-20T01:00:00.000Z".into(),
        acknowledged_at: None,
        acknowledged_by: None,
        note: None,
        ttl_at: i64::MAX / 2,
    };
    rig.store
        .write(
            &dainty_module_api::ActorContext::new(dainty_module_api::Actor::System),
            move |tx, repos| repos.activity.insert_flag_if_absent(tx, &flag).map(|_| ()),
        )
        .await
        .unwrap();
    let player = operator(
        &rig,
        "GET",
        "/admin/v1/activity/players/act-user-1?from=2026-10-01&to=2026-10-30",
        None,
    )
    .await
    .json();
    let keys: Vec<String> = player.as_object().unwrap().keys().cloned().collect();
    assert_eq!(
        keys,
        [
            "userId",
            "from",
            "to",
            "monitorEnabled",
            "days",
            "totals",
            "unlocks",
            "hunts",
            "flags",
            "timeline"
        ]
    );
    assert_eq!(player["flags"][0]["userId"], "act-user-1");
    let flags = operator(&rig, "GET", "/admin/v1/activity/flags", None).await.json();
    assert_eq!(flags["total"], 1);
    let id = "/admin/v1/activity/flags/KILLS_PER_HOUR:act-user-1:2026-10-20";
    let ack = operator(
        &rig,
        "PATCH",
        id,
        Some(&json!({"acknowledged": true, "note": "checked"})),
    )
    .await;
    assert_eq!(ack.status, 200, "{}", ack.body);
    assert_eq!(ack.json()["flag"]["status"], "acknowledged");
    let twice = operator(&rig, "PATCH", id, Some(&json!({"acknowledged": true}))).await;
    assert_eq!(twice.json()["flag"]["status"], "acknowledged");
    assert_eq!(
        rig.audits()
            .await
            .iter()
            .filter(|a| a.action == "activity.flag.acknowledge")
            .count(),
        1
    );
    let missing = operator(
        &rig,
        "PATCH",
        "/admin/v1/activity/flags/nope",
        Some(&json!({"acknowledged": true})),
    )
    .await;
    assert_eq!(missing.code(), "ACTIVITY_FLAG_NOT_FOUND");
}

// ------------------------------------------------------------------ Launcher Guard

fn signals(extra: &[(&str, Value)]) -> Value {
    let mut base = json!({
        "gameProcessId": 42, "gameStartedAt": "2026-10-20T00:00:00Z", "moduleCount": 10,
        "moduleSetDigest": "a".repeat(64), "privateExecutableBytes": 1024, "writableExecutableBytes": 0,
        "unexpectedModuleCount": 0, "protectedPageMismatchCount": 0, "debuggerPresent": false
    });
    for (k, v) in extra {
        base[*k] = v.clone();
    }
    base
}

fn sign_heartbeat(
    key: &aws_lc_rs::signature::Ed25519KeyPair,
    session: &Value,
    sequence: u64,
    challenge: &str,
    observed: &str,
    s: &Value,
) -> String {
    use base64::Engine as _;
    let parsed = rustic_isles_launcher_api::guard::Signals::parse(s).unwrap();
    let material = rustic_isles_launcher_api::guard::signature_material(
        session["role"].as_str().unwrap(),
        session["guardSessionId"].as_str().unwrap(),
        sequence,
        challenge,
        session["manifestId"].as_str().unwrap(),
        session["manifestSequence"].as_u64().unwrap(),
        observed,
        &parsed,
    );
    base64::engine::general_purpose::STANDARD.encode(key.sign(material.as_bytes()).as_ref())
}

#[tokio::test]
async fn guard_flow() {
    use aws_lc_rs::signature::KeyPair as _;
    use base64::Engine as _;
    let rig = Rig::new(options(true)).await;
    let manifest_id = "12345678-1234-1234-1234-123456789abc";
    let dir = rig
        .updates
        .path()
        .join("guard")
        .join("647472")
        .join("stable")
        .join("windows-x86_64");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("latest.json"),
        rig.envelope(&guard_payload(manifest_id, 3, "0.1.0")).to_string(),
    )
    .unwrap();
    let user = "cccccccc-0000-4000-8000-000000000003";
    let account = rig.player(user, "Guarded", None, &["player"]).await;
    let bearer = rig.bearer(user, &account).await;
    let pair = {
        let rng = aws_lc_rs::rand::SystemRandom::new();
        let pkcs8 = aws_lc_rs::signature::Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        aws_lc_rs::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()
    };
    let public = base64::engine::general_purpose::STANDARD.encode(pair.public_key().as_ref());
    let start = |body: Value| {
        Request1::new("POST", "/launcher/v1/guard/sessions")
            .json(&body)
            .header("authorization", &bearer)
    };
    let bad_role = rig.send(start(json!({"role": "worker"}))).await;
    assert_eq!(bad_role.code(), "GUARD_REPORT_INVALID");
    let stale = rig
        .send(start(
            json!({"manifestId": manifest_id, "manifestSequence": 2, "launcherVersion": "0.1.0", "publicKey": public}),
        ))
        .await;
    assert_eq!(stale.code(), "GUARD_MANIFEST_MISMATCH");
    let session = rig
        .send(start(
            json!({"manifestId": manifest_id, "manifestSequence": 3, "launcherVersion": "0.1.5", "publicKey": public}),
        ))
        .await;
    assert_eq!(session.status, 200, "{}", session.body);
    let session = session.json();
    assert_eq!(session["heartbeatIntervalMs"], 15000);
    let id = session["guardSessionId"].as_str().unwrap().to_owned();
    let status = rig
        .send(
            Request1::new(
                "GET",
                Box::leak(format!("/launcher/v1/guard/sessions/{id}").into_boxed_str()),
            )
            .header("authorization", &bearer),
        )
        .await
        .json();
    assert_eq!(status["status"], "CHALLENGED");
    let beat = |sequence: u64, challenge: &str, s: &Value| {
        let observed = iso(now_ms());
        let signature = sign_heartbeat(&pair, &session, sequence, challenge, &observed, s);
        json!({"guardSessionId": id, "sequence": sequence, "observedAt": observed, "signature": signature, "signals": s})
    };
    let hb = |body: Value| {
        Request1::new("POST", "/launcher/v1/guard/heartbeat")
            .json(&body)
            .header("authorization", &bearer)
    };
    let first_challenge = session["challenge"].as_str().unwrap().to_owned();
    let ok = rig.send(hb(beat(1, &first_challenge, &signals(&[])))).await;
    assert_eq!(ok.status, 200, "{}", ok.body);
    assert_eq!(ok.json()["status"], "HEALTHY");
    let next = ok.json()["nextChallenge"].as_str().unwrap().to_owned();
    assert_ne!(next, first_challenge);
    let replay = rig.send(hb(beat(1, &next, &signals(&[])))).await;
    assert_eq!(replay.code(), "GUARD_SESSION_INVALID");
    let old = rig.send(hb(beat(2, &first_challenge, &signals(&[])))).await;
    assert_eq!(old.code(), "GUARD_SIGNATURE_INVALID");
    let dirty = signals(&[
        ("debuggerPresent", json!(true)),
        ("unexpectedModuleCount", json!(1)),
        (
            "unexpectedModules",
            json!([{"fileName": "x.dll", "signer": "unsigned", "sha256": "b".repeat(64), "size": 5}]),
        ),
    ]);
    let degraded = rig.send(hb(beat(2, &next, &dirty))).await.json();
    assert_eq!(
        (degraded["status"].clone(), degraded["riskScore"].clone()),
        (json!("DEGRADED"), json!(65))
    );
    assert_eq!(degraded["reasons"], json!(["DEBUGGER", "UNEXPECTED_UNSIGNED:1"]));
    let findings = operator(&rig, "GET", "/admin/v1/guard/findings?signer=UNSIGNED", None)
        .await
        .json();
    assert_eq!(findings["total"], 1);
    let third = degraded["nextChallenge"].as_str().unwrap().to_owned();
    let moved = rig
        .send(hb(beat(3, &third, &signals(&[("gameProcessId", json!(43))]))))
        .await
        .json();
    assert_eq!(moved["riskScore"], 100);
    assert!(moved["reasons"].as_array().unwrap().contains(&json!("PROCESS_CHANGED")));

    let listed = operator(&rig, "GET", "/admin/v1/guard/sessions", None).await.json();
    assert!(listed["sessions"][0].get("challenge").is_none() && listed["sessions"][0].get("publicKeyBase64").is_none());
    assert_eq!(
        operator(&rig, "GET", "/admin/v1/guard/sessions?status=healthy", None)
            .await
            .code(),
        "GUARD_QUERY_INVALID"
    );
    assert_eq!(
        operator(&rig, "GET", "/admin/v1/guard/sessions?minRisk=50.5", None)
            .await
            .code(),
        "GUARD_QUERY_INVALID"
    );
    let one = operator(
        &rig,
        "GET",
        Box::leak(format!("/admin/v1/guard/sessions/{id}").into_boxed_str()),
        None,
    )
    .await
    .json();
    assert_eq!(one["history"].as_array().unwrap().len(), 3);
    assert_eq!(one["history"][0]["riskScore"], 100);
    assert_eq!(
        operator(&rig, "GET", "/admin/v1/guard/sessions/nope", None)
            .await
            .code(),
        "GUARD_SESSION_NOT_FOUND"
    );
    let again = rig
        .send(start(
            json!({"manifestId": manifest_id, "manifestSequence": 3, "launcherVersion": "0.1.5", "publicKey": public}),
        ))
        .await;
    assert_eq!(again.status, 200);
    let revoked = operator(
        &rig,
        "GET",
        Box::leak(format!("/admin/v1/guard/sessions/{id}").into_boxed_str()),
        None,
    )
    .await
    .json();
    assert_eq!(revoked["session"]["status"], "REVOKED");
}

// ------------------------------------------------------------------ seams and registration

#[tokio::test]
async fn seams() {
    use rustic_isles_auth::{Erasure, Notices, Operations};
    let bare = rustic_isles_launcher_api::LauncherApiSeams::new();
    assert_eq!(bare.policy().await, rustic_isles_auth::OperationsPolicy::default());
    let view = bare.public_view(0).await.unwrap();
    assert_eq!(view, rustic_isles_auth::empty_notices(0));
    let rig = Rig::new(options(true)).await;
    let seams = rig
        .shared
        .slot::<rustic_isles_launcher_api::LauncherApiSeams>()
        .unwrap();
    let account = rig.player("online-1", "Onliner", None, &["player"]).await;
    let launcher = rustic_isles_auth::LauncherAccount {
        account_id: account.to_string(),
        user_id: "online-1".into(),
        email: None,
        display_name: "Onliner".into(),
        display_name_normalized: "onliner".into(),
        password_hash: None,
        status: rustic_isles_auth::Status::Active,
        approval: None,
        roles: vec![],
        roles_updated_at: None,
        created_at: String::new(),
        last_login_at: None,
        username_set: None,
        consent: None,
        deletion: None,
        email_verified_at: None,
        email_verification_pending_since: None,
        timeout: None,
    };
    assert!(seams.blockers(&launcher).await.unwrap().is_empty());
    rig.state.touch("online-1", rustic_isles::replay::now_ms());
    assert_eq!(
        seams.blockers(&launcher).await.unwrap(),
        ["account is currently online"]
    );
    std::fs::create_dir_all(rig.logs.path().join("online-1").join("abcdef12-0001")).unwrap();
    seams.remove_uploaded_logs(&launcher).await.unwrap();
    assert!(!rig.logs.path().join("online-1").exists());
}

#[test]
fn this_half_registers_its_44_rows() {
    let mut public = rustic_isles::Handlers::new();
    rustic_isles_launcher_api::register_public(&mut public);
    let mut admin = rustic_isles::Handlers::new();
    rustic_isles_launcher_api::register_admin(&mut admin);
    public.check().unwrap();
    admin.check().unwrap();
    let reforged: Vec<_> = routes::all()
        .iter()
        .filter(|r| {
            r.service == Service::Backend
                && matches!(r.group, routes::Group::LauncherApi | routes::Group::Guard)
                && r.when != rustic_isles::When::Rifts
        })
        .collect();
    assert_eq!(reforged.len(), 66);
    let (mut p, mut a) = (0, 0);
    for row in &reforged {
        let in_public = public.contains(row.service, row.method, row.path);
        let in_admin = admin.contains(row.service, row.method, row.path);
        assert!(in_public ^ in_admin, "{} {}", row.method.as_str(), row.path);
        if in_public {
            p += 1;
        } else {
            a += 1;
        }
    }
    assert_eq!((p, a), (44, 22));
}
