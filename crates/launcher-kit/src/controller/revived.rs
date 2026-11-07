// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! The page's Dauntless 1.4.4 calls (the bridge still names them), refused: a Dauntless Revived server is
//! listed but never current (`settings.rs`), so nothing here plays (RD5, RD22).

use super::Controller;
use crate::bridge::types::{ActionResult, ErrorCode, InstallMode};

impl Controller {
    /// `revived_install`.
    pub async fn revived_install(&self, _mode: InstallMode) -> ActionResult {
        ActionResult::err(ErrorCode::Unknown)
    }

    /// `revived_choose_folder`.
    pub async fn revived_choose_folder(&self) -> ActionResult {
        ActionResult::err(ErrorCode::Unknown)
    }

    /// `revived_use_detected`.
    pub async fn revived_use_detected(&self, _index: u32) -> ActionResult {
        ActionResult::err(ErrorCode::Unknown)
    }
}
