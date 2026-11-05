// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The request a target receives (spec §6.1) and the recorded answer (§6.5).

use std::collections::BTreeMap;

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::{Map, Value};

use crate::capture::{CaptureBody, HttpRecord};
use crate::credentials::{ReplayTargetAuth, substitute_str, substitute_value};
use crate::ids::{IdMap, apply_ids_str, apply_ids_value};
use rustic_isles::replay::{REPLAY_NOW_HEADER, REPLAY_SEED_HEADER};

/// 2026-10-26T12:00:00Z: the pinned clock of a record whose `t` does not parse (§15 item 4).
pub const FALLBACK_NOW_MS: i64 = 1_793_016_000_000;

/// Dropped from recorded headers; the transport writes its own.
pub(crate) const HOP_HEADERS: [&str; 5] = [
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
    "keep-alive",
];

// Everything but ASCII letters, digits and `-` `.` `_` `*`, so a space is `%20`
const FORM: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'.').remove(b'_').remove(b'*');

/// What one side sends for one record. `Debug` hides the target, header values and body: they carry minted tokens
/// and keys.
#[derive(Clone, PartialEq)]
pub struct PreparedHttpRequest {
    pub method: String,
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub inexact: bool,
}

impl std::fmt::Debug for PreparedHttpRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.headers.iter().map(|(n, _)| n.as_str()).collect();
        f.debug_struct("PreparedHttpRequest")
            .field("method", &self.method)
            .field("target_len", &self.target.len())
            .field("header_names", &names)
            .field("body_len", &self.body.len())
            .field("inexact", &self.inexact)
            .finish()
    }
}

impl PreparedHttpRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }
}

/// A header value as text: an array's strings joined with `, `, any other non-string its JSON text.
pub(crate) fn header_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Array(items) => items.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "),
        other => service_kit::jsjson::stringify(other),
    }
}

fn has_header(headers: &Map<String, Value>, name: &str) -> bool {
    headers.keys().any(|k| k.eq_ignore_ascii_case(name))
}

fn form_encode(text: &str) -> String {
    utf8_percent_encode(text, FORM).to_string()
}

/// §6.1 for one side: ids applied, placeholders substituted, the replay headers appended when `pin_clock`.
pub fn prepare_http_request(
    record: &HttpRecord,
    auth: &ReplayTargetAuth,
    id_map: &IdMap,
    pin_clock: bool,
) -> PreparedHttpRequest {
    let now = record.timestamp_ms().unwrap_or(FALLBACK_NOW_MS);
    let mut inexact = record.aborted || record.req_body.is_lossy() || record.res_body.is_lossy();
    let text = |raw: &str, inexact: &mut bool| {
        let sub = substitute_str(&apply_ids_str(raw, id_map), auth, now);
        *inexact |= sub.inexact;
        sub.value
    };

    let mut target = text(&record.target, &mut inexact);
    if record.route == "backend" {
        // The gateway's `/game/` mount, undone: the harness talks to the backend directly
        if target == "/game/login" {
            target = "/login".into();
        } else if let Some(query) = target.strip_prefix("/game/login?") {
            target = format!("/login?{query}");
        }
    }

    let oauth = record.target.starts_with("/account/api/oauth/token");
    let mut headers = Vec::new();
    for (name, value) in &record.req_headers {
        let name = name.to_ascii_lowercase();
        if HOP_HEADERS.contains(&name.as_str()) {
            continue;
        }
        let raw = header_text(value);
        let mut header_inexact = false;
        let value = text(&raw, &mut header_inexact);
        // The client's Basic credentials there are not secrets the answer depends on
        let basic_exempt = name == "authorization" && oauth && raw.to_ascii_lowercase().starts_with("basic <redacted>");
        inexact |= header_inexact && !basic_exempt;
        headers.push((name, value));
    }

    let body = match &record.req_body {
        CaptureBody::Null | CaptureBody::Binary(_) | CaptureBody::Omitted(_) => Vec::new(),
        CaptureBody::Json(value) => {
            let sub = substitute_value(&apply_ids_value(value, id_map), auth, now);
            inexact |= sub.inexact;
            if !has_header(&record.req_headers, "content-type") {
                headers.push(("content-type".into(), "application/json".into()));
            }
            service_kit::jsjson::stringify(&sub.value).into_bytes()
        }
        CaptureBody::Form(pairs) => {
            let encoded: Vec<String> = pairs
                .iter()
                .map(|(k, v)| {
                    let k = text(k, &mut inexact);
                    let v = text(v, &mut inexact);
                    format!("{}={}", form_encode(&k), form_encode(&v))
                })
                .collect();
            if !has_header(&record.req_headers, "content-type") {
                headers.push(("content-type".into(), "application/x-www-form-urlencoded".into()));
            }
            encoded.join("&").into_bytes()
        }
        CaptureBody::Text(t) => text(t, &mut inexact).into_bytes(),
    };

    if !has_header(&record.req_headers, "x-forwarded-for") && !record.ip.is_empty() {
        headers.push(("x-forwarded-for".into(), record.ip.clone()));
    }
    if let Some(secret) = &auth.gateway_secret {
        headers.retain(|(n, _)| n != "x-gateway-secret");
        headers.push(("x-gateway-secret".into(), secret.clone()));
    }
    if pin_clock {
        headers.push((REPLAY_NOW_HEADER.into(), now.to_string()));
        // The raw target, before ids and the `/game/login` rewrite, so both sides send the same seed
        headers.push((
            REPLAY_SEED_HEADER.into(),
            format!("{}:{}:{}:{}", record.conn, record.seq, record.method, record.target),
        ));
    }

    PreparedHttpRequest {
        method: record.method.clone(),
        target,
        headers,
        body,
        inexact,
    }
}

/// One side's answer: lowercased header names, the body as text.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReplayHttpResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body_text: String,
}

impl ReplayHttpResponse {
    /// The capture's own answer (§6.5). A form is rebuilt unencoded (§15 item 22).
    pub fn from_recorded(record: &HttpRecord) -> Self {
        let mut headers: BTreeMap<String, String> = BTreeMap::new();
        for (name, value) in &record.res_headers {
            let value = header_text(value);
            headers
                .entry(name.to_ascii_lowercase())
                .and_modify(|v| {
                    v.push_str(", ");
                    v.push_str(&value);
                })
                .or_insert(value);
        }
        let body_text = match &record.res_body {
            CaptureBody::Json(v) => service_kit::jsjson::stringify(v),
            CaptureBody::Form(pairs) => pairs
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("&"),
            CaptureBody::Text(t) => t.clone(),
            CaptureBody::Null | CaptureBody::Binary(_) | CaptureBody::Omitted(_) => String::new(),
        };
        Self {
            status: record.status,
            headers,
            body_text,
        }
    }
}
