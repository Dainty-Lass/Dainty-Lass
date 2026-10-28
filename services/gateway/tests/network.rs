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

#[tokio::test]
async fn region_report_handler_reaches_ring_node_record_region_pings_authenticated() {
    use std::sync::Arc;

    use module_api::GameVersion;
    use module_api::network::{GlobalId, HomeApi, HomeError, IdKind, NodeId, NodeIdentity, SessionTicket};
    use network_plugin::Clock;
    use network_plugin::home::{SigningKey, TicketConfig, Tickets};
    use network_plugin::inprocess::InProcessNetwork;
    use network_plugin::ring::{RegionPing, RingConfig, RingNode, UnconfiguredDirector};

    struct NoopAllowlist;
    impl network_plugin::ring::AllowlistSink for NoopAllowlist {
        fn admit_player(&self, _: &GlobalId, _: &GlobalId, _: &str, _: u16) {}
    }

    struct TicketVerifyingHome {
        identity: NodeIdentity,
        tickets: Arc<Tickets>,
    }

    impl HomeApi for TicketVerifyingHome {
        fn resolve_home<'a>(&'a self, _: &'a GlobalId) -> module_api::BoxFuture<'a, Result<NodeIdentity, HomeError>> {
            let id = self.identity.clone();
            Box::pin(async move { Ok(id) })
        }

        fn session_ticket<'a>(
            &'a self,
            _: &'a GlobalId,
            _: &'a NodeId,
        ) -> module_api::BoxFuture<'a, Result<SessionTicket, HomeError>> {
            Box::pin(async { Err(HomeError::NotInNetwork) })
        }

        fn admit<'a>(
            &'a self,
            ticket: &'a SessionTicket,
            game: &'a GameVersion,
        ) -> module_api::BoxFuture<'a, Result<GlobalId, HomeError>> {
            let expect = network_plugin::home::Expect {
                issuer: &self.identity.node,
                audience: &self.identity.node,
                game: Some(game),
                account: None,
            };
            let res = self
                .tickets
                .verify(ticket, &expect)
                .map(|c| c.sub)
                .map_err(|e| HomeError::Denied(e.to_string()));
            Box::pin(async move { res })
        }
    }

    let _serial = serial().await;
    let w = world(Vec::new()).await;
    let port = w.gateway.port;

    let node: NodeId = "oregon-1".parse().unwrap();
    let identity = NodeIdentity {
        node: node.clone(),
        region: "oregon".parse().unwrap(),
    };
    let clock = Arc::new(Clock::default());
    let tickets = Arc::new(Tickets::new(
        &identity,
        SigningKey::generate().unwrap(),
        clock.clone(),
        TicketConfig::default(),
    ));
    let net = InProcessNetwork::new();
    let ring = RingNode::new(
        RingConfig::new(identity.clone()),
        clock,
        Arc::new(net.link(&node)),
        Arc::new(TicketVerifyingHome {
            identity: identity.clone(),
            tickets: tickets.clone(),
        }),
        Arc::new(UnconfiguredDirector),
        Arc::new(NoopAllowlist),
    );
    let account = GlobalId::new(IdKind::Account, &node);
    ring.register_local_account_key(FAKE_KEY, account.clone());

    let game: GameVersion = "1.14.7".parse().unwrap();
    let ticket_account = GlobalId::new(IdKind::Account, &node);
    let session_ticket = tickets.issue(&ticket_account, &node, &game, None).unwrap();

    let handler_ring = ring.clone();
    w.gateway
        .gateway
        .set_region_handler(Arc::new(move |api_key, ticket_token, regions| {
            let ring = handler_ring.clone();
            Box::pin(async move {
                let mut pings = Vec::with_capacity(regions.len());
                for r in regions {
                    let Ok(region) = r.region.parse() else {
                        return false;
                    };
                    let rtt_ms = r.rtt_ms.map(|ms| ms.round() as u32);
                    pings.push(RegionPing { region, rtt_ms });
                }
                let parsed = match ticket_token.as_deref() {
                    Some(tok) => match network_plugin::home::ticket::session_ticket_from_token(tok) {
                        Ok(pair) => Some(pair),
                        Err(_) => return false,
                    },
                    None => None,
                };
                let ticket_ref = parsed.as_ref().map(|(t, g)| (t, g));
                ring.record_region_pings_authenticated(api_key.as_deref(), ticket_ref, pings)
                    .await
                    .is_ok()
            })
        }));

    // Registered local API key succeeds and stores sorted region pings in RingNode
    let report = r#"{"regions":[{"region":"oregon","rttMs":95.0},{"region":"uk","rttMs":18.0},{"region":"germany","rttMs":null}]}"#;
    let ok = https_request(port, "/network/v1/regions", post(report, Some(FAKE_KEY)))
        .await
        .unwrap();
    assert_eq!(ok.status, 204);
    assert_eq!(
        ring.region_pings_for(&account),
        Some(vec![
            RegionPing {
                region: "uk".parse().unwrap(),
                rtt_ms: Some(18),
            },
            RegionPing {
                region: "oregon".parse().unwrap(),
                rtt_ms: Some(95),
            },
            RegionPing {
                region: "germany".parse().unwrap(),
                rtt_ms: None,
            },
        ])
    );

    // Unknown API key is refused with 401 when the handler is attached
    let unknown_key = "RRK_000000000000000000000000000000000000000000000000";
    let denied = https_request(port, "/network/v1/regions", post(report, Some(unknown_key)))
        .await
        .unwrap();
    assert_eq!(denied.status, 401);

    // Session ticket header is parsed and verified via RingNode::record_region_pings_authenticated
    let ticket_req = Options {
        method: Some("POST"),
        headers: vec![("x-reforged-session-ticket", session_ticket.token.clone())],
        body: Some(report.as_bytes().to_vec()),
        ..Options::default()
    };
    let ticket_ok = https_request(port, "/network/v1/regions", ticket_req).await.unwrap();
    assert_eq!(ticket_ok.status, 204);
    assert!(ring.region_pings_for(&ticket_account).is_some());

    w.gateway.close().await;
}
