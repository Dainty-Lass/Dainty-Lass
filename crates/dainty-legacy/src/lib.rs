// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! dainty-legacy: imports old installs into Dainty Lass (docs/port/spec-legacy-import.md). A Dauntless Revived
//! 1.4.4 server's accounts, bans, Discord links and invites, or one player's offline save, go into the 1.14.7
//! database rustic-isles made; a Reforged Node kit's `host.json`, folders and MongoDB are upgraded in place.
//!
//! Two entry points run the same imports: the `legacy` plugin's console commands ([`plugin`]) and the
//! `dainty-legacy import <kind>` command line ([`cli`]).

#![cfg_attr(
    test,
    allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")
)]

pub mod args;
pub mod cli;
pub mod data;
pub mod drsave;
mod envfile;
pub mod error;
pub mod plugin;
pub mod reforged;
pub mod report;
mod source;
mod target;

use std::path::Path;

pub use args::{ImportArgs, Kind, Target};
pub use error::{ImportError, USAGE};
pub use report::{Report, Skipped};
pub use target::SOFT_DELETE;

use source::{Holder, Now, Origin};

const KEYS_NOTE: &str =
    "1.4.4 account keys are not carried: 1.14.7 keeps keys as HMACs, so players sign in through the launcher";
const ADMINS_NOTE: &str = "1.4.4 admins were imported as players: give them their roles in the admin UI";
const TIMEOUTS_NOTE: &str = "this 1.14.7 database keeps no timeouts yet (rustic-isles before migration 0610), so \
                             timed bans were imported as bans: update the server and import again, or lift them in \
                             the admin UI when they run out";

/// Runs one import. `node` names the accounts' home unless `--node` does.
pub async fn run(args: ImportArgs, node: &str) -> Result<Report, ImportError> {
    let node = args.node.as_ref().map_or(node, |n| n.as_str()).to_owned();
    match args.kind {
        Kind::Reforged => reforged::run(&args).await,
        Kind::Revived | Kind::OfflineSave => import_1_4_4(args, node).await,
    }
}

/// Checks a 1.4.4 server database as an import does, without a target: what the fuzz target drives.
pub fn read_server_database(path: &Path) -> Result<usize, ImportError> {
    source::load(path, Origin::Server, &Holder::Server, &Now::current()).map(|s| s.accounts.len())
}

/// Checks a bare offline database or a `.drsave` as an import does, without a target.
pub fn read_offline_save(path: &Path) -> Result<usize, ImportError> {
    load_save(path, &Now::current()).map(|s| s.accounts.len())
}

fn load_save(path: &Path, now: &Now) -> Result<source::Source, ImportError> {
    if !path.is_file() {
        return Err(ImportError::io(path, "there is no file here"));
    }
    if !drsave::is_container(path)? {
        return source::load(path, Origin::Offline, &Holder::Bare, now);
    }
    let dir = tempfile::Builder::new()
        .prefix("dainty-legacy-save-")
        .tempdir()
        .map_err(|e| ImportError::Io(format!("a private temporary folder: {e}")))?;
    let save = drsave::extract(path, dir.path())?;
    let holder = Holder::Container {
        user_id: &save.user_id,
        username: &save.username,
        key: save.key.expose(),
    };
    source::load(&save.database, Origin::Offline, &holder, now)
}

async fn import_1_4_4(args: ImportArgs, node: String) -> Result<Report, ImportError> {
    let Some(target) = &args.target else {
        return Err(ImportError::Usage(
            "which 1.14.7 database to import into? --to or --env-file".into(),
        ));
    };
    let config = envfile::database(target)?;
    let now = Now::current();
    let kind = args.kind;
    let path = args.source.clone();
    let read_now = now.clone();
    let source = tokio::task::spawn_blocking(move || match kind {
        Kind::OfflineSave => load_save(&path, &read_now),
        _ => source::load(&path, Origin::Server, &Holder::Server, &read_now),
    })
    .await
    .map_err(|_| ImportError::Io("the import's reading task stopped".into()))??;

    let without_link = source.accounts.iter().filter(|a| a.link.is_none()).count();
    let database = target::open(config).await?;
    let label = kind.as_str();
    let outcome = target::write(
        &database,
        target::Plan {
            accounts: source.accounts,
            invites: source.invites,
            node,
            source: label,
            now,
            dry_run: args.dry_run,
        },
    )
    .await?;

    let mut report = Report::new(label, args.dry_run);
    report.imported = outcome.imported;
    report.unchanged = outcome.unchanged;
    report.skipped = outcome.skipped;
    report.dropped = source.dropped;
    report.note(KEYS_NOTE);
    if report.imported.contains_key("1.4.4 admins imported as players") {
        report.note(ADMINS_NOTE);
    }
    if report.imported.contains_key("temporary bans made permanent") {
        report.note(TIMEOUTS_NOTE);
    }
    if kind == Kind::OfflineSave {
        if without_link > 0 {
            report.note(
                "the save's account had no Discord link, so its player has no way to sign in yet: link one from \
                 the admin UI",
            );
        }
    } else if without_link > 0 {
        report.note(format!(
            "{without_link} 1.4.4 account(s) had no Discord link, so their players have no way to sign in yet: \
             link one from the admin UI"
        ));
    }
    Ok(report)
}
