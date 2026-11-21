// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Certificate fingerprints as the invite's `fp` writes them.

use sha2::{Digest, Sha256};

/// A certificate fingerprint as 64 lowercase hex digits, however it was written: a `sha256:` prefix and `:`, space
/// or `-` separators are accepted, as the launcher kit's `core::pins` takes them.
#[must_use]
pub fn normalize_pin(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let hex_part = trimmed
        .strip_prefix("sha256:")
        .or_else(|| trimmed.strip_prefix("SHA256:"))
        .unwrap_or(trimmed);
    let mut out = String::with_capacity(64);
    for ch in hex_part.chars() {
        if matches!(ch, ':' | ' ' | '-') {
            continue;
        }
        if !ch.is_ascii_hexdigit() || out.len() == 64 {
            return None;
        }
        out.push(ch.to_ascii_lowercase());
    }
    (out.len() == 64).then_some(out)
}

/// SHA-256 of `data` as lowercase hex.
#[must_use]
pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_in_any_spelling() {
        let pin = "ab".repeat(32);
        assert_eq!(normalize_pin(&pin).as_deref(), Some(pin.as_str()));
        assert_eq!(
            normalize_pin(&format!(" sha256:{}", "AB:".repeat(31) + "AB")).as_deref(),
            Some(pin.as_str())
        );
        assert_eq!(normalize_pin(&"ab".repeat(33)), None);
        assert_eq!(normalize_pin("sha1:abcd"), None);
        assert_eq!(normalize_pin(&"zz".repeat(32)), None);
    }
}
