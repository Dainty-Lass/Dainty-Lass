// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! What the access log may show of a target or a user agent: no token, account key or long secret-looking run,
//! however it was encoded.

use std::sync::LazyLock;

use dainty_service_kit::js;
use regex::Regex;

fn pattern(text: &str) -> Regex {
    Regex::new(text).unwrap_or_else(|_| unreachable!("a fixed pattern"))
}

static JWT: LazyLock<Regex> = LazyLock::new(|| pattern(r"eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]*"));
static ACCOUNT_KEY: LazyLock<Regex> = LazyLock::new(|| pattern(r"RRK_[A-Za-z0-9]+"));
static HEX_RUN: LazyLock<Regex> = LazyLock::new(|| pattern(r"[0-9A-Fa-f]{40,}"));
static WORD_RUN: LazyLock<Regex> = LazyLock::new(|| pattern(r"[A-Za-z0-9_-]{64,}"));

const SECRET_NAMES: [&str; 10] = [
    "key", "token", "secret", "pass", "auth", "code", "session", "jwt", "sig", "cred",
];

const MAX_TARGET: usize = 400;
const MAX_USER_AGENT: usize = 160;

/// Tokens, account keys and long hex or word runs replaced.
pub fn redact_text(text: &str) -> String {
    let text = JWT.replace_all(text, "<token>");
    let text = ACCOUNT_KEY.replace_all(&text, "<key>");
    let text = HEX_RUN.replace_all(&text, "<redacted>");
    WORD_RUN.replace_all(&text, "<redacted>").into_owned()
}

/// Redacted as decoded when decoding exposes something to redact (a token split by `%2E`), else as written.
fn redact_encoded(raw: &str, plus_is_space: bool) -> String {
    let spaced;
    let source = if plus_is_space {
        spaced = raw.replace('+', " ");
        spaced.as_str()
    } else {
        raw
    };
    if let Some(decoded) = js::decode_uri_component(source)
        && decoded != raw
    {
        let redacted = redact_text(&decoded);
        if redacted != decoded {
            return redacted;
        }
    }
    redact_text(raw)
}

fn controls_to_question(text: &str) -> String {
    text.chars()
        .map(|c| if c <= '\u{1f}' || c == '\u{7f}' { '?' } else { c })
        .collect()
}

/// At most `max` UTF-16 units of whole characters, then `...`.
fn clip_utf16(text: String, max: usize) -> String {
    if text.encode_utf16().count() <= max {
        return text;
    }
    let mut units = 0;
    let mut out = String::new();
    for c in text.chars() {
        units += c.len_utf16();
        if units > max {
            break;
        }
        out.push(c);
    }
    out.push_str("...");
    out
}

/// The access log's target.
pub fn redact_target(raw: &str) -> String {
    let (path, query) = match raw.split_once('?') {
        Some((p, q)) => (p, Some(q)),
        None => (raw, None),
    };
    let mut out = redact_encoded(path, false);
    if let Some(query) = query {
        let parts: Vec<String> = query
            .split('&')
            .map(|part| match part.split_once('=') {
                None => redact_encoded(part, true),
                Some((name, value)) => {
                    let decoded = js::decode_uri_component(&name.replace('+', " ")).unwrap_or_else(|| name.to_owned());
                    let lower = decoded.to_lowercase();
                    let shown_name = redact_encoded(name, true);
                    if SECRET_NAMES.iter().any(|s| lower.contains(s)) {
                        format!("{shown_name}=<redacted>")
                    } else {
                        format!("{shown_name}={}", redact_encoded(value, true))
                    }
                }
            })
            .collect();
        out.push('?');
        out.push_str(&parts.join("&"));
    }
    clip_utf16(controls_to_question(&out), MAX_TARGET)
}

/// The access log's `ua`: the bytes as Latin-1, redacted, clipped.
pub fn redact_user_agent(bytes: &[u8]) -> String {
    let text: String = bytes.iter().map(|&b| char::from(b)).collect();
    clip_utf16(controls_to_question(&redact_text(&text)), MAX_USER_AGENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text() {
        assert_eq!(redact_text("eyJ eyJa. eyJa.b eyJa.b. x"), "eyJ eyJa. eyJa.b <token> x");
        assert_eq!(redact_text("RRK_ab12 RRK_"), "<key> RRK_");
        assert_eq!(redact_text(&"a".repeat(40)), "<redacted>");
        assert_eq!(redact_text(&"z".repeat(63)), "z".repeat(63));
        assert_eq!(redact_text(&"z".repeat(64)), "<redacted>");
        let path = "/progression/UID-00000000-0000-4000-8000-00000000f609";
        assert_eq!(redact_text(path), path);
    }

    #[test]
    fn targets() {
        assert_eq!(
            redact_target("/a?accountId=UID-1&key=abc&access_token=zzz&Code=1234"),
            "/a?accountId=UID-1&key=<redacted>&access_token=<redacted>&Code=<redacted>"
        );
        assert_eq!(redact_target("/a?flag&b=%zz"), "/a?flag&b=%zz");
        assert_eq!(redact_target("/a\r\nforged"), "/a??forged");
        assert_eq!(redact_target("/x/eyJa%2Eb%2Ec"), "/x/<token>");
        assert_eq!(redact_target("/x?"), "/x?");
        assert_eq!(redact_target("/k/RRK_abc"), "/k/<key>");
        let long = format!("/{}", "a/".repeat(300));
        let clipped = redact_target(&long);
        assert_eq!(clipped.len(), 403);
        assert!(clipped.ends_with("..."));
    }

    #[test]
    fn user_agent() {
        assert_eq!(redact_user_agent(b"Game/1.0\x01 RRK_x"), "Game/1.0? <key>");
        assert_eq!(redact_user_agent(&[0xe9]), "\u{e9}");
        assert!(redact_user_agent(&b"a ".repeat(100)).ends_with("..."));
    }
}
