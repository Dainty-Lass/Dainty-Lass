// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! `[split]` (`docs/split-roles.md`): one server run as two halves on two machines or containers, the metagame (the
//! backend, content, admin) on one and the game servers (the Director and what it launches) on the other. Each half
//! is a node of the same `[network]`, and reaches the other's loopback port through the node link (`Service::Tunnel`),
//! so nothing but the gateways listens off loopback.
//!
//! ```toml
//! [split]
//! role = "metagame"
//! peer = "games-1"
//! ```

use dainty_module_api::network::NodeId;
use serde_json::{Map, Value};

use super::{NetworkConfig, ServerMode};
use crate::error::HostError;

/// Which half this server is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitRole {
    /// The backend, content and admin API; the Director's port is the tunnel to the peer
    Metagame,
    /// The Director and its game servers; the backend's port is the tunnel to the peer
    GameServers,
}

impl SplitRole {
    /// The name settings.toml spells it with.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Metagame => "metagame",
            Self::GameServers => "gameservers",
        }
    }

    /// `RUSTIC_ISLES_PORT_SERVICES`: the module's port service this half runs.
    pub fn port_service(self) -> &'static str {
        match self {
            Self::Metagame => "backend",
            Self::GameServers => "director",
        }
    }
}

/// `[split]`: this server as one half of a split server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitConfig {
    /// Which half this is.
    pub role: SplitRole,
    /// The other half: a node of `[[network.nodes]]`
    pub peer: NodeId,
    /// Game servers half only: its game ports stay open to everyone, since players' heartbeats reach the other
    /// half's gateway and this half's allowlist never sees them. Must be said, never assumed.
    pub open_game_ports: bool,
}

fn bad(text: impl Into<String>) -> HostError {
    HostError::message(text)
}

impl SplitConfig {
    /// Reads `[split]`; fails on a key it doesn't know or a value it doesn't take.
    pub fn from_json(value: &Value) -> Result<Self, HostError> {
        let obj = value.as_object().ok_or_else(|| bad("split must be a table"))?;
        for key in obj.keys() {
            if !["role", "peer", "openGamePorts"].contains(&key.as_str()) {
                return Err(bad(format!("split.{key} is not a setting")));
            }
        }
        let role = match obj.get("role").and_then(Value::as_str) {
            Some("metagame") => SplitRole::Metagame,
            Some("gameservers") => SplitRole::GameServers,
            _ => return Err(bad("split.role must be \"metagame\" or \"gameservers\"")),
        };
        let peer = obj
            .get("peer")
            .and_then(Value::as_str)
            .ok_or_else(|| bad("split.peer must be the other half's node id (a-z, 0-9 and -)"))?
            .parse::<NodeId>()
            .map_err(|e| bad(format!("split.peer: {e}")))?;
        let open_game_ports = match obj.get("openGamePorts") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(b)) => *b,
            Some(_) => return Err(bad("split.open_game_ports must be true or false")),
        };
        if open_game_ports && role == SplitRole::Metagame {
            return Err(bad(
                "split.open_game_ports is the game servers half's: the metagame half runs none",
            ));
        }
        Ok(Self {
            role,
            peer,
            open_game_ports,
        })
    }

    /// The table settings.toml writes.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut out = Map::new();
        out.insert("role".into(), Value::from(self.role.as_str()));
        out.insert("peer".into(), Value::from(self.peer.to_string()));
        if self.open_game_ports {
            out.insert("openGamePorts".into(), Value::Bool(true));
        }
        Value::Object(out)
    }

    /// The halves talk over the node link, so both are nodes of one network, which runs in public mode only.
    pub fn check(&self, mode: ServerMode, network: Option<&NetworkConfig>) -> Result<(), HostError> {
        let Some(network) = network else {
            return Err(bad(
                "[split] needs [network]: the two halves are nodes of one network and talk over its node link",
            ));
        };
        if network.node == self.peer {
            return Err(bad("split.peer is this node: it names the other half"));
        }
        if !network.nodes.iter().any(|node| node.id == self.peer) {
            return Err(bad(format!(
                "split.peer {} is not one of [[network.nodes]]: list the other half there",
                self.peer
            )));
        }
        if mode == ServerMode::Public && self.role == SplitRole::GameServers && !self.open_game_ports {
            return Err(bad(
                "a public game servers half can't guard its game ports: players' heartbeats reach the metagame half's \
                 gateway, not this one's allowlist. Set split.open_game_ports = true to run with them open",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_and_writes_back() {
        let value = json!({"role": "gameservers", "peer": "meta-1", "openGamePorts": true});
        let split = SplitConfig::from_json(&value).unwrap();
        assert_eq!(split.role, SplitRole::GameServers);
        assert_eq!(split.role.port_service(), "director");
        assert_eq!(split.to_json(), value);
        let meta = SplitConfig::from_json(&json!({"role": "metagame", "peer": "games-1"})).unwrap();
        assert_eq!(meta.to_json(), json!({"role": "metagame", "peer": "games-1"}));
    }

    #[test]
    fn refuses_what_it_does_not_know() {
        for value in [
            json!({"role": "all", "peer": "a"}),
            json!({"role": "metagame"}),
            json!({"role": "metagame", "peer": "Not A Node"}),
            json!({"role": "metagame", "peer": "a", "extra": 1}),
            json!({"role": "metagame", "peer": "a", "openGamePorts": true}),
            json!({"role": "gameservers", "peer": "a", "openGamePorts": "yes"}),
        ] {
            assert!(SplitConfig::from_json(&value).is_err(), "{value}");
        }
    }
}
