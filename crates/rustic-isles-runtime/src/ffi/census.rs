//! The census's reader (spec-runtime-census 4.3): R0's region rule, with the last 64 passing regions remembered for
//! one census so a walk of the whole object array is not a `VirtualQuery` per read.

use super::memory::query;
use rustic_isles_runtime_core::memory::{Memory, Region, region_allows};
use std::cell::{Cell, RefCell};

/// Regions remembered, most recent first.
const REGIONS: usize = 64;

/// One census's reader; dropped when the census ends.
#[derive(Default)]
pub struct RegionCache {
    regions: RefCell<Vec<Region>>,
    queries: Cell<u64>,
}

impl RegionCache {
    /// `VirtualQuery` calls made so far.
    pub fn queries(&self) -> u64 {
        self.queries.get()
    }

    fn allows(&self, ptr: usize, size: usize, aligned: bool) -> bool {
        let mut regions = self.regions.borrow_mut();
        if let Some(hit) = regions
            .iter()
            .position(|&region| region_allows(ptr, size, Some(region), aligned))
        {
            if hit > 0 {
                let region = regions.remove(hit);
                regions.insert(0, region);
            }
            return true;
        }
        self.queries.set(self.queries.get() + 1);
        let region = query(ptr);
        if !region_allows(ptr, size, region, aligned) {
            return false;
        }
        if let Some(region) = region {
            regions.insert(0, region);
            regions.truncate(REGIONS);
        }
        true
    }
}

impl Memory for RegionCache {
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
        // SAFETY: the range lies in a region VirtualQuery reported committed and readable during this census, on the
        // game thread (census call 3); `copy` because a read can reach `out`'s own heap
        unsafe { std::ptr::copy(addr as *const u8, out.as_mut_ptr(), out.len()) };
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_through_the_cache() {
        let block = vec![7u8; 4096];
        let address = block.as_ptr() as usize;
        let cache = RegionCache::default();
        let mut out = [0u8; 16];
        assert!(cache.read(address, &mut out));
        assert_eq!(out, [7; 16]);
        let after_first = cache.queries();
        assert!(cache.read(address + 64, &mut out));
        assert_eq!(cache.queries(), after_first, "the region is remembered");
        assert!(!cache.read(0, &mut out));
        assert!(!cache.readable(16, 8), "the first page is never mapped");
    }
}
