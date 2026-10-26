//! RF1's routes for this crate, in mount order. Launcher Guard and the Guard manifests stay isles' (crate docs).

use rifts_seam::{Component, Method, Mount, Route, StubRoutes};

pub const ROUTES: &[Route] = &[
    Route {
        method: Method::Post,
        path: "/mystic/v1/hunts/:sessionId/event-batches",
        mount: Mount::Early,
        handler: "antiCheat.ts:hunts.submitEvidence",
    },
    Route {
        method: Method::Post,
        path: "/mystic/v1/hunts/:sessionId/completion",
        mount: Mount::Early,
        handler: "antiCheat.ts:hunts.complete",
    },
    Route {
        method: Method::Get,
        path: "/mystic/v1/hunts/:sessionId/reward/self",
        mount: Mount::Early,
        handler: "p2pHuntStatus.ts:GetSelfHuntReward",
    },
    Route {
        method: Method::Post,
        path: "/internal/v1/hunt-guard/shadow-comparisons",
        mount: Mount::Early,
        handler: "antiCheat.ts:hunts.recordShadowComparison",
    },
    Route {
        method: Method::Patch,
        path: "/admin/v1/operations/reward-rollout",
        mount: Mount::Admin,
        handler: "p2pAdminOperations.ts:UpdateRewardRollout",
    },
    Route {
        method: Method::Patch,
        path: "/admin/v1/operations/alerts/:alertId/ack",
        mount: Mount::Admin,
        handler: "p2pAdminOperations.ts:AcknowledgeOperationsAlert",
    },
];

pub fn stub_routes() -> StubRoutes {
    StubRoutes {
        component: Component::Guard,
        routes: ROUTES,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rifts_seam::{Caller, RequestInfo, RouteCall, RouteHandler, SeamError, Unimplemented, poll_once};

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
            let expected = Unimplemented::new(Component::Guard, route.handler);
            assert_eq!(answer, Err(SeamError::Unimplemented(expected)), "{}", route.path);
        }
    }
}
