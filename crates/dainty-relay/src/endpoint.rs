// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Where the relay and the launcher's client connect, and why a connection failed.

use std::fmt;
use std::net::IpAddr;

use dainty_protocol::invite::{is_private_mode_host, is_valid_fingerprint, is_valid_host};

use crate::pinned::{ConnectError, PinnedTarget};

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
    /// The host name did not resolve.
    Dns(String),
    /// The host refused or reset the connection.
    Refused(String),
    /// The TLS handshake failed for a reason other than the pin.
    Tls(String),
}

/// What kind of failure an [`HttpError`] is, for the log and for which sentence the player reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// The name did not resolve.
    Dns,
    /// Nothing listens there, or a firewall resets it.
    Refused,
    /// No answer within the deadline.
    Timeout,
    /// The TLS handshake failed.
    Tls,
    /// The certificate is not the pinned one.
    Pin,
    /// The server answered with an error status.
    Status,
    /// The HTTP exchange broke, or the endpoint was refused before connecting.
    Other,
    /// The answer was larger than allowed.
    TooLarge,
    /// The caller cancelled.
    Aborted,
}

impl FailureKind {
    /// The kind of an HTTP status, or none for a success or redirect.
    pub fn of_status(status: u16) -> Option<Self> {
        (status >= 400).then_some(Self::Status)
    }

    /// The name the log uses.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Dns => "dns",
            Self::Refused => "refused",
            Self::Timeout => "timeout",
            Self::Tls => "tls",
            Self::Pin => "pin",
            Self::Status => "status",
            Self::Other => "other",
            Self::TooLarge => "too_large",
            Self::Aborted => "aborted",
        }
    }
}

impl fmt::Display for FailureKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl HttpError {
    /// The kind of this failure.
    pub fn kind(&self) -> FailureKind {
        match self {
            Self::Pin { .. } => FailureKind::Pin,
            Self::Timeout => FailureKind::Timeout,
            Self::Dns(_) => FailureKind::Dns,
            Self::Refused(_) => FailureKind::Refused,
            Self::Tls(_) => FailureKind::Tls,
            Self::Io(_) | Self::Protocol(_) => FailureKind::Other,
            Self::TooLarge => FailureKind::TooLarge,
            Self::Aborted => FailureKind::Aborted,
        }
    }

    /// Worth one more try of an idempotent request. A pin mismatch never is: the server would show the same
    /// certificate. Nor is a timeout, as the player has already waited the whole deadline.
    #[must_use]
    pub fn is_transient(&self) -> bool {
        matches!(
            self,
            Self::Dns(_) | Self::Refused(_) | Self::Tls(_) | Self::Io(_) | Self::Protocol(_)
        )
    }
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
            Self::Dns(msg) => write!(f, "name lookup failed: {msg}"),
            Self::Refused(msg) => write!(f, "connection refused: {msg}"),
            Self::Tls(msg) => write!(f, "TLS handshake failed: {msg}"),
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
            ConnectError::Dns(msg) => Self::Dns(msg),
            ConnectError::Refused(msg) => Self::Refused(msg),
            ConnectError::Tls(msg) => Self::Tls(msg),
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
