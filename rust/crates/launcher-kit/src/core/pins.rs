// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Pinned paths, hashes, ports, resource thresholds, and hex digest normalization helpers.

use sha2::{Digest, Sha256};

/// Relative directory from the game root containing the Windows 64-bit shipping binaries.
pub const WIN64_RELATIVE_DIR: &str = "Archon/Binaries/Win64";

/// File name of the shipping game executable.
pub const EXE_NAME: &str = "Dauntless-Win64-Shipping.exe";

/// Relative path from the game root to the shipping game executable.
pub const EXE_RELATIVE_PATH: &str = "Archon/Binaries/Win64/Dauntless-Win64-Shipping.exe";

/// Lowercase hex SHA-256 digest of the expected 1.14.7 shipping executable.
pub const PINNED_EXE_SHA256: &str =
    "f3f95fbf22709ef077a11632ed95d675b7209b7e0172d9ecb9ba7d02a1fbb66b";

/// Visual C++ Redistributable DLLs required in the system directory on Windows.
pub const VC_RUNTIME_FILES: [&str; 2] = ["MSVCP140.dll", "VCRUNTIME140_1.dll"];

/// Local loopback port used for XMPP relay traffic.
pub const XMPP_PORT: u16 = 61099;

/// Default connection timeout in milliseconds.
pub const CONNECT_TIMEOUT_MS: u64 = 5000;

/// Interval in milliseconds between background server status polls.
pub const STATUS_POLL_MS: u64 = 15000;

/// Minimum extra free disk space buffer required above a download's byte size (256 MiB).
pub const DISK_MARGIN_BYTES: u64 = 256 * 1024 * 1024;

pub const SHA256_HEX_LEN: usize = 64;
pub const SHA1_HEX_LEN: usize = 40;

pub fn normalize_hex<const N: usize>(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let hex_part = trimmed
        .strip_prefix("sha256:")
        .or_else(|| trimmed.strip_prefix("SHA256:"))
        .or_else(|| trimmed.strip_prefix("sha1:"))
        .or_else(|| trimmed.strip_prefix("SHA1:"))
        .unwrap_or(trimmed);

    let mut out = String::with_capacity(N);
    for ch in hex_part.chars() {
        if ch == ':' || ch == ' ' || ch == '-' {
            continue;
        }
        if !ch.is_ascii_hexdigit() {
            return None;
        }
        out.push(ch.to_ascii_lowercase());
        if out.len() > N {
            return None;
        }
    }
    if out.len() == N {
        Some(out)
    } else {
        None
    }
}

pub fn normalize_pin(raw: &str) -> Option<String> {
    normalize_hex::<SHA256_HEX_LEN>(raw)
}

pub fn normalize_sha256(raw: &str) -> Option<String> {
    normalize_hex::<SHA256_HEX_LEN>(raw)
}

pub fn normalize_sha1(raw: &str) -> Option<String> {
    normalize_hex::<SHA1_HEX_LEN>(raw)
}

pub fn is_lower_hex(s: &str, expected_len: usize) -> bool {
    s.len() == expected_len
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

pub fn sha256_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    lower_hex(&digest)
}
