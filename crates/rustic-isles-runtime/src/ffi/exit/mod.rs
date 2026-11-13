//! R3, process exit and crash handling (spec-runtime-r3): the exit export detours, the vectored exception handler,
//! the engine's fatal-error writer and the `ReportCrash` wait. The texts and decisions are `core::exit`'s.

mod crash_wait;
mod engine_error;
mod exports;
#[cfg(test)]
mod tests;
mod veh;

use super::memory::read_unaligned;
use super::{log, win};
use crate::handlers::{self, TickHandler};
use rustic_isles_runtime_core::exit::{Addr, CAPTURE_FRAMES, FixedText, Tail, stack_line, trim};
use std::cell::Cell;
use std::ffi::c_void;
use std::fmt::Write;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use windows_sys::Win32::Foundation::HMODULE;
use windows_sys::Win32::System::Diagnostics::Debug::RtlCaptureStackBackTrace;
use windows_sys::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleFileNameW,
    GetModuleHandleExW,
};

/// One install line's result: `ok`, or the reason (R3 3.4).
pub type Install = (&'static str, Result<(), &'static str>);

static GAME_TID: AtomicU32 = AtomicU32::new(0);
static IMAGE_BASE: AtomicUsize = AtomicUsize::new(0);
static OWN_START: AtomicUsize = AtomicUsize::new(0);
static OWN_END: AtomicUsize = AtomicUsize::new(0);
/// One `[Panic]` line per id, EXIT-1 to EXIT-14.
static PANICKED: [AtomicBool; 15] = [const { AtomicBool::new(false) }; 15];

thread_local! {
    // Const, no destructor: touching it never allocates or registers anything (R3 3.1)
    static INSIDE: Cell<bool> = const { Cell::new(false) };
}

/// Set while this thread runs an R3 body; cleared on drop.
struct Inside;

impl Drop for Inside {
    fn drop(&mut self) {
        INSIDE.set(false);
    }
}

/// R3 3.1 steps 1 and 2: `None` when the hook must only forward (detaching, or already inside R3 on this thread).
fn enter() -> Option<Inside> {
    if detaching() || INSIDE.get() {
        return None;
    }
    INSIDE.set(true);
    Some(Inside)
}

#[cfg(not(test))]
fn detaching() -> bool {
    super::detaching()
}

#[cfg(test)]
fn detaching() -> bool {
    super::detaching() || tests::detaching_override()
}

/// Writes one R3 line (tests also keep it on this thread).
fn emit(line: &str) {
    #[cfg(test)]
    tests::capture(line);
    log::line(line);
}

/// The `[Panic]` line, once per id.
fn panicked(number: usize, what: &str) {
    if let Some(flag) = PANICKED.get(number)
        && !flag.swap(true, Ordering::SeqCst)
    {
        emit(&format!("[Panic] EXIT-{number} {what}"));
    }
}

/// SP-3's first call records the server game thread (R3 3.3).
fn record_game_thread(_engine: usize, _delta: f32) {
    let _ = GAME_TID.compare_exchange(0, win::thread_id(), Ordering::SeqCst, Ordering::SeqCst);
}

/// Registers R3's tick handler; Init calls it before `handlers::freeze()`.
pub fn register() {
    let _ = handlers::register_tick(TickHandler {
        slice: "R3",
        order: i32::MIN,
        before: Some(record_game_thread),
        after: None,
    });
}

/// The module holding `address`: its handle as an address.
fn module_base(address: usize) -> Option<usize> {
    let mut module: HMODULE = std::ptr::null_mut();
    // SAFETY: FROM_ADDRESS takes any address; UNCHANGED_REFCOUNT takes no reference
    let found = unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            address as *const u16,
            &mut module,
        )
    };
    (found != 0 && !module.is_null()).then_some(module as usize)
}

/// Records the executable's base and this DLL's image range, for the address text and the trim.
fn init_ranges(base: usize) {
    IMAGE_BASE.store(base, Ordering::SeqCst);
    let Some(own) = module_base(init_ranges as *const () as usize) else {
        return;
    };
    // SizeOfImage: the optional header's +56, after the 24-byte signature and file header
    let size = read_unaligned::<i32>(own + 0x3C)
        .and_then(|lfanew| usize::try_from(lfanew).ok())
        .and_then(|lfanew| read_unaligned::<u32>(own + lfanew + 24 + 56));
    if let Some(size) = size {
        OWN_START.store(own, Ordering::SeqCst);
        OWN_END.store(own + size as usize, Ordering::SeqCst);
    }
}

/// An address with the module that holds it, for [`Addr`].
struct Located {
    raw: usize,
    name: FixedText<128>,
    offset: Option<usize>,
}

impl Located {
    fn new(raw: usize) -> Self {
        let mut located = Located {
            raw,
            name: FixedText::new(),
            offset: None,
        };
        if raw == 0 {
            return located;
        }
        let Some(module) = module_base(raw) else {
            return located;
        };
        let mut path = [0u16; 260];
        // SAFETY: the buffer holds 260 units
        let len = unsafe { GetModuleFileNameW(module as HMODULE, path.as_mut_ptr(), 260) } as usize;
        let path = &path[..len.min(260)];
        let start = path
            .iter()
            .rposition(|&unit| unit == u16::from(b'\\') || unit == u16::from(b'/'))
            .map_or(0, |at| at + 1);
        for character in char::decode_utf16(path[start..].iter().copied()) {
            if located
                .name
                .write_char(character.unwrap_or(char::REPLACEMENT_CHARACTER))
                .is_err()
            {
                break;
            }
        }
        located.offset = Some(raw - module);
        located
    }

    fn addr(&self) -> Addr<'_> {
        Addr {
            raw: self.raw,
            module: self.offset.map(|offset| (self.name.as_str(), offset)),
            image_base: IMAGE_BASE.load(Ordering::Relaxed),
        }
    }
}

/// This thread's stack, own frames trimmed, at most 32.
fn stack() -> Vec<usize> {
    let mut frames = [std::ptr::null_mut::<c_void>(); CAPTURE_FRAMES];
    // SAFETY: the buffer holds CAPTURE_FRAMES entries
    let count =
        unsafe { RtlCaptureStackBackTrace(0, CAPTURE_FRAMES as u32, frames.as_mut_ptr(), std::ptr::null_mut()) };
    let frames: Vec<usize> = frames[..usize::from(count).min(CAPTURE_FRAMES)]
        .iter()
        .map(|&frame| frame as usize)
        .collect();
    let own = OWN_START.load(Ordering::Relaxed)..OWN_END.load(Ordering::Relaxed);
    trim(&frames, &own).to_vec()
}

/// The stack line for `frames`.
fn write_stack(tag: &str, code: u32, frames: &[usize]) {
    let located: Vec<Located> = frames.iter().map(|&frame| Located::new(frame)).collect();
    let addrs: Vec<Addr<'_>> = located.iter().map(Located::addr).collect();
    emit(&stack_line(tag, code, &addrs));
}

/// The thread fields, with `ret` as the caller.
fn tail_with<R>(ret: &Located, body: impl FnOnce(&Tail<'_>) -> R) -> R {
    let tail = Tail {
        tid: win::thread_id(),
        game_tid: GAME_TID.load(Ordering::Relaxed),
        ret: ret.addr(),
    };
    body(&tail)
}

/// Init, before R3's installs: the executable's base and this DLL's range.
pub fn prepare(base: usize) {
    init_ranges(base);
}

/// EXIT-1 to EXIT-11; `hooks_off` is MinHook's failed status, which every detour id then reports.
pub fn install_exports(hooks_off: Option<&'static str>) -> Vec<Install> {
    exports::install(hooks_off)
}

/// EXIT-12 (no MinHook).
pub fn install_veh(base: usize) -> Install {
    ("EXIT-12", veh::install(base))
}

/// EXIT-13 (server).
pub fn install_engine_error(base: usize, hooks_off: Option<&'static str>) -> Install {
    ("EXIT-13", engine_error::install(base, hooks_off))
}

/// EXIT-14 (server, no MinHook).
pub fn install_crash_wait() -> Install {
    ("EXIT-14", crash_wait::install())
}
