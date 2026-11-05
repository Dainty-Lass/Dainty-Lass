// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! HTTP/1.1 over plain TCP, one connection per request (spec §6.4), and the in-process router endpoint.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::Duration;

use axum::Router;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::ReplayError;
use crate::request::{HOP_HEADERS, PreparedHttpRequest, ReplayHttpResponse};

/// `host:port` of a base URL: `http://` removed, the path dropped, port 80 when none is given.
fn address(base_url: &str) -> (String, String) {
    let trimmed = base_url.trim();
    let rest = trimmed.strip_prefix("http://").unwrap_or(trimmed);
    let authority = rest.split('/').next().unwrap_or_default().to_owned();
    let connect = if authority.contains(':') {
        authority.clone()
    } else {
        format!("{authority}:80")
    };
    (authority, connect)
}

fn request_bytes(host: &str, request: &PreparedHttpRequest) -> Vec<u8> {
    let path = if request.target.starts_with('/') {
        request.target.as_str()
    } else {
        "/"
    };
    let mut head = format!(
        "{} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nContent-Length: {}\r\n",
        request.method,
        request.body.len()
    );
    for (name, value) in &request.headers {
        if HOP_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
            continue;
        }
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    let mut bytes = head.into_bytes();
    bytes.extend_from_slice(&request.body);
    bytes
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn dechunk(mut body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(line_end) = find(body, b"\r\n") {
        let line = String::from_utf8_lossy(&body[..line_end]);
        let size_text = line.split(';').next().unwrap_or_default().trim();
        let Ok(size) = usize::from_str_radix(size_text, 16) else {
            break;
        };
        if size == 0 {
            break;
        }
        let data = &body[line_end + 2..];
        if data.len() < size {
            break;
        }
        out.extend_from_slice(&data[..size]);
        body = data.get(size + 2..).unwrap_or_default();
    }
    out
}

/// A raw answer read to end of stream: head, status, headers (repeats joined), the body de-chunked when chunked.
pub fn parse_http_response(bytes: &[u8]) -> Result<ReplayHttpResponse, String> {
    let head_end =
        find(bytes, b"\r\n\r\n").ok_or_else(|| "malformed HTTP response (missing header terminator)".to_owned())?;
    let head = String::from_utf8_lossy(&bytes[..head_end]);
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| format!("invalid HTTP status line: \"{status_line}\""))?;
    let mut headers: BTreeMap<String, String> = BTreeMap::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim().to_owned();
        headers
            .entry(name.trim().to_ascii_lowercase())
            .and_modify(|v| {
                v.push_str(", ");
                v.push_str(&value);
            })
            .or_insert(value);
    }
    let raw_body = &bytes[head_end + 4..];
    let chunked = headers
        .get("transfer-encoding")
        .is_some_and(|te| te.to_ascii_lowercase().contains("chunked"));
    let body = if chunked { dechunk(raw_body) } else { raw_body.to_vec() };
    Ok(ReplayHttpResponse {
        status,
        headers,
        body_text: String::from_utf8_lossy(&body).into_owned(),
    })
}

/// Sends one prepared request to `base_url` (plain HTTP only; `https://` fails to connect).
pub async fn send_http_request(
    base_url: &str,
    request: &PreparedHttpRequest,
) -> Result<ReplayHttpResponse, ReplayError> {
    let fail = |message: String| ReplayError::Transport {
        target: format!("{base_url}{}", request.target),
        message,
    };
    let (host, connect) = address(base_url);
    let mut stream = TcpStream::connect(connect.as_str())
        .await
        .map_err(|e| fail(e.to_string()))?;
    stream
        .write_all(&request_bytes(&host, request))
        .await
        .map_err(|e| fail(e.to_string()))?;
    let mut answer = Vec::new();
    stream.read_to_end(&mut answer).await.map_err(|e| fail(e.to_string()))?;
    parse_http_response(&answer).map_err(fail)
}

/// A router served on a fresh loopback listener until [`EphemeralServer::stop`].
pub struct EphemeralServer {
    pub addr: SocketAddr,
    stop: Option<oneshot::Sender<()>>,
    task: JoinHandle<std::io::Result<()>>,
}

impl EphemeralServer {
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub async fn stop(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        let _ = (&mut self.task).await;
    }
}

/// Serves `router` on `127.0.0.1:0` with a 50 ms grace on stop.
pub async fn serve_ephemeral(router: Router) -> std::io::Result<EphemeralServer> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let (stop, stopped) = oneshot::channel::<()>();
    let task = tokio::spawn(service_kit::serve::serve(
        listener,
        router,
        async move {
            let _ = stopped.await;
        },
        Duration::from_millis(50),
    ));
    Ok(EphemeralServer {
        addr,
        stop: Some(stop),
        task,
    })
}

/// One request to `router` through a listener made for it.
pub async fn dispatch_router_request(
    router: &Router,
    request: &PreparedHttpRequest,
) -> Result<ReplayHttpResponse, ReplayError> {
    let server = serve_ephemeral(router.clone())
        .await
        .map_err(|e| ReplayError::Transport {
            target: format!("router{}", request.target),
            message: e.to_string(),
        })?;
    let answer = send_http_request(&server.base_url(), request).await;
    server.stop().await;
    answer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u5_chunked_body() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n";
        let answer = parse_http_response(raw).unwrap();
        assert_eq!(answer.status, 200);
        assert_eq!(answer.body_text, "hello");
    }

    #[test]
    fn heads_and_status_lines() {
        assert_eq!(
            parse_http_response(b"HTTP/1.1 200 OK\r\n").unwrap_err(),
            "malformed HTTP response (missing header terminator)"
        );
        assert_eq!(
            parse_http_response(b"HTTP/1.1 abc\r\n\r\n").unwrap_err(),
            "invalid HTTP status line: \"HTTP/1.1 abc\""
        );
        let raw = b"HTTP/1.1 404 Not Found\r\nX-A: 1\r\nx-a:  2 \r\nContent-Length: 99\r\n\r\nbody";
        let answer = parse_http_response(raw).unwrap();
        assert_eq!(answer.headers["x-a"], "1, 2");
        assert_eq!(answer.body_text, "body");
        let short = b"HTTP/1.1 200 OK\r\ntransfer-encoding: gzip, chunked\r\n\r\n3;x=1\r\nabc\r\nff\r\nshort";
        assert_eq!(parse_http_response(short).unwrap().body_text, "abc");
    }

    #[test]
    fn addresses() {
        assert_eq!(
            address(" http://127.0.0.1:61000/x/y "),
            ("127.0.0.1:61000".into(), "127.0.0.1:61000".into())
        );
        assert_eq!(
            address("example.test"),
            ("example.test".into(), "example.test:80".into())
        );
    }
}
