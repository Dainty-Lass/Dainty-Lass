//! R10's raw calls (spec-runtime-r10 7, 8.2, 9): the control pipe and its descriptor, the ready pipe, and the
//! capability header's `SetHeader` on the HTTP-1 chain. The decisions are `crate::player_host`'s.

use super::broker::{self, NextCell};
use super::http::{Lent, in_flight, read_url, set_header};
use super::{detaching, log, win};
use rustic_isles_runtime_core::http::is_under;
use rustic_isles_runtime_core::offsets::target;
use rustic_isles_runtime_core::player_host::{
    FENCE_TIMEOUT_EXIT_CODE, FENCE_TIMEOUT_MS, HEADER, HEADER_ORDER, MAX_MESSAGE, READY_RETRY_MS, READY_WAIT_MS,
    fence_ack, is_fence, lines,
};
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, GENERIC_WRITE, GetLastError, HANDLE, HLOCAL,
    INVALID_HANDLE_VALUE, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_FIRST_PIPE_INSTANCE, FlushFileBuffers, OPEN_EXISTING,
    PIPE_ACCESS_DUPLEX, ReadFile, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, WriteFile,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeClientProcessId, PIPE_READMODE_MESSAGE,
    PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE, PIPE_WAIT, WaitNamedPipeW,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

type ProcessRequestFn = unsafe extern "C" fn(*mut c_void) -> u8;

/// A handle closed on drop.
struct Owned(HANDLE);

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: a handle this module opened and nothing else closes
        unsafe { CloseHandle(self.0) };
    }
}

fn last_error() -> u32 {
    // SAFETY: no arguments
    unsafe { GetLastError() }
}

/// A descriptor `ConvertStringSecurityDescriptorToSecurityDescriptorW` allocated; freed with `LocalFree`.
pub(crate) struct Descriptor(PSECURITY_DESCRIPTOR);

impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: LocalAlloc'd by the conversion
        unsafe { LocalFree(self.0 as HLOCAL) };
    }
}

impl Descriptor {
    pub(crate) fn as_ptr(&self) -> PSECURITY_DESCRIPTOR {
        self.0
    }
}

/// This process token's user SID as SDDL text.
fn user_sid() -> Option<String> {
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: the current process's pseudo-handle; `token` receives a handle closed by `Owned`
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return None;
    }
    let token = Owned(token);
    let mut len = 0u32;
    // SAFETY: a null buffer of size 0 asks for the size
    unsafe { GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut len) };
    if len == 0 {
        return None;
    }
    // u64 words keep TOKEN_USER aligned
    let mut buffer = vec![0u64; (len as usize).div_ceil(8)];
    // SAFETY: `buffer` holds at least `len` bytes
    if unsafe { GetTokenInformation(token.0, TokenUser, buffer.as_mut_ptr().cast(), len, &mut len) } == 0 {
        return None;
    }
    // SAFETY: GetTokenInformation(TokenUser) filled a TOKEN_USER at the start of the buffer
    let sid = unsafe { (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    let mut text: *mut u16 = std::ptr::null_mut();
    // SAFETY: a SID inside `buffer`, alive for the call; `text` is LocalAlloc'd and freed below
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 || text.is_null() {
        return None;
    }
    // SAFETY: a NUL-terminated string the call allocated
    let units = unsafe {
        let mut len = 0;
        while *text.add(len) != 0 {
            len += 1;
        }
        std::slice::from_raw_parts(text, len).to_vec()
    };
    // SAFETY: as above
    unsafe { LocalFree(text as HLOCAL) };
    String::from_utf16(&units).ok()
}

/// The control pipe's protected DACL: this user and `LocalSystem` only (departure 1).
pub(crate) fn descriptor() -> Option<Descriptor> {
    let sddl = win::to_wide(&format!("D:P(A;;GA;;;{})(A;;GA;;;SY)", user_sid()?));
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: a NUL-terminated SDDL string; the descriptor is LocalAlloc'd and owned by `Descriptor`
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    };
    (ok != 0 && !descriptor.is_null()).then_some(Descriptor(descriptor))
}

/// What one control connection came to. Every outcome has fenced the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The exact fence, acknowledged.
    Fenced,
    /// Anything else (7.1 steps 1 to 4): fenced, no acknowledgement.
    Refused,
}

/// 7.1 steps 1 to 5 on `name`: one connection from `expected_pid`, one exact fence. `fence` runs before the ack.
pub(crate) fn serve_control(
    name: &str,
    expected_pid: u32,
    session: &str,
    generation: &str,
    fence: &dyn Fn(),
) -> Outcome {
    let refuse = |line: &str| {
        log::line(line);
        fence();
        Outcome::Refused
    };
    let Some(descriptor) = descriptor() else {
        return refuse(&lines::pipe_failed(last_error()));
    };
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.as_ptr(),
        bInheritHandle: 0,
    };
    let wide = win::to_wide(name);
    let buffer = MAX_MESSAGE as u32 + 1;
    // SAFETY: a NUL-terminated name and attributes whose descriptor lives past the call
    let pipe = unsafe {
        CreateNamedPipeW(
            wide.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            buffer,
            buffer,
            0,
            &attributes,
        )
    };
    if pipe == INVALID_HANDLE_VALUE || pipe.is_null() {
        return refuse(&lines::pipe_failed(last_error()));
    }
    let pipe = Owned(pipe);
    // SAFETY: a pipe this function created, waited on synchronously
    if unsafe { ConnectNamedPipe(pipe.0, std::ptr::null_mut()) } == 0 {
        let error = last_error();
        if error != ERROR_PIPE_CONNECTED {
            return refuse(&lines::not_connected(error));
        }
    }
    let disconnect = || {
        // SAFETY: the connected pipe
        unsafe { DisconnectNamedPipe(pipe.0) };
    };
    let mut client = 0u32;
    // SAFETY: `client` receives the pid
    if unsafe { GetNamedPipeClientProcessId(pipe.0, &mut client) } == 0 || client != expected_pid {
        disconnect();
        return refuse(lines::UNEXPECTED_PID);
    }
    let mut message = [0u8; MAX_MESSAGE];
    let mut read = 0u32;
    // SAFETY: `message` holds MAX_MESSAGE bytes; a longer message fails with ERROR_MORE_DATA
    let ok = unsafe {
        ReadFile(
            pipe.0,
            message.as_mut_ptr(),
            MAX_MESSAGE as u32,
            &mut read,
            std::ptr::null_mut(),
        )
    };
    let got = &message[..(read as usize).min(MAX_MESSAGE)];
    if ok == 0 || got.is_empty() || !is_fence(got, session, generation) {
        disconnect();
        return refuse(lines::MALFORMED);
    }
    // The fence holds before the launcher hears of it; a failed write changes nothing
    fence();
    let ack = fence_ack(session, generation);
    let mut written = 0u32;
    // SAFETY: the ack's bytes, alive for the call
    unsafe {
        WriteFile(
            pipe.0,
            ack.as_ptr(),
            ack.len() as u32,
            &mut written,
            std::ptr::null_mut(),
        );
        FlushFileBuffers(pipe.0);
    }
    disconnect();
    drop(pipe);
    log::line(lines::FENCED);
    Outcome::Fenced
}

/// 6 step 4: the control thread. Every outcome fences; 5 s later it ends the process itself (departure 3).
pub(crate) fn start_control(
    name: String,
    expected_pid: u32,
    session: String,
    generation: String,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("rir-player-host".into())
        .spawn(move || {
            serve_control(&name, expected_pid, &session, &generation, &crate::player_host::fence);
            win::sleep_ms(FENCE_TIMEOUT_MS);
            log::line(lines::FENCE_TIMEOUT);
            win::terminate(FENCE_TIMEOUT_EXIT_CODE);
        })
        .map(drop)
}

/// 8.2: opens the launcher's ready pipe (retrying a busy instance for up to 5 s) and writes `message` once.
pub(crate) fn report_ready(name: &str, message: &str) -> bool {
    let wide = win::to_wide(name);
    let start = win::tick();
    let mut attempt = 0u32;
    let file = loop {
        attempt += 1;
        // SAFETY: a NUL-terminated name
        if unsafe { WaitNamedPipeW(wide.as_ptr(), READY_WAIT_MS) } == 0 {
            log::line(&lines::ready_unavailable(last_error(), attempt));
            return false;
        }
        // SAFETY: a NUL-terminated name; identification-only impersonation (departure 2)
        let file = unsafe {
            CreateFileW(
                wide.as_ptr(),
                GENERIC_WRITE,
                0,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                std::ptr::null_mut(),
            )
        };
        if file != INVALID_HANDLE_VALUE && !file.is_null() {
            break Owned(file);
        }
        let error = last_error();
        log::line(&lines::ready_open_failed(error, attempt));
        if error != ERROR_PIPE_BUSY || win::tick().saturating_sub(start) >= u64::from(READY_WAIT_MS) {
            return false;
        }
        win::sleep_ms(READY_RETRY_MS);
    };
    let mut written = 0u32;
    // SAFETY: the message's bytes, alive for the call
    let ok = unsafe {
        WriteFile(
            file.0,
            message.as_ptr(),
            message.len() as u32,
            &mut written,
            std::ptr::null_mut(),
        )
    };
    // SAFETY: the open handle
    unsafe { FlushFileBuffers(file.0) };
    drop(file);
    if ok == 0 || written as usize != message.len() {
        log::line(lines::READY_WRITE_FAILED);
        return false;
    }
    log::line(lines::READY_REPORTED);
    true
}

static NEXT: NextCell = NextCell::new();
static PANICKED: AtomicBool = AtomicBool::new(false);

/// 9 step 2: the capability header on an idle request under the API base. True when `SetHeader` ran.
pub(crate) fn add_header(request: usize, api_base: &[u16], capability: &[u16]) -> bool {
    let Some(url) = read_url(request) else {
        return false;
    };
    if in_flight(request) != Some(false) || !is_under(&url.text, api_base) {
        return false;
    }
    set_header(request, &Lent::of(HEADER), &Lent::new(capability))
}

fn api_base() -> &'static [u16] {
    static BASE: OnceLock<Vec<u16>> = OnceLock::new();
    BASE.get_or_init(|| crate::config::backend_url().api_base().encode_utf16().collect())
}

unsafe extern "C" fn header_detour(request: *mut c_void) -> u8 {
    let next = NEXT.load();
    if next == 0 {
        return 0;
    }
    // SAFETY: the broker wrote a function of ProcessRequest's signature into this cell
    let next: ProcessRequestFn = unsafe { std::mem::transmute::<usize, ProcessRequestFn>(next) };
    if !detaching() {
        let body = catch_unwind(AssertUnwindSafe(|| {
            if let Some(capability) = crate::player_host::capability() {
                add_header(request as usize, api_base(), capability);
            }
        }));
        if body.is_err() && !PANICKED.swap(true, Ordering::SeqCst) {
            log::line("[Panic] HTTP-1@R10 handler panicked; the request goes on to the chain");
        }
    }
    // SAFETY: the original argument; every request reaches the game
    unsafe { next(request) }
}

/// `HTTP-1@R10`: R10's wrapper on R2's dispatch point, inside R2's (order 100).
pub fn install_header(base: usize) -> Result<(), i32> {
    let unknown = super::minhook::MH_UNKNOWN;
    let target = target("HTTP-1").ok_or(unknown)?;
    let address = usize::try_from(target.rva)
        .ok()
        .and_then(|rva| base.checked_add(rva))
        .ok_or(unknown)?;
    broker::hook(address, header_detour as *const () as usize, HEADER_ORDER, &NEXT)
}

#[cfg(test)]
mod tests;
