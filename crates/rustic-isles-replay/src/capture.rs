// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The gateway's capture lines (spec §3): what is read, what is skipped, and the corpus order.

use std::path::Path;

use serde_json::{Map, Value, json};

use crate::ReplayError;

/// A recorded body (§3.3). `Binary` and `Omitted` carry only the size: the bytes are not in the capture.
#[derive(Debug, Clone, PartialEq)]
pub enum CaptureBody {
    Null,
    Json(Value),
    Form(Vec<(String, String)>),
    Text(String),
    Binary(u64),
    Omitted(u64),
}

impl CaptureBody {
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

    pub fn is_lossy(&self) -> bool {
        matches!(self, Self::Binary(_) | Self::Omitted(_))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HttpRecord {
    pub t: String,
    pub src: String,
    pub conn: u64,
    pub seq: u64,
    pub ip: String,
    pub method: String,
    pub target: String,
    pub route: String,
    pub req_headers: Map<String, Value>,
    pub req_body: CaptureBody,
    pub status: u16,
    pub res_headers: Map<String, Value>,
    pub res_body: CaptureBody,
    pub ms: u64,
    pub aborted: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChatRecord {
    pub t: String,
    pub src: String,
    pub conn: u64,
    pub event: Option<String>,
    pub dir: Option<String>,
    pub text: Option<String>,
    pub omitted: Option<u64>,
    pub compressed: Option<u64>,
    pub ip: Option<String>,
    pub target: Option<String>,
    pub via: Option<String>,
    pub reason: Option<String>,
}

// The spec's shape; a corpus is loaded whole once, so the size is no concern
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum CaptureRecord {
    Http(HttpRecord),
    Chat(ChatRecord),
}

fn time_ms(t: &str) -> Option<i64> {
    service_kit::js::date_parse(t)
}

impl HttpRecord {
    pub fn timestamp(&self) -> &str {
        &self.t
    }

    /// `t` as `Date.parse` reads it.
    pub fn timestamp_ms(&self) -> Option<i64> {
        time_ms(&self.t)
    }
}

impl ChatRecord {
    pub fn timestamp(&self) -> &str {
        &self.t
    }

    pub fn timestamp_ms(&self) -> Option<i64> {
        time_ms(&self.t)
    }
}

impl CaptureRecord {
    pub fn timestamp(&self) -> &str {
        match self {
            Self::Http(r) => r.timestamp(),
            Self::Chat(r) => r.timestamp(),
        }
    }

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
