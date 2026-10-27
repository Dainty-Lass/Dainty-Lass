//! The installer against a fake CDN: three HTTPS hosts on 127.0.0.1:0 with the tests' own CA, serving chunks made
//! from a synthetic manifest. Nothing reaches Epic.

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::*;
use crate::manifest::synthetic::Build;
use crate::testing::{Cdn, EXE, Host, Version, build, cdn, version, version_of};

/// Some(owned), or None when the library can't be read
struct Owns(Option<bool>);

impl Gate for Owns {
    async fn require(&self) -> Result<(), Refusal> {
        match self.0 {
            Some(true) => Ok(()),
            Some(false) => Err(Refusal::NotOwned),
            None => Err(Refusal::Unreachable),
        }
    }
}

struct Refuses(Refusal);

impl Gate for Refuses {
    async fn require(&self) -> Result<(), Refusal> {
        Err(self.0)
    }
}

/// A library holding these app names
struct Library(Vec<String>);

impl crate::library::Library for Library {
    async fn records(&self, _: &crate::auth::Session) -> Result<Vec<crate::library::Record>, LibraryError> {
        Ok(self
            .0
            .iter()
            .map(|app_name| crate::library::Record {
                app_name: app_name.clone(),
                ..Default::default()
            })
            .collect())
    }
}

fn options(cdn: &Cdn, free: u64) -> Options {
    let mut options = Options::new(Arc::new(move |_: &Path| Ok(free)));
    options.bases = cdn.bases.clone();
    options.tls = Some(cdn.tls.clone());
    options.backoff = Arc::new(|_| Duration::ZERO);
    options.parallel = 3;
    options.timeout = Duration::from_secs(10);
    options
}

fn target<'a>(v: &'a Version, dir: &Path, exe_sha256: [u8; 32]) -> Target<'a> {
    Target {
        manifest: &v.manifest,
        manifest_sha1: v.pin,
        folder: dir.join("games").join("1.14.7"),
        executable: Some(Executable {
            path: InstallPath::new(EXE).unwrap(),
            sha256: exe_sha256,
        }),
        resume_file: dir.join("data").join("epic").join("1.14.7.resume"),
    }
}

fn exe_sha256(v: &Version) -> [u8; 32] {
    Sha256::digest(v.build.file_data(&v.build.files[1])).into()
}

fn assert_installed(v: &Version, folder: &Path) {
    for test in &v.build.files {
        let on_disk = std::fs::read(folder.join(&test.name)).unwrap();
        assert_eq!(on_disk, v.build.file_data(test), "{}", test.name);
    }
}

fn no_progress(_: &Progress) {}

fn chunk_paths(v: &Version) -> HashSet<String> {
    v.files.keys().filter(|p| p.ends_with(".chunk")).cloned().collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn installs_from_three_hosts_when_one_redirects_and_one_corrupts() {
    let v = version();
    let cdn = cdn(&[Host::Redirect, Host::Corrupt, Host::Good], v.files.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let t = target(&v, dir.path(), exe_sha256(&v));
    let last = Mutex::new(None);
    let on_progress = |p: &Progress| *last.lock().unwrap() = Some(p.clone());
    let installed = install(
        &t,
        &options(&cdn, u64::MAX),
        &Owns(Some(true)),
        &Control::default(),
        &on_progress,
    )
    .await
    .unwrap();
    assert_eq!(installed.files, 5);
    assert_installed(&v, &t.folder);
    assert!(!t.resume_file.exists(), "nothing left to resume");
    let last = last.lock().unwrap().clone().unwrap();
    assert_eq!(last.stage, Stage::Done);
    assert_eq!((last.files_done, last.files_total), (5, 5));
    assert_eq!((last.chunks_done, last.chunks_total), (6, 6));
    assert_eq!(last.downloaded, last.download_total);
    // The redirect was not followed: every chunk came from the good host, the only one that served it whole
    assert_eq!(cdn.requests(2).into_iter().collect::<HashSet<_>>(), chunk_paths(&v));
    assert!(!cdn.requests(0).is_empty() && !cdn.requests(1).is_empty());
    // A host that failed goes to the back: neither bad host is asked for all six chunks
    assert!(cdn.requests(0).len() + cdn.requests(1).len() < 12);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_download_killed_halfway_resumes_without_fetching_again() {
    let v = version();
    let dir = tempfile::tempdir().unwrap();
    let t = target(&v, dir.path(), exe_sha256(&v));
    let first = cdn(&[Host::Stall(3)], v.files.clone()).await;
    let mut opts = options(&first, u64::MAX);
    opts.parallel = 1;
    let killed = tokio::time::timeout(
        Duration::from_secs(3),
        install(&t, &opts, &Owns(Some(true)), &Control::default(), &no_progress),
    )
    .await;
    assert!(killed.is_err(), "the fourth chunk never comes");
    let recorded = std::fs::read_to_string(&t.resume_file).unwrap();
    let done = recorded.lines().filter(|l| l.starts_with("c ")).count();
    assert_eq!(done, 3, "{recorded}");

    let second = cdn(&[Host::Good], v.files.clone()).await;
    install(
        &t,
        &options(&second, u64::MAX),
        &Owns(Some(true)),
        &Control::default(),
        &no_progress,
    )
    .await
    .unwrap();
    assert_installed(&v, &t.folder);
    let asked: HashSet<String> = second.requests(0).into_iter().collect();
    let before: HashSet<String> = first.requests(0).into_iter().take(3).collect();
    assert_eq!(asked.len(), 3);
    assert!(asked.is_disjoint(&before));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_that_fails_its_check_has_its_chunks_fetched_again() {
    let v = version();
    let dir = tempfile::tempdir().unwrap();
    let t = target(&v, dir.path(), exe_sha256(&v));
    // Two of the exe's three chunks: it isn't checked before the kill
    let first = cdn(&[Host::Stall(2)], v.files.clone()).await;
    let mut opts = options(&first, u64::MAX);
    opts.parallel = 1;
    let _ = tokio::time::timeout(
        Duration::from_secs(3),
        install(&t, &opts, &Owns(Some(true)), &Control::default(), &no_progress),
    )
    .await;
    // Damage on disk the resume file can't know about: the exe's first chunk is recorded but now wrong
    let exe = t.folder.join(EXE);
    let mut bytes = std::fs::read(&exe).unwrap();
    bytes[0] ^= 0xFF;
    std::fs::write(&exe, &bytes).unwrap();

    let second = cdn(&[Host::Good], v.files.clone()).await;
    install(
        &t,
        &options(&second, u64::MAX),
        &Owns(Some(true)),
        &Control::default(),
        &no_progress,
    )
    .await
    .unwrap();
    assert_installed(&v, &t.folder);
    let chunk0 = format!(
        "/Builds/Jackal/{}",
        v.manifest.chunks()[0].cdn_path(v.manifest.feature_level)
    );
    assert!(second.requests(0).contains(&chunk0), "fetched again");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_that_fails_twice_stops_the_install() {
    let v = version_of(Build {
        wrong_sha1: Some(3),
        ..build()
    });
    let dir = tempfile::tempdir().unwrap();
    let t = target(&v, dir.path(), exe_sha256(&v));
    let cdn = cdn(&[Host::Good], v.files.clone()).await;
    let result = install(
        &t,
        &options(&cdn, u64::MAX),
        &Owns(Some(true)),
        &Control::default(),
        &no_progress,
    )
    .await;
    assert_eq!(
        result,
        Err(DownloadError::FileMismatch {
            name: "Archon/Content/Paks/b.pak".into()
        })
    );
    // b.pak's two chunks, once more each
    let fetched = cdn.requests(0);
    assert_eq!(fetched.len(), 6 + 2, "{fetched:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn pause_resume_and_cancel() {
    let v = version();
    let dir = tempfile::tempdir().unwrap();
    let t = target(&v, dir.path(), exe_sha256(&v));
    let cdn = cdn(&[Host::Good], v.files.clone()).await;
    let control = Control::default();
    assert!(control.pause());
    assert!(!control.pause());
    let opts = options(&cdn, u64::MAX);
    let paused = Arc::new(AtomicUsize::new(0));
    let seen = paused.clone();
    let on_progress = move |p: &Progress| {
        if p.stage == Stage::Paused {
            seen.fetch_add(1, Ordering::SeqCst);
        }
    };
    let run = install(&t, &opts, &Owns(Some(true)), &control, &on_progress);
    let resumer = async {
        while paused.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(cdn.requests(0).is_empty(), "nothing fetched while paused");
        assert!(control.resume());
    };
    let (installed, ()) = tokio::join!(run, resumer);
    installed.unwrap();
    assert_installed(&v, &t.folder);

    let dir = tempfile::tempdir().unwrap();
    let t = target(&v, dir.path(), exe_sha256(&v));
    let control = Control::default();
    assert!(control.cancel());
    assert!(!control.resume());
    let result = install(&t, &opts, &Owns(Some(true)), &control, &no_progress).await;
    assert_eq!(result, Err(DownloadError::Cancelled));
    assert!(t.resume_file.exists(), "a cancelled download resumes later");
}

#[tokio::test(flavor = "multi_thread")]
async fn too_little_space_is_refused_before_anything_is_fetched() {
    let v = version();
    let cdn = cdn(&[Host::Good], v.files.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let t = target(&v, dir.path(), exe_sha256(&v));
    let result = install(
        &t,
        &options(&cdn, 100),
        &Owns(Some(true)),
        &Control::default(),
        &no_progress,
    )
    .await;
    assert_eq!(
        result,
        Err(DownloadError::NoSpace {
            needed: v.manifest.install_size(),
            available: 100
        })
    );
    assert!(cdn.requests(0).is_empty());
    assert!(!t.folder.join(EXE).exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn nothing_is_fetched_for_an_account_without_dauntless() {
    let v = version();
    let cdn = cdn(&[Host::Good], v.files.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let t = target(&v, dir.path(), exe_sha256(&v));
    let opts = options(&cdn, u64::MAX);
    let result = install(&t, &opts, &Owns(Some(false)), &Control::default(), &no_progress).await;
    assert_eq!(result, Err(DownloadError::NotOwned));
    let result = install(&t, &opts, &Owns(None), &Control::default(), &no_progress).await;
    assert_eq!(result, Err(DownloadError::Library));
    let result = install(
        &t,
        &opts,
        &Refuses(Refusal::SignedOut),
        &Control::default(),
        &no_progress,
    )
    .await;
    assert_eq!(result, Err(DownloadError::SignedOut));

    // DE3's cached check on a session: the library says no (and an unreadable one is never let through)
    let token = || crate::auth::Token::new("not-a-real-token".into());
    let session = crate::auth::Session::new("account".into(), "Player".into(), token(), i64::MAX, token(), i64::MAX);
    let other = Library(vec!["Fortnite".into()]);
    let ownership = Ownership::new(other, None);
    let result = install(
        &t,
        &opts,
        &Owned(&ownership, &session),
        &Control::default(),
        &no_progress,
    )
    .await;
    assert_eq!(result, Err(DownloadError::NotOwned));
    let expired = crate::auth::Session::new("another".into(), "Player".into(), token(), 0, token(), 0);
    let result = install(
        &t,
        &opts,
        &Owned(&ownership, &expired),
        &Control::default(),
        &no_progress,
    )
    .await;
    assert_eq!(result, Err(DownloadError::SignedOut));
    assert!(cdn.requests(0).is_empty());
    assert!(!t.folder.exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_chunk_epic_deleted_everywhere_makes_the_version_unavailable() {
    let v = version();
    let cdn = cdn(&[Host::Missing, Host::Missing, Host::Missing], v.files.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let t = target(&v, dir.path(), exe_sha256(&v));
    let result = install(
        &t,
        &options(&cdn, u64::MAX),
        &Owns(Some(true)),
        &Control::default(),
        &no_progress,
    )
    .await;
    assert!(
        matches!(result, Err(DownloadError::ChunkUnavailable { deleted: true, .. })),
        "{result:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_executable_must_be_the_one_the_profile_pins() {
    let v = version();
    let cdn = cdn(&[Host::Good], v.files.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let t = target(&v, dir.path(), [0; 32]);
    let result = install(
        &t,
        &options(&cdn, u64::MAX),
        &Owns(Some(true)),
        &Control::default(),
        &no_progress,
    )
    .await;
    assert_eq!(result, Err(DownloadError::ExecutableHash { name: EXE.into() }));
    assert!(t.resume_file.exists(), "not installed");

    let mut t = target(&v, dir.path(), [0; 32]);
    t.executable = Some(Executable {
        path: InstallPath::new("Archon/Binaries/Win64/Other.exe").unwrap(),
        sha256: [0; 32],
    });
    let result = install(
        &t,
        &options(&cdn, u64::MAX),
        &Owns(Some(true)),
        &Control::default(),
        &no_progress,
    )
    .await;
    assert!(matches!(result, Err(DownloadError::ExecutableMissing { .. })));
}

#[test]
fn names_that_leave_the_folder_never_reach_the_installer() {
    for name in [
        "../evil.dll",
        "Archon/../../evil.dll",
        "C:/Windows/evil.dll",
        "/etc/evil",
        "a\\..\\b",
    ] {
        let mut b = build();
        b.files[0].name = name.into();
        let bytes = b.bytes();
        let result = Manifest::parse(&bytes, &Sha1Digest::of(&bytes));
        assert!(
            matches!(result, Err(crate::manifest::ManifestError::UnsafePath { .. })),
            "{name}: {result:?}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_link_inside_the_folder_is_refused() {
    let v = version();
    let cdn = cdn(&[Host::Good], v.files.clone()).await;
    let dir = tempfile::tempdir().unwrap();
    let t = target(&v, dir.path(), exe_sha256(&v));
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::create_dir_all(&t.folder).unwrap();
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(&outside, t.folder.join("Archon"));
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_dir(&outside, t.folder.join("Archon"));
    if let Err(e) = linked {
        eprintln!("skipped: this account can't make symlinks ({e})");
        return;
    }
    let result = install(
        &t,
        &options(&cdn, u64::MAX),
        &Owns(Some(true)),
        &Control::default(),
        &no_progress,
    )
    .await;
    assert_eq!(result, Err(DownloadError::LinkInFolder { name: "Archon".into() }));
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
    assert!(cdn.requests(0).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_manifest_comes_from_the_first_source_that_gives_the_pinned_one() {
    let v = version();
    let mut files = v.files.clone();
    files.insert("/m/other.manifest".into(), build_other().bytes());
    let cdn = cdn(&[Host::Good], files).await;
    let https = Https::new(Some(cdn.tls.clone()), Duration::from_secs(10)).unwrap();
    let host = cdn.bases[0].trim_end_matches("/Builds/Jackal").to_owned();
    let sources = vec![
        format!("{host}/m/missing.manifest"),
        format!("{host}/m/other.manifest"),
        format!("{host}/m/good.manifest"),
    ];
    let good = v.files["/m/good.manifest"].clone();

    let server = async { Err::<Vec<u8>, _>("unreachable".to_owned()) };
    let (manifest, origin) = fetch_manifest(Some(server), &sources, &v.pin, &https).await.unwrap();
    assert_eq!(origin, ManifestOrigin::Source(sources[2].clone()));
    assert_eq!(manifest.files().len(), 5);

    // The joined server's copy comes first, and is checked like the rest
    let server = async { Ok(good.clone()) };
    let (_, origin) = fetch_manifest(Some(server), &sources, &v.pin, &https).await.unwrap();
    assert_eq!(origin, ManifestOrigin::Server);
    let before = cdn.requests(0).len();
    let tampered = async {
        let mut bytes = good.clone();
        bytes[60] ^= 1;
        Ok(bytes)
    };
    let (_, origin) = fetch_manifest(Some(tampered), &sources, &v.pin, &https).await.unwrap();
    assert_eq!(origin, ManifestOrigin::Source(sources[2].clone()));
    assert_eq!(cdn.requests(0).len(), before + 3);

    let err = fetch_manifest(None::<std::future::Ready<_>>, &sources[..2], &v.pin, &https)
        .await
        .unwrap_err();
    assert_eq!(err.tried.len(), 2);
    let plain = vec!["http://127.0.0.1/m/good.manifest".to_owned()];
    let err = fetch_manifest(None::<std::future::Ready<_>>, &plain, &v.pin, &https)
        .await
        .unwrap_err();
    assert_eq!(err.tried[0].1, FetchError::NotHttps.to_string());
}

fn build_other() -> Build {
    let mut b = build();
    b.files.pop();
    b
}
