//! RF1's routes for this crate, in mount order; the last is the Director's (RF1, decision 2).

use std::sync::Arc;

use rifts_seam::{
    BoxFuture, Caller, CapabilityScope, Component, HOST_CAPABILITY_HEADER, Json, Method, Mount, Refusal, Reply, Route,
    RouteCall, RouteHandler, SeamError, StubRoutes, Unimplemented,
};
use serde::Serialize;
use serde_json::Value;

use crate::RealmsError;
use crate::capabilities::HostCapabilities;
use crate::control::ControlChannel;
use crate::directory::RealmDirectory;
use crate::input::{
    ParseFailure, parse_account_id_param, parse_ban_body, parse_create_realm_body, parse_heartbeat_body,
    parse_invite_body, parse_invite_code_param, parse_join_body, parse_known_host_key_query, parse_list_query,
    parse_name_param, parse_settings_body, parse_username_body,
};
use crate::leases::Hosting;
use crate::ports::Clock;
use crate::service::to_seam;

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

pub struct RealmsRoutesParts {
    pub directory: Option<Arc<RealmDirectory>>,
    pub control: Option<Arc<ControlChannel>>,
    pub hosting: Option<Arc<Hosting>>,
    pub capabilities: Option<Arc<HostCapabilities>>,
    pub clock: Arc<dyn Clock>,
    pub realms_enabled: bool,
}

#[derive(Clone)]
pub struct RealmsRoutes {
    directory: Option<Arc<RealmDirectory>>,
    control: Option<Arc<ControlChannel>>,
    hosting: Option<Arc<Hosting>>,
    capabilities: Option<Arc<HostCapabilities>>,
    clock: Arc<dyn Clock>,
    realms_enabled: bool,
}

impl RealmsRoutes {
    pub fn new(parts: RealmsRoutesParts) -> Self {
        Self {
            directory: parts.directory,
            control: parts.control,
            hosting: parts.hosting,
            capabilities: parts.capabilities,
            clock: parts.clock,
            realms_enabled: parts.realms_enabled,
        }
    }

    fn param<'a>(call: &'a RouteCall, name: &str) -> Option<&'a str> {
        call.params
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    fn query_param<'a>(query: &'a str, name: &str) -> Option<&'a str> {
        query.split('&').filter(|part| !part.is_empty()).find_map(|part| {
            let (key, value) = part.split_once('=').unwrap_or((part, ""));
            (key == name).then_some(value)
        })
    }

    fn launcher_account(call: &RouteCall) -> Result<&str, RealmsError> {
        match &call.caller {
            Caller::Launcher(account) => Ok(&account.0),
            _ => Err(RealmsError::Refused(Refusal {
                code: "AUTH_UNAUTHORIZED".to_owned(),
                message: "Sign in required.".to_owned(),
            })),
        }
    }

    fn parse_json_body(call: &RouteCall) -> Result<Option<Value>, RealmsError> {
        if call.body.is_empty() {
            return Ok(None);
        }
        serde_json::from_slice(&call.body).map(Some).map_err(|_| {
            RealmsError::Refused(Refusal {
                code: "REALM_VALIDATION_FAILED".to_owned(),
                message: "The request body is not valid JSON.".to_owned(),
            })
        })
    }

    fn parse_p2p_object(call: &RouteCall) -> Result<Value, RealmsError> {
        let malformed = || {
            RealmsError::Refused(Refusal {
                code: "P2P_TICKET_INVALID".to_owned(),
                message: "The request body must be a JSON object.".to_owned(),
            })
        };
        if call.body.is_empty() {
            return Ok(Value::Object(serde_json::Map::new()));
        }
        match serde_json::from_slice::<Value>(&call.body) {
            Ok(value @ Value::Object(_)) => Ok(value),
            _ => Err(malformed()),
        }
    }

    fn from_parse(failure: ParseFailure) -> RealmsError {
        RealmsError::Refused(Refusal {
            code: failure.code.to_owned(),
            message: failure.message,
        })
    }

    fn account_param(call: &RouteCall) -> Result<String, RealmsError> {
        let raw = Self::param(call, "accountId").unwrap_or_default();
        parse_account_id_param(raw).ok_or_else(|| {
            RealmsError::Refused(Refusal {
                code: "REALM_VALIDATION_FAILED".to_owned(),
                message: "accountId is malformed.".to_owned(),
            })
        })
    }

    fn ok_json<T: Serialize>(value: &T) -> Result<Reply, RealmsError> {
        let text = serde_json::to_string(value).map_err(|e| RealmsError::Database(e.to_string()))?;
        Ok(Reply {
            status: 200,
            body: Json(text),
        })
    }

    async fn dispatch(&self, call: &RouteCall) -> Result<Reply, RealmsError> {
        let unimplemented = || RealmsError::Unimplemented(Unimplemented::new(Component::Realms, call.route.handler));
        if call.route.path.starts_with("/launcher/v1/realms") {
            let Some(directory) = &self.directory else {
                return Err(unimplemented());
            };
            if !self.realms_enabled {
                return Err(RealmsError::Refused(Refusal {
                    code: "REALMS_DISABLED".to_owned(),
                    message: "Realms are not available on this server.".to_owned(),
                }));
            }
            let account = Self::launcher_account(call)?;
            return match call.route.handler {
                "realmsService.ts:RealmsService.createRealm" => {
                    let body = Self::parse_json_body(call)?;
                    let input = parse_create_realm_body(body.as_ref()).map_err(Self::from_parse)?;
                    Self::ok_json(&directory.create_realm(account, &input).await?)
                }
                "realmsService.ts:RealmsService.getMine" => Self::ok_json(&directory.get_mine(account).await?),
                "realmsService.ts:RealmsService.listListed" => {
                    let (limit, offset) = parse_list_query(
                        Self::query_param(&call.query, "limit"),
                        Self::query_param(&call.query, "offset"),
                    )
                    .map_err(Self::from_parse)?;
                    Self::ok_json(&directory.list_listed(account, limit, offset).await?)
                }
                "realmsService.ts:RealmsService.getByName" => {
                    let name = parse_name_param(Self::param(call, "name")).map_err(Self::from_parse)?;
                    let known = parse_known_host_key_query(Self::query_param(&call.query, "knownHostKey"))
                        .map_err(Self::from_parse)?;
                    Self::ok_json(&directory.get_by_name(account, &name, known.as_deref()).await?)
                }
                "realmsService.ts:RealmsService.previewInvite" => {
                    let code = parse_invite_code_param(Self::param(call, "code").unwrap_or_default())
                        .map_err(Self::from_parse)?;
                    Self::ok_json(&directory.preview_invite(account, &code).await?)
                }
                "realmsService.ts:RealmsService.join" => {
                    let body = Self::parse_json_body(call)?;
                    let input = parse_join_body(body.as_ref()).map_err(Self::from_parse)?;
                    Self::ok_json(&directory.join(account, &input).await?)
                }
                "realmsService.ts:RealmsService.leave" => {
                    let name = parse_name_param(Self::param(call, "name")).map_err(Self::from_parse)?;
                    Self::ok_json(&directory.leave(account, &name).await?)
                }
                "realmsService.ts:RealmsService.updateSettings" => {
                    let body = Self::parse_json_body(call)?;
                    let input = parse_settings_body(body.as_ref()).map_err(Self::from_parse)?;
                    Self::ok_json(&directory.update_settings(account, &input).await?)
                }
                "realmsService.ts:RealmsService.createInvite" => {
                    let body = Self::parse_json_body(call)?;
                    let input = parse_invite_body(body.as_ref()).map_err(Self::from_parse)?;
                    Self::ok_json(&directory.create_invite(account, &input).await?)
                }
                "realmsService.ts:RealmsService.listInvites" => Self::ok_json(&directory.list_invites(account).await?),
                "realmsService.ts:RealmsService.revokeInvite" => {
                    let code = parse_invite_code_param(Self::param(call, "code").unwrap_or_default())
                        .map_err(Self::from_parse)?;
                    Self::ok_json(&directory.revoke_invite(account, &code).await?)
                }
                "realmsService.ts:RealmsService.kick" => {
                    let target = Self::account_param(call)?;
                    Self::ok_json(&directory.kick(account, &target).await?)
                }
                "realmsService.ts:RealmsService.ban" => {
                    let target = Self::account_param(call)?;
                    let body = Self::parse_json_body(call)?;
                    let reason = parse_ban_body(body.as_ref()).map_err(Self::from_parse)?;
                    Self::ok_json(&directory.ban(account, &target, reason.as_deref()).await?)
                }
                "realmsService.ts:RealmsService.unban" => {
                    let target = Self::account_param(call)?;
                    Self::ok_json(&directory.unban(account, &target).await?)
                }
                "realmsService.ts:RealmsService.heartbeat" => {
                    let body = Self::parse_json_body(call)?;
                    let input = parse_heartbeat_body(body.as_ref()).map_err(Self::from_parse)?;
                    Self::ok_json(&directory.heartbeat(account, &input).await?)
                }
                "realmsService.ts:RealmsService.release" => Self::ok_json(&directory.release(account).await?),
                "realmsService.ts:RealmsService.listJoinRequests" => {
                    Self::ok_json(&directory.list_join_requests(account).await?)
                }
                "realmsService.ts:RealmsService.approveJoinRequest" => {
                    let target = Self::account_param(call)?;
                    Self::ok_json(&directory.approve_join_request(account, &target).await?)
                }
                "realmsService.ts:RealmsService.denyJoinRequest" => {
                    let target = Self::account_param(call)?;
                    Self::ok_json(&directory.deny_join_request(account, &target).await?)
                }
                "realmsService.ts:RealmsService.addMemberByUsername" => {
                    let body = Self::parse_json_body(call)?;
                    let username = parse_username_body(body.as_ref()).map_err(Self::from_parse)?;
                    Self::ok_json(&directory.add_member_by_username(account, &username).await?)
                }
                "realmsService.ts:RealmsService.invitePlayerByUsername" => {
                    let body = Self::parse_json_body(call)?;
                    let username = parse_username_body(body.as_ref()).map_err(Self::from_parse)?;
                    Self::ok_json(&directory.invite_player_by_username(account, &username).await?)
                }
                "realmsService.ts:RealmsService.cancelPlayerInvite" => {
                    let target = Self::account_param(call)?;
                    Self::ok_json(&directory.cancel_player_invite(account, &target).await?)
                }
                "realmsService.ts:RealmsService.listMyPlayerInvites" => {
                    Self::ok_json(&directory.list_my_player_invites(account).await?)
                }
                "realmsService.ts:RealmsService.acceptPlayerInvite" => {
                    let name = parse_name_param(Self::param(call, "name")).map_err(Self::from_parse)?;
                    Self::ok_json(&directory.accept_player_invite(account, &name).await?)
                }
                "realmsService.ts:RealmsService.declinePlayerInvite" => {
                    let name = parse_name_param(Self::param(call, "name")).map_err(Self::from_parse)?;
                    Self::ok_json(&directory.decline_player_invite(account, &name).await?)
                }
                "realmsService.ts:RealmsService.presenceHeartbeat" => {
                    Self::ok_json(&directory.presence_heartbeat(account).await?)
                }
                "realmsService.ts:RealmsService.presenceLeave" => {
                    Self::ok_json(&directory.presence_leave(account).await?)
                }
                "realmsService.ts:RealmsService.presenceStatus" => {
                    Self::ok_json(&directory.presence_status(account).await?)
                }
                "realmsLauncher.ts:CreateRealmsRouter (catch-all)" => Err(RealmsError::Refused(Refusal {
                    code: "REALM_ROUTE_NOT_FOUND".to_owned(),
                    message: "Unknown realms route.".to_owned(),
                })),
                _ => Err(unimplemented()),
            };
        }

        match call.route.handler {
            "p2pControl.ts:PollP2PControl" => {
                let Some(control) = &self.control else {
                    return Err(unimplemented());
                };
                let account = Self::launcher_account(call)?;
                let after = Self::query_param(&call.query, "afterSequence");
                let wait = Self::query_param(&call.query, "waitMs");
                Self::ok_json(&control.poll(account, after, wait).await?)
            }
            "p2pControl.ts:AcknowledgeP2PControl" => {
                let Some(control) = &self.control else {
                    return Err(unimplemented());
                };
                let account = Self::launcher_account(call)?;
                let message_id = Self::param(call, "messageId").unwrap_or_default();
                Self::ok_json(&control.ack(account, message_id).await?)
            }
            "p2pHostLifecycle.ts:RenewHostLease" => {
                let (Some(hosting), Some(capabilities)) = (&self.hosting, &self.capabilities) else {
                    return Err(unimplemented());
                };
                let session_id = Self::param(call, "sessionId").unwrap_or_default();
                let capability = capabilities
                    .check(
                        call.request.header(HOST_CAPABILITY_HEADER),
                        session_id,
                        CapabilityScope::LeaseHeartbeat,
                        self.clock.now(),
                    )
                    .await?;
                let body = Self::parse_p2p_object(call)?;
                Self::ok_json(&hosting.renew(&capability, &body).await?)
            }
            "p2pHostLifecycle.ts:SubmitHostPresence" => {
                let Some(hosting) = &self.hosting else {
                    return Err(unimplemented());
                };
                let account = Self::launcher_account(call)?;
                let body = Self::parse_p2p_object(call)?;
                Self::ok_json(&hosting.submit_presence(account, &body).await?)
            }
            "p2pHostLifecycle.ts:RespondToHostOffer" => {
                let Some(hosting) = &self.hosting else {
                    return Err(unimplemented());
                };
                let account = Self::launcher_account(call)?;
                let offer_id = Self::param(call, "offerId").unwrap_or_default();
                let body = Self::parse_p2p_object(call)?;
                Self::ok_json(&hosting.respond_to_offer(account, offer_id, &body).await?)
            }
            "p2pHostLifecycle.ts:RecordParticipantObservation" => {
                let Some(hosting) = &self.hosting else {
                    return Err(unimplemented());
                };
                let account = Self::launcher_account(call)?;
                let session_id = Self::param(call, "sessionId").unwrap_or_default();
                let body = Self::parse_p2p_object(call)?;
                Self::ok_json(&hosting.observe(account, session_id, &body).await?)
            }
            _ => Err(unimplemented()),
        }
    }
}

impl RouteHandler for RealmsRoutes {
    fn routes(&self) -> &'static [Route] {
        ROUTES
    }

    fn handle(&self, call: RouteCall) -> BoxFuture<'_, Result<Reply, SeamError>> {
        Box::pin(async move { self.dispatch(&call).await.map_err(to_seam) })
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

    #[test]
    fn realms_routes_without_store_answers_unimplemented() {
        let handler = RealmsRoutes::new(RealmsRoutesParts {
            directory: None,
            control: None,
            hosting: None,
            capabilities: None,
            clock: Arc::new(crate::ports::SystemClock),
            realms_enabled: true,
        });
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
