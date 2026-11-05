//! `routes/launcherAuth.ts`: the launcher API's handlers and its CORS gate. Each handler is Node's `Handle`: the
//! controller's JSON with the route's success status, or the `LauncherApiError` it threw as the launcher's error
//! envelope; anything unexpected is logged and answered as `INTERNAL`.

use axum::extract::Request;
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use service_kit::json::Json;

use super::account;
use super::discord_flow;
use super::email;
use super::errors::LauncherApiError;
use super::launcher::{self, GameSessionRequest, RegisterInput, Reply};
use super::middleware::{LauncherAuth, header as header_of, ip_of, launcher_auth};
use super::util::{field, query_values, read_body};
use crate::http::{Call, Shared};

/// `Handle`'s answer.
fn handled(reply: Reply, status: u16) -> Response {
    match reply {
        Ok(value) => (
            StatusCode::from_u16(status).unwrap_or(StatusCode::OK),
            Json(if value.is_null() { json!({}) } else { value }),
        )
            .into_response(),
        Err(error) => error.into_response(),
    }
}

/// A body parse refusal, or the parsed body (`None` is Express 5's undefined `req.body`).
async fn body_of(call: &mut Call) -> Result<Option<Value>, Response> {
    read_body(&mut call.request).await
}

/// `GetDeviceFields`.
fn device_fields(body: Option<&Value>) -> (String, String) {
    let text = |name: &str| {
        field(body, name)
            .and_then(Value::as_str)
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
    };
    (
        text("deviceId").unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        text("deviceName").unwrap_or_else(|| "Unknown device".to_owned()),
    )
}

async fn authed(shared: &Shared, headers: &HeaderMap) -> Result<LauncherAuth, Response> {
    launcher_auth(shared, headers)
        .await
        .map_err(IntoResponse::into_response)
}

pub async fn register(mut call: Call) -> Response {
    let body = match body_of(&mut call).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    let ip = ip_of(&call);
    let reply = launcher::register_account(
        &call.shared,
        RegisterInput {
            display_name: field(body.as_ref(), "displayName"),
            email: field(body.as_ref(), "email"),
            password: field(body.as_ref(), "password"),
            consent: field(body.as_ref(), "consent"),
            ip: &ip,
        },
    )
    .await;
    handled(reply, 202)
}

pub async fn login(mut call: Call) -> Response {
    let body = match body_of(&mut call).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    let (device_id, device_name) = device_fields(body.as_ref());
    let reply = launcher::login_with_password(
        &call.shared,
        field(body.as_ref(), "email"),
        field(body.as_ref(), "password"),
        &device_id,
        &device_name,
        &ip_of(&call),
    )
    .await;
    handled(reply, 200)
}

pub async fn refresh(mut call: Call) -> Response {
    let body = match body_of(&mut call).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    let (device_id, device_name) = device_fields(body.as_ref());
    let reply = launcher::refresh_session(
        &call.shared,
        field(body.as_ref(), "refreshToken"),
        &device_id,
        &device_name,
    )
    .await;
    handled(reply, 200)
}

pub async fn logout(call: Call) -> Response {
    let auth = match authed(&call.shared, call.request.headers()).await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    handled(launcher::logout(&call.shared, &auth.account, &auth.sid).await, 200)
}

pub async fn logout_all(call: Call) -> Response {
    let auth = match authed(&call.shared, call.request.headers()).await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    handled(launcher::logout_all(&call.shared, &auth.account).await, 200)
}

pub async fn discord_start(call: Call) -> Response {
    handled(discord_flow::start_discord_auth(&call.shared).await, 200)
}

/// The browser's redirect: `res.redirect(302, url)`, whose body depends on what the browser accepts.
fn redirect(call: &Call, url: &str) -> Response {
    let wants_html = header_of(call.request.headers(), "accept").is_some_and(|accept| {
        let html = accept.find("text/html");
        let text = accept.find("text/plain").or_else(|| accept.find("*/*"));
        html.is_some_and(|h| text.is_none_or(|t| h < t))
    });
    let (content_type, body) = if wants_html {
        (
            "text/html; charset=utf-8",
            format!("<p>Found. Redirecting to {}</p>", escape_html(url)),
        )
    } else {
        ("text/plain; charset=utf-8", format!("Found. Redirecting to {url}"))
    };
    let mut response = (StatusCode::FOUND, body).into_response();
    if let Ok(location) = HeaderValue::from_str(url) {
        response.headers_mut().insert(header::LOCATION, location);
    }
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    // `res.format` varies the answer by what the browser accepts
    response
        .headers_mut()
        .insert(header::VARY, HeaderValue::from_static("Accept"));
    response
}

/// `escape-html`.
fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub async fn discord_callback(call: Call) -> Response {
    // `req.query.code as string | undefined`: a repeated parameter is an array, which is not a string
    let query = call.request.uri().query().map(str::to_owned);
    let code = query_values(query.as_deref(), "code");
    let state = query_values(query.as_deref(), "state");
    let single = |values: &[String]| (values.len() == 1).then(|| values[0].clone());
    let url =
        discord_flow::handle_discord_callback(&call.shared, single(&code).as_deref(), single(&state).as_deref()).await;
    redirect(&call, &url)
}

pub async fn discord_complete(mut call: Call) -> Response {
    let body = match body_of(&mut call).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    let (device_id, device_name) = device_fields(body.as_ref());
    let reply =
        discord_flow::complete_discord_login(&call.shared, field(body.as_ref(), "code"), &device_id, &device_name)
            .await;
    handled(reply, 200)
}

pub async fn email_verify(mut call: Call) -> Response {
    let body = match body_of(&mut call).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    let (device_id, device_name) = device_fields(body.as_ref());
    let reply = email::verify_email_with_password(
        &call.shared,
        field(body.as_ref(), "email"),
        field(body.as_ref(), "password"),
        field(body.as_ref(), "code"),
        &device_id,
        &device_name,
        &ip_of(&call),
    )
    .await;
    handled(reply, 200)
}

pub async fn email_resend(mut call: Call) -> Response {
    let body = match body_of(&mut call).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    handled(
        email::resend_email_code(&call.shared, field(body.as_ref(), "email"), &ip_of(&call)),
        200,
    )
}

pub async fn password_forgot(mut call: Call) -> Response {
    let body = match body_of(&mut call).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    handled(
        email::request_password_reset_code(&call.shared, field(body.as_ref(), "email"), &ip_of(&call)),
        200,
    )
}

pub async fn password_reset(mut call: Call) -> Response {
    let body = match body_of(&mut call).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    let (device_id, device_name) = device_fields(body.as_ref());
    let reply = email::reset_password_with_code(
        &call.shared,
        field(body.as_ref(), "email"),
        field(body.as_ref(), "code"),
        field(body.as_ref(), "newPassword"),
        &device_id,
        &device_name,
        &ip_of(&call),
    )
    .await;
    handled(reply, 200)
}

pub async fn discord_link_start(call: Call) -> Response {
    match authed(&call.shared, call.request.headers()).await {
        Ok(auth) => handled(discord_flow::start_discord_link(&call.shared, &auth.user_id).await, 200),
        Err(response) => response,
    }
}

pub async fn discord_unlink_start(call: Call) -> Response {
    match authed(&call.shared, call.request.headers()).await {
        Ok(auth) => handled(
            discord_flow::start_discord_unlink(&call.shared, &auth.user_id, &ip_of(&call)).await,
            200,
        ),
        Err(response) => response,
    }
}

pub async fn discord_unlink(mut call: Call) -> Response {
    let auth = match authed(&call.shared, call.request.headers()).await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let body = match body_of(&mut call).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    handled(
        discord_flow::confirm_discord_unlink(&call.shared, &auth.user_id, field(body.as_ref(), "code"), &ip_of(&call))
            .await,
        200,
    )
}

pub async fn me(call: Call) -> Response {
    match authed(&call.shared, call.request.headers()).await {
        Ok(auth) => handled(launcher::get_account_view(&call.shared, &auth.user_id).await, 200),
        Err(response) => response,
    }
}

/// Availability check for the registration screen (pre-auth, read-only): `String(req.query.name ?? "")`.
pub async fn username_check(call: Call) -> Response {
    let name = query_values(call.request.uri().query(), "name").join(",");
    handled(launcher::check_username_available(&call.shared, &name).await, 200)
}

pub async fn username_set(mut call: Call) -> Response {
    let auth = match authed(&call.shared, call.request.headers()).await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let body = match body_of(&mut call).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    handled(
        launcher::set_username(&call.shared, &auth.user_id, field(body.as_ref(), "username")).await,
        200,
    )
}

pub async fn game_sessions(mut call: Call) -> Response {
    let auth = match authed(&call.shared, call.request.headers()).await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let body = match body_of(&mut call).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    let body = body.as_ref();
    // The route reads all of these before it checks any: an invalid channel is reported first
    let build_changelist = service_kit::js::number(field(body, "buildChangelist"));
    let executable = field(body, "executableSha256");
    let channel = match super::runtime_authorization::parse_runtime_channel(field(body, "runtimeChannel")) {
        Ok(channel) => channel,
        Err(error) => return error.into_response(),
    };
    if !build_changelist.is_finite() {
        return LauncherApiError::new("AUTH_VALIDATION_FAILED", "buildChangelist is required.").into_response();
    }
    let Some(executable) = executable.and_then(Value::as_str).filter(|e| !e.is_empty()) else {
        return LauncherApiError::new("AUTH_VALIDATION_FAILED", "executableSha256 is required.").into_response();
    };
    let reply = launcher::request_game_exchange_code(
        &call.shared,
        &auth.account,
        &auth.user_id,
        &auth.sid,
        GameSessionRequest {
            build_changelist,
            executable_sha256: executable,
            runtime_channel: channel,
            runtime_manifest_version: field(body, "runtimeManifestVersion"),
            runtime_artifacts: field(body, "runtimeArtifacts"),
            realm: field(body, "realm"),
            launcher: field(body, "launcher"),
        },
    )
    .await;
    handled(reply, 200)
}

pub async fn policy(call: Call) -> Response {
    match authed(&call.shared, call.request.headers()).await {
        Ok(auth) => handled(account::get_launcher_policy(&call.shared, &auth.user_id).await, 200),
        Err(response) => response,
    }
}

pub async fn status(call: Call) -> Response {
    handled(account::launcher_status(&call.shared).await, 200)
}

pub async fn legal(_call: Call) -> Response {
    handled(Ok(account::legal_requirements()), 200)
}

/// `GET /launcher/v1/notices`: cacheable for half a minute (the contract).
pub async fn notices(call: Call) -> Response {
    let view = account::public_launcher_notices(&call.shared).await;
    let mut response = Json(view).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=30"));
    response
}

pub async fn consent_get(call: Call) -> Response {
    match authed(&call.shared, call.request.headers()).await {
        Ok(auth) => handled(account::get_consent_state(&call.shared, &auth.user_id).await, 200),
        Err(response) => response,
    }
}

pub async fn consent_accept(mut call: Call) -> Response {
    let auth = match authed(&call.shared, call.request.headers()).await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let body = match body_of(&mut call).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    handled(
        account::accept_consent(&call.shared, &auth.user_id, body.as_ref(), super::util::now_ms()).await,
        200,
    )
}

pub async fn deletion_get(call: Call) -> Response {
    match authed(&call.shared, call.request.headers()).await {
        Ok(auth) => handled(account::get_deletion_state(&call.shared, &auth.user_id).await, 200),
        Err(response) => response,
    }
}

pub async fn deletion_request(mut call: Call) -> Response {
    let auth = match authed(&call.shared, call.request.headers()).await {
        Ok(auth) => auth,
        Err(response) => return response,
    };
    let body = match body_of(&mut call).await {
        Ok(body) => body,
        Err(response) => return response,
    };
    handled(
        account::request_account_deletion(&call.shared, &auth.user_id, body.as_ref(), super::util::now_ms()).await,
        200,
    )
}

pub async fn deletion_cancel(call: Call) -> Response {
    match authed(&call.shared, call.request.headers()).await {
        Ok(auth) => handled(
            account::cancel_account_deletion(&call.shared, &auth.user_id, super::util::now_ms()).await,
            200,
        ),
        Err(response) => response,
    }
}

// ------------------------------------------------------------------ the launcher router's CORS

const DEFAULT_ALLOWED_ORIGINS: [&str; 2] = ["http://localhost:1420", "https://tauri.localhost"];

/// What the launcher router's CORS middleware does to a request for a `/launcher/` path: an origin outside the list is
/// refused (403, empty), `OPTIONS` is answered (204), and an allowed origin's headers go on whatever answers.
#[derive(Debug)]
pub enum Cors {
    /// Not a launcher path, or no `Origin`
    Pass,
    /// 403 for an origin outside `LAUNCHER_ALLOWED_ORIGINS`, 204 for `OPTIONS`
    Answer(Response),
    /// Add these headers to the answer
    Headers(Vec<(HeaderName, HeaderValue)>),
}

fn allowed_origins(shared: &Shared) -> Vec<String> {
    DEFAULT_ALLOWED_ORIGINS
        .iter()
        .map(|origin| (*origin).to_owned())
        .chain(
            shared
                .settings
                .raw("LAUNCHER_ALLOWED_ORIGINS")
                .unwrap_or_default()
                .split(',')
                .map(|origin| service_kit::js::trim(origin).to_owned())
                .filter(|origin| !origin.is_empty()),
        )
        .collect()
}

fn cors_headers(origin: &str) -> Vec<(HeaderName, HeaderValue)> {
    let mut headers = Vec::new();
    if let Ok(origin) = HeaderValue::from_str(origin) {
        headers.push((header::ACCESS_CONTROL_ALLOW_ORIGIN, origin));
    }
    headers.push((header::VARY, HeaderValue::from_static("Origin")));
    // DELETE is for DELETE /launcher/v1/account/deletion (restore a deactivated account); nothing else uses it
    headers.push((
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, DELETE, OPTIONS"),
    ));
    headers.push((
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Content-Type, Authorization"),
    ));
    headers
}

/// The launcher router's `use` middleware, which also runs for every `/launcher/` request that reaches it, on routes of
/// the routers mounted after it. Only the launcher API (a browser or WebView) needs CORS; the game client and game
/// servers are not browsers.
pub fn launcher_cors(shared: &Shared, request: &Request) -> Cors {
    if !request.uri().path().to_ascii_lowercase().starts_with("/launcher/") {
        return Cors::Pass;
    }
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .filter(|o| !o.is_empty());
    let mut headers = Vec::new();
    if let Some(origin) = origin {
        if !allowed_origins(shared).iter().any(|allowed| allowed == origin) {
            tracing::warn!("[CORS] Rejected launcher origin: {origin}");
            return Cors::Answer(StatusCode::FORBIDDEN.into_response());
        }
        headers = cors_headers(origin);
        tracing::info!("[CORS] Allowed origin: {origin}");
    }
    if request.method() == axum::http::Method::OPTIONS {
        let mut response = StatusCode::NO_CONTENT.into_response();
        for (name, value) in headers {
            response.headers_mut().insert(name, value);
        }
        return Cors::Answer(response);
    }
    if headers.is_empty() {
        Cors::Pass
    } else {
        Cors::Headers(headers)
    }
}
