// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! `dainty-host history` and `revert` (DD23, DH3b), and the console's and dashboard's `history` and `revert`:
//! one [`Job`] for all of them, run by [`run`] against the server's databases (`dainty_admin::history`).
//!
//! The host kit has the machine and `admin.env`, so these need no key, as `admin key` does not. A change they
//! make is logged as `host:console`. A dry run keeps nothing: `--apply <plan id>` takes the same selection
//! again, plans it again and applies only if the plan has the id the dry run showed, so what runs is what
//! was shown (`PlanChanged` otherwise: run the dry run again).
//!
//! The words are `dainty_admin::history_text`'s, so the console, the launcher and the admin API say the same.

use std::process::ExitCode;

use crate::Outcome;
use crate::cli::{HistoryArgs, HistoryCommand, HistoryFilterArgs, RevertArgs};
use crate::error::HostError;
use crate::paths::Paths;

/// The change log's actor for the host kit's commands, the console and the dashboard: `host:console`.
pub const ACTOR: &str = "console";

/// Which changes, as typed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Select {
    /// Only changes to this account.
    pub account: Option<String>,
    /// Only changes made by this actor.
    pub actor: Option<String>,
    /// Only changes from this time on.
    pub since: Option<String>,
    /// Only changes before this time.
    pub until: Option<String>,
}

impl From<&HistoryFilterArgs> for Select {
    fn from(args: &HistoryFilterArgs) -> Self {
        Self {
            account: args.account.clone(),
            actor: args.actor.clone(),
            since: args.since.clone(),
            until: args.until.clone(),
        }
    }
}

/// One `history` or `revert` job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
    /// A page of history, newest first. The limit is as typed; [`run`] checks it.
    Page {
        /// Which changes.
        select: Select,
        /// How many changes a page shows, as typed.
        limit: String,
        /// Where the page starts: the cursor the previous page printed.
        cursor: Option<String>,
    },
    /// Each log's hash chain.
    Verify,
    /// A dry run, or with a plan id the apply.
    Revert {
        /// Which changes to undo.
        select: Select,
        /// The plan id of the dry run to apply; a dry run when `None`.
        apply: Option<String>,
    },
}

/// Where the job came from, for the commands it prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// `dainty-host history` or `revert`.
    Cli,
    /// The console or the dashboard's prompt.
    Console,
}

/// What a job printed, and whether it ended well (`history verify` does not when a chain is broken).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Done {
    /// What to print.
    pub lines: Vec<String>,
    /// False when the job failed, or `history verify` found a broken chain.
    pub ok: bool,
}

/// The console's `history` usage line.
pub const HISTORY_USAGE: &str = "usage: history [--account id] [--actor who] [--since time] [--until time] [--limit N] [--cursor c] | history verify";
/// The console's `revert` usage line.
pub const REVERT_USAGE: &str =
    "usage: revert (--actor who | --account id | --since time) [--until time] [--dry-run | --apply <plan id>]";

/// The console's `history ...` and `revert ...` words (the line is already split on white space).
pub fn parse_console(word: &str, args: &[&str]) -> Result<Job, String> {
    let revert = word == "revert";
    let usage = if revert { REVERT_USAGE } else { HISTORY_USAGE };
    if !revert && args == ["verify"] {
        return Ok(Job::Verify);
    }
    // Text only, so nothing typed reaches the database layer or the terminal again as a control sequence
    if args.iter().any(|arg| arg.chars().any(char::is_control)) {
        return Err("a command is text only".into());
    }
    let mut select = Select::default();
    let (mut limit, mut cursor, mut apply, mut dry) = ("50".to_owned(), None, None, false);
    let mut words = args.iter().copied();
    while let Some(word) = words.next() {
        let (name, inline) = match word.split_once('=') {
            Some((name, value)) => (name, Some(value)),
            None => (word, None),
        };
        if name == "--dry-run" && revert && inline.is_none() {
            dry = true;
            continue;
        }
        let value = inline.or_else(|| words.next()).map(str::to_owned);
        match name {
            "--account" => select.account = Some(value.ok_or(usage)?),
            "--actor" => select.actor = Some(value.ok_or(usage)?),
            "--since" => select.since = Some(value.ok_or(usage)?),
            "--until" => select.until = Some(value.ok_or(usage)?),
            "--limit" if !revert => limit = value.ok_or(usage)?,
            "--cursor" if !revert => cursor = Some(value.ok_or(usage)?),
            "--apply" if revert => apply = Some(value.ok_or(usage)?),
            _ => return Err(usage.to_owned()),
        }
    }
    if revert && dry && apply.is_some() {
        return Err("--dry-run and --apply don't go together".into());
    }
    Ok(if revert {
        Job::Revert { select, apply }
    } else {
        Job::Page { select, limit, cursor }
    })
}

/// Runs the job against the server's databases, reading `admin.env`; the error is the line to show.
#[cfg(feature = "admin")]
pub async fn run(paths: &Paths, job: &Job, surface: Surface) -> Result<Done, String> {
    use dainty_admin::history::{self, HistorySet, Selection};
    use dainty_admin::history_text;

    let selection = |select: &Select| Selection {
        account: select.account.clone(),
        actor: select.actor.clone(),
        since: select.since.clone(),
        until: select.until.clone(),
    };
    let prefix = match surface {
        Surface::Cli => "dainty-host ",
        Surface::Console => "",
    };
    let env = paths.config.join("admin.env");
    if !env.is_file() {
        return Err("there is no admin.env yet: run dainty-host setup first".into());
    }
    let node = history::node_database(Some(&env))?;
    let set = HistorySet::open(&node).await.map_err(|e| e.to_string())?;
    let now = dainty_admin::now_ms();
    match job {
        Job::Page { select, limit, cursor } => {
            let limit = limit
                .trim()
                .parse::<i64>()
                .ok()
                .filter(|n| (1..=history::MAX_PAGE).contains(n))
                .ok_or(format!("--limit is a whole number from 1 to {}", history::MAX_PAGE))?;
            let filter = selection(select).to_filter(now).map_err(|e| e.to_string())?;
            let page = set
                .query(&filter, cursor.as_deref(), limit)
                .await
                .map_err(|e| e.to_string())?;
            let flags = history_text::flags(&filter);
            let again = format!("{prefix}history{}{flags}", if flags.is_empty() { "" } else { " " });
            Ok(Done {
                lines: history_text::entries(&page, &again),
                ok: true,
            })
        }
        Job::Verify => {
            let all = set.verify().await.map_err(|e| e.to_string())?;
            Ok(Done {
                lines: history_text::verified(&all),
                ok: history_text::all_verified(&all),
            })
        }
        Job::Revert { select, apply } => {
            let filter = selection(select).to_filter(now).map_err(|e| e.to_string())?;
            let command = format!("{prefix}revert");
            match apply {
                None => {
                    let plan = set.dry_run(&filter).await.map_err(|e| e.to_string())?;
                    Ok(Done {
                        lines: history_text::plan(&plan, &command),
                        ok: true,
                    })
                }
                Some(id) => {
                    let actor = history::host_actor(ACTOR);
                    let applied = set.apply(&actor, &filter, id.trim()).await.map_err(|e| e.to_string())?;
                    Ok(Done {
                        lines: history_text::applied(&applied, &actor.as_text()),
                        ok: true,
                    })
                }
            }
        }
    }
}

/// Fails: this build has no admin service, which the history tools come with.
#[cfg(not(feature = "admin"))]
pub async fn run(_paths: &Paths, _job: &Job, _surface: Surface) -> Result<Done, String> {
    Err("this build of the host kit has no admin service, which the history tools come with".into())
}

fn run_cli(paths: &Paths, job: &Job) -> Outcome {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| HostError::message(format!("could not start the runtime: {e}")))?;
    let done = runtime
        .block_on(run(paths, job, Surface::Cli))
        .map_err(HostError::message)?;
    for line in &done.lines {
        println!("{line}");
    }
    Ok(if done.ok { ExitCode::SUCCESS } else { ExitCode::FAILURE })
}

/// Runs `dainty-host history`.
pub fn run_history(paths: &Paths, args: &HistoryArgs) -> Outcome {
    let job = match args.command {
        Some(HistoryCommand::Verify) => Job::Verify,
        None => Job::Page {
            select: Select::from(&args.filter),
            limit: args.limit.clone(),
            cursor: args.cursor.clone(),
        },
    };
    run_cli(paths, &job)
}

/// Without `--apply` this is a dry run: nothing is written until a plan id is given.
pub fn run_revert(paths: &Paths, args: &RevertArgs) -> Outcome {
    run_cli(
        paths,
        &Job::Revert {
            select: Select::from(&args.filter),
            apply: args.apply.clone(),
        },
    )
}
