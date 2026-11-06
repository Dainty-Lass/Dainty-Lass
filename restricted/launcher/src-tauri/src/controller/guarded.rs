//! The guarded admin actions for the server GUI (DD23, DH6): the destructive and bulk actions that wait for a
//! delay or a second operator, listed and decided by the host in this process, as `dainty-host admin
//! guarded` does. The host is an owner no admin key can be, so it approves or cancels what any key asked;
//! the running server carries an approved action out within 30 seconds. Only the server GUI links the host
//! kit, so only it has these.

use serde_json::Value;

use super::{Controller, err};
use crate::bridge::types::{ErrorCode, GuardedActionInfo, GuardedList, GuardedOutcome, LauncherError};
use crate::platform::log;

/// A failure of the store as the page hears it: the codes it has words for, else a host failure.
fn refuse(failure: host::guarded::Failure) -> LauncherError {
    match failure.code {
        "not_found" => err(ErrorCode::GuardedNotFound, None),
        "not_waiting" => err(ErrorCode::GuardedNotWaiting, None),
        "bad_id" => err(ErrorCode::Unknown, None),
        "not_configured" => err(ErrorCode::HostFailed, Some("not_configured".to_owned())),
        _ => {
            log::warn(&format!("guarded actions: {}", failure.message));
            err(ErrorCode::HostFailed, Some("guarded".to_owned()))
        }
    }
}

fn info(value: &Value) -> Result<GuardedActionInfo, LauncherError> {
    serde_json::from_value(value.clone()).map_err(|error| {
        log::warn(&format!("guarded actions: an action could not be read: {error}"));
        err(ErrorCode::HostFailed, Some("guarded".to_owned()))
    })
}

impl Controller {
    /// What waits, oldest first.
    pub async fn guarded_list(&self) -> GuardedList {
        let Some(workspace) = self.workspace() else {
            return GuardedList::Err(err(ErrorCode::HostFailed, Some("workspace".to_owned())));
        };
        match host::guarded::list(&workspace).await {
            Ok(actions) => match actions.iter().map(info).collect() {
                Ok(actions) => GuardedList::Ok(actions),
                Err(error) => GuardedList::Err(error),
            },
            Err(failure) => GuardedList::Err(refuse(failure)),
        }
    }

    /// Approves (the server runs it within seconds) or cancels the action `id`, already checked.
    pub async fn guarded_decide(&self, id: &str, approve: bool) -> GuardedOutcome {
        let Some(workspace) = self.workspace() else {
            return GuardedOutcome::Err(err(ErrorCode::HostFailed, Some("workspace".to_owned())));
        };
        let decided = if approve {
            host::guarded::approve(&workspace, id).await
        } else {
            host::guarded::cancel(&workspace, id).await
        };
        match decided {
            Ok(action) => match info(&action) {
                Ok(action) => {
                    log::info(&format!(
                        "guarded actions: {} {} from the server GUI",
                        action.id,
                        if approve { "approved" } else { "canceled" }
                    ));
                    GuardedOutcome::Ok(Box::new(action))
                }
                Err(error) => GuardedOutcome::Err(error),
            },
            Err(failure) => GuardedOutcome::Err(refuse(failure)),
        }
    }
}
