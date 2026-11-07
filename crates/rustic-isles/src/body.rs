// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `req.body` for handlers (core spec §3.9). The pipeline parses nothing, so a malformed body on a route that
//! never reads it passes (§15 item 4); a handler that reads one answers as Express's parsers do.

use axum::body::{Body, Bytes};
use axum::extract::Request;
use axum::response::{IntoResponse, Response};
use dainty_service_kit::body::{BodyError, ExpressBody};
use http::header;
use http::request::Parts;
use http_body_util::BodyExt;
use serde_json::{Map, Value};

/// 100 KiB, Express's default.
pub const BODY_LIMIT: usize = 100 * 1024;

#[derive(Clone, Copy)]
enum Kind {
    Json,
    Form,
}

/// `type-is`'s `hasBody`: a transfer encoding, a numeric length, or (HTTP/1.0 style) no `host`.
fn has_body(parts: &Parts) -> bool {
    let length = parts
        .headers
        .get(header::CONTENT_LENGTH)
        .is_some_and(|v| String::from_utf8_lossy(v.as_bytes()).trim().parse::<u64>().is_ok());
    parts.headers.contains_key(header::TRANSFER_ENCODING) || length || !parts.headers.contains_key(header::HOST)
}

fn kind(parts: &Parts) -> Result<Option<Kind>, BodyError> {
    let Some(raw) = parts.headers.get(header::CONTENT_TYPE) else {
        return Ok(None);
    };
    let text = String::from_utf8_lossy(raw.as_bytes());
    let mut params = text.split(';');
    let essence = params.next().unwrap_or("").trim();
    let kind = if essence.eq_ignore_ascii_case("application/json") {
        Kind::Json
    } else if essence.eq_ignore_ascii_case("application/x-www-form-urlencoded") {
        Kind::Form
    } else {
        return Ok(None);
    };
    for param in params {
        if let Some((name, value)) = param.split_once('=')
            && name.trim().eq_ignore_ascii_case("charset")
            && !value.trim().trim_matches('"').eq_ignore_ascii_case("utf-8")
        {
            return Err(BodyError::UnsupportedCharset);
        }
    }
    Ok(Some(kind))
}

fn parse_json(text: &str) -> Result<Value, BodyError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    if text.is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    let first = text.trim_start_matches([' ', '\t', '\n', '\r']);
    if !first.starts_with(['{', '[']) {
        return Err(BodyError::Malformed);
    }
    dainty_service_kit::jsjson::parse(text).map_err(|_| BodyError::Malformed)
}

async fn parse(parts: &Parts, body: Body, limit: usize) -> Result<(Option<Value>, Bytes), BodyError> {
    if !has_body(parts) {
        return Ok((None, Bytes::new()));
    }
    let Some(kind) = kind(parts)? else {
        return Ok((None, Bytes::new()));
    };
    if let Some(encoding) = parts.headers.get(header::CONTENT_ENCODING) {
        let encoding = String::from_utf8_lossy(encoding.as_bytes()).trim().to_owned();
        if !encoding.eq_ignore_ascii_case("identity") {
            return Err(BodyError::UnsupportedEncoding);
        }
    }
    let bytes = match http_body_util::Limited::new(body, limit).collect().await {
        Ok(collected) => collected.to_bytes(),
        Err(e) if e.downcast_ref::<http_body_util::LengthLimitError>().is_some() => {
            return Err(BodyError::TooLarge);
        }
        Err(_) => return Err(BodyError::Aborted),
    };
    let text = std::str::from_utf8(&bytes).map_err(|_| BodyError::Malformed)?;
    let value = match kind {
        Kind::Json => parse_json(text)?,
        Kind::Form => dainty_service_kit::body::parse_form(text)?,
    };
    Ok((Some(value), bytes))
}

/// The body under `limit`: `Ok(None)` when absent, else Express's error page.
pub async fn read_body_parts_limit(parts: &Parts, body: Body, limit: usize) -> Result<Option<Value>, Response> {
    if let Some(ExpressBody(parsed)) = parts.extensions.get::<ExpressBody>() {
        return Ok(parsed.clone());
    }
    parse(parts, body, limit)
        .await
        .map(|(value, _)| value)
        .map_err(|e| dainty_service_kit::body::error_page(e.status()))
}

/// [`read_body_parts_limit`] at the default limit.
pub async fn read_body_parts(parts: &Parts, body: Body) -> Result<Option<Value>, Response> {
    read_body_parts_limit(parts, body, BODY_LIMIT).await
}

/// Parses once and leaves the result in the request's extensions (and the bytes in its body) for later readers.
pub async fn read_body(request: &mut Request) -> Result<Option<Value>, Response> {
    if let Some(ExpressBody(parsed)) = request.extensions().get::<ExpressBody>() {
        return Ok(parsed.clone());
    }
    let taken = std::mem::replace(request, Request::new(Body::empty()));
    let (mut parts, body) = taken.into_parts();
    match parse(&parts, body, BODY_LIMIT).await {
        Ok((value, bytes)) => {
            parts.extensions.insert(ExpressBody(value.clone()));
            *request = Request::from_parts(parts, Body::from(bytes));
            Ok(value)
        }
        Err(e) => {
            *request = Request::from_parts(parts, Body::empty());
            Err(e.into_response())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::StatusCode;

    fn request(content_type: Option<&str>, body: &str) -> Request {
        let mut builder = Request::builder().method("POST").uri("/x").header("host", "h");
        if let Some(ct) = content_type {
            builder = builder.header("content-type", ct);
        }
        builder
            .header("content-length", body.len().to_string())
            .body(Body::from(body.to_owned()))
            .unwrap()
    }

    async fn status(content_type: Option<&str>, body: &str) -> Result<Option<Value>, StatusCode> {
        read_body(&mut request(content_type, body))
            .await
            .map_err(|r| r.status())
    }

    #[tokio::test]
    async fn express_rules() {
        let json = Some("application/json");
        assert_eq!(status(json, "{\"a\":1.0}").await, Ok(Some(serde_json::json!({"a": 1}))));
        assert_eq!(status(json, "\u{feff}").await, Ok(Some(serde_json::json!({}))));
        assert_eq!(status(json, "").await, Ok(Some(serde_json::json!({}))));
        assert_eq!(status(json, "1").await, Err(StatusCode::BAD_REQUEST));
        assert_eq!(status(Some("text/plain"), "x").await, Ok(None));
        assert_eq!(status(None, "x").await, Ok(None));
        assert_eq!(
            status(Some("application/json; charset=latin1"), "{}").await,
            Err(StatusCode::UNSUPPORTED_MEDIA_TYPE)
        );
        assert_eq!(
            status(Some("application/x-www-form-urlencoded"), "a=b+c").await,
            Ok(Some(serde_json::json!({"a": "b c"})))
        );
        let big = format!("[\"{}\"]", "x".repeat(BODY_LIMIT));
        assert_eq!(status(json, &big).await, Err(StatusCode::PAYLOAD_TOO_LARGE));
        let mut gz = request(json, "{}");
        gz.headers_mut().insert("content-encoding", "gzip".parse().unwrap());
        assert_eq!(
            read_body(&mut gz).await.map_err(|r| r.status()),
            Err(StatusCode::UNSUPPORTED_MEDIA_TYPE)
        );
    }

    #[tokio::test]
    async fn a_parsed_body_is_kept() {
        let mut req = request(Some("application/json"), "{\"a\":1}");
        let first = read_body(&mut req).await.unwrap();
        assert_eq!(read_body(&mut req).await.unwrap(), first);
        let bytes = req.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(&bytes[..], b"{\"a\":1}");
    }
}
