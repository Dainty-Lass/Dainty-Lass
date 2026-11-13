//! The pure half of the MIT game runtime (`rustic-isles-runtime.dll`, RT-1): everything that can be decided without
//! the game or Windows, so `cargo test` covers it on every platform (spec-runtime-r0 section 2, spec-runtime-r1).

pub mod chain;
pub mod config;
pub mod engine;
pub mod env;
pub mod exit;
pub mod flags;
pub mod frame;
pub mod http;
pub mod log;
pub mod login;
pub mod memory;
pub mod names;
pub mod offsets;
pub mod patch;
pub mod pdata;
pub mod registry;

/// The seam ABI the AGPL runtime checks before it routes a hook through this DLL (R0 7.3).
pub const SEAM_VERSION: u32 = 1;

/// The UTF-16 units of an ASCII literal, for comparing against engine and Windows text.
pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// Whether `haystack` holds `needle` (both UTF-16).
pub fn wide_contains(haystack: &[u16], needle: &[u16]) -> bool {
    needle.is_empty() || haystack.windows(needle.len()).any(|window| window == needle)
}

/// Where `needle` first starts in `haystack`.
pub fn wide_find(haystack: &[u16], needle: &[u16]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack.windows(needle.len()).position(|window| window == needle)
}
