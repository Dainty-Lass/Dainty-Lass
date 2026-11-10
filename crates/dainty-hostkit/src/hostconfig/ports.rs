// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! `[ports]`: the loopback TCP ports the server's own services listen on, for a host whose machine holds one of
//! the defaults ([`PORTS`]). The gateway's port is not here: a private server's is in every invite it handed out,
//! and a public one's is `public_port`.
//!
//! ```toml
//! [ports]
//! backend = 41000
//! director = 41001
//! ```

use serde_json::{Map, Value};

use crate::error::HostError;
use crate::paths::{PORTS, Ports};

/// The keys `[ports]` takes, as host.json and host.toml both spell them.
pub const KEYS: [&str; 6] = ["backend", "director", "qos", "content", "admin", "allowlist"];

/// `[ports]`; a port left out keeps its default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PortsConfig {
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
        let port = |key: &str| -> Result<Option<u16>, HostError> {
            match obj.get(key).filter(|v| !v.is_null()) {
                None => Ok(None),
                Some(v) => v
                    .as_u64()
                    .and_then(|n| u16::try_from(n).ok())
                    .filter(|n| *n != 0)
                    .map(Some)
                    .ok_or_else(|| bad(format!("ports.{key} must be a TCP port from 1 to 65535"))),
            }
        };
        let config = Self {
            backend: port("backend")?,
            director: port("director")?,
            qos: port("qos")?,
            content: port("content")?,
            admin: port("admin")?,
            allowlist: port("allowlist")?,
        };
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

    /// The table host.toml writes.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        for (key, port) in self.set() {
            out.insert(key.into(), Value::from(port));
        }
        Value::Object(out)
    }

    fn set(&self) -> Vec<(&'static str, u16)> {
        [
            ("backend", self.backend),
            ("director", self.director),
            ("qos", self.qos),
            ("content", self.content),
            ("admin", self.admin),
            ("allowlist", self.allowlist),
        ]
        .into_iter()
        .filter_map(|(key, port)| port.map(|p| (key, p)))
        .collect()
    }

    /// Every port in `[ports]`, the defaults filled in, with its key.
    #[must_use]
    pub fn named(&self) -> [(&'static str, u16); 6] {
        let p = self.resolve();
        [
            ("backend", p.backend),
            ("director", p.director),
            ("qos", p.qos),
            ("content", p.content),
            ("admin", p.admin),
            ("allowlist", p.allowlist),
        ]
    }

    /// [`PORTS`] with these ports over it.
    #[must_use]
    pub fn resolve(&self) -> Ports {
        Ports {
            backend: self.backend.unwrap_or(PORTS.backend),
            director: self.director.unwrap_or(PORTS.director),
            qos: self.qos.unwrap_or(PORTS.qos),
            content: self.content.unwrap_or(PORTS.content),
            admin: self.admin.unwrap_or(PORTS.admin),
            allowlist: self.allowlist.unwrap_or(PORTS.allowlist),
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
        let value = json!({"backend": 42000, "admin": 42003});
        let ports = PortsConfig::from_json(&value).unwrap();
        assert_eq!(ports.backend, Some(42000));
        assert_eq!(ports.to_json(), value);
        let resolved = ports.resolve();
        assert_eq!((resolved.backend, resolved.admin), (42000, 42003));
        assert_eq!(resolved.director, PORTS.director);
        assert_eq!(resolved.private_gateway, PORTS.private_gateway);
        assert_eq!(PortsConfig::from_json(&json!({})).unwrap().resolve(), PORTS);
    }

    #[test]
    fn refuses_what_it_does_not_know() {
        for value in [
            json!({"backend": 0}),
            json!({"backend": 65536}),
            json!({"backend": -1}),
            json!({"backend": "41000"}),
            json!({"gateway": 61000}),
            json!({"backend": 42000, "director": 42000}),
            json!({"content": PORTS.admin}),
            json!(41000),
        ] {
            assert!(PortsConfig::from_json(&value).is_err(), "{value}");
        }
    }
}
