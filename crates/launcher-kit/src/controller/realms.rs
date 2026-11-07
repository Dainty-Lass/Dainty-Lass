// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! Realms (RD27 spec §4.3), answered as a build without them does. The realm adapter over `rifts-host` replaces
//! this module behind the `realms` feature and fills [`RealmsState`].

use crate::bridge::checks::not_in_this_build;
use crate::bridge::types::{RealmKind, RealmOutcome, RealmView};

use super::Controller;

/// What the controller keeps for realms.
#[derive(Debug, Default)]
pub struct RealmsState {
    /// The last realms view.
    pub view: Option<RealmView>,
}

impl Controller {
    /// `realm_host`.
    pub async fn realm_host(&self, kind: RealmKind) -> RealmOutcome {
        let _ = kind;
        RealmOutcome::Err(not_in_this_build("realm_host"))
    }

    /// `realm_join`.
    pub async fn realm_join(&self, realm: &str) -> RealmOutcome {
        let _ = realm;
        RealmOutcome::Err(not_in_this_build("realm_join"))
    }

    /// `realm_stop`.
    pub async fn realm_stop(&self) -> RealmOutcome {
        RealmOutcome::Err(not_in_this_build("realm_stop"))
    }

    /// `realm_get`.
    #[must_use]
    pub fn realm_get(&self) -> RealmOutcome {
        RealmOutcome::Err(not_in_this_build("realm_get"))
    }

    /// Ends realm sessions on logout, a server change and exit.
    pub async fn realms_shutdown(&self) {}
}
