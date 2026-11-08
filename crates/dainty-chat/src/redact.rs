// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Credentials taken out of a log line: JWTs, account keys and anything long enough to be a secret.

use dainty_protocol::account::ACCOUNT_KEY_PREFIX;

/// A Dauntless Revived database's account keys
const LEGACY_KEY_PREFIX: &str = "UUK_";
/// The account key's prefix before the 2026-11-06 rename
const OLD_KEY_PREFIX: &str = "RRK_";
const KEY_HEX: usize = 48;
const LONG_RUN: usize = 64;

fn is_token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// `text` with JWTs, account keys (current and legacy) and long token-like runs replaced.
pub fn redact(text: &str) -> String {
    let text = jwts(text);
    let text = keys(&text, ACCOUNT_KEY_PREFIX);
    let text = keys(&text, LEGACY_KEY_PREFIX);
    let text = keys(&text, OLD_KEY_PREFIX);
    long_runs(&text)
}

/// Length of the run of token bytes at `from`.
fn run(bytes: &[u8], from: usize) -> usize {
    bytes[from.min(bytes.len())..]
        .iter()
        .take_while(|&&b| is_token_byte(b))
        .count()
}

fn jwts(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut from = 0;
    let mut search = 0;
    while let Some(at) = text[search..].find("eyJ").map(|i| search + i) {
        let mut end = at + 3;
        let mut whole = true;
        for part in 0..3 {
            if part > 0 {
                if bytes.get(end) != Some(&b'.') {
                    whole = false;
                    break;
                }
                end += 1;
            }
            let len = run(bytes, end);
            if len == 0 {
                whole = false;
                break;
            }
            end += len;
        }
        if whole {
            out.push_str(&text[from..at]);
            out.push_str("<token>");
            from = end;
            search = end;
        } else {
            search = at + 3;
        }
    }
    out.push_str(&text[from..]);
    out
}

fn keys(text: &str, prefix: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut from = 0;
    let mut search = 0;
    while let Some(at) = text[search..].find(prefix).map(|i| search + i) {
        let start = at + prefix.len();
        let hex = bytes[start..].iter().take_while(|b| b.is_ascii_hexdigit()).count();
        if hex == KEY_HEX {
            out.push_str(&text[from..at]);
            out.push_str("<redacted>");
            from = start + KEY_HEX;
            search = from;
        } else {
            search = start;
        }
    }
    out.push_str(&text[from..]);
    out
}

fn long_runs(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut from = 0;
    let mut i = 0;
    while i < bytes.len() {
        let len = run(bytes, i);
        if len >= LONG_RUN {
            out.push_str(&text[from..i]);
            out.push_str("<redacted>");
            from = i + len;
        }
        i += len.max(1);
    }
    out.push_str(&text[from..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_node() {
        for same in [
            "eyJ.a.b",
            "eyJa..b",
            "eyJ.only",
            "eyJabc.def",
            "/character/UID-1234",
            "plain text",
        ] {
            assert_eq!(redact(same), same);
        }
        assert_eq!(redact("xeyJa.b.c.d"), "x<token>.d");
        assert_eq!(redact(&format!("eyJa.b.c{}", "z".repeat(70))), "<token>");
        assert!(redact("/account/api/oauth/sessions/kill/eyJhbGc.eyJ1c2Vy.c2ln").ends_with("/<token>"));
        assert_eq!(redact(&format!("UUK_{}", "a".repeat(48))), "<redacted>");
        let longer = format!("UUK_{}", "a".repeat(49));
        assert_eq!(redact(&longer), longer);
        assert_eq!(redact(&format!("key=DLK_{}&x=1", "b".repeat(48))), "key=<redacted>&x=1");
        assert_eq!(redact(&"x".repeat(64)), "<redacted>");
        assert_eq!(redact(&"x".repeat(63)), "x".repeat(63));
        assert_eq!(redact(&format!("a {} b", "-".repeat(80))), "a <redacted> b");
    }

    #[test]
    fn credentials_never_reach_the_log() {
        let key = format!("DLK_{}", "0123456789abcdef".repeat(3));
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJwbGF5ZXIifQ.c2lnbmF0dXJl";
        let line = format!("login key={key} token={jwt} secret={}", "Q".repeat(100));
        let out = redact(&line);
        assert!(!out.contains(&key));
        assert!(!out.contains(jwt));
        assert!(!out.contains(&"Q".repeat(64)));
        assert_eq!(out, "login key=<redacted> token=<token> secret=<redacted>");
        assert_eq!(redact("é eyJé"), "é eyJé");
    }
}
