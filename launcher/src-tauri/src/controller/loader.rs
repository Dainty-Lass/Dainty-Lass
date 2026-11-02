//! The runtime loader's sessions (RD9), for PLAY and the Host tools alike: PLAY refuses while a leftover
//! ini or a Host tools DLL sits beside the loader, so this is in every build, `host` feature or not.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::Controller;
use crate::bridge::types::ErrorCode;
use crate::platform::loader_session::{self, EXPORTER_DLL, LoaderSession, RECORD_FILE, SessionError, SessionKind};
use crate::platform::log;

impl Controller {
    /// Where the session record is kept.
    fn loader_record(&self) -> PathBuf {
        self.c.user_data.join(RECORD_FILE)
    }

    /// The Host tools' DLL names a PLAY refuses beside the loader: the host's Dumper-7 and CatalogExporter.
    fn loader_known(&self) -> Vec<String> {
        let mut known = vec![EXPORTER_DLL.to_owned()];
        if let Some(dll) = self.s().dumper_dll {
            known.push(
                Path::new(&dll)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            );
        }
        known
    }

    /// The pre-launch check (RD9): what must not be beside the loader for an ordinary PLAY.
    pub(super) fn loader_leftovers(&self, win64: &Path) -> Vec<String> {
        let known = self.loader_known();
        let known: Vec<&str> = known.iter().map(String::as_str).collect();
        loader_session::leftovers(win64, &self.loader_record(), &known)
    }

    /// Begins a loader session in `win64` for `dll`; the page's error when it cannot.
    pub(super) fn begin_loader_session(
        &self,
        kind: SessionKind,
        win64: &Path,
        dll: &Path,
    ) -> Result<LoaderSession, crate::bridge::types::LauncherError> {
        let known = self.loader_known();
        let known: Vec<&str> = known.iter().map(String::as_str).collect();
        loader_session::begin(kind, win64, dll, &self.loader_record(), &known).map_err(|error| {
            log::warn(&format!("host: loader session not begun: {}", error.detail()));
            match error {
                SessionError::Leftovers(files) => self.fail(ErrorCode::LoaderLeftovers, Some(files.join(", "))),
                other => self.fail(ErrorCode::HostFailed, Some(other.detail())),
            }
        })
    }

    /// Ends a session: the ini and the DLL go, the runtime comes back. A DLL the game just let go of may
    /// still be held for a moment, so it is tried a few times.
    pub(super) async fn end_loader_session(&self, session: LoaderSession) {
        let mut last = None;
        for attempt in 0..5 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(400)).await;
            }
            let try_end = session.clone();
            match tokio::task::spawn_blocking(move || try_end.end()).await {
                Ok(Ok(())) => {
                    log::info(&format!(
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
        log::error(&format!(
            "host: the loader session's files were not removed ({}); PLAY refuses until they are",
            last.unwrap_or_default()
        ));
    }

    /// A session a closed launcher did not finish is undone at the next start.
    pub(super) async fn recover_loader_session(&self) {
        let record = self.loader_record();
        match tokio::task::spawn_blocking(move || loader_session::recover(&record)).await {
            Ok(Some(Ok(()))) => log::info("host: an unfinished loader session was undone"),
            Ok(Some(Err(error))) => log::warn(&format!(
                "host: an unfinished loader session could not be undone: {}",
                log::describe_error(&error)
            )),
            _ => {}
        }
    }
}
