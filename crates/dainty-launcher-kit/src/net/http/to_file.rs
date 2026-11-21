// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Streaming a GET response straight to a file, for runtime and game downloads.

use std::fmt;
use std::path::Path;
use std::time::Duration;

use http_body_util::BodyExt;
use hyper::Method;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

use crate::net::http::{Endpoint, HttpError, bounded, build_request, check_endpoint, header_names, open};

/// How a file download is sent and bounded.
pub struct FileRequestOptions<'a> {
    /// Extra headers, such as the account key's.
    pub headers: Vec<(&'static str, String)>,
    /// Longest wait for the connection, the answer or the next body chunk.
    pub idle_timeout: Duration,
    /// Largest body accepted before [`HttpError::TooLarge`].
    pub max_bytes: u64,
    /// Cancels the download when triggered.
    pub cancel: Option<CancellationToken>,
    /// Called with each chunk's size as it is written, for progress.
    pub on_bytes: Option<&'a mut (dyn FnMut(u64) + Send)>,
}

// Header values carry the account key
impl fmt::Debug for FileRequestOptions<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileRequestOptions")
            .field("headers", &header_names(&self.headers))
            .field("idle_timeout", &self.idle_timeout)
            .field("max_bytes", &self.max_bytes)
            .field("cancel", &self.cancel.is_some())
            .field("on_bytes", &self.on_bytes.is_some())
            .finish()
    }
}

/// Why a download failed.
#[derive(Debug)]
pub enum FileRequestError {
    /// The request failed.
    Http(HttpError),
    /// Writing the file failed.
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

impl From<HttpError> for FileRequestError {
    fn from(err: HttpError) -> Self {
        Self::Http(err)
    }
}

/// GETs `path_and_query` into `target` and returns the status. Only a 200's body is written, and a
/// download that fails once the file exists has it removed.
pub async fn request_to_file(
    endpoint: &Endpoint,
    path_and_query: &str,
    target: &Path,
    mut options: FileRequestOptions<'_>,
) -> Result<u16, FileRequestError> {
    let pin = check_endpoint(endpoint)?;
    let req = build_request(
        endpoint,
        Method::GET,
        path_and_query,
        "application/octet-stream",
        &options.headers,
        None,
    )?;
    let cancel = options.cancel.clone();
    if cancel.as_ref().is_some_and(CancellationToken::is_cancelled) {
        return Err(HttpError::Aborted.into());
    }

    let idle = options.idle_timeout;
    let answer = async {
        let mut sender = open(endpoint, pin, idle).await?;
        sender
            .send_request(req)
            .await
            .map_err(|e| HttpError::Protocol(e.to_string()))
    };
    let res = bounded(answer, idle, cancel.as_ref()).await?;

    let status = res.status().as_u16();
    if status != 200 {
        return Ok(status);
    }
    let declared = res
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok());
    if declared.is_some_and(|len| len > options.max_bytes) {
        return Err(HttpError::TooLarge.into());
    }

    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(FileRequestError::Disk)?;
    }
    let mut file = File::create(target).await.map_err(FileRequestError::Disk)?;
    let mut body = res.into_body();
    let mut total: u64 = 0;

    let streamed: Result<(), FileRequestError> = async {
        loop {
            let next = bounded(async { Ok(body.frame().await) }, idle, cancel.as_ref()).await?;
            let Some(frame) = next else { break };
            let frame = frame.map_err(|e| HttpError::Protocol(e.to_string()))?;
            let Some(chunk) = frame.data_ref() else { continue };
            if chunk.is_empty() {
                continue;
            }
            let len = chunk.len() as u64;
            total = total.saturating_add(len);
            if total > options.max_bytes {
                return Err(HttpError::TooLarge.into());
            }
            file.write_all(chunk).await.map_err(FileRequestError::Disk)?;
            if let Some(cb) = options.on_bytes.as_deref_mut() {
                cb(len);
            }
        }
        file.flush().await.map_err(FileRequestError::Disk)
    }
    .await;

    drop(file);
    if let Err(err) = streamed {
        let _ = tokio::fs::remove_file(target).await;
        return Err(err);
    }
    Ok(status)
}
