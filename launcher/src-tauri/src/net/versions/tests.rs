//! The list against synthetic manifests and stand-in sources. The archived manifests are Epic's files: the
//! opt-in test fetches them (`REFORGED_TEST_EPIC_MANIFEST=1`), as `crates/epic/tests/archive.rs` does.

use std::collections::HashMap;
use std::sync::Mutex;

use bytes::Bytes;
use epic::manifest::Sha1Digest;
use module_api::{ClientProfile, EpicBuild, GameVersion, Sha1Hex};
use serde_json::json;

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
            arguments: vec![],
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

#[derive(Default)]
struct Fake {
    profiles: HashMap<String, Vec<KnownProfile>>,
    /// By "host version"
    server_copies: HashMap<String, Vec<u8>>,
    urls: HashMap<String, Vec<u8>>,
    asked: Mutex<Vec<String>>,
}

impl Fake {
    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

impl Sources for Fake {
    async fn server_profiles(&self, server: &Endpoint) -> Vec<KnownProfile> {
        self.asked.lock().unwrap().push(format!("profiles {}", server.host));
        self.profiles.get(&server.host).cloned().unwrap_or_default()
    }

    async fn server_manifest(&self, server: &Endpoint, version: &GameVersion) -> Option<Bytes> {
        let key = format!("{} {version}", server.host);
        self.asked.lock().unwrap().push(format!("manifest {key}"));
        self.server_copies.get(&key).cloned().map(Bytes::from)
    }

    async fn url(&self, url: &str) -> Option<Bytes> {
        self.asked.lock().unwrap().push(url.to_owned());
        self.urls.get(url).cloned().map(Bytes::from)
    }
}

fn server(host: &str) -> Endpoint {
    Endpoint {
        host: host.into(),
        port: 61000,
        pin: None,
    }
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
    fake.profiles.insert("home".into(), profiles.clone());
    // The server's copy is corrupt, the module's own is missing, the archive's is right
    let mut corrupt = manifest_a();
    let last = corrupt.len() - 1;
    corrupt[last] ^= 1;
    fake.server_copies.insert("home 1.14.7".into(), corrupt);
    fake.urls.insert(SRC_A.into(), manifest_a());
    let list = block_on(list(
        &fake,
        profiles,
        &[server("other"), server("home")],
        &ManifestCache::default(),
    ));
    assert_eq!(sizes_of(&list[0]), Some(A_SIZES));
    assert_eq!(
        fake.asked(),
        ["profiles other", "profiles home", "manifest home 1.14.7", SRC_A2, SRC_A]
    );
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
    fake.profiles
        .insert("first".into(), vec![repinned, extra.clone(), revived]);
    let mut renamed = extra.clone();
    renamed.display_name = "Second's 2.1.1".into();
    fake.profiles.insert("second".into(), vec![renamed]);
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
    assert!(!fake.asked().contains(&"manifest first 1.14.7".to_owned()));
    // Both servers serve 2.1.1 with the same pin; the second had the copy
    assert_eq!(
        sizes_of(&list[0]),
        Some(Sizes {
            download: 32,
            install: 64
        })
    );
}

// ---------------------------------------------------------------- the served profiles

fn served(version: &str, source: &str) -> serde_json::Value {
    json!({
        "game_version": version,
        "display_name": format!("Rustic Isles {version}"),
        "client": {
            "executable": "Archon/Binaries/Win64/Dauntless-Win64-Shipping.exe",
            "epic": {
                "version": format!("{version}.1"),
                "manifest_sha1": "aa70748e4c0a776a991f2bac10ad342adaa31476",
                "manifest_sources": [source],
                "executable_sha256": "ab".repeat(32),
            }
        }
    })
}

#[test]
fn served_profiles_are_checked_one_by_one() {
    let body = json!({ "profiles": [
        served("1.14.7", "https://a.example/m"),
        served("1.14.7", "https://b.example/m"),
        served("1.12.0", "http://plain.example/m"),
        served("1.12", "file:///etc/passwd"),
        { "game_version": "1.12.0", "display_name": "x", "client": { "executable": "../evil.exe" } },
        { "game_version": "1.x", "display_name": "x", "client": { "executable": "a.exe" } },
        { "game_version": "2.0.0", "display_name": "Line\nbreak", "client": { "executable": "a.exe" } },
        { "game_version": "1.4.4", "display_name": "Dauntless Revived",
          "client": { "executable": "a.exe", "surprise": true } },
        { "game_version": "1.4.4", "display_name": "Dauntless Revived", "client": { "executable": "a.exe" } },
    ] });
    let read = parse::read_profiles(Some(&body));
    let versions: Vec<String> = read.iter().map(|p| p.game_version.to_string()).collect();
    assert_eq!(versions, ["1.14.7", "1.4.4"]);
    assert_eq!(
        read[0].client.epic.as_ref().unwrap().manifest_sources,
        ["https://a.example/m"]
    );
    assert!(read[1].client.epic.is_none());

    for nothing in [
        json!(null),
        json!([]),
        json!({ "profiles": {} }),
        json!({ "other": [] }),
    ] {
        assert!(parse::read_profiles(Some(&nothing)).is_empty(), "{nothing}");
    }
    assert!(parse::read_profiles(None).is_empty());
    let many: Vec<_> = (0..40)
        .map(|i| served(&format!("1.{i}.0"), "https://a.example/m"))
        .collect();
    assert_eq!(
        parse::read_profiles(Some(&json!({ "profiles": many }))).len(),
        parse::MAX_PROFILES
    );
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
        [
            ("1.14.7".to_owned(), "Rustic Isles 1.14.7", Some("1.14.7.647472")),
            ("1.12.0".to_owned(), "Rustic Isles 1.12", Some("1.12.0.392819")),
            ("1.4.4".to_owned(), "Dauntless Revived", None),
        ]
    );
    let epic = table[0].client.epic.as_ref().unwrap();
    assert_eq!(
        epic.manifest_sha1.to_string(),
        "aa70748e4c0a776a991f2bac10ad342adaa31476"
    );
    assert_eq!(epic.executable_sha256.to_string(), crate::core::pins::PINNED_EXE_SHA256);
    assert_eq!(
        table[1].client.epic.as_ref().unwrap().manifest_sha1.to_string(),
        "46b4097c070afc29ed783e77a68ffa936b6a3504"
    );
    #[cfg(feature = "legacy")]
    assert_eq!(
        table[2].client.executable_sha256.unwrap().to_string(),
        crate::core::legacy_pins::PINNED_EXE_SHA256
    );
    for profile in &table {
        // Each passes the check a served one gets
        let body = json!({ "profiles": [{
            "game_version": profile.game_version,
            "display_name": profile.display_name,
            "client": profile.client,
        }] });
        assert_eq!(parse::read_profiles(Some(&body)), std::slice::from_ref(profile));
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
        [
            ("1.14.7", GameVersionInstall::Epic, Some(EpicManifestState::Unreachable)),
            ("1.12.0", GameVersionInstall::Epic, Some(EpicManifestState::Unreachable)),
            ("1.4.4", GameVersionInstall::Locate, None),
        ]
    );
}

/// The real archive over the network: `REFORGED_TEST_EPIC_MANIFEST=1`.
#[test]
fn the_archived_manifests_give_the_archives_sizes() {
    if std::env::var("REFORGED_TEST_EPIC_MANIFEST").as_deref() != Ok("1") {
        eprintln!("set REFORGED_TEST_EPIC_MANIFEST=1 to fetch the archived 1.14.7 and 1.12.0 manifests");
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
    assert_eq!(
        sizes_of(&list[1]),
        Some(Sizes {
            download: 15_822_495_601,
            install: 16_421_864_653
        })
    );
}
