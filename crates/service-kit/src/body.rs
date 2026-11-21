// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

use std::convert::Infallible;
use std::io::Read;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::{StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use flate2::read::{DeflateDecoder, GzDecoder, ZlibDecoder};
use http_body_util::{BodyExt, LengthLimitError, Limited};
use percent_encoding::percent_decode_str;
use serde_json::{Map, Value};

pub const DEFAULT_LIMIT: usize = 50 * 1024 * 1024;
const PARAMETER_LIMIT: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyError {
    Malformed,
    TooLarge,
    TooManyParameters,
    UnsupportedCharset,
    UnsupportedEncoding,
    Aborted,
}

impl BodyError {
    pub fn status(self) -> StatusCode {
        match self {
            Self::Malformed | Self::Aborted => StatusCode::BAD_REQUEST,
            Self::TooLarge | Self::TooManyParameters => StatusCode::PAYLOAD_TOO_LARGE,
            Self::UnsupportedCharset | Self::UnsupportedEncoding => StatusCode::UNSUPPORTED_MEDIA_TYPE,
        }
    }
}

impl IntoResponse for BodyError {
    fn into_response(self) -> Response {
        error_page(self.status())
    }
}

pub fn error_page(status: StatusCode) -> Response {
    let reason = status.canonical_reason().unwrap_or("Error");
    let html = format!(
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>Error</title>\n</head>\n<body>\n<pre>{reason}</pre>\n</body>\n</html>\n"
    );
    (
        status,
        [
            (header::CONTENT_SECURITY_POLICY, "default-src 'none'"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
        ],
        html,
    )
        .into_response()
}

pub type OnBodyError = dyn Fn(&Parts, BodyError) -> Option<Response> + Send + Sync;

#[derive(Clone)]
pub struct BodyConfig {
    pub limit: usize,
    pub on_error: Arc<OnBodyError>,
}

impl Default for BodyConfig {
    fn default() -> Self {
        Self {
            limit: DEFAULT_LIMIT,
            on_error: Arc::new(|_, _| None),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExpressBody(pub Option<Value>);

impl<S: Send + Sync> FromRequestParts<S> for ExpressBody {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(parts.extensions.get::<Self>().cloned().unwrap_or(Self(None)))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BodyKind {
    Json,
    Form,
}

fn inspect_content_type(headers: &axum::http::HeaderMap) -> Result<Option<BodyKind>, BodyError> {
    let Some(raw) = headers.get(header::CONTENT_TYPE) else {
        return Ok(None);
    };
    let Ok(text) = raw.to_str() else {
        return Ok(None);
    };
    let mut segments = text.split(';');
    let essence = segments.next().unwrap_or("").trim();
    let kind = if essence.eq_ignore_ascii_case("application/json") {
        BodyKind::Json
    } else if essence.eq_ignore_ascii_case("application/x-www-form-urlencoded") {
        BodyKind::Form
    } else {
        return Ok(None);
    };

    for param in segments {
        if let Some((k, v)) = param.split_once('=')
            && k.trim().eq_ignore_ascii_case("charset")
        {
            let charset = v.trim().trim_matches('"');
            if !charset.eq_ignore_ascii_case("utf-8") {
                return Err(BodyError::UnsupportedCharset);
            }
        }
    }

    Ok(Some(kind))
}

fn read_bounded(reader: impl Read, limit: usize) -> Result<Vec<u8>, BodyError> {
    let mut buf = Vec::new();
    let max = (limit as u64).saturating_add(1);
    reader
        .take(max)
        .read_to_end(&mut buf)
        .map_err(|_| BodyError::Malformed)?;
    if buf.len() > limit {
        return Err(BodyError::TooLarge);
    }
    Ok(buf)
}

fn decode_body_bytes(parts: &Parts, raw: &[u8], limit: usize) -> Result<Vec<u8>, BodyError> {
    let encoding = match parts.headers.get(header::CONTENT_ENCODING) {
        None => "identity",
        Some(val) => val.to_str().map_err(|_| BodyError::UnsupportedEncoding)?.trim(),
    };

    if encoding.is_empty() || encoding.eq_ignore_ascii_case("identity") {
        return Ok(raw.to_vec());
    }
    if encoding.eq_ignore_ascii_case("gzip") {
        return read_bounded(GzDecoder::new(raw), limit);
    }
    if encoding.eq_ignore_ascii_case("deflate") {
        return read_bounded(ZlibDecoder::new(raw), limit).or_else(|_| read_bounded(DeflateDecoder::new(raw), limit));
    }
    Err(BodyError::UnsupportedEncoding)
}

fn fail_response(parts: &Parts, config: &BodyConfig, err: BodyError) -> Response {
    (config.on_error)(parts, err).unwrap_or_else(|| err.into_response())
}

pub async fn parse_bodies(State(config): State<BodyConfig>, mut request: Request, next: Next) -> Response {
    let kind = match inspect_content_type(request.headers()) {
        Ok(Some(k)) => k,
        Ok(None) => {
            request.extensions_mut().insert(ExpressBody(None));
            return next.run(request).await;
        }
        Err(err) => {
            let (parts, _) = request.into_parts();
            return fail_response(&parts, &config, err);
        }
    };

    let (mut parts, body) = request.into_parts();
    let collected = match Limited::new(body, config.limit).collect().await {
        Ok(c) => c.to_bytes(),
        Err(err) => {
            let body_err = if err.downcast_ref::<LengthLimitError>().is_some() {
                BodyError::TooLarge
            } else {
                BodyError::Aborted
            };
            return fail_response(&parts, &config, body_err);
        }
    };

    let decoded = match decode_body_bytes(&parts, &collected, config.limit) {
        Ok(d) => d,
        Err(err) => return fail_response(&parts, &config, err),
    };

    let Ok(text) = std::str::from_utf8(&decoded) else {
        return fail_response(&parts, &config, BodyError::Malformed);
    };

    let parsed = match kind {
        BodyKind::Json => parse_json(text),
        BodyKind::Form => parse_form(text),
    };

    match parsed {
        Ok(val) => {
            parts.extensions.insert(ExpressBody(Some(val)));
            let rebuilt = Request::from_parts(parts, Body::from(collected));
            next.run(rebuilt).await
        }
        Err(err) => fail_response(&parts, &config, err),
    }
}

pub fn parse_json(text: &str) -> Result<Value, BodyError> {
    if text.is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    let trimmed = text.trim_start_matches([' ', '\t', '\n', '\r']);
    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        return Err(BodyError::Malformed);
    }
    serde_json::from_str(text).map_err(|_| BodyError::Malformed)
}

fn decode_form_part(raw: &str) -> String {
    let plus_spaced = raw.replace('+', " ");
    percent_decode_str(&plus_spaced)
        .decode_utf8()
        .map_or_else(|_| plus_spaced.clone(), std::borrow::Cow::into_owned)
}

pub fn parse_form(text: &str) -> Result<Value, BodyError> {
    let pairs: Vec<&str> = text.split('&').filter(|s| !s.is_empty()).collect();
    if pairs.len() > PARAMETER_LIMIT {
        return Err(BodyError::TooManyParameters);
    }

    let mut map = Map::new();
    for pair in pairs {
        let (raw_key, raw_val) = pair.split_once('=').unwrap_or((pair, ""));
        let key = decode_form_part(raw_key);
        if key.is_empty() {
            continue;
        }
        let val = Value::String(decode_form_part(raw_val));
        match map.entry(key) {
            serde_json::map::Entry::Vacant(e) => {
                e.insert(val);
            }
            serde_json::map::Entry::Occupied(mut e) => match e.get_mut() {
                Value::Array(arr) => arr.push(val),
                existing => {
                    let prev = existing.take();
                    *existing = Value::Array(vec![prev, val]);
                }
            },
        }
    }
    Ok(Value::Object(map))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_json_enforces_strict_mode_and_empty_object() {
        assert_eq!(parse_json("").unwrap(), json!({}));
        assert_eq!(parse_json("  {\"a\": 1}").unwrap(), json!({"a": 1}));
        assert_eq!(parse_json("\n[1, 2]").unwrap(), json!([1, 2]));
        assert_eq!(parse_json("\"scalar\""), Err(BodyError::Malformed));
        assert_eq!(parse_json("123"), Err(BodyError::Malformed));
        assert_eq!(parse_json("   "), Err(BodyError::Malformed));
    }

    #[test]
    fn parse_form_decodes_repeats_and_limits_params() {
        assert_eq!(
            parse_form("a=hello+world&b=1&b=2&=ignored&c").unwrap(),
            json!({"a": "hello world", "b": ["1", "2"], "c": ""})
        );
        assert_eq!(parse_form("bad=%FF+ok").unwrap(), json!({"bad": "%FF ok"}));
        let too_many = (0..1001).map(|i| format!("k{i}=v")).collect::<Vec<_>>().join("&");
        assert_eq!(parse_form(&too_many), Err(BodyError::TooManyParameters));
    }
}
