// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 Silent Spud

//! `dainty-host update`'s machinery: finding a release, checking it against `SHA256SUMS.txt` and its build
//! attestation, unpacking it safely, and installing it in place without overwriting files the host changed.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use flate2::read::GzDecoder;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::HostError;
use crate::run::{RunOptions, run};

/// The manifest at a release's root, listing every file with its SHA-256.
pub const RELEASE_FILE: &str = "release.json";
/// The checksum file published beside each release archive.
pub const SUMS_FILE: &str = "SHA256SUMS.txt";
/// The variable that points updates at another source: a GitHub `owner/repo`, a folder or a URL.
pub const SOURCE_ENV: &str = "DAINTY_SERVER_RELEASE";
/// The prefix of a server release's git tag.
pub const TAG_PREFIX: &str = "server-v";
/// The prefix of a release archive's file name.
pub const ARCHIVE_PREFIX: &str = "DaintyLassServer-";
/// The GitHub repository releases come from, fixed at build time.
pub const DEFAULT_REPOSITORY: &str = env!("DAINTY_RELEASE_REPOSITORY");

/// Why an update stopped; [`Problem::code`] is the machine-readable form `update --json` prints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// No release matches what was asked for.
    NoRelease(String),
    /// A download failed.
    DownloadFailed(String),
    /// The archive is not listed in [`SUMS_FILE`].
    NotInSums(String),
    /// A downloaded file's SHA-256 is not the one listed.
    Checksum {
        /// The file.
        file: String,
        /// The SHA-256 listed, hex.
        expected: String,
        /// The SHA-256 it has, hex.
        actual: String,
    },
    /// The release's build provenance did not verify.
    Attestation(String),
    /// The archive could not be unpacked, or holds no valid release.
    BadArchive(String),
    /// Files the host changed that the update would overwrite.
    LocalChanges(Vec<String>),
}

impl Problem {
    /// A snake_case code for the problem.
    pub fn code(&self) -> &'static str {
        match self {
            Self::NoRelease(_) => "no_release",
            Self::DownloadFailed(_) => "download_failed",
            Self::NotInSums(_) => "not_in_sums",
            Self::Checksum { .. } => "checksum",
            Self::Attestation(_) => "attestation",
            Self::BadArchive(_) => "bad_archive",
            Self::LocalChanges(_) => "local_changes",
        }
    }

    /// The problem's particulars without the sentence around them.
    pub fn detail(&self) -> Option<String> {
        match self {
            Self::NoRelease(s)
            | Self::DownloadFailed(s)
            | Self::NotInSums(s)
            | Self::Attestation(s)
            | Self::BadArchive(s) => Some(s.clone()),
            Self::Checksum { file, expected, actual } => Some(format!("{file}: expected {expected}, got {actual}")),
            Self::LocalChanges(files) => Some(files.join(", ")),
        }
    }

    /// The sentence the host reads.
    pub fn message(&self) -> String {
        match self {
            Self::NoRelease(s) => s.clone(),
            Self::DownloadFailed(s) => format!("download failed: {s}"),
            Self::NotInSums(file) => format!("{file} is not listed in {SUMS_FILE}"),
            Self::Checksum { file, expected, actual } => {
                format!("{file} is not the file the release lists (expected SHA-256 {expected}, got {actual})")
            }
            Self::Attestation(s) => format!("release attestation verification failed: {s}"),
            Self::BadArchive(s) => format!("invalid release archive: {s}"),
            Self::LocalChanges(files) => {
                format!("refusing to overwrite locally modified files: {}", files.join(", "))
            }
        }
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message())
    }
}

/// Why an update failed.
#[derive(Debug)]
pub enum UpdateError {
    /// Something about the release refuses it.
    Problem(Problem),
    /// A local error: a file, a command or the network stack.
    Host(HostError),
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Problem(p) => write!(f, "{p}"),
            Self::Host(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for UpdateError {}

impl From<Problem> for UpdateError {
    fn from(p: Problem) -> Self {
        Self::Problem(p)
    }
}

impl From<HostError> for UpdateError {
    fn from(e: HostError) -> Self {
        Self::Host(e)
    }
}

impl From<io::Error> for UpdateError {
    fn from(e: io::Error) -> Self {
        Self::Host(HostError::from(e))
    }
}

/// What the update steps return.
pub type UpdateResult<T> = Result<T, UpdateError>;

/// A semantic version, compared as SemVer orders them (build metadata ignored).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    /// Major, minor and patch.
    pub core: [u64; 3],
    /// The pre-release identifiers; empty for a release.
    pub pre: Vec<String>,
}

fn valid_semver_ident(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn parse_numeric_no_leading_zero(s: &str) -> Option<u64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if s.len() > 1 && s.starts_with('0') {
        return None;
    }
    s.parse::<u64>().ok()
}

impl Version {
    /// Parses a SemVer string; `None` for anything that is not one.
    pub fn parse(text: &str) -> Option<Self> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return None;
        }
        let without_build = match trimmed.split_once('+') {
            Some((before, build)) => {
                if build.is_empty() || !build.split('.').all(valid_semver_ident) {
                    return None;
                }
                before
            }
            None => trimmed,
        };
        let (core_str, pre_vec) = match without_build.split_once('-') {
            Some((c, pre_str)) => {
                if pre_str.is_empty() {
                    return None;
                }
                let mut pre = Vec::new();
                for part in pre_str.split('.') {
                    if !valid_semver_ident(part) {
                        return None;
                    }
                    if part.bytes().all(|b| b.is_ascii_digit()) && part.len() > 1 && part.starts_with('0') {
                        return None;
                    }
                    pre.push(part.to_owned());
                }
                (c, pre)
            }
            None => (without_build, Vec::new()),
        };

        let mut parts = core_str.split('.');
        let major = parse_numeric_no_leading_zero(parts.next()?)?;
        let minor = parse_numeric_no_leading_zero(parts.next()?)?;
        let patch = parse_numeric_no_leading_zero(parts.next()?)?;
        if parts.next().is_some() {
            return None;
        }

        Some(Self {
            core: [major, minor, patch],
            pre: pre_vec,
        })
    }

    /// Whether it is a pre-release.
    pub fn is_prerelease(&self) -> bool {
        !self.pre.is_empty()
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.core.cmp(&other.core) {
            Ordering::Equal => {}
            ord => return ord,
        }
        match (self.pre.is_empty(), other.pre.is_empty()) {
            (true, true) => Ordering::Equal,
            (true, false) => Ordering::Greater,
            (false, true) => Ordering::Less,
            (false, false) => {
                for (a, b) in self.pre.iter().zip(other.pre.iter()) {
                    let a_num = a
                        .bytes()
                        .all(|c| c.is_ascii_digit())
                        .then(|| a.parse::<u64>().ok())
                        .flatten();
                    let b_num = b
                        .bytes()
                        .all(|c| c.is_ascii_digit())
                        .then(|| b.parse::<u64>().ok())
                        .flatten();
                    let ord = match (a_num, b_num) {
                        (Some(an), Some(bn)) => an.cmp(&bn),
                        (Some(_), None) => Ordering::Less,
                        (None, Some(_)) => Ordering::Greater,
                        (None, None) => a.cmp(b),
                    };
                    if ord != Ordering::Equal {
                        return ord;
                    }
                }
                self.pre.len().cmp(&other.pre.len())
            }
        }
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// One file a release holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestFile {
    /// Its path from the release root, with forward slashes.
    pub path: String,
    /// Its SHA-256, lowercase hex.
    pub sha256: String,
    /// Its size in bytes.
    pub size: u64,
}

/// A release's [`RELEASE_FILE`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// The release's version.
    pub version: String,
    /// The git commit it was built from.
    pub commit: String,
    /// The Rust target it was built for.
    pub target: String,
    /// The platform name in its archive's name, like `windows-x64`.
    pub platform: String,
    /// The GitHub repository it came from, which later updates use.
    pub repository: Option<String>,
    /// Every file it holds.
    pub files: Vec<ManifestFile>,
}

fn is_safe_relative_path(path: &str) -> bool {
    if path.is_empty() || path.starts_with('/') || path.starts_with('\\') || path.contains('\\') || path.contains(':') {
        return false;
    }
    let p = Path::new(path);
    for comp in p.components() {
        match comp {
            Component::Normal(_) => {}
            _ => return false,
        }
    }
    true
}

impl Manifest {
    /// Parses a release.json; the error says what is missing or malformed, and an unsafe file path is refused.
    pub fn parse(text: &str) -> Result<Self, String> {
        let stripped = text.strip_prefix('\u{feff}').unwrap_or(text);
        let val: Value = serde_json::from_str(stripped).map_err(|e| format!("invalid JSON: {e}"))?;
        let obj = val
            .as_object()
            .ok_or_else(|| "release.json must be a JSON object".to_owned())?;

        let version = obj
            .get("version")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty() && Version::parse(s).is_some())
            .ok_or_else(|| "release.json is missing a valid semver version".to_owned())?
            .to_owned();

        let commit = obj
            .get("commit")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_owned();

        let target = obj
            .get("target")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_owned();

        let platform = obj
            .get("platform")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_owned();

        let repository = obj
            .get("repository")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);

        let files_arr = obj
            .get("files")
            .and_then(Value::as_array)
            .ok_or_else(|| "release.json is missing files array".to_owned())?;

        let mut files = Vec::with_capacity(files_arr.len());
        for entry in files_arr {
            let entry_obj = entry
                .as_object()
                .ok_or_else(|| "file entry in release.json must be an object".to_owned())?;
            let raw_path = entry_obj
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| "file entry is missing path".to_owned())?;
            if !is_safe_relative_path(raw_path) {
                return Err(format!("unsafe file path in release.json: {raw_path}"));
            }
            let norm_path = raw_path.to_owned();
            let sha256 = entry_obj
                .get("sha256")
                .and_then(Value::as_str)
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
                .ok_or_else(|| format!("invalid sha256 for {norm_path}"))?;
            let size = match entry_obj.get("size") {
                Some(v) => v.as_u64().ok_or_else(|| format!("invalid size for {norm_path}"))?,
                None => 0,
            };
            files.push(ManifestFile {
                path: norm_path,
                sha256,
                size,
            });
        }

        Ok(Self {
            version,
            commit,
            target,
            platform,
            repository,
            files,
        })
    }

    /// The listed file at `path`, matched with forward slashes.
    pub fn file(&self, path: &str) -> Option<&ManifestFile> {
        let norm = path.replace('\\', "/");
        self.files.iter().find(|f| f.path == norm)
    }

    /// The commit's first 12 characters, or `unknown`.
    pub fn short_commit(&self) -> &str {
        if self.commit.is_empty() {
            "unknown"
        } else {
            let end = self.commit.len().min(12);
            &self.commit[..end]
        }
    }
}

/// Where a download comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    /// A file in a release folder.
    File(PathBuf),
    /// A URL.
    Url(String),
}

/// Where releases are looked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// A GitHub repository, `owner/repo`.
    GitHub(String),
    /// A folder holding release archives and their checksums.
    Folder(PathBuf),
    /// A URL whose folder holds them.
    Url(String),
}

impl Source {
    /// The source [`SOURCE_ENV`] names, else the installed release's repository, else [`DEFAULT_REPOSITORY`].
    pub fn from_env(installed: Option<&Manifest>) -> Self {
        if let Ok(val) = std::env::var(SOURCE_ENV) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                return Self::parse(trimmed);
            }
        }
        if let Some(repo) = installed
            .and_then(|m| m.repository.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            return Self::GitHub(repo.to_owned());
        }
        Self::GitHub(DEFAULT_REPOSITORY.to_owned())
    }

    /// Reads `http://` or `https://` as a URL, `github:owner/repo` or a bare `owner/repo` that is no path on disk as
    /// a repository, and anything else as a folder.
    pub fn parse(value: &str) -> Self {
        let trimmed = value.trim();
        if trimmed.starts_with("https://") || trimmed.starts_with("http://") {
            return Self::Url(trimmed.trim_end_matches('/').to_owned());
        }
        if let Some(repo) = trimmed.strip_prefix("github:") {
            return Self::GitHub(repo.trim().to_owned());
        }
        let is_owner_repo = !trimmed.starts_with('.')
            && !trimmed.starts_with('/')
            && !trimmed.contains('\\')
            && !trimmed.contains(':')
            && trimmed.split('/').count() == 2
            && trimmed.split('/').all(|p| !p.is_empty())
            && !Path::new(trimmed).exists();
        if is_owner_repo {
            Self::GitHub(trimmed.to_owned())
        } else {
            Self::Folder(PathBuf::from(trimmed))
        }
    }

    /// The source as the host reads it.
    pub fn describe(&self) -> String {
        match self {
            Self::GitHub(repo) => format!("GitHub ({repo})"),
            Self::Folder(path) => format!("folder {}", path.display()),
            Self::Url(url) => url.clone(),
        }
    }
}

/// An `http://` URL whose host is not loopback.
fn is_remote_plain_http(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("http://") else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if let Some(v6) = host_port.strip_prefix('[') {
        v6.split(']').next().unwrap_or_default()
    } else {
        host_port.split(':').next().unwrap_or_default()
    };
    let loopback =
        host.eq_ignore_ascii_case("localhost") || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback());
    !loopback
}

/// The release an update would install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Its version.
    pub version: String,
    /// Its archive's file name.
    pub archive: String,
    /// Where the archive is.
    pub archive_at: Location,
    /// Where its [`SUMS_FILE`] is.
    pub sums_at: Location,
    /// The GitHub repository it came from, for the attestation check; `None` from a folder or URL.
    pub repository: Option<String>,
}

/// Which release to look for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wanted<'a> {
    /// The newest, past the installed version if one is given.
    Newest(Option<&'a str>),
    /// This version.
    Exactly(&'a str),
}

/// What came of the build provenance check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attestation {
    /// `gh attestation verify` verified the archive.
    Verified,
    /// Not checked, and why.
    Skipped(String),
}

/// What an install did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    /// The version installed before; `None` for a first install.
    pub from: Option<String>,
    /// The version installed now.
    pub to: String,
    /// How many files were written.
    pub files: usize,
    /// How many files of the old release were removed.
    pub removed: usize,
    /// Whether the game runtime's files changed; `None` without an earlier release.
    pub runtime_changed: Option<bool>,
}

/// This machine's release platform: `windows-x64` or `linux-x64`; `None` elsewhere.
pub fn platform() -> Option<&'static str> {
    if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some("windows-x64")
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Some("linux-x64")
    } else {
        None
    }
}

/// This machine's release platform, or an error on one that has no releases.
pub fn this_platform() -> Result<&'static str, HostError> {
    platform().ok_or_else(|| HostError::message("self-update is only supported on windows-x64 and linux-x64"))
}

fn archive_ext(platform: &str) -> &'static str {
    if platform.starts_with("windows") {
        ".zip"
    } else {
        ".tar.gz"
    }
}

/// The archive's file name for `version` on `platform`.
pub fn archive_name(version: &str, platform: &str) -> String {
    let ext = archive_ext(platform);
    format!("{ARCHIVE_PREFIX}{version}-{platform}{ext}")
}

/// The version in a release archive's name for `platform`; `None` for another platform's or a foreign file.
pub fn archive_version<'a>(name: &'a str, platform: &str) -> Option<&'a str> {
    let ext = archive_ext(platform);
    let suffix = format!("-{platform}{ext}");
    let rest = name.strip_prefix(ARCHIVE_PREFIX)?;
    let ver = rest.strip_suffix(&suffix)?;
    Version::parse(ver)?;
    Some(ver)
}

/// The highest SemVer of `versions`; pre-releases only when the installed one is one, or when there is nothing else.
pub fn pick_newest<'a>(versions: &[&'a str], installed: Option<&str>) -> Option<&'a str> {
    let allow_prerelease = installed.and_then(Version::parse).is_some_and(|v| v.is_prerelease());

    let mut best: Option<(&'a str, Version)> = None;
    let mut best_pre: Option<(&'a str, Version)> = None;
    for &raw in versions {
        let Some(ver) = Version::parse(raw) else {
            continue;
        };
        if ver.is_prerelease() && !allow_prerelease {
            if best_pre.as_ref().is_none_or(|(_, b)| ver > *b) {
                best_pre = Some((raw, ver));
            }
            continue;
        }
        if best.as_ref().is_none_or(|(_, b)| ver > *b) {
            best = Some((raw, ver));
        }
    }
    best.or(best_pre).map(|(s, _)| s)
}

/// The release.json at `root`; `None` when there is none (a checkout).
pub fn read_manifest(root: &Path) -> Result<Option<Manifest>, HostError> {
    let file = root.join(RELEASE_FILE);
    if !file.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&file)?;
    Manifest::parse(&text)
        .map(Some)
        .map_err(|e| HostError::message(format!("invalid {}: {e}", file.display())))
}

/// The file's SHA-256, lowercase hex.
pub fn sha256_file(file: &Path) -> Result<String, HostError> {
    let mut f = File::open(file)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// The (lowercase SHA-256, file name) pairs of a SHA256SUMS text; comments and malformed lines are skipped.
pub fn parse_sums(text: &str) -> Vec<(String, String)> {
    let stripped = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut out = Vec::new();
    for line in stripped.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let Some(hash) = parts.next() else {
            continue;
        };
        let Some(rest) = parts.next() else {
            continue;
        };
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        let file = rest.strip_prefix('*').unwrap_or(rest);
        let file_name = file.rsplit(['/', '\\']).next().unwrap_or(file).to_owned();
        if !file_name.is_empty() {
            out.push((hash.to_ascii_lowercase(), file_name));
        }
    }
    out
}

fn normalize_wanted_version(v: &str) -> &str {
    v.trim()
        .strip_prefix(TAG_PREFIX)
        .or_else(|| v.trim().strip_prefix('v'))
        .unwrap_or_else(|| v.trim())
}

fn select_candidate(
    candidates: &[Candidate],
    wanted: Wanted<'_>,
    source_desc: &str,
    platform: &str,
) -> UpdateResult<Candidate> {
    match wanted {
        Wanted::Exactly(target) => {
            let norm = normalize_wanted_version(target);
            candidates.iter().find(|c| c.version == norm).cloned().ok_or_else(|| {
                UpdateError::Problem(Problem::NoRelease(format!(
                    "release {norm} for {platform} was not found in {source_desc}"
                )))
            })
        }
        Wanted::Newest(installed) => {
            let versions: Vec<&str> = candidates.iter().map(|c| c.version.as_str()).collect();
            let chosen = pick_newest(&versions, installed).ok_or_else(|| {
                UpdateError::Problem(Problem::NoRelease(format!(
                    "no release for {platform} was found in {source_desc}"
                )))
            })?;
            candidates.iter().find(|c| c.version == chosen).cloned().ok_or_else(|| {
                UpdateError::Problem(Problem::NoRelease(format!(
                    "no release for {platform} was found in {source_desc}"
                )))
            })
        }
    }
}

/// The release `wanted` picks from a GitHub releases API answer, with its archive and checksum asset URLs.
pub fn candidates_from_github(
    releases: &Value,
    repo: &str,
    platform: &str,
    wanted: Wanted<'_>,
) -> UpdateResult<Candidate> {
    let single;
    let list: &[Value] = if let Some(arr) = releases.as_array() {
        arr.as_slice()
    } else if releases.is_object() {
        single = [releases.clone()];
        &single
    } else {
        return Err(UpdateError::Problem(Problem::NoRelease(
            "GitHub releases response is not a JSON array".to_owned(),
        )));
    };

    let mut candidates = Vec::new();
    for rel in list {
        let Some(obj) = rel.as_object() else {
            continue;
        };
        if obj.get("draft").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let Some(tag) = obj.get("tag_name").and_then(Value::as_str) else {
            continue;
        };
        let Some(version) = tag.strip_prefix(TAG_PREFIX) else {
            continue;
        };
        if Version::parse(version).is_none() {
            continue;
        }
        let expected_archive = archive_name(version, platform);
        let Some(assets) = obj.get("assets").and_then(Value::as_array) else {
            continue;
        };
        let mut archive_url = None;
        let mut sums_url = None;
        for asset in assets {
            let Some(aobj) = asset.as_object() else {
                continue;
            };
            let Some(name) = aobj.get("name").and_then(Value::as_str) else {
                continue;
            };
            let Some(url) = aobj.get("browser_download_url").and_then(Value::as_str) else {
                continue;
            };
            if name == expected_archive {
                archive_url = Some(url.to_owned());
            } else if name == SUMS_FILE {
                sums_url = Some(url.to_owned());
            }
        }
        if let (Some(a_url), Some(s_url)) = (archive_url, sums_url) {
            candidates.push(Candidate {
                version: version.to_owned(),
                archive: expected_archive,
                archive_at: Location::Url(a_url),
                sums_at: Location::Url(s_url),
                repository: Some(repo.to_owned()),
            });
        }
    }

    select_candidate(&candidates, wanted, &format!("GitHub ({repo})"), platform)
}

/// Finds the release `wanted` picks for `platform` in `source`.
pub fn find_release(source: &Source, platform: &str, wanted: Wanted<'_>) -> UpdateResult<Candidate> {
    match source {
        Source::GitHub(repo) => {
            let api_url = format!("https://api.github.com/repos/{repo}/releases?per_page=30");
            let body = http::get_text(&api_url, true).map_err(|e| UpdateError::Problem(Problem::DownloadFailed(e)))?;
            let json: Value = serde_json::from_str(&body).map_err(|e| {
                UpdateError::Problem(Problem::DownloadFailed(format!("invalid JSON from {api_url}: {e}")))
            })?;
            candidates_from_github(&json, repo, platform, wanted)
        }
        Source::Folder(dir) => {
            if !dir.is_dir() {
                return Err(UpdateError::Problem(Problem::NoRelease(format!(
                    "release folder does not exist: {}",
                    dir.display()
                ))));
            }
            let sums_path = dir.join(SUMS_FILE);
            let mut candidates = Vec::new();
            let mut seen = BTreeSet::new();
            if let Ok(sums_text) = fs::read_to_string(&sums_path) {
                for (_, file_name) in parse_sums(&sums_text) {
                    if let Some(ver) = archive_version(&file_name, platform)
                        && seen.insert(file_name.clone())
                    {
                        candidates.push(Candidate {
                            version: ver.to_owned(),
                            archive: file_name.clone(),
                            archive_at: Location::File(dir.join(&file_name)),
                            sums_at: Location::File(sums_path.clone()),
                            repository: None,
                        });
                    }
                }
            }
            for entry in fs::read_dir(dir)? {
                let entry = entry?;
                let file_name = entry.file_name().to_string_lossy().into_owned();
                if let Some(ver) = archive_version(&file_name, platform)
                    && seen.insert(file_name.clone())
                {
                    candidates.push(Candidate {
                        version: ver.to_owned(),
                        archive: file_name.clone(),
                        archive_at: Location::File(dir.join(&file_name)),
                        sums_at: Location::File(sums_path.clone()),
                        repository: None,
                    });
                }
            }
            select_candidate(&candidates, wanted, &source.describe(), platform)
        }
        Source::Url(url) => {
            if is_remote_plain_http(url) {
                // The checksums come from the same place as the archive and a URL has no attestation
                return Err(UpdateError::Problem(Problem::DownloadFailed(format!(
                    "{url}: a release source off this machine must be https://"
                ))));
            }
            let clean = url.trim_end_matches('/');
            if let Some((base, file_name)) = clean.rsplit_once('/')
                && let Some(ver) = archive_version(file_name, platform)
            {
                let c = Candidate {
                    version: ver.to_owned(),
                    archive: file_name.to_owned(),
                    archive_at: Location::Url(clean.to_owned()),
                    sums_at: Location::Url(format!("{base}/{SUMS_FILE}")),
                    repository: None,
                };
                return select_candidate(&[c], wanted, &source.describe(), platform);
            }
            let sums_url = format!("{clean}/{SUMS_FILE}");
            let sums_text =
                http::get_text(&sums_url, false).map_err(|e| UpdateError::Problem(Problem::DownloadFailed(e)))?;
            let mut candidates = Vec::new();
            for (_, name) in parse_sums(&sums_text) {
                if let Some(ver) = archive_version(&name, platform) {
                    candidates.push(Candidate {
                        version: ver.to_owned(),
                        archive: name.clone(),
                        archive_at: Location::Url(format!("{clean}/{name}")),
                        sums_at: Location::Url(sums_url.clone()),
                        repository: None,
                    });
                }
            }
            select_candidate(&candidates, wanted, &source.describe(), platform)
        }
    }
}

fn fetch_location(loc: &Location, dest: &Path) -> UpdateResult<()> {
    match loc {
        Location::File(src) => {
            if !src.is_file() {
                return Err(UpdateError::Problem(Problem::DownloadFailed(format!(
                    "file not found: {}",
                    src.display()
                ))));
            }
            if src != dest {
                fs::copy(src, dest)?;
            }
            Ok(())
        }
        Location::Url(url) => http::download(url, dest).map_err(|e| UpdateError::Problem(Problem::DownloadFailed(e))),
    }
}

/// Downloads the candidate's archive and checksums into `dir` and checks the archive's SHA-256; returns the archive.
/// Nothing downloaded is kept when the check fails.
pub fn fetch(candidate: &Candidate, dir: &Path) -> UpdateResult<PathBuf> {
    fs::create_dir_all(dir)?;
    let sums_dest = dir.join(SUMS_FILE);
    let archive_dest = dir.join(&candidate.archive);
    let cleanup = || {
        if !matches!(&candidate.sums_at, Location::File(src) if src == &sums_dest) {
            let _ = fs::remove_file(&sums_dest);
        }
        if !matches!(&candidate.archive_at, Location::File(src) if src == &archive_dest) {
            let _ = fs::remove_file(&archive_dest);
        }
    };

    if let Err(e) = fetch_location(&candidate.sums_at, &sums_dest) {
        cleanup();
        return Err(e);
    }
    let sums_text = match fs::read_to_string(&sums_dest) {
        Ok(t) => t,
        Err(e) => {
            cleanup();
            return Err(e.into());
        }
    };
    let sums = parse_sums(&sums_text);
    let Some(expected) = sums
        .into_iter()
        .find(|(_, name)| name == &candidate.archive)
        .map(|(hash, _)| hash)
    else {
        cleanup();
        return Err(UpdateError::Problem(Problem::NotInSums(candidate.archive.clone())));
    };

    if let Err(e) = fetch_location(&candidate.archive_at, &archive_dest) {
        cleanup();
        return Err(e);
    }
    let actual = match sha256_file(&archive_dest) {
        Ok(h) => h,
        Err(e) => {
            cleanup();
            return Err(e.into());
        }
    };
    if actual != expected {
        cleanup();
        return Err(UpdateError::Problem(Problem::Checksum {
            file: candidate.archive.clone(),
            expected,
            actual,
        }));
    }
    Ok(archive_dest)
}

/// Checks the archive's build provenance with `gh attestation verify`, logging to `log_file`.
/// Skipped, not failed, when the release has no repository or `gh` is missing.
pub fn verify_attestation(archive: &Path, candidate: &Candidate, log_file: &Path) -> UpdateResult<Attestation> {
    let Some(repo) = candidate.repository.as_deref() else {
        return Ok(Attestation::Skipped(
            "release did not come from a GitHub repository".to_owned(),
        ));
    };
    let gh_found = crate::which::search_path("gh", std::env::var_os("PATH").as_deref());
    let Some(gh_bin) = gh_found else {
        return Ok(Attestation::Skipped("gh CLI is not installed".to_owned()));
    };

    let archive_str = archive.to_string_lossy().into_owned();
    let cwd = archive
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let res = run(
        &gh_bin.to_string_lossy(),
        &["attestation", "verify", &archive_str, "--repo", repo],
        &RunOptions {
            cwd,
            env: None,
            log_file: Some(log_file.to_path_buf()),
            quiet: true,
        },
    )?;

    if res.code == 0 {
        Ok(Attestation::Verified)
    } else {
        let detail = res
            .last_line
            .unwrap_or_else(|| format!("gh attestation verify exited with {}", res.code));
        Err(UpdateError::Problem(Problem::Attestation(detail)))
    }
}

fn validate_archive_entry_path(raw: &Path) -> UpdateResult<PathBuf> {
    let mut clean = PathBuf::new();
    for comp in raw.components() {
        match comp {
            Component::Normal(part) => clean.push(part),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(UpdateError::Problem(Problem::BadArchive(format!(
                    "unsafe path in archive: {}",
                    raw.display()
                ))));
            }
        }
    }
    if clean.as_os_str().is_empty() {
        return Err(UpdateError::Problem(Problem::BadArchive(
            "empty path in archive".to_owned(),
        )));
    }
    Ok(clean)
}

/// The most an archive may unpack to: a release is well under this, and a gzip or deflate bomb is refused.
pub const MAX_EXTRACTED_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// Copies one entry, taking what it writes from `budget`; an archive past the budget is refused.
fn copy_bounded(entry: &mut impl Read, out: &mut File, budget: &mut u64) -> UpdateResult<()> {
    let written = io::copy(&mut entry.take(budget.saturating_add(1)), out)?;
    if written > *budget {
        return Err(UpdateError::Problem(Problem::BadArchive(format!(
            "archive unpacks to more than {MAX_EXTRACTED_BYTES} bytes"
        ))));
    }
    *budget -= written;
    Ok(())
}

fn extract_zip(archive: &Path, dest: &Path, mut budget: u64) -> UpdateResult<()> {
    let file = File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| UpdateError::Problem(Problem::BadArchive(format!("invalid zip: {e}"))))?;

    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| UpdateError::Problem(Problem::BadArchive(format!("corrupt zip entry: {e}"))))?;
        let entry_name = entry
            .name()
            .map_err(|e| UpdateError::Problem(Problem::BadArchive(format!("invalid zip entry name: {e}"))))?
            .into_owned();
        if entry.is_symlink() {
            return Err(UpdateError::Problem(Problem::BadArchive(format!(
                "archive contains symlink: {entry_name}"
            ))));
        }
        let raw_name = entry_name.replace('\\', "/");
        if raw_name.trim_matches('/').is_empty() {
            continue;
        }
        let rel = validate_archive_entry_path(Path::new(&raw_name))?;
        let out_path = dest.join(&rel);
        if entry.is_dir() || entry_name.ends_with('/') {
            fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut out_file = File::create(&out_path)?;
            copy_bounded(&mut entry, &mut out_file, &mut budget)?;
            #[cfg(unix)]
            if let Some(mode) = entry.unix_mode() {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&out_path, fs::Permissions::from_mode(mode & 0o777));
            }
        }
    }
    Ok(())
}

fn extract_tar_gz(archive: &Path, dest: &Path, mut budget: u64) -> UpdateResult<()> {
    let file = File::open(archive)?;
    let gz = GzDecoder::new(file);
    let mut tar = tar::Archive::new(gz);

    let entries = tar
        .entries()
        .map_err(|e| UpdateError::Problem(Problem::BadArchive(format!("invalid tar.gz: {e}"))))?;

    for entry_res in entries {
        let mut entry =
            entry_res.map_err(|e| UpdateError::Problem(Problem::BadArchive(format!("corrupt tar entry: {e}"))))?;
        let entry_type = entry.header().entry_type();
        if entry_type.is_symlink() || entry_type.is_hard_link() {
            return Err(UpdateError::Problem(Problem::BadArchive(
                "archive contains symbolic or hard link".to_owned(),
            )));
        }
        let path_cow = entry
            .path()
            .map_err(|e| UpdateError::Problem(Problem::BadArchive(format!("invalid tar path: {e}"))))?;
        if path_cow.as_os_str().is_empty() || path_cow.as_ref() == Path::new(".") {
            continue;
        }
        let rel = validate_archive_entry_path(&path_cow)?;
        let out_path = dest.join(&rel);
        if entry_type.is_dir() {
            fs::create_dir_all(&out_path)?;
        } else if entry_type.is_file() {
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut out_file = File::create(&out_path)?;
            copy_bounded(&mut entry, &mut out_file, &mut budget)?;
            #[cfg(unix)]
            if let Ok(mode) = entry.header().mode() {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&out_path, fs::Permissions::from_mode(mode & 0o777));
            }
        }
    }
    Ok(())
}

/// Unpacks a zip or tar.gz into `dest` (emptied first), refusing paths that escape it and links.
/// Returns the release root: `dest`, or its one folder when that holds the release.json.
pub fn extract(archive: &Path, dest: &Path) -> UpdateResult<PathBuf> {
    extract_within(archive, dest, MAX_EXTRACTED_BYTES)
}

fn extract_within(archive: &Path, dest: &Path, budget: u64) -> UpdateResult<PathBuf> {
    if dest.exists() {
        fs::remove_dir_all(dest)?;
    }
    fs::create_dir_all(dest)?;
    let name = archive
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if name.ends_with(".zip") {
        extract_zip(archive, dest, budget)?;
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        extract_tar_gz(archive, dest, budget)?;
    } else {
        return Err(UpdateError::Problem(Problem::BadArchive(format!(
            "unsupported archive format: {}",
            archive.display()
        ))));
    }

    if dest.join(RELEASE_FILE).is_file() {
        return Ok(dest.to_path_buf());
    }
    let mut subdirs = Vec::new();
    for entry in fs::read_dir(dest)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            subdirs.push(path);
        }
    }
    if subdirs.len() == 1 && subdirs[0].join(RELEASE_FILE).is_file() {
        return Ok(subdirs.remove(0));
    }
    Ok(dest.to_path_buf())
}

/// The listed files at `root` that are missing or whose size or SHA-256 differ.
pub fn damaged_files(root: &Path, manifest: &Manifest) -> Result<Vec<String>, HostError> {
    let mut damaged = Vec::new();
    for f in &manifest.files {
        let full = root.join(&f.path);
        let Ok(meta) = fs::metadata(&full) else {
            damaged.push(f.path.clone());
            continue;
        };
        if !meta.is_file() || (f.size > 0 && meta.len() != f.size) {
            damaged.push(f.path.clone());
            continue;
        }
        let actual = sha256_file(&full)?;
        if actual != f.sha256 {
            damaged.push(f.path.clone());
        }
    }
    Ok(damaged)
}

fn is_runtime_manifest_path(path: &str) -> bool {
    let norm = path.replace('\\', "/");
    norm.starts_with("modules/rustic-isles/native/runtime/")
        || norm.starts_with("native/runtime/")
        || norm.ends_with("/Rustic-Isles.dll")
        || norm.ends_with("/winmm.dll")
        || norm.ends_with("/rustic-isles-runtime.dll")
        || norm == "Rustic-Isles.dll"
        || norm == "rustic-isles-runtime.dll"
        || norm == "winmm.dll"
        // An older release's manifest lists the runtime under its name from before the Rustic Isles rename
        || norm.ends_with("/MysticParadox.dll")
        || norm == "MysticParadox.dll"
}

/// Whether the game runtime's files differ between the two releases.
pub fn runtime_changed(previous: &Manifest, next: &Manifest) -> bool {
    let prev_rt: BTreeMap<&str, &str> = previous
        .files
        .iter()
        .filter(|f| is_runtime_manifest_path(&f.path))
        .map(|f| (f.path.as_str(), f.sha256.as_str()))
        .collect();
    let next_rt: BTreeMap<&str, &str> = next
        .files
        .iter()
        .filter(|f| is_runtime_manifest_path(&f.path))
        .map(|f| (f.path.as_str(), f.sha256.as_str()))
        .collect();
    prev_rt != next_rt
}

/// Files at `root` the host changed since `previous` was installed, unless they already match the new release.
/// Without a previous release, a git checkout's uncommitted changes.
pub fn local_changes(root: &Path, previous: Option<&Manifest>, next: &Manifest) -> Result<Vec<String>, HostError> {
    let Some(prev) = previous else {
        if !root.join(RELEASE_FILE).is_file()
            && root.join(".git").exists()
            && let Ok(Some(diff_out)) = crate::run::capture("git", &["diff", "--name-only", "HEAD"], Some(root))
        {
            let mut changed = Vec::new();
            for line in diff_out.lines() {
                let path = line.trim().replace('\\', "/");
                if path.is_empty() || path.ends_with("package-lock.json") {
                    continue;
                }
                let full = root.join(&path);
                if let Some(nf) = next.file(&path)
                    && full.is_file()
                    && sha256_file(&full).ok().as_deref() == Some(nf.sha256.as_str())
                {
                    continue;
                }
                changed.push(path);
            }
            return Ok(changed);
        }
        return Ok(Vec::new());
    };
    let mut changed = Vec::new();
    for pf in &prev.files {
        let full = root.join(&pf.path);
        if !full.is_file() {
            continue;
        }
        let actual = sha256_file(&full)?;
        if actual == pf.sha256 {
            continue;
        }
        if let Some(nf) = next.file(&pf.path)
            && actual == nf.sha256
        {
            continue;
        }
        changed.push(pf.path.clone());
    }
    Ok(changed)
}

fn sweep_aside_in_dir(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if matches!(name.as_str(), ".workspace" | ".git" | "node_modules" | "target") {
                continue;
            }
            sweep_aside_in_dir(&path);
        } else if name.contains(".rr-old-") || name.contains(".old-") {
            let _ = fs::remove_file(&path);
        }
    }
}

/// Removes files an earlier install moved aside because they were locked.
pub fn remove_moved_aside(root: &Path) {
    sweep_aside_in_dir(root);
}

/// The file that says an install is under way; an update that finds it finishes the interrupted install.
pub fn installing_marker(root: &Path) -> PathBuf {
    root.join(".workspace").join("update").join("installing")
}

fn install_single_file(src: &Path, dst: &Path) -> io::Result<()> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent)?;
    }
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let file_name = dst.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = dst.with_file_name(format!(".{file_name}.tmp-{millis}"));
    fs::copy(src, &tmp)?;

    #[cfg(unix)]
    if let Ok(meta) = fs::metadata(src) {
        let _ = fs::set_permissions(&tmp, meta.permissions());
    }

    if fs::rename(&tmp, dst).is_ok() {
        return Ok(());
    }

    let aside = dst.with_file_name(format!("{file_name}.rr-old-{millis}"));
    let _ = fs::remove_file(&aside);
    if dst.exists() {
        fs::rename(dst, &aside)?;
    }
    match fs::rename(&tmp, dst) {
        Ok(()) => {
            let _ = fs::remove_file(&aside);
            Ok(())
        }
        Err(e) => {
            if aside.exists() {
                let _ = fs::rename(&aside, dst);
            }
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

fn remove_empty_parent_dirs(mut dir: Option<&Path>, root: &Path) {
    while let Some(d) = dir {
        if d == root || !d.starts_with(root) {
            break;
        }
        if fs::remove_dir(d).is_err() {
            break;
        }
        dir = d.parent();
    }
}

/// Installs the staged release over `root`, file by file, with [`installing_marker`] held until it is done.
/// Returns the new manifest and what changed; a damaged staged release or [`Problem::LocalChanges`] stops it before it
/// writes.
pub fn install(staged: &Path, root: &Path, previous: Option<&Manifest>) -> UpdateResult<(Manifest, Installed)> {
    let next = read_manifest(staged)
        .map_err(|e| UpdateError::Problem(Problem::BadArchive(e.to_string())))?
        .ok_or_else(|| {
            UpdateError::Problem(Problem::BadArchive(format!(
                "{RELEASE_FILE} is missing from the release archive"
            )))
        })?;

    let bad = damaged_files(staged, &next)?;
    if !bad.is_empty() {
        return Err(UpdateError::Problem(Problem::BadArchive(format!(
            "staged archive is missing or has corrupt files: {}",
            bad.join(", ")
        ))));
    }

    let modified = local_changes(root, previous, &next)?;
    if !modified.is_empty() {
        return Err(UpdateError::Problem(Problem::LocalChanges(modified)));
    }

    remove_moved_aside(root);

    let marker = installing_marker(root);
    if let Some(parent) = marker.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&marker, format!("{}\n", next.version))?;

    // What the retired Node services installed beside the release (RB15); setup moves the game data beside them
    for rel in [
        "modules/rustic-isles/reference/1.14.7/ParadoxBackend/node_modules",
        "modules/rustic-isles/reference/1.14.7/ParadoxDirector/node_modules",
        "ParadoxBackend/node_modules",
        "ParadoxDirector/node_modules",
    ] {
        let nm = rel.split('/').fold(root.to_path_buf(), |p, s| p.join(s));
        if nm.is_dir() {
            let _ = fs::remove_dir_all(&nm);
        }
    }

    let mut next_paths = BTreeSet::new();
    let mut written_files = 0usize;
    for f in &next.files {
        next_paths.insert(f.path.clone());
        let src = staged.join(&f.path);
        let dst = root.join(&f.path);
        if previous.is_some() && dst.is_file() && sha256_file(&dst).ok().as_deref() == Some(f.sha256.as_str()) {
            continue;
        }
        install_single_file(&src, &dst)?;
        written_files += 1;
    }

    let mut removed = 0usize;
    if let Some(prev) = previous {
        for pf in &prev.files {
            if !next_paths.contains(&pf.path) {
                let old_path = root.join(&pf.path);
                if old_path.is_file() && fs::remove_file(&old_path).is_ok() {
                    removed += 1;
                    remove_empty_parent_dirs(old_path.parent(), root);
                }
            }
        }
    }

    install_single_file(&staged.join(RELEASE_FILE), &root.join(RELEASE_FILE))?;
    let _ = fs::remove_file(&marker);

    let rt_changed = previous.map(|prev| runtime_changed(prev, &next));
    let info = Installed {
        from: previous.map(|p| p.version.clone()),
        to: next.version.clone(),
        files: written_files,
        removed,
        runtime_changed: rt_changed,
    };
    Ok((next, info))
}

/// Plain HTTPS GETs for the update, following redirects.
pub mod http {
    use std::fs::{self, File};
    use std::io::Write;
    use std::path::Path;

    use bytes::Bytes;
    use http_body_util::{BodyExt, Empty};
    use hyper::Request;

    async fn fetch_bytes(url: &str, api: bool) -> Result<Vec<u8>, String> {
        let client: dainty_http::Client<Empty<Bytes>> = dainty_http::client(dainty_http::Schemes::HttpOrHttps);

        let mut current = url.to_owned();
        for _ in 0..5 {
            let uri: hyper::Uri = current.parse().map_err(|e| format!("invalid URL {current}: {e}"))?;
            let mut builder = Request::builder()
                .method("GET")
                .uri(&uri)
                .header("User-Agent", "dainty-lass-hostkit");
            if api {
                builder = builder.header("Accept", "application/vnd.github+json");
            }
            let req = builder
                .body(Empty::<Bytes>::new())
                .map_err(|e| format!("{current}: {e}"))?;
            let resp = client.request(req).await.map_err(|e| format!("{current}: {e}"))?;
            let status = resp.status();
            if status.is_redirection()
                && let Some(loc) = resp.headers().get(hyper::header::LOCATION)
                && let Ok(loc_str) = loc.to_str()
            {
                current = loc_str.to_owned();
                continue;
            }
            if !status.is_success() {
                return Err(format!("HTTP {} from {current}", status.as_u16()));
            }
            let expected_len = resp
                .headers()
                .get(hyper::header::CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.trim().parse::<u64>().ok());
            let body = resp
                .into_body()
                .collect()
                .await
                .map_err(|e| format!("download from {current} was interrupted: {e}"))?
                .to_bytes();
            if let Some(len) = expected_len
                && (body.len() as u64) < len
            {
                return Err(format!(
                    "download from {current} was interrupted ({} of {len} bytes)",
                    body.len()
                ));
            }
            return Ok(body.to_vec());
        }
        Err(format!("too many redirects fetching {url}"))
    }

    fn run_async<T>(fut: impl std::future::Future<Output = Result<T, String>>) -> Result<T, String> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        rt.block_on(fut)
    }

    /// The body at `url` as text; `api` asks for GitHub's API JSON.
    pub fn get_text(url: &str, api: bool) -> Result<String, String> {
        let bytes = run_async(fetch_bytes(url, api))?;
        String::from_utf8(bytes).map_err(|e| format!("invalid UTF-8 from {url}: {e}"))
    }

    /// Downloads `url` to `file`, making its folder.
    pub fn download(url: &str, file: &Path) -> Result<(), String> {
        let bytes = run_async(fetch_bytes(url, false))?;
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut out = File::create(file).map_err(|e| e.to_string())?;
        if let Err(e) = out.write_all(&bytes) {
            drop(out);
            let _ = fs::remove_file(file);
            return Err(e.to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    #[test]
    fn an_archive_past_its_unpacked_budget_is_refused() {
        let tmp = TempDir::new().unwrap();
        let archive = tmp.path().join("bomb.tar.gz");
        let gz = flate2::write::GzEncoder::new(File::create(&archive).unwrap(), flate2::Compression::best());
        let mut builder = tar::Builder::new(gz);
        let data = vec![0u8; 64 * 1024];
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, "release.json", data.as_slice())
            .unwrap();
        builder.into_inner().unwrap().finish().unwrap();

        let dest = tmp.path().join("out");
        let refused = extract_within(&archive, &dest, 1024).unwrap_err();
        assert!(refused.to_string().contains("unpacks to more than"), "{refused}");
        assert!(extract_within(&archive, &dest, 64 * 1024).is_ok());
    }

    #[test]
    fn a_plain_http_source_must_be_on_this_machine() {
        for url in [
            "http://127.0.0.1:8080/r",
            "http://localhost/r",
            "http://[::1]:9/r",
            "https://example.com/r",
        ] {
            assert!(!is_remote_plain_http(url), "{url}");
        }
        for url in [
            "http://example.com/r",
            "http://127.0.0.1.example.com/r",
            "http://localhost@evil.test/r",
        ] {
            assert!(is_remote_plain_http(url), "{url}");
        }
        let refused = find_release(
            &Source::parse("http://example.com/releases"),
            "linux-x64",
            Wanted::Newest(None),
        );
        assert!(refused.unwrap_err().to_string().contains("must be https://"));
    }

    #[test]
    fn semver_parsing_and_ordering() {
        let v1 = Version::parse("0.1.0").unwrap();
        let v2 = Version::parse("0.2.0-alpha.1").unwrap();
        let v3 = Version::parse("0.2.0-alpha.2").unwrap();
        let v4 = Version::parse("0.2.0-beta.1").unwrap();
        let v5 = Version::parse("0.2.0").unwrap();

        assert!(v1 < v2);
        assert!(v2 < v3);
        assert!(v3 < v4);
        assert!(v4 < v5);
        assert!(!v1.is_prerelease());
        assert!(v2.is_prerelease());
        assert!(Version::parse("01.0.0").is_none());
        assert!(Version::parse("1.0").is_none());
    }

    #[test]
    fn pick_newest_skips_prereleases_unless_installed_is_prerelease() {
        let versions = ["0.1.0", "0.2.0", "0.3.0-rc.1"];
        assert_eq!(pick_newest(&versions, None), Some("0.2.0"));
        assert_eq!(pick_newest(&versions, Some("0.1.0")), Some("0.2.0"));
        assert_eq!(pick_newest(&versions, Some("0.3.0-alpha.1")), Some("0.3.0-rc.1"));
    }

    #[test]
    fn archive_name_and_version_roundtrip() {
        let win = archive_name("0.4.2", "windows-x64");
        assert_eq!(win, "DaintyLassServer-0.4.2-windows-x64.zip");
        assert_eq!(archive_version(&win, "windows-x64"), Some("0.4.2"));
        assert_eq!(archive_version(&win, "linux-x64"), None);

        let lin = archive_name("0.4.2", "linux-x64");
        assert_eq!(lin, "DaintyLassServer-0.4.2-linux-x64.tar.gz");
        assert_eq!(archive_version(&lin, "linux-x64"), Some("0.4.2"));
    }

    #[test]
    fn candidates_from_github_selects_matching_release() {
        let releases = json!([
            {
                "tag_name": "server-v0.1.0",
                "draft": false,
                "assets": [
                    {
                        "name": "DaintyLassServer-0.1.0-linux-x64.tar.gz",
                        "browser_download_url": "https://example.org/0.1.0.tar.gz"
                    },
                    {
                        "name": "SHA256SUMS.txt",
                        "browser_download_url": "https://example.org/0.1.0-sums.txt"
                    }
                ]
            },
            {
                "tag_name": "server-v0.2.0",
                "draft": false,
                "assets": [
                    {
                        "name": "DaintyLassServer-0.2.0-linux-x64.tar.gz",
                        "browser_download_url": "https://example.org/0.2.0.tar.gz"
                    },
                    {
                        "name": "SHA256SUMS.txt",
                        "browser_download_url": "https://example.org/0.2.0-sums.txt"
                    }
                ]
            }
        ]);

        let c =
            candidates_from_github(&releases, "Dainty-Lass/Dainty-Lass", "linux-x64", Wanted::Newest(None)).unwrap();
        assert_eq!(c.version, "0.2.0");
        assert_eq!(c.archive, "DaintyLassServer-0.2.0-linux-x64.tar.gz");
    }

    #[test]
    fn install_verifies_staged_files_and_refuses_local_changes() {
        let tmp = TempDir::new().unwrap();
        let staged = tmp.path().join("staged");
        let root = tmp.path().join("root");
        fs::create_dir_all(&staged).unwrap();
        fs::create_dir_all(&root).unwrap();

        let hello_bytes = b"hello world\n";
        fs::write(staged.join("README.txt"), hello_bytes).unwrap();
        let hello_sha = sha256_file(&staged.join("README.txt")).unwrap();

        let manifest_json = json!({
            "version": "0.2.0",
            "commit": "abcdef1234567890",
            "target": "x86_64-unknown-linux-gnu",
            "platform": "linux-x64",
            "repository": "Dainty-Lass/Dainty-Lass",
            "files": [
                {
                    "path": "README.txt",
                    "sha256": hello_sha,
                    "size": hello_bytes.len()
                }
            ]
        });
        fs::write(
            staged.join(RELEASE_FILE),
            serde_json::to_string_pretty(&manifest_json).unwrap(),
        )
        .unwrap();

        let (installed_manifest, stats) = install(&staged, &root, None).unwrap();
        assert_eq!(installed_manifest.version, "0.2.0");
        assert_eq!(installed_manifest.short_commit(), "abcdef123456");
        assert_eq!(stats.files, 1);
        assert!(root.join("README.txt").is_file());
        assert!(!installing_marker(&root).exists());

        // Modify README.txt locally and verify a subsequent install with different hash refuses to overwrite it
        fs::write(root.join("README.txt"), b"locally edited\n").unwrap();
        let next_bytes = b"version 0.3.0\n";
        fs::write(staged.join("README.txt"), next_bytes).unwrap();
        let next_sha = sha256_file(&staged.join("README.txt")).unwrap();
        let next_manifest_json = json!({
            "version": "0.3.0",
            "commit": "1234567890abcdef",
            "target": "x86_64-unknown-linux-gnu",
            "platform": "linux-x64",
            "files": [
                {
                    "path": "README.txt",
                    "sha256": next_sha,
                    "size": next_bytes.len()
                }
            ]
        });
        fs::write(
            staged.join(RELEASE_FILE),
            serde_json::to_string_pretty(&next_manifest_json).unwrap(),
        )
        .unwrap();

        let err = install(&staged, &root, Some(&installed_manifest)).unwrap_err();
        match err {
            UpdateError::Problem(Problem::LocalChanges(files)) => {
                assert_eq!(files, vec!["README.txt".to_owned()]);
            }
            other => panic!("expected LocalChanges, got {other:?}"),
        }
    }
}
