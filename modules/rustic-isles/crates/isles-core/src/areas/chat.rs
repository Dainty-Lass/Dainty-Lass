//! RU5's `Chat` group: friends, Slayer link, guild and the voice stub (`routes/friends.ts`, `routes/guild.ts`,
//! `/vivox/login` of `routes/system.ts`). The XMPP side is DaintyLass's chat plugin (DD21); its rules are
//! [`isles_chat`]'s.
//!
//! Friend edges are per account (DD18): the owner's edge lives with the owner, and an invite writes the friend's
//! edge as an account write of its own, because the friend may be homed elsewhere (DD17). The client sees userIds.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::to_bytes;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use module_api::{Actor, ActorContext};
use serde_json::{Value, json};
use service_kit::json::Json;

use crate::db::IslesStore;
use crate::http::{Call, Handlers, not_implemented};
use crate::ids::{AccountId, UserId};
use crate::repo::auth::AccountRecord;
use crate::repo::chat::{FriendEdgeRecord, FriendshipDirection, FriendshipStatus};
use crate::repo::{RepoError, RepoResult};
use crate::routes::{Method, Service};

pub mod rules;

/// What `HasParadoxBackendAuth` leaves on `req.AuthData` that these routes read. The auth area's middleware puts it in
/// the request's extensions; a request without one is unauthenticated (401), which is how these routes fail closed
/// until it is wired.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AuthData {
    /// `AuthData.userId`: absent for a game server call that names no player
    pub user_id: Option<String>,
    pub is_gameserver: bool,
}

/// `new Date().toISOString()`, injectable so the golden tests can fix the clock.
pub type Clock = Arc<dyn Fn() -> String + Send + Sync>;

/// `Date#toISOString` of now: `2026-10-20T12:00:00.000Z`.
pub fn iso_now() -> String {
    let elapsed = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    iso_from_millis(i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX))
}

/// Milliseconds since the epoch as `Date#toISOString` writes them (years 0000 to 9999).
pub fn iso_from_millis(millis: i64) -> String {
    let (days, in_day) = (millis.div_euclid(86_400_000), millis.rem_euclid(86_400_000));
    // Howard Hinnant's civil_from_days
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        in_day / 3_600_000,
        in_day / 60_000 % 60,
        in_day / 1000 % 60,
        in_day % 1000
    )
}

impl From<&crate::areas::auth::AuthData> for AuthData {
    fn from(auth: &crate::areas::auth::AuthData) -> Self {
        Self {
            user_id: auth.user_id.clone(),
            is_gameserver: auth.is_gameserver,
        }
    }
}

fn auth(call: &Call) -> Option<AuthData> {
    call.request
        .extensions()
        .get::<crate::areas::auth::AuthData>()
        .map(AuthData::from)
        .or_else(|| call.request.extensions().get::<AuthData>().cloned())
}

/// `res.status(401); res.send()`
fn unauthorized() -> Response {
    StatusCode::UNAUTHORIZED.into_response()
}

fn no_content() -> Response {
    StatusCode::NO_CONTENT.into_response()
}

fn json_reply(status: StatusCode, body: &Value) -> Response {
    (status, Json(body.clone())).into_response()
}

/// `{ code: null, message: "OK", payload }`
fn envelope(payload: Value) -> Value {
    json!({ "code": null, "message": "OK", "payload": payload })
}

/// A database error is Node's unhandled rejection: Express answers 500. A write that reaches a node that isn't the
/// account's home is DN5's to forward; until then the player is told to retry.
fn failed(call: &Call, error: &RepoError) -> Response {
    match error {
        RepoError::NotHome { .. } | RepoError::Home(_) => {
            tracing::warn!("{} {}: {error}", call.route.method.as_str(), call.route.path);
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
        _ => {
            tracing::error!("{} {}: {error}", call.route.method.as_str(), call.route.path);
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Mounts this area's handlers.
pub fn register(handlers: &mut Handlers) {
    register_with(handlers, Arc::new(iso_now));
}

pub fn register_with(handlers: &mut Handlers, clock: Clock) {
    let backend = Service::Backend;
    handlers.on(backend, Method::Get, "/vivox/login", |call| {
        Box::pin(async move {
            // systemRouter: HasParadoxBackendAuth, then 404 with no body
            if auth(&call).is_none() {
                return unauthorized();
            }
            StatusCode::NOT_FOUND.into_response()
        })
    });
    handlers.on(backend, Method::Get, "/guild/invite/player", |call| {
        Box::pin(async move {
            if auth(&call).is_none() {
                return unauthorized();
            }
            json_reply(StatusCode::OK, &envelope(json!({ "invites": [] })))
        })
    });
    handlers.on(backend, Method::Get, "/guild", |call| {
        Box::pin(async move {
            if auth(&call).is_none() {
                return unauthorized();
            }
            no_content()
        })
    });

    handlers.on(backend, Method::Get, "/friends/api/public/friends/:accountId", |call| {
        Box::pin(async move {
            let Some(auth) = auth(&call) else { return unauthorized() };
            let Some(store) = call.shared.store.clone() else {
                return not_implemented(call.route, &call.request_id);
            };
            let include_pending = include_pending(call.request.uri().query());
            let Some(owner) = auth.user_id else {
                return json_reply(StatusCode::OK, &json!([]));
            };
            match friends_list(&store, &owner, include_pending).await {
                Ok(list) => json_reply(StatusCode::OK, &list),
                Err(error) => failed(&call, &error),
            }
        })
    });
    let invite_clock = clock;
    handlers.on(
        backend,
        Method::Post,
        "/friends/api/public/friends/:accountId/:friendId",
        move |call| {
            let clock = invite_clock.clone();
            Box::pin(async move {
                let Some(auth) = auth(&call) else { return unauthorized() };
                let Some(store) = call.shared.store.clone() else {
                    return not_implemented(call.route, &call.request_id);
                };
                let friend = call.params.get("friendId").unwrap_or_default().to_owned();
                if friend.is_empty() || auth.user_id.as_deref() == Some(friend.as_str()) {
                    return StatusCode::BAD_REQUEST.into_response();
                }
                let Some(owner) = auth.user_id else { return no_content() };
                match friend_invite(&store, &owner, &friend, &*clock).await {
                    Ok(()) => no_content(),
                    Err(error) => failed(&call, &error),
                }
            })
        },
    );
    handlers.on(
        backend,
        Method::Delete,
        "/friends/api/public/friends/:accountId/:friendId",
        |call| {
            Box::pin(async move {
                let Some(auth) = auth(&call) else { return unauthorized() };
                let Some(store) = call.shared.store.clone() else {
                    return not_implemented(call.route, &call.request_id);
                };
                let friend = call.params.get("friendId").unwrap_or_default().to_owned();
                let Some(owner) = auth.user_id else { return no_content() };
                match friend_remove(&store, &owner, &friend).await {
                    Ok(()) => no_content(),
                    Err(error) => failed(&call, &error),
                }
            })
        },
    );
    // Not behind HasParadoxBackendAuth in friends.ts: benign stubs
    handlers.on(backend, Method::Get, "/friends/api/public/blocklist/:accountId", |_| {
        Box::pin(async move { json_reply(StatusCode::OK, &json!([])) })
    });
    handlers.on(
        backend,
        Method::Get,
        "/friends/api/public/list/:namespace/:accountId/recentPlayers",
        |_| Box::pin(async move { json_reply(StatusCode::OK, &json!([])) }),
    );
    handlers.on(backend, Method::Get, "/friends/api/v1/:accountId/settings", |_| {
        Box::pin(async move {
            json_reply(
                StatusCode::OK,
                &json!({ "acceptInvites": "public", "mutualPrivacy": "ALL" }),
            )
        })
    });

    handlers.on(backend, Method::Post, "/slayerlink/availability", |call| {
        Box::pin(async move {
            // express.json() runs before every router, so a bad body is refused before auth is asked
            let mut call = call;
            let body = match read_json_body(&mut call).await {
                Ok(body) => body,
                Err(status) => return status.into_response(),
            };
            if auth(&call).is_none() {
                return unauthorized();
            }
            json_reply(StatusCode::OK, &envelope(slayer_link_availability(body.as_ref())))
        })
    });
    handlers.on(backend, Method::Get, "/slayerlink/status_good", |call| {
        Box::pin(async move {
            if auth(&call).is_none() {
                return unauthorized();
            }
            json_reply(StatusCode::OK, &envelope(slayer_link_status()))
        })
    });
}

/// Express 5's default (`simple`) query parser: only the key spelled `includePending`, once, as the text `true`. A
/// repeated key is an array whose `String()` is `true,true`, and `includePending[]` is a different key.
pub fn include_pending(query: Option<&str>) -> bool {
    let decode = |text: &str| {
        let text = text.replace('+', " ");
        percent_encoding::percent_decode_str(&text)
            .decode_utf8_lossy()
            .into_owned()
    };
    let mut values = query
        .unwrap_or_default()
        .split('&')
        .filter(|pair| !pair.is_empty())
        .filter_map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (decode(key) == "includePending").then(|| decode(value))
        });
    matches!((values.next(), values.next()), (Some(only), None) if only == "true")
}

/// `express.json()` (default 100 kB limit, strict): the parsed object or array of a body sent as `application/json`.
/// Any other content type leaves the body unread, as `req.body === undefined`.
async fn read_json_body(call: &mut Call) -> Result<Option<Value>, StatusCode> {
    let is_json = call
        .request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(';')
                .next()
                .is_some_and(|media| media.trim().eq_ignore_ascii_case("application/json"))
        });
    if !is_json {
        return Ok(None);
    }
    let body = std::mem::take(call.request.body_mut());
    let bytes = to_bytes(body, 100 * 1024)
        .await
        .map_err(|_| StatusCode::PAYLOAD_TOO_LARGE)?;
    let text = String::from_utf8_lossy(&bytes);
    let trimmed = text.trim_start_matches([' ', '\t', '\n', '\r']);
    if trimmed.is_empty() {
        return Ok(Some(json!({})));
    }
    // strict: only an object or an array
    if !trimmed.starts_with(['{', '[']) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let parsed = serde_json::from_str::<Value>(&text).map_err(|_| StatusCode::BAD_REQUEST)?;
    Ok(Some(parsed))
}

/// `SLAYER_LINK_MAX_ACCOUNTS`
pub const SLAYER_LINK_MAX_ACCOUNTS: usize = 256;
const SLAYER_LINK_MAX_ID_LENGTH: usize = 128;

/// `SlayerLinkAvailabilityPayload`: every well-formed requested account, not available (no link can be made). The
/// length is JavaScript's, in UTF-16 code units.
pub fn slayer_link_availability(body: Option<&Value>) -> Value {
    let requested = body
        .and_then(|body| body.get("account_ids"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let availability: Vec<Value> = requested
        .iter()
        .filter_map(Value::as_str)
        .filter(|id| {
            let length = id.encode_utf16().count();
            length > 0 && length <= SLAYER_LINK_MAX_ID_LENGTH
        })
        .take(SLAYER_LINK_MAX_ACCOUNTS)
        .map(|id| json!({ "account_id": id, "available": false }))
        .collect();
    json!({ "availability": availability })
}

/// `SlayerLinkStatusPayload`: the captured config, no invites and no links.
pub fn slayer_link_status() -> Value {
    json!({ "config": { "invite_expiry_hours": 48, "link_duration_hours": 168 }, "invites": [], "links": [] })
}

async fn account_of(store: &IslesStore, user: &str) -> RepoResult<Option<AccountRecord>> {
    let user = UserId(user.to_owned());
    store
        .read(move |conn, repos| repos.accounts.find_by_user_id(conn, &user))
        .await
}

fn acting(account: &AccountId) -> ActorContext {
    ActorContext::new(Actor::Player {
        account: account.global().clone(),
    })
}

async fn put_edge(store: &IslesStore, actor: &ActorContext, edge: FriendEdgeRecord) -> RepoResult<()> {
    let owner = edge.owner.clone();
    store
        .account_write(&owner, actor, move |tx, repos| repos.friendships.upsert(tx, &edge))
        .await
}

fn edge(
    owner: &AccountRecord,
    other: &AccountRecord,
    status: FriendshipStatus,
    direction: Option<FriendshipDirection>,
    created: &str,
) -> FriendEdgeRecord {
    FriendEdgeRecord {
        owner: owner.id.clone(),
        other: other.id.clone(),
        other_user_id: other.user_id.clone(),
        status,
        direction,
        created: created.to_owned(),
        favorite: false,
    }
}

/// `GET /friends/api/public/friends/:accountId`: the owner's accepted edges, and with `includePending` the pending
/// ones too, in Epic's shape (`ToEpicFriend`). An owner with no account has no edges.
pub async fn friends_list(store: &IslesStore, owner: &str, include_pending: bool) -> RepoResult<Value> {
    let Some(account) = account_of(store, owner).await? else {
        return Ok(json!([]));
    };
    let id = account.id;
    let edges = store
        .read(move |conn, repos| repos.friendships.list_for_owner(conn, &id))
        .await?;
    let friends: Vec<Value> = edges
        .iter()
        .filter(|e| {
            e.status == FriendshipStatus::Accepted || (include_pending && e.status == FriendshipStatus::Pending)
        })
        .map(|e| {
            json!({
                "accountId": e.other_user_id.as_str(),
                "status": if e.status == FriendshipStatus::Accepted { "ACCEPTED" } else { "PENDING" },
                "direction": e.direction.unwrap_or(FriendshipDirection::Outbound).as_str(),
                "created": e.created,
                "favorite": e.favorite,
            })
        })
        .collect();
    tracing::info!(
        "Friends list for {owner}: {} (includePending={include_pending})",
        friends.len()
    );
    Ok(Value::Array(friends))
}

/// `POST .../friends/:accountId/:friendId`: sends an invite, or accepts the one `friend` already sent. A friend who
/// has no account takes no edge (Node kept an edge for any string; here an edge names an account, DD18).
pub async fn friend_invite(
    store: &IslesStore,
    owner: &str,
    friend: &str,
    clock: &(dyn Fn() -> String + Send + Sync),
) -> RepoResult<()> {
    let (Some(owner_account), Some(friend_account)) =
        (account_of(store, owner).await?, account_of(store, friend).await?)
    else {
        return Ok(());
    };
    let (a, b) = (owner_account.id.clone(), friend_account.id.clone());
    let existing = store
        .read(move |conn, repos| repos.friendships.find(conn, &a, &b))
        .await?;
    let now = clock();
    let actor = acting(&owner_account.id);
    use FriendshipDirection::{Inbound, Outbound};
    use FriendshipStatus::{Accepted, Pending};
    match existing {
        Some(e) if e.status == Accepted => {}
        Some(e) if e.status == Pending && e.direction == Some(Inbound) => {
            // They already invited us: both edges become ACCEPTED
            put_edge(
                store,
                &actor,
                edge(&owner_account, &friend_account, Accepted, None, &e.created),
            )
            .await?;
            put_edge(
                store,
                &actor,
                edge(&friend_account, &owner_account, Accepted, None, &now),
            )
            .await?;
            tracing::info!("{owner} accepted friend invite from {friend}");
        }
        _ => {
            // Outbound on us, inbound on them
            put_edge(
                store,
                &actor,
                edge(&owner_account, &friend_account, Pending, Some(Outbound), &now),
            )
            .await?;
            put_edge(
                store,
                &actor,
                edge(&friend_account, &owner_account, Pending, Some(Inbound), &now),
            )
            .await?;
            tracing::info!("{owner} sent friend invite to {friend}");
        }
    }
    Ok(())
}

/// `DELETE .../friends/:accountId/:friendId`: rejects an invite or removes a friend, dropping both edges. The owner's
/// edge is found by the friend's userId, so it goes even when the friend's account is gone.
pub async fn friend_remove(store: &IslesStore, owner: &str, friend: &str) -> RepoResult<()> {
    let Some(owner_account) = account_of(store, owner).await? else {
        return Ok(());
    };
    let id = owner_account.id.clone();
    let edges = store
        .read(move |conn, repos| repos.friendships.list_for_owner(conn, &id))
        .await?;
    let actor = acting(&owner_account.id);
    for e in edges.iter().filter(|e| e.other_user_id.as_str() == friend) {
        let (owner_id, other_id) = (e.owner.clone(), e.other.clone());
        store
            .account_write(&owner_id.clone(), &actor, move |tx, repos| {
                repos.friendships.remove(tx, &owner_id, &other_id)
            })
            .await?;
    }
    if let Some(friend_account) = account_of(store, friend).await? {
        let (friend_id, owner_id) = (friend_account.id.clone(), owner_account.id.clone());
        store
            .account_write(&friend_id.clone(), &actor, move |tx, repos| {
                repos.friendships.remove(tx, &friend_id, &owner_id)
            })
            .await?;
    }
    tracing::info!("{owner} removed/rejected friend {friend}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps_are_to_iso_string() {
        assert_eq!(iso_from_millis(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso_from_millis(1_792_497_601_000), "2026-10-20T12:00:01.000Z");
        // 2024-02-29T23:59:59.999Z, a leap day
        assert_eq!(iso_from_millis(1_709_251_199_999), "2024-02-29T23:59:59.999Z");
        assert_eq!(iso_from_millis(-1), "1969-12-31T23:59:59.999Z");
        assert!(iso_now().ends_with('Z') && iso_now().len() == 24);
    }

    #[test]
    fn include_pending_reads_like_express() {
        assert!(include_pending(Some("includePending=true")));
        assert!(include_pending(Some("x=1&includePending=tr%75e")));
        assert!(!include_pending(None));
        assert!(!include_pending(Some("includePending=TRUE")));
        assert!(!include_pending(Some("includePending=true&includePending=true")));
        assert!(!include_pending(Some("includePending[]=true")));
        assert!(!include_pending(Some("includePending")));
    }

    #[test]
    fn availability_follows_the_node_payload_builder() {
        // friendsSlayerLink.test.ts
        let body = json!({ "account_ids": ["XQBF5VHGFJHR5AUILEPYI55MUQ", "", 7, "PLNTNCL7PNFUHBRPEBVSTAZ47A"] });
        assert_eq!(
            slayer_link_availability(Some(&body)),
            json!({ "availability": [
                { "account_id": "XQBF5VHGFJHR5AUILEPYI55MUQ", "available": false },
                { "account_id": "PLNTNCL7PNFUHBRPEBVSTAZ47A", "available": false },
            ] })
        );
        let none = json!({ "availability": [] });
        assert_eq!(
            slayer_link_availability(Some(&json!({ "account_ids": "not a list" }))),
            none
        );
        assert_eq!(slayer_link_availability(None), none);
        let long = json!({ "account_ids": ["x".repeat(129)] });
        assert_eq!(
            slayer_link_availability(Some(&long)),
            none,
            "an id over 128 characters is dropped"
        );
        let many: Vec<String> = (0..SLAYER_LINK_MAX_ACCOUNTS + 40)
            .map(|i| format!("ACCOUNT{i}"))
            .collect();
        let many = json!({ "account_ids": many });
        let answered = slayer_link_availability(Some(&many));
        assert_eq!(
            answered["availability"].as_array().map(Vec::len),
            Some(SLAYER_LINK_MAX_ACCOUNTS)
        );
        // JavaScript counts UTF-16 units: sixty-four emoji fit, sixty-five do not
        let emoji = json!({ "account_ids": ["😀".repeat(64), "😀".repeat(65)] });
        assert_eq!(
            slayer_link_availability(Some(&emoji))["availability"]
                .as_array()
                .map(Vec::len),
            Some(1)
        );
    }

    #[test]
    fn the_status_payload_is_the_captured_shape() {
        assert_eq!(
            slayer_link_status(),
            json!({ "config": { "invite_expiry_hours": 48, "link_duration_hours": 168 }, "invites": [], "links": [] })
        );
    }
}
