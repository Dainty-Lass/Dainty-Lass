// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `[guard]` (DD23 layer 4, DH6): when a destructive or bulk admin action waits for a delay or a second
//! operator. New to host.toml, so a file without it keeps today behaviour, which is the admin API defaults:
//! ten bans or timeouts an hour from one key, a revert above a hundred changes and the revoking of an admin or
//! owner key each wait ten minutes. Each setting is kept as read, so writing the file back changes nothing
//! unasked, and only the ones set reach `admin.env`.
//!
//! ```toml
//! [guard]
//! ban_limit = 10            # bans and timeouts one key may make in the window before the next one is held
//! ban_window_minutes = 60
//! ban_guard = "delay"       # "delay", "approval" (a second operator) or "off"
//! revert_limit = 100        # a history revert of more changes than this is held
//! revert_guard = "delay"
//! revoke_guard = "delay"    # revoking an admin or owner key
//! delay_minutes = 10        # how long a delay waits
//! expire_hours = 48         # how long one nobody approved stays before it expires
//! ```

use serde_json::{Map, Value};

use super::{bad, integer, shown};
use crate::error::HostError;

/// The limits of `dainty_admin::guard::GuardSettings`, checked here so a bad value stops `setup`.
const MAX_LIMIT: f64 = 1_000_000.0;
const MAX_WINDOW_MINUTES: f64 = 1_440.0;
const MAX_DELAY_MINUTES: f64 = 1_440.0;
const MAX_EXPIRE_HOURS: f64 = 720.0;

/// A whole-number setting: its host.json key, the `admin.env` variable and its largest value.
const NUMBERS: [(&str, &str, f64); 5] = [
    ("banLimit", "DAINTY_GUARD_BAN_LIMIT", MAX_LIMIT),
    (
        "banWindowMinutes",
        "DAINTY_GUARD_BAN_WINDOW_MINUTES",
        MAX_WINDOW_MINUTES,
    ),
    ("revertLimit", "DAINTY_GUARD_REVERT_LIMIT", MAX_LIMIT),
    ("delayMinutes", "DAINTY_GUARD_DELAY_MINUTES", MAX_DELAY_MINUTES),
    ("expireHours", "DAINTY_GUARD_EXPIRE_HOURS", MAX_EXPIRE_HOURS),
];

/// A rule setting: what to do with an action over its threshold.
const RULES: [(&str, &str); 3] = [
    ("banGuard", "DAINTY_GUARD_BAN"),
    ("revertGuard", "DAINTY_GUARD_REVERT"),
    ("revokeGuard", "DAINTY_GUARD_REVOKE"),
];

/// What a `*_guard` setting may say.
pub const RULE_WORDS: [&str; 3] = ["off", "delay", "approval"];

/// `[guard]`, each setting `None` when host.toml leaves it out (the admin API's default then holds).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GuardConfig {
    /// Bans and timeouts one key may make in the window before the next is held.
    pub ban_limit: Option<u32>,
    /// The window `ban_limit` counts over, in minutes.
    pub ban_window_minutes: Option<u32>,
    /// What holds a ban over the limit: `off`, `delay` or `approval`.
    pub ban_guard: Option<String>,
    /// A revert of more changes than this is held.
    pub revert_limit: Option<u32>,
    /// What holds such a revert: `off`, `delay` or `approval`.
    pub revert_guard: Option<String>,
    /// What holds the revoking of an admin or owner key: `off`, `delay` or `approval`.
    pub revoke_guard: Option<String>,
    /// How long a delay waits, in minutes.
    pub delay_minutes: Option<u32>,
    /// How long a held action nobody approved stays before it expires, in hours.
    pub expire_hours: Option<u32>,
}

/// Checks a `guard` object as host.json carries it.
pub fn validate(guard: &Value) -> Result<(), HostError> {
    let g = guard.as_object().ok_or_else(|| bad("guard must be an object"))?;
    for (key, _, max) in NUMBERS {
        if let Some(value) = g.get(key)
            && !integer(Some(value)).is_some_and(|n| (1.0..=max).contains(&n))
        {
            return Err(bad(format!(
                "guard.{key} must be a whole number from 1 to {max}, not {}",
                shown(Some(value))
            )));
        }
    }
    for (key, _) in RULES {
        if let Some(value) = g.get(key)
            && !value.as_str().is_some_and(|w| RULE_WORDS.contains(&w))
        {
            return Err(bad(format!(
                "guard.{key} must be \"off\", \"delay\" or \"approval\", not {}",
                shown(Some(value))
            )));
        }
    }
    Ok(())
}

impl GuardConfig {
    /// Reads a validated `guard` table.
    pub fn from_json(g: &Map<String, Value>) -> Self {
        let number = |key: &str| g.get(key).and_then(Value::as_u64).map(|n| n as u32);
        let word = |key: &str| g.get(key).and_then(Value::as_str).map(str::to_owned);
        Self {
            ban_limit: number("banLimit"),
            ban_window_minutes: number("banWindowMinutes"),
            ban_guard: word("banGuard"),
            revert_limit: number("revertLimit"),
            revert_guard: word("revertGuard"),
            revoke_guard: word("revokeGuard"),
            delay_minutes: number("delayMinutes"),
            expire_hours: number("expireHours"),
        }
    }

    /// host.json form: the keys that were set, in host.toml order.
    pub fn to_json(&self) -> Value {
        let number = |value: Option<u32>| value.map(Value::from);
        let word = |value: &Option<String>| value.clone().map(Value::String);
        let fields = [
            ("banLimit", number(self.ban_limit)),
            ("banWindowMinutes", number(self.ban_window_minutes)),
            ("banGuard", word(&self.ban_guard)),
            ("revertLimit", number(self.revert_limit)),
            ("revertGuard", word(&self.revert_guard)),
            ("revokeGuard", word(&self.revoke_guard)),
            ("delayMinutes", number(self.delay_minutes)),
            ("expireHours", number(self.expire_hours)),
        ];
        Value::Object(
            fields
                .into_iter()
                .filter_map(|(key, value)| value.map(|v| (key.to_owned(), v)))
                .collect(),
        )
    }

    /// The `admin.env` lines: only what the host set, so a file without `[guard]` writes none.
    pub fn env(&self) -> Vec<(&'static str, String)> {
        let json = self.to_json();
        let mut out = Vec::new();
        for (key, name, _) in NUMBERS {
            if let Some(n) = json.get(key).and_then(Value::as_u64) {
                out.push((name, n.to_string()));
            }
        }
        for (key, name) in RULES {
            if let Some(word) = json.get(key).and_then(Value::as_str) {
                out.push((name, word.to_owned()));
            }
        }
        out
    }
}
