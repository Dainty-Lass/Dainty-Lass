// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Comparison (spec §8), the gateway redaction alignment (§7.8) and chat stanzas (§9.4).

use std::collections::BTreeSet;
use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;
use serde_json::Value;

use crate::normalize::{ISO, IdNormalizer, normalize_headers};
use crate::pattern;
use crate::request::ReplayHttpResponse;

/// One difference (§8.1). Serialized with its variant name as `kind`, first.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind")]
pub enum DiffKind {
    StatusDiff {
        left: u16,
        right: u16,
    },
    HeaderDiff {
        header: String,
        left: Option<String>,
        right: Option<String>,
    },
    ValueDiff {
        path: String,
        left: Value,
        right: Value,
    },
    TextDiff {
        left: String,
        right: String,
    },
    ChatStanzaDiff {
        conn: u64,
        index: usize,
        left: Option<String>,
        right: Option<String>,
    },
}

const REDACTED: &str = "<redacted>";
// The gateway's secret-name list
const SECRET_NAMES: [&str; 11] = [
    "key", "token", "secret", "pass", "auth", "jwt", "sig", "cred", "recovery", "invite", "cookie",
];
static HEX_RUN: LazyLock<Regex> = LazyLock::new(|| pattern(r"[0-9A-Fa-f]{40,}"));
static LONG_RUN: LazyLock<Regex> = LazyLock::new(|| pattern(r"[A-Za-z0-9_+/=-]{100,}"));

fn is_secret(value: &Value, hint: Option<&str>) -> bool {
    if let Some(hint) = hint {
        let hint = hint.to_ascii_lowercase();
        if SECRET_NAMES.iter().any(|n| hint.contains(n)) {
            return true;
        }
    }
    value
        .as_str()
        .is_some_and(|s| HEX_RUN.is_match(s) || LONG_RUN.is_match(s))
}

fn is_redacted(value: &Value) -> bool {
    value.as_str() == Some(REDACTED)
}

/// Where the gateway removed a secret from one side for good, the other side's secret is removed too (§7.8).
fn align_redacted(left: &mut Value, right: &mut Value, hint: Option<&str>) {
    match (is_redacted(left), is_redacted(right)) {
        (true, false) if is_secret(right, hint) => {
            *right = Value::String(REDACTED.into());
            return;
        }
        (false, true) if is_secret(left, hint) => {
            *left = Value::String(REDACTED.into());
            return;
        }
        _ => {}
    }
    match (left, right) {
        (Value::Object(l), Value::Object(r)) => {
            for (key, lv) in l.iter_mut() {
                if let Some(rv) = r.get_mut(key) {
                    align_redacted(lv, rv, Some(key));
                }
            }
        }
        (Value::Array(l), Value::Array(r)) => {
            for (lv, rv) in l.iter_mut().zip(r.iter_mut()) {
                align_redacted(lv, rv, hint);
            }
        }
        _ => {}
    }
}

fn compare_values(path: &str, left: &Value, right: &Value, diffs: &mut Vec<DiffKind>) {
    match (left, right) {
        (Value::Number(a), Value::Number(b)) if a == b || a.as_f64() == b.as_f64() => {}
        (Value::Array(a), Value::Array(b)) => {
            for i in 0..a.len().max(b.len()) {
                let l = a.get(i).unwrap_or(&Value::Null);
                let r = b.get(i).unwrap_or(&Value::Null);
                compare_values(&format!("{path}[{i}]"), l, r, diffs);
            }
        }
        (Value::Object(a), Value::Object(b)) => {
            let keys: BTreeSet<&String> = a.keys().chain(b.keys()).collect();
            for key in keys {
                let l = a.get(key).unwrap_or(&Value::Null);
                let r = b.get(key).unwrap_or(&Value::Null);
                compare_values(&format!("{path}.{key}"), l, r, diffs);
            }
        }
        (a, b) if a == b => {}
        (a, b) => diffs.push(DiffKind::ValueDiff {
            path: path.to_owned(),
            left: a.clone(),
            right: b.clone(),
        }),
    }
}

fn compare_bodies(
    left: &str,
    right: &str,
    left_norm: &mut IdNormalizer,
    right_norm: &mut IdNormalizer,
    diffs: &mut Vec<DiffKind>,
) {
    let parsed = (
        serde_json::from_str::<Value>(left),
        serde_json::from_str::<Value>(right),
    );
    let (Ok(left_raw), Ok(right_raw)) = parsed else {
        let l = left_norm.normalize_raw_text(left);
        let r = right_norm.normalize_raw_text(right);
        if l != r {
            diffs.push(DiffKind::TextDiff { left: l, right: r });
        }
        return;
    };
    let mut l = left_norm.normalize_value(&left_raw, None);
    let mut r = right_norm.normalize_value(&right_raw, None);
    align_redacted(&mut l, &mut r, None);
    let before = diffs.len();
    compare_values("$", &l, &r, diffs);
    if diffs.len() > before || !left_norm.config().check_raw_json_text {
        return;
    }
    let stringify = service_kit::jsjson::stringify;
    let (l_text, r_text) = (stringify(&l), stringify(&r));
    // Whitespace, number spelling or escapes the parse hid
    let l_odd = left != stringify(&left_raw);
    let r_odd = right != stringify(&right_raw);
    if l_text != r_text || l_odd != r_odd {
        diffs.push(DiffKind::TextDiff {
            left: if l_odd { left.to_owned() } else { l_text },
            right: if r_odd { right.to_owned() } else { r_text },
        });
    }
}

/// The differences of one HTTP pair: status, headers in name order, then the body (§8.2). Left is normalized
/// before right, headers before bodies, which fixes the numbering.
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
    let lh = normalize_headers(&left.headers, left_norm);
    let rh = normalize_headers(&right.headers, right_norm);
    let names: BTreeSet<&String> = lh.keys().chain(rh.keys()).collect();
    for name in names {
        let (l, r) = (lh.get(name), rh.get(name));
        if l != r {
            diffs.push(DiffKind::HeaderDiff {
                header: name.clone(),
                left: l.cloned(),
                right: r.cloned(),
            });
        }
    }
    compare_bodies(&left.body_text, &right.body_text, left_norm, right_norm, &mut diffs);
    diffs
}

/// Empty stanzas, keepalive pings and test barriers are not compared.
pub fn is_filtered_chat_stanza(stanza: &str) -> bool {
    stanza.trim().is_empty()
        || (stanza.contains("urn:xmpp:ping") && stanza.contains("<ping"))
        || stanza.contains("replay-barrier-")
}

static STREAM_ID_SINGLE: LazyLock<Regex> = LazyLock::new(|| pattern(r"(<stream:stream\b[^>]*?\sid=')[^']*'"));
static STREAM_ID_DOUBLE: LazyLock<Regex> = LazyLock::new(|| pattern(r#"(<stream:stream\b[^>]*?\sid=")[^"]*""#));
static SRV_ID: LazyLock<Regex> = LazyLock::new(|| pattern(r#"id=(['"])srv-[0-9A-Fa-f-]+(['"])"#));

/// Trimmed; the stream id, server-made `srv-` ids and every ISO time (no sentinels here, §15 item 19) masked.
pub fn normalize_chat_stanza(stanza: &str) -> String {
    let out = stanza.trim();
    let out = STREAM_ID_SINGLE.replace_all(out, "${1}<stream>'");
    let out = STREAM_ID_DOUBLE.replace_all(&out, "${1}<stream>\"");
    let out = SRV_ID.replace_all(&out, |c: &regex::Captures| {
        if c[1] == c[2] {
            format!("id={}srv-<id>{}", &c[1], &c[2])
        } else {
            c[0].to_owned()
        }
    });
    ISO.replace_all(&out, "<date>").into_owned()
}

/// Index by index over the longer filtered list.
pub fn compare_chat_stanzas(conn: u64, left: &[String], right: &[String]) -> Vec<DiffKind> {
    let keep = |list: &[String]| -> Vec<String> {
        list.iter()
            .filter(|s| !is_filtered_chat_stanza(s))
            .map(|s| normalize_chat_stanza(s))
            .collect()
    };
    let (l, r) = (keep(left), keep(right));
    (0..l.len().max(r.len()))
        .filter_map(|index| {
            let (a, b) = (l.get(index), r.get(index));
            (a != b).then(|| DiffKind::ChatStanzaDiff {
                conn,
                index,
                left: a.cloned(),
                right: b.cloned(),
            })
        })
        .collect()
}
