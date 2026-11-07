// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The Node services' JSON log lines (`{"t","level","msg",...}`), for ports whose log lines are a contract.

use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use chrono::Utc;
use serde_json::{Map, Value};

/// A log line's level, ordered by severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Level {
    /// Detail for debugging.
    Debug = 10,
    /// Normal operation.
    Info = 20,
    /// Something to look at.
    Warn = 30,
    /// A failure.
    Error = 40,
}

impl Level {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }

    const fn from_u8(val: u8) -> Option<Self> {
        match val {
            10 => Some(Self::Debug),
            20 => Some(Self::Info),
            30 => Some(Self::Warn),
            40 => Some(Self::Error),
            _ => None,
        }
    }
}

/// Takes each formatted line and its level instead of stdout and stderr.
pub type Sink = Box<dyn Fn(&str, Level) + Send + Sync>;

static LEVEL_OVERRIDE: AtomicU8 = AtomicU8::new(0);
static SINK: OnceLock<Mutex<Option<Arc<Sink>>>> = OnceLock::new();

fn sink_cell() -> &'static Mutex<Option<Arc<Sink>>> {
    SINK.get_or_init(|| Mutex::new(None))
}

fn env_level() -> Level {
    static ENV_LEVEL: OnceLock<Level> = OnceLock::new();
    *ENV_LEVEL.get_or_init(|| {
        match std::env::var("LOG_LEVEL")
            .ok()
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("debug") => Level::Debug,
            Some("warn") => Level::Warn,
            Some("error") => Level::Error,
            _ => Level::Info,
        }
    })
}

fn current_level() -> Level {
    Level::from_u8(LEVEL_OVERRIDE.load(Ordering::Relaxed)).unwrap_or_else(env_level)
}

/// Sends lines to `sink`, or back to stdout and stderr with `None`.
pub fn set_sink(sink: Option<Sink>) {
    let mut guard = sink_cell().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    *guard = sink.map(Arc::new);
}

/// Overrides the level from `LOG_LEVEL`.
pub fn set_level(level: Level) {
    LEVEL_OVERRIDE.store(level as u8, Ordering::Relaxed);
}

/// Writes `msg` with `fields` merged in, when `level` is at or above the current one.
pub fn log(level: Level, msg: &str, fields: Value) {
    if level < current_level() {
        return;
    }

    let mut map = Map::new();
    let ts = Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    map.insert("t".to_owned(), Value::String(ts));
    map.insert("level".to_owned(), Value::String(level.as_str().to_owned()));
    map.insert("msg".to_owned(), Value::String(msg.to_owned()));

    if let Value::Object(extra) = fields {
        for (k, v) in extra {
            map.insert(k, v);
        }
    }

    let text = crate::jsjson::stringify(&Value::Object(map));

    let custom_sink = sink_cell()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();

    if let Some(sink) = custom_sink {
        sink(&text, level);
    } else if crate::telemetry::tracing_bridge_active() {
        match level {
            Level::Debug => tracing::debug!("{msg} ({text})"),
            Level::Info => tracing::info!("{msg} ({text})"),
            Level::Warn => tracing::warn!("{msg} ({text})"),
            Level::Error => tracing::error!("{msg} ({text})"),
        }
    } else {
        match level {
            Level::Warn | Level::Error => {
                let mut err = std::io::stderr().lock();
                let _ = writeln!(err, "{text}");
            }
            Level::Debug | Level::Info => {
                let mut out = std::io::stdout().lock();
                let _ = writeln!(out, "{text}");
            }
        }
    }
}

/// [`log`] at [`Level::Info`].
pub fn info(msg: &str, fields: Value) {
    log(Level::Info, msg, fields);
}

/// [`log`] at [`Level::Warn`].
pub fn warn(msg: &str, fields: Value) {
    log(Level::Warn, msg, fields);
}

/// [`log`] at [`Level::Error`].
pub fn error(msg: &str, fields: Value) {
    log(Level::Error, msg, fields);
}

/// [`log`] at [`Level::Debug`].
pub fn debug(msg: &str, fields: Value) {
    log(Level::Debug, msg, fields);
}

const MAX_THROTTLE_KEYS: usize = 10_000;

struct ThrottleEntry {
    last_ms: i64,
    suppressed: u64,
}

/// Lets one line per key through per interval and counts the rest.
pub struct Throttle {
    interval_ms: i64,
    entries: Mutex<HashMap<String, ThrottleEntry>>,
}

impl Throttle {
    /// A throttle letting one line per key through every `interval_ms`.
    pub fn new(interval_ms: i64) -> Self {
        Self {
            interval_ms,
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// `Some(suppressed since the last one)` when a line for `key` may go out at `now` (ms), `None` to drop it.
    pub fn allow(&self, key: &str, now: i64) -> Option<u64> {
        let mut map = self.entries.lock().unwrap_or_else(std::sync::PoisonError::into_inner);

        if let Some(entry) = map.get_mut(key) {
            if now.saturating_sub(entry.last_ms) < self.interval_ms {
                entry.suppressed += 1;
                return None;
            }
            let suppressed = entry.suppressed;
            entry.last_ms = now;
            entry.suppressed = 0;
            return Some(suppressed);
        }

        if map.len() >= MAX_THROTTLE_KEYS {
            let interval = self.interval_ms;
            map.retain(|_, v| now.saturating_sub(v.last_ms) < interval);
            if map.len() >= MAX_THROTTLE_KEYS
                && let Some(oldest_key) = map.iter().min_by_key(|(_, v)| v.last_ms).map(|(k, _)| k.clone())
            {
                map.remove(&oldest_key);
            }
        }

        map.insert(
            key.to_owned(),
            ThrottleEntry {
                last_ms: now,
                suppressed: 0,
            },
        );
        Some(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn emits_structured_json_to_sink_and_respects_level() {
        let captured: Arc<Mutex<Vec<(String, Level)>>> = Arc::new(Mutex::new(Vec::new()));
        let cap_clone = captured.clone();
        set_level(Level::Info);
        set_sink(Some(Box::new(move |line, lvl| {
            cap_clone.lock().unwrap().push((line.to_owned(), lvl));
        })));

        debug("ignored debug", json!({"a": 1}));
        info("started", json!({"port": 8080}));

        set_sink(None);

        let logs = captured.lock().unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].1, Level::Info);
        let parsed: Value = serde_json::from_str(&logs[0].0).unwrap();
        assert_eq!(parsed["level"], "info");
        assert_eq!(parsed["msg"], "started");
        assert_eq!(parsed["port"], 8080);
        assert!(parsed["t"].as_str().unwrap().ends_with('Z'));
    }

    #[test]
    fn throttle_suppresses_within_interval_and_reports_count() {
        let throttle = Throttle::new(1000);
        assert_eq!(throttle.allow("k1", 100), Some(0));
        assert_eq!(throttle.allow("k1", 500), None);
        assert_eq!(throttle.allow("k1", 999), None);
        assert_eq!(throttle.allow("k1", 1100), Some(2));
        assert_eq!(throttle.allow("k1", 2100), Some(0));
    }
}
