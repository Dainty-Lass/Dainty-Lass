//! EXIT-12 (R3 5): the vectored exception handler. It logs faults within the shared budget and recovers one known
//! engine fault, the FName header load at `+0x02660278`.

use super::super::memory::{function_entry, read, read_unaligned, readable};
use super::exports::BUDGET;
use super::{Located, emit, enter, panicked, stack, write_stack};
use crate::ffi::log;
use crate::lifecycle::write_status_once;
use rustic_isles_runtime_core::exit::{
    Exception, FNAME_NOT_ARMED_LINE, FixedText, Occurrences, Registers, STACK_OVERFLOW, exception_line, filtered,
    fname_armed, fname_line, fname_line_due, is_fname_fault, params_of, write_exception_line,
};
use rustic_isles_runtime_core::offsets::exit::FNAME_HEADER_LOAD;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicUsize, Ordering};
use windows_sys::Win32::System::Diagnostics::Debug::{
    AddVectoredExceptionHandler, EXCEPTION_CONTINUE_EXECUTION, EXCEPTION_CONTINUE_SEARCH, EXCEPTION_POINTERS,
    EXCEPTION_RECORD,
};

/// `EXCEPTION_RECORD`'s size on x64.
pub(super) const RECORD_SIZE: usize = 0x98;
/// `CONTEXT`'s size on x64.
pub(super) const CONTEXT_SIZE: usize = 0x4D0;

/// The `CONTEXT` fields the handler reads, by offset (checked against `windows-sys` in the tests).
pub(super) mod context {
    pub const RAX: usize = 0x78;
    pub const RCX: usize = 0x80;
    pub const RDX: usize = 0x88;
    pub const RBX: usize = 0x90;
    pub const RSP: usize = 0x98;
    pub const RSI: usize = 0xA8;
    pub const RDI: usize = 0xB0;
    pub const R8: usize = 0xB8;
    pub const R9: usize = 0xC0;
    pub const RIP: usize = 0xF8;
}

/// The header load's address once armed, 0 while not.
static FNAME_SITE: AtomicUsize = AtomicUsize::new(0);
static FNAME_HITS: Occurrences = Occurrences::new();
static HANDLER: AtomicUsize = AtomicUsize::new(0);

/// What a recovered load reads instead: a zero header, an empty narrow name. Never written.
#[repr(C, align(8))]
struct EmptyEntry([u8; 32]);
static EMPTY_ENTRY: EmptyEntry = EmptyEntry([0; 32]);

/// Arms the recovery only on the expected instruction in the expected function (R3 5.2).
fn arm(base: usize) {
    let site = base.checked_add(FNAME_HEADER_LOAD as usize);
    let bytes = site.and_then(read_unaligned::<[u8; 3]>);
    if let Some(site) = site
        && fname_armed(bytes, function_entry(FNAME_HEADER_LOAD))
    {
        FNAME_SITE.store(site, Ordering::SeqCst);
    } else {
        log::defer(FNAME_NOT_ARMED_LINE.to_owned());
    }
}

/// Points the recovery at another address (a test page).
#[cfg(test)]
pub(super) fn arm_at(site: usize) {
    FNAME_SITE.store(site, Ordering::SeqCst);
}

/// `AddVectoredExceptionHandler(1, handler)` once; the handle is kept (R3 5.1).
pub(super) fn install(base: usize) -> Result<(), &'static str> {
    arm(base);
    if HANDLER.load(Ordering::SeqCst) != 0 {
        return Ok(());
    }
    // SAFETY: a handler of the right signature that lives as long as the (pinned) DLL
    let handle = unsafe { AddVectoredExceptionHandler(1, Some(handler)) };
    if handle.is_null() {
        return Err("AddVectoredExceptionHandler failed");
    }
    HANDLER.store(handle as usize, Ordering::SeqCst);
    Ok(())
}

unsafe extern "system" fn handler(info: *mut EXCEPTION_POINTERS) -> i32 {
    let Some(inside) = enter() else {
        return EXCEPTION_CONTINUE_SEARCH;
    };
    let verdict = catch_unwind(AssertUnwindSafe(|| handle(info as usize))).unwrap_or_else(|_| {
        panicked(12, "handler panicked; the exception goes on");
        EXCEPTION_CONTINUE_SEARCH
    });
    drop(inside);
    verdict
}

fn handle(info: usize) -> i32 {
    let pointers = read::<[usize; 2]>(info);
    let record = pointers
        .map(|[record, _]| record)
        .filter(|&record| readable(record, RECORD_SIZE))
        .and_then(read::<EXCEPTION_RECORD>);
    let context = pointers
        .map(|[_, context]| context)
        .filter(|&context| readable(context, CONTEXT_SIZE));
    let register = |offset: usize| context.and_then(|context| read::<u64>(context + offset));
    let code = record.as_ref().map(|record| record.ExceptionCode as u32);
    if let (Some(record), Some(context), Some(code)) = (record.as_ref(), context, code) {
        let site = FNAME_SITE.load(Ordering::Relaxed);
        if let Some(rip) = register(context::RIP)
            && is_fname_fault(code, rip, site, site != 0)
        {
            recover(record, context);
            return EXCEPTION_CONTINUE_EXECUTION;
        }
        if filtered(code) {
            return EXCEPTION_CONTINUE_SEARCH;
        }
    }
    if !BUDGET.take() {
        return EXCEPTION_CONTINUE_SEARCH;
    }
    write_status_once();
    let code = code.unwrap_or(0);
    let address = Located::new(record.as_ref().map_or(0, |record| record.ExceptionAddress as usize));
    let params = record.as_ref().map_or(&[][..], |record| {
        params_of(record.NumberParameters, &record.ExceptionInformation)
    });
    let mut exception = Exception {
        tag: "VEH",
        code,
        flags: record.as_ref().map_or(0, |record| record.ExceptionFlags),
        address: address.addr(),
        params,
        registers: None,
    };
    if code == STACK_OVERFLOW {
        // Out of stack: no registers, no walk, one fixed buffer (R3 5.4)
        let mut text = FixedText::<1024>::new();
        let _ = write_exception_line(&mut text, &exception);
        emit(text.as_str());
        return EXCEPTION_CONTINUE_SEARCH;
    }
    let rip = Located::new(register(context::RIP).map_or(0, |rip| rip as usize));
    if context.is_some() {
        let value = |offset| register(offset).unwrap_or(0);
        exception.registers = Some(Registers {
            rip: rip.addr(),
            rsp: value(context::RSP),
            rax: value(context::RAX),
            rcx: value(context::RCX),
            rdx: value(context::RDX),
            rbx: value(context::RBX),
            rsi: value(context::RSI),
            rdi: value(context::RDI),
            r8: value(context::R8),
            r9: value(context::R9),
        });
    }
    emit(&exception_line(&exception));
    write_stack("VEH", code, &stack());
    EXCEPTION_CONTINUE_SEARCH
}

/// Points `Rdi` at the empty entry so the `movzx` reads a zero header; every time, throttled lines (R3 5.2).
fn recover(record: &EXCEPTION_RECORD, context: usize) {
    let slot = context + context::RDI;
    let old = read::<u64>(slot).unwrap_or(0);
    // SAFETY: the context is the faulting thread's, readable and writable for CONTEXT_SIZE; Rdi is inside it
    unsafe { std::ptr::write_volatile(slot as *mut u64, &EMPTY_ENTRY as *const EmptyEntry as u64) };
    let occurrence = FNAME_HITS.next();
    if fname_line_due(occurrence) {
        let access = (record.NumberParameters >= 2).then_some(record.ExceptionInformation[1]);
        write_status_once();
        emit(&fname_line(old, access, occurrence));
    }
}
