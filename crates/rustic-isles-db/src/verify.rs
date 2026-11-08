// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `verify`'s comparison (spec §10.3): per table, the count, a SHA-256 over the canonical records and the
//! mismatching keys, the source's extraction against the target read back.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use crate::Area;
use crate::tables::{Doc, TABLES, Table, TargetState};

/// One table's comparison.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionVerification {
    /// The table's name.
    pub name: String,
    /// The area the table belongs to.
    pub area: Area,
    /// Rows migrate would produce from the source.
    pub expected_count: usize,
    /// Rows in the store.
    pub actual_count: usize,
    /// SHA-256 over the expected rows' keys and content hashes, lower-case hex.
    pub expected_sha256: String,
    /// The same hash over the store's rows.
    pub actual_sha256: String,
    /// Whether the counts and hashes match and no row differs.
    pub ok: bool,
    /// Rows missing, extra or with different content, one line each.
    pub mismatches: Vec<String>,
}

/// `verify`'s result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyReport {
    /// Whether every table matched.
    pub ok: bool,
    /// Each selected table, in comparison order.
    pub collections: Vec<CollectionVerification>,
}

fn hex(bytes: &[u8]) -> String {
    ::hex::encode(bytes)
}

pub(crate) fn sha256_hex(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes()))
}

/// The record a key's hash is taken over (§10.4): the two key tables hash the key itself.
fn canonical(table: &Table, row: &Doc) -> Value {
    match table.name {
        "game_server_api_keys" | "user_api_keys" => row.get("key_hash").cloned().unwrap_or(Value::Null),
        _ => Value::Object(row.clone()),
    }
}

fn key_text(table: &Table, key: &[String]) -> String {
    match table.name {
        "launcher_notices" | "operations_policy" => "current".into(),
        // Joined tuples can collide; the later wins (P12)
        _ => key.join(":"),
    }
}

fn hashes<'a>(table: &Table, rows: impl Iterator<Item = (&'a Vec<String>, &'a Doc)>) -> BTreeMap<String, String> {
    rows.map(|(key, row)| (key_text(table, key), sha256_hex(&canonical(table, row).to_string())))
        .collect()
}

fn table_hash(entries: &BTreeMap<String, String>) -> String {
    let mut hasher = Sha256::new();
    for (key, hash) in entries {
        hasher.update(format!("{key}:{hash}\n").as_bytes());
    }
    hex(&hasher.finalize())
}

/// The migration's untouched seed row reads as no policy (§10.2).
pub(crate) fn is_seed_policy(row: &Doc) -> bool {
    row.get("id") == Some(&Value::from(1))
        && row.get("guard_enforcement").and_then(Value::as_str) == Some("ENFORCE")
        && row.get("diagnostics_profile").and_then(Value::as_str) == Some("PRODUCTION")
        && row.get("version") == Some(&Value::from(0))
        && ["updated_at", "updated_by", "diagnostics_expires_at"]
            .iter()
            .all(|k| row.get(*k).is_none_or(Value::is_null))
}

/// Every table of `areas` in §10.3's order. `all_areas`: with `--area` given, only the actual accounts the
/// expected side names are compared.
pub(crate) fn compare(
    expected: &TargetState,
    actual: &TargetState,
    areas: &BTreeSet<Area>,
    all_areas: bool,
) -> Vec<CollectionVerification> {
    let mut out = Vec::new();
    for table in TABLES.iter().filter(|t| areas.contains(&t.area)) {
        let want = hashes(table, expected.rows(table.name));
        let got_rows = actual.rows(table.name).filter(|(key, row)| match table.name {
            "accounts" if !all_areas => expected.get("accounts", key).is_some(),
            "operations_policy" => expected.count("operations_policy") > 0 || !is_seed_policy(row),
            _ => true,
        });
        let got = hashes(table, got_rows);
        let mut mismatches = Vec::new();
        for (key, hash) in &want {
            match got.get(key) {
                None => mismatches.push(format!("missing record {key}")),
                Some(other) if other != hash => {
                    mismatches.push(format!("content hash mismatch for {key}: expected {hash}, got {other}"));
                }
                Some(_) => {}
            }
        }
        for key in got.keys().filter(|k| !want.contains_key(*k)) {
            mismatches.push(format!("unexpected extra record {key}"));
        }
        let (expected_sha256, actual_sha256) = (table_hash(&want), table_hash(&got));
        out.push(CollectionVerification {
            name: table.name.into(),
            area: table.area,
            expected_count: want.len(),
            actual_count: got.len(),
            ok: want.len() == got.len() && expected_sha256 == actual_sha256 && mismatches.is_empty(),
            expected_sha256,
            actual_sha256,
            mismatches,
        });
    }
    out
}
