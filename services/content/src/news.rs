// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The server's news: a file the host edits (`CONTENT_NEWS_FILE`), and a raw GitHub file
//! (`CONTENT_NEWS_URL`) fetched in the background at most once a minute. A bad edit or a failed fetch keeps
//! the last good news.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, UNIX_EPOCH};

use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use crate::client::{self, BodyError, Reply};
use crate::input::{NewsItem, parse_news};
use crate::{Clock, log};

pub const FILE_RECHECK_MS: i64 = 5_000;
pub const MAX_FILE_BYTES: u64 = 1 << 20;
pub const REFETCH_MS: i64 = 60_000;
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(5);
pub const MAX_REMOTE_BYTES: usize = 1 << 20;

/// One GET of the news URL, with the stored ETag when there is one.
pub type Fetch = Arc<dyn Fn(String, Option<String>) -> BoxFuture<'static, Result<Reply, String>> + Send + Sync>;

#[derive(Default)]
struct State {
    file_items: Vec<NewsItem>,
    file_checked_at: Option<i64>,
    file_seen: Option<(u64, u128)>,
    file_missing: bool,
    /// Set once a fetch succeeded, an empty list included
    remote_items: Option<Vec<NewsItem>>,
    etag: Option<String>,
    refreshing: bool,
    last_attempt: Option<i64>,
}

pub struct News {
    file: Option<PathBuf>,
    url: Option<String>,
    clock: Clock,
    fetch: Fetch,
    state: Mutex<State>,
}

impl News {
    pub fn new(file: Option<PathBuf>, url: Option<String>, clock: Clock) -> Self {
        let fetch: Fetch = Arc::new(|url, etag| {
            Box::pin(async move {
                let mut headers = Vec::new();
                if let Some(etag) = &etag {
                    headers.push(("if-none-match", etag.as_str()));
                }
                client::get(&url, &headers, FETCH_TIMEOUT, MAX_REMOTE_BYTES).await
            })
        });
        Self::with_fetch(file, url, clock, fetch)
    }

    pub fn with_fetch(file: Option<PathBuf>, url: Option<String>, clock: Clock, fetch: Fetch) -> Self {
        Self {
            file,
            url,
            clock,
            fetch,
            state: Mutex::default(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn file(&self) -> Option<&Path> {
        self.file.as_deref()
    }

    pub fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }

    /// `{"items":[...]}`: the remote news once a fetch succeeded, else the file's. Blocking: it may read the file.
    pub fn body(&self) -> String {
        self.check_file();
        let state = self.lock();
        let items = state.remote_items.as_ref().unwrap_or(&state.file_items);
        let items: Vec<Value> = items.iter().map(NewsItem::to_json).collect();
        service_kit::jsjson::stringify(&json!({"items": items}))
    }

    fn check_file(&self) {
        let Some(file) = &self.file else {
            return;
        };
        let now = (self.clock)();
        let mut state = self.lock();
        if state.file_checked_at.is_some_and(|at| now - at < FILE_RECHECK_MS) {
            return;
        }
        state.file_checked_at = Some(now);
        let shown = file.display().to_string();
        let meta = match fs::metadata(file) {
            Ok(meta) => meta,
            Err(_) => {
                if !state.file_missing {
                    log::warn("news file not found; serving no news", json!({"file": shown}));
                }
                state.file_items.clear();
                state.file_missing = true;
                state.file_seen = None;
                return;
            }
        };
        state.file_missing = false;
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos());
        let seen = (meta.len(), modified);
        if state.file_seen == Some(seen) {
            return;
        }
        state.file_seen = Some(seen);
        if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
            log::warn(
                "news file skipped: not a regular file or over 1 MB",
                json!({"file": shown}),
            );
            return;
        }
        let parsed = fs::read(file)
            .map_err(|e| e.to_string())
            .and_then(|bytes| String::from_utf8(bytes).map_err(|e| e.to_string()))
            .and_then(|text| {
                let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
                serde_json::from_str::<Value>(text).map_err(|e| e.to_string())
            })
            .and_then(|doc| parse_news(&doc));
        match parsed {
            Ok(items) => {
                log::info("news loaded", json!({"file": shown, "items": items.len()}));
                state.file_items = items;
            }
            Err(error) => log::warn(
                "news file could not be read; keeping the last good version",
                json!({"file": shown, "error": error}),
            ),
        }
    }

    /// Starts one background fetch of the GitHub news, unless one is running or the last began under a minute
    /// ago. Never waits for it.
    pub fn poll_remote(self: &Arc<Self>) {
        let Some(url) = self.url.clone() else {
            return;
        };
        let etag = {
            let now = (self.clock)();
            let mut state = self.lock();
            if state.refreshing || state.last_attempt.is_some_and(|at| now - at < REFETCH_MS) {
                return;
            }
            state.refreshing = true;
            state.last_attempt = Some(now);
            state.etag.clone()
        };
        let news = self.clone();
        tokio::spawn(async move {
            let result = (news.fetch)(url, etag).await.and_then(read_remote);
            let mut state = news.lock();
            state.refreshing = false;
            match result {
                Ok(None) => {}
                Ok(Some((items, etag))) => {
                    state.remote_items = Some(items);
                    state.etag = etag;
                }
                Err(error) => log::warn(
                    "GitHub news unavailable; keeping last good news",
                    json!({"error": error}),
                ),
            }
        });
    }

    /// Whether a fetch is running, for tests.
    #[doc(hidden)]
    pub fn refreshing(&self) -> bool {
        self.lock().refreshing
    }
}

/// New remote news and the ETag it came with.
type Fresh = (Vec<NewsItem>, Option<String>);

/// A fetch's reply: `None` for a 304.
fn read_remote(reply: Reply) -> Result<Option<Fresh>, String> {
    if reply.status == 304 {
        return Ok(None);
    }
    if !(200..300).contains(&reply.status) {
        return Err(format!("GitHub news HTTP {}", reply.status));
    }
    let body = match reply.body {
        Ok(body) => body,
        Err(BodyError::TooLarge) => return Err("GitHub news exceeds 1 MB".to_owned()),
        Err(BodyError::Failed(error)) => return Err(error),
    };
    let text = String::from_utf8_lossy(&body);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let doc: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let items = parse_news(&doc)?;
    let listed = match &doc {
        Value::Array(entries) => entries.len(),
        _ => doc.get("items").and_then(Value::as_array).map_or(0, Vec::len),
    };
    if listed > 0 && items.is_empty() {
        return Err("GitHub news contains no usable entries".to_owned());
    }
    let etag = reply
        .headers
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    Ok(Some((items, etag)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

    use bytes::Bytes;

    use super::*;

    fn clock() -> (Clock, Arc<AtomicI64>) {
        let now = Arc::new(AtomicI64::new(1_000_000));
        let read = now.clone();
        (Arc::new(move || read.load(Ordering::SeqCst)), now)
    }

    #[test]
    fn the_file_none_missing_changed_broken_and_array() {
        let (clock, now) = clock();
        assert_eq!(News::new(None, None, clock.clone()).body(), r#"{"items":[]}"#);
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("news.json");
        let news = News::new(Some(path.clone()), None, clock);
        assert_eq!(news.body(), r#"{"items":[]}"#);
        fs::write(&path, r#"{"items":[{"date":"2026-01-01","title":"One","body":"b"}]}"#).unwrap();
        // Within 5 s of the last look nothing is read
        assert_eq!(news.body(), r#"{"items":[]}"#);
        now.fetch_add(5_000, Ordering::SeqCst);
        assert_eq!(
            news.body(),
            r#"{"items":[{"date":"2026-01-01T00:00:00.000Z","title":"One","body":"b"}]}"#
        );
        fs::write(&path, r#"{"items":[{"date":"2026-01-01","title":"One","body":"b"}"#).unwrap();
        now.fetch_add(5_000, Ordering::SeqCst);
        assert!(
            news.body().contains("\"One\""),
            "a broken edit keeps the last good version"
        );
        fs::write(
            &path,
            r#"[{"date":"2026-02-01","title":"Two"},{"date":"2026-01-01","title":"Old!"}]"#,
        )
        .unwrap();
        now.fetch_add(5_000, Ordering::SeqCst);
        assert_eq!(
            news.body(),
            r#"{"items":[{"date":"2026-02-01T00:00:00.000Z","title":"Two","body":""},{"date":"2026-01-01T00:00:00.000Z","title":"Old!","body":""}]}"#
        );
        fs::remove_file(&path).unwrap();
        now.fetch_add(5_000, Ordering::SeqCst);
        assert_eq!(news.body(), r#"{"items":[]}"#);
    }

    fn reply(status: u16, body: Result<&'static str, BodyError>, etag: Option<&str>) -> Reply {
        let mut headers = http::HeaderMap::new();
        if let Some(etag) = etag {
            headers.insert("etag", etag.parse().unwrap());
        }
        Reply {
            status,
            headers,
            body: body.map(|b| Bytes::from_static(b.as_bytes())),
        }
    }

    type Script = Arc<Mutex<Vec<Result<Reply, String>>>>;

    type Sent = Arc<Mutex<Vec<Option<String>>>>;

    fn remote(script: Vec<Result<Reply, String>>) -> (Arc<News>, Arc<AtomicI64>, Sent) {
        let (clock, now) = clock();
        let script: Script = Arc::new(Mutex::new(script.into_iter().rev().collect()));
        let sent = Arc::new(Mutex::new(Vec::new()));
        let seen = sent.clone();
        let fetch: Fetch = Arc::new(move |_, etag| {
            seen.lock().unwrap().push(etag);
            let next = script.lock().unwrap().pop().unwrap_or(Err("no more".into()));
            Box::pin(async move {
                tokio::time::sleep(Duration::from_millis(20)).await;
                next
            })
        });
        let url = Some("https://raw.githubusercontent.com/o/r/main/news.json".to_owned());
        (Arc::new(News::with_fetch(None, url, clock, fetch)), now, sent)
    }

    async fn settle(news: &News) {
        tokio::time::sleep(Duration::from_millis(5)).await;
        while news.refreshing() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    #[tokio::test]
    async fn github_once_a_minute_keeping_last_good() {
        let good = r#"{"items":[{"date":"2026-01-01","title":"Old"},{"date":"2026-03-01","title":"New"}]}"#;
        let (news, now, sent) = remote(vec![
            Ok(reply(200, Ok(good), Some("\"v1\""))),
            Ok(reply(304, Ok(""), None)),
            Ok(reply(503, Ok(""), None)),
            Ok(reply(200, Ok("nope"), None)),
            Ok(reply(200, Ok(r#"{"items":[{}]}"#), None)),
            Ok(reply(200, Err(BodyError::TooLarge), None)),
            Err("fetch failed: x".into()),
            Ok(reply(200, Ok(r#"{"items":[]}"#), None)),
        ]);
        news.poll_remote();
        settle(&news).await;
        let body = news.body();
        assert!(body.find("New").unwrap() < body.find("Old").unwrap(), "{body}");
        // One fetch per minute
        news.poll_remote();
        settle(&news).await;
        assert_eq!(sent.lock().unwrap().len(), 1);
        for _ in 0..6 {
            now.fetch_add(60_000, Ordering::SeqCst);
            news.poll_remote();
            settle(&news).await;
            assert!(news.body().contains("New"));
        }
        // The 304 sent the stored ETag
        assert_eq!(sent.lock().unwrap()[1].as_deref(), Some("\"v1\""));
        now.fetch_add(60_000, Ordering::SeqCst);
        news.poll_remote();
        settle(&news).await;
        assert_eq!(news.body(), r#"{"items":[]}"#);
    }

    #[tokio::test]
    async fn concurrent_polls_share_one_fetch() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = calls.clone();
        let fetch: Fetch = Arc::new(move |_, _| {
            counted.fetch_add(1, Ordering::SeqCst);
            Box::pin(async {
                tokio::time::sleep(Duration::from_millis(50)).await;
                Err("x".to_owned())
            })
        });
        let (clock, _) = clock();
        let news = Arc::new(News::with_fetch(None, Some("https://x".into()), clock, fetch));
        for _ in 0..5 {
            news.poll_remote();
        }
        settle(&news).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
