//! What Rustic Isles' route table reads from players (spec-rustic-isles-core §14): any path against every row.
//! Never panics, and every Express row agrees with a reference model of §4.1 (segment counts, ASCII
//! case-insensitive literals, non-empty params, decode failures refused). The regex crate matches in linear time.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rustic_isles::{Pattern, routes};

fn trim_one(path: &str) -> &str {
    if path.len() > 1 { path.strip_suffix('/').unwrap_or(path) } else { path }
}

/// `None` no match, `Some(false)` a param fails to decode, `Some(true)` a match.
fn model(row: &str, path: &str) -> Option<bool> {
    let want: Vec<&str> = trim_one(row).strip_prefix('/')?.split('/').collect();
    let got: Vec<&str> = trim_one(path).strip_prefix('/')?.split('/').collect();
    if want.len() != got.len() {
        return None;
    }
    let mut decodes = true;
    for (w, g) in want.iter().zip(&got) {
        if w.starts_with(':') {
            if g.is_empty() {
                return None;
            }
            decodes &= dainty_service_kit::js::decode_uri_component(g).is_some();
        } else if !w.eq_ignore_ascii_case(g) {
            return None;
        }
    }
    Some(decodes)
}

fuzz_target!(|data: &[u8]| {
    let path = String::from_utf8_lossy(data);
    for row in routes::all() {
        let Ok(pattern) = Pattern::parse(row.path) else {
            panic!("{} does not compile", row.path);
        };
        let answer = pattern.matches(&path);
        if !row.is_regex() {
            assert_eq!(answer.as_ref().map(Result::is_ok), model(row.path, &path), "{} {path:?}", row.path);
        }
    }
});
