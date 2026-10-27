//! The network layer (rows L19 to L25): pinned TLS, the HTTP client, the host API, the relay, verify and
//! the server's game runtime. DR's downloader and hunt-region probe come in with the `legacy` feature,
//! for Dauntless Revived's 1.4.4 servers.

#[cfg(feature = "legacy")]
pub mod autoregion;
pub mod connector;
#[cfg(feature = "legacy")]
pub mod downloader;
pub mod hostapi;
pub mod http;
pub mod pinned;
pub mod regionping;
pub mod relay;
pub mod relay_owner;
pub mod runtime_files;
pub mod verify;
pub mod versions;
