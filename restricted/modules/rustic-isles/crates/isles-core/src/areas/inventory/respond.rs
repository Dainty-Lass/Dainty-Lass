//! Replies as Express writes them for this area's routes.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};
use service_kit::body::error_page;
use service_kit::json::Json;

use crate::repo::RepoError;

/// `res.status(status).json(body)`.
pub fn json(status: u16, body: Value) -> Response {
    (
        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        Json(body),
    )
        .into_response()
}

/// `res.status(status); res.send()`: no body.
pub fn empty(status: u16) -> Response {
    StatusCode::from_u16(status)
        .unwrap_or(StatusCode::INTERNAL_SERVER_ERROR)
        .into_response()
}

/// `res.json(undefined)`: `JSON.stringify(undefined)` is nothing, so Express sends 200 and an empty body that still
/// carries the JSON content type.
pub fn json_undefined() -> Response {
    let mut response = StatusCode::OK.into_response();
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static(service_kit::json::JSON_UTF8),
    );
    response
}

/// An error Node lets reach Express's final handler: the production error page.
pub fn server_error() -> Response {
    error_page(StatusCode::INTERNAL_SERVER_ERROR)
}

/// The launcher envelope (`SendLauncherError`) every not-yet-ported route answers with.
pub fn not_implemented(message: &str, request_id: &str) -> Response {
    json(
        501,
        json!({ "error": { "code": "NOT_IMPLEMENTED", "message": message, "requestId": request_id } }),
    )
}

/// A reply whose body is JSON text already (a rustic-rifts `Reply`).
pub fn raw_json(status: u16, text: String) -> Response {
    let mut response = (
        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        text,
    )
        .into_response();
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static(service_kit::json::JSON_UTF8),
    );
    response
}

/// A repository error: a port not wired yet is the 501 envelope, anything else a 500.
pub fn repo_error(error: &RepoError, request_id: &str) -> Response {
    match error {
        RepoError::Unimplemented { .. } => not_implemented(&format!("isles: {error}"), request_id),
        other => {
            tracing::error!("inventory: {other}");
            server_error()
        }
    }
}

/// `{code: null, message, payload: {}}`, the shape of this area's refusals.
pub fn refusal(status: u16, message: &str) -> Response {
    json(status, json!({ "code": null, "message": message, "payload": {} }))
}
