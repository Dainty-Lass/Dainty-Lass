// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The list against synthetic manifests and stand-in sources. The archived manifests are Epic's files: the
//! opt-in test fetches them (`DAINTY_TEST_EPIC_MANIFEST=1`), as `crates/epic/tests/archive.rs` does.

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use epic::manifest::Sha1Digest;
use http_body_util::Full;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use module_api::{ClientProfile, EpicBuild, GameVersion, Sha1Hex};
use serde_json::json;
use tokio::net::TcpListener;

use self::parse::ServedVersion;
use super::*;
use crate::bridge::types::{EpicManifestState, GameVersionInstall, GameVersionOrigin};

// ---------------------------------------------------------------- synthetic manifests

fn fstring(out: &mut Vec<u8>, text: &str) {
    if text.is_empty() {
        out.extend(0i32.to_le_bytes());
    } else {
        out.extend((text.len() as i32 + 1).to_le_bytes());
        out.extend(text.as_bytes());
        out.push(0);
    }
}

fn section(out: &mut Vec<u8>, body: &[u8]) {
    out.extend((5 + body.len() as u32).to_le_bytes());
    out.push(0);
    out.extend(body);
}

/// A stored (unpacked) manifest of data version 0: `chunks` are (window size, download size), each file
/// one part of `size` bytes from chunk `index`.
fn manifest(build: &str, chunks: &[(u32, i64)], files: &[(&str, usize, u32)]) -> Vec<u8> {
    let guid = |i: usize| [i as u32 + 1, 2, 3, 4];
    let mut data = Vec::new();

    let mut meta = Vec::new();
    meta.extend(18u32.to_le_bytes());
    meta.push(0);
    meta.extend(7u32.to_le_bytes());
    for text in ["Jackal", build, "Dauntless.exe", ""] {
        fstring(&mut meta, text);
    }
    meta.extend(0u32.to_le_bytes());
    for _ in 0..3 {
        fstring(&mut meta, "");
    }
    section(&mut data, &meta);

    let mut list = Vec::new();
    list.extend((chunks.len() as u32).to_le_bytes());
    (0..chunks.len()).for_each(|i| guid(i).iter().for_each(|v| list.extend(v.to_le_bytes())));
    chunks.iter().for_each(|_| list.extend(0u64.to_le_bytes()));
    chunks.iter().for_each(|_| list.extend([0u8; 20]));
    chunks.iter().for_each(|_| list.push(0));
    chunks.iter().for_each(|(window, _)| list.extend(window.to_le_bytes()));
    chunks
        .iter()
        .for_each(|(_, download)| list.extend(download.to_le_bytes()));
    section(&mut data, &list);

    let mut fml = Vec::new();
    fml.extend((files.len() as u32).to_le_bytes());
    files.iter().for_each(|(name, _, _)| fstring(&mut fml, name));
    files.iter().for_each(|_| fstring(&mut fml, ""));
    files.iter().for_each(|_| fml.extend([0u8; 20]));
    files.iter().for_each(|_| fml.push(0));
    files.iter().for_each(|_| fml.extend(0u32.to_le_bytes()));
    for (_, chunk, size) in files {
        fml.extend(1u32.to_le_bytes());
        fml.extend(28u32.to_le_bytes());
        guid(*chunk).iter().for_each(|v| fml.extend(v.to_le_bytes()));
        fml.extend(0u32.to_le_bytes());
        fml.extend(size.to_le_bytes());
    }
    section(&mut data, &fml);

    let mut out = Vec::new();
    out.extend(0x44BE_C00Cu32.to_le_bytes());
    out.extend(41u32.to_le_bytes());
    out.extend((data.len() as u32).to_le_bytes());
    out.extend((data.len() as u32).to_le_bytes());
    out.extend(Sha1Digest::of(&data).0);
    out.push(0);
    out.extend(18u32.to_le_bytes());
    out.extend(data);
    out
}

/// 1.14.7-like: two chunks, three files; 1.12.0-like: one chunk, one file
fn manifest_a() -> Vec<u8> {
    manifest(
        "1.14.7.647472",
        &[(1000, 600), (800, 500)],
        &[
            ("Version.txt", 0, 59),
            ("Dauntless.exe", 0, 900),
            ("Archon/a.pak", 1, 800),
        ],
    )
}
const A_SIZES: Sizes = Sizes {
    download: 600 + 500,
    install: 59 + 900 + 800,
};

fn manifest_b() -> Vec<u8> {
    manifest("1.12.0.392819", &[(4096, 2048)], &[("Archon/b.pak", 0, 4000)])
}
const B_SIZES: Sizes = Sizes {
    download: 2048,
    install: 4000,
};

fn pin_of(bytes: &[u8]) -> Sha1Hex {
    Sha1Hex::from_bytes(Sha1Digest::of(bytes).0)
}

fn profile(version: &str, name: &str, epic: Option<(&str, &[u8], &[&str])>) -> KnownProfile {
    KnownProfile {
        game_version: version.parse().unwrap(),
        display_name: name.into(),
        client: ClientProfile {
            executable: "Archon/Binaries/Win64/Dauntless-Win64-Shipping.exe".into(),
            executable_sha256: None,
            files: vec![],
            arguments: vec![],
            environment: std::collections::BTreeMap::new(),
            ini_edits: vec![],
            epic: epic.map(|(build, bytes, sources)| EpicBuild {
                version: build.into(),
                manifest_sha1: pin_of(bytes),
                manifest_sources: sources.iter().map(|s| (*s).to_owned()).collect(),
                executable_sha256: "ab".repeat(32).parse().unwrap(),
            }),
        },
    }
}

const SRC_A: &str = "https://archive.example/a.manifest";
const SRC_A2: &str = "https://module.example/a.manifest";
const SRC_B: &str = "https://archive.example/b.manifest";

fn compiled_in() -> Vec<KnownProfile> {
    vec![
        profile("1.4.4", "Dauntless Revived", None),
        profile(
            "1.14.7",
            "Rustic Isles 1.14.7",
            Some(("1.14.7.647472", &manifest_a(), &[SRC_A])),
        ),
        profile(
            "1.12.0",
            "Rustic Isles 1.12",
            Some(("1.12.0.392819", &manifest_b(), &[SRC_B])),
        ),
    ]
}

// ---------------------------------------------------------------- stand-in sources

// Built from pieces, so the repository's secret check never sees a whole key
const KEY: &str = concat!("DLK_", "dddddddddddddddd", "dddddddddddddddd", "dddddddddddddddd");

/// What a joined server's content holds, as DF4b's routes give it; each request is written down by its
/// route, so the tests see which routes were asked in which order.
#[derive(Default)]
struct Fake {
    /// By host: the rows of `/versions`, each with its profile
    versions: HashMap<String, Vec<(ServedVersion, ClientProfile)>>,
    /// By "host version": the Epic manifest the server holds
    server_copies: HashMap<String, Vec<u8>>,
    urls: HashMap<String, Vec<u8>>,
    asked: Mutex<Vec<String>>,
}

impl Fake {
    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }

    /// A server serving these profiles; `epic_manifest` says which it also holds the manifest for.
    fn serve(&mut self, host: &str, profiles: Vec<(KnownProfile, bool)>) {
        let rows = profiles
            .into_iter()
            .map(|(profile, epic_manifest)| {
                let row = ServedVersion {
                    game_version: profile.game_version,
                    display_name: profile.display_name,
                    client: true,
                    epic_manifest,
                };
                (row, profile.client)
            })
            .collect();
        self.versions.insert(host.into(), rows);
    }

    fn note(&self, server: &JoinedServer, path: &str) {
        self.asked
            .lock()
            .unwrap()
            .push(format!("{} {path}", server.endpoint.host));
    }
}

impl Sources for Fake {
    async fn server_versions(&self, server: &JoinedServer) -> Vec<ServedVersion> {
        self.note(server, VERSIONS_PATH);
        let rows = self.versions.get(&server.endpoint.host);
        rows.map(|rows| rows.iter().map(|(row, _)| row.clone()).collect())
            .unwrap_or_default()
    }

    async fn server_client(&self, server: &JoinedServer, version: &GameVersion) -> Option<ClientProfile> {
        self.note(server, &server_client_path(version));
        let rows = self.versions.get(&server.endpoint.host)?;
        rows.iter()
            .find(|(row, _)| row.game_version == *version)
            .map(|(_, client)| client.clone())
    }

    /// The route needs the key: without it the server answers 401, which is none
    async fn server_manifest(&self, server: &JoinedServer, version: &GameVersion) -> Option<Bytes> {
        self.note(server, &server_manifest_path(version));
        server.key.as_ref()?;
        let key = format!("{} {version}", server.endpoint.host);
        self.server_copies.get(&key).cloned().map(Bytes::from)
    }

    async fn url(&self, url: &str) -> Option<Bytes> {
        self.asked.lock().unwrap().push(url.to_owned());
        self.urls.get(url).cloned().map(Bytes::from)
    }
}

fn server(host: &str) -> JoinedServer {
    JoinedServer {
        endpoint: Endpoint {
            host: host.into(),
            port: 61000,
            pin: None,
        },
        key: Some(KEY.into()),
    }
}

/// The list with a cache of its own
async fn list_with<S: Sources>(
    sources: &S,
    compiled_in: Vec<KnownProfile>,
    servers: &[JoinedServer],
) -> Vec<GameVersionEntry> {
    list(sources, compiled_in, servers, &ManifestCache::default()).await
}

fn block_on<T>(work: impl Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(work)
}

fn sizes_of(entry: &GameVersionEntry) -> Option<Sizes> {
    let epic = entry.epic.as_ref()?;
    Some(Sizes {
        download: epic.download_bytes?,
        install: epic.install_bytes?,
    })
}

fn state_of(entry: &GameVersionEntry) -> Option<EpicManifestState> {
    entry.epic.as_ref().map(|epic| epic.manifest)
}

// ---------------------------------------------------------------- the list

#[test]
fn the_list_is_the_profiles_newest_first_with_the_manifests_sizes() {
    let mut fake = Fake::default();
    fake.urls.insert(SRC_A.into(), manifest_a());
    fake.urls.insert(SRC_B.into(), manifest_b());
    let cache = ManifestCache::default();
    let list = block_on(list(&fake, compiled_in(), &[], &cache));

    let versions: Vec<(&str, &str)> = list
        .iter()
        .map(|e| (e.version.as_str(), e.display_name.as_str()))
        .collect();
    assert_eq!(
        versions,
        [
            ("1.14.7", "Rustic Isles 1.14.7"),
            ("1.12.0", "Rustic Isles 1.12"),
            ("1.4.4", "Dauntless Revived")
        ]
    );
    assert!(list.iter().all(|e| e.origin == GameVersionOrigin::Launcher));
    assert_eq!(list[0].install, GameVersionInstall::Epic);
    assert_eq!(list[0].epic.as_ref().unwrap().build_version, "1.14.7.647472");
    assert_eq!(state_of(&list[0]), Some(EpicManifestState::Ready));
    assert_eq!(sizes_of(&list[0]), Some(A_SIZES));
    assert_eq!(sizes_of(&list[1]), Some(B_SIZES));
    assert_eq!(list[2].install, GameVersionInstall::Locate);
    assert_eq!(list[2].epic, None);
}

#[test]
fn manifests_are_fetched_once_and_kept_by_pin() {
    let mut fake = Fake::default();
    fake.urls.insert(SRC_A.into(), manifest_a());
    fake.urls.insert(SRC_B.into(), manifest_b());
    let cache = ManifestCache::default();
    block_on(list(&fake, compiled_in(), &[], &cache));
    assert_eq!(fake.asked(), [SRC_A, SRC_B]);
    let again = block_on(list(&fake, compiled_in(), &[], &cache));
    assert_eq!(fake.asked().len(), 2, "nothing fetched the second time");
    assert_eq!(sizes_of(&again[0]), Some(A_SIZES));
}

#[test]
fn a_failed_manifest_says_why_and_is_asked_again_next_time() {
    let mut fake = Fake::default();
    // A's source answers with B's file: wrong pin. B's source doesn't answer at all
    fake.urls.insert(SRC_A.into(), manifest_b());
    let cache = ManifestCache::default();
    let list = block_on(list(&fake, compiled_in(), &[], &cache));
    assert_eq!(state_of(&list[0]), Some(EpicManifestState::Invalid));
    assert_eq!(state_of(&list[1]), Some(EpicManifestState::Unreachable));
    assert_eq!(sizes_of(&list[0]), None);
    assert_eq!(list[0].epic.as_ref().unwrap().download_bytes, None);
    block_on(super::list(&fake, compiled_in(), &[], &cache));
    assert_eq!(fake.asked().len(), 4);
}

#[test]
fn a_pinned_file_of_another_build_is_refused() {
    let other = manifest("1.14.6.615145", &[(10, 10)], &[("x", 0, 10)]);
    let mut fake = Fake::default();
    fake.urls.insert(SRC_A.into(), other.clone());
    let profiles = vec![profile(
        "1.14.7",
        "Rustic Isles 1.14.7",
        Some(("1.14.7.647472", &other, &[SRC_A])),
    )];
    let list = block_on(list(&fake, profiles, &[], &ManifestCache::default()));
    assert_eq!(state_of(&list[0]), Some(EpicManifestState::Invalid));
}

#[test]
fn the_sources_are_tried_in_order_after_a_server_that_serves_the_version() {
    let mut fake = Fake::default();
    let profiles = vec![profile(
        "1.14.7",
        "Rustic Isles 1.14.7",
        Some(("1.14.7.647472", &manifest_a(), &[SRC_A2, SRC_A])),
    )];
    // "other" lists the version without holding its manifest; "home" holds a corrupt one
    fake.serve("other", vec![(profiles[0].clone(), false)]);
    fake.serve("home", vec![(profiles[0].clone(), true)]);
    let mut corrupt = manifest_a();
    let last = corrupt.len() - 1;
    corrupt[last] ^= 1;
    fake.server_copies.insert("home 1.14.7".into(), corrupt);
    // The module's own copy is missing, the archive's is right
    fake.urls.insert(SRC_A.into(), manifest_a());
    let list = block_on(list(
        &fake,
        profiles,
        &[server("other"), server("home")],
        &ManifestCache::default(),
    ));
    assert_eq!(sizes_of(&list[0]), Some(A_SIZES));
    let versions = "/content/v1/versions";
    let client = "/content/v1/versions/1.14.7/client";
    let asked = fake.asked();
    // Each server's list, then its profile (the servers run side by side); then only "home", which holds
    // the manifest, is asked for it, and the sources follow in order
    assert_eq!(
        asked,
        [
            format!("other {versions}"),
            format!("other {client}"),
            format!("home {versions}"),
            format!("home {client}"),
            "home /content/v1/versions/1.14.7/epic/manifest".to_owned(),
            SRC_A2.to_owned(),
            SRC_A.to_owned()
        ]
    );
}

#[test]
fn a_server_without_the_key_is_never_asked_to_prove_a_manifest_it_would_refuse() {
    let mut fake = Fake::default();
    let profiles = vec![profile(
        "1.14.7",
        "Rustic Isles 1.14.7",
        Some(("1.14.7.647472", &manifest_a(), &[SRC_A])),
    )];
    fake.serve("home", vec![(profiles[0].clone(), true)]);
    fake.server_copies.insert("home 1.14.7".into(), manifest_a());
    fake.urls.insert(SRC_A.into(), manifest_b());
    let mut keyless = server("home");
    keyless.key = None;
    let list = block_on(list(&fake, profiles.clone(), &[keyless], &ManifestCache::default()));
    // 401 for want of a key: the archive is tried, and its file is not the pinned one
    assert_eq!(state_of(&list[0]), Some(EpicManifestState::Invalid));
    // With the key the server's copy is taken and the archive is never asked
    let list = block_on(list_with(&fake, profiles, &[server("home")]));
    assert_eq!(sizes_of(&list[0]), Some(A_SIZES));
    assert_eq!(fake.asked().iter().filter(|path| path.as_str() == SRC_A).count(), 1);
}

#[test]
fn a_joined_server_prints_without_its_key() {
    let shown = format!("{:?}", server("home"));
    assert!(shown.contains("<redacted>") && !shown.contains("DLK_"), "{shown}");
}

#[test]
fn servers_add_versions_but_cannot_repin_a_compiled_in_one() {
    let mut fake = Fake::default();
    let repinned = profile(
        "1.14.7",
        "Somebody's 1.14.7",
        Some(("1.14.7.647472", b"another file", &[SRC_A])),
    );
    let extra_bytes = manifest("2.1.1.682875", &[(64, 32)], &[("y", 0, 64)]);
    let extra = profile(
        "2.1.1",
        "Rustic Isles 2.1.1",
        Some(("2.1.1.682875", &extra_bytes, &["https://archive.example/c.manifest"])),
    );
    let revived = profile("1.4.4", "Somebody's Revived", None);
    fake.serve(
        "first",
        vec![(repinned, true), (extra.clone(), false), (revived, false)],
    );
    let mut renamed = extra.clone();
    renamed.display_name = "Second's 2.1.1".into();
    fake.serve("second", vec![(renamed, true)]);
    fake.server_copies.insert("second 2.1.1".into(), extra_bytes);
    fake.urls.insert(SRC_A.into(), manifest_a());

    let list = block_on(list(
        &fake,
        compiled_in(),
        &[server("first"), server("second")],
        &ManifestCache::default(),
    ));
    let names: Vec<(&str, &str, GameVersionOrigin)> = list
        .iter()
        .map(|e| (e.version.as_str(), e.display_name.as_str(), e.origin))
        .collect();
    assert_eq!(
        names,
        [
            ("2.1.1", "Rustic Isles 2.1.1", GameVersionOrigin::Server),
            ("1.14.7", "Rustic Isles 1.14.7", GameVersionOrigin::Launcher),
            ("1.12.0", "Rustic Isles 1.12", GameVersionOrigin::Launcher),
            ("1.4.4", "Dauntless Revived", GameVersionOrigin::Launcher),
        ]
    );
    // The compiled-in pin held: first's 1.14.7 copy was never asked for
    assert_eq!(sizes_of(&list[1]), Some(A_SIZES));
    assert!(
        !fake
            .asked()
            .contains(&"first /content/v1/versions/1.14.7/epic/manifest".to_owned())
    );
    // Both servers serve 2.1.1 with the same pin; the second had the copy
    assert_eq!(
        sizes_of(&list[0]),
        Some(Sizes {
            download: 32,
            install: 64
        })
    );
}

// ---------------------------------------------------------------- the served versions

fn row(version: &str, client: bool, epic_manifest: bool) -> serde_json::Value {
    json!({
        "game_version": version, "module": format!("isles-{version}"), "display_name": format!("Rustic Isles {version}"),
        "invite_aliases": ["reforged"], "unverified": false, "client": client, "epic_manifest": epic_manifest,
    })
}

fn served_client(source: &str) -> serde_json::Value {
    json!({
        "executable": "Archon/Binaries/Win64/Dauntless-Win64-Shipping.exe",
        "epic": {
            "version": "1.14.7.647472",
            "manifest_sha1": "aa70748e4c0a776a991f2bac10ad342adaa31476",
            "manifest_sources": [source],
            "executable_sha256": "ab".repeat(32),
        }
    })
}

#[test]
fn the_version_list_reads_the_rows_df4b_serves() {
    let body = json!({ "default_version": "1.14.7", "versions": [
        row("1.14.7", true, true),
        row("1.14.7", true, false),
        row("1.12.0", false, false),
        { "game_version": "1.x", "display_name": "x", "client": true },
        { "display_name": "no version" },
        { "game_version": "2.0.0", "display_name": "Bare", "future_field": [1] },
    ] });
    let rows = parse::read_versions(Some(&body));
    let read: Vec<(String, bool, bool)> = rows
        .iter()
        .map(|r| (r.game_version.to_string(), r.client, r.epic_manifest))
        .collect();
    assert_eq!(
        read,
        [
            ("1.14.7".to_owned(), true, true),
            ("1.12.0".to_owned(), false, false),
            ("2.0.0".to_owned(), false, false)
        ]
    );
    assert_eq!(rows[0].display_name, "Rustic Isles 1.14.7");

    for nothing in [
        json!(null),
        json!([]),
        json!({ "versions": {} }),
        json!({ "default_version": "1.14.7" }),
    ] {
        assert!(parse::read_versions(Some(&nothing)).is_empty(), "{nothing}");
    }
    assert!(parse::read_versions(None).is_empty());
    let many: Vec<_> = (0..40).map(|i| row(&format!("1.{i}.0"), true, false)).collect();
    assert_eq!(
        parse::read_versions(Some(&json!({ "versions": many }))).len(),
        parse::MAX_VERSIONS
    );
}

#[test]
fn a_served_profile_is_checked_as_a_module_manifest_is() {
    let version = |v: &str, name: &str| ServedVersion {
        game_version: v.parse().unwrap(),
        display_name: name.into(),
        client: true,
        epic_manifest: false,
    };
    let read = |value: serde_json::Value| parse::read_client(Some(&value));
    let good = read(served_client("https://a.example/m")).unwrap();
    let known = parse::checked(&version("1.14.7", "Rustic Isles 1.14.7"), good.clone()).unwrap();
    assert_eq!(known.game_version.to_string(), "1.14.7");
    assert_eq!(known.client, good);
    assert!(read(json!({ "executable": "a.exe" })).is_some_and(|c| c.epic.is_none()));

    // Refused by serde: not a profile, or a field no profile has
    assert!(read(json!({ "executable": "a.exe", "surprise": true })).is_none());
    assert!(read(json!({ "executable_sha256": "ab" })).is_none());
    assert!(read(json!([])).is_none());
    assert!(parse::read_client(None).is_none());
    // Refused by `validate`
    for bad in [
        read(served_client("http://plain.example/m")).unwrap(),
        read(served_client("file:///etc/passwd")).unwrap(),
        read(json!({ "executable": "../evil.exe" })).unwrap(),
        read(json!({ "executable": "C:/evil.exe" })).unwrap(),
    ] {
        assert!(parse::checked(&version("1.14.7", "x"), bad).is_none());
    }
    assert!(parse::checked(&version("1.14.7", "Line\nbreak"), good).is_none());
}

#[test]
fn a_version_without_a_profile_is_not_asked_for_one() {
    let mut fake = Fake::default();
    fake.serve("home", vec![(profile("2.0.0", "Rustic Isles 2.0.0", None), false)]);
    if let Some(rows) = fake.versions.get_mut("home") {
        rows[0].0.client = false;
    }
    let list = block_on(list_with(&fake, vec![], &[server("home")]));
    assert!(list.is_empty());
    assert_eq!(fake.asked(), ["home /content/v1/versions"]);
}

// ---------------------------------------------------------------- DF4b's routes, over HTTP

/// (path, key header) of every request the server saw
type Seen = Arc<Mutex<Vec<(String, Option<String>)>>>;

async fn content(request: hyper::Request<Incoming>, seen: Seen, manifest: Vec<u8>) -> hyper::Response<Full<Bytes>> {
    let path = request.uri().path().to_owned();
    let key = request
        .headers()
        .get("x-dainty-user-api-key")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    seen.lock().unwrap().push((path.clone(), key.clone()));
    let reply = |status: u16, kind: &str, body: Vec<u8>| {
        hyper::Response::builder()
            .status(status)
            .header("content-type", kind)
            .body(Full::new(Bytes::from(body)))
            .unwrap()
    };
    let json =
        |status: u16, value: serde_json::Value| reply(status, "application/json", value.to_string().into_bytes());
    // The profile's own source is a host that can't be reached: a client that asked it would show Unreachable
    let mut pinned = served_client("https://archive.invalid/never-asked.manifest");
    pinned["epic"]["manifest_sha1"] = json!(pin_of(&manifest).to_string());
    match path.as_str() {
        "/content/v1/versions" => json(
            200,
            json!({ "default_version": "1.14.7", "versions": [row("1.14.7", true, true), row("1.12.0", false, false)] }),
        ),
        "/content/v1/versions/1.14.7/client" => json(200, pinned),
        "/content/v1/versions/1.14.7/epic/manifest" if key.is_none() => json(401, json!({ "error": "unauthorized" })),
        "/content/v1/versions/1.14.7/epic/manifest" => reply(200, "application/octet-stream", manifest),
        _ => json(404, json!({ "error": "unknown_version" })),
    }
}

async fn content_server(manifest: Vec<u8>) -> (u16, Seen) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen: Seen = Arc::default();
    let record = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else { return };
            let (record, manifest) = (record.clone(), manifest.clone());
            tokio::spawn(async move {
                let service = service_fn(move |request| {
                    let (record, manifest) = (record.clone(), manifest.clone());
                    async move { Ok::<_, Infallible>(content(request, record, manifest).await) }
                });
                let http = hyper::server::conn::http1::Builder::new();
                let _ = http.serve_connection(TokioIo::new(tcp), service).await;
            });
        }
    });
    (port, seen)
}

fn loopback(port: u16, key: Option<&str>) -> JoinedServer {
    JoinedServer {
        endpoint: Endpoint {
            host: "127.0.0.1".into(),
            port,
            pin: None,
        },
        key: key.map(str::to_owned),
    }
}

#[test]
fn the_network_client_speaks_the_routes_and_sends_the_key_only_to_the_manifest() {
    block_on(async {
        let (port, seen) = content_server(manifest_a()).await;
        let list = list_with(&HttpSources::default(), vec![], &[loopback(port, Some(KEY))]).await;
        // 1.12.0 has no profile, so it is not listed; 1.14.7's profile came from the server, and so did its
        // manifest
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].version, "1.14.7");
        assert_eq!(list[0].origin, GameVersionOrigin::Server);
        assert_eq!(state_of(&list[0]), Some(EpicManifestState::Ready));
        assert_eq!(sizes_of(&list[0]), Some(A_SIZES));
        let seen = seen.lock().unwrap().clone();
        let paths: Vec<&str> = seen.iter().map(|(path, _)| path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "/content/v1/versions",
                "/content/v1/versions/1.14.7/client",
                "/content/v1/versions/1.14.7/epic/manifest"
            ]
        );
        let keyed: Vec<bool> = seen.iter().map(|(_, key)| key.as_deref() == Some(KEY)).collect();
        assert_eq!(keyed, [false, false, true]);

        // Without a key the manifest route is not asked at all: the server would only answer 401
        let (port, seen) = content_server(manifest_a()).await;
        list_with(&HttpSources::default(), vec![], &[loopback(port, None)]).await;
        let seen = seen.lock().unwrap().clone();
        assert!(
            seen.iter().all(|(path, _)| !path.ends_with("/epic/manifest")),
            "{seen:?}"
        );
    });
}

// ---------------------------------------------------------------- the compiled-in table

#[test]
fn the_table_is_whole() {
    let table = builtin::profiles();
    let rows: Vec<(String, &str, Option<&str>)> = table
        .iter()
        .map(|p| {
            (
                p.game_version.to_string(),
                p.display_name.as_str(),
                p.client.epic.as_ref().map(|e| e.version.as_str()),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [("1.14.7".to_owned(), "Rustic Isles 1.14.7", Some("1.14.7.647472")),]
    );
    let epic = table[0].client.epic.as_ref().unwrap();
    assert_eq!(
        epic.manifest_sha1.to_string(),
        "aa70748e4c0a776a991f2bac10ad342adaa31476"
    );
    assert_eq!(epic.executable_sha256.to_string(), crate::core::pins::PINNED_EXE_SHA256);
    assert_eq!(crate::core::pins::EXE_RELATIVE_PATH, epic::builds::EXECUTABLE);
    for profile in &table {
        // Each passes the check a served one gets, and reads back as served
        let row = ServedVersion {
            game_version: profile.game_version.clone(),
            display_name: profile.display_name.clone(),
            client: true,
            epic_manifest: false,
        };
        let served = parse::read_client(Some(&serde_json::to_value(&profile.client).unwrap()));
        assert_eq!(parse::checked(&row, served.unwrap()).as_ref(), Some(profile));
        if let Some(epic) = &profile.client.epic {
            assert_eq!(profile.client.executable_sha256, Some(epic.executable_sha256));
            assert!(
                epic.manifest_sources[0]
                    .starts_with("https://raw.githubusercontent.com/addison2k/DauntlessManifestArchive/")
            );
        }
    }
}

#[test]
fn with_nothing_reachable_the_list_is_still_the_table() {
    let list = block_on(list(
        &Fake::default(),
        builtin::profiles(),
        &[],
        &ManifestCache::default(),
    ));
    let states: Vec<(&str, GameVersionInstall, Option<EpicManifestState>)> = list
        .iter()
        .map(|e| (e.version.as_str(), e.install, state_of(e)))
        .collect();
    assert_eq!(
        states,
        [("1.14.7", GameVersionInstall::Epic, Some(EpicManifestState::Unreachable)),]
    );
}

/// The real archive over the network: `DAINTY_TEST_EPIC_MANIFEST=1`.
#[test]
fn the_archived_manifests_give_the_archives_sizes() {
    if std::env::var("DAINTY_TEST_EPIC_MANIFEST").as_deref() != Ok("1") {
        eprintln!("set DAINTY_TEST_EPIC_MANIFEST=1 to fetch the archived 1.14.7 manifest");
        return;
    }
    let list = block_on(list(
        &HttpSources::default(),
        builtin::profiles(),
        &[],
        &ManifestCache::default(),
    ));
    assert_eq!(state_of(&list[0]), Some(EpicManifestState::Ready));
    assert_eq!(
        sizes_of(&list[0]),
        Some(Sizes {
            download: 15_985_792_608,
            install: 16_521_629_423
        })
    );
}
