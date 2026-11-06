// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Chat's settings: where it listens, the gateway's secret and its limits on history.

use std::net::{IpAddr, Ipv4Addr};

/// The port chat listens on unless the env file says otherwise.
pub const DEFAULT_PORT: u16 = 61006;
/// The paths the game's client opens its WebSocket on
pub const WS_PATHS: [&str; 3] = ["//", "/", "/__ws/xmpp"];

/// What chat does with a room nick that does not name its sender (`nick.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NickCheck {
    /// A nick that doesn't name its sender is refused
    Enforce,
    /// Only logged, but for another account's id, which is always refused
    Log,
}

impl NickCheck {
    /// The env file's name for it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Enforce => "enforce",
            Self::Log => "log",
        }
    }
}

/// Chat's settings from its env file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// The address to listen on; loopback only.
    pub host: IpAddr,
    /// The port, 0 for any.
    pub port: u16,
    /// The secret the gateway sends in `x-reforged-gateway`, so its `x-forwarded-for` is trusted. Never logged.
    pub gateway_secret: Option<String>,
    /// What a wrong nick does.
    pub nick_check: NickCheck,
    /// Whether to log every stanza in and out, with credentials redacted.
    pub trace: bool,
    /// Room messages a joiner gets
    pub history_on_join: i64,
    /// Waiting direct messages delivered at one bind
    pub offline_batch: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            host: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: DEFAULT_PORT,
            gateway_secret: None,
            nick_check: NickCheck::Enforce,
            trace: false,
            history_on_join: 20,
            offline_batch: 100,
        }
    }
}

impl Settings {
    /// What is wrong with the settings, as sentences for the host; empty when they work.
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if !self.host.is_loopback() {
            problems.push(format!(
                "chat must listen on 127.0.0.1 or ::1 (the gateway forwards chat there), not {}",
                self.host
            ));
        }
        if self.gateway_secret.as_deref() == Some("") {
            problems.push(String::from("the gateway secret is empty"));
        }
        if !(0..=1000).contains(&self.history_on_join) || !(1..=1000).contains(&self.offline_batch) {
            problems.push(String::from("history_on_join is 0 to 1000 and offline_batch 1 to 1000"));
        }
        problems
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_stays_on_loopback() {
        assert!(Settings::default().problems().is_empty());
        assert_eq!(Settings::default().port, 61006);
        let ipv6 = Settings {
            host: "::1".parse().unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
            ..Settings::default()
        };
        assert!(ipv6.problems().is_empty());
        let all = Settings {
            host: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            gateway_secret: Some(String::new()),
            history_on_join: 1001,
            ..Settings::default()
        };
        assert_eq!(
            all.problems(),
            vec![
                String::from("chat must listen on 127.0.0.1 or ::1 (the gateway forwards chat there), not 0.0.0.0"),
                String::from("the gateway secret is empty"),
                String::from("history_on_join is 0 to 1000 and offline_batch 1 to 1000"),
            ]
        );
        let batch = Settings {
            offline_batch: 0,
            ..Settings::default()
        };
        assert_eq!(batch.problems().len(), 1);
        assert_eq!(NickCheck::Log.as_str(), "log");
    }
}
