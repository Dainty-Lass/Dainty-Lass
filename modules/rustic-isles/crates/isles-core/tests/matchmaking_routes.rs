//! Golden tests for the 18 routes of the matchmaking-parties group: `tests/golden/scenario.json` is replayed against
//! the Rust backend over real HTTP, and every answer (status, content type, `X-Mystic-Party-Revision`, body text with
//! its key order) must equal what Mystic Paradox's own routers answered to the same scenario, recorded in
//! `tests/golden/matchmaking-parties.json` by `tests/golden/capture.cjs` (the real `routes/matchmaking.ts` and
//! `routes/party.ts` on Node 22, in a container). Only the ids Node draws at random are normalised.
//!
//! The scenario runs on both engines when `REFORGED_TEST_POSTGRES_URL` names a server.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

mod support;

use std::sync::Arc;
use std::time::Duration;

use isles_core::areas::matchmaking::testkit::{FakeDeploy, Reply, send, serve, shared};
use regex::Regex;
use serde_json::{Value, json};

const SCENARIO: &str = include_str!("golden/scenario.json");
const GOLDEN: &str = include_str!("golden/matchmaking-parties.json");

/// The ids Node draws at random: UUIDs, and a party's id (`<hex>_<base64 build>` there, `<ULID>.<node>_<base64 build>`
/// here, which the chat rules need).
fn normalise(text: &str) -> String {
    let uuid = Regex::new(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}").unwrap();
    let party = Regex::new(r"(?:[0-9a-f]{32}|[0-9A-HJKMNP-TV-Z]{26}\.[a-z0-9-]+)_[A-Za-z0-9+/=]*").unwrap();
    party
        .replace_all(&uuid.replace_all(text, "<uuid>"), "<party>")
        .into_owned()
}

fn fill(value: &Value, answers: &std::collections::HashMap<String, Value>) -> Value {
    match value {
        Value::String(text) if text.starts_with('$') => {
            let (name, field) = text[1..].split_once('.').unwrap();
            answers[name][field].clone()
        }
        Value::Object(map) => Value::Object(map.iter().map(|(k, v)| (k.clone(), fill(v, answers))).collect()),
        other => other.clone(),
    }
}

/// One scenario step, sent raw when it has `raw`.
async fn replay(
    address: std::net::SocketAddr,
    step: &Value,
    answers: &std::collections::HashMap<String, Value>,
) -> Reply {
    let (method, path) = (step["method"].as_str().unwrap(), step["path"].as_str().unwrap());
    let user = step.get("user").and_then(Value::as_str);
    if let Some(raw) = step.get("raw").and_then(Value::as_str) {
        return send_raw(address, method, path, user, raw).await;
    }
    let body = step.get("body").map(|b| fill(b, answers));
    send(address, method, path, user, body).await
}

async fn send_raw(address: std::net::SocketAddr, method: &str, path: &str, user: Option<&str>, raw: &str) -> Reply {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let user = user.map(|u| format!("x-test-user: {u}\r\n")).unwrap_or_default();
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n{user}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{raw}",
        raw.len()
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).await.unwrap();
    let text = String::from_utf8_lossy(&bytes).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap();
    let mut lines = head.lines();
    let status = lines.next().unwrap().split(' ').nth(1).unwrap().parse().unwrap();
    let headers = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_owned()))
        .collect();
    Reply {
        status,
        headers,
        body: body.to_owned(),
    }
}

/// Runs the scenario; the answers by step name, in order.
async fn run(target: &support::Target) -> Vec<(String, Reply)> {
    for name in ["alice", "bob", "carol"] {
        target.account(name).await;
    }
    let deploy = FakeDeploy::online("203.0.113.10", 7777);
    deploy.set_ramsgate(true, "203.0.113.10", 7777);
    let mut module = shared(
        &[
            ("HUNT_GO_NOW_MIN_DELAY_MS", "40"),
            ("HUNT_GO_NOW_MAX_DELAY_MS", "40"),
            ("HUB_GO_NOW_MIN_DELAY_MS", "40"),
            ("HUB_GO_NOW_MAX_DELAY_MS", "40"),
            ("GO_NOW_LOAD_PADDING_PER_JOINER_MS", "1"),
            ("QOS_TARGET_URL", "qos.example.net"),
        ],
        deploy,
    );
    module.store = Some(target.store.clone());
    let address = serve(Arc::new(module)).await;
    let steps: Vec<Value> = serde_json::from_str(SCENARIO).unwrap();
    let mut answers = std::collections::HashMap::new();
    let mut out = Vec::new();
    for step in &steps {
        if let Some(ms) = step.get("sleep").and_then(Value::as_u64) {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            continue;
        }
        let reply = replay(address, step, &answers).await;
        let name = step["name"].as_str().unwrap().to_owned();
        answers.insert(name.clone(), serde_json::from_str(&reply.body).unwrap_or(Value::Null));
        out.push((name, reply));
    }
    out
}

fn summary(reply: &Reply) -> Value {
    json!({
        "status": reply.status,
        "contentType": reply.header("content-type"),
        "partyRevision": reply.header("x-mystic-party-revision"),
        "body": normalise(&reply.body),
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn every_answer_is_what_node_answered_on_1_14_7() {
    let golden: Vec<Value> = serde_json::from_str(GOLDEN).unwrap();
    for target in support::targets() {
        let answers = run(&target).await;
        assert_eq!(answers.len(), golden.len());
        for ((name, reply), expected) in answers.iter().zip(&golden) {
            assert_eq!(name, expected["name"].as_str().unwrap());
            let want = json!({
                "status": expected["status"],
                "contentType": expected["contentType"],
                "partyRevision": expected["partyRevision"],
                "body": normalise(expected["body"].as_str().unwrap()),
            });
            assert_eq!(summary(reply), want, "{name} {} on {}", expected["path"], target.engine);
        }
    }
}
