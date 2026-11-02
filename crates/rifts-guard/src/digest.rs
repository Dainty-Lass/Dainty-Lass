// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

use std::fmt::Write as _;

use sha2::{Digest as _, Sha256};

/// SHA-256 as lower-case hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex_of_parts(&[bytes])
}

/// SHA-256 over the parts one after another, as lower-case hex.
pub(crate) fn hex_of_parts(parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part);
    }
    let mut out = String::with_capacity(64);
    for byte in hasher.finalize() {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// 64 characters of `0-9a-f`.
pub(crate) fn is_hex64(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
        assert_eq!(
            sha256_hex(&vec![b'a'; 1_000_000]),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn split_updates_hash_the_concatenation() {
        let whole = sha256_hex(b"rifts-guard-batch-v1\n{}");
        assert_eq!(hex_of_parts(&[b"rifts-guard-batch-v1\n", b"{}"]), whole);
        assert_eq!(hex_of_parts(&[b"rifts-", b"", b"guard-batch-v1\n{", b"}"]), whole);
        assert!(is_hex64(&whole));
        assert!(!is_hex64(&whole.to_uppercase()));
        assert!(!is_hex64(&whole[1..]));
    }
}
