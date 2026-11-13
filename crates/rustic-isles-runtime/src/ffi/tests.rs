//! The broker and the depth guard on real code in the test process (R0 10.1). Hooks are never removed, so each
//! test hooks its own function.

use super::broker::{self, NextCell};
use super::dispatch::{PePoint, pe_dispatch};
use super::exports::{rir_chain_add_inner, rir_chain_remove_inner, rir_original, rir_owns, rir_seam_version};
use std::cell::RefCell;
use std::ffi::c_void;
use std::hint::black_box;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

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
static AGPL_NEXT: AtomicUsize = AtomicUsize::new(0);

extern "C" fn mit_outer(a: u64, b: u64) -> u64 {
    trace("outer");
    next_fn(OUTER_NEXT.load())(a + 1, b) + 100
}

extern "C" fn mit_inner(a: u64, b: u64) -> u64 {
    trace("inner");
    next_fn(INNER_NEXT.load())(a, b + 1) + 1000
}

extern "C" fn agpl_wrapper(a: u64, b: u64) -> u64 {
    trace("agpl");
    next_fn(AGPL_NEXT.load(Ordering::Acquire))(a, b) + 10_000
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

    broker::hook(target, address(mit_inner), 5, &INNER_NEXT).unwrap();
    broker::hook(target, address(mit_outer), -1, &OUTER_NEXT).unwrap();
    let mut next: *mut c_void = std::ptr::null_mut();
    let id = c"TEST-1";
    // SAFETY: valid pointers for the export
    let status = unsafe {
        rir_chain_add_inner(
            target as *mut c_void,
            address(agpl_wrapper) as *mut c_void,
            id.as_ptr(),
            &mut next,
        )
    };
    assert_eq!(status, 0);
    AGPL_NEXT.store(next as usize, Ordering::Release);
    let mut second: *mut c_void = std::ptr::null_mut();
    // SAFETY: as above
    let again = unsafe {
        rir_chain_add_inner(
            target as *mut c_void,
            address(absorber) as *mut c_void,
            id.as_ptr(),
            &mut second,
        )
    };
    assert_eq!(again, 3, "the AGPL band holds one wrapper");
    assert_eq!(
        rir_original(target as *mut c_void),
        next,
        "the AGPL's next is the trampoline"
    );

    // outer: a=2 → inner: b=2 → agpl → original 2+4=6, then +10000, +1000, +100
    assert_eq!(call(target, 1, 1), 11_106);
    assert_eq!(take_trace(), ["outer", "inner", "agpl", "original"]);

    assert_eq!(
        rir_chain_remove_inner(target as *mut c_void, address(agpl_wrapper) as *mut c_void),
        0
    );
    assert_eq!(
        rir_chain_remove_inner(target as *mut c_void, address(agpl_wrapper) as *mut c_void),
        4
    );
    assert_eq!(call(target, 1, 1), 1_106);
    assert_eq!(take_trace(), ["outer", "inner", "original"]);

    let original = next_fn(rir_original(target as *mut c_void) as usize);
    assert_eq!(original(1, 1), 3);
    assert_eq!(take_trace(), ["original"], "original() runs past every wrapper");

    broker::hook(target, address(absorber), -10, &ABSORB_NEXT).unwrap();
    assert_eq!(call(target, 1, 1), 7);
    assert_eq!(take_trace(), ["absorb"], "an absorbing wrapper stops the chain");
    broker::unhook(target, address(absorber)).unwrap();
    assert_eq!(call(target, 1, 1), 1_106);
    assert_eq!(take_trace(), ["outer", "inner", "original"]);
}

#[test]
fn original_of_an_unhooked_target_is_itself() {
    let unhooked = 0x1234_5678usize as *mut c_void;
    assert_eq!(rir_original(unhooked), unhooked);
    assert_eq!(rir_chain_remove_inner(unhooked, unhooked), 4);
}

#[test]
fn seam_answers() {
    assert_eq!(rir_seam_version(), 1);
    // SAFETY: NUL-terminated strings or null
    unsafe {
        assert_eq!(rir_owns(c"SP-1".as_ptr()), 1);
        assert_eq!(rir_owns(c"SP-3".as_ptr()), 1);
        assert_eq!(rir_owns(c"HTTP-1".as_ptr()), 1);
        assert_eq!(rir_owns(c"HTTP-1@R2".as_ptr()), 1);
        assert_eq!(rir_owns(c"HTTP-1@R10".as_ptr()), 0);
        assert_eq!(rir_owns(c"SP-2@R20".as_ptr()), 0);
        assert_eq!(rir_owns(c"".as_ptr()), 0);
        assert_eq!(rir_owns(std::ptr::null()), 0);
        let long = std::ffi::CString::new("S".repeat(48)).unwrap();
        assert_eq!(rir_owns(long.as_ptr()), 0);
    }
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
