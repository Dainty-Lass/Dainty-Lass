//! R11's DLL tests (17.1) on test-allocated memory; the manual tick and NETD-3 need the game and are not run here.

use super::{netmode, sanitize, spline};
use crate::ffi::broker;
use std::ffi::c_void;
use std::hint::black_box;
use std::sync::atomic::{AtomicU32, Ordering};
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_NOACCESS, PAGE_PROTECTION_FLAGS, PAGE_READWRITE, VirtualAlloc,
    VirtualFree,
};

const PAGE: usize = 4096;

struct Page(usize);

impl Page {
    fn new(protect: PAGE_PROTECTION_FLAGS) -> Self {
        let base = unsafe { VirtualAlloc(std::ptr::null(), PAGE, MEM_COMMIT | MEM_RESERVE, protect) };
        assert!(!base.is_null());
        Self(base as usize)
    }
    fn put<T>(&self, offset: usize, value: T) {
        unsafe { std::ptr::write_unaligned((self.0 + offset) as *mut T, value) };
    }
    fn get<T: Copy>(&self, offset: usize) -> T {
        unsafe { std::ptr::read_unaligned((self.0 + offset) as *const T) }
    }
}

impl Drop for Page {
    fn drop(&mut self) {
        unsafe { VirtualFree(self.0 as *mut _, 0, MEM_RELEASE) };
    }
}

#[test]
fn net_mode_answers_one_in_the_whole_register() {
    let garbage = 0xEE47_C001usize as *mut c_void;
    assert_eq!(unsafe { netmode::internal_detour(garbage) }, 1);
    assert_eq!(unsafe { netmode::world_detour(garbage) }, 1);
    // Read back as 64 bits: `mov eax, 1` clears the upper half, a byte answer would keep the pointer's bits
    let wide = |function: usize| {
        let function = unsafe { std::mem::transmute::<usize, unsafe extern "C" fn(*mut c_void) -> u64>(function) };
        unsafe { function(garbage) }
    };
    assert_eq!(wide(netmode::internal_detour as *const () as usize), 1);
    assert_eq!(wide(netmode::world_detour as *const () as usize), 1);
}

#[test]
fn sanitizer_compacts_a_no_access_connection() {
    let driver = Page::new(PAGE_READWRITE);
    let good = Page::new(PAGE_READWRITE);
    let bad = Page::new(PAGE_NOACCESS);
    let data = driver.0 + 0x400;
    driver.put(0x90, data);
    driver.put(0x98, 2i32);
    driver.put(0x9C, 4i32);
    driver.put(0x400, bad.0);
    driver.put(0x408, good.0);
    assert!(!sanitize(driver.0, "test-compact"));
    assert_eq!(driver.get::<usize>(0x400), good.0);
    assert_eq!(driver.get::<usize>(0x408), 0);
    assert_eq!(driver.get::<i32>(0x98), 1);
    assert_eq!(driver.get::<i32>(0x9C), 4);
    assert!(sanitize(driver.0, "test-compact"), "safe on the next frame");
    assert!(!sanitize(bad.0, "test-compact"), "an unreadable driver");
}

static QUERY_CALLS: AtomicU32 = AtomicU32::new(0);

#[inline(never)]
extern "C" fn fake_query(component: *mut c_void, out: *mut f32, input: f32, _apply: i32) -> *mut f32 {
    QUERY_CALLS.fetch_add(1, Ordering::SeqCst);
    black_box(component);
    unsafe {
        *out = black_box(input);
        *out.add(1) = 1.0;
        *out.add(2) = 2.0;
    }
    out
}

#[test]
fn spline_guard_calls_next_or_answers_zero() {
    let target = fake_query as *const () as usize;
    broker::hook(target, spline::detour as *const () as usize, 0, &spline::NEXT).unwrap();
    let query = unsafe {
        std::mem::transmute::<usize, extern "C" fn(*mut c_void, *mut f32, f32, i32) -> *mut f32>(black_box(target))
    };
    let component = Page::new(PAGE_READWRITE);
    let points = Page::new(PAGE_READWRITE);
    component.put(0x448, points.0);
    component.put(0x450, 4i32);
    let mut out = [9.0f32; 3];
    let answer = query(component.0 as *mut c_void, out.as_mut_ptr(), 5.0, 0);
    assert_eq!(answer, out.as_mut_ptr());
    assert_eq!(out, [5.0, 1.0, 2.0]);
    assert_eq!(QUERY_CALLS.load(Ordering::SeqCst), 1);
    // a curve whose points sit on a no-access page
    let locked = Page::new(PAGE_NOACCESS);
    component.put(0x448, locked.0);
    let mut out = [9.0f32; 3];
    let answer = query(component.0 as *mut c_void, out.as_mut_ptr(), 5.0, 0);
    assert_eq!(answer, out.as_mut_ptr());
    assert_eq!(out, [0.0, 0.0, 0.0]);
    assert_eq!(QUERY_CALLS.load(Ordering::SeqCst), 1, "the game's query was not called");
    assert!(!spline::usable(0), "a null component");
}
