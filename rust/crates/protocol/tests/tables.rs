// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use protocol::account::{UsernameCheck, check_username, extract_account_key, is_plausible_account_key};
use protocol::invite::{Invite, parse_invite};
use protocol::status::{ServerStatus, parse_server_status};
use serde::Deserialize;
use serde_json::Value;

fn table<T: for<'a> Deserialize<'a>>(name: &str) -> T {
    let path = format!("{}/tests/cases/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[derive(Deserialize)]
struct Invites {
    accept: Vec<Accept>,
    reject: Vec<Reject>,
}

#[derive(Deserialize)]
struct Accept {
    input: String,
    invite: Invite,
}

#[derive(Deserialize)]
struct Reject {
    input: String,
    error: String,
}

#[test]
fn invites() {
    let cases: Invites = table("invite.json");
    assert!(cases.accept.len() > 30 && cases.reject.len() > 150);
    assert!(cases.reject.iter().filter(|case| case.error == "revived").count() > 30);
    for case in cases.accept {
        assert_eq!(parse_invite(&case.input), Ok(case.invite), "{:?}", case.input);
    }
    for case in cases.reject {
        // With the legacy feature a dauntless-revived:// invite answers as the same reforged-revival:// one
        #[cfg(feature = "legacy")]
        if case.error == "revived" {
            let reborn = reborn_spelling(&case.input);
            let expected = parse_invite(&reborn).map(|invite| Invite {
                version: protocol::invite::revived_version(),
                ..invite
            });
            assert_eq!(parse_invite(&case.input), expected, "{:?}", case.input);
            continue;
        }
        assert_eq!(
            parse_invite(&case.input).map_err(|error| error.as_str()),
            Err(case.error.as_str()),
            "{:?}",
            case.input
        );
    }
}

/// The input with `from` (any case) replaced by `to`, or None when `from` is not in it.
fn respell(input: &str, from: &str, to: &str) -> Option<String> {
    let at = input.to_ascii_lowercase().find(from)?;
    Some(format!("{}{to}{}", &input[..at], &input[at + from.len()..]))
}

/// The input with `version=<version>&` put first in the query.
fn with_version(input: &str, version: &str) -> String {
    input.replacen('?', &format!("?version={version}&"), 1)
}

#[test]
fn reforged_revival_invites_are_dauntless_ones_with_version_1_14_7() {
    use protocol::invite::revived_version;
    let cases: Invites = table("invite.json");
    let mut checked = 0;
    for case in &cases.accept {
        let unified = respell(&case.input, "reforged-revival", "dauntless").unwrap();
        assert_eq!(parse_invite(&unified), Ok(case.invite.clone()), "{unified:?}");
        assert_eq!(
            parse_invite(&with_version(&unified, "1.14.7")),
            Ok(case.invite.clone()),
            "{unified:?}"
        );
        assert_eq!(
            parse_invite(&with_version(&case.input, "1.14.7")),
            Ok(case.invite.clone())
        );
        // An alias never takes another game's version
        assert_eq!(
            parse_invite(&with_version(&case.input, "1.4.4")).map_err(|e| e.as_str()),
            Err("params"),
            "{:?}",
            case.input
        );
        let legacy = parse_invite(&with_version(&unified, "1.4.4"));
        if cfg!(feature = "legacy") {
            assert_eq!(
                legacy,
                Ok(Invite {
                    version: revived_version(),
                    ..case.invite.clone()
                })
            );
        } else {
            assert_eq!(legacy.map_err(|e| e.as_str()), Err("revived"));
        }
        // A version's alias reads as the version
        assert_eq!(
            parse_invite(&with_version(&unified, "reforged")),
            Ok(case.invite.clone())
        );
        assert_eq!(
            parse_invite(&with_version(&unified, "revived")),
            parse_invite(&with_version(&unified, "1.4.4"))
        );
        assert_eq!(
            parse_invite(&with_version(&case.input, "revived")).map_err(|e| e.as_str()),
            Err("params")
        );
        // Only a version this code knows; the old `game` names are not versions
        for unknown in ["1.4", "reborn", "legacy", "Reforged"] {
            assert_eq!(
                parse_invite(&with_version(&unified, unknown)).map_err(|e| e.as_str()),
                Err("params")
            );
        }
        checked += 1;
    }
    for case in cases
        .reject
        .iter()
        .filter(|case| case.error != "revived" && case.error != "too_long")
    {
        let Some(unified) = respell(&case.input, "reforged-revival", "dauntless") else {
            continue;
        };
        assert_eq!(
            parse_invite(&unified).map_err(|error| error.as_str()),
            Err(case.error.as_str()),
            "{unified:?}"
        );
        checked += 1;
    }
    assert!(checked > 150);
}

/// The input with Dauntless Revived's scheme (any case) replaced by Reforged's.
#[cfg(feature = "legacy")]
fn reborn_spelling(input: &str) -> String {
    respell(input, "dauntless-revived", "reforged-revival").unwrap()
}

#[derive(Deserialize)]
struct Status {
    input: Value,
    expected: Option<ServerStatus>,
    /// `JSON.stringify` of the TypeScript answer
    text: String,
}

#[test]
fn statuses() {
    let cases: Vec<Status> = table("status.json");
    assert!(cases.len() > 90);
    for case in cases {
        let parsed = parse_server_status(&case.input);
        assert_eq!(parsed, case.expected, "{}", case.input);
        // And the same JSON text the TypeScript hands the page
        assert_eq!(serde_json::to_string(&parsed).unwrap(), case.text, "{}", case.input);
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Usernames {
    check: Vec<Check>,
    plausible_key: Vec<Plausible>,
    extract_key: Vec<Extract>,
}

#[derive(Deserialize)]
struct Check {
    input: Value,
    result: String,
}

#[derive(Deserialize)]
struct Plausible {
    input: String,
    ok: bool,
}

#[derive(Deserialize)]
struct Extract {
    input: String,
    key: Option<String>,
}

#[test]
fn usernames_and_keys() {
    let cases: Usernames = table("username.json");
    for case in cases.check {
        // The page may send a non-string, which TypeScript reads as empty
        let result = case.input.as_str().map_or(UsernameCheck::Empty, check_username);
        assert_eq!(result.as_str(), case.result, "{:?}", case.input);
    }
    for case in cases.plausible_key {
        assert_eq!(is_plausible_account_key(&case.input), case.ok, "{:?}", case.input);
    }
    for case in cases.extract_key {
        assert_eq!(
            extract_account_key(&case.input),
            case.key.as_deref(),
            "{:?}",
            case.input
        );
    }
}
