// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The gateway: the only listener off loopback (TLS in public mode, plain HTTP inside a tailnet in private mode),
//! which judges every request by its route policy before anything reaches a loopback service, and the allowlist
//! helper, which opens the game's UDP ports only to players who logged in through it. Rewritten from the RD27
//! clean-room specification (docs/port/spec-rd27-gateway.md).

pub mod allowlist;
pub mod capture;
pub mod cert;
pub mod cli;
pub mod config;
pub mod feed;
#[allow(clippy::module_inception)]
pub mod gateway;
pub mod guard;
pub mod ip;
pub mod modules;
pub mod network;
pub mod nodelink;
pub mod policy;
pub mod ratelimit;
pub mod redact;
pub mod server;
pub mod service;

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// JavaScript's `toISOString` of a millisecond time.
pub(crate) fn iso_ms(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .unwrap_or_default()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

/// `JSON.stringify` of a string, for error texts that quote what they were given.
pub(crate) fn json_string(text: &str) -> String {
    serde_json::Value::String(text.to_owned()).to_string()
}

/// Lowercase hex of bytes.
pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::with_capacity(bytes.len() * 2), |mut out, b| {
        _ = write!(out, "{b:02x}");
        out
    })
}

/// Reads a whole number as Node's `Number(text)` does, within `min..=max`.
pub(crate) fn whole_number(text: &str, min: f64, max: f64) -> Option<f64> {
    let value = service_kit::js::number(Some(&serde_json::Value::String(text.to_owned())));
    (value.is_finite() && value.fract() == 0.0 && value >= min && value <= max).then_some(value)
}
