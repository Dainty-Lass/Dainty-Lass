//! The per-port log file (R0 4.3): one `WriteFile` per line, a handle reopened when
//! it goes stale, and a private open-append-close whenever the shared handle is busy, so logging never deadlocks.

use super::flags::path_next_to_exe;
use super::win::{local_time, pid, sleep_ms, tick, yield_now};
use rustic_isles_runtime_core::log::{file_name, format_line, format_raw, writable};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard, TryLockError};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileInformationByHandle,
};

const IDENTITY_CHECK_MS: u64 = 5000;
const LOCK_TRIES: u32 = 50;
const MAX_DEFERRED: usize = 256;

static READY: AtomicBool = AtomicBool::new(false);
static PORT: AtomicU32 = AtomicU32::new(0);
static WRITER: Mutex<Writer> = Mutex::new(Writer {
    file: None,
    port: 0,
    checked: 0,
});
static DEFERRED: Mutex<Vec<String>> = Mutex::new(Vec::new());

struct Writer {
    file: Option<File>,
    port: u16,
    checked: u64,
}

/// The port the log is named for (R1's launch port, 0 on a client).
pub fn set_port(port: u16) {
    PORT.store(u32::from(port), Ordering::SeqCst);
}

fn port() -> u16 {
    u16::try_from(PORT.load(Ordering::SeqCst)).unwrap_or(0)
}

/// Init is done: lines may be written (never under the loader lock, R0 3.1).
pub fn set_ready() {
    READY.store(true, Ordering::SeqCst);
}

fn enabled() -> bool {
    READY.load(Ordering::SeqCst) && !super::detaching()
}

/// A prefixed line in this process's port log.
pub fn line(message: &str) {
    if enabled() {
        write(port(), &format_line(local_time(), pid(), message));
    }
}

/// A raw line (no prefix) in this process's port log.
pub fn raw(message: &str) {
    raw_for_port(port(), message);
}

/// A raw line in another port's log, through a private open so the shared handle never thrashes.
pub fn raw_for_port(port: u16, message: &str) {
    if enabled() {
        write(port, &format_raw(message));
    }
}

/// Keeps a line for after the status lines: installs run under the loader lock, where nothing is written.
pub fn defer(message: String) {
    if let Ok(mut deferred) = DEFERRED.lock()
        && deferred.len() < MAX_DEFERRED
    {
        deferred.push(message);
    }
}

/// Writes the deferred lines.
pub fn flush_deferred() {
    let lines = DEFERRED
        .lock()
        .map(|mut deferred| std::mem::take(&mut *deferred))
        .unwrap_or_default();
    for message in lines {
        line(&message);
    }
}

fn open(port: u16) -> Option<File> {
    open_append(&path_next_to_exe(&file_name(port))?)
}

// `append`, not `access_mode(FILE_APPEND_DATA)`: std refuses `create` without write or append access
fn open_append(path: &std::path::Path) -> Option<File> {
    OpenOptions::new()
        .append(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .create(true)
        .open(path)
        .ok()
}

fn identity(file: &File) -> Option<(u32, u32, u32, u32)> {
    // SAFETY: plain data the call fills, on a handle the File owns
    let info: BY_HANDLE_FILE_INFORMATION = unsafe {
        let mut info = std::mem::zeroed();
        if GetFileInformationByHandle(file.as_raw_handle(), &mut info) == 0 {
            return None;
        }
        info
    };
    Some((
        info.dwVolumeSerialNumber,
        info.nFileIndexHigh,
        info.nFileIndexLow,
        info.nNumberOfLinks,
    ))
}

/// The open handle no longer names the file at the path: deleted (no links) or rotated away.
fn stale(file: &File, port: u16) -> bool {
    let Some((volume, high, low, links)) = identity(file) else {
        return true;
    };
    if links == 0 {
        return true;
    }
    let at_path = path_next_to_exe(&file_name(port))
        .and_then(|path| OpenOptions::new().access_mode(0).share_mode(7).open(path).ok())
        .and_then(|file| identity(&file));
    !matches!(at_path, Some((v, h, l, _)) if (v, h, l) == (volume, high, low))
}

fn lock_briefly() -> Option<MutexGuard<'static, Writer>> {
    for attempt in 0..LOCK_TRIES {
        match WRITER.try_lock() {
            Ok(guard) => return Some(guard),
            Err(TryLockError::Poisoned(poisoned)) => return Some(poisoned.into_inner()),
            Err(TryLockError::WouldBlock) => match attempt {
                0..4 => std::hint::spin_loop(),
                4..8 => yield_now(),
                _ => sleep_ms(1),
            },
        }
    }
    None
}

fn private_append(port: u16, text: &str) {
    if let Some(mut file) = open(port) {
        let _ = file.write(text.as_bytes());
    }
}

fn write(port: u16, text: &str) {
    if !writable(text) {
        return;
    }
    // Another port's line never moves the shared handle
    if port != self::port() {
        private_append(port, text);
        return;
    }
    let Some(mut writer) = lock_briefly() else {
        private_append(port, text);
        return;
    };
    if writer.port != port {
        writer.file = None;
    }
    let now = tick();
    if let Some(file) = &writer.file
        && now.saturating_sub(writer.checked) >= IDENTITY_CHECK_MS
    {
        let stale = stale(file, port);
        writer.checked = now;
        if stale {
            writer.file = None;
        }
    }
    for _ in 0..2 {
        if writer.file.is_none() {
            writer.file = open(port);
            writer.port = port;
            writer.checked = now;
        }
        let Some(file) = writer.file.as_mut() else {
            return;
        };
        // One WriteFile per line, so no other writer's line lands inside it
        if file.write(text.as_bytes()).is_ok_and(|written| written == text.len()) {
            return;
        }
        writer.file = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_append_creates_then_appends() {
        let path = std::env::temp_dir().join(format!("rustic_isles_log_test_{}.log", std::process::id()));
        let _ = std::fs::remove_file(&path);
        open_append(&path).unwrap().write_all(b"one\n").unwrap();
        open_append(&path).unwrap().write_all(b"two\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\ntwo\n");
        std::fs::remove_file(&path).unwrap();
    }
}
