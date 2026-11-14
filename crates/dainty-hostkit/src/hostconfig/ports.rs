// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `[ports]`: the TCP ports the server's own services listen on, for a host whose machine holds one of the
//! defaults ([`PORTS`]). `gateway` is a private server's gateway, which its invites name; a public one's is
//! `public_port`. On Windows, setup and start fill this in themselves when a port is lent out (`winports`).
//!
//! ```toml
//! [ports]
//! backend = 41000
//! director = 41001
//! ```

use serde_json::{Map, Value};

use crate::error::HostError;
use crate::paths::{PORTS, Ports};

/// The keys `[ports]` takes, as host.json and settings.toml both spell them.
pub const KEYS: [&str; 8] = [
    "gateway",
    "backend",
    "director",
    "qos",
    "content",
    "admin",
    "allowlist",
    "chat",
];

/// `[ports]`; a port left out keeps its default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PortsConfig {
    /// A private server's gateway.
    pub gateway: Option<u16>,
    /// The backend's.
    pub backend: Option<u16>,
    /// The Director's.
    pub director: Option<u16>,
    /// The backend's QoS listener's.
    pub qos: Option<u16>,
    /// The content server's.
    pub content: Option<u16>,
    /// The admin service's.
    pub admin: Option<u16>,
    /// The allowlist's.
    pub allowlist: Option<u16>,
    /// Chat's, behind the gateway.
    pub chat: Option<u16>,
}

fn bad(text: impl Into<String>) -> HostError {
    HostError::message(text)
}

impl PortsConfig {
    /// Reads `[ports]`; fails on a key it doesn't know, a value that is not a TCP port, or two services on one port.
    pub fn from_json(value: &Value) -> Result<Self, HostError> {
        let obj = value.as_object().ok_or_else(|| bad("ports must be a table"))?;
        for key in obj.keys() {
            if !KEYS.contains(&key.as_str()) {
                return Err(bad(format!("ports.{key} is not a setting")));
            }
        }
        let mut config = Self::default();
        for key in KEYS {
            let port = match obj.get(key).filter(|v| !v.is_null()) {
                None => None,
                Some(v) => Some(
                    v.as_u64()
                        .and_then(|n| u16::try_from(n).ok())
                        .filter(|n| *n != 0)
                        .ok_or_else(|| bad(format!("ports.{key} must be a TCP port from 1 to 65535")))?,
                ),
            };
            if let Some(slot) = config.slot(key) {
                *slot = port;
            }
        }
        let named = config.named();
        for (i, (key, port)) in named.iter().enumerate() {
            if let Some((other, _)) = named[..i].iter().find(|(_, p)| p == port) {
                return Err(bad(format!(
                    "ports.{key} and ports.{other} are both {port}: give each its own"
                )));
            }
        }
        Ok(config)
    }

    /// The setting `key` names, or `None` for a key not in [`KEYS`].
    pub fn slot(&mut self, key: &str) -> Option<&mut Option<u16>> {
        Some(match key {
            "gateway" => &mut self.gateway,
            "backend" => &mut self.backend,
            "director" => &mut self.director,
            "qos" => &mut self.qos,
            "content" => &mut self.content,
            "admin" => &mut self.admin,
            "allowlist" => &mut self.allowlist,
            "chat" => &mut self.chat,
            _ => return None,
        })
    }

    /// The table settings.toml writes.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut copy = *self;
        let mut out = Map::new();
        for key in KEYS {
            if let Some(port) = copy.slot(key).and_then(|slot| *slot) {
                out.insert(key.into(), Value::from(port));
            }
        }
        Value::Object(out)
    }

    /// Every port in `[ports]`, the defaults filled in, with its key.
    #[must_use]
    pub fn named(&self) -> [(&'static str, u16); 8] {
        let p = self.resolve();
        [
            ("gateway", p.private_gateway),
            ("backend", p.backend),
            ("director", p.director),
            ("qos", p.qos),
            ("content", p.content),
            ("admin", p.admin),
            ("allowlist", p.allowlist),
            ("chat", p.chat),
        ]
    }

    /// [`PORTS`] with these ports over it.
    #[must_use]
    pub fn resolve(&self) -> Ports {
        Ports {
            private_gateway: self.gateway.unwrap_or(PORTS.private_gateway),
            backend: self.backend.unwrap_or(PORTS.backend),
            director: self.director.unwrap_or(PORTS.director),
            qos: self.qos.unwrap_or(PORTS.qos),
            content: self.content.unwrap_or(PORTS.content),
            admin: self.admin.unwrap_or(PORTS.admin),
            allowlist: self.allowlist.unwrap_or(PORTS.allowlist),
            chat: self.chat.unwrap_or(PORTS.chat),
            ..PORTS
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_and_writes_back() {
        let value = json!({"gateway": 41080, "backend": 42000, "admin": 42003, "chat": 42006});
        let ports = PortsConfig::from_json(&value).unwrap();
        assert_eq!(ports.backend, Some(42000));
        assert_eq!(ports.to_json(), value);
        let resolved = ports.resolve();
        assert_eq!(
            (
                resolved.private_gateway,
                resolved.backend,
                resolved.admin,
                resolved.chat
            ),
            (41080, 42000, 42003, 42006)
        );
        assert_eq!(resolved.director, PORTS.director);
        assert_eq!(resolved.public_gateway, PORTS.public_gateway);
        assert_eq!(PortsConfig::from_json(&json!({})).unwrap().resolve(), PORTS);
    }

    #[test]
    fn refuses_what_it_does_not_know() {
        for value in [
            json!({"backend": 0}),
            json!({"backend": 65536}),
            json!({"backend": -1}),
            json!({"backend": "41000"}),
            json!({"relay": 61100}),
            json!({"backend": 42000, "director": 42000}),
            json!({"content": PORTS.admin}),
            json!({"chat": PORTS.private_gateway}),
            json!(41000),
        ] {
            assert!(PortsConfig::from_json(&value).is_err(), "{value}");
        }
    }
}
