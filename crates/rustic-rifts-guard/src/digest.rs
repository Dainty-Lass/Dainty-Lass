// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

use sha2::{Digest as _, Sha256};

/// SHA-256 as lower-case hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex_of_parts(&[bytes])
}

/// SHA-256 over the parts one after another, as lower-case hex.
pub(crate) fn hex_of_parts(parts: &[&[u8]]) -> String {
    let digest = parts
        .iter()
        .fold(Sha256::new(), |hasher, part| hasher.chain_update(part))
        .finalize();
    hex::encode(digest)
}

/// 64 characters of `0-9a-f`.
pub(crate) fn is_hex64(text: &str) -> bool {
    text.len() == 2 * 32 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::*;

    // FIPS 180-2's vectors.
    const VECTORS: [(&str, &str); 3] = [
        ("", "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
        (
            "abc",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
        (
            "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
        ),
    ];

    #[test]
    fn known_vectors() {
        for (input, digest) in VECTORS {
            assert_eq!(sha256_hex(input.as_bytes()), digest, "{input}");
        }
        let million = "a".repeat(1_000_000);
        let digest = "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0";
        assert_eq!(sha256_hex(million.as_bytes()), digest);
    }

    #[test]
    fn split_updates_hash_the_concatenation() {
        let whole = sha256_hex(b"rifts-guard-batch-v1\n{}");
        let splits: [&[&[u8]]; 2] = [
            &[b"rifts-guard-batch-v1\n", b"{}"],
            &[b"rifts-", b"", b"guard-batch-v1\n{", b"}"],
        ];
        for parts in splits {
            assert_eq!(hex_of_parts(parts), whole);
        }
        assert!(is_hex64(&whole));
        for bad in [whole.to_uppercase(), whole[1..].to_owned()] {
            assert!(!is_hex64(&bad), "{bad}");
        }
    }
}
