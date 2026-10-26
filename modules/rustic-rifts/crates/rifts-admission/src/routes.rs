//! RF1's routes for this crate, in mount order.

use rifts_seam::{Component, Method, Mount, Route, StubRoutes};

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
