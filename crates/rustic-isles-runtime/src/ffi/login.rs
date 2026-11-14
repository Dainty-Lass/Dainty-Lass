//! R4's detour bodies and raw reads (spec-runtime-r4): CL-1 the engine command line, CL-2 `IsEOSEnabled`, CL-5
//! EAC's error report, CL-7 the EAC setting, CL-8/9 the latency-test DNS names. All band `Mit`, order 0.

use super::broker::{self, NextCell};
use super::memory::{query, read, read_bytes};
use super::{detaching, log, minhook, win};
use rustic_isles_runtime_core::login::eac::{self, Cl7};
use rustic_isles_runtime_core::login::qos;
use rustic_isles_runtime_core::offsets::{FoundBy, data, target};
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};

type GetLineFn = unsafe extern "C" fn() -> *const u16;
type IsEosFn = unsafe extern "C" fn() -> u8;
type HandleErrorFn = unsafe extern "C" fn(*mut c_void, *const c_void) -> u64;
type GetAddrInfoWFn = unsafe extern "system" fn(*const u16, *const u16, *const c_void, *mut *mut c_void) -> i32;
type GetAddrInfoAFn = unsafe extern "system" fn(*const u8, *const u8, *const c_void, *mut *mut c_void) -> i32;

/// The engine's command line buffer is far shorter; a line with no NUL within this is passed through.
const MAX_LINE_UNITS: usize = 32_768;
const CHUNK_BYTES: usize = 4096;

// One referenced function makes the loader map ws2_32.dll before DllMain, so CL-8/9 never miss it (R4 call 7)
#[link(name = "ws2_32", kind = "raw-dylib")]
unsafe extern "system" {
    fn WSAGetLastError() -> i32;
}

/// Keeps the static `ws2_32.dll` import.
pub fn ws2_32_anchor() -> usize {
    std::hint::black_box(WSAGetLastError as *const () as usize)
}

/// Up to `max_units` units of `unit` bytes from `ptr`, region by region, stopping after the first NUL unit or the
/// first unreadable byte.
fn read_units(ptr: usize, unit: usize, max_units: usize) -> Vec<u8> {
    let want = max_units * unit;
    let mut out: Vec<u8> = Vec::new();
    let mut at = ptr;
    while out.len() < want {
        let Some(region) = query(at) else {
            break;
        };
        let end = region.base.saturating_add(region.size);
        let take = end.saturating_sub(at).min(want - out.len()).min(CHUNK_BYTES);
        let take = take - take % unit;
        if take == 0 {
            break;
        }
        let mut chunk = vec![0u8; take];
        if !read_bytes(at, &mut chunk) {
            break;
        }
        let nul = chunk.chunks_exact(unit).any(|unit| unit.iter().all(|&byte| byte == 0));
        out.extend_from_slice(&chunk);
        at += take;
        if nul {
            break;
        }
    }
    out
}

fn to_wide(bytes: &[u8]) -> Vec<u16> {
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect()
}

/// A wide string up to its NUL, or `None` when no NUL comes within `max_units`.
fn wide_until_nul(ptr: usize, max_units: usize) -> Option<Vec<u16>> {
    let mut units = to_wide(&read_units(ptr, 2, max_units));
    let end = units.iter().position(|&unit| unit == 0)?;
    units.truncate(end);
    Some(units)
}

/// CL-1's state: its next cell, the frozen line and how a line is built.
pub struct CommandLinePoint {
    /// The wrapper's next.
    pub next: NextCell,
    served: OnceLock<&'static [u16]>,
    build: fn(&[u16]) -> Vec<u16>,
}

impl CommandLinePoint {
    /// A point with no line yet; `build` turns the engine's line into the served one (no NUL).
    pub const fn new(build: fn(&[u16]) -> Vec<u16>) -> Self {
        Self {
            next: NextCell::new(),
            served: OnceLock::new(),
            build,
        }
    }
}

/// CL-1 in this process.
pub static CL1: CommandLinePoint = CommandLinePoint::new(crate::login::cmdline::serve);

/// The CL-1 body (R4 3.2): the first non-empty engine line is served with the switches, then the same pointer on
/// every call, after detach too (a static buffer: returning it neither allocates nor logs).
pub(crate) fn command_line(point: &CommandLinePoint) -> *const u16 {
    let next = point.next.load();
    if next == 0 {
        return std::ptr::null();
    }
    // SAFETY: the broker wrote a function of FCommandLine::Get's signature into this cell
    let original = unsafe { std::mem::transmute::<usize, GetLineFn>(next)() };
    if original.is_null() {
        return original;
    }
    if let Some(line) = point.served.get() {
        return line.as_ptr();
    }
    if detaching() {
        return original;
    }
    let served = catch_unwind(AssertUnwindSafe(|| {
        // An empty answer is not frozen (R4 departure 1): the engine's real line comes later
        let engine = wide_until_nul(original as usize, MAX_LINE_UNITS).filter(|line| !line.is_empty())?;
        let line = point.served.get_or_init(|| {
            let mut line = (point.build)(&engine);
            line.push(0);
            // The engine keeps the pointer: never freed
            Vec::leak(line)
        });
        Some(line.as_ptr())
    }));
    served.ok().flatten().unwrap_or(original)
}

unsafe extern "C" fn cl1_detour() -> *const u16 {
    command_line(&CL1)
}

static CL2_NEXT: NextCell = NextCell::new();
static CL2_LOGGED: AtomicBool = AtomicBool::new(false);

/// CL-2 (R4 4): absorbed with 0, so the client takes the MCP login through the engine's HTTP stack.
unsafe extern "C" fn cl2_detour() -> u8 {
    if detaching() {
        let next = CL2_NEXT.load();
        if next == 0 {
            return 0;
        }
        // SAFETY: the broker wrote a function of IsEOSEnabled's signature into this cell
        return unsafe { std::mem::transmute::<usize, IsEosFn>(next)() };
    }
    if !CL2_LOGGED.swap(true, Ordering::SeqCst) {
        let _ = catch_unwind(|| {
            crate::lifecycle::write_status_once();
            log::line("[MitRuntime] [Login] IsEOSEnabled -> 0 (MCP login)");
        });
    }
    0
}

/// CL-5's state.
pub struct EacPoint {
    /// The wrapper's next.
    pub next: NextCell,
    absorbed: AtomicU32,
    forwarded: AtomicU32,
}

impl EacPoint {
    /// A point with nothing logged yet.
    pub const fn new() -> Self {
        Self {
            next: NextCell::new(),
            absorbed: AtomicU32::new(0),
            forwarded: AtomicU32::new(0),
        }
    }
}

impl Default for EacPoint {
    fn default() -> Self {
        Self::new()
    }
}

static CL5: EacPoint = EacPoint::new();

/// The `FString`'s text: `min(Num, 512)` units minus the last; empty on any failed read.
fn fstring_text(message: usize) -> Vec<u16> {
    if !super::memory::readable(message, 0x10) {
        return Vec::new();
    }
    let (Some(data), Some(num)) = (read::<usize>(message), read::<i32>(message + 8)) else {
        return Vec::new();
    };
    let count = eac::units_to_read(num);
    if count == 0 {
        return Vec::new();
    }
    let mut bytes = vec![0u8; count * 2];
    if !read_bytes(data, &mut bytes) {
        return Vec::new();
    }
    eac::message_text(&to_wide(&bytes)).to_vec()
}

fn log_counted(counter: &AtomicU32, line: impl FnOnce() -> String) {
    if counter.fetch_add(1, Ordering::SeqCst) < eac::LOGGED {
        crate::lifecycle::write_status_once();
        log::line(&line());
    }
}

/// The CL-5 body (R4 5.2): the expected client-init error is absorbed with 1; any other goes on to the game.
pub(crate) fn handle_error(point: &EacPoint, this: *mut c_void, message: *const c_void) -> u64 {
    let next = point.next.load();
    if next == 0 {
        return 0;
    }
    // SAFETY: the broker wrote a function of HandleError's signature into this cell
    let next: HandleErrorFn = unsafe { std::mem::transmute::<usize, HandleErrorFn>(next) };
    // SAFETY: the original arguments
    let call_next = || unsafe { next(this, message) };
    if detaching() {
        return call_next();
    }
    let text = catch_unwind(|| fstring_text(message as usize)).unwrap_or_default();
    if eac::absorbs(&text) {
        let _ = catch_unwind(|| log_counted(&point.absorbed, || eac::absorbed_line(&text)));
        return 1;
    }
    let _ = catch_unwind(|| log_counted(&point.forwarded, || eac::forwarded_line(&text)));
    call_next()
}

unsafe extern "C" fn cl5_detour(this: *mut c_void, message: *const c_void) -> u64 {
    handle_error(&CL5, this, message)
}

static LOOPBACK_W: [u16; 10] = [49, 50, 55, 46, 48, 46, 48, 46, 49, 0];
static LOOPBACK_A: [u8; 10] = *b"127.0.0.1\0";
static QOS_LOGGED: AtomicU32 = AtomicU32::new(0);

/// Whether the node at `node` (units of `unit` bytes) is a latency-test host; logs the first rewrites.
fn qos_rewrites(node: usize, unit: usize) -> bool {
    catch_unwind(|| {
        let bytes = read_units(node, unit, qos::MAX_NODE_UNITS);
        let units: Vec<u16> = if unit == 2 {
            to_wide(&bytes)
        } else {
            bytes.iter().map(|&byte| u16::from(byte)).collect()
        };
        let Some(name) = qos::node_name(&units) else {
            return false;
        };
        let rewrites = qos::rewrites(name);
        if rewrites && QOS_LOGGED.fetch_add(1, Ordering::SeqCst) < qos::LOGGED {
            crate::lifecycle::write_status_once();
            log::line(&qos::rewrite_line(name));
        }
        rewrites
    })
    .unwrap_or(false)
}

/// The CL-8 body (R4 6.2).
pub(crate) fn get_addr_info_w(
    next: &NextCell,
    node: *const u16,
    service: *const u16,
    hints: *const c_void,
    result: *mut *mut c_void,
) -> i32 {
    let next = next.load();
    if next == 0 {
        return -1;
    }
    // SAFETY: the broker wrote a function of GetAddrInfoW's signature into this cell
    let next: GetAddrInfoWFn = unsafe { std::mem::transmute::<usize, GetAddrInfoWFn>(next) };
    let node = if !detaching() && !node.is_null() && qos_rewrites(node as usize, 2) {
        LOOPBACK_W.as_ptr()
    } else {
        node
    };
    // SAFETY: the original arguments, the node perhaps replaced by a static NUL-terminated string
    unsafe { next(node, service, hints, result) }
}

/// The CL-9 body (R4 6.2).
pub(crate) fn get_addr_info_a(
    next: &NextCell,
    node: *const u8,
    service: *const u8,
    hints: *const c_void,
    result: *mut *mut c_void,
) -> i32 {
    let next = next.load();
    if next == 0 {
        return -1;
    }
    // SAFETY: the broker wrote a function of getaddrinfo's signature into this cell
    let next: GetAddrInfoAFn = unsafe { std::mem::transmute::<usize, GetAddrInfoAFn>(next) };
    let node = if !detaching() && !node.is_null() && qos_rewrites(node as usize, 1) {
        LOOPBACK_A.as_ptr()
    } else {
        node
    };
    // SAFETY: as above
    unsafe { next(node, service, hints, result) }
}

static CL8_NEXT: NextCell = NextCell::new();
static CL9_NEXT: NextCell = NextCell::new();

unsafe extern "system" fn cl8_detour(
    node: *const u16,
    service: *const u16,
    hints: *const c_void,
    result: *mut *mut c_void,
) -> i32 {
    get_addr_info_w(&CL8_NEXT, node, service, hints, result)
}

unsafe extern "system" fn cl9_detour(
    node: *const u8,
    service: *const u8,
    hints: *const c_void,
    result: *mut *mut c_void,
) -> i32 {
    get_addr_info_a(&CL9_NEXT, node, service, hints, result)
}

/// CL-7's write rule on `address` (R4 5.1).
pub fn write_eac_setting(address: usize) -> Cl7 {
    let decision = eac::cl7_decision(address, query(address));
    if decision == Cl7::Write {
        // SAFETY: one byte in a committed page whose protection allows writing, just checked
        unsafe { std::ptr::write_volatile(address as *mut u8, 0) };
    }
    decision
}

fn detour_for(id: &str) -> Option<(usize, &'static NextCell)> {
    Some(match id {
        "CL-1" => (cl1_detour as *const () as usize, &CL1.next),
        "CL-2" => (cl2_detour as *const () as usize, &CL2_NEXT),
        "CL-5" => (cl5_detour as *const () as usize, &CL5.next),
        "CL-8" => (cl8_detour as *const () as usize, &CL8_NEXT),
        "CL-9" => (cl9_detour as *const () as usize, &CL9_NEXT),
        _ => return None,
    })
}

fn export(module: &str, procedure: &str) -> Result<usize, &'static str> {
    let module = win::to_wide(module);
    // SAFETY: a NUL-terminated name; no library is loaded (R0's Init never loads one)
    let handle = unsafe { GetModuleHandleW(module.as_ptr()) };
    if handle.is_null() {
        return Err("ws2_32 not loaded");
    }
    let mut name = procedure.as_bytes().to_vec();
    name.push(0);
    // SAFETY: a module handle and a NUL-terminated name
    match unsafe { GetProcAddress(handle, name.as_ptr()) } {
        Some(function) => Ok(function as usize),
        None => Err("export not found"),
    }
}

/// Installs one R4 id; the error is the status line's reason.
pub fn install(id: &str, base: usize) -> Result<(), &'static str> {
    if id == "CL-7" {
        let address = usize::try_from(data::EAC_ENABLED)
            .ok()
            .and_then(|rva| base.checked_add(rva))
            .unwrap_or(0);
        return write_eac_setting(address).status();
    }
    let unknown = || minhook::status_name(minhook::MH_UNKNOWN);
    let line = target(id).ok_or_else(unknown)?;
    let (wrapper, next) = detour_for(id).ok_or_else(unknown)?;
    let address = match line.found_by {
        FoundBy::Export(module, procedure) => {
            ws2_32_anchor();
            export(module, procedure)?
        }
        _ => usize::try_from(line.rva)
            .ok()
            .and_then(|rva| base.checked_add(rva))
            .ok_or_else(unknown)?,
    };
    broker::hook(address, wrapper, 0, next).map_err(minhook::status_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustic_isles_runtime_core::login::cmdline::build_served_line;
    use std::hint::black_box;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_NOACCESS, PAGE_PROTECTION_FLAGS, PAGE_READONLY, PAGE_READWRITE,
        VirtualAlloc, VirtualFree,
    };

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn read_line(ptr: *const u16) -> String {
        String::from_utf16_lossy(&wide_until_nul(ptr as usize, MAX_LINE_UNITS).unwrap())
    }

    static FAKE_LINE: AtomicUsize = AtomicUsize::new(0);

    #[inline(never)]
    extern "C" fn fake_get_line() -> *const u16 {
        black_box(FAKE_LINE.load(Ordering::SeqCst)) as *const u16
    }

    fn client_build(engine: &[u16]) -> Vec<u16> {
        build_served_line(engine, &[], &[], false, false).line
    }

    static TEST_CL1: CommandLinePoint = CommandLinePoint::new(client_build);

    unsafe extern "C" fn test_cl1_detour() -> *const u16 {
        command_line(&TEST_CL1)
    }

    #[test]
    fn command_line_served_once_non_empty() {
        let target = fake_get_line as *const () as usize;
        broker::hook(target, test_cl1_detour as *const () as usize, 0, &TEST_CL1.next).unwrap();
        let get = || black_box(fake_get_line as extern "C" fn() -> *const u16)();

        FAKE_LINE.store(0, Ordering::SeqCst);
        assert!(get().is_null(), "null passes through");
        let empty = wide("");
        FAKE_LINE.store(empty.as_ptr() as usize, Ordering::SeqCst);
        assert_eq!(get(), empty.as_ptr(), "an empty answer passes through");
        assert!(TEST_CL1.served.get().is_none(), "and is not frozen");

        let first = wide("game.exe -AUTH_TYPE=exchangecode");
        FAKE_LINE.store(first.as_ptr() as usize, Ordering::SeqCst);
        let served = get();
        assert_ne!(served, first.as_ptr());
        assert_eq!(read_line(served), "game.exe -AUTH_TYPE=exchangecode -NoEAC -DisableEOS");

        let changed = wide("other.exe");
        FAKE_LINE.store(changed.as_ptr() as usize, Ordering::SeqCst);
        assert_eq!(get(), served, "the same line on every later call");
        FAKE_LINE.store(0, Ordering::SeqCst);
        assert!(get().is_null());
    }

    #[derive(Debug, Default, Clone, PartialEq)]
    struct Seen {
        node: String,
        service: usize,
        hints: usize,
        result: usize,
    }

    static SEEN: Mutex<Option<Seen>> = Mutex::new(None);

    #[inline(never)]
    extern "system" fn fake_get_addr_info_w(
        node: *const u16,
        service: *const u16,
        hints: *const c_void,
        result: *mut *mut c_void,
    ) -> i32 {
        *SEEN.lock().unwrap() = Some(Seen {
            node: read_line(node),
            service: service as usize,
            hints: hints as usize,
            result: result as usize,
        });
        black_box(11)
    }

    static TEST_CL8: NextCell = NextCell::new();

    unsafe extern "system" fn test_cl8_detour(
        node: *const u16,
        service: *const u16,
        hints: *const c_void,
        result: *mut *mut c_void,
    ) -> i32 {
        get_addr_info_w(&TEST_CL8, node, service, hints, result)
    }

    #[test]
    fn qos_rewrites_the_latency_host() {
        let target = fake_get_addr_info_w as *const () as usize;
        broker::hook(target, test_cl8_detour as *const () as usize, 0, &TEST_CL8).unwrap();
        type F = extern "system" fn(*const u16, *const u16, *const c_void, *mut *mut c_void) -> i32;
        let call = |node: &[u16]| {
            let service = wide("80");
            let answer = black_box(fake_get_addr_info_w as F)(
                node.as_ptr(),
                service.as_ptr(),
                0x1234 as *const c_void,
                0x5678 as *mut *mut c_void,
            );
            assert_eq!(answer, 11);
            let seen = SEEN.lock().unwrap().take().unwrap();
            assert_eq!(
                (seen.service, seen.hints, seen.result),
                (service.as_ptr() as usize, 0x1234, 0x5678),
                "the other arguments are untouched"
            );
            seen.node
        };
        assert_eq!(call(&wide("euw11.gce.latency-test.steelyard.ca")), "127.0.0.1");
        assert_eq!(call(&wide("backend.example")), "backend.example");
        assert_eq!(
            call(&wide("GCE.LATENCY-TEST.STEELYARD.CA")),
            "GCE.LATENCY-TEST.STEELYARD.CA"
        );
    }

    static FAKE_EAC_CALLS: AtomicU32 = AtomicU32::new(0);

    #[inline(never)]
    extern "C" fn fake_handle_error(_this: *mut c_void, _message: *const c_void) -> u64 {
        FAKE_EAC_CALLS.fetch_add(1, Ordering::SeqCst);
        black_box(0x5500)
    }

    static TEST_CL5: EacPoint = EacPoint::new();

    unsafe extern "C" fn test_cl5_detour(this: *mut c_void, message: *const c_void) -> u64 {
        handle_error(&TEST_CL5, this, message)
    }

    #[repr(C, align(8))]
    struct FString {
        data: *const u16,
        num: i32,
        max: i32,
    }

    #[test]
    fn eac_absorbs_the_client_init_error() {
        let target = fake_handle_error as *const () as usize;
        broker::hook(target, test_cl5_detour as *const () as usize, 0, &TEST_CL5.next).unwrap();
        type F = extern "C" fn(*mut c_void, *const c_void) -> u64;
        let call = |text: &str| {
            let units = wide(text);
            let message = FString {
                data: units.as_ptr(),
                num: i32::try_from(units.len()).unwrap(),
                max: i32::try_from(units.len()).unwrap(),
            };
            let before = FAKE_EAC_CALLS.load(Ordering::SeqCst);
            let answer = black_box(fake_handle_error as F)(std::ptr::null_mut(), &message as *const FString as _);
            (answer, FAKE_EAC_CALLS.load(Ordering::SeqCst) - before)
        };
        assert_eq!(call("Failed to initialize IGameClient"), (1, 0));
        assert_eq!(call("Failed to create the client"), (1, 0));
        assert_eq!(call("igameclient"), (0x5500, 1));
        assert_eq!(call(""), (0x5500, 1));
        let null = black_box(fake_handle_error as F)(std::ptr::null_mut(), std::ptr::null());
        assert_eq!(null, 0x5500, "an unreadable message is forwarded");
    }

    struct Page(usize);

    impl Page {
        fn new(protect: PAGE_PROTECTION_FLAGS) -> Self {
            let base = unsafe { VirtualAlloc(std::ptr::null(), 4096, MEM_COMMIT | MEM_RESERVE, protect) };
            assert!(!base.is_null());
            Self(base as usize)
        }
    }

    impl Drop for Page {
        fn drop(&mut self) {
            unsafe { VirtualFree(self.0 as *mut _, 0, MEM_RELEASE) };
        }
    }

    #[test]
    fn cl7_write_rule() {
        let writable = Page::new(PAGE_READWRITE);
        unsafe { std::ptr::write_volatile((writable.0 + 0x44) as *mut u8, 1) };
        assert_eq!(write_eac_setting(writable.0 + 0x44), Cl7::Write);
        assert_eq!(super::super::memory::read_unaligned::<u8>(writable.0 + 0x44), Some(0));
        let read_only = Page::new(PAGE_READONLY);
        assert_eq!(write_eac_setting(read_only.0 + 0x44), Cl7::NotWritable);
        let no_access = Page::new(PAGE_NOACCESS);
        assert_eq!(write_eac_setting(no_access.0 + 0x44), Cl7::Unreadable);
        assert_eq!(write_eac_setting(0), Cl7::Unreadable);
    }

    #[test]
    fn read_units_stops_at_the_region_end() {
        let page = Page::new(PAGE_READWRITE);
        unsafe { std::ptr::write_bytes(page.0 as *mut u8, b'a', 4096) };
        assert_eq!(
            wide_until_nul(page.0, MAX_LINE_UNITS),
            None,
            "no NUL before the region ends"
        );
        assert_eq!(read_units(page.0 + 4090, 1, qos::MAX_NODE_UNITS).len(), 6);
        assert!(read_units(0, 2, 10).is_empty());
    }

    #[test]
    fn ws2_32_is_loaded() {
        assert_ne!(ws2_32_anchor(), 0);
        assert!(export("ws2_32.dll", "GetAddrInfoW").is_ok());
        assert!(export("ws2_32.dll", "getaddrinfo").is_ok());
        assert_eq!(export("ws2_32.dll", "NoSuchExport"), Err("export not found"));
        assert_eq!(export("no-such-module.dll", "x"), Err("ws2_32 not loaded"));
    }
}
