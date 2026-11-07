// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The character rules every field, line and request id is checked against.

/// 1..=`max` bytes, each ASCII alphanumeric or one of `extra`.
pub fn safe_token(text: &[u8], max: usize, extra: &[u8]) -> bool {
    (1..=max).contains(&text.len()) && text.iter().all(|b| b.is_ascii_alphanumeric() || extra.contains(b))
}

/// [`safe_token`] over a string's bytes.
pub fn safe_str(text: &str, max: usize, extra: &[u8]) -> bool {
    safe_token(text.as_bytes(), max, extra)
}

/// An account id as the transport takes it.
pub fn is_account(text: &str) -> bool {
    safe_str(text, 64, b"-")
}

/// The first `max` bytes, cut back to a character boundary.
pub fn truncate(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Exactly 64 lowercase hex digits.
pub fn is_hex64(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens() {
        assert!(safe_str("abc-1", 5, b"-"));
        assert!(!safe_str("abc-12", 5, b"-"));
        assert!(!safe_str("", 5, b"-"));
        assert!(!safe_str("a b", 5, b"-"));
        assert!(is_account("member-0"));
        assert!(!is_account("bad account!"));
        assert_eq!(truncate("héllo", 2), "h");
        assert_eq!(truncate("abc", 8), "abc");
    }
}
