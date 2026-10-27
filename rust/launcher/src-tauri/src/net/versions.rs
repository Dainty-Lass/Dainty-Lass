//! The game versions the launcher offers (DE4, DD22): the compiled-in client profiles (`builtin`) and those
//! the joined servers' content serves, each Epic build with its download and install size read from its
//! manifest. The manifest is fetched only when the list is asked for and its sizes are kept by SHA-1, so
//! it is fetched once a session. Sizes are metadata: no game file is fetched here.
//!
//! A joined server is asked on DF4b's routes: `/content/v1/versions` lists what its modules serve, then each
//! version with a profile gets `/versions/<version>/client`. A manifest comes from a server that holds one
//! for the version first (`/versions/<version>/epic/manifest`, with the account key, which goes to that
//! server and no other), then the profile's sources in order (the module's own copy, then the archive), and
//! is taken only when it hashes to the profile's pin (`docs/security/epic.md`). A compiled-in version keeps
//! its own pin: a server's profile for it with another pin is ignored. DE5 fetches through the same routes
//! (`server_manifest_path`).

pub mod builtin;
pub mod parse;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use bytes::Bytes;
use epic::manifest::{Manifest, Sha1Digest};
use futures_util::future::join_all;
use http_body_util::{BodyExt, Empty, Limited};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use module_api::{ClientProfile, GameVersion};

use super::hostapi::key_headers;
use super::http::{Endpoint, RequestOptions, parse_json_body, request};
use crate::bridge::types::{
    EpicManifestState, EpicVersionInfo, GameVersionEntry, GameVersionInstall, GameVersionOrigin,
};

/// DF4b's routes on content (`services/content/src/modules.rs`): the versions its modules serve, and under
/// each `<version>/client` and `<version>/epic/manifest`. A version or an alias is a path segment.
pub const VERSIONS_PATH: &str = "/content/v1/versions";

/// `/content/v1/versions/<version>/client`: the module's `ClientProfile` as JSON
pub fn server_client_path(version: &GameVersion) -> String {
    format!("{VERSIONS_PATH}/{version}/client")
}

/// `/content/v1/versions/<version>/epic/manifest`: the pinned file, for a request with the account key.
/// 404 `no_epic_manifest` means this server has none, 503 `epic_manifest_invalid` that its file fails the
/// pin; either way the next source is tried.
pub fn server_manifest_path(version: &GameVersion) -> String {
    format!("{VERSIONS_PATH}/{version}/epic/manifest")
}

const PROFILES_TIMEOUT: Duration = Duration::from_secs(5);
const PROFILES_MAX_BYTES: usize = 256 * 1024;
const MANIFEST_TIMEOUT: Duration = Duration::from_secs(20);
/// For one version, every source included: with the profiles' 5 s, inside the page's minute
const VERSION_DEADLINE: Duration = Duration::from_secs(40);
/// 1.14.7's is about 1 MB packed (`epic::manifest::MAX_DATA` caps it unpacked)
pub const MANIFEST_MAX_BYTES: usize = 16 << 20;

/// A game version and what the launcher needs to install and start it: a module manifest, narrowed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownProfile {
    pub game_version: GameVersion,
    pub display_name: String,
    pub client: ClientProfile,
}

/// A profile a server served, and whether that server also holds the version's Epic manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServedProfile {
    pub profile: KnownProfile,
    pub epic_manifest: bool,
}

/// A server this PC joined, with the account key made for it: the key goes to this server's Epic manifest
/// route and nowhere else, and never into a log.
#[derive(Clone, PartialEq, Eq)]
pub struct JoinedServer {
    pub endpoint: Endpoint,
    pub key: Option<String>,
}

impl std::fmt::Debug for JoinedServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JoinedServer")
            .field("endpoint", &self.endpoint)
            .field("key", &self.key.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// Where a list's manifests and server profiles come from: the network, or a test's stand-in.
pub trait Sources: Sync {
    /// `GET /versions`: the versions a joined server's modules serve; none when it serves none or doesn't
    /// answer.
    fn server_versions(&self, server: &JoinedServer) -> impl Future<Output = Vec<parse::ServedVersion>> + Send;
    /// `GET /versions/<version>/client`, unchecked.
    fn server_client(
        &self,
        server: &JoinedServer,
        version: &GameVersion,
    ) -> impl Future<Output = Option<ClientProfile>> + Send;
    /// `GET /versions/<version>/epic/manifest` with the key, unchecked; none for any refusal.
    fn server_manifest(
        &self,
        server: &JoinedServer,
        version: &GameVersion,
    ) -> impl Future<Output = Option<Bytes>> + Send;
    /// An `https` source's copy, unchecked.
    fn url(&self, url: &str) -> impl Future<Output = Option<Bytes>> + Send;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sizes {
    pub download: u64,
    pub install: u64,
}

/// What a manifest gave, by pin. Only sizes are kept: a source that failed is asked again next time.
#[derive(Default)]
pub struct ManifestCache(Mutex<HashMap<[u8; 20], Sizes>>);

impl ManifestCache {
    fn get(&self, pin: &Sha1Digest) -> Option<Sizes> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&pin.0)
            .copied()
    }

    fn put(&self, pin: &Sha1Digest, sizes: Sizes) {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(pin.0, sizes);
    }
}

/// One version on the list while it is being made.
struct Candidate {
    profile: KnownProfile,
    origin: GameVersionOrigin,
    /// The joined servers (by index) that hold this version's Epic manifest, under this pin
    servers: Vec<usize>,
}

fn same_pin(a: &ClientProfile, b: &ClientProfile) -> bool {
    match (&a.epic, &b.epic) {
        (Some(a), Some(b)) => a.manifest_sha1 == b.manifest_sha1,
        (None, None) => true,
        _ => false,
    }
}

/// The compiled-in profiles, then each server's in the order given (the current server first): a version
/// is listed once, from the first that names it.
fn merge(compiled_in: Vec<KnownProfile>, served: Vec<Vec<ServedProfile>>) -> Vec<Candidate> {
    let mut list: Vec<Candidate> = compiled_in
        .into_iter()
        .map(|profile| Candidate {
            profile,
            origin: GameVersionOrigin::Launcher,
            servers: Vec::new(),
        })
        .collect();
    for (server, profiles) in served.into_iter().enumerate() {
        for ServedProfile { profile, epic_manifest } in profiles {
            let holds = if epic_manifest { vec![server] } else { Vec::new() };
            match list.iter_mut().find(|c| c.profile.game_version == profile.game_version) {
                Some(known) if same_pin(&known.profile.client, &profile.client) => known.servers.extend(holds),
                Some(_) => {}
                None => list.push(Candidate {
                    profile,
                    origin: GameVersionOrigin::Server,
                    servers: holds,
                }),
            }
        }
    }
    list.sort_by(|a, b| b.profile.game_version.cmp(&a.profile.game_version));
    list
}

enum Read {
    Sizes(Sizes),
    /// The bytes don't hash to the pin, or aren't the build the profile names
    Invalid,
}

fn read(bytes: &[u8], pin: &Sha1Digest, build: &str) -> Read {
    match Manifest::parse(bytes, pin) {
        Ok(manifest) if manifest.app_name == epic::library::APP_NAME && manifest.build_version == build => {
            Read::Sizes(Sizes {
                download: manifest.download_size(),
                install: manifest.install_size(),
            })
        }
        _ => Read::Invalid,
    }
}

/// The sizes of one Epic build: from the cache, or from the first source whose copy checks out.
async fn sizes<S: Sources>(
    sources: &S,
    servers: &[JoinedServer],
    candidate: &Candidate,
    cache: &ManifestCache,
) -> (EpicManifestState, Option<Sizes>) {
    let Some(epic) = &candidate.profile.client.epic else {
        return (EpicManifestState::Invalid, None);
    };
    let pin = Sha1Digest(*epic.manifest_sha1.as_bytes());
    if let Some(sizes) = cache.get(&pin) {
        return (EpicManifestState::Ready, Some(sizes));
    }
    let mut answered = false;
    let from_servers = candidate.servers.iter().filter_map(|i| servers.get(*i));
    for server in from_servers {
        if let Some(bytes) = sources.server_manifest(server, &candidate.profile.game_version).await {
            answered = true;
            if let Read::Sizes(sizes) = read(&bytes, &pin, &epic.version) {
                cache.put(&pin, sizes);
                return (EpicManifestState::Ready, Some(sizes));
            }
        }
    }
    for url in &epic.manifest_sources {
        if let Some(bytes) = sources.url(url).await {
            answered = true;
            if let Read::Sizes(sizes) = read(&bytes, &pin, &epic.version) {
                cache.put(&pin, sizes);
                return (EpicManifestState::Ready, Some(sizes));
            }
        }
    }
    let state = if answered {
        EpicManifestState::Invalid
    } else {
        EpicManifestState::Unreachable
    };
    (state, None)
}

/// A server's profiles: its version list, then the `/client` of each version that has one (all at once).
/// An entry that fails the module manifest's own checks is dropped on its own.
async fn server_profiles<S: Sources>(sources: &S, server: &JoinedServer) -> Vec<ServedProfile> {
    let rows = sources.server_versions(server).await;
    let profiles = join_all(rows.iter().map(|row| async {
        if !row.client {
            return None;
        }
        let client = sources.server_client(server, &row.game_version).await?;
        parse::checked(row, client)
    }))
    .await;
    rows.iter()
        .zip(profiles)
        .filter_map(|(row, profile)| {
            profile.map(|profile| ServedProfile {
                profile,
                epic_manifest: row.epic_manifest,
            })
        })
        .collect()
}

/// The list: every server asked for its profiles at once, then every Epic build's sizes at once.
pub async fn list<S: Sources>(
    sources: &S,
    compiled_in: Vec<KnownProfile>,
    servers: &[JoinedServer],
    cache: &ManifestCache,
) -> Vec<GameVersionEntry> {
    let served = join_all(servers.iter().map(|server| server_profiles(sources, server))).await;
    let candidates = merge(compiled_in, served);
    let read = join_all(candidates.iter().map(|candidate| async {
        tokio::time::timeout(VERSION_DEADLINE, sizes(sources, servers, candidate, cache))
            .await
            .unwrap_or((EpicManifestState::Unreachable, None))
    }))
    .await;
    candidates
        .into_iter()
        .zip(read)
        .map(|(candidate, (manifest, found))| {
            let epic = candidate.profile.client.epic.map(|epic| EpicVersionInfo {
                build_version: epic.version,
                manifest,
                download_bytes: found.map(|s| s.download),
                install_bytes: found.map(|s| s.install),
            });
            GameVersionEntry {
                version: candidate.profile.game_version.to_string(),
                display_name: candidate.profile.display_name,
                origin: candidate.origin,
                install: if epic.is_some() {
                    GameVersionInstall::Epic
                } else {
                    GameVersionInstall::Locate
                },
                epic,
            }
        })
        .collect()
}

type HttpsClient =
    Client<hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>, Empty<Bytes>>;

/// The network: joined servers through `net::http` (pinned TLS in public mode; the key only on the Epic
/// manifest route), and the profiles' `https` sources with the webpki roots, no redirect followed.
pub struct HttpSources {
    client: Option<HttpsClient>,
}

impl Default for HttpSources {
    fn default() -> Self {
        let client = hyper_rustls::HttpsConnectorBuilder::new()
            .with_provider_and_webpki_roots(rustls::crypto::aws_lc_rs::default_provider())
            .ok()
            .map(|builder| builder.https_only().enable_http1().build())
            .map(|connector| Client::builder(TokioExecutor::new()).build(connector));
        Self { client }
    }
}

impl Sources for HttpSources {
    async fn server_versions(&self, server: &JoinedServer) -> Vec<parse::ServedVersion> {
        let options = RequestOptions::get(PROFILES_TIMEOUT, PROFILES_MAX_BYTES);
        match request(&server.endpoint, VERSIONS_PATH, options).await {
            Ok(response) if response.status == 200 => parse::read_versions(parse_json_body(&response).as_ref()),
            _ => Vec::new(),
        }
    }

    async fn server_client(&self, server: &JoinedServer, version: &GameVersion) -> Option<ClientProfile> {
        let options = RequestOptions::get(PROFILES_TIMEOUT, PROFILES_MAX_BYTES);
        match request(&server.endpoint, &server_client_path(version), options).await {
            Ok(response) if response.status == 200 => parse::read_client(parse_json_body(&response).as_ref()),
            _ => None,
        }
    }

    async fn server_manifest(&self, server: &JoinedServer, version: &GameVersion) -> Option<Bytes> {
        // The route needs the key, and `key_headers` sends it only over the pinned or private connection
        let mut options = RequestOptions::get(MANIFEST_TIMEOUT, MANIFEST_MAX_BYTES);
        options.headers = key_headers(&server.endpoint, server.key.as_deref());
        if options.headers.is_empty() {
            return None;
        }
        options.headers.push(("accept", "application/octet-stream".to_owned()));
        match request(&server.endpoint, &server_manifest_path(version), options).await {
            Ok(response) if response.status == 200 => Some(response.body),
            _ => None,
        }
    }

    async fn url(&self, url: &str) -> Option<Bytes> {
        let client = self.client.as_ref()?;
        if !url.starts_with("https://") {
            return None;
        }
        let request = hyper::Request::get(url).body(Empty::new()).ok()?;
        let work = async {
            let response = client.request(request).await.ok()?;
            if response.status() != hyper::StatusCode::OK {
                return None;
            }
            Limited::new(response.into_body(), MANIFEST_MAX_BYTES)
                .collect()
                .await
                .ok()
                .map(|body| body.to_bytes())
        };
        tokio::time::timeout(MANIFEST_TIMEOUT, work).await.ok().flatten()
    }
}
