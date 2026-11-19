// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The launcher's log file, with every line redacted (spec §2.7.4).
//!
//! Lines go to `<logs>/launcher.log` once [`set_folder`] is called, and to stderr before that; each launcher session
//! starts a fresh one ([`start_session`]). Nothing here
//! returns an error: logging never fails the launcher.

use std::fmt::Write as _;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, MutexGuard, PoisonError};

use chrono::{DateTime, SecondsFormat, Utc};
use regex::Regex;
use tracing::field::{Field, Visit};
use tracing::{Event, Metadata, Subscriber};
use tracing_log::NormalizeEvent as _;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt as _};

/// The log file's name in the log folder.
pub const LOG_FILE: &str = "launcher.log";
/// What [`LOG_FILE`] is renamed to once it grows past [`MAX_LOG_BYTES`].
pub const OLD_LOG_FILE: &str = "launcher.old.log";
/// The size past which the log is rotated before the next write.
pub const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;
/// What a redacted value is replaced with.
pub const HIDDEN: &str = "<hidden>";

const KEY_PREFIX: &str = "DLK_";
// The prefix before the 2026-11-06 rename: a migrated key is still hidden
const LEGACY_KEY_PREFIX: &str = "RRK_";
const MIN_SECRET_UNITS: usize = 6;
const MIN_KEY_REST_UNITS: usize = 16;

/// A line's severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Normal progress.
    Info,
    /// Something failed but the launcher carries on.
    Warn,
    /// Something failed.
    Error,
}

impl Level {
    /// The word written in the line.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
        }
    }
}

// JavaScript's `\s`: Rust's `\s` adds U+0085 and lacks U+FEFF, and the masks must stop where the old ones did.
const JS_SPACE: &str =
    r"\t\n\x0B\x0C\r \x{A0}\x{1680}\x{2000}-\x{200A}\x{2028}\x{2029}\x{202F}\x{205F}\x{3000}\x{FEFF}";

/// An ASCII-only case-insensitive literal: JavaScript's `/i` without `u` doesn't fold `K` (U+212A) to `k`.
fn ascii_ci(literal: &str) -> String {
    literal
        .chars()
        .map(|c| {
            if c.is_ascii_alphabetic() {
                format!("[{}{}]", c.to_ascii_lowercase(), c.to_ascii_uppercase())
            } else {
                regex::escape(&c.to_string())
            }
        })
        .collect()
}

struct Patterns {
    key: Regex,
    auth_password: Regex,
    api_key_header: Regex,
}

static PATTERNS: LazyLock<Option<Patterns>> = LazyLock::new(|| {
    let auth = format!(r#"({})[^{JS_SPACE}"']*"#, ascii_ci("-AUTH_PASSWORD="));
    let header = format!(
        r#"({}["']?[{JS_SPACE}]*[:=][{JS_SPACE}]*["']?)[^{JS_SPACE}"',}}]+"#,
        ascii_ci("x-dainty-user-api-key")
    );
    Some(Patterns {
        key: Regex::new(r"(?:DLK|RRK)_[A-Za-z0-9_-]+").ok()?,
        auth_password: Regex::new(&auth).ok()?,
        api_key_header: Regex::new(&header).ok()?,
    })
});

fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// The secrets the log hides, and the masks it applies.
#[derive(Debug, Clone, Default)]
pub struct Redactor {
    // Registration order: a key is replaced whole before its `DLK_`-less rest is.
    secrets: Vec<String>,
}

impl Redactor {
    /// An empty redactor that still applies the masks.
    #[must_use]
    pub const fn new() -> Self {
        Self { secrets: Vec::new() }
    }

    fn remember(&mut self, value: &str) {
        if !self.secrets.iter().any(|s| s == value) {
            self.secrets.push(value.to_owned());
        }
    }

    /// Remembers `value` when it has at least 6 UTF-16 units, and an account key's part after `DLK_` (or `DLK_`) when that
    /// has at least 16 units and no line breaks.
    pub fn register(&mut self, value: &str) {
        if utf16_len(value) < MIN_SECRET_UNITS {
            return;
        }
        self.remember(value);
        if let Some(rest) = value
            .strip_prefix(KEY_PREFIX)
            .or_else(|| value.strip_prefix(LEGACY_KEY_PREFIX))
            && utf16_len(rest) >= MIN_KEY_REST_UNITS
            && !rest.contains(['\r', '\n'])
        {
            self.remember(rest);
        }
    }

    /// Forgets `value` and its `DLK_`-less rest.
    pub fn unregister(&mut self, value: &str) {
        let rest = value
            .strip_prefix(KEY_PREFIX)
            .or_else(|| value.strip_prefix(LEGACY_KEY_PREFIX));
        self.secrets.retain(|s| s != value && Some(s.as_str()) != rest);
    }

    /// Whether `value` is remembered.
    #[must_use]
    pub fn is_registered(&self, value: &str) -> bool {
        self.secrets.iter().any(|s| s == value)
    }

    /// `text` with every remembered secret, account key, `-AUTH_PASSWORD=` value and user API key header hidden.
    #[must_use]
    pub fn redact(&self, text: &str) -> String {
        let mut out = text.to_owned();
        for secret in &self.secrets {
            if out.contains(secret.as_str()) {
                out = out.replace(secret.as_str(), HIDDEN);
            }
        }
        // A pattern that failed to build hides the whole line rather than let a key through.
        let Some(p) = PATTERNS.as_ref() else {
            return HIDDEN.to_owned();
        };
        let out = p.key.replace_all(&out, "DLK_<hidden>");
        let out = p.auth_password.replace_all(&out, "${1}<hidden>");
        p.api_key_header.replace_all(&out, "${1}<hidden>").into_owned()
    }
}

#[derive(Default)]
struct State {
    redactor: Redactor,
    folder: Option<PathBuf>,
}

static STATE: Mutex<State> = Mutex::new(State {
    redactor: Redactor::new(),
    folder: None,
});

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Hides `value` in every later line (see [`Redactor::register`]).
pub fn register_secret(value: &str) {
    state().redactor.register(value);
}

/// Stops hiding `value` (see [`Redactor::unregister`]).
pub fn unregister_secret(value: &str) {
    state().redactor.unregister(value);
}

/// `text` as the log would write it, for text shown elsewhere such as the game's console.
#[must_use]
pub fn redact(text: &str) -> String {
    state().redactor.redact(text)
}

/// An error's one-line description: its text, redacted.
#[must_use]
pub fn describe_error(error: &dyn std::error::Error) -> String {
    redact(&error.to_string())
}

/// Sends later lines to `<dir>/launcher.log`.
pub fn set_folder(dir: &Path) {
    state().folder = Some(dir.to_path_buf());
}

/// Starts this launcher session's log in `dir`: the last session's files are kept as `launcher-<time>-*.log`,
/// the oldest beyond [`KEPT_SESSIONS`] removed, then later lines go to a fresh `launcher.log`.
pub fn start_session(dir: &Path) {
    if let Err(failed) = rotate_sessions(dir, KEPT_SESSIONS) {
        let _ = writeln!(std::io::stderr(), "could not rotate the launcher logs: {failed}");
    }
    set_folder(dir);
}

/// Earlier sessions whose logs are kept.
pub const KEPT_SESSIONS: usize = 10;
const SESSION_PREFIX: &str = "launcher-";

/// Archives the current log (and its overflow) under the time it was last written, then prunes to `keep`
/// sessions. Archived names sort in time order.
pub fn rotate_sessions(dir: &Path, keep: usize) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let current = dir.join(LOG_FILE);
    if let Ok(meta) = std::fs::metadata(&current) {
        let time: DateTime<Utc> = meta.modified().map_or_else(|_| Utc::now(), DateTime::from);
        let stamp = time.format("%Y%m%dT%H%M%S%3fZ").to_string();
        let mut stamp_free = stamp.clone();
        for n in 1..100 {
            if !dir.join(format!("{SESSION_PREFIX}{stamp_free}-2.log")).exists() {
                break;
            }
            stamp_free = format!("{stamp}{n:02}");
        }
        let old = dir.join(OLD_LOG_FILE);
        if old.is_file() {
            std::fs::rename(&old, dir.join(format!("{SESSION_PREFIX}{stamp_free}-1.log")))?;
        }
        std::fs::rename(&current, dir.join(format!("{SESSION_PREFIX}{stamp_free}-2.log")))?;
    }
    let mut sessions: Vec<(String, PathBuf)> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let stamp = name
                .strip_prefix(SESSION_PREFIX)?
                .strip_suffix(".log")?
                .rsplit_once('-')?
                .0;
            Some((stamp.to_owned(), entry.path()))
        })
        .collect();
    sessions.sort();
    let mut stamps: Vec<&str> = sessions.iter().map(|(s, _)| s.as_str()).collect();
    stamps.dedup();
    if stamps.len() > keep {
        let cut = stamps[stamps.len() - keep].to_owned();
        for (stamp, path) in &sessions {
            if *stamp < cut {
                std::fs::remove_file(path)?;
            }
        }
    }
    Ok(())
}

/// The last `max` bytes of this session's log, from a line start. Lines were redacted as they were written.
#[must_use]
pub fn session_tail(max: usize) -> Option<String> {
    let folder = folder()?;
    let bytes = std::fs::read(folder.join(LOG_FILE)).ok()?;
    let start = bytes.len().saturating_sub(max);
    let tail = &bytes[start..];
    let tail = if start > 0 {
        tail.iter()
            .position(|b| *b == b'\n')
            .map_or(&tail[..0], |i| &tail[i + 1..])
    } else {
        tail
    };
    Some(String::from_utf8_lossy(tail).into_owned())
}

/// The log folder, once set.
#[must_use]
pub fn folder() -> Option<PathBuf> {
    state().folder.clone()
}

/// One line without its line break: `<UTC time with milliseconds>Z <LEVEL> <message>`.
#[must_use]
pub fn format_line(time: DateTime<Utc>, level: Level, message: &str) -> String {
    format!(
        "{} {} {message}",
        time.to_rfc3339_opts(SecondsFormat::Millis, true),
        level.as_str()
    )
}

fn append(folder: &Path, line: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(folder)?;
    let file = folder.join(LOG_FILE);
    if std::fs::metadata(&file).is_ok_and(|m| m.len() > MAX_LOG_BYTES) {
        std::fs::rename(&file, folder.join(OLD_LOG_FILE))?;
    }
    let mut handle = OpenOptions::new().create(true).append(true).open(&file)?;
    handle.write_all(line.as_bytes())
}

/// Writes one redacted line.
pub fn write(level: Level, message: &str) {
    let state = state();
    let mut line = format_line(Utc::now(), level, &state.redactor.redact(message));
    line.push('\n');
    let written = state.folder.as_deref().is_some_and(|f| append(f, &line).is_ok());
    if !written {
        let _ = std::io::stderr().write_all(line.as_bytes());
    }
}

/// Writes an INFO line.
pub fn info(message: impl AsRef<str>) {
    write(Level::Info, message.as_ref());
}

/// Writes a WARN line.
pub fn warn(message: impl AsRef<str>) {
    write(Level::Warn, message.as_ref());
}

/// Writes an ERROR line.
pub fn error(message: impl AsRef<str>) {
    write(Level::Error, message.as_ref());
}

#[derive(Default)]
struct EventText {
    message: String,
    fields: String,
}

impl Visit for EventText {
    fn record_str(&mut self, field: &Field, value: &str) {
        // `tracing-log` carries the record's target and location as `log.*` fields; the target is written first.
        if field.name().starts_with("log.") {
            return;
        }
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            let _ = write!(self.fields, " {}={value}", field.name());
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name().starts_with("log.") {
            return;
        }
        if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
        } else {
            let _ = write!(self.fields, " {}={value:?}", field.name());
        }
    }
}

/// Writes library events (`tracing`, INFO and above) as `<target>: <message> <field>=<value>...`.
#[derive(Debug, Clone, Copy, Default)]
pub struct LogLayer;

impl<S: Subscriber> Layer<S> for LogLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let normalized = event.normalized_metadata();
        let meta = normalized.as_ref().unwrap_or_else(|| event.metadata());
        if !accepts(meta.target(), *meta.level()) {
            return;
        }
        let level = match *meta.level() {
            tracing::Level::ERROR => Level::Error,
            tracing::Level::WARN => Level::Warn,
            tracing::Level::INFO => Level::Info,
            _ => return,
        };
        let mut text = EventText::default();
        event.record(&mut text);
        write(level, &format!("{}: {}{}", meta.target(), text.message, text.fields));
    }
}

/// Whether the subscriber passes an event or span: INFO and above, nothing from `egs-api`.
#[must_use]
pub fn accepts(target: &str, level: tracing::Level) -> bool {
    level <= tracing::Level::INFO && !dainty_epic::is_egs_api_target(target)
}

/// The subscriber-wide filter of [`accepts`]. An event the `log` crate forwarded through `tracing-log` has the
/// target `log`, so its real target is checked once the event exists.
#[derive(Debug, Clone, Copy, Default)]
pub struct LogFilter;

impl<S: Subscriber> Layer<S> for LogFilter {
    fn enabled(&self, metadata: &Metadata<'_>, _ctx: Context<'_, S>) -> bool {
        accepts(metadata.target(), *metadata.level())
    }

    fn event_enabled(&self, event: &Event<'_>, _ctx: Context<'_, S>) -> bool {
        event
            .normalized_metadata()
            .is_none_or(|meta| accepts(meta.target(), *meta.level()))
    }
}

/// Installs [`LogLayer`] as the process's `tracing` subscriber and logs panics as `panic: <info>`. False when
/// another subscriber was already installed; the panic hook is set either way.
pub fn install() -> bool {
    std::panic::set_hook(Box::new(|info| error(format!("panic: {info}"))));
    // A global filter: egs-api's events can carry tokens and must not reach any layer, not just this one.
    let subscriber = tracing_subscriber::registry().with(LogFilter).with(LogLayer);
    tracing::subscriber::set_global_default(subscriber).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_has_utc_millis_and_level() {
        let time = DateTime::parse_from_rfc3339("2026-11-06T09:08:07.5+02:00")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(
            format_line(time, Level::Warn, "hello"),
            "2026-11-06T07:08:07.500Z WARN hello"
        );
    }

    #[test]
    fn append_rotates_past_two_mib() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join(LOG_FILE);
        std::fs::write(&log, vec![b'x'; usize::try_from(MAX_LOG_BYTES).unwrap() + 1]).unwrap();
        append(tmp.path(), "next\n").unwrap();
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "next\n");
        assert_eq!(
            std::fs::metadata(tmp.path().join(OLD_LOG_FILE)).unwrap().len(),
            MAX_LOG_BYTES + 1
        );

        // Exactly 2 MiB is not over the limit.
        std::fs::write(&log, vec![b'x'; usize::try_from(MAX_LOG_BYTES).unwrap()]).unwrap();
        append(tmp.path(), "more\n").unwrap();
        assert_eq!(std::fs::metadata(&log).unwrap().len(), MAX_LOG_BYTES + 5);
    }

    #[test]
    fn sessions_rotate_and_keep_the_newest() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        for n in 0..14 {
            std::fs::write(dir.join(format!("launcher-2026111{}T000000{n:03}Z-2.log", n / 10)), "s").unwrap();
        }
        std::fs::write(dir.join("launcher-20261101T000000000Z-1.log"), "old part").unwrap();
        std::fs::write(dir.join(LOG_FILE), "current").unwrap();
        std::fs::write(dir.join(OLD_LOG_FILE), "overflow").unwrap();
        std::fs::write(dir.join("notes.txt"), "kept").unwrap();
        rotate_sessions(dir, 3).unwrap();

        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert!(!dir.join(LOG_FILE).exists() && !dir.join(OLD_LOG_FILE).exists());
        assert_eq!(names.len(), 5, "{names:?}");
        assert_eq!(names[0], "launcher-20261111T000000012Z-2.log");
        assert_eq!(names[1], "launcher-20261111T000000013Z-2.log");
        // The just-archived session is newest, with its overflow beside it.
        assert!(names[2].ends_with("-1.log") && names[3].ends_with("-2.log"));
        assert_eq!(std::fs::read_to_string(dir.join(&names[2])).unwrap(), "overflow");
        assert_eq!(std::fs::read_to_string(dir.join(&names[3])).unwrap(), "current");
        assert_eq!(names[4], "notes.txt");

        // A second rotation in the same instant doesn't overwrite the first.
        std::fs::write(dir.join(LOG_FILE), "next").unwrap();
        rotate_sessions(dir, 10).unwrap();
        let count = std::fs::read_dir(dir).unwrap().count();
        assert_eq!(count, 6);
    }

    #[test]
    fn subscriber_filter_drops_egs_api_and_debug() {
        assert!(accepts("launcher", tracing::Level::INFO));
        assert!(accepts("launcher", tracing::Level::ERROR));
        assert!(!accepts("launcher", tracing::Level::DEBUG));
        assert!(!accepts("egs_api", tracing::Level::ERROR));
        assert!(!accepts("egs_api::api", tracing::Level::WARN));
    }

    // The only test here that touches the process-wide state.
    #[test]
    fn events_and_lines_reach_the_file_redacted() {
        let tmp = tempfile::tempdir().unwrap();
        set_folder(tmp.path());
        register_secret("hunter2-secret");
        let subscriber = tracing_subscriber::registry().with(LogLayer);
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target: "kit", host = "example", port = 5, "connected");
            tracing::debug!(target: "kit", "dropped");
        });
        // Forwarded from the `log` crate: the real target is normalised back, and egs-api's never reaches a layer.
        let subscriber = tracing_subscriber::registry().with(LogFilter).with(LogLayer);
        tracing::subscriber::with_default(subscriber, || {
            let forward = |target: &str, text: &str| {
                tracing_log::format_trace(
                    &tracing_log::log::Record::builder()
                        .args(format_args!("{text}"))
                        .level(tracing_log::log::Level::Warn)
                        .target(target)
                        .module_path(Some(target))
                        .build(),
                )
                .unwrap();
            };
            forward("egs_api::api", "Bearer leaked-token");
            forward("egs_api", "leaked-too");
            forward("hyper::client", "retrying");
            tracing::warn!(target: "egs_api::api", "direct leak");
        });
        warn("token hunter2-secret DLK_abc");
        unregister_secret("hunter2-secret");
        info("hunter2-secret");

        let text = std::fs::read_to_string(tmp.path().join(LOG_FILE)).unwrap();
        let messages: Vec<&str> = text.lines().map(|l| l.split_once(' ').unwrap().1).collect();
        assert_eq!(
            messages,
            [
                "INFO kit: connected host=example port=5",
                "WARN hyper::client: retrying",
                "WARN token <hidden> DLK_<hidden>",
                "INFO hunter2-secret",
            ]
        );
    }
}
