//! Slice R4 (spec-runtime-r4): client login, Easy Anti-Cheat and the engine command line. The pure rules; the
//! DLL's `login` module installs the detours that apply them.

pub mod cmdline;
pub mod eac;
pub mod qos;

/// Whether `haystack` holds `needle`, ASCII letters compared case-insensitively.
fn contains_ignore_ascii_case(haystack: &[u16], needle: &str) -> bool {
    let needle: Vec<u16> = needle.encode_utf16().collect();
    needle.is_empty()
        || haystack
            .windows(needle.len())
            .any(|window| eq_ignore_ascii_case(window, &needle))
}

/// UTF-16 equality with ASCII letters folded.
pub fn eq_ignore_ascii_case(left: &[u16], right: &[u16]) -> bool {
    left.len() == right.len() && left.iter().zip(right).all(|(&a, &b)| fold(a) == fold(b))
}

fn fold(unit: u16) -> u16 {
    if (u16::from(b'A')..=u16::from(b'Z')).contains(&unit) {
        unit + 32
    } else {
        unit
    }
}
