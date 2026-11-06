// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Email addresses and one-time codes (spec §7.1, §7.3).

use crate::util;

/// How long an emailed code stays valid: 5 minutes.
pub const CODE_TTL_MS: i64 = 5 * 60 * 1000;
/// Wrong entries a code allows before it is spent.
pub const CODE_MAX_ATTEMPTS: u32 = 5;

/// One `@`, not first, a local part of at most 64 units, a dotted domain of at least 3; no space, tab, CR or LF.
pub fn is_valid_email(text: &str) -> bool {
    let len = util::utf16_len(text);
    if !(1..=254).contains(&len) || text.contains([' ', '\t', '\r', '\n']) {
        return false;
    }
    let mut parts = text.split('@');
    let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    !local.is_empty()
        && util::utf16_len(local) <= 64
        && util::utf16_len(domain) >= 3
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
}

/// `m***e@gmail.com`: the local part's first unit, `***`, its last when longer than 2, `@`, the domain.
pub fn mask_email(text: &str) -> String {
    let Some(at) = text.rfind('@').filter(|&at| at > 0) else {
        return "***".to_owned();
    };
    let (local, domain) = (&text[..at], &text[at + 1..]);
    let units: Vec<u16> = local.encode_utf16().collect();
    let first = String::from_utf16_lossy(&units[..1]);
    let last = if units.len() > 2 {
        String::from_utf16_lossy(&units[units.len() - 1..])
    } else {
        String::new()
    };
    format!("{first}***{last}@{domain}")
}

/// Six ASCII digits.
pub fn is_code_shape(text: &str) -> bool {
    text.len() == 6 && text.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        assert!(is_valid_email("Slayer1@Example.Test"));
        for bad in ["nope", "@x.com", "a@b", "a@.bc", "a@bc.", "a b@c.de", "a@b@c.de", ""] {
            assert!(!is_valid_email(bad), "{bad}");
        }
        assert!(!is_valid_email(&format!("{}@b.co", "a".repeat(65))));
    }

    #[test]
    fn masks_and_codes() {
        assert_eq!(mask_email("maxine@gmail.com"), "m***e@gmail.com");
        assert_eq!(mask_email("mx@gmail.com"), "m***@gmail.com");
        assert_eq!(mask_email("a@b.co"), "a***@b.co");
        assert_eq!(mask_email("nope"), "***");
        assert_eq!(mask_email("@x.com"), "***");
        assert!(is_code_shape("012345"));
        for bad in [
            "12345",
            "1234567",
            "12 456",
            "12345a",
            "\u{ff11}\u{ff12}\u{ff13}\u{ff14}\u{ff15}\u{ff16}",
        ] {
            assert!(!is_code_shape(bad));
        }
    }
}
