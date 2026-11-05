// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! BSON to JSON and back (spec §4.4, §4.5). Every length is checked against the bytes left before use, and
//! nesting stops at [`MAX_DEPTH`] (§17 item 7, a departure: restricted recursed without a bound).

use base64::Engine as _;
use serde_json::{Map, Value};

use super::MongoError;
use crate::read::iso_ms;

/// Above MongoDB's own nesting limit (100), so nothing a real server returns is refused.
pub const MAX_DEPTH: usize = 200;

/// One BSON document, decoded to a JSON object.
pub fn decode_document(bytes: &[u8]) -> Result<Map<String, Value>, MongoError> {
    decode(bytes, 1).map_err(MongoError::Bson)
}

/// A JSON object as one BSON document. A key holding NUL can't be written as a C string, so it is refused.
pub fn encode_document(document: &Map<String, Value>) -> Result<Vec<u8>, MongoError> {
    let mut out = Vec::new();
    write_json_doc(&mut out, document)?;
    Ok(out)
}

fn decode(bytes: &[u8], depth: usize) -> Result<Map<String, Value>, String> {
    if depth > MAX_DEPTH {
        return Err(format!("BSON nesting deeper than {MAX_DEPTH}"));
    }
    if bytes.len() < 5 {
        return Err(format!("BSON document too short: {} bytes", bytes.len()));
    }
    let declared = i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let len = match usize::try_from(declared) {
        Ok(n) if n >= 5 && n <= bytes.len() => n,
        _ => {
            return Err(format!(
                "invalid BSON document length {declared} (slice len {})",
                bytes.len()
            ));
        }
    };
    if bytes[len - 1] != 0 {
        return Err("BSON document missing trailing NUL".into());
    }
    let body = &bytes[..len - 1];
    let mut cursor = Cursor { body, pos: 4 };
    let mut out = Map::new();
    while cursor.pos < body.len() {
        let tag = body[cursor.pos];
        cursor.pos += 1;
        let key = cursor.cstring("key")?;
        let value = cursor.value(tag, depth)?;
        out.insert(key, value);
    }
    Ok(out)
}

struct Cursor<'a> {
    body: &'a [u8],
    pos: usize,
}

impl Cursor<'_> {
    fn take(&mut self, n: usize, what: &str) -> Result<&[u8], String> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|end| *end <= self.body.len())
            .ok_or_else(|| format!("truncated BSON {what}"))?;
        let out = &self.body[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    fn i32(&mut self, what: &str) -> Result<i32, String> {
        let b = self.take(4, what)?;
        Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn i64(&mut self, what: &str) -> Result<i64, String> {
        let b = self.take(8, what)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(i64::from_le_bytes(a))
    }

    fn cstring(&mut self, what: &str) -> Result<String, String> {
        let rest = &self.body[self.pos..];
        let end = rest
            .iter()
            .position(|b| *b == 0)
            .ok_or_else(|| "unterminated C string in BSON".to_owned())?;
        let text = std::str::from_utf8(&rest[..end]).map_err(|e| match what {
            "key" => format!("invalid UTF-8 in BSON key: {e}"),
            _ => format!("invalid UTF-8 in BSON string: {e}"),
        })?;
        self.pos += end + 1;
        Ok(text.to_owned())
    }

    fn string(&mut self) -> Result<String, String> {
        let declared = self.i32("string length")?;
        let left = self.body.len() - self.pos;
        let len = match usize::try_from(declared) {
            Ok(n) if n >= 1 && n <= left => n,
            _ => return Err(format!("invalid BSON string length {declared}")),
        };
        let bytes = self.take(len, "string")?;
        let (text, nul) = bytes.split_at(len - 1);
        if nul != [0] {
            return Err("BSON string missing trailing NUL".into());
        }
        std::str::from_utf8(text)
            .map(str::to_owned)
            .map_err(|e| format!("invalid UTF-8 in BSON string: {e}"))
    }

    fn nested(&mut self, depth: usize, what: &str) -> Result<Map<String, Value>, String> {
        let left = self.body.len() - self.pos;
        if left < 4 {
            return Err(format!("truncated BSON {what}"));
        }
        let b = &self.body[self.pos..self.pos + 4];
        let declared = i32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        let len = match usize::try_from(declared) {
            Ok(n) if n >= 5 && n <= left => n,
            _ => return Err(format!("invalid BSON {what} length {declared}")),
        };
        let slice = &self.body[self.pos..self.pos + len];
        self.pos += len;
        decode(slice, depth + 1)
    }

    fn value(&mut self, tag: u8, depth: usize) -> Result<Value, String> {
        Ok(match tag {
            0x01 => {
                let b = self.take(8, "double")?;
                let mut a = [0u8; 8];
                a.copy_from_slice(b);
                Value::from(f64::from_le_bytes(a))
            }
            0x02 | 0x0D | 0x0E => Value::String(self.string()?),
            0x03 => Value::Object(self.nested(depth, "subdocument")?),
            // Elements are keyed: a repeated index keeps its first position and its last value
            0x04 => Value::Array(self.nested(depth, "array")?.into_iter().map(|(_, v)| v).collect()),
            0x05 => {
                let declared = self.i32("binary")?;
                self.take(1, "binary")?;
                let len = usize::try_from(declared).map_err(|_| "truncated BSON binary payload".to_owned())?;
                let payload = self.take(len, "binary payload")?;
                Value::String(base64::engine::general_purpose::STANDARD.encode(payload))
            }
            0x06 | 0x0A | 0x7F | 0xFF => Value::Null,
            0x07 => Value::String(hex(self.take(12, "ObjectId")?)),
            0x08 => Value::Bool(self.take(1, "bool")?[0] != 0),
            0x09 => Value::String(iso_ms(self.i64("datetime")?)),
            0x0B => {
                let pattern = self.cstring("regex")?;
                let options = self.cstring("regex")?;
                Value::String(format!("/{pattern}/{options}"))
            }
            0x10 => Value::from(self.i32("int32")?),
            0x11 | 0x12 => Value::from(self.i64("int64")?),
            0x13 => Value::String(hex(self.take(16, "decimal128")?)),
            other => return Err(format!("unsupported BSON element tag 0x{other:02x}")),
        })
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A command or reply value: JSON, plus the BSON types JSON has no word for. The reading commands use only the
/// scalars; documents and arrays are the writer's.
#[cfg_attr(not(feature = "export-mongo"), allow(dead_code))]
pub(crate) enum B<'a> {
    Str(&'a str),
    I32(i32),
    I64(i64),
    Bool(bool),
    Bin(&'a [u8]),
    Json(&'a Value),
    Doc(Vec<(&'a str, B<'a>)>),
    Arr(Vec<B<'a>>),
}

pub(crate) fn encode_command(fields: &[(&str, B<'_>)]) -> Result<Vec<u8>, MongoError> {
    let mut out = Vec::new();
    write_doc(&mut out, fields)?;
    Ok(out)
}

fn nul_key() -> MongoError {
    MongoError::Command("BSON key contains NUL".into())
}

fn begin(out: &mut Vec<u8>) -> usize {
    let start = out.len();
    out.extend_from_slice(&[0; 4]);
    start
}

fn finish(out: &mut Vec<u8>, start: usize) -> Result<(), MongoError> {
    out.push(0);
    let len = i32::try_from(out.len() - start).map_err(|_| MongoError::Command("BSON document too large".into()))?;
    out[start..start + 4].copy_from_slice(&len.to_le_bytes());
    Ok(())
}

fn key(out: &mut Vec<u8>, tag: u8, name: &str) -> Result<(), MongoError> {
    if name.as_bytes().contains(&0) {
        return Err(nul_key());
    }
    out.push(tag);
    out.extend_from_slice(name.as_bytes());
    out.push(0);
    Ok(())
}

fn write_doc(out: &mut Vec<u8>, fields: &[(&str, B<'_>)]) -> Result<(), MongoError> {
    let start = begin(out);
    for (name, value) in fields {
        write_value(out, name, value)?;
    }
    finish(out, start)
}

fn write_value(out: &mut Vec<u8>, name: &str, value: &B<'_>) -> Result<(), MongoError> {
    match value {
        B::Str(s) => {
            key(out, 0x02, name)?;
            write_str(out, s)?;
        }
        B::I32(n) => {
            key(out, 0x10, name)?;
            out.extend_from_slice(&n.to_le_bytes());
        }
        B::I64(n) => {
            key(out, 0x12, name)?;
            out.extend_from_slice(&n.to_le_bytes());
        }
        B::Bool(b) => {
            key(out, 0x08, name)?;
            out.push(u8::from(*b));
        }
        B::Bin(bytes) => {
            key(out, 0x05, name)?;
            let len = i32::try_from(bytes.len()).map_err(|_| MongoError::Command("binary too large".into()))?;
            out.extend_from_slice(&len.to_le_bytes());
            out.push(0);
            out.extend_from_slice(bytes);
        }
        B::Json(v) => write_json(out, name, v)?,
        B::Doc(fields) => {
            key(out, 0x03, name)?;
            write_doc(out, fields)?;
        }
        B::Arr(items) => {
            key(out, 0x04, name)?;
            let start = begin(out);
            for (i, item) in items.iter().enumerate() {
                write_value(out, &i.to_string(), item)?;
            }
            finish(out, start)?;
        }
    }
    Ok(())
}

fn write_str(out: &mut Vec<u8>, s: &str) -> Result<(), MongoError> {
    let len = i32::try_from(s.len() + 1).map_err(|_| MongoError::Command("string too large".into()))?;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(s.as_bytes());
    out.push(0);
    Ok(())
}

fn write_json_doc(out: &mut Vec<u8>, map: &Map<String, Value>) -> Result<(), MongoError> {
    let start = begin(out);
    for (name, value) in map {
        write_json(out, name, value)?;
    }
    finish(out, start)
}

fn write_json(out: &mut Vec<u8>, name: &str, value: &Value) -> Result<(), MongoError> {
    match value {
        Value::Null => key(out, 0x0A, name)?,
        Value::Bool(b) => {
            key(out, 0x08, name)?;
            out.push(u8::from(*b));
        }
        Value::Number(n) => match n.as_i64() {
            Some(i) => match i32::try_from(i) {
                Ok(small) => {
                    key(out, 0x10, name)?;
                    out.extend_from_slice(&small.to_le_bytes());
                }
                Err(_) => {
                    key(out, 0x12, name)?;
                    out.extend_from_slice(&i.to_le_bytes());
                }
            },
            None => {
                key(out, 0x01, name)?;
                out.extend_from_slice(&n.as_f64().unwrap_or(f64::NAN).to_le_bytes());
            }
        },
        Value::String(s) => {
            key(out, 0x02, name)?;
            write_str(out, s)?;
        }
        Value::Array(items) => {
            key(out, 0x04, name)?;
            let start = begin(out);
            for (i, item) in items.iter().enumerate() {
                write_json(out, &i.to_string(), item)?;
            }
            finish(out, start)?;
        }
        Value::Object(map) => {
            key(out, 0x03, name)?;
            write_json_doc(out, map)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn scalars_round_trip() {
        let doc = obj(json!({"s": "text", "i": 42, "l": 9_000_000_000_i64, "d": 3.5, "b": true, "n": null}));
        let back = decode_document(&encode_document(&doc).unwrap()).unwrap();
        assert_eq!(back, doc);
        // int32 for 42, int64 for 9e9, double for 3.5 and for an integral f64
        let tags: Vec<u8> = [
            json!("t"),
            json!(42),
            json!(9_000_000_000_i64),
            json!(3.5),
            json!(2.0),
            json!(true),
        ]
        .into_iter()
        .map(|v| encode_document(&obj(json!({ "k": v }))).unwrap()[4])
        .collect();
        assert_eq!(tags, vec![0x02, 0x10, 0x12, 0x01, 0x01, 0x08]);
    }

    #[test]
    fn truncated_and_overlong_inputs_are_errors() {
        let bytes = encode_document(&obj(json!({"a": "hello", "b": {"c": [1, 2]}}))).unwrap();
        for n in 0..bytes.len() {
            assert!(decode_document(&bytes[..n]).is_err(), "prefix {n}");
        }
        let mut long = bytes.clone();
        long[0] = 0xFF;
        assert!(
            decode_document(&long)
                .unwrap_err()
                .to_string()
                .contains("invalid BSON document length")
        );
        assert_eq!(
            decode_document(&[1, 2]).unwrap_err().to_string(),
            "BSON decode error: BSON document too short: 2 bytes"
        );
        let mut neg = bytes;
        neg[0..4].copy_from_slice(&(-1i32).to_le_bytes());
        assert!(decode_document(&neg).is_err());
    }

    fn doc_with(tag: u8, payload: &[u8]) -> Vec<u8> {
        let mut body = vec![tag, b'x', 0];
        body.extend_from_slice(payload);
        let len = i32::try_from(body.len() + 5).unwrap();
        let mut out = len.to_le_bytes().to_vec();
        out.extend(body);
        out.push(0);
        out
    }

    #[test]
    fn special_types_decode_to_text() {
        let ms: i64 = 1_772_359_200_000;
        let d = decode_document(&doc_with(0x09, &ms.to_le_bytes())).unwrap();
        assert_eq!(d["x"], "2026-03-01T10:00:00.000Z");
        let oid: Vec<u8> = (1..=12).collect();
        let d = decode_document(&doc_with(0x07, &oid)).unwrap();
        assert_eq!(d["x"], "0102030405060708090a0b0c");
        let d = decode_document(&doc_with(0x0B, b"a.b\0i\0")).unwrap();
        assert_eq!(d["x"], "/a.b/i");
        let d = decode_document(&doc_with(0x05, &[3, 0, 0, 0, 0, b'a', b'b', b'c'])).unwrap();
        assert_eq!(d["x"], "YWJj");
        let d = decode_document(&doc_with(0x01, &f64::NAN.to_le_bytes())).unwrap();
        assert_eq!(d["x"], Value::Null);
        let d = decode_document(&doc_with(0x08, &[7])).unwrap();
        assert_eq!(d["x"], true);
        let err = decode_document(&doc_with(0x0C, &[])).unwrap_err();
        assert_eq!(err.to_string(), "BSON decode error: unsupported BSON element tag 0x0c");
    }

    #[test]
    fn duplicate_keys_keep_first_position_and_last_value() {
        let mut body = Vec::new();
        for (k, v) in [(b'a', 1i32), (b'b', 2), (b'a', 3)] {
            body.extend_from_slice(&[0x10, k, 0]);
            body.extend_from_slice(&v.to_le_bytes());
        }
        let len = i32::try_from(body.len() + 5).unwrap();
        let mut bytes = len.to_le_bytes().to_vec();
        bytes.extend(body);
        bytes.push(0);
        let d = decode_document(&bytes).unwrap();
        assert_eq!(Value::Object(d).to_string(), r#"{"a":3,"b":2}"#);
    }

    #[test]
    fn deep_nesting_is_an_error_not_a_crash() {
        // 10 000 levels of {"a": {...}} built inside out
        let mut inner = vec![5, 0, 0, 0, 0];
        for _ in 0..10_000 {
            let mut body = vec![0x03, b'a', 0];
            body.extend_from_slice(&inner);
            let len = i32::try_from(body.len() + 5).unwrap();
            let mut next = len.to_le_bytes().to_vec();
            next.extend(body);
            next.push(0);
            inner = next;
        }
        let err = decode_document(&inner).unwrap_err();
        assert_eq!(err.to_string(), "BSON decode error: BSON nesting deeper than 200");
    }

    #[test]
    fn nul_in_a_key_is_refused() {
        let doc = obj(json!({"a\u{0}b": 1}));
        assert!(encode_document(&doc).is_err());
    }
}
