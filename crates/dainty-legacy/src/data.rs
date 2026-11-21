// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The two data files (spec §2): the 1.4.4 metagame's history as facts, and the Node kit's keys and folders.
//! Embedded and parsed once; a parse failure is a bug, which `tests::both_parse` catches.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

use crate::error::ImportError;

const REVIVED: &str = include_str!("../data/revived-1.4.4.toml");
const REFORGED: &str = include_str!("../data/reforged.toml");

/// `revived-1.4.4.toml`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Revived {
    /// Every table any 1.4.4 migration leaves behind.
    pub tables: Vec<String>,
    /// Every trigger any 1.4.4 migration leaves behind.
    pub triggers: Vec<String>,
    /// The tables an import reads.
    pub carried: Vec<String>,
    /// Credentials and bookkeeping: not counted as left behind.
    pub not_player_data: Vec<String>,
    /// The tables an offline save must have emptied.
    pub cleared: Vec<String>,
    /// The Rust metagame's history.
    pub diesel: Diesel,
    /// The Node metagame's history, oldest first.
    pub drizzle: Vec<Drizzle>,
    /// The columns the importer reads, and the migration that made each.
    pub column: Vec<Column>,
}

/// `[diesel]`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Diesel {
    /// The versions, ascending.
    pub versions: Vec<String>,
}

/// One `[[drizzle]]` migration.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Drizzle {
    /// The journal's tag.
    pub tag: String,
    /// The journal's `when`, Drizzle's `created_at`.
    pub when: i64,
    /// SHA-256 of the file with LF line ends.
    pub sha256_lf: String,
    /// SHA-256 of the file with CRLF line ends.
    pub sha256_crlf: String,
}

/// One `[[column]]`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Column {
    /// Its table.
    pub table: String,
    /// Its name.
    pub name: String,
    /// The drizzle index that made it: present at prefix k exactly when k > since.
    pub since: usize,
}

impl Revived {
    /// The prefix a Diesel database is at.
    pub fn full_prefix(&self) -> usize {
        self.drizzle.len()
    }

    /// Whether a database at `prefix` has `table.column`.
    pub fn has_column(&self, prefix: usize, table: &str, column: &str) -> bool {
        self.column
            .iter()
            .any(|c| c.table == table && c.name == column && prefix > c.since)
    }

    /// Whether a database at `prefix` has `table`, by the columns read from it.
    pub fn has_table(&self, prefix: usize, table: &str) -> bool {
        self.column.iter().any(|c| c.table == table && prefix > c.since)
    }
}

/// `reforged.toml`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reforged {
    /// Top-level host.json keys left out of settings.toml.
    pub retired_keys: Vec<String>,
    /// Module ids left out of `modules`.
    pub retired_modules: Vec<String>,
    /// The comment settings.toml starts with.
    pub header: String,
    /// The top-level keys, `[json, toml]`, in settings.toml's order.
    pub top: Vec<(String, String)>,
    /// The host kit's `SECRET_KEYS`.
    pub secret_keys: Vec<String>,
    /// What `<sdk>` expands to.
    pub sdk_files: Vec<String>,
    /// Folder moves, `[from, to]`, in order.
    pub moves: Vec<(String, String)>,
    /// Folders nothing uses any more.
    pub old_folders: Vec<String>,
    /// Each table's keys, `[json, toml]`, in settings.toml's order.
    pub keys: BTreeMap<String, Vec<(String, String)>>,
    /// A secret's new name to its old one.
    pub secret_renames: BTreeMap<String, String>,
}

fn parsed<T: for<'de> Deserialize<'de>>(
    cell: &'static OnceLock<Result<T, String>>,
    text: &str,
    name: &str,
) -> Result<&'static T, ImportError> {
    cell.get_or_init(|| toml::from_str(text).map_err(|e| format!("{name} does not parse: {e}")))
        .as_ref()
        .map_err(|e| ImportError::Io(e.clone()))
}

/// The parsed `revived-1.4.4.toml`.
pub fn revived() -> Result<&'static Revived, ImportError> {
    static CELL: OnceLock<Result<Revived, String>> = OnceLock::new();
    parsed(&CELL, REVIVED, "revived-1.4.4.toml")
}

/// The parsed `reforged.toml`.
pub fn reforged() -> Result<&'static Reforged, ImportError> {
    static CELL: OnceLock<Result<Reforged, String>> = OnceLock::new();
    parsed(&CELL, REFORGED, "reforged.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_parse() {
        let revived = revived().unwrap();
        assert_eq!(revived.drizzle.len(), 31);
        assert_eq!(revived.drizzle[0].tag, "0000_natural_korg");
        assert_eq!(revived.drizzle[30].when, 1792200000002);
        assert!(revived.drizzle.windows(2).all(|w| w[0].when < w[1].when));
        assert!(
            revived
                .drizzle
                .iter()
                .all(|d| d.sha256_lf.len() == 64 && d.sha256_crlf.len() == 64)
        );
        assert_eq!(revived.diesel.versions, ["0001"]);
        assert_eq!(revived.column.len(), 16);
        assert!(revived.carried.iter().all(|t| revived.tables.contains(t)));
        assert!(revived.cleared.iter().all(|t| revived.tables.contains(t)));
        assert!(revived.column.iter().all(|c| revived.tables.contains(&c.table)));

        let reforged = reforged().unwrap();
        assert!(reforged.header.starts_with("# This server's settings"));
        assert_eq!(reforged.top.len(), 21);
        assert_eq!(reforged.keys.len(), 10);
        assert!(reforged.keys.keys().all(|k| reforged.top.iter().any(|(j, _)| j == k)));
        assert_eq!(reforged.moves.len(), 14);
        assert_eq!(reforged.secret_keys.len(), 11);
        assert!(reforged.secret_renames.keys().all(|k| reforged.secret_keys.contains(k)));
    }

    #[test]
    fn prefixes() {
        let revived = revived().unwrap();
        assert!(!revived.has_column(8, "users", "isAdmin"));
        assert!(revived.has_column(9, "users", "isAdmin"));
        assert!(!revived.has_table(23, "account_bans"));
        assert!(revived.has_table(24, "account_bans"));
        assert!(revived.has_column(1, "users", "userId"));
    }
}
