// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Finding a program: on `PATH`, and Windows' `Program Files`.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Returns the primary Windows `Program Files` directory (`ProgramW6432` or `ProgramFiles`).
#[must_use]
pub fn program_files() -> PathBuf {
    std::env::var_os("ProgramW6432")
        .or_else(|| std::env::var_os("ProgramFiles"))
        .map_or_else(|| PathBuf::from(r"C:\Program Files"), PathBuf::from)
}

/// Helper trait allowing [`search_path`] to accept either a single binary name (`&str`) or a slice (`&[&str]`).
pub trait SearchNames<'a> {
    /// The names to look for.
    fn as_names(&self) -> Vec<&'a str>;
}

impl<'a> SearchNames<'a> for &'a str {
    fn as_names(&self) -> Vec<&'a str> {
        vec![*self]
    }
}

impl<'a> SearchNames<'a> for &'a [&'a str] {
    fn as_names(&self) -> Vec<&'a str> {
        self.to_vec()
    }
}

impl<'a, const N: usize> SearchNames<'a> for [&'a str; N] {
    fn as_names(&self) -> Vec<&'a str> {
        self.to_vec()
    }
}

impl<'a, const N: usize> SearchNames<'a> for &'a [&'a str; N] {
    fn as_names(&self) -> Vec<&'a str> {
        self.to_vec()
    }
}

/// Searches `path` (`PATH`) for the first existing executable matching `names`.
#[must_use]
pub fn search_path<'a>(names: impl SearchNames<'a>, path: Option<&OsStr>) -> Option<PathBuf> {
    let path = path?;
    let name_list = names.as_names();
    for dir in std::env::split_paths(path) {
        let cleaned = dir.to_string_lossy();
        let cleaned = cleaned.trim().trim_matches('"');
        if cleaned.is_empty() {
            continue;
        }
        let dir_path = Path::new(cleaned);
        for &name in &name_list {
            let candidate = dir_path.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
            if cfg!(windows) && !name.to_ascii_lowercase().ends_with(".exe") {
                let with_exe = dir_path.join(format!("{name}.exe"));
                if with_exe.is_file() {
                    return Some(with_exe);
                }
            }
        }
    }
    None
}
