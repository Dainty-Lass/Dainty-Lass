#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

//! The network's routes the gateway answers itself (DN7): the node list, the ping and the region
//! report stub. None of them reaches an upstream.

mod common;

use common::*;
use gateway::network::NetworkNode;
use serde_json::json;

const FAKE_KEY: &str = "RRK_fedcba9876543210fedcba9876543210fedcba9876543210";

struct World {
    _cert: TestCert,
    backend: Upstream,
    content: Upstream,
    gateway: Running,
}

async fn world(nodes: Vec<NetworkNode>) -> World {
    let cert = make_cert();
    let backend = start_upstream("backend", None).await;
    let content = start_upstream("content", None).await;
    let ports = Ports {
        backend: backend.port,
        content: content.port,
        ws: dead_port().await,
    };
    let mut config = test_config(&cert, &ports, &[]);
    config.network = nodes;
    let gateway = start_gateway(&cert, config, None).await;
    World {
        _cert: cert,
        backend,
        content,
        gateway,
    }
}

fn post(body: &str, key: Option<&str>) -> Options<'static> {
    Options {
        method: Some("POST"),
        headers: key
            .map(|key| vec![("x-reforged-user-api-key", key.to_owned())])
            .unwrap_or_default(),
        body: Some(body.as_bytes().to_vec()),
        ..Options::default()
    }
}

#[tokio::test]
async fn a_server_outside_a_network_lists_no_nodes() {
    let _serial = serial().await;
    let w = world(Vec::new()).await;
    let reply = https_request(w.gateway.port, "/network/v1/nodes", Options::default())
        .await
        .unwrap();
    assert_eq!(reply.status, 200);
    assert_eq!(reply.text(), r#"{"nodes":[]}"#);
    assert_eq!(reply.headers["content-type"], "application/json");
    assert_eq!(reply.headers["cache-control"], "no-store");
    let ping = https_request(w.gateway.port, "/network/v1/ping", Options::default())
        .await
        .unwrap();
    assert_eq!(ping.status, 204);
    assert!(ping.body.is_empty());
    assert_eq!(w.backend.count() + w.content.count(), 0);
    w.gateway.close().await;
}

#[tokio::test]
async fn a_node_lists_its_network() {
    let _serial = serial().await;
    let nodes = vec![
        NetworkNode {
            region: "oregon".into(),
            host: "or.example.org".into(),
            port: 443,
            fingerprint: "ab".repeat(32),
        },
        NetworkNode {
            region: "uk".into(),
            host: "203.0.113.7".into(),
            port: 8443,
            fingerprint: "cd".repeat(32),
        },
    ];
    let w = world(nodes).await;
    let reply = https_request(w.gateway.port, "/network/v1/nodes", Options::default())
        .await
        .unwrap();
    assert_eq!(
        reply.json(),
        json!({ "nodes": [
            { "region": "oregon", "host": "or.example.org", "port": 443, "fingerprint": "ab".repeat(32) },
            { "region": "uk", "host": "203.0.113.7", "port": 8443, "fingerprint": "cd".repeat(32) },
        ] })
    );
    assert_eq!(w.backend.count(), 0);
    w.gateway.close().await;
}

#[tokio::test]
async fn the_region_report_needs_a_key_and_a_well_formed_list_and_stores_nothing() {
    let _serial = serial().await;
    let w = world(Vec::new()).await;
    let port = w.gateway.port;
    let report = r#"{"regions":[{"region":"uk","rttMs":18.5},{"region":"oregon","rttMs":null}]}"#;
    let done = https_request(port, "/network/v1/regions", post(report, Some(FAKE_KEY)))
        .await
        .unwrap();
    assert_eq!(done.status, 204);
    let no_key = https_request(port, "/network/v1/regions", post(report, None))
        .await
        .unwrap();
    assert_eq!(
        (no_key.status, no_key.json()),
        (401, json!({ "error": "unauthorized" }))
    );
    let odd_key = https_request(port, "/network/v1/regions", post(report, Some("RRK x")))
        .await
        .unwrap();
    assert_eq!(odd_key.status, 401);
    let bad = https_request(
        port,
        "/network/v1/regions",
        post(r#"{"regions":[{"region":"UK","rttMs":1}]}"#, Some(FAKE_KEY)),
    )
    .await
    .unwrap();
    assert_eq!((bad.status, bad.json()), (400, json!({ "error": "bad_request" })));
    let big = format!(r#"{{"regions":[],"pad":"{}"}}"#, "x".repeat(5000));
    let too_big = https_request(port, "/network/v1/regions", post(&big, Some(FAKE_KEY)))
        .await
        .unwrap();
    assert_eq!(too_big.status, 413);
    let wrong_method = https_request(port, "/network/v1/regions", Options::default())
        .await
        .unwrap();
    assert_eq!(wrong_method.status, 405);
    assert_eq!(w.backend.count() + w.content.count(), 0);
    w.gateway.close().await;
}

#[tokio::test]
async fn the_games_wrapped_calls_still_go_to_the_backend() {
    let _serial = serial().await;
    let w = world(Vec::new()).await;
    let reply = https_request(
        w.gateway.port,
        "/__origin/gamedev-prod.steelyard.ca/network/v1/nodes",
        Options::default(),
    )
    .await
    .unwrap();
    assert_eq!(reply.json()["upstream"], "backend");
    assert_eq!(w.backend.count(), 1);
    w.gateway.close().await;
}
