// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! A minimal HTTP/1.1 client: one request per connection, pinned TLS to public hosts, bodies bounded.

use std::fmt;
use std::future::Future;
use std::net::IpAddr;
use std::time::Duration;

use bytes::Bytes;
use dainty_protocol::invite::{is_private_mode_host, is_valid_fingerprint, is_valid_host};
use http_body_util::{BodyExt, Full};
use hyper::client::conn::http1::SendRequest;
use hyper::{Method, Request};
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;
use tokio_util::sync::CancellationToken;

use crate::net::pinned::{ConnectError, PinnedTarget, pinned_connect};

pub mod to_file;

#[cfg(test)]
mod tests;

/// A server to talk to: plain HTTP to a private-mode host, or TLS pinned to a certificate's SHA-256.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    /// Host name or IPv4 address, as `dainty_protocol::invite::is_valid_host` takes it.
    pub host: String,
    /// TCP port, never 0.
    pub port: u16,
    /// The certificate's SHA-256 fingerprint, lowercase hex; any host but Tailscale's or loopback must have one.
    pub pin: Option<String>,
}

/// How one request is sent and bounded.
#[derive(Clone)]
pub struct RequestOptions {
    /// HTTP method.
    pub method: Method,
    /// Extra headers, such as the account key's. One named like a default (`accept`, `host`) replaces it.
    pub headers: Vec<(&'static str, String)>,
    /// JSON body, sent with its content type and length.
    pub body: Option<String>,
    /// Deadline for the whole request, connect included.
    pub timeout: Duration,
    /// Largest body accepted before [`HttpError::TooLarge`].
    pub max_bytes: usize,
    /// Ends the request with [`HttpError::Aborted`] when triggered, or before it starts if it already was.
    pub cancel: Option<CancellationToken>,
}

impl RequestOptions {
    /// A GET with no extra headers, body or cancel token.
    pub fn get(timeout: Duration, max_bytes: usize) -> Self {
        Self {
            method: Method::GET,
            headers: Vec::new(),
            body: None,
            timeout,
            max_bytes,
            cancel: None,
        }
    }
}

// Header values carry the account key
impl fmt::Debug for RequestOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RequestOptions")
            .field("method", &self.method)
            .field("headers", &header_names(&self.headers))
            .field("body_bytes", &self.body.as_ref().map(String::len))
            .field("timeout", &self.timeout)
            .field("max_bytes", &self.max_bytes)
            .field("cancel", &self.cancel.is_some())
            .finish()
    }
}

pub(crate) fn header_names(headers: &[(&'static str, String)]) -> Vec<&'static str> {
    headers.iter().map(|(name, _)| *name).collect()
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
        /// The fingerprint the server presented, lowercase hex SHA-256.
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
            Self::Pin { actual } => write!(f, "certificate pin mismatch: the server presented {actual}"),
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
            ConnectError::Pin { actual, .. } => Self::Pin { actual },
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

/// The endpoint's pin, or none for a private-mode host. Fails on port 0, a bad host, a bad pin, or a host
/// without a pin that is neither Tailscale's nor loopback: the key and the player's traffic never go out in clear.
pub fn check_endpoint(endpoint: &Endpoint) -> Result<Option<String>, HttpError> {
    if endpoint.port == 0 {
        return Err(HttpError::Io("port cannot be 0".to_owned()));
    }
    if !is_valid_host(&endpoint.host) {
        return Err(HttpError::Io("invalid host".to_owned()));
    }
    match &endpoint.pin {
        Some(pin) if is_valid_fingerprint(pin) => Ok(Some(pin.clone())),
        Some(_) => Err(HttpError::Io("invalid certificate pin".to_owned())),
        None if is_private_mode_host(&endpoint.host) => Ok(None),
        None => Err(HttpError::Io("public endpoint requires a certificate pin".to_owned())),
    }
}

/// [`check_endpoint`] for a caller that needs only the verdict.
pub fn validate_endpoint(endpoint: &Endpoint) -> Result<(), HttpError> {
    check_endpoint(endpoint).map(|_| ())
}

/// The endpoint as a pinned TLS target, when it has a pin.
pub fn pinned_target(endpoint: &Endpoint) -> Option<PinnedTarget> {
    endpoint.pin.as_ref().map(|pin| PinnedTarget {
        host: endpoint.host.clone(),
        port: endpoint.port,
        fingerprint: pin.clone(),
    })
}

/// The `Host` header: the bare host only for pinned TLS on 443, else `<host>:<port>`.
pub fn host_header(endpoint: &Endpoint) -> String {
    if endpoint.pin.is_some() && endpoint.port == 443 {
        endpoint.host.clone()
    } else {
        format!("{}:{}", endpoint.host, endpoint.port)
    }
}

pub(crate) type Sender = SendRequest<Full<Bytes>>;

/// A fresh HTTP/1.1 connection: pinned TLS when `pin` is set, else plain TCP with no delay.
pub(crate) async fn open(endpoint: &Endpoint, pin: Option<String>, timeout: Duration) -> Result<Sender, HttpError> {
    match pin {
        Some(fingerprint) => {
            let target = PinnedTarget {
                host: endpoint.host.clone(),
                port: endpoint.port,
                fingerprint,
            };
            let tls = pinned_connect(&target, timeout).await?;
            handshake(TokioIo::new(tls)).await
        }
        None => {
            let tcp = TcpStream::connect((endpoint.host.as_str(), endpoint.port))
                .await
                .map_err(|e| HttpError::Io(e.to_string()))?;
            let _ = tcp.set_nodelay(true);
            handshake(TokioIo::new(tcp)).await
        }
    }
}

async fn handshake<T>(io: T) -> Result<Sender, HttpError>
where
    T: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
{
    let (sender, conn) = hyper::client::conn::http1::handshake(io)
        .await
        .map_err(|e| HttpError::Protocol(e.to_string()))?;
    tokio::spawn(async move {
        let _ = conn.await;
    });
    Ok(sender)
}

/// The request with the default headers in order (`accept`, `host`), the caller's, then the body's type and length.
pub(crate) fn build_request(
    endpoint: &Endpoint,
    method: Method,
    path: &str,
    accept: &str,
    headers: &[(&'static str, String)],
    body: Option<&str>,
) -> Result<Request<Full<Bytes>>, HttpError> {
    if !path.starts_with('/') {
        return Err(HttpError::Protocol("request path must start with '/'".to_owned()));
    }
    let overridden = |name: &str| headers.iter().any(|(own, _)| own.eq_ignore_ascii_case(name));
    let mut builder = Request::builder().method(method).uri(path);
    if !overridden("accept") {
        builder = builder.header("accept", accept);
    }
    if !overridden("host") {
        builder = builder.header("host", host_header(endpoint));
    }
    for (name, value) in headers {
        builder = builder.header(*name, value.as_str());
    }
    let body = match body {
        Some(text) => {
            builder = builder
                .header("content-type", "application/json")
                .header("content-length", text.len().to_string());
            Bytes::copy_from_slice(text.as_bytes())
        }
        None => Bytes::new(),
    };
    builder
        .body(Full::new(body))
        .map_err(|e| HttpError::Protocol(e.to_string()))
}

/// `fut` within `timeout`, ended early by `cancel`.
pub(crate) async fn bounded<T>(
    fut: impl Future<Output = Result<T, HttpError>>,
    timeout: Duration,
    cancel: Option<&CancellationToken>,
) -> Result<T, HttpError> {
    let timed = async {
        tokio::time::timeout(timeout, fut)
            .await
            .map_err(|_| HttpError::Timeout)?
    };
    match cancel {
        Some(token) => tokio::select! {
            biased;
            () = token.cancelled() => Err(HttpError::Aborted),
            result = timed => result,
        },
        None => timed.await,
    }
}

/// Sends one request over a fresh connection and reads the body, bounded by `options`. Redirects are answered,
/// never followed.
pub async fn request(endpoint: &Endpoint, path: &str, options: RequestOptions) -> Result<HttpResponse, HttpError> {
    let pin = check_endpoint(endpoint)?;
    let req = build_request(
        endpoint,
        options.method.clone(),
        path,
        "application/json",
        &options.headers,
        options.body.as_deref(),
    )?;
    if options.cancel.as_ref().is_some_and(CancellationToken::is_cancelled) {
        return Err(HttpError::Aborted);
    }

    let exchange = async {
        let mut sender = open(endpoint, pin, options.timeout).await?;
        let res = sender
            .send_request(req)
            .await
            .map_err(|e| HttpError::Protocol(e.to_string()))?;
        let status = res.status().as_u16();
        let mut body = res.into_body();
        let mut collected = Vec::new();
        while let Some(frame) = body.frame().await {
            let frame = frame.map_err(|e| HttpError::Protocol(e.to_string()))?;
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
    bounded(exchange, options.timeout, options.cancel.as_ref()).await
}

/// The body as JSON, or none when it is empty or not JSON. Invalid UTF-8 is replaced before parsing.
pub fn parse_json_body(response: &HttpResponse) -> Option<serde_json::Value> {
    // An empty body never parses, so the content type cannot change the answer and is not consulted
    if response.body.is_empty() {
        return None;
    }
    serde_json::from_str(&String::from_utf8_lossy(&response.body)).ok()
}
