// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! A change-log entry as one line of JSON: what DH4 ships off the server, and what a restore reads back
//! from storage it doesn't control. Decoding refuses anything the log couldn't have written (DX2's
//! `history_record` fuzz target); whether the entries chain is [`super::Chain`]'s to say.
//!
//! `row_key`, `before` and `after` stay the text the log stored, as JSON strings, since the hash covers
//! that text byte for byte.

use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use super::{Actor, ChangeEntry, chain::is_hash};

/// The longest record read back: a row with its blobs, twice, as hex.
pub const MAX_RECORD: usize = 64 << 20;

/// The operations the log writes.
pub const OPS: [&str; 6] = ["insert", "update", "delete", "soft_delete", "restore", "migrate"];

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RecordError {
    #[error("a change-log record over {MAX_RECORD} bytes")]
    TooLong,
    #[error("not a change-log record: {0}")]
    Json(String),
    #[error("not a change-log record: {0}")]
    Invalid(&'static str),
    #[error("entry {0} has no hash yet, so it can't be shipped")]
    Unsealed(i64),
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record<'a> {
    id: i64,
    at: i64,
    #[serde(borrow)]
    actor: Cow<'a, str>,
    #[serde(borrow)]
    table_name: Cow<'a, str>,
    #[serde(borrow)]
    row_key: Cow<'a, str>,
    #[serde(borrow)]
    op: Cow<'a, str>,
    #[serde(borrow)]
    before: Option<Cow<'a, str>>,
    #[serde(borrow)]
    after: Option<Cow<'a, str>>,
    #[serde(borrow)]
    prev_hash: Cow<'a, str>,
    #[serde(borrow)]
    hash: Cow<'a, str>,
}

/// One line, without its newline: JSON escapes every control character in a string.
pub fn encode(entry: &ChangeEntry) -> Result<String, RecordError> {
    let (Some(prev_hash), Some(hash)) = (&entry.prev_hash, &entry.hash) else {
        return Err(RecordError::Unsealed(entry.id));
    };
    let record = Record {
        id: entry.id,
        at: entry.at,
        actor: entry.actor.as_str().into(),
        table_name: entry.table_name.as_str().into(),
        row_key: entry.row_key.as_str().into(),
        op: entry.op.as_str().into(),
        before: entry.before.as_deref().map(Into::into),
        after: entry.after.as_deref().map(Into::into),
        prev_hash: prev_hash.as_str().into(),
        hash: hash.as_str().into(),
    };
    serde_json::to_string(&record).map_err(|e| RecordError::Json(e.to_string()))
}

fn is_object(text: &str) -> bool {
    serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(text).is_ok()
}

/// One record, as [`encode`] writes it.
pub fn decode(bytes: &[u8]) -> Result<ChangeEntry, RecordError> {
    if bytes.len() > MAX_RECORD {
        return Err(RecordError::TooLong);
    }
    let record: Record<'_> = serde_json::from_slice(bytes).map_err(|e| RecordError::Json(e.to_string()))?;
    let invalid = |why| Err(RecordError::Invalid(why));
    if record.id < 1 {
        return invalid("an id below 1");
    }
    if Actor::parse(&record.actor).is_none() {
        return invalid("an actor the log doesn't write");
    }
    if record.table_name.is_empty() || record.table_name.chars().any(char::is_control) {
        return invalid("a table name the log doesn't write");
    }
    // insert and migrate have only an after, delete only a before, the rest both.
    let shape = match &*record.op {
        "insert" | "migrate" => (false, true),
        "delete" => (true, false),
        op if OPS.contains(&op) => (true, true),
        _ => return invalid("an operation the log doesn't write"),
    };
    if (record.before.is_some(), record.after.is_some()) != shape {
        return invalid("a before or after its operation doesn't have");
    }
    if !is_object(&record.row_key)
        || ![&record.before, &record.after]
            .iter()
            .all(|r| r.as_deref().is_none_or(is_object))
    {
        return invalid("a row that isn't a JSON object");
    }
    if !is_hash(&record.prev_hash) || !is_hash(&record.hash) {
        return invalid("a hash that isn't 64 lower-case hex digits");
    }
    Ok(ChangeEntry {
        id: record.id,
        at: record.at,
        actor: record.actor.into_owned(),
        table_name: record.table_name.into_owned(),
        row_key: record.row_key.into_owned(),
        op: record.op.into_owned(),
        before: record.before.map(Cow::into_owned),
        after: record.after.map(Cow::into_owned),
        prev_hash: Some(record.prev_hash.into_owned()),
        hash: Some(record.hash.into_owned()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::{Chain, GENESIS, entry_hash};

    fn sealed() -> ChangeEntry {
        let mut entry = ChangeEntry {
            id: 3,
            at: 1_700_000_000_000,
            actor: "admin:owner".into(),
            table_name: "accounts".into(),
            row_key: r#"{"id":1}"#.into(),
            op: "update".into(),
            before: Some(r#"{"id":1,"name":"it's \"quoted\"\nünïcode"}"#.into()),
            after: Some(r#"{"id":1,"name":"alexa"}"#.into()),
            prev_hash: Some(GENESIS.into()),
            hash: None,
        };
        entry.hash = Some(entry_hash(GENESIS, &entry));
        entry
    }

    #[test]
    fn a_record_reads_back_as_the_entry_it_was() {
        let entry = sealed();
        let line = encode(&entry).unwrap();
        assert!(!line.contains('\n'));
        let back = decode(line.as_bytes()).unwrap();
        assert_eq!(back, entry);
        Chain::genesis().check(&back).unwrap();

        let mut unsealed = entry;
        unsealed.hash = None;
        assert_eq!(encode(&unsealed), Err(RecordError::Unsealed(3)));
    }

    #[test]
    fn anything_the_log_could_not_have_written_is_refused() {
        let good: serde_json::Value = serde_json::from_str(&encode(&sealed()).unwrap()).unwrap();
        let with = |field: &str, value: serde_json::Value| {
            let mut record = good.clone();
            record[field] = value;
            decode(record.to_string().as_bytes())
        };
        use serde_json::json;
        for (field, value) in [
            ("id", json!(0)),
            ("id", json!("3")),
            ("actor", json!("robot:x")),
            ("table_name", json!("")),
            ("table_name", json!("a\u{7}")),
            ("op", json!("drop")),
            ("op", json!("insert")),
            ("before", json!(null)),
            ("row_key", json!("[1]")),
            ("after", json!("{")),
            ("prev_hash", json!("")),
            ("hash", json!("A".repeat(64))),
            ("hash", json!(null)),
            ("extra", json!(1)),
        ] {
            assert!(with(field, value.clone()).is_err(), "{field}: {value}");
        }
        let mut missing = good.clone();
        missing.as_object_mut().unwrap().remove("op");
        assert!(decode(missing.to_string().as_bytes()).is_err());
        let line = encode(&sealed()).unwrap();
        let twice = line.replacen("\"id\":3", "\"id\":3,\"id\":4", 1);
        assert!(decode(twice.as_bytes()).is_err(), "a field twice");
        assert_eq!(decode(&vec![b' '; MAX_RECORD + 1]), Err(RecordError::TooLong));
    }
}
