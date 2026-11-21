// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The shadow replay harness of Rustic Isles (RU8; spec-rustic-isles-replay): reads the gateway's capture, sends
//! each recorded request again to one or two targets with working credentials and core's replay headers, follows the
//! ids each target mints, normalizes what legitimately differs and classifies what remains.

mod capture;
pub mod cli;
mod compare;
mod credentials;
mod ids;
mod normalize;
mod options;
mod request;
mod runner;
mod transport;

pub use capture::{
    CaptureBody, CaptureRecord, ChatRecord, HttpRecord, load_corpus, parse_capture_line, parse_capture_str,
};
pub use compare::{
    DiffKind, compare_chat_stanzas, compare_http_responses, is_filtered_chat_stanza, normalize_chat_stanza,
};
pub use credentials::{
    ANON_SUBJECT, DEFAULT_HS256_SECRET, ReplayTargetAuth, Substituted, mint_hs256_jwt, mint_rs256_jwt, substitute_str,
    substitute_value,
};
pub use ids::{IdMap, apply_ids_str, apply_ids_value, learn_ids};
pub use normalize::{
    IGNORED_RESPONSE_HEADERS, IdNormalizer, NormalizerConfig, normalize_headers, normalize_jwts_in_str,
};
pub use options::{ReplayOptions, key_map_from_json, load_key_map, run_replay_cli};
pub use request::{FALLBACK_NOW_MS, PreparedHttpRequest, ReplayHttpResponse, prepare_http_request};
pub use runner::{EntryKind, ReplayConfig, ReplayEndpoint, ReplayEntryResult, ReplayReport, ReplayTarget, run_replay};
pub use transport::{
    EphemeralServer, dispatch_router_request, parse_http_response, send_http_request, serve_ephemeral,
};

pub(crate) fn pattern(text: &str) -> regex::Regex {
    regex::Regex::new(text).unwrap_or_else(|_| unreachable!("a fixed pattern"))
}

/// §9.5. A token error never ends a run: substitution turns it into an inexact record.
#[derive(Debug, thiserror::Error)]
pub enum ReplayError {
    /// A corpus or report file could not be read or written.
    #[error("I/O error at {path}: {message}")]
    Io {
        /// The path read or written.
        path: String,
        /// The I/O error's text.
        message: String,
    },
    /// A bad option.
    #[error("invalid replay configuration: {0}")]
    Config(String),
    /// A token could not be signed.
    #[error("token signing error: {0}")]
    Token(String),
    /// A request to a side failed to send or answer.
    #[error("HTTP transport error calling {target}: {message}")]
    Transport {
        /// The URL or address called.
        target: String,
        /// What went wrong.
        message: String,
    },
    /// The report could not be serialized.
    #[error("JSON serialization error: {0}")]
    Json(String),
}

impl ReplayError {
    /// The message inside the kind's prefix (§10.3 wraps corpus and run errors as configuration errors).
    pub fn inner_message(&self) -> String {
        match self {
            Self::Config(m) | Self::Token(m) | Self::Json(m) => m.clone(),
            other => other.to_string(),
        }
    }
}
