// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The chat area's backend routes (spec §6; core Appendix A B35, B80, B81, B137-B144): the friends API, the Slayer
//! link stubs, the guild stubs and `/vivox/login`. The `:accountId` param is never read: the owner is always the
//! caller, so one player cannot read or change another's list.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::Request;
use axum::response::{IntoResponse, Response};
use dainty_module_api::{Actor, ActorContext};
use dainty_service_kit::json::Json;
use http::{StatusCode, header};
use http_body_util::BodyExt;
use percent_encoding::percent_decode_str;
use rustic_isles::{Call, Handlers, Method, RouteDef, Service, not_implemented, replay, season};
use rustic_isles_auth::AuthData;
use rustic_isles_store::repo::auth::AccountRecord;
use rustic_isles_store::repo::chat::{FriendEdgeRecord, FriendshipDirection, FriendshipStatus};
use rustic_isles_store::{IslesStore, RepoError, RepoResult, UserId};
use serde_json::{Value, json};

/// The ISO text of now; tests pass a counting one.
pub type Clock = Arc<dyn Fn() -> String + Send + Sync>;

/// The most ids one availability answer lists.
pub const SLAYER_LINK_MAX_ACCOUNTS: usize = 256;
/// The longest id it lists, in UTF-16 code units.
const SLAYER_LINK_MAX_ID: usize = 128;
const BODY_LIMIT: usize = rustic_isles::BODY_LIMIT;

/// Adds the chat routes, timestamped by the wall clock.
pub fn register(handlers: &mut Handlers) {
    register_with(handlers, Arc::new(|| season::to_iso_string(replay::now_ms())));
}

/// Adds the chat routes with `clock` for the times they write.
pub fn register_with(handlers: &mut Handlers, clock: Clock) {
    use Method::{Delete, Get, Post};
    let b = Service::Backend;
    handlers.on(b, Get, "/vivox/login", |call| {
        authed(call, |_| empty(StatusCode::NOT_FOUND))
    });
    handlers.on(b, Get, "/guild/invite/player", |call| {
        authed(call, |_| envelope(json!({"invites": []})))
    });
    handlers.on(b, Get, "/guild", |call| authed(call, |_| empty(StatusCode::NO_CONTENT)));
    handlers.on(b, Get, "/friends/api/public/friends/:accountId", friends_list);
    let post_clock = Arc::clone(&clock);
    handlers.on(
        b,
        Post,
        "/friends/api/public/friends/:accountId/:friendId",
        move |call| invite_route(call, Arc::clone(&post_clock)),
    );
    handlers.on(
        b,
        Delete,
        "/friends/api/public/friends/:accountId/:friendId",
        remove_route,
    );
    handlers.on(b, Get, "/friends/api/public/blocklist/:accountId", |_| async {
        Json(json!([])).into_response()
    });
    handlers.on(
        b,
        Get,
        "/friends/api/public/list/:namespace/:accountId/recentPlayers",
        |_| async { Json(json!([])).into_response() },
    );
    handlers.on(b, Get, "/friends/api/v1/:accountId/settings", |_| async {
        Json(json!({"acceptInvites": "public", "mutualPrivacy": "ALL"})).into_response()
    });
    handlers.on(b, Post, "/slayerlink/availability", availability_route);
    handlers.on(b, Get, "/slayerlink/status_good", |call| {
        authed(call, |_| envelope(slayer_link_status()))
    });
}

fn empty(status: StatusCode) -> Response {
    status.into_response()
}

fn envelope(payload: Value) -> Response {
    Json(json!({"code": null, "message": "OK", "payload": payload})).into_response()
}

/// Game auth: the caller another layer already found, else auth's own answer.
async fn game_auth(call: &mut Call) -> Result<AuthData, Response> {
    if let Some(data) = call.request.extensions().get::<AuthData>() {
        return Ok(data.clone());
    }
    let shared = Arc::clone(&call.shared);
    shared.auth().game_auth(&mut call.request).await
}

async fn authed(mut call: Call, answer: impl FnOnce(AuthData) -> Response) -> Response {
    match game_auth(&mut call).await {
        Ok(data) => answer(data),
        Err(refused) => refused,
    }
}

/// `NotHome` and `Home` (the account lives elsewhere, DN5 not yet forwarding) are 503, the rest 500.
fn store_failure(route: &RouteDef, error: &RepoError) -> Response {
    let (method, path) = (route.method.as_str(), route.path);
    match error {
        RepoError::NotHome { .. } | RepoError::Home(_) => {
            tracing::warn!("{method} {path}: {error}");
            empty(StatusCode::SERVICE_UNAVAILABLE)
        }
        _ => {
            tracing::error!("{method} {path}: {error}");
            empty(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// Auth, then the store (core's 501 without one).
async fn prelude(call: &mut Call) -> Result<(AuthData, Arc<IslesStore>), Response> {
    let data = game_auth(call).await?;
    let store = call
        .shared
        .slot::<IslesStore>()
        .ok_or_else(|| not_implemented(call.route, &call.request_id))?;
    Ok((data, store))
}

fn decode_query_part(part: &str) -> String {
    percent_decode_str(&part.replace('+', " "))
        .decode_utf8_lossy()
        .into_owned()
}

/// Express 5's simple query parser: true only for exactly one `includePending` whose value is exactly `true`.
pub fn include_pending(query: Option<&str>) -> bool {
    let mut values = query
        .unwrap_or("")
        .split('&')
        .filter(|piece| !piece.is_empty())
        .filter_map(|piece| {
            let (key, value) = piece.split_once('=').unwrap_or((piece, ""));
            (decode_query_part(key) == "includePending").then(|| decode_query_part(value))
        });
    match (values.next(), values.next()) {
        (Some(value), None) => value == "true",
        _ => false,
    }
}

async fn account_by_user_id(store: &IslesStore, user_id: &str) -> RepoResult<Option<AccountRecord>> {
    let key = UserId::new(user_id);
    store
        .read(move |conn, repos| repos.accounts.find_by_user_id(conn, &key))
        .await
}

async fn friends_list(mut call: Call) -> Response {
    let (data, store) = match prelude(&mut call).await {
        Ok(found) => found,
        Err(answer) => return answer,
    };
    let pending = include_pending(call.request.uri().query());
    let Some(caller) = data.user_id else {
        return Json(json!([])).into_response();
    };
    let listed = async {
        let Some(account) = account_by_user_id(&store, &caller).await? else {
            return Ok(Vec::new());
        };
        let owner = account.id;
        store
            .read(move |conn, repos| repos.friendships.list_for_owner(conn, &owner))
            .await
    };
    let edges = match listed.await {
        Ok(edges) => edges,
        Err(e) => return store_failure(call.route, &e),
    };
    let shown: Vec<Value> = edges
        .iter()
        .filter(|e| e.status == FriendshipStatus::Accepted || (pending && e.status == FriendshipStatus::Pending))
        .map(|e| {
            json!({
                "accountId": e.other_user_id.as_str(),
                "status": e.status.as_str(),
                "direction": e.direction.unwrap_or(FriendshipDirection::Outbound).as_str(),
                "created": e.created,
                "favorite": e.favorite,
            })
        })
        .collect();
    tracing::info!("Friends list for {caller}: {} (includePending={pending})", shown.len());
    Json(Value::Array(shown)).into_response()
}

/// How one side of a friendship is written: its state and the time it carries.
struct Side {
    status: FriendshipStatus,
    direction: Option<FriendshipDirection>,
    since: String,
}

/// `from`'s row pointing at `to`; every invite and accept resets `favorite` (spec §13 item 9).
fn friend_row((from, to): (&AccountRecord, &AccountRecord), side: Side) -> FriendEdgeRecord {
    FriendEdgeRecord {
        owner: from.id.clone(),
        other: to.id.clone(),
        other_user_id: to.user_id.clone(),
        status: side.status,
        direction: side.direction,
        created: side.since,
        favorite: false,
    }
}

/// Each edge is its own write on its owner's home, actor the caller (DD17: the friend may live elsewhere).
async fn write_edge(store: &IslesStore, actor: &ActorContext, record: FriendEdgeRecord) -> RepoResult<()> {
    let owner = record.owner.clone();
    store
        .account_write(&owner, actor, move |tx, repos| repos.friendships.upsert(tx, &record))
        .await
}

fn player_actor(account: &AccountRecord) -> ActorContext {
    ActorContext::new(Actor::Player {
        account: account.id.global().clone(),
    })
}

/// §6.3: no edge for a stranger (DD18); an invite to someone who invited first accepts.
async fn invite(store: &IslesStore, caller: &str, friend: &str, clock: &Clock) -> RepoResult<()> {
    let (Some(ours), Some(theirs)) = (
        account_by_user_id(store, caller).await?,
        account_by_user_id(store, friend).await?,
    ) else {
        return Ok(());
    };
    let now = clock();
    let (owner, other) = (ours.id.clone(), theirs.id.clone());
    let existing = store
        .read(move |conn, repos| repos.friendships.find(conn, &owner, &other))
        .await?;
    let actor = player_actor(&ours);
    match existing {
        Some(e) if e.status == FriendshipStatus::Accepted => {}
        Some(e) if e.status == FriendshipStatus::Pending && e.direction == Some(FriendshipDirection::Inbound) => {
            // The caller keeps the invite's time, the friend gets now
            let accepted = |since| Side {
                status: FriendshipStatus::Accepted,
                direction: None,
                since,
            };
            write_edge(store, &actor, friend_row((&ours, &theirs), accepted(e.created))).await?;
            write_edge(store, &actor, friend_row((&theirs, &ours), accepted(now))).await?;
            tracing::info!("{caller} accepted friend invite from {friend}");
        }
        _ => {
            let pending = |direction, since| Side {
                status: FriendshipStatus::Pending,
                direction: Some(direction),
                since,
            };
            let outbound = pending(FriendshipDirection::Outbound, now.clone());
            write_edge(store, &actor, friend_row((&ours, &theirs), outbound)).await?;
            let inbound = pending(FriendshipDirection::Inbound, now);
            write_edge(store, &actor, friend_row((&theirs, &ours), inbound)).await?;
            tracing::info!("{caller} sent friend invite to {friend}");
        }
    }
    Ok(())
}

async fn invite_route(mut call: Call, clock: Clock) -> Response {
    let (data, store) = match prelude(&mut call).await {
        Ok(found) => found,
        Err(answer) => return answer,
    };
    let friend = call.params.get("friendId").unwrap_or("").to_owned();
    if friend.is_empty() || data.user_id.as_deref() == Some(friend.as_str()) {
        return empty(StatusCode::BAD_REQUEST);
    }
    let Some(caller) = data.user_id else {
        return empty(StatusCode::NO_CONTENT);
    };
    match invite(&store, &caller, &friend, &clock).await {
        Ok(()) => empty(StatusCode::NO_CONTENT),
        Err(e) => store_failure(call.route, &e),
    }
}

/// The caller's edges to that userId go even when the friend's account is gone.
async fn remove(store: &IslesStore, caller: &str, friend: &str) -> RepoResult<()> {
    let Some(ours) = account_by_user_id(store, caller).await? else {
        return Ok(());
    };
    let actor = player_actor(&ours);
    let (owner, target) = (ours.id.clone(), friend.to_owned());
    store
        .account_write(&ours.id, &actor, move |tx, repos| {
            let edges = repos.friendships.list_for_owner(tx.conn(), &owner)?;
            for e in edges.iter().filter(|e| e.other_user_id.as_str() == target) {
                repos.friendships.remove(tx, &owner, &e.other)?;
            }
            Ok(())
        })
        .await?;
    if let Some(theirs) = account_by_user_id(store, friend).await? {
        let (owner, other) = (theirs.id.clone(), ours.id.clone());
        store
            .account_write(&theirs.id, &actor, move |tx, repos| {
                repos.friendships.remove(tx, &owner, &other)
            })
            .await?;
    }
    tracing::info!("{caller} removed/rejected friend {friend}");
    Ok(())
}

async fn remove_route(mut call: Call) -> Response {
    let (data, store) = match prelude(&mut call).await {
        Ok(found) => found,
        Err(answer) => return answer,
    };
    // No self check, unlike POST
    let Some(caller) = data.user_id else {
        return empty(StatusCode::NO_CONTENT);
    };
    let friend = call.params.get("friendId").unwrap_or("").to_owned();
    match remove(&store, &caller, &friend).await {
        Ok(()) => empty(StatusCode::NO_CONTENT),
        Err(e) => store_failure(call.route, &e),
    }
}

/// `type-is`'s `hasBody`: a transfer encoding, a numeric length, or no `host`.
fn has_body(request: &Request) -> bool {
    let headers = request.headers();
    let length = headers
        .get(header::CONTENT_LENGTH)
        .is_some_and(|v| String::from_utf8_lossy(v.as_bytes()).trim().parse::<u64>().is_ok());
    headers.contains_key(header::TRANSFER_ENCODING) || length || !headers.contains_key(header::HOST)
}

/// Restricted's own reader (coordinator decision, item 3): Express's JSON and form rules, but bare 400 and 413, no
/// charset or encoding check, and a BOM kept (so it is malformed JSON).
async fn read_body(request: &mut Request) -> Result<Option<Value>, StatusCode> {
    if !has_body(request) {
        return Ok(None);
    }
    let essence = request
        .headers()
        .get(header::CONTENT_TYPE)
        .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
        .unwrap_or_default();
    let essence = essence.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    let form = match essence.as_str() {
        "application/json" => false,
        "application/x-www-form-urlencoded" => true,
        _ => return Ok(None),
    };
    let body = std::mem::replace(request.body_mut(), Body::empty());
    let bytes = match http_body_util::Limited::new(body, BODY_LIMIT).collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(e) if e.downcast_ref::<http_body_util::LengthLimitError>().is_some() => {
            return Err(StatusCode::PAYLOAD_TOO_LARGE);
        }
        Err(_) => return Err(StatusCode::BAD_REQUEST),
    };
    let text = std::str::from_utf8(&bytes).map_err(|_| StatusCode::BAD_REQUEST)?;
    if form {
        return dainty_service_kit::body::parse_form(text)
            .map(Some)
            .map_err(|e| e.status());
    }
    if text.is_empty() {
        return Ok(Some(json!({})));
    }
    if !text.trim_start_matches([' ', '\t', '\n', '\r']).starts_with(['{', '[']) {
        return Err(StatusCode::BAD_REQUEST);
    }
    dainty_service_kit::jsjson::parse(text)
        .map(Some)
        .map_err(|_| StatusCode::BAD_REQUEST)
}

/// `{"availability":[..]}`: the body's `account_ids` strings of 1 to 128 UTF-16 units, the first 256, none available.
pub fn slayer_link_availability(body: Option<&Value>) -> Value {
    let ids = body
        .and_then(Value::as_object)
        .and_then(|o| o.get("account_ids"))
        .and_then(Value::as_array);
    let availability: Vec<Value> = ids
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|id| (1..=SLAYER_LINK_MAX_ID).contains(&id.encode_utf16().take(SLAYER_LINK_MAX_ID + 1).count()))
        .take(SLAYER_LINK_MAX_ACCOUNTS)
        .map(|id| json!({"account_id": id, "available": false}))
        .collect();
    json!({"availability": availability})
}

/// The Slayer link status body: no invites, no links (1.14.7 has no Slayer link).
pub fn slayer_link_status() -> Value {
    json!({
        "config": {"invite_expiry_hours": 48, "link_duration_hours": 168},
        "invites": [],
        "links": [],
    })
}

/// The body first, as Express's `express.json()` ran before every router: a body error is answered before auth.
async fn availability_route(mut call: Call) -> Response {
    let body = match read_body(&mut call.request).await {
        Ok(body) => body,
        Err(status) => return empty(status),
    };
    match game_auth(&mut call).await {
        Ok(_) => envelope(slayer_link_availability(body.as_ref())),
        Err(refused) => refused,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn include_pending_as_express_reads_it() {
        for yes in [
            "includePending=true",
            "a=1&includePending=tr%75e",
            "&&includePending=true&",
        ] {
            assert!(include_pending(Some(yes)), "{yes}");
        }
        for no in [
            "",
            "includePending=TRUE",
            "includePending",
            "includePending=",
            "includePending[]=true",
            "includePending=true&includePending=true",
            "includePending=true+",
        ] {
            assert!(!include_pending(Some(no)), "{no}");
        }
        assert!(!include_pending(None));
        assert!(include_pending(Some("include%50ending=true")));
    }

    #[test]
    fn availability_caps() {
        let ids: Vec<String> = (0..300).map(|i| format!("ID{i}")).collect();
        let answer = slayer_link_availability(Some(&json!({ "account_ids": ids })));
        let list = answer["availability"].as_array().unwrap();
        assert_eq!(list.len(), 256);
        assert_eq!(list[255], json!({"account_id": "ID255", "available": false}));
        let emoji = "\u{1F600}";
        let body = json!({"account_ids": [emoji.repeat(64), emoji.repeat(65), "x".repeat(129), "y".repeat(128)]});
        let answer = slayer_link_availability(Some(&body));
        let listed: Vec<&str> = answer["availability"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["account_id"].as_str().unwrap())
            .collect();
        assert_eq!(listed, [emoji.repeat(64), "y".repeat(128)]);
        for nothing in [json!([]), json!({"account_ids": "AB"}), json!({})] {
            assert_eq!(slayer_link_availability(Some(&nothing)), json!({"availability": []}));
        }
        assert_eq!(slayer_link_availability(None), json!({"availability": []}));
    }
}
