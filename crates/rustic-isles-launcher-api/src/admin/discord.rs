// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Admin Discord sign-in (admin spec §5): an allow-listed, PKCE, browser-bound OAuth flow answered with 302s.

use axum::response::Response;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use dainty_module_api::{Actor, ActorContext};
use http::{HeaderValue, StatusCode};
use rustic_isles::UserId;
use rustic_isles_auth::DiscordConfig;
use rustic_isles_store::repo::auth::{
    ApprovalStatus, AuthIdentityRecord, CreateOutcome, DiscordOAuthTransactionRecord, LauncherAccountRecord,
    OperationalStatus,
};
use rustic_isles_store::repo::reforged::AdminAuditRecord;
use rustic_isles_store::{AccountId, RepoError};
use serde_json::Value;

use super::auth::read_cookie;
use super::signin::{open_session, secure_suffix, session_cookie};
use super::{AdminContext, LauncherApiAdmin, admin_error, fresh_id, user_id_of};
use crate::launcher_api::is_account_eligible;
use crate::util::{self, form_encode, hmac_sha256, iso, now_ms, object, system_now_ms};

/// The cookie binding a Discord sign-in to the browser that started it.
pub const BINDING_COOKIE: &str = "rustic_isles_admin_oauth";
const SYSTEM_ACTOR: &str = "system:admin-discord-oauth";
const AUTHORIZE: &str = "https://discord.com/api/oauth2/authorize";

/// A setting read as a secure URL (§5.1): origin, path and query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecureUrl {
    /// Scheme, host and port.
    pub origin: String,
    /// The path.
    pub path: String,
    /// The query, without `?`; empty when none.
    pub query: String,
}

impl SecureUrl {
    /// The URL as text.
    pub fn written(&self) -> String {
        if self.query.is_empty() {
            format!("{}{}", self.origin, self.path)
        } else {
            format!("{}{}?{}", self.origin, self.path, self.query)
        }
    }
}

/// `{VAR}`'s text as a secure URL; the error is the code.
pub fn secure_url(var: &str, raw: Option<&str>, production: bool) -> Result<SecureUrl, String> {
    let text = util::trim(raw.unwrap_or(""));
    if text.is_empty() {
        return Err(format!("{var}_NOT_CONFIGURED"));
    }
    let invalid = || format!("{var}_INVALID");
    if text.contains('#') {
        return Err(invalid());
    }
    let Some((scheme, rest)) = text.split_once("://") else {
        return Err(invalid());
    };
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err(invalid());
    }
    let (before, query) = rest.split_once('?').unwrap_or((rest, ""));
    let (authority, path) = match before.find('/') {
        Some(at) => (&before[..at], &before[at..]),
        None => (before, "/"),
    };
    if authority.is_empty() || authority.contains('@') {
        return Err(invalid());
    }
    let host = if let Some(bracketed) = authority.strip_prefix('[') {
        let Some(end) = bracketed.find(']') else {
            return Err(invalid());
        };
        if end == 0 {
            return Err(invalid());
        }
        &authority[..end + 2]
    } else {
        authority.split(':').next().unwrap_or("")
    };
    if host.is_empty() {
        return Err(invalid());
    }
    let local = matches!(host.to_lowercase().as_str(), "localhost" | "127.0.0.1" | "[::1]");
    if scheme != "https" && !(!production && scheme == "http" && local) {
        return Err(format!("{var}_MUST_USE_HTTPS"));
    }
    Ok(SecureUrl {
        origin: format!("{scheme}://{}", authority.to_lowercase()),
        path: path.to_owned(),
        query: query.to_owned(),
    })
}

/// The admin Discord configuration; its `Debug` leaves the secret out.
#[derive(Clone)]
pub struct AdminDiscordConfig {
    /// The Discord application's id (`DISCORD_CLIENT_ID`).
    pub client_id: String,
    /// The Discord application's secret (`DISCORD_CLIENT_SECRET`).
    pub client_secret: String,
    /// Where Discord sends the browser back (`ADMIN_DISCORD_REDIRECT_URI`).
    pub redirect_uri: String,
    /// The dashboard page the flow returns to (`ADMIN_FRONTEND_URL`).
    pub frontend: String,
    /// Discord user ids allowed to sign in (`ADMIN_DISCORD_ALLOWED_IDS`).
    pub allowed: Vec<String>,
}

impl std::fmt::Debug for AdminDiscordConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdminDiscordConfig")
            .field("client_id", &self.client_id)
            .field("redirect_uri", &self.redirect_uri)
            .field("frontend", &self.frontend)
            .field("allowed", &self.allowed)
            .finish_non_exhaustive()
    }
}

fn is_snowflake(text: &str) -> bool {
    (17..=20).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit())
}

/// `ADMIN_DISCORD_ALLOWED_IDS`, de-duplicated in order.
pub fn allowed_ids(raw: Option<&str>) -> Result<Vec<String>, String> {
    let text = raw.unwrap_or("").trim();
    if text.is_empty() {
        return Err("ADMIN_DISCORD_ALLOWED_IDS_NOT_CONFIGURED".into());
    }
    let mut out: Vec<String> = Vec::new();
    for id in text.split(',').map(str::trim).filter(|i| !i.is_empty()) {
        if !is_snowflake(id) {
            return Err("ADMIN_DISCORD_ALLOWED_IDS_INVALID".into());
        }
        if !out.iter().any(|o| o == id) {
            out.push(id.to_owned());
        }
    }
    if out.is_empty() {
        return Err("ADMIN_DISCORD_ALLOWED_IDS_INVALID".into());
    }
    Ok(out)
}

/// Reads the configuration; the error is the code the dashboard is sent.
pub fn read_config(settings: &rustic_isles::Settings) -> Result<AdminDiscordConfig, String> {
    let production = settings.raw("NODE_ENV") == Some("production");
    let client_id = settings
        .get("DISCORD_CLIENT_ID")
        .ok_or("ADMIN_DISCORD_NOT_CONFIGURED")?;
    let client_secret = settings
        .get("DISCORD_CLIENT_SECRET")
        .ok_or("ADMIN_DISCORD_NOT_CONFIGURED")?;
    let redirect = secure_url(
        "ADMIN_DISCORD_REDIRECT_URI",
        settings.raw("ADMIN_DISCORD_REDIRECT_URI"),
        production,
    )?;
    if !redirect.query.is_empty() || util::trim(settings.raw("ADMIN_DISCORD_REDIRECT_URI").unwrap_or("")).contains('?')
    {
        return Err("ADMIN_DISCORD_REDIRECT_URI_INVALID".into());
    }
    let raw_frontend = settings.raw("ADMIN_FRONTEND_URL");
    let mut frontend = secure_url("ADMIN_FRONTEND_URL", raw_frontend, production)?;
    if util::trim(raw_frontend.unwrap_or("")).contains('?') {
        return Err("ADMIN_FRONTEND_URL_INVALID".into());
    }
    if !frontend.path.ends_with('/') {
        frontend.path.push('/');
    }
    if redirect.origin != frontend.origin {
        return Err("ADMIN_DISCORD_REDIRECT_URI_INVALID".into());
    }
    let allowed = allowed_ids(settings.raw("ADMIN_DISCORD_ALLOWED_IDS"))?;
    Ok(AdminDiscordConfig {
        client_id: client_id.to_owned(),
        client_secret: client_secret.to_owned(),
        redirect_uri: redirect.written(),
        frontend: frontend.written(),
        allowed,
    })
}

/// `v1.<nonce>.<tag>`, the tag the binding's HMAC over `v1`, NUL, nonce.
pub fn make_state(binding: &[u8], nonce: &str) -> String {
    let mut message = b"v1\0".to_vec();
    message.extend_from_slice(nonce.as_bytes());
    format!("v1.{nonce}.{}", hex::encode(hmac_sha256(binding, &message)))
}

fn is_lower_hex(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Whether the state was made for this browser binding.
pub fn verify_state(binding: Option<&str>, state: &str) -> bool {
    let Some(binding) = binding else {
        return false;
    };
    let shape = binding.len() == 43
        && binding
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    let Ok(bytes) = URL_SAFE_NO_PAD.decode(binding) else {
        return false;
    };
    if !shape || bytes.len() != 32 {
        return false;
    }
    let mut parts = state.splitn(3, '.');
    let (Some("v1"), Some(nonce), Some(tag)) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    if !is_lower_hex(nonce, 48) || !is_lower_hex(tag, 64) {
        return false;
    }
    util::ct_eq(make_state(&bytes, nonce).as_bytes(), state.as_bytes())
}

/// `A` and the Discord id in base 36.
pub fn bootstrap_name(id: &str) -> String {
    let mut n: u128 = id.parse().unwrap_or(0);
    let mut digits = Vec::new();
    loop {
        let d = (n % 36) as u8;
        digits.push(if d < 10 { b'0' + d } else { b'a' + d - 10 });
        n /= 36;
        if n == 0 {
            break;
        }
    }
    digits.reverse();
    format!("A{}", String::from_utf8_lossy(&digits))
}

fn unavailable() -> Response {
    admin_error(
        StatusCode::SERVICE_UNAVAILABLE,
        "ADMIN_DISCORD_NOT_CONFIGURED",
        "Discord sign-in isn't configured safely on this server yet.",
    )
}

fn redirect(location: &str, cookies: &[String]) -> Response {
    let mut response = util::empty(StatusCode::FOUND);
    util::set_header(&mut response, "location", location);
    for cookie in cookies {
        if let Ok(value) = HeaderValue::from_str(cookie) {
            response.headers_mut().append("set-cookie", value);
        }
    }
    response
}

fn with_error(frontend: &str, code: &str) -> String {
    format!("{frontend}?error={}", form_encode(code))
}

fn burn(settings: &rustic_isles::Settings) -> String {
    format!(
        "{BINDING_COOKIE}=; Path=/admin/v1/auth/discord; HttpOnly; SameSite=Lax; Max-Age=0{}",
        secure_suffix(settings)
    )
}

/// `GET /admin/v1/auth/discord/start`: redirects to Discord with a fresh PKCE challenge and binding cookie.
pub async fn start(ctx: AdminContext) -> Response {
    let settings = ctx.shared.settings();
    let config = match read_config(settings) {
        Ok(config) => config,
        Err(code) => {
            tracing::error!("[ADMIN] Discord auth start failed: {code}");
            return unavailable();
        }
    };
    let key = format!("admin-discord-start:{}", ctx.request.client_ip);
    if ctx
        .shared
        .auth()
        .limiter()
        .is_limited_at(&key, 12, 15 * 60_000, system_now_ms())
    {
        return redirect(&with_error(&config.frontend, "rate_limited"), &[]);
    }
    let binding_bytes = util::random_bytes::<32>();
    let binding = URL_SAFE_NO_PAD.encode(binding_bytes);
    let state = make_state(&binding_bytes, &util::random_hex::<24>());
    let verifier = util::random_base64url::<32>();
    let challenge = URL_SAFE_NO_PAD.encode(util::sha256(verifier.as_bytes()));
    let now = now_ms();
    let record = DiscordOAuthTransactionRecord {
        state: state.clone(),
        code_verifier: verifier,
        created_at: iso(now),
        expires_at: iso(now + 5 * 60_000),
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
        return unavailable();
    }
    let params = [
        ("client_id", config.client_id.as_str()),
        ("redirect_uri", config.redirect_uri.as_str()),
        ("response_type", "code"),
        ("scope", "identify"),
        ("state", state.as_str()),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
    ];
    let query: Vec<String> = params
        .iter()
        .map(|(k, v)| format!("{}={}", form_encode(k), form_encode(v)))
        .collect();
    let cookie = format!(
        "{BINDING_COOKIE}={binding}; Path=/admin/v1/auth/discord; HttpOnly; SameSite=Lax; Max-Age=300{}",
        secure_suffix(settings)
    );
    redirect(&format!("{AUTHORIZE}?{}", query.join("&")), &[cookie])
}

enum Resolved {
    Admitted(Box<LauncherAccountRecord>),
    Ineligible,
}

const LINKED_MISSING: &str = "ADMIN_DISCORD_LINKED_ACCOUNT_MISSING";

/// `GET /admin/v1/auth/discord/callback`: finishes the sign-in and redirects to the dashboard.
pub async fn callback(ctx: AdminContext) -> Response {
    let settings = ctx.shared.settings();
    let burned = burn(settings);
    let config = match read_config(settings) {
        Ok(config) => config,
        Err(code) => {
            tracing::error!("[ADMIN] Discord auth callback configuration failed: {code}");
            let mut response = unavailable();
            if let Ok(value) = HeaderValue::from_str(&burned) {
                response.headers_mut().append("set-cookie", value);
            }
            return response;
        }
    };
    let fail = |code: &str| redirect(&with_error(&config.frontend, code), std::slice::from_ref(&burned));
    let key = format!("admin-discord-callback:{}", ctx.request.client_ip);
    if ctx
        .shared
        .auth()
        .limiter()
        .is_limited_at(&key, 10, 15 * 60_000, system_now_ms())
    {
        return fail("rate_limited");
    }
    let (Some(code), Some(state)) = (ctx.request.single("code"), ctx.request.single("state")) else {
        return fail("discord_cancelled");
    };
    let (code, state) = (code.to_owned(), state.to_owned());
    let consume = state.clone();
    let now_iso = iso(now_ms());
    let consumed = ctx
        .store
        .write(&ActorContext::new(Actor::System), move |tx, repos| {
            repos
                .discord_oauth_transactions
                .consume_by_state(tx, &consume, &now_iso)
        })
        .await;
    let transaction = match consumed {
        Ok(Some(t)) => t,
        Ok(None) => return fail("discord_expired"),
        Err(e) => {
            tracing::error!("[ADMIN] Discord auth completion failed: {e}");
            return fail("discord_failed");
        }
    };
    let binding = read_cookie(&ctx.request.headers, BINDING_COOKIE).ok().flatten();
    if !verify_state(binding.as_deref(), &state) {
        tracing::warn!(
            "[ADMIN] rejected Discord OAuth state without its browser binding ip={}",
            ctx.request.client_ip
        );
        return fail("discord_state_mismatch");
    }
    let discord_config = DiscordConfig {
        client_id: config.client_id.clone(),
        client_secret: config.client_secret.clone(),
        redirect_uri: config.redirect_uri.clone(),
    };
    let admin = LauncherApiAdmin::of(&ctx.shared);
    let user = match admin
        .discord()
        .exchange_code(&discord_config, &code, &transaction.code_verifier)
        .await
    {
        Ok(user) if is_snowflake(&user.id) => user,
        _ => return fail("discord_cancelled"),
    };
    if !config.allowed.contains(&user.id) {
        tracing::warn!("[ADMIN] rejected Discord admin login from disallowed id {}", user.id);
        return fail("discord_not_allowed");
    }
    let ip = ctx.request.client_ip.clone();
    let node = ctx.shared.node().node.clone();
    let discord_user = user.clone();
    let resolved = ctx
        .store
        .write(&ActorContext::new(Actor::System), move |tx, repos| {
            let now = iso(now_ms());
            let audit = |target: String, action: &str, old: Option<Value>, new: Value, reason: &str| AdminAuditRecord {
                id: fresh_id(),
                actor_user_id: SYSTEM_ACTOR.to_owned(),
                target_user_id: Some(target),
                action: action.to_owned(),
                old_state: old,
                new_state: Some(new),
                reason: Some(reason.to_owned()),
                ip: ip.clone(),
                request_id: fresh_id(),
                created_at: now.clone(),
            };
            if let Some(identity) = repos
                .auth_identities
                .find_by_provider_subject(tx.conn(), &discord_user.id)?
            {
                let Some(account) = repos.launcher_accounts.find_by_account(tx.conn(), &identity.account)? else {
                    return Err(RepoError::Data(LINKED_MISSING.into()));
                };
                if !is_account_eligible(&account) {
                    return Ok(Resolved::Ineligible);
                }
                if account.roles.iter().any(|r| r == "admin") {
                    return Ok(Resolved::Admitted(Box::new(account)));
                }
                let mut roles = account.roles.clone();
                roles.push("admin".into());
                let Some(updated) =
                    repos
                        .launcher_accounts
                        .update_roles(tx, &account.account, &roles, &now, SYSTEM_ACTOR)?
                else {
                    return Err(RepoError::Data("ADMIN_DISCORD_ACCOUNT_DISAPPEARED".into()));
                };
                let user_id = user_id_of(tx.conn(), repos, &account.account);
                let as_list = |r: &[String]| Value::Array(r.iter().cloned().map(Value::from).collect());
                repos.admin.append_audit(
                    tx,
                    &audit(
                        user_id.clone(),
                        "admin.discord.promote",
                        Some(object(vec![("roles", as_list(&account.roles))])),
                        object(vec![("roles", as_list(&updated.roles))]),
                        "Allow-listed Discord identity promoted an eligible linked account.",
                    ),
                )?;
                tracing::info!("[ADMIN] Eligible linked account promoted via Discord: {user_id}");
                return Ok(Resolved::Admitted(Box::new(updated)));
            }
            let name = bootstrap_name(&discord_user.id);
            let account_id = AccountId::new(&node);
            let user_id = UserId::random();
            let record = LauncherAccountRecord {
                account: account_id.clone(),
                user_id: user_id.clone(),
                email: None,
                display_name_normalized: name.to_lowercase(),
                display_name: name,
                password_hash: None,
                status: OperationalStatus::Active,
                approval_status: Some(ApprovalStatus::Approved),
                approval_updated_at: Some(now.clone()),
                approval_updated_by: Some(SYSTEM_ACTOR.to_owned()),
                approval_reason: None,
                roles: vec!["admin".into()],
                roles_updated_at: Some(now.clone()),
                roles_updated_by: Some(SYSTEM_ACTOR.to_owned()),
                created_at: now.clone(),
                last_login_at: None,
                username_set: Some(true),
                consent: None,
                deletion: None,
                email_verified_at: None,
                email_verification_pending_since: None,
                timeout: None,
            };
            if repos.launcher_accounts.create(tx, &record)? != CreateOutcome::Created {
                return Err(RepoError::Data("ADMIN_DISCORD_BOOTSTRAP_CONFLICT".into()));
            }
            let identity = AuthIdentityRecord {
                provider_subject: discord_user.id.clone(),
                account: account_id.clone(),
                provider_username: discord_user.username.clone(),
                provider_avatar_url: discord_user
                    .avatar
                    .as_ref()
                    .map(|a| format!("https://cdn.discordapp.com/avatars/{}/{a}.png", discord_user.id)),
                linked_at: now.clone(),
            };
            if !repos.auth_identities.create(tx, &identity)? {
                return Err(RepoError::Data("ADMIN_DISCORD_IDENTITY_CONFLICT".into()));
            }
            repos.admin.append_audit(
                tx,
                &audit(
                    user_id.as_str().to_owned(),
                    "admin.discord.bootstrap",
                    None,
                    object(vec![
                        ("status", Value::from("active")),
                        ("approvalStatus", Value::from("approved")),
                        ("roles", Value::Array(vec![Value::from("admin")])),
                    ]),
                    "Allow-listed Discord identity bootstrapped the administrator account.",
                ),
            )?;
            tracing::info!("[ADMIN] Administrator account bootstrapped via Discord: {user_id}");
            let created = repos.launcher_accounts.find_by_account(tx.conn(), &account_id)?;
            Ok(Resolved::Admitted(Box::new(created.unwrap_or(record))))
        })
        .await;
    let account = match resolved {
        Ok(Resolved::Admitted(account)) => *account,
        Ok(Resolved::Ineligible) => {
            tracing::warn!(
                "[ADMIN] rejected Discord admin login for ineligible linked account id {}",
                user.id
            );
            return fail("discord_account_ineligible");
        }
        Err(e) => {
            tracing::error!("[ADMIN] Discord auth completion failed: {e}");
            return fail("discord_failed");
        }
    };
    if !account.roles.iter().any(|r| r == "admin") || !is_account_eligible(&account) {
        return fail("discord_not_allowed");
    }
    let lookup = account.account.clone();
    let user_id = ctx
        .store
        .read(move |conn, repos| Ok(user_id_of(conn, repos, &lookup)))
        .await
        .unwrap_or_else(|_| account.account.to_string());
    let agent = ctx
        .request
        .header("user-agent")
        .unwrap_or_else(|| "discord-oauth".to_owned());
    let (raw, _) = match open_session(&ctx.store, &account, &user_id, &ctx.request.client_ip, &agent).await {
        Ok(opened) => opened,
        Err(e) => {
            tracing::error!("[ADMIN] Discord auth completion failed: {e}");
            return fail("discord_failed");
        }
    };
    redirect(&config.frontend, &[burned.clone(), session_cookie(&raw, settings)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_lists_and_names() {
        assert!(allowed_ids(None).is_err() && allowed_ids(Some("")).is_err());
        assert!(allowed_ids(Some("not-a-snowflake")).is_err());
        assert!(allowed_ids(Some("100000000000000001,123")).is_err());
        assert_eq!(
            allowed_ids(Some(" 100000000000000001, 100000000000000001,123456789012345678 ")).unwrap(),
            ["100000000000000001", "123456789012345678"]
        );
        let name = bootstrap_name("100000000000000001");
        assert_eq!(name, "Arcn1hssight");
        assert!((3..=16).contains(&name.len()) && name.bytes().all(|b| b.is_ascii_alphanumeric()));
    }

    #[test]
    fn states() {
        let binding = [0x11u8; 32];
        let nonce = "a".repeat(48);
        let state = make_state(&binding, &nonce);
        assert_eq!(state, make_state(&binding, &nonce));
        let text = URL_SAFE_NO_PAD.encode(binding);
        assert!(verify_state(Some(&text), &state));
        assert!(!verify_state(Some(&URL_SAFE_NO_PAD.encode([0x22u8; 32])), &state));
        assert!(!verify_state(None, &state));
        assert!(!verify_state(Some(&text), &format!("{state}x")));
    }

    #[test]
    fn secure_urls() {
        let ok = secure_url("X", Some("http://localhost:4173/admin"), false).unwrap();
        assert_eq!(ok.written(), "http://localhost:4173/admin");
        assert_eq!(secure_url("X", Some(""), false).unwrap_err(), "X_NOT_CONFIGURED");
        assert_eq!(secure_url("X", Some("https://a@b/"), false).unwrap_err(), "X_INVALID");
        assert_eq!(secure_url("X", Some("https://b/#x"), false).unwrap_err(), "X_INVALID");
        assert_eq!(secure_url("X", Some("ftp://b/"), false).unwrap_err(), "X_INVALID");
        assert_eq!(secure_url("X", Some("https://[::1/"), false).unwrap_err(), "X_INVALID");
        assert_eq!(
            secure_url("X", Some("http://example.com/"), false).unwrap_err(),
            "X_MUST_USE_HTTPS"
        );
        assert_eq!(
            secure_url("X", Some("http://localhost/"), true).unwrap_err(),
            "X_MUST_USE_HTTPS"
        );
        assert_eq!(
            secure_url("X", Some("HTTPS://Example.COM:443/a?b=c"), true)
                .unwrap()
                .written(),
            "https://example.com:443/a?b=c"
        );
    }
}
