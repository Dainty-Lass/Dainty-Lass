// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! A connector for hyper-util's pooled client that always reaches one endpoint, for callers that send many requests
//! to the invite's host and want them kept alive.

use std::error::Error as StdError;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use hyper::Uri;
use hyper::rt::{Read, ReadBufCursor, Write};
use hyper_util::client::legacy::connect::{Connected, Connection};
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;
use tokio_rustls::client::TlsStream;

use crate::endpoint::{Endpoint, HttpError, check_endpoint};
use crate::pinned::{ConnectError, PinnedTarget, pinned_connect, tcp_connect};

/// Connects to its endpoint whatever URI the client asks for, so the URI only carries the path. Clones share
/// the count of connections opened.
#[derive(Clone)]
pub struct Connector {
    inner: Arc<Inner>,
}

struct Inner {
    host: String,
    port: u16,
    pin: Option<String>,
    timeout: Duration,
    opened: AtomicUsize,
}

impl Connector {
    /// A connector for `endpoint`, refused as [`check_endpoint`] refuses it. `timeout` bounds each connect,
    /// the TLS handshake included.
    pub fn new(endpoint: &Endpoint, timeout: Duration) -> Result<Self, HttpError> {
        let pin = check_endpoint(endpoint)?;
        Ok(Self {
            inner: Arc::new(Inner {
                host: endpoint.host.clone(),
                port: endpoint.port,
                pin,
                timeout,
                opened: AtomicUsize::new(0),
            }),
        })
    }

    /// Connections opened so far.
    pub fn connections_opened(&self) -> usize {
        self.inner.opened.load(Ordering::Relaxed)
    }

    async fn connect(inner: Arc<Inner>) -> Result<Stream, ConnectError> {
        let stream = match &inner.pin {
            Some(fingerprint) => {
                let target = PinnedTarget {
                    host: inner.host.clone(),
                    port: inner.port,
                    fingerprint: fingerprint.clone(),
                };
                let tls = pinned_connect(&target, inner.timeout).await?;
                Stream::Tls(TokioIo::new(Box::new(tls)))
            }
            None => {
                let tcp = tokio::time::timeout(inner.timeout, tcp_connect(&inner.host, inner.port))
                    .await
                    .map_err(|_| ConnectError::Timeout)??;
                Stream::Plain(TokioIo::new(tcp))
            }
        };
        inner.opened.fetch_add(1, Ordering::Relaxed);
        Ok(stream)
    }
}

impl tower::Service<Uri> for Connector {
    type Response = Stream;
    type Error = ConnectError;
    type Future = Pin<Box<dyn Future<Output = Result<Stream, ConnectError>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _uri: Uri) -> Self::Future {
        Box::pin(Self::connect(Arc::clone(&self.inner)))
    }
}

/// One connection the [`Connector`] opened.
pub enum Stream {
    /// Plain TCP to a private-mode host.
    Plain(TokioIo<TcpStream>),
    /// TLS pinned to the endpoint's certificate.
    Tls(TokioIo<Box<TlsStream<TcpStream>>>),
}

impl Connection for Stream {
    fn connected(&self) -> Connected {
        Connected::new()
    }
}

impl Read for Stream {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: ReadBufCursor<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Plain(io) => Pin::new(io).poll_read(cx, buf),
            Self::Tls(io) => Pin::new(io).poll_read(cx, buf),
        }
    }
}

impl Write for Stream {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Plain(io) => Pin::new(io).poll_write(cx, buf),
            Self::Tls(io) => Pin::new(io).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Plain(io) => Pin::new(io).poll_flush(cx),
            Self::Tls(io) => Pin::new(io).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Plain(io) => Pin::new(io).poll_shutdown(cx),
            Self::Tls(io) => Pin::new(io).poll_shutdown(cx),
        }
    }
}

/// The fingerprint the server presented, when a pin mismatch is anywhere in `error`'s source chain (the pooled
/// client wraps the connector's error).
pub fn pin_mismatch(error: &(dyn StdError + 'static)) -> Option<String> {
    let mut current = Some(error);
    while let Some(err) = current {
        if let Some(ConnectError::Pin { actual, .. }) = err.downcast_ref::<ConnectError>() {
            return Some(actual.clone());
        }
        if let Some(HttpError::Pin { actual }) = err.downcast_ref::<HttpError>() {
            return Some(actual.clone());
        }
        current = err.source();
    }
    None
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

    use std::convert::Infallible;

    use bytes::Bytes;
    use http_body_util::{BodyExt, Empty, Full};
    use hyper::body::Incoming;
    use hyper::service::service_fn;
    use hyper::{Request, Response};
    use hyper_util::client::legacy::Client;
    use hyper_util::rt::TokioExecutor;
    use tokio::net::TcpListener;

    use super::*;
    use crate::pinned::certificate_fingerprint;
    use crate::pinned::test_tls::{acceptor, certificate};

    const SECONDS: Duration = Duration::from_secs(3);

    async fn echo_path(request: Request<Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
        Ok(Response::new(Full::new(Bytes::from(request.uri().path().to_owned()))))
    }

    /// A keep-alive server answering each request with its path; with `tls`, also the certificate's fingerprint.
    async fn server(tls: bool) -> (u16, Option<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (cert, key) = certificate();
        let pin = tls.then(|| certificate_fingerprint(cert.as_ref()));
        let tls = tls.then(|| acceptor(cert, key));
        tokio::spawn(async move {
            loop {
                let Ok((tcp, _)) = listener.accept().await else { return };
                let tls = tls.clone();
                tokio::spawn(async move {
                    let http = hyper::server::conn::http1::Builder::new();
                    match tls {
                        Some(tls) => {
                            if let Ok(stream) = tls.accept(tcp).await {
                                let _ = http.serve_connection(TokioIo::new(stream), service_fn(echo_path)).await;
                            }
                        }
                        None => {
                            let _ = http.serve_connection(TokioIo::new(tcp), service_fn(echo_path)).await;
                        }
                    }
                });
            }
        });
        (port, pin)
    }

    fn endpoint(port: u16, pin: Option<String>) -> Endpoint {
        Endpoint {
            host: "127.0.0.1".to_owned(),
            port,
            pin,
        }
    }

    async fn get(client: &Client<Connector, Empty<Bytes>>, uri: &str) -> String {
        let response = client.get(uri.parse().unwrap()).await.unwrap();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        String::from_utf8(body.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn every_uri_reaches_the_one_host_over_a_pooled_connection() {
        for tls in [false, true] {
            let (port, pin) = server(tls).await;
            let connector = Connector::new(&endpoint(port, pin), SECONDS).unwrap();
            let client = Client::builder(TokioExecutor::new()).build(connector.clone());
            assert_eq!(get(&client, "http://ignored.invalid/a").await, "/a");
            assert_eq!(get(&client, "http://ignored.invalid/b").await, "/b");
            assert_eq!(connector.connections_opened(), 1, "tls: {tls}");
            // The pool keys by authority, so another one opens another connection, still to the one host
            assert_eq!(get(&client, "http://other.invalid:1/c").await, "/c");
            assert_eq!(connector.connections_opened(), 2, "tls: {tls}");
        }
    }

    #[tokio::test]
    async fn a_wrong_pin_is_found_through_the_clients_error() {
        let (port, pin) = server(true).await;
        let connector = Connector::new(&endpoint(port, Some("0".repeat(64))), SECONDS).unwrap();
        let client: Client<_, Empty<Bytes>> = Client::builder(TokioExecutor::new()).build(connector.clone());
        let err = client.get("http://x.invalid/".parse().unwrap()).await.unwrap_err();
        assert_eq!(pin_mismatch(&err), pin);
        assert_eq!(connector.connections_opened(), 0);
    }

    #[test]
    fn pin_mismatch_walks_the_source_chain() {
        #[derive(Debug)]
        struct Wrapper(ConnectError);
        impl std::fmt::Display for Wrapper {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("wrapped")
            }
        }
        impl StdError for Wrapper {
            fn source(&self) -> Option<&(dyn StdError + 'static)> {
                Some(&self.0)
            }
        }
        let pin = ConnectError::Pin {
            expected: "0".repeat(64),
            actual: "a".repeat(64),
        };
        assert_eq!(pin_mismatch(&Wrapper(pin)), Some("a".repeat(64)));
        assert_eq!(pin_mismatch(&Wrapper(ConnectError::Timeout)), None);
        assert_eq!(
            pin_mismatch(&HttpError::Pin { actual: "b".repeat(64) }),
            Some("b".repeat(64))
        );
    }

    #[tokio::test]
    async fn a_refused_endpoint_makes_no_connector_and_a_silent_one_times_out() {
        assert!(Connector::new(&endpoint(0, None), SECONDS).is_err());
        let public = Endpoint {
            host: "203.0.113.5".to_owned(),
            port: 61000,
            pin: None,
        };
        assert!(Connector::new(&public, SECONDS).is_err());

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        // Accepts and never finishes the TLS handshake
        let held = tokio::spawn(async move {
            let conn = listener.accept().await;
            std::future::pending::<()>().await;
            drop(conn);
        });
        let mut connector = Connector::new(&endpoint(port, Some("a".repeat(64))), Duration::from_millis(200)).unwrap();
        let err = tower::Service::call(&mut connector, Uri::from_static("http://x.invalid/"))
            .await
            .err()
            .unwrap();
        assert_eq!(err, ConnectError::Timeout);
        held.abort();
    }
}
