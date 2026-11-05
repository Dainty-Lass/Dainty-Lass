// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! RFC 6238 one-time codes for the admin sign-in (spec §5.5).

use aws_lc_rs::hmac;
use serde_json::Value;
use subtle::ConstantTimeEq as _;

const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// RFC 4648 base32: upper-cased, white space, `=` and `-` ignored; anything else outside the alphabet refuses.
pub fn decode_base32(secret: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut buffer: u32 = 0;
    let mut bits = 0u32;
    for c in secret.chars() {
        if c.is_whitespace() || c == '=' || c == '-' {
            continue;
        }
        let upper = c.to_ascii_uppercase();
        let value = ALPHABET.iter().position(|&a| char::from(a) == upper)?;
        buffer = (buffer << 5) | value as u32;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(out)
}

fn code_at(key: &[u8], step: i64) -> u32 {
    let key = hmac::Key::new(hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, key);
    let tag = hmac::sign(&key, &step.to_be_bytes());
    let mac = tag.as_ref();
    let offset = usize::from(mac[mac.len() - 1] & 0x0f);
    let binary = (u32::from(mac[offset] & 0x7f) << 24)
        | (u32::from(mac[offset + 1]) << 16)
        | (u32::from(mac[offset + 2]) << 8)
        | u32::from(mac[offset + 3]);
    binary % 1_000_000
}

/// Six ASCII digits as a JSON string, for the current 30-second step or one either side.
pub fn verify_totp(secret: &str, candidate: Option<&Value>, now_ms: i64) -> bool {
    let Some(Value::String(candidate)) = candidate else {
        return false;
    };
    if candidate.len() != 6 || !candidate.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let Some(key) = decode_base32(secret) else {
        return false;
    };
    if key.is_empty() {
        return false;
    }
    let step = now_ms.div_euclid(30_000);
    let mut matched = false;
    for s in [step - 1, step, step + 1] {
        let code = format!("{:06}", code_at(&key, s));
        matched |= bool::from(code.as_bytes().ct_eq(candidate.as_bytes()));
    }
    matched
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SECRET: &str = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";

    #[test]
    fn rfc_6238_codes_verify_within_one_step() {
        assert!(verify_totp(SECRET, Some(&json!("287082")), 59_000));
        assert!(verify_totp(SECRET, Some(&json!("287082")), 89_000));
        assert!(!verify_totp(SECRET, Some(&json!("287082")), 149_000));
        assert!(!verify_totp(SECRET, Some(&json!(287082)), 59_000));
        assert!(!verify_totp(SECRET, Some(&json!("28708")), 59_000));
        assert!(!verify_totp(SECRET, Some(&json!("28708x")), 59_000));
        assert!(!verify_totp(SECRET, None, 59_000));
        assert!(!verify_totp("not*base32", Some(&json!("287082")), 59_000));
        assert!(verify_totp(
            "gezd gnbv-gy3t qojq gezd gnbv gy3t qojq====",
            Some(&json!("287082")),
            59_000
        ));
    }
}
