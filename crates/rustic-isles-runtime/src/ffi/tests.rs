//! The broker and the depth guard on real code in the test process (R0 10.1). Hooks are never removed, so each
//! test hooks its own function.

use super::broker::{self, NextCell};
use super::dispatch::{PePoint, pe_dispatch};
use std::cell::RefCell;
use std::ffi::c_void;
use std::hint::black_box;
use std::sync::atomic::{AtomicU32, Ordering};

type AddFn = extern "C" fn(u64, u64) -> u64;

thread_local! {
    static TRACE: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
}

fn trace(step: &'static str) {
    TRACE.with(|trace| trace.borrow_mut().push(step));
}

fn take_trace() -> Vec<&'static str> {
    TRACE.with(|trace| std::mem::take(&mut *trace.borrow_mut()))
}

fn next_fn(cell: usize) -> AddFn {
    assert_ne!(cell, 0);
    // SAFETY: the broker wrote a function of this signature
    unsafe { std::mem::transmute::<usize, AddFn>(cell) }
}

#[inline(never)]
extern "C" fn target_add(a: u64, b: u64) -> u64 {
    trace("original");
    black_box(a).wrapping_add(black_box(b).wrapping_mul(2))
}

static OUTER_NEXT: NextCell = NextCell::new();
static INNER_NEXT: NextCell = NextCell::new();
static ABSORB_NEXT: NextCell = NextCell::new();

extern "C" fn mit_outer(a: u64, b: u64) -> u64 {
    trace("outer");
    next_fn(OUTER_NEXT.load())(a + 1, b) + 100
}

extern "C" fn mit_inner(a: u64, b: u64) -> u64 {
    trace("inner");
    next_fn(INNER_NEXT.load())(a, b + 1) + 1000
}

extern "C" fn absorber(_: u64, _: u64) -> u64 {
    trace("absorb");
    7
}

fn call(target: usize, a: u64, b: u64) -> u64 {
    // Through the target's address, so the patched entry runs
    black_box(next_fn(target))(a, b)
}

fn address(function: AddFn) -> usize {
    function as *const () as usize
}

#[test]
fn broker_chain_on_real_code() {
    let target = address(target_add);
    assert_eq!(call(target, 1, 1), 3);
    assert_eq!(take_trace(), ["original"]);
    assert_eq!(broker::original(target), target, "unhooked: the target itself");

    broker::hook(target, address(mit_inner), 5, &INNER_NEXT).unwrap();
    broker::hook(target, address(mit_outer), -1, &OUTER_NEXT).unwrap();
    assert_eq!(
        broker::hook(target, address(mit_outer), 0, &OUTER_NEXT),
        Err(3),
        "a wrapper is chained once"
    );

    // outer: a=2 → inner: b=2 → original 2+4=6, then +1000, +100
    assert_eq!(call(target, 1, 1), 1_106);
    assert_eq!(take_trace(), ["outer", "inner", "original"]);

    let original = next_fn(broker::original(target));
    assert_eq!(original(1, 1), 3);
    assert_eq!(take_trace(), ["original"], "original() runs past every wrapper");

    broker::hook(target, address(absorber), -10, &ABSORB_NEXT).unwrap();
    assert_eq!(call(target, 1, 1), 7);
    assert_eq!(take_trace(), ["absorb"], "an absorbing wrapper stops the chain");
    broker::unhook(target, address(absorber)).unwrap();
    assert_eq!(broker::unhook(target, address(absorber)), Err(4));
    assert_eq!(call(target, 1, 1), 1_106);
    assert_eq!(take_trace(), ["outer", "inner", "original"]);
}

#[test]
fn original_of_an_unhooked_target_is_itself() {
    assert_eq!(broker::original(0x1234_5678), 0x1234_5678);
    assert_eq!(broker::unhook(0x1234_5678, 0x1234_5678), Err(4));
}

static DEPTH_POINT: PePoint = PePoint::new("TEST-PE", None, false);
static BODY_RUNS: AtomicU32 = AtomicU32::new(0);

type PeFn = extern "C" fn(*mut c_void, *mut c_void, *mut c_void);

#[inline(never)]
extern "C" fn recursing(object: *mut c_void, function: *mut c_void, params: *mut c_void) {
    if BODY_RUNS.fetch_add(1, Ordering::SeqCst) + 1 < 40 {
        black_box(recursing as PeFn)(object, function, params);
    }
}

unsafe extern "C" fn depth_detour(object: *mut c_void, function: *mut c_void, params: *mut c_void) {
    pe_dispatch(&DEPTH_POINT, object, function, params);
}

#[test]
fn depth_guard_absorbs_from_33() {
    let target = recursing as *const () as usize;
    broker::hook(target, depth_detour as *const () as usize, 0, &DEPTH_POINT.next).unwrap();
    let null = std::ptr::null_mut();
    for _ in 0..2 {
        BODY_RUNS.store(0, Ordering::SeqCst);
        black_box(recursing as PeFn)(null, null, null);
        assert_eq!(
            BODY_RUNS.load(Ordering::SeqCst),
            32,
            "depths 1 to 32 ran; 33 was absorbed"
        );
    }
}
