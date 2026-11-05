//! What the database tool's MongoDB client decodes (spec-rustic-isles-db §15): arbitrary bytes as one BSON
//! document. Nothing panics or overflows the stack (nesting is capped), and a decoded document re-encodes and
//! decodes to an equal value modulo number widths.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rustic_isles_db::mongo::{decode_document, encode_document};
use serde_json::Value;

/// Numbers compared as floats: an int64 that fits re-encodes as int32, a whole double stays a double.
fn widen(v: &Value) -> Value {
    match v {
        Value::Number(n) => Value::from(n.as_f64().unwrap_or(0.0)),
        Value::Array(items) => Value::Array(items.iter().map(widen).collect()),
        Value::Object(map) => Value::Object(map.iter().map(|(k, v)| (k.clone(), widen(v))).collect()),
        other => other.clone(),
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(doc) = decode_document(data) else { return };
    // Keys are C strings on the wire, so a decoded key never holds NUL and always re-encodes
    let bytes = encode_document(&doc).expect("a decoded document re-encodes");
    let again = decode_document(&bytes).expect("an encoded document decodes");
    assert_eq!(widen(&Value::Object(again)), widen(&Value::Object(doc)));
});
