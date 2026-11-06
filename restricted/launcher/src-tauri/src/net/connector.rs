//! The relay's pooled connections to the invite's host (row L19), on launcher-kit's `net::connector` (RL2).

use std::time::Duration;

use launcher_kit::net::connector as kit;

use super::http::{Endpoint, HttpError};
use super::pinned::PinnedTarget;

pub use kit::Connector as HostConnector;

/// Where every connection goes, whatever URI the request names.
#[derive(Debug, Clone)]
pub enum Host {
    Pinned(PinnedTarget),
    Plain { host: String, port: u16 },
}

impl Host {
    fn endpoint(&self) -> Endpoint {
        match self {
            Self::Pinned(target) => Endpoint {
                host: target.host.clone(),
                port: target.port,
                pin: Some(target.fingerprint.clone()),
            },
            Self::Plain { host, port } => Endpoint {
                host: host.clone(),
                port: *port,
                pin: None,
            },
        }
    }
}

/// The connector for `host`; refused like any request to it would be (`http::check_endpoint`).
pub fn host_connector(host: &Host, timeout: Duration) -> Result<HostConnector, HttpError> {
    HostConnector::new(&host.endpoint(), timeout)
}

/// The pin error somewhere in an error's sources, with the fingerprint the server showed.
pub fn pin_error(error: &(dyn std::error::Error + 'static)) -> Option<Option<String>> {
    kit::pin_mismatch(error).map(Some)
}
