//! 1.12.0 serves the 1.14.7 routes the Node 1.14.7 backend recorded, except where RU3c's diff says it differs: the
//! stored weapon level is served (`weapons_at_base_level` off), a gameserver call that names no player or no character
//! is not resolved to the configured account (`no_player_policy` off), and the reforge routes and the empty-characterId
//! regex are 1.14.7's, so 1.12 answers its 404. Unverified against 1.12 traffic (DD9): these expectations are derived
//! from the 1.14.7 recording and the diff, not recorded.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use std::collections::BTreeMap;

use isles_core::Settings;
use isles_core::areas::inventory::testkit::{Harness, Reply, SCENARIOS, recorded, replay};

fn by_name(replies: Vec<(String, Reply)>) -> BTreeMap<String, Reply> {
    replies.into_iter().collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn it_differs_from_1_14_7_only_where_the_diff_says() {
    let dir = tempfile::tempdir().unwrap();
    let seed: serde_json::Value = serde_json::from_str(SCENARIOS).unwrap();
    // The same host setting; 1.12.0 ignores it (its policy is off)
    let settings = Settings::from_pairs([("GAMESERVER_NO_PLAYER_USER_ID", "u1")]);
    let harness = Harness::start(dir.path(), isles_1_12_0::version(), settings, &seed["seed"]).await;
    let have = by_name(replay(&harness).await);
    let node = by_name(recorded());
    assert_eq!(have.len(), node.len());

    // Not on 1.12.0 at all: the reforge routes and the regex route (the named route needs a characterId)
    for name in have
        .keys()
        .filter(|n| n.starts_with("reforge, ") && !n.ends_with("unauthenticated"))
    {
        assert_eq!((have[name].status, have[name].body.as_str()), (404, ""), "{name}");
    }
    assert_eq!(have["reforge, unauthenticated"].status, 404);
    assert_eq!(have["migration stub, empty characterId (regex)"].status, 404);

    // The stored level is served
    let level = |name: &str| -> String {
        let body = &have[name].body;
        let at = body
            .find("CurrentLevel\\\":")
            .unwrap_or_else(|| panic!("{name}: {body}"));
        body[at + 15..].chars().take_while(char::is_ascii_digit).collect()
    };
    assert_eq!(level("read, gameserver"), "5");
    assert_eq!(level("tx, save with a newer version"), "6");
    assert_eq!(level("tx, player bearer can save"), "7");
    assert_eq!(level("item, newer version"), "9");

    // No player, no character: nothing is resolved
    assert_eq!(have["read, no characterId resolves to the first character"].status, 400);
    assert_eq!(
        have["tx, empty characterId resolves to the first character"].body,
        r#"{"code":null,"message":"Invalid inventory request: characterId must be a non-empty string","payload":{}}"#
    );
    assert_eq!(
        have["tx, empty accountId and characterId from a bearer-less gameserver"].status,
        400
    );
    // ...so the gameserver's INVALID stays the sentinel and the call is a no-op echo
    assert_eq!(
        have["tx, a gameserver's sentinel account becomes the configured one"].body,
        r#"{"createdInstancedItems":[],"updatedInstancedItems":[],"updatedStackedItems":[{"catalogId":"MAT_A","quantity":1}],"removedInstancedItems":"odd"}"#
    );

    // Everything else is the recording, apart from what those differences left behind
    let level_masked = |text: &str| {
        let marker = "CurrentLevel\\\":";
        let mut out = String::new();
        let mut rest = text;
        while let Some(at) = rest.find(marker) {
            out.push_str(&rest[..at + marker.len()]);
            out.push('0');
            rest = rest[at + marker.len()..].trim_start_matches(|c: char| c.is_ascii_digit());
        }
        out.push_str(rest);
        out
    };
    // (an empty body names no character, which 1.14.7 resolved for the gameserver)
    assert_eq!(
        have["tx, no body"].body,
        r#"{"code":null,"message":"Invalid inventory request: characterId must be a non-empty string","payload":{}}"#
    );
    let derived = [
        "tx, no body",
        "migration stub, empty characterId (regex)",
        "read, no characterId resolves to the first character",
        "tx, empty characterId resolves to the first character",
        "tx, empty accountId and characterId from a bearer-less gameserver",
        "tx, a gameserver's sentinel account becomes the configured one",
        "read after the transactions",
    ];
    for (name, want) in &node {
        if name.starts_with("reforge, ") || derived.contains(&name.as_str()) {
            continue;
        }
        let mut have = have[name].clone();
        have.body = level_masked(&have.body);
        assert_eq!(&have, want, "{name}");
    }
}
