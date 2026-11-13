//! Safe wrappers over the Win32 calls the runtime makes: environment, command line, time, process.

use rustic_isles_runtime_core::env::{Raw, check_env_value};
use rustic_isles_runtime_core::log::LineTime;
use std::ffi::c_void;
use windows_sys::Win32::Foundation::{HLOCAL, LocalFree, SYSTEMTIME};
use windows_sys::Win32::System::Environment::{
    FreeEnvironmentStringsW, GetCommandLineW, GetEnvironmentStringsW, GetEnvironmentVariableW, SetEnvironmentVariableW,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::SystemInformation::{GetLocalTime, GetTickCount64};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId, Sleep, SwitchToThread, TerminateProcess,
};
use windows_sys::Win32::UI::Shell::CommandLineToArgvW;

/// A NUL-terminated UTF-16 copy of `text`.
pub fn to_wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The units of a NUL-terminated UTF-16 string.
///
/// # Safety
/// `ptr` is null or points at a NUL-terminated UTF-16 string.
unsafe fn units_until_nul(ptr: *const u16) -> Vec<u16> {
    if ptr.is_null() {
        return Vec::new();
    }
    let mut len = 0;
    // SAFETY: the caller promises a terminator
    unsafe {
        while *ptr.add(len) != 0 {
            len += 1;
        }
        std::slice::from_raw_parts(ptr, len).to_vec()
    }
}

/// The variable's units, or `Ok(None)` when unset; `Err(len)` when longer than `cap` (the value is never read).
fn env_units(name: &str, cap: usize) -> Result<Option<Vec<u16>>, usize> {
    let name = to_wide(name);
    for _ in 0..4 {
        // SAFETY: a null buffer of size 0 asks for the size
        let needed = unsafe { GetEnvironmentVariableW(name.as_ptr(), std::ptr::null_mut(), 0) } as usize;
        if needed == 0 {
            return Ok(None);
        }
        let len = needed - 1;
        if len > cap {
            return Err(len);
        }
        let mut buffer = vec![0u16; needed];
        let size = u32::try_from(needed).unwrap_or(u32::MAX);
        // SAFETY: `buffer` holds `size` units
        let written = unsafe { GetEnvironmentVariableW(name.as_ptr(), buffer.as_mut_ptr(), size) } as usize;
        if written < needed {
            buffer.truncate(written);
            return Ok(Some(buffer));
        }
        // The value grew between the calls: ask again
    }
    Ok(None)
}

/// R0's `env_value(name, max)`.
pub fn env_value(name: &str, max: usize) -> Option<String> {
    match env_units(name, max) {
        Ok(units) => check_env_value(units.as_deref(), max),
        Err(_) => None,
    }
}

/// R1's raw read with a cap in UTF-16 units.
pub fn env_raw(name: &str, cap: usize) -> Raw {
    match env_units(name, cap) {
        Ok(units) => Raw::from_units(units.as_deref(), cap),
        Err(len) => Raw::TooLong(len),
    }
}

/// Whether the variable exists, an empty one included (R1 3).
pub fn env_exists(name: &str) -> bool {
    let name = to_wide(name);
    // SAFETY: a null buffer of size 0 asks for the size
    unsafe { GetEnvironmentVariableW(name.as_ptr(), std::ptr::null_mut(), 0) != 0 }
}

/// Sets a variable in this process.
pub fn set_env(name: &str, value: &str) -> bool {
    let (name, value) = (to_wide(name), to_wide(value));
    // SAFETY: both are NUL-terminated
    unsafe { SetEnvironmentVariableW(name.as_ptr(), value.as_ptr()) != 0 }
}

/// The environment block's entries, in block order (entries that are not valid UTF-16 are skipped).
pub fn env_block() -> Vec<String> {
    // SAFETY: the block is a list of NUL-terminated strings ending with an empty one; freed once below
    unsafe {
        let block = GetEnvironmentStringsW();
        if block.is_null() {
            return Vec::new();
        }
        let mut entries = Vec::new();
        let mut at = block.cast_const();
        loop {
            let entry = units_until_nul(at);
            if entry.is_empty() {
                break;
            }
            at = at.add(entry.len() + 1);
            if let Ok(text) = String::from_utf16(&entry) {
                entries.push(text);
            }
        }
        FreeEnvironmentStringsW(block);
        entries
    }
}

/// `GetCommandLineW` as units. Never logged: it carries the exchange code or the deploy key.
pub fn command_line() -> Vec<u16> {
    // SAFETY: the process's command line is NUL-terminated and lives for the process
    unsafe { units_until_nul(GetCommandLineW()) }
}

/// `CommandLineToArgvW(GetCommandLineW())`, every entry copied (a pointer into the argv block crashed the AGPL).
pub fn argv() -> Vec<Vec<u16>> {
    let mut count = 0i32;
    // SAFETY: the array of `count` NUL-terminated strings is freed once with LocalFree after the copies
    unsafe {
        let list = CommandLineToArgvW(GetCommandLineW(), &mut count);
        if list.is_null() {
            return Vec::new();
        }
        let entries = (0..usize::try_from(count).unwrap_or(0))
            .map(|index| units_until_nul(*list.add(index)))
            .collect();
        LocalFree(list as HLOCAL);
        entries
    }
}

/// The game executable's image base.
pub fn image_base() -> usize {
    // SAFETY: null asks for the executable
    unsafe { GetModuleHandleW(std::ptr::null()) as usize }
}

/// `GetTickCount64`.
pub fn tick() -> u64 {
    // SAFETY: no arguments
    unsafe { GetTickCount64() }
}

/// The local time of day.
pub fn local_time() -> LineTime {
    // SAFETY: SYSTEMTIME is plain data GetLocalTime fills
    let time: SYSTEMTIME = unsafe {
        let mut time = std::mem::zeroed();
        GetLocalTime(&mut time);
        time
    };
    LineTime {
        hour: time.wHour,
        minute: time.wMinute,
        second: time.wSecond,
        millis: time.wMilliseconds,
    }
}

/// This process's id.
pub fn pid() -> u32 {
    // SAFETY: no arguments
    unsafe { GetCurrentProcessId() }
}

/// This thread's id.
pub fn thread_id() -> u32 {
    // SAFETY: no arguments
    unsafe { GetCurrentThreadId() }
}

/// Ends the process now with `code`.
pub fn terminate(code: u32) {
    // SAFETY: the pseudo-handle of this process
    unsafe {
        TerminateProcess(GetCurrentProcess(), code);
    }
}

/// Gives up the rest of this time slice.
pub fn yield_now() {
    // SAFETY: no arguments
    unsafe {
        SwitchToThread();
    }
}

/// Sleeps `ms` milliseconds.
pub fn sleep_ms(ms: u32) {
    // SAFETY: no arguments
    unsafe { Sleep(ms) }
}

/// An address as the `*mut c_void` C APIs take.
pub fn void(address: usize) -> *mut c_void {
    address as *mut c_void
}
