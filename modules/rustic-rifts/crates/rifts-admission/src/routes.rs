//! RF1's routes for this crate, in mount order.

use std::sync::Arc;

use rifts_seam::{
    BoxFuture, Caller, CapabilityScope, Component, GlobalId, HOST_CAPABILITY_HEADER, Json, Method, Mount, NodeId,
    Refusal, Reply, Route, RouteCall, RouteHandler, SeamError, StubRoutes, Ulid, Unimplemented,
};
use serde_json::Value;

use crate::service::{ConsumeTicket, HostCapability, refused};
use crate::{AdmissionError, AdmissionService};

pub const ROUTES: &[Route] = &[
    Route {
        method: Method::Post,
        path: "/mystic/v1/hunts/:sessionId/tickets/self",
        mount: Mount::Early,
        handler: "p2pAdmission.ts:IssueSelfAdmissionTicket",
    },
    Route {
        method: Method::Post,
        path: "/mystic/v1/hunts/:sessionId/tickets/consume",
        mount: Mount::Early,
        handler: "p2pAdmission.ts:ConsumeAdmissionTicket",
    },
    Route {
        method: Method::Post,
        path: "/mystic/v1/hunts/:sessionId/peers/:admissionId/disconnect",
        mount: Mount::Early,
        handler: "p2pAdmission.ts:RecordPeerDisconnected",
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/p2p/hunts/:sessionId/join-bootstrap",
        mount: Mount::Normal,
        handler: "p2pLauncher.ts:CreateJoinBootstrap",
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/p2p/hunts/:sessionId/peers",
        mount: Mount::Normal,
        handler: "realmHubAdmission.ts:ListHubPeers",
    },
];

pub fn stub_routes() -> StubRoutes {
    StubRoutes {
        component: Component::Admission,
        routes: ROUTES,
    }
}

/// Resolves an `x-mystic-host-capability` header for routes where the HTTP layer leaves `Caller::Anonymous`.
pub type CapabilityResolver = Arc<
    dyn Fn(Option<&str>, &str, CapabilityScope) -> BoxFuture<'static, Result<HostCapability, AdmissionError>>
        + Send
        + Sync,
>;

#[derive(Clone)]
pub struct AdmissionRoutes {
    service: Arc<dyn AdmissionService>,
    resolver: Option<CapabilityResolver>,
    node: NodeId,
}

fn parse_uuid_bits(wire: &str) -> Option<u128> {
    let bytes = wire.as_bytes();
    if bytes.len() != 36 {
        return None;
    }
    let mut hex = String::with_capacity(32);
    for (index, &byte) in bytes.iter().enumerate() {
        if matches!(index, 8 | 13 | 18 | 23) {
            if byte != b'-' {
                return None;
            }
        } else if byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte) {
            hex.push(char::from(byte));
        } else {
            return None;
        }
    }
    u128::from_str_radix(&hex, 16).ok()
}

fn to_seam_error(error: AdmissionError) -> SeamError {
    match error {
        AdmissionError::Unimplemented(u) => SeamError::Unimplemented(u),
        AdmissionError::Refused(r) => SeamError::Refused(r),
        other => SeamError::Refused(Refusal {
            code: "INTERNAL".to_owned(),
            message: other.to_string(),
        }),
    }
}

impl AdmissionRoutes {
    pub fn new(service: Arc<dyn AdmissionService>, resolver: Option<CapabilityResolver>, node: NodeId) -> Self {
        Self {
            service,
            resolver,
            node,
        }
    }

    fn param<'a>(call: &'a RouteCall, name: &str) -> &'a str {
        call.params
            .iter()
            .find(|(key, _)| key == name)
            .map_or("", |(_, value)| value.as_str())
    }

    fn session_id(&self, raw: &str) -> rifts_seam::SessionId {
        let stripped = raw.split_once(':').map_or(raw, |(_, rest)| rest);
        if let Ok(session) = stripped.parse::<rifts_seam::SessionId>() {
            return session;
        }
        let bits = parse_uuid_bits(stripped).unwrap_or(0);
        rifts_seam::SessionId(GlobalId {
            ulid: Ulid(bits),
            node: self.node.clone(),
        })
    }

    fn launcher_account(call: &RouteCall) -> Result<rifts_seam::AccountId, AdmissionError> {
        match &call.caller {
            Caller::Launcher(account) => Ok(account.clone()),
            _ => Err(refused("AUTH_UNAUTHORIZED", "Sign in required.")),
        }
    }

    async fn host_capability(
        &self,
        call: &RouteCall,
        scope: CapabilityScope,
    ) -> Result<HostCapability, AdmissionError> {
        if let Caller::HostRuntime(runtime) = &call.caller
            && let Some(cap) = HostCapability::from_runtime(runtime)
        {
            return Ok(cap);
        }
        let Some(resolver) = &self.resolver else {
            return Err(refused("P2P_CAPABILITY_REQUIRED", "Missing host capability header."));
        };
        let session_id = Self::param(call, "sessionId");
        resolver(call.request.header(HOST_CAPABILITY_HEADER), session_id, scope).await
    }

    fn parse_object_body(call: &RouteCall) -> Result<Value, AdmissionError> {
        if call.body.is_empty() {
            return Ok(Value::Object(serde_json::Map::new()));
        }
        match serde_json::from_slice::<Value>(&call.body) {
            Ok(value @ Value::Object(_)) => Ok(value),
            _ => Err(refused("P2P_TICKET_INVALID", "The request body must be a JSON object.")),
        }
    }

    fn parse_generation(body: &Value) -> Option<i64> {
        let raw = body.get("generation")?;
        raw.as_i64().or_else(|| {
            let f = raw.as_f64()?;
            (f.is_finite() && f.fract() == 0.0 && f.abs() <= 9_007_199_254_740_991.0).then_some(f as i64)
        })
    }

    async fn dispatch(&self, call: &RouteCall) -> Result<Reply, AdmissionError> {
        match call.route.handler {
            "p2pAdmission.ts:IssueSelfAdmissionTicket" => {
                let account = Self::launcher_account(call)?;
                let session = self.session_id(Self::param(call, "sessionId"));
                let issued = self.service.issue_self_ticket(account, session).await?;
                Ok(Reply {
                    status: 200,
                    body: issued.to_json(),
                })
            }
            "p2pAdmission.ts:ConsumeAdmissionTicket" => {
                let capability = self.host_capability(call, CapabilityScope::PeerAdmitted).await?;
                let body = Self::parse_object_body(call)?;
                let req_id = if call.request.request_id.is_empty() {
                    "missing"
                } else {
                    &call.request.request_id
                };
                let request = ConsumeTicket::from_body(&body, req_id)?;
                let consumed = self.service.consume_ticket(capability, request).await?;
                Ok(Reply {
                    status: 200,
                    body: consumed.to_json(),
                })
            }
            "p2pAdmission.ts:RecordPeerDisconnected" => {
                let capability = self.host_capability(call, CapabilityScope::PeerAdmitted).await?;
                let admission_id = Self::param(call, "admissionId").to_owned();
                let disconnected = self
                    .service
                    .record_peer_disconnected(capability, admission_id, None)
                    .await?;
                Ok(Reply {
                    status: 200,
                    body: Json(format!("{{\"disconnected\":{disconnected}}}")),
                })
            }
            "p2pLauncher.ts:CreateJoinBootstrap" => {
                let account = Self::launcher_account(call)?;
                let session = self.session_id(Self::param(call, "sessionId"));
                let bootstrap = self.service.create_join_bootstrap(account, session).await?;
                Ok(Reply {
                    status: 200,
                    body: bootstrap.to_json(),
                })
            }
            "realmHubAdmission.ts:ListHubPeers" => {
                let account = Self::launcher_account(call)?;
                let session = self.session_id(Self::param(call, "sessionId"));
                let body = Self::parse_object_body(call)?;
                let generation = Self::parse_generation(&body);
                let peers = self.service.list_hub_peers(account, session, generation).await?;
                Ok(Reply {
                    status: 200,
                    body: peers.to_json(),
                })
            }
            _ => Err(AdmissionError::Unimplemented(Unimplemented::new(
                Component::Admission,
                call.route.handler,
            ))),
        }
    }
}

impl RouteHandler for AdmissionRoutes {
    fn routes(&self) -> &'static [Route] {
        ROUTES
    }

    fn handle(&self, call: RouteCall) -> BoxFuture<'_, Result<Reply, SeamError>> {
        Box::pin(async move { self.dispatch(&call).await.map_err(to_seam_error) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rifts_seam::{Caller, RequestInfo, RouteCall, RouteHandler, SeamError, Unimplemented, poll_once};

    #[test]
    fn routes_match_the_inventory() {
        let inventory: serde_json::Value = serde_json::from_str(include_str!("../../../docs/inventory.json")).unwrap();
        let expected: Vec<(&str, &str)> = inventory["routes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|route| route["component"] == "rifts-admission")
            .map(|route| (route["method"].as_str().unwrap(), route["path"].as_str().unwrap()))
            .collect();
        let actual: Vec<(&str, &str)> = ROUTES.iter().map(|route| (route.method.as_str(), route.path)).collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn every_route_answers_unimplemented() {
        let handler = stub_routes();
        for route in ROUTES {
            let call = RouteCall {
                route: *route,
                params: Vec::new(),
                query: String::new(),
                request: RequestInfo::default(),
                caller: Caller::Anonymous,
                body: Vec::new(),
            };
            let answer = poll_once(handler.handle(call)).unwrap();
            let expected = Unimplemented::new(Component::Admission, route.handler);
            assert_eq!(answer, Err(SeamError::Unimplemented(expected)), "{}", route.path);
        }
    }
}
