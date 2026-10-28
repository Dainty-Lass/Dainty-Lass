//! RU8 shadow replay harness and response normalizer.
//!
//! Parses gateway capture `.jsonl` corpora (`kind: "http"` and `kind: "chat"`), deterministically substitutes
//! capture placeholders (`<token:UID-...>`, `<token>`, `<key:sha256>`, `<gskey>`, `<plain:account>`), replays
//! traffic against live HTTP/chat targets or in-process `axum::Router` instances (or compares a recorded corpus
//! against a live target), normalizes legitimate non-deterministic wire differences (minted JWTs, opaque tokens,
//! volatile headers, unpinned timestamps, and first-appearance dynamic IDs), and classifies diffs into
//! [`DiffKind::StatusDiff`], [`DiffKind::HeaderDiff`], [`DiffKind::ValueDiff`], [`DiffKind::TextDiff`], and
//! [`DiffKind::ChatStanzaDiff`].

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::{STANDARD as BASE64_STANDARD, URL_SAFE_NO_PAD as BASE64_URL};
use hmac::{Hmac, KeyInit, Mac};
use percent_encoding::{AsciiSet, CONTROLS, utf8_percent_encode};
use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

use crate::game_data::season_window::{REPLAY_NOW_HEADER, REPLAY_SEED_HEADER};

type HmacSha256 = Hmac<Sha256>;

const FORM_ENCODE_SET: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'!')
    .add(b'"')
    .add(b'#')
    .add(b'$')
    .add(b'%')
    .add(b'&')
    .add(b'\'')
    .add(b'(')
    .add(b')')
    .add(b'+')
    .add(b',')
    .add(b'/')
    .add(b':')
    .add(b';')
    .add(b'<')
    .add(b'=')
    .add(b'>')
    .add(b'?')
    .add(b'@')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}')
    .add(b'~');

/// Errors returned by the shadow replay harness.
#[derive(Debug, thiserror::Error)]
pub enum ReplayError {
    #[error("I/O error at {path}: {message}")]
    Io { path: String, message: String },
    #[error("invalid replay configuration: {0}")]
    Config(String),
    #[error("token signing error: {0}")]
    Token(String),
    #[error("HTTP transport error calling {target}: {message}")]
    Transport { target: String, message: String },
    #[error("JSON serialization error: {0}")]
    Json(String),
}

// ---------------------------------------------------------------------------
// 1. Capture JSONL Record Types & Parser
// ---------------------------------------------------------------------------

/// Captured HTTP request or response body representation from `dainty-lass` gateway `capture.rs`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub enum CaptureBody {
    #[default]
    Null,
    Json(Value),
    Form(Vec<(String, String)>),
    Text(String),
    Binary(u64),
    Omitted(u64),
}

impl CaptureBody {
    /// Parses the gateway capture body object (`null` | `{"json":..}` | `{"form":..}` | `{"text":..}` | `{"binary":n}` | `{"omitted":n}`).
    pub fn from_wire(value: Option<&Value>) -> Self {
        let Some(Value::Object(map)) = value else {
            return Self::Null;
        };
        if let Some(json_val) = map.get("json") {
            return Self::Json(json_val.clone());
        }
        if let Some(Value::Array(pairs)) = map.get("form") {
            let mut form = Vec::with_capacity(pairs.len());
            for pair in pairs {
                if let Value::Array(items) = pair
                    && items.len() == 2
                {
                    let k = items[0].as_str().unwrap_or_default().to_owned();
                    let v = items[1].as_str().unwrap_or_default().to_owned();
                    form.push((k, v));
                }
            }
            return Self::Form(form);
        }
        if let Some(Value::String(text)) = map.get("text") {
            return Self::Text(text.clone());
        }
        if let Some(n) = map.get("binary").and_then(Value::as_u64) {
            return Self::Binary(n);
        }
        if let Some(n) = map.get("omitted").and_then(Value::as_u64) {
            return Self::Omitted(n);
        }
        Self::Null
    }

    /// Converts back to the gateway capture JSON representation.
    pub fn to_wire(&self) -> Value {
        match self {
            Self::Null => Value::Null,
            Self::Json(v) => json!({ "json": v }),
            Self::Form(pairs) => {
                let arr: Vec<Value> = pairs.iter().map(|(k, v)| json!([k, v])).collect();
                json!({ "form": arr })
            }
            Self::Text(t) => json!({ "text": t }),
            Self::Binary(n) => json!({ "binary": n }),
            Self::Omitted(n) => json!({ "omitted": n }),
        }
    }

    /// True if the captured body could not be replayed byte-for-byte (`binary` or `omitted`).
    pub fn is_lossy(&self) -> bool {
        matches!(self, Self::Binary(_) | Self::Omitted(_))
    }
}

/// One `"kind": "http"` line from a gateway capture `.jsonl` file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

/// One `"kind": "chat"` line from a gateway capture `.jsonl` file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

/// A parsed gateway capture record.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CaptureRecord {
    Http(Box<HttpRecord>),
    Chat(ChatRecord),
}

impl CaptureRecord {
    pub fn timestamp(&self) -> &str {
        match self {
            Self::Http(h) => &h.t,
            Self::Chat(c) => &c.t,
        }
    }

    pub fn timestamp_ms(&self) -> i64 {
        service_kit::js::date_parse(self.timestamp()).unwrap_or(0)
    }
}

/// Parses a single JSONL line into a [`CaptureRecord`]. Blank or truncated lines return `None`.
pub fn parse_capture_line(line: &str) -> Option<CaptureRecord> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let Value::Object(obj) = serde_json::from_str::<Value>(trimmed).ok()? else {
        return None;
    };
    let kind = obj.get("kind").and_then(Value::as_str)?;
    let t = obj.get("t").and_then(Value::as_str).unwrap_or_default().to_owned();
    let src = obj.get("src").and_then(Value::as_str).unwrap_or("gateway").to_owned();
    let conn = obj.get("conn").and_then(Value::as_u64)?;
    match kind {
        "http" => {
            let method = obj.get("method").and_then(Value::as_str)?.to_owned();
            let target = obj.get("target").and_then(Value::as_str)?.to_owned();
            let seq = obj.get("seq").and_then(Value::as_u64).unwrap_or(0);
            let ip = obj.get("ip").and_then(Value::as_str).unwrap_or("127.0.0.1").to_owned();
            let route = obj.get("route").and_then(Value::as_str).unwrap_or("backend").to_owned();
            let req_headers = obj
                .get("req_headers")
                .or_else(|| obj.get("reqHeaders"))
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            let req_body = CaptureBody::from_wire(obj.get("req_body").or_else(|| obj.get("reqBody")));
            let status = obj
                .get("status")
                .and_then(Value::as_u64)
                .and_then(|n| u16::try_from(n).ok())
                .unwrap_or(0);
            let res_headers = obj
                .get("res_headers")
                .or_else(|| obj.get("resHeaders"))
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            let res_body = CaptureBody::from_wire(obj.get("res_body").or_else(|| obj.get("resBody")));
            let ms = obj.get("ms").and_then(Value::as_u64).unwrap_or(0);
            let aborted = obj.get("aborted").and_then(Value::as_bool).unwrap_or(false);
            Some(CaptureRecord::Http(Box::new(HttpRecord {
                t,
                src,
                conn,
                seq,
                ip,
                method,
                target,
                route,
                req_headers,
                req_body,
                status,
                res_headers,
                res_body,
                ms,
                aborted,
            })))
        }
        "chat" => {
            let str_field = |name: &str| obj.get(name).and_then(Value::as_str).map(str::to_owned);
            Some(CaptureRecord::Chat(ChatRecord {
                t,
                src,
                conn,
                event: str_field("event"),
                dir: str_field("dir"),
                text: str_field("text"),
                omitted: obj.get("omitted").and_then(Value::as_u64),
                compressed: obj.get("compressed").and_then(Value::as_u64),
                ip: str_field("ip"),
                target: str_field("target"),
                via: str_field("via"),
                reason: str_field("reason"),
            }))
        }
        _ => None,
    }
}

/// Parses JSONL text into a vector of [`CaptureRecord`]s in line order.
pub fn parse_capture_str(text: &str) -> Vec<CaptureRecord> {
    text.lines().filter_map(parse_capture_line).collect()
}

/// Loads a capture `.jsonl` file or directory of `.jsonl` files, stably sorted by timestamp `t`.
pub fn load_corpus(path: &Path) -> Result<Vec<CaptureRecord>, ReplayError> {
    let files = if path.is_dir() {
        let entries = fs::read_dir(path).map_err(|e| ReplayError::Io {
            path: path.display().to_string(),
            message: e.to_string(),
        })?;
        let mut list: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|ext| ext.to_str()) == Some("jsonl"))
            .collect();
        list.sort();
        list
    } else {
        vec![path.to_path_buf()]
    };

    let mut indexed: Vec<(i64, usize, usize, CaptureRecord)> = Vec::new();
    for (file_idx, file) in files.iter().enumerate() {
        let content = fs::read_to_string(file).map_err(|e| ReplayError::Io {
            path: file.display().to_string(),
            message: e.to_string(),
        })?;
        for (line_idx, line) in content.lines().enumerate() {
            if let Some(record) = parse_capture_line(line) {
                indexed.push((record.timestamp_ms(), file_idx, line_idx, record));
            }
        }
    }
    indexed.sort_by_key(|a| (a.0, a.1, a.2));
    Ok(indexed.into_iter().map(|(_, _, _, r)| r).collect())
}

// ---------------------------------------------------------------------------
// 2. Deterministic Placeholder Substitution
// ---------------------------------------------------------------------------

static TOKEN_SUB_RE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"<token:([^>]+)>").ok());
static KEY_HASH_RE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"<key:([0-9a-fA-F]{64})>").ok());
static PLAIN_SUB_RE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"<plain:([^>]+)>").ok());

/// Credentials and key mappings used to substitute capture placeholders for one replay target.
#[derive(Debug, Clone, Default)]
pub struct ReplayTargetAuth {
    /// HS256 secret (`JWT_SECRET`) when targeting a service using symmetric JWTs.
    pub jwt_secret: Option<String>,
    /// Base64-encoded PEM RSA private key (`AUTH_SIGNING_PRIVKEY_B64`) when targeting `isles-core`.
    pub signing_key_b64: Option<String>,
    /// Game-server API key (`GAMESERVER_KEY` / `X-MysticParadox-GameServer-Key`).
    pub gs_key: Option<String>,
    /// Optional `x-gateway-secret` header injected on forwarded requests.
    pub gateway_secret: Option<String>,
    /// Map of lowercase 64-char SHA-256 hex digest -> raw `RRK_...` or `UUK_...` account key.
    pub key_map: HashMap<String, String>,
    /// When true, unknown `<key:sha256>` placeholders are replaced with a deterministic `RRK_replay_<hash[..24]>` key.
    pub deterministic_fallback_keys: bool,
}

impl ReplayTargetAuth {
    /// Creates a [`ReplayTargetAuth`] pre-configured with `isles-core`'s process-wide throwaway RS256 test signing key
    /// and optional `gs_key`.
    pub fn with_test_keys(gs_key: Option<&str>) -> Self {
        let settings = crate::areas::auth::tokens::test_key_settings(&[]);
        let privkey = settings.raw("AUTH_SIGNING_PRIVKEY_B64").map(str::to_owned);
        Self {
            jwt_secret: None,
            signing_key_b64: privkey,
            gs_key: gs_key.map(str::to_owned),
            gateway_secret: None,
            key_map: HashMap::new(),
            deterministic_fallback_keys: true,
        }
    }

    /// Mints a valid JWT for `subject` (`<token:UID-...>`). Uses RS256 when `signing_key_b64` is configured,
    /// otherwise HS256 with `jwt_secret` (defaulting to `"replay-default-jwt-secret-key-32b"`).
    pub fn mint_token(&self, subject: &str, now_ms: i64) -> Result<String, ReplayError> {
        if let Some(privkey_b64) = &self.signing_key_b64 {
            return mint_rs256_jwt(subject, privkey_b64, now_ms);
        }
        let secret = self
            .jwt_secret
            .as_deref()
            .unwrap_or("replay-default-jwt-secret-key-32b");
        mint_hs256_jwt(subject, secret, now_ms)
    }

    /// Computes the SHA-256 hex key-map entry for a raw `RRK_...` or `UUK_...` account key and inserts it.
    pub fn register_account_key(&mut self, raw_key: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(raw_key.as_bytes());
        let hash = hex_lower(&hasher.finalize());
        self.key_map.insert(hash.clone(), raw_key.to_owned());
        hash
    }
}

/// Mints a deterministic RS256 game JWT (`iss: "paradox-backend"`, `aud: "paradox-backend"`) for `subject`
/// using base64-encoded RSA PEM private key `privkey_b64`.
pub fn mint_rs256_jwt(subject: &str, privkey_b64: &str, now_ms: i64) -> Result<String, ReplayError> {
    let pem_bytes = BASE64_STANDARD
        .decode(privkey_b64.trim())
        .map_err(|e| ReplayError::Token(format!("invalid base64 signing key: {e}")))?;
    let encoding_key = jsonwebtoken::EncodingKey::from_rsa_pem(&pem_bytes)
        .map_err(|e| ReplayError::Token(format!("invalid RSA PEM signing key: {e}")))?;
    let iat = now_ms.div_euclid(1000);
    let exp = iat.saturating_add(86_400).max(4_102_444_800);
    let claims = json!({
        "userId": subject,
        "iat": iat,
        "exp": exp,
        "aud": "paradox-backend",
        "iss": "paradox-backend",
    });
    jsonwebtoken::encode(
        &jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
        &claims,
        &encoding_key,
    )
    .map_err(|e| ReplayError::Token(e.to_string()))
}

/// Mints a deterministic HS256 JWT (`header.payload.sig`) for `subject`.
pub fn mint_hs256_jwt(subject: &str, secret: &str, now_ms: i64) -> Result<String, ReplayError> {
    let iat = now_ms.div_euclid(1000);
    let exp = iat.saturating_add(86_400).max(4_102_444_800);
    let header = BASE64_URL.encode(r#"{"alg":"HS256","typ":"JWT"}"#.as_bytes());
    let payload_json = json!({
        "userId": subject,
        "sub": subject,
        "iat": iat,
        "exp": exp,
        "iss": "mysticparadox",
        "aud": "mysticparadox-game"
    });
    let payload_bytes = serde_json::to_vec(&payload_json).map_err(|e| ReplayError::Json(e.to_string()))?;
    let payload = BASE64_URL.encode(&payload_bytes);
    let signing_input = format!("{header}.{payload}");
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).map_err(|e| ReplayError::Token(e.to_string()))?;
    mac.update(signing_input.as_bytes());
    let sig = BASE64_URL.encode(mac.finalize().into_bytes());
    Ok(format!("{signing_input}.{sig}"))
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

/// Result of substituting placeholders in a string or JSON value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Substituted<T> {
    pub value: T,
    /// True when the input contained `<redacted>` or an unresolvable `<key:...>` / `<gskey>`.
    pub inexact: bool,
}

/// Substitutes `<token:...>`, `<token>`, `<key:...>`, `<gskey>`, and `<plain:...>` in a string.
pub fn substitute_str(text: &str, auth: &ReplayTargetAuth, now_ms: i64) -> Substituted<String> {
    let mut inexact = text.contains("<redacted>");
    let mut current = text.to_owned();

    if let Some(re) = TOKEN_SUB_RE.as_ref()
        && current.contains("<token:")
    {
        current = re
            .replace_all(&current, |caps: &Captures<'_>| {
                let sub = &caps[1];
                match auth.mint_token(sub, now_ms) {
                    Ok(tok) => tok,
                    Err(_) => {
                        inexact = true;
                        caps[0].to_owned()
                    }
                }
            })
            .into_owned();
    }

    if current.contains("<token>") {
        match auth.mint_token("UID-replay-anon", now_ms) {
            Ok(tok) => current = current.replace("<token>", &tok),
            Err(_) => inexact = true,
        }
    }

    if let Some(re) = KEY_HASH_RE.as_ref()
        && current.contains("<key:")
    {
        current = re
            .replace_all(&current, |caps: &Captures<'_>| {
                let hash = caps[1].to_ascii_lowercase();
                if let Some(raw_key) = auth.key_map.get(&hash) {
                    raw_key.clone()
                } else if auth.deterministic_fallback_keys {
                    format!("RRK_replay_{}", &hash[..24])
                } else {
                    inexact = true;
                    caps[0].to_owned()
                }
            })
            .into_owned();
    }

    if current.contains("<gskey>") {
        if let Some(gs) = &auth.gs_key {
            current = current.replace("<gskey>", gs);
        } else {
            inexact = true;
        }
    }

    if let Some(re) = PLAIN_SUB_RE.as_ref()
        && current.contains("<plain:")
    {
        current = re
            .replace_all(&current, |caps: &Captures<'_>| {
                let account = &caps[1];
                match auth.mint_token(account, now_ms) {
                    Ok(tok) => {
                        let raw = format!("\0{account}\0{tok}");
                        BASE64_STANDARD.encode(raw.as_bytes())
                    }
                    Err(_) => {
                        inexact = true;
                        caps[0].to_owned()
                    }
                }
            })
            .into_owned();
    }

    Substituted {
        value: current,
        inexact,
    }
}

/// Substitutes placeholders recursively across a JSON [`Value`].
pub fn substitute_value(value: &Value, auth: &ReplayTargetAuth, now_ms: i64) -> Substituted<Value> {
    match value {
        Value::String(s) => {
            let sub = substitute_str(s, auth, now_ms);
            Substituted {
                value: Value::String(sub.value),
                inexact: sub.inexact,
            }
        }
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            let mut inexact = false;
            for item in items {
                let sub = substitute_value(item, auth, now_ms);
                inexact |= sub.inexact;
                out.push(sub.value);
            }
            Substituted {
                value: Value::Array(out),
                inexact,
            }
        }
        Value::Object(map) => {
            let mut out = Map::new();
            let mut inexact = false;
            for (k, v) in map {
                let sub = substitute_value(v, auth, now_ms);
                inexact |= sub.inexact;
                out.insert(k.clone(), sub.value);
            }
            Substituted {
                value: Value::Object(out),
                inexact,
            }
        }
        other => Substituted {
            value: other.clone(),
            inexact: false,
        },
    }
}

// ---------------------------------------------------------------------------
// 3. Learned ID Mapping (`learn_ids` / `apply_ids`)
// ---------------------------------------------------------------------------

static UUID_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b").ok());
static PARTY_ID_RE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"\b(?:[a-z0-9-]+-[0-9a-f]{16,32}|[0-9a-f]{32}|[0-9A-HJKMNP-TV-Z]{26}\.[a-z0-9-]+)_([A-Za-z0-9+/=-]+)")
        .ok()
});
static GRANT_ID_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?i)\bgs-[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b").ok());
static INSTANCE_ID_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"\b(?:[A-Z0-9]{26}|[0-9a-f]{16,24})\b").ok());
static HEX32_RE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"\b[0-9a-f]{32}\b").ok());

fn looks_like_dynamic_id(text: &str, key_hint: Option<&str>) -> bool {
    if matches!(
        key_hint,
        Some("id" | "characterId" | "instanceId" | "partyId" | "candidateId" | "gameSessionId" | "grantId")
    ) && !text.is_empty()
        && text != "INVALID"
    {
        return true;
    }
    GRANT_ID_RE.as_ref().is_some_and(|r| r.is_match(text))
        || PARTY_ID_RE.as_ref().is_some_and(|r| r.is_match(text))
        || UUID_RE.as_ref().is_some_and(|r| r.is_match(text))
        || (INSTANCE_ID_RE.as_ref().is_some_and(|r| r.is_match(text)) && text.bytes().any(|b| b.is_ascii_digit()))
        || HEX32_RE.as_ref().is_some_and(|r| r.is_match(text))
}

/// Learns generated ID mappings from `recorded` (corpus response) -> `actual` (target response) at matching JSON paths.
pub fn learn_ids(recorded: &Value, actual: &Value, id_map: &mut HashMap<String, String>) {
    learn_ids_at(recorded, actual, None, id_map);
}

fn learn_ids_at(recorded: &Value, actual: &Value, key_hint: Option<&str>, id_map: &mut HashMap<String, String>) {
    match (recorded, actual) {
        (Value::String(rec_str), Value::String(act_str)) => {
            if rec_str != act_str && looks_like_dynamic_id(rec_str, key_hint) && !act_str.is_empty() {
                id_map.entry(rec_str.clone()).or_insert_with(|| act_str.clone());
            } else if rec_str.starts_with('{') && act_str.starts_with('{') {
                // Double-encoded JSON strings (e.g. Loadout `data` or Character `data`)
                if let (Ok(rec_inner), Ok(act_inner)) = (
                    serde_json::from_str::<Value>(rec_str),
                    serde_json::from_str::<Value>(act_str),
                ) {
                    learn_ids_at(&rec_inner, &act_inner, None, id_map);
                }
            }
        }
        (Value::Array(rec_arr), Value::Array(act_arr)) => {
            for (r, a) in rec_arr.iter().zip(act_arr.iter()) {
                learn_ids_at(r, a, key_hint, id_map);
            }
        }
        (Value::Object(rec_obj), Value::Object(act_obj)) => {
            for (k, r_val) in rec_obj {
                if let Some(a_val) = act_obj.get(k) {
                    learn_ids_at(r_val, a_val, Some(k.as_str()), id_map);
                }
            }
        }
        _ => {}
    }
}

/// Applies learned ID substitutions (`recorded_id -> target_id`) to a string.
pub fn apply_ids_str(text: &str, id_map: &HashMap<String, String>) -> String {
    if id_map.is_empty() {
        return text.to_owned();
    }
    let mut keys: Vec<(&String, &String)> = id_map.iter().collect();
    // Replace longest IDs first so a prefix never corrupts a longer token.
    keys.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(b.0)));
    let mut out = text.to_owned();
    for (from, to) in keys {
        if out.contains(from.as_str()) {
            out = out.replace(from.as_str(), to.as_str());
        }
    }
    out
}

/// Applies learned ID substitutions recursively across a JSON [`Value`].
pub fn apply_ids_value(value: &Value, id_map: &HashMap<String, String>) -> Value {
    if id_map.is_empty() {
        return value.clone();
    }
    match value {
        Value::String(s) => Value::String(apply_ids_str(s, id_map)),
        Value::Array(items) => Value::Array(items.iter().map(|i| apply_ids_value(i, id_map)).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (apply_ids_str(k, id_map), apply_ids_value(v, id_map)))
                .collect(),
        ),
        other => other.clone(),
    }
}

// ---------------------------------------------------------------------------
// 4. Request Preparation
// ---------------------------------------------------------------------------

const HOP_BY_HOP_REQUEST_HEADERS: [&str; 5] = [
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
    "keep-alive",
];

/// An HTTP request prepared from an [`HttpRecord`] for one target.
#[derive(Debug, Clone)]
pub struct PreparedHttpRequest {
    pub method: String,
    pub target: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub inexact: bool,
}

/// Prepares an [`HttpRecord`] for a target by applying learned IDs, substituting placeholders, and optionally
/// attaching clock/seed pinning headers.
pub fn prepare_http_request(
    record: &HttpRecord,
    auth: &ReplayTargetAuth,
    id_map: &HashMap<String, String>,
    pin_clock: bool,
) -> PreparedHttpRequest {
    let now_ms = service_kit::js::date_parse(&record.t).unwrap_or(1_793_016_000_000);
    let mut inexact = record.aborted || record.req_body.is_lossy() || record.res_body.is_lossy();

    let raw_target = apply_ids_str(&record.target, id_map);
    let sub_target = substitute_str(&raw_target, auth, now_ms);
    inexact |= sub_target.inexact;
    let mut target = sub_target.value;
    // Gateway `/game/login` -> backend `/login` normalization when replaying against backend router directly
    if (record.route == "backend" || record.route == "metagame") && target == "/game/login" {
        target = "/login".to_owned();
    } else if (record.route == "backend" || record.route == "metagame")
        && let Some(rest) = target.strip_prefix("/game/login?")
    {
        target = format!("/login?{rest}");
    }

    let mut headers: Vec<(String, String)> = Vec::new();
    let mut has_content_type = false;
    let mut has_forwarded_for = false;

    for (name, val) in &record.req_headers {
        let lower = name.to_ascii_lowercase();
        if HOP_BY_HOP_REQUEST_HEADERS.contains(&lower.as_str()) {
            continue;
        }
        if lower == "content-type" {
            has_content_type = true;
        }
        if lower == "x-forwarded-for" {
            has_forwarded_for = true;
        }
        let raw_val = match val {
            Value::String(s) => s.clone(),
            Value::Array(arr) => arr.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "),
            other => other.to_string(),
        };
        let mapped = apply_ids_str(&raw_val, id_map);
        let sub = substitute_str(&mapped, auth, now_ms);
        let ignore_basic_redacted = lower == "authorization"
            && raw_val.to_ascii_lowercase().starts_with("basic <redacted>")
            && target.starts_with("/account/api/oauth/token");
        if !ignore_basic_redacted {
            inexact |= sub.inexact;
        }
        headers.push((lower, sub.value));
    }

    let body = match &record.req_body {
        CaptureBody::Null | CaptureBody::Binary(_) | CaptureBody::Omitted(_) => Vec::new(),
        CaptureBody::Json(val) => {
            let mapped = apply_ids_value(val, id_map);
            let sub = substitute_value(&mapped, auth, now_ms);
            inexact |= sub.inexact;
            if !has_content_type {
                headers.push(("content-type".to_owned(), "application/json".to_owned()));
            }
            service_kit::jsjson::stringify(&sub.value).into_bytes()
        }
        CaptureBody::Form(pairs) => {
            let mut encoded_pairs = Vec::with_capacity(pairs.len());
            for (k, v) in pairs {
                let mapped_k = apply_ids_str(k, id_map);
                let sub_k = substitute_str(&mapped_k, auth, now_ms);
                let mapped_v = apply_ids_str(v, id_map);
                let sub_v = substitute_str(&mapped_v, auth, now_ms);
                inexact |= sub_k.inexact || sub_v.inexact;
                let ek = utf8_percent_encode(&sub_k.value, FORM_ENCODE_SET).to_string();
                let ev = utf8_percent_encode(&sub_v.value, FORM_ENCODE_SET).to_string();
                encoded_pairs.push(format!("{ek}={ev}"));
            }
            if !has_content_type {
                headers.push((
                    "content-type".to_owned(),
                    "application/x-www-form-urlencoded".to_owned(),
                ));
            }
            encoded_pairs.join("&").into_bytes()
        }
        CaptureBody::Text(text) => {
            let mapped = apply_ids_str(text, id_map);
            let sub = substitute_str(&mapped, auth, now_ms);
            inexact |= sub.inexact;
            sub.value.into_bytes()
        }
    };

    if !has_forwarded_for && !record.ip.is_empty() {
        headers.push(("x-forwarded-for".to_owned(), record.ip.clone()));
    }
    if let Some(secret) = &auth.gateway_secret {
        headers.retain(|(k, _)| k != "x-gateway-secret");
        headers.push(("x-gateway-secret".to_owned(), secret.clone()));
    }
    if pin_clock {
        headers.push((REPLAY_NOW_HEADER.to_owned(), now_ms.to_string()));
        headers.push((
            REPLAY_SEED_HEADER.to_owned(),
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

// ---------------------------------------------------------------------------
// 5. Response Normalizer & Diff Classification
// ---------------------------------------------------------------------------

static JWT_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+").ok());
static ISO_DATE_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"\b\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})\b").ok());
static LOGIN_TIME_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"\b\d{4}\.\d{2}\.\d{2}-\d{2}\.\d{2}\.\d{2}\b").ok());
static DISPLAY_DATE_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"\b(?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec) \d{1,2}, \d{4}\b").ok());
static CODE_CHALLENGE_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"code_challenge=[A-Za-z0-9_-]{40,64}").ok());
static UPTIME_JSON_RE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r#""uptimeSeconds"\s*:\s*\d+"#).ok());
static PID_JSON_RE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r#""processId"\s*:\s*\d+"#).ok());

///Volatile transport headers stripped before comparing HTTP responses.
pub const IGNORED_RESPONSE_HEADERS: &[&str] = &[
    "date",
    "etag",
    "x-powered-by",
    "connection",
    "keep-alive",
    "content-length",
    "transfer-encoding",
    "x-mysticparadox-request-id",
    "x-request-id",
    "server",
    "access-control-allow-origin",
    "access-control-allow-methods",
    "access-control-allow-headers",
    "access-control-allow-credentials",
    "vary",
];

/// Raw HTTP response captured or returned by a live target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayHttpResponse {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
    pub body_text: String,
}

impl ReplayHttpResponse {
    /// Constructs a [`ReplayHttpResponse`] from a recorded [`HttpRecord`].
    pub fn from_recorded(record: &HttpRecord) -> Self {
        let mut headers = BTreeMap::new();
        for (k, v) in &record.res_headers {
            let val = match v {
                Value::String(s) => s.clone(),
                Value::Array(arr) => arr.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "),
                other => other.to_string(),
            };
            headers.insert(k.to_ascii_lowercase(), val);
        }
        let body_text = match &record.res_body {
            CaptureBody::Null | CaptureBody::Binary(_) | CaptureBody::Omitted(_) => String::new(),
            CaptureBody::Json(val) => service_kit::jsjson::stringify(val),
            CaptureBody::Form(pairs) => pairs
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("&"),
            CaptureBody::Text(text) => text.clone(),
        };
        Self {
            status: record.status,
            headers,
            body_text,
        }
    }
}

/// Classification of a difference between the left and right replay targets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum DiffKind {
    /// HTTP status code differed (`200` vs `500`, etc.).
    StatusDiff { left: u16, right: u16 },
    /// Normalized HTTP response header differed (`content-type`, `x-mystic-party-revision`, etc.).
    HeaderDiff {
        header: String,
        left: Option<String>,
        right: Option<String>,
    },
    /// Semantic JSON value differed at `path` after normalization.
    ValueDiff { path: String, left: Value, right: Value },
    /// Raw JSON text (e.g. object key ordering or number formatting) or non-JSON body text differed after normalization.
    TextDiff { left: String, right: String },
    /// XMPP chat stanza differed on connection `conn` at stanza index `index`.
    ChatStanzaDiff {
        conn: u64,
        index: usize,
        left: Option<String>,
        right: Option<String>,
    },
}

/// Options controlling the response normalizer.
#[derive(Debug, Clone)]
pub struct NormalizerConfig {
    /// Normalize ISO-8601, login-time, and display-date timestamps (`<date>`, `<login-time>`, `<display-date>`).
    pub normalize_timestamps: bool,
    /// Normalize generated UUIDs, instance IDs, party IDs, and grant IDs by first-appearance numbering per side.
    pub normalize_dynamic_ids: bool,
    /// Also compare raw JSON text (catching key-order and float-format differences even when `ValueDiff` is empty).
    pub check_raw_json_text: bool,
}

impl Default for NormalizerConfig {
    fn default() -> Self {
        Self {
            normalize_timestamps: true,
            normalize_dynamic_ids: true,
            check_raw_json_text: true,
        }
    }
}

/// Per-target stateful normalizer that assigns sequential aliases (`<uuid#1>`, `<iid#1>`, `<party#1>`) by first
/// appearance so co-reference structure is preserved across a replay session.
#[derive(Debug, Clone, Default)]
pub struct IdNormalizer {
    config: NormalizerConfig,
    uuids: HashMap<String, usize>,
    iids: HashMap<String, usize>,
    parties: HashMap<String, usize>,
    grants: HashMap<String, usize>,
}

impl IdNormalizer {
    pub fn new(config: NormalizerConfig) -> Self {
        Self {
            config,
            ..Self::default()
        }
    }

    fn alias_grant(&mut self, raw: &str) -> String {
        let key = raw.to_ascii_lowercase();
        let next = self.grants.len() + 1;
        let idx = *self.grants.entry(key).or_insert(next);
        format!("gs-<uuid#{idx}>")
    }

    fn alias_party(&mut self, full: &str, suffix: &str) -> String {
        let prefix = full.strip_suffix(&format!("_{suffix}")).unwrap_or(full).to_owned();
        let next = self.parties.len() + 1;
        let idx = *self.parties.entry(prefix).or_insert(next);
        format!("<party#{idx}>_{suffix}")
    }

    fn alias_uuid(&mut self, raw: &str) -> String {
        let key = raw.to_ascii_lowercase();
        let next = self.uuids.len() + 1;
        let idx = *self.uuids.entry(key).or_insert(next);
        format!("<uuid#{idx}>")
    }

    fn alias_iid(&mut self, raw: &str) -> String {
        let next = self.iids.len() + 1;
        let idx = *self.iids.entry(raw.to_owned()).or_insert(next);
        format!("<iid#{idx}>")
    }

    /// Normalizes a string value (verifying JWTs, timestamps, and dynamic IDs).
    pub fn normalize_str(&mut self, text: &str, key_hint: Option<&str>) -> String {
        // Double-encoded JSON object inside a string field (e.g. Character `data` or Loadout `data`)
        if (text.starts_with('{') && text.ends_with('}'))
            && let Ok(parsed) = serde_json::from_str::<Value>(text)
        {
            let norm_val = self.normalize_value(&parsed, None);
            return service_kit::jsjson::stringify(&norm_val);
        }

        let mut out = normalize_jwts_in_str(text);

        if matches!(key_hint, Some("refresh_token" | "refreshToken" | "challengeSignature"))
            && out.len() >= 12
            && !out.starts_with("<jwt")
        {
            return "<opaque-token>".to_owned();
        }
        if key_hint == Some("nonce") && !out.is_empty() {
            return "<nonce>".to_owned();
        }
        if let Some(re) = CODE_CHALLENGE_RE.as_ref() {
            out = re.replace_all(&out, "code_challenge=<challenge>").into_owned();
        }

        if self.config.normalize_timestamps {
            if let Some(re) = ISO_DATE_RE.as_ref() {
                out = re
                    .replace_all(&out, |caps: &Captures<'_>| {
                        if caps[0].starts_with("2085-09-09") || caps[0].starts_with("2099-") {
                            caps[0].to_owned()
                        } else {
                            "<date>".to_owned()
                        }
                    })
                    .into_owned();
            }
            if let Some(re) = LOGIN_TIME_RE.as_ref() {
                out = re.replace_all(&out, "<login-time>").into_owned();
            }
            if let Some(re) = DISPLAY_DATE_RE.as_ref() {
                out = re.replace_all(&out, "<display-date>").into_owned();
            }
        }

        if self.config.normalize_dynamic_ids {
            if let Some(re) = GRANT_ID_RE.as_ref() {
                out = re
                    .replace_all(&out, |caps: &Captures<'_>| self.alias_grant(&caps[0]))
                    .into_owned();
            }
            if let Some(re) = PARTY_ID_RE.as_ref() {
                out = re
                    .replace_all(&out, |caps: &Captures<'_>| self.alias_party(&caps[0], &caps[1]))
                    .into_owned();
            }
            if let Some(re) = UUID_RE.as_ref() {
                out = re
                    .replace_all(&out, |caps: &Captures<'_>| self.alias_uuid(&caps[0]))
                    .into_owned();
            }
            if let Some(re) = INSTANCE_ID_RE.as_ref() {
                out = re
                    .replace_all(&out, |caps: &Captures<'_>| {
                        let m = &caps[0];
                        if m.bytes().any(|b| b.is_ascii_digit()) || key_hint == Some("instanceId") {
                            self.alias_iid(m)
                        } else {
                            m.to_owned()
                        }
                    })
                    .into_owned();
            }
        }

        out
    }

    /// Normalizes a JSON [`Value`] recursively.
    pub fn normalize_value(&mut self, value: &Value, key_hint: Option<&str>) -> Value {
        match value {
            Value::String(s) => Value::String(self.normalize_str(s, key_hint)),
            Value::Number(_) => {
                if matches!(key_hint, Some("uptimeSeconds" | "processId")) {
                    Value::from(0)
                } else {
                    value.clone()
                }
            }
            Value::Array(items) => {
                Value::Array(items.iter().map(|item| self.normalize_value(item, key_hint)).collect())
            }
            Value::Object(map) => {
                let mut out = Map::new();
                for (k, v) in map {
                    out.insert(k.clone(), self.normalize_value(v, Some(k.as_str())));
                }
                Value::Object(out)
            }
            other => other.clone(),
        }
    }

    /// Normalizes raw response body text (including `uptimeSeconds` / `processId` JSON numbers and dynamic tokens/IDs).
    pub fn normalize_raw_text(&mut self, text: &str) -> String {
        let mut out = text.to_owned();
        if let Some(re) = UPTIME_JSON_RE.as_ref() {
            out = re.replace_all(&out, r#""uptimeSeconds":0"#).into_owned();
        }
        if let Some(re) = PID_JSON_RE.as_ref() {
            out = re.replace_all(&out, r#""processId":0"#).into_owned();
        }
        self.normalize_str(&out, None)
    }
}

/// Inspects any JWT (`eyJ...`) in `text`: verifies that header and payload decode as valid JSON objects with an
/// `"alg"` header, and replaces valid JWTs with `<jwt:<sub>>` (from `userId` or `sub`) or `<jwt>`. Malformed tokens
/// are left untouched so structural corruption is caught.
pub fn normalize_jwts_in_str(text: &str) -> String {
    let mut current = text.to_owned();
    if current.contains("<token:")
        && let Some(re) = TOKEN_SUB_RE.as_ref()
    {
        current = re.replace_all(&current, "<jwt:$1>").into_owned();
    }
    if current.contains("<token>") {
        current = current.replace("<token>", "<jwt>");
    }
    let Some(re) = JWT_RE.as_ref() else {
        return current;
    };
    if !current.contains("eyJ") {
        return current;
    }
    re.replace_all(&current, |caps: &Captures<'_>| {
        let token = &caps[0];
        let mut parts = token.split('.');
        let (Some(h_b64), Some(p_b64), Some(sig_b64), None) = (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return token.to_owned();
        };
        if sig_b64.is_empty() {
            return token.to_owned();
        }
        let (Ok(h_bytes), Ok(p_bytes)) = (decode_b64url(h_b64), decode_b64url(p_b64)) else {
            return token.to_owned();
        };
        let (Ok(Value::Object(header)), Ok(Value::Object(payload))) = (
            serde_json::from_slice::<Value>(&h_bytes),
            serde_json::from_slice::<Value>(&p_bytes),
        ) else {
            return token.to_owned();
        };
        if header.get("alg").and_then(Value::as_str).is_none_or(str::is_empty) {
            return token.to_owned();
        }
        let subject = payload
            .get("userId")
            .and_then(Value::as_str)
            .or_else(|| payload.get("sub").and_then(Value::as_str))
            .filter(|s| !s.is_empty());
        match subject {
            Some(sub) => format!("<jwt:{sub}>"),
            None => "<jwt>".to_owned(),
        }
    })
    .into_owned()
}

fn decode_b64url(input: &str) -> Result<Vec<u8>, base64::DecodeError> {
    let trimmed = input.trim_end_matches('=');
    BASE64_URL.decode(trimmed)
}

/// Normalizes HTTP response headers, stripping volatile transport headers and canonicalizing `content-type`.
pub fn normalize_headers(
    headers: &BTreeMap<String, String>,
    normalizer: &mut IdNormalizer,
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for (k, v) in headers {
        let lower = k.to_ascii_lowercase();
        if IGNORED_RESPONSE_HEADERS.contains(&lower.as_str()) {
            continue;
        }
        let norm_val = if lower == "content-type" {
            normalize_content_type(v)
        } else {
            normalizer.normalize_str(v.trim(), Some(&lower))
        };
        out.insert(lower, norm_val);
    }
    out
}

fn normalize_content_type(raw: &str) -> String {
    let parts: Vec<String> = raw
        .split(';')
        .map(|p| p.trim().to_ascii_lowercase())
        .filter(|p| !p.is_empty())
        .collect();
    parts.join("; ")
}

/// Compares two [`ReplayHttpResponse`]s using the left and right [`IdNormalizer`] states and returns all classified
/// diffs (`StatusDiff`, `HeaderDiff`, `ValueDiff`, `TextDiff`).
pub fn compare_http_responses(
    left: &ReplayHttpResponse,
    right: &ReplayHttpResponse,
    left_norm: &mut IdNormalizer,
    right_norm: &mut IdNormalizer,
) -> Vec<DiffKind> {
    let mut diffs = Vec::new();

    if left.status != right.status {
        diffs.push(DiffKind::StatusDiff {
            left: left.status,
            right: right.status,
        });
    }

    let left_headers = normalize_headers(&left.headers, left_norm);
    let right_headers = normalize_headers(&right.headers, right_norm);
    let all_header_keys: BTreeSet<&String> = left_headers.keys().chain(right_headers.keys()).collect();
    for key in all_header_keys {
        let l_val = left_headers.get(key).cloned();
        let r_val = right_headers.get(key).cloned();
        if l_val != r_val {
            diffs.push(DiffKind::HeaderDiff {
                header: key.clone(),
                left: l_val,
                right: r_val,
            });
        }
    }

    let left_json = serde_json::from_str::<Value>(&left.body_text).ok();
    let right_json = serde_json::from_str::<Value>(&right.body_text).ok();

    match (left_json, right_json) {
        (Some(l_val), Some(r_val)) => {
            let norm_l = left_norm.normalize_value(&l_val, None);
            let norm_r = right_norm.normalize_value(&r_val, None);

            let mut value_diffs = Vec::new();
            diff_json_values("$", &norm_l, &norm_r, &mut value_diffs);
            let has_value_diffs = !value_diffs.is_empty();
            diffs.extend(value_diffs);

            if left_norm.config.check_raw_json_text && !has_value_diffs {
                // Canonicalize via jsjson::stringify on the normalized Value (which preserves object key insertion order!)
                let l_wire = service_kit::jsjson::stringify(&norm_l);
                let r_wire = service_kit::jsjson::stringify(&norm_r);
                let l_canon_orig = service_kit::jsjson::stringify(&l_val);
                let r_canon_orig = service_kit::jsjson::stringify(&r_val);
                let l_ws_diff = left.body_text != l_canon_orig;
                let r_ws_diff = right.body_text != r_canon_orig;
                if l_wire != r_wire || l_ws_diff != r_ws_diff {
                    diffs.push(DiffKind::TextDiff {
                        left: if l_ws_diff { left.body_text.clone() } else { l_wire },
                        right: if r_ws_diff { right.body_text.clone() } else { r_wire },
                    });
                }
            }
        }
        _ => {
            let norm_l = left_norm.normalize_raw_text(&left.body_text);
            let norm_r = right_norm.normalize_raw_text(&right.body_text);
            if norm_l != norm_r {
                diffs.push(DiffKind::TextDiff {
                    left: norm_l,
                    right: norm_r,
                });
            }
        }
    }

    diffs
}

fn diff_json_values(path: &str, left: &Value, right: &Value, out: &mut Vec<DiffKind>) {
    if left == right {
        return;
    }
    match (left, right) {
        (Value::Number(ln), Value::Number(rn)) => {
            if ln.as_f64() == rn.as_f64() {
                return;
            }
            out.push(DiffKind::ValueDiff {
                path: path.to_owned(),
                left: left.clone(),
                right: right.clone(),
            });
        }
        (Value::Array(la), Value::Array(ra)) => {
            let max_len = la.len().max(ra.len());
            for i in 0..max_len {
                let child_path = format!("{path}[{i}]");
                match (la.get(i), ra.get(i)) {
                    (Some(lv), Some(rv)) => diff_json_values(&child_path, lv, rv, out),
                    (Some(lv), None) => out.push(DiffKind::ValueDiff {
                        path: child_path,
                        left: lv.clone(),
                        right: Value::Null,
                    }),
                    (None, Some(rv)) => out.push(DiffKind::ValueDiff {
                        path: child_path,
                        left: Value::Null,
                        right: rv.clone(),
                    }),
                    (None, None) => {}
                }
            }
        }
        (Value::Object(lo), Value::Object(ro)) => {
            let keys: BTreeSet<&String> = lo.keys().chain(ro.keys()).collect();
            for k in keys {
                let child_path = format!("{path}.{k}");
                match (lo.get(k), ro.get(k)) {
                    (Some(lv), Some(rv)) => diff_json_values(&child_path, lv, rv, out),
                    (Some(lv), None) => out.push(DiffKind::ValueDiff {
                        path: child_path,
                        left: lv.clone(),
                        right: Value::Null,
                    }),
                    (None, Some(rv)) => out.push(DiffKind::ValueDiff {
                        path: child_path,
                        left: Value::Null,
                        right: rv.clone(),
                    }),
                    (None, None) => {}
                }
            }
        }
        _ => {
            out.push(DiffKind::ValueDiff {
                path: path.to_owned(),
                left: left.clone(),
                right: right.clone(),
            });
        }
    }
}

// ---------------------------------------------------------------------------
// 6. XMPP Chat Stanza Normalization & Comparison
// ---------------------------------------------------------------------------

static STREAM_ID_RE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r#"(<stream:stream\b[^>]*?\bid=['"])[^'"]+(['"])"#).ok());
static SRV_ID_RE: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r#"(\bid=['"])srv-[0-9a-fA-F-]+(['"])"#).ok());

/// Returns true if an XMPP server stanza is a keepalive ping or replay barrier that should be ignored during diffing.
pub fn is_filtered_chat_stanza(stanza: &str) -> bool {
    let trimmed = stanza.trim();
    if trimmed.is_empty() {
        return true;
    }
    if trimmed.contains("urn:xmpp:ping") && trimmed.contains("<ping") {
        return true;
    }
    if trimmed.contains("replay-barrier-") {
        return true;
    }
    false
}

/// Normalizes volatile XMPP attributes (`<stream:stream id="...">`, `id="srv-..."`, timestamps) in a chat stanza.
pub fn normalize_chat_stanza(stanza: &str) -> String {
    let mut out = stanza.trim().to_owned();
    if let Some(re) = STREAM_ID_RE.as_ref() {
        out = re.replace_all(&out, "${1}<stream>${2}").into_owned();
    }
    if let Some(re) = SRV_ID_RE.as_ref() {
        out = re.replace_all(&out, "${1}srv-<id>${2}").into_owned();
    }
    if let Some(re) = ISO_DATE_RE.as_ref() {
        out = re.replace_all(&out, "<date>").into_owned();
    }
    out
}

/// Compares two sequences of outbound server chat stanzas for connection `conn`.
pub fn compare_chat_stanzas(conn: u64, left: &[String], right: &[String]) -> Vec<DiffKind> {
    let norm_left: Vec<String> = left
        .iter()
        .filter(|s| !is_filtered_chat_stanza(s))
        .map(|s| normalize_chat_stanza(s))
        .collect();
    let norm_right: Vec<String> = right
        .iter()
        .filter(|s| !is_filtered_chat_stanza(s))
        .map(|s| normalize_chat_stanza(s))
        .collect();

    let mut diffs = Vec::new();
    let max_len = norm_left.len().max(norm_right.len());
    for idx in 0..max_len {
        let l = norm_left.get(idx).cloned();
        let r = norm_right.get(idx).cloned();
        if l != r {
            diffs.push(DiffKind::ChatStanzaDiff {
                conn,
                index: idx,
                left: l,
                right: r,
            });
        }
    }
    diffs
}

// ---------------------------------------------------------------------------
// 7. Replay Targets (Live HTTP, In-Process Axum Router, or Recorded Corpus)
// ---------------------------------------------------------------------------

/// Endpoint configuration for one side (`left` or `right`) of a shadow replay run.
#[derive(Clone, Default)]
pub enum ReplayEndpoint {
    /// Uses the responses recorded in the capture `.jsonl` corpus itself as the baseline.
    #[default]
    Recorded,
    /// Sends live HTTP requests to `base_url` (and optional `director_url` for `route == "director"`).
    Http {
        base_url: String,
        director_url: Option<String>,
        chat_addr: Option<String>,
    },
    /// Dispatches directly into in-process `axum::Router` instances without requiring external processes.
    Router {
        backend: axum::Router,
        director: Option<axum::Router>,
    },
}

impl std::fmt::Debug for ReplayEndpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Recorded => write!(f, "Recorded"),
            Self::Http {
                base_url,
                director_url,
                chat_addr,
            } => f
                .debug_struct("Http")
                .field("base_url", base_url)
                .field("director_url", director_url)
                .field("chat_addr", chat_addr)
                .finish(),
            Self::Router { director, .. } => f
                .debug_struct("Router")
                .field("has_director", &director.is_some())
                .finish(),
        }
    }
}

/// Complete target configuration for one side (`left` or `right`) of a shadow replay.
#[derive(Debug, Clone, Default)]
pub struct ReplayTarget {
    pub name: String,
    pub endpoint: ReplayEndpoint,
    pub auth: ReplayTargetAuth,
    pub pin_clock: bool,
}

impl ReplayTarget {
    pub fn recorded() -> Self {
        Self {
            name: "recorded".to_owned(),
            endpoint: ReplayEndpoint::Recorded,
            auth: ReplayTargetAuth::default(),
            pin_clock: false,
        }
    }

    pub fn http(name: impl Into<String>, base_url: impl Into<String>, auth: ReplayTargetAuth) -> Self {
        Self {
            name: name.into(),
            endpoint: ReplayEndpoint::Http {
                base_url: base_url.into(),
                director_url: None,
                chat_addr: None,
            },
            auth,
            pin_clock: true,
        }
    }

    pub fn router(
        name: impl Into<String>,
        backend: axum::Router,
        director: Option<axum::Router>,
        auth: ReplayTargetAuth,
    ) -> Self {
        Self {
            name: name.into(),
            endpoint: ReplayEndpoint::Router { backend, director },
            auth,
            pin_clock: true,
        }
    }
}

/// Sends a [`PreparedHttpRequest`] to a live HTTP server (`http://host:port`) over TCP.
pub async fn send_http_request(base_url: &str, req: &PreparedHttpRequest) -> Result<ReplayHttpResponse, ReplayError> {
    let trimmed = base_url
        .trim()
        .strip_prefix("http://")
        .unwrap_or_else(|| base_url.trim());
    let host_port = trimmed.split('/').next().unwrap_or(trimmed);
    let addr = if host_port.contains(':') {
        host_port.to_owned()
    } else {
        format!("{host_port}:80")
    };

    let mut stream = TcpStream::connect(&addr).await.map_err(|e| ReplayError::Transport {
        target: format!("{base_url}{}", req.target),
        message: e.to_string(),
    })?;

    let path = if req.target.starts_with('/') {
        req.target.as_str()
    } else {
        "/"
    };
    let mut raw_req = format!(
        "{} {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Length: {}\r\n",
        req.method,
        path,
        host_port,
        req.body.len()
    );
    for (k, v) in &req.headers {
        if !HOP_BY_HOP_REQUEST_HEADERS.contains(&k.to_ascii_lowercase().as_str()) {
            raw_req.push_str(&format!("{k}: {v}\r\n"));
        }
    }
    raw_req.push_str("\r\n");

    stream
        .write_all(raw_req.as_bytes())
        .await
        .map_err(|e| ReplayError::Transport {
            target: format!("{base_url}{}", req.target),
            message: e.to_string(),
        })?;
    if !req.body.is_empty() {
        stream.write_all(&req.body).await.map_err(|e| ReplayError::Transport {
            target: format!("{base_url}{}", req.target),
            message: e.to_string(),
        })?;
    }

    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.map_err(|e| ReplayError::Transport {
        target: format!("{base_url}{}", req.target),
        message: e.to_string(),
    })?;

    parse_raw_http_response(&buf, &req.target)
}

fn parse_raw_http_response(buf: &[u8], target: &str) -> Result<ReplayHttpResponse, ReplayError> {
    let sep = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| ReplayError::Transport {
            target: target.to_owned(),
            message: "malformed HTTP response (missing header terminator)".to_owned(),
        })?;
    let head = String::from_utf8_lossy(&buf[..sep]);
    let mut body_bytes = &buf[sep + 4..];

    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| ReplayError::Transport {
            target: target.to_owned(),
            message: format!("invalid HTTP status line: {status_line:?}"),
        })?;

    let mut headers = BTreeMap::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim().to_ascii_lowercase();
            let val = v.trim().to_owned();
            headers
                .entry(key)
                .and_modify(|existing: &mut String| {
                    existing.push_str(", ");
                    existing.push_str(&val);
                })
                .or_insert(val);
        }
    }

    let decoded_chunked;
    if headers
        .get("transfer-encoding")
        .is_some_and(|te| te.to_ascii_lowercase().contains("chunked"))
    {
        decoded_chunked = decode_chunked_body(body_bytes);
        body_bytes = &decoded_chunked;
    }

    let body_text = String::from_utf8_lossy(body_bytes).into_owned();
    Ok(ReplayHttpResponse {
        status,
        headers,
        body_text,
    })
}

fn decode_chunked_body(mut input: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(pos) = input.windows(2).position(|w| w == b"\r\n") {
        let size_str = String::from_utf8_lossy(&input[..pos]);
        let size_hex = size_str.split(';').next().unwrap_or_default().trim();
        let Ok(size) = usize::from_str_radix(size_hex, 16) else {
            break;
        };
        input = &input[pos + 2..];
        if size == 0 || input.len() < size {
            break;
        }
        out.extend_from_slice(&input[..size]);
        input = &input[size..];
        if input.starts_with(b"\r\n") {
            input = &input[2..];
        }
    }
    out
}

/// Constructs a JSON `axum::response::Response` with the given HTTP status code.
pub fn json_status_response(status: u16, value: Value) -> axum::response::Response {
    use axum::response::IntoResponse as _;
    let code = axum::http::StatusCode::from_u16(status).unwrap_or(axum::http::StatusCode::OK);
    (code, service_kit::json::Json(value)).into_response()
}

/// Spawns an `axum::Router` on an ephemeral `127.0.0.1:0` loopback listener and returns `(base_url, stop_handle)`.
pub async fn serve_ephemeral(
    router: axum::Router,
) -> Result<(String, oneshot::Sender<()>, tokio::task::JoinHandle<()>), ReplayError> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|e| ReplayError::Transport {
            target: "127.0.0.1:0".to_owned(),
            message: e.to_string(),
        })?;
    let addr: SocketAddr = listener.local_addr().map_err(|e| ReplayError::Transport {
        target: "127.0.0.1:0".to_owned(),
        message: e.to_string(),
    })?;
    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    let task = tokio::spawn(async move {
        let stop = async {
            let _ = stop_rx.await;
        };
        let _ = service_kit::serve::serve(listener, router, stop, Duration::from_millis(50)).await;
    });
    Ok((format!("http://{addr}"), stop_tx, task))
}

/// Dispatches a [`PreparedHttpRequest`] into an `axum::Router` over an ephemeral loopback listener.
pub async fn dispatch_router_request(
    router: &axum::Router,
    req: &PreparedHttpRequest,
) -> Result<ReplayHttpResponse, ReplayError> {
    let (base_url, stop_tx, task) = serve_ephemeral(router.clone()).await?;
    let res = send_http_request(&base_url, req).await;
    let _ = stop_tx.send(());
    let _ = task.await;
    res
}

// ---------------------------------------------------------------------------
// 8. Replay Runner & Report
// ---------------------------------------------------------------------------

/// Per-request or per-chat-connection diff entry in a [`ReplayReport`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplayEntryResult {
    pub index: usize,
    pub kind: String,
    pub conn: u64,
    pub seq: u64,
    pub method: String,
    pub target: String,
    pub route: String,
    pub inexact: bool,
    pub left_status: Option<u16>,
    pub right_status: Option<u16>,
    pub diffs: Vec<DiffKind>,
}

/// Summary report produced by a shadow replay run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplayReport {
    pub total_records: usize,
    pub http_replayed: usize,
    pub chat_connections_replayed: usize,
    pub skipped_records: usize,
    pub exact_matched: usize,
    pub inexact_matched: usize,
    pub inexact_diffed: usize,
    pub unexplained_diffed: usize,
    pub status_diffs: usize,
    pub header_diffs: usize,
    pub value_diffs: usize,
    pub text_diffs: usize,
    pub chat_stanza_diffs: usize,
    pub duration_ms: u64,
    pub entries: Vec<ReplayEntryResult>,
}

impl ReplayReport {
    /// True when every exact replay entry matched with zero unexplained diffs.
    pub fn passed(&self) -> bool {
        self.unexplained_diffed == 0
    }
}

/// Configuration for a shadow replay run.
#[derive(Debug, Clone, Default)]
pub struct ReplayConfig {
    pub left: ReplayTarget,
    pub right: ReplayTarget,
    pub normalizer: NormalizerConfig,
    /// Optional substring filter on `target` path or `route`.
    pub filter: Option<String>,
}

async fn execute_target_http(
    target: &ReplayTarget,
    record: &HttpRecord,
    id_map: &mut HashMap<String, String>,
) -> Result<(ReplayHttpResponse, bool), ReplayError> {
    match &target.endpoint {
        ReplayEndpoint::Recorded => Ok((
            ReplayHttpResponse::from_recorded(record),
            record.aborted || record.req_body.is_lossy() || record.res_body.is_lossy(),
        )),
        ReplayEndpoint::Http {
            base_url, director_url, ..
        } => {
            let prep = prepare_http_request(record, &target.auth, id_map, target.pin_clock);
            let url = if record.route == "director" {
                director_url.as_deref().unwrap_or(base_url.as_str())
            } else {
                base_url.as_str()
            };
            let resp = send_http_request(url, &prep).await?;
            if let (CaptureBody::Json(rec_json), Ok(act_json)) =
                (&record.res_body, serde_json::from_str::<Value>(&resp.body_text))
            {
                learn_ids(rec_json, &act_json, id_map);
            }
            Ok((resp, prep.inexact))
        }
        ReplayEndpoint::Router { backend, director } => {
            let prep = prepare_http_request(record, &target.auth, id_map, target.pin_clock);
            let router = if record.route == "director" {
                director.as_ref().unwrap_or(backend)
            } else {
                backend
            };
            let resp = dispatch_router_request(router, &prep).await?;
            if let (CaptureBody::Json(rec_json), Ok(act_json)) =
                (&record.res_body, serde_json::from_str::<Value>(&resp.body_text))
            {
                learn_ids(rec_json, &act_json, id_map);
            }
            Ok((resp, prep.inexact))
        }
    }
}

/// Runs the shadow replay harness over `records` with the given [`ReplayConfig`].
pub async fn run_replay(records: &[CaptureRecord], config: &ReplayConfig) -> Result<ReplayReport, ReplayError> {
    let started = Instant::now();
    let mut left_id_map: HashMap<String, String> = HashMap::new();
    let mut right_id_map: HashMap<String, String> = HashMap::new();
    let mut left_norm = IdNormalizer::new(config.normalizer.clone());
    let mut right_norm = IdNormalizer::new(config.normalizer.clone());

    let mut entries = Vec::new();
    let mut http_replayed = 0usize;
    let mut skipped_records = 0usize;
    let mut exact_matched = 0usize;
    let mut inexact_matched = 0usize;
    let mut inexact_diffed = 0usize;
    let mut unexplained_diffed = 0usize;
    let mut status_diffs = 0usize;
    let mut header_diffs = 0usize;
    let mut value_diffs = 0usize;
    let mut text_diffs = 0usize;
    let mut chat_stanza_diffs = 0usize;

    // Group chat stanzas by connection id while replaying HTTP records in timestamp order.
    let mut chat_by_conn: BTreeMap<u64, Vec<&ChatRecord>> = BTreeMap::new();

    for (index, record) in records.iter().enumerate() {
        match record {
            CaptureRecord::Http(http) => {
                if let Some(filter) = &config.filter
                    && !http.target.contains(filter.as_str())
                    && !http.route.contains(filter.as_str())
                {
                    skipped_records += 1;
                    continue;
                }
                if http.aborted || matches!(http.route.as_str(), "content" | "network") {
                    skipped_records += 1;
                    continue;
                }
                if http.route == "director" && !target_supports_director(&config.right) {
                    skipped_records += 1;
                    continue;
                }

                http_replayed += 1;
                let (left_resp, left_inexact) = execute_target_http(&config.left, http, &mut left_id_map).await?;
                let (right_resp, right_inexact) = execute_target_http(&config.right, http, &mut right_id_map).await?;
                let inexact = left_inexact || right_inexact;

                // When left is a live server and right is a live server, also cross-learn IDs from left -> right
                // when the corpus had no body or when left generated fresh IDs.
                if !matches!(config.left.endpoint, ReplayEndpoint::Recorded)
                    && matches!(http.res_body, CaptureBody::Null)
                    && let (Ok(l_json), Ok(r_json)) = (
                        serde_json::from_str::<Value>(&left_resp.body_text),
                        serde_json::from_str::<Value>(&right_resp.body_text),
                    )
                {
                    learn_ids(&l_json, &r_json, &mut right_id_map);
                }

                let diffs = compare_http_responses(&left_resp, &right_resp, &mut left_norm, &mut right_norm);
                for d in &diffs {
                    match d {
                        DiffKind::StatusDiff { .. } => status_diffs += 1,
                        DiffKind::HeaderDiff { .. } => header_diffs += 1,
                        DiffKind::ValueDiff { .. } => value_diffs += 1,
                        DiffKind::TextDiff { .. } => text_diffs += 1,
                        DiffKind::ChatStanzaDiff { .. } => chat_stanza_diffs += 1,
                    }
                }

                if diffs.is_empty() {
                    if inexact {
                        inexact_matched += 1;
                    } else {
                        exact_matched += 1;
                    }
                } else if inexact {
                    inexact_diffed += 1;
                } else {
                    unexplained_diffed += 1;
                }

                entries.push(ReplayEntryResult {
                    index,
                    kind: "http".to_owned(),
                    conn: http.conn,
                    seq: http.seq,
                    method: http.method.clone(),
                    target: http.target.clone(),
                    route: http.route.clone(),
                    inexact,
                    left_status: Some(left_resp.status),
                    right_status: Some(right_resp.status),
                    diffs,
                });
            }
            CaptureRecord::Chat(chat) => {
                if let Some(filter) = &config.filter
                    && !filter.contains("chat")
                {
                    skipped_records += 1;
                    continue;
                }
                chat_by_conn.entry(chat.conn).or_default().push(chat);
            }
        }
    }

    let mut chat_connections_replayed = 0usize;
    for (conn, frames) in chat_by_conn {
        chat_connections_replayed += 1;
        let (left_stanzas, left_inexact) = replay_chat_connection(&config.left, &frames, &left_id_map).await?;
        let (right_stanzas, right_inexact) = replay_chat_connection(&config.right, &frames, &right_id_map).await?;
        let inexact = left_inexact || right_inexact;
        let diffs = compare_chat_stanzas(conn, &left_stanzas, &right_stanzas);
        chat_stanza_diffs += diffs.len();

        if diffs.is_empty() {
            if inexact {
                inexact_matched += 1;
            } else {
                exact_matched += 1;
            }
        } else if inexact {
            inexact_diffed += 1;
        } else {
            unexplained_diffed += 1;
        }

        entries.push(ReplayEntryResult {
            index: entries.len(),
            kind: "chat".to_owned(),
            conn,
            seq: 0,
            method: "XMPP".to_owned(),
            target: format!("chat:conn:{conn}"),
            route: "chat".to_owned(),
            inexact,
            left_status: None,
            right_status: None,
            diffs,
        });
    }

    Ok(ReplayReport {
        total_records: records.len(),
        http_replayed,
        chat_connections_replayed,
        skipped_records,
        exact_matched,
        inexact_matched,
        inexact_diffed,
        unexplained_diffed,
        status_diffs,
        header_diffs,
        value_diffs,
        text_diffs,
        chat_stanza_diffs,
        duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        entries,
    })
}

fn target_supports_director(target: &ReplayTarget) -> bool {
    match &target.endpoint {
        ReplayEndpoint::Recorded => true,
        ReplayEndpoint::Http { director_url, .. } => director_url.is_some(),
        ReplayEndpoint::Router { director, .. } => director.is_some(),
    }
}

async fn replay_chat_connection(
    target: &ReplayTarget,
    frames: &[&ChatRecord],
    id_map: &HashMap<String, String>,
) -> Result<(Vec<String>, bool), ReplayError> {
    let mut inexact = frames.iter().any(|f| f.omitted.is_some() || f.compressed.is_some());
    let chat_addr = match &target.endpoint {
        ReplayEndpoint::Http {
            chat_addr: Some(addr), ..
        } => Some(addr.as_str()),
        _ => None,
    };

    if let Some(addr) = chat_addr {
        let mut stream = TcpStream::connect(addr).await.map_err(|e| ReplayError::Transport {
            target: format!("chat://{addr}"),
            message: e.to_string(),
        })?;
        let mut out_stanzas = Vec::new();
        for frame in frames {
            if frame.dir.as_deref() == Some("in")
                && let Some(text) = &frame.text
            {
                let now_ms = service_kit::js::date_parse(&frame.t).unwrap_or(1_793_016_000_000);
                let mapped = apply_ids_str(text, id_map);
                let sub = substitute_str(&mapped, &target.auth, now_ms);
                inexact |= sub.inexact;
                stream
                    .write_all(sub.value.as_bytes())
                    .await
                    .map_err(|e| ReplayError::Transport {
                        target: format!("chat://{addr}"),
                        message: e.to_string(),
                    })?;
                let mut buf = vec![0u8; 8192];
                if let Ok(Ok(n)) = tokio::time::timeout(Duration::from_millis(50), stream.read(&mut buf)).await
                    && n > 0
                {
                    out_stanzas.push(String::from_utf8_lossy(&buf[..n]).into_owned());
                }
            }
        }
        return Ok((out_stanzas, inexact));
    }

    // Recorded or in-process chat transcript extraction (verifying placeholder substitution on inbound frames
    // and normalizing outbound frames).
    let mut out_stanzas = Vec::new();
    for frame in frames {
        let now_ms = service_kit::js::date_parse(&frame.t).unwrap_or(1_793_016_000_000);
        if frame.dir.as_deref() == Some("in")
            && let Some(text) = &frame.text
        {
            let mapped = apply_ids_str(text, id_map);
            let sub = substitute_str(&mapped, &target.auth, now_ms);
            inexact |= sub.inexact;
        } else if frame.dir.as_deref() == Some("out")
            && let Some(text) = &frame.text
        {
            let mapped = apply_ids_str(text, id_map);
            out_stanzas.push(mapped);
        }
    }
    Ok((out_stanzas, inexact))
}
