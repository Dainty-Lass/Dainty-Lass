// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! JSON and form bodies, encodings and refusals through the body middleware.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

use std::io::Write;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use axum::middleware;
use axum::response::IntoResponse;
use axum::routing::post;
use dainty_service_kit::body::{BodyConfig, BodyError, ExpressBody, parse_bodies};
use dainty_service_kit::json::Json;
use flate2::Compression;
use flate2::write::{GzEncoder, ZlibEncoder};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

async fn echo_handler(ExpressBody(body): ExpressBody, raw: axum::body::Bytes) -> impl IntoResponse {
    Json(json!({
        "parsed": body,
        "raw": String::from_utf8_lossy(&raw),
    }))
}

fn app_with_config(config: BodyConfig) -> Router {
    Router::new()
        .route("/echo", post(echo_handler))
        .layer(middleware::from_fn_with_state(config, parse_bodies))
}

#[tokio::test]
async fn parses_json_and_preserves_raw_downstream_body() {
    let app = app_with_config(BodyConfig::default());
    let req = Request::builder()
        .method("POST")
        .uri("/echo")
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .body(Body::from(r#"{"hello":"world"}"#))
        .unwrap();

    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let val: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(val["parsed"], json!({"hello": "world"}));
    assert_eq!(val["raw"], r#"{"hello":"world"}"#);
}

#[tokio::test]
async fn parses_urlencoded_form_and_gzip_deflate_bodies() {
    let app = app_with_config(BodyConfig::default());

    let req = Request::builder()
        .method("POST")
        .uri("/echo")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from("x=1&x=2&msg=hi+there%21"))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let val: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(val["parsed"], json!({"x": ["1", "2"], "msg": "hi there!"}));

    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    gz.write_all(br#"{"compressed":true}"#).unwrap();
    let gz_bytes = gz.finish().unwrap();

    let req_gz = Request::builder()
        .method("POST")
        .uri("/echo")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CONTENT_ENCODING, "gzip")
        .body(Body::from(gz_bytes))
        .unwrap();
    let resp_gz = app.clone().oneshot(req_gz).await.unwrap();
    assert_eq!(resp_gz.status(), StatusCode::OK);
    let val_gz: Value = serde_json::from_slice(&resp_gz.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(val_gz["parsed"], json!({"compressed": true}));

    let mut zl = ZlibEncoder::new(Vec::new(), Compression::default());
    zl.write_all(br#"{"deflated":1}"#).unwrap();
    let zl_bytes = zl.finish().unwrap();

    let req_zl = Request::builder()
        .method("POST")
        .uri("/echo")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CONTENT_ENCODING, "deflate")
        .body(Body::from(zl_bytes))
        .unwrap();
    let resp_zl = app.oneshot(req_zl).await.unwrap();
    assert_eq!(resp_zl.status(), StatusCode::OK);
    let val_zl: Value = serde_json::from_slice(&resp_zl.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(val_zl["parsed"], json!({"deflated": 1}));
}

#[tokio::test]
async fn rejects_malformed_too_large_and_unsupported_charset_or_encoding() {
    let config = BodyConfig {
        limit: 16,
        on_error: Arc::new(|_, _| None),
    };
    let app = app_with_config(config);

    // Too large
    let req_large = Request::builder()
        .method("POST")
        .uri("/echo")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"long_key":"0123456789abcdef"}"#))
        .unwrap();
    let resp_large = app.clone().oneshot(req_large).await.unwrap();
    assert_eq!(resp_large.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        resp_large.headers().get(header::CONTENT_SECURITY_POLICY).unwrap(),
        "default-src 'none'"
    );

    // Unsupported charset
    let req_charset = Request::builder()
        .method("POST")
        .uri("/echo")
        .header(header::CONTENT_TYPE, "application/json; charset=iso-8859-1")
        .body(Body::from("{}"))
        .unwrap();
    let resp_charset = app.clone().oneshot(req_charset).await.unwrap();
    assert_eq!(resp_charset.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

    // Unsupported content-encoding
    let req_enc = Request::builder()
        .method("POST")
        .uri("/echo")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CONTENT_ENCODING, "br")
        .body(Body::from("{}"))
        .unwrap();
    let resp_enc = app.clone().oneshot(req_enc).await.unwrap();
    assert_eq!(resp_enc.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

    // Custom on_error override
    let custom_app = app_with_config(BodyConfig {
        limit: 16,
        on_error: Arc::new(|_, err| {
            assert_eq!(err, BodyError::Malformed);
            Some((StatusCode::UNPROCESSABLE_ENTITY, "custom error").into_response())
        }),
    });
    let req_bad = Request::builder()
        .method("POST")
        .uri("/echo")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{bad"))
        .unwrap();
    let resp_bad = custom_app.oneshot(req_bad).await.unwrap();
    assert_eq!(resp_bad.status(), StatusCode::UNPROCESSABLE_ENTITY);
}
