//! The auth area against Node's own answers (RU5 auth): `tests/golden/auth/scenario.json` was run through the reference
//! backend (the real `app.ts` over SQLite, under Node 22; `golden/auth/run.sh`) and its answers recorded as
//! `golden.json`, with tokens, ids, times and random codes normalized. The same requests are made here over real HTTP
//! to the module's pipeline, on a SQLite file of its own, and every status, header and body (field order included)
//! must be equal. Re-record only when the Node side changed on purpose.
#![allow(clippy::unwrap_used, clippy::expect_used)] // test helpers

use std::collections::BTreeMap;
use std::path::PathBuf;

use isles_core::areas::auth::test_support::{Reply, TestServer, TestVersion, send};
use regex::{Captures, Regex};
use serde_json::{Map, Value, json};

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/auth")
}

fn read(name: &str) -> Value {
    let text = std::fs::read_to_string(golden_dir().join(name)).unwrap();
    serde_json::from_str(&text).unwrap()
}

// ---- the shared normalization (record.cjs does the same)

fn replace_runs(text: &str, length: usize, with: &str) -> String {
    let bytes = text.as_bytes();
    let hex = |b: u8| b.is_ascii_digit() || (b'a'..=b'f').contains(&b);
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        if hex(bytes[i]) {
            let start = i;
            while i < bytes.len() && hex(bytes[i]) {
                i += 1;
            }
            if i - start == length {
                out.push_str(with);
            } else {
                out.push_str(&text[start..i]);
            }
        } else {
            let start = i;
            while i < bytes.len() && !hex(bytes[i]) {
                i += 1;
            }
            out.push_str(&text[start..i]);
        }
    }
    out
}

fn normalize_text(text: &str) -> String {
    let jwt = Regex::new(r"eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+").unwrap();
    let uuid = Regex::new(r"(?i)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}").unwrap();
    let iso = Regex::new(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}Z").unwrap();
    let challenge = Regex::new(r"code_challenge=[A-Za-z0-9_-]{43}").unwrap();
    let text = jwt.replace_all(text, "<jwt>").into_owned();
    let text = uuid.replace_all(&text, "<uuid>").into_owned();
    let text = iso
        .replace_all(&text, |m: &Captures| {
            if m[0].starts_with("2085-09-09") {
                m[0].to_owned()
            } else {
                "<iso>".to_owned()
            }
        })
        .into_owned();
    let text = replace_runs(&text, 64, "<hex64>");
    let text = replace_runs(&text, 48, "<hex48>");
    challenge.replace_all(&text, "code_challenge=<challenge>").into_owned()
}

fn normalize_value(value: &Value, key: Option<&str>) -> Value {
    match value {
        Value::String(text) => Value::String(if key == Some("nonce") {
            "<nonce>".into()
        } else {
            normalize_text(text)
        }),
        Value::Array(items) => Value::Array(items.iter().map(|v| normalize_value(v, None)).collect()),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(name, v)| (name.clone(), normalize_value(v, Some(name))))
                .collect::<Map<_, _>>(),
        ),
        other => other.clone(),
    }
}

const KEPT_HEADERS: [&str; 7] = [
    "content-type",
    "location",
    "cache-control",
    "access-control-allow-origin",
    "access-control-allow-methods",
    "access-control-allow-headers",
    "vary",
];

fn normalized_answer(reply: &Reply) -> Value {
    let content_type = reply.header("content-type").unwrap_or_default();
    let body = if content_type.contains("json") {
        match serde_json::from_str::<Value>(&reply.body) {
            Ok(parsed) => service_kit::jsjson::stringify(&normalize_value(&parsed, None)),
            Err(_) => normalize_text(&reply.body),
        }
    } else {
        normalize_text(&reply.body)
    };
    let mut headers = Map::new();
    for name in KEPT_HEADERS {
        if let Some(value) = reply.header(name) {
            headers.insert(name.to_owned(), json!(normalize_text(value)));
        }
    }
    json!({ "status": reply.status, "headers": headers, "body": body })
}

// ---- the scenario

fn fill(template: &Value, vars: &BTreeMap<String, String>) -> Value {
    match template {
        Value::String(text) => {
            let pattern = Regex::new(r"\{\{([^}]+)\}\}").unwrap();
            Value::String(
                pattern
                    .replace_all(text, |m: &Captures| {
                        vars.get(&m[1])
                            .unwrap_or_else(|| panic!("no variable {}", &m[1]))
                            .clone()
                    })
                    .into_owned(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(|v| fill(v, vars)).collect()),
        Value::Object(fields) => Value::Object(fields.iter().map(|(k, v)| (k.clone(), fill(v, vars))).collect()),
        other => other.clone(),
    }
}

fn capture(spec: &str, reply: &Reply) -> String {
    let parts: Vec<&str> = spec.split('~').collect();
    let source = if parts[0].is_empty() && parts.len() == 3 {
        reply.header(parts[1]).unwrap_or_default().to_owned()
    } else {
        let mut value = reply.json();
        for key in parts[0].split('.') {
            value = value.get(key).cloned().unwrap_or(Value::Null);
        }
        let text = match &value {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        };
        if parts.len() == 1 {
            return text;
        }
        text
    };
    let pattern = Regex::new(parts[parts.len() - 1]).unwrap();
    pattern
        .captures(&source)
        .unwrap_or_else(|| panic!("capture {spec} found nothing in {source}"))[1]
        .to_owned()
}

/// The test server for the base environment plus whatever `env` steps set so far.
fn server(
    version: TestVersion,
    store_of: Option<&TestServer>,
    extra: &BTreeMap<String, String>,
    updates: &str,
) -> TestServer {
    let mut settings: Vec<(&str, &str)> = vec![
        ("AUTH_MODE", "LAUNCHER"),
        ("APPROVED_EXECUTABLE_SHA256", EXECUTABLE),
        ("LAUNCHER_UPDATE_ROOT", updates),
        ("MYSTICPARADOX_OFFICIAL_PLAY", "on"),
        ("GAMESERVER_API_KEYS", "gs-key-1"),
    ];
    settings.extend(extra.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    match store_of {
        Some(other) => TestServer::with_store_of(other, &settings),
        None => TestServer::for_version(version, &settings),
    }
}

const EXECUTABLE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn write_manifest(dir: &std::path::Path) {
    let path = dir.join("runtime/client/stable/windows-x86_64/latest.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let body = json!({
        "version": "0.1.78",
        "targetChangelist": 647_472,
        "sha256": "b".repeat(64),
        "extraFiles": [{ "name": "winmm.dll", "sha256": "c".repeat(64) }]
    });
    std::fs::write(path, body.to_string()).unwrap();
}

/// Runs the scenario for one version and returns the steps that differ from `golden`.
async fn run(version: TestVersion, label: &str, scenario_file: &str, golden: &[Value]) -> Vec<String> {
    let scenario = read(scenario_file);
    let updates = tempfile::tempdir().unwrap();
    write_manifest(updates.path());
    let updates_path = updates.path().display().to_string();
    let mut extra: BTreeMap<String, String> = BTreeMap::new();
    let mut current = server(version, None, &extra, &updates_path);
    isles_core::areas::auth::apikeys::register_gameserver_api_keys(&current.shared)
        .await
        .unwrap();
    let mut vars: BTreeMap<String, String> = BTreeMap::new();
    if version == TestVersion::V1120 {
        // The stand-in persistence of the 1.12.0 recording knows one game account
        let account = isles_core::repo::auth::AccountRecord {
            id: isles_core::AccountId::new(&module_api::NodeIdentity::standalone().node),
            user_id: isles_core::UserId("u1".into()),
            name: "Fox".into(),
            notes: 0,
        };
        current
            .store()
            .write(&isles_core::areas::auth::data::system_actor(), move |tx, repos| {
                repos.accounts.create(tx, &account)
            })
            .await
            .unwrap();
        vars.insert("tok".into(), current.game_token("u1"));
    }
    let mut failures = Vec::new();
    let mut counter = 0u32;
    // The recorded answers are only those of steps this version runs, in order
    let mut recorded = golden.iter();
    for step in scenario["steps"].as_array().unwrap() {
        if let Some(versions) = step.get("versions").and_then(Value::as_array)
            && !versions.iter().any(|v| v == label)
        {
            continue;
        }
        let name = step["name"].as_str().unwrap();
        let Some(want) = recorded.next() else {
            failures.push(format!("{name}: the golden file has no more answers"));
            break;
        };
        if want["name"] != step["name"] {
            failures.push(format!("{name}: the golden file is at {}", want["name"]));
            break;
        }
        if let Some(values) = step.get("env").and_then(Value::as_object) {
            for (key, value) in values {
                extra.insert(key.clone(), value.as_str().unwrap().to_owned());
            }
            current = server(version, Some(&current), &extra, &updates_path);
            continue;
        }
        if let Some(spec) = step.get("mailcode") {
            let email = spec["email"].as_str().unwrap();
            let code = current
                .mail
                .code_for(email)
                .unwrap_or_else(|| panic!("no mail to {email}"));
            vars.insert(spec["var"].as_str().unwrap().to_owned(), code);
            continue;
        }
        counter += 1;
        let mut headers: Vec<(String, String)> = Vec::new();
        if let Some(fields) = step.get("headers") {
            for (key, value) in fill(fields, &vars).as_object().unwrap() {
                headers.push((key.clone(), value.as_str().unwrap().to_owned()));
            }
        }
        if !headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("x-forwarded-for")) {
            headers.insert(
                0,
                ("x-forwarded-for".into(), format!("198.51.100.{}", counter % 250 + 1)),
            );
        }
        let body = if let Some(json) = step.get("json") {
            Some(("application/json", fill(json, &vars).to_string()))
        } else if let Some(raw) = step.get("raw") {
            Some((
                step.get("contentType")
                    .and_then(Value::as_str)
                    .unwrap_or("application/json"),
                raw.as_str().unwrap().to_owned(),
            ))
        } else {
            step.get("form")
                .map(|form| ("application/x-www-form-urlencoded", form.as_str().unwrap().to_owned()))
        };
        let path = fill(&step["path"], &vars);
        let address = current.address().await;
        let reply = send(
            address,
            step["method"].as_str().unwrap(),
            path.as_str().unwrap(),
            body,
            &headers,
        )
        .await;
        if let Some(captures) = step.get("capture").and_then(Value::as_object) {
            for (variable, spec) in captures {
                let spec = spec.as_str().unwrap();
                if reply.status < 400 || spec.starts_with('~') {
                    vars.insert(variable.clone(), capture(spec, &reply));
                }
            }
        }
        let got = normalized_answer(&reply);
        let expected = json!({ "status": want["status"], "headers": want["headers"], "body": want["body"] });
        if got != expected {
            failures.push(format!("{name} ({label}):\n   node: {expected}\n   rust: {got}"));
        }
    }
    failures
}

#[tokio::test(flavor = "multi_thread")]
async fn the_auth_routes_answer_as_the_reference_backend_does() {
    let golden = read("golden.json");
    let failures = run(
        TestVersion::V1147,
        "1.14.7",
        "scenario.json",
        golden.as_array().unwrap(),
    )
    .await;
    assert!(
        failures.is_empty(),
        "{} steps differ:
{}",
        failures.len(),
        failures.join(
            "
"
        )
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_1_12_module_answers_the_game_client_routes_as_1_12_0_does() {
    let golden = read("golden112.json");
    let failures = run(
        TestVersion::V1120,
        "1.12.0",
        "scenario112.json",
        golden.as_array().unwrap(),
    )
    .await;
    assert!(
        failures.is_empty(),
        "{} steps differ:
{}",
        failures.len(),
        failures.join(
            "
"
        )
    );
}
