// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The request log and request metrics as core's `Observer` (public spec §4.5).

use std::sync::{Arc, LazyLock};
use std::time::Instant;

use axum::extract::Request;
use http::StatusCode;
use regex::Regex;
use rustic_isles::{Observation, Observer};

use crate::state::LauncherApiState;

static KILL: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"(?i)(/sessions/kill/)[^/?#]+").ok());
static JWT: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"eyJ[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]*").ok());
static SECRET_PARAM: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(
        r"(?i)([?&](?:token|access_token|refresh_token|code|exchange_code|authorization|api_key|apikey|signature)=)[^&#]*",
    )
    .ok()
});

/// Kill-session tokens, JWT-shaped segments and secret query values redacted.
pub fn sanitize_url_for_log(url: &str) -> String {
    let mut out = url.to_owned();
    if let Some(re) = KILL.as_ref() {
        out = re.replace_all(&out, "${1}[REDACTED]").into_owned();
    }
    if let Some(re) = JWT.as_ref() {
        out = re.replace_all(&out, "[REDACTED-JWT]").into_owned();
    }
    if let Some(re) = SECRET_PARAM.as_ref() {
        out = re.replace_all(&out, "${1}[REDACTED]").into_owned();
    }
    out
}

/// The metric class of a path (case-sensitive prefixes).
pub fn metric_class(path: &str) -> &'static str {
    let path = path.split(['?', '#']).next().unwrap_or("");
    if path.starts_with("/admin/") {
        "admin"
    } else if path.starts_with("/health/") {
        "health"
    } else if path.starts_with("/launcher/") {
        "launcher"
    } else {
        "game"
    }
}

/// A hunt lease or event batch, logged only when refused. D3 moved these rows to `/rustic/v1/hunts`, no alias.
pub fn is_quiet_route(path: &str) -> bool {
    let Some(rest) = path.strip_prefix("/rustic/v1/hunts/") else {
        return false;
    };
    let mut parts = rest.split('/');
    let (Some(id), Some(tail), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    !id.is_empty() && matches!(tail, "lease" | "event-batches")
}

/// The target with the `/__origin/<host>` segment split off.
fn split_origin(target: &str) -> (String, Option<String>) {
    let Some(rest) = target.strip_prefix("/__origin/") else {
        return (target.to_owned(), None);
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let host = rest[..end].to_lowercase();
    let tail = &rest[end..];
    let tail = if tail.is_empty() { "/" } else { tail };
    (tail.to_owned(), Some(host))
}

/// Core's observer over a module's metrics.
#[derive(Debug, Clone)]
pub struct RequestObserver {
    state: Arc<LauncherApiState>,
}

impl RequestObserver {
    /// An observer recording into the module's metrics.
    pub fn new(state: Arc<LauncherApiState>) -> Self {
        Self { state }
    }
}

struct Seen {
    state: Arc<LauncherApiState>,
    started: Instant,
    request_id: String,
    method: String,
    url: String,
    class: &'static str,
    line: String,
    quiet: bool,
}

impl Observer for RequestObserver {
    fn begin(&self, request: &Request, request_id: &str) -> Box<dyn Observation> {
        let target = request.uri().path_and_query().map_or("/", |p| p.as_str());
        let (target, origin) = split_origin(target);
        let path = target.split(['?', '#']).next().unwrap_or("").to_owned();
        let class = metric_class(&path);
        let method = request.method().as_str().to_owned();
        let metrics = self.state.metrics();
        metrics.increment(
            "rustic_isles_http_requests_total",
            1.0,
            &[("class", class), ("method", &method)],
        );
        metrics.adjust("rustic_isles_http_inflight", 1.0, &[]);
        let headers = request.headers();
        let host = headers
            .get("host")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("?")
            .to_owned();
        let url = sanitize_url_for_log(&target);
        let gs_key = if headers.contains_key("x-rustic-isles-gameserver-apikey") {
            "Y"
        } else {
            "N"
        };
        let bearer = headers
            .get("authorization")
            .map(|v| {
                String::from_utf8_lossy(v.as_bytes())
                    .to_lowercase()
                    .starts_with("bearer ")
            })
            .unwrap_or(false);
        let origin_text = origin.map(|o| format!(" origin={o}")).unwrap_or_default();
        let line = format!(
            "[REQ] requestId={request_id} {method} {host}{url}{origin_text} gsKey={gs_key} bearer={}",
            if bearer { "Y" } else { "N" }
        );
        let quiet = is_quiet_route(&path);
        if !quiet {
            tracing::info!("{line}");
        }
        Box::new(Seen {
            state: Arc::clone(&self.state),
            started: Instant::now(),
            request_id: request_id.to_owned(),
            method,
            url,
            class,
            line,
            quiet,
        })
    }
}

impl Observation for Seen {
    fn finish(self: Box<Self>, status: StatusCode) {
        let metrics = self.state.metrics();
        metrics.adjust("rustic_isles_http_inflight", -1.0, &[]);
        let bucket = format!("{}xx", status.as_u16() / 100);
        metrics.increment(
            "rustic_isles_http_responses_total",
            1.0,
            &[("class", self.class), ("status", &bucket)],
        );
        let ms = self.started.elapsed().as_millis();
        #[allow(clippy::cast_precision_loss, reason = "a request duration in ms, far below 2^53")]
        metrics.observe(
            "rustic_isles_http_request_duration_ms",
            ms as f64,
            &[("class", self.class)],
        );
        if self.quiet {
            if status.as_u16() < 400 {
                return;
            }
            tracing::info!("{}", self.line);
        }
        tracing::info!(
            "[RES] requestId={} {} {} \u{2192} {} ({ms}ms)",
            self.request_id,
            self.method,
            self.url,
            status.as_u16()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizing() {
        assert_eq!(
            sanitize_url_for_log("/notification/x?token=abc.def&x=1"),
            "/notification/x?token=[REDACTED]&x=1"
        );
        assert_eq!(
            sanitize_url_for_log("/x?CODE=secret&state=ok&access_token=another"),
            "/x?CODE=[REDACTED]&state=ok&access_token=[REDACTED]"
        );
        assert_eq!(sanitize_url_for_log("/balance?currency=c"), "/balance?currency=c");
        assert_eq!(
            sanitize_url_for_log("/account/api/oauth/sessions/kill/eyJhbGciOi.eyJzdWIi.sig"),
            "/account/api/oauth/sessions/kill/[REDACTED]"
        );
        assert_eq!(
            sanitize_url_for_log("/x/eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ4In0.sigvalue/y"),
            "/x/[REDACTED-JWT]/y"
        );
    }

    #[test]
    fn classes_and_quiet_routes() {
        assert_eq!(metric_class("/admin/v1/me"), "admin");
        assert_eq!(metric_class("/health/live"), "health");
        assert_eq!(metric_class("/launcher/v1/updates/x"), "launcher");
        assert_eq!(metric_class("/character"), "game");
        assert_eq!(metric_class("/internal/v1/metrics"), "game");
        assert!(is_quiet_route("/rustic/v1/hunts/abc/lease"));
        assert!(is_quiet_route("/rustic/v1/hunts/abc/event-batches"));
        assert!(!is_quiet_route("/rustic/v1/hunts/abc/end"));
        assert!(!is_quiet_route("/rustic/v1/hunts//lease"));
        assert!(!is_quiet_route("/mystic/v1/hunts/abc/lease"));
        assert_eq!(
            split_origin("/__origin/Steelyard.CA?x=1"),
            ("?x=1".to_owned(), Some("steelyard.ca".to_owned()))
        );
        assert_eq!(split_origin("/__origin/h"), ("/".to_owned(), Some("h".to_owned())));
    }
}
