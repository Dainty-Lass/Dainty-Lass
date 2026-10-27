//! What a host may change about chat. DR read these from the metagame's env file (`chat/mod.rs`'s
//! `Settings::from_env`); a core plugin gets them from the controller, which reads `host.toml` (DF3), so
//! nothing here reads the environment.

use std::net::{IpAddr, Ipv4Addr};

/// The plugin's loopback port, behind the gateway: free in `PORTS` and `BOTH_LEGACY_PORTS`.
pub const DEFAULT_PORT: u16 = 61006;

/// The WebSocket paths the clients ask for (Mystic Paradox's `REALTIME_WS_PATHS`): both clients build the URL
/// from a host-only address and ask for `//`; `/` and `/__ws/xmpp` are kept. Anything else is a 404.
pub const WS_PATHS: [&str; 3] = ["//", "/", "/__ws/xmpp"];

/// DR's `CHAT_NICK_CHECK`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NickCheck {
    Enforce,
    /// Admits room nicknames that fail the resource, format or name rule, with a warning (a rollback switch);
    /// another account's id is refused either way
    Log,
}

impl NickCheck {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Enforce => "enforce",
            Self::Log => "log",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// 127.0.0.1 or ::1 only: the gateway is the only listener off loopback (AGENTS.md)
    pub host: IpAddr,
    /// 0 picks a free port (tests)
    pub port: u16,
    /// `GATEWAY_SECRET`: with it, a connection from loopback that presents it in `X-Reforged-Gateway` is the
    /// gateway, and the player's address is its `X-Forwarded-For`
    pub gateway_secret: Option<String>,
    pub nick_check: NickCheck,
    /// DR's `CHAT_TRACE`: frames in the log, redacted
    pub trace: bool,
    /// Messages of a room's history a joiner is sent, for rooms whose rules keep history (0: none)
    pub history_on_join: i64,
    /// Waiting direct messages sent at one login, oldest first; the rest wait for the next
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
    /// What would keep chat from starting.
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if !self.host.is_loopback() {
            problems.push(format!(
                "chat must listen on 127.0.0.1 or ::1 (the gateway forwards chat there), not {}",
                self.host
            ));
        }
        if self.gateway_secret.as_deref().is_some_and(str::is_empty) {
            problems.push("the gateway secret is empty".to_owned());
        }
        if !(0..=1000).contains(&self.history_on_join) || !(1..=1000).contains(&self.offline_batch) {
            problems.push("history_on_join is 0 to 1000 and offline_batch 1 to 1000".to_owned());
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
        let lan = Settings {
            host: "0.0.0.0".parse().unwrap(),
            ..Settings::default()
        };
        assert_eq!(lan.problems().len(), 1);
        let v6 = Settings {
            host: "::1".parse().unwrap(),
            ..Settings::default()
        };
        assert!(v6.problems().is_empty());
        let empty = Settings {
            gateway_secret: Some(String::new()),
            ..Settings::default()
        };
        assert_eq!(empty.problems().len(), 1);
    }
}
