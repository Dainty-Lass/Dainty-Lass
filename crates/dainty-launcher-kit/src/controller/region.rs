// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Region ping (DN7, on `net::regionping`): the joined server is the player's home, so its node list
//! is the one timed and its account key the one the order goes with.

use super::Controller;
use crate::bridge::types::{ErrorCode, LauncherError, RegionPing, ServerGame};
use crate::net::regionping::{PingSettings, region_ping};

impl Controller {
    /// `region_ping`.
    pub async fn region_ping(&self) -> RegionPing {
        let Some(sv) = self.server() else {
            return RegionPing::Err(LauncherError::new(ErrorCode::ServerUnreachable));
        };
        // Only 1.14.7 joins a network (DD16)
        if sv.game == ServerGame::Revived {
            return RegionPing::Ok {
                regions: Vec::new(),
                reported: false,
            };
        }
        let key = self.load_key().await;
        region_ping(&sv.endpoint(), key.as_deref(), &PingSettings::default()).await
    }
}
