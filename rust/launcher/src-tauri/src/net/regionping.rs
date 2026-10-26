//! Region ping (DN7, DD19): the joined server lists its network's nodes, the launcher times a few
//! requests to each node's gateway and sends the regions, nearest first, back to the joined server,
//! the player's home. Each node is reached over TLS pinned to the fingerprint its home listed, as the
//! home is pinned to the invite's, and is sent no key: only the home gets the account key, with the
//! report (AGENTS.md, DD17).
//!
//! The timing runs on one connection per node: the handshake is paid once, then each request is one
//! round trip to the gateway, which answers the ping itself.

/// The node list's reader, without the network: the fuzz target `launcher_region` compiles it in
pub mod parse;

use std::time::{Duration, Instant};

use bytes::Bytes;
use futures_util::StreamExt;
use http_body_util::{BodyExt, Empty, Limited};
use hyper::client::conn::http1::SendRequest;
use hyper::{Method, Request};
use hyper_util::rt::TokioIo;
use serde_json::json;

use self::parse::{NodeEntry, NodeList, read_nodes};
use super::hostapi::key_headers;
use super::http::{Endpoint, HttpError, RequestOptions, host_header, parse_json_body, request};
use super::pinned::{ConnectError, PinnedTarget, pinned_connect};
use crate::bridge::types::{ErrorCode, LauncherError, RegionPing, RegionProblem, RegionRtt};

pub const NODES_PATH: &str = "/network/v1/nodes";
pub const PING_PATH: &str = "/network/v1/ping";
pub const REPORT_PATH: &str = "/network/v1/regions";

const HOME_TIMEOUT: Duration = Duration::from_secs(5);
const NODES_MAX_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone)]
pub struct PingSettings {
    /// Requests timed per node; the median is kept
    pub samples: usize,
    pub connect_timeout: Duration,
    pub sample_timeout: Duration,
    /// For one node from connect to the last answer: a slow node keeps what it answered by then
    pub node_deadline: Duration,
    /// Nodes timed at once
    pub concurrency: usize,
}

/// At most 16 nodes, four at a time, 4 s each: under 30 s with the two calls to the home, inside the
/// page's minute.
impl Default for PingSettings {
    fn default() -> Self {
        Self {
            samples: 5,
            connect_timeout: Duration::from_secs(2),
            sample_timeout: Duration::from_millis(1500),
            node_deadline: Duration::from_secs(4),
            concurrency: 4,
        }
    }
}

/// The joined server's network, empty outside one. No key: the list is public.
pub async fn fetch_nodes(home: &Endpoint) -> Result<Vec<NodeEntry>, ErrorCode> {
    let response = request(home, NODES_PATH, RequestOptions::get(HOME_TIMEOUT, NODES_MAX_BYTES))
        .await
        .map_err(|error| match error {
            HttpError::Pin { .. } => ErrorCode::CertMismatch,
            _ => ErrorCode::ServerUnreachable,
        })?;
    match read_nodes(response.status, parse_json_body(&response).as_ref()) {
        NodeList::Nodes(nodes) => Ok(nodes),
        NodeList::BadAnswer => Err(ErrorCode::ServerError),
    }
}

async fn ping_once(sender: &mut SendRequest<Empty<Bytes>>, host: &str) -> Result<bool, ()> {
    sender.ready().await.map_err(|_| ())?;
    let ping = Request::builder()
        .method(Method::GET)
        .uri(PING_PATH)
        .header("host", host)
        .body(Empty::new())
        .map_err(|_| ())?;
    let response = sender.send_request(ping).await.map_err(|_| ())?;
    let answered = response.status().is_success();
    Limited::new(response.into_body(), 1024)
        .collect()
        .await
        .map_err(|_| ())?;
    Ok(answered)
}

/// The round trips one node answered within its deadline, or why there were none.
async fn time_node(node: &NodeEntry, settings: &PingSettings) -> Result<Vec<Duration>, RegionProblem> {
    let deadline = Instant::now() + settings.node_deadline;
    let target = PinnedTarget {
        host: node.host.clone(),
        port: node.port,
        fingerprint: node.fingerprint.clone(),
    };
    let stream = pinned_connect(&target, settings.connect_timeout.min(settings.node_deadline))
        .await
        .map_err(|error| match error {
            ConnectError::Pin { .. } => RegionProblem::CertMismatch,
            ConnectError::Timeout => RegionProblem::Timeout,
            ConnectError::Io(_) => RegionProblem::Unreachable,
        })?;
    let (mut sender, connection) = hyper::client::conn::http1::handshake::<_, Empty<Bytes>>(TokioIo::new(stream))
        .await
        .map_err(|_| RegionProblem::Unreachable)?;
    let driver = tokio::spawn(connection);
    let host = host_header(&Endpoint {
        host: node.host.clone(),
        port: node.port,
        pin: Some(node.fingerprint.clone()),
    });
    let mut times = Vec::with_capacity(settings.samples);
    let mut problem = RegionProblem::Timeout;
    for _ in 0..settings.samples {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        let started = Instant::now();
        match tokio::time::timeout(settings.sample_timeout.min(left), ping_once(&mut sender, &host)).await {
            Ok(Ok(true)) => times.push(started.elapsed()),
            Ok(Ok(false)) => {
                problem = RegionProblem::BadAnswer;
                break;
            }
            Ok(Err(())) => {
                problem = RegionProblem::Unreachable;
                break;
            }
            // A request still in flight leaves the connection unusable
            Err(_) => break,
        }
    }
    driver.abort();
    if times.is_empty() { Err(problem) } else { Ok(times) }
}

pub fn median(times: &mut [Duration]) -> Option<Duration> {
    times.sort_unstable();
    let middle = times.len() / 2;
    match times.len() {
        0 => None,
        n if n % 2 == 1 => Some(times[middle]),
        _ => Some((times[middle - 1] + times[middle]) / 2),
    }
}

fn tenths_of_ms(time: Duration) -> f64 {
    (time.as_micros() as f64 / 100.0).round() / 10.0
}

async fn measure_node(node: NodeEntry, settings: &PingSettings) -> RegionRtt {
    match time_node(&node, settings).await {
        Ok(mut times) => RegionRtt {
            region: node.region,
            rtt_ms: median(&mut times).map(tenths_of_ms),
            samples: times.len() as u32,
            problem: None,
        },
        Err(problem) => RegionRtt {
            region: node.region,
            rtt_ms: None,
            samples: 0,
            problem: Some(problem),
        },
    }
}

/// Nearest first; the nodes that never answered last. Ties and the silent ones by region id, so the
/// order is the same however the measurements finished.
pub fn order(regions: &mut [RegionRtt]) {
    regions.sort_by(|a, b| match (a.rtt_ms, b.rtt_ms) {
        (Some(x), Some(y)) => x.total_cmp(&y).then_with(|| a.region.cmp(&b.region)),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => a.region.cmp(&b.region),
    });
}

/// Every node timed, a few at a time, then ordered.
pub async fn measure(nodes: Vec<NodeEntry>, settings: &PingSettings) -> Vec<RegionRtt> {
    let mut regions: Vec<RegionRtt> = futures_util::stream::iter(nodes)
        .map(|node| measure_node(node, settings))
        .buffer_unordered(settings.concurrency.max(1))
        .collect()
        .await;
    order(&mut regions);
    regions
}

/// The body of `POST /network/v1/regions`: the regions in order, each with its round trip or null.
pub fn report_body(regions: &[RegionRtt]) -> String {
    let regions: Vec<_> = regions
        .iter()
        .map(|region| json!({ "region": region.region, "rttMs": region.rtt_ms }))
        .collect();
    json!({ "regions": regions }).to_string()
}

/// Sends the order to the home, with the account key and only where the key may go (`key_headers`).
/// True when the home took it.
pub async fn report(home: &Endpoint, key: &str, regions: &[RegionRtt]) -> bool {
    let headers = key_headers(home, Some(key));
    if headers.is_empty() {
        return false;
    }
    let options = RequestOptions {
        method: Method::POST,
        headers,
        body: Some(report_body(regions)),
        ..RequestOptions::get(HOME_TIMEOUT, 4096)
    };
    request(home, REPORT_PATH, options)
        .await
        .is_ok_and(|response| (200..300).contains(&response.status))
}

/// `region_ping` for the joined server: its nodes timed and ordered, then the order reported when the
/// player has a key for it. Outside a network nothing is measured or sent.
pub async fn region_ping(home: &Endpoint, key: Option<&str>, settings: &PingSettings) -> RegionPing {
    let nodes = match fetch_nodes(home).await {
        Ok(nodes) => nodes,
        Err(code) => return RegionPing::Err(LauncherError::new(code)),
    };
    if nodes.is_empty() {
        return RegionPing::Ok {
            regions: Vec::new(),
            reported: false,
        };
    }
    let regions = measure(nodes, settings).await;
    let reported = match key {
        Some(key) => report(home, key, &regions).await,
        None => false,
    };
    RegionPing::Ok { regions, reported }
}

#[cfg(test)]
mod tests;
