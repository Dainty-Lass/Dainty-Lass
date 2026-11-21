// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Installing a version from Epic (DE5, DD22), on `dainty_epic::download` through `net::epic_install`'s job. One install
//! runs at a time, each version into a folder of its own; the resume file and the record of a finished install
//! sit in `userData/epic/`, never in the game folder. The build is the compiled-in client profile's (DE4,
//! `net::versions::builtin`): its pin, its sources and the executable's hash.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use dainty_epic::download::{
    DownloadError, Executable, Gate, Https, ManifestOrigin, Options, Refusal, Stage, Target, fetch_manifest,
};
use dainty_epic::manifest::{InstallPath, Manifest, Sha1Digest};
use dainty_module_api::GameVersion;

use super::Controller;
use super::platform::FolderPurpose;
use super::stored::edit;
use crate::bridge::types::{
    ActionResult, ErrorCode, InstallInfo, InstallOutcome, InstallState, LauncherError, NoticeCode, TaskKind,
    TaskProgress, UninstallOutcome, UninstallPlan,
};
use crate::net::epic_install::{InstallJob, InstallProgress};
use crate::net::versions::{HttpSources, KnownProfile, Sources, builtin};
use crate::platform::epic_library::{self, EpicOwnership};
use crate::platform::epic_session::Epic;
use crate::platform::game_removal::{Removal, RemovalError, Scope};
use crate::platform::log;

/// The Epic session and DE3's ownership check, as the installer's gate: it refreshes the session, then asks the
/// cached answer, and refuses when either fails.
pub struct EpicGate {
    /// The signed-in Epic account.
    pub epic: Arc<Epic>,
    /// Its ownership check.
    pub ownership: Arc<EpicOwnership>,
}

impl EpicGate {
    /// The page's code for a refusal.
    pub async fn check(&self) -> Result<(), LauncherError> {
        let session = self.epic.fresh_session().await?;
        epic_library::require(&self.ownership, session.as_ref()).await
    }
}

impl Gate for EpicGate {
    async fn require(&self) -> Result<(), Refusal> {
        self.check().await.map_err(|error| match error.code {
            ErrorCode::EpicNotOwned => Refusal::NotOwned,
            ErrorCode::EpicSignedOut => Refusal::SignedOut,
            _ => Refusal::Unreachable,
        })
    }
}

/// The compiled-in profile that names an Epic build for `version`.
fn known_profile(version: &str) -> Option<KnownProfile> {
    let version = version.parse::<GameVersion>().ok()?;
    builtin::profiles()
        .into_iter()
        .find(|profile| profile.game_version == version && profile.client.epic.is_some())
}

/// Where versions go until the player chooses a folder: `~/Games/Dauntless`.
fn default_install_root() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join("Games").join("Dauntless"))
}

/// What the page is told when an install stops.
pub fn launcher_error(error: &DownloadError) -> LauncherError {
    let code = match error {
        DownloadError::NotOwned => ErrorCode::EpicNotOwned,
        DownloadError::SignedOut => ErrorCode::EpicSignedOut,
        DownloadError::Library => ErrorCode::EpicLibraryFailed,
        DownloadError::NoSpace { needed, .. } => {
            return LauncherError::with_detail(ErrorCode::DiskSpace, &needed.to_string());
        }
        DownloadError::Io { kind, .. } if *kind == std::io::ErrorKind::StorageFull => ErrorCode::DiskSpace,
        DownloadError::LinkInFolder { name } => return LauncherError::with_detail(ErrorCode::FolderInvalid, name),
        DownloadError::ChunkUnavailable { deleted: true, .. } => ErrorCode::GameVersionUnavailable,
        DownloadError::ExecutableHash { .. } | DownloadError::ExecutableMissing { .. } => ErrorCode::GameWrongVersion,
        DownloadError::FileMismatch { name } => return LauncherError::with_detail(ErrorCode::DownloadFailed, name),
        DownloadError::Cancelled => ErrorCode::Cancelled,
        _ => ErrorCode::DownloadFailed,
    };
    LauncherError::new(code)
}

/// What the page is told when an uninstall stops: the file that would not go, as `detail`.
fn removal_error(error: &RemovalError) -> LauncherError {
    match error {
        RemovalError::Missing => LauncherError::new(ErrorCode::GameFolderNotFound),
        RemovalError::FolderIsLink => LauncherError::new(ErrorCode::FolderInvalid),
        RemovalError::Io { path, .. } => {
            let name = path.file_name().map(|name| name.to_string_lossy().into_owned());
            LauncherError::with_detail(ErrorCode::Unknown, &name.unwrap_or_default())
        }
    }
}

/// `path` is `folder` or inside it; Windows paths compare without regard to case.
fn inside(path: &Path, folder: &Path) -> bool {
    if cfg!(windows) {
        let lower = |p: &Path| PathBuf::from(p.to_string_lossy().replace('/', "\\").to_lowercase());
        lower(path).starts_with(lower(folder))
    } else {
        path.starts_with(folder)
    }
}

/// The page's task progress for an install.
fn task(progress: InstallProgress) -> TaskProgress {
    TaskProgress {
        kind: TaskKind::Install,
        total_bytes: progress.total_bytes,
        done_bytes: progress.done_bytes,
        files_total: progress.files_total,
        files_done: progress.files_done,
        bytes_per_second: progress.bytes_per_second,
        eta_seconds: progress.eta_seconds,
        current_file: progress.current_file,
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

enum Entry {
    Running { job: Arc<InstallJob>, folder: PathBuf },
    Stopped { error: LauncherError, folder: PathBuf },
}

/// The installs this run of the launcher knows about, by version.
#[derive(Default)]
pub struct Installs {
    entries: Mutex<HashMap<String, Entry>>,
}

impl Controller {
    fn epic_dir(&self) -> PathBuf {
        self.user_data().join("epic")
    }

    fn installed_folder(&self, version: &str) -> Option<String> {
        std::fs::read_to_string(self.epic_dir().join(format!("{version}.installed"))).ok()
    }

    fn resume_file(&self, version: &str) -> PathBuf {
        self.epic_dir().join(format!("{version}.resume"))
    }

    /// Names the folder an install made for itself: only that one is deleted when the download is discarded.
    fn created_file(&self, version: &str) -> PathBuf {
        self.epic_dir().join(format!("{version}.created"))
    }

    /// Names the folder an install made, kept once it finished: uninstalling deletes all of that folder and only
    /// the manifest's files from any other.
    fn owned_file(&self, version: &str) -> PathBuf {
        self.epic_dir().join(format!("{version}.owned"))
    }

    /// The manifest's file names, one a line, so an uninstall needs no download.
    fn files_file(&self, version: &str) -> PathBuf {
        self.epic_dir().join(format!("{version}.files"))
    }

    /// The folder versions install into, one folder each: the player's choice, or `~/Games/Dauntless`.
    fn install_root(&self) -> Option<PathBuf> {
        std::fs::read_to_string(self.epic_dir().join("install-root"))
            .ok()
            .map(|text| PathBuf::from(text.trim()))
            .filter(|path| path.is_absolute())
            .or_else(default_install_root)
    }

    fn install_folder(&self, version: &str) -> Option<PathBuf> {
        Some(self.install_root()?.join(version))
    }

    /// Whether an install runs, or one stopped part-way waits to continue in the folder it began in.
    fn install_pending(&self) -> bool {
        let stopped = lock(&self.shared.installs.entries)
            .values()
            .any(|entry| matches!(entry, Entry::Running { .. } | Entry::Stopped { .. }));
        let resumable = std::fs::read_dir(self.epic_dir())
            .map(|dir| {
                dir.flatten()
                    .any(|e| e.path().extension().is_some_and(|x| x == "resume"))
            })
            .unwrap_or(false);
        stopped || resumable
    }

    /// "Change folder": a folder dialog for where versions install. Refused while an install runs or waits to
    /// continue, since its resume record is for the folder it began in.
    pub async fn game_install_choose_folder(&self) -> ActionResult {
        if self.install_pending() {
            return ActionResult::err(ErrorCode::Busy);
        }
        let start = self.install_root();
        let picked = self
            .platform()
            .choose_folder(self.language(), FolderPurpose::GameInstall, start)
            .await;
        let Some(picked) = picked else {
            return ActionResult::err(ErrorCode::Cancelled);
        };
        if !picked.is_absolute() || !picked.is_dir() {
            return ActionResult::err(ErrorCode::FolderInvalid);
        }
        if self.install_pending() {
            return ActionResult::err(ErrorCode::Busy);
        }
        let saved = std::fs::create_dir_all(self.epic_dir()).and_then(|()| {
            std::fs::write(
                self.epic_dir().join("install-root"),
                picked.to_string_lossy().as_bytes(),
            )
        });
        match saved {
            Ok(()) => ActionResult::Ok,
            Err(error) => {
                log::warn(format!("the install folder was not saved: {error}"));
                ActionResult::err(ErrorCode::FolderInvalid)
            }
        }
    }

    /// Starts (or picks up) the install of `version`. An account that doesn't own Dauntless, or no session, is
    /// refused here, before anything is created.
    pub async fn game_install_start(&self, version: &str, gate: Arc<EpicGate>) -> InstallOutcome {
        let Some(profile) = known_profile(version) else {
            return InstallOutcome::Err(LauncherError::with_detail(ErrorCode::GameVersionUnknown, version));
        };
        if self.installed_folder(version).is_some() {
            return self.game_install_get(version);
        }
        if let Err(error) = gate.check().await {
            return InstallOutcome::Err(error);
        }
        let Some(folder) = self.install_folder(version) else {
            return InstallOutcome::Err(LauncherError::new(ErrorCode::FolderInvalid));
        };
        if !folder.exists() {
            let made = std::fs::create_dir_all(self.epic_dir())
                .and_then(|()| std::fs::write(self.created_file(version), folder.to_string_lossy().as_bytes()));
            if let Err(error) = made {
                log::warn(format!("the install's folder record was not saved: {error}"));
            }
        }
        let this = self.clone();
        let job = {
            let mut entries = lock(&self.shared.installs.entries);
            let running = entries
                .iter()
                .find(|(_, entry)| matches!(entry, Entry::Running { .. }))
                .map(|(running, _)| running.clone());
            match running {
                Some(running) if running == version => {
                    drop(entries);
                    return self.game_install_get(version);
                }
                Some(_) => return InstallOutcome::Err(LauncherError::new(ErrorCode::Busy)),
                None => {}
            }
            let job = Arc::new(InstallJob::new());
            entries.insert(
                version.to_owned(),
                Entry::Running {
                    job: job.clone(),
                    folder: folder.clone(),
                },
            );
            job
        };
        let owned = version.to_owned();
        // Not a tracked task: exit doesn't wait for a download, whose resume file picks it up again
        tokio::spawn(async move { this.run_install(owned, profile, job, folder, gate).await });
        self.game_install_get(version)
    }

    async fn run_install(
        &self,
        version: String,
        profile: KnownProfile,
        job: Arc<InstallJob>,
        folder: PathBuf,
        gate: Arc<EpicGate>,
    ) {
        log::info(format!("installing {version} from Epic into {}", folder.display()));
        let outcome = self.install_version(&version, &profile, &job, &folder, &gate).await;
        let record = self.epic_dir().join(format!("{version}.installed"));
        let outcome = outcome.and_then(|files| {
            let names: String = files.iter().map(|file| format!("{}\n", file.as_str())).collect();
            std::fs::create_dir_all(self.epic_dir())
                .and_then(|()| std::fs::write(self.files_file(&version), names))
                .and_then(|()| std::fs::write(&record, folder.to_string_lossy().as_bytes()))
                .map_err(|_| LauncherError::with_detail(ErrorCode::DownloadFailed, "record"))
        });
        match outcome {
            Ok(()) => {
                // An installed game is never the discard's to delete, but the uninstall's when the install made it
                let made = std::fs::read_to_string(self.created_file(&version)).ok();
                if made.is_some_and(|made| Path::new(&made) == folder)
                    && let Err(error) = std::fs::write(self.owned_file(&version), folder.to_string_lossy().as_bytes())
                {
                    log::warn(format!("the install's folder record was not kept: {error}"));
                }
                let _ = std::fs::remove_file(self.created_file(&version));
                lock(&self.shared.installs.entries).remove(&version);
                log::info(format!("installed {version} from Epic"));
                self.set_notice(NoticeCode::InstallDone);
            }
            Err(error) => {
                log::warn(format!("installing {version} stopped: {:?}", error.code));
                lock(&self.shared.installs.entries).insert(version, Entry::Stopped { error, folder });
                self.publish();
            }
        }
    }

    /// The joined servers' copies of the manifest come first (DD22), each from its own route with its own key;
    /// a copy that is not the pinned one is passed over by `fetch_manifest` like any other source.
    async fn server_manifest(&self, version: &GameVersion, pin: &Sha1Digest) -> Result<Vec<u8>, String> {
        let sources = HttpSources::default();
        for server in self.joined_servers().await {
            if let Some(bytes) = sources.server_manifest(&server, version).await
                && Sha1Digest::of(&bytes) == *pin
            {
                return Ok(bytes.to_vec());
            }
        }
        Err("no joined server holds it".to_owned())
    }

    /// The version's manifest under its pin.
    async fn pinned_manifest(
        &self,
        version: &str,
        profile: &KnownProfile,
    ) -> Result<(Manifest, Sha1Digest), LauncherError> {
        let broken = || LauncherError::with_detail(ErrorCode::Unknown, "client_profile");
        let epic = profile.client.epic.as_ref().ok_or_else(broken)?;
        let pin = Sha1Digest(*epic.manifest_sha1.as_bytes());
        let https = Https::new(None, Duration::from_secs(60)).map_err(|e| launcher_error(&e.into()))?;
        let server = self.server_manifest(&profile.game_version, &pin);
        let (manifest, origin) = fetch_manifest(Some(server), &epic.manifest_sources, &pin, &https)
            .await
            .map_err(|unavailable| {
                for (origin, why) in &unavailable.tried {
                    log::warn(format!("manifest for {version} from {origin:?}: {why}"));
                }
                LauncherError::new(ErrorCode::EpicManifestUnavailable)
            })?;
        match origin {
            ManifestOrigin::Server => log::info(format!("manifest for {version} from a joined server")),
            ManifestOrigin::Source(url) => log::info(format!("manifest for {version} from {url}")),
        }
        Ok((manifest, pin))
    }

    /// Installs `version`, answering the manifest's file names.
    async fn install_version(
        &self,
        version: &str,
        profile: &KnownProfile,
        job: &InstallJob,
        folder: &Path,
        gate: &EpicGate,
    ) -> Result<Vec<InstallPath>, LauncherError> {
        let broken = || LauncherError::with_detail(ErrorCode::Unknown, "client_profile");
        let epic = profile.client.epic.as_ref().ok_or_else(broken)?;
        let executable = Executable {
            path: InstallPath::new(&profile.client.executable).map_err(|_| broken())?,
            sha256: *epic.executable_sha256.as_bytes(),
        };
        let (manifest, pin) = self.pinned_manifest(version, profile).await?;
        let target = Target {
            manifest: &manifest,
            manifest_sha1: pin,
            folder: folder.to_path_buf(),
            executable: Some(executable),
            resume_file: self.resume_file(version),
        };
        let options = Options::new(Arc::new(|path: &Path| {
            crate::platform::system::free_bytes(path).ok_or_else(|| std::io::Error::other("free space unknown"))
        }));
        let platform = Arc::clone(&self.shared.platform);
        let report = move |progress: InstallProgress| platform.emit_progress(&task(progress));
        job.run(&target, &options, gate, &report)
            .await
            .map_err(|e| launcher_error(&e))?;
        Ok(manifest.files().iter().map(|file| file.path.clone()).collect())
    }

    fn with_job(&self, version: &str, act: impl FnOnce(&InstallJob) -> bool) -> ActionResult {
        match lock(&self.shared.installs.entries).get(version) {
            Some(Entry::Running { job, .. }) if act(job) => ActionResult::Ok,
            _ => ActionResult::err(ErrorCode::Unknown),
        }
    }

    /// `game_install_pause`.
    pub fn game_install_pause(&self, version: &str) -> ActionResult {
        self.with_job(version, InstallJob::pause)
    }

    /// `game_install_resume`.
    pub fn game_install_resume(&self, version: &str) -> ActionResult {
        self.with_job(version, InstallJob::resume)
    }

    /// Stops the download; its resume file stays, so starting again picks it up.
    pub fn game_install_cancel(&self, version: &str) -> ActionResult {
        self.with_job(version, InstallJob::cancel)
    }

    /// Cancels the download altogether: stops it if it runs, then deletes what it downloaded and its resume
    /// record, so the version is not installed again. The folder goes only if this install made it; one that was
    /// there before keeps its files.
    pub async fn game_install_discard(&self, version: &str) -> ActionResult {
        if self.installed_folder(version).is_some() {
            return ActionResult::err(ErrorCode::Busy);
        }
        let running = match lock(&self.shared.installs.entries).get(version) {
            Some(Entry::Running { job, .. }) => job.cancel(),
            _ => false,
        };
        // The job ends between chunks; until it has, its files are still being written
        for _ in 0..300 {
            if !running
                || !matches!(
                    lock(&self.shared.installs.entries).get(version),
                    Some(Entry::Running { .. })
                )
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        if matches!(
            lock(&self.shared.installs.entries).get(version),
            Some(Entry::Running { .. })
        ) {
            return ActionResult::err(ErrorCode::Busy);
        }
        let made = std::fs::read_to_string(self.created_file(version))
            .ok()
            .map(PathBuf::from);
        let folder = self.install_folder(version);
        let removed = match (made, folder) {
            (Some(made), Some(folder)) if made == folder && folder.is_dir() => {
                tokio::task::spawn_blocking(move || std::fs::remove_dir_all(&folder))
                    .await
                    .unwrap_or_else(|_| Err(std::io::Error::other("the delete stopped")))
            }
            _ => Ok(()),
        };
        if let Err(error) = removed {
            log::warn(format!("the download of {version} was not deleted: {error}"));
            return ActionResult::err(ErrorCode::DownloadFailed);
        }
        let _ = std::fs::remove_file(self.resume_file(version));
        let _ = std::fs::remove_file(self.created_file(version));
        lock(&self.shared.installs.entries).remove(version);
        log::info(format!("the download of {version} was cancelled and deleted"));
        self.publish();
        ActionResult::Ok
    }

    /// The installed folder of `version` and what goes from it: all of it when the install made it, otherwise the
    /// manifest's files (from the record, or the manifest again for an install from before the record).
    async fn uninstall_scope(&self, version: &str) -> Result<(PathBuf, Option<Vec<InstallPath>>), LauncherError> {
        let Some(profile) = known_profile(version) else {
            return Err(LauncherError::with_detail(ErrorCode::GameVersionUnknown, version));
        };
        let Some(folder) = self.installed_folder(version).map(PathBuf::from) else {
            return Err(LauncherError::new(ErrorCode::GameFolderNotFound));
        };
        let owned = std::fs::read_to_string(self.owned_file(version)).ok();
        if owned.is_some_and(|owned| Path::new(&owned) == folder) {
            return Ok((folder, None));
        }
        let recorded = std::fs::read_to_string(self.files_file(version)).ok().and_then(|text| {
            text.lines()
                .filter(|line| !line.is_empty())
                .map(|line| InstallPath::new(line).ok())
                .collect::<Option<Vec<_>>>()
        });
        let files = match recorded {
            Some(files) if !files.is_empty() => files,
            _ => {
                let (manifest, _) = self.pinned_manifest(version, &profile).await?;
                manifest.files().iter().map(|file| file.path.clone()).collect()
            }
        };
        Ok((folder, Some(files)))
    }

    async fn plan_removal(&self, version: &str) -> Result<Removal, LauncherError> {
        let (folder, files) = self.uninstall_scope(version).await?;
        tokio::task::spawn_blocking(move || {
            let scope = files.as_deref().map_or(Scope::Whole, Scope::Listed);
            Removal::plan(&folder, &scope)
        })
        .await
        .map_err(|_| LauncherError::new(ErrorCode::Unknown))?
        .map_err(|error| removal_error(&error))
    }

    /// `game_uninstall_plan`: the folder and the bytes an uninstall frees, for the page to confirm.
    pub async fn game_uninstall_plan(&self, version: &str) -> UninstallOutcome {
        match self.plan_removal(version).await {
            Ok(removal) => UninstallOutcome::Ok(UninstallPlan {
                version: version.to_owned(),
                folder: removal.folder().to_string_lossy().into_owned(),
                bytes: removal.bytes(),
                whole_folder: removal.whole(),
            }),
            Err(error) => UninstallOutcome::Err(error),
        }
    }

    /// `game_uninstall`: deletes an installed version for good and forgets it, and the game folder when it was
    /// this one. Refused while any game client runs, since it holds its files.
    pub async fn game_uninstall(&self, version: &str) -> ActionResult {
        if self.shared.game.is_running() || !self.platform().running_clients().await.is_empty() {
            return ActionResult::err(ErrorCode::AlreadyRunning);
        }
        {
            let state = self.lock();
            if state.busy || state.task.is_some() || lock(&self.shared.installs.entries).contains_key(version) {
                return ActionResult::err(ErrorCode::Busy);
            }
        }
        let removal = match self.plan_removal(version).await {
            Ok(removal) => removal,
            // Already gone from disk: only the records are left to forget
            Err(error) if error.code == ErrorCode::GameFolderNotFound && self.installed_folder(version).is_some() => {
                return self.forget_install(version, None).await;
            }
            Err(error) => return ActionResult::Err(error),
        };
        let folder = removal.folder().to_path_buf();
        log::info(format!(
            "uninstalling {version}: {} files, {} bytes{}",
            removal.count(),
            removal.bytes(),
            if removal.whole() { ", the whole folder" } else { "" }
        ));
        let removed = tokio::task::spawn_blocking(move || removal.remove())
            .await
            .unwrap_or(Err(RemovalError::Missing));
        if let Err(error) = removed {
            log::warn(format!("uninstalling {version} stopped: {error}"));
            return ActionResult::Err(removal_error(&error));
        }
        self.forget_install(version, Some(&folder)).await
    }

    async fn forget_install(&self, version: &str, folder: Option<&Path>) -> ActionResult {
        let folder = folder
            .map(Path::to_path_buf)
            .or_else(|| self.installed_folder(version).map(PathBuf::from));
        for record in [
            self.epic_dir().join(format!("{version}.installed")),
            self.files_file(version),
            self.owned_file(version),
            self.resume_file(version),
            self.created_file(version),
        ] {
            let _ = std::fs::remove_file(record);
        }
        let root = self.stored().game_root().map(PathBuf::from);
        if let (Some(root), Some(folder)) = (root, folder)
            && inside(&root, &folder)
        {
            self.edit_settings(|map| edit::field(map, "gameRoot", serde_json::Value::Null))
                .await;
        }
        log::info(format!("uninstalled {version}"));
        self.look_at_game(false).await;
        self.look_at_runtime().await;
        self.publish();
        ActionResult::Ok
    }

    /// `game_install_get`.
    pub fn game_install_get(&self, version: &str) -> InstallOutcome {
        if known_profile(version).is_none() {
            return InstallOutcome::Err(LauncherError::with_detail(ErrorCode::GameVersionUnknown, version));
        }
        let info = |state, folder: Option<String>, error| InstallInfo {
            version: version.to_owned(),
            state,
            folder,
            progress: None,
            error,
        };
        let entries = lock(&self.shared.installs.entries);
        let found = match entries.get(version) {
            Some(Entry::Running { job, folder }) => {
                let state = match job.last().map(|progress| progress.stage) {
                    None | Some(Stage::Preparing) => InstallState::Preparing,
                    Some(Stage::Downloading) => InstallState::Downloading,
                    Some(Stage::Paused) => InstallState::Paused,
                    Some(Stage::Finishing | Stage::Done) => InstallState::Finishing,
                };
                InstallInfo {
                    progress: job.progress().map(task),
                    ..info(state, Some(folder.to_string_lossy().into_owned()), None)
                }
            }
            Some(Entry::Stopped { error, folder }) => {
                let state = if self.resume_file(version).exists() {
                    InstallState::Resumable
                } else {
                    InstallState::Failed
                };
                info(state, Some(folder.to_string_lossy().into_owned()), Some(error.clone()))
            }
            None => match self.installed_folder(version) {
                Some(folder) => info(InstallState::Installed, Some(folder), None),
                None if self.resume_file(version).exists() => info(
                    InstallState::Resumable,
                    self.install_folder(version).map(|f| f.to_string_lossy().into_owned()),
                    None,
                ),
                // Where it would go, so the page can show it before anything is downloaded
                None => info(
                    InstallState::NotInstalled,
                    self.install_folder(version).map(|f| f.to_string_lossy().into_owned()),
                    None,
                ),
            },
        };
        InstallOutcome::Ok(found)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

    use super::*;
    use dainty_epic::manifest::Guid;

    #[test]
    fn the_compiled_in_profiles_name_what_an_install_needs() {
        let version = "1.14.7";
        let profile = known_profile(version).unwrap();
        let epic = profile.client.epic.as_ref().unwrap();
        assert!(!epic.manifest_sources.is_empty());
        assert!(InstallPath::new(&profile.client.executable).is_ok());
        assert!(InstallPath::new(version).is_ok(), "a version names a folder");
        // The executable the install checks is the one the launcher checks before PLAY
        let current = known_profile("1.14.7").unwrap();
        assert_eq!(
            current.client.epic.unwrap().executable_sha256.to_string(),
            crate::core::pins::PINNED_EXE_SHA256
        );
        assert_eq!(current.client.executable, crate::core::pins::EXE_RELATIVE_PATH);
        // 1.4.4 has no archived manifest (DD22), and anything else is no version
        assert!(known_profile("1.4.4").is_none());
        assert!(known_profile("9.9.9").is_none());
        assert!(known_profile("../evil").is_none());
    }

    #[test]
    fn install_errors_map_to_codes_the_page_knows() {
        let code = |e: DownloadError| launcher_error(&e).code;
        assert_eq!(code(DownloadError::NotOwned), ErrorCode::EpicNotOwned);
        assert_eq!(code(DownloadError::SignedOut), ErrorCode::EpicSignedOut);
        assert_eq!(code(DownloadError::Library), ErrorCode::EpicLibraryFailed);
        assert_eq!(
            launcher_error(&DownloadError::NoSpace {
                needed: 5,
                available: 1
            }),
            LauncherError::with_detail(ErrorCode::DiskSpace, "5")
        );
        let chunk = |deleted| DownloadError::ChunkUnavailable {
            guid: Guid([0; 4]),
            deleted,
            reasons: Vec::new(),
        };
        assert_eq!(code(chunk(true)), ErrorCode::GameVersionUnavailable);
        assert_eq!(code(chunk(false)), ErrorCode::DownloadFailed);
        assert_eq!(
            code(DownloadError::ExecutableHash { name: "x".into() }),
            ErrorCode::GameWrongVersion
        );
        assert_eq!(code(DownloadError::Cancelled), ErrorCode::Cancelled);
        assert_eq!(
            code(DownloadError::LinkInFolder { name: "a".into() }),
            ErrorCode::FolderInvalid
        );
    }
}
