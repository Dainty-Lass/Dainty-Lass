//! `mystic_loader.ini`: the extra DLLs a host loads after the runtime.

use std::path::{Path, PathBuf};

use crate::runtime;

pub const FILE: &str = "mystic_loader.ini";

/// The values the ini lists, in file order; text that isn't UTF-8 lists none.
pub fn values(bytes: &[u8]) -> Vec<String> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Vec::new();
    };
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    text.split('\n')
        .filter_map(|line| {
            let line = line.strip_suffix('\r').unwrap_or(line);
            let line = line.split([';', '#']).next().unwrap_or_default().trim();
            if line.is_empty() || line.starts_with('[') {
                return None;
            }
            let value = line.split_once('=').map_or(line, |(_, value)| value.trim());
            (!value.is_empty()).then(|| value.to_owned())
        })
        .collect()
}

/// The paths to load: runtime names are left to the runtime choice, so an ini never loads a second copy.
pub fn paths(folder: &Path, values: &[String]) -> Vec<PathBuf> {
    values
        .iter()
        .filter(|value| !runtime::is_runtime_name(value))
        .map(|value| folder.join(value))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> Vec<String> {
        values(text.as_bytes())
    }

    #[test]
    fn the_spec_examples() {
        assert_eq!(parse("extra.dll"), ["extra.dll"]);
        assert_eq!(parse("; note\n\n[mods]\n  b.dll  \n"), ["b.dll"]);
        assert_eq!(parse("dll1 = a.dll\nx=  "), ["a.dll"]);
        assert_eq!(parse("a.dll ; trailing\nc.dll#x"), ["a.dll", "c.dll"]);
        assert_eq!(parse("k = v = w.dll"), ["v = w.dll"]);
        assert_eq!(
            parse("Rustic-Isles.dll\nrustic-isles.DLL\nMYSTPAXINTERNALSERVER.dll\nUndauntedInternalServer.dll"),
            [
                "Rustic-Isles.dll",
                "rustic-isles.DLL",
                "MYSTPAXINTERNALSERVER.dll",
                "UndauntedInternalServer.dll"
            ]
        );
        assert_eq!(parse("C:\\mods\\x.dll"), ["C:\\mods\\x.dll"]);
    }

    #[test]
    fn crlf_line_ends() {
        assert_eq!(parse("[mods]\r\na.dll\r\nb = c.dll\r\n"), ["a.dll", "c.dll"]);
    }

    #[test]
    fn a_leading_bom_is_skipped() {
        assert_eq!(parse("\u{feff}[mods]\na.dll"), ["a.dll"]);
        assert_eq!(parse("\u{feff}a.dll"), ["a.dll"]);
    }

    #[test]
    fn invalid_utf8_gives_no_values() {
        assert!(values(b"a.dll\n\xff\xfeb.dll").is_empty());
    }

    #[test]
    fn runtime_names_are_skipped_in_any_case() {
        let folder = Path::new("C:\\Game\\Binaries\\Win64");
        let values = parse(
            "Rustic-Isles.dll\nrustic-isles.DLL\nMYSTPAXINTERNALSERVER.dll\nmysticparadox.dll\nUndauntedInternalServer.dll\nsub\\Rustic-Isles.dll",
        );
        assert_eq!(
            paths(folder, &values),
            [
                folder.join("UndauntedInternalServer.dll"),
                folder.join("sub\\Rustic-Isles.dll")
            ]
        );
    }

    #[test]
    fn an_absolute_value_replaces_the_folder() {
        let folder = Path::new("C:\\Game\\Binaries\\Win64");
        assert_eq!(
            paths(folder, &parse("C:\\mods\\x.dll")),
            [PathBuf::from("C:\\mods\\x.dll")]
        );
    }
}
