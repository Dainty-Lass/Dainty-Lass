// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `--env-file` and the other env files read (spec §3, §11.4, §11.5).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use dainty_store::DatabaseConfig;

use crate::args::Target;
use crate::error::ImportError;

fn unquote(value: &str) -> &str {
    for quote in ['"', '\''] {
        if let Some(inner) = value.strip_prefix(quote).and_then(|v| v.strip_suffix(quote)) {
            return inner;
        }
    }
    value
}

/// `NAME=value` lines, blank and `#` lines skipped, both sides trimmed and one layer of quotes taken off the
/// value; the last line for a name wins.
pub(crate) fn parse(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((name, value)) = line.split_once('=') {
            out.insert(name.trim().to_owned(), unquote(value.trim()).to_owned());
        }
    }
    out
}

/// The names in `secrets.env` with a non-empty value; the values are looked at, never kept (§11.4).
pub(crate) fn secret_names(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        if let Some((name, value)) = line.split_once('=')
            && !unquote(value.trim()).is_empty()
        {
            out.insert(name.trim().to_owned());
        }
    }
    out
}

/// Reads an env file, or `None` when there is none.
pub(crate) fn read_optional(path: &Path) -> Result<Option<BTreeMap<String, String>>, ImportError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(parse(&String::from_utf8_lossy(&bytes)))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(ImportError::io(path, e)),
    }
}

/// The 1.14.7 database the arguments name. A `DAINTY_X` setting is read as `REFORGED_X` too, an older kit's
/// file.
pub(crate) fn database(target: &Target) -> Result<DatabaseConfig, ImportError> {
    let file = match target {
        Target::To(path) => return Ok(DatabaseConfig::Sqlite { file: path.clone() }),
        Target::EnvFile(file) => file,
    };
    let text = std::fs::read(file).map_err(|e| ImportError::io(file, e))?;
    let values = parse(&String::from_utf8_lossy(&text));
    let get = |name: &str| {
        values.get(name).cloned().or_else(|| {
            name.strip_prefix("DAINTY_")
                .and_then(|rest| values.get(&format!("REFORGED_{rest}")).cloned())
        })
    };
    let engine = get("DAINTY_DB_ENGINE");
    let sqlite = engine
        .as_deref()
        .is_none_or(|e| e.trim().is_empty() || e.trim() == "sqlite");
    if sqlite && get("DAINTY_DB_FILE").is_none_or(|f| f.trim().is_empty()) {
        return Err(ImportError::Invalid(format!(
            "{}: no DAINTY_DB_FILE, so not the 1.14.7 backend's env file",
            file.display()
        )));
    }
    DatabaseConfig::from_lookup(get, PathBuf::new())
        .map_err(|e| ImportError::Invalid(format!("{}: {e}", file.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_like_the_spec() {
        let values = parse("# c\n\n A = 1 \nB=\"two\"\nC='three'\nA=4\nnot a line\nD=\"half\n");
        assert_eq!(values["A"], "4");
        assert_eq!(values["B"], "two");
        assert_eq!(values["C"], "three");
        assert_eq!(values["D"], "\"half");
        assert_eq!(values.len(), 4);
    }

    #[test]
    fn secret_names_skip_empty_values() {
        let names = secret_names("export A=x\nB=\nC=\"\"\n# D=y\nE = 'z'\n");
        assert_eq!(names.into_iter().collect::<Vec<_>>(), ["A", "E"]);
    }

    #[test]
    fn env_file_targets() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("backend.env");
        std::fs::write(&file, "REFORGED_DB_FILE=/srv/isles.db\n").unwrap();
        assert_eq!(
            database(&Target::EnvFile(file.clone())).unwrap(),
            DatabaseConfig::Sqlite {
                file: "/srv/isles.db".into()
            }
        );
        std::fs::write(&file, "GATEWAY_PORT=1\n").unwrap();
        let err = database(&Target::EnvFile(file.clone())).unwrap_err().to_string();
        assert!(
            err.ends_with(": no DAINTY_DB_FILE, so not the 1.14.7 backend's env file"),
            "{err}"
        );
        std::fs::write(&file, "DAINTY_DB_ENGINE=postgres\n").unwrap();
        let err = database(&Target::EnvFile(file.clone())).unwrap_err().to_string();
        assert!(err.ends_with("DAINTY_DB_PASSWORD is not set"), "{err}");
        std::fs::write(&file, "DAINTY_DB_ENGINE=postgres\nREFORGED_DB_PASSWORD=hunter2\n").unwrap();
        let DatabaseConfig::Postgres(pg) = database(&Target::EnvFile(file)).unwrap() else {
            panic!("not postgres")
        };
        assert_eq!(pg.password, "hunter2");
        assert!(!format!("{pg:?}").contains("hunter2"));
    }
}
