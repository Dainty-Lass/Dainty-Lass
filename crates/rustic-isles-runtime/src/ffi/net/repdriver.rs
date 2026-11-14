//! NETD-2 `SetReplicationDriver` (9) with the player character's default-object boost, and NETD-3
//! `CreateReplicationDriver` (10), which makes the engine build the game's replication graph.

use super::{Callee, budgeted, hook, log, panicked, rva_address, store};
use crate::ffi::broker::NextCell;
use crate::ffi::exit::Install;
use crate::ffi::memory::{read, read_unaligned, readable};
use crate::ffi::{detaching, engine, flags, reflect};
use rustic_isles_runtime_core::net::{
    CONSTRUCT_MISMATCH, CREATE_DRIVER_CHECKED, CREATE_LOGGED, DEFAULT_OBJECT_CHECKED, DEFAULT_OBJECT_MISSING,
    DEFAULT_OBJECT_MISSING_KIND, GATE_LOGGED, LineBudget, SET_REP_LOGGED, always_relevant_set, boosted_rates, budgets,
    class_text, construct_params, create_class, create_construct_null, create_constructed, create_game_returned,
    create_gate, flags as names, graph_class_line, rates_boosted, rates_of, set_rep_driver,
};
use rustic_isles_runtime_core::offsets::net::{
    ACTOR_ALWAYS_RELEVANT_BIT, ACTOR_ALWAYS_RELEVANT_BYTE, ACTOR_NET_UPDATE_FREQUENCY, CLASS_ABSTRACT, CLASS_FLAGS,
    NETDRIVER_REP_DRIVER_CLASS, REP_DRIVER_GATE, REP_GRAPH_FEATURES, STATIC_CONSTRUCT_OBJECT,
    STATIC_CONSTRUCT_OBJECT_BYTES, TRANSIENT_PACKAGE,
};
use std::cell::Cell;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

type SetReplicationDriverFn = unsafe extern "C" fn(*mut c_void, *mut c_void);
type CreateReplicationDriverFn = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> *mut c_void;
type StaticConstructFn = unsafe extern "C" fn(*const u8) -> *mut c_void;

static SET_NEXT: NextCell = NextCell::new();
static SET_CALLS: AtomicU32 = AtomicU32::new(0);
static SET_PANICKED: AtomicBool = AtomicBool::new(false);
static RATES_LOGGED: AtomicBool = AtomicBool::new(false);
static RELEVANT_LOGGED: AtomicBool = AtomicBool::new(false);
static MISSING: LineBudget = LineBudget::new(budgets::DEFAULT_OBJECT_MISSING);
static BOOST_OFF: OnceLock<bool> = OnceLock::new();
static ALWAYS_RELEVANT: OnceLock<bool> = OnceLock::new();

static CREATE_NEXT: NextCell = NextCell::new();
static CREATE_CALLS: AtomicU32 = AtomicU32::new(0);
static CREATE_PANICKED: AtomicBool = AtomicBool::new(false);
static CONSTRUCT: Callee = Callee::new(STATIC_CONSTRUCT_OBJECT, STATIC_CONSTRUCT_OBJECT_BYTES);
static GRAPH_CLASS: OnceLock<Option<(usize, &'static str)>> = OnceLock::new();
static SELF_CONSTRUCTED: AtomicBool = AtomicBool::new(false);

/// The graph class NETD-3 chose (10.1), once searched; R12 reads it.
pub fn graph_class() -> Option<usize> {
    GRAPH_CLASS.get().copied().flatten().map(|(class, _)| class)
}

/// Whether the last NETD-3 call returned a graph it built itself (R12 reads it).
pub fn graph_self_constructed() -> bool {
    SELF_CONSTRUCTED.load(Ordering::SeqCst)
}

/// An object's short name, `null` for 0 or unnamed.
fn short_name(object: usize) -> String {
    if object == 0 {
        return "null".to_owned();
    }
    engine::engine()
        .and_then(|engine| engine.object_name(object))
        .unwrap_or_else(|| "null".to_owned())
}

unsafe extern "C" fn set_detour(driver: *mut c_void, rep_driver: *mut c_void) {
    let next = SET_NEXT.load();
    if next == 0 {
        return;
    }
    // SAFETY: the broker wrote a function of SetReplicationDriver's signature into this cell
    let next: SetReplicationDriverFn = unsafe { std::mem::transmute::<usize, SetReplicationDriverFn>(next) };
    if !detaching() {
        crate::lifecycle::write_status_once();
        if catch_unwind(AssertUnwindSafe(|| set_body(driver as usize, rep_driver as usize))).is_err() {
            panicked("NETD-2", &SET_PANICKED);
        }
    }
    // SAFETY: the original arguments, unchanged (only the game's graph opens actor channels on this build)
    unsafe { next(driver, rep_driver) }
}

fn set_body(driver: usize, rep_driver: usize) {
    let n = SET_CALLS.fetch_add(1, Ordering::Relaxed);
    if n < SET_REP_LOGGED {
        let class = (rep_driver != 0).then(|| reflect::class_name_of(rep_driver)).flatten();
        log::line(&set_rep_driver(
            n,
            driver,
            rep_driver,
            &class_text(rep_driver, class.as_deref()),
        ));
    }
    let boost = !*BOOST_OFF.get_or_init(|| flags::flag(names::DISABLE_PLAYER_REP_BOOST));
    let relevant = *ALWAYS_RELEVANT.get_or_init(|| flags::flag(names::PLAYER_ALWAYS_RELEVANT));
    if boost || relevant {
        boost_default_object(boost, relevant);
    }
}

/// 9.1: before the graph initialises, so it registers every player pawn at period 1. Looked up again on each call
/// until found, since the class may load after the first driver.
fn boost_default_object(boost: bool, relevant: bool) {
    let object = reflect::find_class("BP_PlayerCharacter_C")
        .and_then(reflect::default_object)
        .filter(|&object| readable(object, DEFAULT_OBJECT_CHECKED));
    let Some(object) = object else {
        budgeted(&MISSING, DEFAULT_OBJECT_MISSING_KIND, || {
            DEFAULT_OBJECT_MISSING.to_owned()
        });
        return;
    };
    if boost {
        // Both rates share one aligned word: `NetUpdateFrequency` at +0x108, `MinNetUpdateFrequency` at +0x10C
        let rates = object + ACTOR_NET_UPDATE_FREQUENCY;
        if let Some(old) = read::<u64>(rates)
            && reflect::write::<u64>(rates, boosted_rates())
            && !RATES_LOGGED.swap(true, Ordering::SeqCst)
        {
            let (rate, min) = rates_of(old);
            log::line(&rates_boosted(rate, min));
        }
    }
    if relevant {
        let at = object + ACTOR_ALWAYS_RELEVANT_BYTE;
        if let Some(old) = reflect::u8_at(at) {
            let new = old | ACTOR_ALWAYS_RELEVANT_BIT;
            if reflect::write::<u8>(at, new) && !RELEVANT_LOGGED.swap(true, Ordering::SeqCst) {
                log::line(&always_relevant_set(old, new));
            }
        }
    }
}

/// What NETD-3 changed and must put back within the same call (step 7).
#[derive(Default)]
struct Restore {
    flags: Cell<Option<(usize, u32)>>,
    features: Cell<Option<(usize, i32)>>,
}

impl Restore {
    fn apply(&self) {
        if let Some((at, flags)) = self.flags.take() {
            store(at, flags);
        }
        if let Some((at, num)) = self.features.take() {
            store(at, num);
        }
    }
}

/// 10.1: searched once per process and kept, found or not.
fn chosen_class() -> Option<(usize, &'static str)> {
    *GRAPH_CLASS.get_or_init(|| {
        let chosen = rustic_isles_runtime_core::net::graph_class(
            reflect::find_class("ArchonReplicationGraph"),
            reflect::find_class("BasicReplicationGraph"),
        );
        log::line(&graph_class_line(chosen));
        chosen
    })
}

unsafe extern "C" fn create_detour(driver: *mut c_void, a2: *mut c_void, a3: *mut c_void) -> *mut c_void {
    let next = CREATE_NEXT.load();
    if next == 0 {
        return std::ptr::null_mut();
    }
    // SAFETY: the broker wrote a function of CreateReplicationDriver's signature into this cell
    let next: CreateReplicationDriverFn = unsafe { std::mem::transmute::<usize, CreateReplicationDriverFn>(next) };
    let call_next = || {
        // SAFETY: the original arguments
        unsafe { next(driver, a2, a3) }
    };
    if detaching() {
        return call_next();
    }
    crate::lifecycle::write_status_once();
    let restore = Restore::default();
    let returned: Cell<Option<*mut c_void>> = Cell::new(None);
    let body = catch_unwind(AssertUnwindSafe(|| {
        create_body(driver as usize, &restore, || {
            let result = call_next();
            returned.set(Some(result));
            result as usize
        })
    }));
    match body {
        Ok(result) => result as *mut c_void,
        Err(_) => {
            panicked("NETD-3", &CREATE_PANICKED);
            restore.apply();
            returned.get().unwrap_or_else(call_next)
        }
    }
}

/// 10 steps 1 to 8; `game` calls the game's own function.
fn create_body(driver: usize, restore: &Restore, game: impl FnOnce() -> usize) -> usize {
    let n = CREATE_CALLS.fetch_add(1, Ordering::Relaxed);
    if let Some(gate) = rva_address(REP_DRIVER_GATE)
        && let Some(old) = read_unaligned::<u32>(gate)
        && store::<u32>(gate, 1)
        && n < GATE_LOGGED
    {
        log::line(&create_gate(n, old));
    }
    let mut class = None;
    if readable(driver, CREATE_DRIVER_CHECKED) {
        let slot = driver + NETDRIVER_REP_DRIVER_CLASS;
        let before = read::<usize>(slot).unwrap_or(0);
        if before == 0
            && let Some((chosen, _)) = chosen_class()
        {
            let _ = reflect::write::<usize>(slot, chosen);
        }
        let after = read::<usize>(slot).unwrap_or(0);
        if n < CREATE_LOGGED {
            log::line(&create_class(n, before, after, &short_name(after)));
        }
        class = (after != 0).then_some(after);
    }
    let class = class.or_else(|| chosen_class().map(|(class, _)| class));
    if let Some(features) = rva_address(REP_GRAPH_FEATURES).map(|array| array + 8)
        && let Some(num) = read_unaligned::<i32>(features)
        && num > 0
        && store::<i32>(features, 0)
    {
        restore.features.set(Some((features, num)));
    }
    if let Some(class) = class {
        let at = class + CLASS_FLAGS;
        if let Some(flags) = read_unaligned::<u32>(at)
            && flags & CLASS_ABSTRACT != 0
            && store::<u32>(at, flags & !CLASS_ABSTRACT)
        {
            restore.flags.set(Some((at, flags)));
        }
    }
    if let Some(class) = class
        && let Some(object) = construct(class, n)
    {
        restore.apply();
        SELF_CONSTRUCTED.store(true, Ordering::SeqCst);
        return object;
    }
    SELF_CONSTRUCTED.store(false, Ordering::SeqCst);
    let result = game();
    restore.apply();
    if n < CREATE_LOGGED {
        log::line(&create_game_returned(n, result, &short_name_of_class(result)));
    }
    result
}

/// The short name of an object's class, `null` for no object.
fn short_name_of_class(object: usize) -> String {
    if object == 0 {
        return "null".to_owned();
    }
    reflect::class_name_of(object).unwrap_or_else(|| "null".to_owned())
}

/// Step 5: the graph built in the transient package; `None` sends the call on to the game.
fn construct(class: usize, n: u32) -> Option<usize> {
    let function = CONSTRUCT.address(|| log::line(CONSTRUCT_MISMATCH))?;
    let outer = rva_address(TRANSIENT_PACKAGE)
        .and_then(read::<usize>)
        .filter(|&outer| outer != 0 && engine::is_live_object(outer))?;
    let params = construct_params(class, outer);
    // SAFETY: the engine function at its checked entry, on the game thread, with an 8-aligned 0x50-byte block
    // holding a live class and the live transient package
    let object = unsafe {
        let function = std::mem::transmute::<usize, StaticConstructFn>(function);
        function(params.as_ptr().cast::<u8>()) as usize
    };
    crate::ffi::memory::frame_tick(true);
    if n < CREATE_LOGGED {
        log::line(&create_constructed(n, object, &short_name_of_class(object)));
    }
    if object == 0 {
        log::line(&create_construct_null(n));
        return None;
    }
    Some(object)
}

/// Installs NETD-2.
pub fn install_set(base: usize, hooks_off: Option<&'static str>) -> Install {
    hook("NETD-2", base, set_detour as *const () as usize, &SET_NEXT, hooks_off)
}

/// Installs NETD-3.
pub fn install_create(base: usize, hooks_off: Option<&'static str>) -> Install {
    hook(
        "NETD-3",
        base,
        create_detour as *const () as usize,
        &CREATE_NEXT,
        hooks_off,
    )
}
