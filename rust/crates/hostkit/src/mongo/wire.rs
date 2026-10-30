// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Minimal MongoDB `OP_MSG` (opcode 2013) binary wire protocol encoder, decoder, and synchronous
//! TCP client for inspecting and initiating a local replica set without an external driver crate.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

/// Opcode for MongoDB `OP_MSG` frames.
pub const OP_MSG_CODE: i32 = 2013;

/// Maximum `OP_MSG` frame size accepted from a server (16 MiB).
const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;

/// Ordered BSON document representation.
pub type Doc = Vec<(String, Value)>;

/// Subset of BSON values needed for `hello`, `replSetGetStatus`, `replSetInitiate`, and `count`.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Double(f64),
    Str(String),
    Doc(Doc),
    Array(Vec<Value>),
    Bool(bool),
    Null,
    Int32(i32),
    Int64(i64),
    Other(u8),
}

impl Value {
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Double(n) => Some(*n),
            Self::Int32(n) => Some(f64::from(*n)),
            Self::Int64(n) => Some(*n as f64),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int32(n) => Some(i64::from(*n)),
            Self::Int64(n) => Some(*n),
            Self::Double(n) => Some(*n as i64),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            Self::Int32(n) => Some(*n != 0),
            Self::Int64(n) => Some(*n != 0),
            Self::Double(n) => Some(*n != 0.0),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_doc(&self) -> Option<&Doc> {
        match self {
            Self::Doc(d) => Some(d),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Self::Array(a) => Some(a.as_slice()),
            _ => None,
        }
    }
}

/// Helper trait allowing `as_str`, `as_f64`, and `as_bool` to accept either `&Value` or `Option<&Value>`.
pub trait ValueLookup<'a> {
    fn into_value(self) -> Option<&'a Value>;
}

impl<'a> ValueLookup<'a> for &'a Value {
    fn into_value(self) -> Option<&'a Value> {
        Some(self)
    }
}

impl<'a> ValueLookup<'a> for Option<&'a Value> {
    fn into_value(self) -> Option<&'a Value> {
        self
    }
}

/// Looks up `key` in `doc`.
#[must_use]
pub fn find<'a>(doc: &'a Doc, key: &str) -> Option<&'a Value> {
    doc.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

/// Looks up `key` in `doc` (alias for [`find`]).
#[must_use]
pub fn get<'a>(doc: &'a Doc, key: &str) -> Option<&'a Value> {
    find(doc, key)
}

/// Extracts a `&str` from a `&Value` or `Option<&Value>`.
#[must_use]
pub fn as_str<'a>(value: impl ValueLookup<'a>) -> Option<&'a str> {
    value.into_value()?.as_str()
}

/// Extracts an `f64` from a `&Value` or `Option<&Value>` (coercing `Int32` and `Int64`).
#[must_use]
pub fn as_f64<'a>(value: impl ValueLookup<'a>) -> Option<f64> {
    value.into_value()?.as_f64()
}

/// Extracts a `bool` from a `&Value` or `Option<&Value>`.
#[must_use]
pub fn as_bool<'a>(value: impl ValueLookup<'a>) -> Option<bool> {
    value.into_value()?.as_bool()
}

fn push_cstring(out: &mut Vec<u8>, s: &str) {
    for &b in s.as_bytes() {
        if b != 0 {
            out.push(b);
        }
    }
    out.push(0);
}

fn encode_element(out: &mut Vec<u8>, key: &str, value: &Value) {
    match value {
        Value::Double(n) => {
            out.push(0x01);
            push_cstring(out, key);
            out.extend_from_slice(&n.to_le_bytes());
        }
        Value::Str(s) => {
            out.push(0x02);
            push_cstring(out, key);
            let len = i32::try_from(s.len().saturating_add(1)).unwrap_or(i32::MAX);
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(s.as_bytes());
            out.push(0);
        }
        Value::Doc(sub) => {
            out.push(0x03);
            push_cstring(out, key);
            out.extend_from_slice(&encode_doc(sub));
        }
        Value::Array(items) => {
            out.push(0x04);
            push_cstring(out, key);
            let indexed: Doc = items
                .iter()
                .enumerate()
                .map(|(i, item)| (i.to_string(), item.clone()))
                .collect();
            out.extend_from_slice(&encode_doc(&indexed));
        }
        Value::Bool(b) => {
            out.push(0x08);
            push_cstring(out, key);
            out.push(u8::from(*b));
        }
        Value::Null | Value::Other(_) => {
            out.push(0x0A);
            push_cstring(out, key);
        }
        Value::Int32(n) => {
            out.push(0x10);
            push_cstring(out, key);
            out.extend_from_slice(&n.to_le_bytes());
        }
        Value::Int64(n) => {
            out.push(0x12);
            push_cstring(out, key);
            out.extend_from_slice(&n.to_le_bytes());
        }
    }
}

/// Encodes `doc` as a BSON byte vector.
#[must_use]
pub fn encode_doc(doc: &Doc) -> Vec<u8> {
    let mut out = vec![0u8; 4];
    for (k, v) in doc {
        encode_element(&mut out, k, v);
    }
    out.push(0);
    let total = i32::try_from(out.len()).unwrap_or(i32::MAX);
    out[..4].copy_from_slice(&total.to_le_bytes());
    out
}

fn bad_bson(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.into())
}

fn read_i32_le(bytes: &[u8], pos: &mut usize) -> io::Result<i32> {
    let slice = bytes
        .get(*pos..*pos + 4)
        .ok_or_else(|| bad_bson("truncated BSON int32"))?;
    let mut arr = [0u8; 4];
    arr.copy_from_slice(slice);
    *pos += 4;
    Ok(i32::from_le_bytes(arr))
}

fn read_i64_le(bytes: &[u8], pos: &mut usize) -> io::Result<i64> {
    let slice = bytes
        .get(*pos..*pos + 8)
        .ok_or_else(|| bad_bson("truncated BSON int64"))?;
    let mut arr = [0u8; 8];
    arr.copy_from_slice(slice);
    *pos += 8;
    Ok(i64::from_le_bytes(arr))
}

fn read_f64_le(bytes: &[u8], pos: &mut usize) -> io::Result<f64> {
    let slice = bytes
        .get(*pos..*pos + 8)
        .ok_or_else(|| bad_bson("truncated BSON double"))?;
    let mut arr = [0u8; 8];
    arr.copy_from_slice(slice);
    *pos += 8;
    Ok(f64::from_le_bytes(arr))
}

fn read_cstring(bytes: &[u8], pos: &mut usize) -> io::Result<String> {
    let rest = bytes
        .get(*pos..)
        .ok_or_else(|| bad_bson("truncated BSON cstring"))?;
    let nul = rest
        .iter()
        .position(|&b| b == 0)
        .ok_or_else(|| bad_bson("unterminated BSON cstring"))?;
    let s = std::str::from_utf8(&rest[..nul])
        .map_err(|_| bad_bson("invalid UTF-8 in BSON cstring"))?
        .to_owned();
    *pos += nul + 1;
    Ok(s)
}

fn read_string(bytes: &[u8], pos: &mut usize) -> io::Result<String> {
    let len = read_i32_le(bytes, pos)?;
    if len < 1 {
        return Err(bad_bson("invalid BSON string length"));
    }
    let len_usize = usize::try_from(len).map_err(|_| bad_bson("negative BSON string length"))?;
    let slice = bytes
        .get(*pos..*pos + len_usize)
        .ok_or_else(|| bad_bson("truncated BSON string"))?;
    if slice.last() != Some(&0) {
        return Err(bad_bson("BSON string missing trailing NUL"));
    }
    let s = std::str::from_utf8(&slice[..len_usize - 1])
        .map_err(|_| bad_bson("invalid UTF-8 in BSON string"))?
        .to_owned();
    *pos += len_usize;
    Ok(s)
}

/// Decodes a BSON document from `bytes`.
pub fn decode_doc(bytes: &[u8]) -> io::Result<Doc> {
    let mut pos = 0;
    let total_i32 = read_i32_le(bytes, &mut pos)?;
    if total_i32 < 5 {
        return Err(bad_bson("BSON document length smaller than 5 bytes"));
    }
    let total = usize::try_from(total_i32).map_err(|_| bad_bson("invalid BSON document length"))?;
    let doc_bytes = bytes
        .get(..total)
        .ok_or_else(|| bad_bson("truncated BSON document"))?;
    if doc_bytes.last() != Some(&0) {
        return Err(bad_bson("BSON document missing trailing NUL"));
    }

    let mut out = Vec::new();
    let end = total - 1;
    while pos < end {
        let tag = doc_bytes[pos];
        pos += 1;
        if tag == 0 {
            break;
        }
        let key = read_cstring(doc_bytes, &mut pos)?;
        let value = match tag {
            0x01 => Value::Double(read_f64_le(doc_bytes, &mut pos)?),
            0x02 | 0x0D | 0x0E => Value::Str(read_string(doc_bytes, &mut pos)?),
            0x03 => {
                let mut peek = pos;
                let sub_len = usize::try_from(read_i32_le(doc_bytes, &mut peek)?)
                    .map_err(|_| bad_bson("invalid subdocument length"))?;
                let sub_slice = doc_bytes
                    .get(pos..pos + sub_len)
                    .ok_or_else(|| bad_bson("truncated BSON subdocument"))?;
                let sub = decode_doc(sub_slice)?;
                pos += sub_len;
                Value::Doc(sub)
            }
            0x04 => {
                let mut peek = pos;
                let sub_len = usize::try_from(read_i32_le(doc_bytes, &mut peek)?)
                    .map_err(|_| bad_bson("invalid array length"))?;
                let sub_slice = doc_bytes
                    .get(pos..pos + sub_len)
                    .ok_or_else(|| bad_bson("truncated BSON array"))?;
                let sub = decode_doc(sub_slice)?;
                pos += sub_len;
                Value::Array(sub.into_iter().map(|(_, v)| v).collect())
            }
            0x05 => {
                let bin_len = usize::try_from(read_i32_le(doc_bytes, &mut pos)?)
                    .map_err(|_| bad_bson("invalid binary length"))?;
                let skip = bin_len.saturating_add(1);
                if pos + skip > end {
                    return Err(bad_bson("truncated BSON binary"));
                }
                pos += skip;
                Value::Other(0x05)
            }
            0x06 | 0x0A | 0x7F | 0xFF => {
                if tag == 0x0A {
                    Value::Null
                } else {
                    Value::Other(tag)
                }
            }
            0x07 => {
                if pos + 12 > end {
                    return Err(bad_bson("truncated BSON ObjectId"));
                }
                pos += 12;
                Value::Other(0x07)
            }
            0x08 => {
                let b = *doc_bytes
                    .get(pos)
                    .ok_or_else(|| bad_bson("truncated BSON bool"))?;
                pos += 1;
                Value::Bool(b != 0)
            }
            0x09 | 0x11 | 0x12 => {
                let n = read_i64_le(doc_bytes, &mut pos)?;
                if tag == 0x12 {
                    Value::Int64(n)
                } else {
                    Value::Other(tag)
                }
            }
            0x0B => {
                let _ = read_cstring(doc_bytes, &mut pos)?;
                let _ = read_cstring(doc_bytes, &mut pos)?;
                Value::Other(0x0B)
            }
            0x10 => Value::Int32(read_i32_le(doc_bytes, &mut pos)?),
            0x13 => {
                if pos + 16 > end {
                    return Err(bad_bson("truncated BSON Decimal128"));
                }
                pos += 16;
                Value::Other(0x13)
            }
            other => {
                return Err(bad_bson(format!("unsupported BSON element tag 0x{other:02x}")));
            }
        };
        out.push((key, value));
    }
    Ok(out)
}

/// Frames `doc` into a complete MongoDB `OP_MSG` wire packet (16-byte header + 4-byte flags + section 0).
#[must_use]
pub fn op_msg(doc: &Doc) -> Vec<u8> {
    let body = encode_doc(doc);
    let total_len = i32::try_from(16 + 4 + 1 + body.len()).unwrap_or(i32::MAX);
    let mut packet = Vec::with_capacity(21 + body.len());
    packet.extend_from_slice(&total_len.to_le_bytes());
    packet.extend_from_slice(&1_i32.to_le_bytes()); // requestID
    packet.extend_from_slice(&0_i32.to_le_bytes()); // responseTo
    packet.extend_from_slice(&OP_MSG_CODE.to_le_bytes()); // opCode = 2013
    packet.extend_from_slice(&0_u32.to_le_bytes()); // flagBits = 0
    packet.push(0); // section kind 0 (body)
    packet.extend_from_slice(&body);
    packet
}

/// Parses a complete `OP_MSG` frame (including the 16-byte message header) and returns its body [`Doc`].
pub fn parse_op_msg(bytes: &[u8]) -> io::Result<Doc> {
    if bytes.len() < 21 {
        return Err(bad_bson("OP_MSG frame shorter than 21 bytes"));
    }
    let mut pos = 0;
    let total_i32 = read_i32_le(bytes, &mut pos)?;
    let total = usize::try_from(total_i32).map_err(|_| bad_bson("negative OP_MSG length"))?;
    if bytes.len() < total || total < 21 {
        return Err(bad_bson("truncated OP_MSG frame"));
    }
    let _req_id = read_i32_le(bytes, &mut pos)?;
    let _resp_to = read_i32_le(bytes, &mut pos)?;
    let op_code = read_i32_le(bytes, &mut pos)?;
    if op_code != OP_MSG_CODE {
        return Err(bad_bson(format!("expected OP_MSG ({OP_MSG_CODE}), got {op_code}")));
    }
    let flags = read_i32_le(bytes, &mut pos)? as u32;
    let end = if (flags & 1) != 0 {
        total.saturating_sub(4)
    } else {
        total
    };

    let mut body_doc: Option<Doc> = None;
    while pos < end {
        let kind = bytes[pos];
        pos += 1;
        match kind {
            0 => {
                let mut peek = pos;
                let doc_len = usize::try_from(read_i32_le(bytes, &mut peek)?)
                    .map_err(|_| bad_bson("invalid section 0 BSON length"))?;
                let slice = bytes
                    .get(pos..pos + doc_len)
                    .ok_or_else(|| bad_bson("truncated section 0 BSON document"))?;
                body_doc = Some(decode_doc(slice)?);
                pos += doc_len;
            }
            1 => {
                let mut peek = pos;
                let sec_size = usize::try_from(read_i32_le(bytes, &mut peek)?)
                    .map_err(|_| bad_bson("invalid section 1 size"))?;
                if sec_size < 4 || pos + sec_size > end {
                    return Err(bad_bson("truncated section 1 document sequence"));
                }
                pos += sec_size;
            }
            other => {
                return Err(bad_bson(format!("unknown OP_MSG section kind {other}")));
            }
        }
    }

    body_doc.ok_or_else(|| bad_bson("OP_MSG contained no body section"))
}

/// Connects to `addr`, sends `doc` inside an `OP_MSG`, and reads back the response [`Doc`].
pub fn command(addr: SocketAddr, doc: &Doc, timeout: Duration) -> io::Result<Doc> {
    let mut stream = TcpStream::connect_timeout(&addr, timeout)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;

    let req = op_msg(doc);
    stream.write_all(&req)?;
    stream.flush()?;

    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let total_i32 = i32::from_le_bytes(len_buf);
    let total = usize::try_from(total_i32).map_err(|_| bad_bson("negative response length"))?;
    if !(21..=MAX_FRAME_BYTES).contains(&total) {
        return Err(bad_bson(format!("invalid OP_MSG response size: {total}")));
    }

    let mut frame = vec![0u8; total];
    frame[..4].copy_from_slice(&len_buf);
    stream.read_exact(&mut frame[4..])?;
    parse_op_msg(&frame)
}

/// Ordered BSON document type used by wire-protocol callers and tests.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Document(pub Vec<(String, Bson)>);

/// BSON value variants corresponding to [`Document`].
#[derive(Debug, Clone, PartialEq)]
pub enum Bson {
    Double(f64),
    String(String),
    Document(Document),
    Array(Vec<Bson>),
    Bool(bool),
    Null,
    Int32(i32),
    Int64(i64),
    Other(u8),
}

impl Document {
    #[must_use]
    pub fn new() -> Self {
        Self(Vec::new())
    }

    #[must_use]
    pub fn with(mut self, key: impl Into<String>, val: Bson) -> Self {
        self.0.push((key.into(), val));
        self
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Bson> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    #[must_use]
    pub fn str(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Bson::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    fn from_doc(doc: Doc) -> Self {
        Self(
            doc.into_iter()
                .map(|(k, v)| (k, Bson::from_value(v)))
                .collect(),
        )
    }

    fn to_doc(&self) -> Doc {
        self.0
            .iter()
            .map(|(k, v)| (k.clone(), v.to_value()))
            .collect()
    }
}

impl Bson {
    fn from_value(val: Value) -> Self {
        match val {
            Value::Double(n) => Self::Double(n),
            Value::Str(s) => Self::String(s),
            Value::Doc(d) => Self::Document(Document::from_doc(d)),
            Value::Array(a) => Self::Array(a.into_iter().map(Self::from_value).collect()),
            Value::Bool(b) => Self::Bool(b),
            Value::Null => Self::Null,
            Value::Int32(n) => Self::Int32(n),
            Value::Int64(n) => Self::Int64(n),
            Value::Other(t) => Self::Other(t),
        }
    }

    fn to_value(&self) -> Value {
        match self {
            Self::Double(n) => Value::Double(*n),
            Self::String(s) => Value::Str(s.clone()),
            Self::Document(d) => Value::Doc(d.to_doc()),
            Self::Array(a) => Value::Array(a.iter().map(Self::to_value).collect()),
            Self::Bool(b) => Value::Bool(*b),
            Self::Null => Value::Null,
            Self::Int32(n) => Value::Int32(*n),
            Self::Int64(n) => Value::Int64(*n),
            Self::Other(t) => Value::Other(*t),
        }
    }
}

/// Reads a single `OP_MSG` request frame from `stream`, returning `(request_id, document)`.
pub fn read_request(stream: &mut impl Read) -> Option<(i32, Document)> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).ok()?;
    let total = usize::try_from(i32::from_le_bytes(len_buf)).ok()?;
    if !(21..=MAX_FRAME_BYTES).contains(&total) {
        return None;
    }
    let mut frame = vec![0u8; total];
    frame[..4].copy_from_slice(&len_buf);
    stream.read_exact(&mut frame[4..]).ok()?;
    let req_id = i32::from_le_bytes([frame[4], frame[5], frame[6], frame[7]]);
    let doc = parse_op_msg(&frame).ok()?;
    Some((req_id, Document::from_doc(doc)))
}

/// Writes an `OP_MSG` reply frame for `response_to` containing `doc` to `stream`.
pub fn write_reply(stream: &mut impl Write, response_to: i32, doc: &Document) -> io::Result<()> {
    let body = encode_doc(&doc.to_doc());
    let total_len = i32::try_from(16 + 4 + 1 + body.len()).unwrap_or(i32::MAX);
    let mut packet = Vec::with_capacity(21 + body.len());
    packet.extend_from_slice(&total_len.to_le_bytes());
    packet.extend_from_slice(&1_i32.to_le_bytes());
    packet.extend_from_slice(&response_to.to_le_bytes());
    packet.extend_from_slice(&OP_MSG_CODE.to_le_bytes());
    packet.extend_from_slice(&0_u32.to_le_bytes());
    packet.push(0);
    packet.extend_from_slice(&body);
    stream.write_all(&packet)?;
    stream.flush()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use std::net::TcpListener;

    #[test]
    fn encodes_and_decodes_bson_documents_and_op_msg_frames() {
        let doc: Doc = vec![
            ("hello".into(), Value::Int32(1)),
            ("$db".into(), Value::Str("admin".into())),
            ("ok".into(), Value::Double(1.0)),
            ("primary".into(), Value::Bool(true)),
            ("count".into(), Value::Int64(42)),
            ("nothing".into(), Value::Null),
            (
                "nested".into(),
                Value::Doc(vec![("setName".into(), Value::Str("rs0".into()))]),
            ),
            (
                "members".into(),
                Value::Array(vec![
                    Value::Doc(vec![
                        ("_id".into(), Value::Int32(0)),
                        ("host".into(), Value::Str("127.0.0.1:27017".into())),
                    ]),
                ]),
            ),
        ];

        let encoded = encode_doc(&doc);
        let decoded = decode_doc(&encoded).unwrap();
        assert_eq!(decoded, doc);

        assert_eq!(as_str(find(&decoded, "$db")), Some("admin"));
        assert_eq!(as_f64(get(&decoded, "ok")), Some(1.0));
        assert_eq!(as_f64(find(&decoded, "hello")), Some(1.0));
        assert_eq!(as_bool(find(&decoded, "primary")), Some(true));

        let frame = op_msg(&doc);
        let round_trip = parse_op_msg(&frame).unwrap();
        assert_eq!(round_trip, doc);
    }

    #[test]
    fn exchanges_op_msg_over_tcp_socket() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();

        let server = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut len_buf = [0u8; 4];
            sock.read_exact(&mut len_buf).unwrap();
            let total = i32::from_le_bytes(len_buf) as usize;
            let mut frame = vec![0u8; total];
            frame[..4].copy_from_slice(&len_buf);
            sock.read_exact(&mut frame[4..]).unwrap();

            let req = parse_op_msg(&frame).unwrap();
            assert_eq!(as_f64(find(&req, "hello")), Some(1.0));

            let reply: Doc = vec![
                ("ok".into(), Value::Double(1.0)),
                ("setName".into(), Value::Str("rs0".into())),
                ("isWritablePrimary".into(), Value::Bool(true)),
            ];
            sock.write_all(&op_msg(&reply)).unwrap();
        });

        let req: Doc = vec![
            ("hello".into(), Value::Int32(1)),
            ("$db".into(), Value::Str("admin".into())),
        ];
        let res = command(addr, &req, Duration::from_secs(2)).unwrap();
        assert_eq!(as_str(find(&res, "setName")), Some("rs0"));
        assert_eq!(as_bool(find(&res, "isWritablePrimary")), Some(true));
        server.join().unwrap();
    }
}
