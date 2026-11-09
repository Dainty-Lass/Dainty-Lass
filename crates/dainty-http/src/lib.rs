// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The one outgoing HTTP/1.1 client the services build: rustls with the webpki roots on aws-lc-rs (D10), named
//! explicitly because another provider compiled in leaves rustls no default. [`call`] bounds every exchange by a
//! time limit and the answer's body by a size limit. Errors never carry the request, whose headers may hold a key.

use std::error::Error as StdError;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http::{HeaderMap, Request, StatusCode};
use http_body::Body;
use http_body_util::{BodyExt as _, Full, LengthLimitError, Limited};
use hyper_rustls::{HttpsConnector, HttpsConnectorBuilder};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;

/// The connector every client here uses.
pub type Connector = HttpsConnector<HttpConnector>;
/// A pooled client sending bodies of type `B`.
pub type Client<B = Full<Bytes>> = hyper_util::client::legacy::Client<Connector, B>;

/// Which URLs a client may reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Schemes {
    /// `https://` only.
    HttpsOnly,
    /// `http://` too: a dashboard, bucket or log server on loopback or a private network.
    HttpOrHttps,
}

/// The webpki roots on aws-lc-rs, for a TLS client that is not HTTP (SMTP) or that adds to it.
pub fn tls_config() -> Result<rustls::ClientConfig, rustls::Error> {
    let builder = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::aws_lc_rs::default_provider()))
        .with_safe_default_protocol_versions()?;
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    Ok(builder.with_root_certificates(roots).with_no_client_auth())
}

/// A client on the webpki roots.
pub fn client<B>(schemes: Schemes) -> Client<B>
where
    B: Body + Send,
    B::Data: Send,
{
    // aws-lc-rs always offers a safe protocol version, so the fallback to rustls' default is never taken
    let builder = HttpsConnectorBuilder::new()
        .with_provider_and_webpki_roots(rustls::crypto::aws_lc_rs::default_provider())
        .unwrap_or_else(|_| HttpsConnectorBuilder::new().with_webpki_roots());
    let connector = match schemes {
        Schemes::HttpsOnly => builder.https_only().enable_http1().build(),
        Schemes::HttpOrHttps => builder.https_or_http().enable_http1().build(),
    };
    hyper_util::client::legacy::Client::builder(TokioExecutor::new()).build(connector)
}

/// A client on `tls` instead of the webpki roots (a test's own CA).
pub fn client_with<B>(schemes: Schemes, tls: rustls::ClientConfig) -> Client<B>
where
    B: Body + Send,
    B::Data: Send,
{
    let builder = HttpsConnectorBuilder::new().with_tls_config(tls);
    let connector = match schemes {
        Schemes::HttpsOnly => builder.https_only().enable_http1().build(),
        Schemes::HttpOrHttps => builder.https_or_http().enable_http1().build(),
    };
    hyper_util::client::legacy::Client::builder(TokioExecutor::new()).build(connector)
}

/// An answer, its body read within the call's limit.
#[derive(Debug, Clone)]
pub struct Answer {
    /// The status.
    pub status: StatusCode,
    /// The headers.
    pub headers: HeaderMap,
    /// The body.
    pub body: Bytes,
}

/// Why a call has no answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallError {
    /// Not answered, body included, in time.
    Timeout,
    /// The connection or TLS failed; hyper's text.
    Connect(String),
    /// The body was larger than the limit.
    TooLarge,
    /// The body broke off; hyper's text.
    Body(String),
}

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout => f.write_str("no answer in time"),
            Self::Connect(error) => write!(f, "could not connect: {error}"),
            Self::TooLarge => f.write_str("an answer larger than the limit"),
            Self::Body(error) => write!(f, "the answer broke off: {error}"),
        }
    }
}

impl StdError for CallError {}

/// Sends `request`; the body is read up to `max_body` bytes, all within `timeout`.
pub async fn call<B>(
    client: &Client<B>,
    request: Request<B>,
    max_body: usize,
    timeout: Duration,
) -> Result<Answer, CallError>
where
    B: Body + Send + Unpin + 'static,
    B::Data: Send,
    B::Error: Into<Box<dyn StdError + Send + Sync>>,
{
    call_with(client, request, |_| max_body, timeout).await
}

/// [`call`], with the body's limit chosen by the status (a small one for an error page).
pub async fn call_with<B>(
    client: &Client<B>,
    request: Request<B>,
    max_body: impl FnOnce(StatusCode) -> usize,
    timeout: Duration,
) -> Result<Answer, CallError>
where
    B: Body + Send + Unpin + 'static,
    B::Data: Send,
    B::Error: Into<Box<dyn StdError + Send + Sync>>,
{
    let exchange = async {
        let response = client
            .request(request)
            .await
            .map_err(|e| CallError::Connect(e.to_string()))?;
        let (parts, body) = response.into_parts();
        let body = Limited::new(body, max_body(parts.status))
            .collect()
            .await
            .map_err(|e| {
                if e.is::<LengthLimitError>() {
                    CallError::TooLarge
                } else {
                    CallError::Body(e.to_string())
                }
            })?
            .to_bytes();
        Ok(Answer {
            status: parts.status,
            headers: parts.headers,
            body,
        })
    };
    tokio::time::timeout(timeout, exchange)
        .await
        .unwrap_or(Err(CallError::Timeout))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "a panic is how a test fails")]

    use http_body_util::Empty;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::net::TcpListener;

    use super::*;

    /// A one-shot server writing `reply` raw.
    async fn serve(reply: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0u8; 1024];
            let _ = socket.read(&mut buffer).await;
            let _ = socket.write_all(reply.as_bytes()).await;
            let _ = socket.shutdown().await;
        });
        format!("http://{address}/")
    }

    fn get(url: &str) -> Request<Empty<Bytes>> {
        Request::get(url).body(Empty::new()).unwrap()
    }

    #[tokio::test]
    async fn reads_within_the_limits() {
        let client = client::<Empty<Bytes>>(Schemes::HttpOrHttps);
        let url = serve("HTTP/1.1 201 Created\r\nx-a: b\r\ncontent-length: 5\r\n\r\nhello").await;
        let answer = call(&client, get(&url), 5, Duration::from_secs(5)).await.unwrap();
        assert_eq!((answer.status, &answer.body[..]), (StatusCode::CREATED, &b"hello"[..]));
        assert_eq!(answer.headers["x-a"], "b");

        let url = serve("HTTP/1.1 500 Oops\r\ncontent-length: 5\r\n\r\nhello").await;
        let small = call_with(
            &client,
            get(&url),
            |s| if s.is_success() { 5 } else { 4 },
            Duration::from_secs(5),
        );
        assert_eq!(small.await.unwrap_err(), CallError::TooLarge);
    }

    #[tokio::test]
    async fn https_only_refuses_http() {
        let client = client::<Empty<Bytes>>(Schemes::HttpsOnly);
        let url = serve("HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n").await;
        let refused = call(&client, get(&url), 1, Duration::from_secs(5)).await;
        assert!(matches!(refused, Err(CallError::Connect(_))), "{refused:?}");
        assert!(tls_config().is_ok());
    }
}
