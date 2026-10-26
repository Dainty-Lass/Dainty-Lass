//! RF1's routes for this crate, in mount order; the last is the Director's (RF1, decision 2).

use rifts_seam::{Component, Method, Mount, Route, StubRoutes};

pub const ROUTES: &[Route] = &[
    Route {
        method: Method::Post,
        path: "/mystic/v1/hunts/:sessionId/host-ready",
        mount: Mount::Early,
        handler: "p2pAdmission.ts:MarkHostReady",
    },
    Route {
        method: Method::Post,
        path: "/mystic/v1/hunts/:sessionId/abort",
        mount: Mount::Early,
        handler: "p2pHuntStatus.ts:AbortHunt",
    },
    Route {
        method: Method::Post,
        path: "/mystic/v1/hunts/:sessionId/end",
        mount: Mount::Early,
        handler: "p2pHuntStatus.ts:EndHunt",
    },
    Route {
        method: Method::Get,
        path: "/mystic/v1/hunts/:sessionId/status",
        mount: Mount::Early,
        handler: "p2pHuntStatus.ts:GetHuntStatus",
    },
    Route {
        method: Method::Post,
        path: "/mystic/v1/hunts/:sessionId/lease",
        mount: Mount::Early,
        handler: "p2pHostLifecycle.ts:RenewHostLease",
    },
    Route {
        method: Method::Patch,
        path: "/admin/v1/operations/p2p-emergency-stop",
        mount: Mount::Admin,
        handler: "p2pAdminOperations.ts:UpdateP2PEmergencyStop",
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/p2p/hunts/:sessionId/host-bootstrap",
        mount: Mount::Normal,
        handler: "p2pLauncher.ts:CreateHostBootstrap",
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/p2p/hunts/:sessionId/route-ready",
        mount: Mount::Normal,
        handler: "p2pLauncher.ts:MarkLocalRouteReady",
    },
    Route {
        method: Method::Get,
        path: "/launcher/v1/p2p/control",
        mount: Mount::Normal,
        handler: "p2pControl.ts:PollP2PControl",
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/p2p/control/:messageId/ack",
        mount: Mount::Normal,
        handler: "p2pControl.ts:AcknowledgeP2PControl",
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/p2p/host-presence",
        mount: Mount::Normal,
        handler: "p2pHostLifecycle.ts:SubmitHostPresence",
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/p2p/host-offers/:offerId/respond",
        mount: Mount::Normal,
        handler: "p2pHostLifecycle.ts:RespondToHostOffer",
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/p2p/hunts/:sessionId/observation",
        mount: Mount::Normal,
        handler: "p2pHostLifecycle.ts:RecordParticipantObservation",
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/realm-hosting/hubs/start",
        mount: Mount::Normal,
        handler: "realmHubLifecycle.ts:CreateRealmHubSession",
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/realm-hosting/hubs/stop",
        mount: Mount::Normal,
        handler: "realmHubLifecycle.ts:StopRealmHubSession",
    },
    Route {
        method: Method::Get,
        path: "/launcher/v1/realm-hosting/status",
        mount: Mount::Normal,
        handler: "realmHostingStatus.ts:GetRealmHostingStatus",
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/realms",
        mount: Mount::Normal,
        handler: "realmsService.ts:RealmsService.createRealm",
    },
    Route {
        method: Method::Get,
        path: "/launcher/v1/realms/mine",
        mount: Mount::Normal,
        handler: "realmsService.ts:RealmsService.getMine",
    },
    Route {
        method: Method::Get,
        path: "/launcher/v1/realms/listed",
        mount: Mount::Normal,
        handler: "realmsService.ts:RealmsService.listListed",
    },
    Route {
        method: Method::Get,
        path: "/launcher/v1/realms/by-name/:name",
        mount: Mount::Normal,
        handler: "realmsService.ts:RealmsService.getByName",
    },
    Route {
        method: Method::Get,
        path: "/launcher/v1/realms/invite/:code",
        mount: Mount::Normal,
        handler: "realmsService.ts:RealmsService.previewInvite",
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/realms/join",
        mount: Mount::Normal,
        handler: "realmsService.ts:RealmsService.join",
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/realms/by-name/:name/leave",
        mount: Mount::Normal,
        handler: "realmsService.ts:RealmsService.leave",
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/realms/mine/settings",
        mount: Mount::Normal,
        handler: "realmsService.ts:RealmsService.updateSettings",
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/realms/mine/invites",
        mount: Mount::Normal,
        handler: "realmsService.ts:RealmsService.createInvite",
    },
    Route {
        method: Method::Get,
        path: "/launcher/v1/realms/mine/invites",
        mount: Mount::Normal,
        handler: "realmsService.ts:RealmsService.listInvites",
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/realms/mine/invites/:code/revoke",
        mount: Mount::Normal,
        handler: "realmsService.ts:RealmsService.revokeInvite",
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/realms/mine/members/:accountId/kick",
        mount: Mount::Normal,
        handler: "realmsService.ts:RealmsService.kick",
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/realms/mine/members/:accountId/ban",
        mount: Mount::Normal,
        handler: "realmsService.ts:RealmsService.ban",
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/realms/mine/members/:accountId/unban",
        mount: Mount::Normal,
        handler: "realmsService.ts:RealmsService.unban",
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/realms/mine/heartbeat",
        mount: Mount::Normal,
        handler: "realmsService.ts:RealmsService.heartbeat",
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/realms/mine/release",
        mount: Mount::Normal,
        handler: "realmsService.ts:RealmsService.release",
    },
    Route {
        method: Method::Get,
        path: "/launcher/v1/realms/mine/requests",
        mount: Mount::Normal,
        handler: "realmsService.ts:RealmsService.listJoinRequests",
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/realms/mine/requests/:accountId/approve",
        mount: Mount::Normal,
        handler: "realmsService.ts:RealmsService.approveJoinRequest",
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/realms/mine/requests/:accountId/deny",
        mount: Mount::Normal,
        handler: "realmsService.ts:RealmsService.denyJoinRequest",
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/realms/mine/members/add",
        mount: Mount::Normal,
        handler: "realmsService.ts:RealmsService.addMemberByUsername",
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/realms/mine/player-invites",
        mount: Mount::Normal,
        handler: "realmsService.ts:RealmsService.invitePlayerByUsername",
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/realms/mine/player-invites/:accountId/cancel",
        mount: Mount::Normal,
        handler: "realmsService.ts:RealmsService.cancelPlayerInvite",
    },
    Route {
        method: Method::Get,
        path: "/launcher/v1/realms/player-invites",
        mount: Mount::Normal,
        handler: "realmsService.ts:RealmsService.listMyPlayerInvites",
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/realms/player-invites/:name/accept",
        mount: Mount::Normal,
        handler: "realmsService.ts:RealmsService.acceptPlayerInvite",
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/realms/player-invites/:name/decline",
        mount: Mount::Normal,
        handler: "realmsService.ts:RealmsService.declinePlayerInvite",
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/realms/presence/heartbeat",
        mount: Mount::Normal,
        handler: "realmsService.ts:RealmsService.presenceHeartbeat",
    },
    Route {
        method: Method::Post,
        path: "/launcher/v1/realms/presence/leave",
        mount: Mount::Normal,
        handler: "realmsService.ts:RealmsService.presenceLeave",
    },
    Route {
        method: Method::Get,
        path: "/launcher/v1/realms/presence",
        mount: Mount::Normal,
        handler: "realmsService.ts:RealmsService.presenceStatus",
    },
    Route {
        method: Method::All,
        path: "/launcher/v1/realms/*",
        mount: Mount::Normal,
        handler: "realmsLauncher.ts:CreateRealmsRouter (catch-all)",
    },
    Route {
        method: Method::Post,
        path: "/api/matchmaker/resolve-hunt-launch-manifest",
        mount: Mount::Director,
        handler: "huntLaunchManifest.ts:ResolvePlayerHostedLaunchManifest",
    },
];

pub fn stub_routes() -> StubRoutes {
    StubRoutes {
        component: Component::Realms,
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
            .filter(|route| route["component"] == "rifts-realms")
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
            let expected = Unimplemented::new(Component::Realms, route.handler);
            assert_eq!(answer, Err(SeamError::Unimplemented(expected)), "{}", route.path);
        }
    }
}
