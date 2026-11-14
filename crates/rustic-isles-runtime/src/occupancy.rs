//! `SP-3@R21` (spec-runtime-r21 3): one `[NetConnEdge]` line per change of the listen driver's client count, for
//! the Director's Training Grounds stop. Servers only. The decisions are `core::occupancy`'s.

use crate::ffi::{log, memory};
use crate::handlers::{self, TickHandler};
use rustic_isles_runtime_core::occupancy::{ConnEdge, ORDER, UNKNOWN, counts};
use rustic_isles_runtime_core::offsets::r8::NET_DRIVER_CLIENT_COUNT;
use rustic_isles_runtime_core::registry;
use std::sync::Mutex;

static EDGE: Mutex<ConnEdge> = Mutex::new(ConnEdge::new());

/// Init, before the handler lists freeze.
pub fn register(server: bool) {
    if !server || !registry::owns(b"SP-3@R21") {
        return;
    }
    let result = handlers::register_tick(TickHandler {
        slice: "R21",
        order: ORDER,
        before: None,
        after: Some(after_tick),
    });
    if let Err(error) = result {
        log::defer(format!("[MitRuntime] R21 handler on SP-3 not registered: {error}"));
    }
}

fn after_tick(_engine: usize, _delta: f32) {
    let Ok(mut edge) = EDGE.try_lock() else {
        return;
    };
    edge.tick();
    // Reported only once listening, as the AGPL did on both its listen paths
    if crate::listen::state().is_none() {
        return;
    }
    let driver = crate::listen::net_driver().unwrap_or(0);
    let mut bytes = [0u8; 8];
    let (count, max) = if driver != 0 && memory::read_bytes(driver + NET_DRIVER_CLIENT_COUNT, &mut bytes) {
        counts(bytes)
    } else {
        (UNKNOWN, UNKNOWN)
    };
    if let Some(line) = edge.report(count, max, driver) {
        log::line(&line);
    }
}
