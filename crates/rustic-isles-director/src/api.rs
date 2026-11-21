// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The `director` port's routes (spec §3): core's dispatcher inside the body parser and Express's 404 page.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::Request;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use dainty_service_kit::body::{BodyConfig, ExpressBody, parse_bodies};
use dainty_service_kit::json::Json;
use http::{HeaderValue, StatusCode, header};
use rustic_isles::{Call, Handlers, Method, MountError, REQUEST_ID_HEADER, Service, Shared};
use serde_json::{Map, Value};

use crate::gameservers::Director;

const BODY_LIMIT: usize = 100 * 1024;

fn json(status: StatusCode, value: Value) -> Response {
    let mut response = Json(value).into_response();
    *response.status_mut() = status;
    response
}

fn ready_body(ok: bool) -> Value {
    let mut out = Map::new();
    out.insert("ok".into(), Value::Bool(ok));
    out.insert("service".into(), Value::from("deployserver"));
    Value::Object(out)
}

/// The connection details: `host` left out when `MY_IP` is unset.
pub(crate) fn details_body(host: Option<String>, port: u16) -> Value {
    let mut out = Map::new();
    if let Some(host) = host {
        out.insert("host".into(), Value::String(host));
    }
    out.insert("port".into(), Value::from(port));
    Value::Object(out)
}

/// D1-D4 (D5 is rifts', through core).
pub fn register(director: &Arc<Director>, handlers: &mut Handlers) {
    let d = Arc::clone(director);
    handlers.on(Service::Director, Method::Get, "/health/ready", move |_: Call| {
        let ready = d.persistent_hubs_ready();
        async move {
            let status = if ready {
                StatusCode::OK
            } else {
                StatusCode::SERVICE_UNAVAILABLE
            };
            json(status, ready_body(ready))
        }
    });
    let d = Arc::clone(director);
    handlers.on(Service::Director, Method::Get, "/api/hub-status", move |_: Call| {
        let status = d.persistent_hub_status();
        async move { json(StatusCode::OK, status) }
    });
    let d = Arc::clone(director);
    handlers.on(Service::Director, Method::Get, "/api/instances", move |_: Call| {
        let instances = d.instances_for_status();
        async move { json(StatusCode::OK, instances) }
    });
    let d = Arc::clone(director);
    handlers.on(
        Service::Director,
        Method::Post,
        "/api/matchmaker/handle-matchmaking-for-player",
        move |call: Call| {
            let d = Arc::clone(&d);
            async move {
                let body = call.request.extensions().get::<ExpressBody>().and_then(|b| b.0.clone());
                match d.handle_matchmaking(body.as_ref()).await {
                    Ok((host, port)) => json(StatusCode::OK, details_body(host, port)),
                    Err(error) => {
                        tracing::error!("handle-matchmaking-for-player failed: {error}");
                        let mut out = Map::new();
                        out.insert("error".into(), Value::from("matchmaking_failed"));
                        json(StatusCode::INTERNAL_SERVER_ERROR, Value::Object(out))
                    }
                }
            }
        },
    );
}

fn html_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// Express's `finalhandler` page for any 404 from inside, keeping the request id.
async fn express_404(request: Request, next: Next) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let inner = next.run(request).await;
    if inner.status() != StatusCode::NOT_FOUND {
        return inner;
    }
    let text = html_escape(&format!("Cannot {method} {path}"));
    let page = format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>Error</title>\n</head>\n<body>\n<pre>{text}</pre>\n</body>\n</html>\n"
    );
    let body = if method == http::Method::HEAD {
        Body::empty()
    } else {
        Body::from(page)
    };
    let mut response = Response::new(body);
    *response.status_mut() = StatusCode::NOT_FOUND;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'"),
    );
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    if let Some(id) = inner.headers().get(REQUEST_ID_HEADER) {
        headers.insert(REQUEST_ID_HEADER, id.clone());
    }
    response
}

/// Core's app for the Director's rows, inside the body parser (100 KiB) and the 404 page.
pub fn router(shared: Arc<Shared>, director: &Arc<Director>) -> Result<axum::Router, MountError> {
    let mut handlers = Handlers::new();
    register(director, &mut handlers);
    let app = rustic_isles::app(shared, Service::Director, &handlers)?;
    let config = BodyConfig {
        limit: BODY_LIMIT,
        ..BodyConfig::default()
    };
    Ok(app
        .layer(middleware::from_fn_with_state(config, parse_bodies))
        .layer(middleware::from_fn(express_404)))
}
