//! `tests/golden/reforged_api.json` (the Reforged API routers under Node 22, `reforged_api_capture.mts`)
//! against the MIT module, a fresh module per case as restricted's `reforged_api_golden.rs` does: status, the headers
//! a client sees and the body, with ids and the uptime normalised the same way.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::path::Path;

use common::{Mit, Tally, send};
use serde_json::{Map, Value, json};

const HEADERS: [&str; 7] = [
    "content-type",
    "cache-control",
    "content-length",
    "location",
    "content-disposition",
    "vary",
    "retry-after",
];

fn normalize(body: &str) -> String {
    let ids = regex::Regex::new(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}").unwrap();
    let uptime = regex::Regex::new(r#""uptimeSeconds":\d+"#).unwrap();
    let body = ids.replace_all(body, "<id>");
    uptime.replace_all(&body, r#""uptimeSeconds":0"#).into_owned()
}

fn normalize_content_length(raw_len: &str, body: &str) -> String {
    let uptime = regex::Regex::new(r#""uptimeSeconds":\d+"#).unwrap();
    let normalized = uptime.replace_all(body, r#""uptimeSeconds":0"#);
    let delta = body.len().saturating_sub(normalized.len());
    raw_len
        .parse::<usize>()
        .map_or_else(|_| raw_len.to_owned(), |n| n.saturating_sub(delta).to_string())
}

fn write_files(dir: &Path, files: &Map<String, Value>) {
    for (name, text) in files {
        let target = dir.join(name);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, text.as_str().unwrap()).unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn reforged_api() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/reforged_api.json");
    // Recorded from Node under the old names (dainty_protocol::renamed)
    let text = dainty_protocol::renamed::renamed(&std::fs::read_to_string(path).unwrap());
    let cases: Value = serde_json::from_str(&text).unwrap();
    let mut tally = Tally::new("reforged_api");
    for (n, case) in cases.as_array().unwrap().iter().enumerate() {
        let name = case["name"].as_str().unwrap();
        let updates = tempfile::tempdir().unwrap();
        write_files(updates.path(), case["files"].as_object().unwrap());
        let mut pairs: Vec<(String, String)> = vec![
            ("LAUNCHER_UPDATE_ROOT".into(), updates.path().display().to_string()),
            (
                "LAUNCHER_LOG_STORAGE_DIR".into(),
                updates.path().join("logs").display().to_string(),
            ),
            ("RUSTIC_ISLES_ADMIN_API".into(), "on".into()),
            (
                "API_KEY_HASH_SECRET".into(),
                "a-test-secret-of-at-least-32-characters".into(),
            ),
        ];
        for (key, value) in case["env"].as_object().unwrap() {
            pairs.retain(|(k, _)| k != key);
            pairs.push((key.clone(), value.as_str().unwrap().to_owned()));
        }
        let borrowed: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        let request = &case["request"];
        let mut mit = match Mit::try_start(&borrowed, |_| {}).await {
            Ok(mit) => mit,
            Err(error) if error.starts_with("Rename these environment keys") => {
                // Restricted's harness serves the backend alone; composed, the Director refuses the same env file
                // at start (spec-rustic-isles-director §11, decision item 1), so this gate can't be reached
                tally.skip(
                    n + 1,
                    name,
                    request["method"].as_str().unwrap(),
                    request["path"].as_str().unwrap(),
                    "the composed module refuses MYSTPAX_ keys at start",
                    json!({ "module start failed": error }),
                );
                continue;
            }
            Err(error) => {
                let expected = &case["response"];
                tally.step(
                    n + 1,
                    name,
                    request["method"].as_str().unwrap(),
                    request["path"].as_str().unwrap(),
                    json!({ "status": expected["status"], "body": expected["body"] }),
                    json!({ "module start failed": error }),
                );
                continue;
            }
        };
        let mut headers: Vec<(String, String)> = request["headers"]
            .as_object()
            .map(|h| {
                h.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned()))
                    .collect()
            })
            .unwrap_or_default();
        let body = request["json"].as_str();
        if body.is_some() {
            headers.push(("content-type".into(), "application/json".into()));
        }
        let method = request["method"].as_str().unwrap();
        let target = request["path"].as_str().unwrap();
        let answer = send(mit.backend, method, target, &headers, body.map(str::as_bytes)).await;

        let expected = &case["response"];
        let compare_body = !expected["body"].is_null();
        let golden_headers = expected["headers"].as_object().unwrap();
        let mut want_headers = Map::new();
        let mut got_headers = Map::new();
        for header in HEADERS {
            if header == "content-length" && !compare_body {
                continue;
            }
            want_headers.insert(
                header.into(),
                golden_headers.get(header).cloned().unwrap_or(Value::Null),
            );
            let got = answer.header(header).map(|raw| {
                if header == "content-length" && name.starts_with("server status") {
                    normalize_content_length(raw, &answer.body)
                } else {
                    normalize(raw)
                }
            });
            got_headers.insert(header.into(), json!(got));
        }
        let mut want = json!({ "status": expected["status"], "headers": want_headers });
        let mut got = json!({ "status": answer.status, "headers": got_headers });
        if compare_body {
            want["body"] = expected["body"].clone();
            got["body"] = json!(normalize(&answer.body));
        }
        if let Some(after) = case.get("servingAfter") {
            let written = std::fs::read_to_string(updates.path().join("serving.json"))
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .map(|v| v["runtimeEnabled"].clone());
            want["serving.json runtimeEnabled"] = after.clone();
            got["serving.json runtimeEnabled"] = written.unwrap_or(Value::Null);
        }
        tally.step(n + 1, name, method, target, want, got);
        mit.stop().await;
    }
    tally.finish();
}
