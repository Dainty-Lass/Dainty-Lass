//! The 17 handlers of RU5's `bounties-store` group, each as `routes/system.ts`, `routes/store.ts` and
//! `controllers/cooldowns.ts` answer it. A request is read in Express's order: the body parsers first (a malformed body
//! is refused before any route), then `HasParadoxBackendAuth`, then the handler.
// An early `Err` is the whole answer, a `Response`
#![allow(clippy::result_large_err)]

use std::sync::Arc;

use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use module_api::{Actor, ActorContext};
use rifts_seam::{HostWrite, HostWriteKind, HostWriteOptions};
use serde_json::{Map, Value, json};
use service_kit::body::error_page;
use service_kit::json::{JSON_UTF8, Json};

use super::authn::{AuthData, PurchaseTokenClaims, StoreSeams, StoreTransaction, TransactionFailure};
use super::bounty_state::{
    BountySaveValidationError, bounty_delete_names_nothing, bounty_group_counts, bounty_state_payload, delete_bounties,
    host_bounty_write_summary, is_authorized_bounty_reader, is_authorized_bounty_writer, is_host_bounty_writer,
    merge_bounty_state, merge_host_bounty_state, newly_claimed_bounty_ids,
};
use super::cooldown_state::{
    CooldownSaveValidationError, cooldown_list, cooldown_state_payload, is_authorized_cooldown_reader,
    is_authorized_cooldown_writer, is_host_cooldown_writer, is_valid_cooldown_id, merge_cooldown_state,
    merge_host_cooldown_state, validate_cooldown_save,
};
use super::entitlements::{entitlements_for_account, grant_entitlements_in_transaction};
use super::monitor;
use super::platinum::project_balances;
use super::store::{
    PurchaseReason, StoreCatalog, catalog_for, daily_claim_transaction_id, daily_reset_window, is_daily_claim_section,
    plan_purchase, sku_listing,
};
use super::util::{now_iso, now_ms};
use super::wallet::get_wallet;
use crate::db::IslesStore;
use crate::http::{Call, Shared, not_implemented, read_body_parts};
use crate::ids::{AccountId, UserId};
use crate::repo::{RepoError, RepoResult};
use crate::routes::Params;
use crate::settings::{BOUNTY_PERSISTENCE, COOLDOWN_PERSISTENCE, SharedSwitch};

/// `MAX_SAVE_ATTEMPTS`: a compare-and-retry loop on the stored version.
const MAX_SAVE_ATTEMPTS: usize = 8;
/// The path sentinel a game server sends for a player without an id.
const NO_PLAYER: &str = "INVALID";

/// One request, read the way Express hands it to a handler.
pub struct Ctx {
    pub shared: Arc<Shared>,
    pub call_route: &'static crate::routes::RouteDef,
    pub params: Params,
    pub auth: Option<AuthData>,
    pub seams: Option<StoreSeams>,
    /// `req.body`: `None` is undefined
    pub body: Option<Value>,
    pub query: String,
    pub request_id: String,
}

fn status(code: u16) -> StatusCode {
    StatusCode::from_u16(code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
}

fn json_reply(code: u16, value: Value) -> Response {
    let mut response = Json(value).into_response();
    *response.status_mut() = status(code);
    response
}

fn empty(code: u16) -> Response {
    status(code).into_response()
}

/// An exception in an async Express 5 handler: the default error page.
fn internal_error(what: &str, error: &dyn std::fmt::Display) -> Response {
    tracing::error!("{what}: {error}");
    error_page(StatusCode::INTERNAL_SERVER_ERROR)
}

/// `{code: null, message, payload}`, the envelope of the state routes.
fn envelope(code: u16, message: &str, payload: Value) -> Response {
    json_reply(code, json!({ "code": null, "message": message, "payload": payload }))
}

/// `req.query.<name>` as Express's simple parser gives it: a string, or an array when the name repeats.
fn query_param(query: &str, name: &str) -> Option<QueryValue> {
    let mut found: Vec<String> = Vec::new();
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let decode = |text: &str| {
            let spaced = text.replace('+', " ");
            service_kit::js::decode_uri_component(&spaced).unwrap_or(spaced)
        };
        if decode(key) == name {
            found.push(decode(value));
        }
    }
    match found.len() {
        0 => None,
        1 => found.pop().map(QueryValue::One),
        _ => Some(QueryValue::Many),
    }
}

enum QueryValue {
    One(String),
    Many,
}

impl Ctx {
    async fn read(call: Call) -> Result<Self, Response> {
        let Call {
            shared,
            route,
            params,
            request,
            origin_host,
            request_id,
        } = call;
        let existing_auth = super::authn::auth_of_request(&request);
        let seams = seams_of_request(&request).or_else(|| StoreSeams::production(&shared, &request_id));
        let query = request.uri().query().unwrap_or_default().to_owned();
        let (parts, body) = request.into_parts();
        let (parts, body) = read_body_parts(parts, body).await?;
        let auth = match existing_auth {
            Some(auth) => Some(auth),
            None => {
                let mut auth_call = Call {
                    shared: shared.clone(),
                    route,
                    params: params.clone(),
                    request: axum::extract::Request::from_parts(parts, axum::body::Body::empty()),
                    origin_host,
                    request_id: request_id.clone(),
                };
                let auth = crate::areas::auth::game_auth(&mut auth_call).await?;
                Some(super::authn::AuthData::from(&auth))
            }
        };
        Ok(Self {
            shared,
            call_route: route,
            params,
            auth,
            seams,
            body,
            query,
            request_id,
        })
    }

    /// `HasParadoxBackendAuth` found no credential: 401, empty.
    fn auth(&self) -> Result<&AuthData, Response> {
        self.auth.as_ref().ok_or_else(|| empty(401))
    }

    fn store(&self) -> Result<&IslesStore, Response> {
        self.shared
            .store
            .as_ref()
            .ok_or_else(|| not_implemented(self.call_route, &self.request_id))
    }

    fn param(&self, name: &str) -> String {
        self.params.get(name).unwrap_or_default().to_owned()
    }

    fn monitor_on(&self) -> bool {
        monitor::enabled(&self.shared.settings)
    }

    fn persistence(&self, switch: SharedSwitch) -> bool {
        let raw = self
            .shared
            .settings
            .raw(switch.env)
            .or_else(|| self.shared.version.tables.switch_default(switch))
            .unwrap_or_default();
        !["0", "false", "off", "no"]
            .iter()
            .any(|off| raw.eq_ignore_ascii_case(off))
    }

    fn catalog(&self) -> Arc<StoreCatalog> {
        catalog_for(&self.shared.game_data, self.shared.version.tables.store_sections)
    }
}

fn seams_of_request(request: &axum::extract::Request) -> Option<StoreSeams> {
    request.extensions().get::<StoreSeams>().cloned()
}

async fn account_of(store: &IslesStore, user_id: &str) -> RepoResult<Option<AccountId>> {
    let user = UserId(user_id.to_owned());
    Ok(store
        .read(move |conn, repos| repos.accounts.find_by_user_id(conn, &user))
        .await?
        .map(|account| account.id))
}

/// The change log's actor: a player acting on their own account, otherwise the server (a game server writes on a
/// player's behalf).
fn actor_for(auth: &AuthData, account: &AccountId, request_id: &str) -> ActorContext {
    let actor = if auth.is_gameserver || auth.is_host_runtime() {
        Actor::System
    } else {
        Actor::Player {
            account: account.global().clone(),
        }
    };
    ActorContext::new(actor).with_request(request_id)
}

// ---- host writes (rustic-rifts' hooks)

async fn authorize_host_write(
    ctx: &Ctx,
    auth: &AuthData,
    kind: HostWriteKind,
    target: &AccountId,
) -> Result<Option<Response>, Response> {
    let Some(runtime) = auth.host_runtime.clone() else {
        return Ok(Some(empty(403)));
    };
    let denied = ctx
        .shared
        .rifts
        .guard()
        .authorize_host_write(
            runtime,
            kind,
            rifts_seam::AccountId(target.to_string()),
            HostWriteOptions::default(),
        )
        .await
        .map_err(|e| internal_error("hostWrites.authorize", &e))?;
    Ok(denied.map(|reply| {
        let mut response = (status(reply.status), [(header::CONTENT_TYPE, JSON_UTF8)], reply.body.0).into_response();
        *response.status_mut() = status(reply.status);
        response
    }))
}

async fn record_host_write(ctx: &Ctx, auth: &AuthData, kind: HostWriteKind, target: &AccountId, summary: Value) {
    let Some(runtime) = auth.host_runtime.clone() else {
        return;
    };
    let write = HostWrite {
        kind,
        target: rifts_seam::AccountId(target.to_string()),
        summary: rifts_seam::Json(summary.to_string()),
    };
    // Node's never throws
    if let Err(error) = ctx.shared.rifts.guard().record_host_write(runtime, write).await {
        tracing::error!("hostWrites.record: {error}");
    }
}

// ---- entitlements

/// `GET /entitlementsv2`.
pub async fn entitlements_get(call: Call) -> Response {
    let ctx = match Ctx::read(call).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    let auth = match ctx.auth() {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let user_id = auth.user_id.clone().unwrap_or_else(|| NO_PLAYER.to_owned());
    tracing::debug!("Entitlements for {user_id}");
    // The hook shapes the answer; no database is needed for an empty list
    let list = if user_id.is_empty() || user_id == NO_PLAYER || ctx.shared.store.is_none() {
        Vec::new()
    } else {
        let Ok(store) = ctx.store() else {
            return empty(500);
        };
        let account = match account_of(store, &user_id).await {
            Ok(account) => account,
            Err(error) => return internal_error("GET /entitlementsv2", &error),
        };
        match account {
            None => Vec::new(),
            Some(account) => match store
                .read(move |conn, repos| entitlements_for_account(repos, conn, &account))
                .await
            {
                Ok(list) => list,
                Err(error) => return internal_error("GET /entitlementsv2", &error),
            },
        }
    };
    json_reply(200, ctx.shared.version.hooks.entitlements_payload(&list))
}

/// `POST /entitlementv2/:userId`: a stub.
pub async fn entitlement_post(call: Call) -> Response {
    let ctx = match Ctx::read(call).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    if let Err(response) = ctx.auth() {
        return response;
    }
    tracing::debug!("Entitlements (stubbed)");
    envelope(200, "OK", json!([]))
}

// ---- cooldowns

/// `GET /cooldown/:userId`.
pub async fn cooldown_get(call: Call) -> Response {
    let ctx = match Ctx::read(call).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    let auth = match ctx.auth() {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let user_id = ctx.param("userId");
    let persistence = ctx.persistence(COOLDOWN_PERSISTENCE);
    let account = match resolve_if_needed(&ctx, auth, &user_id, persistence).await {
        Ok(account) => account,
        Err(response) => return response,
    };
    if !is_authorized_cooldown_reader(auth, &user_id, account.as_ref()) {
        tracing::warn!(
            "Rejected cross-account cooldown read actor={} target={user_id}",
            auth.actor()
        );
        return empty(403);
    }
    let on = ctx.monitor_on();
    if !persistence || user_id == NO_PLAYER {
        let why = if persistence {
            "no-player sentinel"
        } else {
            "persistence off (stub)"
        };
        monitor::cooldown_load(on, &user_id, auth.monitor_source(), &[], why);
        return envelope(200, "OK", json!({}));
    }
    let record = match (&account, ctx.store()) {
        (Some(account), Ok(store)) => {
            let account = account.clone();
            match store
                .read(move |conn, repos| repos.cooldown_states.get(conn, &account))
                .await
            {
                Ok(record) => record,
                Err(error) => return internal_error("GET /cooldown", &error),
            }
        }
        (_, Err(response)) => return response,
        (None, _) => None,
    };
    let version = record.as_ref().map_or(0, |r| r.update_version);
    monitor::cooldown_load(
        on,
        &user_id,
        auth.monitor_source(),
        &cooldown_list(record.as_ref()),
        &format!("stored version={version}, served as an id->date map"),
    );
    envelope(200, "OK", cooldown_state_payload(record.as_ref()))
}

/// The target's account, looked up only where the answer needs it: a game server or the player themselves is judged
/// without one, a host runtime by its roster.
async fn resolve_if_needed(
    ctx: &Ctx,
    auth: &AuthData,
    user_id: &str,
    persistence: bool,
) -> Result<Option<AccountId>, Response> {
    let needed = persistence && user_id != NO_PLAYER && ctx.shared.store.is_some();
    if !needed && !auth.is_host_runtime() {
        return Ok(None);
    }
    let Ok(store) = ctx.store() else {
        return Ok(None);
    };
    account_of(store, user_id)
        .await
        .map_err(|e| internal_error("account lookup", &e))
}

/// The save loop of `SaveCooldowns`: read, merge, compare-and-set on the stored version, up to 8 times.
async fn save_cooldowns(
    store: &IslesStore,
    account: &AccountId,
    actor: &ActorContext,
    body: Option<Value>,
    host_rules: bool,
) -> RepoResult<
    Result<
        (
            crate::repo::bounties::CooldownStateRecord,
            Option<super::cooldown_state::HostCooldownChanges>,
        ),
        CooldownSaveValidationError,
    >,
> {
    let id = account.clone();
    store
        .account_write(account, actor, move |tx, repos| {
            for _ in 0..MAX_SAVE_ATTEMPTS {
                let previous = repos.cooldown_states.get(tx.conn(), &id)?;
                let now = now_iso();
                let (next, changes) = if host_rules {
                    match merge_host_cooldown_state(&id, previous.as_ref(), body.as_ref(), &now) {
                        Ok((record, changes)) => (record, Some(changes)),
                        Err(e) => return Ok(Err(e)),
                    }
                } else {
                    match validate_cooldown_save(body.as_ref()) {
                        Ok(incoming) => (merge_cooldown_state(&id, previous.as_ref(), &incoming, &now), None),
                        Err(e) => return Ok(Err(e)),
                    }
                };
                if let Some(saved) =
                    repos
                        .cooldown_states
                        .save_if_version(tx, &next, previous.as_ref().map(|p| p.update_version))?
                {
                    return Ok(Ok((saved, changes)));
                }
            }
            Err(RepoError::Data(format!("Cooldown save remained contended for {id}")))
        })
        .await
}

/// `PUT /cooldown/batch/:userId`.
pub async fn cooldown_batch(call: Call) -> Response {
    let ctx = match Ctx::read(call).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    let auth = match ctx.auth() {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let user_id = ctx.param("userId");
    // A player-host runtime may save cooldowns too, but only later than the stored date
    let is_host_write = !is_authorized_cooldown_writer(auth) && is_host_cooldown_writer(auth);
    if !is_authorized_cooldown_writer(auth) && !is_host_write {
        tracing::warn!(
            "Rejected cooldown write without gameserver authority actor={} target={user_id}",
            auth.actor()
        );
        return empty(403);
    }
    let on = ctx.monitor_on();
    let source = auth.monitor_source();
    let persistence = ctx.persistence(COOLDOWN_PERSISTENCE);
    if !persistence || user_id == NO_PLAYER {
        let why = if persistence {
            "no-player sentinel, not stored"
        } else {
            "persistence off (stub), not stored"
        };
        monitor::cooldown_save(on, &user_id, source, ctx.body.as_ref(), why);
        return envelope(200, "OK", json!({}));
    }
    let store = match ctx.store() {
        Ok(store) => store,
        Err(response) => return response,
    };
    let account = match account_of(store, &user_id).await {
        Ok(Some(account)) => account,
        // Nothing to key the state by: answered as a save that stored nothing
        Ok(None) => {
            monitor::cooldown_save(on, &user_id, source, ctx.body.as_ref(), "unknown account, not stored");
            return envelope(200, "OK", json!({}));
        }
        Err(error) => return internal_error("PUT /cooldown/batch", &error),
    };
    if is_host_write {
        match authorize_host_write(&ctx, auth, HostWriteKind::Cooldown, &account).await {
            Ok(None) => {}
            Ok(Some(response)) => {
                tracing::warn!("Rejected host cooldown write actor={} target={user_id}", auth.actor());
                return response;
            }
            Err(response) => return response,
        }
    }
    let actor = actor_for(auth, &account, &ctx.request_id);
    match save_cooldowns(store, &account, &actor, ctx.body.clone(), is_host_write).await {
        Ok(Ok((record, changes))) => {
            let mut outcome = format!(
                "stored -> {} cooldown(s) version={}",
                record.entries.len(),
                record.update_version
            );
            if let Some(c) = &changes {
                outcome.push_str(&format!(
                    " host: added={} raised={} unchanged={} keptLater={} droppedNew={}",
                    c.added_count, c.raised_count, c.unchanged, c.kept_later_count, c.dropped_new
                ));
            }
            monitor::cooldown_save(on, &user_id, source, ctx.body.as_ref(), &outcome);
            if is_host_write {
                let mut summary = Map::new();
                summary.insert("route".into(), "batch".into());
                if let Some(changes) = &changes {
                    summary.extend(changes.to_json());
                }
                summary.insert("version".into(), record.update_version.into());
                record_host_write(&ctx, auth, HostWriteKind::Cooldown, &account, Value::Object(summary)).await;
            }
            envelope(200, "OK", cooldown_state_payload(Some(&record)))
        }
        Ok(Err(error)) => {
            monitor::cooldown_save(on, &user_id, source, ctx.body.as_ref(), &format!("REJECTED {error}"));
            envelope(400, "Bad Request", json!({}))
        }
        Err(error) => internal_error("PUT /cooldown/batch", &error),
    }
}

/// `PUT` and `POST /cooldown/:userId/:cooldownId`: `StartCooldownEndpoint`, one cooldown started at the backend's own
/// now (a harvested sprout, `UCooldownComponent::StartCooldown`). A 2xx releases the harvest reward, so it never
/// answers 200 without storing: persistence off is 503, the INVALID sentinel 400. Any body is ignored, so a caller
/// cannot backdate a cooldown.
pub async fn cooldown_start(call: Call) -> Response {
    // `/cooldown/batch/<uid>` fits this path too; the handler skips it before auth runs
    if call.params.get("userId") == Some("batch") {
        return empty(404);
    }
    let ctx = match Ctx::read(call).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    let auth = match ctx.auth() {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let user_id = ctx.param("userId");
    let cooldown_id = ctx.param("cooldownId");
    let on = ctx.monitor_on();
    let source = auth.monitor_source();
    let refuse = |code: u16, message: &str, outcome: &str| {
        monitor::cooldown_start(on, &user_id, source, &cooldown_id, outcome);
        if code == 403 {
            tracing::warn!(
                "Rejected cooldown write without gameserver authority actor={} target={user_id}",
                auth.actor()
            );
            return empty(403);
        }
        envelope(code, message, json!({}))
    };
    let is_host_write = !is_authorized_cooldown_writer(auth) && is_host_cooldown_writer(auth);
    if !is_authorized_cooldown_writer(auth) && !is_host_write {
        return refuse(403, "Forbidden", "REFUSED without gameserver authority");
    }
    if !is_valid_cooldown_id(&cooldown_id) {
        return refuse(400, "Bad Request", "REJECTED malformed cooldown id");
    }
    if user_id == NO_PLAYER {
        return refuse(400, "Bad Request", "REFUSED no-player sentinel, not stored");
    }
    if !ctx.persistence(COOLDOWN_PERSISTENCE) {
        return refuse(503, "Service Unavailable", "REFUSED persistence off, not stored");
    }
    let store = match ctx.store() {
        Ok(store) => store,
        Err(response) => return response,
    };
    let account = match account_of(store, &user_id).await {
        Ok(Some(account)) => account,
        Ok(None) => return refuse(404, "Not Found", "REFUSED unknown account, not stored"),
        Err(error) => return internal_error("PUT /cooldown", &error),
    };
    if is_host_write {
        match authorize_host_write(&ctx, auth, HostWriteKind::Cooldown, &account).await {
            Ok(None) => {}
            Ok(Some(response)) => {
                monitor::cooldown_start(on, &user_id, source, &cooldown_id, "REFUSED host write");
                tracing::warn!("Rejected host cooldown write actor={} target={user_id}", auth.actor());
                return response;
            }
            Err(response) => return response,
        }
    }
    let started = now_iso();
    let body = json!({ "cooldowns": [{ "cooldown_id": cooldown_id, "cooldown_started_date": started }] });
    let actor = actor_for(auth, &account, &ctx.request_id);
    match save_cooldowns(store, &account, &actor, Some(body), is_host_write).await {
        Ok(Ok((record, changes))) => {
            if is_host_write {
                let mut summary = Map::new();
                summary.insert("route".into(), "start".into());
                summary.insert("id".into(), cooldown_id.clone().into());
                summary.insert("startedAt".into(), started.clone().into());
                if let Some(changes) = &changes {
                    summary.extend(changes.to_json());
                }
                record_host_write(&ctx, auth, HostWriteKind::Cooldown, &account, Value::Object(summary)).await;
            }
            let mut kept = String::new();
            if let Some(c) = &changes {
                if c.kept_later_count > 0 {
                    kept.push_str(" (kept the stored later date: a host cannot move a cooldown earlier)");
                }
                if c.refused_future_count > 0 {
                    kept.push_str(" (refused: a host cannot start a cooldown that far ahead)");
                }
            }
            monitor::cooldown_start(
                on,
                &user_id,
                source,
                &cooldown_id,
                &format!(
                    "stored @{started} -> {} cooldown(s) version={}{kept}",
                    record.entries.len(),
                    record.update_version
                ),
            );
            envelope(200, "OK", cooldown_state_payload(Some(&record)))
        }
        Ok(Err(error)) => refuse(400, "Bad Request", &format!("REJECTED {error}")),
        Err(error) => {
            // A store failure answers 500 (no 2xx, so no harvest reward)
            monitor::cooldown_start(on, &user_id, source, &cooldown_id, &format!("ERROR {error}"));
            internal_error("PUT /cooldown", &error)
        }
    }
}

// ---- bounties

/// `GET /bounty/game-data`: a stub.
pub async fn bounty_game_data(call: Call) -> Response {
    let ctx = match Ctx::read(call).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    if let Err(response) = ctx.auth() {
        return response;
    }
    tracing::info!("Bounty game data (stubbed)");
    envelope(
        200,
        "OK",
        json!({
            "max_slots": 4,
            "num_draft_options": 3,
            "num_spicy_options": 1,
            "bounty_token_id": "TOKEN_BOUNTY_DRAFT",
            "premium_bounty_token_id": "TOKEN_BOUNTY_DRAFT_PREMIUM",
            "num_tokens_hp_start": 4,
            "num_tokens_per_day": 0,
            "bounty_token_grant_hour": 0,
            "history_length": 10,
            "bronze_count": 9,
            "silver_count": 3,
            "gold_count": 1,
            "new_season_reset_bounties": false,
            "bounty_data": [],
            "item_grant_data": [],
            "token_rollover_warning_days": 1000,
            "automatic_draft": false,
            "automatic_claim": false,
            "delete_claimed_bounties": false,
        }),
    )
}

/// `GET /bounty/:userId`.
pub async fn bounty_get(call: Call) -> Response {
    let ctx = match Ctx::read(call).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    let auth = match ctx.auth() {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let user_id = ctx.param("userId");
    let persistence = ctx.persistence(BOUNTY_PERSISTENCE);
    let account = match resolve_if_needed(&ctx, auth, &user_id, persistence).await {
        Ok(account) => account,
        Err(response) => return response,
    };
    if !is_authorized_bounty_reader(auth, &user_id, account.as_ref()) {
        tracing::warn!(
            "Rejected cross-account bounty read actor={} target={user_id}",
            auth.actor()
        );
        return empty(403);
    }
    let on = ctx.monitor_on();
    let source = auth.monitor_source();
    if !persistence || user_id == NO_PLAYER {
        let why = if persistence {
            "no-player sentinel"
        } else {
            "persistence off (stub)"
        };
        monitor::bounty_load(on, &user_id, source, why);
        return envelope(200, "OK", bounty_state_payload(None));
    }
    let record = match (&account, ctx.store()) {
        (Some(account), Ok(store)) => {
            let account = account.clone();
            match store
                .read(move |conn, repos| repos.bounty_states.get(conn, &account))
                .await
            {
                Ok(record) => record,
                Err(error) => return internal_error("GET /bounty", &error),
            }
        }
        (_, Err(response)) => return response,
        (None, _) => None,
    };
    monitor::bounty_load(
        on,
        &user_id,
        source,
        &format!(
            "stored {} version={}",
            bounty_group_counts(record.as_ref()),
            record.as_ref().map_or(0, |r| r.update_version)
        ),
    );
    envelope(200, "OK", bounty_state_payload(record.as_ref()))
}

type BountySave = Result<
    (
        crate::repo::bounties::BountyStateRecord,
        super::bounty_state::BountyMergeSummary,
    ),
    BountySaveValidationError,
>;

/// `SaveBountyState` and its host twin: read, merge, compare-and-set on the stored version, up to 8 times.
async fn save_bounty_state(
    store: &IslesStore,
    account: &AccountId,
    actor: &ActorContext,
    body: Option<Value>,
    host: bool,
) -> RepoResult<BountySave> {
    let id = account.clone();
    store
        .account_write(account, actor, move |tx, repos| {
            for _ in 0..MAX_SAVE_ATTEMPTS {
                let previous = repos.bounty_states.get(tx.conn(), &id)?;
                let now = now_iso();
                let merged = if host {
                    merge_host_bounty_state(&id, previous.as_ref(), body.as_ref(), &now)
                } else {
                    merge_bounty_state(&id, previous.as_ref(), body.as_ref(), &now)
                };
                let (record, summary) = match merged {
                    Ok(merged) => merged,
                    Err(error) => return Ok(Err(error)),
                };
                if let Some(saved) =
                    repos
                        .bounty_states
                        .save_if_version(tx, &record, previous.as_ref().map(|p| p.update_version))?
                {
                    return Ok(Ok((saved, summary)));
                }
            }
            Err(RepoError::Data(format!(
                "Bounty state save remained contended for {id}"
            )))
        })
        .await
}

/// `POST /bounty/:userId`.
pub async fn bounty_post(call: Call) -> Response {
    let ctx = match Ctx::read(call).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    let auth = match ctx.auth() {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let user_id = ctx.param("userId");
    // A player-host runtime (a Paradox-account realm) saves bounties like a dedicated server, claims included, once the
    // host-write hooks allow it
    let is_host_write = !is_authorized_bounty_writer(auth) && is_host_bounty_writer(auth);
    if !is_authorized_bounty_writer(auth) && !is_host_write {
        tracing::warn!(
            "Rejected bounty write without gameserver authority actor={} target={user_id}",
            auth.actor()
        );
        return empty(403);
    }
    let on = ctx.monitor_on();
    let source = auth.monitor_source();
    let persistence = ctx.persistence(BOUNTY_PERSISTENCE);
    if !persistence || user_id == NO_PLAYER {
        let why = if persistence {
            "no-player sentinel, not stored"
        } else {
            "persistence off (stub), not stored"
        };
        monitor::bounty_save(on, &user_id, source, ctx.body.as_ref(), why);
        return envelope(200, "OK", bounty_state_payload(None));
    }
    let store = match ctx.store() {
        Ok(store) => store,
        Err(response) => return response,
    };
    if auth.is_gameserver
        && !is_host_write
        && let Some(foreign_account) = crate::areas::matchmaking::resolve_foreign_account(&ctx.shared, &user_id).await
    {
        let instance_id = ctx
            .shared
            .matchmaking
            .ring_instance_for_player(&foreign_account)
            .unwrap_or_else(|| module_api::GlobalId::new(module_api::network::IdKind::Hunt, &ctx.shared.node.node));
        let module_name = format!("isles-{}", ctx.shared.version.tables.game_version.replace('.', "-"));
        let Ok(module) = module_api::ModuleId::new(module_name) else {
            return internal_error("POST /bounty", &"invalid module id");
        };
        let bounty_progress = match ctx.body.clone() {
            Some(Value::Object(map)) => Value::Object(map),
            other => json!({ "bounty": other }),
        };
        let rewards = module_api::network::HuntRewards {
            hunt: instance_id,
            module,
            account: foreign_account.clone(),
            xp_grant: None,
            loot_write: None,
            objective_write: Some(json!({ "bounty_progress": bounty_progress })),
        };
        if let Err(error) = ctx.shared.home().ring_forward_hunt_rewards(&rewards).await {
            return internal_error("POST /bounty", &error);
        }
        return envelope(200, "OK", bounty_state_payload(None));
    }
    let account = match account_of(store, &user_id).await {
        Ok(Some(account)) => account,
        Ok(None) => {
            monitor::bounty_save(on, &user_id, source, ctx.body.as_ref(), "unknown account, not stored");
            return envelope(200, "OK", bounty_state_payload(None));
        }
        Err(error) => return internal_error("POST /bounty", &error),
    };
    if is_host_write {
        match authorize_host_write(&ctx, auth, HostWriteKind::Bounty, &account).await {
            Ok(None) => {}
            Ok(Some(response)) => {
                tracing::warn!("Rejected host bounty write actor={} target={user_id}", auth.actor());
                return response;
            }
            Err(response) => return response,
        }
    }
    let actor = actor_for(auth, &account, &ctx.request_id);
    match save_bounty_state(store, &account, &actor, ctx.body.clone(), is_host_write).await {
        Ok(Ok((record, summary))) => {
            let mut outcome = format!(
                "stored {}{} incoming={} removed={} -> {} version={}",
                summary.mode,
                summary.group.map_or_else(String::new, |g| format!(":{}", g.as_str())),
                summary.incoming,
                summary.removed,
                bounty_group_counts(Some(&record)),
                record.update_version
            );
            if let Some(host) = &summary.host {
                outcome.push_str(&format!(
                    " host: keptClaimedWeekly={} ignoredWeekly={} clampedProgress={}",
                    host.kept_claimed_weekly, host.ignored_weekly, host.clamped_progress
                ));
            }
            monitor::bounty_save(on, &user_id, source, ctx.body.as_ref(), &outcome);
            let claimed = newly_claimed_bounty_ids(&record).len();
            tracing::debug!("{claimed} season challenge(s) claimed");
            crate::areas::reforged_api::activity::record_challenge_claims_activity(&ctx.shared, &user_id, claimed);
            if is_host_write {
                record_host_write(
                    &ctx,
                    auth,
                    HostWriteKind::Bounty,
                    &account,
                    host_bounty_write_summary(&summary, &record),
                )
                .await;
            }
            // The same envelope as the GET (the full merged state), as the live service answered a save
            envelope(200, "OK", bounty_state_payload(Some(&record)))
        }
        Ok(Err(error)) => {
            monitor::bounty_save(on, &user_id, source, ctx.body.as_ref(), &format!("REJECTED {error}"));
            envelope(400, "Bad Request", json!({}))
        }
        Err(error) => internal_error("POST /bounty", &error),
    }
}

/// `POST` and `DELETE /bounty/delete/:userId` (`HandleBountyDelete`). Claiming a completed bounty deletes it here first;
/// the game grants the reward only after a 200. Season challenges are never deleted, and a body that names none is
/// refused; a call that names nothing (the reset, with no body) changes nothing and answers 200.
pub async fn bounty_delete(call: Call) -> Response {
    let ctx = match Ctx::read(call).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    let auth = match ctx.auth() {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let user_id = ctx.param("userId");
    let raw: String = ctx
        .body
        .as_ref()
        .map_or_else(|| "null".to_owned(), service_kit::jsjson::stringify)
        .chars()
        .take(2000)
        .collect();
    let is_host_write = !is_authorized_bounty_writer(auth) && is_host_bounty_writer(auth);
    if !is_authorized_bounty_writer(auth) && !is_host_write {
        tracing::warn!(
            "Rejected bounty delete without gameserver authority actor={} target={user_id}",
            auth.actor()
        );
        return empty(403);
    }
    // The host-write hook runs before the body is looked at, as in Node; the target account is looked up for it
    let persistence = ctx.persistence(BOUNTY_PERSISTENCE);
    let account = if is_host_write || (persistence && user_id != NO_PLAYER) {
        match ctx.store() {
            Ok(store) => match account_of(store, &user_id).await {
                Ok(account) => account,
                Err(error) => return internal_error("POST /bounty/delete", &error),
            },
            Err(response) => return response,
        }
    } else {
        None
    };
    if is_host_write {
        let Some(target) = &account else {
            return empty(403);
        };
        match authorize_host_write(&ctx, auth, HostWriteKind::Bounty, target).await {
            Ok(None) => {}
            Ok(Some(response)) => return response,
            Err(response) => return response,
        }
    }
    if bounty_delete_names_nothing(ctx.body.as_ref()) {
        tracing::info!(
            "Bounty reset request acknowledged, nothing changed (source={})",
            auth.monitor_source()
        );
        return envelope(200, "OK", json!({}));
    }
    if !persistence || user_id == NO_PLAYER {
        tracing::info!(
            "[DraftMon] BOUNTY-DELETE user={user_id} {}, not stored body={raw}",
            if persistence {
                "no-player sentinel"
            } else {
                "persistence off (stub)"
            }
        );
        return envelope(200, "OK", bounty_state_payload(None));
    }
    let (Some(account), Ok(store)) = (account, ctx.store()) else {
        tracing::info!("[DraftMon] BOUNTY-DELETE user={user_id} unknown account, not stored body={raw}");
        return envelope(200, "OK", bounty_state_payload(None));
    };
    let actor = actor_for(auth, &account, &ctx.request_id);
    let id = account.clone();
    let body = ctx.body.clone();
    let result = store
        .account_write(&account, &actor, move |tx, repos| {
            for _ in 0..MAX_SAVE_ATTEMPTS {
                let previous = repos.bounty_states.get(tx.conn(), &id)?;
                let (record, summary) = match delete_bounties(&id, previous.as_ref(), body.as_ref(), &now_iso()) {
                    Ok(deleted) => deleted,
                    Err(error) => return Ok(Err(error)),
                };
                if let Some(saved) =
                    repos
                        .bounty_states
                        .save_if_version(tx, &record, previous.as_ref().map(|p| p.update_version))?
                {
                    return Ok(Ok((saved, summary)));
                }
            }
            Err(RepoError::Data(format!("Bounty delete remained contended for {id}")))
        })
        .await;
    match result {
        Ok(Ok((record, summary))) => {
            tracing::info!(
                "[DraftMon] BOUNTY-DELETE user={user_id} deleted=[{}] kept=[{}] -> {} version={} body={raw}",
                summary.deleted.join(", "),
                summary.kept.join(", "),
                bounty_group_counts(Some(&record)),
                record.update_version
            );
            envelope(200, "OK", bounty_state_payload(Some(&record)))
        }
        Ok(Err(error)) => {
            tracing::warn!("[DraftMon] BOUNTY-DELETE user={user_id} REJECTED {error} body={raw}");
            envelope(400, "Bad Request", json!({}))
        }
        Err(error) => internal_error("POST /bounty/delete", &error),
    }
}

// ---- wallet and store

/// The authenticated user's account and wallet; no account (or `INVALID`) is an empty wallet, as Node's `GetWallet`
/// answers `{}`. The first access creates the wallet with the starter grant.
async fn wallet_for(ctx: &Ctx, auth: &AuthData) -> Result<(String, Option<AccountId>, Vec<(String, f64)>), Response> {
    let user_id = auth.user_id.clone().unwrap_or_else(|| NO_PLAYER.to_owned());
    if user_id.is_empty() || user_id == NO_PLAYER {
        return Ok((user_id, None, Vec::new()));
    }
    let store = ctx.store()?;
    let Some(account) = account_of(store, &user_id)
        .await
        .map_err(|e| internal_error("account lookup", &e))?
    else {
        return Ok((user_id, None, Vec::new()));
    };
    let id = account.clone();
    let found = store
        .read(move |conn, repos| repos.wallets.find(conn, &id))
        .await
        .map_err(|e| internal_error("wallet", &e))?;
    let balances = match found {
        Some(wallet) => wallet.balances,
        None => {
            let id = account.clone();
            let actor = actor_for(auth, &account, &ctx.request_id);
            store
                .account_write(&account, &actor, move |tx, repos| get_wallet(repos, tx, &id))
                .await
                .map_err(|e| internal_error("wallet", &e))?
        }
    };
    Ok((user_id, Some(account), balances))
}

/// `POST /reconcile`.
pub async fn reconcile(call: Call) -> Response {
    let ctx = match Ctx::read(call).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    let auth = match ctx.auth() {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let (user_id, _, wallet) = match wallet_for(&ctx, auth).await {
        Ok(wallet) => wallet,
        Err(response) => return response,
    };
    let balances = project_balances(&wallet, Map::new());
    let get = |key: &str| {
        balances
            .get(key)
            .map_or_else(|| "0".to_owned(), service_kit::jsjson::stringify)
    };
    tracing::info!(
        "Reconcile for {user_id}: Rams={} CombatMerit={}",
        get("CURRENCY_NOTES"),
        get("CURRENCY_PJM_WEAPON")
    );
    json_reply(200, json!({ "balances": balances, "refreshInventory": true }))
}

/// `GET /creator`: SupportACreator, stubbed.
pub async fn creator(call: Call) -> Response {
    let ctx = match Ctx::read(call).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    if let Err(response) = ctx.auth() {
        return response;
    }
    tracing::info!("SupportACreator (stubbed)");
    json_reply(
        200,
        json!({
            "expirationDate": "2099-01-01T01:00:00.041Z",
            "slug": "MROWMROW",
            "success": true,
        }),
    )
}

/// The currencies `GET /balance` always lists, in the client's captured order; the value is the starting balance.
const BALANCE_BASE: [(&str, i64); 52] = [
    ("id_currency_s20_coin", 0),
    ("CURRENCY_GAUNTLET_COIN_FADED", 0),
    ("CURRENCY_S20_COIN", 0),
    ("CURRENCY_S18_COIN", 0),
    ("id_currency_seasonal_coin", 0),
    ("id_currency_s18_coin", 0),
    ("id_currency_weapon_token", 25),
    ("id_currency_celldust", 0),
    ("id_currency_event_ramsgiving", 0),
    ("CURRENCY_NOTES", -1),
    ("id_currency_event_frostfall", 0),
    ("CURRENCY_EVENT_DARKHARVEST", 0),
    ("CURRENCY_S19_COIN", 0),
    ("id_currency_s16_coin", 0),
    ("CURRENCY_S16_COIN", 0),
    ("id_currency_gauntlet_coin", 0),
    ("id_currency_s13_coin", 0),
    ("CURRENCY_MARKS_STEEL", 0),
    ("CURRENCY_S13_COIN", 0),
    ("CURRENCY_EVENT_FROSTFALL", 0),
    ("CURRENCY_GAUNTLET_COIN", 0),
    ("id_currency_marks_steel", 0),
    ("id_currency_rewardcache", 0),
    ("CURRENCY_PRESTIGE", 0),
    ("CURRENCY_SEASONAL_COIN", 0),
    ("CURRENCY_REWARDCACHE", 0),
    ("id_currency_token_exchange_speed_up", 0),
    ("id_currency_event_springtide", 0),
    ("CURRENCY_TOKEN_EXCHANGE_SPEED_UP", 0),
    ("id_currency_gauntlet_coin_faded", 0),
    ("CURRENCY_S15_COIN", 0),
    ("CURRENCY_PLATINUM", 0),
    ("id_currency_platinum", 0),
    ("id_currency_s15_coin", 0),
    ("id_currency_marks_gilded", 0),
    ("id_currency_event_darkharvest", 0),
    ("id_currency_event_saintsbond", 0),
    ("CURRENCY_EVENT_SPRINGTIDE", 0),
    ("id_currency_s19_coin", 0),
    ("id_currency_notes", -1),
    ("id_currency_prestige", 0),
    ("id_currency_s13_daily", 0),
    ("CURRENCY_WEAPON_TOKEN", 25),
    ("CURRENCY_MARKS_GILDED", 0),
    ("CURRENCY_S13_DAILY", 0),
    ("CURRENCY_CELLDUST", 0),
    ("CURRENCY_S14_COIN", 0),
    ("CURRENCY_EVENT_SAINTSBOND", 0),
    ("CURRENCY_S17_COIN", 0),
    ("id_currency_s14_coin", 0),
    ("CURRENCY_EVENT_RAMSGIVING", 0),
    ("id_currency_s17_coin", 0),
];

/// `GET /balance`: the captured currency list with the wallet laid over it (both naming forms), the platinum buckets
/// collapsed to the one key the client reads.
pub async fn balance(call: Call) -> Response {
    let ctx = match Ctx::read(call).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    let auth = match ctx.auth() {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let (user_id, _, wallet) = match wallet_for(&ctx, auth).await {
        Ok(wallet) => wallet,
        Err(response) => return response,
    };
    let held = |id: &str| wallet.iter().find(|(k, _)| k == id).map(|(_, v)| *v);
    // `Wallet.CURRENCY_NOTES ?? GetNotesForUser(userId)`, which is the wallet's value or 0
    let notes = held("CURRENCY_NOTES").unwrap_or(0.0);
    tracing::info!(
        "Fetched balance for userId {user_id}: Rams(NOTES)={} CombatMerit(PJM_WEAPON)={}",
        super::util::num(notes),
        super::util::num(held("CURRENCY_PJM_WEAPON").unwrap_or(0.0))
    );
    let base: Map<String, Value> = BALANCE_BASE
        .iter()
        .map(|(id, value)| {
            let amount = if *value < 0 {
                super::util::num(notes)
            } else {
                Value::from(*value)
            };
            ((*id).to_owned(), amount)
        })
        .collect();
    json_reply(200, Value::Object(project_balances(&wallet, base)))
}

/// `GET /product/skus/public?requiredTags=<tag>`: one tag per store section.
pub async fn product_skus(call: Call) -> Response {
    let ctx = match Ctx::read(call).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    let auth = match ctx.auth() {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let tag = match query_param(&ctx.query, "requiredTags") {
        Some(QueryValue::One(tag)) if !tag.is_empty() => tag,
        _ => {
            return json_reply(
                400,
                json!({ "code": "400", "message": "missing requiredTags query parameter" }),
            );
        }
    };
    let catalog = ctx.catalog();
    let skus = match skus_for_player(&ctx, &catalog, &tag, auth.user_id.as_deref()).await {
        Ok(skus) => skus,
        Err(response) => return response,
    };
    if skus.is_empty() {
        // A daily section already logged why (claimed this window, no data file, lookup failed)
        if !is_daily_claim_section(&tag) {
            tracing::info!("Store SKUs requested for unimplemented tag '{tag}' - returning empty");
        }
        return json_reply(200, json!([]));
    }
    let listing: Vec<Value> = skus.iter().map(|sku| sku_listing(&ctx.shared.settings, sku)).collect();
    tracing::info!("Store SKUs: {tag} -> {} SKUs", listing.len());
    json_reply(200, Value::Array(listing))
}

/// `GetStoreSkusForPlayer`: every section as is but the daily ones, whose offer is left out once this window's claim is
/// in the inventory ledger (the only "claimed" signal the client honours) and whenever the claim cannot be checked.
async fn skus_for_player(
    ctx: &Ctx,
    catalog: &StoreCatalog,
    tag: &str,
    user_id: Option<&str>,
) -> Result<Vec<super::store::StoreSku>, Response> {
    let skus = catalog.skus(tag);
    if !is_daily_claim_section(tag) {
        return Ok(skus.to_vec());
    }
    if skus.is_empty() {
        tracing::info!(
            "[Store:{tag}] no daily offer loaded (fountain_daily_bundle.json missing or rejected) - serving none"
        );
        return Ok(Vec::new());
    }
    let Some(user_id) = user_id.filter(|id| !id.is_empty() && *id != NO_PLAYER) else {
        tracing::warn!("[Store:{tag}] daily offer requested without a player id - serving none");
        return Ok(Vec::new());
    };
    let window = daily_reset_window(now_ms());
    let account = match ctx.shared.store.as_ref() {
        Some(store) => account_of(store, user_id)
            .await
            .map_err(|e| internal_error("account lookup", &e))?,
        None => None,
    };
    let (Some(account), Some(seams)) = (account, ctx.seams.as_ref()) else {
        tracing::warn!("[Store:{tag}] {user_id} claim lookup unavailable - hiding the offer");
        return Ok(Vec::new());
    };
    let mut open = Vec::new();
    for sku in skus {
        let id = daily_claim_transaction_id(tag, user_id, &sku.id, &window.key);
        match seams.inventory.find_claim(account.clone(), id).await {
            Ok(Some(_)) => {}
            Ok(None) => open.push(sku.clone()),
            Err(error) => {
                tracing::warn!(
                    "[Store:{tag}] {user_id} claim lookup failed for {} - hiding the offer: {error}",
                    sku.id
                );
            }
        }
    }
    tracing::info!(
        "[Store:{tag}] {user_id} window={} -> {}",
        window.key,
        if open.is_empty() {
            "claimed or unavailable".to_owned()
        } else {
            format!(
                "open ({})",
                open.iter().map(|s| s.id.as_str()).collect::<Vec<_>>().join(", ")
            )
        }
    );
    Ok(open)
}

/// `GET /token/:currency/:catalogId`: mints a purchase token for a SKU. The currency segment is client-side routing
/// only: the price and currency charged come from the SKU's own prices when the token is redeemed.
pub async fn token(call: Call) -> Response {
    let ctx = match Ctx::read(call).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    let auth = match ctx.auth() {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let catalog_id = ctx.param("catalogId");
    let catalog = ctx.catalog();
    let Some((store_tag, sku)) = catalog.find_by_id_or_catalog_id(&catalog_id) else {
        tracing::warn!(
            "[Store] GetPurchaseToken: unknown or ambiguous sku/catalogId '{catalog_id}' (currency={}) for {}",
            ctx.param("currency"),
            auth.actor()
        );
        return json_reply(404, json!({ "code": "404", "message": "unknown_sku" }));
    };
    let Some(seams) = ctx.seams.as_ref() else {
        return not_implemented(ctx.call_route, &ctx.request_id);
    };
    let user_id = auth.user_id.clone().unwrap_or_default();
    // Single slayer per account, and no characterId in this flow's request at all: resolved once, here, and carried
    // inside the token's claims
    let character_id = match (ctx.shared.store.as_ref(), user_id.as_str()) {
        (Some(store), id) if !id.is_empty() => match account_of(store, id).await {
            Ok(Some(account)) => seams
                .characters
                .first_character_id(account)
                .await
                .map_err(|e| internal_error("GetCharactersForUid", &e)),
            Ok(None) => Ok(None),
            Err(error) => Err(internal_error("account lookup", &error)),
        },
        _ => Ok(None),
    };
    let character_id = match character_id {
        Ok(Some(id)) => id,
        Ok(None) => {
            tracing::error!("[Store:{store_tag}] GetPurchaseToken: no character found for {user_id}");
            return json_reply(500, json!({ "code": "500", "message": "no_character" }));
        }
        Err(response) => return response,
    };
    let claims = PurchaseTokenClaims {
        user_id: user_id.clone(),
        character_id,
        sku_id: sku.id.clone(),
        store_tag: Some(store_tag.to_owned()),
    };
    match seams.tokens.sign(&claims) {
        Ok(token) => {
            tracing::info!("[Store:{store_tag}] Minted purchase token for {user_id} sku={}", sku.id);
            json_reply(200, json!({ "purchaseToken": token }))
        }
        Err(error) => internal_error("SignStorePurchaseToken", &error),
    }
}

/// `PurchaseStoreSku`: plans the purchase and hands the transaction to the inventory area.
async fn purchase_store_sku(
    ctx: &Ctx,
    catalog: &StoreCatalog,
    seams: &StoreSeams,
    claims: &PurchaseTokenClaims,
    store_tag: &str,
) -> Result<(), PurchaseReason> {
    let user_id = &claims.user_id;
    let Some(sku) = catalog.from_sku(store_tag, &claims.sku_id) else {
        return Err(PurchaseReason::UnknownSku);
    };
    let plan = plan_purchase(
        &ctx.shared.settings,
        sku,
        store_tag,
        user_id,
        now_ms(),
        &uuid::Uuid::new_v4().simple().to_string(),
    )?;
    let Some(store) = ctx.shared.store.as_ref() else {
        return Err(PurchaseReason::TransactionFailed);
    };
    let account = match account_of(store, user_id).await {
        Ok(Some(account)) => account,
        Ok(None) => return Err(PurchaseReason::TransactionFailed),
        Err(error) => {
            tracing::error!("[Store:{store_tag}] {user_id} account lookup failed: {error}");
            return Err(PurchaseReason::TransactionFailed);
        }
    };
    // The ledger inside the transaction stays the authority for a same-character replay; this refuses a claim this
    // window already holds under ANOTHER character, and names new vs replay for the [GrantMon] line. A failed lookup
    // refuses the claim: fail closed.
    let mut claim_state = "-";
    if plan.daily_window.is_some() {
        match seams
            .inventory
            .find_claim(account.clone(), plan.transaction_id.clone())
            .await
        {
            Err(error) => {
                tracing::warn!(
                    "[Store:{store_tag}] {user_id} claim lookup failed for {} - claim refused: {error}",
                    claim_state_id(sku)
                );
                return Err(PurchaseReason::TransactionFailed);
            }
            Ok(Some(existing)) if existing.character_id.as_deref() != Some(claims.character_id.as_str()) => {
                tracing::warn!(
                    "[Store:{store_tag}] {user_id} already claimed {} in window {} under character {} - refused for character {}",
                    sku.id,
                    plan.daily_window.as_ref().map_or("", |w| w.key.as_str()),
                    existing.character_id.as_deref().unwrap_or("?"),
                    claims.character_id
                );
                return Err(PurchaseReason::AlreadyPurchasedDifferentRequest);
            }
            Ok(Some(_)) => claim_state = "replay (no new grant)",
            Ok(None) => claim_state = "new",
        }
    }
    let transaction = StoreTransaction {
        account,
        user_id: user_id.clone(),
        character_id: claims.character_id.clone(),
        transaction_id: plan.transaction_id.clone(),
        instanced_to_add: plan.instanced_to_add.clone(),
        stacked_to_add: plan.stacked_to_add.clone(),
        stacked_to_remove: plan.stacked_to_remove.clone(),
        ownership_already_validated: true,
        diagnostic_label: format!("store:{store_tag}:{}", sku.id),
        grants: plan.grants.clone(),
        sku_id: sku.id.clone(),
    };
    match seams.inventory.run(transaction).await {
        Ok(_) => {
            tracing::info!(
                "[Store:{store_tag}] {user_id} purchased {} (transactionId={})",
                sku.id,
                plan.transaction_id
            );
            // A daily claim is a free grant that no game server issues, so it never passes POST /inventory's [GrantMon]
            if let (Some(window), true) = (&plan.daily_window, ctx.monitor_on()) {
                let list = |items: &[Value]| {
                    items
                        .iter()
                        .map(|i| {
                            format!(
                                "{} x{}",
                                service_kit::js::string(i.get("catalogId")),
                                service_kit::js::string(i.get("quantity"))
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                tracing::info!(
                    "[GrantMon] txn={} user={user_id} src=store source=\"store:{store_tag}\" sku={} window={} add=[{}] remove=[{}] claim={claim_state}",
                    plan.transaction_id,
                    sku.id,
                    window.key,
                    list(&plan.stacked_to_add),
                    list(&plan.stacked_to_remove)
                );
            }
            Ok(())
        }
        Err(TransactionFailure::InsufficientBalance) => Err(PurchaseReason::InsufficientBalance),
        Err(TransactionFailure::Conflict) => Err(PurchaseReason::Conflict),
        Err(TransactionFailure::Mismatch) => Err(PurchaseReason::AlreadyPurchasedDifferentRequest),
        Err(TransactionFailure::Failed(error)) => {
            tracing::error!("[Store:{store_tag}] {user_id} purchase of {} failed: {error}", sku.id);
            Err(PurchaseReason::TransactionFailed)
        }
    }
}

fn claim_state_id(sku: &super::store::StoreSku) -> &str {
    &sku.id
}

/// `POST /notification/:currency?token=...`: redeems a purchase token. Success is 204 with no body.
pub async fn notification(call: Call) -> Response {
    let ctx = match Ctx::read(call).await {
        Ok(ctx) => ctx,
        Err(response) => return response,
    };
    let auth = match ctx.auth() {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let user_id = auth.user_id.clone();
    let token = match query_param(&ctx.query, "token") {
        Some(QueryValue::One(token)) if !token.is_empty() => token,
        _ => return json_reply(400, json!({ "code": "400", "message": "missing token" })),
    };
    let Some(seams) = ctx.seams.as_ref() else {
        return not_implemented(ctx.call_route, &ctx.request_id);
    };
    let Ok(claims) = seams.tokens.validate(&token) else {
        tracing::warn!(
            "[Store] BuyFromPurchaseToken: invalid/expired token from {}",
            auth.actor()
        );
        return json_reply(401, json!({ "code": "401", "message": "invalid_token" }));
    };
    if user_id.as_deref() != Some(claims.user_id.as_str()) {
        tracing::error!(
            "[Store] BuyFromPurchaseToken: token userId {} does not match authenticated {}",
            claims.user_id,
            auth.actor()
        );
        return json_reply(403, json!({ "code": "403", "message": "token_user_mismatch" }));
    }
    let catalog = ctx.catalog();
    // Tokens minted before the store registry did not carry storeTag: accepted only when the SKU still resolves
    // unambiguously; every new token is explicitly store-bound
    let store_tag = claims.store_tag.clone().or_else(|| {
        catalog
            .find_by_id_or_catalog_id(&claims.sku_id)
            .map(|(tag, _)| tag.to_owned())
    });
    let Some(store_tag) = store_tag else {
        tracing::warn!(
            "[Store] BuyFromPurchaseToken: unknown or ambiguous sku={} for {}",
            claims.sku_id,
            auth.actor()
        );
        return json_reply(404, json!({ "code": "404", "message": "unknown_sku" }));
    };
    match purchase_store_sku(&ctx, &catalog, seams, &claims, &store_tag).await {
        Ok(()) => {
            tracing::info!(
                "[Store:{store_tag}] purchase completed for {} sku={}",
                auth.actor(),
                claims.sku_id
            );
            empty(204)
        }
        Err(reason) => {
            tracing::warn!(
                "[Store:{store_tag}] purchase denied for {} sku={}: {}",
                auth.actor(),
                claims.sku_id,
                reason.as_str()
            );
            json_reply(
                reason.status(),
                json!({ "code": reason.status().to_string(), "message": reason.as_str() }),
            )
        }
    }
}

/// What the inventory area calls to grant a purchase's entitlements and charge nothing else here:
/// `GrantEntitlementsInTransaction` with the account resolved.
pub fn grant_sku_entitlements(
    repos: &crate::repo::Repositories,
    tx: &mut crate::repo::Tx<'_>,
    transaction: &StoreTransaction,
) -> RepoResult<Vec<String>> {
    grant_entitlements_in_transaction(
        repos,
        tx,
        &transaction.account,
        &transaction.grants.entitlements,
        Some(&transaction.sku_id),
    )
}
