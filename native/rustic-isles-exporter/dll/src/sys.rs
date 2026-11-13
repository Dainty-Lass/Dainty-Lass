//! The only `unsafe` in the exporter: `DllMain`, the kernel32 and user32 calls, and the `ProcessEvent` call through
//! the object's vtable (spec §9, seam 1). Everything else reads memory through `Process`, which fails instead of
//! faulting.

use std::ffi::c_void;
use std::path::PathBuf;
use std::time::Duration;

use catalog_exporter::engine::Engine;
use catalog_exporter::flags::Flags;
use catalog_exporter::memory::{Calls, Memory};
use catalog_exporter::run::{self, Host};
use catalog_exporter::status::Status;
use catalog_exporter::{clock, paths, sdk};

type Handle = *mut c_void;

const DLL_PROCESS_ATTACH: u32 = 1;
const TRUE: i32 = 1;
const MB_OK: u32 = 0;
const MB_ICONINFORMATION: u32 = 0x40;
const MB_SETFOREGROUND: u32 = 0x1_0000;
const MB_TOPMOST: u32 = 0x4_0000;
const WM_CLOSE: u32 = 0x0010;
/// How long the closing notice waits before the game is asked to close anyway (spec §3 step 7).
const CLOSE_AFTER: Duration = Duration::from_secs(10);
/// How long the game gets to close its own way before it is ended (spec §3 step 7).
const END_AFTER: Duration = Duration::from_secs(15);

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetCurrentProcess() -> Handle;
    fn ReadProcessMemory(
        process: Handle,
        base: *const c_void,
        buffer: *mut c_void,
        size: usize,
        read: *mut usize,
    ) -> i32;
    fn GetModuleHandleW(name: *const u16) -> Handle;
    fn DisableThreadLibraryCalls(module: Handle) -> i32;
    fn OutputDebugStringA(text: *const u8);
    fn GetCurrentProcessId() -> u32;
    fn TerminateProcess(process: Handle, code: u32) -> i32;
}

#[link(name = "user32")]
unsafe extern "system" {
    fn MessageBeep(kind: u32) -> i32;
    fn MessageBoxW(owner: Handle, text: *const u16, caption: *const u16, kind: u32) -> i32;
    fn EnumWindows(callback: extern "system" fn(Handle, isize) -> i32, data: isize) -> i32;
    fn GetWindowThreadProcessId(window: Handle, process: *mut u32) -> u32;
    fn PostMessageW(window: Handle, message: u32, wparam: usize, lparam: isize) -> i32;
}

/// The game's own memory, read with `ReadProcessMemory` so an unmapped or freed page fails the read (spec §4.4).
pub struct Process;

impl Memory for Process {
    fn read(&self, addr: u64, buf: &mut [u8]) -> bool {
        if buf.is_empty() {
            return true;
        }
        let Ok(base) = usize::try_from(addr) else {
            return false;
        };
        let mut done = 0usize;
        // SAFETY: `buf` is valid for `buf.len()` bytes of writes; the source is only ever read by the kernel, which
        // reports an unreadable range as a failure rather than faulting this thread.
        let ok = unsafe {
            ReadProcessMemory(
                GetCurrentProcess(),
                base as *const c_void,
                buf.as_mut_ptr().cast(),
                buf.len(),
                &mut done,
            )
        };
        ok != 0 && done == buf.len()
    }
}

/// `ProcessEvent` at the SDK's vtable index (spec §4.2).
pub struct GameCalls {
    index: u64,
}

type ProcessEvent = unsafe extern "system" fn(Handle, Handle, *mut c_void);

impl Calls for GameCalls {
    fn process_event(&self, object: u64, function: u64, params: &mut [u64]) -> bool {
        let mut word = [0u8; 8];
        if !Process.read(object, &mut word) {
            return false;
        }
        let vtable = u64::from_le_bytes(word);
        if vtable == 0 || !Process.read(vtable + self.index * 8, &mut word) {
            return false;
        }
        let Ok(target) = usize::try_from(u64::from_le_bytes(word)) else {
            return false;
        };
        if target == 0 {
            return false;
        }
        let (Ok(object), Ok(function)) = (usize::try_from(object), usize::try_from(function)) else {
            return false;
        };
        // SAFETY: `target` is the object's own `ProcessEvent` slot at the SDK's index, read from its vtable; it
        // is called as the SDK's wrappers call it, `(this, UFunction*, params)`, with a parameter block sized and
        // laid out from the SDK and 8-aligned. Like the old exporter it runs on this worker thread, never the game's.
        unsafe {
            let call = std::mem::transmute::<usize, ProcessEvent>(target);
            call(object as Handle, function as Handle, params.as_mut_ptr().cast());
        }
        true
    }
}

fn image_base() -> u64 {
    // SAFETY: a null name asks for the process's own executable, which is loaded for the process's lifetime.
    let base = unsafe { GetModuleHandleW(std::ptr::null()) };
    base as u64
}

fn debug(text: &str) {
    let mut bytes: Vec<u8> = text.bytes().filter(|&byte| byte != 0).collect();
    bytes.push(0);
    // SAFETY: `bytes` is NUL-terminated and outlives the call.
    unsafe { OutputDebugStringA(bytes.as_ptr()) };
}

fn beep() {
    // SAFETY: no pointers; plays the default sound.
    unsafe { MessageBeep(MB_OK) };
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Says the export is done, then closes the game once the notice is dismissed or [`CLOSE_AFTER`] has passed (spec §3
/// step 7): a box behind a fullscreen game must not hold it open.
fn close_game() {
    let (done, dismissed) = std::sync::mpsc::channel();
    let _ = std::thread::Builder::new()
        .name("CatalogExporter notice".to_owned())
        .spawn(move || {
            let text = wide("Export complete, closing the game.");
            let caption = wide("CatalogExporter");
            // SAFETY: both strings are NUL-terminated UTF-16 that outlive the call; no owner window.
            unsafe {
                MessageBoxW(
                    std::ptr::null_mut(),
                    text.as_ptr(),
                    caption.as_ptr(),
                    MB_OK | MB_ICONINFORMATION | MB_SETFOREGROUND | MB_TOPMOST,
                )
            };
            let _ = done.send(());
        });
    let _ = dismissed.recv_timeout(CLOSE_AFTER);
    debug("CatalogExporter: closing the game");
    // The game's own quit, as its window's close button: ExitProcess from here faults in libcef's teardown
    // SAFETY: the callback only reads the window handle and posts to windows of this process.
    unsafe { EnumWindows(close_own_window, 0) };
    std::thread::sleep(END_AFTER);
    debug("CatalogExporter: the game did not close; ending it");
    // SAFETY: ends this process with code 0 and runs no DLL teardown, which is what faulted.
    unsafe { TerminateProcess(GetCurrentProcess(), 0) };
}

/// Posts `WM_CLOSE` to each top-level window of this process.
extern "system" fn close_own_window(window: Handle, _data: isize) -> i32 {
    let mut owner = 0u32;
    // SAFETY: `window` comes from EnumWindows and `owner` is a valid out pointer.
    unsafe { GetWindowThreadProcessId(window, &raw mut owner) };
    // SAFETY: no pointers; GetCurrentProcessId takes none.
    if owner == unsafe { GetCurrentProcessId() } {
        // SAFETY: posting a message copies its arguments; the window may be gone, which only fails the post.
        unsafe { PostMessageW(window, WM_CLOSE, 0, 0) };
    }
    TRUE
}

/// # Safety
///
/// Called by the Windows loader only.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllMain(module: Handle, reason: u32, _reserved: *mut c_void) -> i32 {
    if reason == DLL_PROCESS_ATTACH {
        // SAFETY: `module` is this DLL's handle, given by the loader.
        unsafe { DisableThreadLibraryCalls(module) };
        // The worker starts once the loader lock is released; DllMain returns at once (spec §3.1).
        let _ = std::thread::Builder::new()
            .name("CatalogExporter".to_owned())
            .spawn(worker);
    }
    TRUE
}

fn worker() {
    // A panic ends the export, never the game (spec §12 call 4).
    let _ = std::panic::catch_unwind(work);
}

fn work() {
    let out = || -> PathBuf { paths::output_dir(paths::root()) };
    if sdk::SDK_SYNTHETIC {
        Status::new(&out(), debug).line("", "built without DAINTY_SDK_DIR (the synthetic SDK): nothing exported");
        return;
    }
    let flags = paths::find_flags(paths::root())
        .and_then(|path| std::fs::read(path).ok())
        .map_or_else(Flags::defaults, |bytes| Flags::parse(&bytes));
    let base = image_base();
    let (Ok(gobjects), Ok(names), Ok(index)) = (
        u64::try_from(sdk::GOBJECTS),
        u64::try_from(sdk::GNAMES),
        u64::try_from(sdk::PROCESS_EVENT_INDEX),
    ) else {
        return;
    };
    let calls = GameCalls { index };
    let engine = Engine::new(&Process, &calls, base + gobjects, base + names);
    let host = Host {
        out: &out,
        debug,
        sleep: &|step: Duration| std::thread::sleep(step),
        now: &clock::Utc::now,
    };
    if run::run(&engine, &flags, &host).is_some() {
        beep();
        close_game();
    }
}
