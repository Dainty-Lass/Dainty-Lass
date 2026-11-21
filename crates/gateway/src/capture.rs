// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The traffic capture for the shadow replay (RD19): every request, answer and chat message, redacted, as JSON
//! lines by UTC day. Personal data: off unless `CAPTURE_DIR` is set, kept `CAPTURE_RETENTION_DAYS`, and the
//! replay's placeholders (`<token:...>`, `<key:...>`, `<plain:...>`) never carry a secret.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock, PoisonError};
use std::task::{Context, Poll};

use http::HeaderMap;
use regex::Regex;
use serde_json::{Map, Value, json};
use service_kit::env::Env;
use service_kit::{js, linelog};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use crate::policy::GAMESERVER_KEY_HEADERS;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureOptions {
    pub dir: PathBuf,
    pub retention_days: u32,
    pub max_body_bytes: usize,
    pub source: String,
}

fn capture_number(env: &Env, name: &str, default: u64, min: u64, max: u64) -> Result<u64, String> {
    match crate::config::optional(env, name) {
        None => Ok(default),
        Some(text) => crate::whole_number(text, min as f64, max as f64)
            .map(|v| v as u64)
            .ok_or_else(|| {
                format!(
                    "{name} must be a whole number from {min} to {max} (got {})",
                    crate::json_string(text)
                )
            }),
    }
}

/// The capture's settings; `None` when `CAPTURE_DIR` is unset.
pub fn load_capture_options(env: &Env) -> Result<Option<CaptureOptions>, String> {
    let Some(dir) = crate::config::optional(env, "CAPTURE_DIR") else {
        return Ok(None);
    };
    let retention_days = capture_number(env, "CAPTURE_RETENTION_DAYS", 14, 1, 90)? as u32;
    let max_body_bytes = capture_number(env, "CAPTURE_MAX_BODY_BYTES", 262_144, 0, 16_777_216)? as usize;
    Ok(Some(CaptureOptions {
        dir: crate::config::absolute(dir),
        retention_days,
        max_body_bytes,
        source: "gateway".to_owned(),
    }))
}

// ---- redaction ----

fn pattern(text: &str) -> Regex {
    Regex::new(text).unwrap_or_else(|_| unreachable!("a fixed pattern"))
}

static JWT: LazyLock<Regex> = LazyLock::new(|| pattern(r"eyJ[A-Za-z0-9_-]+\.([A-Za-z0-9_-]+)\.[A-Za-z0-9_-]+"));
static ACCOUNT_KEY: LazyLock<Regex> = LazyLock::new(|| pattern(r"(?:RRK_|UUK_)[A-Za-z0-9]+"));
static HEX_RUN: LazyLock<Regex> = LazyLock::new(|| pattern(r"[0-9A-Fa-f]{40,}"));
static LONG_RUN: LazyLock<Regex> = LazyLock::new(|| pattern(r"[A-Za-z0-9_+/=-]{100,}"));
static MARK: LazyLock<Regex> = LazyLock::new(|| pattern("\u{1}([0-9]+)\u{1}"));
static PLACEHOLDER: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"^(?:<token(?::[A-Za-z0-9_-]{1,64})?>|<key:[0-9a-f]{64}>)$"));
static AUTH: LazyLock<Regex> = LazyLock::new(|| pattern(r"(<auth\b[^>]*>)([^<]*)(</auth>)"));
static PASSWORD: LazyLock<Regex> = LazyLock::new(|| pattern(r"(<password\b[^>]*>)[^<]*(</password>)"));

const SECRET_NAMES: [&str; 11] = [
    "key", "token", "secret", "pass", "auth", "jwt", "sig", "cred", "recovery", "invite", "cookie",
];

fn is_subject(text: &str) -> bool {
    (1..=64).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Both base64 alphabets, other characters skipped, stopping at the first `=`; leftover bits dropped.
pub fn decode_base64_lenient(text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for b in text.bytes() {
        let value = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' => break,
            _ => continue,
        };
        acc = (acc << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    out
}

fn token_placeholder(middle: &str) -> String {
    let claims: Option<Value> = serde_json::from_slice(&decode_base64_lenient(middle)).ok();
    let subject = claims.as_ref().and_then(|c| match (c.get("userId"), c.get("sub")) {
        (Some(Value::String(id)), _) => Some(id.clone()),
        (_, Some(Value::Null) | None) => None,
        (_, Some(Value::String(sub))) => Some(sub.clone()),
        (_, Some(other)) => Some(js::string(Some(other))),
    });
    match subject.filter(|s| is_subject(s)) {
        Some(s) => format!("<token:{s}>"),
        None => "<token>".to_owned(),
    }
}

/// Tokens become `<token:<subject>>`, account keys `<key:<sha256>>`, long hex and long base64-ish runs
/// `<redacted>`. The placeholders go in last, so the run rules never cut one.
pub fn redact_capture_text(text: &str) -> String {
    let mut held: Vec<String> = Vec::new();
    let text = text.replace('\u{1}', "?");
    let text = JWT.replace_all(&text, |c: &regex::Captures<'_>| {
        held.push(token_placeholder(&c[1]));
        format!("\u{1}{}\u{1}", held.len() - 1)
    });
    let text = ACCOUNT_KEY.replace_all(&text, |c: &regex::Captures<'_>| {
        held.push(format!("<key:{}>", crate::hex(&Sha256::digest(c[0].as_bytes()))));
        format!("\u{1}{}\u{1}", held.len() - 1)
    });
    let text = HEX_RUN.replace_all(&text, "<redacted>");
    let text = LONG_RUN.replace_all(&text, "<redacted>");
    MARK.replace_all(&text, |c: &regex::Captures<'_>| {
        c[1].parse::<usize>()
            .ok()
            .and_then(|i| held.get(i).cloned())
            .unwrap_or_default()
    })
    .into_owned()
}

fn secret_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    SECRET_NAMES.iter().any(|s| lower.contains(s))
}

/// A value under `name`: secret-sounding names keep only a lone token or key placeholder.
fn redact_named(name: &str, value: &str) -> String {
    let redacted = redact_capture_text(value);
    if secret_name(name) && !PLACEHOLDER.is_match(&redacted) {
        return "<redacted>".to_owned();
    }
    redacted
}

fn redact_json_named(value: &Value, name: Option<&str>) -> Value {
    match value {
        Value::String(s) => Value::String(match name {
            Some(n) => redact_named(n, s),
            None => redact_capture_text(s),
        }),
        Value::Array(items) => Value::Array(items.iter().map(|v| redact_json_named(v, name)).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                // A name is redacted as text too, so a key used as a name never survives
                .map(|(k, v)| (redact_capture_text(k), redact_json_named(v, Some(k))))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Strings by the name of the field that holds them; array items by the array's name.
pub fn redact_capture_json(value: &Value) -> Value {
    redact_json_named(value, None)
}

fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}

/// Every header but cookies and the gateway secret, its values joined with `, `, redacted.
pub fn redact_capture_headers(headers: &HeaderMap) -> Value {
    let mut out = Map::new();
    for name in headers.keys() {
        let name = name.as_str();
        if matches!(name, "cookie" | "set-cookie" | "x-reforged-gateway") {
            continue;
        }
        let joined = headers
            .get_all(name)
            .iter()
            .map(|v| latin1(v.as_bytes()))
            .collect::<Vec<_>>()
            .join(", ");
        let value = if GAMESERVER_KEY_HEADERS.contains(&name) {
            "<gskey>".to_owned()
        } else if name == "authorization" {
            redact_capture_text(&joined)
        } else {
            let short = ["x-reforged-", "x-rustic-isles-", "x-mysticparadox-"]
                .iter()
                .find_map(|p| name.strip_prefix(p))
                .unwrap_or(name);
            redact_named(short, &joined)
        };
        out.insert(name.to_owned(), Value::String(value));
    }
    Value::Object(out)
}

/// A body: `null`, `{"omitted"}`, `{"json"}`, `{"form"}`, `{"text"}` or `{"binary"}`.
pub fn redact_capture_body(kept: &[u8], content_type: Option<&str>, total: u64, max: usize) -> Value {
    if total == 0 {
        return Value::Null;
    }
    if total > max as u64 || (kept.len() as u64) < total {
        return json!({ "omitted": total });
    }
    let text = String::from_utf8_lossy(kept);
    let kind = content_type.unwrap_or("").to_lowercase();
    let start = text.trim_start_matches(js::is_space);
    if (kind.contains("json") || start.starts_with('[') || start.starts_with('{'))
        && let Ok(value) = serde_json::from_str::<Value>(&text)
    {
        return json!({ "json": redact_capture_json(&value) });
    }
    if kind.contains("x-www-form-urlencoded") {
        let pairs: Vec<Value> = url::form_urlencoded::parse(kept)
            .map(|(k, v)| json!([redact_capture_text(&k), redact_named(&k, &v)]))
            .collect();
        return json!({ "form": pairs });
    }
    let control = text
        .chars()
        .any(|c| ('\u{0}'..='\u{8}').contains(&c) || ('\u{e}'..='\u{1f}').contains(&c));
    if !control && std::str::from_utf8(kept).is_ok() {
        return json!({ "text": redact_capture_text(&text) });
    }
    json!({ "binary": total })
}

/// A chat frame: an `<auth>` element becomes `<plain:<account>>` or `<redacted>`, a password `<redacted>`, then
/// the whole frame is redacted as text.
pub fn redact_chat_frame(text: &str) -> String {
    let text = AUTH.replace_all(text, |c: &regex::Captures<'_>| {
        let decoded = decode_base64_lenient(c[2].trim());
        let account = decoded
            .split(|&b| b == 0)
            .nth(1)
            .and_then(|part| std::str::from_utf8(part).ok())
            .filter(|a| is_subject(a))
            .map_or_else(|| "<redacted>".to_owned(), |a| format!("<plain:{a}>"));
        format!("{}{account}{}", &c[1], &c[3])
    });
    let text = PASSWORD.replace_all(&text, "${1}<redacted>${2}");
    redact_capture_text(&text)
}

pub fn redact_capture_target(raw: &str) -> String {
    redact_capture_text(raw)
}

// ---- WebSocket messages ----

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatMessage {
    Text(String),
    Omitted(u64),
    Compressed(u64),
}

#[derive(Debug, Default)]
struct Open {
    compressed: bool,
    total: u64,
    kept: Vec<u8>,
}

#[derive(Debug)]
enum Phase {
    Head(Vec<u8>),
    Payload {
        remaining: u64,
        mask: Option<[u8; 4]>,
        offset: usize,
        data: bool,
        fin: bool,
    },
}

/// Whole data messages from one direction of a WebSocket, without changing a byte; the same messages however the
/// bytes are split.
#[derive(Debug)]
pub struct FrameReader {
    cap: usize,
    phase: Phase,
    open: Option<Open>,
}

fn head_len(head: &[u8]) -> Option<usize> {
    let second = *head.get(1)?;
    let extra = match second & 0x7f {
        126 => 2,
        127 => 8,
        _ => 0,
    };
    Some(2 + extra + if second & 0x80 != 0 { 4 } else { 0 })
}

impl FrameReader {
    pub fn new(cap: usize) -> Self {
        Self {
            cap,
            phase: Phase::Head(Vec::new()),
            open: None,
        }
    }

    fn finish(&mut self, out: &mut Vec<ChatMessage>) {
        if let Some(open) = self.open.take() {
            out.push(if open.compressed {
                ChatMessage::Compressed(open.total)
            } else if open.total > self.cap as u64 {
                ChatMessage::Omitted(open.total)
            } else {
                ChatMessage::Text(String::from_utf8_lossy(&open.kept).into_owned())
            });
        }
    }

    /// A complete frame head: what it starts, and the payload phase.
    fn start_frame(&mut self, head: &[u8], out: &mut Vec<ChatMessage>) {
        let (fin, rsv1, opcode) = (head[0] & 0x80 != 0, head[0] & 0x40 != 0, head[0] & 0x0f);
        let masked = head[1] & 0x80 != 0;
        let (len, at) = match head[1] & 0x7f {
            126 => (u64::from(u16::from_be_bytes([head[2], head[3]])), 4),
            127 => {
                let mut b = [0u8; 8];
                b.copy_from_slice(&head[2..10]);
                (u64::from_be_bytes(b), 10)
            }
            n => (u64::from(n), 2),
        };
        let mask = masked.then(|| [head[at], head[at + 1], head[at + 2], head[at + 3]]);
        let data = if opcode >= 8 {
            false
        } else if opcode != 0 {
            self.open = Some(Open {
                compressed: rsv1,
                ..Open::default()
            });
            true
        } else {
            self.open.is_some()
        };
        if len == 0 {
            if data && fin {
                self.finish(out);
            }
            self.phase = Phase::Head(Vec::new());
        } else {
            self.phase = Phase::Payload {
                remaining: len,
                mask,
                offset: 0,
                data,
                fin,
            };
        }
    }

    /// The messages these bytes complete.
    pub fn push(&mut self, mut bytes: &[u8]) -> Vec<ChatMessage> {
        let mut out = Vec::new();
        while !bytes.is_empty() {
            match &mut self.phase {
                Phase::Head(head) => {
                    head.push(bytes[0]);
                    bytes = &bytes[1..];
                    if head_len(head).is_some_and(|n| head.len() == n) {
                        let head = std::mem::take(head);
                        self.start_frame(&head, &mut out);
                    }
                }
                Phase::Payload {
                    remaining,
                    mask,
                    offset,
                    data,
                    fin,
                } => {
                    let n = (*remaining).min(bytes.len() as u64) as usize;
                    if *data && let Some(open) = self.open.as_mut() {
                        open.total += n as u64;
                        let room = (self.cap + 1).saturating_sub(open.kept.len()).min(n);
                        for (i, &b) in bytes[..room].iter().enumerate() {
                            let m = mask.map_or(0, |m| m[(*offset + i) % 4]);
                            open.kept.push(b ^ m);
                        }
                    }
                    *offset += n;
                    *remaining -= n as u64;
                    bytes = &bytes[n..];
                    if *remaining == 0 {
                        let end = *data && *fin;
                        self.phase = Phase::Head(Vec::new());
                        if end {
                            self.finish(&mut out);
                        }
                    }
                }
            }
        }
        out
    }
}

// ---- the files ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// From the game
    In,
    /// From the backend
    Out,
}

struct Day {
    day: String,
    file: std::fs::File,
}

pub struct CaptureWriter {
    pub options: CaptureOptions,
    secrets: Mutex<Vec<String>>,
    day: Mutex<Option<Day>>,
    stopped: std::sync::atomic::AtomicBool,
}

fn file_day(name: &str) -> Option<&str> {
    let stem = name.strip_suffix(".jsonl")?;
    let at = stem.len().checked_sub(10).filter(|&at| stem.is_char_boundary(at))?;
    let (prefix, day) = stem.split_at(at);
    let prefix = prefix.strip_suffix('-')?;
    let well_formed = !prefix.is_empty()
        && prefix.bytes().all(|b| b.is_ascii_lowercase())
        && day.bytes().all(|b| b.is_ascii_digit() || b == b'-')
        && chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").is_ok();
    well_formed.then_some(day)
}

impl CaptureWriter {
    /// Makes the folder (0700 where there are modes) and prunes it.
    pub fn new(options: CaptureOptions) -> Result<Self, String> {
        let made = std::fs::create_dir_all(&options.dir);
        #[cfg(unix)]
        let made = made.and_then(|()| {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&options.dir, std::fs::Permissions::from_mode(0o700))
        });
        made.map_err(|e| format!("Traffic capture: {}: {e}", options.dir.display()))?;
        let writer = Self {
            options,
            secrets: Mutex::default(),
            day: Mutex::default(),
            stopped: false.into(),
        };
        writer.prune(crate::now_ms());
        Ok(writer)
    }

    /// Replaced with `<redacted>` wherever it appears in a line (at least 8 characters).
    pub fn add_secret(&self, secret: &str) {
        if secret.len() >= 8 {
            let escaped = Value::String(secret.to_owned()).to_string();
            let inner = escaped[1..escaped.len() - 1].to_owned();
            self.secrets.lock().unwrap_or_else(PoisonError::into_inner).push(inner);
        }
    }

    /// Deletes the day files older than the retention; others are left alone.
    pub fn prune(&self, now_ms: i64) {
        let cutoff = crate::iso_ms(now_ms - i64::from(self.options.retention_days) * 86_400_000);
        let cutoff = &cutoff[..10];
        let Ok(entries) = std::fs::read_dir(&self.options.dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if file_day(&name).is_some_and(|day| day < cutoff) {
                _ = std::fs::remove_file(entry.path());
            }
        }
    }

    fn open(path: &Path) -> std::io::Result<std::fs::File> {
        let mut options = std::fs::OpenOptions::new();
        options.append(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        options.open(path)
    }

    /// One record at `now_ms`: `t` and `src` first, then its fields.
    pub fn write_at(&self, record: &Value, now_ms: i64) {
        if self.stopped.load(Ordering::Relaxed) {
            return;
        }
        let mut line = Map::new();
        line.insert("t".into(), Value::String(crate::iso_ms(now_ms)));
        line.insert("src".into(), Value::String(self.options.source.clone()));
        if let Value::Object(fields) = record {
            for (k, v) in fields {
                line.insert(k.clone(), v.clone());
            }
        }
        let mut text = Value::Object(line).to_string();
        for secret in self.secrets.lock().unwrap_or_else(PoisonError::into_inner).iter() {
            text = text.replace(secret.as_str(), "<redacted>");
        }
        text.push('\n');
        let today = crate::iso_ms(now_ms)[..10].to_owned();
        let mut day = self.day.lock().unwrap_or_else(PoisonError::into_inner);
        let result = (|| {
            if day.as_ref().is_none_or(|d| d.day != today) {
                let path = self.options.dir.join(format!("{}-{today}.jsonl", self.options.source));
                *day = Some(Day {
                    day: today.clone(),
                    file: Self::open(&path)?,
                });
                self.prune(now_ms);
            }
            match day.as_mut() {
                Some(d) => d.file.write_all(text.as_bytes()),
                None => Ok(()),
            }
        })();
        if let Err(error) = result {
            self.stopped.store(true, Ordering::Relaxed);
            *day = None;
            linelog::warn(&format!("traffic capture stopped: {error}"), json!({}));
        }
    }

    pub fn write(&self, record: &Value) {
        self.write_at(record, crate::now_ms());
    }
}

/// Bytes of a body up to a cap, and the count of all of them.
#[derive(Debug, Default, Clone)]
pub struct BodyTap {
    pub kept: Vec<u8>,
    pub total: u64,
    cap: usize,
}

impl BodyTap {
    pub fn new(cap: usize) -> Self {
        Self {
            kept: Vec::new(),
            total: 0,
            cap,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.total += bytes.len() as u64;
        let room = self.cap.saturating_sub(self.kept.len()).min(bytes.len());
        self.kept.extend_from_slice(&bytes[..room]);
    }

    pub fn redacted(&self, content_type: Option<&str>) -> Value {
        redact_capture_body(&self.kept, content_type, self.total, self.cap)
    }
}

/// The capture shared by every connection.
#[derive(Clone)]
pub struct Capture {
    writer: Arc<CaptureWriter>,
    conns: Arc<AtomicU64>,
    seq: Arc<AtomicU64>,
}

/// One connection's number, given the first time it has something recorded.
#[derive(Clone)]
pub struct CaptureConn {
    capture: Capture,
    number: Arc<OnceLock<u64>>,
}

impl Capture {
    pub fn new(writer: CaptureWriter) -> Self {
        Self {
            writer: Arc::new(writer),
            conns: Arc::default(),
            seq: Arc::default(),
        }
    }

    pub fn writer(&self) -> &CaptureWriter {
        &self.writer
    }

    pub fn max_body(&self) -> usize {
        self.writer.options.max_body_bytes
    }

    pub fn connection(&self) -> CaptureConn {
        CaptureConn {
            capture: self.clone(),
            number: Arc::default(),
        }
    }
}

/// One answered request, as the replay reads it.
pub struct HttpRecord<'a> {
    pub ip: &'a str,
    pub method: &'a str,
    pub target: &'a str,
    pub route: &'a str,
    pub req_headers: &'a HeaderMap,
    pub req_body: &'a BodyTap,
    pub status: u16,
    pub res_headers: &'a HeaderMap,
    pub res_body: &'a BodyTap,
    pub ms: u64,
    pub aborted: bool,
}

impl CaptureConn {
    fn number(&self) -> u64 {
        *self
            .number
            .get_or_init(|| self.capture.conns.fetch_add(1, Ordering::Relaxed) + 1)
    }

    pub fn http(&self, r: &HttpRecord<'_>) {
        let conn = self.number();
        let seq = self.capture.seq.fetch_add(1, Ordering::Relaxed) + 1;
        let content_type = |h: &HeaderMap| h.get(http::header::CONTENT_TYPE).map(|v| latin1(v.as_bytes()));
        let mut record = json!({
            "kind": "http",
            "conn": conn,
            "seq": seq,
            "ip": r.ip,
            "method": r.method,
            "target": redact_capture_target(r.target),
            "route": r.route,
            "reqHeaders": redact_capture_headers(r.req_headers),
            "reqBody": r.req_body.redacted(content_type(r.req_headers).as_deref()),
            "status": r.status,
            "resHeaders": redact_capture_headers(r.res_headers),
            "resBody": r.res_body.redacted(content_type(r.res_headers).as_deref()),
            "ms": r.ms,
        });
        if r.aborted
            && let Value::Object(map) = &mut record
        {
            map.insert("aborted".into(), Value::Bool(true));
        }
        self.capture.writer.write(&record);
    }

    pub fn chat_open(&self, ip: &str, target: &str) {
        let record = json!({"kind": "chat", "conn": self.number(), "event": "open", "ip": ip,
            "target": redact_capture_target(target)});
        self.capture.writer.write(&record);
    }

    pub fn chat_message(&self, direction: Direction, message: &ChatMessage) {
        let dir = match direction {
            Direction::In => "in",
            Direction::Out => "out",
        };
        let mut record = json!({"kind": "chat", "conn": self.number(), "dir": dir});
        if let Value::Object(map) = &mut record {
            let (key, value) = match message {
                ChatMessage::Text(text) => ("text", Value::String(redact_chat_frame(text))),
                ChatMessage::Omitted(n) => ("omitted", json!(n)),
                ChatMessage::Compressed(n) => ("compressed", json!(n)),
            };
            map.insert(key.into(), value);
        }
        self.capture.writer.write(&record);
    }

    pub fn chat_close(&self) {
        let record = json!({"kind": "chat", "conn": self.number(), "event": "close", "reason": "socket"});
        self.capture.writer.write(&record);
    }
}

/// The client side of a chat tunnel, read through a [`FrameReader`] each way without changing a byte.
pub struct ChatTap<S> {
    inner: S,
    conn: CaptureConn,
    from_game: FrameReader,
    to_game: FrameReader,
}

impl<S> ChatTap<S> {
    pub fn new(inner: S, conn: CaptureConn) -> Self {
        let cap = conn.capture.max_body();
        Self {
            inner,
            conn,
            from_game: FrameReader::new(cap),
            to_game: FrameReader::new(cap),
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for ChatTap<S> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let polled = Pin::new(&mut self.inner).poll_read(cx, buf);
        if let Poll::Ready(Ok(())) = polled {
            let fresh = buf.filled()[before..].to_vec();
            for message in self.from_game.push(&fresh) {
                self.conn.chat_message(Direction::In, &message);
            }
        }
        polled
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for ChatTap<S> {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        let polled = Pin::new(&mut self.inner).poll_write(cx, buf);
        if let Poll::Ready(Ok(n)) = polled {
            for message in self.to_game.push(&buf[..n]) {
                self.conn.chat_message(Direction::Out, &message);
            }
        }
        polled
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    fn jwt(claims: &str) -> String {
        format!("eyJhbGciOiJub25lIn0.{}.c2ln", URL_SAFE_NO_PAD.encode(claims))
    }

    #[test]
    fn text_placeholders() {
        assert_eq!(redact_capture_text(&jwt(r#"{"userId":"UID-1"}"#)), "<token:UID-1>");
        assert_eq!(redact_capture_text(&jwt(r#"{"sub":"abc"}"#)), "<token:abc>");
        assert_eq!(redact_capture_text(&jwt(r#"{"sub":null}"#)), "<token>");
        assert_eq!(redact_capture_text(&jwt(r#"{"userId":"a b"}"#)), "<token>");
        let key = "RRK_abc123";
        assert_eq!(
            redact_capture_text(key),
            format!("<key:{}>", crate::hex(&Sha256::digest(key.as_bytes())))
        );
        assert_eq!(redact_capture_text("a\u{1}0\u{1}b"), "a?0?b");
        assert_eq!(redact_capture_text(&"f".repeat(40)), "<redacted>");
        assert_eq!(redact_capture_text(&"Z".repeat(100)), "<redacted>");
        assert_eq!(redact_capture_text(&"Z".repeat(99)), "Z".repeat(99));
    }

    #[test]
    fn lenient_base64() {
        assert_eq!(decode_base64_lenient("aGk="), b"hi");
        assert_eq!(decode_base64_lenient("a G\nk"), b"hi");
        assert_eq!(decode_base64_lenient("-_8"), vec![0xfb, 0xff]);
        assert_eq!(decode_base64_lenient("+/8="), vec![0xfb, 0xff]);
    }

    #[test]
    fn json_example() {
        let token = jwt(r#"{"userId":"UID-9"}"#);
        let input = json!({"Username":"Alex","InviteCode":"ABCD-EFGH","Password":"hunter2","UUK":"UUK_k1",
            "nested":[{"access_token": token,"note":"x"}], "n": 5});
        let hash = crate::hex(&Sha256::digest(b"UUK_k1"));
        assert_eq!(
            redact_capture_json(&input),
            json!({"Username":"Alex","InviteCode":"<redacted>","Password":"<redacted>","UUK":format!("<key:{hash}>"),
                "nested":[{"access_token":"<token:UID-9>","note":"x"}], "n": 5})
        );
    }

    #[test]
    fn headers() {
        let mut h = HeaderMap::new();
        h.insert("user-agent", "Game/1".parse().unwrap());
        h.insert("cookie", "a=b".parse().unwrap());
        h.insert("x-reforged-gateway", "s".parse().unwrap());
        h.insert("x-reforged-user-api-key", "RRK_abc".parse().unwrap());
        h.insert("x-reforged-admin-key", "adm".parse().unwrap());
        h.insert("x-rustic-isles-gameserver-key", "g".parse().unwrap());
        h.append("accept", "a".parse().unwrap());
        h.append("accept", "b".parse().unwrap());
        let v = redact_capture_headers(&h);
        let hash = crate::hex(&Sha256::digest(b"RRK_abc"));
        assert_eq!(
            v,
            json!({"user-agent":"Game/1","x-reforged-user-api-key":format!("<key:{hash}>"),
                "x-reforged-admin-key":"<redacted>","x-rustic-isles-gameserver-key":"<gskey>","accept":"a, b"})
        );
    }

    #[test]
    fn bodies() {
        let json_type = Some("application/json");
        assert_eq!(redact_capture_body(b"20000", json_type, 5, 100), json!({"json":20000}));
        assert_eq!(
            redact_capture_body(b"{nope", json_type, 5, 100),
            json!({"text":"{nope"})
        );
        assert_eq!(redact_capture_body(&[0xff, 0x41], None, 2, 100), json!({"binary":2}));
        assert_eq!(redact_capture_body(b"", None, 0, 100), Value::Null);
        assert_eq!(redact_capture_body(b"abc", None, 5, 100), json!({"omitted":5}));
        assert_eq!(redact_capture_body(b"abc", None, 3, 2), json!({"omitted":3}));
        assert_eq!(
            redact_capture_body(
                b"user=a&password=b+c",
                Some("application/x-www-form-urlencoded"),
                19,
                100
            ),
            json!({"form":[["user","a"],["password","<redacted>"]]})
        );
        assert_eq!(redact_capture_body(b" [1]", None, 4, 100), json!({"json":[1]}));
    }

    #[test]
    fn chat_frames() {
        let auth = base64::engine::general_purpose::STANDARD.encode(b"\0player_1\0secret");
        let frame = format!("<auth mechanism='PLAIN'> {auth} </auth><password>x</password>");
        assert_eq!(
            redact_chat_frame(&frame),
            "<auth mechanism='PLAIN'><plain:player_1></auth><password><redacted></password>"
        );
        let bad = base64::engine::general_purpose::STANDARD.encode(b"\0a b\0x");
        assert_eq!(
            redact_chat_frame(&format!("<auth>{bad}</auth>")),
            "<auth><redacted></auth>"
        );
    }

    fn frame(fin: bool, opcode: u8, rsv1: bool, payload: &[u8], mask: Option<[u8; 4]>) -> Vec<u8> {
        let mut out = vec![(u8::from(fin) << 7) | (u8::from(rsv1) << 6) | opcode];
        let m = if mask.is_some() { 0x80 } else { 0 };
        match payload.len() {
            n if n < 126 => out.push(m | n as u8),
            n if n < 65536 => {
                out.push(m | 126);
                out.extend_from_slice(&(n as u16).to_be_bytes());
            }
            n => {
                out.push(m | 127);
                out.extend_from_slice(&(n as u64).to_be_bytes());
            }
        }
        if let Some(k) = mask {
            out.extend_from_slice(&k);
        }
        out.extend(
            payload
                .iter()
                .enumerate()
                .map(|(i, b)| b ^ mask.map_or(0, |k| k[i % 4])),
        );
        out
    }

    #[test]
    fn frame_reader_is_split_proof() {
        let mut stream = Vec::new();
        stream.extend(frame(false, 1, false, b"hel", Some([1, 2, 3, 4])));
        stream.extend(frame(true, 9, false, b"ping", None));
        stream.extend(frame(true, 0, false, b"lo", Some([9, 9, 9, 9])));
        stream.extend(frame(true, 1, false, b"", None));
        stream.extend(frame(true, 2, true, b"zz", None));
        stream.extend(frame(true, 1, false, &vec![b'x'; 300], None));
        stream.extend(frame(false, 1, false, b"dropped", None));
        stream.extend(frame(true, 1, false, b"last", None));
        let expected = vec![
            ChatMessage::Text("hello".into()),
            ChatMessage::Text(String::new()),
            ChatMessage::Compressed(2),
            ChatMessage::Omitted(300),
            ChatMessage::Text("last".into()),
        ];
        let mut whole = FrameReader::new(200);
        assert_eq!(whole.push(&stream), expected);
        let mut single = FrameReader::new(200);
        let got: Vec<ChatMessage> = stream.iter().flat_map(|b| single.push(&[*b])).collect();
        assert_eq!(got, expected);
        let mut big = FrameReader::new(100_000);
        assert_eq!(big.push(&frame(true, 1, false, &vec![b'y'; 70_000], None)).len(), 1);
    }

    #[test]
    fn writer_lines_and_pruning() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("gateway-2000-01-01.jsonl"), "").unwrap();
        std::fs::write(dir.path().join("other.txt"), "").unwrap();
        std::fs::write(dir.path().join("Gateway-2000-01-02.jsonl"), "").unwrap();
        let w = CaptureWriter::new(CaptureOptions {
            dir: dir.path().to_owned(),
            retention_days: 14,
            max_body_bytes: 10,
            source: "gateway".into(),
        })
        .unwrap();
        assert!(!dir.path().join("gateway-2000-01-01.jsonl").exists());
        assert!(dir.path().join("other.txt").exists() && dir.path().join("Gateway-2000-01-02.jsonl").exists());
        let secret = "s3cr\"t-value";
        w.add_secret(secret);
        w.add_secret("short");
        let at = chrono::DateTime::parse_from_rfc3339("2026-10-21T00:00:01Z")
            .unwrap()
            .timestamp_millis();
        w.write_at(&json!({"kind":"http","seq":2,"echo":secret,"s":"short"}), at);
        let text = std::fs::read_to_string(dir.path().join("gateway-2026-10-21.jsonl")).unwrap();
        assert_eq!(
            text,
            "{\"t\":\"2026-10-21T00:00:01.000Z\",\"src\":\"gateway\",\"kind\":\"http\",\"seq\":2,\"echo\":\"<redacted>\",\"s\":\"short\"}\n"
        );
        assert_eq!(file_day("gateway-2026-10-21.jsonl"), Some("2026-10-21"));
        assert_eq!(file_day("gate-way-2026-10-21.jsonl"), None);
    }

    #[test]
    fn settings() {
        let env = Env::from_pairs([("CAPTURE_DIR", " cap "), ("CAPTURE_RETENTION_DAYS", "91")]);
        assert_eq!(
            load_capture_options(&env).unwrap_err(),
            "CAPTURE_RETENTION_DAYS must be a whole number from 1 to 90 (got \"91\")"
        );
        assert_eq!(load_capture_options(&Env::default()).unwrap(), None);
        let env = Env::from_pairs([("CAPTURE_DIR", "cap")]);
        let o = load_capture_options(&env).unwrap().unwrap();
        assert_eq!((o.retention_days, o.max_body_bytes), (14, 262_144));
        assert!(o.dir.is_absolute());
    }
}
