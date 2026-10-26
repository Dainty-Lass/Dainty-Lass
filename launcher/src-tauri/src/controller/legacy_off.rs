//! A build without the `legacy` feature: `legacy.rs`'s names, with nothing to play. A Dauntless Revived
//! server is listed but never current (`settings.rs`), so the page's 1.4.4 calls are refused as before
//! (RD5, RD22).

use std::path::Path;

use super::stored::StoredServer;
use super::{Controller, Outcome, State, err};
use crate::bridge::types::{ActionResult, ErrorCode, Game, InstallMode, Phase, RevivedInfo};
use crate::net::hostapi::{Flavor, REFORGED};

#[derive(Clone, Default)]
pub struct LegacyConfig;

#[derive(Default)]
pub struct LegacyState;

pub struct Parts;

impl Parts {
    pub fn new(_user_data: &Path) -> Self {
        Self
    }
}

pub(super) fn flavor(_sv: &StoredServer) -> Flavor {
    REFORGED
}

impl Controller {
    pub(super) fn legacy_phase(&self, _st: &State) -> Option<Phase> {
        None
    }

    pub(super) fn revived_info(_st: &State) -> Option<RevivedInfo> {
        None
    }

    /// A saved Dauntless Revived server is refused as its invite would be (`switch_server`)
    pub(super) fn other_edition(&self, _sv: &StoredServer) -> Option<Game> {
        None
    }

    /// The edition is always `reborn`
    pub(super) fn edition_change(&self, _wanted: Option<Game>) -> Option<Game> {
        None
    }

    pub(super) async fn edition_changed(&self) {}

    pub(super) fn forget_legacy_server(_st: &mut State) {}

    pub(super) async fn inspect_revived(&self, _detect: bool) {}

    pub(super) fn cancel_revived(&self) {}

    pub(super) async fn play_revived(&self, _sv: &StoredServer) -> Outcome {
        Err(err(ErrorCode::Unknown, None))
    }

    pub(super) async fn revived_game_ended(&self) {}

    pub async fn revived_install(&self, _mode: InstallMode) -> ActionResult {
        ActionResult::err(ErrorCode::Unknown)
    }

    pub async fn revived_choose_folder(&self) -> ActionResult {
        ActionResult::err(ErrorCode::Unknown)
    }

    pub async fn revived_use_detected(&self, _index: u32) -> ActionResult {
        ActionResult::err(ErrorCode::Unknown)
    }
}
