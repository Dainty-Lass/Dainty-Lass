//! RU5 reforged-api: every case of `golden/reforged_api.json` is requested from the Rust backend and must be answered
//! as the reference routers answered it under Node 22 (`golden/reforged_api_capture.mts`): status, the headers a client
//! sees and the body text. A case that differs is the Rust side's bug, unless it is listed in `KNOWN` with its reason.
//!
//! The cases run on both versions' routes where the area exists in both (the Node-scope routes are served by the 1.12
//! module too, RU3c seam 7); the reference served 1.14.7's changelist.
#![allow(clippy::unwrap_used, clippy::expect_used)] // tests

mod common;

use common::{Fakes, send, serve, shared, store};
use isles_core::areas::reforged_api::launcher_api::forget_status;
use isles_core::areas::reforged_api::launcher_auth::set_authenticator;
use isles_core::routes::RouteSet;
use serde_json::Value;
use std::path::Path;

const HEADERS: [&str; 7] = [
    "content-type",
    "cache-control",
    "content-length",
    "location",
    "content-disposition",
    "vary",
    "retry-after",
];

/// Differences the Rust side keeps on purpose, by case name.
const KNOWN: [(&str, &str); 0] = [];

fn normalize(body: &str) -> String {
    let ids = regex::Regex::new(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}").unwrap();
    let uptime = regex::Regex::new(r#""uptimeSeconds":\d+"#).unwrap();
    let body = ids.replace_all(body, "<id>");
    uptime.replace_all(&body, r#""uptimeSeconds":0"#).into_owned()
}

fn write_files(dir: &Path, files: &serde_json::Map<String, Value>) {
    for (name, text) in files {
        let target = dir.join(name);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, text.as_str().unwrap()).unwrap();
    }
}

async fn replay(case: &Value, routes: RouteSet) -> Vec<String> {
    let name = case["name"].as_str().unwrap();
    // The status is cached for five seconds, process-wide
    forget_status();
    let dir = tempfile::tempdir().unwrap();
    let db = tempfile::tempdir().unwrap();
    write_files(dir.path(), case["files"].as_object().unwrap());
    let mut pairs: Vec<(String, String)> = vec![
        ("LAUNCHER_UPDATE_ROOT".into(), dir.path().display().to_string()),
        (
            "LAUNCHER_LOG_STORAGE_DIR".into(),
            dir.path().join("logs").display().to_string(),
        ),
        ("MYSTICPARADOX_ADMIN_API".into(), "on".into()),
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
    let fakes = Fakes::default();
    let address = serve(shared(routes, &borrowed, Some(store(&db, &fakes)))).await;

    let request = &case["request"];
    let headers: Vec<(String, String)> = request["headers"]
        .as_object()
        .map(|h| {
            h.iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_owned()))
                .collect()
        })
        .unwrap_or_default();
    let borrowed_headers: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let body = request["json"]
        .as_str()
        .map(|json| ("application/json", json.as_bytes()));
    let answer = send(
        address,
        request["method"].as_str().unwrap(),
        request["path"].as_str().unwrap(),
        &borrowed_headers,
        body,
    )
    .await;

    let expected = &case["response"];
    let mut diffs = Vec::new();
    let mut check = |what: &str, got: String, want: String| {
        if got != want {
            diffs.push(format!("{name} [{routes:?}] {what}: got {got:?}, wanted {want:?}"));
        }
    };
    check("status", answer.status.to_string(), expected["status"].to_string());
    let compare_body = !expected["body"].is_null();
    let golden_headers = expected["headers"].as_object().unwrap();
    for header in HEADERS {
        if header == "content-length" && !compare_body {
            continue;
        }
        // uptimeSeconds grows a digit once the run is ten seconds old, and only the shape of the body is recorded
        if header == "content-length" && name.starts_with("server status") {
            continue;
        }
        let want = golden_headers.get(header).map(|v| v.as_str().unwrap().to_owned());
        let got = answer.header(header).map(normalize);
        check(header, format!("{got:?}"), format!("{want:?}"));
    }
    if compare_body {
        check(
            "body",
            normalize(&answer.text()),
            expected["body"].as_str().unwrap().to_owned(),
        );
    }
    // The serving switch is a file the next request reads
    if let Some(after) = case.get("servingAfter") {
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.path().join("serving.json")).unwrap()).unwrap();
        check("serving.json", written["runtimeEnabled"].to_string(), after.to_string());
    }
    diffs
}

#[tokio::test(flavor = "multi_thread")]
async fn the_rust_backend_answers_as_the_reference_routers_did() {
    // No launcher authentication is registered: the cases that need a sign-in get Node's "Sign in required"
    set_authenticator(None);
    let cases: Value = serde_json::from_str(
        &std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/reforged_api.json")).unwrap(),
    )
    .unwrap();
    let cases = cases.as_array().unwrap();
    assert!(cases.len() > 100);
    let mut diffs = Vec::new();
    for case in cases {
        let name = case["name"].as_str().unwrap();
        if KNOWN.iter().any(|(known, _)| *known == name) {
            continue;
        }
        diffs.extend(replay(case, RouteSet::V1147).await);
        // What names the changelist (the feeds, the status) is the reference's 647472 only
        let path = case["request"]["path"].as_str().unwrap();
        let by_changelist = path.starts_with("/launcher/v1/runtime/")
            || path.starts_with("/launcher/v1/guard-manifests/")
            || path.starts_with("/reforged/api/ServerStatus");
        // Only 1.14.7 has the login queue's maintenance probe (RU3c)
        if !by_changelist && path != "/maintenance/status" {
            diffs.extend(replay(case, RouteSet::V1120).await);
        }
    }
    assert!(diffs.is_empty(), "{} differences:\n{}", diffs.len(), diffs.join("\n"));
}
