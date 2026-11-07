// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The 1.4.4 commands, refused (RD5). The project's own refusal module replaces this one.

use crate::bridge::checks::not_in_this_build;
use crate::bridge::types::{ActionResult, InstallMode};

use super::Controller;

impl Controller {
    /// `revived_install`.
    pub async fn revived_install(&self, mode: InstallMode) -> ActionResult {
        let _ = mode;
        ActionResult::Err(not_in_this_build("revived_install"))
    }

    /// `revived_choose_folder`.
    pub async fn revived_choose_folder(&self) -> ActionResult {
        ActionResult::Err(not_in_this_build("revived_choose_folder"))
    }

    /// `revived_use_detected`.
    pub async fn revived_use_detected(&self, index: usize) -> ActionResult {
        let _ = index;
        ActionResult::Err(not_in_this_build("revived_use_detected"))
    }
}
