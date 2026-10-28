//! RU5's `CharactersLoadouts` group: `GET`/`PUT`/`POST /character` and the five `/loadout` routes
//! (`routes/character.ts`, `routes/loadout.ts`). RU3c: hook `no_player_policy`, table `TARGET_CHANGELIST` (and the
//! starter manifest table the seeding reads). `/loadout/.../active/:index` is served on 1.12 too: its client calls it,
//! and 1.12.0's Node answered 404.
//!
//! The game-data work is in the submodules: [`slots`] is `loadoutSlots.ts` (in `repo::characters`), [`data`] the
//! character-data rules, [`starter`] the starter manifest, [`controller`] the controllers over the repositories.

// The early returns of a handler are the Response it answers with
#![allow(clippy::result_large_err)]

pub mod controller;
pub mod data;
#[doc(hidden)]
pub mod golden;
pub mod starter;

use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use module_api::{Actor, ActorContext};
use rifts_seam::{AccountId as RiftsAccount, HostWrite, HostWriteKind, HostWriteOptions, RuntimeAuth};
use serde_json::{Value, json};
use service_kit::body::error_page;
use service_kit::json::Json;
use service_kit::{js, jsjson};

use self::controller::{CtlError, LoadoutState, loadout_payload};
use crate::db::IslesStore;
use std::sync::Arc;

use crate::http::{Call, Handlers, Shared, not_implemented};
use crate::repo::characters::slots::{self, MAX_TOTAL_LOADOUT_SLOTS};
use crate::routes::{Method, Params, RouteDef, Service};

/// `req.AuthData` as these routes read it. The auth middleware (RU5 auth's) puts it in the request's extensions
/// before the handler runs, and a request without it is refused 401, as `HasParadoxBackendAuth` refuses one it
/// couldn't authenticate. Integration adapts [`auth_data`] if auth's type differs.
#[derive(Debug, Clone, Default)]
pub struct AuthData {
    /// The player: the bearer's `userId`, or the configured no-player account of a bearer-less gameserver call
    pub user_id: Option<String>,
    pub is_gameserver: bool,
    /// A player-hosted runtime (`IsPlayerHostRuntime`); never set together with `is_gameserver`
    pub host_runtime: Option<RuntimeAuth>,
}

/// UE4/Phoenix's own "no logged-in player" sentinel: never a real account, so never created, read or written for.
const NO_PLAYER_SENTINEL: &str = "INVALID";

fn is_no_player(user: Option<&str>) -> bool {
    user.is_none_or(|user| user == NO_PLAYER_SENTINEL)
}

impl From<&crate::areas::auth::AuthData> for AuthData {
    fn from(auth: &crate::areas::auth::AuthData) -> Self {
        Self {
            user_id: auth.user_id.clone(),
            is_gameserver: auth.is_gameserver,
            host_runtime: auth.runtime.clone(),
        }
    }
}

pub fn auth_data(request: &Request) -> Option<AuthData> {
    request
        .extensions()
        .get::<crate::areas::auth::AuthData>()
        .map(AuthData::from)
        .or_else(|| request.extensions().get::<AuthData>().cloned())
}

/// What a handler keeps of its [`Call`] across an await: the request's body isn't `Sync`, so it is split off.
struct Ctx {
    shared: Arc<Shared>,
    route: &'static RouteDef,
    params: Params,
    request_id: String,
}

fn split(call: Call) -> (Ctx, Request) {
    let Call {
        shared,
        route,
        params,
        request,
        request_id,
        ..
    } = call;
    (
        Ctx {
            shared,
            route,
            params,
            request_id,
        },
        request,
    )
}

fn empty(status: StatusCode) -> Response {
    status.into_response()
}

fn reply(status: StatusCode, body: Value) -> Response {
    (status, Json(body)).into_response()
}

/// Express's error page for a thrown error: what a bug or an unreadable record answers.
fn failed(call: &Ctx, error: &CtlError) -> Response {
    tracing::error!("{}: {error} (request {})", call.route.name(), call.request_id);
    error_page(StatusCode::INTERNAL_SERVER_ERROR)
}

/// `req.body` as `express.json()` and `express.urlencoded()` leave it: `None` for any other content type.
async fn body_of(request: &mut Request) -> Result<Option<Value>, Response> {
    crate::http::read_body(request).await
}

/// The database, or the answer of a module started without one.
fn store_of(call: &Ctx) -> Result<IslesStore, Response> {
    call.shared
        .store
        .clone()
        .ok_or_else(|| not_implemented(call.route, &call.request_id))
}

/// Who a write is logged to (DD23): the player, or the server for a gameserver's or a host's own authority.
async fn actor_of(store: &IslesStore, call: &Ctx, auth: &AuthData, user: Option<&str>) -> ActorContext {
    let player = match (auth.is_gameserver || auth.host_runtime.is_some(), user) {
        (false, Some(user)) => controller::find_account(store, user).await.ok().flatten(),
        _ => None,
    };
    let actor = match player {
        Some(account) => Actor::Player {
            account: account.global().clone(),
        },
        None => Actor::System,
    };
    ActorContext::new(actor).with_request(call.request_id.clone())
}

/// The route's parameters as strings, with `undefined` as the empty string (every route here has them).
fn param<'a>(call: &'a Ctx, name: &str) -> &'a str {
    call.params.get(name).unwrap_or_default()
}

fn body_string(body: &Value, key: &str) -> Option<String> {
    match body.get(key) {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => Some(text.clone()),
        Some(other) => Some(js::string(Some(other))),
    }
}

/// `GET /character`.
async fn get_character(call: Call) -> Response {
    let (call, request) = split(call);
    let Some(auth) = auth_data(&request) else {
        return not_implemented(call.route, &call.request_id);
    };
    let store = match store_of(&call) {
        Ok(store) => store,
        Err(response) => return response,
    };
    // A dedicated hub asks with the gameserver key and no bearer; the hook resolves it to the configured account
    // (1.12.0 has none, so it gets []). A server request that carries a real player's bearer keeps that player.
    let mut user = auth.user_id.clone();
    if is_no_player(user.as_deref()) && auth.is_gameserver {
        let policy = call.shared.version.hooks.no_player_policy();
        if let Some(account) = policy.bearerless_account(&call.shared.settings) {
            tracing::info!("GET /character: gameserver no-player request resolved to '{account}'");
            user = Some(account);
        }
    }
    let Some(user) = user.filter(|u| u != NO_PLAYER_SENTINEL) else {
        return reply(StatusCode::OK, json!([]));
    };
    let actor = actor_of(&store, &call, &auth, Some(&user)).await;
    match controller::characters_for_user(&store, &call.shared.version.tables, &actor, &user).await {
        Ok(characters) => {
            tracing::info!("Retrieved {} characters for {user}", characters.len());
            reply(StatusCode::OK, Value::Array(characters))
        }
        Err(error) => failed(&call, &error),
    }
}

/// `PUT /character`.
async fn put_character(call: Call) -> Response {
    let (call, mut request) = split(call);
    let Some(auth) = auth_data(&request) else {
        return not_implemented(call.route, &call.request_id);
    };
    let body = match body_of(&mut request).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    let store = match store_of(&call) {
        Ok(store) => store,
        Err(response) => return response,
    };
    // `req.body.name` on an unparsed body throws
    let Some(body) = body else {
        return error_page(StatusCode::INTERNAL_SERVER_ERROR);
    };
    let name = body_string(&body, "name").unwrap_or_else(|| "undefined".into());
    if is_no_player(auth.user_id.as_deref()) {
        tracing::debug!(
            "PUT /character called with no-player sentinel - refusing to create a character, not touching DB"
        );
        return empty(StatusCode::BAD_REQUEST);
    }
    let user = auth.user_id.clone().unwrap_or_default();
    tracing::info!("Creating a character named {name} for user {user}");
    let actor = actor_of(&store, &call, &auth, Some(&user)).await;
    match controller::create_character_for_user(&store, &call.shared.version.tables, &actor, &user, &name).await {
        Ok(character) => reply(StatusCode::OK, character),
        Err(error) => failed(&call, &error),
    }
}

/// The 409 the 1.12 client recognises as "conflict, do a follow-up GET /character" (PostCharacter.md). Phoenix may
/// compare the exact bytes, so the whitespace, the space in the title and the four spaces before `Conflict` stay.
/// Express re-labels the header's charset on a string body, so the wire says `utf-8`.
const CONFLICT_PAGE: &str = "<html>\n<head>\n<meta http-equiv=\"Content-Type\" content=\"text/html;charset=ISO-8859-1\"/>\n<title>Error 409 </title>\n</head>\n<body>\n<h2>HTTP ERROR: 409</h2>\n<p>Problem accessing /character. Reason:\n<pre>    Conflict</pre></p>\n<hr />\n</body>\n</html>\n";

fn conflict_page() -> Response {
    let mut response = (StatusCode::CONFLICT, CONFLICT_PAGE).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    response
}

/// `POST /character`.
async fn post_character(call: Call) -> Response {
    let (call, mut request) = split(call);
    let Some(auth) = auth_data(&request) else {
        return not_implemented(call.route, &call.request_id);
    };
    let body = match body_of(&mut request).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    let store = match store_of(&call) {
        Ok(store) => store,
        Err(response) => return response,
    };
    let Some(body) = body else {
        return error_page(StatusCode::INTERNAL_SERVER_ERROR);
    };
    let tables = call.shared.version.tables;
    let mut character_id = body_string(&body, "characterId");
    // A gameserver relays a client's save with its own key and the client's bearer, and without `accountId`
    let user = if auth.is_gameserver {
        body_string(&body, "accountId").or_else(|| auth.user_id.clone())
    } else {
        auth.user_id.clone()
    };

    // The hub's phantom player saves with an empty characterId: resolve it as the read side does, to the account's
    // first character (1.14.7 only). A sentinel account is never given a character here.
    let resolves = call.shared.version.hooks.no_player_policy() == crate::hooks::NoPlayerPolicy::ConfiguredAccount;
    if resolves
        && auth.is_gameserver
        && character_id.as_deref().is_none_or(str::is_empty)
        && !is_no_player(user.as_deref())
    {
        let user = user.clone().unwrap_or_default();
        let actor = actor_of(&store, &call, &auth, Some(&user)).await;
        match controller::characters_for_user(&store, &tables, &actor, &user).await {
            Ok(characters) => match characters
                .first()
                .and_then(|c| c["id"].as_str())
                .filter(|id| !id.is_empty())
            {
                Some(resolved) => {
                    tracing::info!(
                        "POST /character (no characterId): gameserver request resolved to '{resolved}' for {user}"
                    );
                    character_id = Some(resolved.to_owned());
                }
                None => tracing::warn!(
                    "POST /character (no characterId): no character exists for {user} - the write cannot be attributed"
                ),
            },
            Err(error) => return failed(&call, &error),
        }
    }

    if is_no_player(user.as_deref()) {
        tracing::warn!("POST /character called without a resolvable userId - refusing to update, not touching DB");
        return empty(StatusCode::BAD_REQUEST);
    }
    let user = user.unwrap_or_default();
    let character_id = character_id.unwrap_or_else(|| "undefined".into());
    tracing::info!(
        "Updating characterId {character_id} for userId {user} (IsGameserver={})",
        if auth.is_gameserver { "Y" } else { "N" }
    );

    let actor = actor_of(&store, &call, &auth, Some(&user)).await;
    let updated = controller::update_character_for_user(
        &store,
        &tables,
        &actor,
        &user,
        &character_id,
        body.get("data"),
        body.get("updateVersion"),
        auth.is_gameserver,
    )
    .await;
    match updated {
        Err(error) => failed(&call, &error),
        Ok(false) => {
            tracing::warn!("Update conflict for characterId {character_id} (userId {user}) - returning HTTP 409 HTML");
            conflict_page()
        }
        Ok(true) => match controller::character_for_user(&store, &tables, &user, &character_id).await {
            Ok(Some(character)) => reply(StatusCode::OK, character),
            // `res.json(undefined)`: the JSON type and no body
            Ok(None) => {
                let mut response = StatusCode::OK.into_response();
                response.headers_mut().insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static(service_kit::json::JSON_UTF8),
                );
                response
            }
            Err(error) => failed(&call, &error),
        },
    }
}

/// The account a loadout request acts for: a gameserver's names the player in the path, a player's is its own bearer.
fn requestor(call: &Ctx, auth: &AuthData) -> Option<String> {
    if auth.is_gameserver {
        Some(param(call, "userId").to_owned())
    } else {
        auth.user_id.clone()
    }
}

fn log_state(state: &LoadoutState, user: &str, character_id: &str) {
    tracing::info!(
        "Fetched {} loadout(s) for userId {user} and characterId {character_id}",
        state.loadouts.len()
    );
}

/// `GET /loadout/:userId//all`: the empty-characterId form. Only the no-player sentinel gets an answer.
async fn get_loadout_without_character(call: Call) -> Response {
    let (call, request) = split(call);
    let Some(auth) = auth_data(&request) else {
        return not_implemented(call.route, &call.request_id);
    };
    if requestor(&call, &auth).as_deref() == Some(NO_PLAYER_SENTINEL) {
        tracing::debug!("GET /loadout called with no-player sentinel and no characterId - returning empty loadouts");
        return reply(StatusCode::OK, controller::empty_loadout_response());
    }
    empty(StatusCode::BAD_REQUEST)
}

/// `GET /loadout/:userId/:characterId/all`.
async fn get_loadout(call: Call) -> Response {
    let (call, request) = split(call);
    let Some(auth) = auth_data(&request) else {
        return not_implemented(call.route, &call.request_id);
    };
    let requestor = requestor(&call, &auth);
    if requestor.as_deref() == Some(NO_PLAYER_SENTINEL) {
        tracing::debug!("GET /loadout called with no-player sentinel - returning empty loadouts, not touching DB");
        return reply(StatusCode::OK, controller::empty_loadout_response());
    }
    let store = match store_of(&call) {
        Ok(store) => store,
        Err(response) => return response,
    };
    // A player's own bearer always carries a userId; without one nothing here can be found
    let user = requestor.unwrap_or_default();
    let character_id = param(&call, "characterId");
    let actor = actor_of(&store, &call, &auth, Some(&user)).await;
    match controller::loadout_state(&store, &call.shared.version.tables, &actor, &user, character_id).await {
        Ok(state) => {
            log_state(&state, &user, character_id);
            reply(StatusCode::OK, loadout_payload(&state))
        }
        Err(CtlError::NotOwned) => {
            tracing::warn!(
                "GET /loadout rejected for userId {user}: character {character_id} does not belong to the user"
            );
            empty(StatusCode::BAD_REQUEST)
        }
        Err(error) => failed(&call, &error),
    }
}

const HOST_DENIED: &str = "This host may not write this player's state.";

/// `AuthorizeLoadoutUnlock`: a dedicated gameserver may unlock for any account, a player-hosted runtime for a player
/// it may write (the host-write hook decides), and a player's own token never. `Err` is the refusal to send.
async fn authorize_unlock(call: &Ctx, auth: &AuthData, target_user: &str) -> Result<(), Response> {
    if let Some(runtime) = &auth.host_runtime {
        let target = match controller::find_account(
            &call
                .shared
                .store
                .clone()
                .ok_or_else(|| not_implemented(call.route, &call.request_id))?,
            target_user,
        )
        .await
        {
            Ok(Some(account)) => account,
            Ok(None) => {
                return Err(reply(
                    StatusCode::FORBIDDEN,
                    json!({ "code": "P2P_HOST_WRITE_DENIED", "message": HOST_DENIED, "payload": null }),
                ));
            }
            Err(error) => return Err(failed(call, &CtlError::Repo(error))),
        };
        let verdict = call
            .shared
            .rifts
            .guard()
            .authorize_host_write(
                runtime.clone(),
                HostWriteKind::LoadoutUnlock,
                RiftsAccount(target.to_string()),
                HostWriteOptions::default(),
            )
            .await;
        return match verdict {
            Ok(None) => Ok(()),
            Ok(Some(refusal)) => {
                let status = StatusCode::from_u16(refusal.status).unwrap_or(StatusCode::FORBIDDEN);
                Err((
                    status,
                    [(
                        header::CONTENT_TYPE,
                        HeaderValue::from_static(service_kit::json::JSON_UTF8),
                    )],
                    refusal.body.0,
                )
                    .into_response())
            }
            Err(error) => {
                tracing::error!("host write authorization: {error}");
                Err(error_page(StatusCode::INTERNAL_SERVER_ERROR))
            }
        };
    }
    if auth.is_gameserver {
        return Ok(());
    }
    tracing::warn!(
        "POST character loadout unlock rejected without gameserver authority userId={target_user} characterId={}",
        param(call, "characterId")
    );
    Err(reply(
        StatusCode::FORBIDDEN,
        json!({ "code": "forbidden", "message": "Loadout slot unlocks require gameserver authority.", "payload": null }),
    ))
}

/// `GetPlayerJourney`'s nodes for the entitlement; `None` for no saved journey (or the sentinel).
async fn journey_nodes(store: &IslesStore, user: &str) -> Result<Option<Value>, CtlError> {
    if user.is_empty() || user == NO_PLAYER_SENTINEL {
        return Ok(None);
    }
    let Some(account) = controller::find_account(store, user).await? else {
        return Ok(None);
    };
    let row = store
        .read(move |conn, repos| repos.player_journey.find(conn, &account))
        .await?;
    Ok(row.map(|row| jsjson::parse(&row.nodes).unwrap_or_else(|_| json!({}))))
}

/// `POST /loadout/:userId/:characterId/unlock/:numSlots`. The path value is a delta hint; the entitlement is
/// derived from the Slayer's Path `LoadoutSlot_01..05` nodes, so a host cannot grant more than the player unlocked.
async fn unlock_loadout_slots(call: Call) -> Response {
    let (call, request) = split(call);
    let Some(auth) = auth_data(&request) else {
        return not_implemented(call.route, &call.request_id);
    };
    let target = param(&call, "userId").to_owned();
    if let Err(refusal) = authorize_unlock(&call, &auth, &target).await {
        return refusal;
    }
    let character_id = param(&call, "characterId").to_owned();
    let requested = js::number(Some(&Value::String(param(&call, "numSlots").to_owned())));
    let valid = requested.fract() == 0.0
        && requested.abs() <= 9_007_199_254_740_991.0
        && (1.0..=f64::from(MAX_TOTAL_LOADOUT_SLOTS)).contains(&requested);
    if target == NO_PLAYER_SENTINEL || !valid {
        tracing::warn!(
            "POST loadout unlock rejected userId={target} characterId={character_id} requested={}; valid gameserver delta range is 1..{MAX_TOTAL_LOADOUT_SLOTS}",
            param(&call, "numSlots")
        );
        return reply(
            StatusCode::BAD_REQUEST,
            json!({
                "code": "invalid_loadout_slot_count",
                "message": format!("Loadout slot unlock delta must be between 1 and {MAX_TOTAL_LOADOUT_SLOTS}."),
                "payload": null
            }),
        );
    }
    let store = match store_of(&call) {
        Ok(store) => store,
        Err(response) => return response,
    };
    let tables = call.shared.version.tables;
    let actor = actor_of(&store, &call, &auth, None).await;
    let outcome = async {
        let nodes = journey_nodes(&store, &target).await?;
        let total = slots::resolve_player_journey_total(nodes.as_ref());
        controller::ensure_total_slots(&store, &tables, &actor, &target, &character_id, total).await?;
        controller::loadout_state(&store, &tables, &actor, &target, &character_id).await
    }
    .await;
    let state = match outcome {
        Ok(state) => state,
        Err(CtlError::NotOwned) => {
            tracing::warn!(
                "POST character loadout unlock rejected for userId {target}: character does not belong to the user"
            );
            return empty(StatusCode::BAD_REQUEST);
        }
        Err(error) => return failed(&call, &error),
    };
    let (num_account, max_account, num_character, max_character) = slots::wire_counts(state.loadouts.len() as f64);
    tracing::info!(
        "Loadout slot entitlement reconciled userId={target} characterId={character_id} gameserverDelta={requested} authoritativeTotalSlots={} wire=account:{num_account}/{max_account},character:{num_character}/{max_character}",
        state.loadouts.len()
    );
    // A host's unlock is tagged like its other writes (counts only, no ids); the tag is best effort
    if let Some(runtime) = &auth.host_runtime
        && let Ok(Some(account)) = controller::find_account(&store, &target).await
    {
        let write = HostWrite {
            kind: HostWriteKind::LoadoutUnlock,
            target: RiftsAccount(account.to_string()),
            summary: rifts_seam::Json(
                json!({ "gameserverDelta": requested, "authoritativeTotalSlots": state.loadouts.len() }).to_string(),
            ),
        };
        if let Err(error) = call
            .shared
            .rifts
            .guard()
            .record_host_write(runtime.clone(), write)
            .await
        {
            tracing::error!("[HostWrite] tag failed kind=LOADOUT_UNLOCK target={target}: {error}");
        }
    }
    reply(StatusCode::OK, loadout_payload(&state))
}

/// `POST /loadout/:userId/:characterId/active/:index`: which slot the carousel selected, stored apart from the
/// slot contents.
async fn set_active_loadout(call: Call) -> Response {
    let (call, request) = split(call);
    let Some(auth) = auth_data(&request) else {
        return not_implemented(call.route, &call.request_id);
    };
    let requestor = requestor(&call, &auth);
    let character_id = param(&call, "characterId").to_owned();
    let index = js::number(Some(&Value::String(param(&call, "index").to_owned())));
    let in_range = index.fract() == 0.0 && (0.0..f64::from(MAX_TOTAL_LOADOUT_SLOTS)).contains(&index);
    if is_no_player(requestor.as_deref()) || !in_range {
        return reply(
            StatusCode::BAD_REQUEST,
            json!({ "code": "invalid_active_loadout", "message": "Active loadout index is invalid.", "payload": null }),
        );
    }
    let store = match store_of(&call) {
        Ok(store) => store,
        Err(response) => return response,
    };
    let user = requestor.unwrap_or_default();
    let tables = call.shared.version.tables;
    let actor = actor_of(&store, &call, &auth, Some(&user)).await;
    match controller::set_active_index(&store, &tables, &actor, &user, &character_id, index as i64).await {
        Err(CtlError::NotOwned) => {
            tracing::warn!("POST active loadout rejected for userId {user}: character does not belong to the user");
            return empty(StatusCode::BAD_REQUEST);
        }
        Err(error) => return failed(&call, &error),
        Ok(false) => {
            tracing::warn!(
                "POST active loadout rejected userId={user} characterId={character_id} index={index}; slot is not unlocked"
            );
            return reply(
                StatusCode::BAD_REQUEST,
                json!({ "code": "loadout_slot_not_unlocked", "message": "The requested loadout slot is not unlocked.", "payload": null }),
            );
        }
        Ok(true) => {}
    }
    match controller::loadout_state(&store, &tables, &actor, &user, &character_id).await {
        Ok(state) => {
            tracing::info!(
                "Active loadout updated userId={user} characterId={character_id} index={}",
                state.active_index
            );
            reply(StatusCode::OK, loadout_payload(&state))
        }
        Err(error) => failed(&call, &error),
    }
}

/// `POST /loadout/:userId/:characterId/:index`: the whole-slot (or `persistent`) replace.
async fn set_loadout(call: Call) -> Response {
    let (call, mut request) = split(call);
    let Some(auth) = auth_data(&request) else {
        return not_implemented(call.route, &call.request_id);
    };
    let body = match body_of(&mut request).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    let Some(body @ Value::Object(_)) = body else {
        tracing::warn!("POST /loadout rejected: request body is not an object");
        return empty(StatusCode::BAD_REQUEST);
    };
    let requestor = requestor(&call, &auth);
    let character_id = param(&call, "characterId").to_owned();
    let index = param(&call, "index").to_owned();
    if requestor.as_deref() == Some(NO_PLAYER_SENTINEL) {
        tracing::warn!(
            "POST /loadout called with no-player sentinel (userId=INVALID) for characterId {character_id} - refusing update, not touching DB"
        );
        return empty(StatusCode::BAD_REQUEST);
    }
    let store = match store_of(&call) {
        Ok(store) => store,
        Err(response) => return response,
    };
    let user = requestor.unwrap_or_default();
    let tables = call.shared.version.tables;
    let actor = actor_of(&store, &call, &auth, Some(&user)).await;
    match controller::set_loadout_data(&store, &actor, &user, &character_id, &index, body.get("data")).await {
        Err(error) => failed(&call, &error),
        Ok(false) => {
            tracing::error!("Failed to update loadout index {index} for userId {user} and characterId {character_id}");
            empty(StatusCode::BAD_REQUEST)
        }
        Ok(true) => {
            tracing::info!(
                "Successfully updated loadout index {index} for userId {user} and characterId {character_id}"
            );
            match controller::loadout_state(&store, &tables, &actor, &user, &character_id).await {
                Ok(state) => {
                    log_state(&state, &user, &character_id);
                    reply(StatusCode::OK, loadout_payload(&state))
                }
                Err(error) => failed(&call, &error),
            }
        }
    }
}

/// Mounts this area's handlers, the paths exactly as the route table has them.
pub fn register(handlers: &mut Handlers) {
    handlers
        .on(Service::Backend, Method::Get, "/character", |call| {
            Box::pin(get_character(call))
        })
        .on(Service::Backend, Method::Put, "/character", |call| {
            Box::pin(put_character(call))
        })
        .on(Service::Backend, Method::Post, "/character", |call| {
            Box::pin(post_character(call))
        })
        .on(Service::Backend, Method::Get, "/loadout/:userId//all", |call| {
            Box::pin(get_loadout_without_character(call))
        })
        .on(
            Service::Backend,
            Method::Get,
            "/loadout/:userId/:characterId/all",
            |call| Box::pin(get_loadout(call)),
        )
        .on(
            Service::Backend,
            Method::Post,
            "/loadout/:userId/:characterId/unlock/:numSlots",
            |call| Box::pin(unlock_loadout_slots(call)),
        )
        .on(
            Service::Backend,
            Method::Post,
            "/loadout/:userId/:characterId/active/:index",
            |call| Box::pin(set_active_loadout(call)),
        )
        .on(
            Service::Backend,
            Method::Post,
            "/loadout/:userId/:characterId/:index",
            |call| Box::pin(set_loadout(call)),
        );
}

/// Test support shared by this area's unit tests.
#[cfg(test)]
pub(crate) mod test_support {
    use crate::tables::{StarterManifest, VersionTables};

    pub const STACKED: &[&str] = &[
        "TOKEN_DAILY_CHALLENGE_DRAFT",
        "QI_BASIC_FLARE_DURABLE",
        "EM_INTRO_BEGINNER_01",
    ];
    pub const EMOJIS: &[&str] = &["EJ_PLAYER_BOSUN_JOY", "EJ_PLAYER_KAT_PRAY", "", "", "", "", "", "", ""];

    pub fn sample_tables(emojis: bool) -> VersionTables {
        VersionTables {
            target_changelist: 647472,
            game_version: "1.14.7",
            active_huntpass_id: "",
            recent_huntpass_seasons: &[],
            patchnotes_release_version: "",
            bounty_item_grant_data: &[],
            store_sections: &[],
            seasonal_coin: None,
            masked_journey_nodes: &[],
            starter_manifest: StarterManifest {
                bootstrap_version: "starter-1.12-v5",
                stacked: STACKED,
                quick_emojis: emojis.then_some(EMOJIS),
            },
            switch_defaults: &[],
        }
    }
}
