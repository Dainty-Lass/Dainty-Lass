// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Sign-in, sign-out and `me` (admin spec §4).

use axum::response::Response;
use http::StatusCode;
use module_api::{Actor, ActorContext};
use rustic_isles_store::IslesStore;
use rustic_isles_store::repo::auth::LauncherAccountRecord;
use rustic_isles_store::repo::reforged::AdminSessionRecord;
use serde_json::Value;

use super::auth::SESSION_COOKIE;
use super::{AdminContext, Via, admin_actor, admin_error, admin_internal, caller_account, public_account};
use crate::launcher_api::is_account_eligible;
use crate::util::{self, iso, now_ms, object, system_now_ms};

const SESSION_MS: i64 = 8 * 3_600_000;

/// `; Secure` in production or with `ADMIN_COOKIE_SECURE` `1`/`true`.
pub fn secure_suffix(settings: &rustic_isles::Settings) -> &'static str {
    let forced = settings
        .raw("ADMIN_COOKIE_SECURE")
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    if settings.raw("NODE_ENV") == Some("production") || forced {
        "; Secure"
    } else {
        ""
    }
}

/// The `set-cookie` value for a new session: eight hours, `HttpOnly`, `SameSite=Strict`.
pub fn session_cookie(raw: &str, settings: &rustic_isles::Settings) -> String {
    format!(
        "{SESSION_COOKIE}={}; Path=/admin/v1; HttpOnly; SameSite=Strict; Max-Age=28800{}",
        util::encode_uri_component(raw),
        secure_suffix(settings)
    )
}

fn cleared_cookie(settings: &rustic_isles::Settings) -> String {
    format!(
        "{SESSION_COOKIE}=; Path=/admin/v1; HttpOnly; SameSite=Strict; Max-Age=0{}",
        secure_suffix(settings)
    )
}

/// A new session for the account: the raw token and the stored record (§4.1 step 6).
pub async fn open_session(
    store: &IslesStore,
    account: &LauncherAccountRecord,
    user_id: &str,
    ip: &str,
    user_agent: &str,
) -> Result<(String, AdminSessionRecord), rustic_isles_store::RepoError> {
    let raw = util::random_hex::<32>();
    let now = now_ms();
    let record = AdminSessionRecord {
        id: util::random_uuid(),
        token_hash: util::sha256_hex(raw.as_bytes()),
        account: account.account.clone(),
        csrf_token: util::random_hex::<32>(),
        created_at: iso(now),
        expires_at: iso(now + SESSION_MS),
        ip: ip.to_owned(),
        user_agent: util::take_chars(user_agent, 300),
        revoked_at: None,
    };
    let stored = record.clone();
    let actor = ActorContext::new(Actor::Admin {
        key: user_id.to_owned(),
    });
    store
        .write(&actor, move |tx, repos| repos.admin.create_session(tx, &stored))
        .await?;
    Ok((raw, record))
}

/// `POST /admin/v1/auth/login`: signs in with a password, rate limited per client.
pub async fn login(ctx: AdminContext) -> Response {
    let key = format!("admin-login:{}", ctx.request.client_ip);
    if ctx
        .shared
        .auth()
        .limiter()
        .is_limited_at(&key, 6, 15 * 60_000, system_now_ms())
    {
        return admin_error(
            StatusCode::TOO_MANY_REQUESTS,
            "ADMIN_RATE_LIMITED",
            "Too many sign-in attempts.",
        );
    }
    let email = ctx
        .request
        .field("email")
        .and_then(Value::as_str)
        .map(|e| util::trim(e).to_lowercase())
        .unwrap_or_default();
    let password = ctx
        .request
        .field("password")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let found = ctx
        .store
        .read(move |conn, repos| {
            let Some(account) = repos.launcher_accounts.find_by_email(conn, &email)? else {
                return Ok(None);
            };
            let user_id = super::user_id_of(conn, repos, &account.account);
            Ok(Some((account, user_id)))
        })
        .await;
    let found = match found {
        Ok(found) => found,
        Err(e) => return admin_internal(e),
    };
    let invalid = || {
        admin_error(
            StatusCode::UNAUTHORIZED,
            "ADMIN_INVALID_CREDENTIALS",
            "Administrator credentials are invalid.",
        )
    };
    let Some((account, user_id)) = found else {
        return invalid();
    };
    let verified = rustic_isles_auth::password::verify_password_blocking(account.password_hash.clone(), password).await;
    if !verified || !account.roles.iter().any(|r| r == "admin") || !is_account_eligible(&account) {
        return invalid();
    }
    let settings = ctx.shared.settings();
    match settings.get("ADMIN_TOTP_SECRET") {
        None if settings.raw("NODE_ENV") == Some("production") => {
            return admin_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "ADMIN_MFA_NOT_CONFIGURED",
                "Administrator MFA is not configured.",
            );
        }
        None => {}
        Some(secret) if !rustic_isles_auth::verify_totp(secret, ctx.request.field("totp"), system_now_ms()) => {
            return admin_error(
                StatusCode::UNAUTHORIZED,
                "ADMIN_MFA_INVALID",
                "The authenticator code is invalid.",
            );
        }
        Some(_) => {}
    }
    let agent = ctx.request.header("user-agent").unwrap_or_default();
    let (raw, session) = match open_session(&ctx.store, &account, &user_id, &ctx.request.client_ip, &agent).await {
        Ok(opened) => opened,
        Err(e) => return admin_internal(e),
    };
    let body = object(vec![
        ("csrfToken", Value::from(session.csrf_token)),
        ("account", public_account(&account, &user_id, &iso(now_ms()))),
    ]);
    util::with_header(util::ok(&body), "set-cookie", &session_cookie(&raw, settings))
}

/// `POST /admin/v1/auth/logout`: ends the caller's session.
pub async fn logout(ctx: AdminContext) -> Response {
    let Some(caller) = ctx.caller.clone() else {
        return admin_internal("sign-out had no caller");
    };
    let empty = util::ok(&object(vec![]));
    if caller.via == Via::Manager {
        return empty;
    }
    if let Some(session) = &caller.session {
        let id = session.id.clone();
        let at = iso(now_ms());
        if let Err(e) = ctx
            .store
            .write(&admin_actor(&caller), move |tx, repos| {
                repos.admin.revoke_session(tx, &id, &at)
            })
            .await
        {
            return admin_internal(e);
        }
    }
    util::with_header(empty, "set-cookie", &cleared_cookie(ctx.shared.settings()))
}

/// `GET /admin/v1/me`: the signed-in caller.
pub async fn me(ctx: AdminContext) -> Response {
    let Some(caller) = ctx.caller.clone() else {
        return admin_internal("me had no caller");
    };
    let csrf = caller
        .session
        .as_ref()
        .map_or(Value::Null, |s| Value::from(s.csrf_token.clone()));
    util::ok(&object(vec![
        ("csrfToken", csrf),
        ("account", caller_account(&caller, &iso(now_ms()))),
    ]))
}
