//! RF1's routes for this crate, in mount order.

use std::sync::Arc;

use rifts_seam::{
    BoxFuture, Caller, Component, GlobalId, Method, Mount, NodeId, Refusal, Reply, Route, RouteCall, RouteHandler,
    SeamError, StubRoutes, Ulid, Unimplemented,
};
use serde_json::Value;

use crate::service::refusal;
use crate::{IceService, RelayError};

pub const ROUTES: &[Route] = &[
    Route {
        method: Method::Patch,
        path: "/admin/v1/operations/coop-transport",
        mount: Mount::Admin,
        handler: "p2pAdminOperations.ts:UpdateCoopTransport",
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/p2p/hunts/:sessionId/transport-config",
        mount: Mount::Normal,
        handler: "p2pIce.ts:CreateIceTransportConfig",
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/p2p/hunts/:sessionId/signal",
        mount: Mount::Normal,
        handler: "p2pIce.ts:SubmitIceSignal",
    },
];

pub fn stub_routes() -> StubRoutes {
    StubRoutes {
        component: Component::Relay,
        routes: ROUTES,
    }
}

#[derive(Clone)]
pub struct RelayRoutes {
    service: Arc<dyn IceService>,
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

fn to_seam_error(error: RelayError) -> SeamError {
    match error {
        RelayError::Unimplemented(u) => SeamError::Unimplemented(u),
        RelayError::Refused(r) => SeamError::Refused(r),
        other => SeamError::Refused(Refusal {
            code: "INTERNAL".to_owned(),
            message: other.to_string(),
        }),
    }
}

impl RelayRoutes {
    pub fn new(service: Arc<dyn IceService>, node: NodeId) -> Self {
        Self { service, node }
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

    fn launcher_account(call: &RouteCall) -> Result<rifts_seam::AccountId, RelayError> {
        match &call.caller {
            Caller::Launcher(account) => Ok(account.clone()),
            _ => Err(refusal("AUTH_UNAUTHORIZED", "Sign in required.")),
        }
    }

    fn parse_object_body(call: &RouteCall) -> Result<Value, RelayError> {
        if call.body.is_empty() {
            return Ok(Value::Object(serde_json::Map::new()));
        }
        match serde_json::from_slice::<Value>(&call.body) {
            Ok(value @ Value::Object(_)) => Ok(value),
            _ => Err(refusal("P2P_TICKET_INVALID", "The request body must be a JSON object.")),
        }
    }

    fn parse_generation(body: &Value) -> Option<i64> {
        let raw = body.get("generation")?;
        raw.as_i64().or_else(|| {
            let f = raw.as_f64()?;
            (f.is_finite() && f.fract() == 0.0 && f.abs() <= 9_007_199_254_740_991.0).then_some(f as i64)
        })
    }

    async fn dispatch(&self, call: &RouteCall) -> Result<Reply, RelayError> {
        match call.route.handler {
            "p2pAdminOperations.ts:UpdateCoopTransport" => {
                let key_id = match &call.caller {
                    Caller::Admin { key_id } => key_id.clone(),
                    _ => "admin".to_owned(),
                };
                let body = serde_json::from_slice::<Value>(&call.body)
                    .unwrap_or_else(|_| Value::Object(serde_json::Map::new()));
                self.service.set_coop_transport(key_id, body).await
            }
            "p2pIce.ts:CreateIceTransportConfig" => {
                let account = Self::launcher_account(call)?;
                let session = self.session_id(Self::param(call, "sessionId"));
                let body = Self::parse_object_body(call)?;
                let generation = Self::parse_generation(&body);
                let json = self.service.transport_config(account, session, generation).await?;
                Ok(Reply {
                    status: 200,
                    body: json,
                })
            }
            "p2pIce.ts:SubmitIceSignal" => {
                let account = Self::launcher_account(call)?;
                let session = self.session_id(Self::param(call, "sessionId"));
                let body = Self::parse_object_body(call)?;
                let json = self.service.submit_signal(account, session, body).await?;
                Ok(Reply {
                    status: 200,
                    body: json,
                })
            }
            _ => Err(RelayError::Unimplemented(Unimplemented::new(
                Component::Relay,
                call.route.handler,
            ))),
        }
    }
}

impl RouteHandler for RelayRoutes {
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
            .filter(|route| route["component"] == "rifts-relay")
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
            let expected = Unimplemented::new(Component::Relay, route.handler);
            assert_eq!(answer, Err(SeamError::Unimplemented(expected)), "{}", route.path);
        }
    }
}
