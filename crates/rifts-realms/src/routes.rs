// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Realms' 46 routes as data, in mount order, and their dispatch. A route whose service is absent answers
//! `Unimplemented` under its handler id, as a stub does.

use std::sync::Arc;

use rifts_seam::{
    BoxFuture, Caller, CapabilityScope, Component, HOST_CAPABILITY_HEADER, Json, Method, Mount, Refusal, Reply, Route,
    RouteCall, RouteHandler, SeamError, StubRoutes, Unimplemented,
};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::RealmsError;
use crate::capabilities::HostCapabilities;
use crate::control::ControlChannel;
use crate::directory::RealmDirectory;
use crate::hosting::HuntLifecycle as _;
use crate::hunt::HostCapabilityRecord;
use crate::input::{
    ParseFailure, parse_account_id_param, parse_ban_body, parse_create_realm_body, parse_heartbeat_body,
    parse_invite_body, parse_invite_code_param, parse_join_body, parse_known_host_key_query, parse_list_query,
    parse_name_param, parse_settings_body, parse_username_body,
};
use crate::leases::Hosting;
use crate::ports::Clock;

// Umbrella spec D3: neutral handler ids. They name the operation in a 501 answer.
const MARK_HOST_READY: &str = "realms.mark_host_ready";
const ABORT_HUNT: &str = "realms.abort_hunt";
const END_HUNT: &str = "realms.end_hunt";
const GET_HUNT_STATUS: &str = "realms.get_hunt_status";
const RENEW_HOST_LEASE: &str = "realms.renew_host_lease";
const UPDATE_EMERGENCY_STOP: &str = "realms.update_emergency_stop";
const CREATE_HOST_BOOTSTRAP: &str = "realms.create_host_bootstrap";
const MARK_LOCAL_ROUTE_READY: &str = "realms.mark_local_route_ready";
const POLL_CONTROL: &str = "realms.poll_control";
const ACKNOWLEDGE_CONTROL: &str = "realms.acknowledge_control";
const SUBMIT_HOST_PRESENCE: &str = "realms.submit_host_presence";
const RESPOND_TO_HOST_OFFER: &str = "realms.respond_to_host_offer";
const RECORD_OBSERVATION: &str = "realms.record_participant_observation";
const START_HUB: &str = "realms.start_hub";
const STOP_HUB: &str = "realms.stop_hub";
const GET_HOSTING_STATUS: &str = "realms.get_hosting_status";
const CREATE_REALM: &str = "realms.create_realm";
const GET_MINE: &str = "realms.get_mine";
const LIST_LISTED: &str = "realms.list_listed";
const GET_BY_NAME: &str = "realms.get_by_name";
const PREVIEW_INVITE: &str = "realms.preview_invite";
const JOIN: &str = "realms.join";
const LEAVE: &str = "realms.leave";
const UPDATE_SETTINGS: &str = "realms.update_settings";
const CREATE_INVITE: &str = "realms.create_invite";
const LIST_INVITES: &str = "realms.list_invites";
const REVOKE_INVITE: &str = "realms.revoke_invite";
const KICK: &str = "realms.kick";
const BAN: &str = "realms.ban";
const UNBAN: &str = "realms.unban";
const HEARTBEAT: &str = "realms.heartbeat";
const RELEASE: &str = "realms.release";
const LIST_JOIN_REQUESTS: &str = "realms.list_join_requests";
const APPROVE_JOIN_REQUEST: &str = "realms.approve_join_request";
const DENY_JOIN_REQUEST: &str = "realms.deny_join_request";
const ADD_MEMBER_BY_USERNAME: &str = "realms.add_member_by_username";
const INVITE_PLAYER_BY_USERNAME: &str = "realms.invite_player_by_username";
const CANCEL_PLAYER_INVITE: &str = "realms.cancel_player_invite";
const LIST_MY_PLAYER_INVITES: &str = "realms.list_my_player_invites";
const ACCEPT_PLAYER_INVITE: &str = "realms.accept_player_invite";
const DECLINE_PLAYER_INVITE: &str = "realms.decline_player_invite";
const PRESENCE_HEARTBEAT: &str = "realms.presence_heartbeat";
const PRESENCE_LEAVE: &str = "realms.presence_leave";
const PRESENCE_STATUS: &str = "realms.presence_status";
const UNKNOWN_REALMS_ROUTE: &str = "realms.unknown_route";
const RESOLVE_LAUNCH_MANIFEST: &str = "realms.resolve_launch_manifest";

const fn route(method: Method, path: &'static str, mount: Mount, handler: &'static str) -> Route {
    Route {
        method,
        path,
        mount,
        handler,
    }
}

use Method::{All, Get, Patch, Post};
use Mount::{Admin, Director, Early, Normal};

/// Isles mounts the realms routes (17-45) with `Cache-Control: no-store` and only when realms are on; the catch-all
/// stays after every other realms route.
pub const ROUTES: &[Route] = &[
    route(Post, "/rustic/v1/hunts/:sessionId/host-ready", Early, MARK_HOST_READY),
    route(Post, "/rustic/v1/hunts/:sessionId/abort", Early, ABORT_HUNT),
    route(Post, "/rustic/v1/hunts/:sessionId/end", Early, END_HUNT),
    route(Get, "/rustic/v1/hunts/:sessionId/status", Early, GET_HUNT_STATUS),
    route(Post, "/rustic/v1/hunts/:sessionId/lease", Early, RENEW_HOST_LEASE),
    route(
        Patch,
        "/admin/v1/operations/p2p-emergency-stop",
        Admin,
        UPDATE_EMERGENCY_STOP,
    ),
    route(
        Post,
        "/launcher/v1/p2p/hunts/:sessionId/host-bootstrap",
        Normal,
        CREATE_HOST_BOOTSTRAP,
    ),
    route(
        Post,
        "/launcher/v1/p2p/hunts/:sessionId/route-ready",
        Normal,
        MARK_LOCAL_ROUTE_READY,
    ),
    route(Get, "/launcher/v1/p2p/control", Normal, POLL_CONTROL),
    route(
        Post,
        "/launcher/v1/p2p/control/:messageId/ack",
        Normal,
        ACKNOWLEDGE_CONTROL,
    ),
    route(Post, "/launcher/v1/p2p/host-presence", Normal, SUBMIT_HOST_PRESENCE),
    route(
        Post,
        "/launcher/v1/p2p/host-offers/:offerId/respond",
        Normal,
        RESPOND_TO_HOST_OFFER,
    ),
    route(
        Post,
        "/launcher/v1/p2p/hunts/:sessionId/observation",
        Normal,
        RECORD_OBSERVATION,
    ),
    route(Post, "/launcher/v1/realm-hosting/hubs/start", Normal, START_HUB),
    route(Post, "/launcher/v1/realm-hosting/hubs/stop", Normal, STOP_HUB),
    route(Get, "/launcher/v1/realm-hosting/status", Normal, GET_HOSTING_STATUS),
    route(Post, "/launcher/v1/realms", Normal, CREATE_REALM),
    route(Get, "/launcher/v1/realms/mine", Normal, GET_MINE),
    route(Get, "/launcher/v1/realms/listed", Normal, LIST_LISTED),
    route(Get, "/launcher/v1/realms/by-name/:name", Normal, GET_BY_NAME),
    route(Get, "/launcher/v1/realms/invite/:code", Normal, PREVIEW_INVITE),
    route(Post, "/launcher/v1/realms/join", Normal, JOIN),
    route(Post, "/launcher/v1/realms/by-name/:name/leave", Normal, LEAVE),
    route(Post, "/launcher/v1/realms/mine/settings", Normal, UPDATE_SETTINGS),
    route(Post, "/launcher/v1/realms/mine/invites", Normal, CREATE_INVITE),
    route(Get, "/launcher/v1/realms/mine/invites", Normal, LIST_INVITES),
    route(
        Post,
        "/launcher/v1/realms/mine/invites/:code/revoke",
        Normal,
        REVOKE_INVITE,
    ),
    route(Post, "/launcher/v1/realms/mine/members/:accountId/kick", Normal, KICK),
    route(Post, "/launcher/v1/realms/mine/members/:accountId/ban", Normal, BAN),
    route(Post, "/launcher/v1/realms/mine/members/:accountId/unban", Normal, UNBAN),
    route(Post, "/launcher/v1/realms/mine/heartbeat", Normal, HEARTBEAT),
    route(Post, "/launcher/v1/realms/mine/release", Normal, RELEASE),
    route(Get, "/launcher/v1/realms/mine/requests", Normal, LIST_JOIN_REQUESTS),
    route(
        Post,
        "/launcher/v1/realms/mine/requests/:accountId/approve",
        Normal,
        APPROVE_JOIN_REQUEST,
    ),
    route(
        Post,
        "/launcher/v1/realms/mine/requests/:accountId/deny",
        Normal,
        DENY_JOIN_REQUEST,
    ),
    route(
        Post,
        "/launcher/v1/realms/mine/members/add",
        Normal,
        ADD_MEMBER_BY_USERNAME,
    ),
    route(
        Post,
        "/launcher/v1/realms/mine/player-invites",
        Normal,
        INVITE_PLAYER_BY_USERNAME,
    ),
    route(
        Post,
        "/launcher/v1/realms/mine/player-invites/:accountId/cancel",
        Normal,
        CANCEL_PLAYER_INVITE,
    ),
    route(
        Get,
        "/launcher/v1/realms/player-invites",
        Normal,
        LIST_MY_PLAYER_INVITES,
    ),
    route(
        Post,
        "/launcher/v1/realms/player-invites/:name/accept",
        Normal,
        ACCEPT_PLAYER_INVITE,
    ),
    route(
        Post,
        "/launcher/v1/realms/player-invites/:name/decline",
        Normal,
        DECLINE_PLAYER_INVITE,
    ),
    route(
        Post,
        "/launcher/v1/realms/presence/heartbeat",
        Normal,
        PRESENCE_HEARTBEAT,
    ),
    route(Post, "/launcher/v1/realms/presence/leave", Normal, PRESENCE_LEAVE),
    route(Get, "/launcher/v1/realms/presence", Normal, PRESENCE_STATUS),
    route(All, "/launcher/v1/realms/*", Normal, UNKNOWN_REALMS_ROUTE),
    route(
        Post,
        "/api/matchmaker/resolve-hunt-launch-manifest",
        Director,
        RESOLVE_LAUNCH_MANIFEST,
    ),
];

/// Every route answers `Unimplemented(Realms, handler)`.
pub fn stub_routes() -> StubRoutes {
    StubRoutes {
        component: Component::Realms,
        routes: ROUTES,
    }
}

/// What the realms routes answer from; an absent part answers its routes as unimplemented.
pub struct RealmsRoutesParts {
    /// The realm directory.
    pub directory: Option<Arc<RealmDirectory>>,
    /// The launcher control channel.
    pub control: Option<Arc<ControlChannel>>,
    /// Player hosting.
    pub hosting: Option<Arc<Hosting>>,
    /// Host capabilities.
    pub capabilities: Option<Arc<HostCapabilities>>,
    /// The time now.
    pub clock: Arc<dyn Clock>,
    /// Whether the realms module is on; the realm routes refuse with `REALMS_DISABLED` when it is off.
    pub realms_enabled: bool,
}

/// Answers the realms, control and hosting routes.
#[derive(Clone)]
pub struct RealmsRoutes {
    parts: Arc<RealmsRoutesParts>,
}

fn refused(code: &str, message: &str) -> RealmsError {
    RealmsError::Refused(Refusal {
        code: code.to_owned(),
        message: message.to_owned(),
    })
}

fn unimplemented(handler: &'static str) -> RealmsError {
    RealmsError::Unimplemented(Unimplemented::new(Component::Realms, handler))
}

/// The database's text never reaches a client.
fn to_seam(error: RealmsError) -> SeamError {
    match error {
        RealmsError::Unimplemented(missing) => SeamError::Unimplemented(missing),
        RealmsError::Refused(refusal) => SeamError::Refused(refusal),
        RealmsError::Database(_) => SeamError::Refused(Refusal {
            code: "INTERNAL".to_owned(),
            message: "The realms request failed.".to_owned(),
        }),
    }
}

fn param<'a>(call: &'a RouteCall, name: &str) -> Option<&'a str> {
    call.params
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

/// The first value of `name`, as written: Express would percent-decode it, this does not.
fn query_param<'a>(call: &'a RouteCall, name: &str) -> Option<&'a str> {
    call.query
        .split('&')
        .filter(|piece| !piece.is_empty())
        .map(|piece| piece.split_once('=').unwrap_or((piece, "")))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value)
}

fn launcher_account(call: &RouteCall) -> Result<&str, RealmsError> {
    match &call.caller {
        Caller::Launcher(account) => Ok(&account.0),
        _ => Err(refused("AUTH_UNAUTHORIZED", "Sign in required.")),
    }
}

/// An empty body is no body.
fn realms_body(call: &RouteCall) -> Result<Option<Value>, RealmsError> {
    if call.body.is_empty() {
        return Ok(None);
    }
    serde_json::from_slice(&call.body)
        .map(Some)
        .map_err(|_| refused("REALM_VALIDATION_FAILED", "The request body is not valid JSON."))
}

/// An empty body is an empty object; anything but an object is refused.
fn p2p_body(call: &RouteCall) -> Result<Map<String, Value>, RealmsError> {
    if call.body.is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_slice(&call.body) {
        Ok(Value::Object(object)) => Ok(object),
        _ => Err(refused("P2P_TICKET_INVALID", "The request body must be a JSON object.")),
    }
}

/// Invalid UTF-8 reads as empty.
fn raw_text(call: &RouteCall) -> Json {
    Json(String::from_utf8(call.body.clone()).unwrap_or_default())
}

fn parse_failure(failure: ParseFailure) -> RealmsError {
    refused(failure.code, &failure.message)
}

fn account_param(call: &RouteCall) -> Result<String, RealmsError> {
    parse_account_id_param(param(call, "accountId").unwrap_or(""))
        .ok_or_else(|| refused("REALM_VALIDATION_FAILED", "accountId is malformed."))
}

/// The realm a leave or player-invite route names, as written: an unknown text finds no realm.
fn name_text(call: &RouteCall) -> String {
    param(call, "name").unwrap_or("").to_owned()
}

fn ok(value: &impl Serialize) -> Result<Reply, RealmsError> {
    let body = serde_json::to_string(value).map_err(|e| RealmsError::Database(e.to_string()))?;
    Ok(Reply {
        status: 200,
        body: Json(body),
    })
}

impl RealmsRoutes {
    /// Routes over these parts.
    pub fn new(parts: RealmsRoutesParts) -> Self {
        Self { parts: Arc::new(parts) }
    }

    async fn dispatch(&self, call: RouteCall) -> Result<Reply, RealmsError> {
        let handler = call.route.handler;
        if call.route.path.starts_with("/launcher/v1/realms") {
            return self.realms_route(&call, handler).await;
        }
        match handler {
            POLL_CONTROL | ACKNOWLEDGE_CONTROL => self.control_route(&call, handler).await,
            _ => self.hosting_route(&call, handler).await,
        }
    }

    /// The launcher's realm routes: the directory, realms on and a signed-in launcher come before anything else.
    async fn realms_route(&self, call: &RouteCall, handler: &'static str) -> Result<Reply, RealmsError> {
        let Some(directory) = &self.parts.directory else {
            return Err(unimplemented(handler));
        };
        if !self.parts.realms_enabled {
            return Err(refused("REALMS_DISABLED", "Realms are not available on this server."));
        }
        let account = launcher_account(call)?;
        let d = directory.as_ref();
        let text = |name: &str| param(call, name).unwrap_or("");
        match handler {
            CREATE_REALM => {
                let input = parse_create_realm_body(realms_body(call)?.as_ref()).map_err(parse_failure)?;
                ok(&d.create_realm(account, &input).await?)
            }
            GET_MINE => ok(&d.get_mine(account).await?),
            LIST_LISTED => {
                let (limit, offset) =
                    parse_list_query(query_param(call, "limit"), query_param(call, "offset")).map_err(parse_failure)?;
                ok(&d.list_listed(account, limit, offset).await?)
            }
            GET_BY_NAME => {
                let name = parse_name_param(param(call, "name")).map_err(parse_failure)?;
                let known = parse_known_host_key_query(query_param(call, "knownHostKey")).map_err(parse_failure)?;
                ok(&d.get_by_name(account, &name, known.as_deref()).await?)
            }
            PREVIEW_INVITE => {
                let code = parse_invite_code_param(text("code")).map_err(parse_failure)?;
                ok(&d.preview_invite(account, &code).await?)
            }
            JOIN => {
                let input = parse_join_body(realms_body(call)?.as_ref()).map_err(parse_failure)?;
                ok(&d.join(account, &input).await?)
            }
            LEAVE => ok(&d.leave(account, &name_text(call)).await?),
            UPDATE_SETTINGS => {
                let input = parse_settings_body(realms_body(call)?.as_ref()).map_err(parse_failure)?;
                ok(&d.update_settings(account, &input).await?)
            }
            CREATE_INVITE => {
                let input = parse_invite_body(realms_body(call)?.as_ref()).map_err(parse_failure)?;
                ok(&d.create_invite(account, &input).await?)
            }
            LIST_INVITES => ok(&d.list_invites(account).await?),
            REVOKE_INVITE => ok(&d.revoke_invite(account, text("code")).await?),
            KICK => ok(&d.kick(account, &account_param(call)?).await?),
            BAN => {
                let target = account_param(call)?;
                let reason = parse_ban_body(realms_body(call)?.as_ref()).map_err(parse_failure)?;
                ok(&d.ban(account, &target, reason).await?)
            }
            UNBAN => ok(&d.unban(account, &account_param(call)?).await?),
            HEARTBEAT => {
                let input = parse_heartbeat_body(realms_body(call)?.as_ref()).map_err(parse_failure)?;
                ok(&d.heartbeat(account, &input).await?)
            }
            RELEASE => ok(&d.release(account).await?),
            LIST_JOIN_REQUESTS => ok(&d.list_join_requests(account).await?),
            APPROVE_JOIN_REQUEST => ok(&d.approve_join_request(account, &account_param(call)?).await?),
            DENY_JOIN_REQUEST => ok(&d.deny_join_request(account, &account_param(call)?).await?),
            ADD_MEMBER_BY_USERNAME => {
                let username = parse_username_body(realms_body(call)?.as_ref()).map_err(parse_failure)?;
                ok(&d.add_member_by_username(account, &username).await?)
            }
            INVITE_PLAYER_BY_USERNAME => {
                let username = parse_username_body(realms_body(call)?.as_ref()).map_err(parse_failure)?;
                ok(&d.invite_player_by_username(account, &username).await?)
            }
            CANCEL_PLAYER_INVITE => ok(&d.cancel_player_invite(account, &account_param(call)?).await?),
            LIST_MY_PLAYER_INVITES => ok(&d.list_my_player_invites(account).await?),
            ACCEPT_PLAYER_INVITE => ok(&d.accept_player_invite(account, &name_text(call)).await?),
            DECLINE_PLAYER_INVITE => ok(&d.decline_player_invite(account, &name_text(call)).await?),
            PRESENCE_HEARTBEAT => ok(&d.presence_heartbeat(account).await?),
            PRESENCE_LEAVE => ok(&d.presence_leave(account).await?),
            PRESENCE_STATUS => ok(&d.presence_status(account).await?),
            _ => Err(refused("REALM_ROUTE_NOT_FOUND", "Unknown realms route.")),
        }
    }

    async fn control_route(&self, call: &RouteCall, handler: &'static str) -> Result<Reply, RealmsError> {
        let Some(control) = &self.parts.control else {
            return Err(unimplemented(handler));
        };
        let account = launcher_account(call)?;
        if handler == POLL_CONTROL {
            let reply = control
                .poll(account, query_param(call, "afterSequence"), query_param(call, "waitMs"))
                .await?;
            return ok(&reply);
        }
        let reply = control.ack(account, param(call, "messageId").unwrap_or("")).await?;
        ok(&reply)
    }

    async fn check(
        &self,
        capabilities: &HostCapabilities,
        call: &RouteCall,
        scope: CapabilityScope,
    ) -> Result<HostCapabilityRecord, RealmsError> {
        let header = call.request.header(HOST_CAPABILITY_HEADER);
        let session = param(call, "sessionId").unwrap_or("");
        capabilities.check(header, session, scope, self.parts.clock.now()).await
    }

    /// The session, hub and admin routes: each needs `Hosting` (and the capabilities for 1, 2, 3 and 5). A
    /// capability is checked before the body is read.
    async fn hosting_route(&self, call: &RouteCall, handler: &'static str) -> Result<Reply, RealmsError> {
        let Some(hosting) = &self.parts.hosting else {
            return Err(unimplemented(handler));
        };
        let hosting = hosting.as_ref();
        let needs_capabilities = matches!(handler, MARK_HOST_READY | ABORT_HUNT | END_HUNT | RENEW_HOST_LEASE);
        let capabilities = self.parts.capabilities.as_deref();
        if needs_capabilities && capabilities.is_none() {
            return Err(unimplemented(handler));
        }
        let session = param(call, "sessionId").unwrap_or("");
        match (handler, capabilities) {
            (MARK_HOST_READY, Some(caps)) => {
                let capability = self.check(caps, call, CapabilityScope::HostReady).await?;
                ok(&hosting.mark_host_ready(&capability, &p2p_body(call)?).await?)
            }
            (ABORT_HUNT, Some(caps)) => {
                let capability = self.check(caps, call, CapabilityScope::Abort).await?;
                ok(&hosting.abort_hunt(&capability, &p2p_body(call)?).await?)
            }
            (END_HUNT, Some(caps)) => {
                let capability = match self.check(caps, call, CapabilityScope::CompleteRequest).await {
                    Ok(capability) => capability,
                    Err(_) => match self.check(caps, call, CapabilityScope::GameServerRuntime).await {
                        Ok(capability) => capability,
                        Err(_) => self.check(caps, call, CapabilityScope::Abort).await?,
                    },
                };
                ok(&hosting.finish_hunt(&capability, &p2p_body(call)?).await?)
            }
            (RENEW_HOST_LEASE, Some(caps)) => {
                let capability = self.check(caps, call, CapabilityScope::LeaseHeartbeat).await?;
                ok(&hosting.renew(&capability, &p2p_body(call)?).await?)
            }
            (GET_HUNT_STATUS, _) => {
                let account = match &call.caller {
                    Caller::Launcher(account) => Some(account.0.clone()),
                    Caller::HostRuntime(runtime) => Some(runtime.host_account.0.clone()),
                    Caller::Admin { .. } => None,
                    Caller::Anonymous => match capabilities {
                        Some(caps) if call.request.header(HOST_CAPABILITY_HEADER).is_some() => {
                            self.check(caps, call, CapabilityScope::LeaseHeartbeat).await?;
                            None
                        }
                        _ => return Err(refused("AUTH_UNAUTHORIZED", "Sign in required.")),
                    },
                };
                ok(&hosting.get_hunt_status(account.as_deref(), session).await?)
            }
            (UPDATE_EMERGENCY_STOP, _) => {
                let Caller::Admin { key_id } = &call.caller else {
                    return Err(refused("AUTH_UNAUTHORIZED", "Admin authentication required."));
                };
                let answer = hosting.set_emergency_stop(key_id.clone(), raw_text(call)).await?;
                Ok(Reply {
                    status: 200,
                    body: answer,
                })
            }
            (RESOLVE_LAUNCH_MANIFEST, _) => {
                let answer = hosting.resolve_launch_manifest(raw_text(call)).await?;
                Ok(Reply {
                    status: 200,
                    body: answer,
                })
            }
            (CREATE_HOST_BOOTSTRAP, _) => {
                let account = launcher_account(call)?;
                ok(&hosting.create_host_bootstrap(account, session).await?)
            }
            (MARK_LOCAL_ROUTE_READY, _) => {
                let account = launcher_account(call)?;
                ok(&hosting
                    .mark_local_route_ready(account, session, &p2p_body(call)?)
                    .await?)
            }
            (SUBMIT_HOST_PRESENCE, _) => {
                let account = launcher_account(call)?;
                ok(&hosting.submit_presence(account, &p2p_body(call)?).await?)
            }
            (RESPOND_TO_HOST_OFFER, _) => {
                let account = launcher_account(call)?;
                let offer = param(call, "offerId").unwrap_or("");
                ok(&hosting.respond_to_offer(account, offer, &p2p_body(call)?).await?)
            }
            (RECORD_OBSERVATION, _) => {
                let account = launcher_account(call)?;
                ok(&hosting.observe(account, session, &p2p_body(call)?).await?)
            }
            (START_HUB, _) => {
                let account = launcher_account(call)?;
                ok(&hosting.start_realm_hub(account, &p2p_body(call)?).await?)
            }
            (STOP_HUB, _) => {
                let account = launcher_account(call)?;
                ok(&hosting.stop_realm_hub(account, &p2p_body(call)?).await?)
            }
            (GET_HOSTING_STATUS, _) => {
                let account = launcher_account(call)?;
                ok(&hosting.get_realm_hosting_status(account).await?)
            }
            _ => Err(unimplemented(handler)),
        }
    }
}

impl RouteHandler for RealmsRoutes {
    fn routes(&self) -> &'static [Route] {
        ROUTES
    }

    fn handle(&self, call: RouteCall) -> BoxFuture<'_, Result<Reply, SeamError>> {
        Box::pin(async move { self.dispatch(call).await.map_err(to_seam) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::SystemClock;
    use rifts_seam::{RequestInfo, poll_once};

    /// spec-rd27-rifts-realms-store §9.1, with the umbrella's D3 renames.
    const EXPECTED: [(&str, &str); 46] = [
        ("POST", "/rustic/v1/hunts/:sessionId/host-ready"),
        ("POST", "/rustic/v1/hunts/:sessionId/abort"),
        ("POST", "/rustic/v1/hunts/:sessionId/end"),
        ("GET", "/rustic/v1/hunts/:sessionId/status"),
        ("POST", "/rustic/v1/hunts/:sessionId/lease"),
        ("PATCH", "/admin/v1/operations/p2p-emergency-stop"),
        ("POST", "/launcher/v1/p2p/hunts/:sessionId/host-bootstrap"),
        ("POST", "/launcher/v1/p2p/hunts/:sessionId/route-ready"),
        ("GET", "/launcher/v1/p2p/control"),
        ("POST", "/launcher/v1/p2p/control/:messageId/ack"),
        ("POST", "/launcher/v1/p2p/host-presence"),
        ("POST", "/launcher/v1/p2p/host-offers/:offerId/respond"),
        ("POST", "/launcher/v1/p2p/hunts/:sessionId/observation"),
        ("POST", "/launcher/v1/realm-hosting/hubs/start"),
        ("POST", "/launcher/v1/realm-hosting/hubs/stop"),
        ("GET", "/launcher/v1/realm-hosting/status"),
        ("POST", "/launcher/v1/realms"),
        ("GET", "/launcher/v1/realms/mine"),
        ("GET", "/launcher/v1/realms/listed"),
        ("GET", "/launcher/v1/realms/by-name/:name"),
        ("GET", "/launcher/v1/realms/invite/:code"),
        ("POST", "/launcher/v1/realms/join"),
        ("POST", "/launcher/v1/realms/by-name/:name/leave"),
        ("POST", "/launcher/v1/realms/mine/settings"),
        ("POST", "/launcher/v1/realms/mine/invites"),
        ("GET", "/launcher/v1/realms/mine/invites"),
        ("POST", "/launcher/v1/realms/mine/invites/:code/revoke"),
        ("POST", "/launcher/v1/realms/mine/members/:accountId/kick"),
        ("POST", "/launcher/v1/realms/mine/members/:accountId/ban"),
        ("POST", "/launcher/v1/realms/mine/members/:accountId/unban"),
        ("POST", "/launcher/v1/realms/mine/heartbeat"),
        ("POST", "/launcher/v1/realms/mine/release"),
        ("GET", "/launcher/v1/realms/mine/requests"),
        ("POST", "/launcher/v1/realms/mine/requests/:accountId/approve"),
        ("POST", "/launcher/v1/realms/mine/requests/:accountId/deny"),
        ("POST", "/launcher/v1/realms/mine/members/add"),
        ("POST", "/launcher/v1/realms/mine/player-invites"),
        ("POST", "/launcher/v1/realms/mine/player-invites/:accountId/cancel"),
        ("GET", "/launcher/v1/realms/player-invites"),
        ("POST", "/launcher/v1/realms/player-invites/:name/accept"),
        ("POST", "/launcher/v1/realms/player-invites/:name/decline"),
        ("POST", "/launcher/v1/realms/presence/heartbeat"),
        ("POST", "/launcher/v1/realms/presence/leave"),
        ("GET", "/launcher/v1/realms/presence"),
        ("ALL", "/launcher/v1/realms/*"),
        ("POST", "/api/matchmaker/resolve-hunt-launch-manifest"),
    ];

    #[test]
    fn the_table_in_mount_order() {
        let actual: Vec<(&str, &str)> = ROUTES.iter().map(|r| (r.method.as_str(), r.path)).collect();
        assert_eq!(actual, EXPECTED);
        let mounts: Vec<Mount> = ROUTES.iter().map(|r| r.mount).collect();
        assert!(mounts[..5].iter().all(|m| *m == Mount::Early));
        assert_eq!(mounts[5], Mount::Admin);
        assert!(mounts[6..45].iter().all(|m| *m == Mount::Normal));
        assert_eq!(mounts[45], Mount::Director);
        let mut handlers: Vec<&str> = ROUTES.iter().map(|r| r.handler).collect();
        assert!(handlers.iter().all(|h| h.starts_with("realms.")));
        handlers.sort_unstable();
        handlers.dedup();
        assert_eq!(handlers.len(), ROUTES.len());
        let catch_all = ROUTES.iter().position(|r| r.path == "/launcher/v1/realms/*").unwrap();
        assert!(
            ROUTES[..catch_all]
                .iter()
                .any(|r| r.path.starts_with("/launcher/v1/realms"))
        );
        assert!(
            !ROUTES[catch_all + 1..]
                .iter()
                .any(|r| r.path.starts_with("/launcher/v1/realms"))
        );
    }

    fn call(route: Route, caller: Caller, query: &str, body: &str) -> RouteCall {
        RouteCall {
            route,
            params: Vec::new(),
            query: query.to_owned(),
            request: RequestInfo::default(),
            caller,
            body: body.as_bytes().to_vec(),
        }
    }

    fn bare() -> RealmsRoutes {
        RealmsRoutes::new(RealmsRoutesParts {
            directory: None,
            control: None,
            hosting: None,
            capabilities: None,
            clock: Arc::new(SystemClock),
            realms_enabled: true,
        })
    }

    #[test]
    fn every_route_is_unimplemented_with_no_services() {
        let stub = stub_routes();
        let routes = bare();
        assert_eq!(routes.routes(), ROUTES);
        for route in ROUTES {
            let expected = Err(SeamError::unimplemented(Component::Realms, route.handler));
            assert_eq!(
                poll_once(stub.handle(call(*route, Caller::Anonymous, "", ""))),
                Some(expected.clone())
            );
            assert_eq!(
                poll_once(routes.handle(call(*route, Caller::Anonymous, "", ""))),
                Some(expected)
            );
        }
    }

    #[test]
    fn request_helpers() {
        let mut c = call(
            ROUTES[8],
            Caller::Anonymous,
            "&waitMs=5&afterSequence=%31&flag&waitMs=9",
            "",
        );
        assert_eq!(query_param(&c, "afterSequence"), Some("%31"));
        assert_eq!(query_param(&c, "waitMs"), Some("5"));
        assert_eq!(query_param(&c, "flag"), Some(""));
        assert_eq!(query_param(&c, "missing"), None);
        c.params = vec![("id".into(), "1".into()), ("id".into(), "2".into())];
        assert_eq!(param(&c, "id"), Some("1"));
        assert_eq!(
            launcher_account(&c),
            Err(refused("AUTH_UNAUTHORIZED", "Sign in required."))
        );
        assert_eq!(realms_body(&c), Ok(None));
        assert_eq!(p2p_body(&c), Ok(Map::new()));
        for bad in ["[1]", "3", "{", "\"x\""] {
            c.body = bad.as_bytes().to_vec();
            assert_eq!(
                p2p_body(&c),
                Err(refused("P2P_TICKET_INVALID", "The request body must be a JSON object."))
            );
        }
        c.body = b"{".to_vec();
        assert_eq!(
            realms_body(&c),
            Err(refused(
                "REALM_VALIDATION_FAILED",
                "The request body is not valid JSON."
            ))
        );
        c.body = vec![0xff, b'{'];
        assert_eq!(raw_text(&c), Json(String::new()));
    }

    #[test]
    fn database_text_never_reaches_a_client() {
        assert_eq!(
            to_seam(RealmsError::Database("disk on fire at /var/db".into())),
            SeamError::Refused(Refusal {
                code: "INTERNAL".into(),
                message: "The realms request failed.".into(),
            })
        );
    }
}
