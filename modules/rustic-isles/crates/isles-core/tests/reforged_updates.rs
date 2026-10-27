//! RU5 reforged-api: the runtime feed (one per changelist), the Guard manifests, the launcher's own update feed, the
//! publisher's serving switch and the launcher log routes, as `routes/launcherUpdates.ts`, `launcherLogs.ts` and
//! `adminLauncherLogs.ts` answer them. Publishing is driven through the handlers' functions with keys of the test's own;
//! the GET routes are requested over a real socket.
#![allow(clippy::unwrap_used, clippy::expect_used)] // tests

mod common;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::http::{HeaderMap, HeaderName, HeaderValue};
use common::{Answer, get, send, serve, shared};
use ed25519_dalek::{Signer, SigningKey};
use isles_core::AccountId;
use isles_core::areas::reforged_api::guard_manifest::Trusted;
use isles_core::areas::reforged_api::launcher_auth::{
    LauncherAuthenticator, LauncherCaller, LauncherFailure, set_authenticator,
};
use isles_core::areas::reforged_api::updates::{Ctx, publish_extra, publish_guard, publish_runtime};
use isles_core::areas::reforged_api::util::{Req, base64_encode, iso, now_ms, sha256_hex};
use isles_core::http::Shared;
use isles_core::repo::auth::{LauncherAccountRecord, OperationalStatus};
use isles_core::routes::{Params, RouteSet};
use module_api::{BoxFuture, NodeIdentity};
use serde_json::{Value, json};

/// The authenticator is process-wide, so the tests that set it take turns.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const PUBLISHER_KEY: &str = "publisher-key";

fn runtime_key() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32])
}

fn guard_key() -> SigningKey {
    SigningKey::from_bytes(&[9u8; 32])
}

fn trusted() -> Trusted {
    Trusted::with_keys(runtime_key().verifying_key(), guard_key().verifying_key())
}

struct Env {
    dir: tempfile::TempDir,
    logs: tempfile::TempDir,
}

fn settings<'a>(env: &'a Env, extra: &[(&'a str, &'a str)]) -> Vec<(String, String)> {
    let mut pairs = vec![
        ("LAUNCHER_UPDATE_ROOT".to_owned(), env.dir.path().display().to_string()),
        (
            "LAUNCHER_LOG_STORAGE_DIR".to_owned(),
            env.logs.path().display().to_string(),
        ),
        ("UPDATE_PUBLISHER_ALLOWED_IPS".to_owned(), "127.0.0.1".to_owned()),
        ("UPDATE_PUBLISHER_API_KEY".to_owned(), PUBLISHER_KEY.to_owned()),
        (
            "UPDATE_PUBLIC_BASE_URL".to_owned(),
            "https://updates.example.test/".to_owned(),
        ),
        ("MYSTICPARADOX_ADMIN_API".to_owned(), "on".to_owned()),
        ("LAUNCHER_LOG_SYNC_TOKEN".to_owned(), "sync-token".to_owned()),
    ];
    pairs.retain(|(name, _)| !extra.iter().any(|(n, _)| n == name));
    pairs.extend(extra.iter().map(|(n, v)| ((*n).to_owned(), (*v).to_owned())));
    pairs
}

fn shared_for(env: &Env, routes: RouteSet, extra: &[(&str, &str)]) -> Shared {
    let pairs = settings(env, extra);
    let borrowed: Vec<(&str, &str)> = pairs.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();
    shared(routes, &borrowed, None)
}

async fn server(env: &Env, routes: RouteSet, extra: &[(&str, &str)]) -> SocketAddr {
    serve(shared_for(env, routes, extra)).await
}

fn env() -> Env {
    Env {
        dir: tempfile::tempdir().unwrap(),
        logs: tempfile::tempdir().unwrap(),
    }
}

fn request(headers: &[(&str, &str)], body: Option<Vec<u8>>, json_body: Option<Value>) -> Req {
    let mut map = HeaderMap::new();
    for (name, value) in headers {
        map.insert(
            HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(value).unwrap(),
        );
    }
    let local: SocketAddr = "127.0.0.1:61000".parse().unwrap();
    Req {
        method: "POST".into(),
        path: "/".into(),
        headers: map,
        query: vec![],
        peer: Some(service_kit::serve::ConnectionInfo {
            peer: "127.0.0.1:50000".parse().unwrap(),
            local,
        }),
        body: json_body,
        raw: body,
    }
}

fn params(pairs: &[(&str, &str)]) -> Params {
    Params(pairs.iter().map(|(n, v)| ((*n).to_owned(), (*v).to_owned())).collect())
}

fn publisher() -> [(&'static str, &'static str); 1] {
    [("x-update-api-key", PUBLISHER_KEY)]
}

async fn body_of(response: axum::response::Response) -> (u16, Value) {
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn runtime_upload(changelist: u32, version: &str, bytes: &[u8], key: &SigningKey) -> (Vec<(String, String)>, Vec<u8>) {
    let signature = base64_encode(&key.sign(bytes).to_bytes());
    let headers = vec![
        ("x-update-api-key".to_owned(), PUBLISHER_KEY.to_owned()),
        ("x-update-version".to_owned(), version.to_owned()),
        ("x-update-changelist".to_owned(), changelist.to_string()),
        ("x-update-signature".to_owned(), signature),
    ];
    (headers, bytes.to_vec())
}

fn as_req(headers: &[(String, String)], body: Vec<u8>) -> Req {
    let pairs: Vec<(&str, &str)> = headers.iter().map(|(n, v)| (n.as_str(), v.as_str())).collect();
    request(&pairs, Some(body), None)
}

const RUNTIME_PARAMS: [(&str, &str); 3] = [
    ("target", "client"),
    ("channel", "stable"),
    ("platform", "windows-x86_64"),
];

#[tokio::test(flavor = "multi_thread")]
async fn a_signed_runtime_is_published_and_served_per_changelist() {
    let env = env();
    let shared = shared_for(&env, RouteSet::V1147, &[]);
    let ctx = Ctx::with(&shared, trusted());
    let dll = b"MZ runtime bytes".to_vec();

    // The gates, in Node's order
    let (headers, body) = runtime_upload(647_472, "1.2.3", &dll, &runtime_key());
    let no_key: Vec<_> = headers
        .iter()
        .filter(|(n, _)| n != "x-update-api-key")
        .cloned()
        .collect();
    let (status, _) = body_of(publish_runtime(
        &ctx,
        &as_req(&no_key, body.clone()),
        &params(&RUNTIME_PARAMS),
    ))
    .await;
    assert_eq!(status, 401);
    let strangers = Ctx::with(
        &shared_for(
            &env,
            RouteSet::V1147,
            &[("UPDATE_PUBLISHER_ALLOWED_IPS", "203.0.113.1")],
        ),
        trusted(),
    );
    let (status, answer) = body_of(publish_runtime(
        &strangers,
        &as_req(&headers, body.clone()),
        &params(&RUNTIME_PARAMS),
    ))
    .await;
    assert_eq!(
        (status, answer),
        (
            403,
            json!({"error": "Update publishing is not allowed from this address."})
        )
    );
    for (name, value) in [
        ("x-update-version", "1.2"),
        ("x-update-version", "../x"),
        ("x-update-changelist", "1"),
    ] {
        let mut bad = headers.clone();
        bad.iter_mut().find(|(n, _)| n == name).unwrap().1 = value.to_owned();
        let (status, answer) = body_of(publish_runtime(
            &ctx,
            &as_req(&bad, body.clone()),
            &params(&RUNTIME_PARAMS),
        ))
        .await;
        assert_eq!(
            (status, answer),
            (400, json!({"error": "Invalid runtime update metadata."})),
            "{name}={value}"
        );
    }
    let wrong_target = params(&[("target", "bot"), ("channel", "stable"), ("platform", "windows-x86_64")]);
    assert_eq!(
        body_of(publish_runtime(&ctx, &as_req(&headers, body.clone()), &wrong_target))
            .await
            .0,
        400
    );
    let (status, answer) = body_of(publish_runtime(
        &ctx,
        &as_req(&headers, vec![]),
        &params(&RUNTIME_PARAMS),
    ))
    .await;
    assert_eq!(
        (status, answer),
        (413, json!({"error": "Runtime update is outside the allowed size."}))
    );
    let (forged, forged_body) = runtime_upload(647_472, "1.2.3", &dll, &guard_key());
    let (status, answer) = body_of(publish_runtime(
        &ctx,
        &as_req(&forged, forged_body),
        &params(&RUNTIME_PARAMS),
    ))
    .await;
    assert_eq!(
        (status, answer),
        (400, json!({"error": "Runtime update signature verification failed."}))
    );

    // Published
    let (status, answer) = body_of(publish_runtime(
        &ctx,
        &as_req(&headers, body.clone()),
        &params(&RUNTIME_PARAMS),
    ))
    .await;
    assert_eq!(status, 201);
    let sha = sha256_hex(&dll);
    assert_eq!(
        answer,
        json!({"target": "client", "channel": "stable", "version": "1.2.3", "sha256": sha})
    );
    let (status, answer) = body_of(publish_runtime(
        &ctx,
        &as_req(&headers, body.clone()),
        &params(&RUNTIME_PARAMS),
    ))
    .await;
    assert_eq!(
        (status, answer),
        (
            409,
            json!({"error": "That target/channel/version is already published and immutable."})
        )
    );
    let on_disk = env.dir.path().join("runtime/647472/client/stable/windows-x86_64");
    let text = std::fs::read_to_string(on_disk.join("latest.json")).unwrap();
    assert!(text.starts_with("{\n  \"schema\": 1,\n  \"component\": \"ParadoxRuntime\",\n") && text.ends_with("}\n"));
    assert_eq!(
        std::fs::read(on_disk.join("1.2.3/ParadoxRuntime-1.2.3-client-647472.dll")).unwrap(),
        dll
    );

    // A companion needs its main file first, an allowlisted name and a signature
    let extra_params = RUNTIME_PARAMS;
    let extra_bytes = b"winmm".to_vec();
    let mut extra_headers = runtime_upload(647_472, "1.2.3", &extra_bytes, &runtime_key()).0;
    extra_headers.push(("x-update-filename".into(), "winmm.dll".into()));
    let (status, _) = body_of(publish_extra(
        &ctx,
        &as_req(&extra_headers, extra_bytes.clone()),
        &params(&extra_params),
    ))
    .await;
    assert_eq!(status, 201);
    let mut stranger = extra_headers.clone();
    stranger.retain(|(n, _)| n != "x-update-filename");
    stranger.push(("x-update-filename".into(), "evil.dll".into()));
    let (status, answer) = body_of(publish_extra(
        &ctx,
        &as_req(&stranger, extra_bytes.clone()),
        &params(&extra_params),
    ))
    .await;
    assert_eq!(
        (status, answer),
        (400, json!({"error": "Invalid extra-file target/platform/name."}))
    );
    let mut other_version = extra_headers.clone();
    other_version
        .iter_mut()
        .find(|(n, _)| n == "x-update-version")
        .unwrap()
        .1 = "9.9.9".into();
    let (status, answer) = body_of(publish_extra(
        &ctx,
        &as_req(&other_version, extra_bytes.clone()),
        &params(&extra_params),
    ))
    .await;
    assert_eq!(
        (status, answer),
        (
            409,
            json!({"error": "Publish the main runtime DLL for this version first."})
        )
    );

    // The feed, over HTTP
    let address = server(&env, RouteSet::V1147, &[]).await;
    let manifest = get(address, "/launcher/v1/runtime/client/stable/windows-x86_64", &[]).await;
    assert_eq!(manifest.status, 200);
    assert_eq!(
        manifest.header("cache-control"),
        Some("no-cache, no-store, must-revalidate")
    );
    let json = manifest.json();
    assert_eq!(
        json.as_object().unwrap().keys().collect::<Vec<_>>(),
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
    assert_eq!(
        json["url"],
        "https://updates.example.test/launcher/v1/runtime/client/stable/windows-x86_64/download"
    );
    assert_eq!(json["extraFiles"][0]["name"], "winmm.dll");
    assert_eq!(
        json["extraFiles"][0]["url"],
        "https://updates.example.test/launcher/v1/runtime/client/stable/windows-x86_64/extra/winmm.dll"
    );
    // The two-segment form is the client feed
    assert_eq!(
        get(address, "/launcher/v1/runtime/stable/windows-x86_64", &[])
            .await
            .json(),
        json
    );
    let dll_answer = get(
        address,
        "/launcher/v1/runtime/client/stable/windows-x86_64/download",
        &[],
    )
    .await;
    assert_eq!((dll_answer.status, dll_answer.body.clone()), (200, dll.clone()));
    assert_eq!(dll_answer.header("content-type"), Some("application/octet-stream"));
    assert_eq!(
        dll_answer.header("cache-control"),
        Some("public, max-age=31536000, immutable")
    );
    assert_eq!(
        dll_answer.header("content-length"),
        Some(dll.len().to_string().as_str())
    );
    let extra = get(
        address,
        "/launcher/v1/runtime/client/stable/windows-x86_64/extra/winmm.dll",
        &[],
    )
    .await;
    assert_eq!((extra.status, extra.body), (200, extra_bytes));
    assert_eq!(
        get(
            address,
            "/launcher/v1/runtime/client/stable/windows-x86_64/extra/other.dll",
            &[]
        )
        .await
        .status,
        404
    );
    assert_eq!(
        get(address, "/launcher/v1/runtime/bot/stable/windows-x86_64", &[])
            .await
            .status,
        400
    );
    assert_eq!(
        get(address, "/launcher/v1/runtime/bot/stable/windows-x86_64/download", &[])
            .await
            .status,
        400
    );
    assert_eq!(
        get(address, "/launcher/v1/runtime/server/stable/windows-x86_64", &[])
            .await
            .status,
        404
    );
    assert_eq!(
        get(address, "/launcher/v1/runtime/client/stable/win%2Fdows", &[])
            .await
            .status,
        400
    );
    let missing = get(address, "/launcher/v1/runtime/client/stable/linux", &[]).await;
    assert_eq!((missing.status, missing.text().as_str()), (404, "Not Found"));
    assert_eq!(missing.header("content-type"), Some("text/plain; charset=utf-8"));

    // The legacy download URL is shadowed by the manifest route: 400, since the channel isn't a target (RU3a)
    let shadowed = get(address, "/launcher/v1/runtime/stable/windows-x86_64/download", &[]).await;
    assert_eq!((shadowed.status, shadowed.text().as_str()), (400, "Bad Request"));

    // The 1.12 module on the same node has a feed of its own
    let old = server(&env, RouteSet::V1120, &[]).await;
    assert_eq!(
        get(old, "/launcher/v1/runtime/client/stable/windows-x86_64", &[])
            .await
            .status,
        404
    );
    let old_ctx = Ctx::with(&shared_for(&env, RouteSet::V1120, &[]), trusted());
    let (headers, body) = runtime_upload(392_819, "1.12.0", &dll, &runtime_key());
    assert_eq!(
        body_of(publish_runtime(
            &old_ctx,
            &as_req(&headers, body),
            &params(&RUNTIME_PARAMS)
        ))
        .await
        .0,
        201
    );
    let old_manifest = get(old, "/launcher/v1/runtime/client/stable/windows-x86_64", &[])
        .await
        .json();
    assert_eq!(
        (
            old_manifest["version"].clone(),
            old_manifest["targetChangelist"].clone()
        ),
        (json!("1.12.0"), json!(392_819))
    );
    assert_eq!(
        get(address, "/launcher/v1/runtime/client/stable/windows-x86_64", &[])
            .await
            .json()["version"],
        "1.2.3"
    );

    // The publisher's switch pauses the feed for launchers: 204, which they take as "no update"
    let off = send(
        address,
        "POST",
        "/launcher/v1/admin/updates/serving",
        &publisher(),
        Some(("application/json", br#"{"runtimeEnabled":false}"#)),
    )
    .await;
    assert_eq!((off.status, off.text().as_str()), (200, r#"{"runtimeEnabled":false}"#));
    let paused = get(address, "/launcher/v1/runtime/client/stable/windows-x86_64", &[]).await;
    assert_eq!((paused.status, paused.body.len()), (204, 0));
    assert_eq!(
        paused.header("cache-control"),
        Some("no-cache, no-store, must-revalidate")
    );
    assert_eq!(
        get(address, "/launcher/v1/admin/updates/serving", &publisher())
            .await
            .text(),
        r#"{"runtimeEnabled":false}"#
    );
    let bad = send(
        address,
        "POST",
        "/launcher/v1/admin/updates/serving",
        &publisher(),
        Some(("application/json", br#"{"runtimeEnabled":"yes"}"#)),
    )
    .await;
    assert_eq!(
        (bad.status, bad.text().as_str()),
        (400, r#"{"error":"runtimeEnabled must be a boolean."}"#)
    );
    assert_eq!(
        get(address, "/launcher/v1/admin/updates/serving", &[]).await.status,
        401
    );
    let remote = server(&env, RouteSet::V1147, &[("UPDATE_PUBLISHER_ALLOWED_IPS", "")]).await;
    assert_eq!(
        get(remote, "/launcher/v1/admin/updates/serving", &publisher())
            .await
            .status,
        403
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_runtime_written_by_node_is_still_served() {
    let env = env();
    let legacy = env.dir.path().join("runtime/client/stable/windows-x86_64");
    std::fs::create_dir_all(legacy.join("1.0.0")).unwrap();
    std::fs::write(
        legacy.join("latest.json"),
        r#"{"version":"1.0.0","targetChangelist":647472,"file":"x.dll","extraFiles":[]}"#,
    )
    .unwrap();
    std::fs::write(legacy.join("1.0.0/x.dll"), b"old").unwrap();
    let address = server(&env, RouteSet::V1147, &[]).await;
    assert_eq!(
        get(address, "/launcher/v1/runtime/client/stable/windows-x86_64", &[])
            .await
            .json()["version"],
        "1.0.0"
    );
    assert_eq!(
        get(
            address,
            "/launcher/v1/runtime/client/stable/windows-x86_64/download",
            &[]
        )
        .await
        .body,
        b"old"
    );
    // It names the changelist it was made for; the other module doesn't take it
    let old = server(&env, RouteSet::V1120, &[]).await;
    assert_eq!(
        get(old, "/launcher/v1/runtime/client/stable/windows-x86_64", &[])
            .await
            .status,
        404
    );
}

struct Accounts;

fn account(roles: &[&str]) -> LauncherAccountRecord {
    LauncherAccountRecord {
        account: AccountId::new(&NodeIdentity::standalone().node),
        email: None,
        display_name_normalized: "x".into(),
        display_name: "X".into(),
        password_hash: None,
        status: OperationalStatus::Active,
        approval_status: None,
        approval_updated_at: None,
        approval_updated_by: None,
        approval_reason: None,
        roles: roles.iter().map(|r| (*r).to_owned()).collect(),
        roles_updated_at: None,
        roles_updated_by: None,
        created_at: iso(0),
        last_login_at: None,
        username_set: Some(true),
        consent: None,
        deletion: None,
        email_verified_at: None,
        email_verification_pending_since: None,
    }
}

impl LauncherAuthenticator for Accounts {
    fn authenticate<'a>(
        &'a self,
        _: &'a Shared,
        token: &'a str,
    ) -> BoxFuture<'a, Result<LauncherCaller, LauncherFailure>> {
        Box::pin(async move {
            let roles: &[&str] = match token {
                "admin" => &["player", "admin"],
                "tester" => &["player", "tester"],
                "player" => &["player"],
                _ => {
                    return Err(LauncherFailure::unauthorized(
                        "Your session has expired. Please sign in again.",
                    ));
                }
            };
            let record = account(roles);
            Ok(LauncherCaller {
                account: record.account.clone(),
                user_id: format!("user-{token}"),
                session: "family".into(),
                record,
            })
        })
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn only_an_admin_reaches_a_non_stable_channel() {
    let _turn = SERIAL.lock().await;
    let env = env();
    let address = server(&env, RouteSet::V1147, &[]).await;
    set_authenticator(None);
    let unauthenticated = get(address, "/launcher/v1/runtime/client/beta/windows-x86_64", &[]).await;
    assert_eq!(unauthenticated.status, 401);
    assert_eq!(unauthenticated.json()["error"]["code"], "AUTH_UNAUTHORIZED");
    assert_eq!(unauthenticated.json()["error"]["message"], "Sign in required.");
    assert_eq!(
        unauthenticated.json()["error"]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        ["code", "message", "requestId"]
    );
    set_authenticator(Some(Arc::new(Accounts)));
    let bearer = |token: &str| [("authorization", format!("Bearer {token}"))];
    let ask = |token: &'static str, path: &'static str| {
        let header = bearer(token);
        async move { get(address, path, &[("authorization", header[0].1.as_str())]).await }
    };
    for path in [
        "/launcher/v1/runtime/client/beta/windows-x86_64",
        "/launcher/v1/runtime/beta/windows-x86_64",
        "/launcher/v1/guard-manifests/dev/windows-x86_64",
    ] {
        let player = ask("player", path).await;
        assert_eq!(player.status, 409, "{path}");
        assert_eq!(player.json()["error"]["code"], "AUTH_CHANNEL_MISMATCH");
        assert_eq!(ask("tester", path).await.status, 409);
        assert_eq!(ask("nobody", path).await.status, 401);
        // An admin passes the gate and finds nothing published
        assert_eq!(ask("admin", path).await.status, 404, "{path}");
    }
    // The stable channel stays public
    assert_eq!(
        get(address, "/launcher/v1/runtime/client/stable/windows-x86_64", &[])
            .await
            .status,
        404
    );
    set_authenticator(None);
}

fn guard_envelope(key: &SigningKey, key_id: &str, sequence: i64, id: &str, channel: &str, changelist: u32) -> Value {
    let now = now_ms();
    let payload = json!({
        "schema": 1,
        "manifestId": id,
        "sequence": sequence,
        "channel": channel,
        "platform": "windows-x86_64",
        "targetChangelist": changelist,
        "minimumLauncherVersion": "0.1.65",
        "issuedAt": iso(now - 3_600_000),
        "expiresAt": iso(now + 30 * 86_400_000),
        "artifacts": [
            {"name": "Game.exe", "role": "GAME", "size": 10, "sha256": "ab".repeat(32), "required": true},
            {"name": "winmm.dll", "role": "UDP_PROXY", "size": 5, "sha256": "CD".repeat(32), "required": true,
             "protectedRegions": [{"rva": 0, "size": 16, "sha256": "ef".repeat(32)}]}
        ],
        "executableMemoryPolicy": {"maximumPrivateExecutableBytes": 1024, "allowlistedModuleHashes": []}
    });
    let bytes = payload.to_string().into_bytes();
    json!({
        "schema": 1,
        "keyId": key_id,
        "payloadBase64": base64_encode(&bytes),
        "signature": base64_encode(&key.sign(&bytes).to_bytes()),
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn guard_manifests_are_signed_and_only_ever_increase() {
    let env = env();
    let shared = shared_for(&env, RouteSet::V1147, &[]);
    let ctx = Ctx::with(&shared, trusted());
    let route = params(&[("channel", "stable"), ("platform", "windows-x86_64")]);
    let id = "11111111-2222-3333-4444-555555555555";
    let publish = |envelope: Value, headers: &[(&str, &str)]| {
        let req = request(headers, None, Some(envelope));
        publish_guard(&ctx, &req, &route)
    };
    let first = guard_envelope(&guard_key(), "guard-2026-10", 5, id, "stable", 647_472);
    let (status, answer) = body_of(publish(first.clone(), &[])).await;
    assert_eq!(
        (status, answer),
        (401, json!({"error": "Invalid update publisher credentials."}))
    );
    let (status, answer) = body_of(publish(first.clone(), &publisher())).await;
    assert_eq!(
        (status, answer),
        (201, json!({"manifestId": id, "sequence": 5, "keyId": "guard-2026-10"}))
    );
    // Same sequence, a lower one, another channel, a wrong key id, a wrong changelist, a forged signature
    let again = guard_envelope(
        &guard_key(),
        "guard-2026-10",
        5,
        "22222222-2222-3333-4444-555555555555",
        "stable",
        647_472,
    );
    let (status, answer) = body_of(publish(again, &publisher())).await;
    assert_eq!(
        (status, answer),
        (
            409,
            json!({"error": "Guard manifest sequence must increase monotonically."})
        )
    );
    let beta = guard_envelope(&guard_key(), "guard-2026-10", 6, id, "beta", 647_472);
    let (status, answer) = body_of(publish(beta, &publisher())).await;
    assert_eq!(
        (status, answer),
        (
            400,
            json!({"error": "Guard manifest route and signed payload do not match."})
        )
    );
    let other_cl = guard_envelope(&guard_key(), "guard-2026-10", 6, id, "stable", 392_819);
    assert_eq!(
        body_of(publish(other_cl, &publisher())).await,
        (400, json!({"error": "Invalid Guard manifest payload."}))
    );
    let forged = guard_envelope(&runtime_key(), "guard-2026-10", 6, id, "stable", 647_472);
    assert_eq!(
        body_of(publish(forged, &publisher())).await,
        (400, json!({"error": "Guard manifest signature verification failed."}))
    );
    let unknown = guard_envelope(&guard_key(), "guard-2030", 6, id, "stable", 647_472);
    assert_eq!(
        body_of(publish(unknown, &publisher())).await,
        (400, json!({"error": "Unknown Guard manifest signing key."}))
    );
    // The runtime key signs Guard manifests too, until older launchers are gone
    let by_runtime = guard_envelope(
        &runtime_key(),
        "runtime-2026-01",
        6,
        "33333333-2222-3333-4444-555555555555",
        "stable",
        647_472,
    );
    assert_eq!(body_of(publish(by_runtime.clone(), &publisher())).await.0, 201);

    let on_disk = env.dir.path().join("guard/647472/stable/windows-x86_64");
    assert!(on_disk.join("5/11111111-2222-3333-4444-555555555555.json").is_file());
    assert_eq!(
        serde_json::from_str::<Value>(&std::fs::read_to_string(on_disk.join("latest.json")).unwrap()).unwrap(),
        by_runtime
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_launcher_feed_offers_only_what_it_can_serve() {
    let env = env();
    let dir = env.dir.path().join("launcher/windows/x86_64");
    std::fs::create_dir_all(dir.join("0.2.0")).unwrap();
    let manifest = |extra: Value| {
        let mut base = json!({
            "version": "0.2.0", "notes": "n", "pub_date": "2026-10-20T00:00:00Z",
            "url": "https://updates.example.test/launcher/v1/updates/windows/x86_64/files/0.2.0/App_0.2.0_x64-setup.exe",
            "signature": "sig", "file": "App_0.2.0_x64-setup.exe", "sha256": "00", "size": 3
        });
        for (k, v) in extra.as_object().unwrap() {
            if v.is_null() {
                base.as_object_mut().unwrap().remove(k);
            } else {
                base[k] = v.clone();
            }
        }
        std::fs::write(dir.join("latest.json"), base.to_string()).unwrap();
    };
    let address = server(&env, RouteSet::V1147, &[]).await;
    let ask = |version: &'static str| async move {
        get(address, &format!("/launcher/v1/updates/windows/x86_64/{version}"), &[]).await
    };

    let nothing = ask("0.1.0").await;
    assert_eq!(nothing.status, 204);
    manifest(json!({}));
    // The installer isn't on this server yet
    assert_eq!(ask("0.1.0").await.status, 204);
    std::fs::write(dir.join("0.2.0/App_0.2.0_x64-setup.exe"), b"exe").unwrap();
    let offered = ask("0.1.0").await;
    assert_eq!(offered.status, 200);
    assert_eq!(
        offered.header("cache-control"),
        Some("no-cache, no-store, must-revalidate")
    );
    assert_eq!(
        offered.json().as_object().unwrap().keys().collect::<Vec<_>>(),
        ["version", "notes", "pub_date", "url", "signature"]
    );
    assert_eq!(ask("0.2.0").await.status, 204);
    assert_eq!(ask("0.10.0").await.status, 204);
    assert_eq!(
        ask("whatever").await.status,
        200,
        "an unreadable current version is offered the update"
    );
    manifest(json!({"signature": ""}));
    assert_eq!(ask("0.1.0").await.status, 204);
    manifest(json!({"url": "http://updates.example.test/x"}));
    assert_eq!(ask("0.1.0").await.status, 204);
    manifest(json!({"url": "https://evil.example/launcher/v1/updates/x"}));
    assert_eq!(ask("0.1.0").await.status, 204);
    manifest(json!({"version": "0.2"}));
    assert_eq!(ask("0.1.0").await.status, 204);
    manifest(json!({}));

    // "download" is a version too, but the literal route is first
    let file = get(address, "/launcher/v1/updates/windows/x86_64/download", &[]).await;
    assert_eq!((file.status, file.body.clone()), (200, b"exe".to_vec()));
    assert_eq!(
        file.header("cache-control"),
        Some("no-cache, no-store, must-revalidate")
    );
    let pinned = get(
        address,
        "/launcher/v1/updates/windows/x86_64/files/0.2.0/App_0.2.0_x64-setup.exe",
        &[],
    )
    .await;
    assert_eq!(pinned.status, 200);
    assert_eq!(
        pinned.header("content-disposition"),
        Some("attachment; filename=\"App_0.2.0_x64-setup.exe\"")
    );
    assert_eq!(
        pinned.header("cache-control"),
        Some("public, max-age=31536000, immutable")
    );
    assert_eq!(
        get(
            address,
            "/launcher/v1/updates/windows/x86_64/files/0.2.0/Other_0.2.0_x64-setup.exe",
            &[]
        )
        .await
        .status,
        404
    );
    assert_eq!(
        get(
            address,
            "/launcher/v1/updates/windows/x86_64/files/0.3.0/App_0.2.0_x64-setup.exe",
            &[]
        )
        .await
        .status,
        404
    );
    assert_eq!(
        get(address, "/launcher/v1/updates/windows/x86_64/files/0.2.0/evil.exe", &[])
            .await
            .status,
        404
    );
    assert_eq!(
        get(address, "/launcher/v1/updates/win%2Fdows/x86_64/0.1.0", &[])
            .await
            .status,
        400
    );

    // A release hosted elsewhere is redirected to, never served
    manifest(
        json!({"file": null, "url": "https://github.com/pranav158/Mystic-Paradox/releases/download/v0.2.0/App.exe"}),
    );
    let redirect = get(address, "/launcher/v1/updates/windows/x86_64/download", &[]).await;
    assert_eq!(redirect.status, 302);
    assert_eq!(
        redirect.header("location"),
        Some("https://github.com/pranav158/Mystic-Paradox/releases/download/v0.2.0/App.exe")
    );
    assert_eq!(ask("0.1.0").await.status, 200);
    // Our own route is never a redirect target (it would loop)
    manifest(json!({"file": null}));
    assert_eq!(
        get(address, "/launcher/v1/updates/windows/x86_64/download", &[])
            .await
            .status,
        404
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn testers_upload_launcher_logs_and_the_sync_script_reads_them() {
    let _turn = SERIAL.lock().await;
    let env = env();
    let address = server(&env, RouteSet::V1147, &[]).await;
    let session = "abcdef12-3456-7890";
    let put = |token: &'static str, name: &'static str, body: &'static [u8]| async move {
        let path = format!("/launcher/v1/logs/sessions/{session}/{name}");
        let header = format!("Bearer {token}");
        send(
            address,
            "PUT",
            &path,
            &[("authorization", header.as_str())],
            Some(("application/octet-stream", body)),
        )
        .await
    };
    set_authenticator(None);
    let anonymous = send(
        address,
        "PUT",
        &format!("/launcher/v1/logs/sessions/{session}/launcher.log"),
        &[],
        Some(("application/octet-stream", b"x")),
    )
    .await;
    assert_eq!(anonymous.status, 401);
    set_authenticator(Some(Arc::new(Accounts)));

    let player = put("player", "launcher.log", b"x").await;
    assert_eq!(player.status, 403);
    assert_eq!(player.json()["error"]["code"], "AUTH_TESTER_REQUIRED");
    assert_eq!(
        player.json()["error"]["message"],
        "Tester access is required to upload logs."
    );
    let bad_name = put("tester", "evil.exe", b"x").await;
    assert_eq!(
        (bad_name.status, bad_name.json()["error"]["message"].clone()),
        (400, json!("Invalid session id or file name."))
    );
    assert_eq!(bad_name.json()["error"]["code"], "AUTH_VALIDATION_FAILED");
    let empty = put("tester", "launcher.log", b"").await;
    assert_eq!(
        (empty.status, empty.json()["error"]["message"].clone()),
        (400, json!("The uploaded file was empty."))
    );
    let wrong_type = send(
        address,
        "PUT",
        &format!("/launcher/v1/logs/sessions/{session}/launcher.log"),
        &[("authorization", "Bearer tester")],
        Some(("text/plain", b"x")),
    )
    .await;
    assert_eq!(wrong_type.json()["error"]["message"], "The uploaded file was empty.");

    let ok = put("tester", "launcher.log", b"log text").await;
    assert_eq!(ok.status, 201);
    let at = ok.json()["uploadedAt"].as_str().unwrap().to_owned();
    assert_eq!(at.len(), 24);
    let dir = env.logs.path().join("user-tester").join(session);
    assert_eq!(std::fs::read(dir.join("launcher.log")).unwrap(), b"log text");
    assert_eq!(
        std::fs::read_to_string(dir.join(".uploaded-at.json")).unwrap(),
        format!("{{\"lastUploadedAt\":\"{at}\"}}\n")
    );
    assert_eq!(put("tester", "metadata.json", b"{}").await.status, 201);

    // The sync script has a credential of its own
    let sync = |path: String, token: Option<&'static str>| async move {
        match token {
            Some(token) => get(address, &path, &[("x-sync-token", token)]).await,
            None => get(address, &path, &[]).await,
        }
    };
    let refused = sync("/admin/v1/launcher-logs".into(), None).await;
    assert_eq!(
        (refused.status, refused.text().as_str()),
        (401, r#"{"error":"Invalid sync credentials."}"#)
    );
    assert_eq!(sync("/admin/v1/launcher-logs".into(), Some("wrong")).await.status, 401);
    let listing = sync("/admin/v1/launcher-logs".into(), Some("sync-token")).await.json();
    assert_eq!(
        listing,
        json!({"sessions": [{"userId": "user-tester", "launchSessionId": session, "uploadedAt": listing["sessions"][0]["uploadedAt"], "files": ["launcher.log", "metadata.json"]}]})
    );
    let since = sync(
        format!(
            "/admin/v1/launcher-logs?since={}",
            listing["sessions"][0]["uploadedAt"].as_str().unwrap()
        ),
        Some("sync-token"),
    )
    .await;
    assert_eq!(since.json(), json!({"sessions": []}));
    let file = sync(
        format!("/admin/v1/launcher-logs/user-tester/{session}/launcher.log"),
        Some("sync-token"),
    )
    .await;
    assert_eq!((file.status, file.body), (200, b"log text".to_vec()));
    assert_eq!(
        sync(
            format!("/admin/v1/launcher-logs/user-tester/{session}/nothing.log"),
            Some("sync-token")
        )
        .await
        .status,
        404
    );
    let traversal = sync("/admin/v1/launcher-logs/..%2F/x/y".into(), Some("sync-token")).await;
    assert_eq!(traversal.status, 400);
    assert_eq!(traversal.text(), r#"{"error":"Invalid path."}"#);
    set_authenticator(None);

    // With the admin API off the sync routes are the shared 404
    let off = server(&env, RouteSet::V1147, &[("MYSTICPARADOX_ADMIN_API", "off")]).await;
    assert_eq!(
        get(off, "/admin/v1/launcher-logs", &[("x-sync-token", "sync-token")])
            .await
            .status,
        404
    );
}

#[allow(dead_code)]
fn unused(_: Answer) {}
