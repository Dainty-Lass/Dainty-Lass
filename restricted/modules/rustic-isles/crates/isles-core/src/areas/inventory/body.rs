//! `express.json()` and `express.urlencoded()` for the requests this area serves: delegates to [`crate::http::read_body_parts`].

// An early answer is the whole Response; boxing it would cost every caller for no gain
#![allow(clippy::result_large_err)]

use axum::extract::Request;
use axum::http::request::Parts;
use axum::response::Response;
use serde_json::Value;

pub use crate::http::BODY_LIMIT as LIMIT;

/// The request with its body read: `req.body` is `None` when no parser applied (`undefined` in Express 5).
pub async fn read(request: Request) -> Result<(Parts, Option<Value>), Response> {
    let (parts, body) = request.into_parts();
    crate::http::read_body_parts(parts, body).await
}
