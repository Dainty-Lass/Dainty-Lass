// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

//! Node links on the gateway's public port (DN3): several nodes on 127.0.0.1:0, each a real gateway, call
//! each other through `TlsPeerLink` over mutual TLS under one cluster CA.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use dainty_gateway::gateway::{Gateway, Tls as GatewayTls};
use dainty_module_api::BoxFuture;
use dainty_module_api::network::NodeId;
use network_plugin::cluster::{ClusterCa, NodeCertRequest, NodeCredentials, Peer};
use network_plugin::{LinkError, NodeService, PeerLink, PeerState, Service, TlsLinkConfig, TlsPeerLink};
use rustls::pki_types::pem::PemObject as _;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::watch;

fn id(text: &str) -> NodeId {
    text.parse().unwrap()
}

/// Answers every call with who called and what it said; remembers the callers.
#[derive(Default)]
struct Echo {
    seen: Mutex<Vec<String>>,
}

impl NodeService for Echo {
    fn serve<'a>(
        &'a self,
        from: &'a NodeId,
        _service: Service,
        body: Vec<u8>,
    ) -> BoxFuture<'a, Result<Vec<u8>, LinkError>> {
        Box::pin(async move {
            self.seen.lock().unwrap().push(from.to_string());
            Ok(format!("{from}:{}", String::from_utf8_lossy(&body)).into_bytes())
        })
    }
}

struct Cluster {
    ca: ClusterCa,
    dir: tempfile::TempDir,
}

struct Files {
    cert: String,
    key: String,
    ca: String,
}

impl Cluster {
    fn new() -> Self {
        Self {
            ca: ClusterCa::create(30).unwrap(),
            dir: tempfile::tempdir().unwrap(),
        }
    }

    /// The files of `node`, signed by `ca` (the cluster's or a foreign one).
    fn files(&self, ca: &ClusterCa, node: &str, region: &str, tag: &str) -> Files {
        let issued = ca
            .issue(&NodeCertRequest {
                node: id(node),
                region: region.parse().unwrap(),
                hosts: vec!["127.0.0.1".into()],
                days: 30,
            })
            .unwrap();
        let path = |name: &str| self.dir.path().join(format!("{node}-{tag}-{name}.pem"));
        let write = |name: &str, text: &str| {
            std::fs::write(path(name), text).unwrap();
            path(name).to_string_lossy().into_owned()
        };
        Files {
            cert: write("cert", &issued.cert_pem),
            key: write("key", &issued.key_pem),
            // What a node holds of the CA: the certificate of the cluster it is meant to be in
            ca: write("ca", self.ca.certificate_pem()),
        }
    }

    fn credentials(&self, files: &Files) -> NodeCredentials {
        NodeCredentials::load(files.cert.as_ref(), files.key.as_ref(), files.ca.as_ref()).unwrap()
    }
}

fn peer(id_text: &str, region: &str, port: u16) -> Peer {
    Peer {
        id: id(id_text),
        region: region.parse().unwrap(),
        host: "127.0.0.1".into(),
        port,
        fingerprint: None,
        link: None,
    }
}

/// One node's gateway, running until closed.
struct Running {
    port: u16,
    gateway: Arc<Gateway>,
    echo: Arc<Echo>,
    stop: watch::Sender<bool>,
    task: tokio::task::JoinHandle<()>,
    _public: TestCert,
}

impl Running {
    async fn close(self) {
        let _ = self.stop.send(true);
        let _ = self.task.await;
    }
}

/// A node's gateway on `listener`, listing `peers`, with `Echo` answering its calls.
async fn start_node(files: &Files, (node, region): (&str, &str), listener: TcpListener, peers: &[Peer]) -> Running {
    let public = make_cert();
    let ports = Ports {
        backend: dead_port().await,
        content: dead_port().await,
        ws: dead_port().await,
    };
    let nodes = serde_json::to_string(peers).unwrap();
    let config = test_config(
        &public,
        &ports,
        &[
            ("GATEWAY_NODE_ID", node),
            ("GATEWAY_NODE_REGION", region),
            ("GATEWAY_NODE_CERT", &files.cert),
            ("GATEWAY_NODE_KEY", &files.key),
            ("GATEWAY_NODE_CA", &files.ca),
            ("GATEWAY_NETWORK_NODES", &nodes),
        ],
    );
    let loaded = dainty_gateway::cert::load(&public.cert_file, &public.key_file).unwrap();
    let tls = GatewayTls {
        config: dainty_gateway::cert::server_config(&loaded).unwrap(),
        info: loaded.info,
    };
    let port = listener.local_addr().unwrap().port();
    let gateway = Gateway::new(config, Some(tls), None, None).unwrap();
    let echo = Arc::new(Echo::default());
    let service: Arc<dyn NodeService> = echo.clone();
    gateway.node_links().unwrap().endpoint().attach(service);
    let (stop, stopping) = watch::channel(false);
    let task = tokio::spawn(gateway.clone().serve(listener, stopping));
    Running {
        port,
        gateway,
        echo,
        stop,
        task,
        _public: public,
    }
}

async fn reserve() -> TcpListener {
    TcpListener::bind("127.0.0.1:0").await.unwrap()
}

fn link_to(creds: NodeCredentials, peers: Vec<Peer>) -> TlsPeerLink {
    let mut config = TlsLinkConfig::new(creds, peers);
    config.connect_timeout = Duration::from_secs(3);
    config.request_timeout = Duration::from_secs(5);
    config.ping_interval = Duration::from_millis(100);
    TlsPeerLink::new(config).unwrap()
}

/// `oregon` and `uk`, listing each other, with a link each way.
struct Pair {
    cluster: Cluster,
    oregon: Running,
    uk: Running,
    oregon_link: TlsPeerLink,
    uk_link: TlsPeerLink,
}

async fn pair() -> Pair {
    let cluster = Cluster::new();
    let (oregon_listener, uk_listener) = (reserve().await, reserve().await);
    let ports = (
        oregon_listener.local_addr().unwrap().port(),
        uk_listener.local_addr().unwrap().port(),
    );
    let (oregon_files, uk_files) = (
        cluster.files(&cluster.ca, "oregon-1", "oregon", "a"),
        cluster.files(&cluster.ca, "uk-1", "uk", "a"),
    );
    let (to_uk, to_oregon) = (peer("uk-1", "uk", ports.1), peer("oregon-1", "oregon", ports.0));
    let oregon = start_node(
        &oregon_files,
        ("oregon-1", "oregon"),
        oregon_listener,
        std::slice::from_ref(&to_uk),
    )
    .await;
    let uk = start_node(&uk_files, ("uk-1", "uk"), uk_listener, std::slice::from_ref(&to_oregon)).await;
    let oregon_link = link_to(cluster.credentials(&oregon_files), vec![to_uk]);
    let uk_link = link_to(cluster.credentials(&uk_files), vec![to_oregon]);
    Pair {
        cluster,
        oregon,
        uk,
        oregon_link,
        uk_link,
    }
}

async fn call(link: &TlsPeerLink, to: &str, body: &str) -> Result<String, LinkError> {
    link.call(&id(to), Service::Directory, body.as_bytes().to_vec())
        .await
        .map(|answer| String::from_utf8(answer).unwrap())
}

#[tokio::test]
async fn nodes_call_each_other_and_the_service_is_told_who_called() {
    let p = pair().await;
    assert_eq!(call(&p.oregon_link, "uk-1", "hello").await.unwrap(), "oregon-1:hello");
    assert_eq!(call(&p.uk_link, "oregon-1", "back").await.unwrap(), "uk-1:back");
    // Same connection again, and many at once
    let bodies: Vec<String> = (0..8).map(|n| n.to_string()).collect();
    let calls: Vec<_> = bodies.iter().map(|body| call(&p.oregon_link, "uk-1", body)).collect();
    for (n, answer) in futures_util::future::join_all(calls).await.into_iter().enumerate() {
        assert_eq!(answer.unwrap(), format!("oregon-1:{n}"));
    }
    assert!(p.uk.echo.seen.lock().unwrap().iter().all(|from| from == "oregon-1"));
    let health = p.oregon_link.peer_health(&id("uk-1")).unwrap();
    assert_eq!(health.state, PeerState::Up);
    assert_eq!(
        p.oregon_link.call(&id("nobody"), Service::Directory, vec![]).await,
        Err(LinkError::UnknownPeer(id("nobody")))
    );
    p.oregon.close().await;
    p.uk.close().await;
}

/// A raw request on a node connection: the status line's code and the body.
async fn raw(creds: &NodeCredentials, port: u16, head: &str, body: &[u8]) -> (u16, String) {
    let connector = tokio_rustls::TlsConnector::from(creds.client_config().unwrap());
    let tcp = tcp_connect(port, None).await.unwrap();
    let name = rustls::pki_types::ServerName::try_from("127.0.0.1").unwrap().to_owned();
    let mut tls = connector.connect(name, tcp).await.unwrap();
    let request = format!("{head}\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    tls.write_all(request.as_bytes()).await.unwrap();
    let _ = tls.write_all(body).await;
    let mut reply = Vec::new();
    let _ = tls.read_to_end(&mut reply).await;
    let text = String::from_utf8_lossy(&reply).into_owned();
    let status = text.split(' ').nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    (status, text.split("\r\n\r\n").nth(1).unwrap_or("").to_owned())
}

fn now_ms() -> i64 {
    network_plugin::Clock::default().now_ms()
}

#[tokio::test]
async fn a_node_of_another_cluster_is_refused_and_refuses() {
    let p = pair().await;
    let foreign_ca = ClusterCa::create(30).unwrap();
    // A whole foreign node: its certificate is its own CA's, and it trusts that CA only
    let mut files = p.cluster.files(&foreign_ca, "oregon-1", "oregon", "foreign");
    let ca_path = p.cluster.dir.path().join("foreign-ca.pem");
    std::fs::write(&ca_path, foreign_ca.certificate_pem()).unwrap();
    files.ca = ca_path.to_string_lossy().into_owned();
    let link = link_to(p.cluster.credentials(&files), vec![peer("uk-1", "uk", p.uk.port)]);
    let result = call(&link, "uk-1", "let me in").await;
    // It does not trust uk's certificate, so nothing was sent
    assert!(matches!(result, Err(LinkError::Untrusted { .. })), "{result:?}");
    assert_eq!(link.peer_health(&id("uk-1")).unwrap().state, PeerState::Untrusted);
    // A client that trusts the real CA but shows the foreign certificate: uk's gateway refuses it
    let intruder = foreign_ca
        .issue(&NodeCertRequest {
            node: id("oregon-1"),
            region: "oregon".parse().unwrap(),
            hosts: vec!["127.0.0.1".into()],
            days: 30,
        })
        .unwrap();
    let chain: Vec<_> = rustls::pki_types::CertificateDer::pem_slice_iter(intruder.cert_pem.as_bytes())
        .collect::<Result<_, _>>()
        .unwrap();
    let key = rustls::pki_types::PrivateKeyDer::from_pem_slice(intruder.key_pem.as_bytes()).unwrap();
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(rustls::pki_types::CertificateDer::from_pem_slice(p.cluster.ca.certificate_pem().as_bytes()).unwrap())
        .unwrap();
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_root_certificates(roots)
        .with_client_auth_cert(chain, key)
        .unwrap();
    config.alpn_protocols = vec![network_plugin::cluster::NODE_ALPN.to_vec()];
    let tcp = tcp_connect(p.uk.port, None).await.unwrap();
    let name = rustls::pki_types::ServerName::try_from("127.0.0.1").unwrap().to_owned();
    let mut served = String::new();
    if let Ok(mut tls) = tokio_rustls::TlsConnector::from(Arc::new(config))
        .connect(name, tcp)
        .await
    {
        let head = "POST /__node/directory HTTP/1.1\r\nHost: x\r\nContent-Length: 2\r\nx-node-time: 1\r\n\r\n{}";
        let _ = tls.write_all(head.as_bytes()).await;
        let mut reply = Vec::new();
        let _ = tls.read_to_end(&mut reply).await;
        served = String::from_utf8_lossy(&reply).into_owned();
    }
    assert!(!served.contains("200"), "{served}");
    assert!(
        p.uk.echo.seen.lock().unwrap().is_empty(),
        "the service heard from an intruder"
    );
    p.oregon.close().await;
    p.uk.close().await;
}

#[tokio::test]
async fn a_cluster_node_the_server_does_not_list_gets_403() {
    let p = pair().await;
    // Signed by the cluster's CA, but uk lists only oregon-1
    let files = p.cluster.files(&p.cluster.ca, "de-1", "germany", "b");
    let link = link_to(p.cluster.credentials(&files), vec![peer("uk-1", "uk", p.uk.port)]);
    let result = call(&link, "uk-1", "hi").await;
    assert!(matches!(result, Err(LinkError::Refused { .. })), "{result:?}");
    assert_eq!(link.peer_health(&id("uk-1")).unwrap().state, PeerState::Refused);
    assert!(p.uk.echo.seen.lock().unwrap().is_empty());
    p.oregon.close().await;
    p.uk.close().await;
}

#[tokio::test]
async fn a_peer_at_the_wrong_address_is_not_that_peer() {
    let p = pair().await;
    // oregon's link believes the node at uk's port is "de-1"
    let files = p.cluster.files(&p.cluster.ca, "oregon-1", "oregon", "c");
    let link = link_to(p.cluster.credentials(&files), vec![peer("de-1", "germany", p.uk.port)]);
    let result = call(&link, "de-1", "hi").await;
    assert!(matches!(result, Err(LinkError::Untrusted { .. })), "{result:?}");
    assert!(p.uk.echo.seen.lock().unwrap().is_empty());
    p.oregon.close().await;
    p.uk.close().await;
}

#[tokio::test]
async fn a_player_connection_cannot_reach_the_node_routes() {
    let _serial = serial().await;
    let p = pair().await;
    for target in [
        "/__node/directory",
        "/__node/ping",
        "/__NODE/ping",
        "/__origin/gamedev-prod.steelyard.ca/__node/directory",
    ] {
        let reply = https_request(p.uk.port, target, Options::default()).await.unwrap();
        assert_eq!(reply.status, 403, "{target}");
        assert_eq!(reply.text(), r#"{"error":"forbidden"}"#, "{target}");
    }
    // The public TLS port is the one the node links share, and nothing about it changed
    assert!(
        https_request(p.uk.port, "/network/v1/ping", Options::default())
            .await
            .unwrap()
            .status
            == 204
    );
    assert!(p.uk.echo.seen.lock().unwrap().is_empty());
    p.oregon.close().await;
    p.uk.close().await;
}

#[tokio::test]
async fn a_node_connection_serves_the_node_routes_and_nothing_else() {
    let p = pair().await;
    let files = p.cluster.files(&p.cluster.ca, "oregon-1", "oregon", "raw");
    let creds = p.cluster.credentials(&files);
    let stamp = format!("x-node-time: {}", now_ms());
    let get = |path: &str| format!("GET {path} HTTP/1.1\r\n{stamp}");
    let (status, body) = raw(&creds, p.uk.port, &get("/__node/ping"), b"").await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains(r#""node":"uk-1""#), "{body}");
    for (head, expected) in [
        (get("/__origin/gamedev-prod.steelyard.ca/__node/ping"), 404),
        (get("/__node/ping?x=1"), 404),
        (get("/__node/PING"), 404),
        (get("/__node/nothing"), 404),
        (get("/content/v1/manifest"), 404),
        (get("/party/invites"), 404),
        (get("/network/v1/nodes"), 404),
        (get("/__node/directory"), 405),
        (format!("POST /__node/ping HTTP/1.1\r\n{stamp}"), 405),
        ("GET /__node/ping HTTP/1.1".to_owned(), 400),
        ("GET /__node/ping HTTP/1.1\r\nx-node-time: soon".to_owned(), 400),
    ] {
        let (status, body) = raw(&creds, p.uk.port, &head, b"").await;
        assert_eq!(status, expected, "{head:?}: {body}");
    }
    assert!(p.uk.echo.seen.lock().unwrap().is_empty());
    p.oregon.close().await;
    p.uk.close().await;
}

#[tokio::test]
async fn a_body_over_the_limit_is_refused_before_it_is_parsed() {
    let p = pair().await;
    assert_eq!(
        p.oregon_link
            .call(
                &id("uk-1"),
                Service::Directory,
                vec![b'x'; network_plugin::link::MAX_MESSAGE + 1]
            )
            .await,
        Err(LinkError::TooLarge)
    );
    // Declared too large: refused from the head
    let files = p.cluster.files(&p.cluster.ca, "oregon-1", "oregon", "big");
    let creds = p.cluster.credentials(&files);
    let head = format!(
        "POST /__node/directory HTTP/1.1\r\nx-node-time: {}\r\nContent-Length: {}",
        now_ms(),
        network_plugin::link::MAX_MESSAGE + 1
    );
    let (status, body) = raw(&creds, p.uk.port, &head, b"").await;
    assert_eq!((status, body.as_str()), (413, r#"{"error":"too_large"}"#));
    // Exactly the limit, each way, goes through; an answer over it is a failure of the service, not delivered
    let max = network_plugin::link::MAX_MESSAGE;
    let exact = vec![b'y'; max - "oregon-1:".len()];
    let answer = p
        .oregon_link
        .call(&id("uk-1"), Service::Directory, exact)
        .await
        .unwrap();
    assert_eq!(answer.len(), max);
    let result = p
        .oregon_link
        .call(&id("uk-1"), Service::Directory, vec![b'y'; max])
        .await;
    assert!(matches!(result, Err(LinkError::Unreachable { .. })), "{result:?}");
    p.oregon.close().await;
    p.uk.close().await;
}

#[tokio::test]
async fn links_survive_a_peer_restart() {
    let p = pair().await;
    assert_eq!(call(&p.oregon_link, "uk-1", "before").await.unwrap(), "oregon-1:before");
    let (files, port) = (p.cluster.files(&p.cluster.ca, "uk-1", "uk", "restart"), p.uk.port);
    p.uk.close().await;
    let down = call(&p.oregon_link, "uk-1", "while down").await;
    assert!(matches!(down, Err(LinkError::Unreachable { .. })), "{down:?}");
    assert_eq!(p.oregon_link.peer_health(&id("uk-1")).unwrap().state, PeerState::Down);
    // The same node comes back on the same address, with a fresh process and certificate
    let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    let uk = start_node(
        &files,
        ("uk-1", "uk"),
        listener,
        &[peer("oregon-1", "oregon", p.oregon.port)],
    )
    .await;
    assert_eq!(call(&p.oregon_link, "uk-1", "after").await.unwrap(), "oregon-1:after");
    let health = p.oregon_link.peer_health(&id("uk-1")).unwrap();
    assert_eq!((health.state, health.failures), (PeerState::Up, 0));
    assert_eq!(uk.echo.seen.lock().unwrap().len(), 1);
    // And the other way, on the connection uk's restart cut
    assert_eq!(
        call(&p.uk_link, "oregon-1", "hello again").await.unwrap(),
        "uk-1:hello again"
    );
    p.oregon.close().await;
    uk.close().await;
}

#[tokio::test]
async fn health_checks_follow_a_peer_down_and_back_up() {
    let p = pair().await;
    let _checks = p.oregon_link.spawn_health_checks();
    let state = |link: &TlsPeerLink| link.peer_health(&id("uk-1")).unwrap().state;
    let until = |want: PeerState, link: TlsPeerLink| async move {
        for _ in 0..100 {
            if state(&link) == want {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("never {want:?}: {:?}", link.health());
    };
    until(PeerState::Up, p.oregon_link.clone()).await;
    let health = p.oregon_link.peer_health(&id("uk-1")).unwrap();
    assert!(health.rtt_ms.is_some() && health.skew_ms.is_some() && health.last_ok_ms.is_some());
    let (files, port) = (p.cluster.files(&p.cluster.ca, "uk-1", "uk", "health"), p.uk.port);
    p.uk.close().await;
    until(PeerState::Down, p.oregon_link.clone()).await;
    let listener = TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    let uk = start_node(
        &files,
        ("uk-1", "uk"),
        listener,
        &[peer("oregon-1", "oregon", p.oregon.port)],
    )
    .await;
    until(PeerState::Up, p.oregon_link.clone()).await;
    p.oregon.close().await;
    uk.close().await;
}

#[tokio::test]
async fn a_peer_whose_clock_is_too_far_off_is_refused_and_reported() {
    let p = pair().await;
    let uk_clock = p.uk.gateway.node_links().unwrap().endpoint().clock().clone();
    assert!(p.oregon_link.ping(&id("uk-1")).await.is_ok());
    // uk's clock runs two minutes fast: oregon refuses its answers, and uk would refuse oregon's requests
    uk_clock.skew_by(120_000);
    let ping = p.oregon_link.ping(&id("uk-1")).await;
    assert!(
        matches!(ping, Err(LinkError::ClockSkew { skew_ms, .. }) if skew_ms > 100_000),
        "{ping:?}"
    );
    let called = call(&p.oregon_link, "uk-1", "hi").await;
    assert!(matches!(called, Err(LinkError::ClockSkew { .. })), "{called:?}");
    let health = p.oregon_link.peer_health(&id("uk-1")).unwrap();
    assert_eq!(health.state, PeerState::ClockSkew);
    assert!(health.skew_ms.unwrap() > 100_000 && health.last_error.is_some());
    uk_clock.skew_by(0);
    // The other way: uk refuses requests from a node whose clock is behind, and says so
    let files = p.cluster.files(&p.cluster.ca, "oregon-1", "oregon", "skewed");
    let config = TlsLinkConfig::new(p.cluster.credentials(&files), vec![peer("uk-1", "uk", p.uk.port)]);
    config.clock.skew_by(-120_000);
    let behind = TlsPeerLink::new(config).unwrap();
    let called = call(&behind, "uk-1", "hi").await;
    assert!(
        matches!(called, Err(LinkError::ClockSkew { skew_ms, .. }) if skew_ms > 100_000),
        "{called:?}"
    );
    assert_eq!(behind.peer_health(&id("uk-1")).unwrap().state, PeerState::ClockSkew);
    assert!(p.uk.echo.seen.lock().unwrap().is_empty());
    // Within the bound is fine, and the clocks agreeing again clears it
    uk_clock.skew_by(20_000);
    assert!(p.oregon_link.ping(&id("uk-1")).await.is_ok());
    uk_clock.skew_by(0);
    assert_eq!(call(&p.oregon_link, "uk-1", "ok").await.unwrap(), "oregon-1:ok");
    assert_eq!(p.oregon_link.peer_health(&id("uk-1")).unwrap().state, PeerState::Up);
    p.oregon.close().await;
    p.uk.close().await;
}

#[tokio::test]
async fn a_server_outside_a_network_answers_as_before() {
    let _serial = serial().await;
    let cluster = Cluster::new();
    let public = make_cert();
    let ports = Ports {
        backend: dead_port().await,
        content: dead_port().await,
        ws: dead_port().await,
    };
    let config = test_config(&public, &ports, &[]);
    assert!(config.node_links.is_none() && config.network.is_empty());
    let running = start_gateway(&public, config, None).await;
    assert!(running.gateway.node_links().is_none());
    let reply = https_request(running.port, "/__node/ping", Options::default())
        .await
        .unwrap();
    assert_eq!(reply.status, 403);
    // A node asking for a node link finds no one to talk to
    let files = cluster.files(&cluster.ca, "uk-1", "uk", "alone");
    let link = link_to(
        cluster.credentials(&files),
        vec![peer("oregon-1", "oregon", running.port)],
    );
    let result = call(&link, "oregon-1", "hi").await;
    assert!(
        matches!(result, Err(LinkError::Unreachable { .. } | LinkError::Refused { .. })),
        "{result:?}"
    );
    assert_eq!(
        TlsPeerLink::new(TlsLinkConfig::new(cluster.credentials(&files), vec![]))
            .unwrap()
            .health(),
        vec![]
    );
    running.close().await;
}

#[test]
fn node_links_are_configured_together_or_not_at_all() {
    let env = |extra: &[(&str, &str)]| {
        let mut pairs = vec![
            ("GATEWAY_CERT", "c.pem"),
            ("GATEWAY_KEY", "k.pem"),
            ("GATEWAY_SECRET", GATEWAY_SECRET),
            ("GATEWAY_ALLOWLIST", "0"),
        ];
        pairs.extend_from_slice(extra);
        dainty_gateway::config::Config::from_env(&dainty_service_kit::env::Env::from_pairs(pairs))
    };
    let peers = r#"[{"id":"uk-1","region":"uk","host":"uk.example.org","port":443,"fingerprint":"ab"}]"#;
    let full = |peers: &str| {
        env(&[
            ("GATEWAY_NODE_ID", "oregon-1"),
            ("GATEWAY_NODE_REGION", "oregon"),
            ("GATEWAY_NODE_CERT", "n.pem"),
            ("GATEWAY_NODE_KEY", "nk.pem"),
            ("GATEWAY_NODE_CA", "ca.pem"),
            ("GATEWAY_NETWORK_NODES", peers),
        ])
    };
    assert!(env(&[]).unwrap().node_links.is_none());
    assert!(
        env(&[("GATEWAY_NETWORK_NODES", "[]")])
            .unwrap_err()
            .contains("GATEWAY_NODE_ID")
    );
    assert!(
        env(&[("GATEWAY_NODE_ID", "oregon-1")])
            .unwrap_err()
            .contains("GATEWAY_NODE_REGION")
    );
    assert!(
        env(&[("GATEWAY_NODE_ID", "Oregon")])
            .unwrap_err()
            .contains("GATEWAY_NODE_ID")
    );
    assert!(
        env(&[("GATEWAY_NODE_ID", "oregon-1"), ("GATEWAY_NODE_REGION", "oregon")])
            .unwrap_err()
            .contains("GATEWAY_NODE_CERT")
    );
    assert!(full("not json").unwrap_err().contains("GATEWAY_NETWORK_NODES"));
    assert!(
        full(r#"[{"id":"oregon-1","region":"oregon","host":"a.example","port":443}]"#)
            .unwrap_err()
            .contains("itself")
    );
    assert!(full(r#"[{"id":"uk-1","region":"uk","host":"a.example","port":443,"extra":1}]"#).is_err());
    // A fingerprint of the wrong shape, a host that isn't one
    assert!(full(peers).unwrap_err().contains("fingerprint"));
    let good = peers.replace("\"ab\"", &format!("\"{}\"", "ab".repeat(32)));
    let config = full(&good).unwrap();
    assert_eq!(config.node_links.as_ref().unwrap().peers.len(), 1);
    assert_eq!(
        (
            config.network[0].region.as_str(),
            config.network[0].host.as_str(),
            config.network[0].port
        ),
        ("uk", "uk.example.org", 443)
    );
    // A private (plain HTTP) server can't link
    let private = env(&[
        ("GATEWAY_TLS", "0"),
        ("GATEWAY_BIND", "127.0.0.1"),
        ("GATEWAY_NODE_ID", "oregon-1"),
    ]);
    assert!(private.unwrap_err().contains("TLS"));
}
