// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The launcher's requests to a server: status, register, user info, the runtime manifest, news and branding
//! (spec §2.6.3). The account key goes only to the server it was made for (AGENTS.md).

use std::time::Duration;

use dainty_protocol::account::{REGISTER_PATH, RegisterRequest, is_plausible_account_key};
use dainty_protocol::manifest::RUNTIME_MANIFEST_PATH;
use dainty_protocol::status::{STATUS_PATH, ServerStatus};
use hyper::Method;

use crate::bridge::types::{ErrorCode, LauncherError};
use crate::net::http::{
    CONNECT_TIMEOUT, Endpoint, FailureKind, HttpError, HttpResponse, REQUEST_TIMEOUT, RequestOptions, check_endpoint,
    request,
};
use crate::net::runtime_files::RuntimeManifestResult;
use crate::platform::log;

pub mod parse;

#[cfg(test)]
mod tests;

pub use parse::{
    Background, Branding, BrandingImage, ImageType, NewsItem, QueueReply, StatusReply, UserInfo, is_branding_path,
    sniff_image,
};

/// Header that carries the account key (`DLK_...`) to the server it was made for.
pub const KEY_HEADER: &str = "x-dainty-user-api-key";
/// Asked when the status route is missing: an older server answers it.
pub const PROBE_PATH: &str = "/dauntless-status";
/// The account lookup route.
pub const USER_INFO_PATH: &str = "/dainty/api/GetUserInfo";
/// The login queue: POST joins or polls it, DELETE leaves.
pub const LOGIN_QUEUE_PATH: &str = "/dainty/api/LoginQueue";
/// The content server's news.
pub const NEWS_PATH: &str = "/content/v1/news";
/// The content server's branding index.
pub const BRANDING_PATH: &str = "/content/v1/branding";
/// The key holder's online playtime (Dainty Lass's own route).
pub const PLAYTIME_PATH: &str = "/dainty/api/Playtime";

const KIB: usize = 1024;
/// Status's deadline when the caller gives none: the answer is small and polled again, so the connect's is enough.
pub const STATUS_TIMEOUT: Duration = CONNECT_TIMEOUT;
const STATUS_MAX_BYTES: usize = 1024 * KIB;
const PROBE_MAX_BYTES: usize = 64 * KIB;
const REGISTER_TIMEOUT: Duration = REQUEST_TIMEOUT;
const REGISTER_MAX_BYTES: usize = 64 * KIB;
const USER_INFO_TIMEOUT: Duration = REQUEST_TIMEOUT;
const USER_INFO_MAX_BYTES: usize = 64 * KIB;
const QUEUE_TIMEOUT: Duration = Duration::from_secs(10);
const QUEUE_MAX_BYTES: usize = 4 * KIB;
const MANIFEST_TIMEOUT: Duration = REQUEST_TIMEOUT;
const MANIFEST_MAX_BYTES: usize = 64 * KIB;
const NEWS_TIMEOUT: Duration = REQUEST_TIMEOUT;
const NEWS_MAX_BYTES: usize = 512 * KIB;
const BRANDING_TIMEOUT: Duration = REQUEST_TIMEOUT;
const BRANDING_MAX_BYTES: usize = 64 * KIB;
const IMAGE_TIMEOUT: Duration = Duration::from_secs(30);
const IMAGE_MAX_BYTES: usize = 15 * 1024 * KIB;

/// The routes and key header of one server family's launcher API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flavor {
    /// Family name.
    pub id: &'static str,
    /// Header the account key goes in.
    pub key_header: &'static str,
    /// Server status route.
    pub status_path: &'static str,
    /// Account registration route.
    pub register_path: &'static str,
    /// Account lookup route.
    pub account_path: &'static str,
    /// Content's game version list route.
    pub versions_path: &'static str,
    /// Runtime manifest route.
    pub runtime_manifest_path: &'static str,
    /// Network node list route, for region ping.
    pub region_ping_path: &'static str,
}

/// Dainty Lass's own routes.
pub const REFORGED: Flavor = Flavor {
    id: "reforged",
    key_header: KEY_HEADER,
    status_path: STATUS_PATH,
    register_path: REGISTER_PATH,
    account_path: USER_INFO_PATH,
    versions_path: "/content/v1/versions",
    runtime_manifest_path: RUNTIME_MANIFEST_PATH,
    region_ping_path: "/network/v1/nodes",
};

/// The account key header for `endpoint`, or none when there is no usable key or the endpoint may not receive one.
pub fn key_headers(endpoint: &Endpoint, key: Option<&str>) -> Vec<(&'static str, String)> {
    key_headers_as(REFORGED, endpoint, key)
}

/// [`key_headers`] with `flavor`'s key header. A plausible key goes only to a pinned endpoint or a private-mode
/// host, never in clear to anyone else.
pub fn key_headers_as(flavor: Flavor, endpoint: &Endpoint, key: Option<&str>) -> Vec<(&'static str, String)> {
    match key {
        Some(key) if is_plausible_account_key(key) && check_endpoint(endpoint).is_ok() => {
            vec![(flavor.key_header, key.to_owned())]
        }
        _ => Vec::new(),
    }
}

/// What asking a server's status came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusOutcome {
    /// The server's status.
    Status(Box<ServerStatus>),
    /// An older server this launcher cannot play on.
    Unsupported,
    /// It answered, but not with anything usable.
    BadAnswer,
    /// Its certificate is not the pinned one.
    CertMismatch {
        /// The fingerprint it presented.
        actual: String,
    },
    /// No answer, and why.
    Unreachable(FailureKind),
}

fn get(timeout: Duration, max_bytes: usize, headers: Vec<(&'static str, String)>) -> RequestOptions {
    RequestOptions {
        headers,
        ..RequestOptions::get(timeout, max_bytes)
    }
}

fn transport_outcome(err: HttpError) -> StatusOutcome {
    match err {
        HttpError::Pin { actual } => StatusOutcome::CertMismatch { actual },
        other => StatusOutcome::Unreachable(other.kind()),
    }
}

/// The sentence the player reads for a request that got no answer. A pin mismatch stays the certificate's own.
pub fn failure_code(kind: FailureKind) -> ErrorCode {
    match kind {
        FailureKind::Pin => ErrorCode::CertMismatch,
        FailureKind::Dns => ErrorCode::ServerDnsFailed,
        FailureKind::Refused => ErrorCode::ServerRefused,
        FailureKind::Timeout => ErrorCode::ServerTimedOut,
        FailureKind::Tls => ErrorCode::ServerTlsFailed,
        FailureKind::Status => ErrorCode::ServerError,
        FailureKind::Aborted => ErrorCode::Cancelled,
        FailureKind::Other | FailureKind::TooLarge => ErrorCode::ServerUnreachable,
    }
}

/// Logs a failed request with its kind (the key is in a header, never in `err`) and answers its sentence.
pub fn logged_failure(endpoint: &Endpoint, path: &str, err: &HttpError) -> LauncherError {
    let kind = err.kind();
    log::warn(format!(
        "{}:{}{path} failed ({kind}): {err}",
        endpoint.host, endpoint.port
    ));
    LauncherError::new(failure_code(kind))
}

/// The server's status, with the key when there is one so the server may show more; 404 falls back to the
/// older probe route, sent without the key.
pub async fn fetch_status(endpoint: &Endpoint, key: Option<&str>, timeout: Option<Duration>) -> StatusOutcome {
    let timeout = timeout.unwrap_or(STATUS_TIMEOUT);
    let options = get(timeout, STATUS_MAX_BYTES, key_headers(endpoint, key));
    let reply = match request(endpoint, REFORGED.status_path, options).await {
        Ok(res) => parse::status_reply(res.status, &res.body),
        Err(err) => return transport_outcome(err),
    };
    match reply {
        StatusReply::Status(status) => StatusOutcome::Status(status),
        StatusReply::BadAnswer => StatusOutcome::BadAnswer,
        StatusReply::Probe => match request(endpoint, PROBE_PATH, get(timeout, PROBE_MAX_BYTES, Vec::new())).await {
            Ok(res) if parse::probe_reply(res.status, &res.body) => StatusOutcome::Unsupported,
            Ok(_) => StatusOutcome::BadAnswer,
            Err(err) => transport_outcome(err),
        },
    }
}

/// Registers `body.username` and answers the new account key.
pub async fn register(endpoint: &Endpoint, body: &RegisterRequest) -> Result<String, LauncherError> {
    let json = serde_json::to_string(body).map_err(|_| LauncherError::new(ErrorCode::BadRequest))?;
    let options = RequestOptions {
        method: Method::POST,
        body: Some(json),
        ..RequestOptions::get(REGISTER_TIMEOUT, REGISTER_MAX_BYTES)
    };
    match request(endpoint, REFORGED.register_path, options).await {
        Ok(res) => {
            log_status(endpoint, REFORGED.register_path, res.status);
            parse::register_reply(res.status, &res.body)
        }
        Err(err) => Err(logged_failure(endpoint, REFORGED.register_path, &err)),
    }
}

/// The account `key` belongs to. An implausible key is refused without asking.
pub async fn fetch_user_info(endpoint: &Endpoint, key: &str) -> Result<UserInfo, LauncherError> {
    if !is_plausible_account_key(key) {
        return Err(LauncherError::new(ErrorCode::KeyRejected));
    }
    let options = get(USER_INFO_TIMEOUT, USER_INFO_MAX_BYTES, key_headers(endpoint, Some(key)));
    match request(endpoint, REFORGED.account_path, options).await {
        Ok(res) => {
            log_status(endpoint, REFORGED.account_path, res.status);
            parse::user_info_reply(res.status, &res.body)
        }
        Err(err) => Err(logged_failure(endpoint, REFORGED.account_path, &err)),
    }
}

/// Logs an error status with its kind; the reply's own parse picks the sentence.
fn log_status(endpoint: &Endpoint, path: &str, status: u16) {
    if let Some(kind) = FailureKind::of_status(status) {
        log::warn(format!(
            "{}:{}{path} answered {status} ({kind})",
            endpoint.host, endpoint.port
        ));
    }
}

/// The key holder's online playtime in seconds; none when the server has none to give.
pub async fn fetch_playtime(endpoint: &Endpoint, key: &str) -> Option<u64> {
    if !is_plausible_account_key(key) {
        return None;
    }
    let options = get(USER_INFO_TIMEOUT, USER_INFO_MAX_BYTES, key_headers(endpoint, Some(key)));
    let res = request(endpoint, PLAYTIME_PATH, options).await.ok()?;
    parse::playtime_reply(res.status, &res.body)
}

fn queue_options(method: Method, endpoint: &Endpoint, key: &str) -> RequestOptions {
    RequestOptions {
        method,
        ..get(QUEUE_TIMEOUT, QUEUE_MAX_BYTES, key_headers(endpoint, Some(key)))
    }
}

/// Joins the server's login queue, or polls the place already held.
pub async fn join_queue(endpoint: &Endpoint, key: &str) -> Result<QueueReply, LauncherError> {
    match request(endpoint, LOGIN_QUEUE_PATH, queue_options(Method::POST, endpoint, key)).await {
        Ok(res) => parse::queue_reply(res.status, &res.body),
        Err(err) => Err(account_error(&err)),
    }
}

/// Gives up the place in line; the server frees a place nobody polls anyway, so a failure is only logged.
pub async fn leave_queue(endpoint: &Endpoint, key: &str) -> bool {
    request(endpoint, LOGIN_QUEUE_PATH, queue_options(Method::DELETE, endpoint, key))
        .await
        .is_ok_and(|res| res.status == 204)
}

/// The server's runtime manifest; it is public, so no key is sent.
pub async fn fetch_runtime_manifest(endpoint: &Endpoint) -> RuntimeManifestResult {
    let options = RequestOptions::get(MANIFEST_TIMEOUT, MANIFEST_MAX_BYTES);
    match request(endpoint, REFORGED.runtime_manifest_path, options).await {
        Ok(res) => parse::runtime_manifest_reply(res.status, &res.body),
        Err(HttpError::Pin { .. }) => RuntimeManifestResult::CertMismatch,
        Err(err) => {
            let _ = logged_failure(endpoint, REFORGED.runtime_manifest_path, &err);
            RuntimeManifestResult::Unreachable
        }
    }
}

/// The server's news, or `fallback` when it has none to give.
pub async fn fetch_news(endpoint: &Endpoint, fallback: &[NewsItem]) -> Vec<NewsItem> {
    request(endpoint, NEWS_PATH, RequestOptions::get(NEWS_TIMEOUT, NEWS_MAX_BYTES))
        .await
        .ok()
        .and_then(|res| parse::news_reply(res.status, &res.body))
        .unwrap_or_else(|| fallback.to_vec())
}

/// The server's branding index; empty when it has none or does not answer.
pub async fn fetch_branding(endpoint: &Endpoint) -> Branding {
    match request(
        endpoint,
        BRANDING_PATH,
        RequestOptions::get(BRANDING_TIMEOUT, BRANDING_MAX_BYTES),
    )
    .await
    {
        Ok(res) => parse::branding_reply(res.status, &res.body),
        Err(_) => Branding::default(),
    }
}

/// One background named by the branding index. Any other path is never asked for.
pub async fn fetch_branding_image(endpoint: &Endpoint, path: &str) -> Option<BrandingImage> {
    if !is_branding_path(path) {
        return None;
    }
    let options = RequestOptions::get(IMAGE_TIMEOUT, IMAGE_MAX_BYTES);
    let HttpResponse { status, body } = request(endpoint, path, options).await.ok()?;
    parse::branding_image_reply(status, body)
}
