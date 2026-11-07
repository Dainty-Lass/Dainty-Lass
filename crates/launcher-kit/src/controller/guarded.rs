// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Guarded actions (DD23), which only the server manager serves. The project's guarded module replaces this one
//! behind `server-gui`.

use crate::bridge::checks::not_in_this_build;
use crate::bridge::types::{GuardedList, GuardedOutcome};

use super::Controller;

impl Controller {
    /// `guarded_list`.
    #[must_use]
    pub fn guarded_list(&self) -> GuardedList {
        GuardedList::Err(not_in_this_build("guarded_list"))
    }

    /// `guarded_approve`.
    pub async fn guarded_approve(&self, id: &str) -> GuardedOutcome {
        let _ = id;
        GuardedOutcome::Err(not_in_this_build("guarded_approve"))
    }

    /// `guarded_cancel`.
    pub async fn guarded_cancel(&self, id: &str) -> GuardedOutcome {
        let _ = id;
        GuardedOutcome::Err(not_in_this_build("guarded_cancel"))
    }
}
