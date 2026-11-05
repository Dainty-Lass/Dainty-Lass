//! `isles-core/tests/golden/characters/` (the Node 1.14.7 backend's characters-loadouts answers, `record.sh`) against
//! the MIT module over HTTP: restricted's `areas::characters::golden` replay and normalisation (ids numbered by first
//! appearance, clock-bound dates masked). `setEnv` restarts the module on the same database; `setup: journey` writes
//! the Slayer's Path row through the store. `as` becomes the real credential: `player:<u>` that player's token,
//! `gameserver` the game server key alone, `gameserver:<u>` the key with `<u>`'s token.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::BTreeMap;

use common::{Caller, Mit, Tally, credentials, send, system};
use isles_core::areas::characters::golden::Normalizer;
use serde_json::{Value, json};

const DIR: &str = "../isles-core/tests/golden/characters";

fn read(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(DIR).join(name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn replace_tokens(text: &str, saved: &BTreeMap<String, String>) -> String {
    let mut out = text.to_owned();
    for (name, value) in saved {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

fn pick(value: &Value, path: &str) -> String {
    let found = path.split('.').try_fold(value, |v, key| match v {
        Value::Array(items) => key.parse::<usize>().ok().and_then(|i| items.get(i)),
        other => other.get(key),
    });
    match found {
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
        None => "undefined".into(),
    }
}

struct Answer {
    number: usize,
    name: String,
    method: String,
    path: String,
    status: u16,
    content_type: Option<String>,
    body: String,
}

#[tokio::test(flavor = "multi_thread")]
async fn characters() {
    let scenario = read("scenario.json");
    let golden = read("node-1.14.7.golden.json");
    let mut mit = Mit::start(&[], |_| {}).await;
    let mut env: BTreeMap<String, String> = BTreeMap::new();
    let mut saved = BTreeMap::new();
    let mut answers = Vec::new();
    for (n, step) in scenario["steps"].as_array().unwrap().iter().enumerate() {
        if let Some(set) = step.get("setEnv").and_then(Value::as_object) {
            for (name, value) in set {
                env.insert(name.clone(), value.as_str().unwrap_or_default().to_owned());
            }
            let pairs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            mit.restart(&pairs).await;
            continue;
        }
        if step.get("setup").and_then(Value::as_str) == Some("journey") {
            let store = mit.store();
            let user = rustic_isles::UserId::new(step["user"].as_str().unwrap());
            let account = store
                .read(move |conn, repos| repos.accounts.find_by_user_id(conn, &user))
                .await
                .unwrap()
                .expect("the scenario's player has an account by now")
                .id;
            let record = rustic_isles_store::repo::progression::PlayerJourneyRecord {
                account: account.clone(),
                nodes: step["nodes"].to_string(),
                update_version: 1,
                revision: 0,
            };
            store
                .account_write(&account, &system(), move |tx, repos| {
                    repos.player_journey.create_if_absent(tx, &record)
                })
                .await
                .unwrap();
            continue;
        }
        let method = step["method"].as_str().unwrap_or("GET");
        let path = replace_tokens(step["path"].as_str().unwrap_or("/"), &saved);
        let (body, content_type) = if let Some(raw) = step.get("rawBody").and_then(Value::as_str) {
            (
                Some(raw.to_owned()),
                Some(step["contentType"].as_str().unwrap_or("application/json").to_owned()),
            )
        } else if let Some(json) = step.get("body") {
            (
                Some(replace_tokens(&json.to_string(), &saved)),
                Some("application/json".to_owned()),
            )
        } else {
            (None, None)
        };
        let kind = step["as"].as_str().unwrap_or("player");
        let (role, user) = kind.split_once(':').map_or((kind, None), |(r, u)| (r, Some(u)));
        let mut headers = match (role, user) {
            ("gameserver", user) => credentials(Caller::Gameserver(user)),
            (_, Some(user)) => credentials(Caller::Player(user)),
            (_, None) => credentials(Caller::Nobody),
        };
        if let Some(content_type) = &content_type {
            headers.push(("content-type".into(), content_type.clone()));
        }
        let reply = send(mit.backend, method, &path, &headers, body.as_deref().map(str::as_bytes)).await;
        if let Some(save) = step.get("save").and_then(Value::as_object)
            && let Ok(parsed) = serde_json::from_str::<Value>(&reply.body)
        {
            for (name, p) in save {
                saved.insert(name.clone(), pick(&parsed, p.as_str().unwrap_or_default()));
            }
        }
        answers.push(Answer {
            number: n + 1,
            name: step["name"].as_str().unwrap_or_default().to_owned(),
            method: method.to_owned(),
            path,
            status: reply.status,
            content_type: reply.header("content-type").map(str::to_owned),
            body: reply.body,
        });
    }
    mit.stop().await;

    let steps = golden["steps"].as_array().unwrap();
    let mut tally = Tally::new("characters");
    if steps.len() != answers.len() {
        tally.note(format!("{} answers recorded, {} replayed", steps.len(), answers.len()));
    }
    // Numbered over each whole run first, as restricted's `compare` does
    let mut want_norm = Normalizer::new();
    for step in steps {
        want_norm.apply(step["body"].as_str().unwrap_or_default());
    }
    let mut got_norm = Normalizer::new();
    for answer in &answers {
        got_norm.apply(&answer.body);
    }
    for (want, got) in steps.iter().zip(&answers) {
        let expected = json!({ "status": want["status"], "content-type": want["contentType"],
            "body": want_norm.apply(want["body"].as_str().unwrap_or_default()) });
        let actual = json!({ "status": got.status, "content-type": got.content_type,
            "body": got_norm.apply(&got.body) });
        tally.step(got.number, &got.name, &got.method, &got.path, expected, actual);
    }
    tally.finish();
}
