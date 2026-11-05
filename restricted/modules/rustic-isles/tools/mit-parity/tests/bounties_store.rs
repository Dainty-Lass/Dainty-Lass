//! `isles-core/tests/golden/bounties_store/1.14.7.json` (Node's bounty, cooldown, entitlement, wallet and store routers,
//! `record.cjs`) against the MIT module: restricted's `bounties_store::harness::replay_golden` comparison (status,
//! content type and body text; a token answer as `<token>`; Express's error page by its start), at the recording's
//! pinned clock, over the golden's game data.
//!
//! The recording replaced the character lookup (every known account has `char-1`) and the inventory transaction (a
//! ledger with a wallet holding 100 of a currency); restricted fakes both. The composed module runs the real ones, so
//! `p1` and `p2` get a `char-1` with an empty inventory and `p1` 100 Steel Marks before the first token, as
//! restricted's replay credits them.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{Caller, Mit, Tally, credentials, credit, seed_account, seed_character, send};
use serde_json::{Value, json};

#[tokio::test(flavor = "multi_thread")]
async fn bounties_store() {
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/isles-core/tests/golden/bounties_store");
    let golden: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("1.14.7.json")).unwrap()).unwrap();
    rustic_isles::replay::pin_now_for_tests(Some(1_793_016_000_000));
    let source = dir.join("game-data");
    let mit = Mit::start(&[], |target| {
        for entry in std::fs::read_dir(&source).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), target.join(entry.file_name())).unwrap();
        }
    })
    .await;
    let store = mit.store();
    let p1 = seed_account(&store, "p1", "p1").await;
    let p2 = seed_account(&store, "p2", "p2").await;
    seed_character(&store, &p1, "char-1", Some(("[]", "[]"))).await;
    seed_character(&store, &p2, "char-1-p2", Some(("[]", "[]"))).await;
    let mut tally = Tally::new("bounties_store");
    tally.note(
        "p2's character is char-1-p2 (character ids are unique in the store; Node's fake named every one char-1)",
    );
    let mut last_token = String::from("none");
    let mut credited = false;
    for (n, step) in golden["steps"].as_array().unwrap().iter().enumerate() {
        let name = step["name"].as_str().unwrap();
        let method = step["method"].as_str().unwrap();
        let path = step["path"].as_str().unwrap().replace("{token}", &last_token);
        let mut headers = match step["auth"].as_str() {
            None => credentials(Caller::Nobody),
            Some("gs") => credentials(Caller::Gameserver(None)),
            Some(other) => credentials(Caller::Player(other.strip_prefix("player:").unwrap_or(other))),
        };
        let body = step["body"].as_str();
        if body.is_some() {
            headers.push(("content-type".into(), "application/json".into()));
        }
        if !credited && path.starts_with("/token/") {
            credited = true;
            // A purchase token is signed at the module's clock and checked at the system's (auth §5.1, §14 item
            // 13), so a token minted under the pinned 2026-10-26 is already expired; restricted's fake tokens have no
            // expiry. From here on the clock is the system's.
            rustic_isles::replay::pin_now_for_tests(None);
            tally.note("clock unpinned from the first /token/ step: purchase tokens are checked at the system clock");
            credit(&store, &p1, "CURRENCY_MARKS_STEEL", 100.0).await;
        }
        let reply = send(mit.backend, method, &path, &headers, body.map(str::as_bytes)).await;
        let mut text = reply.body.clone();
        if path.starts_with("/token/") && reply.status == 200 {
            last_token = reply.json()["purchaseToken"].as_str().unwrap_or("none").to_owned();
            text = r#"{"purchaseToken":"<token>"}"#.to_owned();
        }
        let want_text = step["text"].as_str().unwrap();
        // Express's error page names the parser's message; only its status and type are the contract
        let page = |t: &str| {
            if t.starts_with("<!DOCTYPE html>") {
                "<!DOCTYPE html>…".to_owned()
            } else {
                t.to_owned()
            }
        };
        let expected =
            json!({ "status": step["status"], "content-type": step["contentType"], "body": page(want_text) });
        let actual =
            json!({ "status": reply.status, "content-type": reply.header("content-type"), "body": page(&text) });
        tally.step(n + 1, name, method, &path, expected, actual);
    }
    rustic_isles::replay::pin_now_for_tests(None);
    tally.finish();
}
