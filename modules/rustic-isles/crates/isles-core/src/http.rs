//! The services' HTTP pipeline, in `app.ts`'s order: the request id, the `/__origin/<host>/` strip, the routes in
//! match order with their mount conditions and the `/admin` gate, then the unstubbed 404. A route no area has a
//! handler for answers 501 in the launcher envelope (RU4); rustic-rifts answers its own routes when loaded (RF2).
//!
//! Not here yet, each its RU5 area's: body parsers and their limits, the request budget (429), the request log and
//! metrics, auth middleware, CORS and Express's automatic `OPTIONS` answers.

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::{Body, to_bytes};
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{HeaderName, HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use module_api::{BoxFuture, NodeIdentity};
use percent_encoding::percent_decode_str;
use rifts_seam::{Caller, Reply, RequestInfo, RouteCall, RouteHandler, SeamError};
use serde_json::json;
use service_kit::json::Json;
use service_kit::serve::ConnectionInfo;

use crate::db::IslesStore;
use crate::game_data::GameDataRoot;
use crate::routes::{Method, Params, Pattern, PatternError, RouteDef, Service, When};
use crate::settings::Settings;
use crate::version::IslesVersion;

pub use rifts_seam::Rifts;

/// `x-mysticparadox-request-id` (`observability/requestContext.ts`): taken from the request when it is safe, else a
/// new UUID, and echoed on the answer.
pub const REQUEST_ID_HEADER: &str = "x-mysticparadox-request-id";

/// Express's default JSON body limit; the routes with their own parsers set theirs when ported.
const BODY_LIMIT: usize = 100 * 1024;

/// What every handler of a module shares.
pub struct Shared {
    pub version: IslesVersion,
    pub settings: Settings,
    pub node: NodeIdentity,
    pub game_data: GameDataRoot,
    /// `None` until a database is configured (`REFORGED_DB_FILE` or `REFORGED_DB_ENGINE`)
    pub store: Option<IslesStore>,
    pub rifts: Rifts,
    /// `MYSTICPARADOX_ADMIN_API`, read once at start as Node does
    pub admin_api: bool,
    /// Matchmaking's in-memory state (DD15): queues, candidates, presence
    pub matchmaking: Arc<crate::areas::matchmaking::state::MatchmakingState>,
}

impl Shared {
    /// A module with nothing configured: no database, no rifts, admin API off.
    pub fn bare(version: IslesVersion) -> Self {
        Self {
            version,
            settings: Settings::default(),
            node: NodeIdentity::standalone(),
            game_data: GameDataRoot::default(),
            store: None,
            rifts: Rifts::dedicated_only(),
            admin_api: false,
            matchmaking: Arc::default(),
        }
    }
}

/// One matched request.
pub struct Call {
    pub shared: Arc<Shared>,
    pub route: &'static RouteDef,
    pub params: Params,
    pub request: Request,
    /// The decoded `/__origin/<host>` the runtime prefixed, lower-case
    pub origin_host: Option<String>,
    pub request_id: String,
}

pub type Handler = Arc<dyn Fn(Call) -> BoxFuture<'static, Response> + Send + Sync>;

/// Handlers by route; an area registers its own with [`Handlers::on`].
#[derive(Default, Clone)]
pub struct Handlers {
    map: HashMap<(Service, Method, &'static str), Handler>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MountError {
    #[error("{0}")]
    Pattern(#[from] PatternError),
    #[error("a handler for {0}, which the route table doesn't have")]
    UnknownRoute(String),
    #[error("rustic-rifts is loaded but answers no {0}")]
    NoRiftsHandler(String),
}

impl Handlers {
    pub fn on<F>(&mut self, service: Service, method: Method, path: &'static str, handler: F) -> &mut Self
    where
        F: Fn(Call) -> BoxFuture<'static, Response> + Send + Sync + 'static,
    {
        self.map.insert((service, method, path), Arc::new(handler));
        self
    }

    pub fn get(&self, route: &RouteDef) -> Option<&Handler> {
        self.map.get(&(route.service, route.method, route.path))
    }

    /// Every registered route must be in the table: a handler for a path nothing mounts is a typo.
    pub fn check(&self) -> Result<(), MountError> {
        for (service, method, path) in self.map.keys() {
            let known = crate::routes::all()
                .iter()
                .any(|r| r.service == *service && r.method == *method && r.path == *path);
            if !known {
                return Err(MountError::UnknownRoute(format!(
                    "{} {} {path}",
                    service.as_str(),
                    method.as_str()
                )));
            }
        }
        Ok(())
    }
}

enum Answer {
    Isles(Option<Handler>),
    /// The route's place in `Rifts::routes()`
    Rifts(usize),
}

struct Mounted {
    def: &'static RouteDef,
    pattern: Pattern,
    answer: Answer,
    /// Index in the whole table, for the `/admin` gate's place
    position: usize,
}

struct App {
    shared: Arc<Shared>,
    service: Service,
    routes: Vec<Mounted>,
    /// Where `app.use("/admin", ...)` sits: the first `/admin/` route's position in the table
    admin_gate: Option<usize>,
}

/// The rifts route that answers a table row, by method and path (rifts' paths are RU3a's).
fn rifts_answer(rifts: &Rifts, def: &RouteDef) -> Option<usize> {
    rifts
        .routes()
        .position(|(route, _)| route.method.as_str() == def.method.as_str() && route.path == def.path)
}

/// Builds one service's axum app: `service`'s routes of the module's version, in match order.
pub fn app(shared: Arc<Shared>, service: Service, handlers: &Handlers) -> Result<axum::Router, MountError> {
    handlers.check()?;
    let table = crate::routes::all();
    let mut routes = Vec::new();
    for (position, def) in table.iter().enumerate() {
        if def.service != service || !shared.version.routes.serves(def) {
            continue;
        }
        let answer = match def.when {
            When::Admin if !shared.admin_api => continue,
            When::Rifts if !shared.rifts.loaded() => continue,
            When::Rifts => match handlers.get(def) {
                Some(handler) => Answer::Isles(Some(handler.clone())),
                None => Answer::Rifts(
                    rifts_answer(&shared.rifts, def).ok_or_else(|| MountError::NoRiftsHandler(def.name()))?,
                ),
            },
            _ => Answer::Isles(handlers.get(def).cloned()),
        };
        routes.push(Mounted {
            def,
            pattern: Pattern::parse(def.path)?,
            answer,
            position,
        });
    }
    let admin_gate = (service == Service::Backend)
        .then(|| {
            table
                .iter()
                .position(|r| r.service == Service::Backend && r.path.starts_with("/admin/"))
        })
        .flatten();
    let app = Arc::new(App {
        shared,
        service,
        routes,
        admin_gate,
    });
    Ok(axum::Router::new().fallback(dispatch).with_state(app))
}

fn request_id(request: &Request) -> String {
    // NormalizeRequestId: 8 to 96 characters, a letter or digit first, then letters, digits and ._:-
    let safe = |value: &str| {
        let value = value.trim();
        (8..=96).contains(&value.len())
            && value.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
            && value
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '-'))
    };
    [REQUEST_ID_HEADER, "x-request-id"]
        .iter()
        .filter_map(|name| request.headers().get(*name)?.to_str().ok())
        .find(|value| safe(value))
        .map(|value| value.trim().to_owned())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
}

const ORIGIN_APEXES: [&str; 4] = [
    "steelyard.ca",
    "steelyard.online",
    "ol.epicgames.com",
    "api.epicgames.dev",
];

/// `isAllowedOriginHost` (`app.ts`): a bare host name under one of the runtime's redirect apexes.
fn allowed_origin_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
        && !host.starts_with('.')
        && !host.ends_with('.')
        && !host.contains("..")
        && ORIGIN_APEXES
            .iter()
            .any(|apex| host == *apex || host.ends_with(&format!(".{apex}")))
}

/// `decodeURIComponent`, which throws on a `%` not followed by two hex digits and on bytes that aren't UTF-8.
fn decode_uri_component(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let well_formed = bytes.iter().enumerate().all(|(i, &b)| {
        b != b'%'
            || (bytes.get(i + 1).is_some_and(u8::is_ascii_hexdigit)
                && bytes.get(i + 2).is_some_and(u8::is_ascii_hexdigit))
    });
    if !well_formed {
        return None;
    }
    percent_decode_str(text)
        .decode_utf8()
        .ok()
        .map(|text| text.into_owned())
}

/// The `/__origin/<host>/` strip: the runtime's `SetURL` hook prefixes the official host to the path. `Err` is
/// Node's empty 400.
fn strip_origin(uri: &Uri) -> Result<Option<(String, Uri)>, ()> {
    let url = uri.path_and_query().map_or("/", |pq| pq.as_str());
    let Some(rest) = url.strip_prefix("/__origin/") else {
        return Ok(None);
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    if end == 0 {
        return Ok(None);
    }
    let host = decode_uri_component(&rest[..end]).ok_or(())?.to_lowercase();
    if !allowed_origin_host(&host) {
        return Err(());
    }
    let tail = &rest[end..];
    let tail = if tail.is_empty() {
        "/".to_owned()
    } else if tail.starts_with('/') {
        tail.to_owned()
    } else {
        format!("/{tail}")
    };
    let uri = tail.parse::<Uri>().map_err(|_| ())?;
    Ok(Some((host, uri)))
}

/// `/admin` and everything under it, as `app.use("/admin", ...)` matches.
fn under_admin(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower == "/admin" || lower.starts_with("/admin/")
}

/// 501 in the launcher envelope (`SendLauncherError`), what RU4 answers on every route.
pub fn not_implemented(route: &RouteDef, request_id: &str) -> Response {
    let message = format!("isles: {} is not implemented yet", route.name());
    let body = json!({ "error": { "code": "NOT_IMPLEMENTED", "message": message, "requestId": request_id } });
    (StatusCode::NOT_IMPLEMENTED, Json(body)).into_response()
}

fn empty(status: StatusCode) -> Response {
    status.into_response()
}

async fn rifts_reply(handler: &dyn RouteHandler, route: rifts_seam::Route, call: Call) -> Response {
    let remote = call
        .request
        .extensions()
        .get::<ConnectInfo<ConnectionInfo>>()
        .map(|info| info.0.peer.ip());
    let headers = call
        .request
        .headers()
        .iter()
        .filter_map(|(name, value)| Some((name.as_str().to_owned(), value.to_str().ok()?.to_owned())))
        .collect();
    let path = call.request.uri().path().to_owned();
    let query = call.request.uri().query().unwrap_or_default().to_owned();
    let Ok(body) = to_bytes(call.request.into_body(), BODY_LIMIT).await else {
        return empty(StatusCode::PAYLOAD_TOO_LARGE);
    };
    let rifts_call = RouteCall {
        route,
        params: call.params.0,
        query,
        request: RequestInfo {
            path,
            headers,
            remote,
            request_id: call.request_id.clone(),
        },
        // The auth middleware that fills this is RU5 auth's
        caller: Caller::Anonymous,
        body: body.to_vec(),
    };
    let reply = match handler.handle(rifts_call).await {
        Ok(reply) => reply,
        Err(SeamError::Unimplemented(unimplemented)) => Reply::not_implemented(&unimplemented, &call.request_id),
        Err(SeamError::Refused(refusal)) => {
            // The status a code is registered under (security/launcherErrors.ts) is RU5 auth's table
            let body =
                json!({ "error": { "code": refusal.code, "message": refusal.message, "requestId": call.request_id } });
            return (StatusCode::BAD_REQUEST, Json(body)).into_response();
        }
        Err(other) => {
            tracing::error!("rifts route {}: {other}", call.route.name());
            return empty(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };
    let status = StatusCode::from_u16(reply.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (
        status,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static(service_kit::json::JSON_UTF8),
        )],
        reply.body.0,
    )
        .into_response()
}

async fn dispatch(State(app): State<Arc<App>>, mut request: Request) -> Response {
    let id = request_id(&request);
    let observation = crate::areas::reforged_api::observe::begin(&request, &id);
    let mut response = answer(&app, &mut request, id.clone()).await;
    observation.finish(response.status().as_u16());
    if let Ok(value) = HeaderValue::from_str(&id) {
        response
            .headers_mut()
            .insert(HeaderName::from_static(REQUEST_ID_HEADER), value);
    }
    response
}

async fn answer(app: &Arc<App>, request: &mut Request, request_id: String) -> Response {
    let mut origin_host = None;
    if app.service == Service::Backend {
        match strip_origin(request.uri()) {
            Ok(Some((host, uri))) => {
                origin_host = Some(host);
                *request.uri_mut() = uri;
            }
            Ok(None) => {}
            Err(()) => return empty(StatusCode::BAD_REQUEST),
        }
    }
    let path = request.uri().path().to_owned();
    let mut gate_passed = false;
    for mounted in &app.routes {
        if !gate_passed && app.admin_gate.is_some_and(|gate| mounted.position >= gate) {
            gate_passed = true;
            if !app.shared.admin_api && under_admin(&path) {
                return empty(StatusCode::NOT_FOUND);
            }
        }
        if !mounted.def.method.accepts(request.method()) {
            continue;
        }
        let params = match mounted.pattern.matches(&path) {
            None => continue,
            Some(Err(_)) => return empty(StatusCode::BAD_REQUEST),
            Some(Ok(params)) => params,
        };
        // Node's Gauntlet handlers call next() for any other origin, so they never shadow a 1.12 route
        if mounted.def.when == When::Gauntlet && !origin_host.as_deref().is_some_and(|h| h.contains("gauntlet")) {
            continue;
        }
        let call = Call {
            shared: app.shared.clone(),
            route: mounted.def,
            params,
            request: std::mem::replace(request, Request::new(Body::empty())),
            origin_host,
            request_id: request_id.clone(),
        };
        return match &mounted.answer {
            Answer::Isles(Some(handler)) => handler(call).await,
            Answer::Isles(None) => not_implemented(mounted.def, &request_id),
            Answer::Rifts(index) => match app.shared.rifts.routes().nth(*index) {
                Some((route, handler)) => rifts_reply(handler, *route, call).await,
                None => empty(StatusCode::INTERNAL_SERVER_ERROR),
            },
        };
    }
    if !gate_passed && !app.shared.admin_api && app.admin_gate.is_some() && under_admin(&path) {
        return empty(StatusCode::NOT_FOUND);
    }
    tracing::warn!("Unstubbed route {} {path}", request.method());
    empty(StatusCode::NOT_FOUND)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_origin_strip_matches_node() {
        let strip = |url: &str| strip_origin(&url.parse::<Uri>().unwrap());
        let (host, uri) = strip("/__origin/Gauntlet-Prod.steelyard.ca/config?x=1")
            .unwrap()
            .unwrap();
        assert_eq!(
            (host.as_str(), uri.to_string().as_str()),
            ("gauntlet-prod.steelyard.ca", "/config?x=1")
        );
        let (_, uri) = strip("/__origin/api.epicgames.dev").unwrap().unwrap();
        assert_eq!(uri.path(), "/");
        assert!(strip("/__origin/evil.example/x").is_err());
        assert!(strip("/__origin/steelyard.ca.evil/x").is_err());
        assert!(strip("/__origin/a%ZZ.steelyard.ca/x").is_err());
        assert!(strip("/__origin/x:1@steelyard.ca/x").is_err());
        assert!(strip("/character").unwrap().is_none());
    }

    #[test]
    fn request_ids_are_kept_only_when_safe() {
        let with = |value: &str| {
            let request = Request::builder()
                .header(REQUEST_ID_HEADER, value)
                .body(Body::empty())
                .unwrap();
            request_id(&request)
        };
        assert_eq!(with("abcd-1234"), "abcd-1234");
        assert_ne!(with("short"), "short");
        assert_ne!(with("-abcdefgh"), "-abcdefgh");
        assert_eq!(with("-abcdefgh").len(), 36);
    }
}
