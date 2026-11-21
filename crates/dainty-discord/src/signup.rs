// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Whether dainty-dash takes signups: its public `GET /api/public/signup` (A8), `{ "enabled": bool, "url": string |
//! null }`, read at most once a minute (D3). Any failure reads as off, so `/key` falls back to a login URI.

use std::sync::Mutex;
use std::time::Duration;

use bytes::Bytes;
use dainty_module_api::BoxFuture;
use http_body_util::Full;
use serde_json::Value;
use tokio::time::Instant;

use crate::bot::SignupSource;

/// The path under the dashboard's URL.
pub const SIGNUP_PATH: &str = "api/public/signup";
/// How long an answer, or a failure, is kept.
pub const CACHE: Duration = Duration::from_secs(60);
/// The largest answer read.
pub const MAX_ANSWER: usize = 4 * 1024;
/// The longest signup URL passed on.
pub const MAX_URL: usize = 512;
const TIMEOUT: Duration = Duration::from_secs(5);

/// The signup page in a dashboard's answer, when signups are on and the page is an http(s) URL.
pub fn parse_answer(body: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(body).ok()?;
    if value.get("enabled").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let text = value.get("url").and_then(Value::as_str)?;
    if text.len() > MAX_URL {
        return None;
    }
    let url = url::Url::parse(text).ok()?;
    let ok = matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none();
    // The URL as parsed: no raw whitespace or markup reaches the message
    ok.then(|| url.to_string())
}

/// dainty-dash, read through the cache.
pub struct DashboardSignup {
    endpoint: Option<url::Url>,
    client: dainty_http::Client,
    cache: Mutex<Option<(Instant, Option<String>)>>,
}

impl DashboardSignup {
    /// Reads `dashboard`'s endpoint; with no dashboard, signups are off.
    pub fn new(dashboard: Option<&url::Url>) -> Self {
        let endpoint = dashboard.and_then(|base| {
            let mut base = base.clone();
            if !base.path().ends_with('/') {
                base.set_path(&format!("{}/", base.path()));
            }
            base.join(SIGNUP_PATH).ok()
        });
        let client = dainty_http::client(dainty_http::Schemes::HttpOrHttps);
        Self {
            endpoint,
            client,
            cache: Mutex::new(None),
        }
    }

    async fn fetch(&self) -> Option<String> {
        let endpoint = self.endpoint.as_ref()?;
        let request = http::Request::get(endpoint.as_str())
            .header(http::header::ACCEPT, "application/json")
            .body(Full::new(Bytes::new()))
            .ok()?;
        let answer = dainty_http::call(&self.client, request, MAX_ANSWER, TIMEOUT)
            .await
            .ok()?;
        if !answer.status.is_success() {
            return None;
        }
        parse_answer(&answer.body)
    }
}

impl SignupSource for DashboardSignup {
    fn signup_url(&self) -> BoxFuture<'_, Option<String>> {
        Box::pin(async move {
            let now = Instant::now();
            let cached = self
                .cache
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
                .filter(|(at, _)| now.duration_since(*at) < CACHE);
            if let Some((_, url)) = cached {
                return url;
            }
            let url = self.fetch().await;
            *self.cache.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some((now, url.clone()));
            url
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_only_an_enabled_http_url() {
        assert_eq!(
            parse_answer(br#"{"enabled":true,"url":"https://dash.example/signup"}"#).as_deref(),
            Some("https://dash.example/signup")
        );
        for body in [
            &br#"{"enabled":false,"url":"https://dash.example/signup"}"#[..],
            br#"{"enabled":true,"url":null}"#,
            br#"{"enabled":"true","url":"https://dash.example/signup"}"#,
            br#"{"enabled":true,"url":"javascript:alert(1)"}"#,
            br#"{"enabled":true,"url":"https://u:p@dash.example/"}"#,
            br#"{"enabled":true,"url":"dash.example/signup"}"#,
            b"<html>",
        ] {
            assert_eq!(parse_answer(body), None, "{}", String::from_utf8_lossy(body));
        }
        let long = format!(
            r#"{{"enabled":true,"url":"https://d.example/{}"}}"#,
            "a".repeat(MAX_URL)
        );
        assert_eq!(parse_answer(long.as_bytes()), None);
    }

    #[test]
    fn the_endpoint_sits_under_the_dashboard() {
        let base = url::Url::parse("https://dash.example/staff").ok();
        let signup = DashboardSignup::new(base.as_ref());
        assert_eq!(
            signup.endpoint.as_ref().map(url::Url::as_str),
            Some("https://dash.example/staff/api/public/signup")
        );
        assert!(DashboardSignup::new(None).endpoint.is_none());
    }

    #[tokio::test]
    async fn no_dashboard_reads_as_off() {
        assert_eq!(DashboardSignup::new(None).signup_url().await, None);
    }
}
