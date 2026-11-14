//! R15, inventory and economy (spec-runtime-r15): supplies without a stack cap, the cell fusion time and the
//! loadout slot repair, as handlers on SP-1 and SP-2. No hook of its own; owns `SP-1@R15` and `SP-2@R15`.

mod repair;
mod writes;

use crate::ffi::economy::ScanMemory;
use crate::ffi::{detaching, engine, log, win};
use crate::handlers::{self, PeCall, PeHandler, Point, Verdict};
use rustic_isles_runtime_core::economy::find_named;
use rustic_isles_runtime_core::engine::{Engine, NameCache};
use rustic_isles_runtime_core::memory::Memory;
use rustic_isles_runtime_core::names::{Kind, Matcher};
use rustic_isles_runtime_core::offsets::data;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{LazyLock, Mutex, MutexGuard};

/// Every call whose function has a name.
const NAMED: Matcher = &[(Kind::Prefix, "")];
const CLIENT_RESTART: Matcher = &[(Kind::Exact, "Function Engine.PlayerController.ClientRestart")];

/// The writes run before other MIT before handlers (6.4).
const WRITES_ORDER: i32 = -100;

/// R15's four handlers (section 8).
pub fn handlers() -> [(Point, PeHandler); 4] {
    [
        (
            Point::Sp1,
            PeHandler {
                slice: "R15",
                order: WRITES_ORDER,
                matcher: NAMED,
                before: Some(writes_client),
                after: None,
            },
        ),
        (
            Point::Sp1,
            PeHandler {
                slice: "R15",
                order: 0,
                matcher: CLIENT_RESTART,
                before: Some(client_restart),
                after: None,
            },
        ),
        (
            Point::Sp1,
            PeHandler {
                slice: "R15",
                order: 0,
                matcher: NAMED,
                before: None,
                after: Some(repair_after),
            },
        ),
        (
            Point::Sp2,
            PeHandler {
                slice: "R15",
                order: WRITES_ORDER,
                matcher: NAMED,
                before: Some(writes_server),
                after: None,
            },
        ),
    ]
}

/// Init registers the handlers before the lists freeze.
pub fn register() {
    for (point, handler) in handlers() {
        if let Err(error) = handlers::register_pe(point, handler) {
            log::defer(format!("[MitRuntime] R15 handler not registered: {error}"));
        }
    }
}

static GAME_THREAD: AtomicU32 = AtomicU32::new(0);

/// The client game thread: the first SP-1 dispatch R15's handlers saw.
fn record_thread() {
    let _ = GAME_THREAD.compare_exchange(0, win::thread_id(), Ordering::SeqCst, Ordering::SeqCst);
}

fn on_game_thread() -> bool {
    GAME_THREAD.load(Ordering::SeqCst) == win::thread_id()
}

fn writes_client(_: &PeCall) -> Verdict {
    if !detaching() {
        record_thread();
        writes::run();
    }
    Verdict::Continue
}

fn writes_server(_: &PeCall) -> Verdict {
    if !detaching() {
        writes::run();
    }
    Verdict::Continue
}

fn client_restart(call: &PeCall) -> Verdict {
    if !detaching() {
        record_thread();
        repair::capture(call.object);
    }
    Verdict::Continue
}

fn repair_after(_: &PeCall) {
    if !detaching() {
        record_thread();
        repair::pass();
    }
}

/// An in-progress mark, cleared on every exit path (a panic included).
struct Busy(&'static AtomicBool);

impl Busy {
    fn enter(flag: &'static AtomicBool) -> Option<Busy> {
        (!flag.swap(true, Ordering::SeqCst)).then_some(Busy(flag))
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

static SCAN_NAMES: LazyLock<NameCache> = LazyLock::new(NameCache::default);

/// Runs `scan` over the object array with a region-caching reader and an engine over it.
fn scan<R>(scan: impl FnOnce(&dyn Memory, &Engine, usize) -> R) -> Option<R> {
    let base = win::image_base();
    let objects = base.checked_add(usize::try_from(data::GOBJECTS).ok()?)?;
    let names = base.checked_add(usize::try_from(data::GNAMES).ok()?)?;
    let memory = ScanMemory::default();
    let engine = Engine::new(&memory, objects, names, Some(&*SCAN_NAMES));
    Some(scan(&memory, &engine, objects))
}

/// Finds the objects of `wanted` whose cell is 0 (or no longer passes `keep`), in one pass. Names go through R0's
/// `full_name`, so a failed name self-check finds nothing (7.2).
fn resolve<const N: usize>(cells: &mut [usize; N], wanted: &[(&str, &str); N], keep: impl Fn(usize) -> bool) {
    for cell in cells.iter_mut() {
        if *cell != 0 && !keep(*cell) {
            *cell = 0;
        }
    }
    if cells.iter().all(|&cell| cell != 0) {
        return;
    }
    let missing: Vec<usize> = (0..N).filter(|&at| cells[at] == 0).collect();
    let names: Vec<(&str, &str)> = missing.iter().map(|&at| wanted[at]).collect();
    let found = scan(|memory, engine, objects| {
        find_named(
            memory,
            objects,
            &names,
            &|object| engine.object_name(object),
            &engine::full_name,
        )
    });
    for (at, object) in missing.into_iter().zip(found.unwrap_or_default()) {
        cells[at] = object.unwrap_or(0);
    }
}

/// The classes R15 tests objects against (native classes are never freed: kept for the process).
#[derive(Clone, Copy)]
enum Class {
    Catalog = 0,
    DataTable = 1,
    Controller = 2,
}

const CLASSES: [(&str, &str); 3] = [
    ("ArchonCatalog", "Class Archon.ArchonCatalog"),
    ("DataTable", "Class Engine.DataTable"),
    ("ArchonPlayerController", "Class Archon.ArchonPlayerController"),
];

static CLASS_CELLS: Mutex<[usize; 3]> = Mutex::new([0; 3]);

fn class(which: Class) -> Option<usize> {
    let mut cells = lock(&CLASS_CELLS);
    let at = which as usize;
    if cells[at] == 0 {
        resolve(&mut cells, &CLASSES, |_| true);
    }
    Some(cells[at]).filter(|&class| class != 0)
}

/// The UFunctions the repair dispatches (7.3).
pub(crate) const FUNCTIONS: [(&str, &str); 6] = [
    (
        "OnRep_ItemInstanceIdChanged",
        "Function Archon.ArchonLoadoutItemSlot.OnRep_ItemInstanceIdChanged",
    ),
    (
        "OnRep_CellsChanged",
        "Function Archon.ArchonLoadoutItemSlot.OnRep_CellsChanged",
    ),
    (
        "OnRep_ItemPartsChanged",
        "Function Archon.ArchonLoadoutItemSlot.OnRep_ItemPartsChanged",
    ),
    (
        "OnRep_DyeChanged",
        "Function Archon.ArchonLoadoutItemSlot.OnRep_DyeChanged",
    ),
    (
        "OnRep_TransmogChanged",
        "Function Archon.ArchonLoadoutItemSlot.OnRep_TransmogChanged",
    ),
    (
        "OnRep_GenericLoadoutChanged",
        "Function Archon.ArchonLoadout.OnRep_GenericLoadoutChanged",
    ),
];

static FUNCTION_CELLS: Mutex<[usize; 6]> = Mutex::new([0; 6]);

/// The six UFunctions (0 where not found), kept while they stay live; missing ones are searched in one pass.
fn functions() -> [usize; 6] {
    let mut cells = lock(&FUNCTION_CELLS);
    resolve(&mut cells, &FUNCTIONS, engine::is_live_object);
    *cells
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_handlers_with_section_8s_orders() {
        let handlers = handlers();
        let shape: Vec<(Point, i32, bool, bool)> = handlers
            .iter()
            .map(|(point, handler)| (*point, handler.order, handler.before.is_some(), handler.after.is_some()))
            .collect();
        assert_eq!(
            shape,
            [
                (Point::Sp1, -100, true, false),
                (Point::Sp1, 0, true, false),
                (Point::Sp1, 0, false, true),
                (Point::Sp2, -100, true, false),
            ]
        );
        assert!(handlers.iter().all(|(_, handler)| handler.slice == "R15"));
        assert_eq!(handlers[1].1.matcher, CLIENT_RESTART);
        for at in [0, 2, 3] {
            assert_eq!(handlers[at].1.matcher, NAMED);
        }
    }

    #[test]
    fn busy_mark_clears() {
        static FLAG: AtomicBool = AtomicBool::new(false);
        let busy = Busy::enter(&FLAG);
        assert!(busy.is_some());
        assert!(Busy::enter(&FLAG).is_none());
        drop(busy);
        assert!(Busy::enter(&FLAG).is_some());
    }
}
