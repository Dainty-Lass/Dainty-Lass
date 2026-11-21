// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! A minimal HTTP/1.1 client: one request per connection, pinned TLS to public hosts, bodies bounded.

use std::fmt;
use std::future::Future;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::client::conn::http1::SendRequest;
use hyper::{Method, Request};
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;
use tokio_util::sync::CancellationToken;

pub use dainty_relay::endpoint::{
    Endpoint, FailureKind, HttpError, check_endpoint, host_header, is_private_or_loopback, pinned_target,
    validate_endpoint,
};

use crate::net::pinned::{PinnedTarget, pinned_connect, tcp_connect};
use crate::platform::log;

pub mod to_file;

#[cfg(test)]
mod tests;

/// How one request is sent and bounded.
#[derive(Clone)]
pub struct RequestOptions {
    /// HTTP method.
    pub method: Method,
    /// Extra headers, such as the account key's. One named like a default (`accept`, `host`) replaces it.
    pub headers: Vec<(&'static str, String)>,
    /// JSON body, sent with its content type and length.
    pub body: Option<String>,
    /// Deadline for one attempt, connect included; the connect alone gets at most [`CONNECT_TIMEOUT`].
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

/// Longest wait for the TCP connect and TLS handshake. Long enough for a resolver retry and a slow VPN or
/// mobile link, short enough that a host that is down is reported well before the request's own deadline.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// The usual deadline for a whole request: room for a server that is slow to answer, such as a register that
/// hashes and writes, after a connect that took most of [`CONNECT_TIMEOUT`].
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// Pause before the one retry of an idempotent request.
const RETRY_DELAY: Duration = Duration::from_millis(500);

/// A response with its body read in full.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    /// HTTP status code.
    pub status: u16,
    /// Response body.
    pub body: Bytes,
}

pub(crate) type Sender = SendRequest<Full<Bytes>>;

/// A fresh HTTP/1.1 connection: pinned TLS when `pin` is set, else plain TCP with no delay. `timeout` bounds the
/// connect and the TLS handshake.
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
            let tcp: TcpStream = tokio::time::timeout(timeout, tcp_connect(&endpoint.host, endpoint.port))
                .await
                .map_err(|_| HttpError::Timeout)??;
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
/// never followed. A GET or HEAD without a body that fails on the way (not a timeout, never a pin mismatch) is sent
/// once more to the same endpoint, so a key it carries goes nowhere new; anything else is sent exactly once.
pub async fn request(endpoint: &Endpoint, path: &str, options: RequestOptions) -> Result<HttpResponse, HttpError> {
    let pin = check_endpoint(endpoint)?;
    let idempotent = matches!(options.method, Method::GET | Method::HEAD) && options.body.is_none();
    // Built before any attempt, so a request that cannot be built is not retried
    let first = build(endpoint, path, &options)?;
    match attempt(endpoint, pin.clone(), first, &options).await {
        Err(err) if idempotent && err.is_transient() => {
            log::info(format!(
                "{} {}:{}{path} failed ({}: {err}); trying once more",
                options.method,
                endpoint.host,
                endpoint.port,
                err.kind()
            ));
            let pause = async {
                tokio::time::sleep(RETRY_DELAY).await;
                Ok(())
            };
            bounded(pause, RETRY_DELAY.saturating_mul(2), options.cancel.as_ref()).await?;
            attempt(endpoint, pin, build(endpoint, path, &options)?, &options).await
        }
        result => result,
    }
}

fn build(endpoint: &Endpoint, path: &str, options: &RequestOptions) -> Result<Request<Full<Bytes>>, HttpError> {
    build_request(
        endpoint,
        options.method.clone(),
        path,
        "application/json",
        &options.headers,
        options.body.as_deref(),
    )
}

/// POSTs `body` as `content_type`, otherwise as [`request`] does but never retried; `options.body` and
/// `options.method` are unused.
pub async fn upload(
    endpoint: &Endpoint,
    path: &str,
    content_type: &str,
    body: Bytes,
    options: RequestOptions,
) -> Result<HttpResponse, HttpError> {
    let pin = check_endpoint(endpoint)?;
    let req = build_request(endpoint, Method::POST, path, "application/json", &options.headers, None)?;
    let (mut parts, _) = req.into_parts();
    let header =
        |text: &str| hyper::header::HeaderValue::from_str(text).map_err(|e| HttpError::Protocol(e.to_string()));
    parts.headers.insert(hyper::header::CONTENT_TYPE, header(content_type)?);
    parts
        .headers
        .insert(hyper::header::CONTENT_LENGTH, header(&body.len().to_string())?);
    attempt(endpoint, pin, Request::from_parts(parts, Full::new(body)), &options).await
}

async fn attempt(
    endpoint: &Endpoint,
    pin: Option<String>,
    req: Request<Full<Bytes>>,
    options: &RequestOptions,
) -> Result<HttpResponse, HttpError> {
    if options.cancel.as_ref().is_some_and(CancellationToken::is_cancelled) {
        return Err(HttpError::Aborted);
    }

    let exchange = async {
        let mut sender = open(endpoint, pin, CONNECT_TIMEOUT.min(options.timeout)).await?;
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
