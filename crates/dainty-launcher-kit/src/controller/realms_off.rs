// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Realm hosting and joining in a build without the `realms` feature (RF7): the commands answer
//! `not_in_this_build`, and shutdown is a no-op.

use super::Controller;
use crate::bridge::types::{ErrorCode, LauncherError, RealmKind, RealmOutcome};

#[derive(Default)]
pub struct RealmsState {
    _priv: (),
}

impl Controller {
    pub(crate) async fn realms_shutdown(&self) {}

    fn realms_not_in_this_build(command: &str) -> RealmOutcome {
        RealmOutcome::Err(LauncherError::with_detail(ErrorCode::NotInThisBuild, command))
    }

    /// `realm_host`.
    pub async fn realm_host(&self, _kind: RealmKind) -> RealmOutcome {
        Self::realms_not_in_this_build("realm_host")
    }

    /// `realm_join`.
    pub async fn realm_join(&self, _realm: &str) -> RealmOutcome {
        Self::realms_not_in_this_build("realm_join")
    }

    /// `realm_stop`.
    pub async fn realm_stop(&self) -> RealmOutcome {
        Self::realms_not_in_this_build("realm_stop")
    }

    /// `realm_get`.
    pub async fn realm_get(&self) -> RealmOutcome {
        Self::realms_not_in_this_build("realm_get")
    }
}
