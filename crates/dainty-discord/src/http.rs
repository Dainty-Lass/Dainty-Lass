// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! One HTTP/1.1 client for Discord's REST API and dainty-dash: webpki roots on aws-lc-rs, a time limit and a body
//! limit on every call.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper_util::client::legacy::Client as HyperClient;
use hyper_util::client::legacy::connect::HttpConnector;

type Https = hyper_rustls::HttpsConnector<HttpConnector>;

/// A TLS client configuration that trusts the webpki roots, on aws-lc-rs.
pub fn tls_config() -> Result<rustls::ClientConfig, String> {
    // aws-lc-rs by name: another provider compiled in leaves rustls no default
    let builder = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::aws_lc_rs::default_provider()))
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("TLS: {e}"))?;
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    Ok(builder.with_root_certificates(roots).with_no_client_auth())
}

/// An answer: its status and body.
#[derive(Debug, Clone)]
pub struct Answer {
    /// The status.
    pub status: http::StatusCode,
    /// The body, within the call's limit.
    pub body: Bytes,
}

/// Why a call has no answer; never carries the request (its headers hold the token).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallError {
    /// Not answered in time.
    Timeout,
    /// The connection or TLS failed.
    Connect,
    /// The body was larger than the limit.
    TooLarge,
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Timeout => "no answer in time",
            Self::Connect => "could not connect",
            Self::TooLarge => "an answer larger than the limit",
        })
    }
}

/// The client.
#[derive(Clone)]
pub struct Client {
    inner: HyperClient<Https, Full<Bytes>>,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Client")
    }
}

impl Client {
    /// A client for https, and for http when `allow_http` (dainty-dash on loopback or a private network).
    pub fn new(allow_http: bool) -> Result<Self, String> {
        let builder = hyper_rustls::HttpsConnectorBuilder::new().with_tls_config(tls_config()?);
        let connector = if allow_http {
            builder.https_or_http().enable_http1().build()
        } else {
            builder.https_only().enable_http1().build()
        };
        Ok(Self {
            inner: HyperClient::builder(hyper_util::rt::TokioExecutor::new()).build(connector),
        })
    }

    /// Sends `request`; the answer's body is read up to `max_body` bytes, all within `timeout`.
    pub async fn call(
        &self,
        request: http::Request<Full<Bytes>>,
        max_body: usize,
        timeout: Duration,
    ) -> Result<Answer, CallError> {
        let call = async {
            let response = self.inner.request(request).await.map_err(|_| CallError::Connect)?;
            let status = response.status();
            let body = Limited::new(response.into_body(), max_body)
                .collect()
                .await
                .map_err(|_| CallError::TooLarge)?
                .to_bytes();
            Ok(Answer { status, body })
        };
        tokio::time::timeout(timeout, call)
            .await
            .unwrap_or(Err(CallError::Timeout))
    }
}
