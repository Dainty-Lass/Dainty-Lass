// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! What an import did (spec §8). Never holds a key, a hash, a password or a secret's value.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Serialize;

/// Something an import left out, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Skipped {
    /// What it was: `account`, `discord_link`, `invite`, `folder`.
    pub kind: String,
    /// Its id in the source.
    pub id: String,
    /// Why it was left out.
    pub reason: String,
}

/// One import's report; the console's `data` is its JSON, fields in this order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Report {
    /// `revived`, `offline-save` or `reforged`.
    pub source: String,
    /// Nothing was written.
    pub dry_run: bool,
    /// What was imported, by name.
    pub imported: BTreeMap<String, u64>,
    /// What was already there, by name.
    pub unchanged: BTreeMap<String, u64>,
    /// What was left out.
    pub skipped: Vec<Skipped>,
    /// Source tables with rows that don't carry to 1.14.7, and how many.
    pub dropped: BTreeMap<String, u64>,
    /// What the host should know or do next.
    pub notes: Vec<String>,
}

impl Report {
    /// An empty report for `source`.
    pub fn new(source: &str, dry_run: bool) -> Self {
        Self {
            source: source.to_owned(),
            dry_run,
            ..Self::default()
        }
    }

    pub(crate) fn count(&mut self, what: &str) {
        *self.imported.entry(what.to_owned()).or_default() += 1;
    }

    pub(crate) fn same(&mut self, what: &str) {
        *self.unchanged.entry(what.to_owned()).or_default() += 1;
    }

    pub(crate) fn skip(&mut self, kind: &str, id: &str, reason: impl Into<String>) {
        self.skipped.push(Skipped {
            kind: kind.to_owned(),
            id: id.to_owned(),
            reason: reason.into(),
        });
    }

    pub(crate) fn note(&mut self, note: impl Into<String>) {
        self.notes.push(note.into());
    }

    /// The text the console and the command line print.
    pub fn text(&self) -> String {
        let mut out = format!(
            "{} import{}:",
            self.source,
            if self.dry_run {
                " (dry run: nothing written)"
            } else {
                ""
            }
        );
        for (what, n) in &self.imported {
            let _ = write!(out, "\n  imported {what}: {n}");
        }
        for (what, n) in &self.unchanged {
            let _ = write!(out, "\n  already there {what}: {n}");
        }
        if self.imported.is_empty() && self.unchanged.is_empty() {
            out.push_str("\n  nothing to import");
        }
        for (table, n) in &self.dropped {
            let _ = write!(out, "\n  not carried to 1.14.7, {table}: {n} row(s)");
        }
        if !self.skipped.is_empty() {
            let _ = write!(out, "\nskipped ({}):", self.skipped.len());
            for s in &self.skipped {
                let _ = write!(out, "\n  {} {}: {}", s.kind, s.id, s.reason);
            }
        }
        for note in &self.notes {
            let _ = write!(out, "\nnote: {note}");
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_and_json() {
        let mut report = Report::new("revived", true);
        assert_eq!(
            report.text(),
            "revived import (dry run: nothing written):\n  nothing to import"
        );
        report.count("accounts");
        report.count("accounts");
        report.same("invites");
        report.dropped.insert("characters".into(), 3);
        report.skip("account", "UID-2", "it has no name");
        report.note("hello");
        assert_eq!(
            report.text(),
            "revived import (dry run: nothing written):\n  imported accounts: 2\n  already there invites: 1\n  \
             not carried to 1.14.7, characters: 3 row(s)\nskipped (1):\n  account UID-2: it has no name\nnote: hello"
        );
        assert_eq!(
            serde_json::to_string(&report).unwrap(),
            r#"{"source":"revived","dry_run":true,"imported":{"accounts":2},"unchanged":{"invites":1},"skipped":[{"kind":"account","id":"UID-2","reason":"it has no name"}],"dropped":{"characters":3},"notes":["hello"]}"#
        );
    }
}
