//! R14's DLL tests (16.1) on test-allocated memory and test functions; the game's RVAs are never called.

use super::apply::{self, ApplyFn};
use super::server::remove_with;
use super::{call_try_activate, with_state};
use crate::ffi::broker;
use crate::ffi::dispatch::{self, PePoint, pe_dispatch};
use rustic_isles_runtime_core::player_role::RoleEntry;
use std::ffi::c_void;
use std::hint::black_box;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

fn noted(loadout: usize) -> bool {
    with_state(|state| {
        state
            .retry
            .borrow()
            .iter()
            .flatten()
            .any(|entry| entry.loadout == loadout)
    })
    .unwrap_or(false)
}

static NOTED_INSIDE: AtomicBool = AtomicBool::new(true);
static APPLIED: AtomicUsize = AtomicUsize::new(0);

#[inline(never)]
extern "C" fn test_apply(loadout: *mut c_void) {
    APPLIED.store(black_box(loadout) as usize, Ordering::SeqCst);
    NOTED_INSIDE.store(noted(loadout as usize), Ordering::SeqCst);
}

#[test]
fn pr1_runs_next_first_then_notes_validity() {
    // A loadout whose slot is not resolved (no game getter in a test): noted, not applied
    let mut loadout = vec![0u64; 0x700 / 8];
    let at = loadout.as_mut_ptr();
    let target = test_apply as *const () as usize;
    broker::hook(target, apply::DETOUR as usize, 0, &apply::NEXT).unwrap();
    // SAFETY: the test function's own signature
    let call = unsafe { std::mem::transmute::<usize, ApplyFn>(target) };
    // SAFETY: a test buffer as the loadout
    unsafe { black_box(call)(at.cast()) };
    assert_eq!(APPLIED.load(Ordering::SeqCst), at as usize, "next ran");
    assert!(!NOTED_INSIDE.load(Ordering::SeqCst), "not noted before next");
    assert!(noted(at as usize), "noted after it");
}

static REMOVED: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

unsafe extern "C" fn remove_next(graph: *mut c_void, info: *mut c_void) {
    if let Ok(mut seen) = REMOVED.lock() {
        seen.push((graph as usize, info as usize));
    }
}

#[test]
fn rg3_passes_an_actor_that_is_not_a_role() {
    let mut actor = vec![0u64; 0x60 / 8];
    let mut info = [actor.as_mut_ptr() as u64];
    let actor_at = actor.as_ptr() as usize;
    with_state(|state| {
        state.roles.borrow_mut().push(RoleEntry {
            role: actor_at,
            ..RoleEntry::default()
        });
    });
    let graph = 0x4321_0000usize as *mut c_void;
    remove_with(Some(remove_next), graph, info.as_mut_ptr().cast());
    let seen = REMOVED.lock().expect("lock").clone();
    assert_eq!(seen, [(graph as usize, info.as_ptr() as usize)]);
    assert_eq!(
        with_state(|state| state.roles.borrow().len()),
        Some(1),
        "nothing purged"
    );
}

/// component, handle, key, instanced, on_ended, event.
type Args = (usize, u32, usize, usize, usize, usize);

static ACTIVATE_ARGS: Mutex<Vec<Args>> = Mutex::new(Vec::new());

extern "C" fn fake_try_activate(
    component: *mut c_void,
    handle: u32,
    key: *const [u8; 16],
    instanced: *mut usize,
    on_ended: *const c_void,
    event: *const c_void,
) -> u8 {
    if let Ok(mut args) = ACTIVATE_ARGS.lock() {
        args.push((
            component as usize,
            handle,
            key as usize,
            instanced as usize,
            on_ended as usize,
            event as usize,
        ));
    }
    1
}

#[test]
fn the_ability_call_order_and_its_null_on_ended() {
    let mut key = [0xAB; 16];
    let key_at = key.as_ptr() as usize;
    let function = fake_try_activate as *const () as usize;
    assert!(call_try_activate(function, 0x1000, 7, &mut key, 0x2000));
    let args = ACTIVATE_ARGS.lock().expect("lock").clone();
    assert_eq!(args.len(), 1);
    let (component, handle, key_ptr, instanced, on_ended, event) = args[0];
    assert_eq!(
        (component, handle, key_ptr, on_ended, event),
        (0x1000, 7, key_at, 0, 0x2000)
    );
    assert_ne!(instanced, 0, "R14's own slot");
    assert!(!call_try_activate(0, 0x1000, 7, &mut key, 0));
}

static DEPTH_POINT: PePoint = PePoint::new("TEST-R14", None, false);
static DEPTHS: Mutex<Vec<u32>> = Mutex::new(Vec::new());

type PeFn = extern "C" fn(*mut c_void, *mut c_void, *mut c_void);

#[inline(never)]
extern "C" fn nesting(object: *mut c_void, function: *mut c_void, params: *mut c_void) {
    let depth = dispatch::depth();
    if let Ok(mut depths) = DEPTHS.lock() {
        depths.push(depth);
    }
    if depth == 1 {
        black_box(nesting as PeFn)(object, function, params);
    }
}

unsafe extern "C" fn nesting_detour(object: *mut c_void, function: *mut c_void, params: *mut c_void) {
    pe_dispatch(&DEPTH_POINT, object, function, params);
}

#[test]
fn depth_is_one_outermost_and_two_nested() {
    let target = nesting as *const () as usize;
    broker::hook(target, nesting_detour as *const () as usize, 0, &DEPTH_POINT.next).unwrap();
    let null = std::ptr::null_mut();
    assert_eq!(dispatch::depth(), 0);
    black_box(nesting as PeFn)(null, null, null);
    assert_eq!(*DEPTHS.lock().expect("lock"), [1, 2]);
    assert_eq!(dispatch::depth(), 0);
}
