//! R12's DLL tests (16.1) on test-allocated memory; the drive's call and the hooks against the game are not run.

use super::guards::{self, RemoteCall};
use super::identity::ProcessGame;
use super::{BUDGETS, with_state};
use rustic_isles_runtime_core::repgraph::{Checked, Queued, Source, drive_target};
use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};
use windows_sys::Win32::System::Memory::{
    MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_READWRITE, VirtualAlloc, VirtualFree,
};

const PAGE: usize = 4096;

/// A page from `VirtualAlloc`: page-aligned, so every pointer field written into it is aligned.
struct Page(usize);

impl Page {
    fn new() -> Self {
        let base = unsafe { VirtualAlloc(std::ptr::null(), PAGE, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE) };
        assert!(!base.is_null());
        Self(base as usize)
    }
    fn put(&self, offset: usize, value: usize) {
        assert!(offset.is_multiple_of(8));
        unsafe { std::ptr::write((self.0 + offset) as *mut usize, value) };
    }
    fn ptr(&self) -> *mut c_void {
        self.0 as *mut c_void
    }
}

impl Drop for Page {
    fn drop(&mut self) {
        unsafe { VirtualFree(self.0 as *mut _, 0, MEM_RELEASE) };
    }
}

static CALLS: AtomicU32 = AtomicU32::new(0);

unsafe extern "C" fn replicate_next(
    _: *mut c_void,
    _: *mut c_void,
    _: *mut c_void,
    _: *mut c_void,
    _: *mut c_void,
    _: *mut c_void,
    frame: u32,
) -> u64 {
    0xDEAD_0000_0000_0000 | u64::from(frame)
}

unsafe extern "C" fn send_next(_: *mut c_void) -> i64 {
    0x1234_5678_9ABC
}

#[allow(clippy::too_many_arguments, reason = "the engine function's own arguments")]
unsafe extern "C" fn rpc_next(
    _: *mut c_void,
    _: *mut c_void,
    _: *mut c_void,
    _: *mut c_void,
    _: *mut c_void,
    _: *mut c_void,
    _: *mut c_void,
    _: *mut c_void,
    _: *mut c_void,
    _: *mut c_void,
    _: u8,
    _: i32,
    _: *mut c_void,
) {
    CALLS.fetch_add(1, Ordering::SeqCst);
}

#[test]
fn send_on_a_quarantined_channel_answers_zero_in_full() {
    let channel = Page::new();
    with_state(|state| {
        state.quarantine.queue(
            &ProcessGame,
            &BUDGETS,
            Queued {
                channel: channel.0,
                actor: 0,
                wrong: 0,
                correct: 0,
                source: Source::ReplicateActorFinal,
                caller: 0,
            },
        )
    });
    // The detour itself, as the graph calls it: a full 64-bit 0
    assert_eq!(unsafe { guards::send_detour(channel.ptr()) }, 0);
    assert_eq!(guards::send_with(Some(send_next), channel.ptr()), 0, "next not called");
    let other = Page::new();
    assert_eq!(
        guards::send_with(Some(send_next), other.ptr()),
        0x1234_5678_9ABC,
        "not a controller: next's answer"
    );
}

#[test]
fn pre_channel_passes_a_non_controller() {
    let actor = Page::new();
    let class = Page::new();
    actor.put(0x10, class.0);
    let null = std::ptr::null_mut();
    let answer = guards::pre_channel_with(Some(replicate_next), null, actor.ptr(), null, null, null, null, 7);
    assert_eq!(answer, 0xDEAD_0000_0000_0007);
}

#[test]
fn rpc_on_an_unbound_channel_is_dropped() {
    let channel = Page::new();
    channel.put(0x70, 0);
    let null = std::ptr::null_mut();
    let call = |channel: *mut c_void, is_server: u8| RemoteCall {
        driver: null,
        channel,
        class_cache: null,
        field_cache: null,
        target_obj: null,
        connection: null,
        function: null,
        parms: null,
        out_parms: null,
        stack: null,
        is_server,
        send_policy: 0,
        remote_function_flags: null,
    };
    let before = CALLS.load(Ordering::SeqCst);
    guards::rpc_with(Some(rpc_next), call(channel.ptr(), 1));
    assert_eq!(CALLS.load(Ordering::SeqCst), before, "dropped");
    guards::rpc_with(Some(rpc_next), call(channel.ptr(), 0));
    assert_eq!(CALLS.load(Ordering::SeqCst), before + 1, "a client call passes");
    channel.put(0x70, channel.0);
    guards::rpc_with(Some(rpc_next), call(channel.ptr(), 1));
    assert_eq!(CALLS.load(Ordering::SeqCst), before + 2, "a bound channel passes");
}

#[test]
fn drive_refuses_a_slot_into_a_test_allocation() {
    let driver = Page::new();
    let graph = Page::new();
    let vtable = Page::new();
    let code = Page::new();
    driver.put(0x6E8, graph.0);
    graph.put(0x00, vtable.0);
    graph.put(0x30, driver.0);
    vtable.put(0x2F8, code.0);
    super::identity::set_image(crate::ffi::win::image_base());
    let mut checked = Checked::new();
    assert_eq!(drive_target(&ProcessGame, &mut checked, Some(driver.0)), None);
    assert_eq!(super::graph(), None, "R9 is not listening in a test");
}
