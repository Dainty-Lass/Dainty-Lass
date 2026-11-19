// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The content server: the game runtime the launcher installs, the server's news and its art pack, and what
//! the attached game modules register (their versions, client profiles and Epic manifests). It listens on
//! loopback only; players reach it through the gateway (docs/security/content.md).
//!
//! Written clean-room from `docs/port/spec-rd27-content.md` (RD27): replies, error codes and field order match
//! the Node server the parity fixtures were recorded from.

pub mod app;
pub mod auth;
pub mod branding;
pub mod cli;
pub mod client;
pub mod config;
pub mod crash;
pub mod input;
pub mod limits;
pub mod log;
pub mod modules;
pub mod news;
pub mod runtime;
pub mod server;
pub mod service;

/// Milliseconds since 1970, the clock every cache and re-check reads.
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// A clock the stores read; tests hand in their own.
pub type Clock = std::sync::Arc<dyn Fn() -> i64 + Send + Sync>;

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    hex::encode(sha2::Sha256::digest(bytes))
}

/// Sends log lines to `sink` until the guard drops; one capture at a time.
#[cfg(test)]
pub(crate) fn log_capture(sink: impl Fn(&str) + Send + Sync + 'static) -> LogCapture {
    static ONE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = ONE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    log::set_sink(Some(Box::new(move |line, _| sink(line))));
    LogCapture(guard)
}

#[cfg(test)]
pub(crate) struct LogCapture(
    #[allow(dead_code, reason = "held only so the log lock lasts as long as the capture")]
    std::sync::MutexGuard<'static, ()>,
);

#[cfg(test)]
impl Drop for LogCapture {
    fn drop(&mut self) {
        log::set_sink(None);
    }
}
