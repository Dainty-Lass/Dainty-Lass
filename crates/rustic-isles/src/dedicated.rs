// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! The dedicated-server seam (core spec §5.6): matchmaking's DeployServer client, which the Director's service
//! fills when it runs in-process.

use dainty_module_api::BoxFuture;
use dainty_module_api::game::GameServer;
use serde_json::{Map, Value};

/// A Director launch (§5.6).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LaunchRequest {
    /// The client's game mode, such as `ISLAND` or `CITY`.
    pub game_mode: Option<String>,
    /// The client's travel arguments.
    pub game_args: Option<String>,
    /// The hunt to start.
    pub hunt_id: String,
    /// User ids
    pub expected_players: Vec<String>,
    /// The Gauntlet level the client chose (`gauntlet_level`: season x 10 000 000 + level)
    pub gauntlet_level: Option<i64>,
    /// Sent as `x-rustic-isles-request-id`
    pub request_id: Option<String>,
}

impl LaunchRequest {
    /// `{"GameMode","GameArgs","HuntId","ExpectedPlayers"}`, absent options left out.
    pub fn body(&self) -> Value {
        let mut body = Map::new();
        if let Some(mode) = &self.game_mode {
            body.insert("GameMode".into(), Value::String(mode.clone()));
        }
        if let Some(args) = &self.game_args {
            body.insert("GameArgs".into(), Value::String(args.clone()));
        }
        body.insert("HuntId".into(), Value::String(self.hunt_id.clone()));
        let players = self.expected_players.iter().cloned().map(Value::String).collect();
        body.insert("ExpectedPlayers".into(), Value::Array(players));
        if let Some(level) = self.gauntlet_level {
            body.insert("GauntletLevel".into(), Value::from(level));
        }
        Value::Object(body)
    }
}

/// The Director's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    /// The port as the Director wrote it (a caller maps one outside u16)
    Online {
        /// The server's address.
        host: String,
        /// The server's port.
        port: i64,
    },
    /// A reason only when nothing was asked
    Refused {
        /// The Director's reason.
        reason: Option<String>,
    },
}

/// One persistent hub as the Director reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveHub {
    /// Whether it reported ready.
    pub ready: bool,
    /// Its address.
    pub host: String,
    /// Its port, as the Director wrote it.
    pub port: i64,
}

/// Ramsgate's and the Training Dojo's hubs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveHubStatus {
    /// The Ramsgate hub, none when it is not running.
    pub ramsgate: Option<LiveHub>,
    /// The Training Dojo hub, none when it is not running.
    pub training: Option<LiveHub>,
}

/// The Director as matchmaking asks it (§5.6): in-process, over `DEPLOYSERVER_URL`, or none.
pub trait DedicatedServers: Send + Sync {
    /// Whether a Director can be asked.
    fn configured(&self) -> bool;
    /// Launches a game server; `Err` is a transport or Director failure text.
    fn launch(&self, request: LaunchRequest) -> BoxFuture<'_, Result<Launch, String>>;
    /// None when unreachable.
    fn hub_status(&self) -> BoxFuture<'_, Option<LiveHubStatus>>;
    /// Every ready game server (MS3); none when this slot can't list them, as over `DEPLOYSERVER_URL`
    fn servers(&self) -> BoxFuture<'_, Option<Vec<GameServer>>> {
        Box::pin(async { None })
    }
    /// Asks every game server it runs to send the player back to the menu (spec-runtime-kick); true once every
    /// one has acted on it (or there was none), false when that can't be confirmed (a slot that can't reach its
    /// servers, or one that didn't act in time), so the kick lifts at the end of its window instead.
    fn kick<'a>(&'a self, _user_id: &'a str) -> BoxFuture<'a, bool> {
        Box::pin(async { false })
    }
}

/// The default slot: no Director to ask.
#[derive(Debug, Clone, Copy, Default)]
pub struct Unconfigured;

impl DedicatedServers for Unconfigured {
    fn configured(&self) -> bool {
        false
    }

    fn launch(&self, _: LaunchRequest) -> BoxFuture<'_, Result<Launch, String>> {
        Box::pin(async { Ok(Launch::Refused { reason: None }) })
    }

    fn hub_status(&self) -> BoxFuture<'_, Option<LiveHubStatus>> {
        Box::pin(async { None })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_body_order() {
        let mut request = LaunchRequest {
            hunt_id: "h".into(),
            expected_players: vec!["u1".into()],
            ..LaunchRequest::default()
        };
        assert_eq!(
            dainty_service_kit::jsjson::stringify(&request.body()),
            r#"{"HuntId":"h","ExpectedPlayers":["u1"]}"#
        );
        request.game_mode = Some("ISLAND".into());
        request.game_args = Some(String::new());
        assert_eq!(
            dainty_service_kit::jsjson::stringify(&request.body()),
            r#"{"GameMode":"ISLAND","GameArgs":"","HuntId":"h","ExpectedPlayers":["u1"]}"#
        );
        request.gauntlet_level = Some(10_000_001);
        assert!(
            dainty_service_kit::jsjson::stringify(&request.body())
                .ends_with(r#""ExpectedPlayers":["u1"],"GauntletLevel":10000001}"#)
        );
    }
}
