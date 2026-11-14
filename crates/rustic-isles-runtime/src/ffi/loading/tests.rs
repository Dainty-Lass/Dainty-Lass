//! spec-runtime-r7 12.1, the DLL part: the bodies against fake game objects in test memory, with `Begin`, the weak
//! resolver and `CompleteLoadInternal` pointed at fakes, and every wrapper through the broker on a test function.

use super::super::broker;
use super::game::{self, fake};
use super::player_data::{Backfill, data_load_complete, failed, load_failed};
use super::progression::{bookkeeping, predicate, read_from_backend};
use super::readiness::has_finished_loading;
use super::restart_gate;
use rustic_isles_runtime_core::offsets::r7;
use std::cell::Cell;
use std::ffi::c_void;
use std::hint::black_box;
use std::sync::Once;
use std::sync::atomic::{AtomicU32, Ordering};

thread_local! {
    static BEGINS: Cell<u32> = const { Cell::new(0) };
}

unsafe extern "C" fn fake_begin(_manager: *mut c_void) {
    BEGINS.with(|begins| begins.set(begins.get() + 1));
}

/// Resolves any pair with a non-zero index to a made-up owner.
unsafe extern "C" fn fake_weak(weak: *const [i32; 2]) -> *mut c_void {
    // SAFETY: R7 passes its own copy of the pair
    let [index, _] = unsafe { *weak };
    if index == 0 {
        std::ptr::null_mut()
    } else {
        (0x1000 + index as usize) as *mut c_void
    }
}

unsafe extern "C" fn fake_complete(_component: *mut c_void) {}

fn fakes() {
    static SET: Once = Once::new();
    SET.call_once(|| {
        game::set_calls(
            fake_begin as *const () as usize,
            fake_weak as *const () as usize,
            fake_complete as *const () as usize,
        );
    });
}

/// A leaked, zeroed, 8-aligned block of `size` bytes.
fn block(size: usize) -> usize {
    Box::leak(vec![0u64; size.div_ceil(8)].into_boxed_slice()).as_mut_ptr() as usize
}

fn put<T: Copy>(addr: usize, value: T) {
    // SAFETY: test blocks are leaked and large enough for every field the tests write
    unsafe { std::ptr::write_unaligned(addr as *mut T, value) };
}

fn get_u8(addr: usize) -> u8 {
    // SAFETY: as above
    unsafe { std::ptr::read_unaligned(addr as *const u8) }
}

struct Player {
    controller: usize,
    manager: usize,
    stuck_loader: usize,
}

/// A controller with a player state (net id `net_id`), a hunt system whose id `Num` is `hunt_num`, and a load
/// manager with two entries: a loader finished (`1/0`) and one stuck loading (`0/1`).
fn player(net_id: usize, hunt_num: i32) -> Player {
    let controller = block(r7::PC_SPAN_MANAGER);
    let state = block(0x260);
    let hunt = block(r7::HUNT_SYSTEM_SPAN);
    let manager = block(r7::MANAGER_SPAN);
    let entries = block(2 * r7::ENTRY_SIZE);
    let done = block(r7::LOADER_SPAN);
    let stuck = block(r7::LOADER_SPAN);
    put(controller + r7::PC_PLAYER_STATE, state);
    put(controller + r7::PC_HUNT_SYSTEM, hunt);
    put(controller + r7::PC_LOAD_MANAGER, manager);
    put(state + r7::PS_NET_ID, net_id);
    put(hunt + r7::HUNT_ID_NUM, hunt_num);
    put(manager + r7::MANAGER_CONTROLLER, controller);
    put(manager + r7::MANAGER_ENTRIES, entries);
    put(manager + r7::MANAGER_ENTRY_COUNT, 2i32);
    put(manager + r7::MANAGER_BEGUN, 1u8);
    put(entries, [5i32, 1]);
    put(entries + r7::ENTRY_LOADER, done);
    put(entries + r7::ENTRY_SIZE, [6i32, 1]);
    put(entries + r7::ENTRY_SIZE + r7::ENTRY_LOADER, stuck);
    put(done + r7::LOADER_LOADED, 1u8);
    put(stuck + r7::LOADER_LOADING, 1u8);
    fake::object(controller, &["BP_PlayerController_C", "ArchonPlayerController"]);
    Player {
        controller,
        manager,
        stuck_loader: stuck,
    }
}

fn counted(calls: &Cell<u32>) -> impl FnMut() + '_ {
    move || calls.set(calls.get() + 1)
}

#[test]
fn load_failure_retries_once() {
    fakes();
    let p = player(0x77, 2);
    let calls = Cell::new(0);
    load_failed(p.manager, &mut counted(&calls));
    assert_eq!(calls.get(), 0, "a retry does not reach the game's handler");
    assert_eq!(BEGINS.with(Cell::get), 1);
    assert_eq!(get_u8(p.stuck_loader + r7::LOADER_LOADING), 0, "bLoading cleared");
    assert_eq!(get_u8(p.manager + r7::MANAGER_BEGUN), 0, "bBegun cleared");

    // The reload sticks again: the second failure goes to the game
    put(p.stuck_loader + r7::LOADER_LOADING, 1u8);
    load_failed(p.manager, &mut counted(&calls));
    assert_eq!(calls.get(), 1);
    assert_eq!(BEGINS.with(Cell::get), 1);

    // LD-2 clears the slot and marks the gate before the original runs; Num 2 is already set
    let marked = Cell::new(false);
    let backfill = data_load_complete(p.controller, 1, &mut || {
        marked.set(restart_gate::loaded(p.controller));
    });
    assert!(marked.get(), "the gate knew before next ran");
    assert_eq!(backfill, Some(Backfill::AlreadySet));
    load_failed(p.manager, &mut counted(&calls));
    assert_eq!(
        (calls.get(), BEGINS.with(Cell::get)),
        (1, 2),
        "a fresh budget after the load completed"
    );

    // The flag path: the game handles it
    failed(p.manager, || true, &mut counted(&calls));
    assert_eq!((calls.get(), BEGINS.with(Cell::get)), (2, 2));

    // Unreadable manager
    load_failed(0, &mut counted(&calls));
    assert_eq!(calls.get(), 3);
}

#[test]
fn load_failure_without_net_id_is_dropped() {
    fakes();
    let p = player(0, 0);
    let calls = Cell::new(0);
    load_failed(p.manager, &mut counted(&calls));
    assert_eq!(calls.get(), 0, "dropped without next");
    assert_eq!(BEGINS.with(Cell::get), 0, "and without a retry");
}

#[test]
fn not_recoverable_goes_to_the_game() {
    fakes();
    let p = player(0x77, 0);
    // A loader byte of 2 is not a bool
    put(p.stuck_loader + r7::LOADER_LOADING, 2u8);
    let calls = Cell::new(0);
    load_failed(p.manager, &mut counted(&calls));
    assert_eq!((calls.get(), BEGINS.with(Cell::get)), (1, 0));
    // A dead controller
    put(p.stuck_loader + r7::LOADER_LOADING, 1u8);
    fake::clear();
    load_failed(p.manager, &mut counted(&calls));
    assert_eq!((calls.get(), BEGINS.with(Cell::get)), (2, 0));
}

#[test]
fn backfill_and_failed_loads() {
    fakes();
    let p = player(0x77, 1);
    let calls = Cell::new(0);
    assert_eq!(
        data_load_complete(p.controller, 0, &mut counted(&calls)),
        None,
        "no backfill on failure"
    );
    assert_eq!(calls.get(), 1);
    // An empty id and a launch naming none: not set
    assert_eq!(super::player_data::backfill(p.controller, ""), Backfill::NoHuntId);
    assert_eq!(super::player_data::backfill(0, "h1"), Backfill::Unreadable);
    // No engine in a test: the function is not found
    assert_eq!(super::player_data::backfill(p.controller, "h1"), Backfill::NoFunction);
}

/// A game mode of `class`, a controller with `net` at `NetConnection`, and params `{controller, ReturnValue = 1}`.
fn restart(class: &'static str, net: usize) -> (usize, usize, usize) {
    let game_mode = block(0x10);
    fake::object(game_mode, &[class, "GameModeBase"]);
    let controller = block(r7::PC_SPAN_GATE);
    put(controller + r7::PC_NET_CONNECTION, net);
    fake::object(controller, &["ArchonPlayerController"]);
    let params = block(r7::CAN_RESTART_PARAMS);
    put(params, controller);
    put(params + r7::CAN_RESTART_RETURN, 1u8);
    (game_mode, controller, params)
}

#[test]
fn restart_gate_holds_a_hunt_player() {
    let (game_mode, controller, params) = restart("BPGM_Hunt_Pangar_C", 0x99);
    restart_gate::apply(game_mode, params, true);
    assert_eq!(
        get_u8(params + r7::CAN_RESTART_RETURN),
        0,
        "held after the chain answered true"
    );
    put(params + r7::CAN_RESTART_RETURN, 1u8);
    restart_gate::mark_loaded(controller);
    restart_gate::apply(game_mode, params, true);
    assert_eq!(
        get_u8(params + r7::CAN_RESTART_RETURN),
        1,
        "released once the data loaded"
    );

    let (game_mode, _, params) = restart("BPGM_City_C", 0x99);
    restart_gate::apply(game_mode, params, true);
    assert_eq!(get_u8(params + r7::CAN_RESTART_RETURN), 1, "a hub is not held");

    let (game_mode, _, params) = restart("BPGM_Hunt_Pangar_C", 0);
    restart_gate::apply(game_mode, params, true);
    assert_eq!(
        get_u8(params + r7::CAN_RESTART_RETURN),
        1,
        "a local controller is not held"
    );

    let (game_mode, _, params) = restart("BPGM_Hunt_Pangar_C", 0x99);
    restart_gate::apply(game_mode, params, false);
    assert_eq!(
        get_u8(params + r7::CAN_RESTART_RETURN),
        1,
        "not a gameserver, or the flag"
    );
    assert!(!restart_gate::hold(game_mode, 0), "a client never holds");
}

#[test]
fn a_read_that_hears_not_loaded_parks() {
    let interface = block(0x10) + 0x10_0000;
    let component = interface - r7::PROGRESSION_LOADER;
    read_from_backend(interface, &mut || {
        assert_eq!(predicate(0, &mut || 0), 0);
    });
    assert_eq!(bookkeeping(component), (true, false), "parked");

    let other = interface + 0x100;
    read_from_backend(other, &mut || {});
    assert_eq!(
        bookkeeping(other - r7::PROGRESSION_LOADER),
        (false, false),
        "never asked"
    );
    read_from_backend(other, &mut || {
        assert_eq!(predicate(0, &mut || 1), 1);
    });
    assert_eq!(bookkeeping(other - r7::PROGRESSION_LOADER), (false, false), "loaded");

    super::progression::complete_load(component, &mut || {});
    assert_eq!(bookkeeping(component), (false, true), "completed");
}

#[test]
fn readiness_answers() {
    let calls = &Cell::new(0);
    let answer = |value: u8| {
        move || {
            calls.set(calls.get() + 1);
            value
        }
    };
    assert_eq!(
        has_finished_loading(0x10, false, &mut answer(1)),
        1,
        "a true answer is kept"
    );
    assert_eq!(has_finished_loading(0x10, true, &mut answer(0)), 1, "the server forces");
    assert_eq!(has_finished_loading(0, false, &mut answer(0)), 0);
    // An idle trigger with no captured possession stays false
    let trigger = block(r7::TRIGGER_SPAN);
    fake::object(trigger, &["island_arrival_cinematic_trigger_bp_C"]);
    assert_eq!(has_finished_loading(trigger, false, &mut answer(0)), 0);
    assert_eq!(calls.get(), 4, "the original runs first, every time");
}

// --- Every wrapper through the broker on a test function ---

static ORIGINALS: AtomicU32 = AtomicU32::new(0);

#[inline(never)]
extern "C" fn target_u8_a(object: *mut c_void) -> u8 {
    ORIGINALS.fetch_add(1, Ordering::SeqCst);
    u8::from(black_box(object as usize) == 1)
}

#[inline(never)]
extern "C" fn target_flag(_controller: *mut c_void, _success: u8) {
    ORIGINALS.fetch_add(black_box(10), Ordering::SeqCst);
}

#[inline(never)]
extern "C" fn target_a(_object: *mut c_void) {
    ORIGINALS.fetch_add(black_box(100), Ordering::SeqCst);
}

#[inline(never)]
extern "C" fn target_u8_b(_object: *mut c_void) -> u8 {
    ORIGINALS.fetch_add(1000, Ordering::SeqCst);
    black_box(7)
}

#[inline(never)]
extern "C" fn target_b(_object: *mut c_void) {
    ORIGINALS.fetch_add(black_box(10_000), Ordering::SeqCst);
}

#[inline(never)]
extern "C" fn target_c(_object: *mut c_void) {
    ORIGINALS.fetch_add(black_box(100_000), Ordering::SeqCst);
}

#[test]
fn every_wrapper_through_the_broker() {
    let targets = [
        target_u8_a as *const () as usize,
        target_flag as *const () as usize,
        target_a as *const () as usize,
        target_u8_b as *const () as usize,
        target_b as *const () as usize,
        target_c as *const () as usize,
    ];
    for ((wrapper, next), target) in super::wrappers().into_iter().zip(targets) {
        broker::hook(target, wrapper, 0, next).unwrap();
    }
    let null = std::ptr::null_mut::<c_void>();
    // SAFETY: each address is a test function of the signature it is called with, now hooked
    unsafe {
        let u8_a = std::mem::transmute::<usize, extern "C" fn(*mut c_void) -> u8>(black_box(targets[0]));
        assert_eq!(u8_a(std::ptr::dangling_mut::<c_void>()), 1);
        assert_eq!(u8_a(null), 0, "a client never forces a null participant");
        std::mem::transmute::<usize, extern "C" fn(*mut c_void, u8)>(black_box(targets[1]))(null, 0);
        std::mem::transmute::<usize, extern "C" fn(*mut c_void)>(black_box(targets[2]))(null);
        let u8_b = std::mem::transmute::<usize, extern "C" fn(*mut c_void) -> u8>(black_box(targets[3]));
        assert_eq!(u8_b(null), 7, "the predicate's answer passes through");
        std::mem::transmute::<usize, extern "C" fn(*mut c_void)>(black_box(targets[4]))(null);
        std::mem::transmute::<usize, extern "C" fn(*mut c_void)>(black_box(targets[5]))(null);
    }
    assert_eq!(ORIGINALS.load(Ordering::SeqCst), 2 + 10 + 100 + 1000 + 10_000 + 100_000);
}
