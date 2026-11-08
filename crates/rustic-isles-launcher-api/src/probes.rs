// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The probes and the metrics route (public spec §4.1 to §4.3). The release-gates probe is the admin half's.

use axum::response::{IntoResponse, Response};
use http::{StatusCode, header};
use rustic_isles::{Call, Settings};
use serde_json::Value;

use crate::state::LauncherApiState;
use crate::util::{self, object, ok, peer_text, store_of};

/// The service name the probes answer, as Node's.
pub const SERVICE: &str = "paradox-backend";

/// `api` for `api`, anything else `combined`.
pub fn role(settings: &Settings) -> &'static str {
    let raw = settings.raw("RUSTIC_ISLES_SERVICE_ROLE").unwrap_or("");
    if raw.trim().to_lowercase() == "api" {
        "api"
    } else {
        "combined"
    }
}

/// `GET /`: `ok`.
pub async fn root(_call: Call) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        "ok",
    )
        .into_response()
}

/// `GET /health/live`: always up.
pub async fn live(call: Call) -> Response {
    ok(&object(vec![
        ("ok", Value::Bool(true)),
        ("service", Value::from(SERVICE)),
        ("role", Value::from(role(call.shared.settings()))),
    ]))
}

/// `GET /health/ready`: 200 when ready and the store answers, else 503 with the reason.
pub async fn ready(call: Call) -> Response {
    let state = LauncherApiState::of(&call.shared);
    let readiness = state.readiness();
    let persistence = match store_of(&call.shared) {
        // A trivial read: row 1 of the policy table
        Some(store) => store
            .read(|conn, repos| repos.admin.read_operations_policy(conn))
            .await
            .is_ok(),
        None => false,
    };
    let ready = readiness.ready && persistence;
    let body = object(vec![
        ("ok", Value::Bool(ready)),
        ("service", Value::from(SERVICE)),
        ("role", Value::from(role(call.shared.settings()))),
        (
            "reason",
            Value::from(if ready { "READY".to_owned() } else { readiness.reason }),
        ),
        (
            "persistence",
            Value::from(if persistence { "ready" } else { "unavailable" }),
        ),
    ]);
    let status = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    util::json(status, &body)
}

/// `GET /maintenance/status`: never in maintenance.
pub async fn maintenance(_call: Call) -> Response {
    ok(&object(vec![("show-status", Value::Bool(false))]))
}

/// `GET /dauntless-status`: the welcome text in each client language.
pub async fn dauntless_status(_call: Call) -> Response {
    const WELCOME: &str = "Welcome to Rustic Isles!";
    let mut pairs = vec![("show-status", Value::Bool(true))];
    for language in ["en", "fr", "it", "es", "de", "pt", "ru", "ja"] {
        pairs.push((language, Value::from(WELCOME)));
    }
    ok(&object(pairs))
}

/// Whether the metrics caller is let in (§4.3).
pub fn metrics_authorized(settings: &Settings, headers: &http::HeaderMap, peer: &str) -> bool {
    let token = settings.raw("RUSTIC_ISLES_METRICS_TOKEN").unwrap_or("").trim();
    if token.is_empty() {
        let production = settings.raw("NODE_ENV") == Some("production");
        return !production && matches!(peer, "127.0.0.1" | "::1" | "::ffff:127.0.0.1");
    }
    let authorization = util::header_lossy(headers, "authorization");
    let supplied = match authorization {
        Some(text) if text.to_lowercase().starts_with("bearer ") => Some(text.get(7..).unwrap_or("").trim().to_owned()),
        _ => util::header_lossy(headers, "x-rustic-isles-metrics-token")
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty()),
    };
    supplied.is_some_and(|s| util::ct_eq(s.as_bytes(), token.as_bytes()))
}

/// `GET /internal/v1/metrics`: the Prometheus text, for a token or a loopback caller outside production.
pub async fn metrics(call: Call) -> Response {
    let settings = call.shared.settings();
    let peer = peer_text(&call.request);
    if !metrics_authorized(settings, call.request.headers(), &peer) {
        let set = settings
            .raw("RUSTIC_ISLES_METRICS_TOKEN")
            .is_some_and(|t| !t.is_empty());
        let status = if set {
            StatusCode::UNAUTHORIZED
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        };
        return (
            status,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            "metrics unavailable\n",
        )
            .into_response();
    }
    let state = LauncherApiState::of(&call.shared);
    let readiness = state.readiness();
    #[allow(clippy::cast_precision_loss, reason = "an epoch-ms timestamp, below 2^53")]
    let changed = readiness.changed_ms as f64 / 1000.0;
    let text = state.metrics().render(role(settings), readiness.ready, changed);
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8; version=0.0.4"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        text,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_tokens() {
        let none = Settings::default();
        let headers = http::HeaderMap::new();
        assert!(metrics_authorized(&none, &headers, "127.0.0.1"));
        assert!(!metrics_authorized(&none, &headers, "10.0.0.1"));
        let prod = Settings::from_pairs([("NODE_ENV", "production")]);
        assert!(!metrics_authorized(&prod, &headers, "127.0.0.1"));
        let spaces = Settings::from_pairs([("RUSTIC_ISLES_METRICS_TOKEN", "   ")]);
        assert!(metrics_authorized(&spaces, &headers, "::1"));
        let token = Settings::from_pairs([("RUSTIC_ISLES_METRICS_TOKEN", "t0ken")]);
        let mut h = http::HeaderMap::new();
        assert!(!metrics_authorized(&token, &h, "127.0.0.1"));
        h.insert("authorization", "Bearer wrong".parse().unwrap());
        assert!(!metrics_authorized(&token, &h, "127.0.0.1"));
        h.insert("authorization", "Bearer t0ken".parse().unwrap());
        assert!(metrics_authorized(&token, &h, "127.0.0.1"));
        let mut x = http::HeaderMap::new();
        x.insert("x-rustic-isles-metrics-token", "t0ken".parse().unwrap());
        assert!(metrics_authorized(&token, &x, "8.8.8.8"));
        x.insert("authorization", "bearer ".parse().unwrap());
        assert!(!metrics_authorized(&token, &x, "8.8.8.8"));
        let mut legacy = http::HeaderMap::new();
        legacy.insert("x-mysticparadox-metrics-token", "t0ken".parse().unwrap());
        assert!(!metrics_authorized(&token, &legacy, "127.0.0.1"));
        assert_eq!(
            role(&Settings::from_pairs([("RUSTIC_ISLES_SERVICE_ROLE", " API ")])),
            "api"
        );
        assert_eq!(
            role(&Settings::from_pairs([("RUSTIC_ISLES_SERVICE_ROLE", "worker")])),
            "combined"
        );
    }
}
