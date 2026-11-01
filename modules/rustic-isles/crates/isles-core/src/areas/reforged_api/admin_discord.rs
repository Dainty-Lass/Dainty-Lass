//! Admin Discord OAuth (`controllers/adminDiscordAuth.ts`): browser-bound OAuth start and callback for the admin
//! dashboard (`/admin/v1/auth/discord/start` and `/admin/v1/auth/discord/callback`).

use std::collections::HashSet;

use axum::http::header::{HeaderName, HeaderValue, LOCATION, SET_COOKIE};
use axum::response::Response;
use base64::Engine as _;
use hmac::{Hmac, KeyInit, Mac};
use module_api::{Actor, ActorContext};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use super::admin::{ADMIN_COOKIE_NAME, Ctx, error, route};
use super::launcher_api::is_account_eligible;
use super::util::{self, hex, is_rate_limited, random_bytes, sha256_hex};
use crate::areas::auth::discord::{DiscordConfig, DiscordUser};
use crate::http::{Handlers, Shared};
use crate::ids::{AccountId, UserId};
use crate::repo::auth::{
    ApprovalStatus, AuthIdentityRecord, CreateOutcome, DiscordOAuthTransactionRecord, LauncherAccountRecord,
    OperationalStatus,
};
use crate::repo::reforged::{AdminAuditRecord, AdminSessionRecord};
use crate::routes::{Method, Service};
use crate::settings::Settings;

type HmacSha256 = Hmac<Sha256>;

const STATE_TTL_MS: i64 = 5 * 60 * 1000;
const ADMIN_SESSION_TTL_MS: i64 = 8 * 60 * 60 * 1000;
const OAUTH_RATE_WINDOW_MS: i64 = 15 * 60 * 1000;
const DISCORD_STATE_NONCE_BYTES: usize = 24;
const DISCORD_BROWSER_BINDING_BYTES: usize = 32;
pub const ADMIN_DISCORD_BINDING_COOKIE_NAME: &str = "rustic_isles_admin_oauth";
/// An OAuth flow begun before an update's restart (crate::legacy)
const LEGACY_ADMIN_DISCORD_BINDING_COOKIE_NAME: &str = "mysticparadox_admin_oauth";
const ADMIN_OAUTH_ACTOR: &str = "system:admin-discord-oauth";

#[derive(Debug, Clone)]
pub struct AdminDiscordConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub allowed_ids: HashSet<String>,
}

fn is_discord_id(text: &str) -> bool {
    (17..=20).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit())
}

pub fn parse_admin_discord_allowed_ids(raw: Option<&str>) -> Result<Vec<String>, &'static str> {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Err("ADMIN_DISCORD_ALLOWED_IDS_NOT_CONFIGURED");
    };
    let mut ids = Vec::new();
    for value in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if !is_discord_id(value) {
            return Err("ADMIN_DISCORD_ALLOWED_IDS_INVALID");
        }
        if !ids.iter().any(|existing| existing == value) {
            ids.push(value.to_owned());
        }
    }
    if ids.is_empty() {
        return Err("ADMIN_DISCORD_ALLOWED_IDS_INVALID");
    }
    Ok(ids)
}

pub fn admin_bootstrap_display_name(discord_id: &str) -> Result<String, &'static str> {
    if !is_discord_id(discord_id) {
        return Err("ADMIN_DISCORD_ID_INVALID");
    }
    let mut n: u128 = discord_id.parse().map_err(|_| "ADMIN_DISCORD_ID_INVALID")?;
    if n == 0 {
        return Ok("A0".to_owned());
    }
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut chars = Vec::new();
    while n > 0 {
        chars.push(DIGITS[(n % 36) as usize] as char);
        n /= 36;
    }
    chars.reverse();
    let mut out = String::with_capacity(1 + chars.len());
    out.push('A');
    for ch in chars {
        out.push(ch);
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedUrl {
    scheme: String,
    origin: String,
    host: String,
    path: String,
    query: Option<String>,
}

impl ParsedUrl {
    fn to_url_string(&self) -> String {
        match &self.query {
            Some(q) if !q.is_empty() => format!("{}{}?{q}", self.origin, self.path),
            _ => format!("{}{}", self.origin, self.path),
        }
    }
}

fn require_secure_url(raw: Option<&str>, settings: &Settings, variable_name: &str) -> Result<ParsedUrl, String> {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Err(format!("{variable_name}_NOT_CONFIGURED"));
    };
    if raw.contains('#') {
        return Err(format!("{variable_name}_INVALID"));
    }
    let (scheme, rest) = raw
        .split_once("://")
        .ok_or_else(|| format!("{variable_name}_INVALID"))?;
    let scheme_lower = scheme.to_ascii_lowercase();
    if !matches!(scheme_lower.as_str(), "http" | "https") {
        return Err(format!("{variable_name}_INVALID"));
    }
    let (authority_and_path, query) = match rest.split_once('?') {
        Some((before, q)) => (before, Some(q.to_owned())),
        None => (rest, None),
    };
    let (authority, path_raw) = match authority_and_path.find('/') {
        Some(idx) => (&authority_and_path[..idx], &authority_and_path[idx..]),
        None => (authority_and_path, "/"),
    };
    if authority.is_empty() || authority.contains('@') {
        return Err(format!("{variable_name}_INVALID"));
    }
    let host = if authority.starts_with('[') {
        let end = authority.find(']').ok_or_else(|| format!("{variable_name}_INVALID"))?;
        &authority[..=end]
    } else {
        authority.split(':').next().unwrap_or(authority)
    };
    if host.is_empty() {
        return Err(format!("{variable_name}_INVALID"));
    }
    let is_prod = settings.raw("NODE_ENV") == Some("production");
    let is_local_dev = !is_prod
        && scheme_lower == "http"
        && matches!(host.to_ascii_lowercase().as_str(), "localhost" | "127.0.0.1" | "[::1]");
    if scheme_lower != "https" && !is_local_dev {
        return Err(format!("{variable_name}_MUST_USE_HTTPS"));
    }
    let origin = format!("{scheme_lower}://{}", authority.to_ascii_lowercase());
    Ok(ParsedUrl {
        scheme: format!("{scheme_lower}:"),
        origin,
        host: host.to_ascii_lowercase(),
        path: path_raw.to_owned(),
        query,
    })
}

fn admin_frontend_url(settings: &Settings) -> Result<ParsedUrl, String> {
    let mut url = require_secure_url(settings.raw("ADMIN_FRONTEND_URL"), settings, "ADMIN_FRONTEND_URL")?;
    if url.query.is_some() {
        return Err("ADMIN_FRONTEND_URL_INVALID".to_owned());
    }
    if !url.path.ends_with('/') {
        url.path.push('/');
    }
    Ok(url)
}

pub fn build_admin_frontend_redirect(settings: &Settings, error_code: Option<&str>) -> Result<String, String> {
    let mut url = admin_frontend_url(settings)?;
    if let Some(code) = error_code.filter(|s| !s.is_empty()) {
        url.query = Some(format!("error={}", form_component(code)));
    }
    Ok(url.to_url_string())
}

fn require_discord_config(settings: &Settings) -> Result<AdminDiscordConfig, String> {
    let client_id = settings
        .get("DISCORD_CLIENT_ID")
        .ok_or_else(|| "ADMIN_DISCORD_NOT_CONFIGURED".to_owned())?
        .to_owned();
    let client_secret = settings
        .get("DISCORD_CLIENT_SECRET")
        .ok_or_else(|| "ADMIN_DISCORD_NOT_CONFIGURED".to_owned())?
        .to_owned();
    let redirect = require_secure_url(
        settings.raw("ADMIN_DISCORD_REDIRECT_URI"),
        settings,
        "ADMIN_DISCORD_REDIRECT_URI",
    )?;
    let frontend = admin_frontend_url(settings)?;
    if redirect.query.is_some() || redirect.origin != frontend.origin {
        return Err("ADMIN_DISCORD_REDIRECT_URI_INVALID".to_owned());
    }
    let allowed = parse_admin_discord_allowed_ids(settings.raw("ADMIN_DISCORD_ALLOWED_IDS"))
        .map_err(str::to_owned)?
        .into_iter()
        .collect();
    Ok(AdminDiscordConfig {
        client_id,
        client_secret,
        redirect_uri: redirect.to_url_string(),
        allowed_ids: allowed,
    })
}

fn is_valid_browser_binding(binding: &str) -> bool {
    binding.len() == 43
        && binding
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

pub fn create_bound_admin_discord_state(
    browser_binding: &str,
    nonce_hex: Option<&str>,
) -> Result<String, &'static str> {
    if !is_valid_browser_binding(browser_binding) {
        return Err("ADMIN_DISCORD_BROWSER_BINDING_INVALID");
    }
    let owned_nonce;
    let nonce = match nonce_hex {
        Some(n) => n,
        None => {
            owned_nonce = hex(&random_bytes(DISCORD_STATE_NONCE_BYTES));
            &owned_nonce
        }
    };
    if nonce.len() != 48 || !nonce.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return Err("ADMIN_DISCORD_STATE_NONCE_INVALID");
    }
    let key = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(browser_binding)
        .map_err(|_| "ADMIN_DISCORD_BROWSER_BINDING_INVALID")?;
    if key.len() != DISCORD_BROWSER_BINDING_BYTES {
        return Err("ADMIN_DISCORD_BROWSER_BINDING_INVALID");
    }
    let mut mac = HmacSha256::new_from_slice(&key).map_err(|_| "ADMIN_DISCORD_BROWSER_BINDING_INVALID")?;
    mac.update(b"v1\0");
    mac.update(nonce.as_bytes());
    let tag = hex(&mac.finalize().into_bytes());
    Ok(format!("v1.{nonce}.{tag}"))
}

pub fn verify_bound_admin_discord_state(state: &str, browser_binding: Option<&str>) -> bool {
    let Some(binding) = browser_binding.filter(|b| is_valid_browser_binding(b)) else {
        return false;
    };
    let Some(rest) = state.strip_prefix("v1.") else {
        return false;
    };
    let Some((nonce, actual_mac)) = rest.split_once('.') else {
        return false;
    };
    if nonce.len() != 48
        || actual_mac.len() != 64
        || !nonce.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        || !actual_mac.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        return false;
    }
    let Ok(expected_state) = create_bound_admin_discord_state(binding, Some(nonce)) else {
        return false;
    };
    let Some(expected_mac) = expected_state.split('.').nth(2) else {
        return false;
    };
    bool::from(actual_mac.as_bytes().ct_eq(expected_mac.as_bytes()))
}

fn cookie_security_suffix(settings: &Settings) -> &'static str {
    let secure = settings.raw("NODE_ENV") == Some("production")
        || settings
            .raw("ADMIN_COOKIE_SECURE")
            .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    if secure { "; Secure" } else { "" }
}

pub fn admin_discord_binding_cookie(settings: &Settings, value: Option<&str>) -> String {
    let encoded = match value {
        Some(v) => percent_encoding::utf8_percent_encode(v, super::updates::COMPONENT).to_string(),
        None => String::new(),
    };
    let max_age = if value.is_some() { STATE_TTL_MS / 1000 } else { 0 };
    format!(
        "{ADMIN_DISCORD_BINDING_COOKIE_NAME}={encoded}; Path=/admin/v1/auth/discord; HttpOnly; SameSite=Lax; Max-Age={max_age}{}",
        cookie_security_suffix(settings)
    )
}

fn admin_cookie_options(settings: &Settings, max_age_seconds: i64) -> String {
    format!(
        "Path=/admin/v1; HttpOnly; SameSite=Strict; Max-Age={max_age_seconds}{}",
        cookie_security_suffix(settings)
    )
}

fn form_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => out.push(byte as char),
            b' ' => out.push('+'),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn read_cookie(header: Option<&str>, name: &str) -> Option<String> {
    let header = header?;
    for part in header.split(';') {
        let part = part.trim();
        let (key, value) = part.split_once('=').unwrap_or((part, ""));
        if key == name {
            return service_kit::js::decode_uri_component(value);
        }
    }
    None
}

fn redirect_response(location: &str, cookies: &[String]) -> Response {
    let mut response = util::empty(302);
    if let Ok(loc) = HeaderValue::from_str(location) {
        response.headers_mut().insert(LOCATION, loc);
    }
    for cookie in cookies {
        if let Ok(val) = HeaderValue::from_str(cookie) {
            response.headers_mut().append(SET_COOKIE, val);
        }
    }
    response
}

async fn start_handler(ctx: Ctx) -> Response {
    let config = match require_discord_config(&ctx.shared.settings) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("[ADMIN] Discord auth start failed: {e}");
            return error(
                503,
                "ADMIN_DISCORD_NOT_CONFIGURED",
                "Discord sign-in isn't configured safely on this server yet.",
            );
        }
    };
    let ip = ctx.req.ip();
    if is_rate_limited(&format!("admin-discord-start:{ip}"), 12, OAUTH_RATE_WINDOW_MS) {
        let Ok(url) = build_admin_frontend_redirect(&ctx.shared.settings, Some("rate_limited")) else {
            return error(
                503,
                "ADMIN_DISCORD_NOT_CONFIGURED",
                "Discord sign-in isn't configured safely on this server yet.",
            );
        };
        return redirect_response(&url, &[]);
    }
    let browser_binding =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random_bytes(DISCORD_BROWSER_BINDING_BYTES));
    let Ok(state) = create_bound_admin_discord_state(&browser_binding, None) else {
        return error(
            503,
            "ADMIN_DISCORD_NOT_CONFIGURED",
            "Discord sign-in isn't configured safely on this server yet.",
        );
    };
    let code_verifier = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random_bytes(32));
    let code_challenge =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(code_verifier.as_bytes()));
    let now = util::now_ms();
    let record = DiscordOAuthTransactionRecord {
        state: state.clone(),
        code_verifier,
        created_at: util::iso(now),
        expires_at: util::iso(now + STATE_TTL_MS),
        consumed_at: None,
        completion_code_hash: None,
        completion_expires_at: None,
        completion_consumed_at: None,
        account: None,
        link_account: None,
    };
    if let Err(e) = ctx
        .store
        .write(&ActorContext::new(Actor::System), move |tx, repos| {
            repos.discord_oauth_transactions.create(tx, &record)
        })
        .await
    {
        tracing::error!("[ADMIN] Discord auth start failed: {e}");
        return error(
            503,
            "ADMIN_DISCORD_NOT_CONFIGURED",
            "Discord sign-in isn't configured safely on this server yet.",
        );
    }
    let params = [
        ("client_id", config.client_id.as_str()),
        ("redirect_uri", config.redirect_uri.as_str()),
        ("response_type", "code"),
        ("scope", "identify"),
        ("state", state.as_str()),
        ("code_challenge", code_challenge.as_str()),
        ("code_challenge_method", "S256"),
    ]
    .iter()
    .map(|(k, v)| format!("{}={}", form_component(k), form_component(v)))
    .collect::<Vec<_>>()
    .join("&");
    let authorize_url = format!("https://discord.com/api/oauth2/authorize?{params}");
    let cookie = admin_discord_binding_cookie(&ctx.shared.settings, Some(&browser_binding));
    redirect_response(&authorize_url, &[cookie])
}

enum AdminResolution {
    Admitted(Box<LauncherAccountRecord>),
    Ineligible,
}

async fn resolve_admin_account(
    shared: &Shared,
    store: &crate::db::IslesStore,
    discord_user: &DiscordUser,
    ip: &str,
) -> Result<AdminResolution, String> {
    let user = discord_user.clone();
    let ip_owned = ip.to_owned();
    let node = shared.node.node.clone();
    store
        .write(&ActorContext::new(Actor::System), move |tx, repos| {
            let now = util::now_iso();
            if let Some(identity) = repos.auth_identities.find_by_provider_subject(tx.conn(), &user.id)? {
                let Some(account) = repos.launcher_accounts.find_by_account(tx.conn(), &identity.account)? else {
                    return Err(crate::repo::RepoError::Data(
                        "ADMIN_DISCORD_LINKED_ACCOUNT_MISSING".into(),
                    ));
                };
                if !is_account_eligible(&account) {
                    return Ok(AdminResolution::Ineligible);
                }
                if account.roles.iter().any(|r| r == "admin") {
                    return Ok(AdminResolution::Admitted(Box::new(account)));
                }
                let mut next_roles = account.roles.clone();
                next_roles.push("admin".to_owned());
                let Some(updated) =
                    repos
                        .launcher_accounts
                        .update_roles(tx, &account.account, &next_roles, &now, ADMIN_OAUTH_ACTOR)?
                else {
                    return Err(crate::repo::RepoError::Data("ADMIN_DISCORD_ACCOUNT_DISAPPEARED".into()));
                };
                repos.admin.append_audit(
                    tx,
                    &AdminAuditRecord {
                        id: uuid::Uuid::new_v4().to_string(),
                        actor_user_id: ADMIN_OAUTH_ACTOR.to_owned(),
                        target_user_id: Some(account.user_id.to_string()),
                        action: "admin.discord.promote".into(),
                        old_state: Some(serde_json::json!({ "roles": account.roles })),
                        new_state: Some(serde_json::json!({ "roles": updated.roles })),
                        reason: Some("Allow-listed Discord identity promoted an eligible linked account.".into()),
                        ip: ip_owned,
                        request_id: uuid::Uuid::new_v4().to_string(),
                        created_at: now,
                    },
                )?;
                tracing::info!(
                    "[ADMIN] Eligible linked account promoted via Discord: {}",
                    updated.user_id
                );
                return Ok(AdminResolution::Admitted(Box::new(updated)));
            }

            let display_name =
                admin_bootstrap_display_name(&user.id).map_err(|e| crate::repo::RepoError::Data(e.to_owned()))?;
            let user_id = UserId::random();
            let account_id = AccountId::new(&node);
            let account = LauncherAccountRecord {
                account: account_id.clone(),
                user_id: user_id.clone(),
                email: None,
                display_name_normalized: display_name.to_lowercase(),
                display_name,
                password_hash: None,
                status: OperationalStatus::Active,
                approval_status: Some(ApprovalStatus::Approved),
                approval_updated_at: Some(now.clone()),
                approval_updated_by: Some(ADMIN_OAUTH_ACTOR.to_owned()),
                approval_reason: None,
                roles: vec!["admin".to_owned()],
                roles_updated_at: Some(now.clone()),
                roles_updated_by: Some(ADMIN_OAUTH_ACTOR.to_owned()),
                created_at: now.clone(),
                last_login_at: None,
                username_set: Some(true),
                consent: None,
                deletion: None,
                email_verified_at: None,
                email_verification_pending_since: None,
                timeout: None,
            };
            match repos.launcher_accounts.create(tx, &account)? {
                CreateOutcome::Created => {}
                _ => {
                    return Err(crate::repo::RepoError::Data("ADMIN_DISCORD_BOOTSTRAP_CONFLICT".into()));
                }
            }
            let created_identity = repos.auth_identities.create(
                tx,
                &AuthIdentityRecord {
                    provider_subject: user.id.clone(),
                    account: account_id,
                    provider_username: user.username.clone(),
                    provider_avatar_url: user
                        .avatar
                        .as_ref()
                        .map(|av| format!("https://cdn.discordapp.com/avatars/{}/{av}.png", user.id)),
                    linked_at: now.clone(),
                },
            )?;
            if !created_identity {
                return Err(crate::repo::RepoError::Data("ADMIN_DISCORD_IDENTITY_CONFLICT".into()));
            }
            repos.admin.append_audit(
                tx,
                &AdminAuditRecord {
                    id: uuid::Uuid::new_v4().to_string(),
                    actor_user_id: ADMIN_OAUTH_ACTOR.to_owned(),
                    target_user_id: Some(user_id.to_string()),
                    action: "admin.discord.bootstrap".into(),
                    old_state: None,
                    new_state: Some(serde_json::json!({
                        "status": "active",
                        "approvalStatus": "approved",
                        "roles": account.roles,
                    })),
                    reason: Some("Allow-listed Discord identity bootstrapped the administrator account.".into()),
                    ip: ip_owned,
                    request_id: uuid::Uuid::new_v4().to_string(),
                    created_at: now,
                },
            )?;
            tracing::info!("[ADMIN] Administrator account bootstrapped via Discord: {user_id}");
            Ok(AdminResolution::Admitted(Box::new(account)))
        })
        .await
        .map_err(|e| e.to_string())
}

async fn callback_handler(ctx: Ctx) -> Response {
    let burn_cookie = admin_discord_binding_cookie(&ctx.shared.settings, None);
    let config = match require_discord_config(&ctx.shared.settings) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("[ADMIN] Discord auth callback configuration failed: {e}");
            let mut resp = error(
                503,
                "ADMIN_DISCORD_NOT_CONFIGURED",
                "Discord sign-in isn't configured safely on this server yet.",
            );
            if let Ok(val) = HeaderValue::from_str(&burn_cookie) {
                resp.headers_mut().insert(HeaderName::from_static("set-cookie"), val);
            }
            return resp;
        }
    };
    let redirect_with_err = |err: Option<&str>, extra_cookie: Option<String>| {
        let url = build_admin_frontend_redirect(&ctx.shared.settings, err).unwrap_or_else(|_| "/".to_owned());
        let mut cookies = vec![burn_cookie.clone()];
        if let Some(c) = extra_cookie {
            cookies.push(c);
        }
        redirect_response(&url, &cookies)
    };
    let ip = ctx.req.ip();
    if is_rate_limited(&format!("admin-discord-callback:{ip}"), 10, OAUTH_RATE_WINDOW_MS) {
        return redirect_with_err(Some("rate_limited"), None);
    }
    let code = ctx.req.query_string("code").map(str::to_owned);
    let state = ctx.req.query_string("state").map(str::to_owned);
    let (Some(code), Some(state)) = (code, state) else {
        return redirect_with_err(Some("discord_cancelled"), None);
    };
    let state_lookup = state.clone();
    let now_iso = util::now_iso();
    let consumed = ctx
        .store
        .write(&ActorContext::new(Actor::System), move |tx, repos| {
            repos
                .discord_oauth_transactions
                .consume_by_state(tx, &state_lookup, &now_iso)
        })
        .await;
    let transaction = match consumed {
        Ok(Some(t)) => t,
        Ok(None) => return redirect_with_err(Some("discord_expired"), None),
        Err(e) => {
            tracing::error!("[ADMIN] Discord auth completion failed: {e}");
            return redirect_with_err(Some("discord_failed"), None);
        }
    };
    let browser_binding = read_cookie(ctx.req.header("cookie"), ADMIN_DISCORD_BINDING_COOKIE_NAME)
        .or_else(|| read_cookie(ctx.req.header("cookie"), LEGACY_ADMIN_DISCORD_BINDING_COOKIE_NAME));
    if !verify_bound_admin_discord_state(&state, browser_binding.as_deref()) {
        tracing::warn!("[ADMIN] rejected Discord OAuth state without its browser binding ip={ip}");
        return redirect_with_err(Some("discord_state_mismatch"), None);
    }
    let discord_cfg = DiscordConfig {
        client_id: config.client_id.clone(),
        client_secret: config.client_secret.clone(),
        redirect_uri: config.redirect_uri.clone(),
    };
    let Ok(discord_user) = ctx
        .shared
        .auth
        .discord
        .exchange_code(&discord_cfg, &code, &transaction.code_verifier)
        .await
    else {
        return redirect_with_err(Some("discord_cancelled"), None);
    };
    if !is_discord_id(&discord_user.id) {
        return redirect_with_err(Some("discord_cancelled"), None);
    }
    if !config.allowed_ids.contains(&discord_user.id) {
        tracing::warn!(
            "[ADMIN] rejected Discord admin login from disallowed id {}",
            discord_user.id
        );
        return redirect_with_err(Some("discord_not_allowed"), None);
    }
    let resolution = match resolve_admin_account(&ctx.shared, &ctx.store, &discord_user, &ip).await {
        Ok(res) => res,
        Err(e) => {
            tracing::error!("[ADMIN] Discord auth completion failed: {e}");
            return redirect_with_err(Some("discord_failed"), None);
        }
    };
    let account = match resolution {
        AdminResolution::Ineligible => {
            tracing::warn!(
                "[ADMIN] rejected Discord admin login for ineligible linked account id {}",
                discord_user.id
            );
            return redirect_with_err(Some("discord_account_ineligible"), None);
        }
        AdminResolution::Admitted(account) => account,
    };
    if !account.roles.iter().any(|r| r == "admin") || !is_account_eligible(&account) {
        return redirect_with_err(Some("discord_not_allowed"), None);
    }
    let raw_token = hex(&random_bytes(32));
    let csrf_token = hex(&random_bytes(32));
    let now_ms = util::now_ms();
    let user_agent: String = ctx
        .req
        .header("user-agent")
        .unwrap_or("discord-oauth")
        .chars()
        .take(300)
        .collect();
    let session = AdminSessionRecord {
        id: uuid::Uuid::new_v4().to_string(),
        token_hash: sha256_hex(raw_token.as_bytes()),
        account: account.account.clone(),
        csrf_token,
        created_at: util::iso(now_ms),
        expires_at: util::iso(now_ms + ADMIN_SESSION_TTL_MS),
        ip,
        user_agent,
        revoked_at: None,
    };
    let actor = ActorContext::new(Actor::Admin {
        key: account.user_id.to_string(),
    });
    if let Err(e) = ctx
        .store
        .write(&actor, move |tx, repos| repos.admin.create_session(tx, &session))
        .await
    {
        tracing::error!("[ADMIN] Discord auth completion failed: {e}");
        return redirect_with_err(Some("discord_failed"), None);
    }
    let encoded_token = percent_encoding::utf8_percent_encode(&raw_token, super::updates::COMPONENT).to_string();
    let session_cookie = format!(
        "{ADMIN_COOKIE_NAME}={encoded_token}; {}",
        admin_cookie_options(&ctx.shared.settings, ADMIN_SESSION_TTL_MS / 1000)
    );
    redirect_with_err(None, Some(session_cookie))
}

pub fn register(handlers: &mut Handlers) {
    let b = Service::Backend;
    handlers.on(b, Method::Get, "/admin/v1/auth/discord/start", |call| {
        Box::pin(route(call, false, start_handler))
    });
    handlers.on(b, Method::Get, "/admin/v1/auth/discord/callback", |call| {
        Box::pin(route(call, false, callback_handler))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_list_fails_closed_and_deduplicates() {
        assert!(parse_admin_discord_allowed_ids(None).is_err());
        assert!(parse_admin_discord_allowed_ids(Some("")).is_err());
        assert!(parse_admin_discord_allowed_ids(Some("not-a-snowflake")).is_err());
        assert!(parse_admin_discord_allowed_ids(Some("100000000000000001,123")).is_err());
        assert_eq!(
            parse_admin_discord_allowed_ids(Some(" 100000000000000001, 100000000000000001,123456789012345678 "))
                .unwrap(),
            vec!["100000000000000001".to_owned(), "123456789012345678".to_owned()]
        );
    }

    #[test]
    fn bootstrap_display_name_is_deterministic_and_valid() {
        let name = admin_bootstrap_display_name("100000000000000001").unwrap();
        assert_eq!(name, "Arcn1hssight");
        assert!((3..=16).contains(&name.len()));
        assert!(name.bytes().all(|b| b.is_ascii_alphanumeric()));
        assert!(admin_bootstrap_display_name("invalid").is_err());
    }

    #[test]
    fn bound_state_verifies_and_rejects_tampering() {
        let b1 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([0x11u8; 32]);
        let b2 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([0x22u8; 32]);
        let nonce = "a".repeat(48);
        let state = create_bound_admin_discord_state(&b1, Some(&nonce)).unwrap();
        assert_eq!(state, create_bound_admin_discord_state(&b1, Some(&nonce)).unwrap());
        assert!(verify_bound_admin_discord_state(&state, Some(&b1)));
        assert!(!verify_bound_admin_discord_state(&state, Some(&b2)));
        assert!(!verify_bound_admin_discord_state(&state, None));
        assert!(!verify_bound_admin_discord_state(&format!("{state}x"), Some(&b1)));
    }
}
