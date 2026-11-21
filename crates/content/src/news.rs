// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The news the launcher shows: a host's own file (`CONTENT_NEWS_FILE`), or a file on GitHub (`CONTENT_NEWS_URL`)
//! fetched in the background so a reply never waits on GitHub. Either keeps its last good items when a newer copy
//! is broken.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, UNIX_EPOCH};

use futures_util::future::BoxFuture;
use serde_json::{Value, json};

use crate::client::{BodyError, Reply};
use crate::input::{NewsItem, header_text, parse_news};
use crate::{Clock, log};

pub const FILE_RECHECK_MS: i64 = 5_000;
pub const MAX_FILE_BYTES: u64 = 1 << 20;
pub const REFETCH_MS: i64 = 60_000;
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(5);
pub const MAX_REMOTE_BYTES: usize = 1 << 20;

/// GETs a URL, with the ETag to send as `If-None-Match` when one is stored. Tests hand in their own.
pub type Fetch = Arc<dyn Fn(String, Option<String>) -> BoxFuture<'static, Result<Reply, String>> + Send + Sync>;

/// What the host's file last gave.
#[derive(Default)]
struct FileSide {
    looked_at: Option<i64>,
    /// Length and modification time (ns) at the last look, so an untouched file is not parsed again
    stamp: Option<(u64, Option<u128>)>,
    gone: bool,
    items: Vec<NewsItem>,
}

/// What GitHub last gave, and whether a fetch is on its way.
#[derive(Default)]
struct RemoteSide {
    in_flight: bool,
    tried_at: Option<i64>,
    etag: Option<String>,
    /// `Some` once a fetch succeeded, even with no items: it then wins over the file
    items: Option<Vec<NewsItem>>,
}

pub struct News {
    file: Option<PathBuf>,
    url: Option<String>,
    clock: Clock,
    fetch: Fetch,
    local: Mutex<FileSide>,
    remote: Mutex<RemoteSide>,
}

fn held<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn github_get(url: String, etag: Option<String>) -> BoxFuture<'static, Result<Reply, String>> {
    Box::pin(async move {
        let mut headers: Vec<(&str, &str)> = Vec::new();
        if let Some(tag) = etag.as_deref() {
            headers.push(("if-none-match", tag));
        }
        crate::client::get(&url, &headers, FETCH_TIMEOUT, MAX_REMOTE_BYTES).await
    })
}

impl News {
    pub fn new(file: Option<PathBuf>, url: Option<String>, clock: Clock) -> Self {
        Self::with_fetch(file, url, clock, Arc::new(github_get))
    }

    pub fn with_fetch(file: Option<PathBuf>, url: Option<String>, clock: Clock, fetch: Fetch) -> Self {
        News {
            file,
            url,
            clock,
            fetch,
            local: Mutex::new(FileSide::default()),
            remote: Mutex::new(RemoteSide::default()),
        }
    }

    pub fn file(&self) -> Option<&Path> {
        self.file.as_deref()
    }

    pub fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }

    /// `{"items":[...]}`: GitHub's items once a fetch succeeded, else the file's. Looks at the file first.
    pub fn body(&self) -> String {
        self.look_at_file();
        let remote = held(&self.remote).items.clone();
        let items = remote.unwrap_or_else(|| held(&self.local).items.clone());
        let listed: Vec<Value> = items.iter().map(NewsItem::to_json).collect();
        service_kit::jsjson::stringify(&json!({ "items": listed }))
    }

    fn look_at_file(&self) {
        let Some(path) = self.file.as_deref() else {
            return;
        };
        let now = (self.clock)();
        let mut side = held(&self.local);
        if side.looked_at.is_some_and(|at| now - at < FILE_RECHECK_MS) {
            return;
        }
        side.looked_at = Some(now);
        let shown = path.display().to_string();
        let meta = match fs::metadata(path) {
            Ok(meta) => meta,
            Err(_) => {
                if !side.gone {
                    log::warn("news file not found; serving no news", json!({ "file": shown }));
                    side.gone = true;
                    side.items.clear();
                    side.stamp = None;
                }
                return;
            }
        };
        side.gone = false;
        let modified = meta
            .modified()
            .ok()
            .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
            .map(|since| since.as_nanos());
        let stamp = (meta.len(), modified);
        if side.stamp == Some(stamp) {
            return;
        }
        side.stamp = Some(stamp);
        if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
            log::warn(
                "news file skipped: not a regular file or over 1 MB",
                json!({ "file": shown }),
            );
            return;
        }
        match read_document(path) {
            Ok(items) => {
                log::info("news loaded", json!({ "file": shown, "items": items.len() }));
                side.items = items;
            }
            Err(error) => log::warn(
                "news file could not be read; keeping the last good version",
                json!({ "file": shown, "error": error }),
            ),
        }
    }

    /// Starts one GitHub refresh in the background unless one is running or the last began under a minute ago.
    pub fn poll_remote(self: &Arc<Self>) {
        let Some(url) = self.url.clone() else {
            return;
        };
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let now = (self.clock)();
        let etag = {
            let mut side = held(&self.remote);
            if side.in_flight || side.tried_at.is_some_and(|at| now - at < REFETCH_MS) {
                return;
            }
            side.in_flight = true;
            side.tried_at = Some(now);
            side.etag.clone()
        };
        let news = Arc::clone(self);
        runtime.spawn(async move {
            // Cleared on drop, so a panicking fetch cannot stop every later refresh
            let _done = InFlight(&news.remote);
            let outcome = match (news.fetch)(url, etag).await {
                Ok(reply) => judge(reply),
                Err(error) => Err(error),
            };
            match outcome {
                Ok(Fetched::Unchanged) => {}
                Ok(Fetched::Fresh { items, etag }) => {
                    let mut side = held(&news.remote);
                    side.items = Some(items);
                    side.etag = etag;
                }
                Err(error) => log::warn(
                    "GitHub news unavailable; keeping last good news",
                    json!({ "error": error }),
                ),
            }
        });
    }

    pub fn refreshing(&self) -> bool {
        held(&self.remote).in_flight
    }
}

struct InFlight<'a>(&'a Mutex<RemoteSide>);

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        held(self.0).in_flight = false;
    }
}

enum Fetched {
    Unchanged,
    Fresh { items: Vec<NewsItem>, etag: Option<String> },
}

fn judge(reply: Reply) -> Result<Fetched, String> {
    if reply.status == 304 {
        return Ok(Fetched::Unchanged);
    }
    if !(200..300).contains(&reply.status) {
        return Err(format!("GitHub news HTTP {}", reply.status));
    }
    let bytes = match reply.body {
        Ok(bytes) => bytes,
        Err(BodyError::TooLarge) => return Err("GitHub news exceeds 1 MB".to_owned()),
        Err(BodyError::Failed(error)) => return Err(error),
    };
    let doc = document(&bytes)?;
    let items = parse_news(&doc)?;
    let listed = match &doc {
        Value::Array(entries) => entries.len(),
        _ => doc.get("items").and_then(Value::as_array).map_or(0, Vec::len),
    };
    if items.is_empty() && listed > 0 {
        return Err("GitHub news contains no usable entries".to_owned());
    }
    Ok(Fetched::Fresh {
        items,
        etag: header_text(&reply.headers, "etag"),
    })
}

/// UTF-8 (lossy), one leading BOM dropped, as JSON.
fn document(bytes: &[u8]) -> Result<Value, String> {
    let text = String::from_utf8_lossy(bytes);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    serde_json::from_str(text).map_err(|e| e.to_string())
}

fn read_document(path: &Path) -> Result<Vec<NewsItem>, String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    parse_news(&document(&bytes)?)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};

    use bytes::Bytes;

    fn fake_clock() -> (Arc<AtomicI64>, Clock) {
        let now = Arc::new(AtomicI64::new(1_000_000));
        let read = Arc::clone(&now);
        (now, Arc::new(move || read.load(Ordering::SeqCst)))
    }

    fn titles(body: &str) -> Vec<String> {
        let doc: Value = serde_json::from_str(body).unwrap();
        doc["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["title"].as_str().unwrap().to_owned())
            .collect()
    }

    fn doc_with(titles: &[(&str, &str)]) -> String {
        let items: Vec<Value> = titles.iter().map(|(d, t)| json!({"date": d, "title": t})).collect();
        json!({ "items": items }).to_string()
    }

    #[test]
    fn the_host_file_is_watched_and_its_last_good_copy_kept() {
        let (now, clock) = fake_clock();
        let none = News::new(None, None, Arc::clone(&clock));
        assert_eq!(none.body(), r#"{"items":[]}"#);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("news.json");
        let news = News::new(Some(path.clone()), None, clock);
        assert_eq!(news.body(), r#"{"items":[]}"#);

        fs::write(&path, doc_with(&[("2026-01-01", "Old"), ("2026-02-01", "New")])).unwrap();
        assert!(titles(&news.body()).is_empty(), "looked again before 5 s");
        now.fetch_add(FILE_RECHECK_MS, Ordering::SeqCst);
        assert_eq!(titles(&news.body()), ["New", "Old"]);

        fs::write(&path, "{ broken").unwrap();
        now.fetch_add(FILE_RECHECK_MS, Ordering::SeqCst);
        assert_eq!(titles(&news.body()), ["New", "Old"]);

        let array = json!([{"date": "2026-03-01", "title": "Listed"}]).to_string();
        fs::write(&path, array).unwrap();
        now.fetch_add(FILE_RECHECK_MS, Ordering::SeqCst);
        assert_eq!(titles(&news.body()), ["Listed"]);

        fs::remove_file(&path).unwrap();
        now.fetch_add(FILE_RECHECK_MS, Ordering::SeqCst);
        assert!(titles(&news.body()).is_empty());
    }

    type Script = Arc<Mutex<Vec<Result<Reply, String>>>>;

    /// A fetch answering from `script` in order, counting calls and keeping the ETags it was sent.
    fn scripted(script: Script, calls: Arc<AtomicUsize>, sent: Arc<Mutex<Vec<Option<String>>>>) -> Fetch {
        Arc::new(move |_url, etag| {
            calls.fetch_add(1, Ordering::SeqCst);
            held(&sent).push(etag);
            let next = held(&script).remove(0);
            Box::pin(async move {
                tokio::task::yield_now().await;
                next
            })
        })
    }

    fn answer(status: u16, body: impl Into<Bytes>, etag: Option<&str>) -> Result<Reply, String> {
        let mut headers = http::HeaderMap::new();
        if let Some(tag) = etag {
            headers.insert("etag", http::HeaderValue::from_str(tag).unwrap());
        }
        Ok(Reply {
            status,
            headers,
            body: Ok(body.into()),
        })
    }

    async fn finished(news: &News) {
        for _ in 0..1000 {
            if !news.refreshing() {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("the refresh did not finish");
    }

    #[tokio::test]
    async fn github_is_asked_once_a_minute_and_keeps_its_last_good_news() {
        let script: Script = Arc::new(Mutex::new(vec![
            answer(
                200,
                doc_with(&[("2026-01-01", "B"), ("2026-05-01", "A")]),
                Some("\"v1\""),
            ),
            answer(304, "", None),
            answer(503, "", None),
            answer(200, "not json", None),
            answer(200, r#"{"items":[{}]}"#, None),
            Ok(Reply {
                status: 200,
                headers: http::HeaderMap::new(),
                body: Err(BodyError::TooLarge),
            }),
            Err("fetch failed: refused".to_owned()),
            answer(200, r#"{"items":[]}"#, None),
        ]));
        let calls = Arc::new(AtomicUsize::new(0));
        let sent = Arc::new(Mutex::new(Vec::new()));
        let (now, clock) = fake_clock();
        let url = "https://raw.githubusercontent.com/o/r/main/news.json".to_owned();
        let fetch = scripted(Arc::clone(&script), Arc::clone(&calls), Arc::clone(&sent));
        let news = Arc::new(News::with_fetch(None, Some(url), clock, fetch));

        news.poll_remote();
        finished(&news).await;
        assert_eq!(titles(&news.body()), ["A", "B"]);
        news.poll_remote();
        finished(&news).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1, "a second poll within the minute");

        for _ in 0..6 {
            now.fetch_add(REFETCH_MS, Ordering::SeqCst);
            news.poll_remote();
            finished(&news).await;
            assert_eq!(titles(&news.body()), ["A", "B"]);
        }

        now.fetch_add(REFETCH_MS, Ordering::SeqCst);
        news.poll_remote();
        finished(&news).await;
        assert_eq!(news.body(), r#"{"items":[]}"#);
        assert_eq!(calls.load(Ordering::SeqCst), 8);
        let sent = held(&sent);
        assert_eq!(sent[0], None);
        assert!(sent[1..].iter().all(|tag| tag.as_deref() == Some("\"v1\"")), "{sent:?}");
    }

    #[tokio::test]
    async fn concurrent_polls_start_one_fetch() {
        let script: Script = Arc::new(Mutex::new(vec![answer(200, "[]", None)]));
        let calls = Arc::new(AtomicUsize::new(0));
        let fetch = scripted(script, Arc::clone(&calls), Arc::new(Mutex::new(Vec::new())));
        let (_, clock) = fake_clock();
        let url = Some("https://raw.githubusercontent.com/o/r/main/n.json".to_owned());
        let news = Arc::new(News::with_fetch(None, url, clock, fetch));
        for _ in 0..5 {
            news.poll_remote();
        }
        finished(&news).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
