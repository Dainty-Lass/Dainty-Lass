//! `isles-core/tests/golden/scenario.json` against `matchmaking-parties.json` (`routes/matchmaking.ts` and
//! `routes/party.ts` on Node 22, `capture.cjs`): status, content type, `X-Mystic-Party-Revision` and body, with
//! restricted's id normalisation. Restricted's fake Director becomes a fake DeployServer on 127.0.0.1:0 answering the
//! two calls the module makes (every launch on 203.0.113.10:7777, Ramsgate ready there).
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;
use std::time::Duration;

use common::{Caller, Mit, Tally, credentials, seed_account, send};
use regex::Regex;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn normalise(text: &str) -> String {
    let uuid = Regex::new(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}").unwrap();
    let party = Regex::new(r"(?:[0-9a-f]{32}|[0-9A-HJKMNP-TV-Z]{26}\.[a-z0-9-]+)_[A-Za-z0-9+/=]*").unwrap();
    party
        .replace_all(&uuid.replace_all(text, "<uuid>"), "<party>")
        .into_owned()
}

fn fill(value: &Value, answers: &HashMap<String, Value>) -> Value {
    match value {
        Value::String(text) if text.starts_with('$') => {
            let (name, field) = text[1..].split_once('.').unwrap();
            answers.get(name).map_or(Value::Null, |a| a[field].clone())
        }
        Value::Object(map) => Value::Object(map.iter().map(|(k, v)| (k.clone(), fill(v, answers))).collect()),
        other => other.clone(),
    }
}

/// A DeployServer that launches everything on one address and reports Ramsgate ready there.
async fn fake_deployserver() -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut raw = Vec::new();
                let mut buf = [0u8; 4096];
                loop {
                    let Ok(n) = stream.read(&mut buf).await else { return };
                    if n == 0 {
                        break;
                    }
                    raw.extend_from_slice(&buf[..n]);
                    if let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&raw[..end]).to_ascii_lowercase();
                        let length = head
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        if raw.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                let head = String::from_utf8_lossy(&raw);
                let line = head.lines().next().unwrap_or_default().to_owned();
                let body = if line.starts_with("POST /api/matchmaker/handle-matchmaking-for-player") {
                    Some(json!({ "host": "203.0.113.10", "port": 7777 }))
                } else if line.starts_with("GET /api/hub-status") {
                    Some(json!({ "ramsgate": { "ready": true, "host": "203.0.113.10", "port": 7777 } }))
                } else {
                    None
                };
                let answer = match body {
                    Some(body) => {
                        let text = body.to_string();
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                            text.len()
                        )
                    }
                    None => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned(),
                };
                let _ = stream.write_all(answer.as_bytes()).await;
            });
        }
    });
    address
}

#[tokio::test(flavor = "multi_thread")]
async fn matchmaking_parties() {
    let golden_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../isles-core/tests/golden");
    let steps: Vec<Value> =
        serde_json::from_str(&std::fs::read_to_string(golden_dir.join("scenario.json")).unwrap()).unwrap();
    let golden: Vec<Value> =
        serde_json::from_str(&std::fs::read_to_string(golden_dir.join("matchmaking-parties.json")).unwrap()).unwrap();
    let deploy = fake_deployserver().await.to_string();
    let mit = Mit::start(
        &[
            ("MATCHMAKING_MODE", "DEPLOYSERVER"),
            ("DEPLOYSERVER_URL", deploy.as_str()),
            ("HUNT_GO_NOW_MIN_DELAY_MS", "40"),
            ("HUNT_GO_NOW_MAX_DELAY_MS", "40"),
            ("HUB_GO_NOW_MIN_DELAY_MS", "40"),
            ("HUB_GO_NOW_MAX_DELAY_MS", "40"),
            ("GO_NOW_LOAD_PADDING_PER_JOINER_MS", "1"),
            ("QOS_TARGET_URL", "qos.example.net"),
        ],
        |_| {},
    )
    .await;
    let store = mit.store();
    for user in ["alice", "bob", "carol"] {
        seed_account(&store, user, user).await;
    }
    let mut tally = Tally::new("matchmaking_parties");
    let mut answers = HashMap::new();
    let mut recorded = golden.iter();
    for (n, step) in steps.iter().enumerate() {
        if let Some(ms) = step.get("sleep").and_then(Value::as_u64) {
            tokio::time::sleep(Duration::from_millis(ms)).await;
            continue;
        }
        let want = recorded.next().unwrap();
        let name = step["name"].as_str().unwrap().to_owned();
        assert_eq!(want["name"].as_str(), Some(name.as_str()));
        let (method, path) = (step["method"].as_str().unwrap(), step["path"].as_str().unwrap());
        let mut headers = match step.get("user").and_then(Value::as_str) {
            Some(user) => credentials(Caller::Player(user)),
            None => credentials(Caller::Nobody),
        };
        let body = if let Some(raw) = step.get("raw").and_then(Value::as_str) {
            Some(raw.to_owned())
        } else {
            step.get("body").map(|b| fill(b, &answers).to_string())
        };
        if body.is_some() {
            headers.push(("content-type".into(), "application/json".into()));
        }
        let reply = send(mit.backend, method, path, &headers, body.as_deref().map(str::as_bytes)).await;
        answers.insert(name.clone(), serde_json::from_str(&reply.body).unwrap_or(Value::Null));
        let expected = json!({
            "status": want["status"], "contentType": want["contentType"], "partyRevision": want["partyRevision"],
            "body": normalise(want["body"].as_str().unwrap()),
        });
        let actual = json!({
            "status": reply.status, "contentType": reply.header("content-type"),
            "partyRevision": reply.header("x-mystic-party-revision"), "body": normalise(&reply.body),
        });
        tally.step(n + 1, &name, method, path, expected, actual);
    }
    tally.finish();
}
