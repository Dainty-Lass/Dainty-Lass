// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The guarded admin actions for the server GUI (DD23, DH6): the destructive and bulk actions that wait for a
//! delay or a second operator, listed and decided by the host in this process, as `dainty-host admin
//! guarded` does. The host is an owner no admin key can be, so it approves or cancels what any key asked;
//! the running server carries an approved action out within 30 seconds. Only the server GUI links the host
//! kit, so only it has these.

use super::Controller;
use crate::bridge::types::{GuardedList, GuardedOutcome};

#[cfg(feature = "server-gui")]
mod on {
    use std::path::PathBuf;

    use serde_json::Value;

    use super::Controller;
    use crate::bridge::types::{ErrorCode, GuardedActionInfo, GuardedList, GuardedOutcome, LauncherError};
    use crate::platform::log;

    /// A failure of the store as the page hears it: the codes it has words for, else a host failure.
    fn refuse(failure: dainty_hostkit::guarded::Failure) -> LauncherError {
        match failure.code {
            "not_found" => LauncherError::new(ErrorCode::GuardedNotFound),
            "not_waiting" => LauncherError::new(ErrorCode::GuardedNotWaiting),
            "bad_id" => LauncherError::new(ErrorCode::Unknown),
            "not_configured" => LauncherError::with_detail(ErrorCode::HostFailed, "not_configured"),
            _ => {
                log::warn(format!("guarded actions: {}", failure.message));
                LauncherError::with_detail(ErrorCode::HostFailed, "guarded")
            }
        }
    }

    fn info(value: &Value) -> Result<GuardedActionInfo, LauncherError> {
        serde_json::from_value(value.clone()).map_err(|error| {
            log::warn(format!("guarded actions: an action could not be read: {error}"));
            LauncherError::with_detail(ErrorCode::HostFailed, "guarded")
        })
    }

    impl Controller {
        fn workspace(&self) -> Option<PathBuf> {
            self.stored().host_workspace().map(PathBuf::from)
        }

        pub(super) async fn guarded_list_on(&self) -> GuardedList {
            let Some(workspace) = self.workspace() else {
                return GuardedList::Err(LauncherError::with_detail(ErrorCode::HostFailed, "workspace"));
            };
            match dainty_hostkit::guarded::list(&workspace).await {
                Ok(actions) => match actions.iter().map(info).collect() {
                    Ok(actions) => GuardedList::Ok(actions),
                    Err(error) => GuardedList::Err(error),
                },
                Err(failure) => GuardedList::Err(refuse(failure)),
            }
        }

        pub(super) async fn guarded_decide(&self, id: &str, approve: bool) -> GuardedOutcome {
            let Some(workspace) = self.workspace() else {
                return GuardedOutcome::Err(LauncherError::with_detail(ErrorCode::HostFailed, "workspace"));
            };
            let decided = if approve {
                dainty_hostkit::guarded::approve(&workspace, id).await
            } else {
                dainty_hostkit::guarded::cancel(&workspace, id).await
            };
            match decided {
                Ok(action) => match info(&action) {
                    Ok(action) => {
                        log::info(format!(
                            "guarded actions: {} {} from the server GUI",
                            action.id,
                            if approve { "approved" } else { "canceled" }
                        ));
                        GuardedOutcome::Ok(action)
                    }
                    Err(error) => GuardedOutcome::Err(error),
                },
                Err(failure) => GuardedOutcome::Err(refuse(failure)),
            }
        }
    }
}

impl Controller {
    /// `guarded_list`: what waits, oldest first.
    pub async fn guarded_list(&self) -> GuardedList {
        #[cfg(feature = "server-gui")]
        {
            self.guarded_list_on().await
        }
        #[cfg(not(feature = "server-gui"))]
        {
            GuardedList::Err(crate::bridge::checks::not_in_this_build("guarded_list"))
        }
    }

    /// `guarded_approve`: the server runs it within seconds. `id` is already checked.
    pub async fn guarded_approve(&self, id: &str) -> GuardedOutcome {
        #[cfg(feature = "server-gui")]
        {
            self.guarded_decide(id, true).await
        }
        #[cfg(not(feature = "server-gui"))]
        {
            let _ = id;
            GuardedOutcome::Err(crate::bridge::checks::not_in_this_build("guarded_approve"))
        }
    }

    /// `guarded_cancel`. `id` is already checked.
    pub async fn guarded_cancel(&self, id: &str) -> GuardedOutcome {
        #[cfg(feature = "server-gui")]
        {
            self.guarded_decide(id, false).await
        }
        #[cfg(not(feature = "server-gui"))]
        {
            let _ = id;
            GuardedOutcome::Err(crate::bridge::checks::not_in_this_build("guarded_cancel"))
        }
    }
}
