// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Runs `cases/hostapi.json`, the MIT table of host API replies written from spec §2.6.3 (L5).

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use bytes::Bytes;
use dainty_launcher_kit::net::hostapi::parse::{
    StatusReply, branding_image_reply, branding_reply, is_branding_path, news_reply, probe_reply, register_reply,
    runtime_manifest_reply, status_reply, user_info_reply,
};
use dainty_launcher_kit::net::hostapi::{KEY_HEADER, key_headers};
use dainty_launcher_kit::net::http::Endpoint;
use dainty_launcher_kit::net::runtime_files::RuntimeManifestResult;
use serde_json::{Value, json};

fn table() -> Value {
    serde_json::from_str(include_str!("cases/hostapi.json")).unwrap()
}

/// The entry with every `@FILL@` replaced by its `fill`.
fn filled(entry: &Value) -> Value {
    fn walk(value: &Value, fill: &str) -> Value {
        match value {
            Value::String(text) => Value::String(text.replace("@FILL@", fill)),
            Value::Array(items) => Value::Array(items.iter().map(|v| walk(v, fill)).collect()),
            Value::Object(map) => Value::Object(map.iter().map(|(k, v)| (k.clone(), walk(v, fill))).collect()),
            other => other.clone(),
        }
    }
    let fill = entry.get("fill").map_or(String::new(), |f| {
        f["text"]
            .as_str()
            .unwrap()
            .repeat(usize::try_from(f["count"].as_u64().unwrap()).unwrap())
    });
    walk(entry, &fill)
}

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&text[at..at + 2], 16).unwrap())
        .collect()
}

fn body_of(case: &Value) -> Vec<u8> {
    if let Some(text) = case.get("body") {
        text.as_str().unwrap().as_bytes().to_vec()
    } else if let Some(json) = case.get("body_json") {
        serde_json::to_vec(json).unwrap()
    } else {
        hex(case["body_hex"].as_str().unwrap())
    }
}

fn outcome<T: serde::Serialize, E: serde::Serialize>(result: Result<T, E>, ok: &str) -> Value {
    match result {
        Ok(value) => json!({ ok: value }),
        Err(err) => json!({ "error": err }),
    }
}

/// The parser's answer in the table's terms.
fn run(parser: &str, status: u16, body: Vec<u8>) -> Value {
    match parser {
        "status" => match status_reply(status, &body) {
            StatusReply::Status(s) => json!({ "status": { "name": s.name, "limited": s.limited } }),
            StatusReply::Probe => json!("probe"),
            StatusReply::BadAnswer => json!("bad_answer"),
        },
        "probe" => json!(if probe_reply(status, &body) {
            "unsupported"
        } else {
            "bad_answer"
        }),
        "register" => outcome(register_reply(status, &body), "key"),
        "user_info" => outcome(user_info_reply(status, &body), "user"),
        "runtime_manifest" => match runtime_manifest_reply(status, &body) {
            RuntimeManifestResult::Ok(manifest) => json!({ "manifest": manifest }),
            RuntimeManifestResult::None => json!("none"),
            RuntimeManifestResult::BadAnswer => json!("bad_answer"),
            other => panic!("a parser never answers {other:?}"),
        },
        "news" => news_reply(status, &body).map_or(json!("fallback"), |items| json!({ "items": items })),
        "branding" => serde_json::to_value(branding_reply(status, &body)).unwrap(),
        "branding_image" => branding_image_reply(status, Bytes::from(body)).map_or(
            Value::Null,
            |image| json!({ "type": image.kind.content_type(), "bytes": image.bytes.len() }),
        ),
        other => panic!("unknown parser {other}"),
    }
}

#[test]
fn every_parser_answers_as_the_table_says() {
    let table = table();
    let cases = table["cases"].as_array().unwrap();
    for parser in [
        "status",
        "probe",
        "register",
        "user_info",
        "runtime_manifest",
        "news",
        "branding",
        "branding_image",
    ] {
        assert!(
            cases.iter().filter(|c| c["parser"] == parser).count() >= 8,
            "{parser} has too few cases"
        );
    }
    for case in cases.iter().map(filled) {
        let name = format!(
            "{} / {}",
            case["parser"].as_str().unwrap(),
            case["name"].as_str().unwrap()
        );
        let status = u16::try_from(case["status"].as_u64().unwrap()).unwrap();
        let got = run(case["parser"].as_str().unwrap(), status, body_of(&case));
        assert_eq!(got, case["expected"], "{name}");
    }
}

#[test]
fn branding_paths_as_the_table_says() {
    let table = table();
    for entry in table["branding_paths"].as_array().unwrap().iter().map(filled) {
        let path = entry["path"].as_str().unwrap();
        assert_eq!(is_branding_path(path), entry["ok"].as_bool().unwrap(), "{path:?}");
    }
}

#[test]
fn the_key_goes_only_where_the_table_says() {
    let table = table();
    for entry in table["key_headers"].as_array().unwrap() {
        let endpoint = Endpoint {
            host: entry["host"].as_str().unwrap().to_owned(),
            port: u16::try_from(entry["port"].as_u64().unwrap()).unwrap(),
            pin: entry["pin"].as_str().map(str::to_owned),
        };
        let key = entry["key"].as_str();
        let headers = key_headers(&endpoint, key);
        let name = entry["name"].as_str().unwrap();
        if entry["sent"].as_bool().unwrap() {
            assert_eq!(headers, [(KEY_HEADER, key.unwrap().to_owned())], "{name}");
        } else {
            assert!(headers.is_empty(), "{name}");
        }
    }
}

#[test]
fn login_queue_replies() {
    use dainty_launcher_kit::bridge::types::ErrorCode;
    use dainty_launcher_kit::net::hostapi::parse::{QueueReply, queue_reply};
    let code = |status: u16, body: &str| queue_reply(status, body.as_bytes()).unwrap_err().code;
    assert_eq!(queue_reply(200, br#"{"admitted":true}"#), Ok(QueueReply::Admitted));
    assert_eq!(
        queue_reply(200, br#"{"admitted":false,"position":3}"#),
        Ok(QueueReply::Waiting(3))
    );
    for bad in [
        r#"{"admitted":false,"position":0}"#,
        r#"{"admitted":false}"#,
        "{}",
        "nope",
    ] {
        assert_eq!(code(200, bad), ErrorCode::ServerError, "{bad}");
    }
    // No queue route on an older server or gateway: PLAY goes ahead
    for status in [403, 404, 405] {
        assert_eq!(
            queue_reply(status, br#"{"error":"forbidden"}"#),
            Ok(QueueReply::Admitted)
        );
    }
    assert_eq!(code(401, "{}"), ErrorCode::KeyRejected);
    assert_eq!(code(403, r#"{"error":"account_disabled"}"#), ErrorCode::AccountBanned);
    assert_eq!(code(429, ""), ErrorCode::RateLimited);
    assert_eq!(code(500, ""), ErrorCode::ServerError);
}
