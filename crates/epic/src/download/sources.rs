//! Where a version's manifest comes from (DD22): the joined server's content copy first, then the game module's own
//! sources from its client profile, in their order (its repository's copy, then the community archive). None is
//! trusted: whichever answers, the bytes are checked against the profile's SHA-1 before a byte is parsed
//! ([`Manifest::parse`]), and a source that gives other bytes is skipped for the next.

use std::future::Future;

use super::cdn::Https;
use crate::manifest::{Manifest, Sha1Digest};

/// 1.14.7's manifest is about 1 MB packed; [`crate::manifest`] refuses anything that unpacks past 64 MiB anyway.
pub const MAX_MANIFEST_BYTES: usize = 16 << 20;

/// Which source gave the manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestOrigin {
    /// The joined server's content service
    Server,
    /// One of the client profile's `manifest_sources`
    Source(String),
}

/// No source gave the pinned manifest: what each answered, in the order asked.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("no source gave the pinned manifest")]
pub struct ManifestUnavailable {
    /// Each source asked and why it failed, in the order asked.
    pub tried: Vec<(ManifestOrigin, String)>,
}

/// The first manifest, in DD22's order, whose SHA-1 is `pin` and which parses. `server` is the joined server's
/// copy (the launcher fetches it through its pinned connection), None when no server is joined.
pub async fn fetch_manifest<F>(
    server: Option<F>,
    sources: &[String],
    pin: &Sha1Digest,
    https: &Https,
) -> Result<(Manifest, ManifestOrigin), ManifestUnavailable>
where
    F: Future<Output = Result<Vec<u8>, String>>,
{
    let mut tried = Vec::new();
    if let Some(server) = server {
        match server.await {
            Ok(bytes) => match Manifest::parse(&bytes, pin) {
                Ok(manifest) => return Ok((manifest, ManifestOrigin::Server)),
                Err(e) => tried.push((ManifestOrigin::Server, e.to_string())),
            },
            Err(e) => tried.push((ManifestOrigin::Server, e)),
        }
    }
    for url in sources {
        let origin = ManifestOrigin::Source(url.clone());
        match https.get(url, MAX_MANIFEST_BYTES).await {
            Ok(bytes) => match Manifest::parse(&bytes, pin) {
                Ok(manifest) => return Ok((manifest, origin)),
                Err(e) => tried.push((origin, e.to_string())),
            },
            Err(e) => tried.push((origin, e.to_string())),
        }
    }
    Err(ManifestUnavailable { tried })
}
