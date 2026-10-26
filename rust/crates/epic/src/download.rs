//! Installing a build from Epic's CDN (DD22): chunks from the three hosts with failover, each checked by
//! [`crate::chunk::decode`], files assembled and checked, then the executable's SHA-256. A stub until DE5, which
//! brings back DR's resumable downloader for it. The CDN asks for no login; ownership is checked first
//! ([`crate::library`]).
//!
//! The client DE5 writes speaks HTTPS only and follows no redirect: a chunk comes from these hosts or not at all.

use std::path::Path;

use crate::NotImplemented;
use crate::manifest::{ChunkInfo, Manifest};

/// Dauntless's build folder on each of Epic's CDN hosts, in the order the archive's guide gives them.
pub const CDN_BASES: [&str; 3] = [
    "https://epicgames-download1.akamaized.net/Builds/Org/o-krlzxj88qrtb69fredeuaf887bl5az/Jackal/default",
    "https://egdownload.fastly-edge.com/Builds/Org/o-krlzxj88qrtb69fredeuaf887bl5az/Jackal/default",
    "https://egs-cloudfront-chunks.epicgamescdn.com/Builds/Org/o-krlzxj88qrtb69fredeuaf887bl5az/Jackal/default",
];

/// A chunk's URL under one of [`CDN_BASES`].
pub fn chunk_url(base: &str, manifest: &Manifest, chunk: &ChunkInfo) -> String {
    format!("{base}/{}", chunk.cdn_path(manifest.feature_level))
}

/// Why an install failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DownloadError {
    #[error(transparent)]
    NotImplemented(#[from] NotImplemented),
}

/// Installs `manifest`'s build into `folder`, one folder per version (DE5).
pub async fn install(manifest: &Manifest, folder: &Path) -> Result<(), DownloadError> {
    let _ = (manifest, folder);
    Err(NotImplemented { row: "DE5" }.into())
}
