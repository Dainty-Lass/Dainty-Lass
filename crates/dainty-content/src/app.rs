// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The routes. A request is matched on its raw path before its method is looked at, answered, and logged once
//! when its reply body ends or is dropped (Node's `res.on("close")`), so a download's line says how much of it
//! reached the connection.

use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;

use bytes::Bytes;
use dainty_module_api::GameVersion;
use http::header::{HeaderName, HeaderValue};
use http::{Method, Request, Response, StatusCode};
use http_body::{Body, Frame, SizeHint};
use serde_json::{Map, Value, json};
use tokio::io::AsyncReadExt as _;
use tokio_util::io::ReaderStream;

use crate::auth::{Account, Auth, Check};
use crate::branding::{Branding, MAX_IMAGE_BYTES};
use crate::config::Dashboard;
use crate::crash::{self, Limited, RateLimit, Sender};
use crate::input::{
    self, KEY_HEADER, Route, VersionKey, cut_utf16, header_text, if_none_match_hits, is_image_file_name,
    is_runtime_file_name,
};
use crate::limits::{Refusal, Slot, Slots};
use crate::log;
use crate::modules::{ContentModules, EpicManifestError};
use crate::news::News;
use crate::runtime::{READ_CHUNK, Status, Store};

const JSON_TYPE: &str = "application/json; charset=utf-8";
const OCTET_STREAM: &str = "application/octet-stream";
const DOWNLOAD_CACHE: &str = "private, no-transform";
const IMAGE_CACHE: &str = "public, max-age=300";
const MAX_LOGGED_PATH: usize = 200;

/// Everything the routes share; cheap to clone.
#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

struct Inner {
    runtime: Store,
    auth: Auth,
    slots: Slots,
    branding: Branding,
    news: Arc<News>,
    modules: Arc<ContentModules>,
    crash: Option<CrashReports>,
}

/// Where crash reports go: the folder, the rate limit and the dashboard copy.
pub struct CrashReports {
    dir: PathBuf,
    limit: RateLimit,
    clock: crate::Clock,
    dashboard: Option<Dashboard>,
}

impl CrashReports {
    /// Reports kept in `dir`, copied to `dashboard` when there is one.
    pub fn new(dir: PathBuf, dashboard: Option<Dashboard>, clock: crate::Clock) -> Self {
        Self {
            dir,
            limit: RateLimit::new(clock.clone()),
            clock,
            dashboard,
        }
    }
}

impl AppState {
    /// Builds the shared state from the parts [`crate::server`] sets up.
    pub fn new(
        runtime: Store,
        auth: Auth,
        slots: Slots,
        branding: Branding,
        news: News,
        modules: Arc<ContentModules>,
        crash: Option<CrashReports>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                runtime,
                auth,
                slots,
                branding,
                news: Arc::new(news),
                modules,
                crash,
            }),
        }
    }

    /// The game runtime published from the runtime folder.
    pub fn runtime(&self) -> &Store {
        &self.inner.runtime
    }

    /// The version-keyed module table.
    pub fn modules(&self) -> &Arc<ContentModules> {
        &self.inner.modules
    }

    /// The download slots.
    pub fn slots(&self) -> &Slots {
        &self.inner.slots
    }
}

/// What the request's log line carries besides the reply's status.
struct Ctx {
    route: &'static str,
    path: String,
    reason: Option<String>,
    account: Option<Account>,
}

enum Content {
    Empty,
    Full(Bytes),
    File(ReaderStream<tokio::io::Take<tokio::fs::File>>),
}

/// A reply before it becomes a response.
struct Reply {
    status: StatusCode,
    headers: Vec<(HeaderName, String)>,
    content: Content,
    slot: Option<Slot>,
    /// A streamed download: the bytes it promises
    expected: Option<u64>,
}

impl Reply {
    fn new(status: StatusCode) -> Self {
        Self {
            status,
            headers: Vec::new(),
            content: Content::Empty,
            slot: None,
            expected: None,
        }
    }

    fn header(mut self, name: HeaderName, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }

    fn json(status: StatusCode, body: String) -> Self {
        let length = body.len().to_string();
        Self::new(status)
            .header(http::header::CONTENT_TYPE, JSON_TYPE)
            .header(http::header::CONTENT_LENGTH, length)
            .header(http::header::CACHE_CONTROL, "no-cache")
            .with(Content::Full(Bytes::from(body)))
    }

    fn error(status: StatusCode, code: &str) -> Self {
        Self::json(status, dainty_service_kit::jsjson::stringify(&json!({"error": code})))
    }

    fn not_modified(etag: &str) -> Self {
        Self::new(StatusCode::NOT_MODIFIED).header(http::header::ETAG, etag)
    }

    fn with(mut self, content: Content) -> Self {
        self.content = content;
        self
    }
}

fn error_with_reason(ctx: &mut Ctx, status: StatusCode, code: &str, reason: &str) -> Reply {
    ctx.reason = Some(reason.to_owned());
    Reply::error(status, code)
}

fn retry_after(reply: Reply, seconds: u32) -> Reply {
    reply.header(http::header::RETRY_AFTER, seconds.to_string())
}

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    tokio::task::spawn_blocking(work).await.ok()
}

/// Answers one request from `peer`.
pub async fn handle<B>(state: AppState, request: Request<B>, peer: SocketAddr) -> Response<ReplyBody>
where
    B: Body<Data = Bytes> + Send,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    let arrived = Instant::now();
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let query = request.uri().query().map(str::to_owned);
    let (parts, body) = request.into_parts();
    let headers = &parts.headers;
    let ip = header_text(headers, "x-forwarded-for")
        .filter(|ip| !ip.is_empty())
        .unwrap_or_else(|| peer.ip().to_string());
    let mut ctx = Ctx {
        route: "other",
        path: cut_utf16(&path, MAX_LOGGED_PATH),
        reason: None,
        account: None,
    };
    let route = Route::of(&path);
    let reply = if path == crash::PATH && state.inner.crash.is_some() {
        ctx.route = "crash-report";
        state.crash_report(&method, headers, body, &mut ctx).await
    } else if route == Route::Unknown {
        Reply::error(StatusCode::NOT_FOUND, "not_found")
    } else if method != Method::GET && method != Method::HEAD {
        Reply::error(StatusCode::METHOD_NOT_ALLOWED, "method_not_allowed").header(http::header::ALLOW, "GET, HEAD")
    } else {
        ctx.route = route_name(route);
        let head = method == Method::HEAD;
        state.route(route, head, query.as_deref(), headers, &mut ctx).await
    };
    respond(reply, ctx, &method, arrived, ip)
}

fn route_name(route: Route<'_>) -> &'static str {
    match route {
        Route::RuntimeManifest => "runtime-manifest",
        Route::Branding => "branding",
        Route::News => "news",
        Route::BrandingImage(_) => "branding-image",
        Route::RuntimeFile(_) => "runtime-file",
        Route::Versions => "versions",
        Route::ClientProfile(_) => "client-profile",
        Route::EpicManifest(_) => "epic-manifest",
        Route::Unknown => "other",
    }
}

fn respond(reply: Reply, ctx: Ctx, method: &Method, arrived: Instant, ip: String) -> Response<ReplyBody> {
    let content = if *method == Method::HEAD {
        Content::Empty
    } else {
        reply.content
    };
    let log = RequestLog {
        arrived,
        method: method.as_str().to_owned(),
        route: ctx.route,
        path: ctx.path,
        status: reply.status.as_u16(),
        ip,
        reason: ctx.reason,
        account: ctx.account,
        expected: reply.expected,
        bytes: 0,
    };
    let mut response = Response::new(ReplyBody {
        content,
        log: Some(log),
        slot: reply.slot,
    });
    *response.status_mut() = reply.status;
    let headers = response.headers_mut();
    headers.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    for (name, value) in reply.headers {
        if let Ok(value) = HeaderValue::from_str(&value) {
            headers.append(name, value);
        }
    }
    response
}

impl AppState {
    async fn route(
        &self,
        route: Route<'_>,
        head: bool,
        query: Option<&str>,
        headers: &http::HeaderMap,
        ctx: &mut Ctx,
    ) -> Reply {
        let condition = header_text(headers, "if-none-match");
        let condition = condition.as_deref();
        match route {
            Route::RuntimeManifest => self.manifest(query, condition, ctx).await,
            Route::RuntimeFile(name) => self.runtime_file(name, head, headers, condition, ctx).await,
            Route::Branding => {
                let branding = self.clone();
                match blocking(move || branding.inner.branding.current()).await {
                    Some(scan) => Reply::json(StatusCode::OK, scan.body.clone()),
                    None => Reply::error(StatusCode::INTERNAL_SERVER_ERROR, "internal"),
                }
            }
            Route::BrandingImage(name) => self.branding_image(name, condition, ctx).await,
            Route::News => {
                self.inner.news.poll_remote();
                let news = self.inner.news.clone();
                match blocking(move || news.body()).await {
                    Some(body) => Reply::json(StatusCode::OK, body),
                    None => Reply::error(StatusCode::INTERNAL_SERVER_ERROR, "internal"),
                }
            }
            Route::Versions => match serde_json::to_string(&self.inner.modules.versions()) {
                Ok(body) => Reply::json(StatusCode::OK, body),
                Err(_) => Reply::error(StatusCode::INTERNAL_SERVER_ERROR, "internal"),
            },
            Route::ClientProfile(name) => {
                ctx.path = name.to_owned();
                self.client_profile(name, condition)
            }
            Route::EpicManifest(name) => {
                ctx.path = name.to_owned();
                self.epic_manifest(name, head, headers, condition, ctx).await
            }
            Route::Unknown => Reply::error(StatusCode::NOT_FOUND, "not_found"),
        }
    }

    /// `POST /content/v1/crash-reports`: checked in the order that reads the least first, the body last.
    async fn crash_report<B>(&self, method: &Method, headers: &http::HeaderMap, body: B, ctx: &mut Ctx) -> Reply
    where
        B: Body<Data = Bytes> + Send,
        B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
    {
        let Some(crash) = &self.inner.crash else {
            return Reply::error(StatusCode::NOT_FOUND, "not_found");
        };
        if method != Method::POST {
            return Reply::error(StatusCode::METHOD_NOT_ALLOWED, "method_not_allowed")
                .header(http::header::ALLOW, "POST");
        }
        let declared = header_text(headers, "content-length").and_then(|n| n.trim().parse::<u64>().ok());
        if declared.is_some_and(|n| n > crash::MAX_UPLOAD as u64) {
            return error_with_reason(ctx, StatusCode::PAYLOAD_TOO_LARGE, "too_large", "declared_length");
        }
        let kind = header_text(headers, "content-type").unwrap_or_default();
        if !kind.eq_ignore_ascii_case(crash::CONTENT_TYPE) {
            return error_with_reason(
                ctx,
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "bad_content_type",
                "content_type",
            );
        }
        let account = match self.account(headers, ctx).await {
            Ok(account) => account,
            Err(reply) => return *reply,
        };
        if let Err(limited) = crash.limit.take(&account.id) {
            let reason = match limited {
                Limited::Account => "account_limit",
                Limited::Total => "total_limit",
            };
            return retry_after(
                error_with_reason(ctx, StatusCode::TOO_MANY_REQUESTS, "rate_limited", reason),
                crash::WINDOW.as_secs() as u32,
            );
        }
        use http_body_util::BodyExt as _;
        let upload = match http_body_util::Limited::new(body, crash::MAX_UPLOAD).collect().await {
            Ok(collected) => collected.to_bytes(),
            Err(failed) => {
                let too_large = failed.downcast_ref::<http_body_util::LengthLimitError>().is_some();
                return if too_large {
                    error_with_reason(ctx, StatusCode::PAYLOAD_TOO_LARGE, "too_large", "body_length")
                } else {
                    error_with_reason(ctx, StatusCode::BAD_REQUEST, "bad_request", "body_read")
                };
            }
        };
        let parsed = match blocking(move || crash::parse(&upload).map(|bundle| (bundle, upload))).await {
            Some(parsed) => parsed,
            None => return Reply::error(StatusCode::INTERNAL_SERVER_ERROR, "internal"),
        };
        let (bundle, upload) = match parsed {
            Ok(parsed) => parsed,
            Err(bad) => return error_with_reason(ctx, StatusCode::BAD_REQUEST, "bad_report", bad.reason()),
        };
        let sender = Sender {
            id: account.id.clone(),
            name: account.username.clone(),
        };
        let (dir, now) = (crash.dir.clone(), (crash.clock)());
        let saved = blocking(move || crash::save(&dir, &bundle, &sender, now, &upload)).await;
        let saved = match saved {
            Some(Ok(saved)) => saved,
            Some(Err(failed)) => {
                return error_with_reason(ctx, StatusCode::INTERNAL_SERVER_ERROR, "internal", &failed.to_string());
            }
            None => return Reply::error(StatusCode::INTERNAL_SERVER_ERROR, "internal"),
        };
        ctx.reason = Some(format!("saved {}", saved.id));
        if let Some(dashboard) = crash.dashboard.clone() {
            let body = saved.dashboard;
            tokio::spawn(async move { crate::client::post_crash_report(&dashboard, &body).await });
        }
        Reply::json(
            StatusCode::CREATED,
            dainty_service_kit::jsjson::stringify(&json!({"id": saved.id})),
        )
    }

    fn requested_version(&self, query: Option<&str>) -> Option<GameVersion> {
        match input::version_query(query)? {
            (VersionKey::Version, name) => self.inner.modules.resolve(&name),
            (VersionKey::Edition, name) => self.inner.modules.resolve_edition(&name),
        }
    }

    /// A version's runtime when it has a folder, else the server's own; off the async threads (it may hash).
    async fn runtime_status(&self, version: Option<GameVersion>) -> (Status, Option<PathBuf>) {
        let folders = version.map(|v| self.inner.modules.runtime_folders(&v));
        let state = self.clone();
        blocking(move || state.inner.runtime.status_for(folders.as_deref()))
            .await
            .unwrap_or_else(|| (Status::Invalid("runtime check failed".to_owned()), None))
    }

    async fn manifest(&self, query: Option<&str>, condition: Option<&str>, ctx: &mut Ctx) -> Reply {
        let (status, _) = self.runtime_status(self.requested_version(query)).await;
        match status {
            Status::None(reason) => error_with_reason(ctx, StatusCode::NOT_FOUND, "no_runtime", &reason),
            Status::Invalid(reason) => {
                error_with_reason(ctx, StatusCode::SERVICE_UNAVAILABLE, "runtime_invalid", &reason)
            }
            Status::Ok(published) => {
                if if_none_match_hits(condition, &published.etag) {
                    return Reply::not_modified(&published.etag);
                }
                Reply::json(StatusCode::OK, published.body.clone()).header(http::header::ETAG, published.etag.clone())
            }
        }
    }

    /// The account key's check: the account, or the reply refusing it.
    async fn account(&self, headers: &http::HeaderMap, ctx: &mut Ctx) -> Result<Account, Box<Reply>> {
        let key = header_text(headers, KEY_HEADER).unwrap_or_default();
        let refused = |ctx: &mut Ctx, reason: &str| {
            Box::new(error_with_reason(ctx, StatusCode::UNAUTHORIZED, "unauthorized", reason))
        };
        if key.is_empty() {
            return Err(refused(ctx, "no_key"));
        }
        match self.inner.auth.check(&key).await {
            Check::Account(account) => {
                ctx.account = Some(account.clone());
                Ok(account)
            }
            Check::Refused => Err(refused(ctx, "bad_key")),
            Check::Unavailable(reason) => Err(Box::new(retry_after(
                error_with_reason(ctx, StatusCode::SERVICE_UNAVAILABLE, "auth_unavailable", &reason),
                5,
            ))),
        }
    }

    fn slot(&self, account: &Account, ctx: &mut Ctx) -> Result<Slot, Box<Reply>> {
        self.inner.slots.acquire(&account.id).map_err(|refusal| {
            Box::new(match refusal {
                Refusal::Account => retry_after(
                    error_with_reason(ctx, StatusCode::TOO_MANY_REQUESTS, "too_many_streams", refusal.reason()),
                    2,
                ),
                Refusal::Total => retry_after(
                    error_with_reason(ctx, StatusCode::SERVICE_UNAVAILABLE, "server_busy", refusal.reason()),
                    10,
                ),
            })
        })
    }

    async fn runtime_file(
        &self,
        name: &str,
        head: bool,
        headers: &http::HeaderMap,
        condition: Option<&str>,
        ctx: &mut Ctx,
    ) -> Reply {
        let (version, file_name) = match name.split_once('/') {
            Some((first, rest)) => match self.inner.modules.resolve(first) {
                Some(version) => (Some(version), rest),
                None => (None, name),
            },
            None => (None, name),
        };
        if !is_runtime_file_name(file_name) {
            return error_with_reason(ctx, StatusCode::NOT_FOUND, "not_found", "not_a_runtime_file");
        }
        ctx.path = file_name.to_owned();
        let account = match self.account(headers, ctx).await {
            Ok(account) => account,
            Err(reply) => return *reply,
        };
        let (status, dir) = self.runtime_status(version).await;
        let Some((listed, dir)) = status.listed(file_name).cloned().zip(dir) else {
            return error_with_reason(ctx, StatusCode::NOT_FOUND, "not_found", "no_runtime");
        };
        // The listed name, never the requested one
        let path = dir.join(&listed.name);
        let etag = format!("\"{}\"", listed.sha256);
        if if_none_match_hits(condition, &etag) {
            return Reply::not_modified(&etag);
        }
        let download = |reply: Reply| {
            reply
                .header(http::header::ETAG, etag.clone())
                .header(http::header::CONTENT_TYPE, OCTET_STREAM)
                .header(http::header::CONTENT_LENGTH, listed.size.to_string())
        };
        if head {
            return download(Reply::new(StatusCode::OK));
        }
        let slot = match self.slot(&account, ctx) {
            Ok(slot) => slot,
            Err(reply) => return *reply,
        };
        let unavailable = |ctx: &mut Ctx, reason: Option<&str>, slot: Slot| {
            ctx.reason = reason.map(str::to_owned);
            let mut reply = Reply::error(StatusCode::SERVICE_UNAVAILABLE, "file_unavailable");
            reply.slot = Some(slot);
            reply
        };
        let file = match tokio::fs::File::open(&path).await {
            Ok(file) => file,
            Err(_) => {
                self.inner.runtime.forget();
                return unavailable(ctx, Some("open_failed"), slot);
            }
        };
        let meta = match file.metadata().await {
            Ok(meta) => meta,
            Err(_) => return unavailable(ctx, None, slot),
        };
        if !meta.is_file() || meta.len() != listed.size {
            self.inner.runtime.forget();
            return retry_after(unavailable(ctx, Some("changed_on_disk"), slot), 10);
        }
        let mut reply = download(Reply::new(StatusCode::OK))
            .header(http::header::CACHE_CONTROL, DOWNLOAD_CACHE)
            .with(stream(file, listed.size));
        reply.slot = Some(slot);
        reply.expected = Some(listed.size);
        reply
    }

    async fn branding_image(&self, name: &str, condition: Option<&str>, ctx: &mut Ctx) -> Reply {
        let not_found = || Reply::error(StatusCode::NOT_FOUND, "not_found");
        if !is_image_file_name(name) {
            return not_found();
        }
        let state = self.clone();
        let Some(scan) = blocking(move || state.inner.branding.current()).await else {
            return not_found();
        };
        let (Some(image), Some(dir)) = (scan.image(name), self.inner.branding.dir()) else {
            return not_found();
        };
        ctx.path = name.to_owned();
        let Ok(file) = tokio::fs::File::open(dir.join(name)).await else {
            return not_found();
        };
        let Ok(meta) = file.metadata().await else {
            return not_found();
        };
        if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_IMAGE_BYTES {
            return not_found();
        }
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_millis());
        let tag = format!("\"{:x}-{modified:x}\"", meta.len());
        let etag = format!("W/{tag}");
        if if_none_match_hits(condition, &tag) {
            return Reply::not_modified(&etag).header(http::header::CACHE_CONTROL, IMAGE_CACHE);
        }
        Reply::new(StatusCode::OK)
            .header(http::header::ETAG, etag)
            .header(http::header::CACHE_CONTROL, IMAGE_CACHE)
            .header(http::header::CONTENT_TYPE, image.content_type)
            .header(http::header::CONTENT_LENGTH, meta.len().to_string())
            .with(stream(file, meta.len()))
    }

    fn client_profile(&self, name: &str, condition: Option<&str>) -> Reply {
        let unknown = || Reply::error(StatusCode::NOT_FOUND, "unknown_version");
        let Some(version) = self.inner.modules.resolve(name) else {
            return unknown();
        };
        let profile = match self.inner.modules.client_profile(&version) {
            None => return unknown(),
            Some(None) => return Reply::error(StatusCode::NOT_FOUND, "no_client_profile"),
            Some(Some(profile)) => profile,
        };
        let Ok(body) = serde_json::to_string(&profile) else {
            return Reply::error(StatusCode::INTERNAL_SERVER_ERROR, "internal");
        };
        let etag = format!("\"{}\"", crate::sha256_hex(body.as_bytes()));
        if if_none_match_hits(condition, &etag) {
            return Reply::not_modified(&etag);
        }
        Reply::json(StatusCode::OK, body).header(http::header::ETAG, etag)
    }

    async fn epic_manifest(
        &self,
        name: &str,
        head: bool,
        headers: &http::HeaderMap,
        condition: Option<&str>,
        ctx: &mut Ctx,
    ) -> Reply {
        // The version before the key: an unknown one is 404 to anyone
        let Some(version) = self.inner.modules.resolve(name) else {
            return Reply::error(StatusCode::NOT_FOUND, "unknown_version");
        };
        let account = match self.account(headers, ctx).await {
            Ok(account) => account,
            Err(reply) => return *reply,
        };
        let modules = self.inner.modules.clone();
        let found = blocking(move || modules.epic_manifest(&version))
            .await
            .unwrap_or_else(|| Err(EpicManifestError::Unavailable("manifest check failed".to_owned())));
        let (bytes, sha1) = match found {
            Ok(found) => found,
            Err(EpicManifestError::UnknownVersion) => return Reply::error(StatusCode::NOT_FOUND, "unknown_version"),
            Err(EpicManifestError::NotServed) => return Reply::error(StatusCode::NOT_FOUND, "no_epic_manifest"),
            Err(EpicManifestError::Unavailable(reason)) => {
                return error_with_reason(
                    ctx,
                    StatusCode::SERVICE_UNAVAILABLE,
                    "epic_manifest_unavailable",
                    &reason,
                );
            }
            Err(EpicManifestError::Mismatch { expected, found }) => {
                let reason = format!("its SHA-1 is {found}, the client profile pins {expected}");
                return error_with_reason(ctx, StatusCode::SERVICE_UNAVAILABLE, "epic_manifest_invalid", &reason);
            }
        };
        let etag = format!("\"{sha1}\"");
        if if_none_match_hits(condition, &etag) {
            return Reply::not_modified(&etag);
        }
        let length = bytes.len() as u64;
        let download = |reply: Reply| {
            reply
                .header(http::header::ETAG, etag.clone())
                .header(http::header::CONTENT_TYPE, OCTET_STREAM)
                .header(http::header::CONTENT_LENGTH, length.to_string())
        };
        if head {
            return download(Reply::new(StatusCode::OK));
        }
        let slot = match self.slot(&account, ctx) {
            Ok(slot) => slot,
            Err(reply) => return *reply,
        };
        let mut reply = download(Reply::new(StatusCode::OK))
            .header(http::header::CACHE_CONTROL, DOWNLOAD_CACHE)
            .with(Content::Full(bytes));
        reply.slot = Some(slot);
        reply.expected = Some(length);
        reply
    }
}

fn stream(file: tokio::fs::File, length: u64) -> Content {
    Content::File(ReaderStream::with_capacity(file.take(length), READ_CHUNK))
}

/// One request's log line, written when the reply body ends or is dropped.
struct RequestLog {
    arrived: Instant,
    method: String,
    route: &'static str,
    path: String,
    status: u16,
    ip: String,
    reason: Option<String>,
    account: Option<Account>,
    expected: Option<u64>,
    bytes: u64,
}

impl RequestLog {
    fn write(self) {
        let mut fields = Map::new();
        fields.insert("method".into(), Value::String(self.method.clone()));
        fields.insert("route".into(), Value::String(self.route.to_owned()));
        fields.insert("path".into(), Value::String(self.path));
        fields.insert("status".into(), json!(self.status));
        fields.insert("ms".into(), json!(self.arrived.elapsed().as_millis() as u64));
        fields.insert("ip".into(), Value::String(self.ip));
        if let Some(reason) = self.reason {
            fields.insert("reason".into(), Value::String(reason));
        }
        if let Some(account) = self.account {
            fields.insert("account".into(), Value::String(account.id));
            fields.insert("user".into(), Value::String(account.username));
        }
        let download = matches!(self.route, "runtime-file" | "epic-manifest") && self.method == "GET";
        let mut msg = "request";
        if let Some(expected) = self.expected {
            fields.insert("bytes".into(), json!(self.bytes));
            fields.insert("expected".into(), json!(expected));
            fields.insert("complete".into(), json!(self.bytes == expected));
            if download {
                msg = "download";
            }
        }
        let level = if matches!(self.status, 400 | 401) || self.status >= 500 {
            log::Level::Warn
        } else {
            log::Level::Info
        };
        log::log(level, msg, Value::Object(fields));
    }
}

/// A reply body that writes its request's log line and gives back its download slot when it ends or drops.
pub struct ReplyBody {
    content: Content,
    log: Option<RequestLog>,
    slot: Option<Slot>,
}

impl ReplyBody {
    fn finish(&mut self) {
        self.slot.take();
        if let Some(log) = self.log.take() {
            log.write();
        }
    }

    fn sent(&mut self, n: usize) {
        if let Some(log) = &mut self.log {
            log.bytes += n as u64;
        }
    }
}

impl Body for ReplyBody {
    type Data = Bytes;
    type Error = io::Error;

    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
        let this = &mut *self;
        let next = match &mut this.content {
            Content::Empty => None,
            Content::Full(bytes) => {
                let bytes = std::mem::take(bytes);
                this.content = Content::Empty;
                (!bytes.is_empty()).then_some(Ok(bytes))
            }
            Content::File(file) => match futures_util::Stream::poll_next(Pin::new(file), cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(next) => next,
            },
        };
        match next {
            Some(Ok(bytes)) => {
                this.sent(bytes.len());
                Poll::Ready(Some(Ok(Frame::data(bytes))))
            }
            Some(Err(error)) => {
                this.finish();
                Poll::Ready(Some(Err(error)))
            }
            None => {
                this.content = Content::Empty;
                this.finish();
                Poll::Ready(None)
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        match &self.content {
            Content::Empty => true,
            Content::Full(bytes) => bytes.is_empty(),
            Content::File(_) => false,
        }
    }

    fn size_hint(&self) -> SizeHint {
        match &self.content {
            Content::Empty => SizeHint::with_exact(0),
            Content::Full(bytes) => SizeHint::with_exact(bytes.len() as u64),
            Content::File(_) => SizeHint::default(),
        }
    }
}

impl Drop for ReplyBody {
    fn drop(&mut self) {
        self.finish();
    }
}
