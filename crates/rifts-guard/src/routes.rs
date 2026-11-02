// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Guard's six routes, in the order the module mounts them.

use std::sync::Arc;

use rifts_seam::{
    BoxFuture, Caller, CapabilityScope, Component, HOST_CAPABILITY_HEADER, Json, Method, Mount, Reply, Route,
    RouteCall, RouteHandler, RuntimeAuth, SeamError, StubRoutes,
};

use crate::errors::{GuardError, INTERNAL_CODE, INTERNAL_MESSAGE};
use crate::evidence::TOO_LARGE;
use crate::json::{BodyError, parse_admin_body, parse_p2p_body, stringified_len};
use crate::operations::Operations;
use crate::{Guard, TrafficDecision, capability_shaped, sha256_hex};

const EVENT_BATCHES: &str = "guard.event_batches";
const COMPLETION: &str = "guard.completion";
const REWARD_SELF: &str = "guard.reward_self";
const SHADOW: &str = "guard.shadow_comparisons";
const REWARD_ROLLOUT: &str = "guard.reward_rollout";
const ALERT_ACK: &str = "guard.alert_ack";

/// `Early` routes come before the module's JSON parser: Guard bounds and parses their bodies itself.
pub const ROUTES: &[Route] = &[
    Route {
        method: Method::Post,
        path: "/rustic/v1/hunts/:sessionId/event-batches",
        mount: Mount::Early,
        handler: EVENT_BATCHES,
    },
    Route {
        method: Method::Post,
        path: "/rustic/v1/hunts/:sessionId/completion",
        mount: Mount::Early,
        handler: COMPLETION,
    },
    Route {
        method: Method::Get,
        path: "/rustic/v1/hunts/:sessionId/reward/self",
        mount: Mount::Early,
        handler: REWARD_SELF,
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
        handler: REWARD_ROLLOUT,
    },
    Route {
        method: Method::Patch,
        path: "/admin/v1/operations/alerts/:alertId/ack",
        mount: Mount::Admin,
        handler: ALERT_ACK,
    },
];

pub fn stub_routes() -> StubRoutes {
    StubRoutes {
        component: Component::Guard,
        routes: ROUTES,
    }
}

fn unauthorized() -> GuardError {
    GuardError::refused("AUTH_UNAUTHORIZED", "Authentication is required.")
}

fn param<'a>(call: &'a RouteCall, name: &str) -> &'a str {
    call.params
        .iter()
        .find(|(key, _)| key == name)
        .map_or("", |(_, value)| value.as_str())
}

#[derive(Clone)]
pub struct GuardRoutes {
    guard: Arc<Guard>,
    has_store: bool,
}

impl GuardRoutes {
    pub fn new(guard: Arc<Guard>) -> Self {
        Self::with_store(guard, true)
    }

    /// Without a store every route answers `Unimplemented` under its handler id.
    pub fn with_store(guard: Arc<Guard>, has_store: bool) -> Self {
        Self { guard, has_store }
    }

    /// A capability header that names a live capability of this hunt with this scope.
    fn host_runtime(&self, call: &RouteCall, scope: CapabilityScope) -> Option<RuntimeAuth> {
        let token = call
            .request
            .header(HOST_CAPABILITY_HEADER)
            .filter(|t| capability_shaped(t))?;
        let digest = sha256_hex(token.as_bytes());
        self.guard
            .hunts()
            .resolve_capability(&digest, param(call, "sessionId"), scope, self.guard.now())
            .ok()
            .flatten()
    }

    async fn hunts(&self, call: RouteCall) -> Result<Reply, GuardError> {
        let body = match parse_p2p_body(&call.request.headers, &call.body) {
            Ok(body) => body,
            Err(BodyError::TooLarge) => return Err(GuardError::refused("GUARD_BATCH_TOO_LARGE", TOO_LARGE)),
            Err(BodyError::Invalid) => {
                return Err(GuardError::refused(
                    "GUARD_BATCH_INVALID",
                    "The Guard request body is not valid JSON.",
                ));
            }
        };
        let handler = call.route.handler;
        if handler == REWARD_SELF {
            let Caller::Launcher(account) = &call.caller else {
                return Err(unauthorized());
            };
            return self.guard.self_reward(account, param(&call, "sessionId")).map(ok);
        }
        if handler == SHADOW {
            return Err(GuardError::refused(
                "HUNT_EVIDENCE_NOT_COLLECTED",
                "This server does not collect hunt evidence.",
            ));
        }
        let scope = if handler == EVENT_BATCHES {
            CapabilityScope::EventsWrite
        } else {
            CapabilityScope::CompleteRequest
        };
        let runtime = match &call.caller {
            Caller::Anonymous => self.host_runtime(&call, scope),
            Caller::HostRuntime(runtime) => Some(runtime.clone()),
            _ => None,
        };
        let Some(runtime) = runtime else {
            return Err(GuardError::refused(
                "P2P_CAPABILITY_INVALID",
                "A valid hunt host capability is required.",
            ));
        };
        let bytes = stringified_len(body.as_ref()) as u64;
        let Some(_permit) = self.guard.request_budget().try_acquire(bytes) else {
            return Err(GuardError::refused(
                "GUARD_REQUEST_BUDGET_EXHAUSTED",
                "Guard request capacity is currently exhausted; retry shortly.",
            ));
        };
        match self.guard.charge_now(&runtime.host_account, &runtime.session, bytes) {
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
        let answer = if handler == EVENT_BATCHES {
            self.guard.submit_parsed(&runtime, body.as_ref())
        } else {
            self.guard.complete_parsed(&runtime, body.as_ref())
        };
        answer.map(ok)
    }

    async fn admin(&self, call: RouteCall) -> Result<Reply, GuardError> {
        let Caller::Admin { key_id } = &call.caller else {
            return Err(unauthorized());
        };
        let body = parse_admin_body(&call.body);
        let ip = call
            .request
            .remote
            .map_or_else(|| "unknown".to_owned(), |ip| ip.to_string());
        let operations: Operations<'_> = self.guard.operations();
        if call.route.handler == REWARD_ROLLOUT {
            operations.set_reward_rollout(key_id, body.as_ref(), &ip).await
        } else {
            operations
                .acknowledge_alert(key_id, param(&call, "alertId"), body.as_ref(), &ip)
                .await
        }
    }

    async fn answer(&self, call: RouteCall) -> Result<Reply, SeamError> {
        if !self.has_store {
            return Err(SeamError::unimplemented(Component::Guard, call.route.handler));
        }
        let request_id = call.request.request_id.clone();
        let result = if call.route.mount == Mount::Admin {
            self.admin(call).await
        } else {
            self.hunts(call).await
        };
        result.map_err(|error| match error {
            GuardError::Internal(detail) => {
                let id = if request_id.is_empty() {
                    "missing"
                } else {
                    request_id.as_str()
                };
                self.guard
                    .log()
                    .error(&format!("Unhandled P2P route error requestId={id}: {detail}"));
                SeamError::Refused(rifts_seam::Refusal {
                    code: INTERNAL_CODE.to_owned(),
                    message: INTERNAL_MESSAGE.to_owned(),
                })
            }
            other => SeamError::from(other),
        })
    }
}

fn ok(body: Json) -> Reply {
    Reply { status: 200, body }
}

impl RouteHandler for GuardRoutes {
    fn routes(&self) -> &'static [Route] {
        ROUTES
    }

    fn handle(&self, call: RouteCall) -> BoxFuture<'_, Result<Reply, SeamError>> {
        Box::pin(self.answer(call))
    }
}
