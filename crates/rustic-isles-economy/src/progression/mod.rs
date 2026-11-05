// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! Progression (progression spec): XP tracks and their curves, the grant ledger, the rank confirm, the objective
//! report and mastery, escalation, the Slayer's Path, encountered content, breadcrumbs and the session supplies.

pub mod reforge;
pub mod tracks;

pub use tracks::{curves, resolve_track_id};

use rustic_isles::Handlers;

#[derive(Clone, Default)]
pub struct ProgressionPorts;

pub fn register(handlers: &mut Handlers, ports: ProgressionPorts) {
    let _ = (handlers, ports);
}
