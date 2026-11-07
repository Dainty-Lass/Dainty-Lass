// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Game installs through Epic (`game_install_*`), unanswered here. The project's install module, over
//! `net::epic_install`, replaces this one and fills [`Installs`].

use std::collections::BTreeMap;

use crate::bridge::checks::not_in_this_build;
use crate::bridge::types::{InstallInfo, InstallOutcome};

use super::Controller;

/// Each game version's install.
#[derive(Debug, Default)]
pub struct Installs {
    /// By game version.
    pub by_version: BTreeMap<String, InstallInfo>,
}

impl Controller {
    /// `game_install_start`.
    pub async fn game_install_start(&self, version: &str) -> InstallOutcome {
        let _ = version;
        InstallOutcome::Err(not_in_this_build("game_install_start"))
    }

    /// `game_install_pause`: whether it applied.
    #[must_use]
    pub fn game_install_pause(&self, version: &str) -> bool {
        let _ = version;
        false
    }

    /// `game_install_resume`: whether it applied.
    #[must_use]
    pub fn game_install_resume(&self, version: &str) -> bool {
        let _ = version;
        false
    }

    /// `game_install_cancel`: whether it applied.
    #[must_use]
    pub fn game_install_cancel(&self, version: &str) -> bool {
        let _ = version;
        false
    }

    /// `game_install_get`.
    #[must_use]
    pub fn game_install_get(&self, version: &str) -> InstallOutcome {
        let _ = version;
        InstallOutcome::Err(not_in_this_build("game_install_get"))
    }
}
