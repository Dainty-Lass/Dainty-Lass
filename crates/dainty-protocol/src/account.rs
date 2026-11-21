// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! Account registration: the request and reply bodies, username rules and account key extraction.

use serde::{Deserialize, Serialize};

use crate::text::{is_js_space, js_length, js_trim};

/// Route the launcher posts a [`RegisterRequest`] to.
pub const REGISTER_PATH: &str = "/dainty/api/Register";
/// Prefix of every account key the server issues.
pub const ACCOUNT_KEY_PREFIX: &str = "DLK_";
/// Shortest accepted username, in bytes (usernames are ASCII).
pub const USERNAME_MIN: usize = 3;
/// Longest accepted username, in bytes.
pub const USERNAME_MAX: usize = 16;

/// Route the launcher posts to begin an OAuth sign-in.
pub const OAUTH_START_PATH: &str = "/dainty/api/OAuthStart";
/// Route the launcher posts the provider's code to, for the account key.
pub const OAUTH_FINISH_PATH: &str = "/dainty/api/OAuthFinish";
/// The loopback port the launcher takes the provider's redirect on (RFC 8252 section 7.3). Fixed, because Discord
/// and most providers match a redirect URI exactly, port included.
pub const OAUTH_REDIRECT_PORT: u16 = 61190;
/// The path of [`OAUTH_REDIRECT_URI`].
pub const OAUTH_REDIRECT_PATH: &str = "/oauth/callback";
/// The redirect URI every server registers with its provider and the launcher listens on.
pub const OAUTH_REDIRECT_URI: &str = "http://127.0.0.1:61190/oauth/callback";

const MAX_KEY_EXTRACT_UTF16_LEN: usize = 8192;

/// Body of a registration request, PascalCase on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct RegisterRequest {
    /// Requested username, checked with [`check_username`].
    pub username: String,
    /// Invite code from the player's invite.
    pub invite_code: String,
}

/// Successful registration reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registered {
    /// The new account key (`DLK_...`), sent as `UUK`.
    #[serde(rename = "UUK")]
    pub key: String,
}

/// Outcome of [`check_username`], serialized as the snake_case code the launcher translates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsernameCheck {
    /// The username is acceptable.
    Ok,
    /// Nothing was entered.
    Empty,
    /// Shorter than [`USERNAME_MIN`].
    TooShort,
    /// Longer than [`USERNAME_MAX`].
    TooLong,
    /// Holds a character other than an ASCII letter, digit or underscore.
    #[serde(rename = "bad_chars")]
    Chars,
}

impl UsernameCheck {
    /// The wire code, as serialized.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Empty => "empty",
            Self::TooShort => "too_short",
            Self::TooLong => "too_long",
            Self::Chars => "bad_chars",
        }
    }
}

/// Validates a candidate username against the registration rules.
#[must_use]
pub fn check_username(raw: &str) -> UsernameCheck {
    if raw.is_empty() {
        return UsernameCheck::Empty;
    }
    if !raw.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return UsernameCheck::Chars;
    }
    let len = raw.len();
    if len < USERNAME_MIN {
        return UsernameCheck::TooShort;
    }
    if len > USERNAME_MAX {
        return UsernameCheck::TooLong;
    }
    UsernameCheck::Ok
}

/// Returns `true` if `raw` is a valid username.
#[must_use]
pub fn is_valid_username(raw: &str) -> bool {
    check_username(raw) == UsernameCheck::Ok
}

/// Returns `true` if `key` is a plausible account key token (`^[A-Za-z0-9_-]{8,128}$`).
#[must_use]
pub fn is_plausible_account_key(key: &str) -> bool {
    (8..=128).contains(&key.len()) && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn is_js_line_terminator(ch: char) -> bool {
    matches!(ch, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// Extracts an account key from pasted text or an account key backup file.
#[must_use]
pub fn extract_account_key(raw: &str) -> Option<&str> {
    let trimmed = js_trim(raw);
    if trimmed.is_empty() || js_length(trimmed) > MAX_KEY_EXTRACT_UTF16_LEN {
        return None;
    }

    if is_plausible_account_key(trimmed) {
        return Some(trimmed);
    }

    // Check line-anchored `^Key:\s*(\S+)` (ECMAScript `/m` mode)
    let mut search_from = 0;
    while let Some(rel) = trimmed[search_from..].find("Key:") {
        let pos = search_from + rel;
        let at_line_start = pos == 0 || trimmed[..pos].chars().next_back().is_some_and(is_js_line_terminator);
        if at_line_start {
            let after_label = &trimmed[pos + 4..];
            let ws_bytes: usize = after_label
                .chars()
                .take_while(|&c| is_js_space(c))
                .map(char::len_utf8)
                .sum();
            let rest = &after_label[ws_bytes..];
            let token_bytes: usize = rest.chars().take_while(|&c| !is_js_space(c)).map(char::len_utf8).sum();
            let token = &rest[..token_bytes];
            if is_plausible_account_key(token) {
                return Some(token);
            }
            break;
        }
        search_from = pos + 4;
    }

    // Check inline `Key:\s*(DLK_[0-9a-fA-F]{48})(?![0-9a-fA-F])`
    let mut inline_from = 0;
    while let Some(rel) = trimmed[inline_from..].find("Key:") {
        let pos = inline_from + rel;
        let after_label = &trimmed[pos + 4..];
        let ws_bytes: usize = after_label
            .chars()
            .take_while(|&c| is_js_space(c))
            .map(char::len_utf8)
            .sum();
        let rest = &after_label[ws_bytes..];
        if let Some(after_prefix) = rest.strip_prefix(ACCOUNT_KEY_PREFIX) {
            let hex_len = after_prefix.bytes().take(48).take_while(u8::is_ascii_hexdigit).count();
            let next_is_hex = after_prefix.as_bytes().get(48).is_some_and(u8::is_ascii_hexdigit);
            if hex_len == 48 && !next_is_hex {
                return Some(&rest[..52]);
            }
        }
        inline_from = pos + 4;
    }

    None
}
