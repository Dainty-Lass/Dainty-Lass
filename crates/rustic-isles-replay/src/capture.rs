// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The gateway's capture lines (spec §3): what is read, what is skipped, and the corpus order.

use std::path::Path;

use serde_json::{Map, Value, json};

use crate::ReplayError;

/// A recorded body (§3.3). `Binary` and `Omitted` carry only the size: the bytes are not in the capture.
#[derive(Debug, Clone, PartialEq)]
pub enum CaptureBody {
    /// No body, or a value matching none of the forms below.
    Null,
    /// `{"json": ...}`: a JSON body.
    Json(Value),
    /// `{"form": [[k, v], ...]}`: form pairs in order.
    Form(Vec<(String, String)>),
    /// `{"text": ...}`: a non-JSON text body.
    Text(String),
    /// `{"binary": n}`: n bytes the capture left out.
    Binary(u64),
    /// `{"omitted": n}`: a body of n bytes past the gateway's cap.
    Omitted(u64),
}

impl CaptureBody {
    /// Reads a capture body value (§3.3); anything unrecognised is [`CaptureBody::Null`].
    pub fn from_wire(value: Option<&Value>) -> Self {
        let Some(Value::Object(map)) = value else {
            return Self::Null;
        };
        if let Some(json) = map.get("json") {
            return Self::Json(json.clone());
        }
        if let Some(Value::Array(items)) = map.get("form") {
            let text = |v: &Value| v.as_str().unwrap_or_default().to_owned();
            let pairs = items
                .iter()
                .filter_map(|item| match item {
                    Value::Array(pair) if pair.len() == 2 => Some((text(&pair[0]), text(&pair[1]))),
                    _ => None,
                })
                .collect();
            return Self::Form(pairs);
        }
        if let Some(Value::String(text)) = map.get("text") {
            return Self::Text(text.clone());
        }
        if let Some(size) = map.get("binary").and_then(Value::as_u64) {
            return Self::Binary(size);
        }
        if let Some(size) = map.get("omitted").and_then(Value::as_u64) {
            return Self::Omitted(size);
        }
        Self::Null
    }

    /// The capture's spelling of the body, the inverse of [`CaptureBody::from_wire`].
    pub fn to_wire(&self) -> Value {
        match self {
            Self::Null => Value::Null,
            Self::Json(v) => json!({ "json": v }),
            Self::Form(pairs) => {
                let items: Vec<Value> = pairs.iter().map(|(k, v)| json!([k, v])).collect();
                json!({ "form": items })
            }
            Self::Text(t) => json!({ "text": t }),
            Self::Binary(n) => json!({ "binary": n }),
            Self::Omitted(n) => json!({ "omitted": n }),
        }
    }

    /// Whether the bytes are missing from the capture, which makes the record inexact.
    pub fn is_lossy(&self) -> bool {
        matches!(self, Self::Binary(_) | Self::Omitted(_))
    }
}

/// One `kind: "http"` capture line (§3.2).
#[derive(Debug, Clone, PartialEq)]
pub struct HttpRecord {
    /// ISO time the gateway wrote the line.
    pub t: String,
    /// The writer's source name, `gateway` by default.
    pub src: String,
    /// Connection number.
    pub conn: u64,
    /// Request sequence number (the gateway's is global, from 1).
    pub seq: u64,
    /// Client address as the gateway saw it.
    pub ip: String,
    /// HTTP method.
    pub method: String,
    /// Path and query, as redacted by the gateway.
    pub target: String,
    /// The gateway's upstream label (`backend`, `content`, `ws`...).
    pub route: String,
    /// Request headers: name to a string or an array of strings.
    pub req_headers: Map<String, Value>,
    /// Request body.
    pub req_body: CaptureBody,
    /// Answer status, 0 when missing or out of range.
    pub status: u16,
    /// Answer headers.
    pub res_headers: Map<String, Value>,
    /// Answer body.
    pub res_body: CaptureBody,
    /// Time taken, ms.
    pub ms: u64,
    /// Whether the answer did not complete.
    pub aborted: bool,
}

/// One `kind: "chat"` capture line (§3.2); a field of the wrong type reads as absent.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChatRecord {
    /// ISO time the gateway wrote the line.
    pub t: String,
    /// The writer's source name, `gateway` by default.
    pub src: String,
    /// Connection number.
    pub conn: u64,
    /// `open` or `close`.
    pub event: Option<String>,
    /// `in` from the game, `out` from the server.
    pub dir: Option<String>,
    /// The frame, redacted.
    pub text: Option<String>,
    /// Size of a frame past the cap.
    pub omitted: Option<u64>,
    /// Size of a compressed frame.
    pub compressed: Option<u64>,
    /// Client address.
    pub ip: Option<String>,
    /// The chat upstream, as the gateway logged it.
    pub target: Option<String>,
    /// The chat route, as the gateway logged it.
    pub via: Option<String>,
    /// Why the connection closed (on `close`).
    pub reason: Option<String>,
}

/// One parsed capture line.
#[allow(
    clippy::large_enum_variant,
    reason = "the spec's shape; a corpus is loaded whole once, so the size is no concern"
)]
#[derive(Debug, Clone, PartialEq)]
pub enum CaptureRecord {
    /// An HTTP request and its answer.
    Http(HttpRecord),
    /// A chat frame or connection event.
    Chat(ChatRecord),
}

fn time_ms(t: &str) -> Option<i64> {
    service_kit::js::date_parse(t)
}

impl HttpRecord {
    /// The ISO time the gateway wrote the line.
    pub fn timestamp(&self) -> &str {
        &self.t
    }

    /// `t` as `Date.parse` reads it.
    pub fn timestamp_ms(&self) -> Option<i64> {
        time_ms(&self.t)
    }
}

impl ChatRecord {
    /// The ISO time the gateway wrote the line.
    pub fn timestamp(&self) -> &str {
        &self.t
    }

    /// `t` as `Date.parse` reads it.
    pub fn timestamp_ms(&self) -> Option<i64> {
        time_ms(&self.t)
    }
}

impl CaptureRecord {
    /// The ISO time the gateway wrote the line.
    pub fn timestamp(&self) -> &str {
        match self {
            Self::Http(r) => r.timestamp(),
            Self::Chat(r) => r.timestamp(),
        }
    }

    /// `t` as `Date.parse` reads it.
    pub fn timestamp_ms(&self) -> Option<i64> {
        match self {
            Self::Http(r) => r.timestamp_ms(),
            Self::Chat(r) => r.timestamp_ms(),
        }
    }
}

fn string_or(map: &Map<String, Value>, key: &str, default: &str) -> String {
    map.get(key).and_then(Value::as_str).unwrap_or(default).to_owned()
}

fn opt_string(map: &Map<String, Value>, key: &str) -> Option<String> {
    map.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn object(map: &Map<String, Value>, snake: &str, camel: &str) -> Map<String, Value> {
    // The snake_case spelling (the harness's own) wins over the gateway's camelCase
    match map.get(snake).or_else(|| map.get(camel)) {
        Some(Value::Object(o)) => o.clone(),
        _ => Map::new(),
    }
}

fn body(map: &Map<String, Value>, snake: &str, camel: &str) -> CaptureBody {
    CaptureBody::from_wire(map.get(snake).or_else(|| map.get(camel)))
}

/// One line (§3.2), or `None` for anything skipped.
pub fn parse_capture_line(line: &str) -> Option<CaptureRecord> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let Ok(Value::Object(map)) = serde_json::from_str::<Value>(line) else {
        return None;
    };
    let kind = map.get("kind")?.as_str()?;
    let conn = map.get("conn")?.as_u64()?;
    let t = string_or(&map, "t", "");
    let src = string_or(&map, "src", "gateway");
    match kind {
        "http" => {
            let method = map.get("method")?.as_str()?.to_owned();
            let target = map.get("target")?.as_str()?.to_owned();
            let status = map
                .get("status")
                .and_then(Value::as_u64)
                .and_then(|s| u16::try_from(s).ok())
                .unwrap_or(0);
            Some(CaptureRecord::Http(HttpRecord {
                t,
                src,
                conn,
                seq: map.get("seq").and_then(Value::as_u64).unwrap_or(0),
                ip: string_or(&map, "ip", "127.0.0.1"),
                method,
                target,
                route: string_or(&map, "route", "backend"),
                req_headers: object(&map, "req_headers", "reqHeaders"),
                req_body: body(&map, "req_body", "reqBody"),
                status,
                res_headers: object(&map, "res_headers", "resHeaders"),
                res_body: body(&map, "res_body", "resBody"),
                ms: map.get("ms").and_then(Value::as_u64).unwrap_or(0),
                aborted: map.get("aborted").and_then(Value::as_bool).unwrap_or(false),
            }))
        }
        "chat" => Some(CaptureRecord::Chat(ChatRecord {
            t,
            src,
            conn,
            event: opt_string(&map, "event"),
            dir: opt_string(&map, "dir"),
            text: opt_string(&map, "text"),
            omitted: map.get("omitted").and_then(Value::as_u64),
            compressed: map.get("compressed").and_then(Value::as_u64),
            ip: opt_string(&map, "ip"),
            target: opt_string(&map, "target"),
            via: opt_string(&map, "via"),
            reason: opt_string(&map, "reason"),
        })),
        _ => None,
    }
}

/// Every record of a text, in line order.
pub fn parse_capture_str(text: &str) -> Vec<CaptureRecord> {
    text.lines().filter_map(parse_capture_line).collect()
}

fn io_error(path: &Path, error: &std::io::Error) -> ReplayError {
    ReplayError::Io {
        path: path.display().to_string(),
        message: error.to_string(),
    }
}

/// A file, or a directory's direct `*.jsonl` entries in path order; sorted by time, then file, then line (§3.4).
pub fn load_corpus(path: &Path) -> Result<Vec<CaptureRecord>, ReplayError> {
    let meta = std::fs::metadata(path).map_err(|e| io_error(path, &e))?;
    let files = if meta.is_dir() {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(path).map_err(|e| io_error(path, &e))? {
            let entry = entry.map_err(|e| io_error(path, &e))?;
            let file = entry.path();
            if file.extension().is_some_and(|ext| ext == "jsonl") && file.is_file() {
                files.push(file);
            }
        }
        files.sort();
        files
    } else {
        vec![path.to_path_buf()]
    };
    let mut records = Vec::new();
    for file in &files {
        let bytes = std::fs::read(file).map_err(|e| io_error(file, &e))?;
        records.extend(parse_capture_str(&String::from_utf8_lossy(&bytes)));
    }
    // The position breaks ties, so file order and then line order hold within one time
    let mut keyed: Vec<(i64, usize, CaptureRecord)> = records
        .into_iter()
        .enumerate()
        .map(|(i, r)| (r.timestamp_ms().unwrap_or(0), i, r))
        .collect();
    keyed.sort_by_key(|(ms, i, _)| (*ms, *i));
    Ok(keyed.into_iter().map(|(_, _, r)| r).collect())
}
