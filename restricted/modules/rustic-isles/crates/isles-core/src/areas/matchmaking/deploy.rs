//! The seam to the Director (Node's DeployServer): what matchmaking asks of it. The Director's side of
//! `POST /api/matchmaker/handle-matchmaking-for-player` is RU6's; this is the client Node's matchmaker is, over
//! plain HTTP to `DEPLOYSERVER_URL` (`host:port`), so a Director in the same process or another answers alike.

use std::time::Duration;

use module_api::BoxFuture;
use serde_json::{Map, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::settings::Settings;

pub const DEPLOYSERVER_MATCHMAKING_PATH: &str = "/api/matchmaker/handle-matchmaking-for-player";
const HUB_STATUS_PATH: &str = "/api/hub-status";
/// Longer than any launch takes; Node's `fetch` waits five minutes for headers
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

/// `{ GameMode, GameArgs, HuntId, ExpectedPlayers }`; a field that is `None` is left out, as `JSON.stringify` leaves
/// out `undefined`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchRequest {
    pub game_mode: Option<String>,
    pub game_args: Option<String>,
    pub hunt_id: String,
    pub expected_players: Vec<String>,
    /// Forwarded as `x-rustic-isles-request-id`
    pub request_id: Option<String>,
}

impl LaunchRequest {
    pub fn body(&self) -> String {
        let mut body = Map::new();
        if let Some(mode) = &self.game_mode {
            body.insert("GameMode".into(), Value::String(mode.clone()));
        }
        if let Some(args) = &self.game_args {
            body.insert("GameArgs".into(), Value::String(args.clone()));
        }
        body.insert("HuntId".into(), Value::String(self.hunt_id.clone()));
        body.insert(
            "ExpectedPlayers".into(),
            Value::Array(self.expected_players.iter().cloned().map(Value::String).collect()),
        );
        Value::Object(body).to_string()
    }
}

/// `DeployServerLaunch`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    Online {
        host: String,
        port: i64,
    },
    /// `succeeded: false`. `reason` is set when no launch was requested because no DeployServer is configured.
    Refused {
        reason: Option<String>,
    },
}

/// `LiveHubStatus`'s hub: `{ ready, host, port, processId }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveHub {
    pub ready: bool,
    pub host: String,
    pub port: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LiveHubStatus {
    pub ramsgate: Option<LiveHub>,
    pub training_dojo: Option<LiveHub>,
}

pub trait DedicatedServers: Send + Sync {
    /// `DedicatedServersConfigured`: `DEPLOYSERVER_URL` holds something other than blank.
    fn configured(&self) -> bool;

    /// `LaunchGameOnDeployserver` past its configuration and hunt-policy checks. `Err` is a fetch that threw, which
    /// fails the request that asked (Express's 500).
    fn launch(&self, request: LaunchRequest) -> BoxFuture<'_, Result<Launch, String>>;

    /// `FetchLiveHubStatus`: `None` when the Director cannot be reached.
    fn hub_status(&self) -> BoxFuture<'_, Option<LiveHubStatus>>;
}

/// No DeployServer (`DEPLOYSERVER_URL` unset, as on a host whose hunts are all player-hosted): nothing is requested.
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

/// `http://<DEPLOYSERVER_URL><path>` with `fetch`'s behaviour: the answer's status and body, or an error.
#[derive(Debug, Clone)]
pub struct HttpDeployServer {
    authority: String,
}

impl HttpDeployServer {
    pub fn from_settings(settings: &Settings) -> std::sync::Arc<dyn DedicatedServers> {
        match settings.raw("DEPLOYSERVER_URL") {
            Some(url) if !url.trim().is_empty() => std::sync::Arc::new(Self {
                authority: url.to_owned(),
            }),
            _ => std::sync::Arc::new(Unconfigured),
        }
    }

    async fn send(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, String)],
        body: Option<String>,
    ) -> Result<(u16, Vec<u8>), String> {
        let work = async {
            let target = if self.authority.contains(':') {
                self.authority.clone()
            } else {
                format!("{}:80", self.authority)
            };
            let mut stream = TcpStream::connect(&target)
                .await
                .map_err(|e| format!("fetch failed: {e}"))?;
            let mut head = format!(
                "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n",
                self.authority
            );
            for (name, value) in headers {
                head.push_str(&format!("{name}: {value}\r\n"));
            }
            match &body {
                Some(body) => head.push_str(&format!("Content-Length: {}\r\n\r\n{body}", body.len())),
                None => head.push_str("\r\n"),
            }
            stream
                .write_all(head.as_bytes())
                .await
                .map_err(|e| format!("fetch failed: {e}"))?;
            let mut raw = Vec::new();
            stream
                .read_to_end(&mut raw)
                .await
                .map_err(|e| format!("fetch failed: {e}"))?;
            parse_response(&raw)
        };
        tokio::time::timeout(REQUEST_TIMEOUT, work)
            .await
            .map_err(|_| "fetch failed: timed out".to_owned())?
    }
}

/// The status and the (de-chunked) body of an HTTP/1.1 response read to the end of the connection.
fn parse_response(raw: &[u8]) -> Result<(u16, Vec<u8>), String> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| "fetch failed: a truncated answer".to_owned())?;
    let head = String::from_utf8_lossy(&raw[..split]);
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| "fetch failed: no status line".to_owned())?;
    let chunked = lines.any(|line| {
        let line = line.to_ascii_lowercase();
        line.starts_with("transfer-encoding:") && line.contains("chunked")
    });
    let body = &raw[split + 4..];
    if !chunked {
        return Ok((status, body.to_vec()));
    }
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(end) = rest.windows(2).position(|w| w == b"\r\n") {
        let size_text = String::from_utf8_lossy(&rest[..end]);
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16)
            .map_err(|_| "fetch failed: a bad chunk".to_owned())?;
        rest = &rest[end + 2..];
        if size == 0 || rest.len() < size {
            break;
        }
        out.extend_from_slice(&rest[..size]);
        rest = rest.get(size + 2..).unwrap_or_default();
    }
    Ok((status, out))
}

fn hub(value: Option<&Value>) -> Option<LiveHub> {
    let value = value?.as_object()?;
    Some(LiveHub {
        ready: value.get("ready").and_then(Value::as_bool).unwrap_or(false),
        host: value.get("host").and_then(Value::as_str).unwrap_or_default().to_owned(),
        port: value.get("port").and_then(Value::as_i64).unwrap_or(0),
    })
}

impl DedicatedServers for HttpDeployServer {
    fn configured(&self) -> bool {
        true
    }

    fn launch(&self, request: LaunchRequest) -> BoxFuture<'_, Result<Launch, String>> {
        Box::pin(async move {
            let mut headers = vec![("Content-Type", "application/json".to_owned())];
            if let Some(id) = &request.request_id {
                headers.push((crate::http::REQUEST_ID_HEADER, id.clone()));
            }
            let (status, body) = self
                .send("POST", DEPLOYSERVER_MATCHMAKING_PATH, &headers, Some(request.body()))
                .await?;
            if status != 200 {
                tracing::error!("DeployServer returned status {status}");
                return Ok(Launch::Refused { reason: None });
            }
            let data: Value = serde_json::from_slice(&body).map_err(|e| format!("invalid DeployServer JSON: {e}"))?;
            let host = data.get("host").and_then(Value::as_str).unwrap_or_default().to_owned();
            let port = data.get("port").and_then(Value::as_i64).unwrap_or(0);
            tracing::info!("DeployServer returned gameserver {host}:{port}");
            Ok(Launch::Online { host, port })
        })
    }

    fn hub_status(&self) -> BoxFuture<'_, Option<LiveHubStatus>> {
        Box::pin(async move {
            match self.send("GET", HUB_STATUS_PATH, &[], None).await {
                Ok((200, body)) => match serde_json::from_slice::<Value>(&body) {
                    Ok(data) => Some(LiveHubStatus {
                        ramsgate: hub(data.get("ramsgate")),
                        training_dojo: hub(data.get("trainingDojo")),
                    }),
                    Err(e) => {
                        tracing::warn!("hub-status unreachable ({e}); treating cached hub candidates as unverified");
                        None
                    }
                },
                Ok((status, _)) => {
                    tracing::warn!("hub-status returned {status}; treating cached hub candidates as unverified");
                    None
                }
                Err(e) => {
                    tracing::warn!("hub-status unreachable ({e}); treating cached hub candidates as unverified");
                    None
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_launch_body_is_what_json_stringify_writes() {
        let request = LaunchRequest {
            game_mode: Some("ISLAND".into()),
            game_args: None,
            hunt_id: "ShatteredIsles_IslandA".into(),
            expected_players: vec!["a".into(), "b".into()],
            request_id: None,
        };
        assert_eq!(
            request.body(),
            r#"{"GameMode":"ISLAND","HuntId":"ShatteredIsles_IslandA","ExpectedPlayers":["a","b"]}"#
        );
    }

    #[test]
    fn answers_are_read_with_or_without_chunks() {
        let plain = b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\n{}";
        assert_eq!(parse_response(plain).unwrap(), (200, b"{}".to_vec()));
        let chunked = b"HTTP/1.1 500 X\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n";
        assert_eq!(parse_response(chunked).unwrap(), (500, b"abcde".to_vec()));
        assert!(parse_response(b"HTTP/1.1 200").is_err());
    }

    #[test]
    fn a_blank_url_is_no_deployserver() {
        assert!(!HttpDeployServer::from_settings(&Settings::default()).configured());
        assert!(!HttpDeployServer::from_settings(&Settings::from_pairs([("DEPLOYSERVER_URL", "  ")])).configured());
        assert!(
            HttpDeployServer::from_settings(&Settings::from_pairs([("DEPLOYSERVER_URL", "127.0.0.1:1")])).configured()
        );
    }
}
