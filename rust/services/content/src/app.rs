//! The routes (`src/app.ts`), all GET or HEAD, behind the gateway:
//!
//! - /content/v1/runtime/manifest: the game runtime the host published, its version and files with
//!   their sizes and SHA-256 (no key needed: the launcher shows it to the player before asking them
//!   to accept it)
//! - /content/v1/runtime/files/<name>: `MysticParadox.dll` or `winmm.dll`; needs
//!   `x-reforged-user-api-key`; always the whole file (no HTTP ranges)
//! - /content/v1/branding: the host's art pack (background image URLs and an accent colour)
//! - /content/v1/branding/<file>: one of those images
//! - /content/v1/news: the host's news items
//! - /content/v1/versions: the game versions loaded modules serve (DF4b, `modules.rs`)
//! - /content/v1/versions/<version>/client: that module's client profile (DD12)
//! - /content/v1/versions/<version>/epic/manifest: its pinned Epic manifest (DD22); needs the key
//!
//! The runtime routes take the version as `?version=`, `?game=` or `?edition=` on the manifest and
//! `files/<version>/<name>`; without one they serve the server's own runtime, 1.14.7.
//!
//! Anything else is 404; there is no directory listing anywhere. Every request is logged once its
//! answer is done (or the client went away), downloads with how many bytes went out.

use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::{Instant, SystemTime};

use axum::body::Body;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::response::Response;
use bytes::Bytes;
use futures_util::StreamExt;
use module_api::GameVersion;
use serde_json::{Map, Value, json};
use tokio::io::AsyncReadExt;

pub use crate::input::{KEY_HEADER, RUNTIME_FILES_PREFIX};

use crate::auth::{AccountInfo, AuthCache, LookupResult};
use crate::branding::{Branding, MAX_IMAGE_BYTES, is_safe_image_name};
use crate::input::{Route, VersionKey, header_text, if_none_match_hits, is_runtime_file_name, version_query};
use crate::limits::{Refusal, Slot, StreamLimiter};
use crate::log;
use crate::modules::{ContentModules, EpicManifestError};
use crate::news::News;
use crate::runtime::{RuntimeStatus, RuntimeStore, sha256_hex};

const READ_CHUNK: usize = 256 * 1024;

pub struct Deps {
    pub runtime: Arc<RuntimeStore>,
    pub auth: AuthCache,
    pub limiter: StreamLimiter,
    pub branding: Branding,
    pub news: Arc<News>,
}

pub type AppState = Arc<App>;

pub struct App {
    deps: Deps,
    /// What loaded modules registered, by game version (DF4b)
    modules: Arc<ContentModules>,
}

impl App {
    pub fn new(deps: Deps) -> AppState {
        Self::with_modules(deps, Arc::default())
    }

    /// An app serving what the controller attaches to `modules` (DF3 holds the same `Arc` as a `CorePlugin`).
    pub fn with_modules(deps: Deps, modules: Arc<ContentModules>) -> AppState {
        Arc::new(Self { deps, modules })
    }

    fn route<'a>(&self, path_only: &'a str) -> Route<'a> {
        Route::of(path_only)
    }

    pub fn deps(&self) -> &Deps {
        &self.deps
    }

    pub fn modules(&self) -> &Arc<ContentModules> {
        &self.modules
    }

    /// The runtime folders a version may have; none for no version (the server's own runtime).
    fn runtime_folders(&self, version: Option<&GameVersion>) -> Vec<String> {
        version.map(|v| self.modules.runtime_folders(v)).unwrap_or_default()
    }

    /// A version's runtime status, read off the async threads (a check may hash both files).
    async fn runtime(&self, version: Option<&GameVersion>) -> RuntimeStatus {
        let store = self.deps.runtime.clone();
        let folders = self.runtime_folders(version);
        tokio::task::spawn_blocking(move || store.get_in(&folders))
            .await
            .unwrap_or_else(|_| RuntimeStatus::Invalid("runtime check failed".into()))
    }
}

pub fn router(state: AppState) -> axum::Router {
    axum::Router::new().fallback(handle).with_state(state)
}

// ---- The request log ----

#[derive(Default)]
struct Info {
    route: &'static str,
    path: Option<String>,
    reason: Option<String>,
    account: Option<String>,
    user: Option<String>,
    /// Bytes sent and expected, for a download
    bytes: Option<(Arc<AtomicU64>, u64)>,
}

/// Written when the body is done or dropped (Node's `res.on("close")`); holds the download slot too.
struct Logged {
    inner: Body,
    method: String,
    path_only: String,
    status: u16,
    started: Instant,
    ip: String,
    info: Info,
    _slot: Option<Slot>,
}

impl http_body::Body for Logged {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Bytes>, axum::Error>>> {
        let polled = Pin::new(&mut self.inner).poll_frame(cx);
        if let Poll::Ready(Some(Ok(frame))) = &polled
            && let (Some(data), Some((sent, _))) = (frame.data_ref(), &self.info.bytes)
        {
            sent.fetch_add(data.len() as u64, Ordering::Relaxed);
        }
        polled
    }

    fn is_end_stream(&self) -> bool {
        self.status != 304 && self.inner.is_end_stream()
    }

    /// A 304 has no length: an empty body's exact 0 would make hyper add `content-length: 0` to the
    /// answer to a HEAD, which Node does not send
    fn size_hint(&self) -> http_body::SizeHint {
        if self.status == 304 {
            http_body::SizeHint::default()
        } else {
            self.inner.size_hint()
        }
    }
}

impl Drop for Logged {
    fn drop(&mut self) {
        let mut fields = Map::new();
        fields.insert("method".into(), json!(self.method));
        fields.insert("route".into(), json!(self.info.route));
        let path = self
            .info
            .path
            .clone()
            .unwrap_or_else(|| crate::input::slice_utf16(&self.path_only, 200));
        fields.insert("path".into(), json!(path));
        fields.insert("status".into(), json!(self.status));
        fields.insert("ms".into(), json!(self.started.elapsed().as_millis() as u64));
        fields.insert("ip".into(), json!(self.ip));
        for (name, value) in [
            ("reason", &self.info.reason),
            ("account", &self.info.account),
            ("user", &self.info.user),
        ] {
            if let Some(value) = value {
                fields.insert(name.into(), json!(value));
            }
        }
        if let Some((sent, expected)) = &self.info.bytes {
            let sent = sent.load(Ordering::Relaxed);
            fields.insert("bytes".into(), json!(sent));
            fields.insert("expected".into(), json!(expected));
            // Every byte handed to the connection (hyper stops reading a body at its length)
            fields.insert("complete".into(), json!(sent == *expected));
        }
        let level = if self.status >= 500 || self.status == 400 || self.status == 401 {
            log::Level::Warn
        } else {
            log::Level::Info
        };
        let download = matches!(self.info.route, "runtime-file" | "files" | "epic-manifest");
        let msg = if download && self.method == "GET" && self.info.bytes.is_some() {
            "download"
        } else {
            "request"
        };
        log::log(level, msg, Value::Object(fields));
    }
}

/// A reply being built: Node sets headers on `res` as it goes.
struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Body,
}

impl Reply {
    fn new() -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("x-content-type-options"),
            HeaderValue::from_static("nosniff"),
        );
        Self {
            status: StatusCode::OK,
            headers,
            body: Body::empty(),
        }
    }

    fn set(&mut self, name: &'static str, value: impl ToString) {
        if let Ok(value) = HeaderValue::from_str(&value.to_string()) {
            self.headers.insert(HeaderName::from_static(name), value);
        }
    }

    /// `SendJson` with text already serialised: it goes out byte for byte.
    fn text(&mut self, head: bool, status: u16, text: String, extra: &[(&'static str, &str)]) {
        self.status = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        self.set("content-type", "application/json; charset=utf-8");
        self.set("content-length", text.len());
        self.set("cache-control", "no-cache");
        for (name, value) in extra {
            self.set(name, value);
        }
        self.body = if head { Body::empty() } else { Body::from(text) };
    }

    /// `SendJson`.
    fn json(&mut self, head: bool, status: u16, body: &Value, extra: &[(&'static str, &str)]) {
        self.text(head, status, service_kit::jsjson::stringify(body), extra);
    }

    fn error(&mut self, head: bool, status: u16, code: &str, extra: &[(&'static str, &str)]) {
        self.json(head, status, &json!({"error": code}), extra);
    }
}

async fn handle(State(app): State<AppState>, ConnectInfo(peer): ConnectInfo<SocketAddr>, request: Request) -> Response {
    let started = Instant::now();
    let method = request.method().clone();
    let target = request
        .uri()
        .path_and_query()
        .map_or_else(|| request.uri().path().to_owned(), |p| p.as_str().to_owned());
    let (path_only, query_only) = match target.split_once('?') {
        Some((p, q)) => (p.to_owned(), Some(q.to_owned())),
        None => (target.clone(), None),
    };
    let head = method == Method::HEAD;
    // The request's headers alone cross the awaits below (a request is not Sync)
    let headers = request.headers().clone();
    drop(request);
    let ip = header_text(&headers, "x-forwarded-for").unwrap_or_else(|| peer.ip().to_string());
    let mut info = Info {
        route: "other",
        ..Info::default()
    };
    let mut reply = Reply::new();
    let mut slot = None;
    let route = app.route(&path_only);
    if route == Route::Unknown {
        reply.error(head, 404, "not_found", &[]);
    } else if method != Method::GET && method != Method::HEAD {
        reply.error(head, 405, "method_not_allowed", &[("allow", "GET, HEAD")]);
    } else {
        match route {
            Route::RuntimeManifest => {
                info.route = "runtime-manifest";
                runtime_manifest(&app, &headers, head, query_only.as_deref(), &mut reply, &mut info).await;
            }
            Route::Branding => {
                info.route = "branding";
                let body = app.deps.branding.body();
                reply.json(head, 200, &body, &[]);
            }
            Route::News => {
                info.route = "news";
                let body = app.deps.news.body();
                reply.json(head, 200, &body, &[]);
            }
            Route::BrandingImage(name) => {
                info.route = "branding-image";
                branding_image(&app, &headers, head, name, &mut reply, &mut info).await;
            }
            Route::RuntimeFile(name) => {
                info.route = "runtime-file";
                slot = runtime_file(&app, &headers, head, name, &mut reply, &mut info).await;
            }
            Route::Versions => {
                info.route = "versions";
                versions(&app, head, &mut reply);
            }
            Route::ClientProfile(version) => {
                info.route = "client-profile";
                client_profile(&app, &headers, head, version, &mut reply, &mut info);
            }
            Route::EpicManifest(version) => {
                info.route = "epic-manifest";
                slot = epic_manifest(&app, &headers, head, version, &mut reply, &mut info).await;
            }
            Route::Unknown => {}
        }
    }
    let status = reply.status.as_u16();
    let logged = Logged {
        inner: reply.body,
        method: method.to_string(),
        path_only,
        status,
        started,
        ip,
        info,
        _slot: slot,
    };
    let mut response = Response::new(Body::new(logged));
    *response.status_mut() = reply.status;
    *response.headers_mut() = reply.headers;
    response
}

/// The version the query names (`input::version_query`); one that names nothing known is ignored, as Node
/// ignores every query here.
fn requested_version(app: &App, query: Option<&str>) -> Option<GameVersion> {
    match version_query(query)? {
        (VersionKey::Version, name) => app.modules.resolve(&name),
        (VersionKey::Edition, name) => app.modules.resolve_edition(&name),
    }
}

async fn runtime_manifest(
    app: &App,
    headers: &HeaderMap,
    head: bool,
    query: Option<&str>,
    reply: &mut Reply,
    info: &mut Info,
) {
    let (body, etag) = match app.runtime(requested_version(app, query).as_ref()).await {
        RuntimeStatus::Ok { body, etag, .. } => (body, etag),
        // 404: nothing published yet (the launcher shows "waiting for the host").
        // 503: something is published but does not check out; the host has to fix it.
        RuntimeStatus::None(reason) => {
            info.reason = Some(reason);
            reply.error(head, 404, "no_runtime", &[]);
            return;
        }
        RuntimeStatus::Invalid(reason) => {
            info.reason = Some(reason);
            reply.error(head, 503, "runtime_invalid", &[]);
            return;
        }
    };
    if if_none_match_hits(header_text(headers, "if-none-match").as_deref(), &etag) {
        reply.status = StatusCode::NOT_MODIFIED;
        reply.set("etag", &etag);
        return;
    }
    reply.text(head, 200, body, &[("etag", &etag)]);
}

/// Milliseconds since 1970, floored (`Math.floor(stat.mtimeMs)`).
fn modified_ms(modified: Option<SystemTime>) -> u128 {
    modified
        .and_then(|m| m.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis())
}

async fn branding_image(app: &App, headers: &HeaderMap, head: bool, name: &str, reply: &mut Reply, info: &mut Info) {
    if !is_safe_image_name(name) {
        reply.error(head, 404, "not_found", &[]);
        return;
    }
    let Some((image, full)) = app.deps.branding.image(name) else {
        reply.error(head, 404, "not_found", &[]);
        return;
    };
    info.path = Some(name.to_owned());
    let Ok(file) = tokio::fs::File::open(&full).await else {
        reply.error(head, 404, "not_found", &[]);
        return;
    };
    let Ok(meta) = file.metadata().await else {
        reply.error(head, 404, "not_found", &[]);
        return;
    };
    if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_IMAGE_BYTES {
        reply.error(head, 404, "not_found", &[]);
        return;
    }
    let size = meta.len();
    let etag = format!("W/\"{size:x}-{:x}\"", modified_ms(meta.modified().ok()));
    reply.set("etag", &etag);
    reply.set("cache-control", "public, max-age=300");
    if if_none_match_hits(
        header_text(headers, "if-none-match").as_deref(),
        etag.trim_start_matches("W/"),
    ) {
        reply.status = StatusCode::NOT_MODIFIED;
        return;
    }
    reply.set("content-type", image.content_type);
    reply.set("content-length", size);
    if !head {
        reply.body = file_body(file, size);
    }
}

/// The file's bytes in 256 KiB reads (`createReadStream`), `length` of them.
fn file_body(file: tokio::fs::File, length: u64) -> Body {
    let stream = tokio_util::io::ReaderStream::with_capacity(file.take(length), READ_CHUNK);
    Body::from_stream(stream.map(|chunk| chunk.map_err(axum::Error::new)))
}

/// One runtime file; the download slot it holds, if it got one.
async fn runtime_file(
    app: &App,
    headers: &HeaderMap,
    head: bool,
    raw_name: &str,
    reply: &mut Reply,
    info: &mut Info,
) -> Option<Slot> {
    let deps = &app.deps;
    // `files/<version>/<name>` names the version's runtime (`?version=` on the manifest)
    let (version, name) = match raw_name.split_once('/') {
        Some((named, file_name)) => match app.modules.resolve(named) {
            Some(version) => (Some(version), file_name),
            None => (None, raw_name),
        },
        None => (None, raw_name),
    };
    if !is_runtime_file_name(name) {
        info.reason = Some("not_a_runtime_file".into());
        reply.error(head, 404, "not_found", &[]);
        return None;
    }
    info.path = Some(name.to_owned());
    let account = account(app, headers, head, reply, info).await?;
    let store = deps.runtime.clone();
    let wanted = name.to_owned();
    let folders = app.runtime_folders(version.as_ref());
    let found = tokio::task::spawn_blocking(move || store.file_in(&folders, &wanted))
        .await
        .ok()
        .flatten();
    let Some((file, full)) = found else {
        info.reason = Some("no_runtime".into());
        reply.error(head, 404, "not_found", &[]);
        return None;
    };
    let etag = format!("\"{}\"", file.sha256);
    if if_none_match_hits(header_text(headers, "if-none-match").as_deref(), &etag) {
        reply.status = StatusCode::NOT_MODIFIED;
        reply.set("etag", &etag);
        return None;
    }
    if head {
        reply.set("etag", &etag);
        reply.set("content-type", "application/octet-stream");
        reply.set("content-length", file.size);
        return None;
    }
    let slot = match deps.limiter.try_acquire(&account.user_id) {
        Ok(slot) => slot,
        Err(refusal) => {
            info.reason = Some(refusal.as_str().into());
            match refusal {
                Refusal::AccountLimit => reply.error(head, 429, "too_many_streams", &[("retry-after", "2")]),
                Refusal::TotalLimit => reply.error(head, 503, "server_busy", &[("retry-after", "10")]),
            }
            return None;
        }
    };
    let opened = match tokio::fs::File::open(&full).await {
        Ok(opened) => opened,
        Err(_) => {
            deps.runtime.invalidate();
            info.reason = Some("open_failed".into());
            reply.error(head, 503, "file_unavailable", &[]);
            return Some(slot);
        }
    };
    let Ok(meta) = opened.metadata().await else {
        reply.error(head, 503, "file_unavailable", &[]);
        return Some(slot);
    };
    // Replaced since the last check (a new publish in progress): never send a size the list does not name
    if !meta.is_file() || meta.len() != file.size {
        deps.runtime.invalidate();
        info.reason = Some("changed_on_disk".into());
        reply.error(head, 503, "file_unavailable", &[("retry-after", "10")]);
        return Some(slot);
    }
    reply.set("etag", &etag);
    reply.set("content-type", "application/octet-stream");
    reply.set("content-length", file.size);
    reply.set("cache-control", "private, no-transform");
    info.bytes = Some((Arc::new(AtomicU64::new(0)), file.size));
    reply.body = file_body(opened, file.size);
    Some(slot)
}

/// The account `x-reforged-user-api-key` names, or the refusal written into `reply`.
async fn account(
    app: &App,
    headers: &HeaderMap,
    head: bool,
    reply: &mut Reply,
    info: &mut Info,
) -> Option<AccountInfo> {
    let Some(key) = header_text(headers, KEY_HEADER).filter(|key| !key.is_empty()) else {
        info.reason = Some("no_key".into());
        reply.error(head, 401, "unauthorized", &[]);
        return None;
    };
    let account = match app.deps.auth.check(&key).await {
        LookupResult::Unavailable(reason) => {
            info.reason = Some(reason);
            reply.error(head, 503, "auth_unavailable", &[("retry-after", "5")]);
            return None;
        }
        LookupResult::Invalid => {
            info.reason = Some("bad_key".into());
            reply.error(head, 401, "unauthorized", &[]);
            return None;
        }
        LookupResult::Ok(account) => account,
    };
    info.account = Some(account.user_id.clone());
    info.user = Some(account.username.clone());
    Some(account)
}

/// A reply body in module-api's own field names, so the launcher reads it with the same types.
fn serialised(value: &impl serde::Serialize) -> Option<String> {
    serde_json::to_string(value).ok()
}

/// The versions loaded modules serve (`modules::VersionsBody`).
fn versions(app: &App, head: bool, reply: &mut Reply) {
    match serialised(&app.modules.versions()) {
        Some(body) => reply.text(head, 200, body, &[]),
        None => reply.error(head, 500, "internal", &[]),
    }
}

/// A version's client profile (`module_api::ClientProfile`), with an ETag of its body.
fn client_profile(app: &App, headers: &HeaderMap, head: bool, raw_version: &str, reply: &mut Reply, info: &mut Info) {
    info.path = Some(raw_version.to_owned());
    let profile = app
        .modules
        .resolve(raw_version)
        .and_then(|version| app.modules.client_profile(&version));
    let profile = match profile {
        None => return reply.error(head, 404, "unknown_version", &[]),
        Some(None) => return reply.error(head, 404, "no_client_profile", &[]),
        Some(Some(profile)) => profile,
    };
    let Some(body) = serialised(&profile) else {
        return reply.error(head, 500, "internal", &[]);
    };
    let etag = format!("\"{}\"", sha256_hex(body.as_bytes()));
    if if_none_match_hits(header_text(headers, "if-none-match").as_deref(), &etag) {
        reply.status = StatusCode::NOT_MODIFIED;
        reply.set("etag", &etag);
        return;
    }
    reply.text(head, 200, body, &[("etag", &etag)]);
}

/// A version's Epic manifest (DD22): the file its module registered, sent only while its SHA-1 is the one
/// the client profile pins. Needs the key and takes a download slot, as a runtime file does.
async fn epic_manifest(
    app: &App,
    headers: &HeaderMap,
    head: bool,
    raw_version: &str,
    reply: &mut Reply,
    info: &mut Info,
) -> Option<Slot> {
    info.path = Some(raw_version.to_owned());
    let Some(version) = app.modules.resolve(raw_version) else {
        reply.error(head, 404, "unknown_version", &[]);
        return None;
    };
    let account = account(app, headers, head, reply, info).await?;
    let modules = app.modules.clone();
    let checked = tokio::task::spawn_blocking(move || modules.epic_manifest(&version))
        .await
        .unwrap_or_else(|_| Err(EpicManifestError::Unavailable("manifest check failed".into())));
    let (bytes, sha1) = match checked {
        Ok(found) => found,
        Err(EpicManifestError::UnknownVersion) => {
            reply.error(head, 404, "unknown_version", &[]);
            return None;
        }
        Err(EpicManifestError::NotServed) => {
            reply.error(head, 404, "no_epic_manifest", &[]);
            return None;
        }
        Err(EpicManifestError::Unavailable(reason)) => {
            info.reason = Some(reason);
            reply.error(head, 503, "epic_manifest_unavailable", &[]);
            return None;
        }
        Err(EpicManifestError::Mismatch { expected, found }) => {
            info.reason = Some(format!("its SHA-1 is {found}, the client profile pins {expected}"));
            reply.error(head, 503, "epic_manifest_invalid", &[]);
            return None;
        }
    };
    let etag = format!("\"{sha1}\"");
    if if_none_match_hits(header_text(headers, "if-none-match").as_deref(), &etag) {
        reply.status = StatusCode::NOT_MODIFIED;
        reply.set("etag", &etag);
        return None;
    }
    let size = bytes.len() as u64;
    if head {
        reply.set("etag", &etag);
        reply.set("content-type", "application/octet-stream");
        reply.set("content-length", size);
        return None;
    }
    let slot = match app.deps.limiter.try_acquire(&account.user_id) {
        Ok(slot) => slot,
        Err(refusal) => {
            info.reason = Some(refusal.as_str().into());
            match refusal {
                Refusal::AccountLimit => reply.error(head, 429, "too_many_streams", &[("retry-after", "2")]),
                Refusal::TotalLimit => reply.error(head, 503, "server_busy", &[("retry-after", "10")]),
            }
            return None;
        }
    };
    reply.set("etag", &etag);
    reply.set("content-type", "application/octet-stream");
    reply.set("content-length", size);
    reply.set("cache-control", "private, no-transform");
    info.bytes = Some((Arc::new(AtomicU64::new(0)), size));
    reply.body = Body::from(bytes);
    Some(slot)
}
