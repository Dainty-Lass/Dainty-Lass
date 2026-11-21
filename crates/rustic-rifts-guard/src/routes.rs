// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Guard's six routes, in the order the module mounts them.

use std::sync::Arc;

use rustic_rifts_seam::{
    BoxFuture, Caller, CapabilityScope, Component, HOST_CAPABILITY_HEADER, Json, Method, Mount, Reply, Route,
    RouteCall, RouteHandler, RuntimeAuth, SeamError, StubRoutes,
};

use crate::{
    Guard, TrafficDecision, capability_shaped,
    errors::{GuardError, INTERNAL_CODE, INTERNAL_MESSAGE},
    evidence::TOO_LARGE,
    json::{BodyError, parse_admin_body, parse_p2p_body, stringified_len},
    sha256_hex,
};

/// The handler ids, which are also the operation names a store-less Guard reports.
mod handler {
    pub const EVENT_BATCHES: &str = "guard.event_batches";
    pub const COMPLETION: &str = "guard.completion";
    pub const REWARD_SELF: &str = "guard.reward_self";
    pub const SHADOW: &str = "guard.shadow_comparisons";
    pub const REWARD_ROLLOUT: &str = "guard.reward_rollout";
    pub const ALERT_ACK: &str = "guard.alert_ack";
}

const fn route(method: Method, mount: Mount, path: &'static str, handler: &'static str) -> Route {
    Route {
        method,
        path,
        mount,
        handler,
    }
}

/// `Early` routes come before the module's JSON parser: Guard bounds and parses their bodies itself.
pub const ROUTES: &[Route] = &[
    route(
        Method::Post,
        Mount::Early,
        "/rustic/v1/hunts/:sessionId/event-batches",
        handler::EVENT_BATCHES,
    ),
    route(
        Method::Post,
        Mount::Early,
        "/rustic/v1/hunts/:sessionId/completion",
        handler::COMPLETION,
    ),
    route(
        Method::Get,
        Mount::Early,
        "/rustic/v1/hunts/:sessionId/reward/self",
        handler::REWARD_SELF,
    ),
    route(
        Method::Post,
        Mount::Early,
        "/internal/v1/hunt-guard/shadow-comparisons",
        handler::SHADOW,
    ),
    route(
        Method::Patch,
        Mount::Admin,
        "/admin/v1/operations/reward-rollout",
        handler::REWARD_ROLLOUT,
    ),
    route(
        Method::Patch,
        Mount::Admin,
        "/admin/v1/operations/alerts/:alertId/ack",
        handler::ALERT_ACK,
    ),
];

/// Every Guard route under its handler id, for a build that answers them `Unimplemented`.
pub fn stub_routes() -> StubRoutes {
    let routes = ROUTES;
    StubRoutes {
        routes,
        component: Component::Guard,
    }
}

/// The refusals the hunt routes give before Guard itself is asked.
#[derive(Clone, Copy)]
enum Gate {
    TooLarge,
    NotJson,
    NoAccount,
    NotCollected,
    NoCapability,
    BudgetExhausted,
    RateLimited,
    TrafficUnavailable,
}

impl Gate {
    fn refusal(self) -> GuardError {
        let (code, text) = match self {
            Self::TooLarge => ("GUARD_BATCH_TOO_LARGE", TOO_LARGE),
            Self::NotJson => ("GUARD_BATCH_INVALID", "The Guard request body is not valid JSON."),
            Self::NoAccount => ("AUTH_UNAUTHORIZED", "Authentication is required."),
            Self::NotCollected => (
                "HUNT_EVIDENCE_NOT_COLLECTED",
                "This server does not collect hunt evidence.",
            ),
            Self::NoCapability => ("P2P_CAPABILITY_INVALID", "A valid hunt host capability is required."),
            Self::BudgetExhausted => (
                "GUARD_REQUEST_BUDGET_EXHAUSTED",
                "Guard request capacity is currently exhausted; retry shortly.",
            ),
            Self::RateLimited => (
                "GUARD_TRAFFIC_RATE_LIMITED",
                "Guard traffic exceeded the bounded host or hunt-session budget; retry shortly.",
            ),
            Self::TrafficUnavailable => (
                "GUARD_TRAFFIC_UNAVAILABLE",
                "Guard traffic capacity is temporarily unavailable; retry shortly.",
            ),
        };
        GuardError::refused(code, text)
    }
}

/// An Express param by name; a missing one reads as empty.
fn param<'a>(call: &'a RouteCall, name: &str) -> &'a str {
    let found = call.params.iter().find(|pair| pair.0 == name);
    found.map_or("", |pair| pair.1.as_str())
}

fn ok(body: Json) -> Reply {
    Reply { status: 200, body }
}

/// Guard's HTTP routes: the hunt evidence routes and the two admin operations routes.
#[derive(Clone)]
pub struct GuardRoutes {
    guard: Arc<Guard>,
    has_store: bool,
}

impl GuardRoutes {
    /// Routes over a Guard backed by a store.
    pub fn new(guard: Arc<Guard>) -> Self {
        Self::with_store(guard, true)
    }

    /// Without a store every route answers `Unimplemented` under its handler id.
    pub fn with_store(guard: Arc<Guard>, has_store: bool) -> Self {
        Self { guard, has_store }
    }

    /// A capability header that names a live capability of this hunt with this scope.
    fn host_runtime(&self, call: &RouteCall, scope: CapabilityScope) -> Option<RuntimeAuth> {
        let token = call.request.header(HOST_CAPABILITY_HEADER)?;
        if !capability_shaped(token) {
            return None;
        }
        let hunts = self.guard.hunts();
        let found = hunts.resolve_capability(
            &sha256_hex(token.as_bytes()),
            param(call, "sessionId"),
            scope,
            self.guard.now(),
        );
        found.unwrap_or(None)
    }

    /// The runtime a hunt route acts for: the module's, or the capability header's when the module saw none.
    fn runtime_for(&self, call: &RouteCall, scope: CapabilityScope) -> Option<RuntimeAuth> {
        match &call.caller {
            Caller::HostRuntime(runtime) => Some(runtime.clone()),
            Caller::Anonymous => self.host_runtime(call, scope),
            _ => None,
        }
    }

    async fn hunts(&self, call: RouteCall) -> Result<Reply, GuardError> {
        let body = parse_p2p_body(&call.request.headers, &call.body).map_err(|error| match error {
            BodyError::TooLarge => Gate::TooLarge.refusal(),
            BodyError::Invalid => Gate::NotJson.refusal(),
        })?;
        let events = match call.route.handler {
            handler::REWARD_SELF => {
                let Caller::Launcher(account) = &call.caller else {
                    return Err(Gate::NoAccount.refusal());
                };
                return self.guard.self_reward(account, param(&call, "sessionId")).map(ok);
            }
            handler::SHADOW => return Err(Gate::NotCollected.refusal()),
            handler::EVENT_BATCHES => true,
            _ => false,
        };
        let scope = if events {
            CapabilityScope::EventsWrite
        } else {
            CapabilityScope::CompleteRequest
        };
        let runtime = self
            .runtime_for(&call, scope)
            .ok_or_else(|| Gate::NoCapability.refusal())?;

        let bytes = stringified_len(body.as_ref()) as u64;
        let _permit = self
            .guard
            .request_budget()
            .try_acquire(bytes)
            .ok_or_else(|| Gate::BudgetExhausted.refusal())?;
        let traffic = self.guard.charge_now(&runtime.host_account, &runtime.session, bytes);
        if let Some(gate) = match traffic {
            TrafficDecision::Accepted => None,
            TrafficDecision::RateLimited => Some(Gate::RateLimited),
            TrafficDecision::Unavailable => Some(Gate::TrafficUnavailable),
        } {
            return Err(gate.refusal());
        }

        let answer = if events {
            self.guard.submit_parsed(&runtime, body.as_ref())
        } else {
            self.guard.complete_parsed(&runtime, body.as_ref())
        };
        answer.map(ok)
    }

    async fn admin(&self, call: RouteCall) -> Result<Reply, GuardError> {
        let Caller::Admin { key_id } = &call.caller else {
            return Err(Gate::NoAccount.refusal());
        };
        let body = parse_admin_body(&call.body);
        let ip = call
            .request
            .remote
            .map(|ip| ip.to_string())
            .unwrap_or_else(|| "unknown".into());
        let operations = self.guard.operations();
        match call.route.handler {
            handler::REWARD_ROLLOUT => operations.set_reward_rollout(key_id, body.as_ref(), &ip).await,
            _ => {
                operations
                    .acknowledge_alert(key_id, param(&call, "alertId"), body.as_ref(), &ip)
                    .await
            }
        }
    }

    /// An internal fault is logged here with the request id and answered with the fixed refusal.
    fn hide_internal(&self, request_id: &str, error: GuardError) -> SeamError {
        let GuardError::Internal(detail) = error else {
            return error.into();
        };
        let id = Some(request_id).filter(|id| !id.is_empty()).unwrap_or("missing");
        let line = format!("Unhandled P2P route error requestId={id}: {detail}");
        self.guard.log().error(&line);
        SeamError::Refused(rustic_rifts_seam::Refusal {
            code: INTERNAL_CODE.into(),
            message: INTERNAL_MESSAGE.into(),
        })
    }

    async fn answer(&self, call: RouteCall) -> Result<Reply, SeamError> {
        if !self.has_store {
            return Err(SeamError::unimplemented(Component::Guard, call.route.handler));
        }
        let request_id = call.request.request_id.clone();
        let admin = call.route.mount == Mount::Admin;
        let result = if admin {
            self.admin(call).await
        } else {
            self.hunts(call).await
        };
        result.map_err(|error| self.hide_internal(&request_id, error))
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
