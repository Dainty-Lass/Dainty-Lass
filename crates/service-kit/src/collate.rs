// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

use std::cmp::Ordering;

const ORDER: &[u8] = b" _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$0123456789abcdefghijklmnopqrstuvwxyz";

const fn build_table() -> [u16; 128] {
    let mut table = [u16::MAX; 128];
    let mut i = 0;
    while i < ORDER.len() {
        let ch = ORDER[i];
        table[ch as usize] = i as u16;
        if ch.is_ascii_lowercase() {
            table[(ch - b'a' + b'A') as usize] = i as u16;
        }
        i += 1;
    }
    table
}

const PRIMARY_TABLE: [u16; 128] = build_table();

fn primary_weight(c: char) -> u32 {
    let code = c as u32;
    if code < 128 {
        u32::from(PRIMARY_TABLE[code as usize])
    } else {
        128 + code
    }
}

fn tertiary_weight(c: char) -> u8 {
    u8::from(c.is_ascii_uppercase())
}

pub fn locale_compare(a: &str, b: &str) -> Ordering {
    let mut iter_a = a.chars().filter(|c| !c.is_ascii_control());
    let mut iter_b = b.chars().filter(|c| !c.is_ascii_control());
    let mut tertiary = Ordering::Equal;

    loop {
        match (iter_a.next(), iter_b.next()) {
            (Some(ca), Some(cb)) => {
                let cmp = primary_weight(ca).cmp(&primary_weight(cb));
                if cmp != Ordering::Equal {
                    return cmp;
                }
                if tertiary == Ordering::Equal {
                    tertiary = tertiary_weight(ca).cmp(&tertiary_weight(cb));
                }
            }
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (None, None) => return tertiary,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_punctuation_digits_and_letters_in_icu_root_order() {
        let mut items = vec!["b", "a", "1", "0", "_", "-", " ", "$"];
        items.sort_by(|x, y| locale_compare(x, y));
        assert_eq!(items, vec![" ", "_", "-", "$", "0", "1", "a", "b"]);
    }

    #[test]
    fn lowercase_precedes_uppercase_when_primary_equal() {
        assert_eq!(locale_compare("a", "A"), Ordering::Less);
        assert_eq!(locale_compare("A", "a"), Ordering::Greater);
        assert_eq!(locale_compare("aB", "Ab"), Ordering::Less);
        // Primary difference on second character outweighs tertiary difference on first character
        assert_eq!(locale_compare("Aa", "aB"), Ordering::Less);
    }

    #[test]
    fn ignores_ascii_control_characters() {
        assert_eq!(locale_compare("a\x00b\n", "ab"), Ordering::Equal);
    }
}
