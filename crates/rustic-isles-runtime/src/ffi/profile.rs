//! The profiler's raw calls (spec-runtime-profile 2): open the game thread, take one stack while it is suspended, and
//! name the module an address is in. Nothing between suspend and resume allocates or takes a lock this crate owns.

use std::ffi::c_void;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, HMODULE};
use windows_sys::Win32::System::Diagnostics::Debug::{
    CONTEXT, CONTEXT_CONTROL_AMD64, CONTEXT_INTEGER_AMD64, GetThreadContext, RtlLookupFunctionEntry, RtlVirtualUnwind,
    UNW_FLAG_NHANDLER,
};
use windows_sys::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleFileNameW,
    GetModuleHandleExW,
};
use windows_sys::Win32::System::Threading::{
    OpenThread, ResumeThread, SuspendThread, THREAD_GET_CONTEXT, THREAD_QUERY_LIMITED_INFORMATION,
    THREAD_SUSPEND_RESUME,
};

/// `GetThreadContext` needs a 16-aligned record.
#[repr(C, align(16))]
struct Aligned(CONTEXT);

/// A thread opened for sampling; closed on drop.
pub struct Thread(HANDLE);

// SAFETY: a kernel handle, used from the one profiler thread
unsafe impl Send for Thread {}

impl Drop for Thread {
    fn drop(&mut self) {
        // SAFETY: the handle OpenThread gave, closed once
        unsafe { CloseHandle(self.0) };
    }
}

/// Opens thread `id` to suspend it and read its context.
pub fn open(id: u32) -> Option<Thread> {
    if id == 0 {
        return None;
    }
    // SAFETY: plain call; a null handle is a failure
    let handle = unsafe {
        OpenThread(
            THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT | THREAD_QUERY_LIMITED_INFORMATION,
            0,
            id,
        )
    };
    (!handle.is_null()).then_some(Thread(handle))
}

/// One stack, innermost first, as function start addresses (the unwind entry's start, or the bare address of a frame
/// without one) into `out`; the count written, 0 when the thread could not be suspended or read.
pub fn sample(thread: &Thread, out: &mut [usize]) -> usize {
    // SAFETY: plain data the calls fill
    let mut context: Aligned = unsafe { std::mem::zeroed() };
    context.0.ContextFlags = CONTEXT_CONTROL_AMD64 | CONTEXT_INTEGER_AMD64;
    // SAFETY: a handle opened with suspend rights; every path below resumes it
    if unsafe { SuspendThread(thread.0) } == u32::MAX {
        return 0;
    }
    // SAFETY: the thread is suspended; the record is aligned and sized for this architecture
    let written = if unsafe { GetThreadContext(thread.0, &mut context.0) } == 0 {
        0
    } else {
        unwind(&mut context.0, out)
    };
    // SAFETY: balances the suspend above
    unsafe { ResumeThread(thread.0) };
    written
}

/// Walks the suspended thread's stack through the loaded images' unwind data, at most `out.len()` frames.
fn unwind(context: &mut CONTEXT, out: &mut [usize]) -> usize {
    let mut written = 0;
    while written < out.len() {
        let pc = context.Rip;
        if pc == 0 {
            break;
        }
        let mut base: u64 = 0;
        // SAFETY: a lookup in the process's own module tables; no history table
        let entry = unsafe { RtlLookupFunctionEntry(pc, &mut base, std::ptr::null_mut()) };
        let Ok(at) = usize::try_from(pc) else {
            break;
        };
        if entry.is_null() {
            // A leaf without unwind data: its return address is at the stack pointer
            out[written] = at;
            written += 1;
            let Ok(sp) = usize::try_from(context.Rsp) else {
                break;
            };
            match super::memory::read::<u64>(sp) {
                Some(ret) => {
                    context.Rip = ret;
                    context.Rsp += 8;
                }
                None => break,
            }
            continue;
        }
        // SAFETY: a non-null entry the lookup returned
        let begin = u64::from(unsafe { (*entry).BeginAddress });
        out[written] = usize::try_from(base + begin).unwrap_or(at);
        written += 1;
        let mut handler: *mut c_void = std::ptr::null_mut();
        let mut frame: u64 = 0;
        // SAFETY: the entry and image base for `pc`; the context is the suspended thread's copy, read through its
        // own stack, which stays mapped while the thread is suspended
        unsafe {
            RtlVirtualUnwind(
                UNW_FLAG_NHANDLER,
                base,
                pc,
                entry,
                context,
                &mut handler,
                &mut frame,
                std::ptr::null_mut(),
            );
        }
        if context.Rip == pc {
            break;
        }
    }
    written
}

/// The base and file name of the module holding `address`.
pub fn module_of(address: usize) -> Option<(usize, String)> {
    let mut module: HMODULE = std::ptr::null_mut();
    // SAFETY: FROM_ADDRESS takes the address itself; UNCHANGED_REFCOUNT leaves nothing to free
    let found = unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            address as *const u16,
            &mut module,
        )
    };
    if found == 0 || module.is_null() {
        return None;
    }
    let mut buffer = [0u16; 260];
    // SAFETY: the buffer holds 260 units
    let len = unsafe { GetModuleFileNameW(module, buffer.as_mut_ptr(), 260) } as usize;
    let path = String::from_utf16_lossy(buffer.get(..len)?);
    let name = path.rsplit(['\\', '/']).next().unwrap_or(&path).to_owned();
    Some((module as usize, name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, mpsc};

    #[inline(never)]
    fn spin(stop: &AtomicBool) {
        while !stop.load(Ordering::Relaxed) {
            std::hint::spin_loop();
        }
    }

    #[test]
    fn samples_another_thread() {
        let stop = Arc::new(AtomicBool::new(false));
        let (send, receive) = mpsc::channel();
        let flag = Arc::clone(&stop);
        let worker = std::thread::spawn(move || {
            // SAFETY: plain call
            let _ = send.send(unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() });
            spin(&flag);
        });
        let id = receive.recv().expect("id");
        let thread = open(id).expect("open");
        let mut out = [0usize; 48];
        let written = sample(&thread, &mut out);
        stop.store(true, Ordering::Relaxed);
        worker.join().expect("join");
        assert!(written >= 2, "a stack of {written}");
        let own = module_of(spin as *const () as usize).expect("module");
        assert!(
            out[..written]
                .iter()
                .any(|&f| module_of(f).is_some_and(|(base, _)| base == own.0))
        );
        assert!(open(0).is_none());
    }

    #[test]
    fn names_modules() {
        let (base, name) = module_of(module_of as *const () as usize).expect("module");
        assert!(base != 0 && !name.is_empty());
        assert!(module_of(16).is_none());
    }
}
