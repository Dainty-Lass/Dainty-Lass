// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The allowlist feed: which addresses logged in, told to the helper fire and forget, so a player's request is
//! never delayed by it.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use bytes::Bytes;
use dainty_service_kit::linelog::{self, Throttle};
use http_body_util::{BodyExt as _, Full, Limited};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use serde_json::json;
use tokio::sync::watch;

use crate::config::Allowlist;

/// Where an in-process helper takes an address (DN8): no HTTP, no secret. `true` when it was admitted.
pub type InProcessSink = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// Milliseconds, replaceable in tests.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

const MAX_REMEMBERED: usize = 50_000;
const FIRST_BACKOFF_MS: i64 = 2000;
const MAX_BACKOFF_MS: i64 = 60_000;

#[derive(Default)]
struct HelperState {
    /// Addresses the helper took, and when
    sent: HashMap<String, i64>,
    on_way: HashSet<String>,
    failures: u32,
    backoff_until: i64,
}

struct Helper {
    allow_url: String,
    state: Mutex<HelperState>,
}

struct Inner {
    helpers: Vec<Helper>,
    secret: String,
    refresh_ms: i64,
    timeout: Duration,
    sink: Option<InProcessSink>,
    sent: AtomicU64,
    clock: Clock,
    client: Client<HttpConnector, Full<Bytes>>,
    in_flight: watch::Sender<usize>,
    throttle: Throttle,
}

/// Reports admitted players' addresses to the allowlist helpers, at most once per refresh interval each.
#[derive(Clone)]
pub struct AllowlistFeed(Arc<Inner>);

enum Outcome {
    Taken,
    Refused,
    Failed(String),
}

impl AllowlistFeed {
    /// A feed for `config`; `None` feeds nothing.
    pub fn new(config: Option<&Allowlist>) -> Self {
        Self::with_clock(config, Arc::new(crate::now_ms))
    }

    /// As [`AllowlistFeed::new`], with `clock` in Unix milliseconds.
    pub fn with_clock(config: Option<&Allowlist>, clock: Clock) -> Self {
        let mut connector = HttpConnector::new();
        connector.set_nodelay(true);
        let client = Client::builder(TokioExecutor::new())
            .pool_max_idle_per_host(4)
            .pool_idle_timeout(Duration::from_secs(2))
            .build(connector);
        let helpers = config
            .map(|c| {
                std::iter::once(&c.endpoint)
                    .chain(&c.additional)
                    .map(|e| Helper {
                        allow_url: format!("http://{}/allow", e.authority()),
                        state: Mutex::default(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self(Arc::new(Inner {
            helpers,
            secret: config.map(|c| c.secret.clone()).unwrap_or_default(),
            refresh_ms: config.map_or(60_000, |c| c.refresh.as_millis() as i64),
            timeout: config.map_or(Duration::from_secs(3), |c| c.timeout),
            sink: None,
            sent: AtomicU64::new(0),
            clock,
            client,
            in_flight: watch::channel(0).0,
            throttle: Throttle::new(60_000),
        }))
    }

    /// A feed with only an in-process helper.
    pub fn from_in_process(sink: InProcessSink) -> Self {
        Self::new(None).with_in_process(sink)
    }

    /// Reports go to `sink` and nothing goes over HTTP.
    pub fn with_in_process(self, sink: InProcessSink) -> Self {
        let inner = Arc::try_unwrap(self.0).unwrap_or_else(|shared| Inner {
            helpers: Vec::new(),
            secret: shared.secret.clone(),
            refresh_ms: shared.refresh_ms,
            timeout: shared.timeout,
            sink: None,
            sent: AtomicU64::new(0),
            clock: shared.clock.clone(),
            client: shared.client.clone(),
            in_flight: watch::channel(0).0,
            throttle: Throttle::new(60_000),
        });
        Self(Arc::new(Inner {
            sink: Some(sink),
            ..inner
        }))
    }

    /// Whether anything at all is fed.
    pub fn is_on(&self) -> bool {
        self.0.sink.is_some() || !self.0.helpers.is_empty()
    }

    /// Addresses sent (in-process ones included).
    pub fn sent(&self) -> u64 {
        self.0.sent.load(Ordering::Relaxed)
    }

    /// Tells every helper about `ip`, without waiting.
    pub fn report(&self, ip: &str) {
        if ip == "unknown" {
            return;
        }
        if let Some(sink) = &self.0.sink {
            _ = sink(ip);
            self.0.sent.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let now = (self.0.clock)();
        for index in 0..self.0.helpers.len() {
            let helper = &self.0.helpers[index];
            {
                let mut state = helper.state.lock().unwrap_or_else(PoisonError::into_inner);
                if now < state.backoff_until || state.on_way.contains(ip) {
                    continue;
                }
                if state.sent.get(ip).is_some_and(|&at| now - at < self.0.refresh_ms) {
                    continue;
                }
                if state.sent.len() > MAX_REMEMBERED {
                    let refresh = self.0.refresh_ms;
                    state.sent.retain(|_, at| now - *at < refresh);
                }
                state.on_way.insert(ip.to_owned());
            }
            self.0.in_flight.send_modify(|n| *n += 1);
            let feed = self.clone();
            let ip = ip.to_owned();
            tokio::spawn(async move {
                let outcome = feed.send(index, &ip).await;
                feed.settle(index, &ip, outcome);
                feed.0.in_flight.send_modify(|n| *n = n.saturating_sub(1));
            });
        }
    }

    async fn send(&self, index: usize, ip: &str) -> Outcome {
        let body = json!({ "ip": ip }).to_string();
        let request = http::Request::post(&self.0.helpers[index].allow_url)
            .header("content-type", "application/json")
            .header("content-length", body.len())
            .header("x-allowlist-secret", &self.0.secret)
            .body(Full::new(Bytes::from(body)));
        let Ok(request) = request else {
            return Outcome::Failed("bad request".to_owned());
        };
        let exchange = async {
            let answer = self.0.client.request(request).await.map_err(|e| e.to_string())?;
            let status = answer.status().as_u16();
            // The reply is only drained; a helper that streams without end is cut off, not buffered
            Limited::new(answer.into_body(), 64 << 10)
                .collect()
                .await
                .map_err(|e| e.to_string())?;
            Ok::<u16, String>(status)
        };
        match tokio::time::timeout(self.0.timeout, exchange).await {
            Err(_) => Outcome::Failed("timed out".to_owned()),
            Ok(Err(e)) => Outcome::Failed(e),
            Ok(Ok(200)) => Outcome::Taken,
            Ok(Ok(400)) => Outcome::Refused,
            Ok(Ok(status)) => Outcome::Failed(format!("status {status}")),
        }
    }

    fn settle(&self, index: usize, ip: &str, outcome: Outcome) {
        let now = (self.0.clock)();
        let mut state = self.0.helpers[index]
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        state.on_way.remove(ip);
        match outcome {
            Outcome::Taken | Outcome::Refused => {
                state.failures = 0;
                state.backoff_until = 0;
                state.sent.insert(ip.to_owned(), now);
                drop(state);
                if matches!(outcome, Outcome::Taken) {
                    self.0.sent.fetch_add(1, Ordering::Relaxed);
                    linelog::debug("allowlist refreshed", json!({ "ip": ip }));
                } else if self.0.throttle.allow(&format!("refused:{ip}"), now).is_some() {
                    linelog::warn(
                        "allowlist helper refused a player's address; that player's game ports stay closed",
                        json!({ "ip": ip }),
                    );
                }
            }
            Outcome::Failed(detail) => {
                state.failures += 1;
                if state.failures >= 3 {
                    let doubling = 1i64 << (state.failures - 3).min(10);
                    state.backoff_until = now + (FIRST_BACKOFF_MS * doubling).min(MAX_BACKOFF_MS);
                }
                drop(state);
                if let Some(suppressed) = self.0.throttle.allow("down", now) {
                    linelog::warn(
                        "allowlist helper did not take an address; players who log in now get no game ports until it answers",
                        json!({ "detail": detail, "suppressed": suppressed }),
                    );
                }
            }
        }
    }

    /// Waits until nothing is on its way to any helper.
    pub async fn idle(&self) {
        let mut rx = self.0.in_flight.subscribe();
        _ = rx.wait_for(|n| *n == 0).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn in_process_sink_takes_everything_but_unknown() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = seen.clone();
        let feed = AllowlistFeed::from_in_process(Arc::new(move |ip: &str| {
            record.lock().unwrap().push(ip.to_owned());
            true
        }));
        feed.report("unknown");
        feed.report("203.0.113.7");
        feed.idle().await;
        assert_eq!(*seen.lock().unwrap(), vec!["203.0.113.7".to_owned()]);
        assert_eq!(feed.sent(), 1);
        assert!(feed.is_on());
        assert!(!AllowlistFeed::new(None).is_on());
    }
}
