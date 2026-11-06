//! GETs to Epic's CDN and to the manifest sources: HTTPS only, with the webpki roots, and no redirect followed (a
//! `3xx` is a failure like any other status but `200`). Bodies are capped by the caller, and no header carries
//! anything of the player's: the CDN asks for no login (`docs/security/epic.md`, "The CDN").

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Empty, Limited};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;

/// Why one GET failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum FetchError {
    /// The URL is not `https`.
    #[error("not an https URL")]
    NotHttps,
    /// The host answered with this non-200 status.
    #[error("HTTP {0}")]
    Status(u16),
    /// The body went over this many bytes.
    #[error("larger than {0} bytes")]
    TooLarge(usize),
    /// The connection failed; holds the error text.
    #[error("{0}")]
    Connection(String),
    /// The request ran past its timeout.
    #[error("timed out")]
    TimedOut,
    /// The TLS client could not be set up.
    #[error("TLS setup failed")]
    Tls,
}

impl FetchError {
    /// Epic answers 403 or 404 for a chunk it has deleted.
    pub fn is_missing(&self) -> bool {
        matches!(self, Self::Status(403 | 404))
    }
}

/// An HTTPS client for the CDN and the manifest sources.
#[derive(Clone)]
pub struct Https {
    client: Client<hyper_rustls::HttpsConnector<HttpConnector>, Empty<Bytes>>,
    timeout: Duration,
}

impl Https {
    /// `tls` replaces the webpki roots (the tests' own CA); None in the launcher and the host kit.
    pub fn new(tls: Option<Arc<rustls::ClientConfig>>, timeout: Duration) -> Result<Self, FetchError> {
        let builder = match tls {
            Some(config) => hyper_rustls::HttpsConnectorBuilder::new().with_tls_config((*config).clone()),
            None => hyper_rustls::HttpsConnectorBuilder::new()
                .with_provider_and_webpki_roots(rustls::crypto::aws_lc_rs::default_provider())
                .map_err(|_| FetchError::Tls)?,
        };
        let connector = builder.https_only().enable_http1().build();
        Ok(Self {
            client: Client::builder(TokioExecutor::new()).build(connector),
            timeout,
        })
    }

    /// The body of a `200` answer of at most `max` bytes.
    pub async fn get(&self, url: &str, max: usize) -> Result<Bytes, FetchError> {
        if !url.starts_with("https://") {
            return Err(FetchError::NotHttps);
        }
        let request = hyper::Request::get(url)
            .header(hyper::header::ACCEPT_ENCODING, "identity")
            .body(Empty::new())
            .map_err(|_| FetchError::NotHttps)?;
        let work = async {
            let response = self
                .client
                .request(request)
                .await
                .map_err(|e| FetchError::Connection(e.to_string()))?;
            let status = response.status().as_u16();
            if status != 200 {
                return Err(FetchError::Status(status));
            }
            Limited::new(response.into_body(), max)
                .collect()
                .await
                .map(|body| body.to_bytes())
                .map_err(|e| match e.downcast::<http_body_util::LengthLimitError>() {
                    Ok(_) => FetchError::TooLarge(max),
                    Err(e) => FetchError::Connection(e.to_string()),
                })
        };
        tokio::time::timeout(self.timeout, work)
            .await
            .map_err(|_| FetchError::TimedOut)?
    }
}
