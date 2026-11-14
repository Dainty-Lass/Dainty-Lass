//! The timed writes (sections 4 and 5) on SP-1 and SP-2: the supply stack caps and the fusion times.

use super::{Busy, Class, class, lock, scan};
use crate::ffi::economy::{ScanMemory, write_i32, write_i64};
use crate::ffi::{engine, log, win};
use rustic_isles_runtime_core::economy::settings::{
    self, CELL_FUSION_MINUTES, KEEP_SUPPLY_STACK_CAPS, MAX_UNITS, Read,
};
use rustic_isles_runtime_core::economy::{Timer, catalog, find_instance, fusion, text};
use rustic_isles_runtime_core::env::Raw;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

const SUPPLY_PERIOD_MS: u64 = 5000;
const FUSION_KNOWN_PERIOD_MS: u64 = 30_000;
const FUSION_SEARCH_PERIOD_MS: u64 = 2000;

/// One guard for both features: two threads may dispatch (departure 5).
static RUNNING: AtomicBool = AtomicBool::new(false);

struct Supplies {
    timer: Timer,
    keep: Option<bool>,
    catalog: usize,
}

struct Fusion {
    timer: Timer,
    minutes: Option<u32>,
    table: usize,
}

static SUPPLIES: Mutex<Supplies> = Mutex::new(Supplies {
    timer: Timer::new(),
    keep: None,
    catalog: 0,
});
static FUSION: Mutex<Fusion> = Mutex::new(Fusion {
    timer: Timer::new(),
    minutes: None,
    table: 0,
});

/// The variable as `settings` reads it: set when present and non-empty, and R0's `env_value`.
fn read_env(name: &str) -> (bool, Option<String>) {
    (
        !matches!(win::env_raw(name, MAX_UNITS), Raw::Unset),
        win::env_value(name, MAX_UNITS),
    )
}

/// Both features' runs, each when due.
pub fn run() {
    let Some(_busy) = Busy::enter(&RUNNING) else {
        return;
    };
    let now = win::tick();
    supplies(now);
    fusion(now);
}

fn supplies(now: u64) {
    let mut state = lock(&SUPPLIES);
    if !state.timer.start(now, SUPPLY_PERIOD_MS) {
        return;
    }
    let keep = *state.keep.get_or_insert_with(|| {
        let (set, value) = read_env(KEEP_SUPPLY_STACK_CAPS);
        let (keep, note) = settings::keep_supply_caps(Read {
            set,
            value: value.as_deref(),
        });
        if let Some(note) = note {
            log::line(&note);
        }
        if keep {
            log::line(text::CAPS_KEPT);
        }
        keep
    });
    if keep {
        return;
    }
    if state.catalog == 0 || !engine::is_live_object(state.catalog) {
        let Some(class) = class(Class::Catalog) else {
            state.catalog = 0;
            return;
        };
        state.catalog = scan(|memory, _, objects| find_instance(memory, objects, class, |_| true))
            .flatten()
            .unwrap_or(0);
        if state.catalog == 0 {
            return;
        }
    }
    if let Some(line) = catalog::uncap(&ScanMemory::default(), state.catalog, &mut write_i32) {
        log::line(&line);
    }
}

fn fusion(now: u64) {
    let mut state = lock(&FUSION);
    let period = if state.table != 0 {
        FUSION_KNOWN_PERIOD_MS
    } else {
        FUSION_SEARCH_PERIOD_MS
    };
    if !state.timer.start(now, period) {
        return;
    }
    let minutes = *state.minutes.get_or_insert_with(|| {
        let (set, value) = read_env(CELL_FUSION_MINUTES);
        let (minutes, note) = settings::fusion_minutes(Read {
            set,
            value: value.as_deref(),
        });
        if let Some(note) = note {
            log::line(&note);
        }
        if minutes == 0 {
            log::line(text::FUSION_KEPT);
        }
        minutes
    });
    if settings::target_ticks(minutes).is_none() {
        return;
    }
    if state.table == 0 || !engine::is_live_object(state.table) {
        let Some(class) = class(Class::DataTable) else {
            state.table = 0;
            return;
        };
        state.table = scan(|memory, engine, objects| {
            fusion::find_table(memory, objects, class, &|object| engine.object_name(object))
        })
        .flatten()
        .unwrap_or(0);
        if state.table == 0 {
            return;
        }
    }
    let table = state.table;
    let line = scan(|memory, engine, _| {
        let name = engine.object_name(table).unwrap_or_default();
        fusion::set_time(
            memory,
            table,
            &name,
            minutes,
            &|element| engine.fname_at(element),
            &mut write_i64,
        )
    })
    .flatten();
    if let Some(line) = line {
        log::line(&line);
    }
}
