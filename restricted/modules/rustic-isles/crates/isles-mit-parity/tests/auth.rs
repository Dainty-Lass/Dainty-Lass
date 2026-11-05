//! `isles-core/tests/golden/auth/` (the reference backend's auth answers, `run.sh`) against the MIT module: restricted's
//! `auth_golden.rs` loop and normalisation, the module restarted on the same database where an `env` step changes the
//! settings, the code mails read from the recording mail.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::BTreeMap;
use std::path::PathBuf;

use common::{Mit, Reply, Tally, send};
use regex::{Captures, Regex};
use serde_json::{Map, Value, json};

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../isles-core/tests/golden/auth")
}

fn read(name: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(golden_dir().join(name)).unwrap()).unwrap()
}

// ---- restricted's normalisation (record.cjs does the same)

fn replace_runs(text: &str, length: usize, with: &str) -> String {
    let bytes = text.as_bytes();
    let hex = |b: u8| b.is_ascii_digit() || (b'a'..=b'f').contains(&b);
    let mut out = String::new();
    let mut i = 0;
    while i < bytes.len() {
        let start = i;
        let is_hex = hex(bytes[i]);
        while i < bytes.len() && hex(bytes[i]) == is_hex {
            i += 1;
        }
        if is_hex && i - start == length {
            out.push_str(with);
        } else {
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
                    .replace_all(text, |m: &Captures| vars.get(&m[1]).cloned().unwrap_or_default())
                    .into_owned(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(|v| fill(v, vars)).collect()),
        Value::Object(fields) => Value::Object(fields.iter().map(|(k, v)| (k.clone(), fill(v, vars))).collect()),
        other => other.clone(),
    }
}

fn capture(spec: &str, reply: &Reply) -> Option<String> {
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
            return Some(text);
        }
        text
    };
    let pattern = Regex::new(parts[parts.len() - 1]).unwrap();
    Some(pattern.captures(&source)?[1].to_owned())
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

fn settings(updates: &str, extra: &BTreeMap<String, String>) -> Vec<(String, String)> {
    let mut pairs: Vec<(String, String)> = vec![
        ("AUTH_MODE".into(), "LAUNCHER".into()),
        ("APPROVED_EXECUTABLE_SHA256".into(), EXECUTABLE.into()),
        ("LAUNCHER_UPDATE_ROOT".into(), updates.into()),
        ("RUSTIC_ISLES_OFFICIAL_PLAY".into(), "on".into()),
        // Restricted's auth harness leaves the request budget at its default
        ("RUSTIC_ISLES_RATE_LIMIT".into(), "on".into()),
    ];
    pairs.extend(extra.iter().map(|(k, v)| (k.clone(), v.clone())));
    pairs
}

#[tokio::test(flavor = "multi_thread")]
async fn auth() {
    let scenario = read("scenario.json");
    let golden = read("golden.json");
    let updates = tempfile::tempdir().unwrap();
    write_manifest(updates.path());
    let updates_path = updates.path().display().to_string();
    let mut extra: BTreeMap<String, String> = BTreeMap::new();
    let pairs = settings(&updates_path, &extra);
    let borrowed: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let mut mit = Mit::start(&borrowed, |_| {}).await;
    let mut vars: BTreeMap<String, String> = BTreeMap::new();
    let mut tally = Tally::new("auth");
    let mut counter = 0u32;
    let mut recorded = golden.as_array().unwrap().iter();
    for (n, step) in scenario["steps"].as_array().unwrap().iter().enumerate() {
        if let Some(versions) = step.get("versions").and_then(Value::as_array)
            && !versions.iter().any(|v| v == "1.14.7")
        {
            continue;
        }
        let name = step["name"].as_str().unwrap();
        let Some(want) = recorded.next() else {
            tally.note(format!("{name}: the golden file has no more answers"));
            break;
        };
        assert_eq!(want["name"], step["name"], "the golden file is out of step");
        if let Some(values) = step.get("env").and_then(Value::as_object) {
            for (key, value) in values {
                extra.insert(key.clone(), value.as_str().unwrap().to_owned());
            }
            let pairs = settings(&updates_path, &extra);
            let borrowed: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            if let Err(error) = mit.try_restart(&borrowed).await {
                tally.note(format!("{name}: the restart failed: {error}"));
            }
            continue;
        }
        if let Some(spec) = step.get("mailcode") {
            let email = spec["email"].as_str().unwrap();
            // Mail is sent in the background (auth §7.3)
            mit.mail.wait_for(1).await;
            let code = mit.mail.last_code(email).unwrap_or_else(|| {
                tally.note(format!("{name}: no mail to {email}"));
                "000000".into()
            });
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
            Some(("application/json".to_owned(), fill(json, &vars).to_string()))
        } else if let Some(raw) = step.get("raw") {
            Some((
                step.get("contentType")
                    .and_then(Value::as_str)
                    .unwrap_or("application/json")
                    .to_owned(),
                raw.as_str().unwrap().to_owned(),
            ))
        } else {
            step.get("form").map(|form| {
                (
                    "application/x-www-form-urlencoded".to_owned(),
                    form.as_str().unwrap().to_owned(),
                )
            })
        };
        if let Some((content_type, _)) = &body {
            headers.push(("content-type".into(), content_type.clone()));
        }
        let path = fill(&step["path"], &vars);
        let path = path.as_str().unwrap();
        let method = step["method"].as_str().unwrap();
        let reply = send(
            mit.backend,
            method,
            path,
            &headers,
            body.as_ref().map(|(_, b)| b.as_bytes()),
        )
        .await;
        if let Some(captures) = step.get("capture").and_then(Value::as_object) {
            for (variable, spec) in captures {
                let spec = spec.as_str().unwrap();
                if (reply.status < 400 || spec.starts_with('~'))
                    && let Some(value) = capture(spec, &reply)
                {
                    vars.insert(variable.clone(), value);
                }
            }
        }
        let got = normalized_answer(&reply);
        let expected = json!({ "status": want["status"], "headers": want["headers"], "body": want["body"] });
        tally.step(n + 1, name, method, path, expected, got);
    }
    mit.stop().await;
    tally.finish();
}
