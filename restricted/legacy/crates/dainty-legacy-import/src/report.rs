//! What an import did: counts by what was carried, what was skipped and why, and what doesn't carry across game
//! versions. It holds ids and names a host already sees in the admin UI, never a key, hash or secret.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
/// A source row the import left out.
pub struct Skipped {
    /// `account`, `discord_link`, `invite`, `folder`...
    pub kind: &'static str,
    /// The row's id.
    pub id: String,
    /// Why it was left out.
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
/// What an import did, for the console and as JSON.
pub struct Report {
    /// `revived`, `offline-save` or `reforged`
    pub source: &'static str,
    /// Whether this was a dry run.
    pub dry_run: bool,
    /// What was written (or would be, on a dry run)
    pub imported: BTreeMap<&'static str, usize>,
    /// Rows already in the target from an earlier run, left as they are
    pub unchanged: BTreeMap<&'static str, usize>,
    /// Rows left out, with why.
    pub skipped: Vec<Skipped>,
    /// Source rows by table that have no 1.14.7 equivalent and were left behind
    pub dropped: BTreeMap<String, usize>,
    /// Things the host should know or do next
    pub notes: Vec<String>,
}

impl Report {
    /// An empty report for `source`.
    pub fn new(source: &'static str, dry_run: bool) -> Self {
        Self {
            source,
            dry_run,
            ..Self::default()
        }
    }

    /// Counts one `what` written.
    pub fn count(&mut self, what: &'static str) {
        *self.imported.entry(what).or_default() += 1;
    }

    /// Counts one `what` already in the target.
    pub fn unchanged(&mut self, what: &'static str) {
        *self.unchanged.entry(what).or_default() += 1;
    }

    /// Records a row left out.
    pub fn skip(&mut self, kind: &'static str, id: impl Into<String>, reason: impl Into<String>) {
        self.skipped.push(Skipped {
            kind,
            id: id.into(),
            reason: reason.into(),
        });
    }

    /// Adds a note for the host.
    pub fn note(&mut self, note: impl Into<String>) {
        self.notes.push(note.into());
    }

    /// How many `what` were written.
    pub fn imported(&self, what: &str) -> usize {
        self.imported.get(what).copied().unwrap_or(0)
    }

    /// The console's text: one line per count, then the skipped rows and the notes.
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
        let section = |out: &mut String, title: &str, counts: &BTreeMap<&'static str, usize>| {
            for (what, n) in counts {
                let _ = write!(out, "\n  {title} {what}: {n}");
            }
        };
        section(&mut out, "imported", &self.imported);
        section(&mut out, "already there", &self.unchanged);
        if self.imported.is_empty() && self.unchanged.is_empty() {
            out.push_str("\n  nothing to import");
        }
        for (table, n) in &self.dropped {
            let _ = write!(out, "\n  not carried to 1.14.7, {table}: {n} row(s)");
        }
        if !self.skipped.is_empty() {
            let _ = write!(out, "\nskipped ({}):", self.skipped.len());
            for skip in &self.skipped {
                let _ = write!(out, "\n  {} {}: {}", skip.kind, skip.id, skip.reason);
            }
        }
        for note in &self.notes {
            let _ = write!(out, "\nnote: {note}");
        }
        out
    }
}
