// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! String handling that matches JavaScript's: UTF-16 lengths, `trim` and `\s`, and the control and bidi
//! characters stripped from display text.

/// Returns `true` if `c` is considered whitespace by ECMAScript (`String.prototype.trim` / `\s`).
#[must_use]
pub fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\u{0009}'..='\u{000D}'
            | '\u{0020}'
            | '\u{00A0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200A}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202F}'
            | '\u{205F}'
            | '\u{3000}'
            | '\u{FEFF}'
    )
}

/// Returns `true` if `ch` is considered whitespace by ECMAScript (`String.prototype.trim` / `\s`).
#[must_use]
pub fn is_js_whitespace(ch: char) -> bool {
    is_js_space(ch)
}

/// Returns the ECMAScript `.length` (number of UTF-16 code units) of `text`.
#[must_use]
pub fn js_length(text: &str) -> usize {
    text.encode_utf16().count()
}

/// Trims leading and trailing ECMAScript whitespace from `s`.
#[must_use]
pub fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_space)
}

/// Returns `true` if `c` is an ASCII/Latin-1 control character or a Unicode bidirectional control.
#[must_use]
pub fn is_unsafe_char(c: char) -> bool {
    matches!(
        c as u32,
        0x0000..=0x001F
            | 0x007F..=0x009F
            | 0x061C
            | 0x200E..=0x200F
            | 0x202A..=0x202E
            | 0x2066..=0x2069
    )
}

/// Returns `true` if `ch` is an ASCII/Latin-1 control character or a Unicode bidirectional control.
#[must_use]
pub fn is_control_or_bidi(ch: char) -> bool {
    is_unsafe_char(ch)
}

/// Returns `true` if `s` contains any control or bidirectional formatting character.
#[must_use]
pub fn has_control_or_format(s: &str) -> bool {
    s.chars().any(is_unsafe_char)
}

/// Removes control and bidirectional formatting characters from `s`.
#[must_use]
pub fn strip_control_and_format(s: &str) -> String {
    s.chars().filter(|&ch| !is_unsafe_char(ch)).collect()
}

/// Collapses runs of ECMAScript whitespace into a single ASCII space and trims both ends.
#[must_use]
pub fn collapse_js_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_ws = false;
    for ch in js_trim(s).chars() {
        if is_js_space(ch) {
            if !in_ws {
                out.push(' ');
                in_ws = true;
            }
        } else {
            out.push(ch);
            in_ws = false;
        }
    }
    out
}

/// Cleans untrusted display text by removing non-whitespace control and bidirectional characters,
/// collapsing ECMAScript whitespace runs to a single ASCII space, trimming, and truncating to
/// `max_chars` Unicode scalar values.
#[must_use]
pub fn clean_display_text(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    let mut count = 0;
    let mut pending_space = false;

    for ch in s.chars() {
        if is_js_space(ch) {
            if !out.is_empty() {
                pending_space = true;
            }
            continue;
        }
        if is_unsafe_char(ch) {
            continue;
        }
        if pending_space {
            if count + 1 >= max_chars {
                if count < max_chars {
                    out.push(' ');
                }
                break;
            }
            out.push(' ');
            count += 1;
            pending_space = false;
        }
        if count >= max_chars {
            break;
        }
        out.push(ch);
        count += 1;
    }

    out
}
