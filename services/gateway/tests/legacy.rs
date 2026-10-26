#![cfg(feature = "legacy")]
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

//! A gateway in front of a Dauntless Revived 1.4.4 server (`GATEWAY_GAME=legacy`): DR's metagame
//! routes (names translated as `services/metagame` has them), its host-only routes and its allowlist
//! feed, against fake upstreams.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use gateway::config::Allowlist;
use gateway::feed::AllowlistFeed;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const HELPER_SECRET: &str = "test-helper-secret-abcdefabcdefabcdefabcdefabcdef";

struct World {
    _cert: TestCert,
    metagame: Upstream,
    content: Upstream,
    helper: Upstream,
    ws: WsUpstream,
    feed: AllowlistFeed,
    gateway: Running,
}

async fn world() -> World {
    let cert = make_cert();
    let handler: Handler = Arc::new(|seen: Seen| {
        Box::pin(async move {
            let path = seen.url.split('?').next().unwrap().to_lowercase();
            let bearer = seen.headers.get("authorization").map(String::as_str) == Some("Bearer good-token");
            match path.as_str() {
                "/heartbeat" | "/game/login" => Some((
                    if bearer { 200 } else { 401 },
                    vec![("x-reforged-gateway", "leaked".to_owned())],
                    b"{}".to_vec(),
                )),
                _ => None,
            }
        })
    });
    let metagame = start_upstream("metagame", Some(handler)).await;
    // A Mystic Paradox backend that must never be reached
    let content = start_upstream("content", None).await;
    let helper = start_upstream(
        "helper",
        Some(Arc::new(|_| {
            Box::pin(async {
                Some((
                    200,
                    vec![("content-type", "application/json".to_owned())],
                    b"{}".to_vec(),
                ))
            })
        })),
    )
    .await;
    let ws = start_ws_upstream().await;
    let feed = AllowlistFeed::new(&Allowlist {
        url: format!("http://127.0.0.1:{}", helper.port),
        secret: HELPER_SECRET.into(),
        refresh: Duration::from_secs(60),
        timeout: Duration::from_secs(2),
        additional_urls: Vec::new(),
    });
    let backend = dead_port().await;
    let ports = Ports {
        backend,
        content: content.port,
        ws: ws.port,
    };
    let metagame_url = format!("http://127.0.0.1:{}", metagame.port);
    let config = test_config(
        &cert,
        &ports,
        &[("GATEWAY_GAME", "legacy"), ("GATEWAY_METAGAME_URL", &metagame_url)],
    );
    let gateway = start_gateway(&cert, config, Some(feed.clone())).await;
    World {
        _cert: cert,
        metagame,
        content,
        helper,
        ws,
        feed,
        gateway,
    }
}

fn post(body: &str) -> Options<'static> {
    Options {
        method: Some("POST"),
        body: Some(body.as_bytes().to_vec()),
        ..Options::default()
    }
}

fn with(mut options: Options<'static>, name: &'static str, value: &str) -> Options<'static> {
    options.headers.push((name, value.to_owned()));
    options
}

fn from(mut options: Options<'static>, local: &'static str) -> Options<'static> {
    options.local = Some(local);
    options
}

#[tokio::test(flavor = "multi_thread")]
async fn proxies_the_metagame_and_never_the_backend() {
    let w = world().await;
    let port = w.gateway.port;
    let reply = https_request(
        port,
        "/party/invites",
        with(Options::default(), "x-reforged-gateway", "forged"),
    )
    .await
    .unwrap();
    assert_eq!(reply.status, 200);
    assert_eq!(reply.json()["upstream"], "metagame");
    let seen = w.metagame.last();
    assert_eq!(seen.headers["x-reforged-gateway"], GATEWAY_SECRET);
    assert_eq!(seen.headers["x-forwarded-proto"], "https");
    assert!(!reply.headers.contains_key("x-powered-by"));

    for target in [
        "/reforged/api/GetUserInfo",
        "/reforged/api/ServerStatus",
        "/reforged/api/HuntRegion",
    ] {
        assert_eq!(
            https_request(port, target, Options::default()).await.unwrap().status,
            200
        );
    }
    // The secret never comes back out
    let heartbeat = https_request(port, "/heartbeat", post("{}")).await.unwrap();
    assert!(!heartbeat.headers.contains_key("x-reforged-gateway"));

    let content = https_request(port, "/content/v1/files/Version.txt", Options::default())
        .await
        .unwrap();
    assert_eq!(content.json()["upstream"], "content");
    assert_eq!(w.content.last().headers["x-reforged-gateway"], GATEWAY_SECRET);
}

#[tokio::test(flavor = "multi_thread")]
async fn refuses_the_metagames_host_only_routes_and_header() {
    let w = world().await;
    let port = w.gateway.port;
    let before = w.metagame.count();
    for (target, options) in [
        ("/reforged/api/CreateInvite", post("{}")),
        ("/reforged/api/GrantEntitlement", post("{}")),
        ("/Reforged/Api/RenameUser/", post("{}")),
        ("/internal/backup", post("{}")),
        ("/internal/key-recovery/start", post("{}")),
        ("/heartbeat", with(post("{}"), "x-mysticparadox-gameserver-apikey", "k")),
    ] {
        let reply = https_request(port, target, options).await.unwrap();
        assert_eq!(reply.status, 403, "{target}");
        assert_eq!(reply.json()["error"], "forbidden", "{target}");
    }
    assert_eq!(w.metagame.count(), before);
}

#[tokio::test(flavor = "multi_thread")]
async fn heartbeats_and_game_logins_with_a_bearer_open_the_game_ports() {
    let w = world().await;
    let port = w.gateway.port;
    let allowed = || -> Vec<String> {
        w.helper
            .seen()
            .iter()
            .filter(|s| s.url == "/allow")
            .map(|s| {
                serde_json::from_slice::<serde_json::Value>(&s.body).unwrap()["ip"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect()
    };
    https_request(port, "/game/login", from(post("{}"), "127.0.0.4"))
        .await
        .unwrap();
    w.feed.idle().await;
    assert!(allowed().is_empty());
    for (target, local) in [("/heartbeat", "127.0.0.5"), ("/game/login", "127.0.0.6")] {
        let reply = https_request(
            port,
            target,
            from(with(post("{}"), "authorization", "Bearer good-token"), local),
        )
        .await
        .unwrap();
        assert_eq!(reply.status, 200);
    }
    w.feed.idle().await;
    let mut ips = allowed();
    ips.sort();
    assert_eq!(ips, ["127.0.0.5", "127.0.0.6"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn chat_goes_to_the_metagame_chat_listener() {
    let w = world().await;
    let mut result = upgrade(w.gateway.port, "/xmpp", &[("Sec-WebSocket-Protocol", "xmpp")], None).await;
    assert_eq!(result.status, 101);
    result.stream.write_all(b"hi").await.unwrap();
    let mut got = vec![0u8; "echo:hi".len()];
    tokio::time::timeout(Duration::from_secs(3), result.stream.read_exact(&mut got))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got, b"echo:hi");
    let (_, seen) = w.ws.last();
    assert_eq!(seen["x-reforged-gateway"], GATEWAY_SECRET);
    assert_eq!(
        upgrade(w.gateway.port, "/reforged/api/ServerStatus", &[], None)
            .await
            .status,
        403
    );
}
