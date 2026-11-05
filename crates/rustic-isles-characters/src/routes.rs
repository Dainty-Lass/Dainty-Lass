// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The eight handlers (spec §3): authentication, the store and area slots, the order of checks per route, and the
//! answers.

use std::fmt::Display;
use std::sync::Arc;

use axum::extract::Request;
use axum::response::{IntoResponse, Response};
use http::{StatusCode, header};
use module_api::{Actor, ActorContext};
use rifts_seam::{HostWrite, HostWriteKind, HostWriteOptions, RuntimeAuth};
use rustic_isles::{Call, Handlers, Method, NoPlayerPolicy, RouteDef, Service, Shared, not_implemented, read_body};
use rustic_isles_auth::{AuthData, RuntimeCaller};
use rustic_isles_store::IslesStore;
use rustic_isles_store::repo::auth::AccountRecord;
use rustic_isles_store::repo::characters::slots;
use serde_json::{Value, json};
use service_kit::json::JSON_UTF8;
use service_kit::{js, jsjson};

use crate::controller::{INVALID, Write, journey_total, lookup};
use crate::{Characters, CharactersError, empty_loadout_payload, loadout_payload};

const AUTH_HEADERS: [&str; 3] = [
    "authorization",
    "x-rustic-isles-gameserver-apikey",
    "x-rustic-isles-gameserver-key",
];

/// The client recognises the 409 and re-reads; it may compare the bytes, so the spacing is kept.
pub const CONFLICT_PAGE: &str = "<html>\n<head>\n<meta http-equiv=\"Content-Type\" \
content=\"text/html;charset=ISO-8859-1\"/>\n<title>Error 409 </title>\n</head>\n<body>\n<h2>HTTP ERROR: 409</h2>\n\
<p>Problem accessing /character. Reason:\n<pre>    Conflict</pre></p>\n<hr />\n</body>\n</html>\n";

const MAX_SAFE: f64 = 9_007_199_254_740_991.0;

fn empty(status: StatusCode) -> Response {
    status.into_response()
}

fn page500() -> Response {
    service_kit::body::error_page(StatusCode::INTERNAL_SERVER_ERROR)
}

fn json_text(status: StatusCode, body: String) -> Response {
    (status, [(header::CONTENT_TYPE, JSON_UTF8)], body).into_response()
}

fn json_ok(value: &Value) -> Response {
    json_text(StatusCode::OK, jsjson::stringify(value))
}

fn refusal(status: StatusCode, code: &str, message: &str) -> Response {
    let body = json!({"code": code, "message": message, "payload": null});
    json_text(status, jsjson::stringify(&body))
}

fn conflict() -> Response {
    (
        StatusCode::CONFLICT,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        CONFLICT_PAGE,
    )
        .into_response()
}

fn no_player(user: Option<&str>) -> bool {
    user.is_none_or(|u| u == INVALID)
}

/// `body.f` as text: absent or `null` is none, a string itself, anything else `String(value)`.
fn field_text(body: &Value, name: &str) -> Option<String> {
    match body.get(name) {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => Some(text.clone()),
        other => Some(js::string(other)),
    }
}

/// A `Number()` of a param that is a whole number in `range`.
fn whole_in(text: &str, range: std::ops::RangeInclusive<f64>) -> Option<u32> {
    let n = js::number(Some(&Value::from(text)));
    let whole = n.is_finite() && n.fract() == 0.0 && n.abs() <= MAX_SAFE;
    // Whole and inside a small range, so the cast is exact
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    (whole && range.contains(&n)).then_some(n as u32)
}

/// One matched request, once it is authenticated.
struct Ctx {
    shared: Arc<Shared>,
    route: &'static RouteDef,
    request_id: String,
    caller: AuthData,
}

impl Ctx {
    fn param<'a>(&self, params: &'a rustic_isles::Params, name: &str) -> &'a str {
        let _ = self;
        params.get(name).unwrap_or("")
    }

    fn failure(&self, error: &dyn Display) -> Response {
        tracing::error!(
            "{} {}: {error} (request {})",
            self.route.method.as_str(),
            self.route.path,
            self.request_id
        );
        page500()
    }

    fn not_implemented(&self) -> Response {
        not_implemented(self.route, &self.request_id)
    }

    fn store(&self) -> Result<Arc<IslesStore>, Response> {
        self.shared.slot::<IslesStore>().ok_or_else(|| self.not_implemented())
    }

    fn area(&self) -> Result<(Arc<IslesStore>, Arc<Characters>), Response> {
        let store = self.store()?;
        let area = self.shared.slot::<Characters>().ok_or_else(|| self.not_implemented())?;
        Ok((store, area))
    }

    fn host(&self) -> Option<&RuntimeCaller> {
        self.caller.runtime.as_ref()
    }

    fn system(&self) -> ActorContext {
        ActorContext::new(Actor::System).with_request(self.request_id.as_str())
    }

    /// A player acting for an account it already has; anyone else (or a new account) is the system.
    async fn actor(&self, store: &IslesStore, acting: Option<&str>) -> Result<ActorContext, Response> {
        let player = !self.caller.is_gameserver && self.host().is_none();
        let Some(user) = acting.filter(|_| player) else {
            return Ok(self.system());
        };
        match lookup(store, user).await {
            Ok(Some(account)) => {
                let actor = Actor::Player {
                    account: account.id.global().clone(),
                };
                Ok(ActorContext::new(actor).with_request(self.request_id.as_str()))
            }
            Ok(None) => Ok(self.system()),
            Err(e) => Err(self.failure(&e)),
        }
    }

    /// The loadout routes' requestor: a game server names it in the path.
    fn requestor(&self, params: &rustic_isles::Params) -> Option<String> {
        if self.caller.is_gameserver {
            params.get("userId").map(str::to_owned)
        } else {
            self.caller.user_id.clone()
        }
    }
}

/// The caller already authenticated, else game auth; a request with none of the three auth headers answers 501
/// (parity with restricted, spec §15 item 1).
async fn authenticate(
    shared: &Shared,
    route: &RouteDef,
    request: &mut Request,
    request_id: &str,
) -> Result<AuthData, Response> {
    if let Some(data) = request.extensions().get::<AuthData>() {
        return Ok(data.clone());
    }
    if !AUTH_HEADERS.iter().any(|name| request.headers().contains_key(*name)) {
        return Err(not_implemented(route, request_id));
    }
    shared.auth().game_auth(request).await
}

struct Entry {
    ctx: Ctx,
    params: rustic_isles::Params,
    body: Option<Value>,
}

/// Reads the body first when `with_body` (a body error answers before auth), then authenticates.
async fn enter(call: Call, with_body: bool) -> Result<Entry, Response> {
    let Call {
        shared,
        route,
        params,
        mut request,
        request_id,
        ..
    } = call;
    let body = if with_body {
        read_body(&mut request).await?
    } else {
        None
    };
    let caller = authenticate(&shared, route, &mut request, &request_id).await?;
    Ok(Entry {
        ctx: Ctx {
            shared,
            route,
            request_id,
            caller,
        },
        params,
        body,
    })
}

macro_rules! answer {
    ($e:expr) => {
        match $e {
            Ok(value) => value,
            Err(response) => return response,
        }
    };
}

pub fn register(handlers: &mut Handlers) {
    let b = Service::Backend;
    handlers.on(b, Method::Get, "/character", get_character);
    handlers.on(b, Method::Put, "/character", put_character);
    handlers.on(b, Method::Post, "/character", post_character);
    handlers.on(b, Method::Get, "/loadout/:userId//all", get_phantom_loadouts);
    handlers.on(b, Method::Get, "/loadout/:userId/:characterId/all", get_loadouts);
    handlers.on(
        b,
        Method::Post,
        "/loadout/:userId/:characterId/unlock/:numSlots",
        unlock_slots,
    );
    handlers.on(
        b,
        Method::Post,
        "/loadout/:userId/:characterId/active/:index",
        set_active,
    );
    handlers.on(b, Method::Post, "/loadout/:userId/:characterId/:index", set_loadout);
}

async fn get_character(call: Call) -> Response {
    let Entry { ctx, .. } = answer!(enter(call, false).await);
    let (store, area) = answer!(ctx.area());
    let tables = ctx.shared.version().tables;
    let mut user = ctx.caller.user_id.clone();
    if no_player(user.as_deref()) && ctx.caller.is_gameserver {
        let policy = ctx.shared.version().hooks.no_player_policy();
        if let Some(account) = policy.bearerless_account(ctx.shared.settings()) {
            tracing::info!("GET /character: gameserver no-player request resolved to {account:?}");
            user = Some(account);
        }
    }
    let Some(user) = user.filter(|u| u != INVALID) else {
        return json_ok(&json!([]));
    };
    let actor = answer!(ctx.actor(&store, Some(&user)).await);
    match area.characters_for_user(&store, tables, &actor, &user).await {
        Ok(characters) => {
            tracing::info!("Retrieved {} characters for {user:?}", characters.len());
            json_ok(&Value::Array(characters))
        }
        Err(e) => ctx.failure(&e),
    }
}

async fn put_character(call: Call) -> Response {
    let Entry { ctx, body, .. } = answer!(enter(call, true).await);
    let (store, area) = answer!(ctx.area());
    let Some(body) = body else {
        return page500();
    };
    let name = field_text(&body, "name").unwrap_or_else(|| "undefined".into());
    let user = ctx.caller.user_id.clone();
    let Some(user) = user.filter(|u| !no_player(Some(u))) else {
        return empty(StatusCode::BAD_REQUEST);
    };
    let actor = answer!(ctx.actor(&store, Some(&user)).await);
    tracing::info!("Creating a character named {name:?} for user {user:?}");
    let tables = ctx.shared.version().tables;
    match area.create_character(&store, tables, &actor, &user, &name).await {
        Ok(character) => json_ok(&character),
        Err(e) => ctx.failure(&e),
    }
}

async fn post_character(call: Call) -> Response {
    let Entry { ctx, body, .. } = answer!(enter(call, true).await);
    let (store, area) = answer!(ctx.area());
    let Some(body) = body else {
        return page500();
    };
    let tables = ctx.shared.version().tables;
    let gameserver = ctx.caller.is_gameserver;
    let mut character_id = field_text(&body, "characterId");
    let acting = match gameserver {
        true => field_text(&body, "accountId").or_else(|| ctx.caller.user_id.clone()),
        false => ctx.caller.user_id.clone(),
    };
    let actor = answer!(ctx.actor(&store, acting.as_deref()).await);
    let policy = ctx.shared.version().hooks.no_player_policy();
    let unnamed = character_id.as_deref().is_none_or(str::is_empty);
    if policy == NoPlayerPolicy::ConfiguredAccount
        && gameserver
        && unnamed
        && let Some(user) = acting.as_deref().filter(|u| !no_player(Some(u)))
    {
        let characters = match area.characters_for_user(&store, tables, &actor, user).await {
            Ok(characters) => characters,
            Err(e) => return ctx.failure(&e),
        };
        let first = characters
            .first()
            .and_then(|c| c.get("id"))
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty());
        match first {
            Some(id) => {
                tracing::info!("POST /character (no characterId): gameserver request resolved to {id:?} for {user:?}");
                character_id = Some(id.to_owned());
            }
            None => tracing::warn!(
                "POST /character (no characterId): no character exists for {user:?} - the write cannot be attributed"
            ),
        }
    }
    let Some(user) = acting.filter(|u| !no_player(Some(u))) else {
        return empty(StatusCode::BAD_REQUEST);
    };
    let character_id = character_id.unwrap_or_else(|| "undefined".into());
    let accepted = area
        .update_character(
            &store,
            tables,
            &actor,
            &user,
            &character_id,
            body.get("data"),
            body.get("updateVersion"),
            gameserver,
        )
        .await;
    match accepted {
        Ok(false) => {
            tracing::warn!(
                "Update conflict for characterId {character_id:?} (userId {user:?}) - returning HTTP 409 HTML"
            );
            conflict()
        }
        Ok(true) => match area.character_for_user(&store, tables, &user, &character_id).await {
            Ok(Some(character)) => json_ok(&character),
            // Node's res.json(undefined)
            Ok(None) => json_text(StatusCode::OK, String::new()),
            Err(e) => ctx.failure(&e),
        },
        Err(e) => ctx.failure(&e),
    }
}

async fn get_phantom_loadouts(call: Call) -> Response {
    let Entry { ctx, params, .. } = answer!(enter(call, false).await);
    match ctx.requestor(&params).as_deref() {
        Some(INVALID) => json_ok(&empty_loadout_payload()),
        _ => empty(StatusCode::BAD_REQUEST),
    }
}

async fn get_loadouts(call: Call) -> Response {
    let Entry { ctx, params, .. } = answer!(enter(call, false).await);
    let requestor = ctx.requestor(&params);
    if requestor.as_deref() == Some(INVALID) {
        return json_ok(&empty_loadout_payload());
    }
    let (store, area) = answer!(ctx.area());
    let user = requestor.unwrap_or_default();
    let character_id = ctx.param(&params, "characterId").to_owned();
    let actor = answer!(ctx.actor(&store, Some(&user)).await);
    let tables = ctx.shared.version().tables;
    match area.loadout_state(&store, tables, &actor, &user, &character_id).await {
        Ok(state) => {
            tracing::info!(
                "Fetched {} loadout(s) for userId {user:?} and characterId {character_id:?}",
                state.loadouts.len()
            );
            json_ok(&loadout_payload(&state))
        }
        Err(CharactersError::NotOwned) => {
            tracing::warn!(
                "GET /loadout rejected for userId {user:?}: character {character_id:?} does not belong to the user"
            );
            empty(StatusCode::BAD_REQUEST)
        }
        Err(e) => ctx.failure(&e),
    }
}

/// A host runtime's unlock goes through rifts' guard for the target's account; `Ok` is the account and the runtime.
async fn host_gate(ctx: &Ctx, runtime: &RuntimeCaller, target: &str) -> Result<(AccountRecord, RuntimeAuth), Response> {
    let store = ctx.store()?;
    let account = match lookup(&store, target).await {
        Ok(Some(account)) => account,
        Ok(None) => {
            let message = "This host may not write this player's state.";
            return Err(refusal(StatusCode::FORBIDDEN, "P2P_HOST_WRITE_DENIED", message));
        }
        Err(e) => return Err(ctx.failure(&e)),
    };
    // No handle from the realms seam: refused as the dedicated-only guard refuses (spec §15 item 2)
    let Some(auth) = runtime.handle_as::<RuntimeAuth>().cloned() else {
        let message = "This runtime cannot write player state.";
        return Err(refusal(StatusCode::FORBIDDEN, "forbidden", message));
    };
    let target_id = rifts_seam::AccountId(account.id.to_string());
    let verdict = ctx
        .shared
        .rifts()
        .guard()
        .authorize_host_write(
            auth.clone(),
            HostWriteKind::LoadoutUnlock,
            target_id,
            HostWriteOptions::default(),
        )
        .await;
    match verdict {
        Ok(None) => Ok((account, auth)),
        Ok(Some(reply)) => {
            let status = StatusCode::from_u16(reply.status).unwrap_or(StatusCode::FORBIDDEN);
            Err(json_text(status, reply.body.0))
        }
        Err(e) => {
            tracing::error!("host write authorization: {e}");
            Err(page500())
        }
    }
}

async fn unlock_slots(call: Call) -> Response {
    let Entry { ctx, params, .. } = answer!(enter(call, false).await);
    let target = ctx.param(&params, "userId").to_owned();
    let character_id = ctx.param(&params, "characterId").to_owned();
    let host = match ctx.host() {
        Some(runtime) => Some(answer!(host_gate(&ctx, runtime, &target).await)),
        None if ctx.caller.is_gameserver => None,
        None => {
            tracing::warn!(
                "POST character loadout unlock rejected without gameserver authority userId={target:?} \
                 characterId={character_id:?}"
            );
            let message = "Loadout slot unlocks require gameserver authority.";
            return refusal(StatusCode::FORBIDDEN, "forbidden", message);
        }
    };
    let num_slots = ctx.param(&params, "numSlots");
    let requested = whole_in(num_slots, 1.0..=6.0);
    let Some(requested) = requested.filter(|_| target != INVALID) else {
        let message = "Loadout slot unlock delta must be between 1 and 6.";
        return refusal(StatusCode::BAD_REQUEST, "invalid_loadout_slot_count", message);
    };
    let (store, area) = answer!(ctx.area());
    let tables = ctx.shared.version().tables;
    let actor = ctx.system();
    let total = match journey_total(&store, &target).await {
        Ok(total) => total,
        Err(e) => return ctx.failure(&e),
    };
    let state = match area
        .ensure_total_slots(&store, tables, &actor, &target, &character_id, total)
        .await
    {
        Ok(()) => area.loadout_state(&store, tables, &actor, &target, &character_id).await,
        Err(e) => Err(e),
    };
    let state = match state {
        Ok(state) => state,
        Err(CharactersError::NotOwned) => {
            tracing::warn!(
                "POST character loadout unlock rejected for userId {target:?}: character does not belong to the user"
            );
            return empty(StatusCode::BAD_REQUEST);
        }
        Err(e) => return ctx.failure(&e),
    };
    let visible = u32::try_from(state.loadouts.len()).unwrap_or(u32::MAX);
    let (a, b, c, d) = slots::wire_counts(f64::from(visible));
    tracing::info!(
        "Loadout slot entitlement reconciled userId={target:?} characterId={character_id:?} \
         gameserverDelta={requested} authoritativeTotalSlots={total} wire=account:{a}/{b},character:{c}/{d}"
    );
    if let Some((account, auth)) = host {
        // Counts only, never the character id; a float as restricted writes it (spec §15 item 5)
        let summary = json!({"gameserverDelta": f64::from(requested), "authoritativeTotalSlots": total});
        let write = HostWrite {
            kind: HostWriteKind::LoadoutUnlock,
            target: rifts_seam::AccountId(account.id.to_string()),
            summary: rifts_seam::Json(summary.to_string()),
        };
        if let Err(e) = ctx.shared.rifts().guard().record_host_write(auth, write).await {
            tracing::error!("[HostWrite] tag failed kind=LOADOUT_UNLOCK target={target:?}: {e}");
        }
    }
    json_ok(&loadout_payload(&state))
}

async fn set_active(call: Call) -> Response {
    let Entry { ctx, params, .. } = answer!(enter(call, false).await);
    let requestor = ctx.requestor(&params);
    let character_id = ctx.param(&params, "characterId").to_owned();
    let index = whole_in(ctx.param(&params, "index"), 0.0..=5.0);
    let (Some(user), Some(index)) = (requestor.filter(|u| u != INVALID), index) else {
        return refusal(
            StatusCode::BAD_REQUEST,
            "invalid_active_loadout",
            "Active loadout index is invalid.",
        );
    };
    let (store, area) = answer!(ctx.area());
    let tables = ctx.shared.version().tables;
    let actor = answer!(ctx.actor(&store, Some(&user)).await);
    match area
        .set_active_index(&store, tables, &actor, &user, &character_id, index)
        .await
    {
        Ok(Write::Accepted) => {}
        Ok(Write::Refused) => {
            let message = "The requested loadout slot is not unlocked.";
            return refusal(StatusCode::BAD_REQUEST, "loadout_slot_not_unlocked", message);
        }
        Err(CharactersError::NotOwned) => {
            tracing::warn!("POST active loadout rejected for userId {user:?}: character does not belong to the user");
            return empty(StatusCode::BAD_REQUEST);
        }
        Err(e) => return ctx.failure(&e),
    }
    match area.loadout_state(&store, tables, &actor, &user, &character_id).await {
        Ok(state) => {
            tracing::info!(
                "Active loadout updated userId={user:?} characterId={character_id:?} index={}",
                state.active_index
            );
            json_ok(&loadout_payload(&state))
        }
        Err(CharactersError::NotOwned) => empty(StatusCode::BAD_REQUEST),
        Err(e) => ctx.failure(&e),
    }
}

async fn set_loadout(call: Call) -> Response {
    let Entry { ctx, params, body } = answer!(enter(call, true).await);
    let Some(body) = body.filter(Value::is_object) else {
        tracing::warn!("POST /loadout rejected: request body is not an object");
        return empty(StatusCode::BAD_REQUEST);
    };
    let requestor = ctx.requestor(&params);
    if requestor.as_deref() == Some(INVALID) {
        return empty(StatusCode::BAD_REQUEST);
    }
    let (store, area) = answer!(ctx.area());
    let user = requestor.unwrap_or_default();
    let character_id = ctx.param(&params, "characterId").to_owned();
    let index = ctx.param(&params, "index").to_owned();
    let actor = answer!(ctx.actor(&store, Some(&user)).await);
    let written = area
        .set_loadout_data(&store, &actor, &user, &character_id, &index, body.get("data"))
        .await;
    match written {
        Ok(Write::Accepted) => {}
        Ok(Write::Refused) => {
            tracing::error!(
                "Failed to update loadout index {index:?} for userId {user:?} and characterId {character_id:?}"
            );
            return empty(StatusCode::BAD_REQUEST);
        }
        Err(e) => return ctx.failure(&e),
    }
    let tables = ctx.shared.version().tables;
    match area.loadout_state(&store, tables, &actor, &user, &character_id).await {
        Ok(state) => json_ok(&loadout_payload(&state)),
        Err(CharactersError::NotOwned) => empty(StatusCode::BAD_REQUEST),
        Err(e) => ctx.failure(&e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conflict_page_bytes() {
        assert_eq!(CONFLICT_PAGE.len(), 245);
        assert_eq!(CONFLICT_PAGE.lines().count(), 12);
        assert!(CONFLICT_PAGE.contains("<title>Error 409 </title>\n") && CONFLICT_PAGE.contains("<pre>    Conflict"));
    }

    #[test]
    fn whole_params() {
        assert_eq!(whole_in("0x2", 1.0..=6.0), Some(2));
        assert_eq!(whole_in(" 3", 1.0..=6.0), Some(3));
        assert_eq!(whole_in(" ", 0.0..=5.0), Some(0));
        assert_eq!(whole_in("1e0", 0.0..=5.0), Some(1));
        for bad in ["0", "7", "1.5", "abc", "-1", "Infinity"] {
            assert_eq!(whole_in(bad, 1.0..=6.0), None, "{bad}");
        }
    }
}
