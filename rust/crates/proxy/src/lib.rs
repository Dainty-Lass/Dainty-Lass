// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

use std::collections::BTreeSet;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use http::HeaderMap;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::time::{Instant, Sleep};

pub const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

pub fn is_hop_by_hop(name: &str) -> bool {
    HOP_BY_HOP.iter().any(|hop| name.eq_ignore_ascii_case(hop))
}

pub fn connection_tokens(headers: &HeaderMap) -> BTreeSet<String> {
    let mut tokens = BTreeSet::new();
    for value in headers.get_all(http::header::CONNECTION) {
        if let Ok(s) = value.to_str() {
            for part in s.split(',') {
                let trimmed = part.trim();
                if !trimmed.is_empty() {
                    tokens.insert(trimmed.to_ascii_lowercase());
                }
            }
        }
    }
    tokens
}

pub fn end_to_end(headers: &HeaderMap, drop: impl Fn(&str) -> bool) -> HeaderMap {
    let conn_tokens = connection_tokens(headers);
    let mut out = HeaderMap::new();
    for (name, value) in headers {
        let s = name.as_str();
        if !is_hop_by_hop(s) && !drop(s) && !conn_tokens.contains(s) {
            out.append(name.clone(), value.clone());
        }
    }
    out
}

pub struct Idle<S> {
    inner: S,
    limit: Duration,
    sleep: Pin<Box<Sleep>>,
    written: u64,
}

impl<S> Idle<S> {
    pub fn new(inner: S, limit: Duration) -> Self {
        Self {
            inner,
            limit,
            sleep: Box::pin(tokio::time::sleep(limit)),
            written: 0,
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Idle<S> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        match Pin::new(&mut self.inner).poll_read(cx, buf) {
            Poll::Ready(Ok(())) => {
                if buf.filled().len() > before {
                    let deadline = Instant::now() + self.limit;
                    self.sleep.as_mut().reset(deadline);
                }
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Err(err)) => Poll::Ready(Err(err)),
            Poll::Pending => match self.sleep.as_mut().poll(cx) {
                Poll::Ready(()) => Poll::Ready(Err(io::Error::new(io::ErrorKind::TimedOut, "idle stream timed out"))),
                Poll::Pending => Poll::Pending,
            },
        }
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Idle<S> {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        match Pin::new(&mut self.inner).poll_write(cx, buf) {
            Poll::Ready(Ok(n)) => {
                if n > 0 {
                    self.written = self.written.saturating_add(n as u64);
                    let deadline = Instant::now() + self.limit;
                    self.sleep.as_mut().reset(deadline);
                }
                Poll::Ready(Ok(n))
            }
            Poll::Ready(Err(err)) => Poll::Ready(Err(err)),
            Poll::Pending => match self.sleep.as_mut().poll(cx) {
                Poll::Ready(()) => Poll::Ready(Err(io::Error::new(io::ErrorKind::TimedOut, "idle stream timed out"))),
                Poll::Pending => Poll::Pending,
            },
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

pub async fn tunnel<A, B>(a: A, b: B, idle: Duration) -> (u64, u64)
where
    A: AsyncRead + AsyncWrite + Unpin,
    B: AsyncRead + AsyncWrite + Unpin,
{
    let mut a = Idle::new(a, idle);
    let mut b = Idle::new(b, idle);
    tokio::io::copy_bidirectional(&mut a, &mut b)
        .await
        .unwrap_or((b.written, a.written))
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::{HeaderName, HeaderValue};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn filters_hop_by_hop_connection_tokens_and_extra_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::CONNECTION,
            HeaderValue::from_static("x-hop, keep-alive, , X-Second"),
        );
        headers.insert(http::header::TRANSFER_ENCODING, HeaderValue::from_static("chunked"));
        headers.insert(http::header::TE, HeaderValue::from_static("trailers"));
        headers.insert(
            HeaderName::from_static("keep-alive"),
            HeaderValue::from_static("timeout=5"),
        );
        headers.insert(HeaderName::from_static("x-hop"), HeaderValue::from_static("drop-me"));
        headers.insert(
            HeaderName::from_static("x-second"),
            HeaderValue::from_static("drop-me-too"),
        );
        headers.insert(http::header::HOST, HeaderValue::from_static("example.com"));
        headers.insert(
            HeaderName::from_static("x-kept"),
            HeaderValue::from_static("preserve-me"),
        );

        assert!(is_hop_by_hop("Connection"));
        assert!(is_hop_by_hop("TRANSFER-ENCODING"));
        assert!(!is_hop_by_hop("x-kept"));

        let tokens = connection_tokens(&headers);
        assert!(tokens.contains("x-hop"));
        assert!(tokens.contains("keep-alive"));
        assert!(tokens.contains("x-second"));

        let kept_map = end_to_end(&headers, |name| name == "host");
        let kept: Vec<(&str, &str)> = kept_map
            .iter()
            .filter_map(|(k, v)| v.to_str().ok().map(|val| (k.as_str(), val)))
            .collect();
        assert_eq!(kept, vec![("x-kept", "preserve-me")]);
    }

    #[tokio::test]
    async fn tunnel_copies_bidirectionally_when_active() -> io::Result<()> {
        let (mut client, proxy_a) = tokio::io::duplex(1024);
        let (proxy_b, mut upstream) = tokio::io::duplex(1024);

        let client_task = tokio::spawn(async move {
            client.write_all(b"ping").await?;
            client.shutdown().await?;
            let mut reply = Vec::new();
            client.read_to_end(&mut reply).await?;
            Ok::<Vec<u8>, io::Error>(reply)
        });

        let upstream_task = tokio::spawn(async move {
            let mut req = Vec::new();
            upstream.read_to_end(&mut req).await?;
            upstream.write_all(b"pong!").await?;
            upstream.shutdown().await?;
            Ok::<Vec<u8>, io::Error>(req)
        });

        let (a_to_b, b_to_a) = tunnel(proxy_a, proxy_b, Duration::from_secs(5)).await;
        assert_eq!(a_to_b, 4);
        assert_eq!(b_to_a, 5);

        let received_req = upstream_task.await.map_err(io::Error::other)??;
        let received_reply = client_task.await.map_err(io::Error::other)??;
        assert_eq!(received_req, b"ping");
        assert_eq!(received_reply, b"pong!");
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn tunnel_times_out_when_idle() {
        let (_client, proxy_a) = tokio::io::duplex(1024);
        let (proxy_b, _upstream) = tokio::io::duplex(1024);

        let (a_to_b, b_to_a) = tunnel(proxy_a, proxy_b, Duration::from_millis(50)).await;
        assert_eq!((a_to_b, b_to_a), (0, 0));
    }
}
