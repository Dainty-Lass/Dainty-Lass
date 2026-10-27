//! The admin API (`routes/admin.ts`, `controllers/admin.ts`, `middleware/HasAdminAuth.ts`): a cookie session with a
//! CSRF token for the dashboard, the players and their access, the audit. Mounted only with `MYSTICPARADOX_ADMIN_API`
//! on; with it off the pipeline answers 404 before any of this (app.ts).
//!
//! Not here yet: the manager's signed operator credential (`security/managerOperator.ts`, Mystic's own manager) and
//! Express's `OPTIONS` answers and origin check for paths this router has no route for.

use std::future::Future;
use std::sync::{Arc, PoisonError, RwLock};

use axum::http::header::{HeaderName, HeaderValue};
use axum::response::Response;
use module_api::{Actor, ActorContext};
use serde_json::{Map, Value, json};

use super::launcher_api::is_account_eligible;
use super::presence;
use super::util::{self, BodyKind, Req, hex, is_rate_limited, random_bytes, sha256_hex};
use crate::db::IslesStore;
use crate::http::{Call, Handlers, Shared, not_implemented};
use crate::ids::{AccountId, UserId};
use crate::repo::auth::{AccessChange, AdminAccountFilter, ApprovalStatus, LauncherAccountRecord, OperationalStatus};
use crate::repo::reforged::{AdminAuditRecord, AdminSessionRecord};
use crate::repo::{RepoError, RepoResult};
use crate::routes::{Method, Params, Service};

pub const ADMIN_COOKIE_NAME: &str = "mysticparadox_admin";
const SESSION_TTL_MS: i64 = 8 * 60 * 60 * 1000;
const BODY_LIMIT: usize = 100 * 1024;
pub const ASSIGNABLE_ROLES: [&str; 1] = ["tester"];
pub const TESTER_ROLE: &str = "tester";
const ONLINE_PRESENCE_WINDOW_MS: i64 = 60_000;

// ---------------------------------------------------------------- what auth supplies

/// The checks of a sign-in that belong to the auth area: the password hash and the authenticator code.
pub trait AdminCredentials: Send + Sync {
    /// `VerifyPassword(hash, password)`
    fn verify_password(&self, hash: Option<&str>, password: &str) -> bool;
    /// `VerifyTotp(secret, code)`
    fn verify_totp(&self, secret: &str, code: Option<&Value>) -> bool;
}

static CREDENTIALS: RwLock<Option<Arc<dyn AdminCredentials>>> = RwLock::new(None);

/// Registers the password and TOTP checks (the auth area at start). Without them every sign-in is refused.
pub fn set_credentials(credentials: Option<Arc<dyn AdminCredentials>>) {
    *CREDENTIALS.write().unwrap_or_else(PoisonError::into_inner) = credentials;
}

pub(super) fn credentials() -> Option<Arc<dyn AdminCredentials>> {
    CREDENTIALS.read().unwrap_or_else(PoisonError::into_inner).clone()
}

// ---------------------------------------------------------------- the request

/// An authenticated operator.
#[derive(Debug, Clone)]
pub struct AdminCaller {
    pub session: Option<AdminSessionRecord>,
    pub account: LauncherAccountRecord,
    /// The client's `userId` of the operator's account (the audit's actor)
    pub user_id: String,
}

pub struct Ctx {
    pub shared: Arc<Shared>,
    pub store: IslesStore,
    pub req: Req,
    pub params: Params,
    pub admin: Option<AdminCaller>,
}

pub fn error(status: u16, code: &str, message: &str) -> Response {
    util::json(status, &json!({ "error": { "code": code, "message": message } }))
}

fn production(shared: &Shared) -> bool {
    shared.settings.raw("NODE_ENV") == Some("production")
}

/// `AllowedOrigins`.
fn allowed_origins(shared: &Shared) -> Vec<String> {
    match shared
        .settings
        .raw("ADMIN_ALLOWED_ORIGINS")
        .filter(|c| !c.trim().is_empty())
    {
        None if production(shared) => vec![],
        None => vec!["http://localhost:4173".into(), "http://localhost:5174".into()],
        Some(configured) => configured
            .split(',')
            .map(|o| o.trim().to_owned())
            .filter(|o| !o.is_empty())
            .collect(),
    }
}

/// `adminRouter.use("/admin/v1", ...)`: the origin check, and the headers every answer carries.
fn origin_headers(shared: &Shared, req: &Req) -> Result<Vec<(&'static str, String)>, Response> {
    let mut headers = Vec::new();
    if let Some(origin) = req.header("origin") {
        if !allowed_origins(shared).iter().any(|o| o == origin) {
            tracing::warn!("[ADMIN] rejected origin {origin}");
            return Err(util::empty(403));
        }
        headers.push(("access-control-allow-origin", origin.to_owned()));
        headers.push(("access-control-allow-credentials", "true".to_owned()));
        headers.push(("access-control-allow-headers", "Content-Type, X-CSRF-Token".to_owned()));
        headers.push((
            "access-control-allow-methods",
            "GET, POST, PUT, PATCH, DELETE, OPTIONS".to_owned(),
        ));
        headers.push(("vary", "Origin".to_owned()));
    }
    headers.push(("cache-control", "no-store".to_owned()));
    headers.push((
        "content-security-policy",
        "default-src 'none'; frame-ancestors 'none'".to_owned(),
    ));
    Ok(headers)
}

fn apply(mut response: Response, headers: &[(&'static str, String)]) -> Response {
    for (name, value) in headers {
        if let Ok(value) = HeaderValue::from_str(value) {
            response.headers_mut().insert(HeaderName::from_static(name), value);
        }
    }
    response
}

/// The cookie `name` of a `Cookie` header, percent-decoded; `Err` where `decodeURIComponent` throws.
fn read_cookie(header: Option<&str>, name: &str) -> Result<Option<String>, ()> {
    let Some(header) = header else {
        return Ok(None);
    };
    for part in header.split(';') {
        let part = part.trim();
        let (key, value) = part.split_once('=').unwrap_or((part, ""));
        if key == name {
            return service_kit::js::decode_uri_component(value).map(Some).ok_or(());
        }
    }
    Ok(None)
}

/// `HasAdminAuth`'s cookie path.
async fn has_admin_auth(store: &IslesStore, req: &Req) -> Result<AdminCaller, Response> {
    let raw = match read_cookie(req.header("cookie"), ADMIN_COOKIE_NAME) {
        Ok(Some(raw)) if !raw.is_empty() => raw,
        Ok(_) => return Err(error(401, "ADMIN_UNAUTHORIZED", "Administrator sign-in required.")),
        Err(()) => {
            return Err(service_kit::body::error_page(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            ));
        }
    };
    let hash = sha256_hex(raw.as_bytes());
    let now = util::now_iso();
    let lookup = (hash, now);
    let found = store
        .read(move |conn, repos| {
            let Some(session) = repos
                .admin
                .find_active_session_by_token_hash(conn, &lookup.0, &lookup.1)?
            else {
                return Ok(None);
            };
            let account = repos.launcher_accounts.find_by_account(conn, &session.account)?;
            let user_id = repos
                .accounts
                .find(conn, &session.account)?
                .map(|a| a.user_id.to_string());
            Ok(Some((session, account, user_id)))
        })
        .await
        .map_err(|e| internal(&e))?;
    let Some((session, account, user_id)) = found else {
        return Err(error(401, "ADMIN_UNAUTHORIZED", "Administrator session expired."));
    };
    let admitted = account
        .as_ref()
        .filter(|a| a.roles.iter().any(|r| r == "admin") && is_account_eligible(a));
    let Some(account) = admitted else {
        let (id, at) = (session.id.clone(), util::now_iso());
        let _ = store
            .write(&ActorContext::new(Actor::System), move |tx, repos| {
                repos.admin.revoke_session(tx, &id, &at)
            })
            .await;
        return Err(error(403, "ADMIN_FORBIDDEN", "Administrator access denied."));
    };
    if !matches!(req.method.as_str(), "GET" | "HEAD" | "OPTIONS") {
        let csrf = req.header("x-csrf-token");
        if csrf != Some(session.csrf_token.as_str()) {
            return Err(error(403, "ADMIN_CSRF", "Invalid request token."));
        }
    }
    let user_id = user_id.unwrap_or_else(|| session.account.to_string());
    Ok(AdminCaller {
        account: account.clone(),
        session: Some(session),
        user_id,
    })
}

fn internal(error: &dyn std::fmt::Display) -> Response {
    tracing::error!("[ADMIN] {error}");
    util::empty(500)
}

/// Runs an admin route: the body parsed, the origin checked, the operator authenticated when the route asks, and
/// the admin headers on whatever comes back.
pub async fn route<F, Fut>(call: Call, need_auth: bool, handler: F) -> Response
where
    F: FnOnce(Ctx) -> Fut,
    Fut: Future<Output = Response>,
{
    let (shared, params, route, request_id) = (
        call.shared.clone(),
        call.params.clone(),
        call.route,
        call.request_id.clone(),
    );
    let req = match Req::read(call.request, BodyKind::Json(BODY_LIMIT)).await {
        Ok(req) => req,
        Err(response) => return response,
    };
    let headers = match origin_headers(&shared, &req) {
        Ok(headers) => headers,
        Err(response) => return response,
    };
    let Some(store) = shared.store.clone() else {
        return apply(not_implemented(route, &request_id), &headers);
    };
    let admin = if need_auth {
        match has_admin_auth(&store, &req).await {
            Ok(caller) => Some(caller),
            Err(response) => return apply(response, &headers),
        }
    } else {
        None
    };
    apply(
        handler(Ctx {
            shared,
            store,
            req,
            params,
            admin,
        })
        .await,
        &headers,
    )
}

impl Ctx {
    pub fn caller(&self) -> &AdminCaller {
        // Only routes mounted behind the auth gate reach for it
        self.admin
            .as_ref()
            .unwrap_or_else(|| unreachable!("a route behind HasAdminAuth"))
    }

    pub fn fields(&self) -> Map<String, Value> {
        self.req.fields()
    }
}

// ---------------------------------------------------------------- shapes

/// `EffectiveApprovalStatus`.
pub fn effective_approval(account: &LauncherAccountRecord) -> &'static str {
    match account.approval_status.unwrap_or(ApprovalStatus::Approved) {
        ApprovalStatus::Pending => "pending",
        ApprovalStatus::Approved => "approved",
        ApprovalStatus::Rejected => "rejected",
    }
}

pub fn status_text(status: OperationalStatus) -> &'static str {
    match status {
        OperationalStatus::Active => "active",
        OperationalStatus::Banned => "banned",
        OperationalStatus::Disabled => "disabled",
    }
}

/// `PublicAccount`: a field that is undefined in Node is left out.
pub fn public_account(account: &LauncherAccountRecord, user_id: &str) -> Value {
    let mut out = Map::new();
    out.insert("userId".into(), json!(user_id));
    out.insert("displayName".into(), json!(account.display_name));
    out.insert("email".into(), json!(account.email.clone().unwrap_or_default()));
    out.insert("approvalStatus".into(), json!(effective_approval(account)));
    out.insert("status".into(), json!(status_text(account.status)));
    out.insert("roles".into(), json!(account.roles));
    let mut optional = |key: &str, value: &Option<String>| {
        if let Some(value) = value {
            out.insert(key.to_owned(), json!(value));
        }
    };
    optional("rolesUpdatedAt", &account.roles_updated_at);
    optional("rolesUpdatedBy", &account.roles_updated_by);
    out.insert("usernameSet".into(), json!(account.username_set != Some(false)));
    out.insert("createdAt".into(), json!(account.created_at));
    let mut optional = |key: &str, value: &Option<String>| {
        if let Some(value) = value {
            out.insert(key.to_owned(), json!(value));
        }
    };
    optional("lastLoginAt", &account.last_login_at);
    optional("approvalUpdatedAt", &account.approval_updated_at);
    optional("approvalUpdatedBy", &account.approval_updated_by);
    optional("approvalReason", &account.approval_reason);
    Value::Object(out)
}

/// The `userId` the client knows an account by.
pub fn user_id_of(
    conn: &mut crate::repo::AnyConnection,
    repos: &crate::repo::Repositories,
    account: &AccountId,
) -> RepoResult<String> {
    Ok(repos
        .accounts
        .find(conn, account)?
        .map_or_else(|| account.to_string(), |game| game.user_id.to_string()))
}

/// `Number.parseInt(text, 10)`: the leading integer, `None` for NaN.
pub fn parse_int(text: &str) -> Option<i64> {
    let text = text.trim_start();
    let (sign, digits) = match text.strip_prefix('-') {
        Some(rest) => (-1, rest),
        None => (1, text.strip_prefix('+').unwrap_or(text)),
    };
    let end = digits.bytes().position(|b| !b.is_ascii_digit()).unwrap_or(digits.len());
    (end > 0)
        .then(|| digits[..end].parse::<i64>().ok().map(|n| sign * n))
        .flatten()
}

/// `String(req.query.name)`: repeated values join with a comma, as an array does.
fn query_text(req: &Req, name: &str) -> Option<String> {
    let values: Vec<&str> = req
        .query
        .iter()
        .filter(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
        .collect();
    (!values.is_empty()).then(|| values.join(","))
}

pub fn page_of(req: &Req) -> i64 {
    1.max(
        parse_int(&query_text(req, "page").unwrap_or_else(|| "1".into()))
            .filter(|n| *n != 0)
            .unwrap_or(1),
    )
}

pub fn limit_of(req: &Req, default: i64, max: i64) -> i64 {
    let raw = parse_int(&query_text(req, "limit").unwrap_or_else(|| default.to_string()))
        .filter(|n| *n != 0)
        .unwrap_or(default);
    max.min(1.max(raw))
}

fn skip_of(page: i64, limit: i64) -> u32 {
    u32::try_from(((page - 1) * limit).clamp(0, i64::from(u32::MAX))).unwrap_or(u32::MAX)
}

/// `ParsePlayerIds`.
pub fn parse_player_ids(value: Option<&str>) -> Option<Vec<String>> {
    let value = value.filter(|v| !v.trim().is_empty())?;
    let mut ids: Vec<String> = Vec::new();
    for id in value.split(',').map(str::trim) {
        let valid =
            (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        if valid && !ids.iter().any(|known| known == id) {
            ids.push(id.to_owned());
        }
    }
    ids.truncate(100);
    Some(ids)
}

fn opaque_token() -> String {
    hex(&random_bytes(32))
}

fn actor(caller: &AdminCaller) -> ActorContext {
    ActorContext::new(Actor::Admin {
        key: caller.user_id.clone(),
    })
}

// ---------------------------------------------------------------- auth routes

fn cookie_options(shared: &Shared, max_age_seconds: i64) -> String {
    let secure = production(shared)
        || shared
            .settings
            .raw("ADMIN_COOKIE_SECURE")
            .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    format!(
        "Path=/admin/v1; HttpOnly; SameSite=Strict; Max-Age={max_age_seconds}{}",
        if secure { "; Secure" } else { "" }
    )
}

fn set_cookie(mut response: Response, value: &str) -> Response {
    if let Ok(value) = HeaderValue::from_str(value) {
        response.headers_mut().append(axum::http::header::SET_COOKIE, value);
    }
    response
}

async fn login(ctx: Ctx) -> Response {
    let ip = ctx.req.ip();
    if is_rate_limited(&format!("admin-login:{ip}"), 6, 15 * 60 * 1000) {
        return error(429, "ADMIN_RATE_LIMITED", "Too many sign-in attempts.");
    }
    let fields = ctx.fields();
    let email = fields
        .get("email")
        .and_then(Value::as_str)
        .map_or_else(String::new, |e| service_kit::js::trim(e).to_lowercase());
    let password = fields
        .get("password")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let lookup = email.clone();
    let found = match ctx
        .store
        .read(move |conn, repos| {
            let Some(account) = repos.launcher_accounts.find_by_email(conn, &lookup)? else {
                return Ok(None);
            };
            let user_id = user_id_of(conn, repos, &account.account)?;
            Ok(Some((account, user_id)))
        })
        .await
    {
        Ok(found) => found,
        Err(e) => return internal(&e),
    };
    let invalid = || {
        error(
            401,
            "ADMIN_INVALID_CREDENTIALS",
            "Administrator credentials are invalid.",
        )
    };
    let Some((account, user_id)) = found else {
        return invalid();
    };
    let verified = credentials().is_some_and(|c| c.verify_password(account.password_hash.as_deref(), &password));
    if !verified || !account.roles.iter().any(|r| r == "admin") || !is_account_eligible(&account) {
        return invalid();
    }
    let secret = ctx.shared.settings.get("ADMIN_TOTP_SECRET").map(str::to_owned);
    match secret {
        None if production(&ctx.shared) => {
            return error(503, "ADMIN_MFA_NOT_CONFIGURED", "Administrator MFA is not configured.");
        }
        Some(secret) if !credentials().is_some_and(|c| c.verify_totp(&secret, fields.get("totp"))) => {
            return error(401, "ADMIN_MFA_INVALID", "The authenticator code is invalid.");
        }
        _ => {}
    }
    let raw_token = opaque_token();
    let csrf = opaque_token();
    let now = util::now_ms();
    let session = AdminSessionRecord {
        id: uuid::Uuid::new_v4().to_string(),
        token_hash: sha256_hex(raw_token.as_bytes()),
        account: account.account.clone(),
        csrf_token: csrf.clone(),
        created_at: util::iso(now),
        expires_at: util::iso(now + SESSION_TTL_MS),
        ip,
        user_agent: ctx
            .req
            .header("user-agent")
            .unwrap_or_default()
            .chars()
            .take(300)
            .collect(),
        revoked_at: None,
    };
    let created = ctx
        .store
        .write(
            &ActorContext::new(Actor::Admin { key: user_id.clone() }),
            move |tx, repos| repos.admin.create_session(tx, &session),
        )
        .await;
    if let Err(e) = created {
        return internal(&e);
    }
    let encoded = percent_encoding::utf8_percent_encode(&raw_token, super::updates::COMPONENT).to_string();
    set_cookie(
        util::json(
            200,
            &json!({ "csrfToken": csrf, "account": public_account(&account, &user_id) }),
        ),
        &format!(
            "{ADMIN_COOKIE_NAME}={encoded}; {}",
            cookie_options(&ctx.shared, SESSION_TTL_MS / 1000)
        ),
    )
}

async fn logout(ctx: Ctx) -> Response {
    let caller = ctx.caller().clone();
    // A manager request has no backend session to revoke or cookie to clear
    let Some(session) = caller.session else {
        return util::json(200, &json!({}));
    };
    let at = util::now_iso();
    let result = ctx
        .store
        .write(&actor(ctx.caller()), move |tx, repos| {
            repos.admin.revoke_session(tx, &session.id, &at)
        })
        .await;
    if let Err(e) = result {
        return internal(&e);
    }
    set_cookie(
        util::json(200, &json!({})),
        &format!("{ADMIN_COOKIE_NAME}=; {}", cookie_options(&ctx.shared, 0)),
    )
}

async fn me(ctx: Ctx) -> Response {
    let caller = ctx.caller();
    // A manager request has no session, so no CSRF token
    let csrf = caller.session.as_ref().map_or(Value::Null, |s| json!(s.csrf_token));
    util::json(
        200,
        &json!({ "csrfToken": csrf, "account": public_account(&caller.account, &caller.user_id) }),
    )
}

// ---------------------------------------------------------------- players

async fn list_players(ctx: Ctx) -> Response {
    let page = page_of(&ctx.req);
    let limit = limit_of(&ctx.req, 25, 100);
    if let Some(ids) = parse_player_ids(query_text(&ctx.req, "ids").as_deref()) {
        let count = ids.len();
        let found = ctx
            .store
            .read(move |conn, repos| {
                let mut out = Vec::new();
                for id in ids {
                    let Some(game) = repos.accounts.find_by_user_id(conn, &UserId(id))? else {
                        continue;
                    };
                    if let Some(account) = repos.launcher_accounts.find_by_account(conn, &game.id)? {
                        out.push(public_account(&account, game.user_id.as_str()));
                    }
                }
                Ok(out)
            })
            .await;
        return match found {
            Ok(accounts) => util::json(
                200,
                &json!({ "accounts": accounts, "total": accounts.len(), "page": 1, "limit": count }),
            ),
            Err(e) => internal(&e),
        };
    }
    let approval = match query_text(&ctx.req, "approvalStatus").as_deref() {
        Some("pending") => Some(ApprovalStatus::Pending),
        Some("approved") => Some(ApprovalStatus::Approved),
        Some("rejected") => Some(ApprovalStatus::Rejected),
        _ => None,
    };
    let status = match query_text(&ctx.req, "status").as_deref() {
        Some("active") => Some(OperationalStatus::Active),
        Some("disabled") => Some(OperationalStatus::Disabled),
        Some("banned") => Some(OperationalStatus::Banned),
        _ => None,
    };
    let filter = AdminAccountFilter {
        approval_status: approval,
        status,
        search: ctx
            .req
            .query_string("search")
            .map(|s| service_kit::js::trim(s).chars().take(100).collect()),
        skip: skip_of(page, limit),
        limit: u32::try_from(limit).unwrap_or(25),
    };
    let result = ctx
        .store
        .read(move |conn, repos| {
            let (records, total) = repos.launcher_accounts.list_for_admin(conn, &filter)?;
            let mut accounts = Vec::new();
            for record in records {
                let user_id = user_id_of(conn, repos, &record.account)?;
                accounts.push(public_account(&record, &user_id));
            }
            Ok((accounts, total))
        })
        .await;
    match result {
        Ok((accounts, total)) => util::json(
            200,
            &json!({ "accounts": accounts, "total": total, "page": page, "limit": limit }),
        ),
        Err(e) => internal(&e),
    }
}

async fn online_players(ctx: Ctx) -> Response {
    let ids = presence::active_players(ONLINE_PRESENCE_WINDOW_MS);
    let result = ctx
        .store
        .read(move |conn, repos| {
            let mut players: Vec<(String, String)> = Vec::new();
            for id in ids {
                let Some(game) = repos.accounts.find_by_user_id(conn, &UserId(id.clone()))? else {
                    continue;
                };
                if let Some(account) = repos.launcher_accounts.find_by_account(conn, &game.id)? {
                    players.push((id, account.display_name));
                }
            }
            Ok(players)
        })
        .await;
    match result {
        Ok(mut players) => {
            players.sort_by(|a, b| service_kit::collate::locale_compare(&a.1, &b.1));
            util::json(
                200,
                &json!({
                    "count": players.len(),
                    "players": players.iter().map(|(id, name)| json!({ "userId": id, "displayName": name })).collect::<Vec<_>>(),
                    "observedAt": util::now_iso(),
                    "source": "xmpp+presence",
                }),
            )
        }
        Err(e) => internal(&e),
    }
}

#[allow(clippy::too_many_arguments)]
fn audit_record(
    caller: &AdminCaller,
    req: &Req,
    target: Option<&str>,
    action: &str,
    old: Value,
    new: Value,
    reason: Option<String>,
    request_id: &str,
    at: &str,
) -> AdminAuditRecord {
    AdminAuditRecord {
        id: uuid::Uuid::new_v4().to_string(),
        actor_user_id: caller.user_id.clone(),
        target_user_id: target.map(str::to_owned),
        action: action.to_owned(),
        old_state: Some(old),
        new_state: Some(new),
        reason,
        ip: req.ip(),
        request_id: request_id.to_owned(),
        created_at: at.to_owned(),
    }
}

fn not_found() -> Response {
    error(404, "NOT_FOUND", "Player not found.")
}

/// A target by `userId`: the game account id, and the launcher account.
fn find_target(
    conn: &mut crate::repo::AnyConnection,
    repos: &crate::repo::Repositories,
    user_id: &str,
) -> RepoResult<Option<(AccountId, LauncherAccountRecord)>> {
    let Some(game) = repos.accounts.find_by_user_id(conn, &UserId(user_id.to_owned()))? else {
        return Ok(None);
    };
    Ok(repos
        .launcher_accounts
        .find_by_account(conn, &game.id)?
        .map(|a| (game.id, a)))
}

fn approval_of(text: &str) -> Option<ApprovalStatus> {
    match text {
        "pending" => Some(ApprovalStatus::Pending),
        "approved" => Some(ApprovalStatus::Approved),
        "rejected" => Some(ApprovalStatus::Rejected),
        _ => None,
    }
}

fn status_of(text: &str) -> Option<OperationalStatus> {
    match text {
        "active" => Some(OperationalStatus::Active),
        "disabled" => Some(OperationalStatus::Disabled),
        "banned" => Some(OperationalStatus::Banned),
        _ => None,
    }
}

fn reason_of(fields: &Map<String, Value>) -> String {
    fields
        .get("reason")
        .and_then(Value::as_str)
        .map_or_else(String::new, |r| service_kit::js::trim(r).chars().take(500).collect())
}

async fn update_access(ctx: Ctx) -> Response {
    let caller = ctx.caller().clone();
    let target_user = ctx.params.get("userId").unwrap_or_default().to_owned();
    let fields = ctx.fields();
    let validation = |message: &str| error(400, "ADMIN_VALIDATION", message);
    let approval_field = fields.get("approvalStatus").filter(|v| !v.is_null());
    let status_field = fields.get("status").filter(|v| !v.is_null());
    let approval = match approval_field {
        None => None,
        Some(v) => match v.as_str().and_then(approval_of) {
            Some(a) => Some(a),
            None => return validation("Invalid approval status."),
        },
    };
    let status = match status_field {
        None => None,
        Some(v) => match v.as_str().and_then(status_of) {
            Some(s) => Some(s),
            None => return validation("Invalid account status."),
        },
    };
    if approval.is_none() && status.is_none() {
        return validation("No access-state change was supplied.");
    }
    let reason = reason_of(&fields);
    if reason.chars().count() < 3 {
        return error(
            400,
            "ADMIN_ACCESS_REASON",
            "Provide a reason of at least 3 characters for every server access change.",
        );
    }
    let would_remove = approval.is_some_and(|a| a != ApprovalStatus::Approved)
        || status.is_some_and(|s| s != OperationalStatus::Active);
    if target_user == caller.user_id && would_remove {
        return error(
            409,
            "ADMIN_SELF_LOCKOUT",
            "You cannot reject, disable, or ban your own administrator account.",
        );
    }
    let request_id = uuid::Uuid::new_v4().to_string();
    let at = util::now_iso();
    let lookup = target_user.clone();
    let found = match ctx
        .store
        .read(move |conn, repos| find_target(conn, repos, &lookup))
        .await
    {
        Ok(found) => found,
        Err(e) => return internal(&e),
    };
    let Some((account_id, before)) = found else {
        return not_found();
    };
    let change = AccessChange {
        approval_status: approval,
        status,
        approval_updated_at: at.clone(),
        approval_updated_by: caller.user_id.clone(),
        approval_reason: Some(reason.clone()),
    };
    let record_ctx = (
        caller.clone(),
        ctx.req.ip(),
        request_id.clone(),
        at.clone(),
        target_user.clone(),
    );
    let (before_state, reason_copy) = (
        json!({ "approvalStatus": effective_approval(&before), "status": status_text(before.status) }),
        reason.clone(),
    );
    let target = account_id.clone();
    let req_ip = record_ctx.1.clone();
    let result = ctx
        .store
        .account_write(&account_id, &actor(&caller), move |tx, repos| {
            let Some(after) = repos.launcher_accounts.set_access_state(tx, &target, &change)? else {
                return Ok(None);
            };
            if !is_account_eligible(&after) {
                repos.refresh_sessions.revoke_all_for_account(tx, &target)?;
                repos.game_exchange_codes.revoke_unused_for_account(tx, &target)?;
            }
            repos.admin.append_audit(
                tx,
                &AdminAuditRecord {
                    id: uuid::Uuid::new_v4().to_string(),
                    actor_user_id: record_ctx.0.user_id.clone(),
                    target_user_id: Some(record_ctx.4.clone()),
                    action: "player.access.update".into(),
                    old_state: Some(before_state),
                    new_state: Some(
                        json!({ "approvalStatus": effective_approval(&after), "status": status_text(after.status) }),
                    ),
                    reason: Some(reason_copy),
                    ip: req_ip,
                    request_id: record_ctx.2.clone(),
                    created_at: record_ctx.3.clone(),
                },
            )?;
            Ok(Some(after))
        })
        .await;
    match result {
        Ok(Some(after)) => util::json(
            200,
            &json!({ "account": public_account(&after, &target_user), "requestId": request_id }),
        ),
        Ok(None) => not_found(),
        Err(RepoError::NotHome { .. }) => {
            error(409, "ADMIN_NOT_HOME", "That player's account lives on another server.")
        }
        Err(e) => internal(&e),
    }
}

async fn update_roles(ctx: Ctx) -> Response {
    let caller = ctx.caller().clone();
    let target_user = ctx.params.get("userId").unwrap_or_default().to_owned();
    let fields = ctx.fields();
    let reason = reason_of(&fields);
    let requested: Option<Vec<String>> = match fields.get("roles") {
        Some(Value::Array(items)) if items.iter().all(Value::is_string) => {
            Some(items.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
        }
        _ => None,
    };
    let Some(requested) = requested else {
        return error(400, "ADMIN_VALIDATION", "roles must be an array of strings.");
    };
    if !requested.iter().all(|r| ASSIGNABLE_ROLES.contains(&r.as_str())) {
        return error(
            400,
            "ADMIN_VALIDATION",
            &format!("Only these roles can be toggled here: {}.", ASSIGNABLE_ROLES.join(", ")),
        );
    }
    let lookup = target_user.clone();
    let found = match ctx
        .store
        .read(move |conn, repos| find_target(conn, repos, &lookup))
        .await
    {
        Ok(found) => found,
        Err(e) => return internal(&e),
    };
    let Some((account_id, before)) = found else {
        return not_found();
    };
    let preserved = before.roles.iter().filter(|r| !ASSIGNABLE_ROLES.contains(&r.as_str()));
    let next: Vec<String> = preserved.cloned().chain(requested).collect();
    let request_id = uuid::Uuid::new_v4().to_string();
    let at = util::now_iso();
    let target = account_id.clone();
    let (by, ip, rid, when, tu) = (
        caller.user_id.clone(),
        ctx.req.ip(),
        request_id.clone(),
        at.clone(),
        target_user.clone(),
    );
    let before_roles = before.roles.clone();
    let result = ctx
        .store
        .account_write(&account_id, &actor(&caller), move |tx, repos| {
            let Some(after) = repos.launcher_accounts.update_roles(tx, &target, &next, &when, &by)? else {
                return Ok(None);
            };
            // A beta/dev exchange code must not survive removal of Tester during its 60-second lifetime
            if before_roles.iter().any(|r| r == TESTER_ROLE) && !after.roles.iter().any(|r| r == TESTER_ROLE) {
                repos.game_exchange_codes.revoke_unused_for_account(tx, &target)?;
            }
            repos.admin.append_audit(
                tx,
                &AdminAuditRecord {
                    id: uuid::Uuid::new_v4().to_string(),
                    actor_user_id: by,
                    target_user_id: Some(tu),
                    action: "player.roles.update".into(),
                    old_state: Some(json!({ "roles": before_roles })),
                    new_state: Some(json!({ "roles": after.roles })),
                    reason: (!reason.is_empty()).then_some(reason),
                    ip,
                    request_id: rid,
                    created_at: when,
                },
            )?;
            Ok(Some(after))
        })
        .await;
    match result {
        Ok(Some(after)) => util::json(
            200,
            &json!({ "account": public_account(&after, &target_user), "requestId": request_id }),
        ),
        Ok(None) => not_found(),
        Err(e) => internal(&e),
    }
}

async fn delete_player(ctx: Ctx) -> Response {
    let caller = ctx.caller().clone();
    let target_user = ctx.params.get("userId").unwrap_or_default().to_owned();
    if target_user == caller.user_id {
        return error(
            409,
            "ADMIN_SELF_DELETE",
            "You cannot delete your own administrator account.",
        );
    }
    let lookup = target_user.clone();
    let found = match ctx
        .store
        .read(move |conn, repos| find_target(conn, repos, &lookup))
        .await
    {
        Ok(found) => found,
        Err(e) => return internal(&e),
    };
    let Some((account_id, target)) = found else {
        return not_found();
    };
    let fields = ctx.fields();
    let expected = format!("DELETE {}", target.display_name);
    if fields.get("confirmation").and_then(Value::as_str) != Some(expected.as_str()) {
        return error(
            400,
            "ADMIN_DELETE_CONFIRMATION",
            &format!("Type \"{expected}\" to confirm permanent deletion."),
        );
    }
    let reason = reason_of(&fields);
    if reason.chars().count() < 3 {
        return error(
            400,
            "ADMIN_DELETE_REASON",
            "Provide a deletion reason for the audit log.",
        );
    }
    let request_id = uuid::Uuid::new_v4().to_string();
    let record = audit_record(
        &caller,
        &ctx.req,
        Some(&target_user),
        "player.account.delete",
        json!({ "approvalStatus": effective_approval(&target), "status": status_text(target.status), "roles": target.roles }),
        json!({ "deleted": true }),
        Some(reason),
        &request_id,
        &util::now_iso(),
    );
    let (target_account, user) = (account_id.clone(), target_user.clone());
    let result = ctx
        .store
        .account_write(&account_id, &actor(&caller), move |tx, repos| {
            let mut deleted = repos.admin.delete_player_data(tx, &target_account, &user, &record)?;
            repos.refresh_sessions.delete_all_for_account(tx, &target_account)?;
            repos
                .game_exchange_codes
                .revoke_unused_for_account(tx, &target_account)?;
            Ok(deleted_with_characters(&mut deleted, tx.conn(), repos, &target_account))
        })
        .await;
    match result {
        Ok(deleted) => {
            let counts: Map<String, Value> = deleted
                .deleted_counts
                .iter()
                .map(|(k, v)| (k.clone(), json!(v)))
                .collect();
            util::json(
                200,
                &json!({ "requestId": request_id, "deletedCounts": counts, "characterCount": deleted.character_count }),
            )
        }
        Err(e) => internal(&e),
    }
}

/// The characters the account had, when the characters area can say.
fn deleted_with_characters(
    deleted: &mut crate::repo::reforged::PlayerDeletion,
    conn: &mut crate::repo::AnyConnection,
    repos: &crate::repo::Repositories,
    account: &AccountId,
) -> crate::repo::reforged::PlayerDeletion {
    deleted.character_count = repos
        .characters
        .find_many_by_account(conn, account)
        .map_or(0, |characters| characters.len() as u64);
    deleted.clone()
}

async fn list_audit(ctx: Ctx) -> Response {
    let page = page_of(&ctx.req);
    let limit = limit_of(&ctx.req, 50, 100);
    let target = ctx.req.query_string("targetUserId").map(str::to_owned);
    let skip = skip_of(page, limit);
    let take = u32::try_from(limit).unwrap_or(50);
    let result = ctx
        .store
        .read(move |conn, repos| repos.admin.list_audit(conn, target.as_deref(), skip, take))
        .await;
    match result {
        Ok((records, total)) => {
            let records: Vec<Value> = records.iter().map(audit_json).collect();
            util::json(
                200,
                &json!({ "records": records, "total": total, "page": page, "limit": limit }),
            )
        }
        Err(e) => internal(&e),
    }
}

/// An audit record as the dashboard reads it: what Node left undefined is left out.
pub fn audit_json(record: &AdminAuditRecord) -> Value {
    let mut out = Map::new();
    out.insert("id".into(), json!(record.id));
    out.insert("actorUserId".into(), json!(record.actor_user_id));
    if let Some(target) = &record.target_user_id {
        out.insert("targetUserId".into(), json!(target));
    }
    out.insert("action".into(), json!(record.action));
    if let Some(old) = &record.old_state {
        out.insert("oldState".into(), old.clone());
    }
    if let Some(new) = &record.new_state {
        out.insert("newState".into(), new.clone());
    }
    if let Some(reason) = &record.reason {
        out.insert("reason".into(), json!(reason));
    }
    out.insert("ip".into(), json!(record.ip));
    out.insert("requestId".into(), json!(record.request_id));
    out.insert("createdAt".into(), json!(record.created_at));
    Value::Object(out)
}

pub fn register(handlers: &mut Handlers) {
    let b = Service::Backend;
    handlers.on(b, Method::Post, "/admin/v1/auth/login", |call| {
        Box::pin(route(call, false, login))
    });
    handlers.on(b, Method::Post, "/admin/v1/auth/logout", |call| {
        Box::pin(route(call, true, logout))
    });
    handlers.on(b, Method::Get, "/admin/v1/me", |call| Box::pin(route(call, true, me)));
    handlers.on(b, Method::Get, "/admin/v1/online-players", |call| {
        Box::pin(route(call, true, online_players))
    });
    handlers.on(b, Method::Get, "/admin/v1/players", |call| {
        Box::pin(route(call, true, list_players))
    });
    handlers.on(b, Method::Patch, "/admin/v1/players/:userId/access", |call| {
        Box::pin(route(call, true, update_access))
    });
    handlers.on(b, Method::Patch, "/admin/v1/players/:userId/roles", |call| {
        Box::pin(route(call, true, update_roles))
    });
    handlers.on(b, Method::Delete, "/admin/v1/players/:userId", |call| {
        Box::pin(route(call, true, delete_player))
    });
    handlers.on(b, Method::Get, "/admin/v1/audit", |call| {
        Box::pin(route(call, true, list_audit))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_int_reads_the_leading_integer() {
        for (text, expected) in [
            ("12", Some(12)),
            (" 7abc", Some(7)),
            ("-3", Some(-3)),
            ("+4", Some(4)),
            ("abc", None),
            ("", None),
            ("1,2", Some(1)),
            ("0", Some(0)),
        ] {
            assert_eq!(parse_int(text), expected, "{text}");
        }
    }

    #[test]
    fn player_ids_are_cleaned_like_node() {
        assert_eq!(parse_player_ids(None), None);
        assert_eq!(parse_player_ids(Some("  ")), None);
        assert_eq!(
            parse_player_ids(Some("a, b ,a,bad id,,c-1_")),
            Some(vec!["a".into(), "b".into(), "c-1_".into()])
        );
        assert_eq!(parse_player_ids(Some("!!")), Some(vec![]));
        let many: String = (0..150).map(|n| format!("u{n},")).collect();
        assert_eq!(parse_player_ids(Some(&many)).map(|ids| ids.len()), Some(100));
    }

    #[test]
    fn cookies_are_read_and_decoded() {
        assert_eq!(
            read_cookie(Some("a=1; mysticparadox_admin=ab%20c=d; b=2"), ADMIN_COOKIE_NAME),
            Ok(Some("ab c=d".into()))
        );
        assert_eq!(read_cookie(Some("a=1"), ADMIN_COOKIE_NAME), Ok(None));
        assert_eq!(read_cookie(None, ADMIN_COOKIE_NAME), Ok(None));
        assert_eq!(
            read_cookie(Some("mysticparadox_admin=%E0%A4%A"), ADMIN_COOKIE_NAME),
            Err(())
        );
    }
}
