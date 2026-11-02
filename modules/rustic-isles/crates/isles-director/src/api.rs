//! The Director's HTTP API (`src/app.ts`, `src/routes/matchmaker.ts`), on loopback for the backend only:
//! it has no authentication. The route table and the pipeline are isles-core's; this registers the four
//! isles handlers. The rifts route (`resolve-hunt-launch-manifest`) is answered by rustic-rifts.
//!
//! Where it differs from Express's order: the three read-only GETs come before the request context in
//! Node, so they carry no request id; isles-core's pipeline gives every answer one.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{FromRequestParts, Request};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, Method as HttpMethod, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use isles_core::http::{Call, Handlers, MountError, Shared};
use isles_core::routes::{Method, Service};
use serde_json::json;
use service_kit::body::{BodyConfig, ExpressBody, parse_bodies};
use service_kit::json::Json;

use crate::gameservers::Director;
use crate::log;
use crate::matchmaker;

/// `express.json()`'s default limit.
const JSON_LIMIT: usize = 100 * 1024;

fn strip_head(method: &HttpMethod, mut response: Response) -> Response {
    if method == HttpMethod::HEAD {
        *response.body_mut() = Body::empty();
    }
    response
}

/// Registers the Director's isles routes over `director`.
pub fn register(director: &Arc<Director>, handlers: &mut Handlers) {
    let d = director.clone();
    handlers.on(Service::Director, Method::Get, "/health/ready", move |call| {
        let d = d.clone();
        Box::pin(async move {
            let method = call.request.method().clone();
            let ready = d.persistent_hubs_ready();
            let status = if ready {
                StatusCode::OK
            } else {
                StatusCode::SERVICE_UNAVAILABLE
            };
            strip_head(
                &method,
                (status, Json(json!({"ok": ready, "service": "deployserver"}))).into_response(),
            )
        })
    });
    // 1.14.7 FENCING: each hub is null unless ready and its process still alive.
    let d = director.clone();
    handlers.on(Service::Director, Method::Get, "/api/hub-status", move |call| {
        let d = d.clone();
        Box::pin(async move {
            let method = call.request.method().clone();
            strip_head(&method, Json(d.persistent_hub_status()).into_response())
        })
    });
    // The running game servers, for the backend's launcher status.
    let d = director.clone();
    handlers.on(Service::Director, Method::Get, "/api/instances", move |call| {
        let d = d.clone();
        Box::pin(async move {
            let method = call.request.method().clone();
            strip_head(&method, Json(d.instances_for_status()).into_response())
        })
    });
    let d = director.clone();
    handlers.on(
        Service::Director,
        Method::Post,
        "/api/matchmaker/handle-matchmaking-for-player",
        move |call| {
            let d = d.clone();
            Box::pin(async move { matchmake(&d, call).await })
        },
    );
}

async fn matchmake(director: &Arc<Director>, call: Call) -> Response {
    let (mut parts, _) = call.request.into_parts();
    let body = ExpressBody::from_request_parts(&mut parts, &())
        .await
        .unwrap_or(ExpressBody(None));
    match matchmaker::handle(director, body.0.as_ref()).await {
        Ok(details) => Json(details).into_response(),
        Err(error) => {
            // The final safety net: the metagame treats a non-200 as a failed matchmaking.
            log::error(&format!("handle-matchmaking-for-player failed: {error}"));
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "matchmaking_failed"})),
            )
                .into_response()
        }
    }
}

/// Express's own answer to a path no route took (`finalhandler`): a small HTML page naming the request.
fn cannot(method: &HttpMethod, path: &str) -> Response {
    let escaped = format!("Cannot {method} {path}")
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;");
    let body = format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>Error</title>\n</head>\n<body>\n<pre>{escaped}</pre>\n</body>\n</html>\n"
    );
    let mut response = (StatusCode::NOT_FOUND, body).into_response();
    let headers = response.headers_mut();
    headers.insert(
        "content-security-policy",
        HeaderValue::from_static("default-src 'none'"),
    );
    headers.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    response
}

/// The Director has no catch-all, so an unmatched path gets Express's page, not the backend's empty 404. None of its
/// handlers answers 404, so an empty one is the pipeline's.
async fn express_404(request: Request, next: Next) -> Response {
    let (method, path) = (request.method().clone(), request.uri().path().to_owned());
    let response = next.run(request).await;
    if response.status() == StatusCode::NOT_FOUND {
        let mut page = strip_head(&method, cannot(&method, &path));
        // The pipeline's request id stays on the page
        if let Some(id) = response.headers().get(isles_core::http::REQUEST_ID_HEADER) {
            page.headers_mut()
                .insert(isles_core::http::REQUEST_ID_HEADER, id.clone());
        }
        return page;
    }
    response
}

/// The Director's router: its handlers over isles-core's pipeline, with the JSON and form parsers
/// (`express.json()`, 100 kB) and Express's 404.
pub fn router(shared: Arc<Shared>, director: &Arc<Director>) -> Result<axum::Router, MountError> {
    let mut handlers = Handlers::default();
    register(director, &mut handlers);
    let app = isles_core::http::app(shared, Service::Director, &handlers)?;
    Ok(app
        .layer(middleware::from_fn_with_state(
            BodyConfig {
                limit: JSON_LIMIT,
                ..BodyConfig::default()
            },
            parse_bodies,
        ))
        .layer(middleware::from_fn(express_404)))
}
