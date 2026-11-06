// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The admin half (admin spec): the `/admin/v1` wrapper and its caller, sign-in, Discord sign-in, players and audit,
//! tester max, the release gates, the host-only routes, the module's admin actions and home operations, and the
//! administrator bootstrap.

pub mod actions;
pub mod auth;
pub mod bootstrap;
pub mod discord;
pub mod host_routes;
pub mod players;
pub mod release_gates;
pub mod signin;
pub mod tester_max;

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, PoisonError};

use axum::body::Bytes;
use axum::response::Response;
use http::{HeaderMap, HeaderValue, Method, StatusCode};
use http_body_util::BodyExt as _;
use module_api::{Actor, ActorContext};
use rustic_isles::{Call, Params, Shared, UserId};
use rustic_isles_auth::DiscordApi;
use rustic_isles_store::repo::auth::LauncherAccountRecord;
use rustic_isles_store::repo::reforged::AdminSessionRecord;
use rustic_isles_store::{AccountId, IslesStore, Repositories};
use serde_json::Value;

use crate::util::{self, object};

pub use auth::{AdminAuthenticatorSlot, authenticate_admin};
pub use tester_max::{EconomyFunding, TesterFunding};

// ------------------------------------------------------------------ per-module state

/// The admin half's per-module state: the operator nonce set, the tester-max busy set, Discord and the tester
/// funding port. The composer puts it in the slots.
pub struct ReforgedAdmin {
    nonces: Mutex<HashMap<String, i64>>,
    busy: Mutex<HashSet<String>>,
    discord: Arc<dyn DiscordApi>,
    funding: Arc<dyn TesterFunding>,
    chat: Option<Arc<rustic_isles_chat::StoreChat>>,
}

// The nonce set holds accepted signatures: only its size is shown
impl std::fmt::Debug for ReforgedAdmin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let nonces = self.nonces.lock().unwrap_or_else(PoisonError::into_inner).len();
        f.debug_struct("ReforgedAdmin")
            .field("nonces", &nonces)
            .field("chat", &self.chat.is_some())
            .finish_non_exhaustive()
    }
}

impl Default for ReforgedAdmin {
    fn default() -> Self {
        Self::new(
            Arc::new(rustic_isles_auth::HttpsDiscord::new()),
            Arc::new(EconomyFunding),
        )
    }
}

impl ReforgedAdmin {
    /// The admin half over a Discord client and the tester max funding.
    pub fn new(discord: Arc<dyn DiscordApi>, funding: Arc<dyn TesterFunding>) -> Self {
        Self {
            nonces: Mutex::new(HashMap::new()),
            busy: Mutex::new(HashSet::new()),
            discord,
            funding,
            chat: None,
        }
    }

    /// The chat crate answers the `chat.*` home operations.
    #[must_use]
    pub fn with_chat(mut self, chat: Arc<rustic_isles_chat::StoreChat>) -> Self {
        self.chat = Some(chat);
        self
    }

    /// The module's, or the module state's fallback when the composer put none in the slots.
    pub fn of(shared: &Shared) -> Arc<Self> {
        shared
            .slot::<Self>()
            .unwrap_or_else(|| crate::state::ReforgedState::of(shared).admin_fallback())
    }

    /// The Discord client sign-in uses.
    pub fn discord(&self) -> &Arc<dyn DiscordApi> {
        &self.discord
    }

    /// What tester max grants through.
    pub fn funding(&self) -> &Arc<dyn TesterFunding> {
        &self.funding
    }

    /// The chat crate, when it answers the `chat.*` home operations.
    pub fn chat(&self) -> Option<&Arc<rustic_isles_chat::StoreChat>> {
        self.chat.as_ref()
    }

    /// Records a verified signature; false when it was accepted in the last 120 s.
    pub(crate) fn accept_nonce(&self, signature: &str, now_ms: i64) -> bool {
        let mut nonces = self.nonces.lock().unwrap_or_else(PoisonError::into_inner);
        nonces.retain(|_, at| now_ms - *at < auth::NONCE_TTL_MS);
        if nonces.contains_key(signature) {
            return false;
        }
        nonces.insert(signature.to_owned(), now_ms);
        true
    }

    /// Marks the target busy; false when a run already holds it.
    pub(crate) fn begin_run(&self, user_id: &str) -> bool {
        self.busy
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(user_id.to_owned())
    }

    pub(crate) fn end_run(&self, user_id: &str) {
        self.busy.lock().unwrap_or_else(PoisonError::into_inner).remove(user_id);
    }
}

// ------------------------------------------------------------------ the caller

/// How the caller signed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    /// A dashboard session cookie.
    Session,
    /// The manager's operator credential.
    Manager,
}

/// The admin caller: a session's account, or the manager's synthetic administrator.
#[derive(Clone)]
pub struct AdminCaller {
    /// The caller's userId; the operator id for the manager.
    pub user_id: String,
    /// The name shown in the audit.
    pub display_name: String,
    /// None for the manager
    pub account: Option<LauncherAccountRecord>,
    /// The session, none for the manager.
    pub session: Option<AdminSessionRecord>,
    /// How the caller signed in.
    pub via: Via,
    /// The manager's `createdAt` (now at acceptance)
    pub created_at: String,
}

// Never the session's token hash or CSRF token
impl std::fmt::Debug for AdminCaller {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdminCaller")
            .field("user_id", &self.user_id)
            .field("via", &self.via)
            .field("account", &self.account.as_ref().map(|a| a.account.to_string()))
            .finish_non_exhaustive()
    }
}

impl AdminCaller {
    /// The caller's account id, none for the manager.
    pub fn account_id(&self) -> Option<&AccountId> {
        self.account.as_ref().map(|a| &a.account)
    }
}

/// `Admin { key: <caller userId> }`.
pub fn admin_actor(caller: &AdminCaller) -> ActorContext {
    ActorContext::new(Actor::Admin {
        key: caller.user_id.clone(),
    })
}

// ------------------------------------------------------------------ the request as handlers see it

/// An admin request as handlers see it, the body already read.
pub struct AdminRequest {
    /// The HTTP method.
    pub method: Method,
    /// The path.
    pub path: String,
    /// Path and query as received
    pub target: String,
    /// The request's headers.
    pub headers: HeaderMap,
    /// Query pairs in order.
    pub query: Vec<(String, String)>,
    /// The body's bytes.
    pub raw_body: Bytes,
    /// The parsed JSON body, none when absent.
    pub body: Option<Value>,
    /// The client address (auth §4.6).
    pub client_ip: String,
    /// The socket peer.
    pub peer: Option<IpAddr>,
}

impl AdminRequest {
    /// A body field, when the body is an object.
    pub fn field(&self, name: &str) -> Option<&Value> {
        util::field(self.body.as_ref(), name)
    }

    /// A query value given exactly once.
    pub fn single(&self, name: &str) -> Option<&str> {
        util::single(&self.query, name)
    }

    /// Every value of a query name joined with `,`.
    pub fn joined(&self, name: &str) -> Option<String> {
        util::joined(&self.query, name)
    }

    /// A header's first value as text.
    pub fn header(&self, name: &str) -> Option<String> {
        util::header_text(&self.headers, name)
    }
}

/// What an admin handler gets.
pub struct AdminContext {
    /// The module's shared state.
    pub shared: Arc<Shared>,
    /// The module's store.
    pub store: Arc<IslesStore>,
    /// The request.
    pub request: AdminRequest,
    /// Path parameters.
    pub params: Params,
    /// The signed-in caller; none on an open route.
    pub caller: Option<AdminCaller>,
    /// The request's id.
    pub request_id: String,
}

impl AdminContext {
    /// A path parameter, empty when absent.
    pub fn param(&self, name: &str) -> &str {
        self.params.get(name).unwrap_or("")
    }

    /// The caller's userId (empty for an open route).
    pub fn caller_user_id(&self) -> String {
        self.caller.as_ref().map(|c| c.user_id.clone()).unwrap_or_default()
    }

    /// The caller as the store's actor, `System` on an open route.
    pub fn actor(&self) -> ActorContext {
        self.caller
            .as_ref()
            .map_or_else(|| ActorContext::new(Actor::System), admin_actor)
    }
}

/// Whether a route needs a signed-in caller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminAuth {
    /// A session or the operator credential is required.
    Required,
    /// Anyone may call it (sign-in itself).
    Open,
}

// ------------------------------------------------------------------ errors and readers

/// `{"error":{"code","message"}}` with the status.
pub fn admin_error(status: StatusCode, code: &str, message: &str) -> Response {
    util::admin_error(status, code, message)
}

/// An empty 500, logged.
pub fn admin_internal(error: impl std::fmt::Display) -> Response {
    tracing::error!("[ADMIN] {error}");
    util::empty(StatusCode::INTERNAL_SERVER_ERROR)
}

/// The 404 for an unknown player.
pub fn player_not_found() -> Response {
    admin_error(StatusCode::NOT_FOUND, "NOT_FOUND", "Player not found.")
}

/// `Number.parseInt(text, 10)`: leading white space, an optional sign, leading digits; none is NaN (`None`).
pub fn parse_int(text: &str) -> Option<i64> {
    let t = text.trim_start_matches(service_kit::js::is_space);
    let (sign, digits) = match t.as_bytes().first() {
        Some(b'-') => (-1, &t[1..]),
        Some(b'+') => (1, &t[1..]),
        _ => (1, t),
    };
    let end = digits.bytes().position(|b| !b.is_ascii_digit()).unwrap_or(digits.len());
    if end == 0 {
        return None;
    }
    let value: i64 = digits[..end].parse().unwrap_or(i64::MAX);
    Some(sign * value)
}

/// The joined `page`, default 1, at least 1.
pub fn page_of(request: &AdminRequest) -> u32 {
    let text = request.joined("page").unwrap_or_else(|| "1".to_owned());
    match parse_int(&text) {
        None | Some(0) => 1,
        Some(n) => u32::try_from(n.max(1)).unwrap_or(u32::MAX),
    }
}

/// The joined `limit`, the default for NaN or 0, clamped to `1..=max`.
pub fn limit_of(request: &AdminRequest, default: u32, max: u32) -> u32 {
    let n = match request.joined("limit").as_deref().and_then(parse_int) {
        None | Some(0) => i64::from(default),
        Some(n) => n,
    };
    u32::try_from(n.clamp(1, i64::from(max))).unwrap_or(default)
}

/// Rows to skip for a 1-based page.
pub fn skip_of(page: u32, limit: u32) -> u32 {
    page.saturating_sub(1).saturating_mul(limit)
}

/// The game account's userId, else the account id text.
pub fn user_id_of(conn: &mut rustic_isles_store::AnyConnection, repos: &Repositories, account: &AccountId) -> String {
    match repos.accounts.find(conn, account) {
        Ok(Some(game)) => game.user_id.as_str().to_owned(),
        _ => account.to_string(),
    }
}

/// The live launcher account whose game account has that userId.
pub fn find_target(
    conn: &mut rustic_isles_store::AnyConnection,
    repos: &Repositories,
    user_id: &str,
) -> rustic_isles_store::RepoResult<Option<LauncherAccountRecord>> {
    let user = UserId::new(user_id);
    if let Some(game) = repos.accounts.find_by_user_id(conn, &user)?
        && let Some(found) = repos.launcher_accounts.find_by_account(conn, &game.id)?
    {
        return Ok(Some(found));
    }
    repos.launcher_accounts.find_by_user_id(conn, &user)
}

// ------------------------------------------------------------------ the public account (§4.4)

/// A launcher account as the dashboard reads it.
pub fn public_account(account: &LauncherAccountRecord, user_id: &str, now_iso: &str) -> Value {
    let mut pairs = vec![
        ("userId", Value::from(user_id)),
        ("displayName", Value::from(account.display_name.clone())),
        ("email", Value::from(account.email.clone().unwrap_or_default())),
        (
            "approvalStatus",
            Value::from(
                account
                    .approval_status
                    .map_or("approved", rustic_isles_store::repo::auth::ApprovalStatus::as_str),
            ),
        ),
        ("status", Value::from(account.status.as_str())),
        (
            "roles",
            Value::Array(account.roles.iter().cloned().map(Value::from).collect()),
        ),
    ];
    let opt = |v: &Option<String>| v.clone().map(Value::from);
    let push = |pairs: &mut Vec<(&str, Value)>, name: &'static str, value: Option<Value>| {
        if let Some(value) = value {
            pairs.push((name, value));
        }
    };
    push(&mut pairs, "rolesUpdatedAt", opt(&account.roles_updated_at));
    push(&mut pairs, "rolesUpdatedBy", opt(&account.roles_updated_by));
    pairs.push(("usernameSet", Value::Bool(account.username_set != Some(false))));
    pairs.push(("createdAt", Value::from(account.created_at.clone())));
    push(&mut pairs, "lastLoginAt", opt(&account.last_login_at));
    push(&mut pairs, "approvalUpdatedAt", opt(&account.approval_updated_at));
    push(&mut pairs, "approvalUpdatedBy", opt(&account.approval_updated_by));
    push(&mut pairs, "approvalReason", opt(&account.approval_reason));
    if let Some(timeout) = account.timeout_at(now_iso) {
        pairs.push(("timeoutUntil", Value::from(timeout.until.clone())));
        pairs.push(("timeoutSince", Value::from(timeout.since.clone())));
        push(&mut pairs, "timeoutReason", timeout.reason.clone().map(Value::from));
    }
    object(pairs)
}

/// The caller's public account; the manager's is synthetic.
pub fn caller_account(caller: &AdminCaller, now_iso: &str) -> Value {
    match &caller.account {
        Some(account) => public_account(account, &caller.user_id, now_iso),
        None => object(vec![
            ("userId", Value::from(caller.user_id.clone())),
            ("displayName", Value::from(caller.display_name.clone())),
            ("email", Value::from("")),
            ("approvalStatus", Value::from("approved")),
            ("status", Value::from("active")),
            ("roles", Value::Array(vec![Value::from("admin")])),
            ("usernameSet", Value::Bool(true)),
            ("createdAt", Value::from(caller.created_at.clone())),
        ]),
    }
}

// ------------------------------------------------------------------ the wrapper (§3.1)

/// `ADMIN_ALLOWED_ORIGINS`, else the two localhost ports outside production.
pub fn allowed_origins(settings: &rustic_isles::Settings) -> Vec<String> {
    let configured: Vec<String> = settings
        .raw("ADMIN_ALLOWED_ORIGINS")
        .unwrap_or("")
        .split(',')
        .map(|o| util::trim(o).to_owned())
        .filter(|o| !o.is_empty())
        .collect();
    if !configured.is_empty() {
        return configured;
    }
    if settings.raw("NODE_ENV") == Some("production") {
        return Vec::new();
    }
    vec!["http://localhost:4173".to_owned(), "http://localhost:5174".to_owned()]
}

/// The origin check: `Err` is the empty 403.
pub fn check_origin(settings: &rustic_isles::Settings, headers: &HeaderMap) -> Result<Option<String>, Response> {
    let Some(origin) = util::header_text(headers, "origin") else {
        return Ok(None);
    };
    if allowed_origins(settings).contains(&origin) {
        return Ok(Some(origin));
    }
    tracing::warn!("[ADMIN] rejected origin {origin}");
    Err(util::empty(StatusCode::FORBIDDEN))
}

/// The admin headers (set, replacing any).
pub fn admin_headers(mut response: Response, origin: Option<&str>) -> Response {
    let headers = response.headers_mut();
    if let Some(origin) = origin
        && let Ok(value) = HeaderValue::from_str(origin)
    {
        headers.insert("access-control-allow-origin", value);
        headers.insert("access-control-allow-credentials", HeaderValue::from_static("true"));
        headers.insert(
            "access-control-allow-headers",
            HeaderValue::from_static("Content-Type, X-CSRF-Token"),
        );
        headers.insert(
            "access-control-allow-methods",
            HeaderValue::from_static("GET, POST, PUT, PATCH, DELETE, OPTIONS"),
        );
        headers.insert("vary", HeaderValue::from_static("Origin"));
    }
    headers.insert("cache-control", HeaderValue::from_static("no-store"));
    headers.insert(
        "content-security-policy",
        HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'"),
    );
    response
}

/// Whether a JSON content type was sent (the operator credential's body rule).
fn is_json_type(headers: &HeaderMap) -> bool {
    util::header_lossy(headers, "content-type")
        .map(|t| {
            t.split(';')
                .next()
                .unwrap_or("")
                .trim()
                .eq_ignore_ascii_case("application/json")
        })
        .unwrap_or(false)
}

/// What the operator credential reads of the body: whether one is declared, and its bytes when JSON was read.
#[derive(Debug, Clone, Default)]
pub struct BodyFacts {
    /// Whether the request declares a body.
    pub declared: bool,
    /// Whether the body was read as JSON.
    pub json_read: bool,
}

fn declared_body(headers: &HeaderMap) -> bool {
    headers.contains_key("transfer-encoding")
        || util::header_lossy(headers, "content-length").is_some_and(|v| v.trim() != "0")
}

/// Every admin-gated handler of both halves runs inside this.
pub async fn admin_route<F, Fut>(call: Call, required: AdminAuth, handler: F) -> Response
where
    F: FnOnce(AdminContext) -> Fut,
    Fut: Future<Output = Response>,
{
    let Call {
        shared,
        route,
        params,
        mut request,
        request_id,
        ..
    } = call;
    let body = match rustic_isles::read_body(&mut request).await {
        Ok(body) => body,
        Err(page) => return page,
    };
    let client_ip = util::client_ip(&request);
    let peer = util::peer_ip(&request);
    let (parts, stream) = request.into_parts();
    let raw_body = if body.is_some() {
        stream.collect().await.map(|c| c.to_bytes()).unwrap_or_default()
    } else {
        Bytes::new()
    };
    let origin = match check_origin(shared.settings(), &parts.headers) {
        Ok(origin) => origin,
        Err(refused) => return refused,
    };
    let Some(store) = util::store_of(&shared) else {
        return admin_headers(util::no_store(route, &request_id), origin.as_deref());
    };
    let facts = BodyFacts {
        declared: declared_body(&parts.headers),
        json_read: body.is_some() && is_json_type(&parts.headers),
    };
    let target = parts.uri.path_and_query().map_or("/", |p| p.as_str()).to_owned();
    let request = AdminRequest {
        method: parts.method.clone(),
        path: parts.uri.path().to_owned(),
        target,
        query: util::query_pairs(parts.uri.query()),
        headers: parts.headers,
        raw_body,
        body,
        client_ip,
        peer,
    };
    let caller = match required {
        AdminAuth::Open => None,
        AdminAuth::Required => match authenticate_admin(&shared, &store, &request, &facts).await {
            Ok(caller) => Some(caller),
            Err(refused) => return admin_headers(refused, origin.as_deref()),
        },
    };
    let context = AdminContext {
        shared,
        store,
        request,
        params,
        caller,
        request_id,
    };
    admin_headers(handler(context).await, origin.as_deref())
}

/// A fresh request id for an audit record (never the replay seed).
pub fn fresh_id() -> String {
    util::random_uuid()
}

// ------------------------------------------------------------------ registration and start-up

type Run = fn(AdminContext) -> module_api::BoxFuture<'static, Response>;

/// The admin half's 18 rows (§1.2), at the route table's exact paths.
pub fn register_admin(handlers: &mut rustic_isles::Handlers) {
    use rustic_isles::Method::{Delete, Get, Patch, Post};
    let b = rustic_isles::Service::Backend;
    let wrapped = |h: &mut rustic_isles::Handlers, method, path: &'static str, auth: AdminAuth, run: Run| {
        h.on(b, method, path, move |call| admin_route(call, auth, run));
    };
    handlers
        .on(b, Get, "/health/release-gates", release_gates::release_gates)
        .on(b, Post, "/reforged/api/CreateInvite", host_routes::create_invite)
        .on(b, Get, "/reforged/api/InviteCodes", host_routes::invite_codes)
        .on(b, Delete, "/reforged/api/InviteCode/:code", host_routes::delete_invite)
        .on(
            b,
            Post,
            "/reforged/api/SetAccountStatus",
            host_routes::set_account_status,
        )
        .on(b, Post, "/reforged/api/Kick", host_routes::kick);
    let (open, required) = (AdminAuth::Open, AdminAuth::Required);
    wrapped(handlers, Post, "/admin/v1/auth/login", open, |c| {
        Box::pin(signin::login(c))
    });
    wrapped(handlers, Post, "/admin/v1/auth/logout", required, |c| {
        Box::pin(signin::logout(c))
    });
    wrapped(handlers, Get, "/admin/v1/auth/discord/start", open, |c| {
        Box::pin(discord::start(c))
    });
    wrapped(handlers, Get, "/admin/v1/auth/discord/callback", open, |c| {
        Box::pin(discord::callback(c))
    });
    wrapped(handlers, Get, "/admin/v1/me", required, |c| Box::pin(signin::me(c)));
    wrapped(handlers, Get, "/admin/v1/online-players", required, |c| {
        Box::pin(players::online(c))
    });
    wrapped(handlers, Get, "/admin/v1/players", required, |c| {
        Box::pin(players::list(c))
    });
    wrapped(handlers, Patch, "/admin/v1/players/:userId/access", required, |c| {
        Box::pin(players::access(c))
    });
    wrapped(handlers, Patch, "/admin/v1/players/:userId/roles", required, |c| {
        Box::pin(players::roles(c))
    });
    wrapped(handlers, Delete, "/admin/v1/players/:userId", required, |c| {
        Box::pin(players::delete(c))
    });
    wrapped(handlers, Get, "/admin/v1/audit", required, |c| {
        Box::pin(players::audit(c))
    });
    wrapped(handlers, Post, "/admin/v1/players/:userId/tester-max", required, |c| {
        Box::pin(tester_max::tester_max(c))
    });
}

struct AdminStartup;

impl rustic_isles::Startup for AdminStartup {
    fn start<'a>(
        &'a self,
        shared: &'a Arc<Shared>,
    ) -> module_api::BoxFuture<'a, Result<rustic_isles::Started, String>> {
        Box::pin(async move {
            auth::check_operator_config(shared.settings())?;
            shared.set_admin_authenticator(Arc::new(AdminAuthenticatorSlot::new(shared)));
            Ok(rustic_isles::Started::default())
        })
    }
}

/// The admin half's start-up: the operator secret check (§3.3), then core's authenticator slot for the `/admin/`
/// rifts rows (§3.5).
pub fn admin_startups() -> Vec<Arc<dyn rustic_isles::Startup>> {
    vec![Arc::new(AdminStartup)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ints() {
        assert_eq!(parse_int("  12abc"), Some(12));
        assert_eq!(parse_int("-4"), Some(-4));
        assert_eq!(parse_int("abc"), None);
        assert_eq!(parse_int(""), None);
        assert_eq!(parse_int("+7"), Some(7));
    }
}
