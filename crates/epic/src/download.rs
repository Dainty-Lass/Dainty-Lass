//! Installing a build from Epic's CDN (DD22, DE5). [`install`] asks the [`Gate`] first and fetches nothing for an
//! account without Dauntless, even though the CDN would serve it. Then, into one folder per version:
//!
//! 1. the free space is checked against what the files still need, and every file is created at its size, each
//!    folder on the way checked not to be a symlink or junction;
//! 2. the chunks the files use are fetched, up to [`Options::parallel`] at once, each from the first of the three
//!    [`CDN_BASES`] that answers (a host that failed goes to the back), checked by [`crate::chunk::decode`]: a
//!    chunk that doesn't match is fetched again from the next host;
//! 3. each chunk's parts are written where the manifest puts them, and once a file has all its parts it is checked
//!    against the manifest's SHA-1 (and SHA-256 and MD5 where it has them); a file that fails has its chunks
//!    fetched once more, a second failure stops the install;
//! 4. last, the executable's SHA-256 from the client profile.
//!
//! The resume file ([`resume`]) records each chunk written and each file checked, so a download that is paused,
//! cancelled or killed picks up where it was. [`Control`] pauses, resumes and cancels a running one.
//!
//! The client speaks HTTPS only and follows no redirect: a chunk comes from these hosts or not at all
//! ([`cdn`]). Written from Epic's formats and Legendary's documented behaviour, not from Dauntless Revived's
//! downloader, which the launcher's AGPL job around this keeps (DD26).

mod cdn;
pub(crate) mod md5;
pub mod resume;
mod sources;
#[cfg(test)]
mod tests;

pub use cdn::{FetchError, Https};
pub use sources::{MAX_MANIFEST_BYTES, ManifestOrigin, ManifestUnavailable, fetch_manifest};

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{File, OpenOptions};
use std::future::Future;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use sha1::Sha1;
use sha2::{Digest, Sha256};
use tokio::sync::watch;
use tokio::task::JoinSet;

use crate::auth::Session;
use crate::chunk;
use crate::library::{Library, LibraryError, Ownership, OwnershipError, require_ownership};
use crate::manifest::{ChunkInfo, FileEntry, Guid, InstallPath, Manifest, Sha1Digest};
use resume::ResumeFile;

/// Dauntless's build folder on each of Epic's CDN hosts, in the order the archive's guide gives them.
pub const CDN_BASES: [&str; 3] = [
    "https://epicgames-download1.akamaized.net/Builds/Org/o-krlzxj88qrtb69fredeuaf887bl5az/Jackal/default",
    "https://egdownload.fastly-edge.com/Builds/Org/o-krlzxj88qrtb69fredeuaf887bl5az/Jackal/default",
    "https://egs-cloudfront-chunks.epicgamescdn.com/Builds/Org/o-krlzxj88qrtb69fredeuaf887bl5az/Jackal/default",
];

/// A `.chunk` body may be this much larger than the manifest's download size before it is refused.
const CHUNK_SLACK: usize = 1024;
/// Files checked at once; each is read whole, so more would only compete for the disk.
const VERIFY_PARALLEL: usize = 2;

/// A chunk's URL under one of [`CDN_BASES`].
pub fn chunk_url(base: &str, manifest: &Manifest, chunk: &ChunkInfo) -> String {
    format!("{base}/{}", chunk.cdn_path(manifest.feature_level))
}

/// Why the gate said no.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Refusal {
    NotOwned,
    /// No Epic session, or Epic refused the stored one
    SignedOut,
    /// The library could not be read
    Unreachable,
}

/// Whether the account may install: asked before anything is created or fetched. The launcher's refreshes the
/// session and asks DE3's cached ownership check; [`Owned`] is the same for a session in hand.
pub trait Gate {
    fn require(&self) -> impl Future<Output = Result<(), Refusal>> + Send;
}

/// A session and the ownership check to put it through ([`require_ownership`]).
pub struct Owned<'a, L>(pub &'a Ownership<L>, pub &'a Session);

impl<L: Library> Gate for Owned<'_, L> {
    async fn require(&self) -> Result<(), Refusal> {
        require_ownership(self.0, self.1).await.map_err(|error| match error {
            OwnershipError::NotOwned => Refusal::NotOwned,
            OwnershipError::Library(LibraryError::SessionExpired) => Refusal::SignedOut,
            _ => Refusal::Unreachable,
        })
    }
}

/// Why an install failed. File names are shown escaped (`{:?}`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum DownloadError {
    #[error("the account does not own Dauntless")]
    NotOwned,
    #[error("there is no Epic session")]
    SignedOut,
    #[error("the Epic library could not be read")]
    Library,
    #[error("{needed} bytes are needed and {available} are free")]
    NoSpace { needed: u64, available: u64 },
    #[error("{name:?} is a symlink or junction inside the install folder")]
    LinkInFolder { name: String },
    #[error("{}: {message}", name.as_deref().unwrap_or("install folder"))]
    Io {
        name: Option<String>,
        kind: io::ErrorKind,
        message: String,
    },
    #[error("no CDN host gave chunk {guid} ({})", reasons.join("; "))]
    ChunkUnavailable {
        guid: Guid,
        /// Every host answered 403 or 404: Epic has deleted it, and the version can't be installed
        deleted: bool,
        reasons: Vec<String>,
    },
    #[error("{name:?} does not match the manifest")]
    FileMismatch { name: String },
    #[error("the client profile's executable {name:?} is not in the manifest")]
    ExecutableMissing { name: String },
    #[error("{name:?} is not the executable the client profile pins")]
    ExecutableHash { name: String },
    #[error("cancelled")]
    Cancelled,
    #[error(transparent)]
    Fetch(#[from] FetchError),
}

fn io_error(name: Option<&InstallPath>) -> impl Fn(io::Error) -> DownloadError + '_ {
    move |e| DownloadError::Io {
        name: name.map(|n| n.to_string()),
        kind: e.kind(),
        message: e.to_string(),
    }
}

/// What [`Control`] asks of a running install.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Run,
    Pause,
    Cancel,
}

/// Pauses, resumes and cancels an install from outside it. A cancel is final; the resume file stays, so a later
/// install of the same version picks up where this one stopped.
#[derive(Clone)]
pub struct Control(Arc<watch::Sender<Mode>>);

impl Default for Control {
    fn default() -> Self {
        Self(Arc::new(watch::channel(Mode::Run).0))
    }
}

impl Control {
    pub fn mode(&self) -> Mode {
        *self.0.borrow()
    }

    /// False when it was not running.
    pub fn pause(&self) -> bool {
        self.set(Mode::Run, Mode::Pause)
    }

    /// False when it was not paused.
    pub fn resume(&self) -> bool {
        self.set(Mode::Pause, Mode::Run)
    }

    pub fn cancel(&self) -> bool {
        self.0.send_if_modified(|mode| {
            let changed = *mode != Mode::Cancel;
            *mode = Mode::Cancel;
            changed
        })
    }

    fn set(&self, from: Mode, to: Mode) -> bool {
        self.0.send_if_modified(|mode| {
            let changed = *mode == from;
            if changed {
                *mode = to;
            }
            changed
        })
    }
}

/// Where an install stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// Checking the space and creating the files
    Preparing,
    Downloading,
    Paused,
    /// Checking the executable
    Finishing,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub stage: Stage,
    /// `.chunk` bytes, of the chunks the files use
    pub download_total: u64,
    pub downloaded: u64,
    pub chunks_total: usize,
    pub chunks_done: usize,
    pub files_total: usize,
    /// Files checked against the manifest
    pub files_done: usize,
    pub install_total: u64,
}

/// The executable the client profile pins (DE4): checked by SHA-256 once every file matches the manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Executable {
    pub path: InstallPath,
    pub sha256: [u8; 32],
}

/// One version to install.
pub struct Target<'a> {
    pub manifest: &'a Manifest,
    /// The pin `manifest` was parsed under; it names the resume file's download
    pub manifest_sha1: Sha1Digest,
    /// This version's own folder (one folder per version)
    pub folder: PathBuf,
    pub executable: Option<Executable>,
    /// In the launcher's or the host kit's data folder
    pub resume_file: PathBuf,
}

pub type Backoff = Arc<dyn Fn(u32) -> Duration + Send + Sync>;
/// Free bytes on the drive that holds a folder (or its nearest existing parent).
pub type SpaceProbe = Arc<dyn Fn(&Path) -> io::Result<u64> + Send + Sync>;

#[derive(Clone)]
pub struct Options {
    pub bases: Vec<String>,
    /// Chunks fetched at once, 1 to 32
    pub parallel: usize,
    /// Passes over every host before a chunk is given up
    pub rounds: u32,
    /// The wait before pass `n` (from 1)
    pub backoff: Backoff,
    /// One request, from connecting to the body's end
    pub timeout: Duration,
    /// The tests' own CA instead of the webpki roots
    pub tls: Option<Arc<rustls::ClientConfig>>,
    pub available_space: SpaceProbe,
}

impl Options {
    pub fn new(available_space: SpaceProbe) -> Self {
        Self {
            bases: CDN_BASES.iter().map(|base| (*base).to_owned()).collect(),
            parallel: 8,
            rounds: 3,
            backoff: Arc::new(|pass| Duration::from_secs(1 << pass.saturating_sub(1).min(3))),
            timeout: Duration::from_secs(60),
            tls: None,
            available_space,
        }
    }
}

/// A finished install.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub folder: PathBuf,
    pub files: usize,
    pub bytes: u64,
}

/// Bytes `from..from + len` of a chunk go to `at` in file `file`.
#[derive(Debug, Clone, Copy)]
struct PartWrite {
    file: usize,
    at: u64,
    from: u32,
    len: u32,
}

/// What the chunk tasks share.
struct Shared {
    https: Https,
    bases: Vec<String>,
    /// Failures since the last success, per host: the hosts are tried fewest first
    failures: Vec<AtomicU32>,
    rounds: u32,
    backoff: Backoff,
    feature_level: u32,
    chunks: Vec<ChunkInfo>,
    writes: Vec<Vec<PartWrite>>,
    paths: Vec<PathBuf>,
    names: Vec<InstallPath>,
}

enum ChunkFailure {
    Unavailable {
        guid: Guid,
        deleted: bool,
        reasons: Vec<String>,
    },
    Write(DownloadError),
}

impl Shared {
    fn host_order(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.bases.len()).collect();
        order.sort_by_key(|&host| self.failures[host].load(Ordering::Relaxed));
        order
    }

    /// Fetches, checks and writes chunk `c`.
    async fn chunk(self: Arc<Self>, c: usize) -> Result<usize, ChunkFailure> {
        let info = &self.chunks[c];
        let cap = usize::try_from(info.download_size)
            .unwrap_or(usize::MAX)
            .saturating_add(CHUNK_SLACK);
        let path = info.cdn_path(self.feature_level);
        let mut reasons = Vec::new();
        let mut deleted = true;
        for pass in 0..self.rounds.max(1) {
            if pass > 0 {
                tokio::time::sleep((self.backoff)(pass)).await;
            }
            for host in self.host_order() {
                let url = format!("{}/{path}", self.bases[host]);
                let why = match self.https.get(&url, cap).await {
                    Ok(raw) => {
                        deleted = false;
                        let expected = info.clone();
                        match tokio::task::spawn_blocking(move || chunk::decode(&raw, &expected)).await {
                            Ok(Ok(data)) => {
                                self.failures[host].store(0, Ordering::Relaxed);
                                return self.write(c, data).await.map_err(ChunkFailure::Write);
                            }
                            Ok(Err(e)) => e.to_string(),
                            Err(e) => e.to_string(),
                        }
                    }
                    Err(e) => {
                        deleted &= e.is_missing();
                        e.to_string()
                    }
                };
                self.failures[host].fetch_add(1, Ordering::Relaxed);
                reasons.push(format!("{}: {why}", self.bases[host]));
            }
        }
        Err(ChunkFailure::Unavailable {
            guid: info.guid,
            deleted,
            reasons,
        })
    }

    async fn write(self: &Arc<Self>, c: usize, data: Vec<u8>) -> Result<usize, DownloadError> {
        let shared = self.clone();
        tokio::task::spawn_blocking(move || {
            for part in &shared.writes[c] {
                let name = &shared.names[part.file];
                let bytes = data
                    .get(part.from as usize..part.from as usize + part.len as usize)
                    .ok_or_else(|| io_error(Some(name))(io::Error::other("part outside its chunk")))?;
                let file = OpenOptions::new()
                    .write(true)
                    .open(&shared.paths[part.file])
                    .map_err(io_error(Some(name)))?;
                write_at(&file, bytes, part.at).map_err(io_error(Some(name)))?;
            }
            Ok(c)
        })
        .await
        .map_err(|e| io_error(None)(io::Error::other(e.to_string())))?
    }
}

#[cfg(unix)]
fn write_at(file: &File, bytes: &[u8], at: u64) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.write_all_at(bytes, at)
}

#[cfg(windows)]
fn write_at(file: &File, bytes: &[u8], at: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    let mut done = 0;
    while done < bytes.len() {
        match file.seek_write(&bytes[done..], at + done as u64)? {
            0 => return Err(io::ErrorKind::WriteZero.into()),
            n => done += n,
        }
    }
    Ok(())
}

/// The length of `name` under `root`, 0 when it doesn't exist yet. Refuses a symlink or junction anywhere on the
/// way, so nothing is written outside the folder through one (`docs/security/epic.md`); with `create`, makes the
/// folders on the way.
fn walk(root: &Path, name: &InstallPath, create: bool) -> Result<u64, DownloadError> {
    let components: Vec<&str> = name.components().collect();
    let mut path = root.to_path_buf();
    for (i, component) in components.iter().enumerate() {
        path.push(component);
        let last = i + 1 == components.len();
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(DownloadError::LinkInFolder {
                    name: components[..=i].join("/"),
                });
            }
            Ok(meta) if last && meta.is_file() => return Ok(meta.len()),
            Ok(meta) if !last && meta.is_dir() => {}
            Ok(_) => {
                return Err(io_error(Some(name))(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "something else is in the way",
                )));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                if last {
                    return Ok(0);
                }
                if create {
                    match std::fs::create_dir(&path) {
                        Err(e) if e.kind() != io::ErrorKind::AlreadyExists => return Err(io_error(Some(name))(e)),
                        _ => {}
                    }
                }
            }
            Err(e) => return Err(io_error(Some(name))(e)),
        }
    }
    Ok(0)
}

/// Checks the space the files still need and creates each at its size; returns each file's length before.
fn prepare(
    folder: &Path,
    files: &[FileEntry],
    paths: &[PathBuf],
    probe: &SpaceProbe,
) -> Result<Vec<u64>, DownloadError> {
    std::fs::create_dir_all(folder).map_err(io_error(None))?;
    let meta = std::fs::symlink_metadata(folder).map_err(io_error(None))?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(DownloadError::LinkInFolder { name: ".".to_owned() });
    }
    let mut before = Vec::with_capacity(files.len());
    let mut needed = 0u64;
    for entry in files {
        let len = walk(folder, &entry.path, false)?;
        needed = needed.saturating_add(entry.size.saturating_sub(len));
        before.push(len);
    }
    let available = probe(folder).map_err(io_error(None))?;
    if needed > available {
        return Err(DownloadError::NoSpace { needed, available });
    }
    for (entry, path) in files.iter().zip(paths) {
        walk(folder, &entry.path, true)?;
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(io_error(Some(&entry.path)))?;
        let len = file.metadata().map_err(io_error(Some(&entry.path)))?.len();
        if len != entry.size {
            file.set_len(entry.size).map_err(io_error(Some(&entry.path)))?;
        }
    }
    Ok(before)
}

/// Whether the file on disk is the manifest's: its size, SHA-1, and SHA-256 and MD5 where listed. A match is
/// flushed to disk before the resume file says so.
fn verify(path: &Path, entry: &FileEntry) -> io::Result<bool> {
    let mut file = File::open(path)?;
    if file.metadata()?.len() != entry.size {
        return Ok(false);
    }
    let mut sha1 = Sha1::new();
    let mut sha256 = entry.sha256.map(|_| Sha256::new());
    let mut md5 = entry.md5.map(|_| md5::Md5::new());
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        sha1.update(&buffer[..n]);
        if let Some(h) = sha256.as_mut() {
            h.update(&buffer[..n]);
        }
        if let Some(h) = md5.as_mut() {
            h.update(&buffer[..n]);
        }
    }
    let matches = Sha1Digest(sha1.finalize().into()) == entry.sha1
        && sha256.is_none_or(|h| Some(<[u8; 32]>::from(h.finalize())) == entry.sha256)
        && md5.is_none_or(|h| Some(h.finish()) == entry.md5);
    if matches {
        OpenOptions::new().write(true).open(path)?.sync_all()?;
        #[cfg(unix)]
        if entry.flags.unix_executable() {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    Ok(matches)
}

fn sha256_of(path: &Path) -> io::Result<[u8; 32]> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            return Ok(hash.finalize().into());
        }
        hash.update(&buffer[..n]);
    }
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, DownloadError> + Send + 'static,
) -> Result<T, DownloadError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| io_error(None)(io::Error::other(e.to_string())))?
}

/// The install's bookkeeping, held by the one task that drives it.
struct Job<'a> {
    manifest: &'a Manifest,
    shared: Arc<Shared>,
    resume: ResumeFile,
    /// Distinct chunks per file, and files per chunk
    file_chunks: Vec<Vec<usize>>,
    chunk_files: Vec<Vec<usize>>,
    done: Vec<bool>,
    verified: Vec<bool>,
    /// Chunks of each file not written yet
    remaining: Vec<usize>,
    /// Files that failed their check once and had their chunks fetched again
    refetched: HashSet<usize>,
    queue: VecDeque<usize>,
    to_verify: VecDeque<usize>,
    fetching: JoinSet<Result<usize, ChunkFailure>>,
    in_flight: HashMap<tokio::task::Id, usize>,
    verifying: JoinSet<(usize, io::Result<bool>)>,
    parallel: usize,
    used: Vec<usize>,
}

impl Job<'_> {
    fn progress(&self, stage: Stage) -> Progress {
        let chunks = &self.shared.chunks;
        Progress {
            stage,
            download_total: self.used.iter().map(|&c| chunks[c].download_size).sum(),
            downloaded: self
                .used
                .iter()
                .filter(|&&c| self.done[c])
                .map(|&c| chunks[c].download_size)
                .sum(),
            chunks_total: self.used.len(),
            chunks_done: self.used.iter().filter(|&&c| self.done[c]).count(),
            files_total: self.verified.len(),
            files_done: self.verified.iter().filter(|v| **v).count(),
            install_total: self.manifest.install_size(),
        }
    }

    fn resume_error(e: io::Error) -> DownloadError {
        DownloadError::Io {
            name: Some("resume file".to_owned()),
            kind: e.kind(),
            message: e.to_string(),
        }
    }

    fn start_work(&mut self) {
        while self.fetching.len() < self.parallel
            && let Some(c) = self.queue.pop_front()
        {
            let handle = self.fetching.spawn(self.shared.clone().chunk(c));
            self.in_flight.insert(handle.id(), c);
        }
        while self.verifying.len() < VERIFY_PARALLEL
            && let Some(f) = self.to_verify.pop_front()
        {
            if self.verified[f] || self.remaining[f] > 0 {
                continue;
            }
            let path = self.shared.paths[f].clone();
            let entry = self.manifest.files()[f].clone();
            self.verifying.spawn(async move {
                let result = tokio::task::spawn_blocking(move || verify(&path, &entry))
                    .await
                    .unwrap_or_else(|e| Err(io::Error::other(e.to_string())));
                (f, result)
            });
        }
    }

    fn idle(&self) -> bool {
        self.queue.is_empty() && self.to_verify.is_empty() && self.fetching.is_empty() && self.verifying.is_empty()
    }

    fn chunk_done(&mut self, c: usize) -> Result<(), DownloadError> {
        if self.done[c] {
            return Ok(());
        }
        self.done[c] = true;
        self.resume
            .chunk_done(self.shared.chunks[c].guid)
            .map_err(Self::resume_error)?;
        for &f in &self.chunk_files[c] {
            if !self.verified[f] {
                self.remaining[f] = self.remaining[f].saturating_sub(1);
                if self.remaining[f] == 0 {
                    self.to_verify.push_back(f);
                }
            }
        }
        Ok(())
    }

    fn on_fetched(
        &mut self,
        joined: Result<(tokio::task::Id, Result<usize, ChunkFailure>), tokio::task::JoinError>,
    ) -> Result<(), DownloadError> {
        match joined {
            Ok((id, result)) => {
                self.in_flight.remove(&id);
                match result {
                    Ok(c) => self.chunk_done(c),
                    Err(ChunkFailure::Write(e)) => Err(e),
                    Err(ChunkFailure::Unavailable { guid, deleted, reasons }) => {
                        Err(DownloadError::ChunkUnavailable { guid, deleted, reasons })
                    }
                }
            }
            // Only a pause cancels a chunk task; it goes back to the front of the queue
            Err(e) if e.is_cancelled() => {
                if let Some(c) = self.in_flight.remove(&e.id()) {
                    self.queue.push_front(c);
                }
                Ok(())
            }
            Err(e) => Err(io_error(None)(io::Error::other(e.to_string()))),
        }
    }

    fn on_verified(
        &mut self,
        joined: Result<(usize, io::Result<bool>), tokio::task::JoinError>,
    ) -> Result<(), DownloadError> {
        let (f, result) = joined.map_err(|e| io_error(None)(io::Error::other(e.to_string())))?;
        // A chunk it shares was fetched again meanwhile: it is checked again once that lands
        if self.verified[f] || self.remaining[f] > 0 {
            return Ok(());
        }
        let name = &self.manifest.files()[f].path;
        match result.map_err(io_error(Some(name)))? {
            true => {
                self.verified[f] = true;
                self.resume.file_verified(f).map_err(Self::resume_error)
            }
            false if self.refetched.insert(f) => self.refetch(f),
            false => Err(DownloadError::FileMismatch { name: name.to_string() }),
        }
    }

    /// A file failed its check: its chunks are fetched again (from the first host that answers).
    fn refetch(&mut self, f: usize) -> Result<(), DownloadError> {
        for c in self.file_chunks[f].clone() {
            if !self.done[c] {
                continue;
            }
            self.done[c] = false;
            self.resume
                .chunk_undone(self.shared.chunks[c].guid)
                .map_err(Self::resume_error)?;
            for &g in &self.chunk_files[c] {
                if !self.verified[g] {
                    self.remaining[g] += 1;
                }
            }
            self.queue.push_front(c);
        }
        if self.remaining[f] == 0 {
            self.to_verify.push_back(f);
        }
        Ok(())
    }

    /// Stops the chunk tasks for a pause or a cancel, keeping what finished; waits for the file checks.
    async fn settle(&mut self) -> Result<(), DownloadError> {
        self.fetching.abort_all();
        while let Some(joined) = self.fetching.join_next_with_id().await {
            self.on_fetched(joined)?;
        }
        while let Some(joined) = self.verifying.join_next().await {
            self.on_verified(joined)?;
        }
        Ok(())
    }
}

/// Installs `target` (DE5): see the module's documentation for the steps.
pub async fn install(
    target: &Target<'_>,
    options: &Options,
    gate: &impl Gate,
    control: &Control,
    on_progress: &(dyn Fn(&Progress) + Send + Sync),
) -> Result<Installed, DownloadError> {
    gate.require().await.map_err(|refusal| match refusal {
        Refusal::NotOwned => DownloadError::NotOwned,
        Refusal::SignedOut => DownloadError::SignedOut,
        _ => DownloadError::Library,
    })?;
    let manifest = target.manifest;
    let files = manifest.files();
    let executable = match &target.executable {
        Some(exe) => Some((
            files
                .iter()
                .position(|f| f.path == exe.path)
                .ok_or_else(|| DownloadError::ExecutableMissing {
                    name: exe.path.to_string(),
                })?,
            exe.sha256,
        )),
        None => None,
    };

    let chunk_index: HashMap<Guid, usize> = manifest.chunks().iter().enumerate().map(|(i, c)| (c.guid, i)).collect();
    let mut writes = vec![Vec::new(); manifest.chunks().len()];
    let mut file_chunks = vec![Vec::new(); files.len()];
    let mut chunk_files: Vec<Vec<usize>> = vec![Vec::new(); manifest.chunks().len()];
    let mut used = Vec::new();
    for (f, entry) in files.iter().enumerate() {
        let mut at = 0u64;
        for part in &entry.parts {
            // The parser refuses a part naming a chunk it doesn't list
            let Some(&c) = chunk_index.get(&part.guid) else {
                continue;
            };
            writes[c].push(PartWrite {
                file: f,
                at,
                from: part.offset,
                len: part.size,
            });
            if !file_chunks[f].contains(&c) {
                file_chunks[f].push(c);
            }
            if chunk_files[c].is_empty() {
                used.push(c);
            }
            if chunk_files[c].last() != Some(&f) {
                chunk_files[c].push(f);
            }
            at += u64::from(part.size);
        }
    }
    let paths: Vec<PathBuf> = files.iter().map(|f| f.path.under(&target.folder)).collect();

    let (resume_path, pin, folder) = (target.resume_file.clone(), target.manifest_sha1, target.folder.clone());
    let (resume, earlier) = blocking(move || {
        ResumeFile::open(&resume_path, &pin, &folder).map_err(|e| DownloadError::Io {
            name: Some("resume file".to_owned()),
            kind: e.kind(),
            message: e.to_string(),
        })
    })
    .await?;

    let shared = Arc::new(Shared {
        https: Https::new(options.tls.clone(), options.timeout)?,
        failures: options.bases.iter().map(|_| AtomicU32::new(0)).collect(),
        bases: options.bases.clone(),
        rounds: options.rounds,
        backoff: options.backoff.clone(),
        feature_level: manifest.feature_level,
        chunks: manifest.chunks().to_vec(),
        writes,
        paths,
        names: files.iter().map(|f| f.path.clone()).collect(),
    });
    let mut job = Job {
        manifest,
        shared: shared.clone(),
        resume,
        file_chunks,
        chunk_files,
        done: manifest
            .chunks()
            .iter()
            .map(|c| earlier.chunks.contains(&c.guid))
            .collect(),
        verified: (0..files.len()).map(|f| earlier.files.contains(&f)).collect(),
        remaining: vec![0; files.len()],
        refetched: HashSet::new(),
        queue: VecDeque::new(),
        to_verify: VecDeque::new(),
        fetching: JoinSet::new(),
        in_flight: HashMap::new(),
        verifying: JoinSet::new(),
        parallel: options.parallel.clamp(1, 32),
        used,
    };
    on_progress(&job.progress(Stage::Preparing));

    let (folder, probe, entries) = (target.folder.clone(), options.available_space.clone(), files.to_vec());
    let before = blocking(move || prepare(&folder, &entries, &shared.paths, &probe)).await?;
    for (f, entry) in files.iter().enumerate() {
        // A file that is missing or of another size has lost whatever the resume file says was written to it
        if before[f] != entry.size {
            job.verified[f] = false;
            for &c in &job.file_chunks[f] {
                job.done[c] = false;
            }
        }
    }
    for f in 0..files.len() {
        job.remaining[f] = job.file_chunks[f].iter().filter(|&&c| !job.done[c]).count();
        if !job.verified[f] && job.remaining[f] == 0 {
            job.to_verify.push_back(f);
        }
    }
    job.queue = job.used.iter().copied().filter(|&c| !job.done[c]).collect();

    let mut mode = control.0.subscribe();
    loop {
        let now = *mode.borrow_and_update();
        match now {
            Mode::Cancel => {
                job.settle().await?;
                return Err(DownloadError::Cancelled);
            }
            Mode::Pause => {
                job.settle().await?;
                on_progress(&job.progress(Stage::Paused));
                while *mode.borrow_and_update() == Mode::Pause {
                    if mode.changed().await.is_err() {
                        return Err(DownloadError::Cancelled);
                    }
                }
                continue;
            }
            Mode::Run => {}
        }
        job.start_work();
        if job.idle() {
            break;
        }
        let before = job.progress(Stage::Downloading);
        tokio::select! {
            Some(joined) = job.fetching.join_next_with_id(), if !job.fetching.is_empty() => job.on_fetched(joined)?,
            Some(joined) = job.verifying.join_next(), if !job.verifying.is_empty() => job.on_verified(joined)?,
            _ = mode.changed() => {}
        }
        let after = job.progress(Stage::Downloading);
        if after != before {
            on_progress(&after);
        }
    }

    on_progress(&job.progress(Stage::Finishing));
    if let Some((f, expected)) = executable {
        let path = job.shared.paths[f].clone();
        let name = files[f].path.clone();
        let actual = blocking(move || sha256_of(&path).map_err(io_error(Some(&name)))).await?;
        if actual != expected {
            return Err(DownloadError::ExecutableHash {
                name: files[f].path.to_string(),
            });
        }
    }
    let done = job.progress(Stage::Done);
    job.resume.remove().map_err(Job::resume_error)?;
    on_progress(&done);
    Ok(Installed {
        folder: target.folder.clone(),
        files: files.len(),
        bytes: manifest.install_size(),
    })
}
