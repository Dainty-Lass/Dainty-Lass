// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The Director over HTTP (spec §10, `DEPLOYSERVER_URL`): one plain HTTP/1.1 exchange per call, for when the
//! Director does not run in-process.

use std::sync::Arc;
use std::time::Duration;

use dainty_module_api::BoxFuture;
use rustic_isles::{DedicatedServers, Launch, LaunchRequest, LiveHub, LiveHubStatus, Settings, Unconfigured};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// D4.
pub const DEPLOYSERVER_MATCHMAKING_PATH: &str = "/api/matchmaker/handle-matchmaking-for-player";
/// D2.
pub const DEPLOYSERVER_HUB_STATUS_PATH: &str = "/api/hub-status";
/// To the end of the answer.
pub const DIRECTOR_TIMEOUT: Duration = Duration::from_secs(300);

/// Node fetched `http://${DEPLOYSERVER_URL}<path>`, so the setting is an authority, not a URL.
pub struct HttpDirector {
    authority: String,
    address: String,
}

impl std::fmt::Debug for HttpDirector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpDirector")
            .field("authority", &self.authority)
            .finish()
    }
}

/// The status and the (de-chunked) body of a whole HTTP/1.1 answer.
pub fn read_answer(raw: &[u8]) -> Result<(u16, Vec<u8>), String> {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| "fetch failed: a truncated answer".to_owned())?;
    let head = String::from_utf8_lossy(&raw[..split]);
    let body = &raw[split + 4..];
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.split(' ').nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| "fetch failed: no status line".to_owned())?;
    let chunked = lines.any(|line| {
        line.split_once(':').is_some_and(|(name, value)| {
            name.trim().eq_ignore_ascii_case("transfer-encoding") && value.to_ascii_lowercase().contains("chunked")
        })
    });
    if chunked {
        return dechunk(body).map(|b| (status, b));
    }
    Ok((status, body.to_vec()))
}

/// Hex sizes with extensions after `;` ignored, ending at a 0 chunk or a short one.
fn dechunk(mut rest: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    while let Some(end) = rest.windows(2).position(|w| w == b"\r\n") {
        let line = String::from_utf8_lossy(&rest[..end]);
        let size_text = line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_text, 16).map_err(|_| "fetch failed: a bad chunk".to_owned())?;
        rest = &rest[end + 2..];
        if size == 0 || rest.len() < size {
            break;
        }
        out.extend_from_slice(&rest[..size]);
        rest = rest.get(size..).unwrap_or(&[]);
        rest = rest.strip_prefix(b"\r\n").unwrap_or(rest);
    }
    Ok(out)
}

fn hub_of(value: Option<&Value>) -> Option<LiveHub> {
    let hub = value?.as_object()?;
    Some(LiveHub {
        ready: hub.get("ready").and_then(Value::as_bool).unwrap_or(false),
        host: hub.get("host").and_then(Value::as_str).unwrap_or("").to_owned(),
        port: hub.get("port").and_then(Value::as_i64).unwrap_or(0),
    })
}

impl HttpDirector {
    /// Core's `Unconfigured` when `DEPLOYSERVER_URL` is unset or blank.
    pub fn from_settings(settings: &Settings) -> Arc<dyn DedicatedServers> {
        match settings
            .raw("DEPLOYSERVER_URL")
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            Some(authority) => Arc::new(Self::new(authority)),
            None => Arc::new(Unconfigured),
        }
    }

    /// Port 80 when the authority names none.
    pub fn new(authority: &str) -> Self {
        let address = if authority.contains(':') {
            authority.to_owned()
        } else {
            format!("{authority}:80")
        };
        Self {
            authority: authority.to_owned(),
            address,
        }
    }

    async fn exchange(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<&str>,
    ) -> Result<Vec<u8>, String> {
        let mut head = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n",
            self.authority
        );
        for (name, value) in headers {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
        if let Some(body) = body {
            head.push_str(&format!("Content-Length: {}\r\n", body.len()));
        }
        head.push_str("\r\n");
        if let Some(body) = body {
            head.push_str(body);
        }
        let io = |e: std::io::Error| format!("fetch failed: {e}");
        let mut stream = tokio::net::TcpStream::connect(&self.address).await.map_err(io)?;
        stream.write_all(head.as_bytes()).await.map_err(io)?;
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.map_err(io)?;
        Ok(raw)
    }

    /// The whole exchange within 300 s.
    async fn request(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: Option<&str>,
    ) -> Result<(u16, Vec<u8>), String> {
        let raw = tokio::time::timeout(DIRECTOR_TIMEOUT, self.exchange(method, path, headers, body))
            .await
            .map_err(|_| "fetch failed: timed out".to_owned())??;
        read_answer(&raw)
    }

    async fn launch_inner(&self, request: LaunchRequest) -> Result<Launch, String> {
        let body = dainty_service_kit::jsjson::stringify(&request.body());
        let mut headers = vec![("Content-Type", "application/json")];
        if let Some(id) = request.request_id.as_deref() {
            headers.push((rustic_isles::REQUEST_ID_HEADER, id));
        }
        let (status, answer) = self
            .request("POST", DEPLOYSERVER_MATCHMAKING_PATH, &headers, Some(&body))
            .await?;
        if status != 200 {
            tracing::error!("DeployServer returned status {status}");
            return Ok(Launch::Refused { reason: None });
        }
        let value: Value = serde_json::from_slice(&answer).map_err(|e| format!("invalid DeployServer JSON: {e}"))?;
        let host = value.get("host").and_then(Value::as_str).unwrap_or("").to_owned();
        let port = value.get("port").and_then(Value::as_i64).unwrap_or(0);
        tracing::info!("DeployServer returned gameserver {host}:{port}");
        Ok(Launch::Online { host, port })
    }

    async fn hub_status_inner(&self) -> Option<LiveHubStatus> {
        let answer = self.request("GET", DEPLOYSERVER_HUB_STATUS_PATH, &[], None).await;
        let value = match answer {
            Ok((200, body)) => serde_json::from_slice::<Value>(&body).map_err(|e| e.to_string()),
            Ok((status, _)) => {
                tracing::warn!("hub-status returned {status}; treating cached hub candidates as unverified");
                return None;
            }
            Err(e) => Err(e),
        };
        match value {
            Ok(value) => Some(LiveHubStatus {
                ramsgate: hub_of(value.get("ramsgate")),
                training: hub_of(value.get("trainingDojo")),
            }),
            Err(e) => {
                tracing::warn!("hub-status unreachable ({e}); treating cached hub candidates as unverified");
                None
            }
        }
    }
}

impl DedicatedServers for HttpDirector {
    fn configured(&self) -> bool {
        true
    }

    fn launch(&self, request: LaunchRequest) -> BoxFuture<'_, Result<Launch, String>> {
        Box::pin(self.launch_inner(request))
    }

    fn hub_status(&self) -> BoxFuture<'_, Option<LiveHubStatus>> {
        Box::pin(self.hub_status_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers() {
        assert_eq!(
            read_answer(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}"),
            Ok((200, b"{}".to_vec()))
        );
        let chunked = b"HTTP/1.1 500 Oops\r\nTransfer-Encoding: Chunked\r\n\r\n3;x=1\r\nabc\r\n2\r\nde\r\n0\r\n\r\n";
        assert_eq!(read_answer(chunked), Ok((500, b"abcde".to_vec())));
        assert_eq!(
            read_answer(b"HTTP/1.1 200 OK\r\n"),
            Err("fetch failed: a truncated answer".into())
        );
        assert_eq!(
            read_answer(b"HTTP/1.1\r\n\r\n"),
            Err("fetch failed: no status line".into())
        );
        let bad = b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\nzz\r\n";
        assert_eq!(read_answer(bad), Err("fetch failed: a bad chunk".into()));
        let short = b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n2\r\nab\r\n9\r\nabc";
        assert_eq!(read_answer(short), Ok((200, b"ab".to_vec())));
    }

    #[test]
    fn urls() {
        let configured = |pairs: &[(&str, &str)]| {
            HttpDirector::from_settings(&Settings::from_pairs(pairs.iter().copied())).configured()
        };
        assert!(!configured(&[]));
        assert!(!configured(&[("DEPLOYSERVER_URL", "  ")]));
        assert!(configured(&[("DEPLOYSERVER_URL", "127.0.0.1:1")]));
        let director = HttpDirector::new("director.local");
        assert_eq!(director.address, "director.local:80");
        assert_eq!(
            format!("{director:?}"),
            "HttpDirector { authority: \"director.local\" }"
        );
    }
}
