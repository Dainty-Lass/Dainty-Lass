// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The 49 routes (spec §3), their dispatch, and a standalone router for tests and a host of its own.

use std::sync::Arc;

use axum::extract::Request;
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use module_api::BoxFuture;
use serde_json::Value;
use service_kit::routing::Params;

use crate::launcher::{Cors, LauncherAuth};
use crate::store::Purpose;
use crate::util::{self, ok};
use crate::{Auth, AuthData, AuthStore, LauncherError};

/// Which of the 49 routes a request matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RouteId {
    /// `GET /features/platform/win`.
    FeaturesPlatformWin,
    /// `GET /account/link/epic/:AccId`.
    AccountLinkEpic,
    /// `GET /isbanned`.
    IsBanned,
    /// `POST /migration/trigger`.
    MigrationTrigger,
    /// `GET /migration/status`.
    MigrationStatus,
    /// `GET /tags`.
    Tags,
    /// `POST /login`.
    Login,
    /// `GET /accountinfo`.
    AccountInfo,
    /// `PUT /gamesession/epic`.
    GameSessionEpic,
    /// `PUT /gamesession/epiceos`.
    GameSessionEpicEos,
    /// `POST /accountinfo/public`.
    AccountInfoPublic,
    /// `POST /account/api/oauth/token`.
    OauthToken,
    /// `GET /account/api/oauth/verify`.
    OauthVerify,
    /// `GET /account/api/public/account/displayName/:displayName`.
    PublicAccountByDisplayName,
    /// `GET /account/api/public/account/:AccId/externalAuths`.
    PublicAccountExternalAuths,
    /// `GET /account/api/public/account/:AccId`.
    PublicAccount,
    /// `DELETE /account/api/oauth/sessions/kill`.
    KillSessions,
    /// `DELETE /account/api/oauth/sessions/kill/:AuthToken`.
    KillSession,
    /// `GET /account/api/public/account`.
    PublicAccounts,
    /// `POST /key/generate`.
    KeyGenerate,
    /// `POST /key/consume`.
    KeyConsume,
    /// `POST /launcher/v1/auth/register`.
    Register,
    /// `POST /launcher/v1/auth/login`.
    LauncherLogin,
    /// `POST /launcher/v1/auth/refresh`.
    Refresh,
    /// `POST /launcher/v1/auth/logout`.
    Logout,
    /// `POST /launcher/v1/auth/logout-all`.
    LogoutAll,
    /// `POST /launcher/v1/auth/email/verify`.
    EmailVerify,
    /// `POST /launcher/v1/auth/email/resend`.
    EmailResend,
    /// `POST /launcher/v1/auth/password/forgot`.
    PasswordForgot,
    /// `POST /launcher/v1/auth/password/reset`.
    PasswordReset,
    /// `POST /launcher/v1/auth/discord/start`.
    DiscordStart,
    /// `GET /launcher/v1/auth/discord/callback`.
    DiscordCallback,
    /// `POST /launcher/v1/auth/discord/complete`.
    DiscordComplete,
    /// `POST /launcher/v1/account/discord/link/start`.
    DiscordLinkStart,
    /// `POST /launcher/v1/account/discord/unlink/start`.
    DiscordUnlinkStart,
    /// `POST /launcher/v1/account/discord/unlink`.
    DiscordUnlink,
    /// `GET /launcher/v1/me`.
    Me,
    /// `GET /launcher/v1/username`.
    UsernameCheck,
    /// `POST /launcher/v1/username`.
    UsernameSet,
    /// `GET /launcher/v1/account/consent`.
    ConsentGet,
    /// `POST /launcher/v1/account/consent`.
    ConsentPost,
    /// `GET /launcher/v1/account/deletion`.
    DeletionGet,
    /// `POST /launcher/v1/account/deletion`.
    DeletionPost,
    /// `DELETE /launcher/v1/account/deletion`.
    DeletionDelete,
    /// `GET /launcher/v1/policy`.
    Policy,
    /// `GET /launcher/v1/status`.
    Status,
    /// `GET /launcher/v1/legal`.
    Legal,
    /// `GET /launcher/v1/notices`.
    Notices,
    /// `POST /launcher/v1/game-sessions`.
    GameSessions,
}

/// One row of the route table.
#[derive(Debug, Clone)]
pub struct Route {
    /// The HTTP method it answers.
    pub method: Method,
    /// The Express-style path, `:name` for a parameter.
    pub path: &'static str,
    /// The handler it dispatches to.
    pub id: RouteId,
}

const fn route(method: Method, path: &'static str, id: RouteId) -> Route {
    Route { method, path, id }
}

static ROUTES: [Route; 49] = [
    route(Method::GET, "/features/platform/win", RouteId::FeaturesPlatformWin),
    route(Method::GET, "/account/link/epic/:AccId", RouteId::AccountLinkEpic),
    route(Method::GET, "/isbanned", RouteId::IsBanned),
    route(Method::POST, "/migration/trigger", RouteId::MigrationTrigger),
    route(Method::GET, "/migration/status", RouteId::MigrationStatus),
    route(Method::GET, "/tags", RouteId::Tags),
    route(Method::POST, "/login", RouteId::Login),
    route(Method::GET, "/accountinfo", RouteId::AccountInfo),
    route(Method::PUT, "/gamesession/epic", RouteId::GameSessionEpic),
    route(Method::PUT, "/gamesession/epiceos", RouteId::GameSessionEpicEos),
    route(Method::POST, "/accountinfo/public", RouteId::AccountInfoPublic),
    route(Method::POST, "/account/api/oauth/token", RouteId::OauthToken),
    route(Method::GET, "/account/api/oauth/verify", RouteId::OauthVerify),
    route(
        Method::GET,
        "/account/api/public/account/displayName/:displayName",
        RouteId::PublicAccountByDisplayName,
    ),
    route(
        Method::GET,
        "/account/api/public/account/:AccId/externalAuths",
        RouteId::PublicAccountExternalAuths,
    ),
    route(
        Method::GET,
        "/account/api/public/account/:AccId",
        RouteId::PublicAccount,
    ),
    route(
        Method::DELETE,
        "/account/api/oauth/sessions/kill",
        RouteId::KillSessions,
    ),
    route(
        Method::DELETE,
        "/account/api/oauth/sessions/kill/:AuthToken",
        RouteId::KillSession,
    ),
    route(Method::GET, "/account/api/public/account", RouteId::PublicAccounts),
    route(Method::POST, "/key/generate", RouteId::KeyGenerate),
    route(Method::POST, "/key/consume", RouteId::KeyConsume),
    route(Method::POST, "/launcher/v1/auth/register", RouteId::Register),
    route(Method::POST, "/launcher/v1/auth/login", RouteId::LauncherLogin),
    route(Method::POST, "/launcher/v1/auth/refresh", RouteId::Refresh),
    route(Method::POST, "/launcher/v1/auth/logout", RouteId::Logout),
    route(Method::POST, "/launcher/v1/auth/logout-all", RouteId::LogoutAll),
    route(Method::POST, "/launcher/v1/auth/email/verify", RouteId::EmailVerify),
    route(Method::POST, "/launcher/v1/auth/email/resend", RouteId::EmailResend),
    route(
        Method::POST,
        "/launcher/v1/auth/password/forgot",
        RouteId::PasswordForgot,
    ),
    route(Method::POST, "/launcher/v1/auth/password/reset", RouteId::PasswordReset),
    route(Method::POST, "/launcher/v1/auth/discord/start", RouteId::DiscordStart),
    route(
        Method::GET,
        "/launcher/v1/auth/discord/callback",
        RouteId::DiscordCallback,
    ),
    route(
        Method::POST,
        "/launcher/v1/auth/discord/complete",
        RouteId::DiscordComplete,
    ),
    route(
        Method::POST,
        "/launcher/v1/account/discord/link/start",
        RouteId::DiscordLinkStart,
    ),
    route(
        Method::POST,
        "/launcher/v1/account/discord/unlink/start",
        RouteId::DiscordUnlinkStart,
    ),
    route(
        Method::POST,
        "/launcher/v1/account/discord/unlink",
        RouteId::DiscordUnlink,
    ),
    route(Method::GET, "/launcher/v1/me", RouteId::Me),
    route(Method::GET, "/launcher/v1/username", RouteId::UsernameCheck),
    route(Method::POST, "/launcher/v1/username", RouteId::UsernameSet),
    route(Method::GET, "/launcher/v1/account/consent", RouteId::ConsentGet),
    route(Method::POST, "/launcher/v1/account/consent", RouteId::ConsentPost),
    route(Method::GET, "/launcher/v1/account/deletion", RouteId::DeletionGet),
    route(Method::POST, "/launcher/v1/account/deletion", RouteId::DeletionPost),
    route(Method::DELETE, "/launcher/v1/account/deletion", RouteId::DeletionDelete),
    route(Method::GET, "/launcher/v1/policy", RouteId::Policy),
    route(Method::GET, "/launcher/v1/status", RouteId::Status),
    route(Method::GET, "/launcher/v1/legal", RouteId::Legal),
    route(Method::GET, "/launcher/v1/notices", RouteId::Notices),
    route(Method::POST, "/launcher/v1/game-sessions", RouteId::GameSessions),
];

/// §3's routes, paths exactly as isles' table has them.
pub fn routes() -> &'static [Route] {
    &ROUTES
}

fn param<'a>(params: &'a [(String, String)], name: &str) -> &'a str {
    params.iter().find(|(k, _)| k == name).map_or("", |(_, v)| v.as_str())
}

impl<S: AuthStore> Auth<S> {
    /// Answers one matched request (the peer from `ConnectInfo<ConnectionInfo>`).
    pub fn handle(&self, id: RouteId, request: Request, params: Vec<(String, String)>) -> BoxFuture<'_, Response> {
        Box::pin(async move { self.dispatch(id, request, params).await })
    }

    async fn game_caller(&self, request: &mut Request) -> Result<AuthData, Response> {
        if let Some(data) = request.extensions().get::<AuthData>() {
            return Ok(data.clone());
        }
        self.game_auth(request).await
    }

    async fn dispatch(&self, id: RouteId, mut request: Request, params: Vec<(String, String)>) -> Response {
        use RouteId as R;
        let stub = |payload: Value| ok(serde_json::json!({"code": null, "message": "OK", "payload": payload}));
        match id {
            R::FeaturesPlatformWin => return stub(serde_json::json!({"crossplay": true, "crossprogression": true})),
            R::AccountLinkEpic => return stub(serde_json::json!({"isLinked": true})),
            R::IsBanned => return ok(serde_json::json!({"isBanned": false})),
            R::OauthToken => return self.oauth_token(request).await,
            R::OauthVerify => return self.oauth_verify(&request),
            R::PublicAccount => {
                let caller = self.bearer_user(&request);
                return self.public_account_route(caller, param(&params, "AccId")).await;
            }
            R::PublicAccountExternalAuths => return self.external_auths(param(&params, "AccId")).await,
            R::PublicAccountByDisplayName => return self.by_display_name(param(&params, "displayName")).await,
            R::KillSessions | R::KillSession => return ok(serde_json::json!({})),
            _ => {}
        }
        if matches!(
            id,
            R::MigrationTrigger
                | R::MigrationStatus
                | R::Tags
                | R::Login
                | R::AccountInfo
                | R::GameSessionEpic
                | R::GameSessionEpicEos
                | R::AccountInfoPublic
                | R::PublicAccounts
                | R::KeyGenerate
                | R::KeyConsume
        ) {
            let data = match self.game_caller(&mut request).await {
                Ok(data) => data,
                Err(response) => return response,
            };
            return match id {
                R::MigrationTrigger => ok(serde_json::json!({"migration_failed": false, "migration_finished": true})),
                R::MigrationStatus => stub(serde_json::json!({"migration_failed": false, "migration_finished": true})),
                R::Tags => self.tags(&data),
                R::AccountInfo => self.account_info(&data).await,
                R::GameSessionEpic | R::GameSessionEpicEos => self.game_session(&request),
                R::PublicAccounts => self.bulk_accounts(&data, request.uri().query()).await,
                R::KeyGenerate => self.key_generate(&data, request).await,
                R::KeyConsume => self.key_consume(&data, request).await,
                R::Login | R::AccountInfoPublic => {
                    let body = match util::read_body(request).await {
                        Ok(body) => body,
                        Err(response) => return response,
                    };
                    if id == R::Login {
                        self.login(&data, body).await
                    } else {
                        self.account_info_public(&data, body).await
                    }
                }
                _ => util::empty(StatusCode::NOT_FOUND),
            };
        }
        match self.launcher_route(id, request).await {
            Ok(response) => response,
            Err(error) => self.error_response(&error),
        }
    }

    async fn caller(&self, headers: &axum::http::HeaderMap) -> Result<LauncherAuth, LauncherError> {
        self.launcher_auth(headers).await
    }

    async fn launcher_route(&self, id: RouteId, request: Request) -> Result<Response, LauncherError> {
        use RouteId as R;
        let ip = crate::client::client_ip(&request);
        let query = request.uri().query().map(str::to_owned);
        let body = |request: Request| async move { util::read_body(request).await };
        macro_rules! body {
            ($request:expr) => {
                match body($request).await {
                    Ok(body) => body,
                    Err(response) => return Ok(response),
                }
            };
        }
        match id {
            R::Register => self.register(&ip, body!(request)).await,
            R::LauncherLogin => self.login_route(&ip, body!(request)).await,
            R::Refresh => self.refresh(body!(request)).await,
            R::Logout | R::LogoutAll => {
                let caller = self.caller(request.headers()).await?;
                self.logout(&caller, id == R::LogoutAll).await
            }
            R::EmailVerify => self.email_verify(&ip, body!(request)).await,
            R::EmailResend => self.code_request(&ip, body!(request), Purpose::Verify).await,
            R::PasswordForgot => self.code_request(&ip, body!(request), Purpose::Reset).await,
            R::PasswordReset => self.password_reset(&ip, body!(request)).await,
            R::DiscordStart => self.discord_start().await,
            R::DiscordCallback => {
                let accept = util::header(request.headers(), "accept");
                Ok(self.discord_callback(query.as_deref(), accept.as_deref()).await)
            }
            R::DiscordComplete => self.discord_complete(body!(request)).await,
            R::DiscordLinkStart => {
                let caller = self.caller(request.headers()).await?;
                self.discord_link_start(&caller).await
            }
            R::DiscordUnlinkStart => {
                let caller = self.caller(request.headers()).await?;
                self.unlink_start(&ip, &caller).await
            }
            R::DiscordUnlink => {
                let caller = self.caller(request.headers()).await?;
                self.unlink_confirm(&ip, &caller, body!(request)).await
            }
            R::Me => {
                let caller = self.caller(request.headers()).await?;
                self.me(&caller).await
            }
            R::UsernameCheck => self.username_check(query.as_deref()).await,
            R::UsernameSet => {
                let caller = self.caller(request.headers()).await?;
                self.username_set(&caller, body!(request)).await
            }
            R::ConsentGet => {
                let caller = self.caller(request.headers()).await?;
                self.consent_get(&caller).await
            }
            R::ConsentPost => {
                let caller = self.caller(request.headers()).await?;
                self.consent_post(&caller, body!(request)).await
            }
            R::DeletionGet => {
                let caller = self.caller(request.headers()).await?;
                self.deletion_get(&caller).await
            }
            R::DeletionPost => {
                let caller = self.caller(request.headers()).await?;
                self.deletion_post(&caller, body!(request)).await
            }
            R::DeletionDelete => {
                let caller = self.caller(request.headers()).await?;
                self.deletion_delete(&caller).await
            }
            R::Policy => {
                let caller = self.caller(request.headers()).await?;
                self.policy_route(&caller).await
            }
            R::Status => self.status_route().await,
            R::Legal => Ok(Self::legal()),
            R::Notices => Ok(self.notices().await),
            R::GameSessions => {
                let caller = self.caller(request.headers()).await?;
                self.game_sessions(&caller, body!(request)).await
            }
            _ => Ok(util::empty(StatusCode::NOT_FOUND)),
        }
    }

    /// Every route on `service_kit::routing` (Express matching) behind the CORS gate.
    pub fn router(self: Arc<Self>) -> axum::Router {
        let mut router = service_kit::routing::Router::<()>::new();
        for route in routes() {
            let auth = Arc::clone(&self);
            let id = route.id;
            let names: Vec<String> = route
                .path
                .split('/')
                .filter_map(|s| s.strip_prefix(':'))
                .map(str::to_owned)
                .collect();
            let handler = move |request: Request| {
                let auth = Arc::clone(&auth);
                let names = names.clone();
                async move {
                    let found = request.extensions().get::<Params>().cloned().unwrap_or_default();
                    let params = names
                        .iter()
                        .map(|n| (n.clone(), found.get(n).unwrap_or("").to_owned()))
                        .collect();
                    auth.handle(id, request, params).await
                }
            };
            router = router.route(route.method.clone(), route.path, handler);
        }
        let service = router.into_service((), service_kit::json::not_found);
        let gate = Arc::clone(&self);
        axum::Router::new()
            .fallback_service(service)
            .layer(axum::middleware::from_fn(move |request: Request, next: Next| {
                let gate = Arc::clone(&gate);
                async move {
                    match gate.launcher_cors(&request) {
                        Cors::Pass => next.run(request).await,
                        Cors::Answer(response) => response,
                        Cors::Headers(headers) => {
                            let mut response = next.run(request).await.into_response();
                            for (name, value) in headers {
                                response.headers_mut().insert(name, value);
                            }
                            response
                        }
                    }
                }
            }))
    }
}
