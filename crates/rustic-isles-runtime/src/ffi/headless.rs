//! R5's unsafe half (spec-runtime-r5): the detour bodies, the hider thread, the game CRT's locale, HL-11's native
//! function and its pointer write, and HL-1's byte stores. `crate::headless` decides when each runs.

use super::broker::{self, NextCell};
use super::memory::{query, read, read_bytes, readable_unaligned};
use super::{detaching, engine, log, minhook, win};
use rustic_isles_runtime_core::headless::{
    self as core, DISABLE_PER_THREAD_LOCALE, DRIVER_TYPE_WARP, ENABLE_PER_THREAD_LOCALE, HIDER_FAST_MS,
    HIDER_FAST_ROUNDS, HIDER_SLOW_MS, LC_ALL, Locale, SW_HIDE, UFUNCTION_FUNC, VIEWPORT_FUNCTION, VIEWPORT_LOGGED,
    reason,
};
use std::ffi::{CStr, c_char, c_void};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, AtomicU32, AtomicUsize, Ordering};
use windows_sys::Win32::Foundation::{HANDLE, HINSTANCE, HMODULE, HWND, LPARAM};
use windows_sys::Win32::Globalization::GetUserDefaultLocaleName;
use windows_sys::Win32::System::Console::GetConsoleWindow;
use windows_sys::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleHandleExW,
    GetModuleHandleW, GetProcAddress,
};
use windows_sys::Win32::System::Memory::{
    PAGE_EXECUTE_READWRITE, PAGE_EXECUTE_WRITECOPY, PAGE_GUARD, PAGE_READWRITE, PAGE_WRITECOPY,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowThreadProcessId, HMENU, IsWindowVisible, ShowWindow,
};
use windows_sys::core::BOOL;

/// `LOCALE_NAME_MAX_LENGTH`.
const LOCALE_NAME_MAX: usize = 85;
/// The longest path HL-6 reads.
const MAX_PATH_UNITS: usize = 32_768;

// --- Modules, exports, writable memory -------------------------------------------------------------------------

/// `GetModuleHandleW(name)`, never `LoadLibrary` (Init runs under the loader lock).
pub fn module(name: &str) -> Option<usize> {
    let name = win::to_wide(name);
    // SAFETY: a NUL-terminated name
    let handle = unsafe { GetModuleHandleW(name.as_ptr()) } as usize;
    (handle != 0).then_some(handle)
}

/// `GetProcAddress(module, name)`, forwarders resolved by the loader.
pub fn export(module: usize, name: &CStr) -> Option<usize> {
    // SAFETY: a module handle and a NUL-terminated name
    unsafe { GetProcAddress(module as HMODULE, name.as_ptr().cast()) }.map(|function| function as usize)
}

/// An export of a module that must already be loaded, or the install reason.
pub fn module_export(module_name: &str, name: &CStr) -> Result<usize, &'static str> {
    let handle = module(module_name).ok_or(reason::MODULE_NOT_LOADED)?;
    export(handle, name).ok_or(reason::EXPORT_NOT_FOUND)
}

/// The module holding `address`.
fn module_of(address: usize) -> Option<usize> {
    let mut handle: HMODULE = std::ptr::null_mut();
    // SAFETY: an address and an out pointer; the reference count is not touched
    let found = unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            address as *const u16,
            &mut handle,
        )
    };
    (found != 0 && !handle.is_null()).then_some(handle as usize)
}

/// Committed, writable, not a guard page, inside one region.
fn writable(ptr: usize, size: usize) -> bool {
    let Some(region) = query(ptr) else {
        return false;
    };
    let write = PAGE_READWRITE | PAGE_WRITECOPY | PAGE_EXECUTE_READWRITE | PAGE_EXECUTE_WRITECOPY;
    readable_unaligned(ptr, size) && region.protect & write != 0 && region.protect & PAGE_GUARD == 0
}

/// A NUL-terminated byte string, at most `cap` bytes, read through checked memory.
fn c_text(ptr: usize, cap: usize) -> Option<String> {
    let mut bytes = Vec::new();
    for at in 0..cap {
        let mut byte = [0u8];
        if !read_bytes(ptr.checked_add(at)?, &mut byte) {
            return None;
        }
        if byte[0] == 0 {
            return Some(String::from_utf8_lossy(&bytes).into_owned());
        }
        bytes.push(byte[0]);
    }
    None
}

// --- HL-1 -----------------------------------------------------------------------------------------------------

static SERVER_FLAGS: OnceLock<Option<(usize, usize)>> = OnceLock::new();

/// Init: finds `GIsServer` and `GIsClient` in the executable's writable data. False when they are not.
pub fn resolve_server_flags(base: usize) -> bool {
    SERVER_FLAGS
        .get_or_init(|| {
            let server = base.checked_add(usize::try_from(core::GIS_SERVER).ok()?)?;
            let client = base.checked_add(usize::try_from(core::GIS_CLIENT).ok()?)?;
            (writable(server, 1) && writable(client, 1)).then_some((server, client))
        })
        .is_some()
}

/// `GIsServer` = 1, `GIsClient` = 0. Never fails, never logs.
pub fn force_server_flags() {
    if let Some(Some((server, client))) = SERVER_FLAGS.get() {
        // SAFETY: both bytes were checked writable in Init and lie in the executable's data, which never moves
        unsafe {
            (*(*server as *const AtomicU8)).store(1, Ordering::Relaxed);
            (*(*client as *const AtomicU8)).store(0, Ordering::Relaxed);
        }
    }
}

// --- HL-3 -----------------------------------------------------------------------------------------------------

type CreateWindowExWFn = unsafe extern "system" fn(
    u32,
    *const u16,
    *const u16,
    u32,
    i32,
    i32,
    i32,
    i32,
    HWND,
    HMENU,
    HINSTANCE,
    *const c_void,
) -> HWND;
type ShowWindowFn = unsafe extern "system" fn(HWND, i32) -> BOOL;

static CREATE_WINDOW_NEXT: NextCell = NextCell::new();
static SHOW_WINDOW_NEXT: NextCell = NextCell::new();
static HIDES: AtomicU32 = AtomicU32::new(0);
static HIDER_ARMED: AtomicBool = AtomicBool::new(false);
static HIDER_STARTED: AtomicBool = AtomicBool::new(false);

/// This process's top-level or child window that is not the console.
fn ours(hwnd: HWND) -> bool {
    if hwnd.is_null() {
        return false;
    }
    let mut pid = 0u32;
    // SAFETY: a window handle (stale handles answer 0) and an out pointer
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    // SAFETY: no arguments
    pid == win::pid() && hwnd != unsafe { GetConsoleWindow() }
}

fn hide(hwnd: HWND) {
    HIDES.fetch_add(1, Ordering::Relaxed);
    // SAFETY: a window handle; the export runs through HL-3's chain, which passes SW_HIDE
    unsafe { ShowWindow(hwnd, SW_HIDE) };
}

unsafe extern "system" fn create_window_detour(
    ex_style: u32,
    class: *const u16,
    name: *const u16,
    style: u32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    parent: HWND,
    menu: HMENU,
    instance: HINSTANCE,
    param: *const c_void,
) -> HWND {
    let next = CREATE_WINDOW_NEXT.load();
    if next == 0 {
        return std::ptr::null_mut();
    }
    // SAFETY: the broker wrote a function of CreateWindowExW's signature into this cell
    let next: CreateWindowExWFn = unsafe { std::mem::transmute::<usize, CreateWindowExWFn>(next) };
    // SAFETY: the caller's arguments, unchanged
    let hwnd = unsafe {
        next(
            ex_style, class, name, style, x, y, width, height, parent, menu, instance, param,
        )
    };
    if !detaching() {
        start_hider_once();
        if !hwnd.is_null() {
            hide(hwnd);
        }
    }
    hwnd
}

unsafe extern "system" fn show_window_detour(hwnd: HWND, cmd: i32) -> BOOL {
    let next = SHOW_WINDOW_NEXT.load();
    if next == 0 {
        return 0;
    }
    // SAFETY: the broker wrote a function of ShowWindow's signature into this cell
    let next: ShowWindowFn = unsafe { std::mem::transmute::<usize, ShowWindowFn>(next) };
    if detaching() {
        // SAFETY: the caller's arguments
        return unsafe { next(hwnd, cmd) };
    }
    start_hider_once();
    let cmd = if cmd != SW_HIDE && ours(hwnd) {
        HIDES.fetch_add(1, Ordering::Relaxed);
        SW_HIDE
    } else {
        cmd
    };
    // SAFETY: the caller's window
    unsafe { next(hwnd, cmd) }
}

/// Hooks both exports (order 0). Each result separately; the other stays when one fails.
pub fn install_windows() -> [Result<(), String>; 2] {
    HIDER_ARMED.store(true, Ordering::SeqCst);
    let hook = |name: &CStr, wrapper: usize, next: &'static NextCell| {
        let target = module_export("user32.dll", name).map_err(str::to_owned)?;
        broker::hook(target, wrapper, 0, next).map_err(|status| minhook::status_name(status).to_owned())
    };
    [
        hook(
            c"CreateWindowExW",
            create_window_detour as *const () as usize,
            &CREATE_WINDOW_NEXT,
        ),
        hook(
            c"ShowWindow",
            show_window_detour as *const () as usize,
            &SHOW_WINDOW_NEXT,
        ),
    ]
}

unsafe extern "system" fn hide_visible(hwnd: HWND, _: LPARAM) -> BOOL {
    // SAFETY: a window handle from EnumWindows
    if ours(hwnd) && unsafe { IsWindowVisible(hwnd) } != 0 {
        hide(hwnd);
    }
    1
}

fn hider() {
    let mut logged = false;
    let mut round = 0u32;
    while !detaching() {
        // SAFETY: a callback of EnumWindows' signature
        unsafe { EnumWindows(Some(hide_visible), 0) };
        if round < HIDER_FAST_ROUNDS {
            if !logged && HIDES.load(Ordering::Relaxed) > 0 && !detaching() {
                logged = true;
                crate::lifecycle::write_status_once();
                log::line(core::HID_WINDOW_LINE);
            }
            round += 1;
            win::sleep_ms(HIDER_FAST_MS);
        } else {
            win::sleep_ms(HIDER_SLOW_MS);
        }
    }
}

/// The hider starts on the first HL-3 detour call or SP-3 tick, never in Init (R0 starts no thread there).
pub fn start_hider_once() {
    if !HIDER_ARMED.load(Ordering::SeqCst) || HIDER_STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Err(error) = std::thread::Builder::new().name("rir-hider".into()).spawn(hider) {
        let code = error
            .raw_os_error()
            .and_then(|code| u32::try_from(code).ok())
            .unwrap_or(0);
        crate::lifecycle::write_status_once();
        log::line(&core::hider_failed_line(code));
    }
}

// --- HL-4 -----------------------------------------------------------------------------------------------------

type StartupFn = unsafe extern "C" fn(*mut c_void);

static WEB_BROWSER_NEXT: NextCell = NextCell::new();
static WEB_BROWSER_LOGGED: AtomicBool = AtomicBool::new(false);

unsafe extern "C" fn web_browser_startup_detour(module: *mut c_void) {
    if detaching() {
        let next = WEB_BROWSER_NEXT.load();
        if next != 0 {
            // SAFETY: the broker wrote a function of StartupModule's signature; R0 3.3 forwards after detach
            unsafe { std::mem::transmute::<usize, StartupFn>(next)(module) };
        }
        return;
    }
    // Absorbs: the original never runs, so CEF never starts
    crate::lifecycle::write_status_once();
    if !WEB_BROWSER_LOGGED.swap(true, Ordering::SeqCst) {
        log::line(core::CEF_SKIPPED_LINE);
    }
}

/// Hooks `StartupModule` at `address` (its prologue already verified).
pub fn install_web_browser(address: usize) -> Result<(), i32> {
    broker::hook(
        address,
        web_browser_startup_detour as *const () as usize,
        0,
        &WEB_BROWSER_NEXT,
    )
}

// --- HL-6 to HL-8 ---------------------------------------------------------------------------------------------

type LoadLibraryExWFn = unsafe extern "system" fn(*const u16, HANDLE, u32) -> HMODULE;
type CreateDeviceFn = unsafe extern "system" fn(
    *mut c_void,
    i32,
    HMODULE,
    u32,
    *const i32,
    u32,
    u32,
    *mut *mut c_void,
    *mut i32,
    *mut *mut c_void,
) -> i32;
type CreateDeviceAndSwapChainFn = unsafe extern "system" fn(
    *mut c_void,
    i32,
    HMODULE,
    u32,
    *const i32,
    u32,
    u32,
    *const c_void,
    *mut *mut c_void,
    *mut *mut c_void,
    *mut i32,
    *mut *mut c_void,
) -> i32;

const NOT_RECORDED: i32 = i32::MIN;
const NO_EXPORT: i32 = i32::MIN + 1;

static LOAD_LIBRARY_NEXT: NextCell = NextCell::new();
static DEVICE_NEXT: NextCell = NextCell::new();
static DEVICE_SWAP_NEXT: NextCell = NextCell::new();
static D3D_HOOKED: AtomicBool = AtomicBool::new(false);
static D3D_RESULTS: [AtomicI32; 2] = [AtomicI32::new(NOT_RECORDED), AtomicI32::new(NOT_RECORDED)];
static D3D_LOGGED: [AtomicBool; 2] = [AtomicBool::new(false), AtomicBool::new(false)];

/// The units of a NUL-terminated path, `None` for null or unreadable.
fn path_units(path: *const u16) -> Option<Vec<u16>> {
    let mut units = Vec::new();
    let mut at = path as usize;
    while units.len() < MAX_PATH_UNITS {
        let unit = super::memory::read_unaligned::<u16>(at)?;
        if unit == 0 {
            return Some(units);
        }
        units.push(unit);
        at = at.checked_add(2)?;
    }
    Some(units)
}

unsafe extern "system" fn load_library_detour(path: *const u16, file: HANDLE, flags: u32) -> HMODULE {
    let next = LOAD_LIBRARY_NEXT.load();
    if next == 0 {
        return std::ptr::null_mut();
    }
    // SAFETY: the broker wrote a function of LoadLibraryExW's signature into this cell
    let next: LoadLibraryExWFn = unsafe { std::mem::transmute::<usize, LoadLibraryExWFn>(next) };
    // SAFETY: the caller's arguments
    let module = unsafe { next(path, file, flags) };
    if !module.is_null()
        && !detaching()
        && !D3D_HOOKED.load(Ordering::SeqCst)
        && core::is_d3d11_path(path_units(path).as_deref())
    {
        // Possibly inside another module's DllMain: records, never logs
        hook_d3d11(module as usize);
    }
    module
}

unsafe extern "system" fn create_device_detour(
    _adapter: *mut c_void,
    _driver_type: i32,
    software: HMODULE,
    flags: u32,
    levels: *const i32,
    n_levels: u32,
    sdk: u32,
    device: *mut *mut c_void,
    level: *mut i32,
    context: *mut *mut c_void,
) -> i32 {
    // SAFETY: the broker wrote a function of D3D11CreateDevice's signature; WARP needs a null adapter
    unsafe {
        std::mem::transmute::<usize, CreateDeviceFn>(DEVICE_NEXT.load())(
            std::ptr::null_mut(),
            DRIVER_TYPE_WARP,
            software,
            flags,
            levels,
            n_levels,
            sdk,
            device,
            level,
            context,
        )
    }
}

unsafe extern "system" fn create_device_and_swap_chain_detour(
    _adapter: *mut c_void,
    _driver_type: i32,
    software: HMODULE,
    flags: u32,
    levels: *const i32,
    n_levels: u32,
    sdk: u32,
    swap_desc: *const c_void,
    swap_chain: *mut *mut c_void,
    device: *mut *mut c_void,
    level: *mut i32,
    context: *mut *mut c_void,
) -> i32 {
    // SAFETY: as above, for D3D11CreateDeviceAndSwapChain
    unsafe {
        std::mem::transmute::<usize, CreateDeviceAndSwapChainFn>(DEVICE_SWAP_NEXT.load())(
            std::ptr::null_mut(),
            DRIVER_TYPE_WARP,
            software,
            flags,
            levels,
            n_levels,
            sdk,
            swap_desc,
            swap_chain,
            device,
            level,
            context,
        )
    }
}

/// HL-7 and HL-8 on `module`, once per process; the results wait for `d3d_lines`.
pub fn hook_d3d11(module: usize) {
    if D3D_HOOKED.swap(true, Ordering::SeqCst) {
        return;
    }
    let exports: [(&CStr, usize, &'static NextCell); 2] = [
        (
            c"D3D11CreateDevice",
            create_device_detour as *const () as usize,
            &DEVICE_NEXT,
        ),
        (
            c"D3D11CreateDeviceAndSwapChain",
            create_device_and_swap_chain_detour as *const () as usize,
            &DEVICE_SWAP_NEXT,
        ),
    ];
    for (slot, (name, wrapper, next)) in exports.into_iter().enumerate() {
        let result = match export(module, name) {
            Some(target) => broker::hook(target, wrapper, 0, next).err().unwrap_or(minhook::MH_OK),
            None => NO_EXPORT,
        };
        D3D_RESULTS[slot].store(result, Ordering::SeqCst);
    }
}

/// HL-6 on `kernel32!LoadLibraryExW`.
pub fn install_load_library() -> Result<(), String> {
    let target = module_export("kernel32.dll", c"LoadLibraryExW").map_err(str::to_owned)?;
    broker::hook(target, load_library_detour as *const () as usize, 0, &LOAD_LIBRARY_NEXT)
        .map_err(|status| minhook::status_name(status).to_owned())
}

/// HL-7 and HL-8's install lines not written yet, each once, as soon as it is recorded.
pub fn d3d_lines() -> Vec<String> {
    let mut lines = Vec::new();
    for (slot, id) in ["HL-7", "HL-8"].into_iter().enumerate() {
        let result = D3D_RESULTS[slot].load(Ordering::SeqCst);
        if result == NOT_RECORDED || D3D_LOGGED[slot].swap(true, Ordering::SeqCst) {
            continue;
        }
        let reason = match result {
            minhook::MH_OK => Ok(()),
            NO_EXPORT => Err(reason::EXPORT_NOT_FOUND),
            status => Err(minhook::status_name(status)),
        };
        lines.push(rustic_isles_runtime_core::log::install_line(id, reason));
    }
    lines
}

// --- HL-12 ----------------------------------------------------------------------------------------------------

type SetLocaleFn = unsafe extern "C" fn(i32, *const c_char) -> *mut c_char;
type ConfigThreadLocaleFn = unsafe extern "C" fn(i32) -> i32;
type LocaleConvFn = unsafe extern "C" fn() -> *mut c_void;

fn user_locale() -> String {
    let mut buffer = [0u16; LOCALE_NAME_MAX];
    // SAFETY: a buffer of the stated length
    let len = unsafe { GetUserDefaultLocaleName(buffer.as_mut_ptr(), LOCALE_NAME_MAX as i32) };
    match usize::try_from(len) {
        Ok(len @ 1..=LOCALE_NAME_MAX) => String::from_utf16_lossy(&buffer[..len - 1]),
        _ => "?".to_owned(),
    }
}

fn decimal_point(localeconv: Option<usize>) -> String {
    let Some(localeconv) = localeconv else {
        return "?".to_owned();
    };
    // SAFETY: ucrtbase's localeconv
    let lconv = unsafe { std::mem::transmute::<usize, LocaleConvFn>(localeconv)() } as usize;
    // `struct lconv`'s first field is `char* decimal_point`
    read::<usize>(lconv)
        .and_then(|text| c_text(text, 8))
        .unwrap_or_else(|| "?".to_owned())
}

/// Steps 2 to 5 of HL-12 on the game's `ucrtbase.dll`: the measurement and the install result.
pub fn fix_locale() -> (Locale, Result<(), &'static str>) {
    let mut locale = Locale {
        user_locale: user_locale(),
        game_crt: false,
        before: "?".to_owned(),
        after: "?".to_owned(),
        numeric: None,
    };
    let Some(crt) = module("ucrtbase.dll") else {
        return (locale, Err(reason::MODULE_NOT_LOADED));
    };
    locale.game_crt = true;
    let setlocale = export(crt, c"setlocale");
    let configthreadlocale = export(crt, c"_configthreadlocale");
    let localeconv = export(crt, c"localeconv");
    locale.before = decimal_point(localeconv);
    let Some(setlocale) = setlocale else {
        locale.after = decimal_point(localeconv);
        return (locale, Err(reason::EXPORT_NOT_FOUND));
    };
    // SAFETY: ucrtbase's own exports with their C signatures
    unsafe {
        let setlocale = std::mem::transmute::<usize, SetLocaleFn>(setlocale);
        let configure = configthreadlocale.map(|function| std::mem::transmute::<usize, ConfigThreadLocaleFn>(function));
        // The calling thread's own locale first, then the process-wide one
        if let Some(configure) = configure {
            configure(ENABLE_PER_THREAD_LOCALE);
        }
        setlocale(LC_ALL, c"C".as_ptr());
        if let Some(configure) = configure {
            configure(DISABLE_PER_THREAD_LOCALE);
        }
        let name = setlocale(LC_ALL, c"C".as_ptr());
        if !name.is_null() {
            locale.numeric = c_text(name as usize, 256);
        }
    }
    locale.after = decimal_point(localeconv);
    (locale, Ok(()))
}

// --- HL-11 ----------------------------------------------------------------------------------------------------

type ExecFn = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void);

static VIEWPORT_ORIGINAL: AtomicUsize = AtomicUsize::new(0);
static SUBSTITUTIONS: AtomicU32 = AtomicU32::new(0);

/// The MIT `GetViewportSize` (UE's native thunk): the original, then (0,0) becomes 1920×1080.
unsafe extern "C" fn viewport_exec(context: *mut c_void, stack: *mut c_void, result: *mut c_void) {
    let original = VIEWPORT_ORIGINAL.load(Ordering::Acquire);
    if original != 0 {
        // SAFETY: the native pointer this function replaced, with the engine's arguments
        unsafe { std::mem::transmute::<usize, ExecFn>(original)(context, stack, result) };
    }
    let slot = result as usize;
    // The return slot holds an FVector2D: two f32, not 8-aligned
    if detaching() || slot == 0 || !writable(slot, 8) {
        return;
    }
    let Some([x, y]) = super::memory::read_unaligned::<[f32; 2]>(slot) else {
        return;
    };
    if let Some((fixed_x, fixed_y)) = core::fix_viewport(x, y) {
        // SAFETY: 8 bytes checked writable above
        unsafe { std::ptr::write_unaligned(slot as *mut [f32; 2], [fixed_x, fixed_y]) };
        let logged = SUBSTITUTIONS
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
                (count < VIEWPORT_LOGGED).then_some(count + 1)
            })
            .is_ok();
        if logged {
            log::line(&core::viewport_substituted_line(x, y));
        }
    }
}

/// One HL-11 try.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Swap {
    /// The UFunction is not loaded (or names are off): try again.
    NotFound,
    /// The UFunction cannot be written yet: try again.
    Unwritable,
    /// The native pointer is no function entry of the executable (its RVA when inside it): stop.
    Refused(Option<u32>),
    /// Swapped from this RVA.
    Swapped(u32),
    /// Already the MIT function.
    AlreadyMit,
}

/// HL-11's find, self-check and swap (11.1).
pub fn swap_viewport() -> Swap {
    let Some(function) = engine::find_object_by_full_name(VIEWPORT_FUNCTION) else {
        return Swap::NotFound;
    };
    if !engine::is_live_object(function) {
        return Swap::NotFound;
    }
    let Some(slot) = function.checked_add(UFUNCTION_FUNC) else {
        return Swap::NotFound;
    };
    let Some(native) = read::<u64>(slot).and_then(|native| usize::try_from(native).ok()) else {
        return Swap::NotFound;
    };
    let mit = viewport_exec as *const () as usize;
    if native == mit {
        return Swap::AlreadyMit;
    }
    let base = win::image_base();
    let rva = (module_of(native) == Some(base))
        .then(|| native.checked_sub(base).and_then(|rva| u32::try_from(rva).ok()))
        .flatten();
    let Some(rva) = rva.filter(|&rva| super::memory::function_entry(rva) == Some(rva)) else {
        return Swap::Refused(rva);
    };
    if !writable(slot, 8) {
        return Swap::Unwritable;
    }
    VIEWPORT_ORIGINAL.store(native, Ordering::Release);
    // SAFETY: an aligned, writable 8-byte field of a live UFunction; one store, never restored (the DLL is pinned)
    unsafe { (*(slot as *const AtomicUsize)).store(mit, Ordering::Release) };
    Swap::Swapped(rva)
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::LibraryLoader::LoadLibraryW;
    use windows_sys::Win32::System::Memory::{MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, VirtualAlloc, VirtualFree};
    use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, SW_SHOW, WS_OVERLAPPEDWINDOW};

    #[test]
    fn windows_stay_hidden() {
        let [create, show] = install_windows();
        assert_eq!(create, Ok(()));
        assert_eq!(show, Ok(()));
        // SAFETY: no arguments
        let console = unsafe { GetConsoleWindow() };
        // SAFETY: a console handle or null
        let console_visible = !console.is_null() && unsafe { IsWindowVisible(console) } != 0;
        let class = win::to_wide("STATIC");
        let title = win::to_wide("rir-test");
        // SAFETY: a predefined class and NUL-terminated strings
        let window = unsafe {
            CreateWindowExW(
                0,
                class.as_ptr(),
                title.as_ptr(),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                100,
                100,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        assert!(!window.is_null());
        // SAFETY: the test's window
        unsafe {
            assert_eq!(IsWindowVisible(window), 0, "hidden on creation");
            ShowWindow(window, SW_SHOW);
            assert_eq!(IsWindowVisible(window), 0, "SW_SHOW leaves it hidden");
            ShowWindow(window, SW_HIDE);
            assert_eq!(IsWindowVisible(window), 0);
        }
        assert!(HIDES.load(Ordering::Relaxed) >= 2);
        win::sleep_ms(HIDER_FAST_MS + 100);
        if !console.is_null() {
            // SAFETY: as above
            assert_eq!(
                unsafe { IsWindowVisible(console) } != 0,
                console_visible,
                "console untouched"
            );
        }
        // SAFETY: the test's window
        unsafe { DestroyWindow(window) };
    }

    #[test]
    fn locale_becomes_c() {
        let name = win::to_wide("ucrtbase.dll");
        // SAFETY: loading a system library is allowed in a test
        let crt = unsafe { LoadLibraryW(name.as_ptr()) } as usize;
        assert_ne!(crt, 0);
        let setlocale = export(crt, c"setlocale").unwrap();
        // SAFETY: ucrtbase's setlocale
        let set = unsafe { std::mem::transmute::<usize, SetLocaleFn>(setlocale)(LC_ALL, c"de-DE".as_ptr()) };
        assert!(!set.is_null(), "de-DE is available");
        assert_eq!(decimal_point(export(crt, c"localeconv")), ",");
        let (locale, result) = fix_locale();
        assert_eq!(result, Ok(()));
        assert!(locale.game_crt);
        assert_eq!(locale.numeric.as_deref(), Some("C"));
        assert!(
            core::locale_line(&locale, false).contains("decimal_before=',' decimal_after='.'"),
            "{}",
            core::locale_line(&locale, false)
        );
    }

    extern "C" fn fake_original(_: *mut c_void, stack: *mut c_void, result: *mut c_void) {
        // The test passes the value to write through `stack`
        if result.is_null() {
            return;
        }
        // SAFETY: the test's pointers
        unsafe { std::ptr::write_unaligned(result as *mut [f32; 2], *(stack as *const [f32; 2])) };
    }

    #[test]
    fn viewport_function() {
        VIEWPORT_ORIGINAL.store(fake_original as *const () as usize, Ordering::Release);
        let mut buffer = [0u8; 16];
        let slot = buffer.as_mut_ptr().wrapping_add(3);
        let run = |value: [f32; 2]| {
            let mut value = value;
            // SAFETY: the MIT function with the test's fake frame and an unaligned return slot
            unsafe { viewport_exec(std::ptr::null_mut(), (&raw mut value).cast(), slot.cast()) };
            // SAFETY: the slot lies in the buffer
            unsafe { std::ptr::read_unaligned(slot as *const [f32; 2]) }
        };
        assert_eq!(run([0.0, 0.0]), [1920.0, 1080.0]);
        assert_eq!(run([2560.0, 1440.0]), [2560.0, 1440.0]);
        assert_eq!(run([f32::NAN, 1080.0]), [1920.0, 1080.0]);
        // SAFETY: a null return slot is skipped
        unsafe { viewport_exec(std::ptr::null_mut(), (&raw mut buffer).cast(), std::ptr::null_mut()) };
    }

    static FAKE_ADAPTER: AtomicUsize = AtomicUsize::new(usize::MAX);
    static FAKE_DRIVER: AtomicI32 = AtomicI32::new(-1);

    #[inline(never)]
    extern "system" fn fake_create_device(
        adapter: *mut c_void,
        driver_type: i32,
        _: HMODULE,
        _: u32,
        _: *const i32,
        _: u32,
        _: u32,
        _: *mut *mut c_void,
        _: *mut i32,
        _: *mut *mut c_void,
    ) -> i32 {
        FAKE_ADAPTER.store(std::hint::black_box(adapter as usize), Ordering::SeqCst);
        FAKE_DRIVER.store(std::hint::black_box(driver_type), Ordering::SeqCst);
        0x1234
    }

    #[test]
    fn warp_forced() {
        let target = fake_create_device as *const () as usize;
        broker::hook(target, create_device_detour as *const () as usize, 0, &DEVICE_NEXT).unwrap();
        let null = std::ptr::null_mut();
        // SAFETY: through the patched entry, with a non-null adapter and the hardware driver type (1)
        let result = unsafe {
            std::hint::black_box(std::mem::transmute::<usize, CreateDeviceFn>(target))(
                0x5000 as *mut c_void,
                1,
                null,
                0,
                std::ptr::null(),
                0,
                7,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(result, 0x1234);
        assert_eq!(FAKE_ADAPTER.load(Ordering::SeqCst), 0, "null adapter");
        assert_eq!(FAKE_DRIVER.load(Ordering::SeqCst), DRIVER_TYPE_WARP);
    }

    #[test]
    fn boot_patch_flag() {
        // SAFETY: a fresh private page, freed below
        let page = unsafe { VirtualAlloc(std::ptr::null(), 4096, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE) } as usize;
        assert_ne!(page, 0);
        // SAFETY: the page is ours
        unsafe { std::ptr::copy_nonoverlapping(core::BOOT_EXPECTED.as_ptr(), page as *mut u8, 6) };
        assert_eq!(
            crate::headless::apply_boot_patch_at(page),
            rustic_isles_runtime_core::patch::PatchResult::Applied
        );
        assert!(crate::headless::boot_patch_applied());
        let other = page + 64;
        // SAFETY: as above
        unsafe { std::ptr::copy_nonoverlapping([0xCCu8; 6].as_ptr(), other as *mut u8, 6) };
        assert_eq!(
            crate::headless::apply_boot_patch_at(other),
            rustic_isles_runtime_core::patch::PatchResult::Mismatch
        );
        assert!(!crate::headless::boot_patch_applied());
        // SAFETY: the page allocated above
        unsafe { VirtualFree(page as *mut c_void, 0, MEM_RELEASE) };
    }
}
