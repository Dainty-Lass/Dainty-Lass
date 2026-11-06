//! `tests/golden/chat/chat.json` (`routes/friends.ts` and `routes/guild.ts` on Node 22, `run.sh`) against
//! the MIT module: status, content type and body text. Restricted's stand-in auth (`x-test-user`) becomes the real
//! credential: a user is that player's game token, an empty one a game server key naming no player, absent nothing.
//!
//! Node's recording read `new Date()` as `2026-10-20T12:00:<n>.000Z`, n counting every read; restricted injects that
//! counter into the chat routes, which the composed module has no seam for. Each request here carries the replay
//! clock at `12:00:<k>` (k the requests so far), and ISO times in the bodies are compared as `<iso>`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{Caller, Mit, Tally, credentials, seed_account, send};
use regex::Regex;
use serde_json::{Value, json};

#[tokio::test(flavor = "multi_thread")]
async fn chat() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/chat/chat.json");
    let steps: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mit = Mit::start(&[], |_| {}).await;
    let store = mit.store();
    for user in ["alice", "bob", "carol"] {
        seed_account(&store, user, user).await;
    }
    let iso = Regex::new(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z").unwrap();
    let mut tally = Tally::new("chat");
    tally.note(
        "ISO times in bodies compared as <iso>: restricted injects Node's per-read clock, the module has no seam",
    );
    for (n, step) in steps.iter().enumerate() {
        let name = step["name"].as_str().unwrap();
        let (method, path) = (step["method"].as_str().unwrap(), step["path"].as_str().unwrap());
        let mut headers = match step["user"].as_str() {
            None => credentials(Caller::Nobody),
            Some("") => credentials(Caller::Gameserver(None)),
            Some(user) => credentials(Caller::Player(user)),
        };
        headers.push((
            "x-isles-replay-now-ms".into(),
            format!("2026-10-20T12:00:{:02}.000Z", n.min(59)),
        ));
        let body: Option<(String, String)> = match &step["body"] {
            Value::Null => None,
            Value::Object(raw) if raw.contains_key("raw") => Some((
                raw["contentType"].as_str().unwrap().to_owned(),
                raw["raw"].as_str().unwrap().to_owned(),
            )),
            other => Some(("application/json".to_owned(), serde_json::to_string(other).unwrap())),
        };
        if let Some((content_type, _)) = &body {
            headers.push(("content-type".into(), content_type.clone()));
        }
        let reply = send(
            mit.backend,
            method,
            path,
            &headers,
            body.as_ref().map(|(_, b)| b.as_bytes()),
        )
        .await;
        let want = &step["expect"];
        let mask = |text: &str| iso.replace_all(text, "<iso>").into_owned();
        let expected = json!({ "status": want["status"], "content-type": want["contentType"],
            "body": mask(want["body"].as_str().unwrap()) });
        let actual = json!({ "status": reply.status, "content-type": reply.header("content-type"),
            "body": mask(&reply.body) });
        tally.step(n + 1, name, method, path, expected, actual);
    }
    tally.finish();
}
