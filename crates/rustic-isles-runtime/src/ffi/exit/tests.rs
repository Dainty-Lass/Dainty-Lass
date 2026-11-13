//! R3 on real code in the test process (spec-runtime-r3 10.1). The real exit exports are never hooked here; EXIT-5
//! is hooked only in a child process.

use super::exports::{self, terminate_detour};
use super::veh::{self, CONTEXT_SIZE, RECORD_SIZE, arm_at, context};
use crate::ffi::broker;
use crate::ffi::memory::{patch_bytes_at, read_bytes};
use rustic_isles_runtime_core::offsets::exit::{REPORT_CRASH_WAIT_BYTES, REPORT_CRASH_WAIT_PATCHED};
use rustic_isles_runtime_core::patch::PatchResult;
use std::cell::{Cell, RefCell};
use std::hint::black_box;
use std::mem::{offset_of, size_of};
use std::sync::atomic::{AtomicUsize, Ordering};
use windows_sys::Win32::System::Diagnostics::Debug::{
    AddVectoredExceptionHandler, CONTEXT, EXCEPTION_CONTINUE_EXECUTION, EXCEPTION_CONTINUE_SEARCH, EXCEPTION_POINTERS,
    EXCEPTION_RECORD, RaiseException, RemoveVectoredExceptionHandler,
};
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_EXECUTE_READ, PAGE_PROTECTION_FLAGS, PAGE_READWRITE, VirtualAlloc,
    VirtualFree, VirtualProtect,
};

thread_local! {
    static CAPTURED: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static DETACHING: Cell<bool> = const { Cell::new(false) };
    static DURING_BODY: Cell<Option<fn()>> = const { Cell::new(None) };
}

pub(super) fn capture(line: &str) {
    let _ = CAPTURED.try_with(|lines| lines.borrow_mut().push(line.to_owned()));
}

fn take_captured() -> Vec<String> {
    CAPTURED.with(|lines| std::mem::take(&mut *lines.borrow_mut()))
}

pub(super) fn detaching_override() -> bool {
    DETACHING.get()
}

pub(super) fn during_body() {
    if let Some(body) = DURING_BODY.get() {
        body();
    }
}

type TerminateFn = extern "system" fn(isize, u32) -> i32;

thread_local! {
    static CALLS: RefCell<Vec<(isize, u32)>> = const { RefCell::new(Vec::new()) };
}

#[inline(never)]
extern "system" fn stand_in(handle: isize, code: u32) -> i32 {
    CALLS.with(|calls| calls.borrow_mut().push((handle, code)));
    black_box(code as i32).wrapping_add(1000)
}

fn call_stand_in(handle: isize, code: u32) -> i32 {
    black_box(stand_in as TerminateFn)(handle, code)
}

fn take_calls() -> Vec<(isize, u32)> {
    CALLS.with(|calls| std::mem::take(&mut *calls.borrow_mut()))
}

#[test]
fn export_detour_on_a_stand_in() {
    let (detour, next) = terminate_detour();
    broker::hook(stand_in as *const () as usize, detour, 0, next).unwrap();
    take_captured();

    assert_eq!(call_stand_in(0x1F4, 7), 1007, "the return value comes back");
    assert_eq!(take_calls(), [(0x1F4, 7)], "the arguments go on unchanged");
    let lines = take_captured();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(
        lines[0].starts_with("[MitExit] kernelbase!TerminateProcess handle=0x1F4 code=7 tid="),
        "{}",
        lines[0]
    );
    assert!(lines[0].contains(" gameTid=0 isGameThread=0 retAddr="), "{}", lines[0]);
    assert!(
        lines[1].starts_with("[MitExit] kernelbase!TerminateProcess code=7 stack="),
        "{}",
        lines[1]
    );

    DETACHING.set(true);
    assert_eq!(call_stand_in(-1, 3), 1003);
    DETACHING.set(false);
    assert_eq!(take_calls(), [(-1, 3)]);
    assert!(take_captured().is_empty(), "nothing is written while detaching");

    DURING_BODY.set(Some(|| {
        assert_eq!(call_stand_in(0, 9), 1009);
    }));
    assert_eq!(call_stand_in(-1, 1), 1001);
    DURING_BODY.set(None);
    assert_eq!(take_calls(), [(0, 9), (-1, 1)], "the nested call forwarded");
    let lines = take_captured();
    assert_eq!(lines.len(), 2, "only the outer call logged: {lines:?}");
    assert!(lines[0].starts_with("[MitExit] kernelbase!TerminateProcess handle=self code=1 "));
}

const CHILD: &str = "RUSTIC_ISLES_R3_RAISE_CHILD";
const MARKER: usize = 0x5233_0005;

unsafe extern "system" fn child_handler(info: *mut EXCEPTION_POINTERS) -> i32 {
    // SAFETY: the dispatcher passes valid pointers
    let record = unsafe { &*(*info).ExceptionRecord };
    let code = record.ExceptionCode as u32;
    let ours = code == 0xE000_0001
        || (code == 0xE06D_7363 && record.NumberParameters == 1 && record.ExceptionInformation[0] == MARKER);
    if ours {
        EXCEPTION_CONTINUE_EXECUTION
    } else {
        EXCEPTION_CONTINUE_SEARCH
    }
}

fn raise_child() {
    // The test binary is "this DLL" here, so every frame up to the thread start is trimmed
    super::prepare(0);
    exports::install_one(4).unwrap();
    // SAFETY: a static handler; both raises are continuable and answered by it
    unsafe {
        AddVectoredExceptionHandler(1, Some(child_handler));
        RaiseException(0xE000_0001, 0, 2, [0x11usize, 0x22].as_ptr());
        RaiseException(0xE06D_7363, 0, 1, [MARKER].as_ptr());
    }
    for line in take_captured() {
        println!("R3LINE {line}");
    }
}

/// EXIT-5 on the real export, hooked only in a child run of this binary (R3 10.1).
#[test]
fn raise_exception_in_a_child() {
    if std::env::var_os(CHILD).is_some() {
        raise_child();
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "ffi::exit::tests::raise_exception_in_a_child",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD, "1")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "child failed: {stdout}");
    let lines: Vec<&str> = stdout
        .lines()
        .filter_map(|line| line.split_once("R3LINE ").map(|(_, line)| line))
        .collect();
    assert_eq!(lines.len(), 2, "{lines:?}");
    assert!(
        lines[0].starts_with("[MitException] kernelbase!RaiseException code=0xE0000001 flags=0x0 address="),
        "{}",
        lines[0]
    );
    assert!(lines[0].ends_with(" params=0x11,0x22"), "{}", lines[0]);
    assert!(
        lines[1].starts_with("[MitExit] kernelbase!RaiseException code=3758096385 stack="),
        "{}",
        lines[1]
    );
}

struct Page(usize);

impl Page {
    fn new() -> Self {
        // SAFETY: a fresh private page
        let base = unsafe { VirtualAlloc(std::ptr::null(), 4096, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE) };
        assert!(!base.is_null());
        Self(base as usize)
    }

    fn code(&self, at: usize, bytes: &[u8]) {
        // SAFETY: inside the page, still writable
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), (self.0 + at) as *mut u8, bytes.len()) };
        let mut old: PAGE_PROTECTION_FLAGS = 0;
        // SAFETY: the page is ours
        assert_ne!(
            unsafe { VirtualProtect(self.0 as *const _, 4096, PAGE_EXECUTE_READ, &mut old) },
            0
        );
    }
}

impl Drop for Page {
    fn drop(&mut self) {
        // SAFETY: allocated by `new`
        unsafe { VirtualFree(self.0 as *mut _, 0, MEM_RELEASE) };
    }
}

static TEST_SITE: AtomicUsize = AtomicUsize::new(0);
const SENTINEL: u64 = 0xBEEF;

/// The test's own outer handler: skips the faulting load to the `ret` and returns a sentinel.
unsafe extern "system" fn outer_handler(info: *mut EXCEPTION_POINTERS) -> i32 {
    // SAFETY: the dispatcher passes valid pointers
    unsafe {
        let record = &*(*info).ExceptionRecord;
        let context = &mut *(*info).ContextRecord;
        let site = TEST_SITE.load(Ordering::SeqCst);
        if record.ExceptionCode as u32 == 0xC000_0005 && site != 0 && context.Rip as usize == site {
            context.Rip += 3;
            context.Rax = SENTINEL;
            return EXCEPTION_CONTINUE_EXECUTION;
        }
    }
    EXCEPTION_CONTINUE_SEARCH
}

/// Calls the page's `movzx eax, word ptr [rdi]; ret` with `rdi`.
fn load_through(page: usize, rdi: usize) -> u64 {
    let result: u64;
    // SAFETY: the page holds a 4-byte leaf that reads [rdi] and returns; a fault there is handled by a VEH
    unsafe {
        std::arch::asm!(
            "call {function}",
            function = in(reg) page,
            inout("rdi") rdi => _,
            out("rax") result,
            clobber_abi("C"),
        );
    }
    result
}

#[test]
fn handler_recovers_the_header_load_on_a_test_page() {
    let page = Page::new();
    page.code(0, &[0x0F, 0xB7, 0x07, 0xC3]);
    veh::install(0).unwrap();
    // SAFETY: a static handler, removed below
    let outer = unsafe { AddVectoredExceptionHandler(0, Some(outer_handler)) };
    assert!(!outer.is_null());
    TEST_SITE.store(page.0, Ordering::SeqCst);
    take_captured();

    arm_at(page.0);
    assert_eq!(
        load_through(page.0, 0x100),
        0,
        "the load read the empty entry's zero header"
    );
    let lines = take_captured();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].starts_with("[MitException] recovered invalid FName text read at +0x2660278 rdi=0x100 access=0x100"),
        "{}",
        lines[0]
    );

    arm_at(0);
    assert_eq!(
        load_through(page.0, 0x100),
        SENTINEL,
        "unarmed: the fault reached the outer handler"
    );
    let lines = take_captured();
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("[MitException] VEH code=0xC0000005 ")
                && line.contains(" params=0x0,0x100 rip=")),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("[MitExit] VEH code=3221225477 stack="))
    );

    TEST_SITE.store(0, Ordering::SeqCst);
    // SAFETY: the handle AddVectoredExceptionHandler returned
    unsafe { RemoveVectoredExceptionHandler(outer) };
}

#[test]
fn context_layout_matches_windows_sys() {
    assert_eq!(size_of::<CONTEXT>(), CONTEXT_SIZE);
    assert_eq!(size_of::<EXCEPTION_RECORD>(), RECORD_SIZE);
    assert_eq!(offset_of!(CONTEXT, Rax), context::RAX);
    assert_eq!(offset_of!(CONTEXT, Rcx), context::RCX);
    assert_eq!(offset_of!(CONTEXT, Rdx), context::RDX);
    assert_eq!(offset_of!(CONTEXT, Rbx), context::RBX);
    assert_eq!(offset_of!(CONTEXT, Rsp), context::RSP);
    assert_eq!(offset_of!(CONTEXT, Rsi), context::RSI);
    assert_eq!(offset_of!(CONTEXT, Rdi), context::RDI);
    assert_eq!(offset_of!(CONTEXT, R8), context::R8);
    assert_eq!(offset_of!(CONTEXT, R9), context::R9);
    assert_eq!(offset_of!(CONTEXT, Rip), context::RIP);
    assert_eq!(size_of::<EXCEPTION_POINTERS>(), 16);
}

#[test]
fn crash_wait_patch_on_a_test_page() {
    let page = Page::new();
    page.code(0x20, &REPORT_CRASH_WAIT_BYTES);
    let site = page.0 + 0x20;
    assert_eq!(
        patch_bytes_at(site, &REPORT_CRASH_WAIT_BYTES, &REPORT_CRASH_WAIT_PATCHED),
        PatchResult::Applied
    );
    assert_eq!(
        patch_bytes_at(site, &REPORT_CRASH_WAIT_BYTES, &REPORT_CRASH_WAIT_PATCHED),
        PatchResult::AlreadyApplied
    );
    let mut now = [0u8; 5];
    assert!(read_bytes(site, &mut now));
    assert_eq!(now, REPORT_CRASH_WAIT_PATCHED);
}
