//! Hook HTTP-1 on `FCurlHttpRequest::ProcessRequest` (spec-runtime-r2): the backend redirect, a server's progression
//! grant id and a DeployServer's key header. Runs on whatever thread issued the request; keeps no per-request state.

use super::broker::{self, NextCell};
use super::memory::{read, read_bytes, read_unaligned, readable};
use super::{detaching, log};
use crate::config;
use rustic_isles_runtime_core::config::auth::AuthMode;
use rustic_isles_runtime_core::http::{
    GRANT_HEADER, GRANT_LOG_BUDGET, GRANT_READBACK_MAX_NUM, GrantValue, KEY_HEADER, KeyInputs, Redirect, grant_id,
    grant_in_flight_line, grant_line, grant_no_random_line, grant_path, redirect, redirect_line, sends_key,
    wants_grant_id,
};
use rustic_isles_runtime_core::offsets::{data, layout, target};
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

type ProcessRequestFn = unsafe extern "C" fn(*mut c_void) -> u8;
type ReallocFn = unsafe extern "C" fn(*mut c_void, usize, u32) -> *mut c_void;
type GetHeaderFn = unsafe extern "C" fn(*mut c_void, *mut FString, *const FString) -> *mut FString;
type SetHeaderFn = unsafe extern "C" fn(*mut c_void, *const FString, *const FString);
type FreeFn = unsafe extern "C" fn(*mut c_void, *mut c_void);

#[link(name = "bcryptprimitives", kind = "raw-dylib")]
unsafe extern "system" {
    fn ProcessPrng(data: *mut u8, len: usize) -> i32;
}

/// The engine's `FString`: `num` counts the NUL.
#[repr(C)]
pub(crate) struct FString {
    data: *mut u16,
    num: i32,
    max: i32,
}

impl FString {
    const fn empty() -> Self {
        Self {
            data: std::ptr::null_mut(),
            num: 0,
            max: 0,
        }
    }
}

/// A NUL-terminated buffer of R2's own, lent to `GetHeader`/`SetHeader` for one call (they copy it).
pub(super) struct Lent(Vec<u16>);

impl Lent {
    pub(super) fn new(text: &[u16]) -> Self {
        Self(text.iter().copied().chain(std::iter::once(0)).collect())
    }

    pub(super) fn of(text: &str) -> Self {
        Self(text.encode_utf16().chain(std::iter::once(0)).collect())
    }

    fn fstring(&self) -> FString {
        let num = i32::try_from(self.0.len()).unwrap_or(i32::MAX);
        FString {
            data: self.0.as_ptr().cast_mut(),
            num,
            max: num,
        }
    }
}

/// Where the engine allocator is: `FMemory::Realloc` and the cell holding `GMalloc`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Allocator {
    realloc: usize,
    gmalloc: usize,
}

/// One call's inputs, read from config by the detour or built by a test.
pub(crate) struct Context<'a> {
    allocator: Allocator,
    api_base: &'a [u16],
    server: bool,
    mode: AuthMode,
    key: Option<&'a [u16]>,
}

static NEXT: NextCell = NextCell::new();
static ALLOCATOR: OnceLock<Allocator> = OnceLock::new();
static REDIRECT_LOGGED: AtomicBool = AtomicBool::new(false);
static GRANTS: AtomicU32 = AtomicU32::new(0);
static PANICKED: AtomicBool = AtomicBool::new(false);

/// The API base as UTF-16, converted once (no lazily built container is first touched under an engine lock).
fn api_base() -> &'static [u16] {
    static BASE: OnceLock<Vec<u16>> = OnceLock::new();
    BASE.get_or_init(|| config::backend_url().api_base().encode_utf16().collect())
}

fn utf16(bytes: &[u8]) -> Vec<u16> {
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&pair| u16::from_le_bytes(pair))
        .collect()
}

pub(super) struct Url {
    data: usize,
    max: i32,
    pub(super) text: Vec<u16>,
}

/// The request's URL (R2 4.1), or `None` when any check fails.
pub(super) fn read_url(request: usize) -> Option<Url> {
    if !readable(request, layout::HTTP_REQUEST_SIZE) {
        return None;
    }
    let field = request + layout::HTTP_REQUEST_URL;
    let data = read::<usize>(field)?;
    let num = read_unaligned::<i32>(field + 8)?;
    let max = read_unaligned::<i32>(field + 12)?;
    if data == 0 || num < 2 || max < num || num > layout::FSTRING_MAX_NUM {
        return None;
    }
    let count = usize::try_from(num).ok()?;
    let mut bytes = vec![0u8; count * 2];
    if !read_bytes(data, &mut bytes) {
        return None;
    }
    let text = utf16(&bytes[..(count - 1) * 2]);
    Some(Url { data, max, text })
}

/// The in-flight word is `1` (`None` when the request is unreadable).
pub(super) fn in_flight(request: usize) -> Option<bool> {
    if !readable(request, layout::HTTP_REQUEST_SIZE) {
        return None;
    }
    read_unaligned::<i32>(request + layout::HTTP_REQUEST_IN_FLIGHT).map(|word| word == 1)
}

/// Vtable slot `index` of the request, after checking all 17 readable.
fn vtable_slot(request: usize, index: usize) -> Option<usize> {
    let vtable = read::<usize>(request)?;
    if !readable(vtable, layout::HTTP_VTABLE_SLOTS * 8) {
        return None;
    }
    read::<usize>(vtable + index * 8).filter(|&slot| slot != 0)
}

pub(super) fn set_header(request: usize, name: &Lent, value: &Lent) -> bool {
    let Some(slot) = vtable_slot(request, layout::HTTP_SET_HEADER_SLOT) else {
        return false;
    };
    // SAFETY: slot 16 of a readable vtable is SetHeader (R2 3.1); both strings live past the call, which copies them
    let set: SetHeaderFn = unsafe { std::mem::transmute::<usize, SetHeaderFn>(slot) };
    let (name, value) = (name.fstring(), value.fstring());
    // SAFETY: as above
    unsafe { set(request as *mut c_void, &name, &value) };
    true
}

/// Frees an engine allocation through `GMalloc`'s `Free`; skipped when `GMalloc` is null or unreadable.
fn engine_free(allocator: Allocator, ptr: usize) {
    let Some(gmalloc) = read::<usize>(allocator.gmalloc).filter(|&gmalloc| gmalloc != 0) else {
        return;
    };
    let Some(vtable) = read::<usize>(gmalloc) else {
        return;
    };
    if !readable(vtable, (layout::MALLOC_FREE_SLOT + 1) * 8) {
        return;
    }
    let Some(free) = read::<usize>(vtable + layout::MALLOC_FREE_SLOT * 8).filter(|&free| free != 0) else {
        return;
    };
    // SAFETY: slot 6 of the engine allocator's vtable is Free(this, ptr), and `ptr` is the engine's own allocation
    let free: FreeFn = unsafe { std::mem::transmute::<usize, FreeFn>(free) };
    // SAFETY: as above
    unsafe { free(gmalloc as *mut c_void, ptr as *mut c_void) };
}

/// 16 bytes from the OS CSPRNG (call 6: `ProcessPrng`).
fn random_bytes() -> Option<[u8; 16]> {
    let mut bytes = [0u8; 16];
    // SAFETY: a writable 16-byte buffer
    let ok = unsafe { ProcessPrng(bytes.as_mut_ptr(), bytes.len()) };
    (ok != 0).then_some(bytes)
}

/// R2 4.4: rewrite an allowlisted URL to the API base. Both roles.
fn redirect_url(context: &Context<'_>, request: usize) {
    let Some(url) = read_url(request) else {
        return;
    };
    if in_flight(request) != Some(false) {
        return;
    }
    let Redirect::To { host, replacement } = redirect(context.api_base, &url.text) else {
        return;
    };
    let Ok(new_num) = i32::try_from(replacement.len() + 1) else {
        return;
    };
    let mut data = url.data;
    if url.max < new_num {
        // SAFETY: FMemory::Realloc's signature (R2 3.1); `data` is the engine's own URL buffer
        let realloc: ReallocFn = unsafe { std::mem::transmute::<usize, ReallocFn>(context.allocator.realloc) };
        // SAFETY: as above; on null the old buffer is still the request's
        data = unsafe { realloc(data as *mut c_void, replacement.len() * 2 + 2, 0) } as usize;
        if data == 0 {
            return;
        }
    }
    let field = request + layout::HTTP_REQUEST_URL;
    // SAFETY: `data` holds at least `new_num` units (max was large enough or Realloc grew it); the request is
    // readable for 0xD0 bytes and the URL field is the engine's to write
    unsafe {
        std::ptr::copy_nonoverlapping(replacement.as_ptr(), data as *mut u16, replacement.len());
        std::ptr::write_unaligned((data as *mut u16).add(replacement.len()), 0);
        std::ptr::write_volatile(field as *mut usize, data);
        std::ptr::write_unaligned((field + 8) as *mut i32, new_num);
        if url.max < new_num {
            std::ptr::write_unaligned((field + 12) as *mut i32, new_num);
        }
    }
    if !REDIRECT_LOGGED.swap(true, Ordering::SeqCst) {
        log::line(&redirect_line(&host));
    }
}

/// R2 5: a progression grant keeps the id of its first attempt, or gets a new one.
fn grant(context: &Context<'_>, request: usize) {
    if !wants_grant_id(context.server, context.mode) {
        return;
    }
    let Some(url) = read_url(request) else {
        return;
    };
    let Some(path) = grant_path(&url.text) else {
        return;
    };
    let logging = GRANTS.fetch_add(1, Ordering::Relaxed) < GRANT_LOG_BUDGET;
    let line = |text: String| {
        if logging {
            log::line(&text);
        }
    };
    match in_flight(request) {
        Some(false) => {}
        Some(true) => return line(grant_in_flight_line(&path)),
        None => return,
    }
    let Some(get) = vtable_slot(request, layout::HTTP_GET_HEADER_SLOT) else {
        return;
    };
    let name = Lent::of(GRANT_HEADER);
    let name_fstring = name.fstring();
    let mut out = FString::empty();
    // SAFETY: slot 2 of a readable vtable is GetHeader in its hidden-return form (R2 3.1)
    let get: GetHeaderFn = unsafe { std::mem::transmute::<usize, GetHeaderFn>(get) };
    // SAFETY: `out` is a zeroed FString of R2's, `name` lives past the call
    unsafe { get(request as *mut c_void, &mut out, &name_fstring) };
    let returned = out.data as usize;
    let reused = returned != 0 && out.num > 1;
    let existing = if reused {
        read_back(returned, out.num)
    } else {
        GrantValue::Unreadable
    };
    if returned != 0 {
        engine_free(context.allocator, returned);
    }
    if reused {
        line(grant_line(&path, &existing, true));
        return;
    }
    let Some(bytes) = random_bytes() else {
        // Fail closed: a refused grant beats one applied twice
        line(grant_no_random_line(&path));
        return;
    };
    let id: Vec<u16> = grant_id(bytes).encode_utf16().collect();
    if set_header(request, &name, &Lent::new(&id)) {
        line(grant_line(&path, &GrantValue::Text(id), false));
    }
}

/// A reused id's text: up to 255 units when readable.
fn read_back(data: usize, num: i32) -> GrantValue {
    if num > GRANT_READBACK_MAX_NUM {
        return GrantValue::Unreadable;
    }
    let Ok(count) = usize::try_from(num - 1) else {
        return GrantValue::Unreadable;
    };
    let mut bytes = vec![0u8; count * 2];
    if !read_bytes(data, &mut bytes) {
        return GrantValue::Unreadable;
    }
    GrantValue::Text(utf16(&bytes))
}

/// R2 6: the DeployServer's key, only on URLs under the API base (call 4). Never logged.
fn key_header(context: &Context<'_>, request: usize) {
    let Some(key) = context.key else {
        return;
    };
    if !context.server {
        return;
    }
    let (Some(url), Some(in_flight)) = (read_url(request), in_flight(request)) else {
        return;
    };
    let inputs = KeyInputs {
        server: context.server,
        mode: context.mode,
        key_present: true,
        in_flight,
        url: &url.text,
        api_base: context.api_base,
    };
    if sends_key(&inputs) {
        set_header(request, &Lent::of(KEY_HEADER), &Lent::new(key));
    }
}

/// The three steps in order (R2 3.2 step 3); a client also lends the help screen's reset its sign-in.
pub(crate) fn process(context: &Context<'_>, request: usize) {
    redirect_url(context, request);
    if context.server {
        grant(context, request);
        key_header(context, request);
    } else {
        authorization(context, request);
    }
}

/// The game's `Authorization` on its own backend requests, kept for the save reset's call (never logged).
fn authorization(context: &Context<'_>, request: usize) {
    let Some(url) = read_url(request) else {
        return;
    };
    if in_flight(request) != Some(false) || !rustic_isles_runtime_core::http::is_under(&url.text, context.api_base) {
        return;
    }
    let Some(get) = vtable_slot(request, layout::HTTP_GET_HEADER_SLOT) else {
        return;
    };
    let name = Lent::of("Authorization");
    let name_fstring = name.fstring();
    let mut out = FString::empty();
    // SAFETY: slot 2 of a readable vtable is GetHeader in its hidden-return form (R2 3.1)
    let get: GetHeaderFn = unsafe { std::mem::transmute::<usize, GetHeaderFn>(get) };
    // SAFETY: `out` is a zeroed FString of ours, `name` lives past the call
    unsafe { get(request as *mut c_void, &mut out, &name_fstring) };
    let returned = out.data as usize;
    if returned == 0 {
        return;
    }
    if out.num > 1
        && let GrantValue::Text(value) = read_header(returned, out.num)
    {
        crate::save_reset::remember_authorization(&value);
    }
    engine_free(context.allocator, returned);
}

/// A header value of up to 8 KiB, as [`read_back`] reads a grant id.
fn read_header(data: usize, num: i32) -> GrantValue {
    if num > 8193 {
        return GrantValue::Unreadable;
    }
    let Ok(count) = usize::try_from(num - 1) else {
        return GrantValue::Unreadable;
    };
    let mut bytes = vec![0u8; count * 2];
    if !read_bytes(data, &mut bytes) {
        return GrantValue::Unreadable;
    }
    GrantValue::Text(utf16(&bytes))
}

unsafe extern "C" fn process_request_detour(request: *mut c_void) -> u8 {
    let next = NEXT.load();
    if next == 0 {
        return 0;
    }
    // SAFETY: the broker wrote a function of ProcessRequest's signature into this cell
    let next: ProcessRequestFn = unsafe { std::mem::transmute::<usize, ProcessRequestFn>(next) };
    if !detaching() {
        crate::lifecycle::write_status_once();
        let body = catch_unwind(AssertUnwindSafe(|| {
            if let Some(&allocator) = ALLOCATOR.get() {
                let context = Context {
                    allocator,
                    api_base: api_base(),
                    server: config::is_server(),
                    mode: config::auth_mode(),
                    key: config::launch().and_then(|launch| launch.api_key.as_deref()),
                };
                process(&context, request as usize);
            }
        }));
        if body.is_err() && !PANICKED.swap(true, Ordering::SeqCst) {
            log::line("[Panic] HTTP-1 handler panicked; the request goes on to the chain");
        }
    }
    // SAFETY: the original argument; every request reaches the game
    unsafe { next(request) }
}

/// Installs HTTP-1 (both roles), after the dispatch chains.
pub fn install(base: usize) -> Result<(), i32> {
    let unknown = super::minhook::MH_UNKNOWN;
    let offset = |rva: u32| usize::try_from(rva).ok().and_then(|rva| base.checked_add(rva));
    let target = target("HTTP-1").ok_or(unknown)?;
    let address = offset(target.rva).ok_or(unknown)?;
    let allocator = Allocator {
        realloc: offset(data::FMEMORY_REALLOC).ok_or(unknown)?,
        gmalloc: offset(data::GMALLOC).ok_or(unknown)?,
    };
    let _ = ALLOCATOR.set(allocator);
    broker::hook(address, process_request_detour as *const () as usize, 0, &NEXT)
}

#[cfg(test)]
mod tests;
