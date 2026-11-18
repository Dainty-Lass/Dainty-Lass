//! Checked game memory (R0 5): every read goes through `VirtualQuery` first, so a bad pointer is `None`, never a
//! fault. Code checks and verified byte patches.

use super::win::{image_base, thread_id};
use rustic_isles_runtime_core::frame::FrameCache;
use rustic_isles_runtime_core::memory::{Memory, Region, region_allows};
use rustic_isles_runtime_core::patch::{PatchResult, decide};
use rustic_isles_runtime_core::pdata::Pdata;
use std::cell::RefCell;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use windows_sys::Win32::System::Diagnostics::Debug::FlushInstructionCache;
use windows_sys::Win32::System::Memory::{
    MEMORY_BASIC_INFORMATION, PAGE_EXECUTE_READWRITE, PAGE_PROTECTION_FLAGS, VirtualProtect, VirtualQuery,
};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

/// `VirtualQuery` on `ptr`.
pub fn query(ptr: usize) -> Option<Region> {
    if ptr == 0 {
        return None;
    }
    // SAFETY: plain data VirtualQuery fills; any address may be queried
    let info: MEMORY_BASIC_INFORMATION = unsafe {
        let mut info = std::mem::zeroed();
        let size = std::mem::size_of::<MEMORY_BASIC_INFORMATION>();
        if VirtualQuery(ptr as *const _, &mut info, size) != size {
            return None;
        }
        info
    };
    Some(Region {
        base: info.BaseAddress as usize,
        size: info.RegionSize,
        state: info.State,
        protect: info.Protect,
    })
}

/// R0 `readable(ptr, size)`: non-null, 8-byte aligned, committed, accessible, inside one region.
pub fn readable(ptr: usize, size: usize) -> bool {
    region_allows(ptr, size, query(ptr), true)
}

/// The region rule without the alignment rule (code bytes, unaligned fields).
pub fn readable_unaligned(ptr: usize, size: usize) -> bool {
    region_allows(ptr, size, query(ptr), false)
}

/// `read<T>(addr)`: `readable`, then a volatile read.
pub fn read<T: Copy>(addr: usize) -> Option<T> {
    // SAFETY: the range was just checked mapped and readable; volatile so the read happens as written
    readable(addr, std::mem::size_of::<T>()).then(|| unsafe { std::ptr::read_volatile(addr as *const T) })
}

/// `read_unaligned<T>(addr)` for code bytes and unaligned fields.
pub fn read_unaligned<T: Copy>(addr: usize) -> Option<T> {
    // SAFETY: the range was just checked mapped and readable
    readable_unaligned(addr, std::mem::size_of::<T>()).then(|| unsafe { std::ptr::read_unaligned(addr as *const T) })
}

/// Copies `out.len()` bytes from `addr` after the region check.
pub fn read_bytes(addr: usize, out: &mut [u8]) -> bool {
    if out.is_empty() {
        return true;
    }
    if !readable_unaligned(addr, out.len()) {
        return false;
    }
    // SAFETY: the source range was just checked. `copy`, not `copy_nonoverlapping`: a read past a heap string can
    // reach `out` itself, freshly allocated on the same heap
    unsafe { std::ptr::copy(addr as *const u8, out.as_mut_ptr(), out.len()) };
    true
}

/// This process's memory, for the core crate's engine model and `.pdata` search.
pub struct ProcessMemory;

impl Memory for ProcessMemory {
    fn readable(&self, ptr: usize, size: usize) -> bool {
        readable_this_frame(ptr, size)
    }

    fn read(&self, addr: usize, out: &mut [u8]) -> bool {
        read_bytes(addr, out)
    }
}

static FRAME: FrameCache = FrameCache::new();
static FRAME_THREAD: AtomicU32 = AtomicU32::new(0);

/// SP-3 records its thread and bumps the epoch at the top of every tick and around the engine tick (R0 5.2).
pub fn frame_tick(bump_only: bool) {
    if !bump_only {
        FRAME_THREAD.store(thread_id(), Ordering::Relaxed);
    }
    FRAME.bump();
}

/// Readable regions remembered for the current epoch, most recent first.
const FRAME_REGIONS: usize = 64;

thread_local! {
    static REGIONS: RefCell<(u32, Vec<Region>)> = const { RefCell::new((0, Vec::new())) };
}

/// `readable` with R0 5.2's lifetime but by region: on the game thread, the last 64 regions that passed are kept until
/// the epoch moves, so objects packed in a few heap regions cost a few queries a frame (spec-runtime-profile 4).
pub fn readable_region_this_frame(ptr: usize, size: usize) -> bool {
    let thread = FRAME_THREAD.load(Ordering::Relaxed);
    if thread == 0 || thread != thread_id() || super::flags::no_frame_pointer_cache() {
        return readable(ptr, size);
    }
    let epoch = FRAME.epoch();
    REGIONS.with(|cell| {
        let mut cell = cell.borrow_mut();
        if cell.0 != epoch {
            cell.0 = epoch;
            cell.1.clear();
        }
        let regions = &mut cell.1;
        if let Some(hit) = regions
            .iter()
            .position(|&region| region_allows(ptr, size, Some(region), true))
        {
            if hit > 0 {
                let region = regions.remove(hit);
                regions.insert(0, region);
            }
            return true;
        }
        let region = query(ptr);
        if !region_allows(ptr, size, region, true) {
            return false;
        }
        if let Some(region) = region {
            regions.insert(0, region);
            regions.truncate(FRAME_REGIONS);
        }
        true
    })
}

/// `read_bytes` through `readable_region_this_frame`, for a hot path that would otherwise pay a `VirtualQuery` per
/// call (spec-runtime-profile 4).
pub fn read_bytes_this_frame(addr: usize, out: &mut [u8]) -> bool {
    if out.is_empty() {
        return true;
    }
    if !readable_region_this_frame(addr, out.len()) {
        return false;
    }
    // SAFETY: the range lies in a region checked readable this epoch on the game thread (R0 5.2's guarantee), or was
    // queried just now off it; `copy` because a read can reach `out`'s own heap
    unsafe { std::ptr::copy(addr as *const u8, out.as_mut_ptr(), out.len()) };
    true
}

/// The game thread's id as SP-3 recorded it (0 before the first tick).
pub fn frame_thread() -> u32 {
    FRAME_THREAD.load(Ordering::Relaxed)
}

/// `readable_this_frame`: the server game thread's cache, else plain `readable`.
pub fn readable_this_frame(ptr: usize, size: usize) -> bool {
    let thread = FRAME_THREAD.load(Ordering::Relaxed);
    if thread == 0 || thread != thread_id() || super::flags::no_frame_pointer_cache() {
        return readable(ptr, size);
    }
    FRAME.readable(ptr, size, readable)
}

/// `patch_bytes` at an address: verify, write under `PAGE_EXECUTE_READWRITE`, restore, flush (R0 5.4).
pub fn patch_bytes_at(address: usize, expected: &[u8], replacement: &[u8]) -> PatchResult {
    let len = expected.len();
    if len == 0 || replacement.len() != len || !readable_unaligned(address, len) {
        return PatchResult::Unreadable;
    }
    let mut current = vec![0u8; len];
    if !read_bytes(address, &mut current) {
        return PatchResult::Unreadable;
    }
    let decision = decide(&current, expected, replacement);
    if decision != PatchResult::Applied {
        return decision;
    }
    // SAFETY: the range is committed and inside one region; its protection is restored after the copy
    unsafe {
        let mut old: PAGE_PROTECTION_FLAGS = 0;
        if VirtualProtect(address as *const _, len, PAGE_EXECUTE_READWRITE, &mut old) == 0 {
            return PatchResult::ProtectFailed;
        }
        std::ptr::copy_nonoverlapping(replacement.as_ptr(), address as *mut u8, len);
        let mut ignored: PAGE_PROTECTION_FLAGS = 0;
        VirtualProtect(address as *const _, len, old, &mut ignored);
        FlushInstructionCache(GetCurrentProcess(), address as *const _, len);
    }
    PatchResult::Applied
}

/// `patch_bytes(rva, expected, replacement)` in the game executable.
pub fn patch_bytes(rva: u32, expected: &[u8], replacement: &[u8]) -> PatchResult {
    match usize::try_from(rva).ok().and_then(|rva| image_base().checked_add(rva)) {
        Some(address) => patch_bytes_at(address, expected, replacement),
        None => PatchResult::Unreadable,
    }
}

fn pdata() -> Option<Pdata> {
    static PDATA: OnceLock<Option<Pdata>> = OnceLock::new();
    *PDATA.get_or_init(|| Pdata::locate(&ProcessMemory, image_base()))
}

/// `function_entry(rva)` in the game executable.
pub fn function_entry(rva: u32) -> Option<u32> {
    pdata()?.function_entry(&ProcessMemory, rva)
}

/// `primary_entry(rva)`, following chained unwind info.
pub fn primary_entry(rva: u32) -> Option<u32> {
    pdata()?.primary_entry(&ProcessMemory, rva)
}

/// `verify_entry(rva, prologue)`: a slice hooking "RVA + bytes" does not hook on `false`.
pub fn verify_entry(rva: u32, prologue: &[u8]) -> bool {
    pdata().is_some_and(|pdata| pdata.verify_entry(&ProcessMemory, rva, prologue))
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_EXECUTE_READ, PAGE_GUARD, PAGE_NOACCESS, PAGE_READWRITE,
        VirtualAlloc, VirtualFree,
    };

    const PAGE: usize = 4096;

    struct Pages(usize);

    impl Pages {
        fn new(count: usize, protect: PAGE_PROTECTION_FLAGS) -> Self {
            let base = unsafe { VirtualAlloc(std::ptr::null(), count * PAGE, MEM_COMMIT | MEM_RESERVE, protect) };
            assert!(!base.is_null());
            Self(base as usize)
        }
        fn protect(&self, page: usize, protect: PAGE_PROTECTION_FLAGS) {
            let mut old = 0;
            assert_ne!(
                unsafe { VirtualProtect((self.0 + page * PAGE) as *const _, PAGE, protect, &mut old) },
                0
            );
        }
    }

    impl Drop for Pages {
        fn drop(&mut self) {
            unsafe { VirtualFree(self.0 as *mut _, 0, MEM_RELEASE) };
        }
    }

    #[test]
    fn readable_rules() {
        let fresh = Pages::new(1, PAGE_READWRITE);
        assert!(readable(fresh.0, 8));
        assert!(readable(fresh.0 + PAGE - 8, 8));
        assert!(!readable(fresh.0 + 4, 8), "unaligned");
        assert!(readable_unaligned(fresh.0 + 4, 8));
        assert_eq!(read::<u64>(fresh.0), Some(0));
        let no_access = Pages::new(1, PAGE_NOACCESS);
        assert!(!readable(no_access.0, 8));
        assert_eq!(read::<u64>(no_access.0), None);
        let guard = Pages::new(1, PAGE_READWRITE | PAGE_GUARD);
        assert!(!readable(guard.0, 8));
        let split = Pages::new(2, PAGE_READWRITE);
        split.protect(1, PAGE_NOACCESS);
        assert!(readable(split.0 + PAGE - 8, 8));
        assert!(!readable(split.0 + PAGE - 8, 16), "crosses the region end");
        assert!(!readable(0, 8));
    }

    #[test]
    fn patch_on_code_bytes() {
        let page = Pages::new(1, PAGE_READWRITE);
        let site = page.0 + 0x10;
        unsafe { std::ptr::copy_nonoverlapping([0xBAu8, 0x60, 0xEA, 0x00, 0x00].as_ptr(), site as *mut u8, 5) };
        page.protect(0, PAGE_EXECUTE_READ);
        let expected = [0xBA, 0x60, 0xEA, 0x00, 0x00];
        let replacement = [0xBA, 0x88, 0x13, 0x00, 0x00];
        assert_eq!(patch_bytes_at(site, &expected, &replacement), PatchResult::Applied);
        let mut now = [0u8; 5];
        assert!(read_bytes(site, &mut now));
        assert_eq!(now, replacement);
        assert_eq!(
            query(site).map(|region| region.protect),
            Some(PAGE_EXECUTE_READ),
            "protection restored"
        );
        assert_eq!(
            patch_bytes_at(site, &expected, &replacement),
            PatchResult::AlreadyApplied
        );
        assert_eq!(
            patch_bytes_at(site, &[1, 2, 3, 4, 5], &[5, 4, 3, 2, 1]),
            PatchResult::Mismatch
        );
        let locked = Pages::new(1, PAGE_NOACCESS);
        assert_eq!(
            patch_bytes_at(locked.0, &expected, &replacement),
            PatchResult::Unreadable
        );
        assert_eq!(
            patch_bytes_at(page.0 + PAGE - 2, &expected, &replacement),
            PatchResult::Unreadable
        );
    }
}
