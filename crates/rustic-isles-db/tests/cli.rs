// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The argument parser (spec §3; §16.2 D1).
#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::path::PathBuf;

use rustic_isles_db::{Area, Command, DbError, LegacyStarter, parse_args};

fn parse(line: &str) -> Result<Command, DbError> {
    parse_args(line.split_whitespace().map(str::to_owned), LegacyStarter::default())
}

fn error(line: &str) -> String {
    parse(line).unwrap_err().to_string()
}

#[test]
fn migrate_takes_every_flag() {
    let Command::Migrate { opts, json } = parse(
        "migrate --from M --to T --node eu-west --area auth,characters --area bounties --dry-run --skip-invalid \
         --game-data-dir G --allow-remote-postgres --json",
    )
    .unwrap() else {
        panic!("not migrate");
    };
    assert!(json && opts.dry_run && opts.skip_invalid && opts.allow_remote_postgres);
    assert_eq!((opts.from_uri.as_str(), opts.to_target.as_str()), ("M", "T"));
    assert_eq!(opts.node.to_string(), "eu-west");
    assert_eq!(
        opts.areas.into_iter().collect::<Vec<_>>(),
        vec![Area::Auth, Area::Characters, Area::Bounties]
    );
    assert_eq!(opts.game_data_dir, Some(PathBuf::from("G")));
}

#[test]
fn verify_parses() {
    let Command::Verify { opts, json } = parse("verify --against M --to T").unwrap() else {
        panic!("not verify");
    };
    assert_eq!((opts.from_uri.as_str(), json), ("M", false));
    assert!(opts.areas.is_empty());
    assert_eq!(opts.effective_areas().len(), 7);
}

#[test]
fn help_errors_and_requirements() {
    assert_eq!(parse("").unwrap(), Command::Help);
    assert_eq!(parse("--help").unwrap(), Command::Help);
    assert_eq!(parse("migrate --from M --help --bogus").unwrap(), Command::Help);
    assert_eq!(
        error("unknown"),
        "invalid argument: unknown subcommand \"unknown\"; expected 'migrate', 'verify' or 'replay'"
    );
    assert!(error("migrate --area nope").contains(
        "unknown area \"nope\"; valid areas: auth, characters, inventory, progression, bounties, parties, reforged"
    ));
    assert_eq!(
        error("migrate --from M"),
        "invalid argument: missing required --to <sqlite-file|postgres-url>"
    );
    assert!(matches!(
        parse("migrate --from M --dry-run").unwrap(),
        Command::Migrate { .. }
    ));
    assert_eq!(
        error("migrate --to T"),
        "invalid argument: missing required --from <mongodb://...>"
    );
    assert_eq!(
        error("verify --against"),
        "invalid argument: --against requires a MongoDB URI"
    );
    assert_eq!(
        error("migrate --bogus"),
        "invalid argument: unexpected flag \"--bogus\""
    );
    assert!(error("export-mongo --from F").contains("unknown subcommand \"export-mongo\""));
    assert!(error("migrate --from M --to T --node ''").starts_with("invalid argument: invalid --node"));
    // A value flag takes the next argument whatever it is
    let Command::Migrate { opts, .. } = parse("migrate --from M --to --dry-run").unwrap() else {
        panic!("not migrate");
    };
    assert_eq!(opts.to_target, "--dry-run");
    assert!(!opts.dry_run);
    assert!(matches!(parse("replay --capture x"), Err(DbError::Replay(_))));
    assert_eq!(Area::parse(" Wallets ").unwrap(), Area::Bounties);
}

#[test]
fn options_debug_hides_passwords() {
    let Command::Migrate { opts, .. } =
        parse("migrate --from mongodb://u:hunter2@h/db --to postgres://p:s3cret@127.0.0.1/x").unwrap()
    else {
        panic!("not migrate");
    };
    let text = format!("{opts:?}");
    assert!(!text.contains("hunter2") && !text.contains("s3cret"), "{text}");
    assert!(text.contains("u:***@h/db"));
}
