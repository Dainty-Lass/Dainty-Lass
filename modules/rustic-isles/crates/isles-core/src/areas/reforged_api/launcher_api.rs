//! `routes/reforgedApi.ts`: the launcher API (invites, one account key per player, the live server status) and the
//! host kit's routes (`/reforged/api/*`, `HostOnly`). Node-scope: both modules serve it, with their own changelist.
//!
//! The key is the game's login: with `AUTH_MODE=APIKEY` the client trades it for a session token at
//! `/account/api/oauth/token`. Only its HMAC is stored (`security/apiKeyHash.ts`).

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant};

use axum::response::Response;
use module_api::{Actor, ActorContext};
use serde_json::{Value, json};

use super::presence;
use super::rules::{
    self, GAME_DATA_FILES, Refusal, RegistrationMode, USER_KEY_HEADER, classify_game_data, is_invite_code,
    looks_like_account_key, make_account_key, normalize_username, status_instances,
};
use super::util::{self, BodyKind, Req, hex, hmac_sha256};
use crate::db::IslesStore;
use crate::http::{Call, Handlers, Shared, not_implemented};
use crate::ids::{AccountId, UserId};
use crate::repo::auth::{AccountRecord, ApprovalStatus, LauncherAccountRecord, OperationalStatus};
use crate::repo::{RepoError, RepoResult};
use crate::routes::{Method, Service};

const STATUS_CACHE_MS: u128 = 5_000;
const GAME_DATA_CACHE_MS: u128 = 60_000;
const DIRECTOR_TIMEOUT: Duration = Duration::from_millis(2_000);
const MAX_PLAYERS_LISTED: usize = 500;
/// Express's default JSON limit
pub(super) const BODY_LIMIT: usize = 100 * 1024;

static BOOTED_AT: LazyLock<Instant> = LazyLock::new(Instant::now);

pub(super) fn actor_host() -> ActorContext {
    ActorContext::new(Actor::Console)
}

fn mode(shared: &Shared) -> RegistrationMode {
    RegistrationMode::from_setting(shared.settings.raw("REGISTRATION_MODE"))
}

pub(super) fn js_slice(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

// ---------------------------------------------------------------- who holds a key

/// `HashApiKey(value, "user")`: HMAC-SHA256 under `API_KEY_HASH_SECRET`, domain-separated by scope. `None` where
/// Node throws (a secret under 32 characters).
pub fn hash_api_key(shared: &Shared, value: &str, scope: &str) -> Option<String> {
    let secret = shared.settings.raw("API_KEY_HASH_SECRET").map(str::trim)?;
    if secret.encode_utf16().count() < 32 {
        return None;
    }
    Some(hex(&hmac_sha256(
        secret.as_bytes(),
        &[scope.as_bytes(), b"\0", value.as_bytes()],
    )))
}

#[derive(Debug, Clone)]
struct Holder {
    user_id: String,
    username: String,
    is_admin: bool,
    account: Option<LauncherAccountRecord>,
}

/// `IsAccountEligible` (`security/accountEligibility.ts`).
pub fn is_account_eligible(account: &LauncherAccountRecord) -> bool {
    account.status == OperationalStatus::Active
        && account.approval_status.unwrap_or(ApprovalStatus::Approved) == ApprovalStatus::Approved
        && account.username_set != Some(false)
}

/// `FindApiKeyRecord`: every stored hash is compared, in constant time, and the first match wins.
fn find_key_owner(rows: &[crate::repo::auth::UserKeyHash], incoming_hex: &str) -> Option<AccountId> {
    let incoming = hex_bytes(incoming_hex);
    let mut found = None;
    for row in rows {
        let stored = hex_bytes(&row.key_hash);
        if util::constant_time_eq(&incoming, &stored) && found.is_none() {
            found = Some(row.account.clone());
        }
    }
    found
}

fn hex_bytes(text: &str) -> Vec<u8> {
    // Buffer.from(hex, "hex") stops at the first pair that isn't hex
    text.as_bytes()
        .chunks(2)
        .map_while(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect()
}

async fn holder_of(shared: &Shared, store: &IslesStore, key: Option<&str>) -> Option<Holder> {
    let key = key.filter(|key| looks_like_account_key(key))?;
    let hashed = hash_api_key(shared, key, "user")?;
    store
        .read(move |conn, repos| {
            let rows = repos.api_keys.find_all_user_key_hashes(conn)?;
            let Some(account) = find_key_owner(&rows, &hashed) else {
                return Ok(None);
            };
            let launcher = repos.launcher_accounts.find_by_account(conn, &account)?;
            let game = repos.accounts.find(conn, &account)?;
            let user_id = game
                .as_ref()
                .map_or_else(|| account.to_string(), |game| game.user_id.to_string());
            let username = launcher
                .as_ref()
                .map(|l| l.display_name.clone())
                .or_else(|| game.map(|g| g.name))
                .unwrap_or_else(|| user_id.clone());
            Ok(Some(Holder {
                user_id,
                username,
                is_admin: launcher.as_ref().is_some_and(|l| l.roles.iter().any(|r| r == "admin")),
                account: launcher,
            }))
        })
        .await
        .ok()
        .flatten()
}

fn eligible(holder: &Holder) -> bool {
    holder.account.as_ref().is_none_or(is_account_eligible)
}

// ---------------------------------------------------------------- the status

fn server_name(shared: &Shared) -> String {
    let name: String = shared
        .settings
        .raw("REFORGED_SERVER_NAME")
        .unwrap_or_default()
        .chars()
        .map(|c| {
            if c <= '\u{1f}' || ('\u{7f}'..='\u{9f}').contains(&c) {
                ' '
            } else {
                c
            }
        })
        .collect();
    let name = service_kit::js::trim(&name);
    if name.is_empty() {
        "DaintyLass server".to_owned()
    } else {
        js_slice(name, 64)
    }
}

fn server_version(shared: &Shared) -> String {
    let version = js_slice(
        service_kit::js::trim(shared.settings.raw("REFORGED_SERVER_VERSION").unwrap_or_default()),
        48,
    );
    if version.is_empty() {
        "0.1.0".to_owned()
    } else {
        version
    }
}

/// `new URL(raw).toString()` for an `https:` URL without credentials, else `null`.
fn https_url_or_null(raw: Option<&str>) -> Value {
    let Some(raw) = raw.map(service_kit::js::trim).filter(|r| !r.is_empty()) else {
        return Value::Null;
    };
    let Some(rest) = raw.strip_prefix("https://").or_else(|| raw.strip_prefix("HTTPS://")) else {
        return Value::Null;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') || authority.contains(char::is_whitespace) {
        return Value::Null;
    }
    // URL.toString() lower-cases the scheme and host and adds the root path
    let tail = &rest[authority.len()..];
    let tail = if tail.is_empty() || tail.starts_with(['?', '#']) {
        format!("/{tail}")
    } else {
        tail.to_owned()
    };
    json!(format!("https://{}{tail}", authority.to_lowercase()))
}

static GAME_DATA_SEEN: Mutex<Option<(Instant, rules::GameDataKind)>> = Mutex::new(None);

fn game_data_kind(shared: &Shared) -> rules::GameDataKind {
    let mut guard = GAME_DATA_SEEN.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((at, kind)) = *guard
        && at.elapsed().as_millis() < GAME_DATA_CACHE_MS
    {
        return kind;
    }
    let kind = shared
        .game_data
        .backend
        .as_deref()
        .map_or(rules::GameDataKind::Missing, |dir| {
            classify_game_data(dir, &GAME_DATA_FILES)
        });
    *guard = Some((Instant::now(), kind));
    kind
}

/// `GET http://<DEPLOYSERVER_URL>/api/instances`: `[]` on any failure, within two seconds.
async fn director_instances(director: &str) -> Value {
    let director = director.trim();
    if director.is_empty() {
        return json!([]);
    }
    tokio::time::timeout(DIRECTOR_TIMEOUT, super::http_get::get_json(director, "/api/instances"))
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| json!([]))
}

async fn players_online(store: &IslesStore, hunt_of: &HashMap<String, String>) -> Vec<(String, Option<String>)> {
    let ids: Vec<String> = presence::online_user_ids()
        .into_iter()
        .take(MAX_PLAYERS_LISTED)
        .collect();
    let names = store
        .read(move |conn, repos| {
            let mut out = Vec::new();
            for id in ids {
                let name = (|| -> RepoResult<Option<String>> {
                    let Some(game) = repos.accounts.find_by_user_id(conn, &UserId(id.clone()))? else {
                        return Ok(None);
                    };
                    let launcher = repos.launcher_accounts.find_by_account(conn, &game.id)?;
                    Ok(Some(launcher.map_or(game.name, |l| l.display_name)))
                })()
                .unwrap_or(None);
                if let Some(name) = name.filter(|n| !n.is_empty()) {
                    out.push((id, name));
                }
            }
            Ok(out)
        })
        .await
        .unwrap_or_default();
    let mut players: Vec<(String, Option<String>)> = names
        .into_iter()
        .map(|(id, name)| (js_slice(&name, 32), hunt_of.get(&id).cloned()))
        .collect();
    players.sort_by(|a, b| service_kit::collate::locale_compare(&a.0, &b.0));
    players
}

type StatusCache = HashMap<(u32, bool), (Instant, Value)>;
static STATUS_SEEN: Mutex<Option<StatusCache>> = Mutex::new(None);

/// Forgets the cached status bodies (a registration or an access change makes them stale).
pub fn forget_status() {
    STATUS_SEEN.lock().unwrap_or_else(PoisonError::into_inner).take();
}

/// `ServerStatusBody` (crates/protocol status.rs, field for field). The limited view hides who is online.
pub async fn server_status_body(shared: &Shared, store: &IslesStore, full: bool) -> Value {
    let changelist = shared.version.tables.target_changelist;
    {
        let guard = STATUS_SEEN.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((at, body)) = guard.as_ref().and_then(|cache| cache.get(&(changelist, full)))
            && at.elapsed().as_millis() < STATUS_CACHE_MS
        {
            return body.clone();
        }
    }
    let executable_hash = shared
        .settings
        .raw("APPROVED_EXECUTABLE_SHA256")
        .unwrap_or_default()
        .to_lowercase();
    let hash_ok =
        executable_hash.len() == 64 && executable_hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    let mut body = json!({
        "name": server_name(shared),
        "online": true,
        "version": server_version(shared),
        "commit": js_slice(service_kit::js::trim(shared.settings.raw("REFORGED_SOURCE_COMMIT").unwrap_or_default()), 64),
        "sourceUrl": https_url_or_null(shared.settings.raw("REFORGED_SOURCE_URL")),
        "registration": mode(shared).as_str(),
        "uptimeSeconds": BOOTED_AT.elapsed().as_secs(),
        "contentPort": null,
        "game": {
            "build": format!("rel-{}-Archon", shared.version.tables.game_version),
            "changelist": changelist,
            "executableSha256": if hash_ok { json!(executable_hash) } else { Value::Null },
        },
        "gameData": game_data_kind(shared).as_str(),
        "chat": shared.settings.raw("REALTIME_XMPP_ENABLED") == Some("true"),
    });
    let Value::Object(map) = &mut body else {
        return body;
    };
    if full {
        let director = director_instances(shared.settings.raw("DEPLOYSERVER_URL").unwrap_or_default()).await;
        let (instances, hunt_of) = status_instances(&director);
        let players = players_online(store, &hunt_of).await;
        map.insert("playersOnline".into(), json!(players.len()));
        map.insert(
            "players".into(),
            Value::Array(
                players
                    .into_iter()
                    .map(|(name, hunt)| {
                        json!({ "name": name, "where": if hunt.is_some() { "hunt" } else { "unknown" }, "instance": hunt })
                    })
                    .collect(),
            ),
        );
        map.insert("instances".into(), Value::Array(instances));
        map.insert("limited".into(), json!(false));
    } else {
        map.insert("playersOnline".into(), json!(0));
        map.insert("players".into(), json!([]));
        map.insert("instances".into(), json!([]));
        map.insert("limited".into(), json!(true));
    }
    STATUS_SEEN
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get_or_insert_with(HashMap::new)
        .insert((changelist, full), (Instant::now(), body.clone()));
    body
}

// ---------------------------------------------------------------- handlers

macro_rules! store_or_501 {
    ($call:expr) => {
        match $call.shared.store.clone() {
            Some(store) => store,
            None => return not_implemented($call.route, &$call.request_id),
        }
    };
}

pub(super) fn refusal(status: u16, refusal: Refusal) -> Response {
    util::json(status, &refusal.body())
}

async fn register_account(call: Call) -> Response {
    let store = store_or_501!(call);
    let shared = call.shared.clone();
    let registration = mode(&shared);
    if registration == RegistrationMode::None {
        return refusal(400, Refusal::RegistrationClosed);
    }
    let req = match Req::read(call.request, BodyKind::Json(BODY_LIMIT)).await {
        Ok(req) => req,
        Err(response) => return response,
    };
    let fields = req.fields();
    let invite_field = fields.get("InviteCode");
    if !matches!(fields.get("Username"), Some(Value::String(_))) || invite_field.is_some_and(|v| !v.is_string()) {
        return refusal(400, Refusal::BadRequest);
    }
    let Some(username) = normalize_username(fields.get("Username")) else {
        return refusal(400, Refusal::UsernameInvalid);
    };
    let code = invite_field
        .and_then(Value::as_str)
        .map_or_else(String::new, |c| service_kit::js::trim(c).to_owned());
    if registration == RegistrationMode::InviteCode && !is_invite_code(&code) {
        return refusal(401, Refusal::InviteInvalid);
    }
    let Some(key_hash) = None.or_else(|| {
        let key = make_account_key();
        hash_api_key(&shared, &key, "user").map(|h| (key, h))
    }) else {
        tracing::error!("[launcher-api] registration failed: API_KEY_HASH_SECRET must contain at least 32 characters.");
        return util::empty(500);
    };
    let (key, key_hash) = key_hash;
    let account = AccountId::new(&shared.node.node);
    let user_id = UserId::random();
    let record = AccountRecord {
        id: account.clone(),
        user_id: user_id.clone(),
        name: username.clone(),
        notes: 0,
    };
    let launcher = LauncherAccountRecord {
        account: account.clone(),
        email: None,
        display_name_normalized: username.to_lowercase(),
        display_name: username.clone(),
        password_hash: None,
        status: OperationalStatus::Active,
        approval_status: Some(ApprovalStatus::Approved),
        approval_updated_at: None,
        approval_updated_by: None,
        approval_reason: None,
        roles: vec!["player".to_owned()],
        roles_updated_at: None,
        roles_updated_by: None,
        created_at: util::now_iso(),
        last_login_at: None,
        username_set: Some(true),
        consent: None,
        deletion: None,
        email_verified_at: None,
        email_verification_pending_since: None,
    };
    let spend = (registration == RegistrationMode::InviteCode).then_some(code);
    let actor = ActorContext::new(Actor::Player {
        account: account.global().clone(),
    });
    // One transaction for the code, the account and the key: a taken name costs no invite use, and no account is
    // ever written without its key. Each refusal is decided before the first write.
    let normalized = launcher.display_name_normalized.clone();
    let outcome = store
        .write(&actor, move |tx, repos| {
            if repos
                .launcher_accounts
                .find_by_display_name_normalized(tx.conn(), &normalized)?
                .is_some()
            {
                return Ok(Err(Refusal::UsernameTaken));
            }
            if let Some(code) = &spend
                && !repos.invites.spend(tx, code)?
            {
                return Ok(Err(Refusal::InviteInvalid));
            }
            repos.accounts.create(tx, &record)?;
            repos.launcher_accounts.create(tx, &launcher)?;
            repos.api_keys.insert_user_key_hash(tx, &record.id, &key_hash)?;
            Ok(Ok(()))
        })
        .await;
    match outcome {
        Ok(Ok(())) => {}
        Ok(Err(why)) => return refusal(if why == Refusal::InviteInvalid { 401 } else { 409 }, why),
        Err(RepoError::Query(diesel::result::Error::DatabaseError(
            diesel::result::DatabaseErrorKind::UniqueViolation,
            _,
        ))) => return refusal(409, Refusal::UsernameTaken),
        Err(error) => {
            tracing::error!("[launcher-api] registration failed: {error}");
            return util::empty(500);
        }
    }
    tracing::info!("[launcher-api] {username} registered as {user_id}");
    forget_status();
    util::json(200, &json!({ "UUK": key }))
}

async fn get_user_info(call: Call) -> Response {
    let store = store_or_501!(call);
    let shared = call.shared.clone();
    let req = match Req::read(call.request, BodyKind::None).await {
        Ok(req) => req,
        Err(response) => return response,
    };
    let Some(holder) = holder_of(&shared, &store, req.header(USER_KEY_HEADER)).await else {
        return util::json(401, &json!({ "error": "unauthorized" }));
    };
    if !eligible(&holder) {
        let account = holder.account.as_ref();
        let banned = account.is_some_and(|a| a.status == OperationalStatus::Banned);
        let reason = js_slice(
            account.and_then(|a| a.approval_reason.as_deref()).unwrap_or_default(),
            500,
        );
        return util::json(
            403,
            &json!({
                "error": if banned { "account_banned" } else { "account_disabled" },
                "reason": if reason.is_empty() { "Ask the server's host." } else { reason.as_str() },
            }),
        );
    }
    util::json(
        200,
        &json!({ "UserId": holder.user_id, "Username": holder.username, "IsAdmin": holder.is_admin }),
    )
}

async fn server_status(call: Call) -> Response {
    let store = store_or_501!(call);
    let shared = call.shared.clone();
    let req = match Req::read(call.request, BodyKind::None).await {
        Ok(req) => req,
        Err(response) => return response,
    };
    let holder = holder_of(&shared, &store, req.header(USER_KEY_HEADER)).await;
    let full = holder.as_ref().is_some_and(eligible);
    let body = server_status_body(&shared, &store, full).await;
    let mut response = util::json(200, &body);
    util::cache_control(&mut response, "no-store");
    util::set_header(&mut response, "vary", USER_KEY_HEADER);
    response
}

fn registration_status(call: &Call) -> Response {
    util::json(200, &json!({ "mode": mode(&call.shared).as_str() }))
}

pub fn register(handlers: &mut Handlers) {
    LazyLock::force(&BOOTED_AT);
    let b = Service::Backend;
    handlers.on(b, Method::Post, "/reforged/api/Register", |call| {
        Box::pin(register_account(call))
    });
    handlers.on(b, Method::Get, "/reforged/api/GetUserInfo", |call| {
        Box::pin(get_user_info(call))
    });
    handlers.on(b, Method::Get, "/reforged/api/ServerStatus", |call| {
        Box::pin(server_status(call))
    });
    handlers.on(b, Method::Get, "/reforged/api/RegistrationStatus", |call| {
        Box::pin(async move { registration_status(&call) })
    });
}
