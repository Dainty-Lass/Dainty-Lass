//! `isles-core/tests/golden/game_data.json` (the game-data group under Node 22, `capture.mjs`) against the MIT module:
//! status, content type and body text, as restricted's `game_data::golden::check` compares them. The clock is the
//! recording's; a player's game token rides on every case, as restricted's does.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{Caller, Mit, Tally, credentials, send};
use serde_json::{Value, json};

fn golden_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../crates/isles-core/tests/golden")
}

/// Every `description` text and every `list` entry becomes `<prose>`; the rest (keys, order, titles, types) stays.
fn structure(text: &str) -> String {
    fn walk(value: &mut Value, key: Option<&str>) {
        match value {
            Value::String(text) if matches!(key, Some("description") | Some("list")) => *text = "<prose>".into(),
            Value::Array(items) => items.iter_mut().for_each(|v| walk(v, key)),
            Value::Object(map) => map.iter_mut().for_each(|(k, v)| walk(v, Some(k))),
            _ => {}
        }
    }
    match serde_json::from_str::<Value>(text) {
        Ok(mut value) => {
            walk(&mut value, None);
            value.to_string()
        }
        Err(_) => text.to_owned(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn game_data() {
    let golden: Value =
        serde_json::from_str(&std::fs::read_to_string(golden_dir().join("game_data.json")).unwrap()).unwrap();
    rustic_isles::replay::pin_now_for_tests(golden["now"].as_i64());
    let synthetic = golden_dir().join("progression_config.synthetic.json");
    let mut mit = Mit::start(&[], |dir| {
        std::fs::copy(&synthetic, dir.join("progression_config.json")).unwrap();
    })
    .await;
    let mut tally = Tally::new("game_data");
    let mut current_env: Vec<(String, String)> = Vec::new();
    for (n, case) in golden["cases"].as_array().unwrap().iter().enumerate() {
        if case["version"] != "1.14.7" {
            continue;
        }
        let name = case["name"].as_str().unwrap();
        let request = &case["request"];
        let env: Vec<(String, String)> = request["env"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
            .collect();
        if env != current_env {
            let pairs: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            mit.restart(&pairs).await;
            current_env = env;
        }
        let text = |key: &str| request[key].as_str();
        let origin = text("origin").map(|h| format!("/__origin/{h}")).unwrap_or_default();
        let (content_type, body) = match (text("json"), text("form")) {
            (Some(json), _) => (Some("application/json"), json),
            (None, Some(form)) => (Some("application/x-www-form-urlencoded"), form),
            (None, None) => (None, ""),
        };
        let mut headers = credentials(Caller::Player("golden-user"));
        if let Some(content_type) = content_type {
            headers.push(("content-type".into(), content_type.into()));
        }
        let method = text("method").unwrap_or("GET");
        let path = format!("{origin}{}", text("path").unwrap_or("/"));
        let reply = send(mit.backend, method, &path, &headers, Some(body.as_bytes())).await;
        let expect = &case["expect"];
        // gamedata §12 item 1 / D1: the patch-notes prose is the implementer's own, so those cases compare structure
        let prose = path.starts_with("/patchnotes/");
        if prose {
            tally.note(format!(
                "{name}: compared on structure only (spec-rustic-isles-gamedata §12 item 1)"
            ));
        }
        let body = |text: &str| if prose { structure(text) } else { text.to_owned() };
        let expected = json!({ "status": expect["status"], "content-type": expect["content_type"],
            "body": body(expect["body"].as_str().unwrap_or_default()) });
        let actual = json!({ "status": reply.status, "content-type": reply.header("content-type"),
            "body": body(&reply.body) });
        tally.step(n + 1, name, method, &path, expected, actual);
    }
    rustic_isles::replay::pin_now_for_tests(None);
    mit.stop().await;
    tally.finish();
}
