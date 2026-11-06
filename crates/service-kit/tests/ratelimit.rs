// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Rate limits and their draft-8 headers through the middleware.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::middleware;
use axum::response::IntoResponse;
use axum::routing::get;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use service_kit::ratelimit::{Limited, Limiter, around, enforce, ip_key};
use tower::ServiceExt;

#[tokio::test]
async fn enforces_rate_limit_and_sets_draft8_headers() {
    let limiter = Arc::new(Limiter::new(
        2,
        Duration::from_secs(60),
        json!({"error": "too_many_requests"}),
    ));
    let limited = Limited::new(limiter, |req| {
        req.headers()
            .get("x-client-ip")
            .and_then(|v| v.to_str().ok())
            .map(ip_key)
    });

    let app = Router::new()
        .route("/ping", get(|| async { "pong" }))
        .layer(middleware::from_fn_with_state(limited, enforce));

    // Request without key passes uncounted without rate-limit headers
    let req_no_key = Request::builder().uri("/ping").body(Body::empty()).unwrap();
    let resp_no_key = app.clone().oneshot(req_no_key).await.unwrap();
    assert_eq!(resp_no_key.status(), StatusCode::OK);
    assert!(resp_no_key.headers().get("ratelimit").is_none());

    // First request for IP
    let req1 = Request::builder()
        .uri("/ping")
        .header("x-client-ip", "203.0.113.5")
        .body(Body::empty())
        .unwrap();
    let resp1 = app.clone().oneshot(req1).await.unwrap();
    assert_eq!(resp1.status(), StatusCode::OK);
    let rl1 = resp1.headers().get("ratelimit").unwrap().to_str().unwrap();
    assert!(rl1.starts_with("\"2-in-1min\"; r=1; t="));
    let rlp1 = resp1.headers().get("ratelimit-policy").unwrap().to_str().unwrap();
    assert!(rlp1.starts_with("\"2-in-1min\"; q=2; w=60; pk=:"));

    // Second request for IP
    let req2 = Request::builder()
        .uri("/ping")
        .header("x-client-ip", "::ffff:203.0.113.5")
        .body(Body::empty())
        .unwrap();
    let resp2 = app.clone().oneshot(req2).await.unwrap();
    assert_eq!(resp2.status(), StatusCode::OK);
    let rl2 = resp2.headers().get("ratelimit").unwrap().to_str().unwrap();
    assert!(rl2.starts_with("\"2-in-1min\"; r=0; t="));

    // Third request exceeds limit
    let req3 = Request::builder()
        .uri("/ping")
        .header("x-client-ip", "203.0.113.5")
        .body(Body::empty())
        .unwrap();
    let resp3 = app.oneshot(req3).await.unwrap();
    assert_eq!(resp3.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(resp3.headers().get(header::RETRY_AFTER).is_some());
    let body_bytes = resp3.into_body().collect().await.unwrap().to_bytes();
    let body_json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(body_json, json!({"error": "too_many_requests"}));
}

#[tokio::test]
async fn skip_successful_undoes_hits_for_2xx_and_3xx_responses() {
    let limiter = Limiter::new(1, Duration::from_secs(60), json!({"limited": true})).skip_successful();

    // 200 OK does not consume quota
    let r1 = around(&limiter, Some("k"), async { StatusCode::OK.into_response() }).await;
    assert_eq!(r1.status(), StatusCode::OK);

    // 401 Unauthorized consumes quota
    let r2 = around(&limiter, Some("k"), async { StatusCode::UNAUTHORIZED.into_response() }).await;
    assert_eq!(r2.status(), StatusCode::UNAUTHORIZED);

    // Next attempt is rate-limited
    let r3 = around(&limiter, Some("k"), async { StatusCode::OK.into_response() }).await;
    assert_eq!(r3.status(), StatusCode::TOO_MANY_REQUESTS);
}
