//! `express.json()` and `express.urlencoded()` for the requests this area serves: what `req.body` holds. Express runs
//! them before routing, so a malformed body is refused even by a route that never reads it; the pipeline here is the
//! areas', so this runs first in each handler, before authentication.

// An early answer is the whole Response; boxing it would cost every caller for no gain
#![allow(clippy::result_large_err)]

use axum::extract::Request;
use axum::http::header::{CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, TRANSFER_ENCODING};
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use serde_json::Value;
use service_kit::body::{BodyError, parse_form, parse_json};

/// Express's default limit for both parsers (`100kb`).
pub const LIMIT: usize = 100 * 1024;

/// The request with its body read: `req.body` is `None` when no parser applied (`undefined` in Express 5).
pub async fn read(request: Request) -> Result<(Parts, Option<Value>), Response> {
    let (parts, body) = request.into_parts();
    let headers = &parts.headers;
    // body-parser skips a request with neither a Content-Length nor a Transfer-Encoding (`typeis.hasBody`)
    let has_body = headers.contains_key(TRANSFER_ENCODING)
        || headers
            .get(CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.trim().parse::<f64>().is_ok());
    let Some(content_type) = headers.get(CONTENT_TYPE).and_then(|v| v.to_str().ok()) else {
        return Ok((parts, None));
    };
    let mut pieces = content_type.split(';');
    let essence = pieces.next().unwrap_or("").trim().to_ascii_lowercase();
    let charset = pieces
        .filter_map(|p| p.split_once('='))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("charset"))
        .map(|(_, value)| value.trim().trim_matches('"').to_ascii_lowercase());
    let json = match essence.as_str() {
        "application/json" => true,
        "application/x-www-form-urlencoded" => false,
        _ => return Ok((parts, None)),
    };
    if !has_body {
        return Ok((parts, None));
    }
    if charset.as_deref().is_some_and(|c| c != "utf-8") {
        return Err(BodyError::UnsupportedCharset.into_response());
    }
    // Compressed request bodies are not inflated here: the game client doesn't send them
    let encoding = headers
        .get(CONTENT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("identity")
        .trim()
        .to_ascii_lowercase();
    if encoding != "identity" {
        return Err(BodyError::UnsupportedEncoding.into_response());
    }
    let bytes = axum::body::to_bytes(body, LIMIT).await.map_err(|error| {
        // axum wraps http-body-util's LengthLimitError, which this crate doesn't depend on to name
        if std::iter::successors(Some(&error as &dyn std::error::Error), |e| e.source())
            .any(|e| e.to_string().contains("length limit"))
        {
            BodyError::TooLarge.into_response()
        } else {
            BodyError::Aborted.into_response()
        }
    })?;
    let text = std::str::from_utf8(&bytes).map_err(|_| BodyError::Malformed.into_response())?;
    let value = if json { parse_json(text) } else { parse_form(text) };
    value
        .map(|value| (parts, Some(value)))
        .map_err(IntoResponse::into_response)
}
