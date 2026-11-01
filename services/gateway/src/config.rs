//! Settings from the environment (`src/config.ts`), read once at start; a bad one stops the start.

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use protocol::GameVersion;
use protocol::invite::reforged_version;
use serde_json::json;
use service_kit::env::Env;

use crate::ratelimit::{BucketSpec, parse_bucket_spec};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
}

impl Endpoint {
    /// `host:port` as a socket address string, IPv6 in brackets.
    pub fn authority(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Rates {
    pub general: BucketSpec,
    pub content: BucketSpec,
    pub register: BucketSpec,
    pub token: BucketSpec,
    /// WebSocket upgrades: the game's chat connection
    pub ws: BucketSpec,
    /// New TCP connections (each costs a TLS handshake)
    pub connect: BucketSpec,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Limits {
    pub max_body_bytes: u64,
    pub max_connections: usize,
    pub max_connections_per_ip: usize,
    pub max_headers_count: usize,
    pub rate: Rates,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timeouts {
    pub handshake: Duration,
    pub headers: Duration,
    pub request: Duration,
    pub keep_alive: Duration,
    pub idle: Duration,
    pub upstream_idle: Duration,
    pub ws_idle: Duration,
    pub shutdown_grace: Duration,
    pub check_interval: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Allowlist {
    /// http://127.0.0.1:61005, the allowlist helper
    pub url: String,
    pub secret: String,
    pub refresh: Duration,
    pub timeout: Duration,
    pub additional_urls: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// `GATEWAY_GAME`: the game this gateway fronts, 1.14.7 on Rustic Isles or Dauntless Revived's 1.4.4
    pub version: GameVersion,
    pub bind_host: String,
    pub port: u16,
    /// Public mode: TLS with the pinned self-signed certificate. Private mode (`GATEWAY_TLS=0`):
    /// plain HTTP, only on a Tailscale or loopback address (Tailscale encrypts the traffic)
    pub tls: bool,
    /// Empty in private mode
    pub cert_file: PathBuf,
    pub key_file: PathBuf,
    pub secret: String,
    /// The Rustic Isles backend, or DR's metagame in legacy mode (`GATEWAY_METAGAME_URL`)
    pub backend: Endpoint,
    pub content: Endpoint,
    pub ws: Endpoint,
    pub allowlist: Option<Allowlist>,
    pub limits: Limits,
    pub timeouts: Timeouts,
    /// The network's nodes the launcher pings (DN7): the peers `[[network.nodes]]` lists with a fingerprint.
    /// Empty for a server outside a network, which is how it answers anyway
    pub network: Vec<crate::network::NetworkNode>,
    /// Node links (DN3); `None` for a server outside a network
    pub node_links: Option<NodeLinkConfig>,
}

/// Dauntless Revived's limits, sized from real 1.4.4 sessions (its README has the numbers). The
/// Rustic Isles backend parses JSON bodies with Express's default 100 KB limit, which fits under
/// the 128 KiB body cap.
pub const DEFAULT_LIMITS: Limits = Limits {
    max_body_bytes: 128 * 1024,
    max_connections: 2048,
    max_connections_per_ip: 128,
    max_headers_count: 100,
    rate: Rates {
        general: BucketSpec::new(300, 180.0),
        content: BucketSpec::new(600, 600.0),
        register: BucketSpec::new(5, 0.2),
        token: BucketSpec::new(10, 1.0),
        // The game reconnects its chat at most every 15-45 s after a failure; a few players behind
        // one address fit many times over
        ws: BucketSpec::new(20, 12.0),
        connect: BucketSpec::new(200, 300.0),
    },
};

pub const DEFAULT_TIMEOUTS: Timeouts = Timeouts {
    handshake: Duration::from_secs(10),
    headers: Duration::from_secs(10),
    request: Duration::from_secs(30),
    keep_alive: Duration::from_secs(65),
    idle: Duration::from_secs(120),
    upstream_idle: Duration::from_secs(120),
    ws_idle: Duration::from_secs(300),
    shutdown_grace: Duration::from_secs(10),
    check_interval: Duration::from_secs(1),
};

fn int(env: &Env, name: &str, default: i64, min: i64, max: i64) -> Result<i64, String> {
    let Some(raw) = env.raw(name).filter(|r| !r.trim().is_empty()) else {
        return Ok(default);
    };
    let value = service_kit::js::number(Some(&json!(raw)));
    if value.fract() != 0.0 || value < min as f64 || value > max as f64 {
        return Err(format!("{name} must be a whole number from {min} to {max}"));
    }
    Ok(value as i64)
}

fn ms(env: &Env, name: &str, default: Duration, min: i64, max: i64) -> Result<Duration, String> {
    let value = int(env, name, default.as_millis() as i64, min, max)?;
    Ok(Duration::from_millis(value as u64))
}

fn optional(env: &Env, name: &str) -> Option<String> {
    env.raw(name)
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(str::to_owned)
}

/// `path.resolve`.
fn resolve(path: &str) -> PathBuf {
    std::path::absolute(Path::new(path)).unwrap_or_else(|_| PathBuf::from(path))
}

pub fn is_loopback_host(host: &str) -> bool {
    if host == "localhost" {
        return true;
    }
    if host.parse::<std::net::Ipv4Addr>().is_ok() {
        return host.starts_with("127.");
    }
    host == "::1" || host == "[::1]"
}

/// `IsTailscaleIPv4`: Tailscale's address range, 100.64.0.0/10.
pub fn is_tailscale_ipv4(host: &str) -> bool {
    host.parse::<std::net::Ipv4Addr>()
        .is_ok_and(|ip| ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1]))
}

/// Upstreams are plain http:// on this machine only: the gateway secret header must never leave it.
pub fn parse_loopback_url(text: &str, name: &str) -> Result<Endpoint, String> {
    let parsed = url::Url::parse(text).map_err(|_| format!("{name} is not a URL"))?;
    if parsed.scheme() != "http" {
        return Err(format!("{name} must be an http:// URL on this machine"));
    }
    let hostname = parsed.host_str().unwrap_or("");
    let host = hostname.trim_start_matches('[').trim_end_matches(']');
    if !is_loopback_host(host) {
        return Err(format!("{name} must point at 127.0.0.1 or ::1 (got {hostname})"));
    }
    if parsed.path() != "/"
        || parsed.query().is_some_and(|q| !q.is_empty())
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(format!("{name} must be just http://host:port"));
    }
    Ok(Endpoint {
        host: host.to_owned(),
        port: parsed.port().unwrap_or(80),
    })
}

/// Shared secrets travel in headers: printable ASCII without spaces, long enough not to guess.
pub fn check_secret(value: Option<String>, name: &str) -> Result<String, String> {
    let Some(value) = value else {
        return Err(format!("{name} is not set"));
    };
    if !(32..=256).contains(&value.len()) || !value.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
        return Err(format!(
            "{name} must be 32 to 256 printable characters without spaces (for example 64 hex characters)"
        ));
    }
    Ok(value)
}

fn helper_url(text: &str, name: &str) -> Result<String, String> {
    let point = parse_loopback_url(text, name)?;
    Ok(format!("http://{}", point.authority()))
}

/// `GATEWAY_GAME`: `reborn` (the default, 1.14.7). `legacy` was Dauntless Revived's 1.4.4, which is no longer served.
pub fn parse_game(raw: Option<&str>) -> Result<GameVersion, String> {
    match raw.map(|r| r.trim().to_ascii_lowercase()).as_deref() {
        None | Some("") | Some("reborn") => Ok(reforged_version()),
        Some("legacy") => Err(
            "GATEWAY_GAME=legacy (Dauntless Revived's 1.4.4) is no longer served; this gateway serves reborn only"
                .into(),
        ),
        Some(_) => Err(format!(
            "GATEWAY_GAME must be reborn (got {})",
            serde_json::to_string(raw.unwrap_or_default()).unwrap_or_default()
        )),
    }
}

/// Links to the other nodes of a network (DD16, DN3). Off unless `GATEWAY_NODE_ID` is set.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeLinkConfig {
    /// This node and its region (`GATEWAY_NODE_ID`, `GATEWAY_NODE_REGION`); the node certificate must say the same
    pub identity: module_api::NodeIdentity,
    pub cert_file: PathBuf,
    pub key_file: PathBuf,
    /// The cluster CA's certificate: never its key
    pub ca_file: PathBuf,
    /// The nodes that may call this one (`GATEWAY_NETWORK_NODES`, a JSON list `network.nodes` writes)
    pub peers: Vec<network_plugin::cluster::Peer>,
}

fn node_link_config(env: &Env, tls: bool) -> Result<Option<NodeLinkConfig>, String> {
    let Some(node) = optional(env, "GATEWAY_NODE_ID") else {
        return match optional(env, "GATEWAY_NETWORK_NODES") {
            Some(_) => Err("GATEWAY_NETWORK_NODES needs GATEWAY_NODE_ID, this node's name".into()),
            None => Ok(None),
        };
    };
    if !tls {
        return Err(
            "GATEWAY_NODE_ID (node links) needs the TLS gateway: a private server (GATEWAY_TLS=0) is plain HTTP".into(),
        );
    }
    let node: module_api::network::NodeId = node.parse().map_err(|e| format!("GATEWAY_NODE_ID: {e}"))?;
    let region = optional(env, "GATEWAY_NODE_REGION").ok_or("GATEWAY_NODE_REGION is not set (this node's region)")?;
    let region = region.parse().map_err(|e| format!("GATEWAY_NODE_REGION: {e}"))?;
    let file = |name: &str| {
        optional(env, name)
            .map(|path| resolve(&path))
            .ok_or_else(|| format!("{name} must name the PEM file `network issue-node-cert` wrote for this node"))
    };
    let peers: Vec<network_plugin::cluster::Peer> = match optional(env, "GATEWAY_NETWORK_NODES") {
        Some(text) => serde_json::from_str(&text).map_err(|e| format!("GATEWAY_NETWORK_NODES: {e}"))?,
        None => Vec::new(),
    };
    network_plugin::cluster::check_peers(&node, &peers).map_err(|e| format!("GATEWAY_NETWORK_NODES: {e}"))?;
    Ok(Some(NodeLinkConfig {
        identity: module_api::NodeIdentity { node, region },
        cert_file: file("GATEWAY_NODE_CERT")?,
        key_file: file("GATEWAY_NODE_KEY")?,
        ca_file: file("GATEWAY_NODE_CA")?,
        peers,
    }))
}

/// The peers the launcher can ping (DN7): those listed with the fingerprint of their public certificate,
/// one per region, at most `MAX_NODES`.
fn listed_nodes(peers: &[network_plugin::cluster::Peer]) -> Vec<crate::network::NetworkNode> {
    let mut out: Vec<crate::network::NetworkNode> = Vec::new();
    for peer in peers {
        let Some(fingerprint) = &peer.fingerprint else {
            continue;
        };
        if out.iter().any(|node| node.region == peer.region.as_str()) || out.len() >= crate::network::MAX_NODES {
            continue;
        }
        out.push(crate::network::NetworkNode {
            region: peer.region.to_string(),
            host: peer.host.clone(),
            port: peer.port,
            fingerprint: fingerprint.clone(),
        });
    }
    out
}

impl Config {
    /// `LoadGatewayConfig`.
    pub fn from_env(env: &Env) -> Result<Self, String> {
        let version = parse_game(env.raw("GATEWAY_GAME"))?;
        let bind_host = optional(env, "GATEWAY_BIND").unwrap_or_else(|| "0.0.0.0".into());
        if bind_host.parse::<IpAddr>().is_err() {
            return Err(format!(
                "GATEWAY_BIND must be an IP address (got {})",
                serde_json::to_string(&bind_host).unwrap_or_default()
            ));
        }
        let tls = env.raw("GATEWAY_TLS") != Some("0");
        // Plain HTTP never faces the internet: the account key and the game's session token travel in it
        if !tls && !is_tailscale_ipv4(&bind_host) && !is_loopback_host(&bind_host) {
            return Err(format!(
                "GATEWAY_TLS=0 (private mode) listens only on this PC's Tailscale address (100.64.0.0/10) or on loopback, not on {bind_host}"
            ));
        }
        let (cert_file, key_file) = match (optional(env, "GATEWAY_CERT"), optional(env, "GATEWAY_KEY")) {
            (Some(cert), Some(key)) if tls => (resolve(&cert), resolve(&key)),
            _ if tls => {
                return Err("GATEWAY_CERT and GATEWAY_KEY must name the PEM certificate and key (make them with `gateway make-cert`), or set GATEWAY_TLS=0 for a private server over Tailscale".into());
            }
            _ => (PathBuf::new(), PathBuf::new()),
        };
        let rate = |name: &str, default: BucketSpec| -> Result<BucketSpec, String> {
            optional(env, name).map_or(Ok(default), |raw| parse_bucket_spec(&raw, name))
        };
        let port = int(env, "GATEWAY_PORT", if tls { 443 } else { 61000 }, 1, 65535)? as u16;
        let secret = check_secret(optional(env, "GATEWAY_SECRET"), "GATEWAY_SECRET")?;
        let upstream = |name: &str, default: &str| {
            parse_loopback_url(&optional(env, name).unwrap_or_else(|| default.into()), name)
        };
        let content = upstream("GATEWAY_CONTENT_URL", "http://127.0.0.1:61002")?;
        // The backend serves the game's chat (XMPP over WebSocket) on its own listener
        let backend = upstream("GATEWAY_BACKEND_URL", "http://127.0.0.1:61000")?;
        let ws = upstream("GATEWAY_WS_URL", "http://127.0.0.1:61000")?;
        // Without the feed nobody's game ports open, so a missing secret is an error, not a silent off
        let allowlist = if env.raw("GATEWAY_ALLOWLIST") == Some("0") {
            None
        } else {
            let url = helper_url(
                &optional(env, "ALLOWLIST_URL").unwrap_or_else(|| "http://127.0.0.1:61005".into()),
                "ALLOWLIST_URL",
            )?;
            let Some(secret) = optional(env, "ALLOWLIST_SECRET") else {
                return Err("ALLOWLIST_SECRET is not set (the allowlist helper's secret; GATEWAY_ALLOWLIST=0 runs without the feed, and then no game ports open for anyone)".into());
            };
            let secret = check_secret(Some(secret), "ALLOWLIST_SECRET")?;
            let refresh = Duration::from_secs(int(env, "GATEWAY_ALLOWLIST_REFRESH_SECONDS", 60, 5, 540)? as u64);
            let additional: Vec<String> = optional(env, "ALLOWLIST_ADDITIONAL_URLS")
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|u| !u.is_empty())
                .map(str::to_owned)
                .collect();
            if additional.len() > 4 {
                return Err("At most four additional allowlist helpers are supported".into());
            }
            let additional_urls = additional
                .iter()
                .map(|u| helper_url(u, "ALLOWLIST_ADDITIONAL_URLS"))
                .collect::<Result<_, _>>()?;
            Some(Allowlist {
                url,
                secret,
                refresh,
                timeout: Duration::from_secs(3),
                additional_urls,
            })
        };
        let d = &DEFAULT_LIMITS;
        let limits = Limits {
            max_body_bytes: int(
                env,
                "GATEWAY_MAX_BODY_BYTES",
                d.max_body_bytes as i64,
                1024,
                64 * 1024 * 1024,
            )? as u64,
            max_connections: int(env, "GATEWAY_MAX_CONNECTIONS", d.max_connections as i64, 1, 100_000)? as usize,
            max_connections_per_ip: int(
                env,
                "GATEWAY_MAX_CONNECTIONS_PER_IP",
                d.max_connections_per_ip as i64,
                1,
                100_000,
            )? as usize,
            max_headers_count: d.max_headers_count,
            rate: Rates {
                general: rate("GATEWAY_RATE_GENERAL", d.rate.general)?,
                content: rate("GATEWAY_RATE_CONTENT", d.rate.content)?,
                register: rate("GATEWAY_RATE_REGISTER", d.rate.register)?,
                token: rate("GATEWAY_RATE_TOKEN", d.rate.token)?,
                ws: rate("GATEWAY_RATE_WS", d.rate.ws)?,
                connect: rate("GATEWAY_RATE_CONNECT", d.rate.connect)?,
            },
        };
        let node_links = node_link_config(env, tls)?;
        let t = &DEFAULT_TIMEOUTS;
        let timeouts = Timeouts {
            handshake: ms(env, "GATEWAY_HANDSHAKE_TIMEOUT_MS", t.handshake, 1000, 120_000)?,
            headers: ms(env, "GATEWAY_HEADERS_TIMEOUT_MS", t.headers, 1000, 120_000)?,
            request: ms(env, "GATEWAY_REQUEST_TIMEOUT_MS", t.request, 1000, 600_000)?,
            keep_alive: ms(env, "GATEWAY_KEEPALIVE_TIMEOUT_MS", t.keep_alive, 1000, 600_000)?,
            idle: ms(env, "GATEWAY_IDLE_TIMEOUT_MS", t.idle, 1000, 3_600_000)?,
            upstream_idle: ms(
                env,
                "GATEWAY_UPSTREAM_IDLE_TIMEOUT_MS",
                t.upstream_idle,
                1000,
                3_600_000,
            )?,
            ws_idle: ms(env, "GATEWAY_WS_IDLE_TIMEOUT_MS", t.ws_idle, 1000, 86_400_000)?,
            shutdown_grace: ms(env, "GATEWAY_SHUTDOWN_GRACE_MS", t.shutdown_grace, 0, 300_000)?,
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
            limits,
            timeouts,
            network: node_links
                .as_ref()
                .map_or_else(Vec::new, |links| listed_nodes(&links.peers)),
            node_links,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(extra: &[(&str, &str)], without: &[&str]) -> Result<Config, String> {
        let gateway_secret = "s".repeat(40);
        let allowlist_secret = "a".repeat(40);
        let mut pairs: Vec<(&str, &str)> = vec![
            ("GATEWAY_CERT", "c.pem"),
            ("GATEWAY_KEY", "k.pem"),
            ("GATEWAY_SECRET", &gateway_secret),
            ("ALLOWLIST_SECRET", &allowlist_secret),
        ];
        pairs.retain(|(k, _)| !without.contains(k));
        pairs.extend_from_slice(extra);
        Config::from_env(&Env::from_pairs(pairs))
    }

    fn fails(extra: &[(&str, &str)], without: &[&str], mentions: &str) {
        let error = load(extra, without).expect_err(mentions);
        assert!(error.contains(mentions), "{error} should mention {mentions}");
    }

    #[test]
    fn the_four_worker_tunnels_but_no_external_helpers() {
        let urls: Vec<String> = [61015, 61025, 61035, 61045]
            .iter()
            .map(|p| format!("http://127.0.0.1:{p}"))
            .collect();
        let joined = urls.join(",");
        let config = load(&[("ALLOWLIST_ADDITIONAL_URLS", &joined)], &[]).unwrap();
        assert_eq!(config.allowlist.unwrap().additional_urls, urls);
        let five = format!("{joined},http://127.0.0.1:61055");
        fails(&[("ALLOWLIST_ADDITIONAL_URLS", &five)], &[], "four");
        fails(
            &[("ALLOWLIST_ADDITIONAL_URLS", "http://203.0.113.5:61005")],
            &[],
            "127.0.0.1",
        );
    }

    #[test]
    fn the_documented_defaults() {
        let config = load(&[], &[]).unwrap();
        assert_eq!(config.bind_host, "0.0.0.0");
        assert_eq!(config.port, 443);
        let endpoint = |port| Endpoint {
            host: "127.0.0.1".into(),
            port,
        };
        assert!(config.tls);
        assert_eq!(config.backend, endpoint(61000));
        assert_eq!(config.content, endpoint(61002));
        assert_eq!(config.ws, endpoint(61000));
        let allowlist = config.allowlist.unwrap();
        assert_eq!(allowlist.url, "http://127.0.0.1:61005");
        assert_eq!(allowlist.refresh, Duration::from_secs(60));
        assert_eq!(config.limits, DEFAULT_LIMITS);
        assert_eq!(config.limits.max_body_bytes, 131_072);
        assert_eq!(config.timeouts, DEFAULT_TIMEOUTS);
        assert!(config.cert_file.is_absolute());
    }

    #[test]
    fn private_mode_is_plain_http_on_a_tailscale_or_loopback_address_only() {
        let private = [("GATEWAY_TLS", "0"), ("GATEWAY_ALLOWLIST", "0")];
        let without = ["GATEWAY_CERT", "GATEWAY_KEY", "ALLOWLIST_SECRET"];
        let config = load(&[private[0], private[1], ("GATEWAY_BIND", "100.101.102.103")], &without).unwrap();
        assert!(!config.tls);
        assert_eq!(config.port, 61000);
        assert_eq!(config.bind_host, "100.101.102.103");
        assert_eq!(config.cert_file, PathBuf::new());
        assert!(
            !load(&[private[0], private[1], ("GATEWAY_BIND", "127.0.0.1")], &without)
                .unwrap()
                .tls
        );
        for bind in ["0.0.0.0", "203.0.113.7", "192.168.1.20", "100.128.0.1", "::"] {
            fails(&[private[0], private[1], ("GATEWAY_BIND", bind)], &without, "Tailscale");
        }
        assert!(load(&[], &[]).unwrap().tls);
        // Any value but 0 is public mode, which needs the certificate
        fails(&[("GATEWAY_TLS", "false")], &["GATEWAY_CERT"], "GATEWAY_TLS=0");
        assert!(is_tailscale_ipv4("100.64.0.1") && is_tailscale_ipv4("100.127.255.255"));
        assert!(!is_tailscale_ipv4("100.63.0.1") && !is_tailscale_ipv4("::ffff:100.64.0.1"));
    }

    #[test]
    fn refuses_missing_and_weak_secrets_and_remote_upstreams() {
        fails(&[], &["GATEWAY_SECRET"], "GATEWAY_SECRET");
        fails(&[("GATEWAY_SECRET", "short")], &["GATEWAY_SECRET"], "GATEWAY_SECRET");
        let spaced = "has spaces ".repeat(4);
        fails(&[("GATEWAY_SECRET", &spaced)], &["GATEWAY_SECRET"], "GATEWAY_SECRET");
        fails(&[], &["GATEWAY_CERT"], "GATEWAY_CERT");
        fails(&[], &["ALLOWLIST_SECRET"], "ALLOWLIST_SECRET");
        assert_eq!(
            load(&[("GATEWAY_ALLOWLIST", "0")], &["ALLOWLIST_SECRET"])
                .unwrap()
                .allowlist,
            None
        );
        fails(&[("GATEWAY_BACKEND_URL", "http://10.0.0.5:61000")], &[], "127.0.0.1");
        fails(&[("GATEWAY_CONTENT_URL", "https://127.0.0.1:61002")], &[], "http://");
        fails(&[("ALLOWLIST_URL", "http://203.0.113.5:61005")], &[], "127.0.0.1");
        fails(
            &[("GATEWAY_BACKEND_URL", "http://127.0.0.1:61000/x")],
            &[],
            "just http://host:port",
        );
        fails(&[("GATEWAY_BIND", "example.org")], &[], "GATEWAY_BIND");
        fails(&[("GATEWAY_RATE_REGISTER", "lots")], &[], "GATEWAY_RATE_REGISTER");
        fails(&[("GATEWAY_PORT", "0")], &[], "GATEWAY_PORT");
        let register = load(&[("GATEWAY_RATE_REGISTER", "2,0.5")], &[])
            .unwrap()
            .limits
            .rate
            .register;
        assert_eq!(register, BucketSpec::new(2, 0.5));
        assert_eq!(
            load(&[("GATEWAY_RATE_WS", "5,2")], &[]).unwrap().limits.rate.ws,
            BucketSpec::new(5, 2.0)
        );
        fails(&[("GATEWAY_RATE_WS", "many")], &[], "GATEWAY_RATE_WS");
        let v6 = load(&[("GATEWAY_WS_URL", "http://[::1]:7000"), ("GATEWAY_BIND", "::")], &[]).unwrap();
        assert_eq!(v6.ws.authority(), "[::1]:7000");
        let localhost = load(&[("ALLOWLIST_URL", "http://localhost:9")], &[]).unwrap();
        assert_eq!(localhost.allowlist.unwrap().url, "http://localhost:9");
    }

    #[test]
    fn the_game_is_reborn() {
        assert_eq!(load(&[], &[]).unwrap().version, reforged_version());
        assert_eq!(
            load(&[("GATEWAY_GAME", " Reborn ")], &[]).unwrap().version,
            reforged_version()
        );
        fails(&[("GATEWAY_GAME", "revived")], &[], "GATEWAY_GAME must be reborn");
        fails(&[("GATEWAY_GAME", "legacy")], &[], "no longer served");
    }
}
