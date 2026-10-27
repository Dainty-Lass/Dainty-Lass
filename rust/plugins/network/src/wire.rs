//! What a node link puts on the wire (DN3): HTTP/1.1 over mutual TLS to the peer's gateway, `POST
//! /__node/<service>` with a JSON body for a call and `GET /__node/ping` for the health check. Both sides
//! read the same definitions, and everything a peer sends is decoded here, strictly, before anything else
//! looks at it (`fuzz/fuzz_targets/network_link.rs`).

use module_api::network::{NodeId, Region};
use serde::{Deserialize, Serialize};

use crate::link::{LinkError, MAX_MESSAGE, Service};

/// Every node-link route lives under this prefix, which the gateway refuses from anyone but a peer.
pub const PREFIX: &str = "/__node";
pub const PING_PATH: &str = "/__node/ping";
/// The sender's clock in unix milliseconds, on every request and answer
pub const TIME_HEADER: &str = "x-node-time";
/// The node-link protocol's version, in a ping's answer; another version is refused
pub const VERSION: u32 = 1;
/// A ping's answer and an error body are a few dozen bytes
pub const MAX_SMALL: usize = 4096;

/// What a node-link request is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Ping,
    Service(Service),
}

impl Target {
    pub fn path(self) -> String {
        match self {
            Self::Ping => PING_PATH.to_owned(),
            Self::Service(service) => format!("{PREFIX}/{}", service.as_str()),
        }
    }
}

/// The route a request target names: exactly `/__node/ping` or `/__node/<service>`. No query, no trailing
/// slash, no case folding, no percent-encoding: a peer is our own code, so anything else is refused.
pub fn parse_target(target: &str) -> Option<Target> {
    let name = target.strip_prefix("/__node/")?;
    if name == "ping" {
        return Some(Target::Ping);
    }
    Service::from_name(name).map(Target::Service)
}

/// A time header: digits only, the way a node writes it.
pub fn parse_time(header: &str) -> Option<i64> {
    if header.is_empty() || header.len() > 16 || !header.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    header.parse().ok()
}

/// The skew of a peer's clock against ours, in ms (positive: the peer is ahead).
pub fn skew_ms(local_ms: i64, remote_ms: i64) -> i64 {
    remote_ms.saturating_sub(local_ms)
}

pub fn within(skew_ms: i64, max: std::time::Duration) -> bool {
    u128::from(skew_ms.unsigned_abs()) <= max.as_millis()
}

/// A ping's answer: who answered, in which region, and its clock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PingAnswer {
    pub version: u32,
    pub node: NodeId,
    pub region: Region,
    pub time_ms: i64,
}

impl PingAnswer {
    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    /// Read whole or refused: the version this build speaks, nothing unknown, at most [`MAX_SMALL`] bytes.
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_SMALL {
            return Err("the ping answer is too long".into());
        }
        let answer: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if answer.version != VERSION {
            return Err(format!(
                "node-link version {} (this node speaks {VERSION})",
                answer.version
            ));
        }
        Ok(answer)
    }
}

/// The codes an error body carries: why a request was refused, for the caller's [`LinkError`].
pub mod code {
    pub const NOT_A_PEER: &str = "not_a_peer";
    pub const NOT_FOUND: &str = "not_found";
    pub const METHOD: &str = "method_not_allowed";
    pub const BAD_REQUEST: &str = "bad_request";
    pub const CLOCK_SKEW: &str = "clock_skew";
    pub const TOO_LARGE: &str = "too_large";
    pub const NO_SERVICE: &str = "no_service";
    pub const FAILED: &str = "service_failed";
}

/// `{"error":"<code>"}`, with `skewMs` for a clock refusal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorBody {
    pub error: String,
    #[serde(rename = "skewMs", default, skip_serializing_if = "Option::is_none")]
    pub skew_ms: Option<i64>,
}

impl ErrorBody {
    pub fn new(error: &str) -> Self {
        Self {
            error: error.to_owned(),
            skew_ms: None,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap_or_default()
    }

    pub fn decode(bytes: &[u8]) -> Option<Self> {
        if bytes.len() > MAX_SMALL {
            return None;
        }
        serde_json::from_slice(bytes).ok()
    }
}

/// The status and error body a [`LinkError`] from a [`NodeService`](crate::link::NodeService) becomes.
pub fn failure(error: &LinkError) -> (u16, ErrorBody) {
    match error {
        LinkError::TooLarge => (413, ErrorBody::new(code::TOO_LARGE)),
        LinkError::NoService { .. } => (503, ErrorBody::new(code::NO_SERVICE)),
        _ => (502, ErrorBody::new(code::FAILED)),
    }
}

fn answered(status: u16, code: &str) -> String {
    if code.is_empty() {
        format!("it answered {status}")
    } else {
        format!("it answered {status} ({code})")
    }
}

/// What a peer's non-200 answer means to the node that called it. `service` is the call's, `None` for a ping.
pub fn link_error(node: &NodeId, service: Option<Service>, status: u16, body: &[u8]) -> LinkError {
    let body = ErrorBody::decode(body);
    let error = body.as_ref().map_or("", |b| b.error.as_str());
    match (error, service) {
        (code::CLOCK_SKEW, _) => LinkError::ClockSkew {
            node: node.clone(),
            // The peer measured ours against its clock; the error is the peer's against ours
            skew_ms: body.and_then(|b| b.skew_ms).map_or(i64::MAX, i64::saturating_neg),
        },
        (code::NO_SERVICE, Some(service)) => LinkError::NoService {
            node: node.clone(),
            service,
        },
        (code::TOO_LARGE, _) => LinkError::TooLarge,
        (code::NOT_A_PEER, _) => LinkError::Refused {
            node: node.clone(),
            reason: "it does not list this node as a peer".into(),
        },
        _ if status == 403 => LinkError::Refused {
            node: node.clone(),
            reason: "forbidden".into(),
        },
        _ if status >= 500 => LinkError::Unreachable {
            node: node.clone(),
            reason: answered(status, error),
        },
        _ => LinkError::Protocol {
            node: node.clone(),
            reason: answered(status, error),
        },
    }
}

/// A body over the cap, by its declared length.
pub fn too_large(len: u64) -> bool {
    len > MAX_MESSAGE as u64
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn only_the_exact_routes_are_targets() {
        assert_eq!(parse_target("/__node/ping"), Some(Target::Ping));
        assert_eq!(
            parse_target("/__node/directory"),
            Some(Target::Service(Service::Directory))
        );
        for bad in [
            "",
            "/__node",
            "/__node/",
            "/__node/ping/",
            "/__node/ping?x=1",
            "/__node/Ping",
            "/__node/directory/x",
            "/__node//directory",
            "/__node/%64irectory",
            "//__node/directory",
            "/__NODE/directory",
            "/__origin/h/__node/directory",
            "/__node/directory ",
        ] {
            assert_eq!(parse_target(bad), None, "{bad:?}");
        }
        for target in [Target::Ping, Target::Service(Service::Directory)] {
            assert_eq!(parse_target(&target.path()), Some(target));
        }
    }

    #[test]
    fn times_are_digits() {
        assert_eq!(parse_time("1700000000000"), Some(1_700_000_000_000));
        for bad in ["", "-1", "+1", "1.5", "1e3", " 1", "12345678901234567"] {
            assert_eq!(parse_time(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_skew_bound() {
        let max = std::time::Duration::from_secs(30);
        assert!(within(0, max) && within(30_000, max) && within(-30_000, max));
        assert!(!within(30_001, max) && !within(-30_001, max) && !within(i64::MIN, max));
        assert_eq!(skew_ms(1000, 4000), 3000);
    }

    #[test]
    fn a_ping_answer_is_read_whole_or_refused() {
        let answer = PingAnswer {
            version: VERSION,
            node: "oregon-1".parse().unwrap(),
            region: "oregon".parse().unwrap(),
            time_ms: 5,
        };
        assert_eq!(PingAnswer::decode(&answer.encode()), Ok(answer));
        for bad in [
            r#"{"version":2,"node":"a","region":"b","time_ms":1}"#,
            r#"{"version":1,"node":"A","region":"b","time_ms":1}"#,
            r#"{"version":1,"node":"a","region":"b","time_ms":1,"x":1}"#,
            r#"{"version":1,"node":"a","region":"b"}"#,
            r#"{"version":1,"node":"a","region":"b","time_ms":"1"}"#,
            "[]",
            "",
        ] {
            assert!(PingAnswer::decode(bad.as_bytes()).is_err(), "{bad}");
        }
        assert!(PingAnswer::decode(&vec![b' '; MAX_SMALL + 1]).is_err());
    }

    #[test]
    fn an_error_answer_becomes_the_callers_error() {
        let node: NodeId = "uk-1".parse().unwrap();
        let body = |code: &str| ErrorBody::new(code).encode();
        assert!(matches!(
            link_error(&node, None, 403, &body(code::NOT_A_PEER)),
            LinkError::Refused { .. }
        ));
        assert!(matches!(
            link_error(&node, None, 403, b"nope"),
            LinkError::Refused { .. }
        ));
        assert_eq!(
            link_error(&node, None, 413, &body(code::TOO_LARGE)),
            LinkError::TooLarge
        );
        assert_eq!(
            link_error(&node, Some(Service::Directory), 503, &body(code::NO_SERVICE)),
            LinkError::NoService {
                node: node.clone(),
                service: Service::Directory
            }
        );
        let skewed = ErrorBody {
            error: code::CLOCK_SKEW.into(),
            skew_ms: Some(90_000),
        };
        assert_eq!(
            link_error(&node, None, 409, &skewed.encode()),
            LinkError::ClockSkew {
                node: node.clone(),
                skew_ms: -90_000
            }
        );
        assert!(matches!(
            link_error(&node, None, 502, &body(code::FAILED)),
            LinkError::Unreachable { .. }
        ));
        assert!(matches!(
            link_error(&node, None, 404, &body(code::NOT_FOUND)),
            LinkError::Protocol { .. }
        ));
    }
}
