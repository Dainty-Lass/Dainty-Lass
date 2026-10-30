// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

use std::collections::HashMap;
use std::fmt::Write;
use std::future::Future;
use std::net::{IpAddr, Ipv6Addr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::http::header::{self, HeaderName, HeaderValue};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    pub remaining: u64,
    pub reset_secs: u64,
    pub limited: bool,
    reset_at: Instant,
}

struct Bucket {
    hits: u64,
    reset_at: Instant,
}

pub struct Limiter {
    limit: u64,
    window: Duration,
    name: String,
    reply: Value,
    skip_successful: bool,
    buckets: Mutex<HashMap<String, Bucket>>,
}

fn format_window_name(window: Duration) -> String {
    let secs = window.as_secs();
    if secs < 60 {
        format!("{secs}sec")
    } else if secs < 3600 {
        format!("{}min", secs / 60)
    } else if secs < 86400 {
        let hrs = secs / 3600;
        if hrs == 1 {
            "1hr".to_owned()
        } else {
            format!("{hrs}hrs")
        }
    } else {
        let days = secs / 86400;
        if days == 1 {
            "1day".to_owned()
        } else {
            format!("{days}days")
        }
    }
}

fn partition_key(key: &str) -> String {
    let digest = Sha256::digest(key.as_bytes());
    let mut hex12 = String::with_capacity(12);
    for b in &digest[..6] {
        let _ = write!(hex12, "{b:02x}");
    }
    BASE64.encode(hex12.as_bytes())
}

impl Limiter {
    pub fn new(limit: u64, window: Duration, reply: Value) -> Self {
        let name = format!("{limit}-in-{}", format_window_name(window));
        Self {
            limit,
            window,
            name,
            reply,
            skip_successful: false,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    pub fn skip_successful(mut self) -> Self {
        self.skip_successful = true;
        self
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn hit(&self, key: &str, now: Instant) -> Decision {
        let mut map = self
            .buckets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let bucket = match map.get_mut(key) {
            Some(b) if now < b.reset_at => {
                b.hits = b.hits.saturating_add(1);
                b
            }
            _ => {
                let reset_at = now + self.window;
                map.insert(key.to_owned(), Bucket { hits: 1, reset_at });
                map.get_mut(key).unwrap_or_else(|| unreachable!())
            }
        };

        let rem_dur = bucket.reset_at.saturating_duration_since(now);
        let reset_secs = if rem_dur.subsec_nanos() > 0 {
            rem_dur.as_secs().saturating_add(1)
        } else {
            rem_dur.as_secs()
        };

        Decision {
            remaining: self.limit.saturating_sub(bucket.hits),
            reset_secs,
            limited: bucket.hits > self.limit,
            reset_at: bucket.reset_at,
        }
    }

    pub fn undo(&self, key: &str, decision: &Decision, now: Instant) {
        if now >= decision.reset_at {
            return;
        }
        let mut map = self
            .buckets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(bucket) = map.get_mut(key)
            && bucket.reset_at == decision.reset_at
        {
            bucket.hits = bucket.hits.saturating_sub(1);
        }
    }

    pub fn sweep(&self, now: Instant) {
        let mut map = self
            .buckets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        map.retain(|_, b| b.reset_at > now);
    }

    pub fn spawn_sweeper(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let limiter = Arc::clone(self);
        let period = limiter.window.max(Duration::from_secs(1));
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(period);
            interval.tick().await;
            loop {
                interval.tick().await;
                limiter.sweep(Instant::now());
            }
        })
    }

    pub fn headers(&self, key: &str, decision: &Decision) -> [(HeaderName, String); 2] {
        let pk = partition_key(key);
        [
            (
                HeaderName::from_static("ratelimit"),
                format!(
                    "\"{}\"; r={}; t={}",
                    self.name, decision.remaining, decision.reset_secs
                ),
            ),
            (
                HeaderName::from_static("ratelimit-policy"),
                format!(
                    "\"{}\"; q={}; w={}; pk=:{pk}:",
                    self.name,
                    self.limit,
                    self.window.as_secs()
                ),
            ),
        ]
    }
}

pub fn ip_key(address: &str) -> String {
    match address.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => v4.to_string(),
        Ok(IpAddr::V6(v6)) => {
            if let Some(v4) = v6.to_ipv4() {
                return v4.to_string();
            }
            let mask = !0u128 << (128 - 56);
            let network = Ipv6Addr::from_bits(v6.to_bits() & mask);
            format!("{network}/56")
        }
        Err(_) => address.to_owned(),
    }
}

type KeyFn = dyn Fn(&Request) -> Option<String> + Send + Sync;

#[derive(Clone)]
pub struct Limited {
    limiter: Arc<Limiter>,
    key: Arc<KeyFn>,
}

impl Limited {
    pub fn new(
        limiter: Arc<Limiter>,
        key: impl Fn(&Request) -> Option<String> + Send + Sync + 'static,
    ) -> Self {
        Self {
            limiter,
            key: Arc::new(key),
        }
    }
}

pub async fn enforce(
    State(limited): State<Limited>,
    request: Request,
    next: Next,
) -> Response {
    let key = (limited.key)(&request);
    around(&limited.limiter, key.as_deref(), next.run(request)).await
}

pub async fn around(
    limiter: &Limiter,
    key: Option<&str>,
    handler: impl Future<Output = Response>,
) -> Response {
    let Some(key) = key else {
        return handler.await;
    };

    let decision = limiter.hit(key, Instant::now());
    let hdrs = limiter.headers(key, &decision);

    let mut response = if decision.limited {
        let mut resp = (StatusCode::TOO_MANY_REQUESTS, crate::json::Json(&limiter.reply)).into_response();
        if let Ok(val) = HeaderValue::from_str(&decision.reset_secs.to_string()) {
            resp.headers_mut().insert(header::RETRY_AFTER, val);
        }
        resp
    } else {
        let resp = handler.await;
        if limiter.skip_successful && resp.status().as_u16() < 400 {
            limiter.undo(key, &decision, Instant::now());
        }
        resp
    };

    for (name, val) in hdrs {
        if let Ok(hv) = HeaderValue::from_str(&val) {
            response.headers_mut().insert(name, hv);
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn formats_policy_names_and_ip_keys() {
        assert_eq!(Limiter::new(10, Duration::from_secs(30), json!({})).name(), "10-in-30sec");
        assert_eq!(Limiter::new(5, Duration::from_secs(900), json!({})).name(), "5-in-15min");
        assert_eq!(Limiter::new(1, Duration::from_secs(3600), json!({})).name(), "1-in-1hr");
        assert_eq!(Limiter::new(2, Duration::from_secs(7200), json!({})).name(), "2-in-2hrs");
        assert_eq!(Limiter::new(100, Duration::from_secs(86400), json!({})).name(), "100-in-1day");
        assert_eq!(Limiter::new(100, Duration::from_secs(172800), json!({})).name(), "100-in-2days");

        assert_eq!(ip_key("192.168.1.1"), "192.168.1.1");
        assert_eq!(ip_key("::ffff:10.0.0.1"), "10.0.0.1");
        assert_eq!(ip_key("2001:db8:1234:5678:9abc:def0:1234:5678"), "2001:db8:1234:5600::/56");
        assert_eq!(ip_key("custom-key"), "custom-key");
    }

    #[test]
    fn hit_undo_and_sweep_track_window_state() {
        let limiter = Limiter::new(2, Duration::from_secs(60), json!({"error": "rate_limited"}));
        let start = Instant::now();

        let d1 = limiter.hit("user1", start);
        assert_eq!(d1.remaining, 1);
        assert!(!d1.limited);

        let d2 = limiter.hit("user1", start + Duration::from_secs(1));
        assert_eq!(d2.remaining, 0);
        assert!(!d2.limited);

        limiter.undo("user1", &d2, start + Duration::from_secs(1));

        let d3 = limiter.hit("user1", start + Duration::from_secs(2));
        assert_eq!(d3.remaining, 0);
        assert!(!d3.limited);

        let d4 = limiter.hit("user1", start + Duration::from_secs(3));
        assert_eq!(d4.remaining, 0);
        assert!(d4.limited);

        let after_window = start + Duration::from_secs(61);
        let d5 = limiter.hit("user1", after_window);
        assert_eq!(d5.remaining, 1);
        assert!(!d5.limited);
    }
}
