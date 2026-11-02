// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The outgoing GETs: the account lookup on the backend, and the GitHub news. Errors read as Node's `fetch`
//! wrote them, since they end up in the log's `reason`.

use std::sync::OnceLock;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt as _, Empty, Limited};
use hyper_rustls::HttpsConnector;
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;

pub const TIMEOUT_TEXT: &str = "The operation was aborted due to timeout";

/// A reply: the status and headers, and the body unless it was over the limit or broke off.
#[derive(Debug)]
pub struct Reply {
    pub status: u16,
    pub headers: http::HeaderMap,
    pub body: Result<Bytes, BodyError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BodyError {
    TooLarge,
    Failed(String),
}

type Https = Client<HttpsConnector<HttpConnector>, Empty<Bytes>>;

fn client() -> &'static Https {
    static CLIENT: OnceLock<Https> = OnceLock::new();
    CLIENT.get_or_init(|| {
        let builder = hyper_rustls::HttpsConnectorBuilder::new();
        let builder = match builder.with_provider_and_webpki_roots(rustls::crypto::aws_lc_rs::default_provider()) {
            Ok(builder) => builder,
            Err(_) => hyper_rustls::HttpsConnectorBuilder::new().with_webpki_roots(),
        };
        let connector = builder.https_or_http().enable_http1().build();
        Client::builder(TokioExecutor::new()).build(connector)
    })
}

/// GETs `url` with `headers`; the whole exchange, body included, within `timeout`. A redirect is an error, as
/// `fetch` with `redirect: "error"` has it.
pub async fn get(url: &str, headers: &[(&str, &str)], timeout: Duration, limit: usize) -> Result<Reply, String> {
    match tokio::time::timeout(timeout, exchange(url, headers, limit)).await {
        Ok(result) => result,
        Err(_) => Err(TIMEOUT_TEXT.to_owned()),
    }
}

async fn exchange(url: &str, headers: &[(&str, &str)], limit: usize) -> Result<Reply, String> {
    let mut request = http::Request::get(url);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let request = request
        .body(Empty::<Bytes>::new())
        .map_err(|e| format!("fetch failed: {e}"))?;
    let response = client()
        .request(request)
        .await
        .map_err(|e| format!("fetch failed: {e}"))?;
    let status = response.status().as_u16();
    if matches!(status, 301 | 302 | 303 | 307 | 308) {
        return Err("fetch failed: unexpected redirect".to_owned());
    }
    let (parts, body) = response.into_parts();
    let body = match Limited::new(body, limit).collect().await {
        Ok(collected) => Ok(collected.to_bytes()),
        Err(error) if error.is::<http_body_util::LengthLimitError>() => Err(BodyError::TooLarge),
        Err(error) => Err(BodyError::Failed(error.to_string())),
    };
    Ok(Reply {
        status,
        headers: parts.headers,
        body,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    /// A one-shot server writing `reply` raw to every connection.
    async fn raw_server(reply: &'static [u8]) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    _ = socket.read(&mut buf).await;
                    _ = socket.write_all(reply).await;
                    tokio::time::sleep(Duration::from_secs(3)).await;
                });
            }
        });
        format!("http://{addr}/")
    }

    #[tokio::test]
    async fn statuses_redirects_limits_and_timeouts() {
        let ok = raw_server(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\netag: \"e\"\r\n\r\nhi").await;
        let reply = get(&ok, &[("accept", "application/json")], Duration::from_secs(5), 10)
            .await
            .unwrap();
        assert_eq!((reply.status, reply.body.unwrap()), (200, Bytes::from_static(b"hi")));
        assert_eq!(reply.headers["etag"], "\"e\"");

        let big = raw_server(b"HTTP/1.1 200 OK\r\ncontent-length: 20\r\n\r\n01234567890123456789").await;
        let reply = get(&big, &[], Duration::from_secs(5), 10).await.unwrap();
        assert_eq!((reply.status, reply.body), (200, Err(BodyError::TooLarge)));

        let moved = raw_server(b"HTTP/1.1 302 Found\r\nlocation: /x\r\ncontent-length: 0\r\n\r\n").await;
        assert_eq!(
            get(&moved, &[], Duration::from_secs(5), 10).await.unwrap_err(),
            "fetch failed: unexpected redirect"
        );
        let unchanged = raw_server(b"HTTP/1.1 304 Not Modified\r\n\r\n").await;
        assert_eq!(
            get(&unchanged, &[], Duration::from_secs(5), 10).await.unwrap().status,
            304
        );

        let slow = raw_server(b"HTTP/1.1 200 OK\r\ncontent-length: 5\r\n\r\nab").await;
        assert_eq!(
            get(&slow, &[], Duration::from_millis(300), 10).await.unwrap_err(),
            TIMEOUT_TEXT
        );

        let free = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/", free.local_addr().unwrap());
        drop(free);
        assert!(
            get(&url, &[], Duration::from_secs(5), 10)
                .await
                .unwrap_err()
                .starts_with("fetch failed: ")
        );
    }
}
