// Copyright (c) 2026 Silent Spud
// SPDX-License-Identifier: MIT

//! `region_ping`, unanswered here. The project's region module, over `net::regionping`, replaces this one.

use crate::bridge::checks::not_in_this_build;
use crate::bridge::types::RegionPing;

use super::Controller;

impl Controller {
    /// `region_ping`.
    pub async fn region_ping(&self) -> RegionPing {
        RegionPing::Err(not_in_this_build("region_ping"))
    }
}
