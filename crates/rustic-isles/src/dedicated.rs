// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The dedicated-server seam (core spec §5.6): matchmaking's DeployServer client, which the Director's service
//! fills when it runs in-process.

use module_api::BoxFuture;
use serde_json::{Map, Value};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LaunchRequest {
    pub game_mode: Option<String>,
    pub game_args: Option<String>,
    pub hunt_id: String,
    /// User ids
    pub expected_players: Vec<String>,
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
        Value::Object(body)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    /// The port as the Director wrote it (a caller maps one outside u16)
    Online { host: String, port: i64 },
    /// A reason only when nothing was asked
    Refused { reason: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveHub {
    pub ready: bool,
    pub host: String,
    pub port: i64,
}

/// Ramsgate's and the Training Dojo's hubs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveHubStatus {
    pub ramsgate: Option<LiveHub>,
    pub training: Option<LiveHub>,
}

pub trait DedicatedServers: Send + Sync {
    /// Whether a Director can be asked.
    fn configured(&self) -> bool;
    fn launch(&self, request: LaunchRequest) -> BoxFuture<'_, Result<Launch, String>>;
    /// None when unreachable.
    fn hub_status(&self) -> BoxFuture<'_, Option<LiveHubStatus>>;
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
            service_kit::jsjson::stringify(&request.body()),
            r#"{"HuntId":"h","ExpectedPlayers":["u1"]}"#
        );
        request.game_mode = Some("ISLAND".into());
        request.game_args = Some(String::new());
        assert_eq!(
            service_kit::jsjson::stringify(&request.body()),
            r#"{"GameMode":"ISLAND","GameArgs":"","HuntId":"h","ExpectedPlayers":["u1"]}"#
        );
    }
}
