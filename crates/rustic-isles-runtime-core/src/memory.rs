//! The pointer-safety rules (R0 5.1) and the memory reader the engine model reads through.

/// `MEM_COMMIT`.
pub const MEM_COMMIT: u32 = 0x1000;
/// `PAGE_NOACCESS`.
pub const PAGE_NOACCESS: u32 = 0x01;
/// `PAGE_GUARD`.
pub const PAGE_GUARD: u32 = 0x100;

/// What `VirtualQuery` said about the region holding an address.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    /// `BaseAddress`.
    pub base: usize,
    /// `RegionSize`.
    pub size: usize,
    /// `State`.
    pub state: u32,
    /// `Protect`.
    pub protect: u32,
}

/// R0 5.1: `[ptr, ptr+size)` committed, neither guard nor no-access, inside one region, no overflow; when
/// `aligned`, also non-null and 8-byte aligned (`readable`). Code reads pass `aligned = false`.
pub fn region_allows(ptr: usize, size: usize, region: Option<Region>, aligned: bool) -> bool {
    if ptr == 0 || (aligned && !ptr.is_multiple_of(8)) {
        return false;
    }
    let Some(region) = region else {
        return false;
    };
    if region.state != MEM_COMMIT || region.protect & (PAGE_GUARD | PAGE_NOACCESS) != 0 || region.protect == 0 {
        return false;
    }
    let Some(end) = ptr.checked_add(size) else {
        return false;
    };
    let Some(region_end) = region.base.checked_add(region.size) else {
        return false;
    };
    ptr >= region.base && end <= region_end
}

/// Checked reads of another process's (the game's) memory. A failed read is `false`, never a fault.
pub trait Memory {
    /// R0 `readable(ptr, size)`: mapped, committed, 8-byte aligned.
    fn readable(&self, ptr: usize, size: usize) -> bool;
    /// Copies `out.len()` bytes from `addr` when the range passes the region rule (no alignment rule).
    fn read(&self, addr: usize, out: &mut [u8]) -> bool;

    /// A little-endian `u16`.
    fn u16_at(&self, addr: usize) -> Option<u16> {
        let mut bytes = [0u8; 2];
        self.read(addr, &mut bytes).then(|| u16::from_le_bytes(bytes))
    }
    /// A little-endian `u32`.
    fn u32_at(&self, addr: usize) -> Option<u32> {
        let mut bytes = [0u8; 4];
        self.read(addr, &mut bytes).then(|| u32::from_le_bytes(bytes))
    }
    /// A little-endian `i32`.
    fn i32_at(&self, addr: usize) -> Option<i32> {
        let mut bytes = [0u8; 4];
        self.read(addr, &mut bytes).then(|| i32::from_le_bytes(bytes))
    }
    /// A pointer.
    fn ptr_at(&self, addr: usize) -> Option<usize> {
        let mut bytes = [0u8; 8];
        self.read(addr, &mut bytes)
            .then(|| u64::from_le_bytes(bytes))
            .and_then(|value| usize::try_from(value).ok())
    }
    /// A byte.
    fn u8_at(&self, addr: usize) -> Option<u8> {
        let mut bytes = [0u8; 1];
        self.read(addr, &mut bytes).then_some(bytes[0])
    }
}

/// A byte buffer standing for memory at `base` (tests, fuzzing). Every committed byte is readable.
pub struct BufferMemory<'a> {
    /// Where the buffer pretends to live.
    pub base: usize,
    /// The bytes.
    pub bytes: &'a [u8],
}

impl Memory for BufferMemory<'_> {
    fn readable(&self, ptr: usize, size: usize) -> bool {
        let region = Region {
            base: self.base,
            size: self.bytes.len(),
            state: MEM_COMMIT,
            protect: 0x04,
        };
        region_allows(ptr, size, Some(region), true)
    }

    fn read(&self, addr: usize, out: &mut [u8]) -> bool {
        let Some(offset) = addr.checked_sub(self.base) else {
            return false;
        };
        let Some(end) = offset.checked_add(out.len()) else {
            return false;
        };
        match self.bytes.get(offset..end) {
            Some(source) => {
                out.copy_from_slice(source);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RW: u32 = 0x04;

    fn region(base: usize, size: usize, state: u32, protect: u32) -> Option<Region> {
        Some(Region {
            base,
            size,
            state,
            protect,
        })
    }

    #[test]
    fn region_rule() {
        let committed = region(0x1000, 0x1000, MEM_COMMIT, RW);
        assert!(region_allows(0x1000, 8, committed, true));
        assert!(region_allows(0x1ff8, 8, committed, true));
        assert!(!region_allows(0x1ffc, 8, committed, false), "crosses the region end");
        assert!(!region_allows(0x1004, 4, committed, true), "unaligned");
        assert!(region_allows(0x1004, 4, committed, false));
        assert!(!region_allows(0, 8, committed, true));
        assert!(!region_allows(0x1000, 8, None, true));
        assert!(
            !region_allows(0x1000, 8, region(0x1000, 0x1000, 0x2000, RW), true),
            "reserved"
        );
        assert!(!region_allows(
            0x1000,
            8,
            region(0x1000, 0x1000, MEM_COMMIT, PAGE_NOACCESS),
            true
        ));
        assert!(!region_allows(
            0x1000,
            8,
            region(0x1000, 0x1000, MEM_COMMIT, RW | PAGE_GUARD),
            true
        ));
        assert!(!region_allows(
            usize::MAX - 7,
            16,
            region(usize::MAX - 0xfff, 0x1000, MEM_COMMIT, RW),
            true
        ));
    }

    #[test]
    fn buffer_reads() {
        let bytes = [1u8, 0, 0, 0, 2, 0, 0, 0, 0xff];
        let memory = BufferMemory {
            base: 0x100,
            bytes: &bytes,
        };
        assert_eq!(memory.u32_at(0x100), Some(1));
        assert_eq!(memory.i32_at(0x104), Some(2));
        assert_eq!(memory.u8_at(0x108), Some(0xff));
        assert_eq!(memory.u32_at(0x106), None);
        assert_eq!(memory.u8_at(0xff), None);
        assert!(memory.readable(0x100, 8));
        assert!(!memory.readable(0x104, 4));
    }
}
