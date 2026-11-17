//! R9's raw half (spec-runtime-r9): the SRV-1 detour and its write into the engine's string, the engine calls
//! (`CreateNamedNetDriver`, `InitListen`, `FMemory::Realloc`) and the ready lines' stdout write. The decisions are `core::listen`'s; the steps are `crate::listen`'s.

use super::broker::{self, NextCell};
use super::memory::{function_entry, read, read_bytes, readable};
use super::{detaching, log, minhook, win};
use rustic_isles_runtime_core::listen::{MAX_ERROR_UNITS, fstring_plan, lines};
use rustic_isles_runtime_core::offsets::{data, r9, target};
use std::ffi::c_void;
use std::fs::File;
use std::io::Write;
use std::mem::ManuallyDrop;
use std::os::windows::io::FromRawHandle;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

type GetMapFn = unsafe extern "C" fn(*mut EngineString) -> *mut EngineString;
type ReallocFn = unsafe extern "C" fn(*mut c_void, usize, u32) -> *mut c_void;
type CreateNamedNetDriverFn = unsafe extern "C" fn(*mut c_void, *mut c_void, u64, u64) -> u8;
type InitListenFn = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void, u8, *mut EngineString) -> u8;

// Not in the windows-sys features this crate enables; kernel32 is mapped in every process anyway
#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn GetStdHandle(which: u32) -> *mut c_void;
}

/// `STD_OUTPUT_HANDLE`.
const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;

/// The engine's `FString`: `num` counts the NUL.
#[repr(C)]
#[derive(Debug)]
pub struct EngineString {
    /// UTF-16 units.
    pub data: *mut u16,
    /// Units with the NUL.
    pub num: i32,
    /// Capacity in units.
    pub max: i32,
}

impl EngineString {
    /// An empty string, as the engine default-constructs one.
    pub const fn empty() -> Self {
        Self {
            data: std::ptr::null_mut(),
            num: 0,
            max: 0,
        }
    }
}

static SRV1_NEXT: NextCell = NextCell::new();
static REALLOC: AtomicUsize = AtomicUsize::new(0);
static ALLOCATION_LOGGED: AtomicBool = AtomicBool::new(false);

/// What SRV-1 did with the engine's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Written {
    /// The URL is in the engine's string.
    Written,
    /// The allocator answered null: the engine's own map stays.
    AllocationFailed,
    /// The string could not be read.
    Unreadable,
}

/// Writes `url` (with its NUL) into the engine-owned string at `string`, growing it through `realloc` (an
/// `FMemory::Realloc`) when it is short: the engine later grows or frees it, so the bytes must be its own (4.4).
pub fn write_map_url(string: usize, url: &[u16], realloc: usize) -> Written {
    // `num` and `max` share one aligned word: `read` refuses the unaligned `+0x0C` on its own
    let (Some(data), Some(counts)) = (read::<usize>(string), read::<u64>(string + 8)) else {
        return Written::Unreadable;
    };
    let (num, max) = (counts as u32 as i32, (counts >> 32) as u32 as i32);
    let Ok(count) = i32::try_from(url.len()) else {
        return Written::Unreadable;
    };
    let plan = fstring_plan(num, max, data == 0, count);
    let mut target = data;
    if plan.realloc {
        if realloc == 0 {
            return Written::AllocationFailed;
        }
        // SAFETY: the engine's allocator at its RVA (or a test's); a null pointer allocates
        let grown = unsafe {
            let realloc = std::mem::transmute::<usize, ReallocFn>(realloc);
            realloc(data as *mut c_void, url.len() * 2, 0)
        };
        if grown.is_null() {
            return Written::AllocationFailed;
        }
        target = grown as usize;
    }
    // SAFETY: `string` was just read as an FString; `target` holds at least `count` units (its max, or the
    // allocation just made)
    unsafe {
        let string = string as *mut EngineString;
        (*string).data = target as *mut u16;
        std::ptr::copy_nonoverlapping(url.as_ptr(), target as *mut u16, url.len());
        (*string).num = plan.num;
        (*string).max = plan.max;
    }
    Written::Written
}

/// The SRV-1 body (4.2): the engine's answer, then the server's map URL written over it.
pub(crate) fn default_map(out: *mut EngineString, next: usize) -> *mut EngineString {
    if next == 0 {
        return out;
    }
    // SAFETY: the broker wrote a function of the getter's signature into this cell
    let ret = unsafe { std::mem::transmute::<usize, GetMapFn>(next)(out) };
    if detaching() || ret.is_null() || !readable(ret as usize, 0x10) {
        return ret;
    }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let Some(url) = crate::listen::map_url() else {
            return;
        };
        if write_map_url(ret as usize, url, REALLOC.load(Ordering::Acquire)) == Written::AllocationFailed
            && !ALLOCATION_LOGGED.swap(true, Ordering::SeqCst)
        {
            log::line(lines::ALLOCATION_FAILED);
        }
    }));
    ret
}

unsafe extern "C" fn srv1_detour(out: *mut EngineString) -> *mut EngineString {
    default_map(out, SRV1_NEXT.load())
}

fn rva_address(rva: u32) -> Option<usize> {
    win::image_base().checked_add(usize::try_from(rva).ok()?)
}

/// Installs SRV-1 on the getter (band `Mit`, order 0).
pub fn install_srv1(base: usize) -> Result<(), &'static str> {
    let unknown = || minhook::status_name(minhook::MH_UNKNOWN);
    let line = target("SRV-1").ok_or_else(unknown)?;
    let offset = |rva: u32| usize::try_from(rva).ok().and_then(|rva| base.checked_add(rva));
    REALLOC.store(offset(data::FMEMORY_REALLOC).ok_or_else(unknown)?, Ordering::Release);
    let address = offset(line.rva).ok_or_else(unknown)?;
    broker::hook(address, srv1_detour as *const () as usize, 0, &SRV1_NEXT).map_err(minhook::status_name)
}

/// `GWorld`, through the checked read.
pub fn gworld() -> Option<usize> {
    read::<usize>(rva_address(data::GWORLD)?).filter(|&world| world != 0)
}

/// `UEngine::CreateNamedNetDriver(engine, context, name, definition)` with one `FName` as both.
pub fn create_named_net_driver(engine: usize, context: usize, name: u64) -> bool {
    let Some(function) = rva_address(data::CREATE_NAMED_NET_DRIVER) else {
        return false;
    };
    // SAFETY: the engine function at its RVA (RT-2), on the game thread, with the engine and a world context the
    // caller read through checked reads
    unsafe {
        let create = std::mem::transmute::<usize, CreateNamedNetDriverFn>(function);
        create(engine as *mut c_void, context as *mut c_void, name, name) != 0
    }
}

/// The driver's `InitListen` (6.3 step 4): its vtable slot, called only when it is a function start in the
/// executable's `.pdata`.
pub fn init_listen(
    driver: usize,
    notify: usize,
    url: &mut [u64; r9::FURL_SIZE / 8],
    error: &mut EngineString,
) -> Result<bool, &'static str> {
    const NOT_AN_ENTRY: &str = "InitListen is not a function entry";
    let vtable = read::<usize>(driver).filter(|&vtable| readable(vtable, r9::NET_DRIVER_VTABLE_SPAN));
    let function = vtable
        .and_then(|vtable| read::<usize>(vtable + r9::NET_DRIVER_INIT_LISTEN_SLOT))
        .ok_or(NOT_AN_ENTRY)?;
    let rva = function
        .checked_sub(win::image_base())
        .and_then(|rva| u32::try_from(rva).ok())
        .ok_or(NOT_AN_ENTRY)?;
    if function_entry(rva) != Some(rva) {
        return Err(NOT_AN_ENTRY);
    }
    // SAFETY: a function start of the executable read from the driver's vtable; the driver, the world's notify
    // interface, a zeroed FURL block and an empty FString this frame owns
    let ok = unsafe {
        let init = std::mem::transmute::<usize, InitListenFn>(function);
        init(
            driver as *mut c_void,
            notify as *mut c_void,
            url.as_mut_ptr().cast(),
            0,
            error,
        )
    };
    Ok(ok != 0)
}

/// `InitListen`'s error text, at most 512 units, lossy. The engine's buffer is not freed (R9 call 8).
pub fn error_text(error: &EngineString) -> Option<String> {
    if error.num <= 1 || error.data.is_null() {
        return None;
    }
    let units = usize::try_from(error.num - 1).ok()?.min(MAX_ERROR_UNITS);
    let mut bytes = vec![0u8; units * 2];
    if !read_bytes(error.data as usize, &mut bytes) {
        return None;
    }
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .take_while(|&unit| unit != 0)
        .collect();
    Some(String::from_utf16_lossy(&units))
}

/// Writes each line with one `WriteFile` to standard output; false on no handle or a short write.
pub fn write_stdout(text: &[String]) -> bool {
    // SAFETY: no pointer arguments
    let handle = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
    if handle.is_null() || handle as isize == -1 {
        return false;
    }
    // SAFETY: the process's standard output handle, borrowed: never closed here
    let mut out = ManuallyDrop::new(unsafe { File::from_raw_handle(handle) });
    text.iter()
        .all(|line| out.write(line.as_bytes()).is_ok_and(|written| written == line.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    unsafe extern "C" fn test_alloc(_: *mut c_void, size: usize, _: u32) -> *mut c_void {
        Vec::leak(vec![0u16; size / 2]).as_mut_ptr().cast()
    }

    unsafe extern "C" fn null_alloc(_: *mut c_void, _: usize, _: u32) -> *mut c_void {
        std::ptr::null_mut()
    }

    fn url() -> Vec<u16> {
        "/Game/Maps/Hub?MaxPlayers=4\0".encode_utf16().collect()
    }

    fn text(string: &EngineString) -> String {
        let units = unsafe { std::slice::from_raw_parts(string.data, string.num as usize) };
        String::from_utf16_lossy(units)
    }

    #[test]
    fn srv1_write_path() {
        let url = url();
        let count = url.len() as i32;
        // max short: a new engine allocation
        let mut old = vec![0x41u16; 4];
        let mut string = EngineString {
            data: old.as_mut_ptr(),
            num: 4,
            max: 4,
        };
        let at = &mut string as *mut EngineString as usize;
        assert_eq!(
            write_map_url(at, &url, test_alloc as *const () as usize),
            Written::Written
        );
        assert_ne!(string.data, old.as_mut_ptr(), "grown");
        assert_eq!((string.num, string.max), (count, count));
        assert_eq!(text(&string), "/Game/Maps/Hub?MaxPlayers=4\0");
        // fits: reused in place, max kept
        let mut roomy = vec![0u16; 64];
        let mut string = EngineString {
            data: roomy.as_mut_ptr(),
            num: 3,
            max: 64,
        };
        let at = &mut string as *mut EngineString as usize;
        assert_eq!(
            write_map_url(at, &url, null_alloc as *const () as usize),
            Written::Written
        );
        assert_eq!(string.data, roomy.as_mut_ptr(), "reused");
        assert_eq!((string.num, string.max), (count, 64));
        assert_eq!(text(&string), "/Game/Maps/Hub?MaxPlayers=4\0");
        // null data with an allocator answering null: left alone
        let mut string = EngineString::empty();
        let at = &mut string as *mut EngineString as usize;
        assert_eq!(
            write_map_url(at, &url, null_alloc as *const () as usize),
            Written::AllocationFailed
        );
        assert!(string.data.is_null());
        assert_eq!((string.num, string.max), (0, 0));
        assert_eq!(
            write_map_url(0, &url, test_alloc as *const () as usize),
            Written::Unreadable
        );
    }

    static NEXT_ANSWER: Mutex<usize> = Mutex::new(0);

    unsafe extern "C" fn fake_getter(out: *mut EngineString) -> *mut EngineString {
        *NEXT_ANSWER.lock().unwrap() += 1;
        out
    }

    #[test]
    fn srv1_without_a_launch_returns_the_engines_answer() {
        let mut roomy = vec![0x42u16; 8];
        let mut string = EngineString {
            data: roomy.as_mut_ptr(),
            num: 2,
            max: 8,
        };
        let ret = default_map(&mut string, fake_getter as *const () as usize);
        assert_eq!(ret, &mut string as *mut EngineString);
        assert_eq!(
            (string.num, string.max),
            (2, 8),
            "no launch in a test process: untouched"
        );
        assert_eq!(*NEXT_ANSWER.lock().unwrap(), 1);
        assert!(default_map(std::ptr::null_mut(), fake_getter as *const () as usize).is_null());
    }

    #[test]
    fn error_text_is_bounded() {
        let mut units: Vec<u16> = "bind failed\0".encode_utf16().collect();
        let error = EngineString {
            data: units.as_mut_ptr(),
            num: units.len() as i32,
            max: units.len() as i32,
        };
        assert_eq!(error_text(&error).as_deref(), Some("bind failed"));
        assert_eq!(error_text(&EngineString::empty()), None);
        let mut long = vec![u16::from(b'x'); 600];
        let error = EngineString {
            data: long.as_mut_ptr(),
            num: 600,
            max: 600,
        };
        assert_eq!(error_text(&error).map(|text| text.len()), Some(MAX_ERROR_UNITS));
    }

    #[link(name = "kernel32", kind = "raw-dylib")]
    unsafe extern "system" {
        fn CreatePipe(read: *mut *mut c_void, write: *mut *mut c_void, attributes: *const c_void, size: u32) -> i32;
        fn SetStdHandle(which: u32, handle: *mut c_void) -> i32;
    }

    #[test]
    fn ready_line_reaches_stdout() {
        use rustic_isles_runtime_core::listen::ready_lines;
        use std::io::Read;
        let (mut reader, mut writer) = (std::ptr::null_mut(), std::ptr::null_mut());
        assert_ne!(
            unsafe { CreatePipe(&mut reader, &mut writer, std::ptr::null(), 4096) },
            0
        );
        let old = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
        assert_ne!(unsafe { SetStdHandle(STD_OUTPUT_HANDLE, writer) }, 0);
        let lines = ready_lines("0f-1a", 8780);
        let written = write_stdout(&lines);
        unsafe { SetStdHandle(STD_OUTPUT_HANDLE, old) };
        drop(unsafe { File::from_raw_handle(writer) });
        assert!(written);
        let mut got = String::new();
        unsafe { File::from_raw_handle(reader) }
            .read_to_string(&mut got)
            .unwrap();
        // Other tests' libtest output can land in the swapped handle while it is ours.
        assert!(
            got.contains("RUSTIC_ISLES_GAMESERVER_READY launchId=0f-1a port=8780\r\n"),
            "{got:?}"
        );
    }
}
