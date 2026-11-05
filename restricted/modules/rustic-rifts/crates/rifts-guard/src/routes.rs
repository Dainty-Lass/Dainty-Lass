//! RF1's routes for this crate, in mount order, and [`GuardRoutes`], which answers them as `p2pHunts.ts` and
//! `p2pAdminOperations.ts` do. Launcher Guard and the Guard manifests stay isles' (crate docs).

use std::sync::Arc;

use rifts_seam::{
    BoxFuture, Caller, CapabilityScope, Component, HOST_CAPABILITY_HEADER, Json, Method, Mount, Refusal, Reply, Route,
    RouteCall, RouteHandler, SeamError, StubRoutes, Unimplemented,
};
use serde_json::Value;

use crate::errors::{GuardError, to_seam};
use crate::json::{BodyError, parse_admin_body, parse_p2p_body, stringified_len};
use crate::traffic::HuntOperation;
use crate::{Guard, GuardOperations, TrafficDecision};

const SUBMIT: &str = "antiCheat.ts:hunts.submitEvidence";
const COMPLETE: &str = "antiCheat.ts:hunts.complete";
const SELF_REWARD: &str = "p2pHuntStatus.ts:GetSelfHuntReward";
const SHADOW: &str = "antiCheat.ts:hunts.recordShadowComparison";
const ROLLOUT: &str = "p2pAdminOperations.ts:UpdateRewardRollout";
const ACK: &str = "p2pAdminOperations.ts:AcknowledgeOperationsAlert";

pub const ROUTES: &[Route] = &[
    Route {
        method: Method::Post,
        path: "/mystic/v1/hunts/:sessionId/event-batches",
        mount: Mount::Early,
        handler: SUBMIT,
    },
    Route {
        method: Method::Post,
        path: "/mystic/v1/hunts/:sessionId/completion",
        mount: Mount::Early,
        handler: COMPLETE,
    },
    Route {
        method: Method::Get,
        path: "/mystic/v1/hunts/:sessionId/reward/self",
        mount: Mount::Early,
        handler: SELF_REWARD,
    },
    Route {
        method: Method::Post,
        path: "/internal/v1/hunt-guard/shadow-comparisons",
        mount: Mount::Early,
        handler: SHADOW,
    },
    Route {
        method: Method::Patch,
        path: "/admin/v1/operations/reward-rollout",
        mount: Mount::Admin,
        handler: ROLLOUT,
    },
    Route {
        method: Method::Patch,
        path: "/admin/v1/operations/alerts/:alertId/ack",
        mount: Mount::Admin,
        handler: ACK,
    },
];

pub fn stub_routes() -> StubRoutes {
    StubRoutes {
        component: Component::Guard,
        routes: ROUTES,
    }
}

/// The six routes on a [`Guard`]. Refusals come back as [`SeamError::Refused`]: isles writes the launcher envelope
/// with the status from [`crate::ERROR_STATUSES`]; the admin routes' own shapes come back as replies.
#[derive(Clone)]
pub struct GuardRoutes {
    guard: Arc<Guard>,
    has_store: bool,
}

impl GuardRoutes {
    pub fn new(guard: Arc<Guard>) -> Self {
        Self { guard, has_store: true }
    }

    pub fn with_store(guard: Arc<Guard>, has_store: bool) -> Self {
        Self { guard, has_store }
    }

    fn param<'a>(call: &'a RouteCall, name: &str) -> &'a str {
        call.params
            .iter()
            .find(|(key, _)| key == name)
            .map_or("", |(_, value)| value.as_str())
    }

    /// `p2pHuntsRouter`: its own parser, then `Handle()` (the Guard budget and traffic windows for a capability call).
    fn hunts_route(&self, call: &RouteCall) -> Result<Reply, GuardError> {
        let body = match parse_p2p_body(&call.request.headers, &call.body) {
            Ok(body) => body,
            Err(BodyError::TooLarge) => {
                return Err(GuardError::refused(
                    "GUARD_BATCH_TOO_LARGE",
                    "The Guard request body exceeds its bounded limit.",
                ));
            }
            Err(BodyError::Invalid) => {
                return Err(GuardError::refused(
                    "GUARD_BATCH_INVALID",
                    "The Guard request body is not valid JSON.",
                ));
            }
        };
        let resolved_caller = match (call.route.handler, &call.caller) {
            (SUBMIT, Caller::Anonymous) => self
                .guard
                .resolve_route_capability(
                    call.request.header(HOST_CAPABILITY_HEADER),
                    Self::param(call, "sessionId"),
                    CapabilityScope::EventsWrite,
                )
                .map_or(Caller::Anonymous, Caller::HostRuntime),
            (COMPLETE, Caller::Anonymous) => self
                .guard
                .resolve_route_capability(
                    call.request.header(HOST_CAPABILITY_HEADER),
                    Self::param(call, "sessionId"),
                    CapabilityScope::CompleteRequest,
                )
                .map_or(Caller::Anonymous, Caller::HostRuntime),
            (_, caller) => caller.clone(),
        };
        let answer = match (call.route.handler, &resolved_caller) {
            (SUBMIT | COMPLETE, Caller::HostRuntime(runtime)) => {
                let bytes = stringified_len(body.as_ref());
                let Some(_permit) = self.guard.request_budget().try_acquire(bytes) else {
                    return Err(GuardError::refused(
                        "GUARD_REQUEST_BUDGET_EXHAUSTED",
                        "Guard request capacity is currently exhausted; retry shortly.",
                    ));
                };
                if HuntOperation::of(call.route.path).counts_against_guard_traffic() {
                    match self.guard.charge_now(&runtime.host_account, &runtime.session, bytes)? {
                        TrafficDecision::Accepted => {}
                        TrafficDecision::RateLimited => {
                            return Err(GuardError::refused(
                                "GUARD_TRAFFIC_RATE_LIMITED",
                                "Guard traffic exceeded the bounded host or hunt-session budget; retry shortly.",
                            ));
                        }
                        TrafficDecision::Unavailable => {
                            return Err(GuardError::refused(
                                "GUARD_TRAFFIC_UNAVAILABLE",
                                "Guard traffic capacity is temporarily unavailable; retry shortly.",
                            ));
                        }
                    }
                }
                if call.route.handler == SUBMIT {
                    self.guard.submit_parsed(runtime, body.as_ref())?
                } else {
                    self.guard.complete_parsed(runtime, body.as_ref())?
                }
            }
            (SUBMIT | COMPLETE, _) => {
                return Err(GuardError::refused(
                    "P2P_CAPABILITY_INVALID",
                    "A valid hunt host capability is required.",
                ));
            }
            (SELF_REWARD, Caller::Launcher(account)) => {
                self.guard.self_reward(account, Self::param(call, "sessionId"))?
            }
            (SELF_REWARD, _) => {
                return Err(GuardError::refused("AUTH_UNAUTHORIZED", "Authentication is required."));
            }
            // The private module's; without it Node refuses before reading the key
            (SHADOW, _) => return Err(crate::errors::not_collected()),
            (handler, _) => return Err(Unimplemented::new(Component::Guard, handler).into()),
        };
        Ok(Reply {
            status: 200,
            body: answer,
        })
    }

    async fn admin_route(&self, call: &RouteCall) -> Result<Reply, GuardError> {
        let Caller::Admin { key_id } = &call.caller else {
            return Err(GuardError::refused("AUTH_UNAUTHORIZED", "Authentication is required."));
        };
        let body: Option<Value> = parse_admin_body(&call.body);
        let ip = call
            .request
            .remote
            .map_or_else(|| "unknown".to_owned(), |remote| remote.to_string());
        if call.route.handler == ROLLOUT {
            self.guard.set_reward_rollout(key_id.clone(), body, ip).await
        } else {
            let alert_id = Self::param(call, "alertId").to_owned();
            self.guard.acknowledge_alert(key_id.clone(), alert_id, body, ip).await
        }
    }

    async fn answer(&self, call: RouteCall) -> Result<Reply, SeamError> {
        if !self.has_store {
            return Err(SeamError::Unimplemented(Unimplemented::new(
                Component::Guard,
                call.route.handler,
            )));
        }
        let result = match call.route.handler {
            SUBMIT | COMPLETE | SELF_REWARD | SHADOW => self.hunts_route(&call),
            ROLLOUT | ACK => self.admin_route(&call).await,
            handler => Err(Unimplemented::new(Component::Guard, handler).into()),
        };
        result.map_err(|error| {
            if let GuardError::Internal(detail) = &error {
                let request_id = if call.request.request_id.is_empty() {
                    "missing"
                } else {
                    &call.request.request_id
                };
                self.guard
                    .log
                    .error(&format!("Unhandled P2P route error requestId={request_id}: {detail}"));
            }
            to_seam(error)
        })
    }
}

impl RouteHandler for GuardRoutes {
    fn routes(&self) -> &'static [Route] {
        ROUTES
    }

    fn handle(&self, call: RouteCall) -> BoxFuture<'_, Result<Reply, SeamError>> {
        Box::pin(self.answer(call))
    }
}

/// The launcher envelope isles writes for a refusal, for tests and for isles' own (`SendLauncherError`, with the
/// request id in place of Node's random one).
pub fn envelope(refusal: &Refusal, request_id: &str) -> Reply {
    Reply {
        status: crate::errors::status_of(&refusal.code),
        body: Json(format!(
            r#"{{"error":{{"code":{},"message":{},"requestId":{}}}}}"#,
            crate::json::js_string(&refusal.code),
            crate::json::js_string(&refusal.message),
            crate::json::js_string(request_id)
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rifts_seam::{RequestInfo, poll_once};

    const ISLES_KEEPS: &[&str] = &[
        "routes/launcherGuard.ts",
        "routes/admin.ts",
        "routes/launcherUpdates.ts",
    ];

    #[test]
    fn routes_match_the_inventory() {
        let inventory: serde_json::Value = serde_json::from_str(include_str!("../../../docs/inventory.json")).unwrap();
        let expected: Vec<(&str, &str)> = inventory["routes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|route| {
                route["component"] == "rifts-guard"
                    && !ISLES_KEEPS
                        .iter()
                        .any(|router| route["router"].as_str().unwrap().ends_with(router))
            })
            .map(|route| (route["method"].as_str().unwrap(), route["path"].as_str().unwrap()))
            .collect();
        let actual: Vec<(&str, &str)> = ROUTES.iter().map(|route| (route.method.as_str(), route.path)).collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn every_stub_route_answers_unimplemented() {
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
            let expected = Unimplemented::new(Component::Guard, route.handler);
            assert_eq!(answer, Err(SeamError::Unimplemented(expected)), "{}", route.path);
        }
    }

    #[test]
    fn guard_routes_without_store_answers_unimplemented() {
        let core: Arc<dyn rifts_seam::IslesCore> = Arc::new(rifts_seam::NoCore);
        let guard = Arc::new(Guard::new(crate::GuardParts {
            config: crate::GuardConfig::default(),
            store: Arc::new(crate::StubGuardStore),
            hunts: Arc::new(crate::NoHuntSessions),
            operations: Arc::new(crate::CoreOperationsPolicy::new(Arc::clone(&core))),
            core,
            clock: Arc::new(crate::ports::SystemClock::default()),
            log: Arc::new(crate::ports::MemoryLog::default()),
        }));
        let handler = GuardRoutes::with_store(guard, false);
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
            let expected = Unimplemented::new(Component::Guard, route.handler);
            assert_eq!(answer, Err(SeamError::Unimplemented(expected)), "{}", route.path);
        }
    }

    #[test]
    fn the_envelope_follows_send_launcher_error() {
        let refusal = Refusal {
            code: "HUNT_EVIDENCE_NOT_COLLECTED".to_owned(),
            message: "This server does not collect hunt evidence.".to_owned(),
        };
        let reply = envelope(&refusal, "r-1");
        assert_eq!(reply.status, 404);
        assert_eq!(
            reply.body.0,
            concat!(
                r#"{"error":{"code":"HUNT_EVIDENCE_NOT_COLLECTED","#,
                r#""message":"This server does not collect hunt evidence.","requestId":"r-1"}}"#
            )
        );
    }
}
