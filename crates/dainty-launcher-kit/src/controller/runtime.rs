// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The server's runtime (RD27 spec §2.4.9): what the server offers, whether the player trusted it, and installing
//! it into the game folder.

use std::path::PathBuf;
use std::time::Instant;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::Controller;
use super::platform::FolderPurpose;
use super::stored::edit;
use crate::bridge::types::{
    ActionResult, ErrorCode, LauncherError, NoticeCode, RuntimeInfo, RuntimeState, TaskKind, TaskProgress,
};
use crate::net::hostapi::key_headers;
use crate::net::runtime_files::{
    self, InstallRequest, RuntimeErrorCode, RuntimeManifestResult, RuntimeProgress, SetShape, accept, check_runtime,
    file_infos, is_complete_runtime_dir, is_trusted, required_runtime_files,
};
use crate::platform::log;

fn task_of(progress: &RuntimeProgress, started: Instant) -> TaskProgress {
    let seconds = started.elapsed().as_secs_f64();
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a rate for the page, not an exact count"
    )]
    let rate = if seconds > 0.0 {
        (progress.done_bytes as f64 / seconds) as u64
    } else {
        0
    };
    let left = progress.total_bytes.saturating_sub(progress.done_bytes);
    TaskProgress {
        kind: TaskKind::Runtime,
        total_bytes: progress.total_bytes,
        done_bytes: progress.done_bytes,
        files_total: u32::try_from(progress.files_total).unwrap_or(u32::MAX),
        files_done: u32::try_from(progress.files_done).unwrap_or(u32::MAX),
        bytes_per_second: rate,
        eta_seconds: (rate > 0).then(|| left / rate),
        current_file: progress.current_file.clone(),
    }
}

impl Controller {
    /// The player trusted the manifest's file set on the current server.
    fn offered_is_trusted(&self) -> bool {
        let state = self.lock();
        match (state.stored.server(), state.manifest.as_ref()) {
            (Some(server), Some(manifest)) => is_trusted(&state.stored.trusted_runtimes(&server.id()), &manifest.files),
            _ => false,
        }
    }

    /// Asks the server what runtime it offers.
    pub(crate) async fn refresh_offer(&self) {
        let Some(server) = self.server() else { return };
        match runtime_files::fetch_runtime_manifest(&server.endpoint()).await {
            RuntimeManifestResult::Ok(manifest) => self.lock().manifest = Some(manifest),
            RuntimeManifestResult::None => self.lock().manifest = None,
            RuntimeManifestResult::CertMismatch => self.unreachable_by_pin(),
            RuntimeManifestResult::BadAnswer | RuntimeManifestResult::Unreachable => {}
        }
        self.look_at_runtime().await;
    }

    /// The runtime's state for the snapshot and PLAY.
    pub(crate) async fn look_at_runtime(&self) {
        let trusted = self.offered_is_trusted();
        let (manifest, local) = {
            let state = self.lock();
            (
                state.manifest.clone(),
                state.stored.local_runtime_dir().map(PathBuf::from),
            )
        };
        let win64 = self.win64();
        let (state, playable) = match &win64 {
            None if manifest.is_some() => (RuntimeState::Missing, false),
            None => (RuntimeState::Unavailable, false),
            Some(win64) => {
                check_runtime(
                    win64,
                    manifest.as_ref(),
                    trusted,
                    local.as_deref(),
                    self.runtime_files(),
                )
                .await
            }
        };
        let info = RuntimeInfo {
            state,
            version: manifest.as_ref().map(|m| m.version.clone()),
            files: manifest.as_ref().map(|m| file_infos(&m.files)).unwrap_or_default(),
            local_dir: local.map(|dir| dir.to_string_lossy().into_owned()),
        };
        let mut state = self.lock();
        state.runtime = info;
        state.runtime_playable = playable;
    }

    /// `trust_runtime`: trusts the offered set, then installs it.
    pub async fn trust_runtime(&self) -> ActionResult {
        let (server, manifest) = {
            let state = self.lock();
            (state.stored.server(), state.manifest.clone())
        };
        let (Some(server), Some(manifest)) = (server, manifest) else {
            return ActionResult::err(ErrorCode::RuntimeUnavailable);
        };
        let id = server.id();
        let mut trusted = self.stored().trusted_runtimes(&id);
        let set = accept(&mut trusted, &manifest.files);
        self.edit_settings(|map| edit::keyed_value(map, "trustedRuntimes", &id, Some(json!(trusted))))
            .await;
        log::info(format!(
            "trusted runtime {} ({})",
            manifest.version,
            set.get(..12).unwrap_or(&set)
        ));
        self.install_runtime().await
    }

    /// `install_runtime`.
    pub async fn install_runtime(&self) -> ActionResult {
        let (server, manifest) = {
            let state = self.lock();
            if state.busy {
                return ActionResult::err(ErrorCode::Busy);
            }
            if self.shared.game.is_running() {
                return ActionResult::err(ErrorCode::AlreadyRunning);
            }
            (state.stored.server(), state.manifest.clone())
        };
        let Some(server) = server else {
            return ActionResult::err(ErrorCode::ServerUnreachable);
        };
        let Some(win64) = self.win64() else {
            return ActionResult::err(ErrorCode::GameFolderNotFound);
        };
        let Some(manifest) = manifest else {
            return ActionResult::err(ErrorCode::RuntimeUnavailable);
        };
        if !self.offered_is_trusted() {
            return ActionResult::err(ErrorCode::RuntimeUntrusted);
        }
        let Some(key) = self.load_key().await else {
            return ActionResult::err(ErrorCode::KeyInvalid);
        };
        let cancel = CancellationToken::new();
        let first = TaskProgress {
            kind: TaskKind::Runtime,
            total_bytes: manifest.files.iter().map(|f| f.size).sum(),
            done_bytes: 0,
            files_total: u32::try_from(manifest.files.len()).unwrap_or(u32::MAX),
            files_done: 0,
            bytes_per_second: 0,
            eta_seconds: None,
            current_file: None,
        };
        {
            let mut state = self.lock();
            if state.busy || state.task.is_some() {
                return ActionResult::err(ErrorCode::Busy);
            }
            state.busy = true;
            state.last_error = None;
            state.task = Some(first);
            state.runtime_cancel = Some(cancel.clone());
        }
        self.publish();

        let started = Instant::now();
        let endpoint = server.endpoint();
        let me = self.clone();
        let on_progress = move |progress: RuntimeProgress| {
            let task = task_of(&progress, started);
            me.lock().task = Some(task.clone());
            me.platform().emit_progress(&task);
        };
        let installed = runtime_files::install_runtime(InstallRequest {
            endpoint: &endpoint,
            headers: key_headers(&endpoint, Some(&key)),
            manifest: &manifest,
            win64_dir: &win64,
            cancel: Some(cancel),
            on_progress: Some(&on_progress),
            files: self.runtime_files(),
        })
        .await;

        let result = match installed {
            Ok(done) => {
                log::info(format!("installed runtime {}", manifest.version));
                let restart = !done.replaced_in_use.is_empty() && self.lock().host.server_up;
                self.lock().notice = Some(if restart {
                    NoticeCode::RuntimeInstalledRestart
                } else {
                    NoticeCode::RuntimeInstalled
                });
                Ok(())
            }
            Err(error) => {
                log::warn(format!("runtime install failed: {error}"));
                let code = match error.code {
                    RuntimeErrorCode::Cancelled => ErrorCode::Cancelled,
                    RuntimeErrorCode::KeyRejected => {
                        self.mark_refused();
                        ErrorCode::KeyRejected
                    }
                    RuntimeErrorCode::CertMismatch => {
                        self.unreachable_by_pin();
                        ErrorCode::CertMismatch
                    }
                    RuntimeErrorCode::NotFound => ErrorCode::RuntimeUnavailable,
                    RuntimeErrorCode::Different => ErrorCode::RuntimeDifferentOnServer,
                    RuntimeErrorCode::DownloadFailed => ErrorCode::DownloadFailed,
                    RuntimeErrorCode::DiskError => ErrorCode::FolderInvalid,
                    RuntimeErrorCode::InUse => ErrorCode::RuntimeInUse,
                    RuntimeErrorCode::Failed => ErrorCode::RuntimeFailed,
                };
                let file = (code != ErrorCode::CertMismatch).then_some(error.file).flatten();
                Err(LauncherError { code, detail: file })
            }
        };
        {
            let mut state = self.lock();
            state.task = None;
            state.runtime_cancel = None;
            state.busy = false;
        }
        self.look_at_runtime().await;
        match result {
            Err(error) if error.code == ErrorCode::Cancelled => {
                self.publish();
                ActionResult::Err(error)
            }
            other => self.finish(other),
        }
    }

    /// `choose_local_runtime`: a folder holding a runtime that replaces the server's.
    pub async fn choose_local_runtime(&self) -> ActionResult {
        if self.lock().busy || self.shared.game.is_running() {
            return ActionResult::err(ErrorCode::Busy);
        }
        let start = self.stored().local_runtime_dir().map(PathBuf::from);
        let language = self.language();
        let Some(folder) = self
            .platform()
            .choose_folder(language, FolderPurpose::Runtime, start)
            .await
        else {
            return ActionResult::err(ErrorCode::Cancelled);
        };
        if !is_complete_runtime_dir(&folder, self.runtime_files()).await {
            return self.fail(LauncherError::with_detail(
                ErrorCode::LocalRuntimeInvalid,
                &required_runtime_files(self.runtime_files(), SetShape::MitOnly).join(", "),
            ));
        }
        let dir = folder.to_string_lossy().into_owned();
        self.edit_settings(|map| edit::field(map, "localRuntimeDir", json!(dir)))
            .await;
        self.look_at_runtime().await;
        self.publish();
        ActionResult::Ok
    }

    /// `clear_local_runtime`.
    pub async fn clear_local_runtime(&self) -> ActionResult {
        self.edit_settings(|map| edit::field(map, "localRuntimeDir", Value::Null))
            .await;
        self.look_at_runtime().await;
        self.publish();
        ActionResult::Ok
    }
}
