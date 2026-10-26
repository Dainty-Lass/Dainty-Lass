//! The legacy edition (`legacy` feature, RD5 reversed): Dauntless 1.4.4 for Dauntless Revived's servers,
//! played as DR's launcher at 24b7624 plays them. Its controller paths (the install folder, install and
//! repair, the download, PLAY) on its modules (`core::manifest`, `net::downloader`, `net::autoregion`,
//! `platform::client_mods`), in the shape of the Electron launcher's `revived` path (`controller.ts`),
//! which the page already draws: `Snapshot.revived`, `revivedInstall`, `revivedChooseFolder` and
//! `revivedUseDetected`.
//!
//! The `gameEdition` setting picks the edition; the current server is always one of it (`settings.rs`).
//! A Dauntless Revived server speaks DR's API (`hostapi::REVIVED`). Its 1.4.4 folder gets the
//! maintainer's UndauntedInternalServer.dll through Reforged's runtime loader (`winmm.dll` and a
//! `mystic_loader.ini` naming it, RD9), never DR's `dxgi.dll`, and DR's client mods; never Reforged's
//! runtime.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;

use super::stored::{StoredServer, set_text};
use super::{Controller, Outcome, State, err, result};
use crate::bridge::types::{
    ActionResult, ConnectProblem, DetectedGame, ErrorCode, Game, GameProblem, InstallMode, NoticeCode, Phase,
    RevivedInfo, TaskKind, TaskProgress,
};
use crate::core::game_language::game_locale;
use crate::core::legacy_pins::{DISK_MARGIN_BYTES, EXE_NAME, EXE_RELATIVE_PATH, RELAY_PORT, XMPP_PORT};
use crate::core::manifest::{GAME_MANIFEST, GameManifest, ManifestFile, compare_manifests, resolve_inside};
use crate::net::downloader::{DownloadErrorCode, DownloadJob, DownloadOptions};
use crate::net::hostapi::legacy::{end_playing, fetch_content_manifest, probe_content, save_hunt_region};
use crate::net::hostapi::{Flavor, REFORGED, REVIVED, fetch_user_info_as};
use crate::net::http::Endpoint;
use crate::net::verify::{Abort, VerifiedCache, VerifyError, VerifyOptions, remove_part_files, verify_install};
use crate::platform::client_mods::{
    FetchBytes, Release, dll_status, ensure_dll_cache, https_get, install_client_mods, install_pinned_dlls,
    remove_dr_proxy, win64_dir,
};
use crate::platform::dialogs::FolderPurpose;
use crate::platform::game_config::{ApplyConfig, apply_game_config};
use crate::platform::game_folder::{GameFolder, locate_existing_game};
use crate::platform::launch::{LaunchRuntime, describe_launch, legacy};
use crate::platform::log;
use crate::platform::system::free_bytes;

use super::platform::Prepared;

/// What the app knows about 1.4.4 before the controller starts.
#[derive(Clone, Default)]
pub struct LegacyConfig {
    /// The folder that holds `client-mods/` (and the two DLLs, when a build bundles them)
    pub resources_dir: PathBuf,
    /// Where 1.4.4 goes when the player chose no folder: Dauntless Revived's own launcher's folder. None:
    /// `userData/revived/Game`
    pub default_dir: Option<PathBuf>,
    /// Dauntless Revived's launcher's `settings.json`, whose `installDir` names a 1.4.4 it downloaded
    pub dr_settings: Option<PathBuf>,
    /// Tests: a small manifest. None: the compiled-in one
    pub manifest: Option<GameManifest>,
    /// Tests: where the DLLs come from. None: HTTPS
    pub dll_fetch: Option<FetchBytes>,
    /// Tests: other pins for the 1.4.4 DLL and the loader. None: the maintainer's release
    pub release: Option<Release>,
    /// Tests: where the relay listens in public mode. None: DR's 61000
    pub relay_port: Option<u16>,
}

/// What the controller keeps for 1.4.4 between calls.
#[derive(Default)]
pub struct LegacyState {
    info: Option<RevivedInfo>,
    job: Option<Arc<DownloadJob>>,
    /// Public mode: does the gateway serve `/content/*`?
    content_probe: Option<bool>,
    /// The server a running 1.4.4 game plays on, told when the game ends (`PlayingEnded`)
    playing: Option<StoredServer>,
}

/// The files this launcher already hashed in the 1.4.4 folder (`userData/revived/verified-files.json`,
/// the Electron launcher's).
pub struct Parts {
    verified: Arc<tokio::sync::Mutex<VerifiedCache>>,
}

impl Parts {
    pub fn new(user_data: &Path) -> Self {
        Self {
            verified: Arc::new(tokio::sync::Mutex::new(VerifiedCache::load(
                user_data.join("revived").join("verified-files.json"),
            ))),
        }
    }
}

/// The API a server is asked through.
pub(super) fn flavor(sv: &StoredServer) -> Flavor {
    if sv.revived() { REVIVED } else { REFORGED }
}

fn blank(kind: TaskKind, total_bytes: u64, files_total: usize) -> TaskProgress {
    TaskProgress {
        kind,
        total_bytes,
        done_bytes: 0,
        files_total: files_total as u32,
        files_done: 0,
        bytes_per_second: 0.0,
        eta_seconds: None,
        current_file: None,
    }
}

fn io_code(error: &std::io::Error) -> ErrorCode {
    match error.kind() {
        std::io::ErrorKind::StorageFull => ErrorCode::DiskSpace,
        std::io::ErrorKind::PermissionDenied => ErrorCode::FolderInvalid,
        _ => ErrorCode::Unknown,
    }
}

/// The game's exe is there with the manifest's size.
fn exe_there(root: &Path, manifest: &GameManifest) -> bool {
    let Some(exe) = manifest.files.iter().find(|file| file.path == EXE_RELATIVE_PATH) else {
        return false;
    };
    resolve_inside(root, &exe.path)
        .ok()
        .and_then(|full| std::fs::metadata(full).ok())
        .is_some_and(|metadata| metadata.is_file() && metadata.len() == exe.size)
}

/// A 1.4.4 already on this PC (`detect.ts`): the folder Dauntless Revived's own launcher downloaded it
/// to, from its settings or its default, so it is not downloaded a second time.
fn detect_revived_installs(c: &LegacyConfig, manifest: &GameManifest) -> Vec<DetectedGame> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    let named = c
        .dr_settings
        .as_ref()
        .and_then(|file| std::fs::read(file).ok())
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .and_then(|raw| raw.get("installDir").and_then(Value::as_str).map(str::to_owned));
    if let Some(dir) = named.filter(|dir| Path::new(dir).is_absolute() && crate::core::text::js_length(dir) < 200) {
        candidates.push(PathBuf::from(dir));
    }
    candidates.extend(c.default_dir.clone());
    candidates.dedup();
    candidates
        .into_iter()
        .filter(|root| exe_there(root, manifest))
        .map(|root| DetectedGame {
            root: root.to_string_lossy().into_owned(),
            source: crate::bridge::types::GameSource::Revived,
            version: Some("1.4.4".into()),
        })
        .collect()
}

impl Controller {
    fn legacy_manifest(&self) -> &GameManifest {
        self.c.legacy.manifest.as_ref().unwrap_or(&GAME_MANIFEST)
    }

    fn release(&self) -> Release {
        self.c.legacy.release.clone().unwrap_or_else(Release::pinned)
    }

    fn revived_default(&self) -> PathBuf {
        self.c
            .legacy
            .default_dir
            .clone()
            .unwrap_or_else(|| self.c.user_data.join("revived").join("Game"))
    }

    fn is_revived(&self) -> bool {
        self.server().is_some_and(|sv| sv.revived())
    }

    /// After Register: the 1.4.4 folder checked against the manifest; the DLLs go in with it.
    pub(super) fn legacy_phase(&self, st: &State) -> Option<Phase> {
        if !st.s().server.as_ref().is_some_and(StoredServer::revived) {
            return None;
        }
        Some(match &st.legacy.info {
            Some(info) if info.problem.is_none() => Phase::Ready,
            _ => Phase::Locate,
        })
    }

    pub(super) fn revived_info(st: &State) -> Option<RevivedInfo> {
        if !st.s().server.as_ref().is_some_and(StoredServer::revived) {
            return None;
        }
        st.legacy.info.clone()
    }

    /// A server of the other edition: the page offers to switch (detail: the server's edition).
    pub(super) fn other_edition(&self, sv: &StoredServer) -> Option<Game> {
        (sv.edition() != self.s().game_edition).then(|| sv.edition())
    }

    /// The edition a patch switches to: the other one, while no task or game runs.
    pub(super) fn edition_change(&self, wanted: Option<Game>) -> Option<Game> {
        let wanted = wanted?;
        let st = self.st();
        (wanted != st.s().game_edition && !st.is_busy() && !self.game.running()).then_some(wanted)
    }

    /// `settings.rs` set aside a current server of the other edition; it stays in the list.
    pub(super) async fn edition_changed(&self) {
        {
            let mut st = self.st();
            if st.s().server.is_none() {
                st.reset_server();
            }
        }
        self.refresh_key_flag();
        self.inspect_revived(true).await;
        self.inspect_runtime().await;
        log::info(&format!("game edition: {}", self.s().game_edition));
    }

    pub(super) fn forget_legacy_server(st: &mut State) {
        st.legacy.content_probe = None;
        st.legacy.info = None;
    }

    /// The 1.4.4 folder: the chosen one, one Dauntless Revived's launcher downloaded (taken the first
    /// time), or the default. Every file is checked by size here; by hash on install and Check and repair.
    pub(super) async fn inspect_revived(&self, detect: bool) {
        if !self.is_revived() {
            self.st().legacy.info = None;
            return;
        }
        let manifest = self.legacy_manifest().clone();
        let previous = self.st().legacy.info.as_ref().map(|info| info.detected.clone());
        let detected = match previous {
            Some(detected) if !detect => detected,
            _ => {
                let (c, manifest) = (self.c.legacy.clone(), manifest.clone());
                tokio::task::spawn_blocking(move || detect_revived_installs(&c, &manifest))
                    .await
                    .unwrap_or_default()
            }
        };
        let mut chosen = self.s().revived_root;
        if chosen.is_none()
            && let Some(found) = detected.first()
        {
            let pick = found.root.clone();
            self.update_settings(|map| set_text(map, "revivedRoot", Some(&pick)))
                .await;
            log::info("using the Dauntless 1.4.4 folder of Dauntless Revived's launcher");
            chosen = Some(found.root.clone());
        }
        let root = chosen
            .clone()
            .unwrap_or_else(|| self.revived_default().to_string_lossy().into_owned());
        let dir = PathBuf::from(&root);
        let (problem, missing_files, dlls_ok) = if exe_there(&dir, &manifest) {
            let quick = verify_install(
                &dir,
                &manifest.files,
                VerifyOptions {
                    concurrency: Some(8),
                    ..VerifyOptions::default()
                },
            )
            .await;
            let missing = quick.map_or(manifest.files.len(), |report| report.bad.len());
            let verified = self.s().revived_verified.as_deref() == Some(root.as_str());
            let problem = (missing > 0 || !verified).then_some(GameProblem::Unverified);
            let dlls = {
                let (dir, release) = (dir.clone(), self.release());
                tokio::task::spawn_blocking(move || dll_status(&dir, &release).iter().all(|(_, ok)| *ok))
                    .await
                    .unwrap_or(false)
            };
            (problem, missing, dlls)
        } else {
            (Some(GameProblem::NotSet), manifest.files.len(), false)
        };
        self.st().legacy.info = Some(RevivedInfo {
            root,
            chosen: chosen.is_some(),
            problem,
            missing_files: missing_files as u32,
            total_bytes: manifest.total_bytes,
            dlls_ok,
            detected,
        });
    }

    fn may_change_folder(&self) -> bool {
        let st = self.st();
        !st.is_busy() && !self.game.running()
    }

    /// Choose a folder: one that holds 1.4.4 already (it, its parent, Archon or Win64), or an empty one to
    /// download it into.
    pub async fn revived_choose_folder(&self) -> ActionResult {
        if !self.may_change_folder() || !self.is_revived() {
            return ActionResult::err(ErrorCode::Busy);
        }
        let start = self
            .st()
            .legacy
            .info
            .as_ref()
            .map_or_else(|| self.revived_default(), |info| PathBuf::from(&info.root));
        let Some(picked) = self
            .p
            .choose_folder(self.language(), FolderPurpose::Revived, start)
            .await
        else {
            return ActionResult::err(ErrorCode::Cancelled);
        };
        result(self.use_revived_path(&picked.to_string_lossy()).await)
    }

    /// A folder for 1.4.4: one that holds it, or one the download may fill: empty, new, or holding only
    /// 1.4.4's own folders (a download that stopped). Another Dauntless (1.14.7) or anything else is never
    /// written into.
    async fn use_revived_path(&self, picked: &str) -> Outcome {
        let host = self.c.host;
        let found = {
            let picked = picked.to_owned();
            tokio::task::spawn_blocking(move || locate_existing_game(&picked, host))
                .await
                .unwrap_or(GameFolder::InvalidPath)
        };
        if !self.may_change_folder() {
            return Err(err(ErrorCode::Busy, None));
        }
        let manifest = self.legacy_manifest();
        match found {
            GameFolder::Found(root) if exe_there(Path::new(&root), manifest) => {
                return self.use_revived_root(&root).await;
            }
            GameFolder::Found(_) => return Err(self.fail(ErrorCode::FolderOtherGame, None)),
            GameFolder::InvalidPath | GameFolder::TooLong => return Err(self.fail(ErrorCode::FolderInvalid, None)),
            GameFolder::NotFound => {}
        }
        let own: std::collections::HashSet<String> = manifest
            .files
            .iter()
            .filter_map(|file| file.path.split('/').next())
            .map(str::to_ascii_lowercase)
            .collect();
        let mut entries = Vec::new();
        if let Ok(mut read) = tokio::fs::read_dir(picked).await {
            while let Ok(Some(entry)) = read.next_entry().await {
                entries.push(entry.file_name().to_string_lossy().to_ascii_lowercase());
            }
        }
        if entries.iter().any(|entry| !own.contains(entry)) {
            return Err(self.fail(ErrorCode::FolderNotEmpty, None));
        }
        self.use_revived_root(picked).await
    }

    pub async fn revived_use_detected(&self, index: u32) -> ActionResult {
        let found = self
            .st()
            .legacy
            .info
            .as_ref()
            .and_then(|info| info.detected.get(index as usize))
            .map(|detected| detected.root.clone());
        match found {
            Some(root) if self.may_change_folder() => result(self.use_revived_root(&root).await),
            Some(_) => ActionResult::err(ErrorCode::Busy),
            None => ActionResult::Err(self.fail(ErrorCode::FolderInvalid, None)),
        }
    }

    async fn use_revived_root(&self, root: &str) -> Outcome {
        self.update_settings(|map| set_text(map, "revivedRoot", Some(root)))
            .await;
        self.inspect_revived(false).await;
        self.publish();
        Ok(())
    }

    // ------------------------------------------------------------------ install / repair

    pub(super) fn cancel_revived(&self) {
        let job = self.st().legacy.job.clone();
        if let Some(job) = job {
            job.cancel();
        }
    }

    /// UE4SS and the client mods (`platform::client_mods`). Never fatal: the game runs without them.
    async fn install_mods(&self, dir: &Path) {
        let (resources, dir) = (self.c.legacy.resources_dir.clone(), dir.to_path_buf());
        match tokio::task::spawn_blocking(move || install_client_mods(&resources, &dir)).await {
            Ok(Ok(Some(done))) => log::info(&format!(
                "client mods: {} installed, {} up to date, {} kept, {} removed, {} switched off",
                done.installed, done.up_to_date, done.kept, done.removed, done.disabled
            )),
            Ok(Ok(None)) => {}
            Ok(Err(error)) => log::warn(&format!("client mods not installed: {error}")),
            Err(error) => log::warn(&format!("client mods not installed: {error}")),
        }
    }

    /// DR's `dxgi.dll` proxy would load the DLL a second time: DR's own goes, another is refused.
    fn clear_dr_proxy(&self, dir: &Path) -> Outcome {
        match remove_dr_proxy(dir) {
            Ok(true) => {
                log::info("removed Dauntless Revived's dxgi.dll: the runtime loader loads the 1.4.4 DLL");
                Ok(())
            }
            Ok(false) => Ok(()),
            Err(error) => {
                log::warn(&format!("PLAY refused: {error}"));
                Err(self.fail(ErrorCode::LoaderLeftovers, Some(error.file)))
            }
        }
    }

    /// The 1.4.4 DLL and the runtime loader next to the game, with its list (RD9), from the launcher's own
    /// copies.
    async fn install_dlls(&self, dir: &Path) -> Outcome {
        let cache = self.c.user_data.join("revived").join("dlls");
        let fetch = self.c.legacy.dll_fetch.clone().unwrap_or_else(https_get);
        let (bundled, release) = (self.c.legacy.resources_dir.clone(), self.release());
        if let Err(error) = ensure_dll_cache(&cache, &bundled, &release, fetch).await {
            log::error(&format!("DLL install failed: {error}"));
            return Err(self.fail(ErrorCode::DllFailed, Some(error.file)));
        }
        let target = dir.to_path_buf();
        match tokio::task::spawn_blocking(move || install_pinned_dlls(&cache, &target, &release)).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                log::error(&format!("DLL install failed: {error}"));
                Err(self.fail(ErrorCode::DllFailed, Some(error.file)))
            }
            Err(error) => {
                log::error(&format!("DLL install failed: {error}"));
                Err(self.fail(ErrorCode::DllFailed, None))
            }
        }
    }

    /// Install (or Check and repair): every file checked against the manifest (by hash, unless this
    /// launcher hashed it already, and always on repair), what is missing or damaged downloaded from the
    /// server's content server, then the client DLLs and mods.
    pub async fn revived_install(&self, mode: InstallMode) -> ActionResult {
        if self.st().is_busy() {
            return ActionResult::err(ErrorCode::Busy);
        }
        if self.game.running() {
            return ActionResult::Err(self.fail(ErrorCode::AlreadyRunning, None));
        }
        let Some(sv) = self.server().filter(StoredServer::revived) else {
            return ActionResult::Err(self.fail(ErrorCode::ServerUnreachable, None));
        };
        if self.st().legacy.info.is_none() {
            self.inspect_revived(false).await;
        }
        let repair = mode == InstallMode::Repair;
        let manifest = self.legacy_manifest().clone();
        let abort = Abort::default();
        let (dir, full_hash) = {
            let mut st = self.st();
            if st.is_busy() {
                return ActionResult::err(ErrorCode::Busy);
            }
            let Some(dir) = st.legacy.info.as_ref().map(|info| info.root.clone()) else {
                return ActionResult::err(ErrorCode::Unknown);
            };
            st.last_error = None;
            st.notice = None;
            st.abort = Some(abort.clone());
            // Files this launcher already verified in this folder are trusted by size; anything else
            // (Repair, a folder picked by hand) is hashed
            let full_hash = repair || st.s().revived_verified.as_deref() != Some(dir.as_str());
            let total = if full_hash {
                manifest.total_bytes
            } else {
                manifest.files.len() as u64
            };
            st.task = Some(blank(TaskKind::Verify, total, manifest.files.len()));
            (PathBuf::from(dir), full_hash)
        };
        let label = if repair { "repair" } else { "install" };
        let outcome = self
            .revived_install_now(repair, full_hash, &manifest, &sv, &dir, abort)
            .await;
        if let Err(error) = self.legacy.verified.lock().await.save().await {
            log::warn(&format!(
                "verified-files cache not saved: {}",
                log::describe_error(&error)
            ));
        }
        {
            let mut st = self.st();
            st.legacy.job = None;
            st.abort = None;
            st.task = None;
        }
        self.inspect_revived(false).await;
        self.publish();
        match outcome {
            Ok(()) => {
                log::info(&format!("Dauntless 1.4.4 {label} finished in {}", dir.display()));
                ActionResult::Ok
            }
            Err(error) => ActionResult::Err(error),
        }
    }

    async fn revived_install_now(
        &self,
        repair: bool,
        full_hash: bool,
        manifest: &GameManifest,
        sv: &StoredServer,
        dir: &Path,
        abort: Abort,
    ) -> Outcome {
        let label = if repair { "repair" } else { "install" };
        let io_failed = |error: std::io::Error| {
            log::error(&format!(
                "Dauntless 1.4.4 {label} failed: {}",
                log::describe_error(&error)
            ));
            self.fail(io_code(&error), None)
        };
        tokio::fs::create_dir_all(dir).await.map_err(io_failed)?;
        let dir_text = dir.to_string_lossy().into_owned();
        if let Some(task) = self.st().task.clone() {
            self.p.emit_progress(&task);
        }
        self.publish();

        // 1. Which files are needed?
        let trusted = if repair {
            None
        } else {
            let cache = Arc::new(self.legacy.verified.lock().await.clone());
            let dir_text = dir_text.clone();
            Some(Arc::new(move |file: &ManifestFile, size: u64, mtime: f64| {
                cache.is_trusted(&dir_text, file, size, mtime)
            }) as crate::net::verify::Trusted)
        };
        let me = self.me.clone();
        let report = verify_install(
            dir,
            &manifest.files,
            VerifyOptions {
                full_hash,
                trusted,
                abort: abort.clone(),
                on_progress: Some(Arc::new(move |progress| {
                    if let Some(this) = me.upgrade() {
                        this.set_task(TaskProgress {
                            done_bytes: progress.done_bytes,
                            files_done: progress.files_done as u32,
                            current_file: progress.current_file,
                            ..blank(TaskKind::Verify, progress.total_bytes, progress.files_total)
                        });
                    }
                })),
                concurrency: None,
            },
        )
        .await;
        let report = match report {
            Ok(report) => report,
            Err(VerifyError::Aborted) => {
                log::info(&format!("Dauntless 1.4.4 {label} cancelled"));
                return Err(err(ErrorCode::Cancelled, None));
            }
            Err(VerifyError::Io(error)) => return Err(io_failed(error)),
        };
        let needed: Vec<ManifestFile> = report.bad.iter().map(|(file, _)| file.clone()).collect();
        {
            let mut cache = self.legacy.verified.lock().await;
            for file in manifest.files.iter().filter(|file| !needed.contains(file)) {
                cache.record(dir, file).await;
            }
            for file in &needed {
                cache.forget(&dir_text, file);
            }
        }

        // 2. Download what is missing or damaged
        if !needed.is_empty() {
            log::info(&format!(
                "Dauntless 1.4.4: {} file(s) to download ({label})",
                needed.len()
            ));
            self.download(manifest, sv, dir, needed).await?;
        }

        // 3. The pinned DLLs and the client mods
        let dlls = self.release().dlls.len();
        self.set_task(blank(TaskKind::Dlls, dlls as u64, dlls));
        self.publish();
        self.clear_dr_proxy(dir)?;
        self.install_dlls(dir).await?;
        self.install_mods(dir).await;
        remove_part_files(dir, &manifest.files).await;
        self.update_settings(|map| {
            set_text(map, "revivedRoot", Some(&dir_text));
            set_text(map, "revivedVerified", Some(&dir_text));
        })
        .await;
        self.st().notice = Some(if repair {
            NoticeCode::RepairDone
        } else {
            NoticeCode::InstallDone
        });
        Ok(())
    }

    /// Private mode: the content server named in ServerStatus. Public mode: behind the gateway, if it
    /// answers.
    async fn content_endpoint(&self, sv: &StoredServer) -> Option<Endpoint> {
        if sv.public() {
            if self.st().legacy.content_probe != Some(true) {
                let probe = probe_content(&sv.endpoint()).await;
                self.st().legacy.content_probe = Some(probe);
            }
            return (self.st().legacy.content_probe == Some(true)).then(|| sv.endpoint());
        }
        let port = self.st().status.as_ref()?.content_port?;
        Some(Endpoint {
            host: sv.host.clone(),
            port,
            pin: None,
        })
    }

    async fn download(
        &self,
        manifest: &GameManifest,
        sv: &StoredServer,
        dir: &Path,
        needed: Vec<ManifestFile>,
    ) -> Outcome {
        let Some(content) = self.content_endpoint(sv).await else {
            return Err(self.fail(ErrorCode::NoContentServer, None));
        };
        let Some(key) = self.keys.load(&sv.slot()).ok().flatten() else {
            return Err(self.fail(ErrorCode::KeyInvalid, None));
        };
        let Some(server_manifest) = fetch_content_manifest(&content).await else {
            return Err(self.fail(ErrorCode::NoContentServer, None));
        };
        if let Err(difference) = compare_manifests(manifest, &server_manifest) {
            log::error(&format!("content server manifest refused: {difference}"));
            return Err(self.fail(ErrorCode::ManifestMismatch, Some(difference)));
        }

        let free = free_bytes(dir);
        let mut on_disk = 0;
        for file in &needed {
            if let Ok(full) = resolve_inside(dir, &file.path) {
                let mut part = full.into_os_string();
                part.push(".part");
                if let Ok(metadata) = tokio::fs::metadata(&part).await {
                    on_disk += metadata.len().min(file.size);
                }
            }
        }
        let total: u64 = needed.iter().map(|file| file.size).sum();
        let required = total - on_disk + DISK_MARGIN_BYTES;
        if free.is_some_and(|free| free < required) {
            return Err(self.fail(ErrorCode::DiskSpace, Some(required.to_string())));
        }

        let download_base_url = match server_manifest.get("downloadBaseUrl") {
            None => None,
            Some(Value::String(url)) => Some(url.clone()),
            Some(_) => {
                log::error("download failed: Invalid content download URL");
                return Err(self.fail(ErrorCode::Unknown, None));
            }
        };
        let files_total = needed.len();
        let mut options = DownloadOptions::new(content, key, dir.to_path_buf(), needed);
        options.download_base_url = download_base_url;
        let me = self.me.clone();
        options.on_progress = Some(Arc::new(move |progress| {
            if let Some(this) = me.upgrade() {
                this.set_task(TaskProgress {
                    kind: TaskKind::Download,
                    total_bytes: progress.total_bytes,
                    done_bytes: progress.done_bytes,
                    files_total: progress.files_total as u32,
                    files_done: progress.files_done as u32,
                    bytes_per_second: progress.bytes_per_second,
                    eta_seconds: progress.eta_seconds,
                    current_file: progress.current_file,
                });
            }
        }));
        options.on_retry = Some(Arc::new(|file, attempt, reason| {
            log::warn(&format!("retry {attempt} for {}: {reason}", file.path));
        }));
        let cache = self.legacy.verified.clone();
        let verified_dir = dir.to_path_buf();
        let rt = self.rt.clone();
        options.on_file_verified = Some(Arc::new(move |file| {
            let (cache, dir, file) = (cache.clone(), verified_dir.clone(), file.clone());
            rt.spawn(async move {
                let mut cache = cache.lock().await;
                cache.record(&dir, &file).await;
                let _ = cache.save().await;
            });
        }));
        let job = match DownloadJob::new(options) {
            Ok(job) => Arc::new(job),
            Err(error) => {
                log::error(&format!("download failed: {}", error.message));
                return Err(self.fail(ErrorCode::Unknown, None));
            }
        };
        self.st().legacy.job = Some(job.clone());
        self.set_task(blank(TaskKind::Download, total, files_total));
        self.publish();
        let outcome = job.run().await;
        self.st().legacy.job = None;
        match outcome {
            Ok(()) => {
                log::info(&format!(
                    "downloaded and verified {files_total} Dauntless 1.4.4 file(s)"
                ));
                Ok(())
            }
            Err(error) if error.code == DownloadErrorCode::Cancelled => {
                log::info("download cancelled");
                Err(err(ErrorCode::Cancelled, None))
            }
            Err(error) => {
                log::error(&format!(
                    "download stopped: {:?} {} {}",
                    error.code,
                    error.file.as_deref().unwrap_or(""),
                    error.message
                ));
                let code = match error.code {
                    DownloadErrorCode::KeyRejected => ErrorCode::KeyRejected,
                    DownloadErrorCode::FileMissingOnServer => ErrorCode::FileMissingOnServer,
                    DownloadErrorCode::FileDifferentOnServer => ErrorCode::FileDifferentOnServer,
                    DownloadErrorCode::DiskSpace => ErrorCode::DiskSpace,
                    DownloadErrorCode::DiskError => ErrorCode::FolderInvalid,
                    DownloadErrorCode::CertMismatch => ErrorCode::CertMismatch,
                    DownloadErrorCode::DownloadFailed | DownloadErrorCode::Cancelled => ErrorCode::DownloadFailed,
                };
                {
                    let mut st = self.st();
                    if error.code == DownloadErrorCode::KeyRejected {
                        st.key_rejected = true;
                    }
                    if error.code == DownloadErrorCode::CertMismatch {
                        st.reachable = Some(false);
                        st.connect_problem = Some(ConnectProblem::CertMismatch);
                    }
                }
                Err(self.fail(code, error.file))
            }
        }
    }

    // ------------------------------------------------------------------ play

    /// PLAY on a Dauntless Revived server, as DR's launcher does: the account checked, the hunt region
    /// chosen, the executable and DLLs checked by hash, the game config written, the relay started in
    /// public mode (private mode connects straight to the host), and the game started with the server
    /// address first on its command line. Reforged's runtime, loader list and `REFORGED_BACKEND_URL` are
    /// not part of it.
    pub(super) async fn play_revived(&self, sv: &StoredServer) -> Outcome {
        let (vc, dx) = self.p.missing_runtimes();
        let missing: Vec<String> = vc.into_iter().chain(dx).collect();
        if !missing.is_empty() {
            return Err(self.fail(ErrorCode::WindowsRuntimeMissing, Some(missing.join(", "))));
        }
        if !self.p.find_running_clients().await.is_empty() {
            return Err(self.fail(ErrorCode::AlreadyRunning, None));
        }
        let Some(key) = self.keys.load(&sv.slot()).ok().flatten() else {
            return Err(self.fail(ErrorCode::KeyInvalid, None));
        };
        let endpoint = sv.endpoint();
        if let Err((code, detail)) = fetch_user_info_as(REVIVED, &endpoint, &key).await {
            self.st().key_rejected = matches!(code, ErrorCode::KeyRejected | ErrorCode::AccountBanned);
            return Err(self.fail(code, detail));
        }
        self.st().key_rejected = false;
        if let Some(region) = crate::net::autoregion::choose_closest_region(&endpoint, &key, None).await {
            match save_hunt_region(&endpoint, &key, region).await {
                Ok(true) => log::info(&format!("hunt region: {region}")),
                Ok(false) => return Err(self.fail(ErrorCode::ServerError, Some("hunt_region".into()))),
                Err(_) => return Err(self.fail(ErrorCode::ServerUnreachable, None)),
            }
        }

        self.inspect_revived(false).await;
        let info = self.st().legacy.info.clone();
        let Some(info) = info.filter(|info| info.problem.is_none()) else {
            let missing = self.st().legacy.info.as_ref().map_or(0, |info| info.missing_files);
            return Err(self.fail(ErrorCode::GameFilesInvalid, Some(missing.to_string())));
        };
        let root = PathBuf::from(&info.root);
        // The executable by hash on every launch, like DR's play.ps1
        let pin = self
            .legacy_manifest()
            .files
            .iter()
            .find(|file| file.path == EXE_RELATIVE_PATH)
            .map(|file| file.sha256.clone());
        let digest = match resolve_inside(&root, EXE_RELATIVE_PATH) {
            Ok(exe) => {
                let mut hashes = self.hashes.lock().await;
                let digest = hashes.hash(&exe, Abort::default(), Arc::new(|_| {})).await.ok();
                let _ = hashes.save().await;
                digest
            }
            Err(_) => None,
        };
        if pin.is_none() || digest != pin {
            self.update_settings(|map| set_text(map, "revivedVerified", None)).await;
            self.inspect_revived(false).await;
            return Err(self.fail(ErrorCode::GameFilesInvalid, Some(EXE_RELATIVE_PATH.into())));
        }
        self.clear_dr_proxy(&root)?;
        if !info.dlls_ok {
            self.install_dlls(&root).await?;
        }
        self.install_mods(&root).await;

        // Where the game connects. Public mode: the local relay, started before the game
        let (mut game_host, mut game_port, mut xmpp_port) = (sv.host.clone(), sv.port, XMPP_PORT);
        if sv.public() {
            let port = self.c.legacy.relay_port.unwrap_or(RELAY_PORT);
            let port = self.start_relay_on(sv, port, None).await?;
            game_host = "127.0.0.1".into();
            game_port = port;
            xmpp_port = port;
        }
        let launched = self
            .launch_revived(sv, &root, &game_host, game_port, xmpp_port, &key)
            .await;
        if launched.is_err() {
            self.stop_relay().await;
        }
        launched
    }

    async fn launch_revived(
        &self,
        sv: &StoredServer,
        root: &Path,
        game_host: &str,
        game_port: u16,
        xmpp_port: u16,
        key: &str,
    ) -> Outcome {
        let mut prepared = match self.p.prepare_game_launch().await {
            Prepared::Native => None,
            Prepared::Runtime(prepared) => Some(prepared),
            Prepared::Missing => return Err(self.fail(ErrorCode::CompatRuntimeMissing, None)),
        };
        if let Some(prepared) = prepared.as_mut() {
            legacy::wine_overrides(&mut prepared.runtime);
        }
        let s = self.s();
        let config_dir = prepared.as_ref().map_or_else(
            || self.c.game_config_dir.clone(),
            |prepared| prepared.config_dir.clone(),
        );
        // 1.4.4 reads its endpoints from Game.ini, so DR's section stays
        let config = ApplyConfig {
            host: game_host,
            port: xmpp_port,
            graphics: s.graphics,
            config_dir,
            safe_window: s.windowed,
            clear_phoenix_endpoints: false,
        };
        let applied = match apply_game_config(&config) {
            Ok(applied) => applied,
            Err(error) => {
                log::error(&format!(
                    "could not write the game config: {}",
                    log::describe_error(&error)
                ));
                return Err(self.fail(ErrorCode::ConfigFailed, None));
            }
        };
        let args = legacy::build_launch_args(&legacy::LaunchSettings {
            host: game_host,
            port: game_port,
            key,
            windowed: s.windowed || applied.display_repaired,
            game_locale: Some(game_locale(s.game_language, &self.c.system_locale)),
        });
        let args = match args {
            Ok(args) => args,
            Err(reason) => {
                log::error(&format!("launch failed: {reason}"));
                return Err(self.fail(ErrorCode::LaunchFailed, None));
            }
        };
        let base = self.seams.env.clone().unwrap_or_else(|| std::env::vars_os().collect());
        let env = legacy::launch_environment(base);
        let via = prepared
            .as_ref()
            .map(|prepared| format!(" via {}", prepared.runtime_name))
            .unwrap_or_default();
        let suffix = if sv.public() {
            format!(" (relay to {}:{})", sv.host, sv.port)
        } else {
            String::new()
        };
        log::info(&format!(
            "starting Dauntless 1.4.4: {}{via}{suffix}",
            describe_launch(EXE_NAME, &args)
        ));
        let runtime: Option<&LaunchRuntime> = prepared.as_ref().map(|prepared| &prepared.runtime);
        self.st().legacy.playing = Some(sv.clone());
        if let Err(error) = self.game.start(&win64_dir(root), &args, runtime, &env) {
            self.st().legacy.playing = None;
            log::error(&format!("launch failed: {}", log::describe_error(&error)));
            return Err(self.fail(ErrorCode::LaunchFailed, None));
        }
        Ok(())
    }

    /// The game ended: DR's server hears it (`PlayingEnded`), with the key read again from the store.
    pub(super) async fn revived_game_ended(&self) {
        let sv = self.st().legacy.playing.take();
        if let Some(sv) = sv
            && let Some(key) = self.keys.load(&sv.slot()).ok().flatten()
        {
            let _ = end_playing(&sv.endpoint(), &key).await;
        }
    }
}
