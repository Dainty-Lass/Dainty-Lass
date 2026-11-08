// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The gateway's settings, read once at start. Every upstream is a loopback URL because every upstream request
//! carries the gateway secret, which must never leave the machine.

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use dainty_module_api::network::{NodeId, NodeIdentity, Region};
use dainty_protocol::GameVersion;
use dainty_service_kit::env::Env;
use dainty_service_kit::js;
use network_plugin::cluster::{self, Peer};

use crate::ratelimit::{BucketSpec, parse_bucket_spec};
use crate::{json_string, whole_number};

/// A loopback upstream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// Host name or IP address.
    pub host: String,
    /// TCP port.
    pub port: u16,
}

impl Endpoint {
    /// An endpoint at `host:port`.
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
        }
    }

    /// `host:port`, the host in brackets when it is IPv6.
    pub fn authority(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

/// The allowlist feed: the helper and up to four more.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Allowlist {
    /// `http://<authority>`
    pub url: String,
    /// The helper's address.
    pub endpoint: Endpoint,
    /// Shared secret sent to the helpers (`ALLOWLIST_SECRET`).
    pub secret: String,
    /// How often an admitted player is reported again, 5 to 540 s (`GATEWAY_ALLOWLIST_REFRESH_SECONDS`).
    pub refresh: Duration,
    /// Up to four more helpers fed the same (`ALLOWLIST_ADDITIONAL_URLS`).
    pub additional: Vec<Endpoint>,
    /// How long one report may take.
    pub timeout: Duration,
}

/// Token buckets per rate-limit class (`GATEWAY_RATE_*`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rates {
    /// Most requests.
    pub general: BucketSpec,
    /// `/content` downloads.
    pub content: BucketSpec,
    /// Account registration.
    pub register: BucketSpec,
    /// Login token requests.
    pub token: BucketSpec,
    /// WebSocket upgrades.
    pub ws: BucketSpec,
    /// New connections.
    pub connect: BucketSpec,
}

/// Size and count limits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    /// Largest request body, in bytes (`GATEWAY_MAX_BODY_BYTES`).
    pub max_body_bytes: u64,
    /// Open connections in all (`GATEWAY_MAX_CONNECTIONS`).
    pub max_connections: usize,
    /// Open connections per client address (`GATEWAY_MAX_CONNECTIONS_PER_IP`).
    pub max_connections_per_ip: usize,
    /// Most request headers.
    pub max_headers: usize,
    /// Rate limits.
    pub rates: Rates,
}

/// Connection and request time limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timeouts {
    /// TLS handshake (`GATEWAY_HANDSHAKE_TIMEOUT_MS`).
    pub handshake: Duration,
    /// Request head (`GATEWAY_HEADERS_TIMEOUT_MS`).
    pub headers: Duration,
    /// Whole request (`GATEWAY_REQUEST_TIMEOUT_MS`).
    pub request: Duration,
    /// Idle keep-alive connection (`GATEWAY_KEEPALIVE_TIMEOUT_MS`).
    pub keep_alive: Duration,
    /// Connection with no traffic (`GATEWAY_IDLE_TIMEOUT_MS`).
    pub idle: Duration,
    /// Idle pooled connection to an upstream (`GATEWAY_UPSTREAM_IDLE_TIMEOUT_MS`).
    pub upstream_idle: Duration,
    /// WebSocket with no traffic (`GATEWAY_WS_IDLE_TIMEOUT_MS`).
    pub ws_idle: Duration,
    /// How long open connections get to finish at shutdown (`GATEWAY_SHUTDOWN_GRACE_MS`).
    pub shutdown_grace: Duration,
    /// Fixed; nothing depends on it
    pub check_interval: Duration,
}

const fn bucket(burst: f64, per_minute: f64) -> BucketSpec {
    BucketSpec { burst, per_minute }
}

/// Limits when nothing overrides them.
pub const DEFAULT_LIMITS: Limits = Limits {
    max_body_bytes: 131_072,
    max_connections: 2048,
    max_connections_per_ip: 128,
    max_headers: 100,
    rates: Rates {
        general: bucket(300.0, 180.0),
        content: bucket(600.0, 600.0),
        register: bucket(5.0, 0.2),
        token: bucket(10.0, 1.0),
        // The game reconnects chat at most every 15-45 s after a failure
        ws: bucket(20.0, 12.0),
        connect: bucket(200.0, 300.0),
    },
};

/// Timeouts when nothing overrides them.
pub const DEFAULT_TIMEOUTS: Timeouts = Timeouts {
    handshake: Duration::from_millis(10_000),
    headers: Duration::from_millis(10_000),
    request: Duration::from_millis(30_000),
    keep_alive: Duration::from_millis(65_000),
    idle: Duration::from_millis(120_000),
    upstream_idle: Duration::from_millis(120_000),
    ws_idle: Duration::from_millis(300_000),
    shutdown_grace: Duration::from_millis(10_000),
    check_interval: Duration::from_millis(1000),
};

/// The node links on the public listener (DN3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeLinkConfig {
    /// This node's id and region (`GATEWAY_NODE_ID`, `GATEWAY_NODE_REGION`).
    pub identity: NodeIdentity,
    /// Node certificate.
    pub cert_file: PathBuf,
    /// Node key.
    pub key_file: PathBuf,
    /// Network CA certificate.
    pub ca_file: PathBuf,
    /// The peers allowed to call.
    pub peers: Vec<Peer>,
}

use crate::network::NetworkNode;

const MAX_NETWORK_NODES: usize = 16;

/// The gateway's settings, from `gateway.env`.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// Game version served (`GATEWAY_GAME`).
    pub version: GameVersion,
    /// Address to listen on (`GATEWAY_BIND`).
    pub bind_host: String,
    /// Port to listen on; 443 in public mode, 61000 in private (`GATEWAY_PORT`).
    pub port: u16,
    /// Public mode (TLS); false is private mode, plain HTTP inside a tailnet
    pub tls: bool,
    /// TLS certificate, PEM (`GATEWAY_CERT`).
    pub cert_file: PathBuf,
    /// TLS key, PEM (`GATEWAY_KEY`).
    pub key_file: PathBuf,
    /// Shared secret for the backend (`GATEWAY_SECRET`).
    pub secret: String,
    /// The game backend (`GATEWAY_BACKEND_URL`).
    pub backend: Endpoint,
    /// The content server (`GATEWAY_CONTENT_URL`).
    pub content: Endpoint,
    /// The chat listener (`GATEWAY_WS_URL`).
    pub ws: Endpoint,
    /// The allowlist feed; `None` with `GATEWAY_ALLOWLIST=0`.
    pub allowlist: Option<Allowlist>,
    /// Size and rate limits.
    pub limits: Limits,
    /// Time limits.
    pub timeouts: Timeouts,
    /// The network's nodes for the launcher (`GATEWAY_NETWORK_NODES`).
    pub network: Vec<NetworkNode>,
    /// Node links, when this node is in a network.
    pub node_links: Option<NodeLinkConfig>,
}

/// `localhost`, an IPv4 address starting `127.`, `::1` or `[::1]`.
pub fn is_loopback_host(host: &str) -> bool {
    host == "localhost" || host.starts_with("127.") || host == "::1" || host == "[::1]"
}

/// 100.64.0.0/10, written as plain IPv4 (an IPv4-mapped IPv6 address is not one).
pub fn is_tailscale_ipv4(text: &str) -> bool {
    !text.contains(':')
        && crate::ip::parse_ip(text)
            .is_some_and(|p| p.version == 4 && p.bytes[0] == 100 && (64..=127).contains(&p.bytes[1]))
}

fn is_loopback_literal(host: &str) -> bool {
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => v4.is_loopback(),
        Ok(IpAddr::V6(v6)) => v6.is_loopback(),
        Err(_) => false,
    }
}

/// `http://host:port` on this machine, and nothing more.
pub fn parse_loopback_url(text: &str, name: &str) -> Result<Endpoint, String> {
    let url = url::Url::parse(text).map_err(|_| format!("{name} is not a URL"))?;
    if url.scheme() != "http" {
        return Err(format!("{name} must be an http:// URL on this machine"));
    }
    let written = url.host_str().unwrap_or_default();
    let host = written.trim_start_matches('[').trim_end_matches(']');
    if !is_loopback_literal(host) {
        return Err(format!("{name} must point at 127.0.0.1 or ::1 (got {written})"));
    }
    if url.path() != "/" || url.query().is_some() || !url.username().is_empty() || url.password().is_some() {
        return Err(format!("{name} must be just http://host:port"));
    }
    Ok(Endpoint::new(host, url.port().unwrap_or(80)))
}

/// 32 to 256 printable ASCII characters without spaces.
pub fn check_secret(value: Option<&str>, name: &str) -> Result<String, String> {
    let Some(value) = value.filter(|v| !v.is_empty()) else {
        return Err(format!("{name} is not set"));
    };
    if !(32..=256).contains(&value.len()) || !value.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
        return Err(format!(
            "{name} must be 32 to 256 printable characters without spaces (for example 64 hex characters)"
        ));
    }
    Ok(value.to_owned())
}

/// `GATEWAY_GAME`: only reborn (1.14.7) is served.
pub fn parse_game(value: Option<&str>) -> Result<GameVersion, String> {
    let Some(raw) = value else {
        return Ok(dainty_protocol::invite::dainty_version());
    };
    match js::trim(raw).to_ascii_lowercase().as_str() {
        "" | "reborn" => Ok(dainty_protocol::invite::dainty_version()),
        "legacy" => Err(
            "GATEWAY_GAME=legacy (Dauntless Revived's 1.4.4) is no longer served; this gateway serves reborn only"
                .to_owned(),
        ),
        _ => Err(format!("GATEWAY_GAME must be reborn (got {})", json_string(raw))),
    }
}

/// An optional value: trimmed, empty is unset.
pub(crate) fn optional<'a>(env: &'a Env, name: &str) -> Option<&'a str> {
    env.raw(name).map(js::trim).filter(|v| !v.is_empty())
}

pub(crate) fn whole(env: &Env, name: &str, default: u64, min: u64, max: u64) -> Result<u64, String> {
    match optional(env, name) {
        None => Ok(default),
        Some(text) => whole_number(text, min as f64, max as f64)
            .map(|v| v as u64)
            .ok_or_else(|| format!("{name} must be a whole number from {min} to {max}")),
    }
}

pub(crate) fn absolute(path: &str) -> PathBuf {
    std::path::absolute(Path::new(path)).unwrap_or_else(|_| PathBuf::from(path))
}

fn millis(env: &Env, name: &str, default: Duration, min: u64, max: u64) -> Result<Duration, String> {
    whole(env, name, default.as_millis() as u64, min, max).map(Duration::from_millis)
}

impl Config {
    /// Reads and checks every setting; the error names the first bad one.
    pub fn from_env(env: &Env) -> Result<Self, String> {
        let version = parse_game(env.raw("GATEWAY_GAME"))?;
        let bind_host = optional(env, "GATEWAY_BIND").unwrap_or("0.0.0.0").to_owned();
        if bind_host.parse::<IpAddr>().is_err() {
            return Err(format!(
                "GATEWAY_BIND must be an IP address (got {})",
                json_string(&bind_host)
            ));
        }
        let tls = optional(env, "GATEWAY_TLS") != Some("0");
        if !tls && !is_tailscale_ipv4(&bind_host) && !is_loopback_host(&bind_host) {
            return Err(format!(
                "GATEWAY_TLS=0 (private mode) listens only on this PC's Tailscale address (100.64.0.0/10) or on loopback, not on {bind_host}"
            ));
        }
        let (cert_file, key_file) = if tls {
            match (optional(env, "GATEWAY_CERT"), optional(env, "GATEWAY_KEY")) {
                (Some(cert), Some(key)) => (absolute(cert), absolute(key)),
                _ => {
                    return Err("GATEWAY_CERT and GATEWAY_KEY must name the PEM certificate and key (make them with `gateway make-cert`), or set GATEWAY_TLS=0 for a private server over Tailscale".to_owned());
                }
            }
        } else {
            (PathBuf::new(), PathBuf::new())
        };
        let port = whole(env, "GATEWAY_PORT", if tls { 443 } else { 61000 }, 1, 65535)? as u16;
        let secret = check_secret(env.get("GATEWAY_SECRET"), "GATEWAY_SECRET")?;
        let url = |name: &str, default: &str| parse_loopback_url(optional(env, name).unwrap_or(default), name);
        let content = url("GATEWAY_CONTENT_URL", "http://127.0.0.1:61002")?;
        let backend = url("GATEWAY_BACKEND_URL", "http://127.0.0.1:61000")?;
        let ws = url("GATEWAY_WS_URL", "http://127.0.0.1:61000")?;
        let allowlist = if optional(env, "GATEWAY_ALLOWLIST") == Some("0") {
            None
        } else {
            let endpoint = url("ALLOWLIST_URL", "http://127.0.0.1:61005")?;
            let secret = env.get("ALLOWLIST_SECRET").ok_or_else(|| {
                "ALLOWLIST_SECRET is not set (the allowlist helper's secret; GATEWAY_ALLOWLIST=0 runs without the feed, and then no game ports open for anyone)".to_owned()
            })?;
            let secret = check_secret(Some(secret), "ALLOWLIST_SECRET")?;
            let refresh = whole(env, "GATEWAY_ALLOWLIST_REFRESH_SECONDS", 60, 5, 540)?;
            let additional: Vec<&str> = optional(env, "ALLOWLIST_ADDITIONAL_URLS")
                .map(|list| list.split(',').map(js::trim).filter(|s| !s.is_empty()).collect())
                .unwrap_or_default();
            if additional.len() > 4 {
                return Err("At most four additional allowlist helpers are supported".to_owned());
            }
            let additional = additional
                .into_iter()
                .map(|u| parse_loopback_url(u, "ALLOWLIST_ADDITIONAL_URLS"))
                .collect::<Result<Vec<_>, _>>()?;
            Some(Allowlist {
                url: format!("http://{}", endpoint.authority()),
                endpoint,
                secret,
                refresh: Duration::from_secs(refresh),
                additional,
                timeout: Duration::from_secs(3),
            })
        };
        let max_body_bytes = whole(env, "GATEWAY_MAX_BODY_BYTES", 131_072, 1024, 67_108_864)?;
        let max_connections = whole(env, "GATEWAY_MAX_CONNECTIONS", 2048, 1, 100_000)? as usize;
        let max_connections_per_ip = whole(env, "GATEWAY_MAX_CONNECTIONS_PER_IP", 128, 1, 100_000)? as usize;
        let rate = |name: &str, default: BucketSpec| match optional(env, name) {
            Some(text) => parse_bucket_spec(text, name),
            None => Ok(default),
        };
        let d = DEFAULT_LIMITS.rates;
        let rates = Rates {
            general: rate("GATEWAY_RATE_GENERAL", d.general)?,
            content: rate("GATEWAY_RATE_CONTENT", d.content)?,
            register: rate("GATEWAY_RATE_REGISTER", d.register)?,
            token: rate("GATEWAY_RATE_TOKEN", d.token)?,
            ws: rate("GATEWAY_RATE_WS", d.ws)?,
            connect: rate("GATEWAY_RATE_CONNECT", d.connect)?,
        };
        let node_links = node_links(env, tls)?;
        let network = node_links.as_ref().map(|n| network_nodes(&n.peers)).unwrap_or_default();
        let t = DEFAULT_TIMEOUTS;
        let timeouts = Timeouts {
            handshake: millis(env, "GATEWAY_HANDSHAKE_TIMEOUT_MS", t.handshake, 1000, 120_000)?,
            headers: millis(env, "GATEWAY_HEADERS_TIMEOUT_MS", t.headers, 1000, 120_000)?,
            request: millis(env, "GATEWAY_REQUEST_TIMEOUT_MS", t.request, 1000, 600_000)?,
            keep_alive: millis(env, "GATEWAY_KEEPALIVE_TIMEOUT_MS", t.keep_alive, 1000, 600_000)?,
            idle: millis(env, "GATEWAY_IDLE_TIMEOUT_MS", t.idle, 1000, 3_600_000)?,
            upstream_idle: millis(
                env,
                "GATEWAY_UPSTREAM_IDLE_TIMEOUT_MS",
                t.upstream_idle,
                1000,
                3_600_000,
            )?,
            ws_idle: millis(env, "GATEWAY_WS_IDLE_TIMEOUT_MS", t.ws_idle, 1000, 86_400_000)?,
            shutdown_grace: millis(env, "GATEWAY_SHUTDOWN_GRACE_MS", t.shutdown_grace, 0, 300_000)?,
            check_interval: t.check_interval,
        };
        Ok(Self {
            version,
            bind_host,
            port,
            tls,
            cert_file,
            key_file,
            secret,
            backend,
            content,
            ws,
            allowlist,
            limits: Limits {
                max_body_bytes,
                max_connections,
                max_connections_per_ip,
                max_headers: DEFAULT_LIMITS.max_headers,
                rates,
            },
            timeouts,
            network,
            node_links,
        })
    }

    /// `<bind>:<port>`, with brackets for IPv6.
    pub fn listen_address(&self) -> String {
        Endpoint::new(self.bind_host.clone(), self.port).authority()
    }
}

fn node_links(env: &Env, tls: bool) -> Result<Option<NodeLinkConfig>, String> {
    let nodes = optional(env, "GATEWAY_NETWORK_NODES");
    let Some(node) = optional(env, "GATEWAY_NODE_ID") else {
        if nodes.is_some() {
            return Err("GATEWAY_NETWORK_NODES needs GATEWAY_NODE_ID, this node's name".to_owned());
        }
        return Ok(None);
    };
    if !tls {
        return Err(
            "GATEWAY_NODE_ID (node links) needs the TLS gateway: a private server (GATEWAY_TLS=0) is plain HTTP"
                .to_owned(),
        );
    }
    let node: NodeId = node.parse().map_err(|e| format!("GATEWAY_NODE_ID: {e}"))?;
    let region = optional(env, "GATEWAY_NODE_REGION")
        .ok_or_else(|| "GATEWAY_NODE_REGION is not set (this node's region)".to_owned())?;
    let region: Region = region.parse().map_err(|e| format!("GATEWAY_NODE_REGION: {e}"))?;
    let file = |name: &str| {
        optional(env, name)
            .map(absolute)
            .ok_or_else(|| format!("{name} must name the PEM file `network issue-node-cert` wrote for this node"))
    };
    let cert_file = file("GATEWAY_NODE_CERT")?;
    let key_file = file("GATEWAY_NODE_KEY")?;
    let ca_file = file("GATEWAY_NODE_CA")?;
    let peers: Vec<Peer> = match nodes {
        Some(text) => serde_json::from_str(text).map_err(|e| format!("GATEWAY_NETWORK_NODES: {e}"))?,
        None => Vec::new(),
    };
    cluster::check_peers(&node, &peers).map_err(|e| format!("GATEWAY_NETWORK_NODES: {e}"))?;
    Ok(Some(NodeLinkConfig {
        identity: NodeIdentity { node, region },
        cert_file,
        key_file,
        ca_file,
        peers,
    }))
}

/// The peers that carry a fingerprint, the first per region, at most 16.
fn network_nodes(peers: &[Peer]) -> Vec<NetworkNode> {
    let mut out: Vec<NetworkNode> = Vec::new();
    for peer in peers {
        let Some(fingerprint) = &peer.fingerprint else {
            continue;
        };
        let region = peer.region.to_string();
        if out.len() >= MAX_NETWORK_NODES || out.iter().any(|n| n.region == region) {
            continue;
        }
        out.push(NetworkNode {
            region,
            host: peer.host.clone(),
            port: peer.port,
            fingerprint: fingerprint.clone(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "0123456789abcdef0123456789abcdef";

    fn env(pairs: &[(&str, &str)]) -> Env {
        let mut all = vec![("GATEWAY_SECRET", SECRET), ("ALLOWLIST_SECRET", SECRET)];
        all.extend_from_slice(pairs);
        Env::from_pairs(all)
    }

    fn error(pairs: &[(&str, &str)]) -> String {
        Config::from_env(&env(pairs)).unwrap_err()
    }

    #[test]
    fn private_defaults() {
        let c = Config::from_env(&env(&[("GATEWAY_TLS", "0"), ("GATEWAY_BIND", "127.0.0.1")])).unwrap();
        assert!(!c.tls);
        assert_eq!(c.port, 61000);
        assert_eq!(c.backend, Endpoint::new("127.0.0.1", 61000));
        assert_eq!(c.content.authority(), "127.0.0.1:61002");
        let a = c.allowlist.unwrap();
        assert_eq!(
            (a.url.as_str(), a.refresh),
            ("http://127.0.0.1:61005", Duration::from_secs(60))
        );
        assert_eq!(c.limits, DEFAULT_LIMITS);
        assert_eq!(c.timeouts, DEFAULT_TIMEOUTS);
        assert!(c.network.is_empty() && c.node_links.is_none());
    }

    #[test]
    fn errors_in_order() {
        assert_eq!(
            error(&[("GATEWAY_GAME", "legacy")]),
            "GATEWAY_GAME=legacy (Dauntless Revived's 1.4.4) is no longer served; this gateway serves reborn only"
        );
        assert_eq!(
            error(&[("GATEWAY_GAME", "x\"")]),
            "GATEWAY_GAME must be reborn (got \"x\\\"\")"
        );
        assert!(parse_game(Some(" Reborn ")).is_ok());
        assert_eq!(
            error(&[("GATEWAY_BIND", "nope")]),
            "GATEWAY_BIND must be an IP address (got \"nope\")"
        );
        assert!(error(&[("GATEWAY_TLS", "0")]).starts_with("GATEWAY_TLS=0 (private mode)"));
        assert!(error(&[("GATEWAY_TLS", "false")]).starts_with("GATEWAY_CERT and GATEWAY_KEY"));
        let private = [("GATEWAY_TLS", "0"), ("GATEWAY_BIND", "100.100.1.2")];
        assert!(Config::from_env(&env(&private)).is_ok());
        assert_eq!(
            error(&[("GATEWAY_TLS", "0"), ("GATEWAY_BIND", "::1"), ("GATEWAY_PORT", "0")]),
            "GATEWAY_PORT must be a whole number from 1 to 65535"
        );
        let p = |extra: (&'static str, &'static str)| {
            let mut v = vec![("GATEWAY_TLS", "0"), ("GATEWAY_BIND", "127.0.0.1")];
            v.push(extra);
            error(&v)
        };
        assert_eq!(
            p(("GATEWAY_SECRET", "short")),
            "GATEWAY_SECRET must be 32 to 256 printable characters without spaces (for example 64 hex characters)"
        );
        assert_eq!(
            p(("GATEWAY_BACKEND_URL", "https://127.0.0.1:1")),
            "GATEWAY_BACKEND_URL must be an http:// URL on this machine"
        );
        assert_eq!(
            p(("GATEWAY_BACKEND_URL", "http://10.0.0.1:1")),
            "GATEWAY_BACKEND_URL must point at 127.0.0.1 or ::1 (got 10.0.0.1)"
        );
        assert_eq!(
            p(("GATEWAY_BACKEND_URL", "http://127.0.0.1:1/x")),
            "GATEWAY_BACKEND_URL must be just http://host:port"
        );
        assert_eq!(p(("GATEWAY_BACKEND_URL", "nope")), "GATEWAY_BACKEND_URL is not a URL");
        assert_eq!(
            p(("ALLOWLIST_SECRET", "")),
            "ALLOWLIST_SECRET is not set (the allowlist helper's secret; GATEWAY_ALLOWLIST=0 runs without the feed, and then no game ports open for anyone)"
        );
        assert_eq!(
            p(("GATEWAY_ALLOWLIST_REFRESH_SECONDS", "4")),
            "GATEWAY_ALLOWLIST_REFRESH_SECONDS must be a whole number from 5 to 540"
        );
        assert_eq!(
            p((
                "ALLOWLIST_ADDITIONAL_URLS",
                "http://127.0.0.1:1,http://127.0.0.1:2,http://127.0.0.1:3,http://127.0.0.1:4,http://127.0.0.1:5"
            )),
            "At most four additional allowlist helpers are supported"
        );
        assert_eq!(
            p(("GATEWAY_MAX_BODY_BYTES", "1023")),
            "GATEWAY_MAX_BODY_BYTES must be a whole number from 1024 to 67108864"
        );
        assert!(p(("GATEWAY_RATE_WS", "x")).starts_with("GATEWAY_RATE_WS must look like"));
        assert_eq!(
            p(("GATEWAY_NETWORK_NODES", "[]")),
            "GATEWAY_NETWORK_NODES needs GATEWAY_NODE_ID, this node's name"
        );
        assert_eq!(
            p(("GATEWAY_NODE_ID", "uk-1")),
            "GATEWAY_NODE_ID (node links) needs the TLS gateway: a private server (GATEWAY_TLS=0) is plain HTTP"
        );
        assert_eq!(
            p(("GATEWAY_SHUTDOWN_GRACE_MS", "1.5")),
            "GATEWAY_SHUTDOWN_GRACE_MS must be a whole number from 0 to 300000"
        );
    }

    #[test]
    fn helpers() {
        assert_eq!(Endpoint::new("::1", 80).authority(), "[::1]:80");
        assert_eq!(
            parse_loopback_url("http://[::1]", "X").unwrap(),
            Endpoint::new("::1", 80)
        );
        assert!(is_tailscale_ipv4("100.64.0.1") && is_tailscale_ipv4("100.127.255.255"));
        assert!(!is_tailscale_ipv4("100.128.0.1") && !is_tailscale_ipv4("::ffff:100.64.0.1"));
        assert!(is_loopback_host("localhost") && is_loopback_host("127.9.9.9") && is_loopback_host("[::1]"));
        assert_eq!(check_secret(None, "S").unwrap_err(), "S is not set");
        assert!(check_secret(Some(&"a b".repeat(20)), "S").is_err());
        assert!(check_secret(Some(&"a".repeat(257)), "S").is_err());
    }

    #[test]
    fn network_nodes_first_per_region() {
        let peer = |id: &str, region: &str, fp: Option<&str>| Peer {
            id: id.parse().unwrap(),
            region: region.parse().unwrap(),
            host: "203.0.113.1".into(),
            port: 443,
            fingerprint: fp.map(str::to_owned),
        };
        let fp = "a".repeat(64);
        let nodes = network_nodes(&[
            peer("a", "uk", None),
            peer("b", "uk", Some(&fp)),
            peer("c", "uk", Some(&fp)),
        ]);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].region, "uk");
    }
}
