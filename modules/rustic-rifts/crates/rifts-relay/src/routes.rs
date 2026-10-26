//! RF1's routes for this crate, in mount order.

use rifts_seam::{Component, Method, Mount, Route, StubRoutes};

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
