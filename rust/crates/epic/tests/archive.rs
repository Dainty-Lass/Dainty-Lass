//! The archived manifests and Epic's CDN, both opt-in, since they reach the network.
//!
//! - `REFORGED_TEST_EPIC_MANIFEST=1` fetches the 1.14.7 and 1.12.0 manifests from the community archive by pinned
//!   commit and SHA-1 and checks what DE5 relies on. They are Epic's files, so they are fetched, not committed.
//! - `REFORGED_TEST_EPIC_CDN=1` is the maintainer's DE1 step (`docs/port/spike-de1-epic.md`): it fetches a few of
//!   1.14.7's chunks from each CDN host, checks them against the manifest and assembles `Version.txt` from them.
//!
//! HTTPS only, no redirect followed, bodies capped.

use std::time::Duration;

use bytes::Bytes;
use epic::chunk;
use epic::download::{CDN_BASES, chunk_url};
use epic::manifest::{Manifest, Sha1Digest};
use http_body_util::{BodyExt, Empty, Limited};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;

const ARCHIVE: &str = "https://raw.githubusercontent.com/addison2k/DauntlessManifestArchive";

struct Pinned {
    url: String,
    sha1: &'static str,
}

/// 1.14.7, added in the archive's first commit; its SHA-1 is the one Mystic's guide pins
fn manifest_1_14_7() -> Pinned {
    Pinned {
        url: format!(
            "{ARCHIVE}/92e6082335b1c2234c88e9aec7fe2b704fc24494/manifests/Windows/Dauntless_1.14.7.647472.manifest"
        ),
        sha1: "aa70748e4c0a776a991f2bac10ad342adaa31476",
    }
}

fn manifest_1_12_0() -> Pinned {
    Pinned {
        url: format!(
            "{ARCHIVE}/2d29ce97939a3e848e68f4b26bf5e285dcc5a70b/manifests/Windows/Dauntless_1.12.0.392819.manifest"
        ),
        sha1: "46b4097c070afc29ed783e77a68ffa936b6a3504",
    }
}

fn opted_in(var: &str, what: &str) -> bool {
    let on = std::env::var(var).as_deref() == Ok("1");
    if !on {
        eprintln!("set {var}=1 to {what}");
    }
    on
}

fn block_on<T>(work: impl Future<Output = T>) -> T {
    match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime.block_on(work),
        Err(e) => panic!("no runtime: {e}"),
    }
}

/// A GET over HTTPS that refuses anything but `200` (redirects included) and bodies over `max` bytes.
async fn get(url: &str, max: usize) -> Result<Bytes, String> {
    let connector = hyper_rustls::HttpsConnectorBuilder::new()
        .with_provider_and_webpki_roots(rustls::crypto::aws_lc_rs::default_provider())
        .map_err(|e| e.to_string())?
        .https_only()
        .enable_http1()
        .build();
    let client: Client<_, Empty<Bytes>> = Client::builder(TokioExecutor::new()).build(connector);
    let request = hyper::Request::get(url).body(Empty::new()).map_err(|e| e.to_string())?;
    let work = async {
        let response = client.request(request).await.map_err(|e| format!("{url}: {e}"))?;
        if response.status() != hyper::StatusCode::OK {
            return Err(format!("{url}: HTTP {}", response.status()));
        }
        Limited::new(response.into_body(), max)
            .collect()
            .await
            .map(|b| b.to_bytes())
            .map_err(|e| format!("{url}: {e}"))
    };
    tokio::time::timeout(Duration::from_secs(120), work)
        .await
        .map_err(|_| format!("{url}: timed out"))?
}

async fn fetch_manifest(pinned: &Pinned) -> Result<Manifest, String> {
    let bytes = get(&pinned.url, 16 << 20).await?;
    let pin = pinned.sha1.parse().map_err(|e| format!("{e}"))?;
    Manifest::parse(&bytes, &pin).map_err(|e| format!("{}: {e}", pinned.url))
}

#[test]
fn the_archived_manifests_parse_as_the_archive_describes_them() {
    if !opted_in(
        "REFORGED_TEST_EPIC_MANIFEST",
        "fetch the archived 1.14.7 and 1.12.0 manifests",
    ) {
        return;
    }
    block_on(async {
        let m = fetch_manifest(&manifest_1_14_7()).await.unwrap();
        assert_eq!(m.app_name, "Jackal");
        assert_eq!(m.build_version, "1.14.7.647472");
        assert_eq!(m.launch_exe, "Dauntless.exe");
        assert_eq!(m.feature_level, 17);
        // Mystic's check and the archive's table: 611 files, 15.39 GiB installed, 14.89 GiB to download
        assert_eq!(m.files().len(), 611);
        assert_eq!(m.chunks().len(), 15_749);
        assert_eq!(m.install_size(), 16_521_629_423);
        assert_eq!(m.download_size(), 15_985_792_608);
        let shipping = m.file("Archon/Binaries/Win64/Dauntless-Win64-Shipping.exe").unwrap();
        assert_eq!(shipping.size, 121_407_264);
        assert_eq!(shipping.sha1.to_string(), "739e69ac336ff107ed263cca1de321b8d1236bd3");
        assert_eq!(m.file("Version.txt").unwrap().size, 59);
        for file in m.files() {
            assert_eq!(file.size, file.parts.iter().map(|p| u64::from(p.size)).sum::<u64>());
            assert!(file.parts.iter().all(|p| m.chunk(&p.guid).is_some()));
        }

        let m = fetch_manifest(&manifest_1_12_0()).await.unwrap();
        assert_eq!(m.app_name, "Jackal");
        assert!(m.build_version.starts_with("1.12.0."), "{}", m.build_version);
        assert_eq!(m.files().len(), 609);
        assert!(m.file("Archon/Binaries/Win64/Dauntless-Win64-Shipping.exe").is_some());
    });
}

/// Downloads game data (a few MB per host): the maintainer runs it, from an account that owns Dauntless (DD22).
#[test]
fn chunks_from_every_cdn_host_hash_as_the_manifest_says() {
    if !opted_in(
        "REFORGED_TEST_EPIC_CDN",
        "fetch a few 1.14.7 chunks from each Epic CDN host",
    ) {
        return;
    }
    block_on(async {
        let m = fetch_manifest(&manifest_1_14_7()).await.unwrap();
        let version_txt = m.file("Version.txt").unwrap();
        let launcher = m.file("Dauntless.exe").unwrap();
        let mut wanted: Vec<_> = version_txt.parts.iter().map(|p| p.guid).collect();
        wanted.extend(launcher.parts.iter().take(2).map(|p| p.guid));
        wanted.sort();
        wanted.dedup();
        for base in CDN_BASES {
            let mut decoded = Vec::new();
            for guid in &wanted {
                let info = m.chunk(guid).unwrap();
                let url = chunk_url(base, &m, info);
                let raw = get(&url, info.download_size as usize + 1024).await.unwrap();
                let data = chunk::decode(&raw, info).unwrap_or_else(|e| panic!("{url}: {e}"));
                eprintln!("ok {url} ({} bytes, {} unpacked)", raw.len(), data.len());
                decoded.push((*guid, data));
            }
            let assembled: Vec<u8> = version_txt
                .parts
                .iter()
                .flat_map(|p| {
                    let (_, data) = decoded.iter().find(|(g, _)| *g == p.guid).unwrap();
                    data[p.offset as usize..(p.offset + p.size) as usize].to_vec()
                })
                .collect();
            assert_eq!(Sha1Digest::of(&assembled), version_txt.sha1, "{base}");
            eprintln!(
                "Version.txt from {base}: {}",
                String::from_utf8_lossy(&assembled).trim()
            );
        }
    });
}
