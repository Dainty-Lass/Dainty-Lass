// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The runtime loader's sessions (RD9), on launcher-kit's `loader_session`. The project's own loader module
//! replaces this one; until then no session begins here, but PLAY still refuses leftovers.

use std::path::{Path, PathBuf};

use crate::platform::loader_session::{self, EXPORTER_DLL, RECORD_FILE};
use crate::platform::log;

use super::Controller;

impl Controller {
    fn loader_record(&self) -> PathBuf {
        self.user_data().join(RECORD_FILE)
    }

    /// Undoes a session a closed launcher left.
    pub(crate) fn recover_loader_session(&self) {
        match loader_session::recover(&self.loader_record()) {
            Some(Ok(())) => log::info("undid the loader session a closed launcher left"),
            Some(Err(error)) => log::warn(format!("could not undo the last loader session: {error}")),
            None => {}
        }
    }

    /// What must not sit beside the loader when PLAY starts the game.
    pub(crate) fn loader_leftovers(&self, win64: &Path) -> Vec<String> {
        let dumper = self.stored().dumper_dll().and_then(|dll| {
            Path::new(dll)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        });
        let mut known: Vec<&str> = vec![EXPORTER_DLL];
        if let Some(dumper) = dumper.as_deref() {
            known.push(dumper);
        }
        loader_session::leftovers(win64, &self.loader_record(), &known)
    }

    /// Ends the session the game ran with, if any.
    pub(crate) fn end_loader_session(&self) {
        let session = self.lock().host.loader_session.take();
        if let Some(session) = session
            && let Err(error) = session.end()
        {
            log::warn(format!("could not end the loader session: {error}"));
        }
    }
}
