//! RU5's `MatchmakingParties` group: multipliers, matchmaking, parties and the game servers' posts. RU3c: hook
//! `candidate_destination`; tables `TARGET_CHANGELIST` and `GAME_VERSION` (the `buildId`, `VersionTables::build_id`);
//! `QOS_PORT`'s listener is a public one, so the module answers on loopback and the gateway publishes it ([`qos`]).
//!
//! Matchmaking tickets, candidates and presence stay in memory (DD15, [`state`]); parties are on `store` under global
//! ids (DD18, `repo::parties`). The Director is reached through [`deploy::DedicatedServers`] (RU6 supplies the
//! other end of `POST /api/matchmaker/handle-matchmaking-for-player`); realms through `rifts_seam`'s hooks; the
//! realms side calls back through [`rifts_core::MatchmakingCore`].
//!
//! Authentication (`HasParadoxBackendAuth`) is RU5 auth's: [`authenticate`] reads the [`AuthData`] its middleware leaves
//! in the request's extensions. Body parsing is the pipeline's (`service_kit::body`); [`read_body`] does the same
//! until the pipeline has a layer for it.

// An `Err` here is the whole HTTP answer, as Express's middleware chain ends a request
#![allow(clippy::result_large_err)]

pub mod controller;
pub mod deploy;
pub mod multipliers;
pub mod outbox;
pub mod party;
pub mod presence;
pub mod qos;
pub mod resolve;
pub mod rifts_core;
pub mod state;
pub mod status;
#[doc(hidden)]
pub mod testkit;
pub mod util;

use axum::body::{Body, to_bytes};
use axum::extract::{ConnectInfo, Request};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use module_api::BoxFuture;
use serde_json::{Map, Value, json};
use service_kit::body::{BodyError, error_page, parse_form, parse_json};
use service_kit::json::Json;
use service_kit::serve::ConnectionInfo;

use self::controller::{JoinRequest, LeaveParty, MatchError, Matchmaker};
use self::party::{PLATFORM, Parties, PartyError};
use self::resolve::{PartySnapshot, resolve_matchmaking_party};
use crate::http::{Call, Handlers, Shared, not_implemented};
use crate::repo::UserId;
use crate::routes::{Method, Service};

/// How recently a party member's game must have touched the backend to follow the leader into a hunt: 2.5 realm
/// presence heartbeats (every 30 s). Shorter than the release gate's 2 minutes, so a member who just closed the game
/// is not frozen into a roster.
const PARTY_MEMBER_ACTIVE_WINDOW_MS: i64 = 75_000;
/// Express's default JSON limit
const BODY_LIMIT: usize = 100 * 1024;

/// `req.AuthData`, which `HasParadoxBackendAuth` leaves on the request.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AuthData {
    pub user_id: Option<String>,
    pub is_gameserver: bool,
    pub claims: Map<String, Value>,
}

/// `HasParadoxBackendAuth`'s verdict, as it reaches this area: the data it left, or no such data, which answers 401
/// with an empty body as the middleware does.
pub fn authenticate(call: &Call) -> Result<AuthData, Response> {
    call.request
        .extensions()
        .get::<AuthData>()
        .cloned()
        .ok_or_else(|| StatusCode::UNAUTHORIZED.into_response())
}

/// An Express failure with no handler of its own: the default error page.
fn server_error(error: &dyn std::fmt::Display, what: &str) -> Response {
    tracing::error!("{what}: {error}");
    error_page(StatusCode::INTERNAL_SERVER_ERROR)
}

fn match_error(error: &MatchError, call: &Call) -> Response {
    server_error(error, &call.route.name())
}

fn party_error(error: &PartyError, call: &Call) -> Response {
    match error {
        // A module started with no database answers as the skeleton does
        PartyError::NoDatabase => not_implemented(call.route, &call.request_id),
        other => server_error(other, &call.route.name()),
    }
}

/// `express.json` / `express.urlencoded`: the request's body as `req.body`, or `None` when no parser applied.
pub async fn read_body(request: &mut Request) -> Result<Option<Value>, Response> {
    let content_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let Some(content_type) = content_type else {
        return Ok(None);
    };
    let mut parts = content_type.split(';');
    let essence = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
    let json = match essence.as_str() {
        "application/json" => true,
        "application/x-www-form-urlencoded" => false,
        _ => return Ok(None),
    };
    let charset = parts
        .filter_map(|p| p.split_once('='))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("charset"))
        .map(|(_, value)| value.trim().trim_matches('"').to_ascii_lowercase());
    if !matches!(charset.as_deref(), None | Some("utf-8")) {
        return Err(BodyError::UnsupportedCharset.into_response());
    }
    let body = std::mem::replace(request.body_mut(), Body::empty());
    let raw = to_bytes(body, BODY_LIMIT)
        .await
        .map_err(|_| BodyError::TooLarge.into_response())?;
    let text = std::str::from_utf8(&raw).map_err(|_| BodyError::Malformed.into_response())?;
    let parsed = if json { parse_json(text) } else { parse_form(text) };
    parsed.map(Some).map_err(IntoResponse::into_response)
}

fn string_field(body: Option<&Value>, key: &str) -> Option<String> {
    body?.get(key)?.as_str().map(str::to_owned)
}

/// The `userId` of a request that needs one.
fn user_of(auth: &AuthData, call: &Call) -> Result<String, Response> {
    auth.user_id
        .clone()
        .ok_or_else(|| server_error(&"no userId in AuthData", &call.route.name()))
}

fn empty_object() -> Response {
    Json(json!({})).into_response()
}

fn matchmaker(call: &Call) -> Matchmaker<'_> {
    Matchmaker {
        shared: &call.shared,
        request_id: Some(call.request_id.clone()),
    }
}

/// `TouchRealmPresence`: a realm member's matchmaking request keeps its realm presence alive. Only when the realms
/// module is on; the touch never delays or fails the request.
fn touch_realm_presence(shared: &Shared, user: &str) {
    if !shared.rifts.loaded() {
        return;
    }
    let (rifts, account) = (shared.rifts.clone(), rifts_seam::AccountId(user.to_owned()));
    tokio::spawn(async move {
        if let Err(error) = rifts.entry().touch_presence(account.clone()).await {
            tracing::warn!("[Realms] presence touch failed for {account}: {error}");
        }
    });
}

async fn player_register(call: Call) -> Response {
    let auth = match authenticate(&call) {
        Ok(auth) => auth,
        Err(refusal) => return refusal,
    };
    tracing::info!(
        "userId {} is registering for matchmaking!",
        auth.user_id.as_deref().unwrap_or("undefined")
    );
    empty_object()
}

/// The client fires this every ~5 s while in matchmaking or in a game. It extends the activity monitor's playtime
/// (gap rule in `activity/activityRules.ts`) and keeps a realm member's presence alive.
async fn alive(call: Call) -> Response {
    let auth = match authenticate(&call) {
        Ok(auth) => auth,
        Err(refusal) => return refusal,
    };
    if let Some(user) = auth.user_id.as_deref() {
        touch_realm_presence(&call.shared, user);
        call.shared
            .matchmaking
            .presence
            .touch(user, call.shared.matchmaking.clock.now_ms());
    }
    empty_object()
}

async fn regions(call: Call) -> Response {
    if let Err(refusal) = authenticate(&call) {
        return refusal;
    }
    tracing::info!("Querying regions for QoS");
    let target = call
        .shared
        .settings
        .raw("QOS_TARGET_URL")
        .map_or(Value::Null, |url| json!(url));
    Json(json!({
        "code": 200,
        "message": "success",
        "payload": {
            "maxPingingStepTime": 3,
            "pingCount": 5,
            "pingFrequency": 0.25,
            "regionUrls": [target]
        }
    }))
    .into_response()
}

/// `RecordHubSessionServed`: the chat rules (DD21) let an account into its region's room once matchmaking has sent it to
/// a hub there. The hub is this node's, so the region is the node's.
async fn record_hub_session(shared: &Shared, user: &str, game_session_id: &str) {
    let Some(store) = &shared.store else { return };
    let id = UserId(user.to_owned());
    match store
        .read(move |conn, repos| repos.accounts.find_by_user_id(conn, &id))
        .await
    {
        Ok(Some(account)) => {
            let now = u64::try_from(shared.matchmaking.clock.now_ms()).unwrap_or_default();
            isles_chat::hub_sessions().record(account.id.global(), game_session_id, &shared.node.region, now);
        }
        Ok(None) => {}
        Err(error) => tracing::warn!("hub session of {user} not recorded: {error}"),
    }
}

async fn status(call: Call) -> Response {
    let auth = match authenticate(&call) {
        Ok(auth) => auth,
        Err(refusal) => return refusal,
    };
    let user = match user_of(&auth, &call) {
        Ok(user) => user,
        Err(failure) => return failure,
    };
    touch_realm_presence(&call.shared, &user);
    let matchmaker = matchmaker(&call);
    let result = match matchmaker.check_and_update_queue_status(&user).await {
        Ok(result) => result,
        Err(error) => return match_error(&error, &call),
    };
    let forwarded = call
        .request
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok());
    let peer = call
        .request
        .extensions()
        .get::<ConnectInfo<ConnectionInfo>>()
        .map(|info| info.0.peer.ip());
    let ip = status::client_ip(peer, forwarded);
    let answer = status::candidate_status_answer(&call.shared, &user, result.as_ref(), ip.as_deref()).await;
    // [Reforged 0025] The client joins City-<gameSessionId> for a dedicated hub; remember which one.
    let state = &call.shared.matchmaking;
    if let Some(session) = answer
        .body
        .as_ref()
        .and_then(|b| b.pointer("/serverInfo/gameSessionId"))
        .and_then(Value::as_str)
        && state.is_hub_game_session_id(session)
    {
        record_hub_session(&call.shared, &user, session).await;
    }
    let code = StatusCode::from_u16(answer.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    match answer.body {
        None => code.into_response(),
        Some(body) => (code, Json(body)).into_response(),
    }
}

/// `RequestShape`: booleans and short strings only, never a party id.
fn request_shape(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => {
            serde_json::to_string(&text.chars().take(40).collect::<String>()).unwrap_or_default()
        }
        Some(Value::Bool(flag)) => flag.to_string(),
        Some(Value::Number(number)) => number.to_string(),
        _ => "absent".to_owned(),
    }
}

async fn join(mut call: Call) -> Response {
    let auth = match authenticate(&call) {
        Ok(auth) => auth,
        Err(refusal) => return refusal,
    };
    let user = match user_of(&auth, &call) {
        Ok(user) => user,
        Err(failure) => return failure,
    };
    touch_realm_presence(&call.shared, &user);
    let body = match read_body(&mut call.request).await {
        Ok(body) => body,
        Err(failure) => return failure,
    };
    // Express 5 leaves req.body undefined without a parsed body, and the handler's first read throws
    let Some(body) = body else {
        return server_error(&"req.body is undefined", &call.route.name());
    };
    let game_mode = body.get("gameMode").cloned();
    let mode = game_mode.as_ref().and_then(Value::as_str).map(str::to_owned);
    let game_args = string_field(Some(&body), "gameArgs");
    let hunt_id = string_field(Some(&body), "playerHuntId");
    let requested = string_field(Some(&body), "partyId").filter(|id| !id.trim().is_empty());
    tracing::info!(
        "UserId {user} wants to join a game with GameMode {} & GameArgs {} & HuntId {} & gameType={} privateMatch={} isPrivate={} partyId={}",
        mode.as_deref().unwrap_or("undefined"),
        game_args.as_deref().unwrap_or("undefined"),
        hunt_id.as_deref().unwrap_or("undefined"),
        request_shape(body.get("gameType")),
        request_shape(body.get("privateMatch")),
        request_shape(body.get("isPrivate")),
        if requested.is_some() { "present" } else { "absent" },
    );

    let parties = Parties::of(&call.shared, Some(call.request_id.clone()));
    let party = match parties.for_player(&user).await {
        Ok(party) => party,
        Err(error) => return party_error(&error, &call),
    };
    let rifts = call.shared.rifts.clone();
    let filter_offline = match rifts
        .realms()
        .filters_offline_party_members(mode.as_deref().unwrap_or_default(), hunt_id.as_deref())
    {
        Ok(filter) => filter,
        Err(error) => return server_error(&error, &call.route.name()),
    };
    // A party member follows the leader as on a dedicated server: online while their game holds a chat connection OR
    // touched the backend within the window (the realm presence heartbeat, the matchmaking heartbeat). A member whose
    // game closed stops touching and drops out after it.
    let state = &call.shared.matchmaking;
    let chat = state.chat.get();
    let now = state.clock.now_ms();
    let snapshot = party.as_ref().map(|p| PartySnapshot {
        party_id: p.party_id.clone(),
        members: p.members.clone(),
        revision: Some(p.revision),
    });
    let resolution = resolve_matchmaking_party(
        mode.as_deref(),
        &user,
        requested.as_deref(),
        snapshot.as_ref(),
        |member| {
            chat.is_online(member)
                || state
                    .presence
                    .is_recently_active(member, PARTY_MEMBER_ACTIVE_WINDOW_MS, now)
        },
        filter_offline,
    );
    if let (true, Some(party)) = (resolution.party_id_mismatch, &party)
        && party.members.len() > 1
    {
        tracing::warn!(
            "UserId {user} requested solo/stale-party ISLAND travel; client partyId={} does not match authoritative partyId={}. Launching for this player only.",
            requested.as_deref().unwrap_or("<missing>"),
            party.party_id
        );
    } else if !resolution.party_id_mismatch && !resolution.excluded_members.is_empty() {
        tracing::warn!(
            "UserId {user} {} party contains {} offline member(s); launching with {} active member(s). excluded={}",
            mode.as_deref().unwrap_or("undefined"),
            resolution.excluded_members.len(),
            resolution.party_members.as_ref().map_or(0, Vec::len),
            resolution.excluded_members.join(",")
        );
    }

    let matchmaker = matchmaker(&call);
    let request = JoinRequest {
        game_mode: mode,
        game_args,
        hunt_id,
        player: user.clone(),
        party_global_id: resolution
            .party_id
            .as_ref()
            .and(party.as_ref())
            .map(|p| p.global_id.to_string()),
        party_id: resolution.party_id,
        party_members: resolution.party_members,
        party_revision: resolution.party_revision,
        build_id: string_field(Some(&body), "buildId"),
    };
    match matchmaker.handle_player_matchmaking(request).await {
        Ok(true) => {}
        Ok(false) => return StatusCode::BAD_REQUEST.into_response(),
        Err(error) => return match_error(&error, &call),
    }
    let entry = match matchmaker.check_and_update_queue_status(&user).await {
        Ok(Some(entry)) => entry,
        Ok(None) => return server_error(&"no candidate after a join", &call.route.name()),
        Err(error) => return match_error(&error, &call),
    };
    let mut answer = Map::new();
    answer.insert("candidateId".into(), json!(entry.candidate_id));
    // Without it the client waited its default 10 s before the first status poll
    answer.insert(
        "candidateStatusPeriodMillis".into(),
        json!(entry.status_period_millis(state.clock.now_ms())),
    );
    if let Some(mode) = game_mode {
        answer.insert("gameMode".into(), mode);
    }
    answer.insert("huntId".into(), json!(entry.hunt_id));
    answer.insert("status".into(), json!("MATCHING"));
    answer.insert("statusReason".into(), Value::Null);
    Json(Value::Object(answer)).into_response()
}

/// The client's Cancel of its search. From the 1.14.7 binary (FUN_140db6dd0): `DELETE /candidate/leave` with no body,
/// and only the party leader sends it. The response the client expects is not known; 200 with `{}` is the safe answer.
/// The other shapes are aliases kept until the log shows which the client sends. Each call logs one `[CandidateLeave]`
/// line with the method, path, body key names (never values) and the outcome.
async fn candidate_leave(mut call: Call) -> Response {
    let auth = match authenticate(&call) {
        Ok(auth) => auth,
        Err(refusal) => return refusal,
    };
    let user = match user_of(&auth, &call) {
        Ok(user) => user,
        Err(failure) => return failure,
    };
    let body_keys = match read_body(&mut call.request).await {
        Ok(body) => logged_body_keys(body.as_ref()),
        Err(failure) => return failure,
    };
    let (method, path) = (call.request.method().clone(), call.request.uri().path().to_owned());
    if call.shared.store.is_none() {
        return not_implemented(call.route, &call.request_id);
    }
    let parties = Parties::of(&call.shared, Some(call.request_id.clone()));
    let party_of = |player: String| -> BoxFuture<'_, MatchResult<Option<LeaveParty>>> {
        let parties = &parties;
        Box::pin(async move {
            let party = parties
                .for_player(&player)
                .await
                .map_err(|e| MatchError(e.to_string()))?;
            Ok(party.map(|p| LeaveParty {
                party_id: p.party_id,
                members: p.members,
            }))
        })
    };
    match matchmaker(&call).cancel_matchmaking_for_leave(&user, &party_of).await {
        Ok(answer) => {
            tracing::info!(
                "[CandidateLeave] user={user} method={method} path={path} bodyKeys=[{body_keys}] candidate={} state={} -> {}",
                answer.candidate_id.as_deref().unwrap_or("none"),
                answer.state,
                answer.outcome.as_str()
            );
            empty_object()
        }
        Err(error) => {
            tracing::error!(
                "[CandidateLeave] user={user} method={method} path={path} bodyKeys=[{body_keys}] candidate=none state=error -> error: {error}"
            );
            (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({}))).into_response()
        }
    }
}

use self::controller::MatchResult;

/// The body key names of a leave, for the log (never the values): at most 12 names, cut to safe characters.
fn logged_body_keys(body: Option<&Value>) -> String {
    let Some(Value::Object(map)) = body else {
        return String::new();
    };
    map.keys()
        .take(12)
        .map(|key| {
            key.chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
                        c
                    } else {
                        '?'
                    }
                })
                .take(40)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join(",")
}

async fn qos(_call: Call) -> Response {
    tracing::info!("QoS Ping");
    // `res.send(string)`
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        )],
        qos::PONG,
    )
        .into_response()
}

// ---- parties (routes/party.ts) ----

/// `POST /party`: the caller's own party state (a solo party is created on the first call). It reflects the party's
/// shared matchmaking state, so members polling this follow the leader into the hunt.
async fn party_state(mut call: Call) -> Response {
    let auth = match authenticate(&call) {
        Ok(auth) => auth,
        Err(refusal) => return refusal,
    };
    let user = match user_of(&auth, &call) {
        Ok(user) => user,
        Err(failure) => return failure,
    };
    let body = match read_body(&mut call.request).await {
        Ok(body) => body,
        Err(failure) => return failure,
    };
    let build_id = string_field(body.as_ref(), "buildId").unwrap_or_default();
    let parties = Parties::of(&call.shared, Some(call.request_id.clone()));
    let party = match parties.get_or_create(&user, &build_id).await {
        Ok(party) => party,
        Err(error) => return party_error(&error, &call),
    };
    let matchmaker = matchmaker(&call);
    let instance = match matchmaker
        .party_instance(&party.party_id, Some(&party.global_id.to_string()))
        .await
    {
        Ok(instance) => instance,
        Err(error) => return match_error(&error, &call),
    };
    let in_hunt = instance.is_some();
    // The party instance carries the shared destination; the candidate map carries this member's own, which followers
    // must see rather than the leader's already-consumed one
    let candidate = match matchmaker.player_candidate(&user).await {
        Ok(candidate) => candidate.or_else(|| instance.clone()),
        Err(error) => return match_error(&error, &call),
    };
    let mut states = Vec::new();
    for member in &party.members {
        let name = match parties.username(member).await {
            Ok(name) => name,
            Err(error) => return party_error(&error, &call),
        };
        states.push(json!({
            "consoleSessionId": null,
            "displayName": name,
            "isMemberOfCandidate": in_hunt || *member == party.leader,
            "platform": PLATFORM,
            "playerId": member,
        }));
    }
    let state = match &instance {
        Some(instance) if instance.ready => "IN_PROGRESS",
        Some(_) => "MATCHING",
        None => "QUEUED_FOR_START",
    };
    let mut response = Json(json!({
        "candidateId": candidate.map(|c| c.candidate_id),
        "candidateState": state,
        "gauntletLevel": null,
        "leaderPlayerId": party.leader,
        "partyId": party.party_id,
        "playerHuntId": instance.map(|i| i.hunt_id),
        "playerStates": states,
    }))
    .into_response();
    if let Ok(value) = HeaderValue::from_str(&party.revision.to_string()) {
        response.headers_mut().insert("x-mystic-party-revision", value);
    }
    response
}

/// `PUT /party/invite`: invite a player to my party (`recipientPlayerId` in the body).
async fn party_invite(mut call: Call) -> Response {
    let auth = match authenticate(&call) {
        Ok(auth) => auth,
        Err(refusal) => return refusal,
    };
    let user = match user_of(&auth, &call) {
        Ok(user) => user,
        Err(failure) => return failure,
    };
    let body = match read_body(&mut call.request).await {
        Ok(body) => body,
        Err(failure) => return failure,
    };
    let build_id = string_field(body.as_ref(), "buildId").unwrap_or_default();
    let parties = Parties::of(&call.shared, Some(call.request_id.clone()));
    if let Some(recipient) = string_field(body.as_ref(), "recipientPlayerId").filter(|r| !r.is_empty()) {
        let result = async {
            let name = parties.username(&user).await?;
            parties.invite(&user, &recipient, &build_id, &name, PLATFORM).await
        }
        .await;
        if let Err(error) = result {
            return party_error(&error, &call);
        }
        tracing::info!("{user} invited {recipient} to their party");
    }
    empty_object()
}

/// `GET /party/invites`: my incoming invites.
async fn party_invites(call: Call) -> Response {
    let auth = match authenticate(&call) {
        Ok(auth) => auth,
        Err(refusal) => return refusal,
    };
    let user = match user_of(&auth, &call) {
        Ok(user) => user,
        Err(failure) => return failure,
    };
    let parties = Parties::of(&call.shared, Some(call.request_id.clone()));
    match parties.invites_for(&user).await {
        Ok(invites) => {
            let invitations: Vec<Value> = invites
                .into_iter()
                .map(|invite| {
                    json!({
                        "partyId": invite.party_id,
                        "recipientPlayerId": user,
                        "sendingDisplayName": invite.sending_display_name,
                        "sendingPlatform": invite.sending_platform,
                        "sendingPlayerId": invite.sending_player_id,
                    })
                })
                .collect();
            Json(json!({ "invitations": invitations })).into_response()
        }
        Err(error) => party_error(&error, &call),
    }
}

/// `PUT /party/invite/accept/:inviterId`.
async fn party_accept(mut call: Call) -> Response {
    let auth = match authenticate(&call) {
        Ok(auth) => auth,
        Err(refusal) => return refusal,
    };
    let user = match user_of(&auth, &call) {
        Ok(user) => user,
        Err(failure) => return failure,
    };
    let body = match read_body(&mut call.request).await {
        Ok(body) => body,
        Err(failure) => return failure,
    };
    let inviter = call.params.get("inviterId").unwrap_or_default().to_owned();
    let build_id = string_field(body.as_ref(), "buildId").unwrap_or_default();
    let parties = Parties::of(&call.shared, Some(call.request_id.clone()));
    if let Err(error) = parties.accept(&user, &inviter, &build_id).await {
        return party_error(&error, &call);
    }
    tracing::info!("{user} accepted party invite from {inviter}");
    empty_object()
}

/// `DELETE /party/member`: leave my party.
async fn party_leave(call: Call) -> Response {
    let auth = match authenticate(&call) {
        Ok(auth) => auth,
        Err(refusal) => return refusal,
    };
    let user = match user_of(&auth, &call) {
        Ok(user) => user,
        Err(failure) => return failure,
    };
    let parties = Parties::of(&call.shared, Some(call.request_id.clone()));
    if let Err(error) = parties.leave(&user).await {
        return party_error(&error, &call);
    }
    tracing::info!("{user} left their party");
    empty_object()
}

/// `DELETE /party/member/:targetId`: kick a member.
async fn party_kick(call: Call) -> Response {
    let auth = match authenticate(&call) {
        Ok(auth) => auth,
        Err(refusal) => return refusal,
    };
    let user = match user_of(&auth, &call) {
        Ok(user) => user,
        Err(failure) => return failure,
    };
    let target = call.params.get("targetId").unwrap_or_default().to_owned();
    let parties = Parties::of(&call.shared, Some(call.request_id.clone()));
    if let Err(error) = parties.kick(&user, &target).await {
        return party_error(&error, &call);
    }
    tracing::info!("{user} kicked {target} from the party");
    empty_object()
}

/// `PUT /party/member/promote/:targetId`: promote a member to leader.
async fn party_promote(call: Call) -> Response {
    let auth = match authenticate(&call) {
        Ok(auth) => auth,
        Err(refusal) => return refusal,
    };
    let user = match user_of(&auth, &call) {
        Ok(user) => user,
        Err(failure) => return failure,
    };
    let target = call.params.get("targetId").unwrap_or_default().to_owned();
    let parties = Parties::of(&call.shared, Some(call.request_id.clone()));
    if let Err(error) = parties.promote(&user, &target).await {
        return party_error(&error, &call);
    }
    tracing::info!("{user} promoted {target} to party leader");
    empty_object()
}

/// Mounts this area's handlers (`handlers.on(Service::Backend, Method::Get, "/path", ...)`, the path exactly as the
/// route table has it); a route with none answers 501.
pub fn register(handlers: &mut Handlers) {
    let backend = Service::Backend;
    handlers
        .on(backend, Method::Post, "/candidate/player/register", |call| {
            Box::pin(player_register(call))
        })
        .on(backend, Method::Post, "/candidate/player/alive", |call| {
            Box::pin(alive(call))
        })
        .on(backend, Method::Get, "/candidate/regions", |call| {
            Box::pin(regions(call))
        })
        .on(backend, Method::Get, "/candidate/status", |call| Box::pin(status(call)))
        .on(backend, Method::Post, "/candidate/join", |call| Box::pin(join(call)))
        .on(backend, Method::Delete, "/candidate/leave", |call| {
            Box::pin(candidate_leave(call))
        })
        .on(backend, Method::Post, "/candidate/leave", |call| {
            Box::pin(candidate_leave(call))
        })
        .on(backend, Method::Put, "/candidate/leave", |call| {
            Box::pin(candidate_leave(call))
        })
        .on(backend, Method::Delete, "/candidate", |call| {
            Box::pin(candidate_leave(call))
        })
        .on(backend, Method::Delete, "/candidate/:candidateId", |call| {
            Box::pin(candidate_leave(call))
        })
        .on(backend, Method::Get, "/QoS", |call| Box::pin(qos(call)))
        .on(backend, Method::Post, "/party", |call| Box::pin(party_state(call)))
        .on(backend, Method::Put, "/party/invite", |call| {
            Box::pin(party_invite(call))
        })
        .on(backend, Method::Get, "/party/invites", |call| {
            Box::pin(party_invites(call))
        })
        .on(backend, Method::Put, "/party/invite/accept/:inviterId", |call| {
            Box::pin(party_accept(call))
        })
        .on(backend, Method::Delete, "/party/member", |call| {
            Box::pin(party_leave(call))
        })
        .on(backend, Method::Delete, "/party/member/:targetId", |call| {
            Box::pin(party_kick(call))
        })
        .on(backend, Method::Put, "/party/member/promote/:targetId", |call| {
            Box::pin(party_promote(call))
        });
}
