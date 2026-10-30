// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

use std::fmt;
use std::path::Path;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Method, Request};
use hyper_util::rt::TokioIo;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio_util::sync::CancellationToken;

use crate::net::http::{Endpoint, HttpError, check_endpoint, host_header};
use crate::net::pinned::{PinnedTarget, pinned_connect, validate_host};

pub struct FileRequestOptions<'a> {
    pub headers: Vec<(&'static str, String)>,
    pub idle_timeout: Duration,
    pub max_bytes: u64,
    pub cancel: Option<CancellationToken>,
    pub on_bytes: Option<&'a mut (dyn FnMut(u64) + Send)>,
}

#[derive(Debug)]
pub enum FileRequestError {
    Http(HttpError),
    Disk(std::io::Error),
}

impl fmt::Display for FileRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(err) => write!(f, "{err}"),
            Self::Disk(err) => write!(f, "disk error: {err}"),
        }
    }
}

impl std::error::Error for FileRequestError {}

pub async fn request_to_file(
    endpoint: &Endpoint,
    path_and_query: &str,
    target: &Path,
    mut options: FileRequestOptions<'_>,
) -> Result<u16, FileRequestError> {
    let checked_pin = check_endpoint(endpoint).map_err(FileRequestError::Http)?;
    if !path_and_query.starts_with('/') {
        return Err(FileRequestError::Http(HttpError::Protocol(
            "request path must start with '/'".to_owned(),
        )));
    }

    if options
        .cancel
        .as_ref()
        .is_some_and(CancellationToken::is_cancelled)
    {
        return Err(FileRequestError::Http(HttpError::Aborted));
    }

    let connect_fut = async {
        let mut sender = match checked_pin {
            Some(fingerprint) => {
                let pinned = PinnedTarget {
                    host: endpoint.host.clone(),
                    port: endpoint.port,
                    fingerprint,
                };
                let tls = pinned_connect(&pinned, options.idle_timeout)
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

        let mut builder = Request::builder()
            .method(Method::GET)
            .uri(path_and_query)
            .header("host", host_header(endpoint))
            .header("connection", "close");

        for (name, value) in &options.headers {
            builder = builder.header(*name, value.as_str());
        }

        let req = builder
            .body(Full::new(Bytes::new()))
            .map_err(|e| HttpError::Protocol(e.to_string()))?;

        sender
            .send_request(req)
            .await
            .map_err(|e| HttpError::Protocol(e.to_string()))
    };

    let res = match &options.cancel {
        Some(token) => tokio::select! {
            () = token.cancelled() => return Err(FileRequestError::Http(HttpError::Aborted)),
            r = tokio::time::timeout(options.idle_timeout, connect_fut) => {
                r.map_err(|_| FileRequestError::Http(HttpError::Timeout))?
                    .map_err(FileRequestError::Http)?
            }
        },
        None => tokio::time::timeout(options.idle_timeout, connect_fut)
            .await
            .map_err(|_| FileRequestError::Http(HttpError::Timeout))?
            .map_err(FileRequestError::Http)?,
    };

    let status = res.status().as_u16();
    if status != 200 {
        return Ok(status);
    }

    if let Some(cl) = res
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        && cl > options.max_bytes
    {
        return Err(FileRequestError::Http(HttpError::TooLarge));
    }

    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(FileRequestError::Disk)?;
    }

    let mut file = File::create(target).await.map_err(FileRequestError::Disk)?;
    let mut body_stream = res.into_body();
    let mut total_bytes: u64 = 0;

    let stream_res: Result<(), FileRequestError> = async {
        loop {
            let next_frame = match &options.cancel {
                Some(token) => tokio::select! {
                    () = token.cancelled() => return Err(FileRequestError::Http(HttpError::Aborted)),
                    r = tokio::time::timeout(options.idle_timeout, body_stream.frame()) => {
                        r.map_err(|_| FileRequestError::Http(HttpError::Timeout))?
                    }
                },
                None => tokio::time::timeout(options.idle_timeout, body_stream.frame())
                    .await
                    .map_err(|_| FileRequestError::Http(HttpError::Timeout))?,
            };

            let Some(frame_res) = next_frame else {
                break;
            };
            let frame =
                frame_res.map_err(|e| FileRequestError::Http(HttpError::Protocol(e.to_string())))?;
            if let Some(chunk) = frame.data_ref() {
                if chunk.is_empty() {
                    continue;
                }
                let chunk_len = chunk.len() as u64;
                total_bytes = total_bytes.saturating_add(chunk_len);
                if total_bytes > options.max_bytes {
                    return Err(FileRequestError::Http(HttpError::TooLarge));
                }
                file.write_all(chunk)
                    .await
                    .map_err(FileRequestError::Disk)?;
                if let Some(cb) = options.on_bytes.as_deref_mut() {
                    cb(chunk_len);
                }
            }
        }
        file.flush().await.map_err(FileRequestError::Disk)?;
        Ok(())
    }
    .await;

    drop(file);
    if let Err(err) = stream_res {
        let _ = tokio::fs::remove_file(target).await;
        return Err(err);
    }

    Ok(status)
}
