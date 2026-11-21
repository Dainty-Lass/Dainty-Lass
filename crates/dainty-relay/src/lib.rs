// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The game's relay to its server: a loopback listener that carries the game's HTTP and WebSocket upgrades over
//! TLS pinned to the invite's certificate (or plain HTTP to a private-mode host). The runtime runs it inside the
//! game, so the game no longer needs the launcher once it has started; the launcher keeps it as a fallback for a
//! server whose runtime predates that.

pub mod connector;
pub mod endpoint;
pub mod owner;
pub mod pinned;
pub mod pins;
pub mod relay;

/// How bad a relay line is; the sink decides where it goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Normal progress.
    Info,
    /// Something failed but the relay carries on.
    Warn,
    /// Something failed.
    Error,
}

/// An upstream error as a log line. The relay never puts a header value in an error, so the text needs no
/// redaction here; a sink that knows secrets redacts them itself.
#[must_use]
pub fn describe_error(error: &dyn std::error::Error) -> String {
    error.to_string()
}
