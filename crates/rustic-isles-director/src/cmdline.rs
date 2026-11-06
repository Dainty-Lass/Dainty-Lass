// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! A Windows command line the MSVC runtime (and Unreal's parser after it) splits back into the same arguments
//! (spec §5.3).

/// Whether an argument must be quoted to stay one argument.
fn needs_quotes(arg: &str) -> bool {
    arg.is_empty() || arg.contains([' ', '\t', '\n', '\u{0b}', '"'])
}

/// Appends one argument: as it is, or in quotes with `"` escaped and the backslashes before a quote doubled.
pub fn quote(arg: &str, out: &mut String) {
    if !needs_quotes(arg) {
        out.push_str(arg);
        return;
    }
    out.push('"');
    let mut backslashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            other => {
                out.extend(std::iter::repeat_n('\\', backslashes));
                out.push(other);
                backslashes = 0;
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', backslashes * 2));
    out.push('"');
}

/// The program quoted whole, then each argument after one space.
pub fn command_line(program: &str, args: &[String]) -> String {
    let mut out = String::with_capacity(program.len() + 2 + args.iter().map(|a| a.len() + 3).sum::<usize>());
    out.push('"');
    out.push_str(program);
    out.push('"');
    for arg in args {
        out.push(' ');
        quote(arg, &mut out);
    }
    out
}

/// A NUL would cut a Windows command line and is refused on every platform (§16 item 12).
pub fn has_nul<'a>(mut texts: impl Iterator<Item = &'a str>) -> bool {
    texts.any(|text| text.contains('\0'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spec_table() {
        let args: Vec<String> = ["plain", "-LogCmds=\"A B\"", "", "a\\b\\", "x y\\"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        assert_eq!(
            command_line("C:\\Game\\x.exe", &args),
            r#""C:\Game\x.exe" plain "-LogCmds=\"A B\"" "" a\b\ "x y\\""#
        );
        let mut out = String::new();
        quote("a\\\"b", &mut out);
        assert_eq!(out, r#""a\\\"b""#);
        assert!(has_nul(["a", "b\0"].into_iter()));
        assert!(!has_nul(["a"].into_iter()));
    }
}
