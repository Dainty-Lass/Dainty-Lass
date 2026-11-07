// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Normalization (spec §7): tokens, volatile values and first-seen dynamic ids rewritten to one canonical form on
//! both sides, so only real differences remain.

use std::collections::{BTreeMap, HashMap};
use std::sync::LazyLock;

use base64::Engine as _;
use base64::alphabet::URL_SAFE;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use regex::{Captures, Regex};
use serde_json::{Map, Value};

use crate::ids::{GRANT, INSTANCE, PARTY, UUID};
use crate::pattern;

/// Dropped before headers are compared (§7.7).
pub const IGNORED_RESPONSE_HEADERS: [&str; 15] = [
    "date",
    "etag",
    "x-powered-by",
    "connection",
    "keep-alive",
    "content-length",
    "transfer-encoding",
    "x-rustic-isles-request-id",
    "x-request-id",
    "server",
    "access-control-allow-origin",
    "access-control-allow-methods",
    "access-control-allow-headers",
    "access-control-allow-credentials",
    "vary",
];

/// §7.1; all on by default and not on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NormalizerConfig {
    /// Replace times with placeholders.
    pub normalize_timestamps: bool,
    /// Number generated ids in order of appearance.
    pub normalize_dynamic_ids: bool,
    /// Compare raw JSON text when the values are equal (§8.3).
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

static TOKEN_PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| pattern(r"<token:([^>]+)>"));
static JWT: LazyLock<Regex> = LazyLock::new(|| pattern(r"eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+"));
static CHALLENGE: LazyLock<Regex> = LazyLock::new(|| pattern(r"code_challenge=[A-Za-z0-9_-]{40,64}"));
pub(crate) static ISO: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"\b\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})\b"));
static LOGIN_TIME: LazyLock<Regex> = LazyLock::new(|| pattern(r"\b\d{4}\.\d{2}\.\d{2}-\d{2}\.\d{2}\.\d{2}\b"));
static DISPLAY_DATE: LazyLock<Regex> =
    LazyLock::new(|| pattern(r"\b(?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec) \d{1,2}, \d{4}\b"));
static RAW_COUNTERS: LazyLock<Regex> = LazyLock::new(|| pattern(r#""(uptimeSeconds|processId)" *: *\d+"#));

// Trailing `=` ignored, non-canonical trailing bits accepted: only the shape matters here
const LENIENT_URL: GeneralPurpose = GeneralPurpose::new(
    &URL_SAFE,
    GeneralPurposeConfig::new()
        .with_decode_padding_mode(DecodePaddingMode::Indifferent)
        .with_decode_allow_trailing_bits(true),
);

fn decode_object(part: &str) -> Option<Map<String, Value>> {
    let bytes = LENIENT_URL.decode(part.trim_end_matches('=')).ok()?;
    match serde_json::from_slice(&bytes).ok()? {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

fn non_empty<'a>(map: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    map.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// Placeholders to `<jwt:X>`/`<jwt>`, and each well-formed token to `<jwt:SUB>` or `<jwt>` (§7.2). The signature is
/// never verified; a malformed token stays, so it shows as a difference.
pub fn normalize_jwts_in_str(text: &str) -> String {
    let text = TOKEN_PLACEHOLDER.replace_all(text, "<jwt:$1>");
    let text = text.replace("<token>", "<jwt>");
    if !text.contains("eyJ") {
        return text;
    }
    JWT.replace_all(&text, |c: &Captures| {
        let token = &c[0];
        let parts: Vec<&str> = token.split('.').collect();
        let checked = (parts.len() == 3 && !parts[2].is_empty())
            .then(|| Some((decode_object(parts[0])?, decode_object(parts[1])?)))
            .flatten();
        match checked {
            Some((header, claims)) if non_empty(&header, "alg").is_some() => {
                match non_empty(&claims, "userId").or_else(|| non_empty(&claims, "sub")) {
                    Some(sub) => format!("<jwt:{sub}>"),
                    None => "<jwt>".to_owned(),
                }
            }
            _ => token.to_owned(),
        }
    })
    .into_owned()
}

#[derive(Debug, Default, Clone)]
struct Counter(HashMap<String, usize>);

impl Counter {
    fn number(&mut self, key: String) -> usize {
        let next = self.0.len() + 1;
        *self.0.entry(key).or_insert(next)
    }
}

/// One side's normalizer: its counters persist over the whole run, so co-reference is kept (§7.4).
#[derive(Debug, Clone, Default)]
pub struct IdNormalizer {
    config: NormalizerConfig,
    grants: Counter,
    parties: Counter,
    uuids: Counter,
    instances: Counter,
}

impl IdNormalizer {
    /// A normalizer with fresh counters.
    pub fn new(config: NormalizerConfig) -> Self {
        Self {
            config,
            ..Self::default()
        }
    }

    /// The switches it was built with.
    pub fn config(&self) -> NormalizerConfig {
        self.config
    }

    /// §7.3, with the enclosing object key (or header name) as `hint`.
    pub fn normalize_str(&mut self, text: &str, hint: Option<&str>) -> String {
        if text.starts_with('{')
            && text.ends_with('}')
            && let Ok(parsed) = serde_json::from_str::<Value>(text)
        {
            let normalized = self.normalize_value(&parsed, None);
            return dainty_service_kit::jsjson::stringify(&normalized);
        }
        let mut out = normalize_jwts_in_str(text);
        if matches!(hint, Some("refresh_token" | "refreshToken" | "challengeSignature")) {
            if out.chars().count() >= 12 && !out.starts_with("<jwt") {
                return "<opaque-token>".to_owned();
            }
        } else if hint == Some("nonce") && !out.is_empty() {
            return "<nonce>".to_owned();
        }
        if out.contains("code_challenge=") {
            out = CHALLENGE.replace_all(&out, "code_challenge=<challenge>").into_owned();
        }
        if self.config.normalize_timestamps {
            out = ISO
                .replace_all(&out, |c: &Captures| {
                    let found = &c[0];
                    // Game sentinels, kept
                    if found.starts_with("2085-09-09") || found.starts_with("2099-") {
                        found.to_owned()
                    } else {
                        "<date>".to_owned()
                    }
                })
                .into_owned();
            out = LOGIN_TIME.replace_all(&out, "<login-time>").into_owned();
            out = DISPLAY_DATE.replace_all(&out, "<display-date>").into_owned();
        }
        if self.config.normalize_dynamic_ids {
            out = GRANT
                .replace_all(&out, |c: &Captures| {
                    format!("gs-<uuid#{}>", self.grants.number(c[0].to_ascii_lowercase()))
                })
                .into_owned();
            out = PARTY
                .replace_all(&out, |c: &Captures| {
                    format!("<party#{}>_{}", self.parties.number(c[1].to_owned()), &c[2])
                })
                .into_owned();
            out = UUID
                .replace_all(&out, |c: &Captures| {
                    format!("<uuid#{}>", self.uuids.number(c[0].to_ascii_lowercase()))
                })
                .into_owned();
            let instance_hint = hint == Some("instanceId");
            out = INSTANCE
                .replace_all(&out, |c: &Captures| {
                    let word = &c[0];
                    if instance_hint || word.bytes().any(|b| b.is_ascii_digit()) {
                        format!("<iid#{}>", self.instances.number(word.to_owned()))
                    } else {
                        word.to_owned()
                    }
                })
                .into_owned();
        }
        out
    }

    /// §7.5: strings by §7.3 under their key, `uptimeSeconds` and `processId` numbers to 0.
    pub fn normalize_value(&mut self, value: &Value, hint: Option<&str>) -> Value {
        match value {
            Value::String(s) => Value::String(self.normalize_str(s, hint)),
            Value::Number(_) if matches!(hint, Some("uptimeSeconds" | "processId")) => Value::from(0),
            Value::Array(items) => Value::Array(items.iter().map(|v| self.normalize_value(v, hint)).collect()),
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), self.normalize_value(v, Some(k))))
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    /// §7.6: a body that is not JSON on at least one side.
    pub fn normalize_raw_text(&mut self, text: &str) -> String {
        let text = RAW_COUNTERS.replace_all(text, "\"$1\":0");
        self.normalize_str(&text, None)
    }
}

fn canonical_content_type(value: &str) -> String {
    value
        .split(';')
        .map(|p| p.trim().to_ascii_lowercase())
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("; ")
}

/// §7.7: volatile headers dropped, `content-type` canonical, every other value normalized under its name.
pub fn normalize_headers(
    headers: &BTreeMap<String, String>,
    normalizer: &mut IdNormalizer,
) -> BTreeMap<String, String> {
    let mut lowered: BTreeMap<String, String> = BTreeMap::new();
    for (name, value) in headers {
        let name = name.to_ascii_lowercase();
        if IGNORED_RESPONSE_HEADERS.contains(&name.as_str()) {
            continue;
        }
        lowered.insert(name, value.clone());
    }
    lowered
        .into_iter()
        .map(|(name, value)| {
            let value = if name == "content-type" {
                canonical_content_type(&value)
            } else {
                normalizer.normalize_str(value.trim(), Some(&name))
            };
            (name, value)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn n10_sentinels_kept() {
        let mut n = IdNormalizer::default();
        assert_eq!(
            n.normalize_str("2085-09-09T00:00:00.000Z", None),
            "2085-09-09T00:00:00.000Z"
        );
        assert_eq!(n.normalize_str("2099-01-01T00:00:00Z", None), "2099-01-01T00:00:00Z");
        assert_eq!(n.normalize_str("at 2026-10-22T17:00:00.123+02:00.", None), "at <date>.");
        assert_eq!(n.normalize_str("2026.10.22-17.00.00", None), "<login-time>");
        assert_eq!(n.normalize_str("Oct 2, 2026", None), "<display-date>");
    }

    #[test]
    fn n11_opaque_values() {
        let mut n = IdNormalizer::default();
        let v = n.normalize_value(
            &json!({"refresh_token": "abcdefghijkl", "nonce": "x", "short": "abc"}),
            None,
        );
        assert_eq!(
            v,
            json!({"refresh_token": "<opaque-token>", "nonce": "<nonce>", "short": "abc"})
        );
        let challenge = format!("a?code_challenge={}&b=1", "A".repeat(43));
        assert_eq!(n.normalize_str(&challenge, None), "a?code_challenge=<challenge>&b=1");
        assert_eq!(n.normalize_str("abcdefghijk", Some("refreshToken")), "abcdefghijk");
    }

    #[test]
    fn n12_double_encoded() {
        let mut n = IdNormalizer::default();
        let v = n.normalize_value(
            &json!({"data": "{\"id\":\"0b9e8a56-3c3f-4d0b-9a3e-1f2d3c4b5a69\",\"n\":1}"}),
            None,
        );
        assert_eq!(v, json!({"data": "{\"id\":\"<uuid#1>\",\"n\":1}"}));
    }

    #[test]
    fn numbering_by_kind() {
        let mut n = IdNormalizer::default();
        let grant = "gs-0B9E8A56-3C3F-4D0B-9A3E-1F2D3C4B5A69";
        assert_eq!(n.normalize_str(grant, None), "gs-<uuid#1>");
        assert_eq!(
            n.normalize_str("1b9e8a56-3c3f-4d0b-9a3e-1f2d3c4b5a69", None),
            "<uuid#1>"
        );
        let party = format!("local-{}_us-east-1", "a1".repeat(12));
        assert_eq!(n.normalize_str(&party, None), "<party#1>_us-east-1");
        assert_eq!(n.normalize_str("aaaa1111bbbb2222", None), "<iid#1>");
        assert_eq!(n.normalize_str("deadbeefdeadbeef", None), "deadbeefdeadbeef");
        assert_eq!(n.normalize_str("deadbeefdeadbeef", Some("instanceId")), "<iid#2>");
        assert_eq!(n.normalize_str("aaaa1111bbbb2222", None), "<iid#1>");
        assert_eq!(
            n.normalize_value(&json!({"uptimeSeconds": 5, "x": 5}), None),
            json!({"uptimeSeconds": 0, "x": 5})
        );
        assert_eq!(
            n.normalize_raw_text(r#"{"processId" : 4412, oops"#),
            r#"{"processId":0, oops"#
        );
    }

    #[test]
    fn n2_malformed_tokens_stay() {
        let bad = "eyJnot_base64_json.eyJalso_bad.sig";
        assert_eq!(normalize_jwts_in_str(bad), bad);
        assert_eq!(normalize_jwts_in_str("<token:UID-a> <token>"), "<jwt:UID-a> <jwt>");
    }

    #[test]
    fn n9_headers() {
        let mut n = IdNormalizer::default();
        let headers: BTreeMap<String, String> = [
            ("Date".to_owned(), "Mon".to_owned()),
            ("Content-Type".to_owned(), "Application/JSON ; Charset=UTF-8".to_owned()),
            (
                "X-Custom".to_owned(),
                " 0b9e8a56-3c3f-4d0b-9a3e-1f2d3c4b5a69 ".to_owned(),
            ),
        ]
        .into();
        let out = normalize_headers(&headers, &mut n);
        assert_eq!(out.len(), 2);
        assert_eq!(out["content-type"], "application/json; charset=utf-8");
        assert_eq!(out["x-custom"], "<uuid#1>");
    }
}
