// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Crash reports after the game exits (`docs/security/crash-reports.md`): a crash folder the game just wrote is
//! offered once, and its report goes only to the server it was played on, never to another one joined later.

use std::path::PathBuf;
use std::time::SystemTime;

use dainty_protocol::invite::played_versions;

use super::Controller;
use crate::bridge::types::{ActionResult, CrashOffer, CrashOfferState, ErrorCode};
use crate::net::hostapi;
use crate::platform::crash::{self, CrashAction, Header, Ledger};
use crate::platform::launch::exit_code_hex;
use crate::platform::log;

/// How much of this session's launcher log a report carries before it is cut to fit.
const LAUNCHER_LOG_TAIL: usize = 256 * 1024;

/// The crash the page was told about.
pub(crate) struct Pending {
    pub(crate) offer: CrashOffer,
    folder: PathBuf,
    header: Header,
    /// The key-store id of the server played on
    server: String,
}

impl Controller {
    /// Remembers when the game started, so only its own crash folders count.
    pub(crate) fn note_game_started(&self) {
        self.lock().game_started = Some(SystemTime::now());
    }

    /// After the game exits: the newest crash folder it wrote, acted on per the player's choice the first time
    /// it is seen.
    pub(crate) async fn look_for_crash(&self, code: Option<i32>) {
        let (since, root) = {
            let mut state = self.lock();
            (state.game_started.take(), state.game.root.clone().map(PathBuf::from))
        };
        let (Some(since), Some(server)) = (since, self.server()) else {
            return;
        };
        let dirs = crash::crash_dirs(root.as_deref(), &self.config().game_config_dir);
        let user_data = self.user_data().to_path_buf();
        let choice = self.stored().crash_reports();
        let found = tokio::task::spawn_blocking(move || {
            let folder = crash::new_crashes(&dirs, since).into_iter().next()?;
            let first = Ledger::load(&user_data).first_sight(&folder.id);
            Some((folder, crash::decide(choice, first)))
        })
        .await
        .ok()
        .flatten();
        let Some((folder, action)) = found else { return };
        log::info(format!("the game left crash folder {} ({action:?})", folder.id));
        if action == CrashAction::Ignore {
            return;
        }
        let header = Header {
            crash_id: folder.id.clone(),
            game_version: played_versions().first().map(ToString::to_string),
            launcher_version: Some(self.config().version.clone()),
            exit_code: code.map(exit_code_hex),
        };
        self.lock().crash = Some(Pending {
            offer: CrashOffer {
                id: folder.id,
                state: CrashOfferState::Ask,
            },
            folder: folder.path,
            header,
            server: server.id(),
        });
        self.publish();
        if action == CrashAction::Send {
            let _ = self.crash_report_send().await;
        }
    }

    /// `crash_report_send`: sends the offered crash's report to the server it was played on.
    pub async fn crash_report_send(&self) -> ActionResult {
        let (folder, header, server) = {
            let mut state = self.lock();
            let Some(pending) = state.crash.as_mut() else {
                return ActionResult::err(ErrorCode::Unknown);
            };
            if matches!(pending.offer.state, CrashOfferState::Sending | CrashOfferState::Sent) {
                return ActionResult::err(ErrorCode::Busy);
            }
            pending.offer.state = CrashOfferState::Sending;
            (pending.folder.clone(), pending.header.clone(), pending.server.clone())
        };
        self.publish();
        let result = self.upload_crash(folder, header, &server).await;
        let state = match &result {
            Ok(()) => CrashOfferState::Sent,
            Err(ErrorCode::RateLimited) => CrashOfferState::RateLimited,
            Err(_) => CrashOfferState::Failed,
        };
        if let Some(pending) = self.lock().crash.as_mut() {
            pending.offer.state = state;
        }
        self.publish();
        match result {
            Ok(()) => ActionResult::Ok,
            Err(code) => ActionResult::err(code),
        }
    }

    async fn upload_crash(&self, folder: PathBuf, header: Header, server_id: &str) -> Result<(), ErrorCode> {
        let server = self.server().filter(|s| s.id() == server_id).ok_or_else(|| {
            log::warn("crash report not sent: the server it was played on is no longer joined");
            ErrorCode::ServerUnreachable
        })?;
        let key = self.load_key().await.ok_or(ErrorCode::KeyInvalid)?;
        let secret = key.clone();
        let packed = tokio::task::spawn_blocking(move || {
            let launcher_log = log::session_tail(LAUNCHER_LOG_TAIL);
            crash::bundle(&folder, &header, launcher_log.as_deref(), &[secret.as_str()])
        })
        .await
        .map_err(|_| ErrorCode::Unknown)?
        .map_err(|failed| {
            log::warn(format!("could not pack the crash report: {failed}"));
            ErrorCode::Unknown
        })?;
        let size = packed.len();
        match hostapi::upload_crash_report(&server.endpoint(), &key, packed).await {
            Ok(()) => {
                log::info(format!("crash report sent ({size} bytes)"));
                Ok(())
            }
            Err(failed) => {
                log::warn(format!("crash report not sent: {:?}", failed.code));
                Err(failed.code)
            }
        }
    }

    /// `crash_report_dismiss`: forgets the offered crash; it is not offered again.
    pub fn crash_report_dismiss(&self) {
        self.lock().crash = None;
        self.publish();
    }
}
