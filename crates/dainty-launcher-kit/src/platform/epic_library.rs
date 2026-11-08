// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Whether the signed-in Epic account owns Dauntless (DE3, DD22), for `epic_library_check` and for a download's
//! start (DE5, through [`require`]). `dainty_epic::library` does the check and keeps the answer a while, in memory and in
//! `epic-ownership.json` in the user data folder; this turns its answers into the bridge's codes.

use std::path::PathBuf;

use dainty_epic::auth::Session;
use dainty_epic::library::{EgsLibrary, Library, LibraryError, Ownership, OwnershipError};

use crate::bridge::types::{EpicLibraryCheck, ErrorCode, LauncherError};

/// The launcher's one ownership check, managed by Tauri.
pub type EpicOwnership = Ownership<EgsLibrary>;

/// The ownership check, caching its answers in `user_data`.
pub fn ownership(user_data: PathBuf) -> EpicOwnership {
    Ownership::new(EgsLibrary, Some(user_data))
}

/// `epic_library_check`: the account's answer, cached or fresh. Not owning the game is an answer, not an error.
pub async fn check<L: Library>(ownership: &Ownership<L>, session: Option<&Session>) -> EpicLibraryCheck {
    let Some(session) = session else {
        return EpicLibraryCheck::Err(LauncherError::new(ErrorCode::EpicSignedOut));
    };
    match ownership.check(session).await {
        Ok(checked) => EpicLibraryCheck::Ok {
            owned: checked.owned,
            checked_at: iso(checked.checked_at),
        },
        Err(error) => EpicLibraryCheck::Err(library_error(error)),
    }
}

/// What a download checks before it starts (DE5): an account without Dauntless gets `epic_not_owned`.
#[cfg_attr(not(test), allow(dead_code, reason = "DE5's game_install_start calls it"))]
pub async fn require<L: Library>(ownership: &Ownership<L>, session: Option<&Session>) -> Result<(), LauncherError> {
    let session = session.ok_or_else(|| LauncherError::new(ErrorCode::EpicSignedOut))?;
    dainty_epic::library::require_ownership(ownership, session)
        .await
        .map_err(|error| match error {
            OwnershipError::NotOwned => LauncherError::new(ErrorCode::EpicNotOwned),
            OwnershipError::Library(error) => library_error(error),
            _ => LauncherError::new(ErrorCode::EpicLibraryFailed),
        })
}

fn library_error(error: LibraryError) -> LauncherError {
    LauncherError::new(match error {
        // DE2 refreshes a session before it expires; one that still has is signed in again
        LibraryError::SessionExpired => ErrorCode::EpicSignedOut,
        _ => ErrorCode::EpicLibraryFailed,
    })
}

/// As JavaScript's `toISOString`, like `lastCheckedAt`.
fn iso(unix_seconds: i64) -> String {
    chrono::DateTime::from_timestamp(unix_seconds, 0)
        .unwrap_or_default()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "a panic is how a test fails")]

    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicI64, Ordering};

    use dainty_epic::auth::Token;
    use dainty_epic::library::{APP_NAME, OWNED_FOR, Record};
    use serde_json::json;

    use super::*;

    const NOW: i64 = 1_800_000_000;

    struct Double(Mutex<Result<Vec<Record>, LibraryError>>);

    impl Library for Double {
        async fn records(&self, _: &Session) -> Result<Vec<Record>, LibraryError> {
            self.0.lock().unwrap().clone()
        }
    }

    fn library(app_names: &[&str]) -> Result<Vec<Record>, LibraryError> {
        Ok(app_names
            .iter()
            .map(|name| Record {
                app_name: (*name).to_owned(),
                ..Record::default()
            })
            .collect())
    }

    fn session() -> Session {
        Session::new(
            "8645b4947bbc4c0092a8b7236df169d1".to_owned(),
            "Slayer".to_owned(),
            Token::new("eg1~access-secret".to_owned()),
            NOW + 30 * 86_400,
            Token::new("eg1~refresh-secret".to_owned()),
            NOW + 60 * 86_400,
        )
    }

    fn ownership(answer: Result<Vec<Record>, LibraryError>) -> (Ownership<Double>, Arc<AtomicI64>) {
        let clock = Arc::new(AtomicI64::new(NOW));
        let read = Arc::clone(&clock);
        let ownership =
            Ownership::new(Double(Mutex::new(answer)), None).with_clock(move || read.load(Ordering::SeqCst));
        (ownership, clock)
    }

    fn run<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(future)
    }

    fn page(answer: &EpicLibraryCheck) -> serde_json::Value {
        serde_json::to_value(answer).unwrap()
    }

    #[test]
    fn the_check_answers_as_the_page_reads_it() {
        let (owner, _) = ownership(library(&["Fortnite", APP_NAME]));
        assert_eq!(
            page(&run(check(&owner, Some(&session())))),
            json!({ "ok": true, "owned": true, "checkedAt": "2027-01-15T08:00:00.000Z" })
        );
        let (not_owner, _) = ownership(library(&["Fortnite"]));
        assert_eq!(
            page(&run(check(&not_owner, Some(&session())))),
            json!({ "ok": true, "owned": false, "checkedAt": "2027-01-15T08:00:00.000Z" })
        );
        assert_eq!(
            page(&run(check(&not_owner, None))),
            json!({ "ok": false, "error": { "code": "epic_signed_out" } })
        );
        let (offline, _) = ownership(Err(LibraryError::Unreachable));
        assert_eq!(
            page(&run(check(&offline, Some(&session())))),
            json!({ "ok": false, "error": { "code": "epic_library_failed" } })
        );
    }

    #[test]
    fn an_account_without_dauntless_cant_start_a_download() {
        let (owner, _) = ownership(library(&[APP_NAME]));
        assert_eq!(run(require(&owner, Some(&session()))), Ok(()));
        let (not_owner, _) = ownership(library(&["Fortnite"]));
        assert_eq!(
            run(require(&not_owner, Some(&session()))),
            Err(LauncherError::new(ErrorCode::EpicNotOwned))
        );
        assert_eq!(
            run(require(&owner, None)),
            Err(LauncherError::new(ErrorCode::EpicSignedOut))
        );
        let (offline, _) = ownership(Err(LibraryError::Unreachable));
        assert_eq!(
            run(require(&offline, Some(&session()))),
            Err(LauncherError::new(ErrorCode::EpicLibraryFailed))
        );
    }

    #[test]
    fn an_expired_session_signs_in_again() {
        let (owner, clock) = ownership(library(&[APP_NAME]));
        clock.store(NOW + OWNED_FOR + 30 * 86_400, Ordering::SeqCst);
        assert_eq!(
            run(require(&owner, Some(&session()))),
            Err(LauncherError::new(ErrorCode::EpicSignedOut))
        );
    }
}
