//! `isles-core/tests/golden/request_budget.json` (express-rate-limit 8.7.0's answers) against the MIT module's
//! request budget, at the recorded times through the replay clock header, as restricted's `pipeline.rs` replays it.
//!
//! Restricted puts a test handler answering `{"ok":true}` at `GET /character`; the composed module has the real route
//! there (401 without a token). So an allowed hit is compared as "not refused" plus the budget's headers, and only
//! the refused hit's status, type and body are compared byte for byte.
#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use common::{Mit, Tally, send};
use serde_json::{Value, json};

#[tokio::test(flavor = "multi_thread")]
async fn request_budget() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/isles-core/tests/golden/request_budget.json");
    let golden: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mit = Mit::start(
        &[
            ("RUSTIC_ISLES_RATE_LIMIT", "on"),
            ("RUSTIC_ISLES_RATE_LIMIT_PER_MINUTE", "2"),
            ("RUSTIC_ISLES_GAMESERVER_RATE_LIMIT_PER_MINUTE", "4"),
            ("RUSTIC_ISLES_HOST_RATE_LIMIT_PER_MINUTE", "5"),
        ],
        |_| {},
    )
    .await;
    let mut tally = Tally::new("request_budget");
    let summary = |status: u64, headers: &dyn Fn(&str) -> Option<String>, body: &str| {
        let refused = status == 429;
        let mut out = json!({
            "status": if refused { json!(429) } else { json!("not refused") },
            "ratelimit": headers("ratelimit"),
            "ratelimit-policy": headers("ratelimit-policy"),
            "retry-after": headers("retry-after"),
        });
        if refused {
            out["content-type"] = json!(headers("content-type"));
            out["body"] = json!(body);
        }
        out
    };
    for (n, case) in golden["cases"].as_array().unwrap().iter().enumerate() {
        let name = case["name"].as_str().unwrap();
        let mut headers = vec![(
            "x-isles-replay-now-ms".to_owned(),
            case["nowMs"].as_i64().unwrap().to_string(),
        )];
        for (k, v) in case["headers"].as_object().unwrap() {
            // D3 (spec-rustic-isles-1-14-7 §7.2): the host capability header has one name now
            let k = if k == "x-mystic-host-capability" {
                tally.note(format!(
                    "{name}: x-mystic-host-capability sent as x-rustic-host-capability (D3)"
                ));
                "x-rustic-host-capability".to_owned()
            } else {
                k.clone()
            };
            headers.push((k, v.as_str().unwrap().to_owned()));
        }
        let reply = send(mit.backend, "GET", "/character", &headers, None).await;
        let want = &case["response"];
        let expected = summary(
            want["status"].as_u64().unwrap(),
            &|h| want["headers"][h].as_str().map(str::to_owned),
            want["body"].as_str().unwrap(),
        );
        let actual = summary(
            u64::from(reply.status),
            &|h| reply.header(h).map(str::to_owned),
            &reply.body,
        );
        if name.starts_with("host capability") {
            // The host bucket is the one rifts' guard names for a capability it recognises; restricted's test guard
            // knows `valid-cap`, the composed module's (dedicated-only) knows none and a real one needs a live host
            tally.skip(
                n + 1,
                name,
                "GET",
                "/character",
                "needs a rifts guard that recognises the capability",
                actual,
            );
            continue;
        }
        tally.step(n + 1, name, "GET", "/character", expected, actual);
    }
    tally.finish();
}
