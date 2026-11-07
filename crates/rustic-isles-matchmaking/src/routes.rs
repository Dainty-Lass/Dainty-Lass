// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The 18 backend rows B88-B90, B93-B107 (spec §3, §4): reading the caller, the candidate, QoS and party routes.

use std::sync::Arc;

use axum::extract::ConnectInfo;
use axum::response::{IntoResponse, Response};
use dainty_module_api::network::{HomeError, JoinDecision};
use dainty_module_api::{GlobalId, IdKind};
use dainty_service_kit::json::JSON_UTF8;
use dainty_service_kit::serve::ConnectionInfo;
use http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use rustic_isles::{Call, Handlers, Method, RouteDef, Service, Shared, not_implemented, read_body};
use rustic_isles_auth::AuthData;
use rustic_isles_chat::HubSessions;
use rustic_isles_store::UserId;
use rustic_rifts_seam::AccountId as RealmAccount;
use serde_json::{Map, Value};

use crate::candidate::MatchCandidate;
use crate::controller::{Ctx, JoinParty, JoinRequest, PARTY_ONLINE_WINDOW_MS};
use crate::party::{PLATFORM, PartyError};
use crate::rules::{self, PartyResolution, ResolvableParty};
use crate::startup::QOS_PONG;
use crate::state::MatchmakingState;
use crate::status::StatusAnswer;

const INSTANCE_HEADER: &str = "x-reforged-instance-id";

fn json(status: StatusCode, value: &Value) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, JSON_UTF8)],
        dainty_service_kit::jsjson::stringify(value),
    )
        .into_response()
}

fn ok(value: &Value) -> Response {
    json(StatusCode::OK, value)
}

fn empty_object() -> Value {
    Value::Object(Map::new())
}

/// Express's error page, after the route's error log.
fn page500(route: &RouteDef, error: &str) -> Response {
    tracing::error!("{}: {error}", route.name());
    dainty_service_kit::body::error_page(StatusCode::INTERNAL_SERVER_ERROR)
}

fn object(pairs: Vec<(&str, Value)>) -> Value {
    Value::Object(pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
}

fn header_value(text: &str) -> Option<HeaderValue> {
    HeaderValue::from_str(text).ok()
}

fn with_headers(mut response: Response, headers: Vec<(&'static str, String)>) -> Response {
    for (name, value) in headers {
        if let Some(value) = header_value(&value) {
            response.headers_mut().insert(HeaderName::from_static(name), value);
        }
    }
    response
}

/// The caller as the pipeline left it, else auth's game auth (its refusal answered as it is).
async fn game_auth(call: &mut Call) -> Result<AuthData, Response> {
    if let Some(data) = call.request.extensions().get::<AuthData>() {
        return Ok(data.clone());
    }
    let shared = Arc::clone(&call.shared);
    shared.auth().game_auth(&mut call.request).await
}

fn user_of(route: &RouteDef, data: &AuthData) -> Result<String, Response> {
    match data.user_id.as_deref().filter(|u| !u.is_empty()) {
        Some(user) => Ok(user.to_owned()),
        None => Err(page500(route, "no userId in AuthData")),
    }
}

async fn signed_in(call: &mut Call) -> Result<String, Response> {
    let data = game_auth(call).await?;
    user_of(call.route, &data)
}

fn forwarded(headers: &HeaderMap) -> Option<String> {
    let values: Vec<String> = headers
        .get_all("x-forwarded-for")
        .iter()
        .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
        .collect();
    (!values.is_empty()).then(|| values.join(","))
}

fn client_of(call: &Call) -> Option<String> {
    let peer = call
        .request
        .extensions()
        .get::<ConnectInfo<ConnectionInfo>>()
        .map(|info| info.0.peer.ip().to_string());
    rules::client_address(peer.as_deref(), forwarded(call.request.headers()).as_deref())
}

fn ctx_of(call: &Call) -> Result<Ctx, Response> {
    let Some(state) = call.shared.slot::<MatchmakingState>() else {
        return Err(page500(call.route, "no matchmaking state"));
    };
    state.bind_shared(&call.shared);
    Ok(Ctx {
        shared: Arc::clone(&call.shared),
        state,
        request_id: Some(call.request_id.clone()),
        client: client_of(call),
    })
}

/// Only with rifts loaded, on a detached task the request never awaits.
fn touch_realm_presence(shared: &Arc<Shared>, user: &str) {
    if !shared.rifts().loaded() {
        return;
    }
    let (shared, account) = (Arc::clone(shared), RealmAccount(user.to_owned()));
    tokio::spawn(async move {
        if let Err(e) = shared.rifts().entry().touch_presence(account.clone()).await {
            tracing::warn!("[Realms] presence touch failed for {account}: {e}");
        }
    });
}

/// After a leave or a kick: a forming player-hosted or realm hunt stops waiting for him.
pub(crate) async fn member_left(shared: &Shared, user: &str) {
    if let Err(e) = shared
        .rifts()
        .realms()
        .party_member_left(RealmAccount(user.to_owned()))
        .await
    {
        tracing::warn!("[PartyLeave] could not update the forming hunt of {user}: {e}");
    }
}

fn party_refusal(call: &Call, error: PartyError) -> Response {
    match error {
        PartyError::NoDatabase => not_implemented(call.route, &call.request_id),
        PartyError::Failed(text) => page500(call.route, &text),
    }
}

macro_rules! answer {
    ($e:expr) => {
        match $e {
            Ok(value) => value,
            Err(response) => return response,
        }
    };
}

// -------------------------------------------------------------------- candidates

async fn register_player(mut call: Call) -> Response {
    let data = answer!(game_auth(&mut call).await);
    let user = data.user_id.as_deref().unwrap_or("undefined");
    tracing::info!("userId {user} is registering for matchmaking!");
    ok(&empty_object())
}

async fn alive(mut call: Call) -> Response {
    let data = answer!(game_auth(&mut call).await);
    if let Some(user) = data.user_id.as_deref().filter(|u| !u.is_empty()) {
        touch_realm_presence(&call.shared, user);
        if let Some(state) = call.shared.slot::<MatchmakingState>() {
            let now = state.now_ms();
            state.presence().touch(user, now);
        }
    }
    ok(&empty_object())
}

async fn regions(mut call: Call) -> Response {
    let _ = answer!(game_auth(&mut call).await);
    tracing::info!("Querying regions for QoS");
    let target = call
        .shared
        .settings()
        .raw("QOS_TARGET_URL")
        .map_or(Value::Null, Value::from);
    let payload = object(vec![
        ("maxPingingStepTime", 3.into()),
        ("pingCount", 5.into()),
        ("pingFrequency", 0.25.into()),
        ("regionUrls", Value::Array(vec![target])),
    ]);
    ok(&object(vec![
        ("code", 200.into()),
        ("message", "success".into()),
        ("payload", payload),
    ]))
}

async fn qos(_call: Call) -> Response {
    tracing::info!("QoS Ping");
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        QOS_PONG,
    )
        .into_response()
}

async fn status(mut call: Call) -> Response {
    let user = answer!(signed_in(&mut call).await);
    touch_realm_presence(&call.shared, &user);
    let ctx = answer!(ctx_of(&call));
    let candidate = match ctx.poll(&user).await {
        Ok(candidate) => candidate,
        Err(e) => return page500(call.route, &e),
    };
    let StatusAnswer { status, body } = ctx.status_answer(&user, candidate.as_ref()).await;
    if let Some(session) = body
        .as_ref()
        .and_then(|b| b.pointer("/serverInfo/gameSessionId"))
        .and_then(Value::as_str)
        .filter(|id| ctx.state.is_hub_game_session_id(id))
    {
        record_hub_session(&ctx, &user, session).await;
    }
    let status = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    match body {
        Some(body) => json(status, &body),
        None => status.into_response(),
    }
}

/// Opens the region room of the hub the player was sent to (chat §5.10); a ring-assigned hub never gets here.
async fn record_hub_session(ctx: &Ctx, user: &str, session: &str) {
    let (Some(store), Some(hubs)) = (ctx.store(), ctx.shared.slot::<HubSessions>()) else {
        return;
    };
    let key = UserId::new(user);
    match store
        .read(move |conn, repos| repos.accounts.find_by_user_id(conn, &key))
        .await
    {
        Ok(Some(account)) => hubs.record(account.id.global(), session, &ctx.shared.node().region, ctx.now()),
        Ok(None) => {}
        Err(e) => tracing::warn!("hub session of {user} not recorded: {e}"),
    }
}

fn string_field<'a>(body: &'a Value, name: &str) -> Option<&'a str> {
    body.get(name).and_then(Value::as_str)
}

/// DN6: a join that names a ring instance; none goes on to the matchmaking join.
async fn ring_join(ctx: &Ctx, user: &str, body: &Value, echo: Option<&Value>) -> Option<Response> {
    let raw = ["instanceId", "playerHuntId", "gameArgs"]
        .iter()
        .find_map(|name| string_field(body, name))?;
    let target = raw
        .parse::<GlobalId>()
        .ok()
        .filter(|id| matches!(id.kind(), IdKind::Hunt | IdKind::HubInstance | IdKind::Party))?;
    let account = ctx.resolve_account(user).await?;
    let address = ctx.client.as_deref().unwrap_or("127.0.0.1");
    let home = ctx.state.home(&ctx.shared);
    match home.ring_resolve_join(&account, address, &target).await {
        Ok(JoinDecision::Local(record)) => {
            let id = rustic_isles::replay::next_uuid();
            let candidate = MatchCandidate {
                ready: true,
                host: record.server_host.clone(),
                port: i64::from(record.server_port),
                game_session_id: record.id.to_string(),
                ..MatchCandidate::matching(&record.queue, &id)
            };
            ctx.state.set_candidate(user, candidate);
            let mut pairs = vec![
                ("candidateId", Value::from(id)),
                ("candidateStatusPeriodMillis", 1000.into()),
            ];
            if let Some(mode) = echo {
                pairs.push(("gameMode", mode.clone()));
            }
            pairs.extend([
                ("huntId", Value::from(record.queue.clone())),
                ("instanceId", Value::from(record.id.to_string())),
                ("status", "MATCHING".into()),
                ("statusReason", Value::Null),
            ]);
            Some(with_headers(
                ok(&object(pairs)),
                vec![(INSTANCE_HEADER, record.id.to_string())],
            ))
        }
        Ok(JoinDecision::Refused {
            instance,
            owner,
            region,
            message,
        }) => {
            let body = object(vec![
                ("error", "WRONG_NODE".into()),
                ("message", message.into()),
                ("instance", instance.to_string().into()),
                ("hostNode", owner.to_string().into()),
                ("region", region.to_string().into()),
            ]);
            Some(json(StatusCode::CONFLICT, &body))
        }
        Ok(JoinDecision::Redirect {
            instance,
            owner,
            gateway,
            ticket,
        }) => {
            let gateway_json = serde_json::to_value(&gateway).unwrap_or(Value::Null);
            let body = object(vec![
                ("status", "REDIRECT".into()),
                ("instance", instance.to_string().into()),
                ("hostNode", owner.to_string().into()),
                ("region", gateway.region.to_string().into()),
                ("gateway", gateway_json),
                ("sessionTicket", ticket.token.clone().into()),
            ]);
            let mut headers = vec![
                ("x-reforged-redirect-host", gateway.host.clone()),
                ("x-reforged-redirect-port", gateway.port.to_string()),
            ];
            if let Some(fingerprint) = &gateway.fingerprint {
                headers.push(("x-reforged-redirect-fingerprint", fingerprint.clone()));
            }
            // The ticket goes to the caller only, never to a log
            headers.push(("x-reforged-session-ticket", ticket.token));
            Some(with_headers(ok(&body), headers))
        }
        Err(HomeError::NotInNetwork) => None,
        Err(e) => {
            tracing::warn!("ring_resolve_join for {target} failed: {e}");
            None
        }
    }
}

fn shown(text: Option<&str>) -> String {
    text.map_or_else(|| "undefined".to_owned(), rules::log_text)
}

async fn join(mut call: Call) -> Response {
    let user = answer!(signed_in(&mut call).await);
    touch_realm_presence(&call.shared, &user);
    let body = answer!(read_body(&mut call.request).await);
    let Some(body) = body else {
        return page500(call.route, "req.body is undefined");
    };
    let ctx = answer!(ctx_of(&call));
    let echo = body.get("gameMode").cloned();
    let mode = string_field(&body, "gameMode").map(str::to_owned);
    let args = string_field(&body, "gameArgs").map(str::to_owned);
    let hunt = string_field(&body, "playerHuntId").map(str::to_owned);
    let build_id = string_field(&body, "buildId").map(str::to_owned);
    let requested = string_field(&body, "partyId")
        .filter(|id| !dainty_service_kit::js::trim(id).is_empty())
        .map(str::to_owned);
    if let Some(response) = ring_join(&ctx, &user, &body, echo.as_ref()).await {
        return response;
    }
    tracing::info!(
        "UserId {user} wants to join a game with GameMode {} & GameArgs {} & HuntId {} & gameType={} privateMatch={} isPrivate={} partyId={}",
        shown(mode.as_deref()),
        shown(args.as_deref()),
        shown(hunt.as_deref()),
        rules::field_shape(body.get("gameType")),
        rules::field_shape(body.get("privateMatch")),
        rules::field_shape(body.get("isPrivate")),
        if requested.is_some() { "present" } else { "absent" },
    );
    let store = ctx.store();
    let party = match ctx.parties(store.as_deref()).for_player(&user).await {
        Ok(party) => party,
        Err(e) => return party_refusal(&call, e),
    };
    let filter = match ctx
        .shared
        .rifts()
        .realms()
        .filters_offline_party_members(mode.as_deref().unwrap_or(""), hunt.as_deref())
    {
        Ok(filter) => filter,
        Err(e) => return page500(call.route, &e.to_string()),
    };
    let now = ctx.now();
    let online = |member: &str| {
        ctx.shared.presence().is_online(member)
            || ctx
                .state
                .presence()
                .is_recently_active(member, PARTY_ONLINE_WINDOW_MS, now)
    };
    let members: Vec<String> = party
        .as_ref()
        .map(|p| p.members.iter().map(|m| m.user_id.as_str().to_owned()).collect())
        .unwrap_or_default();
    let resolvable = party.as_ref().map(|p| ResolvableParty {
        party_id: &p.party_id,
        members: &members,
        revision: p.revision,
    });
    let resolution: PartyResolution =
        rules::resolve_matchmaking_party(mode.as_deref(), &user, requested.as_deref(), resolvable, online, filter);
    if let Some(record) = party.as_ref() {
        if resolution.mismatch && record.members.len() > 1 {
            tracing::warn!(
                "UserId {user} requested solo/stale-party ISLAND travel; client partyId={} does not match authoritative partyId={}. Launching for this player only.",
                requested.as_deref().unwrap_or("<missing>"),
                record.party_id
            );
        } else if !resolution.excluded.is_empty() {
            tracing::warn!(
                "UserId {user} {} party contains {} offline member(s); launching with {} active member(s). excluded={}",
                mode.as_deref().unwrap_or("undefined"),
                resolution.excluded.len(),
                resolution.members.as_ref().map_or(0, Vec::len),
                resolution.excluded.join(",")
            );
        }
    }
    let global_id = resolution.party_id.as_ref().and(party.as_ref()).map(|p| p.id.clone());
    let request = JoinRequest {
        player: user.clone(),
        mode,
        args,
        hunt,
        build_id,
        party: JoinParty {
            party_id: resolution.party_id,
            members: resolution.members,
            revision: resolution.revision,
            global_id,
        },
    };
    match ctx.join(&request).await {
        Ok(true) => {}
        Ok(false) => return StatusCode::BAD_REQUEST.into_response(),
        Err(e) => return page500(call.route, &e),
    }
    let candidate = match ctx.poll(&user).await {
        Ok(Some(candidate)) => candidate,
        Ok(None) => return page500(call.route, "no candidate after a join"),
        Err(e) => return page500(call.route, &e),
    };
    let mut pairs = vec![
        ("candidateId", Value::from(candidate.candidate_id.clone())),
        (
            "candidateStatusPeriodMillis",
            candidate.status_period_millis(ctx.now()).into(),
        ),
    ];
    if let Some(mode) = echo {
        pairs.push(("gameMode", mode));
    }
    pairs.extend([
        ("huntId", Value::from(candidate.hunt_id.clone())),
        ("status", "MATCHING".into()),
        ("statusReason", Value::Null),
    ]);
    let response = ok(&object(pairs));
    if candidate.is_ring_assigned() {
        return with_headers(response, vec![(INSTANCE_HEADER, candidate.game_session_id)]);
    }
    response
}

async fn leave(mut call: Call) -> Response {
    let user = answer!(signed_in(&mut call).await);
    let body = answer!(read_body(&mut call.request).await);
    let keys = rules::body_keys(body.as_ref());
    let ctx = answer!(ctx_of(&call));
    if ctx.store().is_none() {
        return not_implemented(call.route, &call.request_id);
    }
    let method = call.request.method().as_str().to_owned();
    let path = call.request.uri().path().to_owned();
    match ctx.cancel_for_leave(&user).await {
        Ok(done) => {
            tracing::info!(
                "[CandidateLeave] user={user} method={method} path={path} bodyKeys=[{keys}] candidate={} state={} -> {}",
                done.candidate_id.as_deref().unwrap_or("none"),
                done.state,
                done.outcome
            );
            ok(&empty_object())
        }
        Err(e) => {
            tracing::error!(
                "[CandidateLeave] user={user} method={method} path={path} bodyKeys=[{keys}] candidate=none state=error -> error: {e}"
            );
            // Not the 500 page: Node answered `{}` (spec §15 item 2)
            json(StatusCode::INTERNAL_SERVER_ERROR, &empty_object())
        }
    }
}

// -------------------------------------------------------------------- parties

fn build_id(body: Option<&Value>) -> String {
    body.and_then(|b| string_field(b, "buildId")).unwrap_or("").to_owned()
}

async fn party(mut call: Call) -> Response {
    let user = answer!(signed_in(&mut call).await);
    let body = answer!(read_body(&mut call.request).await);
    let build = build_id(body.as_ref());
    let ctx = answer!(ctx_of(&call));
    let store = ctx.store();
    let parties = ctx.parties(store.as_deref());
    let record = match parties.get_or_create(&user, &build).await {
        Ok(record) => record,
        Err(e) => return party_refusal(&call, e),
    };
    let instance = match ctx.party_instance(&record.party_id, Some(&record.id)).await {
        Ok(instance) => instance,
        Err(e) => return page500(call.route, &e),
    };
    let own = match ctx.player_candidate(&user).await {
        Ok(own) => own,
        Err(e) => return page500(call.route, &e),
    };
    let now = ctx.now();
    let state = match &instance {
        Some(i) if i.is_client_ready(now) => "IN_PROGRESS",
        Some(_) => "MATCHING",
        None => "QUEUED_FOR_START",
    };
    let shown_candidate = own.as_ref().or(instance.as_ref());
    let mut players = Vec::with_capacity(record.members.len());
    for member in &record.members {
        let id = member.user_id.as_str();
        let name = match parties.username(id).await {
            Ok(name) => name,
            Err(e) => return party_refusal(&call, e),
        };
        let member_of = instance.is_some() || id == record.leader.user_id.as_str();
        players.push(object(vec![
            ("consoleSessionId", Value::Null),
            ("displayName", name.into()),
            ("isMemberOfCandidate", member_of.into()),
            ("platform", PLATFORM.into()),
            ("playerId", id.into()),
        ]));
    }
    let body = object(vec![
        (
            "candidateId",
            shown_candidate.map_or(Value::Null, |c| c.candidate_id.clone().into()),
        ),
        ("candidateState", state.into()),
        ("gauntletLevel", Value::Null),
        ("leaderPlayerId", record.leader.user_id.as_str().into()),
        ("partyId", record.party_id.clone().into()),
        (
            "playerHuntId",
            instance.as_ref().map_or(Value::Null, |c| c.hunt_id.clone().into()),
        ),
        ("playerStates", Value::Array(players)),
    ]);
    // Node's own header name, a wire fact (spec §15 item 1)
    with_headers(
        ok(&body),
        vec![("x-mystic-party-revision", record.revision.to_string())],
    )
}

async fn invite(mut call: Call) -> Response {
    let user = answer!(signed_in(&mut call).await);
    let body = answer!(read_body(&mut call.request).await);
    let build = build_id(body.as_ref());
    let recipient = body
        .as_ref()
        .and_then(|b| string_field(b, "recipientPlayerId"))
        .filter(|r| !r.is_empty())
        .map(str::to_owned);
    // Without a recipient nothing is read or written, so this answers `{}` even with no database
    if let Some(recipient) = recipient {
        let ctx = answer!(ctx_of(&call));
        let store = ctx.store();
        let parties = ctx.parties(store.as_deref());
        let sent = async {
            let name = parties.username(&user).await?;
            parties.invite(&user, &recipient, &build, &name, PLATFORM).await
        };
        if let Err(e) = sent.await {
            return party_refusal(&call, e);
        }
        tracing::info!("{user} invited {recipient} to their party");
    }
    ok(&empty_object())
}

async fn invites(mut call: Call) -> Response {
    let user = answer!(signed_in(&mut call).await);
    let ctx = answer!(ctx_of(&call));
    let store = ctx.store();
    let listed = match ctx.parties(store.as_deref()).invites_for(&user).await {
        Ok(listed) => listed,
        Err(e) => return party_refusal(&call, e),
    };
    let invitations = listed
        .into_iter()
        .map(|i| {
            object(vec![
                ("partyId", i.party_id.into()),
                ("recipientPlayerId", user.clone().into()),
                ("sendingDisplayName", i.display_name.into()),
                ("sendingPlatform", i.platform.into()),
                ("sendingPlayerId", i.sender_user_id.into()),
            ])
        })
        .collect();
    ok(&object(vec![("invitations", Value::Array(invitations))]))
}

fn param(call: &Call, name: &str) -> String {
    call.params.get(name).unwrap_or("").to_owned()
}

async fn accept(mut call: Call) -> Response {
    let user = answer!(signed_in(&mut call).await);
    let body = answer!(read_body(&mut call.request).await);
    let build = build_id(body.as_ref());
    let inviter = param(&call, "inviterId");
    let ctx = answer!(ctx_of(&call));
    let store = ctx.store();
    if let Err(e) = ctx.parties(store.as_deref()).accept(&user, &inviter, &build).await {
        return party_refusal(&call, e);
    }
    tracing::info!("{user} accepted party invite from {inviter}");
    ok(&empty_object())
}

async fn leave_party(mut call: Call) -> Response {
    let user = answer!(signed_in(&mut call).await);
    let ctx = answer!(ctx_of(&call));
    let store = ctx.store();
    if let Err(e) = ctx.parties(store.as_deref()).leave(&user).await {
        return party_refusal(&call, e);
    }
    member_left(&ctx.shared, &user).await;
    tracing::info!("{user} left their party");
    ok(&empty_object())
}

async fn kick(mut call: Call) -> Response {
    let user = answer!(signed_in(&mut call).await);
    let target = param(&call, "targetId");
    let ctx = answer!(ctx_of(&call));
    let store = ctx.store();
    match ctx.parties(store.as_deref()).kick(&user, &target).await {
        // Only a departure stops a forming hunt waiting (a departure from Node, spec §15 item 8)
        Ok(true) => member_left(&ctx.shared, &target).await,
        Ok(false) => {}
        Err(e) => return party_refusal(&call, e),
    }
    tracing::info!("{user} kicked {target} from the party");
    ok(&empty_object())
}

async fn promote(mut call: Call) -> Response {
    let user = answer!(signed_in(&mut call).await);
    let target = param(&call, "targetId");
    let ctx = answer!(ctx_of(&call));
    let store = ctx.store();
    if let Err(e) = ctx.parties(store.as_deref()).promote(&user, &target).await {
        return party_refusal(&call, e);
    }
    tracing::info!("{user} promoted {target} to party leader");
    ok(&empty_object())
}

/// The 18 handlers, paths exactly as core's rows.
pub fn register(handlers: &mut Handlers) {
    use Method::{Delete, Get, Post, Put};
    let b = Service::Backend;
    handlers.on(b, Post, "/candidate/player/register", register_player);
    handlers.on(b, Post, "/candidate/player/alive", alive);
    handlers.on(b, Get, "/candidate/regions", regions);
    handlers.on(b, Get, "/candidate/status", status);
    handlers.on(b, Post, "/candidate/join", join);
    // One handler; the 1.14.7 client sends `DELETE /candidate/leave` from the leader only (FUN_140db6dd0), and the
    // other shapes are kept until a log shows which one it sends
    for (method, path) in [
        (Delete, "/candidate/leave"),
        (Post, "/candidate/leave"),
        (Put, "/candidate/leave"),
        (Delete, "/candidate"),
        (Delete, "/candidate/:candidateId"),
    ] {
        handlers.on(b, method, path, leave);
    }
    handlers.on(b, Get, "/QoS", qos);
    handlers.on(b, Post, "/party", party);
    handlers.on(b, Put, "/party/invite", invite);
    handlers.on(b, Get, "/party/invites", invites);
    handlers.on(b, Put, "/party/invite/accept/:inviterId", accept);
    handlers.on(b, Delete, "/party/member", leave_party);
    handlers.on(b, Delete, "/party/member/:targetId", kick);
    handlers.on(b, Put, "/party/member/promote/:targetId", promote);
}
