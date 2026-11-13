//! R15's unsafe code (spec-runtime-r15 7.3, 7.4): the checked writes into game memory, the virtual `ProcessEvent`
//! call, the loading evaluator's call, and a region-caching reader for the object-array scans.

use super::broker;
use super::memory::{ProcessMemory, query, read};
use super::win::image_base;
use rustic_isles_runtime_core::economy::image_size;
use rustic_isles_runtime_core::memory::{Memory, Region, region_allows};
use rustic_isles_runtime_core::offsets::r15;
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::sync::OnceLock;
use windows_sys::Win32::System::Memory::{
    PAGE_EXECUTE_READWRITE, PAGE_EXECUTE_WRITECOPY, PAGE_READWRITE, PAGE_WRITECOPY,
};

type ProcessEventFn = unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void);
type LoadedFn = unsafe extern "C" fn(*mut c_void) -> u8;

/// R0's `readable` and a writable protection (7.4).
fn writable(addr: usize, size: usize) -> bool {
    let region = query(addr);
    region_allows(addr, size, region, true)
        && region.is_some_and(|region| {
            matches!(
                region.protect & 0xFF,
                PAGE_READWRITE | PAGE_WRITECOPY | PAGE_EXECUTE_READWRITE | PAGE_EXECUTE_WRITECOPY
            )
        })
}

/// `write_i32`: a volatile aligned write when the range is readable and writable, else nothing.
pub fn write_i32(addr: usize, value: i32) -> bool {
    if !writable(addr, 4) || !addr.is_multiple_of(4) {
        return false;
    }
    // SAFETY: the range was just checked committed, writable and inside one region
    unsafe { std::ptr::write_volatile(addr as *mut i32, value) };
    true
}

/// `write_i64`.
pub fn write_i64(addr: usize, value: i64) -> bool {
    if !writable(addr, 8) {
        return false;
    }
    // SAFETY: as `write_i32`; `readable` already required 8-byte alignment
    unsafe { std::ptr::write_volatile(addr as *mut i64, value) };
    true
}

/// The executable's image range, from its PE header.
fn image_range() -> Option<(usize, usize)> {
    static RANGE: OnceLock<Option<(usize, usize)>> = OnceLock::new();
    *RANGE.get_or_init(|| {
        let base = image_base();
        let size = image_size(&ProcessMemory, base)?;
        Some((base, base.checked_add(size)?))
    })
}

/// `process_event(object, function, params)` through the object's vtable entry `0x44` (7.3), which lies in the
/// executable; `false` when a check failed and nothing was called.
pub fn process_event(object: usize, function: usize, params: usize) -> bool {
    if object == 0 || function == 0 {
        return false;
    }
    let Some(vtable) = read::<usize>(object) else {
        return false;
    };
    let Some(entry) = vtable.checked_add(r15::VTABLE_PROCESS_EVENT).and_then(read::<usize>) else {
        return false;
    };
    let Some((start, end)) = image_range() else {
        return false;
    };
    if entry < start || entry >= end {
        return false;
    }
    // SAFETY: the entry is the object's own ProcessEvent in the game's code; the arguments are a live object,
    // a UFunction and a params block the caller owns (or null)
    unsafe {
        let call = std::mem::transmute::<usize, ProcessEventFn>(entry);
        call(object as *mut c_void, function as *mut c_void, params as *mut c_void);
    }
    true
}

/// The game's loading evaluator for `object`, past every wrapper on LD-1 (6.3).
pub fn has_finished_loading(object: usize) -> bool {
    let Some(target) = usize::try_from(r15::HAS_FINISHED_LOADING)
        .ok()
        .and_then(|rva| image_base().checked_add(rva))
    else {
        return false;
    };
    let original = broker::original(target);
    // SAFETY: the game's `bool (UObject*)` evaluator (or its trampoline) on a live, readable object
    unsafe {
        let call = std::mem::transmute::<usize, LoadedFn>(original);
        call(object as *mut c_void) != 0
    }
}

/// Checked reads that remember the last few regions `VirtualQuery` returned, for one object-array scan: a scan
/// reads every object, and a query per read would stall the game thread for seconds. Lives for one scan only.
pub struct ScanMemory {
    regions: RefCell<[Option<Region>; 8]>,
    next: Cell<usize>,
}

impl Default for ScanMemory {
    fn default() -> Self {
        Self {
            regions: RefCell::new([None; 8]),
            next: Cell::new(0),
        }
    }
}

impl ScanMemory {
    fn allows(&self, ptr: usize, size: usize, aligned: bool) -> bool {
        if self
            .regions
            .borrow()
            .iter()
            .any(|&region| region.is_some() && region_allows(ptr, size, region, aligned))
        {
            return true;
        }
        let region = query(ptr);
        let allowed = region_allows(ptr, size, region, aligned);
        if allowed {
            let at = self.next.get();
            self.regions.borrow_mut()[at] = region;
            self.next.set((at + 1) % 8);
        }
        allowed
    }
}

impl Memory for ScanMemory {
    fn readable(&self, ptr: usize, size: usize) -> bool {
        self.allows(ptr, size, true)
    }

    fn read(&self, addr: usize, out: &mut [u8]) -> bool {
        if out.is_empty() {
            return true;
        }
        if !self.allows(addr, out.len(), false) {
            return false;
        }
        // SAFETY: the source range lies in a region this scan found committed and accessible
        unsafe { std::ptr::copy_nonoverlapping(addr as *const u8, out.as_mut_ptr(), out.len()) };
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Memory::{
        MEM_COMMIT, MEM_RELEASE, MEM_RESERVE, PAGE_NOACCESS, PAGE_PROTECTION_FLAGS, PAGE_READONLY, VirtualAlloc,
        VirtualFree,
    };

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
    fn checked_writes() {
        let page = Page::new(PAGE_READWRITE);
        assert!(write_i32(page.0 + 0x10, -7));
        assert!(write_i64(page.0 + 0x18, 18_000_000_000));
        assert_eq!(read::<i32>(page.0 + 0x10), Some(-7));
        assert_eq!(read::<i64>(page.0 + 0x18), Some(18_000_000_000));
        assert!(!write_i64(page.0 + 0x14, 1), "unaligned");
        assert!(!write_i64(page.0 + 4092, 1), "past the region");
        let read_only = Page::new(PAGE_READONLY);
        assert!(!write_i32(read_only.0, 1));
        assert!(!write_i64(read_only.0, 1));
        assert_eq!(read::<i64>(read_only.0), Some(0));
        let locked = Page::new(PAGE_NOACCESS);
        assert!(!write_i32(locked.0, 1));
        assert!(!write_i64(locked.0, 1));
    }

    #[test]
    fn scan_memory_reads() {
        let page = Page::new(PAGE_READWRITE);
        assert!(write_i64(page.0 + 8, 42));
        let memory = ScanMemory::default();
        assert_eq!(memory.ptr_at(page.0 + 8), Some(42));
        assert_eq!(memory.ptr_at(page.0 + 8), Some(42), "from the remembered region");
        assert!(!memory.readable(page.0 + 4090, 8));
        let locked = Page::new(PAGE_NOACCESS);
        assert_eq!(memory.u8_at(locked.0), None);
    }

    #[test]
    fn process_event_refuses_outside_the_image() {
        let page = Page::new(PAGE_READWRITE);
        // A vtable whose entry 0x44 points at the page itself: not the executable's code
        assert!(write_i64(page.0, i64::try_from(page.0 + 0x100).unwrap()));
        assert!(write_i64(page.0 + 0x100 + 0x220, i64::try_from(page.0).unwrap()));
        assert!(!process_event(page.0, page.0, 0));
        assert!(!process_event(0, page.0, 0));
    }
}
