// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! One dispatcher per listener (core spec §3): the replay scope, the request id, the origin strip, the one count
//! of the request budget, launcher CORS, the admin gate and the walk of the route table in Express's order.

use std::net::IpAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{ConnectInfo, Request};
use axum::response::{IntoResponse, Response};
use http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use http_body_util::BodyExt;
use rustic_isles_auth::{CallerKind, Cors, LauncherError};
use serde_json::{Map, Value};
use service_kit::json::JSON_UTF8;
use service_kit::serve::ConnectionInfo;

use crate::body::BODY_LIMIT;
use crate::handlers::{Call, Handler, Handlers, MountError};
use crate::pattern::{Params, Pattern};
use crate::replay::{self, ReplayScope};
use crate::routes::{self, Method, RouteDef, Service, When};
use crate::shared::Shared;

pub const REQUEST_ID_HEADER: &str = "x-rustic-isles-request-id";
/// Selects the game server budget whether or not the key is good; auth checks it later.
pub const GAMESERVER_KEY_HEADER: &str = "x-rustic-isles-gameserver-apikey";
/// A player host's capability; rifts' guard names its bucket.
pub const HOST_CAPABILITY_HEADER: &str = "x-mystic-host-capability";

const ORIGIN_PREFIX: &str = "/__origin/";
const APEXES: [&str; 4] = [
    "steelyard.ca",
    "steelyard.online",
    "ol.epicgames.com",
    "api.epicgames.dev",
];

/// What the `/__origin/<host>/` strip does to a request target (§3.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OriginStrip {
    Unchanged,
    Stripped { host: String, target: String },
    Refused,
}

fn is_allowed_host(host: &str) -> bool {
    let shaped = (1..=253).contains(&host.len())
        && host
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
        && !host.starts_with('.')
        && !host.ends_with('.')
        && !host.contains("..");
    shaped
        && APEXES
            .iter()
            .any(|apex| host == *apex || host.strip_suffix(apex).is_some_and(|sub| sub.ends_with('.')))
}

/// The 1.14.7 runtime's `SetURL` hook prefixes the official host to every path; only the official apexes (and
/// their subdomains) are taken.
pub fn strip_origin(target: &str) -> OriginStrip {
    let Some(rest) = target.strip_prefix(ORIGIN_PREFIX) else {
        return OriginStrip::Unchanged;
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (segment, tail) = rest.split_at(end);
    if segment.is_empty() {
        return OriginStrip::Unchanged;
    }
    let Some(decoded) = service_kit::js::decode_uri_component(segment) else {
        return OriginStrip::Refused;
    };
    let host = decoded.to_lowercase();
    if !is_allowed_host(&host) {
        return OriginStrip::Refused;
    }
    let tail = match tail {
        "" => String::from("/"),
        t if t.starts_with('/') => t.to_owned(),
        t => format!("/{t}"),
    };
    match tail.parse::<http::uri::PathAndQuery>() {
        Ok(parsed) if parsed.as_str().starts_with('/') => OriginStrip::Stripped {
            host,
            target: parsed.as_str().to_owned(),
        },
        _ => OriginStrip::Refused,
    }
}

/// 8 to 96 characters once trimmed, a letter or digit first, then letters, digits, `.`, `_`, `:`, `-`.
pub fn safe_request_id(raw: &str) -> Option<&str> {
    let id = raw.trim();
    let mut bytes = id.bytes();
    let first = bytes.next()?;
    let safe = (8..=96).contains(&id.len())
        && first.is_ascii_alphanumeric()
        && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-'));
    safe.then_some(id)
}

/// The first safe of `x-rustic-isles-request-id`, `x-request-id`; else a fresh, never seeded UUID.
pub fn request_id_of(headers: &HeaderMap) -> String {
    [REQUEST_ID_HEADER, "x-request-id"]
        .iter()
        .filter_map(|name| headers.get(*name))
        .find_map(|value| safe_request_id(&String::from_utf8_lossy(value.as_bytes())).map(str::to_owned))
        .unwrap_or_else(replay::random_uuid)
}

/// `/admin` or `/admin/...`, ASCII case-insensitive.
pub fn is_admin_path(path: &str) -> bool {
    has_prefix(path, "/admin")
}

/// `/mystic/v1/hunts` or `/mystic/v1/hunts/...`, ASCII case-insensitive: counted before anything else.
pub fn is_hunts_path(path: &str) -> bool {
    has_prefix(path, "/mystic/v1/hunts")
}

fn has_prefix(path: &str, prefix: &str) -> bool {
    let n = prefix.len();
    path.len() >= n
        && path.as_bytes()[..n].eq_ignore_ascii_case(prefix.as_bytes())
        && (path.len() == n || path.as_bytes()[n] == b'/')
}

fn empty(status: StatusCode) -> Response {
    status.into_response()
}

fn json(status: StatusCode, value: &Value) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, JSON_UTF8)],
        service_kit::jsjson::stringify(value),
    )
        .into_response()
}

fn error_envelope(code: &str, message: &str, request_id: &str) -> Value {
    let mut error = Map::new();
    error.insert("code".into(), Value::from(code));
    error.insert("message".into(), Value::from(message));
    error.insert("requestId".into(), Value::from(request_id));
    let mut outer = Map::new();
    outer.insert("error".into(), Value::Object(error));
    Value::Object(outer)
}

/// The 501 a row with no handler answers; a regex row shows its literal.
pub fn not_implemented(route: &RouteDef, request_id: &str) -> Response {
    let message = format!("isles: {} {} is not implemented yet", route.method.as_str(), route.path);
    json(
        StatusCode::NOT_IMPLEMENTED,
        &error_envelope("NOT_IMPLEMENTED", &message, request_id),
    )
}

enum Answer {
    Handler(Handler),
    NotImplemented,
    /// The position of the rifts route in `Rifts::routes`, which lends its handlers
    Rifts(usize, &'static rifts_seam::Route),
}

struct Mounted {
    row: &'static RouteDef,
    index: usize,
    pattern: Pattern,
    answer: Answer,
}

struct App {
    shared: Arc<Shared>,
    service: Service,
    mounted: Vec<Mounted>,
    budget_mark: usize,
    admin_mark: usize,
}

fn rifts_method_matches(theirs: rifts_seam::Method, ours: Method) -> bool {
    matches!(
        (theirs, ours),
        (rifts_seam::Method::All, _)
            | (rifts_seam::Method::Get, Method::Get)
            | (rifts_seam::Method::Post, Method::Post)
            | (rifts_seam::Method::Patch, Method::Patch)
    )
}

fn mark(predicate: impl Fn(&RouteDef) -> bool) -> usize {
    routes::all().iter().position(predicate).unwrap_or(usize::MAX)
}

/// The service's app: a router whose fallback is the dispatcher.
pub fn app(shared: Arc<Shared>, service: Service, handlers: &Handlers) -> Result<axum::Router, MountError> {
    handlers.check()?;
    let rifts = shared.rifts().clone();
    let mut mounted = Vec::new();
    for (index, row) in routes::all().iter().enumerate() {
        let in_set = shared.version().routes.routes(service).contains(&row);
        let on = match row.when {
            When::Always | When::Gauntlet => true,
            When::Admin => shared.admin_api(),
            When::Rifts => rifts.loaded(),
        };
        if !in_set || !on {
            continue;
        }
        let answer = match handlers.get(row) {
            Some(handler) => Answer::Handler(Arc::clone(handler)),
            None if row.when == When::Rifts => {
                let found = rifts
                    .routes()
                    .enumerate()
                    .find(|(_, (route, _))| route.path == row.path && rifts_method_matches(route.method, row.method));
                match found {
                    Some((position, (route, _))) => Answer::Rifts(position, route),
                    None => {
                        return Err(MountError::NoRiftsHandler {
                            method: row.method.as_str(),
                            path: row.path,
                        });
                    }
                }
            }
            None => Answer::NotImplemented,
        };
        mounted.push(Mounted {
            row,
            index,
            pattern: Pattern::parse(row.path)?,
            answer,
        });
    }
    let state = Arc::new(App {
        shared,
        service,
        mounted,
        budget_mark: mark(|r| r.method == Method::Get && r.path == "/internal/v1/metrics"),
        admin_mark: mark(|r| r.path.starts_with("/admin/")),
    });
    Ok(axum::Router::new().fallback(move |request: Request| {
        let state = Arc::clone(&state);
        async move { dispatch(state, request).await }
    }))
}

async fn dispatch(state: Arc<App>, request: Request) -> Response {
    let scope = if state.shared.settings().replay_headers() {
        ReplayScope::from_headers(request.headers())
    } else {
        ReplayScope::default()
    };
    replay::with_replay_scope(scope, answer(state, request)).await
}

async fn answer(state: Arc<App>, request: Request) -> Response {
    let request_id = request_id_of(request.headers());
    let observation = state.shared.observer().begin(&request, &request_id);
    let head = request.method() == http::Method::HEAD;
    let mut response = Walk::new(&state, request_id.clone()).run(request).await;
    observation.finish(response.status());
    if let Ok(value) = HeaderValue::from_str(&request_id) {
        response.headers_mut().insert(REQUEST_ID_HEADER, value);
    }
    if head {
        *response.body_mut() = Body::empty();
    }
    response
}

struct Walk<'a> {
    app: &'a App,
    request_id: String,
    counted: bool,
    budget_headers: Vec<(HeaderName, HeaderValue)>,
    cors_headers: Vec<(HeaderName, HeaderValue)>,
}

fn peer(request: &Request) -> Option<IpAddr> {
    request
        .extensions()
        .get::<ConnectInfo<ConnectionInfo>>()
        .map(|info| info.0.peer.ip())
}

/// What the budget reads from a request, taken before any await (a body is not `Sync`).
struct Probe {
    ip_key: String,
    gameserver: bool,
    /// `x-mystic-host-capability`, first value trimmed, when not empty
    capability: Option<String>,
    path: String,
    peer: Option<IpAddr>,
}

impl Probe {
    fn of(request: &Request) -> Self {
        let capability = first_text(request.headers(), HOST_CAPABILITY_HEADER)
            .map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty());
        Self {
            ip_key: rustic_isles_auth::rate_limit_key(&rustic_isles_auth::client_ip(request)),
            gameserver: request.headers().contains_key(GAMESERVER_KEY_HEADER),
            capability,
            path: request.uri().path().to_owned(),
            peer: peer(request),
        }
    }
}

fn first_text(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
}

impl<'a> Walk<'a> {
    fn new(app: &'a App, request_id: String) -> Self {
        Self {
            app,
            request_id,
            counted: false,
            budget_headers: Vec::new(),
            cors_headers: Vec::new(),
        }
    }

    fn finish(&self, mut response: Response) -> Response {
        for (name, value) in self.budget_headers.iter().chain(self.cors_headers.iter()) {
            response.headers_mut().insert(name.clone(), value.clone());
        }
        response
    }

    /// The one count of the request budget; `Err` is the 429.
    async fn count(&mut self, probe: Probe) -> Result<(), Response> {
        if self.counted {
            return Ok(());
        }
        self.counted = true;
        let shared = &self.app.shared;
        let settings = shared.settings();
        if !settings.rate_limit_enabled() {
            return Ok(());
        }
        let (bucket, limit) = if probe.gameserver {
            (
                format!("gameserver:{}", probe.ip_key),
                settings.gameserver_rate_limit_per_minute(),
            )
        } else {
            match self.host_bucket(&probe).await {
                Some(bucket) => (bucket, settings.host_rate_limit_per_minute()),
                None => (format!("client:{}", probe.ip_key), settings.rate_limit_per_minute()),
            }
        };
        let verdict = shared
            .auth()
            .limiter()
            .check_request_budget_at(&bucket, limit, replay::now_ms());
        let mut headers = vec![
            ("ratelimit", verdict.rate_limit.as_str()),
            ("ratelimit-policy", verdict.rate_limit_policy.as_str()),
        ];
        if let Some(retry) = &verdict.retry_after {
            headers.push(("retry-after", retry.as_str()));
        }
        for (name, value) in headers {
            if let Ok(value) = HeaderValue::from_str(value) {
                self.budget_headers.push((HeaderName::from_static(name), value));
            }
        }
        if verdict.limited {
            let body = serde_json::json!({"error": "Too many requests. Please retry later."});
            return Err(self.finish(json(StatusCode::TOO_MANY_REQUESTS, &body)));
        }
        Ok(())
    }

    async fn host_bucket(&self, probe: &Probe) -> Option<String> {
        let capability = probe.capability.clone()?;
        let summary = rifts_seam::RequestInfo {
            path: probe.path.clone(),
            headers: vec![(HOST_CAPABILITY_HEADER.to_owned(), capability)],
            remote: probe.peer,
            request_id: self.request_id.clone(),
        };
        let bucket = self.app.shared.rifts().guard().rate_limit_bucket(summary).await;
        bucket.ok().flatten().filter(|name| !name.is_empty())
    }

    /// Launcher CORS, asked only after the count (auth spec §4.5); `Err` is the answer to return.
    fn cors(&mut self, request: &Request) -> Result<(), Response> {
        match self.app.shared.auth().launcher_cors(request) {
            Cors::Pass => Ok(()),
            Cors::Answer(response) => Err(self.finish(response)),
            Cors::Headers(headers) => {
                self.cors_headers = headers;
                Ok(())
            }
        }
    }

    fn admin_gate(&self, path: &str) -> Result<(), Response> {
        if !self.app.shared.admin_api() && is_admin_path(path) {
            return Err(self.finish(empty(StatusCode::NOT_FOUND)));
        }
        Ok(())
    }

    async fn run(mut self, mut request: Request) -> Response {
        let backend = self.app.service == Service::Backend;
        let mut origin_host = None;
        if backend {
            let target = request.uri().path_and_query().map_or("/", |p| p.as_str()).to_owned();
            match strip_origin(&target) {
                OriginStrip::Unchanged => {}
                OriginStrip::Refused => return empty(StatusCode::BAD_REQUEST),
                OriginStrip::Stripped { host, target } => match http::Uri::try_from(target) {
                    Ok(uri) => {
                        *request.uri_mut() = uri;
                        origin_host = Some(host);
                    }
                    Err(_) => return empty(StatusCode::BAD_REQUEST),
                },
            }
            if is_hunts_path(request.uri().path())
                && let Err(refused) = self.count(Probe::of(&request)).await
            {
                return refused;
            }
        }
        let path = request.uri().path().to_owned();
        let (mut at_budget, mut at_admin) = (!backend, !backend);
        let app = self.app;
        for mounted in &app.mounted {
            if !at_budget && mounted.index >= app.budget_mark {
                at_budget = true;
                // The probe is taken first: a request ref may not live across the count's await
                if let Err(response) = self.count(Probe::of(&request)).await {
                    return response;
                }
                if let Err(response) = self.cors(&request) {
                    return response;
                }
            }
            if !at_admin && mounted.index >= app.admin_mark {
                at_admin = true;
                if let Err(response) = self.admin_gate(&path) {
                    return response;
                }
            }
            if !mounted.row.method.accepts(request.method()) {
                continue;
            }
            let params = match mounted.pattern.matches(&path) {
                None => continue,
                Some(Err(_)) => return self.finish(empty(StatusCode::BAD_REQUEST)),
                Some(Ok(params)) => params,
            };
            if mounted.row.when == When::Gauntlet && !origin_host.as_deref().is_some_and(|h| h.contains("gauntlet")) {
                continue;
            }
            let response = self.answer(mounted, params, request, origin_host).await;
            return self.finish(response);
        }
        if !at_budget {
            if let Err(response) = self.count(Probe::of(&request)).await {
                return response;
            }
            if let Err(response) = self.cors(&request) {
                return response;
            }
        }
        if !at_admin && let Err(response) = self.admin_gate(&path) {
            return response;
        }
        tracing::warn!("Unstubbed route {} {}", request.method(), path);
        self.finish(empty(StatusCode::NOT_FOUND))
    }

    async fn answer(
        &self,
        mounted: &Mounted,
        params: Params,
        request: Request,
        origin_host: Option<String>,
    ) -> Response {
        match &mounted.answer {
            Answer::Handler(handler) => {
                handler(Call {
                    shared: Arc::clone(&self.app.shared),
                    route: mounted.row,
                    params,
                    request,
                    origin_host,
                    request_id: self.request_id.clone(),
                })
                .await
            }
            Answer::NotImplemented => not_implemented(mounted.row, &self.request_id),
            Answer::Rifts(position, route) => self.relay(mounted.row, *position, route, params, request).await,
        }
    }

    async fn caller(
        &self,
        row: &RouteDef,
        headers: &HeaderMap,
        peer: Option<IpAddr>,
    ) -> Result<rifts_seam::Caller, Response> {
        let auth = self.app.shared.auth();
        let refuse = |error: LauncherError| error.response(&replay::next_uuid());
        match rustic_isles_auth::rifts_caller_kind(row.method.as_str(), row.path) {
            CallerKind::None | CallerKind::HostCapability => Ok(rifts_seam::Caller::Anonymous),
            CallerKind::Launcher { active } => {
                let caller = auth.launcher_auth(headers).await.map_err(refuse)?;
                if let Some(consent) = active {
                    auth.require_active_account(&caller.user_id, consent)
                        .await
                        .map_err(refuse)?;
                }
                Ok(rifts_seam::Caller::Launcher(rifts_seam::AccountId(caller.user_id)))
            }
            CallerKind::Admin => {
                let Some(authenticator) = self.app.shared.admin_authenticator() else {
                    return Err(crate::shared::admin_refusal());
                };
                let admin = authenticator.authenticate(headers, peer).await?;
                Ok(rifts_seam::Caller::Admin { key_id: admin.key_id })
            }
        }
    }

    async fn relay(
        &self,
        row: &RouteDef,
        position: usize,
        route: &'static rifts_seam::Route,
        params: Params,
        request: Request,
    ) -> Response {
        let remote = rustic_isles_auth::client_ip(&request)
            .parse()
            .ok()
            .or_else(|| peer(&request));
        let peer = peer(&request);
        let (parts, body) = request.into_parts();
        let caller = match self.caller(row, &parts.headers, peer).await {
            Ok(caller) => caller,
            Err(response) => return response,
        };
        let headers = parts
            .headers
            .iter()
            .filter_map(|(name, value)| Some((name.as_str().to_owned(), value.to_str().ok()?.to_owned())))
            .collect();
        let body = match http_body_util::Limited::new(body, BODY_LIMIT).collect().await {
            Ok(collected) => collected.to_bytes().to_vec(),
            Err(e) if e.downcast_ref::<http_body_util::LengthLimitError>().is_some() => {
                return empty(StatusCode::PAYLOAD_TOO_LARGE);
            }
            Err(_) => return empty(StatusCode::BAD_REQUEST),
        };
        let call = rifts_seam::RouteCall {
            route: *route,
            params: params.into_pairs(),
            query: parts.uri.query().unwrap_or("").to_owned(),
            request: rifts_seam::RequestInfo {
                path: parts.uri.path().to_owned(),
                headers,
                remote,
                request_id: self.request_id.clone(),
            },
            caller,
            body,
        };
        let rifts = self.app.shared.rifts();
        let Some((_, handler)) = rifts.routes().nth(position) else {
            return empty(StatusCode::INTERNAL_SERVER_ERROR);
        };
        let reply = match handler.handle(call).await {
            Ok(reply) => reply,
            Err(rifts_seam::SeamError::Unimplemented(missing)) => {
                rifts_seam::Reply::not_implemented(&missing, &self.request_id)
            }
            Err(rifts_seam::SeamError::Refused(refusal)) => {
                let status = rustic_isles_auth::status_for(&refusal.code);
                let status = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
                let body = error_envelope(&refusal.code, &refusal.message, &self.request_id);
                return json(status, &body);
            }
            Err(error) => {
                tracing::error!("rifts route {}: {error}", row.name());
                return empty(StatusCode::INTERNAL_SERVER_ERROR);
            }
        };
        let status = StatusCode::from_u16(reply.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        (status, [(header::CONTENT_TYPE, JSON_UTF8)], reply.body.0).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stripped(host: &str, target: &str) -> OriginStrip {
        OriginStrip::Stripped {
            host: host.to_owned(),
            target: target.to_owned(),
        }
    }

    #[test]
    fn origin_strip() {
        assert_eq!(
            strip_origin("/__origin/Gauntlet-Prod.steelyard.ca/config?x=1"),
            stripped("gauntlet-prod.steelyard.ca", "/config?x=1")
        );
        assert_eq!(
            strip_origin("/__origin/api.epicgames.dev"),
            stripped("api.epicgames.dev", "/")
        );
        assert_eq!(
            strip_origin("/__origin/steelyard.ca?x=1"),
            stripped("steelyard.ca", "/?x=1")
        );
        for refused in [
            "/__origin/evil.example/x",
            "/__origin/steelyard.ca.evil/x",
            "/__origin/a%ZZ.steelyard.ca/x",
            "/__origin/x:1@steelyard.ca/x",
            "/__origin/.steelyard.ca/x",
            "/__origin/a..steelyard.ca/x",
            "/__origin/xsteelyard.ca/x",
        ] {
            assert_eq!(strip_origin(refused), OriginStrip::Refused, "{refused}");
        }
        assert_eq!(strip_origin("/character"), OriginStrip::Unchanged);
        assert_eq!(strip_origin("/__origin//x"), OriginStrip::Unchanged);
    }

    #[test]
    fn request_ids() {
        assert_eq!(safe_request_id(" abcd-1234 "), Some("abcd-1234"));
        assert_eq!(safe_request_id("short"), None);
        assert_eq!(safe_request_id("-abcdefgh"), None);
        assert_eq!(safe_request_id(&"a".repeat(97)), None);
        let mut headers = HeaderMap::new();
        headers.insert(REQUEST_ID_HEADER, "short".parse().unwrap());
        headers.insert("x-request-id", "fallback-1".parse().unwrap());
        assert_eq!(request_id_of(&headers), "fallback-1");
        headers.remove("x-request-id");
        assert_eq!(request_id_of(&headers).len(), 36);
    }

    #[test]
    fn prefixes() {
        assert!(is_admin_path("/Admin") && is_admin_path("/ADMIN/v1") && !is_admin_path("/administrator"));
        assert!(is_hunts_path("/Mystic/V1/Hunts/x") && !is_hunts_path("/mystic/v1/huntsx"));
    }
}
