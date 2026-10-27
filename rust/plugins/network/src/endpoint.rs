//! The answering side of a node link (DN3), independent of the transport: the gateway's `/__node` routes
//! terminate mutual TLS, find who the caller is from its certificate and hand the request here. The
//! endpoint decides whether the caller is a peer, whether the request is one a node link makes and the
//! clocks agree, and then calls the [`NodeService`] attached to it. It never sees a certificate: `from` is
//! whatever the transport verified.

use std::collections::BTreeSet;
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use module_api::NodeIdentity;
use module_api::network::NodeId;

use crate::link::{Clock, MAX_CLOCK_SKEW, MAX_MESSAGE, NodeService};
use crate::wire::{self, ErrorBody, PingAnswer, Target, code};

/// What to answer: a status, a JSON body and this node's clock (the `x-node-time` header).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub body: Vec<u8>,
    pub time_ms: i64,
}

/// What a request's head says, read before its body is.
#[derive(Debug, Clone, Copy)]
pub struct Head<'a> {
    pub method: &'a str,
    /// The request target as received: path and any query
    pub target: &'a str,
    pub content_length: Option<u64>,
    /// The `x-node-time` header, if there is exactly one
    pub time: Option<&'a str>,
}

pub struct NodeEndpoint {
    me: NodeIdentity,
    peers: BTreeSet<NodeId>,
    max_skew: Duration,
    clock: Arc<Clock>,
    service: RwLock<Option<Arc<dyn NodeService>>>,
}

impl NodeEndpoint {
    /// This node, and the peers it answers: anyone else with a cluster certificate is refused.
    pub fn new(me: NodeIdentity, peers: impl IntoIterator<Item = NodeId>) -> Self {
        Self {
            me,
            peers: peers.into_iter().collect(),
            max_skew: MAX_CLOCK_SKEW,
            clock: Arc::new(Clock::default()),
            service: RwLock::new(None),
        }
    }

    pub fn with_clock(mut self, clock: Arc<Clock>) -> Self {
        self.clock = clock;
        self
    }

    pub fn identity(&self) -> &NodeIdentity {
        &self.me
    }

    pub fn clock(&self) -> &Arc<Clock> {
        &self.clock
    }

    pub fn is_peer(&self, node: &NodeId) -> bool {
        self.peers.contains(node)
    }

    /// Answers calls with `service` from now on, replacing any earlier one. Until then a call is `no_service`
    /// (a ping still answers: the node is up).
    pub fn attach(&self, service: Arc<dyn NodeService>) {
        *self.service.write().unwrap_or_else(PoisonError::into_inner) = Some(service);
    }

    pub fn detach(&self) {
        *self.service.write().unwrap_or_else(PoisonError::into_inner) = None;
    }

    fn reply(&self, status: u16, body: Vec<u8>) -> Reply {
        Reply {
            status,
            body,
            time_ms: self.clock.now_ms(),
        }
    }

    fn refuse(&self, status: u16, error: &str) -> Reply {
        self.reply(status, ErrorBody::new(error).encode())
    }

    /// Everything the head settles: the caller is a peer, the route and method are a node link's, the clocks
    /// agree and the body isn't declared too large. `Err` is the answer to send, without reading the body.
    pub fn admit(&self, from: &NodeId, head: &Head<'_>) -> Result<Target, Reply> {
        if !self.is_peer(from) {
            return Err(self.refuse(403, code::NOT_A_PEER));
        }
        let Some(target) = wire::parse_target(head.target) else {
            return Err(self.refuse(404, code::NOT_FOUND));
        };
        let wanted = match target {
            Target::Ping => "GET",
            Target::Service(_) => "POST",
        };
        if head.method != wanted {
            return Err(self.refuse(405, code::METHOD));
        }
        let Some(theirs) = head.time.and_then(wire::parse_time) else {
            return Err(self.refuse(400, code::BAD_REQUEST));
        };
        let skew = wire::skew_ms(self.clock.now_ms(), theirs);
        if !wire::within(skew, self.max_skew) {
            tracing::warn!(peer = %from, skew_ms = skew, "node link refused: the peer's clock is off");
            return Err(self.reply(
                409,
                ErrorBody {
                    error: code::CLOCK_SKEW.to_owned(),
                    skew_ms: Some(skew),
                }
                .encode(),
            ));
        }
        if head.content_length.is_some_and(wire::too_large) {
            return Err(self.refuse(413, code::TOO_LARGE));
        }
        Ok(target)
    }

    /// Answers an admitted request. A body over [`MAX_MESSAGE`] is refused before it reaches the service.
    pub async fn serve(&self, from: &NodeId, target: Target, body: Vec<u8>) -> Reply {
        let service = match target {
            Target::Ping => {
                let answer = PingAnswer {
                    version: wire::VERSION,
                    node: self.me.node.clone(),
                    region: self.me.region.clone(),
                    time_ms: self.clock.now_ms(),
                };
                return self.reply(200, answer.encode());
            }
            Target::Service(service) => service,
        };
        if body.len() > MAX_MESSAGE {
            return self.refuse(413, code::TOO_LARGE);
        }
        let attached = self.service.read().unwrap_or_else(PoisonError::into_inner).clone();
        let Some(attached) = attached else {
            return self.refuse(503, code::NO_SERVICE);
        };
        match attached.serve(from, service, body).await {
            Ok(answer) if answer.len() > MAX_MESSAGE => {
                tracing::error!(peer = %from, service = service.as_str(), "a node service's answer is over the limit");
                self.refuse(502, code::FAILED)
            }
            Ok(answer) => self.reply(200, answer),
            Err(error) => {
                let (status, body) = wire::failure(&error);
                if status >= 500 && !matches!(error, crate::link::LinkError::NoService { .. }) {
                    tracing::warn!(peer = %from, service = service.as_str(), %error, "a node service failed");
                }
                self.reply(status, body.encode())
            }
        }
    }
}
