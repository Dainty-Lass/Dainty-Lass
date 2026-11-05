// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The tool's own MongoDB client (spec §4): OP_MSG over plain TCP, no `mongodb` crate (RB15). Reading and
//! writing are two types: [`MongoClient`] can send only `hello`, `saslStart`, `saslContinue`, `find` and
//! `getMore`, so migrate and verify are read-only by construction; [`MongoWriter`] exists only with
//! `export-mongo`.

mod bson;
#[cfg(any(test, feature = "test-support"))]
mod mock;
#[doc(hidden)]
pub mod scram;
mod uri;

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use base64::Engine as _;
use serde_json::{Map, Value};

pub(crate) use bson::B;
pub use bson::{MAX_DEPTH, decode_document, encode_document};
#[cfg(any(test, feature = "test-support"))]
pub use mock::MockMongoServer;
pub use uri::MongoUri;

const OP_MSG: i32 = 2013;
const MAX_REPLY: usize = 64 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(30);
const BATCH: i32 = 500;

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

/// What the connection runs over: a TCP stream, or bytes in memory for the fuzz target.
pub trait Stream: Read + Write + Send {}
impl<T: Read + Write + Send> Stream for T {}

struct Connection {
    stream: Box<dyn Stream>,
    next_id: i32,
}

impl Connection {
    fn send(&mut self, name: &str, fields: &[(&str, B<'_>)]) -> Result<Map<String, Value>, MongoError> {
        let body = bson::encode_command(fields)?;
        let total =
            i32::try_from(16 + 4 + 1 + body.len()).map_err(|_| MongoError::Command(format!("{name}: too large")))?;
        self.next_id = self.next_id.wrapping_add(1);
        let mut message = Vec::with_capacity(body.len() + 21);
        for n in [total, self.next_id, 0, OP_MSG] {
            message.extend_from_slice(&n.to_le_bytes());
        }
        message.extend_from_slice(&[0, 0, 0, 0, 0]);
        message.extend_from_slice(&body);
        self.stream.write_all(&message)?;
        self.stream.flush()?;
        let reply = read_reply(&mut self.stream)?;
        check_reply(name, reply)
    }
}

/// One OP_MSG reply's body document.
#[doc(hidden)]
pub fn read_reply(stream: &mut dyn Read) -> Result<Map<String, Value>, MongoError> {
    let mut header = [0u8; 16];
    stream.read_exact(&mut header)?;
    let word = |i: usize| i32::from_le_bytes([header[i], header[i + 1], header[i + 2], header[i + 3]]);
    let opcode = word(12);
    if opcode != OP_MSG {
        return Err(MongoError::Command(format!(
            "unexpected MongoDB response opcode {opcode}"
        )));
    }
    let declared = word(0);
    let total = match usize::try_from(declared) {
        Ok(n) if (21..=MAX_REPLY).contains(&n) => n,
        _ => {
            return Err(MongoError::Command(format!(
                "invalid MongoDB response length {declared}"
            )));
        }
    };
    let mut rest = vec![0u8; total - 16];
    stream.read_exact(&mut rest)?;
    let mut pos = 4;
    while pos < rest.len() {
        let kind = rest[pos];
        pos += 1;
        match kind {
            0 => return decode_document(&rest[pos..]),
            1 => {
                let size = rest
                    .get(pos..pos + 4)
                    .map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .and_then(|n| usize::try_from(n).ok())
                    .filter(|n| *n >= 4 && pos + n <= rest.len())
                    .ok_or_else(|| MongoError::Command("truncated OP_MSG kind 1 section".into()))?;
                pos += size;
            }
            k => return Err(MongoError::Command(format!("unknown OP_MSG section kind {k}"))),
        }
    }
    Err(MongoError::Command("OP_MSG response contained no body section".into()))
}

fn check_reply(name: &str, reply: Map<String, Value>) -> Result<Map<String, Value>, MongoError> {
    if reply.get("ok").and_then(Value::as_f64).unwrap_or(0.0) != 1.0 {
        let message = reply
            .get("errmsg")
            .and_then(Value::as_str)
            .unwrap_or("command returned ok != 1");
        return Err(MongoError::Command(format!("{name}: {message}")));
    }
    if let Some(Value::Array(errors)) = reply.get("writeErrors")
        && let Some(first) = errors.first()
    {
        let message = first
            .get("errmsg")
            .and_then(Value::as_str)
            .unwrap_or("writeError reported by MongoDB");
        return Err(MongoError::Command(format!("{name}: {message}")));
    }
    Ok(reply)
}

fn open_host(host: &str, port: u16, uri: &MongoUri) -> Result<Connection, MongoError> {
    let address = (host, port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, format!("no address for {host}")))?;
    let stream = TcpStream::connect_timeout(&address, TIMEOUT)?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;
    let mut conn = Connection {
        stream: Box::new(stream),
        next_id: 0,
    };
    conn.send("hello", &[("hello", B::I32(1)), ("$db", B::Str("admin"))])?;
    if let Some((user, password)) = uri.credentials() {
        authenticate(&mut conn, user, password, &uri.auth_source, Scram::random_nonce())?;
    }
    Ok(conn)
}

use scram::Scram;

fn payload_text(reply: &Map<String, Value>, step: &str) -> Result<Vec<u8>, MongoError> {
    let text = reply
        .get("payload")
        .and_then(Value::as_str)
        .ok_or_else(|| MongoError::Auth(format!("missing payload in {step} response")))?;
    base64::engine::general_purpose::STANDARD
        .decode(text)
        .map_err(|e| MongoError::Auth(format!("invalid base64 in {step}: {e}")))
}

fn authenticate(conn: &mut Connection, user: &str, password: &str, db: &str, nonce: String) -> Result<(), MongoError> {
    let mut scram = Scram::new(user, password, nonce);
    let first = scram.client_first();
    let reply = conn.send(
        "saslStart",
        &[
            ("saslStart", B::I32(1)),
            ("mechanism", B::Str("SCRAM-SHA-256")),
            ("payload", B::Bin(first.as_bytes())),
            ("autoAuthorize", B::I32(1)),
            ("$db", B::Str(db)),
        ],
    )?;
    let conversation = reply
        .get("conversationId")
        .and_then(Value::as_i64)
        .and_then(|n| i32::try_from(n).ok())
        .unwrap_or(1);
    let server_first = payload_text(&reply, "saslStart")?;
    let client_final = scram.client_final(&server_first).map_err(MongoError::Auth)?;
    let reply = conn.send(
        "saslContinue",
        &[
            ("saslContinue", B::I32(1)),
            ("conversationId", B::I32(conversation)),
            ("payload", B::Bin(client_final.as_bytes())),
            ("$db", B::Str(db)),
        ],
    )?;
    let server_final = payload_text(&reply, "saslContinue")?;
    scram.verify_server_final(&server_final).map_err(MongoError::Auth)?;
    if reply.get("done") != Some(&Value::Bool(true)) {
        conn.send(
            "saslContinue(final)",
            &[
                ("saslContinue", B::I32(1)),
                ("conversationId", B::I32(conversation)),
                ("payload", B::Bin(&[])),
                ("$db", B::Str(db)),
            ],
        )?;
    }
    Ok(())
}

fn connect(uri: &MongoUri) -> Result<Connection, MongoError> {
    let mut last = MongoError::InvalidUri("no hosts in MongoDB URI".into());
    for (host, port) in &uri.hosts {
        match open_host(host, *port, uri) {
            Ok(conn) => return Ok(conn),
            Err(e) => last = e,
        }
    }
    Err(last)
}

fn cursor_of<'a>(reply: &'a Map<String, Value>, what: &str) -> Result<&'a Map<String, Value>, MongoError> {
    reply
        .get("cursor")
        .and_then(Value::as_object)
        .ok_or_else(|| MongoError::Command(format!("{what}: missing cursor object")))
}

fn documents(cursor: &Map<String, Value>, batch: &str) -> impl Iterator<Item = Map<String, Value>> {
    cursor
        .get(batch)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|d| d.as_object().cloned())
}

/// The reading side: the five commands migrate and verify send, and nothing else.
pub struct MongoClient {
    conn: Connection,
    database: String,
}

impl MongoClient {
    /// The first host that connects, answers `hello` and (with credentials) authenticates; no discovery.
    pub fn connect(uri: &MongoUri) -> Result<Self, MongoError> {
        Ok(Self {
            conn: connect(uri)?,
            database: uri.database.clone(),
        })
    }

    /// A client over a stream that has already been greeted (the fuzz target's in-memory server).
    #[doc(hidden)]
    pub fn over_stream(stream: Box<dyn Stream>, database: &str) -> Self {
        Self {
            conn: Connection { stream, next_id: 0 },
            database: database.to_owned(),
        }
    }

    pub fn database(&self) -> &str {
        &self.database
    }

    /// Every document, in natural order, held in memory; a missing collection reads empty.
    pub fn find_all(&mut self, collection: &str) -> Result<Vec<Map<String, Value>>, MongoError> {
        let db = self.database.clone();
        let reply = self.conn.send(
            "find",
            &[
                ("find", B::Str(collection)),
                ("batchSize", B::I32(BATCH)),
                ("$db", B::Str(&db)),
            ],
        )?;
        let cursor = cursor_of(&reply, &format!("find({collection})"))?;
        let mut out: Vec<_> = documents(cursor, "firstBatch").collect();
        let mut id = cursor.get("id").and_then(Value::as_i64).unwrap_or(0);
        while id != 0 {
            let reply = self.conn.send(
                "getMore",
                &[
                    ("getMore", B::I64(id)),
                    ("collection", B::Str(collection)),
                    ("batchSize", B::I32(BATCH)),
                    ("$db", B::Str(&db)),
                ],
            )?;
            let cursor = cursor_of(&reply, &format!("getMore({collection})"))?;
            out.extend(documents(cursor, "nextBatch"));
            id = cursor.get("id").and_then(Value::as_i64).unwrap_or(0);
        }
        Ok(out)
    }

    /// Whether the collection holds a document (the host kit's update path).
    pub fn collection_has_documents(&mut self, collection: &str) -> Result<bool, MongoError> {
        let db = self.database.clone();
        let reply = self.conn.send(
            "find",
            &[
                ("find", B::Str(collection)),
                ("limit", B::I32(1)),
                ("singleBatch", B::Bool(true)),
                ("batchSize", B::I32(1)),
                ("$db", B::Str(&db)),
            ],
        )?;
        let cursor = cursor_of(&reply, &format!("find({collection})"))?;
        Ok(cursor
            .get("firstBatch")
            .and_then(Value::as_array)
            .is_some_and(|b| !b.is_empty()))
    }
}

/// The writing side, for export-mongo only (RB13): replaces whole collections.
#[cfg(feature = "export-mongo")]
pub struct MongoWriter {
    conn: Connection,
    database: String,
}

#[cfg(feature = "export-mongo")]
impl MongoWriter {
    pub fn connect(uri: &MongoUri) -> Result<Self, MongoError> {
        Ok(Self {
            conn: connect(uri)?,
            database: uri.database.clone(),
        })
    }

    /// Deletes every document, then inserts `documents` 500 at a time. Not transactional.
    pub fn replace_collection(&mut self, collection: &str, documents: &[Map<String, Value>]) -> Result<(), MongoError> {
        if documents.iter().any(has_nul_key) {
            return Err(MongoError::Command(format!("BSON key contains NUL: {collection}")));
        }
        let db = self.database.clone();
        self.conn.send(
            "delete",
            &[
                ("delete", B::Str(collection)),
                (
                    "deletes",
                    B::Arr(vec![B::Doc(vec![("q", B::Doc(Vec::new())), ("limit", B::I32(0))])]),
                ),
                ("$db", B::Str(&db)),
            ],
        )?;
        for chunk in documents.chunks(BATCH as usize) {
            let values: Vec<Value> = chunk.iter().cloned().map(Value::Object).collect();
            self.conn.send(
                "insert",
                &[
                    ("insert", B::Str(collection)),
                    ("documents", B::Arr(values.iter().map(B::Json).collect())),
                    ("$db", B::Str(&db)),
                ],
            )?;
        }
        Ok(())
    }
}

#[cfg(feature = "export-mongo")]
fn has_nul_key(document: &Map<String, Value>) -> bool {
    fn value(v: &Value) -> bool {
        match v {
            Value::Object(m) => has_nul_key(m),
            Value::Array(items) => items.iter().any(value),
            _ => false,
        }
    }
    document.iter().any(|(k, v)| k.contains('\0') || value(v))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn reply(body: &[u8], opcode: i32, kind1: Option<&[u8]>) -> Vec<u8> {
        let mut rest = vec![0, 0, 0, 0];
        if let Some(section) = kind1 {
            rest.push(1);
            rest.extend_from_slice(section);
        }
        rest.push(0);
        rest.extend_from_slice(body);
        let total = i32::try_from(rest.len() + 16).unwrap();
        let mut out = Vec::new();
        for n in [total, 1, 1, opcode] {
            out.extend_from_slice(&n.to_le_bytes());
        }
        out.extend(rest);
        out
    }

    #[test]
    fn replies_are_bounded_and_walked() {
        let body = encode_document(json!({"ok": 1}).as_object().unwrap()).unwrap();
        let ok = reply(&body, OP_MSG, None);
        assert_eq!(read_reply(&mut ok.as_slice()).unwrap()["ok"], 1);
        let mut section = 8i32.to_le_bytes().to_vec();
        section.extend_from_slice(b"abcd");
        let skipped = reply(&body, OP_MSG, Some(&section));
        assert_eq!(read_reply(&mut skipped.as_slice()).unwrap()["ok"], 1);
        let err = |bytes: Vec<u8>| read_reply(&mut bytes.as_slice()).unwrap_err().to_string();
        assert_eq!(
            err(reply(&body, 1, None)),
            "MongoDB command error: unexpected MongoDB response opcode 1"
        );
        let mut huge = ok.clone();
        huge[0..4].copy_from_slice(&(65 * 1024 * 1024i32).to_le_bytes());
        assert!(err(huge).contains("invalid MongoDB response length"));
        let mut short = ok;
        short[0..4].copy_from_slice(&20i32.to_le_bytes());
        assert!(err(short).contains("invalid MongoDB response length 20"));
    }

    #[test]
    fn failed_commands_name_themselves() {
        let failed = json!({"ok": 0, "errmsg": "nope"}).as_object().cloned().unwrap();
        assert_eq!(
            check_reply("find", failed).unwrap_err().to_string(),
            "MongoDB command error: find: nope"
        );
        let write = json!({"ok": 1, "writeErrors": [{"errmsg": "dup"}]})
            .as_object()
            .cloned()
            .unwrap();
        assert_eq!(
            check_reply("insert", write).unwrap_err().to_string(),
            "MongoDB command error: insert: dup"
        );
        let missing = json!({}).as_object().cloned().unwrap();
        assert!(
            check_reply("hello", missing)
                .unwrap_err()
                .to_string()
                .contains("ok != 1")
        );
    }
}
