// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! `game_version_list`, empty here. The project's versions module, over `net::versions`, replaces this one.

use crate::bridge::types::GameVersionList;

use super::Controller;

impl Controller {
    /// `game_version_list`.
    pub async fn game_version_list(&self) -> GameVersionList {
        GameVersionList { versions: Vec::new() }
    }
}
