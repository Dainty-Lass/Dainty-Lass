//! Read-only MongoDB wire-protocol (`OP_MSG`, opcode 2013) + BSON client and an in-process wire-protocol
//! test server. Speaks only `hello`, optional SCRAM-SHA-256 `saslStart`/`saslContinue`, `find`, and `getMore`,
//! so the migration tool cannot mutate the source MongoDB cluster by construction.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::{SecondsFormat, TimeZone, Utc};
use hmac::{Hmac, KeyInit, Mac};
use percent_encoding::percent_decode_str;
use serde_json::{Map, Number, Value};
use sha2::{Digest, Sha256};

const OP_MSG: i32 = 2013;
const DEFAULT_BATCH_SIZE: i32 = 500;
const IO_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, thiserror::Error)]
pub enum MongoError {
    #[error("invalid MongoDB URI: {0}")]
    InvalidUri(String),
    #[error("MongoDB I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("BSON decode error: {0}")]
    Bson(String),
    #[error("MongoDB command error: {0}")]
    Command(String),
    #[error("MongoDB authentication failed: {0}")]
    Auth(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MongoUri {
    pub hosts: Vec<(String, u16)>,
    pub database: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub auth_source: String,
}

impl MongoUri {
    pub fn parse(raw: &str) -> Result<Self, MongoError> {
        let rest = raw
            .strip_prefix("mongodb://")
            .ok_or_else(|| MongoError::InvalidUri("URI must start with mongodb://".into()))?;

        let (before_query, query) = match rest.split_once('?') {
            Some((b, q)) => (b, Some(q)),
            None => (rest, None),
        };

        let (authority, path) = match before_query.split_once('/') {
            Some((a, p)) => (a, p),
            None => (before_query, ""),
        };

        let (creds, host_list) = match authority.rsplit_once('@') {
            Some((c, h)) => (Some(c), h),
            None => (None, authority),
        };

        let (username, password) = if let Some(creds) = creds {
            let (u, p) = creds
                .split_once(':')
                .ok_or_else(|| MongoError::InvalidUri("credentials must be username:password".into()))?;
            let u_dec = percent_decode_str(u)
                .decode_utf8()
                .map_err(|e| MongoError::InvalidUri(format!("invalid username encoding: {e}")))?
                .into_owned();
            let p_dec = percent_decode_str(p)
                .decode_utf8()
                .map_err(|e| MongoError::InvalidUri(format!("invalid password encoding: {e}")))?
                .into_owned();
            (Some(u_dec), Some(p_dec))
        } else {
            (None, None)
        };

        let mut hosts = Vec::new();
        for entry in host_list.split(',') {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }
            let (host, port) = if let Some((h, p)) = entry.rsplit_once(':') {
                let port: u16 = p
                    .parse()
                    .map_err(|_| MongoError::InvalidUri(format!("invalid port in {entry:?}")))?;
                (h.to_owned(), port)
            } else {
                (entry.to_owned(), 27017)
            };
            if host.is_empty() {
                return Err(MongoError::InvalidUri("empty host in MongoDB URI".into()));
            }
            hosts.push((host, port));
        }

        if hosts.is_empty() {
            return Err(MongoError::InvalidUri("no hosts in MongoDB URI".into()));
        }

        let db_raw = path.trim_matches('/');
        let database = if db_raw.is_empty() {
            "dauntless".to_owned()
        } else {
            percent_decode_str(db_raw)
                .decode_utf8()
                .map_err(|e| MongoError::InvalidUri(format!("invalid database name: {e}")))?
                .into_owned()
        };

        let mut auth_source = "admin".to_owned();
        if let Some(q) = query {
            for pair in q.split('&') {
                if let Some((k, v)) = pair.split_once('=')
                    && k.eq_ignore_ascii_case("authSource")
                    && !v.is_empty()
                {
                    auth_source = percent_decode_str(v)
                        .decode_utf8()
                        .map_err(|e| MongoError::InvalidUri(format!("invalid authSource: {e}")))?
                        .into_owned();
                }
            }
        }

        Ok(Self {
            hosts,
            database,
            username,
            password,
            auth_source,
        })
    }
}

/// Low-level BSON document builder so command fields (`getMore`, `batchSize`, etc.) use exact BSON wire types.
#[derive(Debug, Clone, Default)]
pub struct BsonBuf {
    elements: Vec<u8>,
}

impl BsonBuf {
    pub fn new() -> Self {
        Self::default()
    }

    fn write_cstring(&mut self, s: &str) {
        self.elements.extend_from_slice(s.as_bytes());
        self.elements.push(0);
    }

    pub fn put_i32(&mut self, key: &str, val: i32) -> &mut Self {
        self.elements.push(0x10);
        self.write_cstring(key);
        self.elements.extend_from_slice(&val.to_le_bytes());
        self
    }

    pub fn put_i64(&mut self, key: &str, val: i64) -> &mut Self {
        self.elements.push(0x12);
        self.write_cstring(key);
        self.elements.extend_from_slice(&val.to_le_bytes());
        self
    }

    pub fn put_f64(&mut self, key: &str, val: f64) -> &mut Self {
        self.elements.push(0x01);
        self.write_cstring(key);
        self.elements.extend_from_slice(&val.to_le_bytes());
        self
    }

    pub fn put_str(&mut self, key: &str, val: &str) -> &mut Self {
        self.elements.push(0x02);
        self.write_cstring(key);
        let len = (val.len() as i32) + 1;
        self.elements.extend_from_slice(&len.to_le_bytes());
        self.elements.extend_from_slice(val.as_bytes());
        self.elements.push(0);
        self
    }

    pub fn put_bool(&mut self, key: &str, val: bool) -> &mut Self {
        self.elements.push(0x08);
        self.write_cstring(key);
        self.elements.push(u8::from(val));
        self
    }

    pub fn put_null(&mut self, key: &str) -> &mut Self {
        self.elements.push(0x0A);
        self.write_cstring(key);
        self
    }

    pub fn put_binary(&mut self, key: &str, bytes: &[u8]) -> &mut Self {
        self.elements.push(0x05);
        self.write_cstring(key);
        let len = bytes.len() as i32;
        self.elements.extend_from_slice(&len.to_le_bytes());
        self.elements.push(0x00);
        self.elements.extend_from_slice(bytes);
        self
    }

    pub fn put_doc(&mut self, key: &str, doc: &BsonBuf) -> &mut Self {
        self.elements.push(0x03);
        self.write_cstring(key);
        let encoded = doc.finish();
        self.elements.extend_from_slice(&encoded);
        self
    }

    pub fn put_array_docs(&mut self, key: &str, docs: &[BsonBuf]) -> &mut Self {
        self.elements.push(0x04);
        self.write_cstring(key);
        let mut arr = BsonBuf::new();
        for (idx, doc) in docs.iter().enumerate() {
            arr.put_doc(&idx.to_string(), doc);
        }
        self.elements.extend_from_slice(&arr.finish());
        self
    }

    pub fn put_json_value(&mut self, key: &str, val: &Value) -> &mut Self {
        match val {
            Value::Null => {
                self.put_null(key);
            }
            Value::Bool(b) => {
                self.put_bool(key, *b);
            }
            Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    if let Ok(i32_val) = i32::try_from(i) {
                        self.put_i32(key, i32_val);
                    } else {
                        self.put_i64(key, i);
                    }
                } else if let Some(f) = n.as_f64() {
                    self.put_f64(key, f);
                }
            }
            Value::String(s) => {
                self.put_str(key, s);
            }
            Value::Array(items) => {
                self.elements.push(0x04);
                self.write_cstring(key);
                let mut arr = BsonBuf::new();
                for (i, item) in items.iter().enumerate() {
                    arr.put_json_value(&i.to_string(), item);
                }
                self.elements.extend_from_slice(&arr.finish());
            }
            Value::Object(map) => {
                let sub = Self::from_json_map(map);
                self.put_doc(key, &sub);
            }
        }
        self
    }

    pub fn from_json_map(map: &Map<String, Value>) -> Self {
        let mut buf = Self::new();
        for (k, v) in map {
            buf.put_json_value(k, v);
        }
        buf
    }

    pub fn finish(&self) -> Vec<u8> {
        let total = (self.elements.len() as i32) + 5;
        let mut out = Vec::with_capacity(total as usize);
        out.extend_from_slice(&total.to_le_bytes());
        out.extend_from_slice(&self.elements);
        out.push(0);
        out
    }
}

/// Decodes a BSON document slice into a `serde_json::Map<String, Value>`.
pub fn decode_bson_doc(bytes: &[u8]) -> Result<Map<String, Value>, MongoError> {
    if bytes.len() < 5 {
        return Err(MongoError::Bson(format!(
            "BSON document too short: {} bytes",
            bytes.len()
        )));
    }
    let total_len = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
    if total_len > bytes.len() || total_len < 5 {
        return Err(MongoError::Bson(format!(
            "invalid BSON document length {total_len} (slice len {})",
            bytes.len()
        )));
    }
    if bytes[total_len - 1] != 0 {
        return Err(MongoError::Bson("BSON document missing trailing NUL".into()));
    }

    let mut pos = 4;
    let end = total_len - 1;
    let mut map = Map::new();

    while pos < end {
        let tag = bytes[pos];
        pos += 1;
        let key = read_cstring(bytes, &mut pos, end)?;
        let value = decode_bson_element(tag, bytes, &mut pos, end)?;
        map.insert(key, value);
    }

    Ok(map)
}

fn read_cstring(bytes: &[u8], pos: &mut usize, end: usize) -> Result<String, MongoError> {
    let start = *pos;
    while *pos < end && bytes[*pos] != 0 {
        *pos += 1;
    }
    if *pos >= end {
        return Err(MongoError::Bson("unterminated C string in BSON".into()));
    }
    let s = std::str::from_utf8(&bytes[start..*pos])
        .map_err(|e| MongoError::Bson(format!("invalid UTF-8 in BSON key: {e}")))?
        .to_owned();
    *pos += 1;
    Ok(s)
}

fn decode_bson_element(tag: u8, bytes: &[u8], pos: &mut usize, end: usize) -> Result<Value, MongoError> {
    match tag {
        0x01 => {
            if *pos + 8 > end {
                return Err(MongoError::Bson("truncated BSON double".into()));
            }
            let mut arr = [0u8; 8];
            arr.copy_from_slice(&bytes[*pos..*pos + 8]);
            *pos += 8;
            let f = f64::from_le_bytes(arr);
            Ok(Number::from_f64(f).map_or(Value::Null, Value::Number))
        }
        0x02 | 0x0D | 0x0E => {
            if *pos + 4 > end {
                return Err(MongoError::Bson("truncated BSON string length".into()));
            }
            let len = i32::from_le_bytes([bytes[*pos], bytes[*pos + 1], bytes[*pos + 2], bytes[*pos + 3]]) as usize;
            *pos += 4;
            if len == 0 || *pos + len > end {
                return Err(MongoError::Bson(format!("invalid BSON string length {len}")));
            }
            if bytes[*pos + len - 1] != 0 {
                return Err(MongoError::Bson("BSON string missing trailing NUL".into()));
            }
            let s = std::str::from_utf8(&bytes[*pos..*pos + len - 1])
                .map_err(|e| MongoError::Bson(format!("invalid UTF-8 in BSON string: {e}")))?
                .to_owned();
            *pos += len;
            Ok(Value::String(s))
        }
        0x03 => {
            if *pos + 4 > end {
                return Err(MongoError::Bson("truncated BSON subdocument".into()));
            }
            let sub_len = i32::from_le_bytes([bytes[*pos], bytes[*pos + 1], bytes[*pos + 2], bytes[*pos + 3]]) as usize;
            if sub_len < 5 || *pos + sub_len > end {
                return Err(MongoError::Bson(format!("invalid BSON subdocument length {sub_len}")));
            }
            let sub = decode_bson_doc(&bytes[*pos..*pos + sub_len])?;
            *pos += sub_len;
            Ok(Value::Object(sub))
        }
        0x04 => {
            if *pos + 4 > end {
                return Err(MongoError::Bson("truncated BSON array".into()));
            }
            let sub_len = i32::from_le_bytes([bytes[*pos], bytes[*pos + 1], bytes[*pos + 2], bytes[*pos + 3]]) as usize;
            if sub_len < 5 || *pos + sub_len > end {
                return Err(MongoError::Bson(format!("invalid BSON array length {sub_len}")));
            }
            let sub = decode_bson_doc(&bytes[*pos..*pos + sub_len])?;
            *pos += sub_len;
            let items: Vec<Value> = sub.into_iter().map(|(_, v)| v).collect();
            Ok(Value::Array(items))
        }
        0x05 => {
            if *pos + 5 > end {
                return Err(MongoError::Bson("truncated BSON binary".into()));
            }
            let len = i32::from_le_bytes([bytes[*pos], bytes[*pos + 1], bytes[*pos + 2], bytes[*pos + 3]]) as usize;
            *pos += 4;
            let _subtype = bytes[*pos];
            *pos += 1;
            if *pos + len > end {
                return Err(MongoError::Bson("truncated BSON binary payload".into()));
            }
            let encoded = BASE64.encode(&bytes[*pos..*pos + len]);
            *pos += len;
            Ok(Value::String(encoded))
        }
        0x06 | 0x0A | 0x7F | 0xFF => Ok(Value::Null),
        0x07 => {
            if *pos + 12 > end {
                return Err(MongoError::Bson("truncated BSON ObjectId".into()));
            }
            let mut hex = String::with_capacity(24);
            for b in &bytes[*pos..*pos + 12] {
                use std::fmt::Write as _;
                let _ = write!(&mut hex, "{b:02x}");
            }
            *pos += 12;
            Ok(Value::String(hex))
        }
        0x08 => {
            if *pos + 1 > end {
                return Err(MongoError::Bson("truncated BSON bool".into()));
            }
            let b = bytes[*pos] != 0;
            *pos += 1;
            Ok(Value::Bool(b))
        }
        0x09 => {
            if *pos + 8 > end {
                return Err(MongoError::Bson("truncated BSON datetime".into()));
            }
            let mut arr = [0u8; 8];
            arr.copy_from_slice(&bytes[*pos..*pos + 8]);
            *pos += 8;
            let ms = i64::from_le_bytes(arr);
            let dt = Utc
                .timestamp_millis_opt(ms)
                .single()
                .unwrap_or(chrono::DateTime::<Utc>::UNIX_EPOCH);
            Ok(Value::String(dt.to_rfc3339_opts(SecondsFormat::Millis, true)))
        }
        0x0B => {
            let pattern = read_cstring(bytes, pos, end)?;
            let options = read_cstring(bytes, pos, end)?;
            Ok(Value::String(format!("/{pattern}/{options}")))
        }
        0x10 => {
            if *pos + 4 > end {
                return Err(MongoError::Bson("truncated BSON int32".into()));
            }
            let v = i32::from_le_bytes([bytes[*pos], bytes[*pos + 1], bytes[*pos + 2], bytes[*pos + 3]]);
            *pos += 4;
            Ok(Value::Number(Number::from(v)))
        }
        0x11 | 0x12 => {
            if *pos + 8 > end {
                return Err(MongoError::Bson("truncated BSON int64".into()));
            }
            let mut arr = [0u8; 8];
            arr.copy_from_slice(&bytes[*pos..*pos + 8]);
            *pos += 8;
            let v = i64::from_le_bytes(arr);
            Ok(Value::Number(Number::from(v)))
        }
        0x13 => {
            if *pos + 16 > end {
                return Err(MongoError::Bson("truncated BSON decimal128".into()));
            }
            let mut hex = String::with_capacity(32);
            for b in &bytes[*pos..*pos + 16] {
                use std::fmt::Write as _;
                let _ = write!(&mut hex, "{b:02x}");
            }
            *pos += 16;
            Ok(Value::String(hex))
        }
        other => Err(MongoError::Bson(format!("unsupported BSON element tag 0x{other:02x}"))),
    }
}

/// Read-only MongoDB client over `OP_MSG`.
pub struct MongoClient {
    stream: TcpStream,
    database: String,
    next_request_id: i32,
}

impl MongoClient {
    pub fn connect(raw_uri: &str) -> Result<Self, MongoError> {
        let uri = MongoUri::parse(raw_uri)?;
        let mut last_err = None;

        for (host, port) in &uri.hosts {
            match Self::connect_host(host, *port, &uri) {
                Ok(client) => return Ok(client),
                Err(err) => last_err = Some(err),
            }
        }

        Err(last_err.unwrap_or_else(|| MongoError::InvalidUri("no reachable MongoDB host".into())))
    }

    fn connect_host(host: &str, port: u16, uri: &MongoUri) -> Result<Self, MongoError> {
        let addr_str = format!("{host}:{port}");
        let addrs: Vec<SocketAddr> = addr_str.to_socket_addrs()?.collect();
        let addr = addrs
            .first()
            .copied()
            .ok_or_else(|| MongoError::InvalidUri(format!("could not resolve {addr_str}")))?;
        let stream = TcpStream::connect_timeout(&addr, IO_TIMEOUT)?;
        stream.set_read_timeout(Some(IO_TIMEOUT))?;
        stream.set_write_timeout(Some(IO_TIMEOUT))?;

        let mut client = Self {
            stream,
            database: uri.database.clone(),
            next_request_id: 1,
        };

        let mut hello = BsonBuf::new();
        hello.put_i32("hello", 1).put_str("$db", "admin");
        let reply = client.command(&hello)?;
        Self::check_ok(&reply, "hello")?;

        if let (Some(username), Some(password)) = (&uri.username, &uri.password) {
            client.authenticate_scram_sha256(username, password, &uri.auth_source)?;
        }

        Ok(client)
    }

    pub fn database(&self) -> &str {
        &self.database
    }

    fn command(&mut self, body_doc: &BsonBuf) -> Result<Map<String, Value>, MongoError> {
        let req_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1);

        let body_bytes = body_doc.finish();
        // OP_MSG header (16 bytes) + flagBits (4 bytes = 0) + section kind (1 byte = 0) + BSON doc
        let msg_len = (16 + 4 + 1 + body_bytes.len()) as i32;
        let mut frame = Vec::with_capacity(msg_len as usize);
        frame.extend_from_slice(&msg_len.to_le_bytes());
        frame.extend_from_slice(&req_id.to_le_bytes());
        frame.extend_from_slice(&0i32.to_le_bytes());
        frame.extend_from_slice(&OP_MSG.to_le_bytes());
        frame.extend_from_slice(&0u32.to_le_bytes());
        frame.push(0u8);
        frame.extend_from_slice(&body_bytes);

        self.stream.write_all(&frame)?;
        self.stream.flush()?;

        let mut hdr = [0u8; 16];
        self.stream.read_exact(&mut hdr)?;
        let resp_len = i32::from_le_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]) as usize;
        let opcode = i32::from_le_bytes([hdr[12], hdr[13], hdr[14], hdr[15]]);
        if opcode != OP_MSG {
            return Err(MongoError::Command(format!(
                "unexpected MongoDB response opcode {opcode}"
            )));
        }
        if !(21..=64 * 1024 * 1024).contains(&resp_len) {
            return Err(MongoError::Command(format!(
                "invalid MongoDB response length {resp_len}"
            )));
        }

        let mut payload = vec![0u8; resp_len - 16];
        self.stream.read_exact(&mut payload)?;

        // payload[0..4] = flagBits, followed by sections
        let mut pos = 4;
        while pos < payload.len() {
            let kind = payload[pos];
            pos += 1;
            match kind {
                0 => {
                    return decode_bson_doc(&payload[pos..]);
                }
                1 => {
                    if pos + 4 > payload.len() {
                        return Err(MongoError::Bson("truncated OP_MSG kind 1 section".into()));
                    }
                    let sec_size =
                        i32::from_le_bytes([payload[pos], payload[pos + 1], payload[pos + 2], payload[pos + 3]])
                            as usize;
                    pos += sec_size;
                }
                other => {
                    return Err(MongoError::Bson(format!("unknown OP_MSG section kind {other}")));
                }
            }
        }

        Err(MongoError::Command("OP_MSG response contained no body section".into()))
    }

    fn check_ok(reply: &Map<String, Value>, cmd_name: &str) -> Result<(), MongoError> {
        let ok = reply.get("ok").and_then(Value::as_f64).unwrap_or(0.0);
        if (ok - 1.0).abs() > f64::EPSILON {
            let errmsg = reply
                .get("errmsg")
                .and_then(Value::as_str)
                .unwrap_or("command returned ok != 1");
            return Err(MongoError::Command(format!("{cmd_name}: {errmsg}")));
        }
        Ok(())
    }

    fn authenticate_scram_sha256(&mut self, username: &str, password: &str, auth_db: &str) -> Result<(), MongoError> {
        let nonce_bytes: [u8; 24] = rand::random();
        let client_nonce = BASE64.encode(nonce_bytes);
        let escaped_user = username.replace('=', "=3D").replace(',', "=2C");
        let client_first_bare = format!("n={escaped_user},r={client_nonce}");
        let client_first = format!("n,,{client_first_bare}");

        let mut start = BsonBuf::new();
        start
            .put_i32("saslStart", 1)
            .put_str("mechanism", "SCRAM-SHA-256")
            .put_binary("payload", client_first.as_bytes())
            .put_i32("autoAuthorize", 1)
            .put_str("$db", auth_db);
        let start_reply = self.command(&start)?;
        Self::check_ok(&start_reply, "saslStart")?;

        let conv_id = start_reply.get("conversationId").and_then(Value::as_i64).unwrap_or(1) as i32;
        let server_first_b64 = start_reply
            .get("payload")
            .and_then(Value::as_str)
            .ok_or_else(|| MongoError::Auth("missing payload in saslStart response".into()))?;
        let server_first_bytes = BASE64
            .decode(server_first_b64)
            .map_err(|e| MongoError::Auth(format!("invalid base64 in saslStart: {e}")))?;
        let server_first = std::str::from_utf8(&server_first_bytes)
            .map_err(|e| MongoError::Auth(format!("invalid UTF-8 in saslStart: {e}")))?;

        let mut server_nonce = None;
        let mut salt_b64 = None;
        let mut iterations = None;
        for part in server_first.split(',') {
            if let Some(r) = part.strip_prefix("r=") {
                server_nonce = Some(r);
            } else if let Some(s) = part.strip_prefix("s=") {
                salt_b64 = Some(s);
            } else if let Some(i) = part.strip_prefix("i=") {
                iterations = i.parse::<u32>().ok();
            }
        }
        let server_nonce = server_nonce.ok_or_else(|| MongoError::Auth("missing r= in SCRAM".into()))?;
        if !server_nonce.starts_with(&client_nonce) {
            return Err(MongoError::Auth("server nonce did not prefix client nonce".into()));
        }
        let salt = BASE64
            .decode(salt_b64.ok_or_else(|| MongoError::Auth("missing s= in SCRAM".into()))?)
            .map_err(|e| MongoError::Auth(format!("invalid salt base64: {e}")))?;
        let iterations = iterations.ok_or_else(|| MongoError::Auth("missing i= in SCRAM".into()))?;

        let salted_password = pbkdf2_hmac_sha256(password.as_bytes(), &salt, iterations);
        let client_key = hmac_sha256(&salted_password, b"Client Key");
        let stored_key: [u8; 32] = Sha256::digest(client_key).into();

        let client_final_without_proof = format!("c=biws,r={server_nonce}");
        let auth_message = format!("{client_first_bare},{server_first},{client_final_without_proof}");
        let client_sig = hmac_sha256(&stored_key, auth_message.as_bytes());
        let mut client_proof = [0u8; 32];
        for i in 0..32 {
            client_proof[i] = client_key[i] ^ client_sig[i];
        }
        let proof_b64 = BASE64.encode(client_proof);
        let client_final = format!("{client_final_without_proof},p={proof_b64}");

        let mut cont = BsonBuf::new();
        cont.put_i32("saslContinue", 1)
            .put_i32("conversationId", conv_id)
            .put_binary("payload", client_final.as_bytes())
            .put_str("$db", auth_db);
        let cont_reply = self.command(&cont)?;
        Self::check_ok(&cont_reply, "saslContinue")?;

        if !cont_reply.get("done").and_then(Value::as_bool).unwrap_or(false) {
            let mut finish = BsonBuf::new();
            finish
                .put_i32("saslContinue", 1)
                .put_i32("conversationId", conv_id)
                .put_binary("payload", b"")
                .put_str("$db", auth_db);
            let finish_reply = self.command(&finish)?;
            Self::check_ok(&finish_reply, "saslContinue(final)")?;
        }

        Ok(())
    }

    /// Reads all documents in `collection` in insertion (`$natural`) order via `find` + `getMore`.
    pub fn find_all(&mut self, collection: &str) -> Result<Vec<Map<String, Value>>, MongoError> {
        let mut cmd = BsonBuf::new();
        cmd.put_str("find", collection)
            .put_i32("batchSize", DEFAULT_BATCH_SIZE)
            .put_str("$db", &self.database);
        let reply = self.command(&cmd)?;
        Self::check_ok(&reply, "find")?;

        let cursor = reply
            .get("cursor")
            .and_then(Value::as_object)
            .ok_or_else(|| MongoError::Command(format!("find({collection}): missing cursor object")))?;

        let mut out = Vec::new();
        if let Some(first_batch) = cursor.get("firstBatch").and_then(Value::as_array) {
            for item in first_batch {
                if let Some(obj) = item.as_object() {
                    out.push(obj.clone());
                }
            }
        }

        let mut cursor_id = cursor.get("id").and_then(Value::as_i64).unwrap_or(0);
        while cursor_id != 0 {
            let mut more = BsonBuf::new();
            more.put_i64("getMore", cursor_id)
                .put_str("collection", collection)
                .put_i32("batchSize", DEFAULT_BATCH_SIZE)
                .put_str("$db", &self.database);
            let more_reply = self.command(&more)?;
            Self::check_ok(&more_reply, "getMore")?;
            let more_cursor = more_reply
                .get("cursor")
                .and_then(Value::as_object)
                .ok_or_else(|| MongoError::Command(format!("getMore({collection}): missing cursor object")))?;
            if let Some(next_batch) = more_cursor.get("nextBatch").and_then(Value::as_array) {
                for item in next_batch {
                    if let Some(obj) = item.as_object() {
                        out.push(obj.clone());
                    }
                }
            }
            cursor_id = more_cursor.get("id").and_then(Value::as_i64).unwrap_or(0);
        }

        Ok(out)
    }
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let Ok(mut mac) = <Hmac<Sha256> as KeyInit>::new_from_slice(key) else {
        return [0u8; 32];
    };
    mac.update(data);
    mac.finalize().into_bytes().into()
}

fn pbkdf2_hmac_sha256(password: &[u8], salt: &[u8], iterations: u32) -> [u8; 32] {
    let mut salt_block = Vec::with_capacity(salt.len() + 4);
    salt_block.extend_from_slice(salt);
    salt_block.extend_from_slice(&1u32.to_be_bytes());
    let mut u = hmac_sha256(password, &salt_block);
    let mut out = u;
    for _ in 1..iterations {
        u = hmac_sha256(password, &u);
        for i in 0..32 {
            out[i] ^= u[i];
        }
    }
    out
}

/// Embedded wire-protocol MongoDB server speaking `OP_MSG` (`hello`, `find`, `getMore`) on `127.0.0.1:0`
/// for fast, self-contained tests of the full TCP + BSON + cursor pagination pipeline.
pub struct MockMongoServer {
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl MockMongoServer {
    pub fn start(collections: BTreeMap<String, Vec<Map<String, Value>>>, max_batch: usize) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);
        let collections = Arc::new(collections);
        let max_batch = max_batch.max(1);

        let handle = std::thread::spawn(move || {
            while !stop_flag.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let cols = Arc::clone(&collections);
                        std::thread::spawn(move || {
                            let _ = handle_mock_client(stream, &cols, max_batch);
                        });
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            addr,
            stop,
            handle: Some(handle),
        })
    }

    pub fn uri(&self, db_name: &str) -> String {
        format!("mongodb://{}/{db_name}", self.addr)
    }
}

impl Drop for MockMongoServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(self.addr);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn handle_mock_client(
    mut stream: TcpStream,
    collections: &BTreeMap<String, Vec<Map<String, Value>>>,
    max_batch: usize,
) -> Result<(), MongoError> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_write_timeout(Some(Duration::from_secs(10)))?;

    let mut cursors: BTreeMap<i64, (String, usize)> = BTreeMap::new();
    let mut next_cursor_id = 100i64;

    loop {
        let mut hdr = [0u8; 16];
        match stream.read_exact(&mut hdr) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(MongoError::Io(e)),
        }
        let msg_len = i32::from_le_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]) as usize;
        let req_id = i32::from_le_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]);
        if msg_len < 21 {
            return Err(MongoError::Bson("OP_MSG frame too short".into()));
        }
        let mut payload = vec![0u8; msg_len - 16];
        stream.read_exact(&mut payload)?;
        let cmd = decode_bson_doc(&payload[5..])?;

        let mut reply = BsonBuf::new();
        if cmd.contains_key("hello") || cmd.contains_key("ismaster") || cmd.contains_key("isMaster") {
            reply
                .put_bool("isWritablePrimary", true)
                .put_i32("maxWireVersion", 21)
                .put_f64("ok", 1.0);
        } else if let Some(col_name) = cmd.get("find").and_then(Value::as_str) {
            let docs = collections.get(col_name).map(Vec::as_slice).unwrap_or(&[]);
            let req_batch = cmd
                .get("batchSize")
                .and_then(Value::as_i64)
                .map(|n| n as usize)
                .unwrap_or(max_batch)
                .min(max_batch);
            let take = docs.len().min(req_batch);
            let batch_docs: Vec<BsonBuf> = docs[..take].iter().map(BsonBuf::from_json_map).collect();
            let cursor_id = if take < docs.len() {
                let cid = next_cursor_id;
                next_cursor_id += 1;
                cursors.insert(cid, (col_name.to_owned(), take));
                cid
            } else {
                0
            };
            let mut cursor = BsonBuf::new();
            cursor
                .put_i64("id", cursor_id)
                .put_str("ns", &format!("dauntless.{col_name}"))
                .put_array_docs("firstBatch", &batch_docs);
            reply.put_doc("cursor", &cursor).put_f64("ok", 1.0);
        } else if let Some(cid) = cmd.get("getMore").and_then(Value::as_i64) {
            if let Some((col_name, offset)) = cursors.remove(&cid) {
                let docs = collections.get(&col_name).map(Vec::as_slice).unwrap_or(&[]);
                let req_batch = cmd
                    .get("batchSize")
                    .and_then(Value::as_i64)
                    .map(|n| n as usize)
                    .unwrap_or(max_batch)
                    .min(max_batch);
                let end = (offset + req_batch).min(docs.len());
                let batch_docs: Vec<BsonBuf> = docs[offset..end].iter().map(BsonBuf::from_json_map).collect();
                let next_cid = if end < docs.len() {
                    cursors.insert(cid, (col_name.clone(), end));
                    cid
                } else {
                    0
                };
                let mut cursor = BsonBuf::new();
                cursor
                    .put_i64("id", next_cid)
                    .put_str("ns", &format!("dauntless.{col_name}"))
                    .put_array_docs("nextBatch", &batch_docs);
                reply.put_doc("cursor", &cursor).put_f64("ok", 1.0);
            } else {
                reply.put_f64("ok", 0.0).put_str("errmsg", "cursor not found");
            }
        } else {
            reply.put_f64("ok", 0.0).put_str("errmsg", "unsupported command");
        }

        let body_bytes = reply.finish();
        let resp_len = (16 + 4 + 1 + body_bytes.len()) as i32;
        let mut frame = Vec::with_capacity(resp_len as usize);
        frame.extend_from_slice(&resp_len.to_le_bytes());
        frame.extend_from_slice(&req_id.to_le_bytes());
        frame.extend_from_slice(&req_id.to_le_bytes());
        frame.extend_from_slice(&OP_MSG.to_le_bytes());
        frame.extend_from_slice(&0u32.to_le_bytes());
        frame.push(0u8);
        frame.extend_from_slice(&body_bytes);
        stream.write_all(&frame)?;
        stream.flush()?;
    }
}
