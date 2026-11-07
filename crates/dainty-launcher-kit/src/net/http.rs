// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! A minimal HTTP/1.1 client: one request per connection, pinned TLS to public hosts, bodies bounded.

use std::fmt;
use std::net::IpAddr;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Method, Request};
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;

use crate::core::pins::normalize_pin;
use crate::net::pinned::{ConnectError, PinnedTarget, pinned_connect, validate_host};

pub mod to_file;

/// A server to talk to: plain HTTP to a private or loopback host, or TLS pinned to a certificate's SHA-256.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// Host name or IP address, IPv6 with or without brackets.
    pub host: String,
    /// TCP port, never 0.
    pub port: u16,
    /// The certificate's SHA-256 fingerprint; a public host must have one.
    pub pin: Option<String>,
}

/// How one request is sent and bounded.
#[derive(Debug, Clone)]
pub struct RequestOptions {
    /// HTTP method.
    pub method: Method,
    /// Extra headers, such as the account key's.
    pub headers: Vec<(&'static str, String)>,
    /// JSON body, sent with its content type and length.
    pub body: Option<String>,
    /// Deadline for the whole request, connect included.
    pub timeout: Duration,
    /// Largest body accepted before [`HttpError::TooLarge`].
    pub max_bytes: usize,
}

impl RequestOptions {
    /// A GET with no extra headers or body.
    pub fn get(timeout: Duration, max_bytes: usize) -> Self {
        Self {
            method: Method::GET,
            headers: Vec::new(),
            body: None,
            timeout,
            max_bytes,
        }
    }
}

/// A response with its body read in full.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    /// HTTP status code.
    pub status: u16,
    /// Response body.
    pub body: Bytes,
}

/// Why a request failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpError {
    /// The server's certificate does not match the pin.
    Pin {
        /// The pinned fingerprint, lowercase hex SHA-256.
        expected: String,
        /// The fingerprint the server presented.
        actual: String,
    },
    /// The deadline passed.
    Timeout,
    /// Connecting or reading failed, or the endpoint was refused before connecting.
    Io(String),
    /// The HTTP exchange itself failed.
    Protocol(String),
    /// The body passed `max_bytes`.
    TooLarge,
    /// The caller cancelled.
    Aborted,
}

impl fmt::Display for HttpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pin { expected, actual } => {
                write!(f, "certificate pin mismatch: expected {expected}, got {actual}")
            }
            Self::Timeout => write!(f, "HTTP request timed out"),
            Self::Io(msg) => write!(f, "HTTP I/O error: {msg}"),
            Self::Protocol(msg) => write!(f, "HTTP protocol error: {msg}"),
            Self::TooLarge => write!(f, "HTTP response exceeded maximum size"),
            Self::Aborted => write!(f, "HTTP request aborted"),
        }
    }
}

impl std::error::Error for HttpError {}

impl From<ConnectError> for HttpError {
    fn from(err: ConnectError) -> Self {
        match err {
            ConnectError::Pin { expected, actual } => Self::Pin { expected, actual },
            ConnectError::Timeout => Self::Timeout,
            ConnectError::Io(msg) => Self::Io(msg),
        }
    }
}

/// `host` is localhost or a loopback, private, link-local, CGNAT (Tailscale) or unique-local address.
pub fn is_private_or_loopback(host: &str) -> bool {
    let clean = host
        .trim()
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or_else(|| host.trim());
    if clean.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match clean.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => {
            let octets = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || (octets[0] == 100 && (64..=127).contains(&octets[1]))
        }
        Ok(IpAddr::V6(v6)) => v6.is_loopback() || v6.is_unique_local(),
        Err(_) => false,
    }
}

/// The endpoint's normalized pin, or none for a private or loopback host.
/// Fails on port 0, a bad host, a bad pin, or a public host without one.
pub fn check_endpoint(endpoint: &Endpoint) -> Result<Option<String>, HttpError> {
    if endpoint.port == 0 {
        return Err(HttpError::Io("port cannot be 0".to_owned()));
    }
    let host = validate_host(&endpoint.host).map_err(HttpError::from)?;
    match &endpoint.pin {
        Some(raw) => {
            let normalized = normalize_pin(raw).ok_or_else(|| HttpError::Io("invalid certificate pin".to_owned()))?;
            Ok(Some(normalized))
        }
        None => {
            if is_private_or_loopback(host) {
                Ok(None)
            } else {
                Err(HttpError::Io("public endpoint requires a certificate pin".to_owned()))
            }
        }
    }
}

/// The `Host` header value, with the port only when it is not the scheme's default.
pub fn host_header(endpoint: &Endpoint) -> String {
    let clean = endpoint
        .host
        .trim()
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or_else(|| endpoint.host.trim());
    let default_port = if endpoint.pin.is_some() { 443 } else { 80 };
    if clean.contains(':') {
        if endpoint.port == default_port {
            format!("[{clean}]")
        } else {
            format!("[{clean}]:{}", endpoint.port)
        }
    } else if endpoint.port == default_port {
        clean.to_owned()
    } else {
        format!("{clean}:{}", endpoint.port)
    }
}

/// Sends one request over a fresh connection and reads the body, bounded by `options`.
pub async fn request(endpoint: &Endpoint, path: &str, options: RequestOptions) -> Result<HttpResponse, HttpError> {
    let checked_pin = check_endpoint(endpoint)?;
    if !path.starts_with('/') {
        return Err(HttpError::Protocol("request path must start with '/'".to_owned()));
    }

    let fut = async {
        let mut sender = match checked_pin {
            Some(fingerprint) => {
                let target = PinnedTarget {
                    host: endpoint.host.clone(),
                    port: endpoint.port,
                    fingerprint,
                };
                let tls = pinned_connect(&target, options.timeout)
                    .await
                    .map_err(HttpError::from)?;
                let (sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
                    .await
                    .map_err(|e| HttpError::Protocol(e.to_string()))?;
                tokio::spawn(async move {
                    let _ = conn.await;
                });
                sender
            }
            None => {
                let host = validate_host(&endpoint.host).map_err(HttpError::from)?;
                let addr = if host.contains(':') {
                    format!("[{host}]:{}", endpoint.port)
                } else {
                    format!("{host}:{}", endpoint.port)
                };
                let tcp = TcpStream::connect(&addr)
                    .await
                    .map_err(|e| HttpError::Io(e.to_string()))?;
                let _ = tcp.set_nodelay(true);
                let (sender, conn) = hyper::client::conn::http1::handshake(TokioIo::new(tcp))
                    .await
                    .map_err(|e| HttpError::Protocol(e.to_string()))?;
                tokio::spawn(async move {
                    let _ = conn.await;
                });
                sender
            }
        };

        let body_bytes = options
            .body
            .as_ref()
            .map(|s| Bytes::copy_from_slice(s.as_bytes()))
            .unwrap_or_default();

        let mut builder = Request::builder()
            .method(options.method)
            .uri(path)
            .header("host", host_header(endpoint))
            .header("connection", "close");

        if !body_bytes.is_empty() {
            builder = builder
                .header("content-length", body_bytes.len().to_string())
                .header("content-type", "application/json");
        }

        for (name, value) in &options.headers {
            builder = builder.header(*name, value.as_str());
        }

        let req = builder
            .body(Full::new(body_bytes))
            .map_err(|e| HttpError::Protocol(e.to_string()))?;

        let res = sender
            .send_request(req)
            .await
            .map_err(|e| HttpError::Protocol(e.to_string()))?;

        let status = res.status().as_u16();
        let mut body_stream = res.into_body();
        let mut collected = Vec::new();

        while let Some(frame_res) = body_stream.frame().await {
            let frame = frame_res.map_err(|e| HttpError::Protocol(e.to_string()))?;
            if let Some(chunk) = frame.data_ref() {
                if collected.len().saturating_add(chunk.len()) > options.max_bytes {
                    return Err(HttpError::TooLarge);
                }
                collected.extend_from_slice(chunk);
            }
        }

        Ok(HttpResponse {
            status,
            body: Bytes::from(collected),
        })
    };

    tokio::time::timeout(options.timeout, fut)
        .await
        .map_err(|_| HttpError::Timeout)?
}

/// The body as JSON, or none when it is not.
pub fn parse_json_body(response: &HttpResponse) -> Option<serde_json::Value> {
    serde_json::from_slice(&response.body).ok()
}
