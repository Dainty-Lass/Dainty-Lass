// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Silent Spud

//! The two pipe ends on Windows (spec-runtime-r10 7, 8.2, 12.2). Overlapped I/O, so every wait has a deadline and
//! gives up early when the server exits.
#![allow(unsafe_code, reason = "the named pipe and token calls; each block has a SAFETY note")]

use std::ffi::c_void;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    ERROR_IO_PENDING, ERROR_MORE_DATA, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED, GENERIC_READ, GENERIC_WRITE,
    GetLastError, HANDLE, HLOCAL, INVALID_HANDLE_VALUE, LocalFree, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, OPEN_EXISTING, PIPE_ACCESS_INBOUND, ReadFile,
    SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, WriteFile,
};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId, GetNamedPipeServerProcessId,
    PIPE_READMODE_MESSAGE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE, PIPE_WAIT, SetNamedPipeHandleState,
    WaitNamedPipeW,
};
use windows_sys::Win32::System::Threading::{CreateEventW, GetCurrentProcess, OpenProcessToken, WaitForSingleObject};

use super::{MAX_MESSAGE, PipeFailure};

/// One more than the largest message, so a read of exactly this many bytes is one too long.
const BUFFER: usize = MAX_MESSAGE + 1;
/// How often a wait looks at whether the server is still running.
const SLICE: Duration = Duration::from_millis(250);

fn last_error() -> u32 {
    // SAFETY: no arguments
    unsafe { GetLastError() }
}

fn os() -> PipeFailure {
    PipeFailure::Os(last_error())
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn owned(handle: HANDLE) -> Result<OwnedHandle, PipeFailure> {
    if handle.is_null() || handle == INVALID_HANDLE_VALUE {
        return Err(os());
    }
    // SAFETY: a handle the call just returned, owned by nothing else
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

fn raw(handle: &OwnedHandle) -> HANDLE {
    handle.as_raw_handle()
}

/// Memory `LocalAlloc`'d by a conversion call, freed on drop.
struct Local(*mut c_void);

impl Drop for Local {
    fn drop(&mut self) {
        // SAFETY: LocalAlloc'd by the call that filled it, freed once
        unsafe { LocalFree(self.0 as HLOCAL) };
    }
}

/// This process token's user SID as SDDL text.
fn user_sid() -> Result<String, PipeFailure> {
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: the current process's pseudo-handle; `token` receives a handle `owned` closes
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(os());
    }
    let token = owned(token)?;
    let mut len = 0u32;
    // SAFETY: a null buffer of size 0 asks for the size
    unsafe { GetTokenInformation(raw(&token), TokenUser, std::ptr::null_mut(), 0, &mut len) };
    if len == 0 {
        return Err(os());
    }
    // u64 words keep TOKEN_USER aligned
    let mut buffer = vec![0u64; (len as usize).div_ceil(8)];
    // SAFETY: `buffer` holds at least `len` bytes
    if unsafe { GetTokenInformation(raw(&token), TokenUser, buffer.as_mut_ptr().cast(), len, &mut len) } == 0 {
        return Err(os());
    }
    // SAFETY: GetTokenInformation(TokenUser) filled a TOKEN_USER at the start of the buffer
    let sid = unsafe { (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    let mut text: *mut u16 = std::ptr::null_mut();
    // SAFETY: a SID inside `buffer`, alive for the call; `text` is LocalAlloc'd and freed by `Local`
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 || text.is_null() {
        return Err(os());
    }
    local_text(&Local(text.cast()))
}

/// A NUL-terminated UTF-16 string a conversion call allocated.
fn local_text(text: &Local) -> Result<String, PipeFailure> {
    let start = text.0.cast::<u16>();
    // SAFETY: a NUL-terminated string the call allocated, read up to its NUL while `text` holds it
    let units = unsafe {
        let mut len = 0;
        while *start.add(len) != 0 {
            len += 1;
        }
        std::slice::from_raw_parts(start, len)
    };
    String::from_utf16(units).map_err(|_| PipeFailure::Os(0))
}

/// The SDDL for a pipe only this user and `LocalSystem` may open, as the runtime's control pipe (7.1 departure 1).
pub(super) fn sddl() -> Result<String, PipeFailure> {
    Ok(format!("D:P(A;;GA;;;{})(A;;GA;;;SY)", user_sid()?))
}

/// The protected DACL of [`sddl`]; no default DACL as a fallback.
struct Descriptor(Local);

impl Descriptor {
    fn new() -> Result<Self, PipeFailure> {
        let text = wide(&sddl()?);
        let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: a NUL-terminated SDDL string; the descriptor is LocalAlloc'd and owned by `Local`
        let ok = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                text.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 || descriptor.is_null() {
            return Err(os());
        }
        Ok(Self(Local(descriptor)))
    }
}

/// An `OVERLAPPED` whose event handle field makes it `!Send`.
struct Overlapped(OVERLAPPED);

// SAFETY: only its owner touches it, and every operation on it is drained before it moves or drops
unsafe impl Send for Overlapped {}

/// One overlapped operation's event and `OVERLAPPED`, boxed so its address holds while the kernel writes it.
struct Pending {
    _event: OwnedHandle,
    overlapped: Box<Overlapped>,
}

impl Pending {
    fn new() -> Result<Self, PipeFailure> {
        // SAFETY: an unnamed manual-reset event, not signalled; `owned` closes it
        let event = owned(unsafe { CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()) })?;
        // SAFETY: OVERLAPPED is plain data, valid when zeroed
        let mut overlapped = Box::new(Overlapped(unsafe { std::mem::zeroed() }));
        overlapped.0.hEvent = raw(&event);
        Ok(Self {
            _event: event,
            overlapped,
        })
    }

    fn as_mut_ptr(&mut self) -> *mut OVERLAPPED {
        &mut self.overlapped.0
    }

    /// Waits for the operation `started` began on `handle` (its BOOL answer, read with `GetLastError` at once),
    /// until `deadline` or until `alive` says no; cancelled and drained on giving up. The bytes moved.
    fn finish(
        &mut self,
        handle: HANDLE,
        started: i32,
        deadline: Instant,
        alive: &dyn Fn() -> bool,
    ) -> Result<u32, PipeFailure> {
        if started == 0 {
            match last_error() {
                ERROR_IO_PENDING => {}
                ERROR_MORE_DATA => return Err(PipeFailure::TooLong),
                error => return Err(PipeFailure::Os(error)),
            }
        }
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let alive_now = alive();
            if left.is_zero() || !alive_now {
                self.cancel(handle);
                return Err(if alive_now {
                    PipeFailure::TimedOut
                } else {
                    PipeFailure::Exited
                });
            }
            let slice = u32::try_from(left.min(SLICE).as_millis()).unwrap_or(u32::MAX);
            // SAFETY: the event this operation signals, owned by `self`
            match unsafe { WaitForSingleObject(self.overlapped.0.hEvent, slice) } {
                WAIT_OBJECT_0 => break,
                WAIT_TIMEOUT => {}
                _ => {
                    let failed = os();
                    self.cancel(handle);
                    return Err(failed);
                }
            }
        }
        let mut moved = 0u32;
        // SAFETY: the completed operation's OVERLAPPED; no wait
        if unsafe { GetOverlappedResult(handle, &self.overlapped.0, &mut moved, 0) } == 0 {
            return Err(match last_error() {
                ERROR_MORE_DATA => PipeFailure::TooLong,
                error => PipeFailure::Os(error),
            });
        }
        Ok(moved)
    }

    /// Cancels the operation and waits until the kernel no longer touches the `OVERLAPPED` or the buffer.
    fn cancel(&mut self, handle: HANDLE) {
        let mut moved = 0u32;
        // SAFETY: this operation's OVERLAPPED; the blocking wait returns once the cancel lands
        unsafe {
            CancelIoEx(handle, &self.overlapped.0);
            GetOverlappedResult(handle, &self.overlapped.0, &mut moved, 1);
        }
    }
}

/// The ready pipe's server end: made before the server starts, its connect already pending (12.2 row 2).
pub(super) struct ReadyListener {
    pipe: OwnedHandle,
    connect: Pending,
    connected: bool,
}

impl ReadyListener {
    /// First instance, inbound, message mode, remote clients rejected, this user and `LocalSystem` only.
    pub(super) fn create(name: &str) -> Result<Self, PipeFailure> {
        let descriptor = Descriptor::new()?;
        let attributes = SECURITY_ATTRIBUTES {
            nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>()).unwrap_or(u32::MAX),
            lpSecurityDescriptor: (descriptor.0).0,
            bInheritHandle: 0,
        };
        let name = wide(name);
        let buffer = u32::try_from(BUFFER).unwrap_or(u32::MAX);
        // SAFETY: a NUL-terminated name and attributes alive for the call; `owned` closes the pipe
        let pipe = owned(unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE | FILE_FLAG_OVERLAPPED,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                0,
                buffer,
                0,
                &attributes,
            )
        })?;
        let mut connect = Pending::new()?;
        // SAFETY: the pipe and an OVERLAPPED that lives in `Self` until the drop drains it
        let started = unsafe { ConnectNamedPipe(raw(&pipe), connect.as_mut_ptr()) };
        let mut listener = Self {
            pipe,
            connect,
            connected: false,
        };
        if started == 0 {
            match last_error() {
                ERROR_IO_PENDING => {}
                ERROR_PIPE_CONNECTED => listener.connected = true,
                error => {
                    listener.connected = true;
                    return Err(PipeFailure::Os(error));
                }
            }
        }
        Ok(listener)
    }

    /// One client, which must be `expected_pid`, and one message of at most 511 bytes from it.
    pub(super) fn accept(
        mut self,
        expected_pid: u32,
        timeout: Duration,
        alive: &dyn Fn() -> bool,
    ) -> Result<Vec<u8>, PipeFailure> {
        let deadline = Instant::now() + timeout;
        let pipe = raw(&self.pipe);
        if !self.connected {
            // `ConnectNamedPipe` already answered pending in `create`, so only its completion is waited for
            let waited = self.connect.finish(pipe, 1, deadline, alive);
            self.connected = true;
            waited?;
        }
        let mut client = 0u32;
        // SAFETY: the connected pipe; `client` receives a pid
        if unsafe { GetNamedPipeClientProcessId(pipe, &mut client) } == 0 {
            return Err(os());
        }
        if client != expected_pid {
            return Err(PipeFailure::WrongPeer);
        }
        let mut buffer = vec![0u8; BUFFER];
        let mut read = Pending::new()?;
        let len = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
        // SAFETY: `buffer` and `read` outlive the operation: `finish` returns only once it completed or drained
        let started = unsafe { ReadFile(pipe, buffer.as_mut_ptr(), len, std::ptr::null_mut(), read.as_mut_ptr()) };
        let moved = read.finish(pipe, started, deadline, alive)? as usize;
        if moved > MAX_MESSAGE {
            return Err(PipeFailure::TooLong);
        }
        buffer.truncate(moved);
        Ok(buffer)
    }
}

impl Drop for ReadyListener {
    fn drop(&mut self) {
        if !self.connected {
            self.connect.cancel(raw(&self.pipe));
        }
    }
}

/// Opens the control pipe as a client, checks it is served by `expected_server_pid`, writes `request` in one write
/// and reads one answer of at most 511 bytes, all within `timeout`.
pub(super) fn fence(
    name: &str,
    expected_server_pid: u32,
    request: &[u8],
    timeout: Duration,
) -> Result<Vec<u8>, PipeFailure> {
    let deadline = Instant::now() + timeout;
    let name = wide(name);
    let always = || true;
    let pipe = loop {
        // SAFETY: a NUL-terminated name; identification-only, so the server end cannot impersonate the launcher
        let handle = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                std::ptr::null_mut(),
            )
        };
        if let Ok(pipe) = owned(handle) {
            break pipe;
        }
        let error = last_error();
        let left = deadline.saturating_duration_since(Instant::now());
        if error != ERROR_PIPE_BUSY || left.is_zero() {
            return Err(PipeFailure::Os(error));
        }
        let wait = u32::try_from(left.min(SLICE).as_millis()).unwrap_or(1).max(1);
        // SAFETY: a NUL-terminated name
        unsafe { WaitNamedPipeW(name.as_ptr(), wait) };
    };
    let handle = raw(&pipe);
    let mut server = 0u32;
    // SAFETY: the open pipe; `server` receives a pid
    if unsafe { GetNamedPipeServerProcessId(handle, &mut server) } == 0 {
        return Err(os());
    }
    if server != expected_server_pid {
        return Err(PipeFailure::WrongPeer);
    }
    let mode = PIPE_READMODE_MESSAGE;
    // SAFETY: the open pipe and a mode alive for the call
    if unsafe { SetNamedPipeHandleState(handle, &mode, std::ptr::null(), std::ptr::null()) } == 0 {
        return Err(os());
    }
    let len = u32::try_from(request.len()).map_err(|_| PipeFailure::TooLong)?;
    let mut write = Pending::new()?;
    // SAFETY: `request` and `write` outlive the operation (see `Pending::finish`)
    let started = unsafe { WriteFile(handle, request.as_ptr(), len, std::ptr::null_mut(), write.as_mut_ptr()) };
    if write.finish(handle, started, deadline, &always)? != len {
        return Err(PipeFailure::ShortWrite);
    }
    let mut buffer = vec![0u8; BUFFER];
    let mut read = Pending::new()?;
    let capacity = u32::try_from(buffer.len()).unwrap_or(u32::MAX);
    // SAFETY: as above
    let started = unsafe {
        ReadFile(
            handle,
            buffer.as_mut_ptr(),
            capacity,
            std::ptr::null_mut(),
            read.as_mut_ptr(),
        )
    };
    let moved = read.finish(handle, started, deadline, &always)? as usize;
    if moved > MAX_MESSAGE {
        return Err(PipeFailure::TooLong);
    }
    buffer.truncate(moved);
    Ok(buffer)
}

/// Test support: the pipe's DACL as SDDL, read through the server end's own handle.
#[cfg(test)]
pub(super) fn sddl_of(listener: &ReadyListener) -> Option<String> {
    use windows_sys::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SE_KERNEL_OBJECT,
    };
    use windows_sys::Win32::Security::DACL_SECURITY_INFORMATION;

    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: the open pipe; the descriptor is LocalAlloc'd and freed by `Local`
    let status = unsafe {
        GetSecurityInfo(
            raw(&listener.pipe),
            SE_KERNEL_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 || descriptor.is_null() {
        return None;
    }
    let descriptor = Local(descriptor);
    let mut text: *mut u16 = std::ptr::null_mut();
    // SAFETY: the descriptor above; `text` is LocalAlloc'd and freed by `Local`
    let ok = unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            descriptor.0,
            SDDL_REVISION_1,
            DACL_SECURITY_INFORMATION,
            &mut text,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 || text.is_null() {
        return None;
    }
    let text = Local(text.cast());
    local_text(&text).ok()
}

/// Test support: one blocking duplex server end shaped as the runtime's control pipe (7.1), which answers the first
/// message with `reply` and closes.
#[cfg(test)]
pub(super) fn serve_control_once(
    name: &str,
    reply: impl FnOnce(&[u8]) -> Option<Vec<u8>> + Send + 'static,
) -> Result<std::thread::JoinHandle<()>, PipeFailure> {
    use windows_sys::Win32::Storage::FileSystem::{FlushFileBuffers, PIPE_ACCESS_DUPLEX};
    use windows_sys::Win32::System::Pipes::DisconnectNamedPipe;

    let descriptor = Descriptor::new()?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>()).unwrap_or(u32::MAX),
        lpSecurityDescriptor: (descriptor.0).0,
        bInheritHandle: 0,
    };
    let name = wide(name);
    // SAFETY: as in `ReadyListener::create`, without overlapped I/O
    let pipe = owned(unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            512,
            512,
            0,
            &attributes,
        )
    })?;
    Ok(std::thread::spawn(move || {
        let handle = raw(&pipe);
        // SAFETY: the pipe, blocking; a client already connected counts
        if unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) } == 0 && last_error() != ERROR_PIPE_CONNECTED {
            return;
        }
        let mut buffer = vec![0u8; 511];
        let mut read = 0u32;
        // SAFETY: `buffer` holds 511 bytes
        let ok = unsafe { ReadFile(handle, buffer.as_mut_ptr(), 511, &mut read, std::ptr::null_mut()) };
        let message = if ok == 0 {
            Vec::new()
        } else {
            buffer[..read as usize].to_vec()
        };
        if let Some(answer) = reply(&message) {
            let mut written = 0u32;
            let len = u32::try_from(answer.len()).unwrap_or(0);
            // SAFETY: `answer` alive for the call
            unsafe {
                WriteFile(handle, answer.as_ptr(), len, &mut written, std::ptr::null_mut());
                FlushFileBuffers(handle);
            }
        }
        // SAFETY: the connected pipe
        unsafe { DisconnectNamedPipe(handle) };
    }))
}
