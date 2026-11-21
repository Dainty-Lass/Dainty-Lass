// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

#![allow(clippy::unwrap_used, clippy::expect_used)]

use axum::body::Body;
use axum::extract::{Extension, State};
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
use service_kit::json::not_found;
use service_kit::routing::{MatchedPattern, Params, Router};
use tower::ServiceExt;

#[tokio::test]
async fn matches_in_registration_order_case_insensitively_and_handles_head() {
    let router = Router::new()
        .get("/items/special", || async { "special-item" })
        .get(
            "/items/:id",
            |params: Params, Extension(matched): Extension<MatchedPattern>| async move {
                format!("{}:{}", matched.0, params.get("id").unwrap_or(""))
            },
        )
        .post(
            "/items/:id",
            |State(prefix): State<String>, params: Params| async move {
                format!("{prefix}:{}", params.get("id").unwrap_or(""))
            },
        );

    assert_eq!(
        router.table(),
        vec![
            (Method::GET, "/items/special".to_owned()),
            (Method::GET, "/items/:id".to_owned()),
            (Method::POST, "/items/:id".to_owned()),
        ]
    );

    let svc = router.into_service("created".to_owned(), not_found);

    // Registration order priority + case-insensitivity + trailing slash
    let req1 = Request::builder()
        .method(Method::GET)
        .uri("/ITEMS/SPECIAL/")
        .body(Body::empty())
        .unwrap();
    let resp1 = svc.clone().oneshot(req1).await.unwrap();
    assert_eq!(resp1.status(), StatusCode::OK);
    let body1 = resp1.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body1[..], b"special-item");

    // Parameter extraction and percent decoding
    let req2 = Request::builder()
        .method(Method::GET)
        .uri("/items/hello%20world")
        .body(Body::empty())
        .unwrap();
    let resp2 = svc.clone().oneshot(req2).await.unwrap();
    assert_eq!(resp2.status(), StatusCode::OK);
    let body2 = resp2.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body2[..], b"/items/:id:hello world");

    // HEAD request on GET route returns 200 with empty body
    let req_head = Request::builder()
        .method(Method::HEAD)
        .uri("/items/42")
        .body(Body::empty())
        .unwrap();
    let resp_head = svc.clone().oneshot(req_head).await.unwrap();
    assert_eq!(resp_head.status(), StatusCode::OK);
    let body_head = resp_head.into_body().collect().await.unwrap().to_bytes();
    assert!(body_head.is_empty());

    // Invalid UTF-8 parameter returns 400 Bad Request
    let req_bad = Request::builder()
        .method(Method::GET)
        .uri("/items/%FF")
        .body(Body::empty())
        .unwrap();
    let resp_bad = svc.clone().oneshot(req_bad).await.unwrap();
    assert_eq!(resp_bad.status(), StatusCode::BAD_REQUEST);

    // Unmatched route calls fallback (404)
    let req_404 = Request::builder()
        .method(Method::DELETE)
        .uri("/items/42")
        .body(Body::empty())
        .unwrap();
    let resp_404 = svc.oneshot(req_404).await.unwrap();
    assert_eq!(resp_404.status(), StatusCode::NOT_FOUND);
}
