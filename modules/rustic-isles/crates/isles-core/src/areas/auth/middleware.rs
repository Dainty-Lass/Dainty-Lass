//! The auth middleware: `HasParadoxBackendAuth` (the game's bearer token, or a game server's API key),
//! `HasLauncherAuth` and `RequireActiveAccount`, as functions a handler calls first and returns the refusal of.
//! Every area's game routes call [`game_auth`]; the launcher's call [`launcher_auth`].
//!
//! Node's middleware ran before the handler and either answered or called `next()`; here the handler asks, gets an
//! [`AuthData`] or the response to send, and goes on. The refusals are Node's byte for byte: 401 and 403 with an
//! empty body for the game, the launcher's error envelope for the launcher.

use std::collections::BTreeSet;

use axum::extract::Request;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use regex::Regex;
use rifts_seam::{AuthOutcome, Caller, RequestInfo, RuntimeAuth};
use serde_json::{Map, Value};
use std::sync::LazyLock;

use super::apikeys::is_valid_gameserver_api_key;
use super::data::launcher_by_user;
use super::errors::LauncherApiError;
use super::policy::{
    assert_account_admitted, assert_account_not_deactivated, assert_consent_current_when_enforced, is_account_eligible,
};
use super::tokens::{validate_game_token, validate_launcher_access_token};
use super::util::{client_ip, peer_ip};
use crate::http::{Call, Shared};
use crate::repo::AccountId;
use crate::repo::auth::LauncherAccountRecord;

pub const GAMESERVER_KEY_HEADER: &str = "x-mysticparadox-gameserver-apikey";
pub const GAMESERVER_KEY_ALT_HEADER: &str = "x-mysticparadox-gameserver-key";

/// Node's `req.AuthData`: the verified token's claims, plus what the caller is.
#[derive(Debug, Clone, Default)]
pub struct AuthData {
    /// `AuthData.userId`, when it is a string
    pub user_id: Option<String>,
    /// `IsGameserver`: the call carried a valid game server key
    pub is_gameserver: bool,
    /// The token's claims (`userId`, `iat`, `exp`, `aud`, `iss`); empty for a bearer-less game server call
    pub claims: Map<String, Value>,
    /// `IsPlayerHostRuntime` and `RuntimeRosterAccountIds`: a player host's runtime that rustic-rifts authenticated
    pub runtime: Option<RuntimeAuth>,
}

impl AuthData {
    /// `RuntimeRosterOf`: the accounts a player host's runtime may act for; `None` when the caller is not one.
    pub fn runtime_roster(&self) -> Option<BTreeSet<String>> {
        self.runtime.as_ref().map(|runtime| {
            runtime
                .roster
                .iter()
                .map(ToString::to_string)
                .filter(|id| !id.is_empty())
                .collect()
        })
    }
}

/// An empty reply with a status, as `res.status(n).send()`.
pub fn empty(status: u16) -> Response {
    StatusCode::from_u16(status)
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
        .into_response()
}

/// Express 5 hands a throwing async handler to its default error handler: a 500 page.
pub fn server_error() -> Response {
    service_kit::body::error_page(StatusCode::INTERNAL_SERVER_ERROR)
}

/// A header as Node reads it: a repeated `authorization` keeps its first value, any other repeated header is the values
/// joined with ", ".
pub fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    let mut values = headers
        .get_all(name)
        .iter()
        .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned());
    let first = values.next()?;
    if name == "authorization" {
        return Some(first);
    }
    Some(values.fold(first, |joined, next| format!("{joined}, {next}")))
}

/// `AuthHeader.startsWith("bearer ")` and its two other spellings.
fn has_bearer_prefix(authorization: &str) -> bool {
    ["bearer ", "Bearer ", "BEARER "]
        .iter()
        .any(|prefix| authorization.starts_with(prefix))
}

/// `AuthHeader.slice("bearer ".length)`: the seven characters, whatever they spell.
pub fn bearer_token(authorization: &str) -> &str {
    authorization.get(7..).unwrap_or_default()
}

static INVALID_SEGMENT: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"(^|/)INVALID(/|$)").ok());

/// `encodeURIComponent`.
fn encode_uri_component(text: &str) -> String {
    use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
    const KEEP: &AsciiSet = &NON_ALPHANUMERIC
        .remove(b'-')
        .remove(b'_')
        .remove(b'.')
        .remove(b'!')
        .remove(b'~')
        .remove(b'*')
        .remove(b'\'')
        .remove(b'(')
        .remove(b')');
    utf8_percent_encode(text, KEEP).to_string()
}

/// A game server's read whose path carries the `INVALID` sentinel is re-pointed at the configured account before any
/// router reads it. Only `req.url` changes, as in Node: the route's parameters were bound to the original path.
/// Writes are never rewritten, so the phantom local player can never overwrite real progression.
fn rewrite_invalid_sentinel(shared: &Shared, request: &mut Request) {
    let method = request.method();
    if method != axum::http::Method::GET && method != axum::http::Method::HEAD {
        return;
    }
    let Some(account) = shared.version.hooks.no_player_policy().path_account(&shared.settings) else {
        return;
    };
    let Some(pattern) = INVALID_SEGMENT.as_ref() else {
        return;
    };
    let path = request.uri().path().to_owned();
    if account.is_empty() || !pattern.is_match(&path) {
        return;
    }
    let replacement = format!("${{1}}{}${{2}}", encode_uri_component(&account).replace('$', "$$"));
    let rewritten = pattern.replace_all(&path, replacement.as_str()).into_owned();
    tracing::info!("Gameserver INVALID sentinel resolved to '{account}' for {method} {rewritten}");
    let url = match request.uri().query() {
        Some(query) => format!("{rewritten}?{query}"),
        None => rewritten,
    };
    if let Ok(uri) = url.parse::<Uri>() {
        *request.uri_mut() = uri;
    }
}

fn request_info(shared: &Shared, request: &Request, request_id: &str) -> RequestInfo {
    let _ = shared;
    RequestInfo {
        path: request.uri().path().to_owned(),
        headers: request
            .headers()
            .iter()
            .filter_map(|(name, value)| Some((name.as_str().to_owned(), value.to_str().ok()?.to_owned())))
            .collect(),
        remote: peer_ip(request),
        request_id: request_id.to_owned(),
    }
}

/// `IsCurrentLauncherAccountEligible`: gameserver, service and dev identities are not launcher accounts and keep their
/// API-key or token behaviour.
pub async fn is_current_launcher_account_eligible(
    shared: &Shared,
    claims: &Map<String, Value>,
) -> Result<bool, crate::repo::RepoError> {
    let Some(user_id) = claims.get("userId").and_then(Value::as_str) else {
        return Ok(true);
    };
    Ok(launcher_by_user(shared, user_id)
        .await?
        .is_none_or(|account| is_account_eligible(&account)))
}

fn user_id_of(claims: &Map<String, Value>) -> Option<String> {
    claims.get("userId").and_then(Value::as_str).map(str::to_owned)
}

/// `HasParadoxBackendAuth`. `Ok` is Node's `next()`; `Err` is the answer it sent. The chat area reads what it needs of
/// `AuthData` from the request's extensions, where this leaves it.
pub async fn game_auth(call: &mut Call) -> Result<AuthData, Response> {
    let auth = authenticate_game(call).await?;
    publish(&mut call.request, &auth);
    Ok(auth)
}

/// Puts [`AuthData`] into the request's extensions; each area adapts it through its own reader (`chat::auth`,
/// `characters::auth_data`, `bounties_store::authn::auth_of_request`, `matchmaking::authenticate`,
/// `progression::AuthData::of_extensions`, `inventory::ports::GameAuth`).
pub fn publish(request: &mut Request, auth: &AuthData) {
    request.extensions_mut().insert(auth.clone());
}

async fn authenticate_game(call: &mut Call) -> Result<AuthData, Response> {
    let shared = call.shared.clone();
    let authorization = header(call.request.headers(), "authorization");
    let gameserver_key = header(call.request.headers(), GAMESERVER_KEY_HEADER)
        .or_else(|| header(call.request.headers(), GAMESERVER_KEY_ALT_HEADER));

    if let Some(key) = gameserver_key {
        let valid = is_valid_gameserver_api_key(&shared, &key).await.map_err(|error| {
            tracing::error!("game server key check: {error}");
            server_error()
        })?;
        if !valid {
            tracing::error!("Invalid Gameserver API Key Auth");
            return Err(empty(401));
        }
        rewrite_invalid_sentinel(&shared, &mut call.request);
        if let Some(authorization) = authorization {
            // The validation is not guarded in Node: a bad token reaches Express's error handler
            let claims =
                validate_game_token(&shared.settings, bearer_token(&authorization)).map_err(|_| server_error())?;
            if !is_current_launcher_account_eligible(&shared, &claims)
                .await
                .map_err(|_| server_error())?
            {
                return Err(empty(403));
            }
            return Ok(AuthData {
                user_id: user_id_of(&claims),
                is_gameserver: true,
                claims,
                runtime: None,
            });
        }
        // A game server request with no player bearer is attributed to the configured identity, if any
        let no_player = shared
            .version
            .hooks
            .no_player_policy()
            .bearerless_account(&shared.settings);
        return Ok(AuthData {
            user_id: no_player,
            is_gameserver: true,
            claims: Map::new(),
            runtime: None,
        });
    }

    // An optional module may authenticate its own runtime credential (rustic-rifts)
    let info = request_info(&shared, &call.request, &call.request_id);
    match shared.rifts.realms().authenticate(info).await {
        Ok(AuthOutcome::Authenticated(runtime)) => {
            return Ok(AuthData {
                user_id: None,
                is_gameserver: false,
                claims: Map::new(),
                runtime: Some(runtime),
            });
        }
        Ok(AuthOutcome::Refused(reply)) => return Err(rifts_reply_response(reply)),
        Ok(AuthOutcome::NotMine) => {}
        Err(error) => {
            tracing::error!("rifts authenticate: {error}");
            return Err(server_error());
        }
    }

    if let Some(ticket_raw) = header(call.request.headers(), "x-reforged-session-ticket")
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
    {
        let Ok((ticket, _ticket_game)) = module_api::network::SessionTicket::from_token(&ticket_raw) else {
            tracing::warn!("Request with malformed x-reforged-session-ticket!");
            return Err(empty(401));
        };
        let Ok(expected_game) = shared.version.tables.game_version.parse::<module_api::GameVersion>() else {
            return Err(server_error());
        };
        let admitted_account = match shared.home().verify_session_ticket(&ticket, &expected_game).await {
            Ok(account) => account,
            Err(error) => {
                tracing::warn!("Request with invalid x-reforged-session-ticket: {error}");
                return Err(empty(401));
            }
        };
        let user_id = admitted_account.to_string();
        let mut claims = Map::new();
        claims.insert("userId".into(), Value::String(user_id.clone()));
        return Ok(AuthData {
            user_id: Some(user_id),
            is_gameserver: false,
            claims,
            runtime: None,
        });
    }

    let Some(authorization) = authorization.filter(|a| has_bearer_prefix(a)) else {
        tracing::error!(
            "Unauthenticated {} to {} which needs Mystic Paradox Metagame auth!",
            call.request.method(),
            call.request.uri().path()
        );
        return Err(empty(401));
    };
    let claims = match validate_game_token(&shared.settings, bearer_token(&authorization)) {
        Ok(claims) => claims,
        Err(_) => {
            tracing::warn!("Request with bad Mystic Paradox Metagame auth!");
            return Err(empty(401));
        }
    };
    // Any failure inside Node's try (a database error included) answers like a bad token
    match is_current_launcher_account_eligible(&shared, &claims).await {
        Ok(true) => Ok(AuthData {
            user_id: user_id_of(&claims),
            is_gameserver: false,
            claims,
            runtime: None,
        }),
        Ok(false) => Err(empty(403)),
        Err(_) => {
            tracing::warn!("Request with bad Mystic Paradox Metagame auth!");
            Err(empty(401))
        }
    }
}

fn rifts_reply_response(reply: rifts_seam::Reply) -> Response {
    let status = StatusCode::from_u16(reply.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (
        status,
        [(axum::http::header::CONTENT_TYPE, service_kit::json::JSON_UTF8)],
        reply.body.0,
    )
        .into_response()
}

// ------------------------------------------------------------------ the launcher

/// `LauncherAuthData` (`{userId, sid}`) and the account it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LauncherAuth {
    pub user_id: String,
    /// The refresh session's family
    pub sid: String,
    pub account: AccountId,
}

fn expired() -> LauncherApiError {
    LauncherApiError::new("AUTH_UNAUTHORIZED", "Your session has expired. Please sign in again.")
}

/// `HasLauncherAuth`: the launcher-audience token, an account that exists, a refresh family that is live, and an
/// account that is admitted.
pub async fn launcher_auth(shared: &Shared, headers: &HeaderMap) -> Result<LauncherAuth, LauncherApiError> {
    let authorization = header(headers, "authorization");
    launcher_auth_account(shared, authorization.as_deref())
        .await
        .map(|(auth, _)| auth)
}

/// [`launcher_auth`] over the header's text, with the account it found (the Reforged API's launcher routes need it).
pub async fn launcher_auth_account(
    shared: &Shared,
    authorization: Option<&str>,
) -> Result<(LauncherAuth, LauncherAccountRecord), LauncherApiError> {
    let Some(authorization) = authorization.filter(|a| has_bearer_prefix(a)) else {
        return Err(LauncherApiError::new("AUTH_UNAUTHORIZED", "Sign in required."));
    };
    let payload = match validate_launcher_access_token(&shared.settings, bearer_token(authorization)) {
        Ok(payload) => payload,
        Err(_) => {
            tracing::warn!("Request with bad launcher auth!");
            return Err(expired());
        }
    };
    let account = match launcher_by_user(shared, &payload.user_id).await {
        Ok(account) => account,
        Err(error) => {
            tracing::warn!("Request with bad launcher auth! {error}");
            return Err(expired());
        }
    };
    let Some(account) = account else { return Err(expired()) };
    let family = payload.sid.clone();
    let id = account.account.clone();
    let now = super::util::iso_now();
    let active = match super::data::store(shared) {
        Ok(store) => {
            let id = id.clone();
            store
                .read(move |conn, repos| repos.refresh_sessions.is_family_active(conn, &family, &id, &now))
                .await
        }
        Err(error) => Err(error),
    };
    if !active.unwrap_or(false) {
        return Err(expired());
    }
    assert_account_admitted(&account)?;
    let auth = LauncherAuth {
        user_id: payload.user_id,
        sid: payload.sid,
        account: id,
    };
    Ok((auth, account))
}

/// `RequireActiveAccount({consent})`: a deactivated account is refused, and with `consent` an account without a current
/// consent while `MYSTICPARADOX_CONSENT_ENFORCE` is on.
pub async fn require_active_account(shared: &Shared, user_id: &str, consent: bool) -> Result<(), LauncherApiError> {
    let account = match launcher_by_user(shared, user_id).await {
        Ok(Some(account)) => account,
        Ok(None) => return Err(LauncherApiError::new("AUTH_UNAUTHORIZED", "Sign in required.")),
        Err(error) => {
            tracing::error!("Unhandled error in the active-account guard: {error}");
            return Err(super::errors::internal());
        }
    };
    assert_account_not_deactivated(&account)?;
    if consent {
        assert_consent_current_when_enforced(&account, &shared.settings)?;
    }
    Ok(())
}

// ------------------------------------------------------------------ the callers rustic-rifts sees

/// What a rifts route needs of its caller, from the middleware Node lists on it (the route inventory's Middleware column).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RiftsAuth {
    /// No auth middleware
    None,
    /// `HasLauncherAuth`, then `RequireActiveAccount` (with `consent`) when `active` holds
    Launcher { active: Option<bool> },
    /// `RequireHostCapability`: rustic-rifts validates the capability itself
    HostCapability,
    /// `HasAdminAuth` (the Reforged API's)
    Admin,
}

fn rifts_auth(method: &str, path: &str) -> RiftsAuth {
    use RiftsAuth::{Admin, HostCapability, Launcher, None};
    if path.starts_with("/admin/") {
        return Admin;
    }
    if path.starts_with("/launcher/v1/realms") {
        // The router's HasLauncherAuth, with RequireActiveAccount on what joins (no consent) or hosts (consent)
        let joins = method == "POST"
            && (matches!(
                path,
                "/launcher/v1/realms/join" | "/launcher/v1/realms/presence/heartbeat"
            ) || (path.starts_with("/launcher/v1/realms/player-invites/") && path.ends_with("/accept")));
        let hosts =
            method == "POST" && (path == "/launcher/v1/realms" || path.starts_with("/launcher/v1/realms/mine/"));
        return Launcher {
            active: if hosts {
                Some(true)
            } else if joins {
                Some(false)
            } else {
                Option::None
            },
        };
    }
    if path.starts_with("/launcher/v1/realm-hosting/hubs/start") {
        return Launcher { active: Some(true) };
    }
    if path.starts_with("/launcher/v1/p2p/") || path.starts_with("/launcher/v1/realm-hosting/") {
        let needs_active = path == "/launcher/v1/p2p/host-presence"
            || (path.starts_with("/launcher/v1/p2p/host-offers/") && path.ends_with("/respond"))
            || ["/host-bootstrap", "/join-bootstrap", "/peers"]
                .iter()
                .any(|s| path.ends_with(s));
        return Launcher {
            active: needs_active.then_some(false),
        };
    }
    if path.starts_with("/launcher/v1/guard/") {
        return Launcher { active: Option::None };
    }
    if path.starts_with("/mystic/v1/hunts/") {
        let launcher = path.ends_with("/tickets/self") || path.ends_with("/status") || path.ends_with("/reward/self");
        return if launcher {
            Launcher { active: Option::None }
        } else {
            HostCapability
        };
    }
    None
}

/// Whether Node's `HasLauncherAuth` stands in front of a rifts route, so a request with no token is refused 401 before
/// rustic-rifts answers.
pub fn rifts_needs_launcher_auth(method: &str, route_path: &str) -> bool {
    matches!(rifts_auth(method, route_path), RiftsAuth::Launcher { .. })
}

/// `Caller` for a rifts route: who the auth middleware says is calling. `Err` is the refusal Node's middleware sent.
pub async fn rifts_caller(
    shared: &Shared,
    method: &str,
    request_path: &str,
    route_path: &str,
    headers: &HeaderMap,
) -> Result<Caller, Response> {
    let _ = request_path;
    match rifts_auth(method, route_path) {
        RiftsAuth::None | RiftsAuth::HostCapability => Ok(Caller::Anonymous),
        RiftsAuth::Admin => match &shared.store {
            Some(store) => {
                let caller =
                    crate::areas::reforged_api::admin::authenticate_headers(shared, store, method, headers).await?;
                Ok(Caller::Admin { key_id: caller.user_id })
            }
            None => Ok(Caller::Anonymous),
        },
        RiftsAuth::Launcher { active } => {
            let auth = launcher_auth(shared, headers)
                .await
                .map_err(IntoResponse::into_response)?;
            if let Some(consent) = active {
                require_active_account(shared, &auth.user_id, consent)
                    .await
                    .map_err(IntoResponse::into_response)?;
            }
            // The account in the form rifts knows it: the global id's text (DD18)
            Ok(Caller::Launcher(rifts_seam::AccountId(auth.account.to_string())))
        }
    }
}

/// The client address the launcher routes rate-limit by.
pub fn ip_of(call: &Call) -> String {
    client_ip(&call.request)
}

/// `ClientAddress` (`security/rateLimit.ts`): `ipKeyGenerator(req.ip || "unknown")` over [`client_ip`].
pub fn rate_limit_address(request: &Request) -> String {
    super::util::ip_key_generator(&client_ip(request))
}
