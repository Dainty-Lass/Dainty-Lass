// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The game folder (RD27 spec §2.4.8): finding it, choosing it and verifying its executable against the pin.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde_json::json;

use super::Controller;
use super::platform::FolderPurpose;
use super::stored::edit;
use crate::bridge::types::{
    ActionResult, DetectedGame, ErrorCode, GameInfo, GameProblem, LauncherError, NoticeCode, TaskKind, TaskProgress,
};
use crate::core::pins::EXE_NAME;
use crate::net::verify::{Abort, VerifyError};
use crate::platform::game_folder::{self, LocateError};
use crate::platform::log;

/// Verify progress is sent at most this often.
pub const VERIFY_PROGRESS_INTERVAL: Duration = Duration::from_millis(150);

/// What is wrong with a folder whose executable is there: a Revived copy first, then the version text, then the
/// executable's hash once known.
fn problem_of(revived: bool, version_ok: bool, hash: Option<&str>, pin: &str) -> Option<GameProblem> {
    match hash {
        _ if revived => Some(GameProblem::Revived),
        _ if !version_ok => Some(GameProblem::WrongVersion),
        None => Some(GameProblem::Unverified),
        Some(hash) if !hash.eq_ignore_ascii_case(pin) => Some(GameProblem::WrongBuild),
        Some(_) => None,
    }
}

/// The error for a folder refused before it is kept, each with its own sentence.
fn locate_refusal(error: LocateError, missing_archon: bool) -> ErrorCode {
    match error {
        LocateError::NotFound if missing_archon => ErrorCode::GameMissingArchon,
        LocateError::NotFound => ErrorCode::GameFolderNotFound,
        LocateError::TooLong => ErrorCode::GamePathTooLong,
        LocateError::Invalid => ErrorCode::FolderInvalid,
    }
}

/// The error a folder kept with a problem answers, when the problem refuses it.
fn kept_refusal(problem: Option<GameProblem>) -> Option<ErrorCode> {
    match problem? {
        GameProblem::WrongVersion => Some(ErrorCode::GameWrongVersion),
        GameProblem::WrongBuild => Some(ErrorCode::GameWrongBuild),
        GameProblem::Revived => Some(ErrorCode::GameRevived),
        GameProblem::NotSet | GameProblem::NotFound | GameProblem::Unverified | GameProblem::MissingArchon => None,
    }
}

impl Controller {
    /// The game folder's Win64 folder, when the folder is known and holds one.
    pub(crate) fn win64(&self) -> Option<PathBuf> {
        let root = self.lock().game.root.clone()?;
        let win64 = game_folder::win64_dir(std::path::Path::new(&root));
        win64.is_dir().then_some(win64)
    }

    async fn detect_installs(&self) -> Vec<DetectedGame> {
        let folders = self.shared.seams.store_folders.clone();
        tokio::task::spawn_blocking(move || match folders {
            Some(folders) => game_folder::detect_in(&folders),
            None => game_folder::detect(),
        })
        .await
        .unwrap_or_default()
    }

    /// Looks at the game folder; with `detect`, also at the stores' installs, taking one when none is chosen.
    pub(crate) async fn look_at_game(&self, detect: bool) {
        let detected = if detect {
            self.detect_installs().await
        } else {
            self.lock().game.detected.clone()
        };
        if detect && self.stored().game_root().is_none() && !detected.is_empty() {
            let supported = detected
                .iter()
                .find(|found| found.version.as_deref().is_some_and(game_folder::is_supported_version));
            let pick = supported.or_else(|| (detected.len() == 1).then(|| &detected[0]));
            if let Some(pick) = pick {
                let root = pick.root.clone();
                log::info(format!("using the game found by {:?}", pick.source));
                self.edit_settings(|map| edit::field(map, "gameRoot", json!(root)))
                    .await;
            }
        }
        let root = self.stored().game_root().map(str::to_owned);
        let mut info = GameInfo {
            root: root.clone(),
            problem: None,
            version_text: None,
            version_ok: false,
            exe_verified: false,
            detected,
        };
        match root.map(PathBuf::from) {
            None => info.problem = Some(GameProblem::NotSet),
            Some(root) => {
                let exe = game_folder::exe_path(&root);
                if exe.is_file() {
                    let version = game_folder::read_version_text(&root);
                    let revived = game_folder::is_revived(&root, version.as_deref());
                    info.version_ok = version.as_deref().is_some_and(game_folder::is_supported_version);
                    info.version_text = version;
                    let cached = self.shared.hashes.lock().await.cached(&exe);
                    info.problem = problem_of(revived, info.version_ok, cached.as_deref(), &self.config().exe_pin);
                    info.exe_verified = cached.is_some() && info.problem.is_none();
                } else if game_folder::missing_archon(&root.to_string_lossy()) {
                    info.problem = Some(GameProblem::MissingArchon);
                } else {
                    info.problem = Some(GameProblem::NotFound);
                }
            }
        }
        self.lock().game = info;
    }

    /// `verify_game`: hashes the executable with progress.
    pub async fn verify_game(&self) -> ActionResult {
        if self.lock().busy || self.shared.game.is_running() {
            return ActionResult::err(ErrorCode::Busy);
        }
        let Some(root) = self.lock().game.root.clone().map(PathBuf::from) else {
            return ActionResult::err(ErrorCode::GameFolderNotFound);
        };
        let exe = game_folder::exe_path(&root);
        let Ok(meta) = std::fs::metadata(&exe)
            .map_err(|_| ())
            .and_then(|m| if m.is_file() { Ok(m) } else { Err(()) })
        else {
            self.look_at_game(false).await;
            return self.fail(LauncherError::new(ErrorCode::GameFolderNotFound));
        };
        let abort = Abort::new();
        let first = TaskProgress {
            kind: TaskKind::Verify,
            total_bytes: meta.len(),
            done_bytes: 0,
            files_total: 1,
            files_done: 0,
            bytes_per_second: 0,
            eta_seconds: None,
            current_file: Some(EXE_NAME.to_owned()),
        };
        {
            let mut state = self.lock();
            if state.busy || state.task.is_some() {
                return ActionResult::err(ErrorCode::Busy);
            }
            state.busy = true;
            state.last_error = None;
            state.task = Some(first.clone());
            state.verify_abort = Some(abort.clone());
        }
        self.publish();
        self.platform().emit_progress(&first);

        let started = Instant::now();
        let done = Arc::new(AtomicU64::new(0));
        let last_sent = Arc::new(Mutex::new(Instant::now()));
        let me = self.weak();
        let on_bytes: Arc<dyn Fn(u64) + Send + Sync> = Arc::new(move |bytes| {
            let total = done.fetch_add(bytes, Ordering::SeqCst) + bytes;
            let mut last = last_sent.lock().unwrap_or_else(PoisonError::into_inner);
            let final_chunk = total >= first.total_bytes;
            if last.elapsed() < VERIFY_PROGRESS_INTERVAL && !final_chunk {
                return;
            }
            *last = Instant::now();
            let Some(me) = Controller::from_weak(&me) else { return };
            let progress = TaskProgress {
                done_bytes: total,
                files_done: u32::from(final_chunk),
                ..first.clone()
            };
            me.lock().task = Some(progress.clone());
            me.platform().emit_progress(&progress);
        });
        let hashed = {
            let mut hashes = self.shared.hashes.lock().await;
            let hashed = hashes.hash(&exe, abort, on_bytes).await;
            if let Err(error) = hashes.save().await {
                log::warn(format!("could not save the hash cache: {error}"));
            }
            hashed
        };
        {
            let mut state = self.lock();
            state.task = None;
            state.verify_abort = None;
            state.busy = false;
        }
        let result = match hashed {
            Ok(hash) => {
                let pinned = hash.eq_ignore_ascii_case(&self.config().exe_pin);
                log::info(format!(
                    "verified the game in {} ms (1.14.7: {pinned})",
                    started.elapsed().as_millis()
                ));
                self.look_at_game(false).await;
                self.look_at_runtime().await;
                if pinned {
                    self.lock().notice = Some(NoticeCode::GameVerified);
                    Ok(())
                } else {
                    let code = kept_refusal(self.lock().game.problem).unwrap_or(ErrorCode::GameWrongVersion);
                    Err(LauncherError::new(code))
                }
            }
            Err(VerifyError::Aborted) => Err(LauncherError::new(ErrorCode::Cancelled)),
            Err(VerifyError::Io(error)) => {
                log::warn(format!("could not read the game: {error}"));
                Err(LauncherError::new(ErrorCode::GameFolderNotFound))
            }
        };
        self.finish(result)
    }

    /// `choose_game_folder`.
    pub async fn choose_game_folder(&self) -> ActionResult {
        if self.lock().busy || self.shared.game.is_running() {
            return ActionResult::err(ErrorCode::Busy);
        }
        let start = self.lock().game.root.clone().map(PathBuf::from);
        let language = self.language();
        match self
            .platform()
            .choose_folder(language, FolderPurpose::Game, start)
            .await
        {
            Some(folder) => self.use_game_path(&folder.to_string_lossy()).await,
            None => ActionResult::err(ErrorCode::Cancelled),
        }
    }

    /// `use_game_path`: a pasted or picked folder.
    pub async fn use_game_path(&self, path: &str) -> ActionResult {
        let refused = || self.lock().busy || self.shared.game.is_running();
        if refused() {
            return ActionResult::err(ErrorCode::Busy);
        }
        let raw = path.to_owned();
        let located = tokio::task::spawn_blocking(move || {
            game_folder::locate(&raw).map_err(|error| (error, game_folder::missing_archon(&raw)))
        })
        .await
        .unwrap_or(Err((LocateError::Invalid, false)));
        if refused() {
            return ActionResult::err(ErrorCode::Busy);
        }
        match located {
            Ok(root) => {
                let root = root.to_string_lossy().into_owned();
                self.edit_settings(|map| edit::field(map, "gameRoot", json!(root)))
                    .await;
                self.look_at_game(false).await;
                self.look_at_runtime().await;
                self.publish();
                if self.lock().game.problem == Some(GameProblem::Unverified) {
                    return self.verify_game().await;
                }
                // A kept folder's problem is the Locate step's banner, not an error as well
                ActionResult::Ok
            }
            Err((error, missing_archon)) => self.fail(LauncherError::new(locate_refusal(error, missing_archon))),
        }
    }

    /// `use_detected_game`.
    pub async fn use_detected_game(&self, index: usize) -> ActionResult {
        let root = self.lock().game.detected.get(index).map(|found| found.root.clone());
        match root {
            Some(root) => self.use_game_path(&root).await,
            None => ActionResult::err(ErrorCode::FolderInvalid),
        }
    }

    /// `open_game_folder`.
    pub fn open_game_folder(&self) {
        let Some(root) = self.lock().game.root.clone().map(PathBuf::from) else {
            return;
        };
        if root.is_dir()
            && let Err(error) = self.platform().open_folder(&root)
        {
            log::warn(format!("could not open the game folder: {error}"));
        }
    }

    /// `cancel_task`: stops a verify or a runtime download.
    pub fn cancel_task(&self) {
        let state = self.lock();
        if let Some(abort) = &state.verify_abort {
            abort.abort();
        }
        if let Some(cancel) = &state.runtime_cancel {
            cancel.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PIN: &str = "ab12";

    #[test]
    fn each_kept_folder_problem_has_its_own_refusal() {
        assert_eq!(problem_of(true, true, Some(PIN), PIN), Some(GameProblem::Revived));
        assert_eq!(
            problem_of(false, false, Some(PIN), PIN),
            Some(GameProblem::WrongVersion)
        );
        assert_eq!(problem_of(false, false, None, PIN), Some(GameProblem::WrongVersion));
        assert_eq!(problem_of(false, true, None, PIN), Some(GameProblem::Unverified));
        assert_eq!(
            problem_of(false, true, Some("cd34"), PIN),
            Some(GameProblem::WrongBuild)
        );
        assert_eq!(problem_of(false, true, Some("AB12"), PIN), None);

        assert_eq!(
            kept_refusal(Some(GameProblem::WrongVersion)),
            Some(ErrorCode::GameWrongVersion)
        );
        assert_eq!(
            kept_refusal(Some(GameProblem::WrongBuild)),
            Some(ErrorCode::GameWrongBuild)
        );
        assert_eq!(kept_refusal(Some(GameProblem::Revived)), Some(ErrorCode::GameRevived));
        assert_eq!(kept_refusal(Some(GameProblem::Unverified)), None);
        assert_eq!(kept_refusal(None), None);
    }

    #[test]
    fn each_refused_pick_has_its_own_refusal() {
        assert_eq!(
            locate_refusal(LocateError::NotFound, true),
            ErrorCode::GameMissingArchon
        );
        assert_eq!(
            locate_refusal(LocateError::NotFound, false),
            ErrorCode::GameFolderNotFound
        );
        assert_eq!(locate_refusal(LocateError::TooLong, false), ErrorCode::GamePathTooLong);
        assert_eq!(locate_refusal(LocateError::Invalid, false), ErrorCode::FolderInvalid);
    }
}
