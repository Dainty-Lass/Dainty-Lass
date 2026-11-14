//! ESC-1's detour on `FGameplayScheduler::Initialize` (R18 3.2) and the system clock it reads.

use super::broker::{self, NextCell};
use super::{detaching, log, minhook, win};
use rustic_isles_runtime_core::config::trials::filetime_to_ticks;
use rustic_isles_runtime_core::offsets::target;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::System::SystemInformation::GetSystemTimeAsFileTime;

// The real return type is unknown; `u64` passes whatever the original left in RAX through (R18 call 3)
type InitializeFn = unsafe extern "C" fn(*mut c_void, *const c_void) -> u64;
type ProcessEventFn = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void);

static NEXT: NextCell = NextCell::new();
static PANICKED: AtomicBool = AtomicBool::new(false);

/// Plans and applies the row writes, then always calls the original with its own arguments.
unsafe extern "C" fn esc1_detour(scheduler: *mut c_void, tables: *const c_void) -> u64 {
    let next = NEXT.load();
    if next == 0 {
        return 0;
    }
    // SAFETY: the broker wrote a function of Initialize's signature into this cell
    let next: InitializeFn = unsafe { std::mem::transmute::<usize, InitializeFn>(next) };
    if !detaching() {
        crate::lifecycle::write_status_once();
        let planned = catch_unwind(AssertUnwindSafe(|| {
            crate::escalation::before_initialize(tables as usize);
        }));
        if planned.is_err() && !PANICKED.swap(true, Ordering::SeqCst) {
            log::line("[Panic] ESC-1 schedule planning panicked; Initialize goes on");
        }
    }
    // SAFETY: the original arguments, to the next wrapper or the original
    unsafe { next(scheduler, tables) }
}

/// Installs ESC-1 through the broker (band `Mit`, order 0).
pub fn install_esc1(base: usize) -> Result<(), i32> {
    let line = target("ESC-1").ok_or(minhook::MH_UNKNOWN)?;
    let address = usize::try_from(line.rva)
        .ok()
        .and_then(|rva| base.checked_add(rva))
        .ok_or(minhook::MH_UNKNOWN)?;
    broker::hook(address, esc1_detour as *const () as usize, 0, &NEXT)
}

/// UTC now in .NET ticks.
pub fn now_ticks() -> i64 {
    let mut time = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    // SAFETY: fills the struct
    unsafe { GetSystemTimeAsFileTime(&mut time) };
    filetime_to_ticks((u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime))
}

/// `UObject::ProcessEvent`'s original past every wrapper, with no parameters: no dispatch handler
/// sees the spawn call itself (R18 4.3 step 6). The caller checked `object` live and `function` found in
/// its class chain. R17's `reflect::call` goes through the hooked entry instead.
pub fn process_event_original(object: usize, function: usize) -> bool {
    let Some(entry) = target("SP-2")
        .and_then(|line| usize::try_from(line.rva).ok())
        .and_then(|rva| win::image_base().checked_add(rva))
    else {
        return false;
    };
    if object == 0 || function == 0 || detaching() {
        return false;
    }
    let original = broker::original(entry);
    // SAFETY: ProcessEvent (or the broker's trampoline to it) with its own signature, on the game thread as the game
    // calls it; a function with no parameters takes a null block
    unsafe {
        let call = std::mem::transmute::<usize, ProcessEventFn>(original);
        call(win::void(object), win::void(function), std::ptr::null_mut());
    }
    true
}
