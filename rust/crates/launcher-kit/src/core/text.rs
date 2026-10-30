// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! ECMAScript-compatible string trimming, UTF-16 code unit length, control/bidi character filtering,
//! and numeric coercion (`Number(str)`).

pub use protocol::text::{is_js_space, is_unsafe_char, js_length, js_trim};

/// Alias for [`js_length`]: returns the number of UTF-16 code units in `text`.
#[must_use]
pub fn utf16_len(text: &str) -> usize {
    js_length(text)
}

/// Returns `true` if `text` has at most `max_units` UTF-16 code units.
#[must_use]
pub fn within_utf16(text: &str, max_units: usize) -> bool {
    utf16_len(text) <= max_units
}

/// Truncates `text` to at most `max_units` UTF-16 code units without splitting surrogate pairs.
#[must_use]
pub fn truncate_utf16(text: &str, max_units: usize) -> &str {
    let mut units = 0;
    for (idx, ch) in text.char_indices() {
        let ch_units = ch.len_utf16();
        if units + ch_units > max_units {
            return &text[..idx];
        }
        units += ch_units;
    }
    text
}

/// Strips unsafe control/bidi characters, collapses ECMAScript whitespace runs into single spaces,
/// trims both ends, and truncates to `max_utf16` UTF-16 code units.
#[must_use]
pub fn clean_text(text: &str, max_utf16: usize) -> String {
    let mut cleaned = String::with_capacity(text.len());
    let mut in_space = false;
    for ch in js_trim(text).chars() {
        if is_unsafe_char(ch) {
            continue;
        }
        if is_js_space(ch) {
            if !in_space && !cleaned.is_empty() {
                cleaned.push(' ');
                in_space = true;
            }
        } else {
            cleaned.push(ch);
            in_space = false;
        }
    }
    if cleaned.ends_with(' ') {
        cleaned.pop();
    }
    truncate_utf16(&cleaned, max_utf16).to_owned()
}

/// Parses `text` according to ECMAScript `ToNumber(String)` / `Number(text)` rules:
/// - Leading and trailing ECMAScript whitespace is trimmed.
/// - An empty (or whitespace-only) string evaluates to `0.0`.
/// - `"Infinity"` and `"+Infinity"` evaluate to `f64::INFINITY`; `"-Infinity"` to `f64::NEG_INFINITY`.
/// - Unsigned `0x`/`0X`, `0o`/`0O`, and `0b`/`0B` integer literals are parsed in base 16, 8, and 2.
/// - Standard decimal integer and floating-point literals are parsed as IEEE-754 `f64`.
/// - All other inputs (including `"NaN"`, `"inf"`, signed hex literals, or trailing junk) return `f64::NAN`.
#[must_use]
pub fn js_number(text: &str) -> f64 {
    let trimmed = js_trim(text);
    if trimmed.is_empty() {
        return 0.0;
    }
    match trimmed {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }

    if let Some(hex) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        if hex.is_empty() || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return f64::NAN;
        }
        return hex.bytes().fold(0.0_f64, |acc, b| {
            let digit = match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' => b - b'a' + 10,
                b'A'..=b'F' => b - b'A' + 10,
                _ => 0,
            };
            acc * 16.0 + f64::from(digit)
        });
    }

    if let Some(oct) = trimmed
        .strip_prefix("0o")
        .or_else(|| trimmed.strip_prefix("0O"))
    {
        if oct.is_empty() || !oct.bytes().all(|b| matches!(b, b'0'..=b'7')) {
            return f64::NAN;
        }
        return oct
            .bytes()
            .fold(0.0_f64, |acc, b| acc * 8.0 + f64::from(b - b'0'));
    }

    if let Some(bin) = trimmed
        .strip_prefix("0b")
        .or_else(|| trimmed.strip_prefix("0B"))
    {
        if bin.is_empty() || !bin.bytes().all(|b| matches!(b, b'0' | b'1')) {
            return f64::NAN;
        }
        return bin
            .bytes()
            .fold(0.0_f64, |acc, b| acc * 2.0 + f64::from(b - b'0'));
    }

    if !is_decimal_literal(trimmed) {
        return f64::NAN;
    }
    trimmed.parse::<f64>().unwrap_or(f64::NAN)
}

fn is_decimal_literal(s: &str) -> bool {
    let s = s
        .strip_prefix('+')
        .or_else(|| s.strip_prefix('-'))
        .unwrap_or(s);
    if s.is_empty() {
        return false;
    }
    let (mantissa, exp_part) = if let Some(idx) = s.find(['e', 'E']) {
        (&s[..idx], Some(&s[idx + 1..]))
    } else {
        (s, None)
    };

    let valid_mantissa = if let Some((int_part, frac_part)) = mantissa.split_once('.') {
        (!int_part.is_empty() || !frac_part.is_empty())
            && int_part.bytes().all(|b| b.is_ascii_digit())
            && frac_part.bytes().all(|b| b.is_ascii_digit())
    } else {
        !mantissa.is_empty() && mantissa.bytes().all(|b| b.is_ascii_digit())
    };
    if !valid_mantissa {
        return false;
    }

    if let Some(exp) = exp_part {
        let exp = exp
            .strip_prefix('+')
            .or_else(|| exp.strip_prefix('-'))
            .unwrap_or(exp);
        if exp.is_empty() || !exp.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_ecmascript_whitespace_and_bom() {
        assert_eq!(js_trim("  \t\r\nhello\u{00a0}\u{feff}\u{3000}"), "hello");
        assert_eq!(js_trim("\u{2000}\u{2028}\u{2029}"), "");
        assert!(!is_js_space('\u{0085}'));
        assert!(!is_js_space('\u{200b}'));
    }

    #[test]
    fn counts_utf16_code_units() {
        assert_eq!(js_length("ascii"), 5);
        assert_eq!(js_length("é"), 1);
        assert_eq!(js_length("🐉"), 2);
        assert_eq!(js_length("a🐉b"), 4);
    }

    #[test]
    fn detects_control_and_bidi_override_characters() {
        assert!(is_unsafe_char('\0'));
        assert!(is_unsafe_char('\x1f'));
        assert!(is_unsafe_char('\x7f'));
        assert!(is_unsafe_char('\u{009f}'));
        assert!(is_unsafe_char('\u{202e}'));
        assert!(is_unsafe_char('\u{2066}'));
        assert!(!is_unsafe_char(' '));
        assert!(!is_unsafe_char('A'));
        assert!(!is_unsafe_char('🐉'));
    }

    #[test]
    fn parses_ecmascript_numbers() {
        assert_eq!(js_number(""), 0.0);
        assert_eq!(js_number(" \t\n\u{feff} "), 0.0);
        assert_eq!(js_number("42"), 42.0);
        assert_eq!(js_number(" -3.5 "), -3.5);
        assert_eq!(js_number(".5"), 0.5);
        assert_eq!(js_number("5."), 5.0);
        assert_eq!(js_number("1e3"), 1000.0);
        assert_eq!(js_number("0xff"), 255.0);
        assert_eq!(js_number("0X10"), 16.0);
        assert_eq!(js_number("0o77"), 63.0);
        assert_eq!(js_number("0b1010"), 10.0);
        assert_eq!(js_number("Infinity"), f64::INFINITY);
        assert_eq!(js_number("+Infinity"), f64::INFINITY);
        assert_eq!(js_number("-Infinity"), f64::NEG_INFINITY);

        assert!(js_number(".").is_nan());
        assert!(js_number("-0x10").is_nan());
        assert!(js_number("0x").is_nan());
        assert!(js_number("0o8").is_nan());
        assert!(js_number("0b2").is_nan());
        assert!(js_number("inf").is_nan());
        assert!(js_number("infinity").is_nan());
        assert!(js_number("NaN").is_nan());
        assert!(js_number("12px").is_nan());
    }
}
