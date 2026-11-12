// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The runtime loader's sessions (RD9), for PLAY and the Host tools alike: PLAY refuses while a leftover
//! ini or a Host tools DLL sits beside the loader, so this is in every build, `host` feature or not. On Windows
//! the launcher injects instead (`platform/inject.rs`), so there a `winmm.dll` it did not install is a leftover
//! too: it would load the runtime a second time.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::Controller;
use crate::bridge::types::{ErrorCode, LauncherError};
use crate::net::runtime_files::{self, LOADER_DLL};
use crate::platform::inject;
use crate::platform::loader_session::{self, EXPORTER_DLL, LoaderSession, RECORD_FILE, SessionError, SessionKind};
use crate::platform::log;

impl Controller {
    /// The runtime loader loads the runtime: under Proton unless the player picked the inject helper.
    pub(crate) fn uses_loader(&self) -> bool {
        !inject::INJECTS && !self.stored().inject_helper()
    }

    /// What the game folder holds for this way of loading the runtime.
    pub(crate) fn runtime_files(&self) -> &'static [&'static str] {
        runtime_files::player_runtime_files(self.uses_loader())
    }

    /// Where the session record is kept.
    fn loader_record(&self) -> PathBuf {
        self.user_data().join(RECORD_FILE)
    }

    /// The Host tools' DLL names a PLAY refuses beside the loader: the host's Dumper-7 and CatalogExporter.
    fn loader_known(&self) -> Vec<String> {
        let mut known = vec![EXPORTER_DLL.to_owned()];
        if let Some(dll) = self.stored().dumper_dll() {
            known.push(
                Path::new(dll)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            );
        }
        known
    }

    /// The pre-launch check (RD9): what must not be beside the loader for an ordinary PLAY.
    pub(crate) fn loader_leftovers(&self, win64: &Path) -> Vec<String> {
        let known = self.loader_known();
        let known: Vec<&str> = known.iter().map(String::as_str).collect();
        let mut found = loader_session::leftovers(win64, &self.loader_record(), &known);
        if !self.uses_loader() {
            runtime_files::retire_loader(win64, self.runtime_files());
            if win64.join(LOADER_DLL).symlink_metadata().is_ok() {
                found.push(LOADER_DLL.to_owned());
                found.sort_by_key(|name| name.to_lowercase());
            }
        }
        found
    }

    /// Begins a loader session in `win64` for `dll`; the page's error when it cannot.
    pub(crate) fn begin_loader_session(
        &self,
        kind: SessionKind,
        win64: &Path,
        dll: &Path,
    ) -> Result<LoaderSession, LauncherError> {
        let known = self.loader_known();
        let known: Vec<&str> = known.iter().map(String::as_str).collect();
        loader_session::begin(kind, win64, dll, &self.loader_record(), &known).map_err(|error| {
            log::warn(format!("host: loader session not begun: {}", error.detail()));
            self.refuse(match error {
                SessionError::Leftovers(files) => {
                    LauncherError::with_detail(ErrorCode::LoaderLeftovers, &files.join(", "))
                }
                other => LauncherError::with_detail(ErrorCode::HostFailed, &other.detail()),
            })
        })
    }

    /// Ends the session the game ran with, if any.
    pub(crate) async fn end_current_loader_session(&self) {
        let session = self.lock().host.loader_session.take();
        if let Some(session) = session {
            self.end_loader_session(session).await;
        }
    }

    /// Ends a session: the ini and the DLL go, the runtime comes back. A DLL the game just let go of may
    /// still be held for a moment, so it is tried a few times.
    pub(crate) async fn end_loader_session(&self, session: LoaderSession) {
        let mut last = None;
        for attempt in 0..5 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(400)).await;
            }
            let try_end = session.clone();
            match tokio::task::spawn_blocking(move || try_end.end()).await {
                Ok(Ok(())) => {
                    log::info(format!(
                        "host: {} and {} removed",
                        loader_session::LOADER_INI,
                        session.dll
                    ));
                    return;
                }
                Ok(Err(error)) => last = Some(log::describe_error(&error)),
                Err(error) => last = Some(error.to_string()),
            }
        }
        log::error(format!(
            "host: the loader session's files were not removed ({}); PLAY refuses until they are",
            last.unwrap_or_default()
        ));
    }

    /// A session a closed launcher did not finish is undone at the next start.
    pub(crate) async fn recover_loader_session(&self) {
        let record = self.loader_record();
        match tokio::task::spawn_blocking(move || loader_session::recover(&record)).await {
            Ok(Some(Ok(()))) => log::info("host: an unfinished loader session was undone"),
            Ok(Some(Err(error))) => log::warn(format!(
                "host: an unfinished loader session could not be undone: {}",
                log::describe_error(&error)
            )),
            _ => {}
        }
    }
}
