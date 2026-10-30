// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

use std::fmt;
use std::net::IpAddr;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Method, Request};
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;

use crate::core::pins::normalize_pin;
use crate::net::pinned::{pinned_connect, validate_host, ConnectError, PinnedTarget};

pub mod to_file;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
    pub pin: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RequestOptions {
    pub method: Method,
    pub headers: Vec<(&'static str, String)>,
    pub body: Option<String>,
    pub timeout: Duration,
    pub max_bytes: usize,
}

impl RequestOptions {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Bytes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpError {
    Pin { expected: String, actual: String },
    Timeout,
    Io(String),
    Protocol(String),
    TooLarge,
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

pub fn check_endpoint(endpoint: &Endpoint) -> Result<Option<String>, HttpError> {
    if endpoint.port == 0 {
        return Err(HttpError::Io("port cannot be 0".to_owned()));
    }
    let host = validate_host(&endpoint.host).map_err(HttpError::from)?;
    match &endpoint.pin {
        Some(raw) => {
            let normalized = normalize_pin(raw)
                .ok_or_else(|| HttpError::Io("invalid certificate pin".to_owned()))?;
            Ok(Some(normalized))
        }
        None => {
            if is_private_or_loopback(host) {
                Ok(None)
            } else {
                Err(HttpError::Io(
                    "public endpoint requires a certificate pin".to_owned(),
                ))
            }
        }
    }
}

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

pub async fn request(
    endpoint: &Endpoint,
    path: &str,
    options: RequestOptions,
) -> Result<HttpResponse, HttpError> {
    let checked_pin = check_endpoint(endpoint)?;
    if !path.starts_with('/') {
        return Err(HttpError::Protocol(
            "request path must start with '/'".to_owned(),
        ));
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

pub fn parse_json_body(response: &HttpResponse) -> Option<serde_json::Value> {
    serde_json::from_slice(&response.body).ok()
}
