// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The gateway library's messages (RD27): what the host passed the gateway and the allowlist in-process, as
//! control traffic only. The data plane (TLS, policy, proxying to backends) stays in the library; the core answers
//! what only it knows: where modules listen, the node services, the remote-enroll handler and the network ring.
//!
//! Bytes travel as base64, and a body over [`PART`] goes as [`ToGateway::Part`]s or [`FromGateway::Part`]s under
//! its call's id before the message that ends it, so a 1 MiB node-link message fits the 1 MiB frames.

use std::collections::{BTreeMap, HashMap};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use module_api::network::{NodeId, NodeIdentity};
use module_api::{ModuleId, PortRange};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use tokio::sync::mpsc;

use crate::FromService;

/// Raw bytes per part: base64 makes it 4/3 larger, still under the frame limit.
pub const PART: usize = 512 * 1024;
/// The most one call's parts add up to: a node link's message (the network plugin's `MAX_MESSAGE`).
pub const MAX_BODY: usize = 1 << 20;
/// Calls with parts outstanding at once; more is a broken peer.
const MAX_OPEN: usize = 64;

/// What the core sends the gateway library.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ToGateway {
    /// Before each Start: how this core composes the gateway
    Configure {
        /// Routes come from the registry's modules (attach) rather than the stand-in for `GATEWAY_GAME`
        registry: bool,
        allowlist: AllowlistMode,
        /// The chat listener's port when the core runs chat on the backend's (compose.rs's override)
        ws_port: Option<u16>,
    },
    /// A module's ports as the controller placed them: before its attach, and again once it runs
    Placed {
        module: ModuleId,
        ports: BTreeMap<String, PortRange>,
        running: bool,
    },
    Stopped {
        module: ModuleId,
    },
    /// The network ring admits a player to a game server's UDP port (DN8, DN9)
    AdmitUdp {
        ip: String,
        port: u16,
    },
    AddPeer {
        node: NodeId,
    },
    RemovePeer {
        node: NodeId,
    },
    /// Whether the core redeems `POST /__node/enroll` (DD27, RM2)
    Enroll {
        present: bool,
    },
    Part {
        id: u64,
        data: Bytes,
    },
    NodeAnswer {
        id: u64,
        result: Result<Bytes, NodeFailure>,
    },
    Enrolled {
        id: u64,
        status: u16,
        body: Bytes,
    },
    RegionAnswer {
        id: u64,
        accepted: bool,
    },
}

/// What the gateway library sends the core.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum FromGateway {
    /// The node links are up, as this node: the core binds its node services to them
    NodeLinks {
        identity: NodeIdentity,
    },
    Part {
        id: u64,
        data: Bytes,
    },
    /// A peer's call the endpoint admitted (DN3): the caller its certificate named, and the service by name
    NodeCall {
        id: u64,
        from: NodeId,
        service: String,
        body: Bytes,
    },
    Enroll {
        id: u64,
        body: Bytes,
    },
    /// `POST /network/v1/regions`, shape-checked already (DN7, DN8)
    RegionReport {
        id: u64,
        api_key: Option<String>,
        ticket: Option<String>,
        regions: Vec<RegionRtt>,
    },
}

/// How the allowlist runs beside this gateway (DF4d).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum AllowlistMode {
    /// No allowlist plugin: UDP ports are not guarded here (private mode)
    Off,
    /// In the library, on `allowlist.env`: it guards modules' UDP ranges and takes the gateway's admits directly
    InProcess { env: Vec<(String, String)> },
    /// In a helper process guarding `base`: a module's UDP ranges must lie inside it
    External { base: PortRange },
    /// A public server without one: every UDP port is refused
    Absent,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegionRtt {
    pub region: String,
    pub rtt_ms: Option<f64>,
}

/// Why the core's node services did not answer, as the endpoint tells the peer (`wire::failure`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NodeFailure {
    TooLarge,
    NoService,
    Refused { reason: String },
    Failed { reason: String },
}

/// Bytes, as base64 on the wire.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Bytes(pub Vec<u8>);

impl Serialize for Bytes {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(&self.0))
    }
}

impl<'de> Deserialize<'de> for Bytes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        STANDARD.decode(text).map(Bytes).map_err(serde::de::Error::custom)
    }
}

/// `body` cut for the wire: the parts that go first, and the last piece the ending message carries.
pub fn split(mut body: Vec<u8>) -> (Vec<Bytes>, Bytes) {
    if body.len() <= PART {
        return (Vec::new(), Bytes(body));
    }
    let mut parts = Vec::with_capacity(body.len() / PART);
    let mut rest = body.split_off(PART);
    parts.push(Bytes(body));
    while rest.len() > PART {
        let tail = rest.split_off(PART);
        parts.push(Bytes(rest));
        rest = tail;
    }
    (parts, Bytes(rest))
}

/// The parts received so far, by call id.
#[derive(Debug, Default)]
pub struct Parts {
    /// `None` once a call went over [`MAX_BODY`]: its ending message is refused
    open: HashMap<u64, Option<Vec<u8>>>,
}

impl Parts {
    pub fn push(&mut self, id: u64, data: Bytes) {
        if !self.open.contains_key(&id) && self.open.len() >= MAX_OPEN {
            return;
        }
        let slot = self.open.entry(id).or_insert_with(|| Some(Vec::new()));
        if let Some(body) = slot {
            if body.len() + data.0.len() > MAX_BODY {
                *slot = None;
            } else {
                body.extend_from_slice(&data.0);
            }
        }
    }

    /// The whole body of call `id`, ending with `last`; `None` when it went over [`MAX_BODY`].
    pub fn finish(&mut self, id: u64, last: Bytes) -> Option<Vec<u8>> {
        match self.open.remove(&id) {
            None if last.0.len() <= MAX_BODY => Some(last.0),
            Some(Some(mut body)) if body.len() + last.0.len() <= MAX_BODY => {
                body.extend_from_slice(&last.0);
                Some(body)
            }
            _ => None,
        }
    }
}

/// The library's way to the core, handed to [`Service::gateway`](crate::Service::gateway).
#[derive(Debug, Clone)]
pub struct Outbox(pub(crate) mpsc::UnboundedSender<FromService>);

impl Outbox {
    pub fn send(&self, message: FromGateway) -> bool {
        self.0.send(FromService::Gateway { message }).is_ok()
    }

    /// Sends `body`'s parts under `id`, then the message `end` makes of the last piece.
    pub fn send_body(&self, id: u64, body: Vec<u8>, end: impl FnOnce(Bytes) -> FromGateway) -> bool {
        let (parts, last) = split(body);
        parts.into_iter().all(|data| self.send(FromGateway::Part { id, data })) && self.send(end(last))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use crate::ToService;

    #[test]
    fn gateway_messages_nest_in_the_service_protocol_and_refuse_unknown_fields() {
        let message = ToService::Gateway {
            message: ToGateway::Configure {
                registry: true,
                allowlist: AllowlistMode::External {
                    base: PortRange {
                        first: 7777,
                        last: 7790,
                    },
                },
                ws_port: None,
            },
        };
        let text = serde_json::to_string(&message).unwrap();
        assert_eq!(
            text,
            r#"{"type":"gateway","message":{"type":"configure","registry":true,"allowlist":{"mode":"external","base":{"first":7777,"last":7790}},"ws_port":null}}"#
        );
        assert_eq!(serde_json::from_str::<ToService>(&text).unwrap(), message);
        let call = FromService::Gateway {
            message: FromGateway::NodeCall {
                id: 3,
                from: "uk-1".parse().unwrap(),
                service: "home".into(),
                body: Bytes(b"{}".to_vec()),
            },
        };
        let text = serde_json::to_string(&call).unwrap();
        assert!(text.contains(r#""body":"e30=""#), "{text}");
        assert_eq!(serde_json::from_str::<FromService>(&text).unwrap(), call);
        assert!(
            serde_json::from_str::<ToService>(
                r#"{"type":"gateway","message":{"type":"stopped","module":"stub","x":1}}"#
            )
            .is_err()
        );
        assert!(serde_json::from_str::<ToService>(r#"{"type":"gateway","message":{"type":"shell"}}"#).is_err());
    }

    #[test]
    fn a_body_goes_in_parts_and_comes_back_whole_up_to_the_limit() {
        let body: Vec<u8> = (0..MAX_BODY).map(|i| (i % 251) as u8).collect();
        let (parts, last) = split(body.clone());
        assert_eq!(parts.len(), 1);
        assert!(parts.iter().all(|p| p.0.len() == PART) && last.0.len() == PART);
        let frame = serde_json::to_vec(&ToService::Gateway {
            message: ToGateway::Part {
                id: 1,
                data: parts[0].clone(),
            },
        })
        .unwrap();
        assert!(frame.len() < module_api::frame::MAX_FRAME);
        let mut open = Parts::default();
        for part in parts {
            open.push(1, part);
        }
        assert_eq!(open.finish(1, last), Some(body));
        // One byte over is refused whole
        open.push(2, Bytes(vec![0; MAX_BODY]));
        assert_eq!(open.finish(2, Bytes(vec![0])), None);
        assert_eq!(open.finish(4, Bytes(b"small".to_vec())), Some(b"small".to_vec()));
        assert_eq!(split(Vec::new()), (Vec::new(), Bytes(Vec::new())));
    }
}
