//! Runtime slice R18 (spec-runtime-r18): ESC-1 and the escalation buff-choice spawn. Servers only; on a client
//! nothing is registered or installed.

use crate::config;
use crate::ffi::{self, engine as live, flags, log, memory, reflect};
use crate::handlers::{self, PeCall, PeHandler, Point, TickHandler, Verdict};
use rustic_isles_runtime_core::escalation::buff_choice::{
    self, BuffChoice, DECLARING_CLASS, MATCHER, NO_SPAWN_FLAG, ORDER, Request, SPAWN_FUNCTION,
};
use rustic_isles_runtime_core::escalation::{find_declared_function, trials};
use rustic_isles_runtime_core::offsets::{layout, r18};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

static MEMORY: memory::ProcessMemory = memory::ProcessMemory;
static STATE: Mutex<BuffChoice> = Mutex::new(BuffChoice::new());
static TICKS: AtomicU64 = AtomicU64::new(0);
static FUNCTION: AtomicUsize = AtomicUsize::new(0);

fn state() -> MutexGuard<'static, BuffChoice> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Init, before the handler lists freeze: the note on SP-2 and the spawn on SP-3.
pub fn register() {
    if !config::is_server() {
        return;
    }
    let note = handlers::register_pe(
        Point::Sp2,
        PeHandler {
            slice: "R18",
            order: ORDER,
            matcher: MATCHER,
            before: Some(note),
            after: None,
        },
    );
    let spawn = handlers::register_tick(TickHandler {
        slice: "R18",
        order: ORDER,
        before: None,
        after: Some(spawn),
    });
    for (point, result) in [("SP-2", note), ("SP-3", spawn)] {
        if let Err(error) = result {
            log::defer(format!("[MitRuntime] R18 handler on {point} not registered: {error}"));
        }
    }
}

/// Installs ESC-1.
pub fn install_esc1(base: usize) -> Result<(), i32> {
    ffi::escalation::install_esc1(base)
}

/// ESC-1's body (R18 3.3 to 3.5): one line per arena table.
pub(crate) fn before_initialize(tables: usize) {
    let Some(engine) = reflect::engine() else {
        return;
    };
    let tables = trials::read_tables(&MEMORY, &engine, tables);
    if tables.is_empty() {
        return;
    }
    let now = ffi::escalation::now_ticks();
    let minutes = config::trials_rotation_minutes();
    let pinned = config::trials_week();
    for table in &tables {
        let plan = trials::plan(table, now, minutes, pinned);
        let mut patched = 0;
        for row in &plan.rows {
            let [start, end] = row.writes();
            let start = reflect::write::<i64>(start.0, start.1);
            let end = reflect::write::<i64>(end.0, end.1);
            if start && end {
                patched += 1;
            }
        }
        log::line(&plan.line(patched));
    }
}

/// SP-2, before: notes the round-finished transition. Never absorbs.
fn note(call: &PeCall) -> Verdict {
    if call.object == 0 || call.params == 0 || flags::flag(NO_SPAWN_FLAG) {
        return Verdict::Continue;
    }
    if let Some(request) = buff_choice::transition(&MEMORY, call.object, call.params) {
        let queued = state().note(request);
        if let Some(line) = queued {
            log::line(&line);
        }
    }
    Verdict::Continue
}

/// The spawn function, found once (a UFunction is never unloaded).
fn spawn_function(component: usize) -> Option<usize> {
    let cached = FUNCTION.load(Ordering::Acquire);
    if cached != 0 {
        return Some(cached);
    }
    let class = memory::read::<usize>(component.checked_add(layout::UOBJECT_CLASS)?)?;
    let engine = reflect::engine()?;
    let found = find_declared_function(&MEMORY, &engine, class, SPAWN_FUNCTION, DECLARING_CLASS)?;
    FUNCTION.store(found, Ordering::Release);
    Some(found)
}

/// SP-3, after the engine tick, every 64th call (R18 4.3).
fn spawn(_engine: usize, _delta: f32) {
    let count = TICKS.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    if !buff_choice::cadence_due(count) {
        return;
    }
    let Some(request) = state().take() else {
        return;
    };
    dispatch(request);
}

fn dispatch(request: Request) {
    let component = request.component;
    if !memory::readable(component, r18::ESC_MODE_READ) || !live::is_live_object(component) {
        log::line(&buff_choice::skipped_line(request));
        return;
    }
    // Not found consumes the request without marking it served
    let Some(function) = spawn_function(component) else {
        log::line(&buff_choice::not_found_line(request));
        return;
    };
    state().mark_dispatched(request);
    log::line(&buff_choice::dispatching_line(request));
    ffi::escalation::process_event_original(component, function);
    log::line(&buff_choice::dispatched_line(request));
}
