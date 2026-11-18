//! The object census (spec-runtime-census): the watchdog asks, R8's after handler runs it on the game thread.

use super::{CONN_COUNT, CONN_STAMP, hub_role};
use crate::ffi::census::RegionCache;
use crate::ffi::{engine, log, win};
use rustic_isles_runtime_core::census::{self as core, Previous, Report};
use rustic_isles_runtime_core::server_life::FRESH_MS;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

static DUE: AtomicBool = AtomicBool::new(false);
static NAMES_OFF_LOGGED: AtomicBool = AtomicBool::new(false);
static PREVIOUS: Mutex<Option<Previous>> = Mutex::new(None);

/// The watchdog's poll `iter` (4.2): asks for a census when one is due.
pub fn poll(iter: u64) {
    if core::due(iter) {
        DUE.store(true, Ordering::Relaxed);
    }
}

/// R8's after handler: runs a census that is due (4.2).
pub fn step() {
    if DUE.swap(false, Ordering::Relaxed) {
        run();
    }
}

fn run() {
    if !engine::names_enabled() {
        if !NAMES_OFF_LOGGED.swap(true, Ordering::Relaxed) {
            log::line(core::NAMES_OFF_LINE);
        }
        return;
    }
    let started = Instant::now();
    let memory = RegionCache::default();
    let Some(engine) = engine::engine_over(&memory) else {
        return;
    };
    let Some(walk) = core::walk(&memory, engine.objects()) else {
        log::line(core::ARRAY_LINE);
        return;
    };
    let resolved = core::resolve(&engine, &walk);
    let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let stamp = CONN_STAMP.load(Ordering::Relaxed);
    let report = Report {
        classes: resolved.classes,
        packages: resolved.packages,
        objects: walk.objects,
        unreadable: walk.unreadable,
        sizes: resolved.sizes,
        ms,
        connections: CONN_COUNT.load(Ordering::Relaxed),
        stale: stamp == 0 || win::tick().saturating_sub(stamp) > FRESH_MS,
        role: hub_role(),
    };
    let Ok(mut previous) = PREVIOUS.lock() else {
        return;
    };
    let (lines, next) = core::lines(&report, previous.as_ref());
    *previous = Some(next);
    drop(previous);
    for line in lines {
        log::line(&line);
    }
}
