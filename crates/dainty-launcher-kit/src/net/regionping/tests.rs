// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Region ping against fake gateways on loopback, each with its own simulated latency.

use std::convert::Infallible;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use http_body_util::Full;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper::{Response, StatusCode};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

use super::*;
use crate::net::hostapi::USER_INFO_PATH;
use crate::net::pinned::certificate_fingerprint;
use crate::net::pinned::tests::{acceptor, certificate};

// Built from pieces, so the repository's secret check never sees a whole key
const KEY: &str = concat!("DLK_", "dddddddddddddddd", "dddddddddddddddd", "dddddddddddddddd");

#[derive(Debug, Clone)]
struct Seen {
    path: String,
    key: Option<String>,
    body: String,
}

#[derive(Clone, Default)]
struct Behaviour {
    /// Before each ping answer
    delay: Duration,
    /// Pings are never answered
    hang: bool,
    /// The node list this server gives, as (status, body)
    nodes: Option<(u16, Value)>,
    /// Pings in flight across the servers that share it: (now, most)
    in_flight: Option<Arc<(AtomicUsize, AtomicUsize)>>,
    /// The report answers 401 until the key has been to the user info route, as a restarted home does
    forgets_keys: bool,
}

struct Fake {
    port: u16,
    fingerprint: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Fake {
    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    fn node(&self, region: &str) -> Value {
        json!({ "region": region, "host": "127.0.0.1", "port": self.port, "fingerprint": self.fingerprint })
    }
}

async fn answer(
    behaviour: Behaviour,
    seen: Arc<Mutex<Vec<Seen>>>,
    request: Request<Incoming>,
) -> Response<Full<Bytes>> {
    let path = request.uri().path().to_owned();
    let key = request
        .headers()
        .get("x-dainty-user-api-key")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let body = request
        .into_body()
        .collect()
        .await
        .map(|body| body.to_bytes())
        .unwrap_or_default();
    seen.lock().unwrap().push(Seen {
        path: path.clone(),
        key,
        body: String::from_utf8_lossy(&body).into_owned(),
    });
    let reply = |status: u16, body: String| {
        Response::builder()
            .status(status)
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from(body)))
            .unwrap()
    };
    match path.as_str() {
        PING_PATH => {
            if let Some(counter) = &behaviour.in_flight {
                let now = counter.0.fetch_add(1, Ordering::SeqCst) + 1;
                counter.1.fetch_max(now, Ordering::SeqCst);
            }
            if behaviour.hang {
                tokio::time::sleep(Duration::from_secs(3600)).await;
            }
            tokio::time::sleep(behaviour.delay).await;
            if let Some(counter) = &behaviour.in_flight {
                counter.0.fetch_sub(1, Ordering::SeqCst);
            }
            Response::builder()
                .status(StatusCode::NO_CONTENT)
                .body(Full::new(Bytes::new()))
                .unwrap()
        }
        NODES_PATH => match &behaviour.nodes {
            Some((status, body)) => reply(*status, body.to_string()),
            None => reply(404, "{}".into()),
        },
        REPORT_PATH => {
            let announced = seen
                .lock()
                .unwrap()
                .iter()
                .any(|seen| seen.path == USER_INFO_PATH && seen.key.as_deref() == Some(KEY));
            if behaviour.forgets_keys && !announced {
                reply(401, String::new())
            } else {
                reply(204, String::new())
            }
        }
        USER_INFO_PATH => reply(
            200,
            json!({ "UserId": "u1", "Username": "Slayer", "IsAdmin": false }).to_string(),
        ),
        _ => reply(404, "{}".into()),
    }
}

/// A gateway on a free loopback port, over TLS with a certificate of its own or plain HTTP.
async fn fake(tls: bool, behaviour: Behaviour) -> Fake {
    let (cert, key) = certificate();
    let fingerprint = certificate_fingerprint(&cert);
    let tls: Option<TlsAcceptor> = tls.then(|| acceptor(cert, key));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen: Arc<Mutex<Vec<Seen>>> = Arc::default();
    let record = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else { return };
            let (tls, behaviour, record) = (tls.clone(), behaviour.clone(), record.clone());
            tokio::spawn(async move {
                let service = service_fn(move |request| {
                    let (behaviour, record) = (behaviour.clone(), record.clone());
                    async move { Ok::<_, Infallible>(answer(behaviour, record, request).await) }
                });
                let http = hyper::server::conn::http1::Builder::new();
                match tls {
                    Some(tls) => {
                        if let Ok(stream) = tls.accept(tcp).await {
                            let _ = http.serve_connection(TokioIo::new(stream), service).await;
                        }
                    }
                    None => {
                        let _ = http.serve_connection(TokioIo::new(tcp), service).await;
                    }
                }
            });
        }
    });
    Fake {
        port,
        fingerprint,
        seen,
    }
}

async fn node(delay_ms: u64) -> Fake {
    fake(
        true,
        Behaviour {
            delay: Duration::from_millis(delay_ms),
            ..Behaviour::default()
        },
    )
    .await
}

async fn home(nodes: Option<(u16, Value)>) -> Fake {
    fake(
        true,
        Behaviour {
            nodes,
            ..Behaviour::default()
        },
    )
    .await
}

fn endpoint(fake: &Fake) -> Endpoint {
    Endpoint {
        host: "127.0.0.1".into(),
        port: fake.port,
        pin: Some(fake.fingerprint.clone()),
    }
}

fn entry(fake: &Fake, region: &str) -> NodeEntry {
    NodeEntry {
        region: region.into(),
        host: "127.0.0.1".into(),
        port: fake.port,
        fingerprint: fake.fingerprint.clone(),
    }
}

fn quick() -> PingSettings {
    PingSettings {
        samples: 3,
        connect_timeout: Duration::from_millis(400),
        sample_timeout: Duration::from_millis(400),
        node_deadline: Duration::from_millis(1500),
        concurrency: 4,
    }
}

fn regions(list: &[RegionRtt]) -> Vec<&str> {
    list.iter().map(|region| region.region.as_str()).collect()
}

#[tokio::test]
async fn regions_come_back_in_the_order_of_their_latency() {
    // Apart by more than Windows' timer step, so the order holds on any machine
    let (slow, fast, middle) = (node(150).await, node(0).await, node(60).await);
    let nodes = vec![entry(&slow, "oregon"), entry(&fast, "uk"), entry(&middle, "new-york")];
    let measured = measure(nodes, &PingSettings::default()).await;
    assert_eq!(regions(&measured), ["uk", "new-york", "oregon"]);
    for region in &measured {
        assert_eq!((region.samples, region.problem), (5, None), "{region:?}");
    }
    let rtt = |at: usize| measured[at].rtt_ms.unwrap();
    assert!(rtt(0) < rtt(1) && rtt(1) < rtt(2), "{measured:?}");
    assert!(rtt(2) >= 140.0, "{measured:?}");
    // One connection per node: every request was a ping
    assert!(
        slow.seen()
            .iter()
            .all(|seen| seen.path == PING_PATH && seen.key.is_none())
    );
    assert_eq!(slow.seen().len(), 5);
}

#[tokio::test]
async fn a_node_with_another_certificate_is_refused_before_anything_is_sent() {
    let (good, impostor, other) = (node(0).await, node(0).await, node(0).await);
    let forged = NodeEntry {
        fingerprint: other.fingerprint.clone(),
        ..entry(&impostor, "de")
    };
    let measured = measure(vec![forged, entry(&good, "uk")], &quick()).await;
    assert_eq!(regions(&measured), ["uk", "de"]);
    assert_eq!(measured[1].problem, Some(RegionProblem::CertMismatch));
    assert_eq!((measured[1].rtt_ms, measured[1].samples), (None, 0));
    assert!(impostor.seen().is_empty());
}

#[tokio::test]
async fn silent_nodes_time_out_or_fail_and_go_last() {
    let good = node(0).await;
    // Takes the connection and never finishes the handshake
    let mute = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mute_entry = NodeEntry {
        port: mute.local_addr().unwrap().port(),
        ..entry(&good, "b-mute")
    };
    let hanging = fake(
        true,
        Behaviour {
            hang: true,
            ..Behaviour::default()
        },
    )
    .await;
    let dead_port = {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        listener.local_addr().unwrap().port()
    };
    let dead = NodeEntry {
        port: dead_port,
        ..entry(&good, "a-dead")
    };
    let started = Instant::now();
    let measured = measure(
        vec![dead, entry(&hanging, "c-hang"), mute_entry, entry(&good, "z-good")],
        &quick(),
    )
    .await;
    assert!(started.elapsed() < Duration::from_secs(3), "{:?}", started.elapsed());
    assert_eq!(regions(&measured), ["z-good", "a-dead", "b-mute", "c-hang"]);
    let problems: Vec<_> = measured.iter().map(|region| region.problem).collect();
    assert_eq!(
        problems[2..],
        [Some(RegionProblem::Timeout), Some(RegionProblem::Timeout)]
    );
    assert_eq!(problems[0], None);
    // Windows retries a refused connect for seconds before it says so
    assert!(
        matches!(problems[1], Some(RegionProblem::Unreachable | RegionProblem::Timeout)),
        "{problems:?}"
    );
    drop(mute);
}

#[tokio::test]
async fn at_most_the_set_number_of_nodes_are_timed_at_once() {
    let counter: Arc<(AtomicUsize, AtomicUsize)> = Arc::default();
    let mut fakes = Vec::new();
    for _ in 0..6 {
        fakes.push(
            fake(
                true,
                Behaviour {
                    delay: Duration::from_millis(30),
                    in_flight: Some(counter.clone()),
                    ..Behaviour::default()
                },
            )
            .await,
        );
    }
    let nodes = fakes
        .iter()
        .enumerate()
        .map(|(n, fake)| entry(fake, &format!("r{n}")))
        .collect();
    let settings = PingSettings {
        concurrency: 2,
        ..quick()
    };
    let measured = measure(nodes, &settings).await;
    assert!(measured.iter().all(|region| region.problem.is_none()), "{measured:?}");
    assert!(counter.1.load(Ordering::SeqCst) <= 2);
}

#[tokio::test]
async fn the_order_goes_to_the_home_with_the_key_and_to_no_node() {
    let (far, near) = (node(120).await, node(0).await);
    let home = home(Some((200, json!({ "nodes": [far.node("oregon"), near.node("uk")] })))).await;
    let answer = region_ping(&endpoint(&home), Some(KEY), &quick()).await;
    let RegionPing::Ok {
        regions: measured,
        reported,
    } = answer
    else {
        panic!("{answer:?}");
    };
    assert!(reported);
    assert_eq!(regions(&measured), ["uk", "oregon"]);
    let seen = home.seen();
    assert_eq!(
        seen.iter().map(|seen| seen.path.as_str()).collect::<Vec<_>>(),
        [NODES_PATH, REPORT_PATH]
    );
    assert_eq!(seen[0].key, None);
    assert_eq!(seen[1].key.as_deref(), Some(KEY));
    let body: Value = serde_json::from_str(&seen[1].body).unwrap();
    assert_eq!(body["regions"][0]["region"], "uk");
    assert_eq!(body["regions"][1]["region"], "oregon");
    assert_eq!(body["regions"][1]["rttMs"], json!(measured[1].rtt_ms));
    for node in [&far, &near] {
        assert!(!node.seen().is_empty());
        assert!(
            node.seen()
                .iter()
                .all(|seen| seen.key.is_none() && seen.path == PING_PATH)
        );
    }
}

#[tokio::test]
async fn a_restarted_home_that_forgot_the_key_gets_it_again_and_takes_the_order() {
    let near = node(0).await;
    let home = fake(
        true,
        Behaviour {
            nodes: Some((200, json!({ "nodes": [near.node("uk")] }))),
            forgets_keys: true,
            ..Behaviour::default()
        },
    )
    .await;
    let answer = region_ping(&endpoint(&home), Some(KEY), &quick()).await;
    assert!(matches!(answer, RegionPing::Ok { reported: true, .. }), "{answer:?}");
    let seen = home.seen();
    assert_eq!(
        seen.iter().map(|seen| seen.path.as_str()).collect::<Vec<_>>(),
        [NODES_PATH, REPORT_PATH, USER_INFO_PATH, REPORT_PATH]
    );
    assert!(seen[1..].iter().all(|seen| seen.key.as_deref() == Some(KEY)));
}

#[tokio::test]
async fn without_a_key_the_order_is_measured_but_not_sent() {
    let near = node(0).await;
    let home = home(Some((200, json!({ "nodes": [near.node("uk")] })))).await;
    let answer = region_ping(&endpoint(&home), None, &quick()).await;
    assert!(
        matches!(&answer, RegionPing::Ok { regions, reported: false } if regions.len() == 1),
        "{answer:?}"
    );
    assert_eq!(home.seen().len(), 1);
}

#[tokio::test]
async fn outside_a_network_nothing_is_measured_or_sent() {
    for nodes in [Some((200, json!({ "nodes": [] }))), None] {
        let home = home(nodes).await;
        let answer = region_ping(&endpoint(&home), Some(KEY), &quick()).await;
        assert_eq!(
            answer,
            RegionPing::Ok {
                regions: Vec::new(),
                reported: false
            }
        );
        assert_eq!(home.seen().len(), 1);
    }
    // A private-mode server over plain HTTP on this PC: its nodes would still be pinned
    let lan = fake(
        false,
        Behaviour {
            nodes: Some((200, json!({ "nodes": [] }))),
            ..Behaviour::default()
        },
    )
    .await;
    let private = Endpoint {
        pin: None,
        ..endpoint(&lan)
    };
    assert!(matches!(
        region_ping(&private, Some(KEY), &quick()).await,
        RegionPing::Ok { reported: false, .. }
    ));
}

#[tokio::test]
async fn a_home_that_cannot_list_its_nodes_is_an_error() {
    let error = |code| RegionPing::Err(LauncherError::new(code));
    let broken = home(Some((500, json!({ "error": "x" })))).await;
    assert_eq!(
        region_ping(&endpoint(&broken), Some(KEY), &quick()).await,
        error(ErrorCode::ServerError)
    );
    let other = home(None).await;
    let forged = Endpoint {
        pin: Some(other.fingerprint.clone()),
        ..endpoint(&broken)
    };
    assert_eq!(
        region_ping(&forged, Some(KEY), &quick()).await,
        error(ErrorCode::CertMismatch)
    );
    assert_eq!(broken.seen().len(), 1, "nothing sent past the mismatch");
    let gone = Endpoint {
        port: {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            listener.local_addr().unwrap().port()
        },
        ..endpoint(&broken)
    };
    assert_eq!(
        region_ping(&gone, Some(KEY), &quick()).await,
        error(ErrorCode::ServerRefused)
    );
}

#[test]
fn medians_and_ties() {
    let ms = Duration::from_millis;
    assert_eq!(median(&mut []), None);
    assert_eq!(median(&mut [ms(30), ms(10), ms(20)]), Some(ms(20)));
    assert_eq!(median(&mut [ms(40), ms(10), ms(20), ms(900)]), Some(ms(30)));
    assert_eq!(tenths_of_ms(Duration::from_micros(18_449)), 18.4);
    let rtt = |region: &str, rtt_ms: Option<f64>| RegionRtt {
        region: region.into(),
        rtt_ms,
        samples: u32::from(rtt_ms.is_some()),
        problem: rtt_ms.is_none().then_some(RegionProblem::Timeout),
    };
    let mut list = vec![
        rtt("zz", None),
        rtt("de", Some(20.0)),
        rtt("aa", None),
        rtt("uk", Some(5.0)),
        rtt("at", Some(20.0)),
    ];
    order(&mut list);
    assert_eq!(regions(&list), ["uk", "at", "de", "aa", "zz"]);
    assert_eq!(
        report_body(&list[..2]),
        r#"{"regions":[{"region":"uk","rttMs":5.0},{"region":"at","rttMs":20.0}]}"#
    );
}
