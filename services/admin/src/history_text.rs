//! History and revert in words (DH3b): what `reforged-host history` and `revert`, the console and the launcher
//! print. Row bodies are never printed, only which row and what would happen to it; the admin API and the
//! launcher's page carry them, redacted (`history::redact`).

use store::Filter;
use store::revert::{Conflict, RevertKind, RowConflict};

use crate::history::{Applied, Page, Plan, Verified};
use crate::input::iso;

/// The longest row key printed in a line; a longer one ends in `...`.
const KEY_SHOWN: usize = 72;

fn shown_key(key: &serde_json::Value) -> String {
    let text = key.to_string();
    if text.chars().count() <= KEY_SHOWN {
        return text;
    }
    let cut: String = text.chars().take(KEY_SHOWN).collect();
    format!("{cut}...")
}

/// A value as a command line takes it: bare when it is plain, in quotes otherwise.
fn word(value: &str) -> String {
    if !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b":_.@/+=,-".contains(&b))
    {
        return value.to_owned();
    }
    let mut out = String::from('"');
    for c in value.chars() {
        if c == '"' || c == '\u{5c}' {
            out.push('\u{5c}');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// The options that make `filter` again: `--actor admin:ab12cd34 --since 2026-10-27T00:00:00Z`.
pub fn flags(filter: &Filter) -> String {
    let mut out = Vec::new();
    if let Some(account) = &filter.account {
        out.push(format!("--account {}", word(account)));
    }
    if let Some(actor) = &filter.actor {
        out.push(format!("--actor {}", word(&actor.as_text())));
    }
    if let Some(since) = filter.since {
        out.push(format!("--since {}", iso(since)));
    }
    if let Some(until) = filter.until {
        out.push(format!("--until {}", iso(until)));
    }
    out.join(" ")
}

/// A page of history, newest first. `more` is the command that read it, without the cursor.
pub fn entries(page: &Page, more: &str) -> Vec<String> {
    if page.entries.is_empty() {
        return vec!["Nothing in the history matches.".into()];
    }
    let mut out = vec![format!(
        "{:<21} {:<7} {:<26} {:<12} {:<22} ROW",
        "TIME", "DB", "ACTOR", "CHANGE", "TABLE"
    )];
    for e in &page.entries {
        out.push(format!(
            "{:<21} {:<7} {:<26} {:<12} {:<22} {}",
            iso(e.entry.at),
            e.database,
            e.entry.actor,
            e.entry.op,
            e.entry.table_name,
            shown_key(&serde_json::from_str(&e.entry.row_key).unwrap_or_default()),
        ));
    }
    if let Some(next) = &page.next {
        out.push(format!("More, older: {more} --cursor {next}"));
    }
    out
}

fn what(kind: RevertKind) -> &'static str {
    match kind {
        RevertKind::Insert => "bring back (it was deleted)",
        RevertKind::Update => "put its earlier values back",
        RevertKind::Delete => "remove (it was added)",
        RevertKind::SoftDelete => "delete (it was added; kept until the retention period ends)",
    }
}

fn why(conflict: &RowConflict) -> String {
    match &conflict.conflict {
        Conflict::ChangedSince { entry, actor, at } => {
            format!("changed since by {actor} at {} (entry {entry})", iso(*at))
        }
        Conflict::NotAsLogged => "it is not as the log left it (a migration or a write around the log)".into(),
        Conflict::KeyChanged => "its key was changed in the history, so the log names it twice".into(),
        Conflict::SchemaChanged { detail } => format!("its table changed ({detail})"),
    }
}

/// A dry run: what would change, what stays as it is and why, what is already right. `apply` is the command
/// that applies exactly this, for the last line.
pub fn plan(plan: &Plan, apply: &str) -> Vec<String> {
    let mut out = vec![
        format!("Revert plan {}", plan.id),
        format!(
            "  {} log entries match {}.",
            plan.entries(),
            match flags(&plan.selection) {
                f if f.is_empty() => "everything".to_owned(),
                f => f,
            }
        ),
    ];
    let line = |database: &str, table: &str, key: &serde_json::Value, text: String| {
        format!("  {database:<7} {table:<22} {:<34} {text}", shown_key(key))
    };
    out.push(format!("Would change {} rows:", plan.actions()));
    for part in &plan.parts {
        for action in &part.plan.actions {
            out.push(line(
                &part.database,
                &action.table,
                &action.key,
                what(action.kind).into(),
            ));
        }
    }
    if plan.conflicts() > 0 {
        out.push(format!("Stays as it is ({} rows):", plan.conflicts()));
        for part in &plan.parts {
            for conflict in &part.plan.conflicts {
                out.push(line(&part.database, &conflict.table, &conflict.key, why(conflict)));
            }
        }
    }
    if plan.unchanged() > 0 {
        out.push(format!("Unchanged, already as they were ({} rows):", plan.unchanged()));
        for part in &plan.parts {
            for row in &part.plan.unchanged {
                out.push(
                    line(&part.database, &row.table, &row.key, String::new())
                        .trim_end()
                        .to_owned(),
                );
            }
        }
    }
    if plan.actions() == 0 {
        out.push("Nothing to revert.".into());
    } else {
        out.push("Nothing is changed by a dry run. To apply exactly this:".into());
        out.push(format!("  {apply} {} --apply {}", flags(&plan.selection), plan.id));
    }
    out
}

/// What an apply did.
pub fn applied(done: &Applied, actor: &str) -> Vec<String> {
    let changed: usize = done.done.iter().map(|(_, o)| o.plan.actions.len()).sum();
    let mut out = vec![format!(
        "Reverted {changed} rows as {actor}, at {}.",
        done.done.first().map_or_else(String::new, |(_, o)| iso(o.at))
    )];
    for (database, outcome) in &done.done {
        out.push(format!("  {database}: {} rows", outcome.plan.actions.len()));
    }
    if done.plan.conflicts() > 0 {
        out.push(format!("{} rows stayed as they are:", done.plan.conflicts()));
        for part in &done.plan.parts {
            for conflict in &part.plan.conflicts {
                out.push(format!(
                    "  {:<7} {:<22} {}  {}",
                    part.database,
                    conflict.table,
                    shown_key(&conflict.key),
                    why(conflict)
                ));
            }
        }
    }
    out.push("The revert is in the history under its own entries: revert it the same way to undo it.".into());
    out
}

/// `history verify`: each log's chain.
pub fn verified(all: &[Verified]) -> Vec<String> {
    let mut out = Vec::new();
    for v in all {
        let head = v.verification.head.as_ref().map_or_else(
            || "no entries yet".to_owned(),
            |h| format!("head entry {} {}", h.id, h.hash),
        );
        match &v.verification.broken {
            None => out.push(format!(
                "{:<7} OK: {} entries chain from the start; {head}",
                v.database, v.verification.entries
            )),
            Some(broken) => out.push(format!(
                "{:<7} BROKEN after {} good entries: {broken}",
                v.database, v.verification.entries
            )),
        }
    }
    out
}

/// Whether every chain checked out.
pub fn all_verified(all: &[Verified]) -> bool {
    all.iter().all(|v| v.verification.broken.is_none())
}
