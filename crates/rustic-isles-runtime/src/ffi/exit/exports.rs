//! EXIT-1 to EXIT-11 (R3 4): one detour per export, each logging its entry line and stack, then forwarding.

use super::super::broker::{self, NextCell};
use super::super::memory::read_bytes;
use super::super::minhook;
use super::{Located, emit, enter, panicked, stack, tail_with, write_stack};
use crate::lifecycle::write_status_once;
use rustic_isles_runtime_core::exit::{
    Budget, EXPORTS, Exception, ExitCall, MAX_PARAMS, entry_line, exception_line, filtered, params_of,
};
use std::panic::{AssertUnwindSafe, catch_unwind};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

/// The budget EXIT-5 and EXIT-12 share (R3 5.3).
pub(super) static BUDGET: Budget = Budget::new();

/// An exit export's body: lines, then the original call with the original arguments (R3 4.2).
fn exit_body<R>(index: usize, call: ExitCall, forward: impl FnOnce() -> R) -> R {
    let Some(inside) = enter() else {
        return forward();
    };
    write_status_once();
    let tag = EXPORTS[index].tag;
    if catch_unwind(AssertUnwindSafe(|| {
        #[cfg(test)]
        super::tests::during_body();
        let frames = stack();
        let ret = Located::new(frames.first().copied().unwrap_or(0));
        emit(&tail_with(&ret, |tail| entry_line(tag, call, tail)));
        write_stack(tag, call.stack_code(), &frames);
    }))
    .is_err()
    {
        panicked(index + 1, "hook panicked; the call goes on");
    }
    // Cleared before forwarding, so the exports the original calls (ExitProcess's NtTerminateProcess) log too
    drop(inside);
    forward()
}

/// EXIT-5's body: the filter and budget before anything else (R3 4.2).
fn raise_body(code: u32, flags: u32, count: u32, args: usize, forward: impl FnOnce()) {
    let Some(inside) = enter() else {
        return forward();
    };
    if filtered(code) || !BUDGET.take() {
        drop(inside);
        return forward();
    }
    write_status_once();
    let tag = EXPORTS[4].tag;
    if catch_unwind(AssertUnwindSafe(|| {
        let mut information = [0usize; MAX_PARAMS];
        let wanted = params_of(count, &information).len();
        let mut bytes = [0u8; MAX_PARAMS * 8];
        let params = if args != 0 && read_bytes(args, &mut bytes[..wanted * 8]) {
            for (slot, chunk) in information.iter_mut().zip(bytes.as_chunks::<8>().0) {
                *slot = usize::from_le_bytes(*chunk);
            }
            &information[..wanted]
        } else {
            &[]
        };
        let frames = stack();
        let ret = Located::new(frames.first().copied().unwrap_or(0));
        emit(&exception_line(&Exception {
            tag,
            code,
            flags,
            address: ret.addr(),
            params,
            registers: None,
        }));
        write_stack(tag, code, &frames);
    }))
    .is_err()
    {
        panicked(5, "hook panicked; the call goes on");
    }
    drop(inside);
    forward()
}

macro_rules! detour {
    ($name:ident, $next:ident, ($($arg:ident: $ty:ty),*) -> $ret:ty, $body:expr) => {
        static $next: NextCell = NextCell::new();

        unsafe extern "system" fn $name($($arg: $ty),*) -> $ret {
            let next = $next.load();
            if next == 0 {
                return <$ret>::default();
            }
            // SAFETY: the broker wrote a function of this export's signature into the cell
            let next = unsafe { std::mem::transmute::<usize, unsafe extern "system" fn($($ty),*) -> $ret>(next) };
            // SAFETY: the original arguments, to the next wrapper or the export
            let forward = move || unsafe { next($($arg),*) };
            $body(forward)
        }
    };
}

detour!(exit_process, EXIT1_NEXT, (code: u32) -> (), |f| exit_body(0, ExitCall::Code(code), f));
detour!(rtl_exit_user_process, EXIT2_NEXT, (code: u32) -> (), |f| exit_body(1, ExitCall::Code(code), f));
detour!(nt_terminate_process, EXIT3_NEXT, (handle: isize, status: i32) -> i32, |f| {
    exit_body(2, ExitCall::NtTerminate { handle, status }, f)
});
detour!(terminate_process, EXIT4_NEXT, (handle: isize, code: u32) -> i32, |f| {
    exit_body(3, ExitCall::Terminate { handle, code }, f)
});
detour!(raise_exception, EXIT5_NEXT, (code: u32, flags: u32, count: u32, args: *const usize) -> (), |f| {
    raise_body(code, flags, count, args as usize, f)
});
detour!(ucrt_exit, EXIT6_NEXT, (code: i32) -> (), |f| exit_body(5, ExitCall::CodeI32(code), f));
detour!(ucrt_quick_exit, EXIT7_NEXT, (code: i32) -> (), |f| exit_body(6, ExitCall::CodeI32(code), f));
detour!(ucrt_abort, EXIT8_NEXT, () -> (), |f| exit_body(7, ExitCall::Abort, f));
detour!(msvcrt_exit, EXIT9_NEXT, (code: i32) -> (), |f| exit_body(8, ExitCall::CodeI32(code), f));
detour!(msvcrt_quick_exit, EXIT10_NEXT, (code: i32) -> (), |f| exit_body(9, ExitCall::CodeI32(code), f));
detour!(msvcrt_abort, EXIT11_NEXT, () -> (), |f| exit_body(10, ExitCall::Abort, f));

/// Each export's detour and next cell, in `EXPORTS` order.
fn detour_of(index: usize) -> Option<(usize, &'static NextCell)> {
    let pairs: [(usize, &'static NextCell); 11] = [
        (exit_process as *const () as usize, &EXIT1_NEXT),
        (rtl_exit_user_process as *const () as usize, &EXIT2_NEXT),
        (nt_terminate_process as *const () as usize, &EXIT3_NEXT),
        (terminate_process as *const () as usize, &EXIT4_NEXT),
        (raise_exception as *const () as usize, &EXIT5_NEXT),
        (ucrt_exit as *const () as usize, &EXIT6_NEXT),
        (ucrt_quick_exit as *const () as usize, &EXIT7_NEXT),
        (ucrt_abort as *const () as usize, &EXIT8_NEXT),
        (msvcrt_exit as *const () as usize, &EXIT9_NEXT),
        (msvcrt_quick_exit as *const () as usize, &EXIT10_NEXT),
        (msvcrt_abort as *const () as usize, &EXIT11_NEXT),
    ];
    pairs.get(index).copied()
}

/// The export's address in a loaded module (never loads one, R0 3.1).
fn resolve(module: &str, export: &str) -> Result<usize, &'static str> {
    let module = super::super::win::to_wide(module);
    // SAFETY: NUL-terminated name
    let handle = unsafe { GetModuleHandleW(module.as_ptr()) };
    if handle.is_null() {
        return Err("module not loaded");
    }
    let name: Vec<u8> = export.bytes().chain(std::iter::once(0)).collect();
    // SAFETY: a loaded module and a NUL-terminated name
    match unsafe { GetProcAddress(handle, name.as_ptr()) } {
        Some(function) => Ok(function as usize),
        None => Err("export not found"),
    }
}

/// Hooks one export through the broker.
pub(super) fn install_one(index: usize) -> Result<(), &'static str> {
    let row = EXPORTS.get(index).ok_or("unknown id")?;
    let (detour, next) = detour_of(index).ok_or("unknown id")?;
    let target = resolve(row.module, row.export)?;
    broker::hook(target, detour, 0, next).map_err(minhook::status_name)
}

/// EXIT-1 to EXIT-11 in table order; with MinHook off, each reports its status.
pub(super) fn install(hooks_off: Option<&'static str>) -> Vec<super::Install> {
    EXPORTS
        .iter()
        .enumerate()
        .map(|(index, row)| match hooks_off {
            Some(status) => (row.id, Err(status)),
            None => (row.id, install_one(index)),
        })
        .collect()
}

/// EXIT-4's detour and next cell, for the stand-in test.
#[cfg(test)]
pub(super) fn terminate_detour() -> (usize, &'static NextCell) {
    (terminate_process as *const () as usize, &EXIT4_NEXT)
}
