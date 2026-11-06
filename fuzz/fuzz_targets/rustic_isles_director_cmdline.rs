//! The Director's Windows command line (spec-rustic-isles-director §5.3, §15): any list of arguments. Never panics,
//! and a reference MSVC splitter gives back exactly the list; a NUL is refused before any spawn.
#![no_main]

use libfuzzer_sys::fuzz_target;
use rustic_isles_director::cmdline::{command_line, has_nul};

/// The MSVC runtime's rules after the program: backslashes are literal unless before a quote.
fn split(line: &str) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    // The program is quoted whole and has no quote of its own
    let mut at = chars.iter().skip(1).position(|c| *c == '"').map_or(chars.len(), |p| p + 2);
    let mut out = Vec::new();
    loop {
        while at < chars.len() && (chars[at] == ' ' || chars[at] == '\t') {
            at += 1;
        }
        if at >= chars.len() {
            return out;
        }
        let (mut arg, mut quoted) = (String::new(), false);
        while at < chars.len() {
            let c = chars[at];
            if c == '\\' {
                let run = chars[at..].iter().take_while(|c| **c == '\\').count();
                at += run;
                if chars.get(at) == Some(&'"') {
                    arg.extend(std::iter::repeat_n('\\', run / 2));
                    if run % 2 == 1 {
                        arg.push('"');
                        at += 1;
                    }
                } else {
                    arg.extend(std::iter::repeat_n('\\', run));
                }
                continue;
            }
            if c == '"' {
                if quoted && chars.get(at + 1) == Some(&'"') {
                    arg.push('"');
                    at += 2;
                    continue;
                }
                quoted = !quoted;
                at += 1;
                continue;
            }
            if !quoted && (c == ' ' || c == '\t') {
                break;
            }
            arg.push(c);
            at += 1;
        }
        out.push(arg);
    }
}

fuzz_target!(|args: Vec<String>| {
    if has_nul(args.iter().map(String::as_str)) {
        return;
    }
    let line = command_line("C:\\Game\\x.exe", &args);
    assert!(line.starts_with("\"C:\\Game\\x.exe\""));
    assert_eq!(split(&line), args);
});
