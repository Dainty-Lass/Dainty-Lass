//! RU5's `Auth` group: Login, EOS and launcher accounts. RU3c: hook `game_session_payload` (and the auth middleware's `no_player_policy`), table `TARGET_CHANGELIST`.
//!
//! - [`middleware`]: `HasParadoxBackendAuth`, `HasLauncherAuth`, `RequireActiveAccount` and the `Caller` rifts sees; every
//!   other area's game routes ask [`middleware::game_auth`] first.
//! - [`errors`]: the launcher's error envelope and its status table (`security/launcherErrors.ts`).
//! - [`runtime`]: what the area keeps in memory and the seams to its neighbours (`Shared::auth`).

// Rejections are axum responses, the way extractors report them; boxing them gains nothing here
#![allow(clippy::result_large_err)]

pub mod account;
pub mod apikeys;
pub mod crypto;
pub mod data;
pub mod discord;
pub mod discord_flow;
pub mod email;
pub mod erasure;
pub mod errors;
pub mod game;
pub mod launcher;
pub mod launcher_routes;
pub mod mail;
pub mod middleware;
pub mod policy;
pub mod runtime;
pub mod runtime_authorization;
#[doc(hidden)]
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod tokens;
pub mod totp;
pub mod util;
pub mod wiring;

pub use errors::LauncherApiError;
pub use middleware::{AuthData, LauncherAuth, game_auth, launcher_auth};
pub use runtime::AuthRuntime;

use crate::http::Handlers;
use crate::routes::{Method, Service};

/// The area's start-up checks and background work, once the module has its database: the secrets Node refuses to start
/// without, the startup lines of the registration switches, the game server keys of `GAMESERVER_API_KEYS`, and the
/// hourly sweep of unverified registrations while verification is required. A module with no database configured
/// (the skeleton) has none of it. The handles are the background tasks, to be aborted when the module stops.
pub async fn start(shared: &std::sync::Arc<crate::http::Shared>) -> Result<Vec<tokio::task::JoinHandle<()>>, String> {
    if shared.store.is_none() {
        return Ok(Vec::new());
    }
    crypto::api_key_hash_secret(&shared.settings).map_err(|e| e.to_string())?;
    tokens::keys(&shared.settings).map_err(|e| e.to_string())?;
    let verification = policy::resolve_email_verification(&shared.settings);
    if verification.unrecognised {
        tracing::warn!(
            "[Startup] WARNING: MYSTICPARADOX_EMAIL_VERIFICATION must be required or off; the unrecognised value is treated as required"
        );
    }
    if verification.required {
        // required needs working SMTP settings: without them the backend refuses to start
        if let mail::SmtpReading::Problems(problems) = mail::read_smtp_settings(&shared.settings) {
            return Err(format!(
                "MYSTICPARADOX_EMAIL_VERIFICATION=required needs working SMTP settings ({} not set or invalid; the required keys are MYSTICPARADOX_SMTP_HOST, MYSTICPARADOX_SMTP_USER and MYSTICPARADOX_SMTP_PASSWORD)",
                problems.join(", ")
            ));
        }
        tracing::info!(
            "[Startup] MYSTICPARADOX_EMAIL_VERIFICATION=required: sign-in needs a verified email (launcher 0.1.67 or later)"
        );
    } else if verification.is_default {
        tracing::info!(
            "[Startup] MYSTICPARADOX_EMAIL_VERIFICATION not set (default off): sign-in does not check the email"
        );
    } else {
        tracing::info!("[Startup] MYSTICPARADOX_EMAIL_VERIFICATION=off: sign-in does not check the email");
    }
    let approval = policy::resolve_registration_approval(&shared.settings);
    if let Some(value) = &approval.unrecognised {
        tracing::warn!(
            "[Startup] WARNING: MYSTICPARADOX_REGISTRATION_APPROVAL must be required or off (got {value:?}); it is treated as required, so new accounts stay pending"
        );
    }
    if approval.legacy_warning {
        tracing::warn!(
            "[Startup] WARNING: MYSTICPARADOX_REGISTRATION_AUTO_APPROVE is replaced by MYSTICPARADOX_REGISTRATION_APPROVAL and has no effect now"
        );
    }
    tracing::info!(
        "[Startup] MYSTICPARADOX_REGISTRATION_APPROVAL={}: new accounts {}",
        if approval.required { "required" } else { "off" },
        if approval.required {
            "need approval"
        } else {
            "are approved at once"
        }
    );
    apikeys::register_gameserver_api_keys(shared)
        .await
        .map_err(|e| e.to_string())?;
    wiring::register_seams();
    let mut tasks = Vec::new();
    if verification.required {
        tasks.push(email::start_unverified_registration_sweeper(shared.clone()));
    }
    tasks.extend(wiring::start_jobs(shared));
    Ok(tasks)
}

/// Mounts this area's handlers, the paths exactly as the route table has them.
pub fn register(handlers: &mut Handlers) {
    use launcher_routes as l;
    let backend = Service::Backend;
    macro_rules! route {
        ($method:ident, $path:literal, $handler:path) => {
            handlers.on(backend, Method::$method, $path, |call| Box::pin($handler(call)));
        };
    }
    // routes/login.ts
    route!(Get, "/features/platform/win", game::features_platform_win);
    route!(Get, "/account/link/epic/:AccId", game::account_link_epic);
    route!(Post, "/login", game::login);
    route!(Get, "/accountinfo", game::accountinfo);
    route!(Get, "/tags", game::tags);
    route!(Put, "/gamesession/epic", game::game_session);
    route!(Put, "/gamesession/epiceos", game::game_session);
    route!(Post, "/accountinfo/public", game::accountinfo_public);
    route!(Post, "/migration/trigger", game::migration_trigger);
    route!(Get, "/migration/status", game::migration_status);
    route!(Get, "/isbanned", game::isbanned);
    // routes/eos.ts
    route!(Post, "/account/api/oauth/token", game::oauth_token);
    route!(Get, "/account/api/oauth/verify", game::oauth_verify);
    route!(Get, "/account/api/public/account/:AccId", game::public_account);
    route!(
        Get,
        "/account/api/public/account/:AccId/externalAuths",
        game::public_account_external_auths
    );
    route!(
        Get,
        "/account/api/public/account/displayName/:displayName",
        game::public_account_by_display_name
    );
    route!(Delete, "/account/api/oauth/sessions/kill", game::sessions_kill);
    route!(
        Delete,
        "/account/api/oauth/sessions/kill/:AuthToken",
        game::sessions_kill
    );
    route!(Get, "/account/api/public/account", game::public_account_bulk);
    // routes/matchmaking.ts: the network encryption keys
    route!(Post, "/key/generate", game::key_generate);
    route!(Post, "/key/consume", game::key_consume);
    // routes/launcherAuth.ts
    route!(Post, "/launcher/v1/auth/register", l::register);
    route!(Post, "/launcher/v1/auth/login", l::login);
    route!(Post, "/launcher/v1/auth/refresh", l::refresh);
    route!(Post, "/launcher/v1/auth/logout", l::logout);
    route!(Post, "/launcher/v1/auth/logout-all", l::logout_all);
    route!(Post, "/launcher/v1/auth/discord/start", l::discord_start);
    route!(Get, "/launcher/v1/auth/discord/callback", l::discord_callback);
    route!(Post, "/launcher/v1/auth/discord/complete", l::discord_complete);
    route!(Post, "/launcher/v1/auth/email/verify", l::email_verify);
    route!(Post, "/launcher/v1/auth/email/resend", l::email_resend);
    route!(Post, "/launcher/v1/auth/password/forgot", l::password_forgot);
    route!(Post, "/launcher/v1/auth/password/reset", l::password_reset);
    route!(Post, "/launcher/v1/account/discord/link/start", l::discord_link_start);
    route!(
        Post,
        "/launcher/v1/account/discord/unlink/start",
        l::discord_unlink_start
    );
    route!(Post, "/launcher/v1/account/discord/unlink", l::discord_unlink);
    route!(Get, "/launcher/v1/me", l::me);
    route!(Get, "/launcher/v1/username", l::username_check);
    route!(Post, "/launcher/v1/username", l::username_set);
    route!(Post, "/launcher/v1/game-sessions", l::game_sessions);
    route!(Get, "/launcher/v1/policy", l::policy);
    route!(Get, "/launcher/v1/status", l::status);
    route!(Get, "/launcher/v1/legal", l::legal);
    route!(Get, "/launcher/v1/notices", l::notices);
    route!(Get, "/launcher/v1/account/consent", l::consent_get);
    route!(Post, "/launcher/v1/account/consent", l::consent_accept);
    route!(Get, "/launcher/v1/account/deletion", l::deletion_get);
    route!(Post, "/launcher/v1/account/deletion", l::deletion_request);
    route!(Delete, "/launcher/v1/account/deletion", l::deletion_cancel);
}
